#!/usr/bin/env python3
"""ci/bench_regression.py — benchmark regression gate (task 8-26).

Re-runs the benchmark suite and fails if any *comparable* benchmark's
jinn/c ratio regressed by more than REGRESSION_PCT versus the committed
baseline in benchmarks/results.csv.

Why ratios and not milliseconds: the committed baseline was measured on a
developer machine and this gate runs on whatever CI hands us. Absolute
times are not portable across hardware; the jinn/c ratio of the *same
run on the same machine* is. A codegen or runtime regression moves the
ratio on any hardware.

What is enforced:
  - rows whose `comparability` column is "comparable" (both sides run the
    same algorithm against the same memory hierarchy), AND
  - whose baseline jinn median is >= MIN_ENFORCE_MS (sub-millisecond rows
    are dominated by process-spawn and timer noise, where a 10% swing is
    tens of microseconds).
Everything skipped is listed by name — a shrinking enforced set is visible
in the log, never silent.

Also failed: a benchmark present in the baseline that no longer compiles,
times out, or exits non-zero (a regression to not-running at all).
"""

import csv
import json
import os
import subprocess
import sys

REGRESSION_PCT = 10.0  # DoD threshold: >10% ratio regression fails
MIN_ENFORCE_MS = 50.0  # baseline jinn median below this -> report, don't gate
RUNS = 3               # medians of 3 on shared runners; local baseline uses 5

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASELINE_CSV = os.path.join(ROOT, "benchmarks", "results.csv")
FRESH_JSON = os.path.join(ROOT, "benchmarks", "results.json")


def load_baseline():
    """Parse the committed results.csv: leading `#` metadata lines, then a
    header row with a `comparability` column."""
    rows = {}
    with open(BASELINE_CSV) as f:
        reader = csv.DictReader(line for line in f if not line.startswith("#"))
        for row in reader:
            if row.get("opt") != "O3":
                continue
            rows[row["benchmark"]] = row
    if not rows:
        sys.exit(f"no O3 baseline rows found in {BASELINE_CSV}")
    return rows


def run_suite():
    cmd = [
        sys.executable, os.path.join(ROOT, "run_benchmarks.py"),
        f"--runs={RUNS}", "--warmup=1", "--langs=jinn,c", "--quiet",
    ]
    print(f"$ {' '.join(cmd)}", flush=True)
    r = subprocess.run(cmd, cwd=ROOT)
    if r.returncode != 0:
        sys.exit("benchmark suite itself failed to run")
    with open(FRESH_JSON) as f:
        return json.load(f)["O3"]


def main():
    baseline = load_baseline()  # read BEFORE the run touches any output file
    fresh = run_suite()

    failures, skipped, enforced = [], [], []
    for name, base in sorted(baseline.items()):
        cur = fresh.get(name)
        if cur is None or "jinn_ms" not in cur:
            err = (cur or {}).get("jinn_err", "missing from fresh run")
            failures.append(f"{name}: baseline exists but fresh run has no result ({err})")
            continue

        comparability = base.get("comparability", "comparable")
        base_ratio = float(base["ratio_c"]) if base.get("ratio_c") else None
        cur_ratio = cur.get("ratio_c")
        base_jinn_ms = float(base["jinn_median_ms"]) if base.get("jinn_median_ms") else None

        if comparability != "comparable" or base_ratio is None:
            skipped.append(f"{name} [{comparability}]")
            continue
        if base_jinn_ms is None or base_jinn_ms < MIN_ENFORCE_MS:
            skipped.append(f"{name} [below {MIN_ENFORCE_MS:.0f}ms noise floor]")
            continue
        if cur_ratio is None:
            failures.append(f"{name}: comparable baseline but fresh run produced no jinn/c ratio")
            continue

        delta_pct = (cur_ratio / base_ratio - 1.0) * 100.0
        enforced.append((name, base_ratio, cur_ratio, delta_pct))
        if delta_pct > REGRESSION_PCT:
            failures.append(
                f"{name}: jinn/c ratio {base_ratio:.2f} -> {cur_ratio:.2f} "
                f"(+{delta_pct:.1f}%, threshold {REGRESSION_PCT:.0f}%)"
            )

    print(f"\nenforced ({len(enforced)}):")
    for name, b, c, d in enforced:
        marker = " <-- REGRESSION" if d > REGRESSION_PCT else ""
        print(f"  {name:<20} {b:.2f} -> {c:.2f}  ({d:+.1f}%){marker}")
    print(f"\nskipped, not gated ({len(skipped)}):")
    for s in skipped:
        print(f"  {s}")

    if failures:
        print(f"\nFAIL: {len(failures)} regression(s):")
        for f in failures:
            print(f"  {f}")
        sys.exit(1)
    print("\nOK: no comparable benchmark regressed beyond "
          f"{REGRESSION_PCT:.0f}% of its baseline jinn/c ratio")


if __name__ == "__main__":
    main()
