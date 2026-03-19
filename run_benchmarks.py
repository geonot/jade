#!/usr/bin/env python3
"""Jade benchmark runner — Jade vs C vs Rust vs Python with full statistics.

Usage:
    python3 run_benchmarks.py                        # default: O3, 5 runs, jade+c
    python3 run_benchmarks.py --opt=all --runs=7     # O0-O3, 7 runs each
    python3 run_benchmarks.py --langs=jade,c,rust    # jade + C + Rust
    python3 run_benchmarks.py --save=v0.0.0-rc1      # tag this run in history
    python3 run_benchmarks.py --compare              # compare vs last saved run
    python3 run_benchmarks.py --bench=fib,sieve      # run only matching benchmarks
    python3 run_benchmarks.py --warmup=2             # 2 warmup runs (not measured)
    python3 run_benchmarks.py --csv                  # also emit CSV file
    python3 run_benchmarks.py --sort=ratio            # sort output by jade/c ratio
    python3 run_benchmarks.py --sort=jade             # sort by jade median time
    python3 run_benchmarks.py --quiet                # compact output, less detail
    python3 run_benchmarks.py --detail               # show min/max/stddev/variance
"""

import os, subprocess, sys, time, json, shutil, platform, math, re, csv as csvmod
from datetime import datetime
from io import StringIO

JADE_DIR = os.path.dirname(os.path.abspath(__file__))
JADEC = os.path.join(JADE_DIR, "target", "release", "jadec")
BENCH_DIR = os.path.join(JADE_DIR, "benchmarks")
CMP_DIR = os.path.join(BENCH_DIR, "comparison")
HISTORY_FILE = os.path.join(BENCH_DIR, "history.json")
CC = "clang"
RUSTC = "rustc"
PYTHON = "python3"

ALL_LANGS = ["jade", "c", "rust", "python"]

def build_compiler():
    print("Building jadec (release)...")
    env = os.environ.copy()
    env["LLVM_SYS_211_PREFIX"] = "/usr/lib/llvm-21"
    r = subprocess.run(["cargo", "build", "--release"], cwd=JADE_DIR, env=env, capture_output=True)
    if r.returncode != 0:
        print("Build failed:", r.stderr.decode())
        sys.exit(1)
    print("Build OK\n")


def compile_jade(jade_path, opt, out_dir):
    name = os.path.basename(jade_path).replace(".jade", "")
    out = os.path.join(out_dir, f"{name}_jade_O{opt}")
    r = subprocess.run([JADEC, jade_path, "-o", out, f"--opt={opt}"], capture_output=True)
    return (out, None) if r.returncode == 0 else (None, r.stderr.decode())


def compile_c(c_path, opt, out_dir):
    name = os.path.basename(c_path).replace(".c", "")
    out = os.path.join(out_dir, f"{name}_c_O{opt}")
    r = subprocess.run([CC, f"-O{opt}", "-o", out, c_path, "-lm"], capture_output=True)
    return (out, None) if r.returncode == 0 else (None, r.stderr.decode())


def compile_rust(rs_path, opt, out_dir):
    name = os.path.basename(rs_path).replace(".rs", "")
    out = os.path.join(out_dir, f"{name}_rs_O{opt}")
    opt_flag = {0: "0", 1: "1", 2: "2", 3: "3"}[opt]
    r = subprocess.run([RUSTC, "-C", f"opt-level={opt_flag}", "-o", out, rs_path], capture_output=True)
    return (out, None) if r.returncode == 0 else (None, r.stderr.decode())


def time_binary(binary, runs, warmup, timeout):
    for _ in range(warmup):
        try:
            subprocess.run([binary], capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            pass
    times = []
    output = None
    for _ in range(runs):
        start = time.perf_counter()
        try:
            r = subprocess.run([binary], capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return None, None, "timeout"
        elapsed = time.perf_counter() - start
        if r.returncode != 0:
            return None, None, f"exit {r.returncode}"
        times.append(elapsed)
        output = r.stdout.decode().strip()
    return times, output, None


def time_python(py_path, runs, warmup, timeout):
    for _ in range(warmup):
        try:
            subprocess.run([PYTHON, py_path], capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            pass
    times = []
    output = None
    for _ in range(runs):
        start = time.perf_counter()
        try:
            r = subprocess.run([PYTHON, py_path], capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return None, None, "timeout"
        elapsed = time.perf_counter() - start
        if r.returncode != 0:
            return None, None, f"exit {r.returncode}"
        times.append(elapsed)
        output = r.stdout.decode().strip()
    return times, output, None


def calc_stats(times):
    """Return (min, median, mean, max, stddev, variance, iqr) from a list of times."""
    t = sorted(times)
    n = len(t)
    mn, mx = t[0], t[-1]
    median = t[n // 2]
    mean = sum(t) / n
    variance = sum((x - mean) ** 2 for x in t) / n if n > 1 else 0.0
    stddev = math.sqrt(variance)
    q1 = t[n // 4] if n >= 4 else mn
    q3 = t[3 * n // 4] if n >= 4 else mx
    iqr = q3 - q1
    return {"min": mn, "median": median, "mean": mean, "max": mx,
            "stddev": stddev, "variance": variance, "iqr": iqr, "samples": t}


def ms(s):
    if s is None:
        return "-"
    if s < 0.001:
        return f"{s*1e6:.0f}us"
    if s < 1.0:
        return f"{s*1000:.1f}ms"
    return f"{s:.2f}s"


def ratio_str(jade_s, other_s):
    if jade_s is None or other_s is None:
        return "-"
    if other_s == 0:
        return "inf"
    r = jade_s / other_s
    return f"{r:.2f}x"


def cpu_info():
    """Detect CPU model and governor if available."""
    info = {"model": "unknown", "governor": "unknown", "cores": os.cpu_count()}
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    info["model"] = line.split(":", 1)[1].strip()
                    break
    except (IOError, OSError):
        pass
    try:
        gov_path = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"
        with open(gov_path) as f:
            info["governor"] = f.read().strip()
    except (IOError, OSError):
        pass
    return info


def load_history():
    if os.path.exists(HISTORY_FILE):
        with open(HISTORY_FILE) as f:
            return json.load(f)
    return []


def save_history(history):
    with open(HISTORY_FILE, "w") as f:
        json.dump(history, f, indent=2)


def discover_benchmarks(bench_filter=None):
    """Find .jade benchmarks, optionally filtered by comma-separated patterns."""
    all_benches = sorted(f.replace(".jade", "") for f in os.listdir(BENCH_DIR) if f.endswith(".jade"))
    if not bench_filter:
        return all_benches
    patterns = [p.strip() for p in bench_filter.split(",")]
    return [b for b in all_benches if any(p in b for p in patterns)]


def discover_comparison_files():
    """Find all C, Rust, Python comparison files."""
    c_files, rs_files, py_files = {}, {}, {}
    if os.path.isdir(CMP_DIR):
        for f in os.listdir(CMP_DIR):
            name = f.rsplit(".", 1)[0]
            if f.endswith(".c"):
                c_files[name] = os.path.join(CMP_DIR, f)
            elif f.endswith(".rs"):
                rs_files[name] = os.path.join(CMP_DIR, f)
            elif f.endswith(".py"):
                py_files[name] = os.path.join(CMP_DIR, f)
    return c_files, rs_files, py_files


def run_lang(lang, name, opt, runs, warmup, timeout, out_dir, c_files, rs_files, py_files):
    """Compile and time a single benchmark for a single language.
    Returns (stats_dict | None, output_str | None, error_str | None, raw_times | None)
    """
    if lang == "jade":
        binary, err = compile_jade(os.path.join(BENCH_DIR, f"{name}.jade"), opt, out_dir)
        if err:
            return None, None, err.strip()[:80], None
        times, out, rerr = time_binary(binary, runs, warmup, timeout)
        if rerr:
            return None, None, rerr, None
        return calc_stats(times), out, None, times

    elif lang == "c":
        if name not in c_files:
            return None, None, None, None
        binary, err = compile_c(c_files[name], opt, out_dir)
        if err:
            return None, None, err.strip()[:80], None
        times, out, rerr = time_binary(binary, runs, warmup, timeout)
        if rerr:
            return None, None, rerr, None
        return calc_stats(times), out, None, times

    elif lang == "rust":
        if name not in rs_files:
            return None, None, None, None
        binary, err = compile_rust(rs_files[name], opt, out_dir)
        if err:
            return None, None, err.strip()[:80], None
        times, out, rerr = time_binary(binary, runs, warmup, timeout)
        if rerr:
            return None, None, rerr, None
        return calc_stats(times), out, None, times

    elif lang == "python":
        if name not in py_files:
            return None, None, None, None
        times, out, rerr = time_python(py_files[name], runs, warmup, timeout)
        if rerr:
            return None, None, rerr, None
        return calc_stats(times), out, None, times

    return None, None, "unknown lang", None


def print_detail_block(name, lang_stats):
    """Print a detailed stats block for one benchmark showing all stats for all langs."""
    print(f"  {name}:")
    for lang, st in lang_stats.items():
        if st is None:
            continue
        print(f"    {lang:>6}: min={ms(st['min'])}  median={ms(st['median'])}  "
              f"mean={ms(st['mean'])}  max={ms(st['max'])}  "
              f"stddev={ms(st['stddev'])}  var={st['variance']*1e6:.1f}us²  "
              f"iqr={ms(st['iqr'])}")


def run_suite(opt_levels, runs, langs, timeout, save_tag, warmup, bench_filter,
              sort_by, quiet, detail, emit_csv, emit_json):
    if "jade" in langs:
        build_compiler()

    benchmarks = discover_benchmarks(bench_filter)
    if not benchmarks:
        print("No benchmarks matched filter.")
        return

    c_files, rs_files, py_files = discover_comparison_files()
    out_dir = os.path.join(BENCH_DIR, "_build")
    os.makedirs(out_dir, exist_ok=True)
    all_results = {}
    all_raw = {}  # for JSON/CSV export with raw times

    hw = cpu_info()
    if not quiet:
        print(f"CPU: {hw['model']}  |  Cores: {hw['cores']}  |  Governor: {hw['governor']}")
        print(f"Runs: {runs}  |  Warmup: {warmup}  |  Timeout: {timeout}s")

    for opt in opt_levels:
        active = [l for l in langs if l != "python"]
        cols = [l.upper() for l in active]
        ratios_from = [l for l in active if l != "jade"]
        if "python" in langs:
            cols.append("PYTHON")
            ratios_from.append("python")

        w = 12
        print(f"\n{'='*120}")
        print(f"  -O{opt}  |  {runs} runs  |  warmup: {warmup}  |  langs: {', '.join(langs)}")
        print(f"{'='*120}")

        hdr = f"{'Bench':<18}"
        for c in cols:
            hdr += f" {c:>{w}}"
        if detail:
            for c in cols:
                hdr += f" {'σ('+c+')':>{w}}"
        for r in ratios_from:
            hdr += f" {'J/'+r.upper():>{w}}"
        if not quiet:
            hdr += f"  {'Output':>10}"
        print(hdr)
        print("-" * len(hdr))

        level = {}
        raw_level = {}
        totals = {l: 0.0 for l in langs}
        rows_for_sort = []

        for name in benchmarks:
            entry = {}
            raw_entry = {}
            medians = {}
            stddevs = {}
            lang_stats_map = {}

            for lang in langs:
                st, out, err, raw_times = run_lang(
                    lang, name, opt, runs, warmup, timeout,
                    out_dir, c_files, rs_files, py_files)
                if err:
                    entry[f"{lang}_err"] = err
                elif st:
                    medians[lang] = st["median"]
                    stddevs[lang] = st["stddev"]
                    totals[lang] += st["median"]
                    entry[f"{lang}_ms"] = round(st["median"] * 1000, 2)
                    entry[f"{lang}_stddev_ms"] = round(st["stddev"] * 1000, 3)
                    entry[f"{lang}_variance_ms2"] = round(st["variance"] * 1e6, 3)
                    entry[f"{lang}_min_ms"] = round(st["min"] * 1000, 2)
                    entry[f"{lang}_max_ms"] = round(st["max"] * 1000, 2)
                    entry[f"{lang}_mean_ms"] = round(st["mean"] * 1000, 2)
                    entry[f"{lang}_iqr_ms"] = round(st["iqr"] * 1000, 3)
                    lang_stats_map[lang] = st
                    if raw_times:
                        raw_entry[lang] = [round(t * 1000, 3) for t in raw_times]
                    if lang == "jade" and out:
                        entry["output"] = (out or "")[:50]

            # Build display row
            row = f"{name:<18}"
            for lang in [l for l in active] + (["python"] if "python" in langs else []):
                if f"{lang}_err" in entry:
                    row += f" {'ERR':>{w}}"
                elif lang in medians:
                    row += f" {ms(medians[lang]):>{w}}"
                else:
                    row += f" {'-':>{w}}"

            if detail:
                for lang in [l for l in active] + (["python"] if "python" in langs else []):
                    if lang in stddevs:
                        row += f" {'±'+ms(stddevs[lang]):>{w}}"
                    else:
                        row += f" {'-':>{w}}"

            jmed = medians.get("jade")
            for r in ratios_from:
                omed = medians.get(r)
                row += f" {ratio_str(jmed, omed):>{w}}"
                if jmed and omed:
                    entry[f"ratio_{r}"] = round(jmed / omed, 2)

            if not quiet:
                out_val = entry.get("output", "")
                row += f"  {out_val[:10]:>10}"

            rows_for_sort.append((name, row, entry, raw_entry, jmed, medians, lang_stats_map))

        # Sort rows
        if sort_by == "jade":
            rows_for_sort.sort(key=lambda x: x[4] if x[4] else float("inf"))
        elif sort_by == "ratio":
            rows_for_sort.sort(key=lambda x: x[2].get("ratio_c", float("inf")))
        elif sort_by == "name":
            rows_for_sort.sort(key=lambda x: x[0])

        for name, row, entry, raw_entry, jmed, medians, lang_stats_map in rows_for_sort:
            print(row)
            level[name] = entry
            raw_level[name] = raw_entry

        # Detail block (if --detail)
        if detail:
            print()
            for name, _, _, _, _, _, lang_stats_map in rows_for_sort:
                print_detail_block(name, lang_stats_map)
            print()

        # Totals
        hdr_len = len(f"{'Bench':<18}") + sum(w + 1 for _ in cols)
        if detail:
            hdr_len += sum(w + 1 for _ in cols)
        hdr_len += sum(w + 1 for _ in ratios_from)
        print("-" * max(hdr_len, 80))
        row = f"{'TOTAL':<18}"
        for l in [l for l in active] + (["python"] if "python" in langs else []):
            row += f" {ms(totals.get(l, 0)):>{w}}"
        if detail:
            row += f" {' ':>{w}}" * len(cols)  # no stddev for totals
        jt = totals.get("jade", 0)
        for r in ratios_from:
            ot = totals.get(r, 0)
            row += f" {ratio_str(jt, ot):>{w}}"
        print(row)

        all_results[f"O{opt}"] = level
        all_raw[f"O{opt}"] = raw_level

    shutil.rmtree(out_dir, ignore_errors=True)

    # Save results
    out_path = os.path.join(BENCH_DIR, "results.json")
    with open(out_path, "w") as f:
        json.dump(all_results, f, indent=2)
    print(f"\nResults -> {out_path}")

    # JSON export with raw per-run data
    if emit_json:
        json_path = os.path.join(BENCH_DIR, "results_full.json")
        full = {
            "timestamp": datetime.now().isoformat(),
            "platform": platform.platform(),
            "cpu": hw,
            "runs": runs,
            "warmup": warmup,
            "opt_levels": opt_levels,
            "langs": langs,
            "stats": all_results,
            "raw_ms": all_raw,
        }
        with open(json_path, "w") as f:
            json.dump(full, f, indent=2)
        print(f"Full JSON -> {json_path}")

    # CSV export
    if emit_csv:
        csv_path = os.path.join(BENCH_DIR, "results.csv")
        with open(csv_path, "w", newline="") as f:
            writer = csvmod.writer(f)
            header = ["opt", "benchmark"]
            for l in langs:
                header.extend([f"{l}_median_ms", f"{l}_stddev_ms", f"{l}_min_ms", f"{l}_max_ms"])
            header.extend([f"ratio_{r}" for r in langs if r != "jade"])
            writer.writerow(header)
            for opt_key, level_data in all_results.items():
                for name, entry in sorted(level_data.items()):
                    row = [opt_key, name]
                    for l in langs:
                        row.append(entry.get(f"{l}_ms", ""))
                        row.append(entry.get(f"{l}_stddev_ms", ""))
                        row.append(entry.get(f"{l}_min_ms", ""))
                        row.append(entry.get(f"{l}_max_ms", ""))
                    for r in langs:
                        if r != "jade":
                            row.append(entry.get(f"ratio_{r}", ""))
                    writer.writerow(row)
        print(f"CSV -> {csv_path}")

    # Save to history
    if save_tag:
        history = load_history()
        record = {
            "tag": save_tag,
            "timestamp": datetime.now().isoformat(),
            "platform": platform.platform(),
            "cpu": hw,
            "python": platform.python_version(),
            "results": all_results,
        }
        history.append(record)
        save_history(history)
        print(f"History -> {HISTORY_FILE} (tag: {save_tag}, {len(history)} entries)")

    # Cross-opt summary
    if len(opt_levels) > 1 and "jade" in langs:
        print(f"\n{'='*80}")
        print("  SUMMARY: Jade/C ratio across optimization levels")
        print(f"{'='*80}")
        header = f"{'Benchmark':<18}"
        for opt in opt_levels:
            header += f" {'O'+str(opt):>10}"
        print(header)
        print("-" * 80)
        for name in benchmarks:
            row = f"{name:<18}"
            for opt in opt_levels:
                key = f"O{opt}"
                r = all_results.get(key, {}).get(name, {})
                rc = r.get("ratio_c")
                row += f" {f'{rc:.2f}x' if rc else '-':>10}"
            print(row)


def main():
    runs, opt_arg, timeout, save_tag, compare = 5, "3", 120, None, False
    warmup = 1
    bench_filter = None
    sort_by = "name"
    quiet = False
    detail = False
    emit_csv = False
    emit_json = False
    langs = ["jade", "c"]  # default: jade + c (fast)

    for arg in sys.argv[1:]:
        if arg.startswith("--runs="):
            runs = int(arg.split("=", 1)[1])
        elif arg.startswith("--opt="):
            opt_arg = arg.split("=", 1)[1]
        elif arg.startswith("--langs="):
            langs = [l.strip() for l in arg.split("=", 1)[1].split(",")]
        elif arg.startswith("--timeout="):
            timeout = int(arg.split("=", 1)[1])
        elif arg.startswith("--save="):
            save_tag = arg.split("=", 1)[1]
        elif arg.startswith("--warmup="):
            warmup = int(arg.split("=", 1)[1])
        elif arg.startswith("--bench="):
            bench_filter = arg.split("=", 1)[1]
        elif arg.startswith("--sort="):
            sort_by = arg.split("=", 1)[1]
        elif arg == "--no-python":
            langs = [l for l in langs if l != "python"]
        elif arg == "--compare":
            compare = True
        elif arg == "--quiet":
            quiet = True
        elif arg == "--detail":
            detail = True
        elif arg == "--csv":
            emit_csv = True
        elif arg == "--json":
            emit_json = True
        elif arg == "--all":
            langs = list(ALL_LANGS)
        elif arg == "--help":
            print(__doc__)
            sys.exit(0)
        else:
            print(f"Unknown flag: {arg}")
            print("Try --help")
            sys.exit(1)

    opt_levels = [0, 1, 2, 3] if opt_arg == "all" else [int(x) for x in opt_arg.split(",")]
    run_suite(opt_levels, runs, langs, timeout, save_tag, warmup, bench_filter,
              sort_by, quiet, detail, emit_csv, emit_json)

    if compare:
        compare_with_last()


def compare_with_last():
    history = load_history()
    if len(history) < 2:
        print("\nNot enough history entries to compare (need >= 2).")
        return

    prev, curr = history[-2], history[-1]
    print(f"\n{'='*90}")
    print(f"  COMPARE: {prev['tag']} -> {curr['tag']}")
    print(f"{'='*90}")
    print(f"{'Bench':<18} {'Prev Jade':>12} {'Curr Jade':>12} {'Delta':>10} {'Change':>10}")
    print("-" * 90)

    for opt_key in sorted(set(list(prev["results"].keys()) + list(curr["results"].keys()))):
        prev_level = prev["results"].get(opt_key, {})
        curr_level = curr["results"].get(opt_key, {})
        benchmarks = sorted(set(list(prev_level.keys()) + list(curr_level.keys())))
        for name in benchmarks:
            p_ms = prev_level.get(name, {}).get("jade_ms")
            c_ms = curr_level.get(name, {}).get("jade_ms")
            if p_ms and c_ms:
                delta = c_ms - p_ms
                pct = ((c_ms / p_ms) - 1) * 100
                sign = "+" if delta > 0 else ""
                marker = "SLOWER" if pct > 5 else ("FASTER" if pct < -5 else "~same")
                print(f"{name:<18} {p_ms:>10.1f}ms {c_ms:>10.1f}ms {sign}{delta:>8.1f}ms {marker:>10}")
            else:
                print(f"{name:<18} {'-':>12} {'-':>12} {'-':>10} {'-':>10}")
    print()


if __name__ == "__main__":
    main()
