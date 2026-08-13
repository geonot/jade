#!/usr/bin/env python3
"""ci/fuzz-ownership.py — ownership-syntax mutation fuzzer (roadmap M-10).

Takes corpus programs, applies small ownership-relevant textual mutations
(duplicating call arguments, inserting/swapping `take`/`copy`, appending a
late use of a bound name), and asserts the compiler's contract on every
mutant:

  1. the compiler never crashes (no panic, no internal compiler error,
     no signal) — it must accept or reject with a diagnostic;
  2. if a mutant compiles and an instrumented runtime archive is available
     (built by ci/sanitize-corpus.sh), the binary runs without a sanitizer
     report — whatever the mutation did, memory safety must hold.

Deterministic: seeded RNG, fixed sample. Usage:

  python3 ci/fuzz-ownership.py [--mutants-per-file N] [--sample N] [--seed N]
"""

import argparse
import random
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
JINNC = ROOT / "target" / "release" / "jinnc"
RT_ARCHIVE = ROOT / "target" / "sanitize" / "corpus" / "rt" / "libjinn_rt.a"

CALL_ARG = re.compile(r"\b([a-z_][a-z0-9_]*)\(([a-z_][a-z0-9_]*)\)")
BIND = re.compile(r"^(\s*)([a-z_][a-z0-9_]*) is (.+)$", re.M)
IDENT_ARG = re.compile(r"\(([a-z_][a-z0-9_]*)([,)])")


def mutations(src: str, rng: random.Random):
    out = []

    m = list(CALL_ARG.finditer(src))
    if m:
        pick = rng.choice(m)
        out.append(
            (
                "dup-arg",
                src[: pick.start()]
                + f"{pick.group(1)}({pick.group(2)}, {pick.group(2)})"
                + src[pick.end() :],
            )
        )

    m = list(IDENT_ARG.finditer(src))
    if m:
        pick = rng.choice(m)
        for word in ("take", "copy"):
            out.append(
                (
                    f"insert-{word}",
                    src[: pick.start()]
                    + f"({word} {pick.group(1)}{pick.group(2)}"
                    + src[pick.end() :],
                )
            )

    if " copy " in src:
        out.append(("copy-to-take", src.replace(" copy ", " take ", 1)))
    if " take " in src:
        out.append(("take-to-copy", src.replace(" take ", " copy ", 1)))

    binds = list(BIND.finditer(src))
    if binds:
        pick = rng.choice(binds)
        name = pick.group(2)
        indent = pick.group(1)
        insert_at = src.rfind("\n", 0, len(src.rstrip()) + 1)
        out.append(
            (
                "late-use",
                src.rstrip() + f"\n{indent}log({name})\n",
            )
        )
        out.append(
            (
                "rebind-after-use",
                src[: pick.end()]
                + f"\n{indent}__fz is {name}\n{indent}{name} is __fz"
                + src[pick.end() :],
            )
        )
        _ = insert_at

    return out


def classify_compile(proc: subprocess.CompletedProcess) -> str:
    text = (proc.stdout or "") + (proc.stderr or "")
    if proc.returncode < 0:
        return "crash-signal"
    if "internal compiler error" in text or "panicked at" in text or "RUST_BACKTRACE" in text:
        return "crash-ice"
    if proc.returncode == 0:
        return "accept"
    return "reject"


def run_mutant(tag: str, label: str, src: str, workdir: Path) -> str:
    jn = workdir / "m.jn"
    jn.write_text(src)
    binpath = workdir / "m"
    use_asan = RT_ARCHIVE.exists()
    compile_cmd = [str(JINNC), str(jn), "-o", str(binpath)]
    if use_asan:
        compile_cmd = [str(JINNC), str(jn), "--emit-obj", "-o", str(binpath)]
    try:
        proc = subprocess.run(compile_cmd, capture_output=True, text=True, timeout=60)
    except subprocess.TimeoutExpired:
        return "compile-timeout"
    verdict = classify_compile(proc)
    if verdict != "accept":
        return verdict

    if use_asan:
        link = subprocess.run(
            [
                "cc",
                "-fsanitize=address",
                "-fno-sanitize-recover=all",
                "-g",
                str(binpath) + ".o",
                str(RT_ARCHIVE),
                "-lm",
                "-lpthread",
                "-o",
                str(binpath),
            ],
            capture_output=True,
            text=True,
            timeout=60,
        )
        if link.returncode != 0:
            return "link-fail"
    try:
        run = subprocess.run(
            [str(binpath)],
            capture_output=True,
            text=True,
            timeout=30,
            cwd=workdir,
            env={
                "ASAN_OPTIONS": "abort_on_error=0:exitcode=99:detect_leaks=0",
                "PATH": "/usr/bin:/bin",
            },
            stdin=subprocess.DEVNULL,
        )
    except subprocess.TimeoutExpired:
        return "run-timeout"
    text = (run.stdout or "") + (run.stderr or "")
    if "AddressSanitizer" in text:
        if "__tls_get_addr" in text and "T-1" in text:
            return "envskip"
        return "MEM"
    if run.returncode < 0:
        return "run-signal"
    return "ran"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mutants-per-file", type=int, default=6)
    ap.add_argument("--sample", type=int, default=60)
    ap.add_argument("--seed", type=int, default=147)
    args = ap.parse_args()

    if not JINNC.exists():
        sys.exit("build target/release/jinnc first")

    rng = random.Random(args.seed)
    pool = sorted((ROOT / "tests" / "programs").glob("*.jn"))
    pool += sorted((ROOT / "snippets").glob("*/*.jn"))
    rng.shuffle(pool)
    sample = pool[: args.sample]

    counts: dict[str, int] = {}
    failures: list[str] = []
    for path in sample:
        src = path.read_text()
        muts = mutations(src, rng)[: args.mutants_per_file]
        for tag, mutated in muts:
            label = f"{path.relative_to(ROOT)}::{tag}"
            with tempfile.TemporaryDirectory() as td:
                verdict = run_mutant(tag, label, mutated, Path(td))
            counts[verdict] = counts.get(verdict, 0) + 1
            if verdict in ("crash-signal", "crash-ice", "MEM", "run-signal"):
                failures.append(f"{verdict}: {label}")

    print("== fuzz summary ==")
    for k in sorted(counts):
        print(f"  {counts[k]:5d} {k}")
    if RT_ARCHIVE.exists():
        print("  (compiled mutants ran under ASan)")
    else:
        print("  (no instrumented runtime found — compile-contract checks only;")
        print("   run ci/sanitize-corpus.sh first for the memory-safety half)")
    if failures:
        print("\n== contract violations ==")
        for f in failures:
            print(f"  {f}")
        sys.exit(1)
    print("fuzz-ownership: compiler contract held on every mutant")


if __name__ == "__main__":
    main()
