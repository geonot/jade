#!/usr/bin/env bash
# Detached, resource-capped benchmark launcher.
#
# Runs run_benchmarks.py fully outside the IDE/terminal session so a runaway
# benchmark (OOM, infinite output, deadlock spin) cannot take down VS Code or
# the shell. Each child inherits a hard address-space cap; the whole run has a
# wall-clock backstop; output is line-buffered to a log file.
#
# Usage:
#   scripts/bench_bg.sh quick   # 1 run, 15s timeout, jinn only  (smoke)
#   scripts/bench_bg.sh perf    # 5 runs, 120s timeout, jinn+c   (timing)
#   scripts/bench_bg.sh <args>  # pass raw args to run_benchmarks.py
#
# Tunables (env):
#   BENCH_MEM_KB   per-process virtual-memory cap in KB   (default 4194304 = 4G)
#   BENCH_WALL     whole-run wall-clock cap, `timeout` fmt (default 30m)
#   BENCH_LOG      log file path                           (default /tmp/jinn-bench/run.log)

set -u
cd "$(dirname "$0")/.." || exit 1

MODE="${1:-quick}"; shift || true
LOGDIR="/tmp/jinn-bench"
mkdir -p "$LOGDIR"
BENCH_LOG="${BENCH_LOG:-$LOGDIR/run.log}"
BENCH_MEM_KB="${BENCH_MEM_KB:-4194304}"
BENCH_WALL="${BENCH_WALL:-30m}"

case "$MODE" in
  quick) ARGS=(--runs=1 --warmup=0 --timeout=15 --langs=jinn) ;;
  perf)  ARGS=(--runs=5 --warmup=1 --timeout=120 --langs=jinn,c) ;;
  *)     ARGS=("$MODE" "$@") ;;
esac

# Hard per-process address-space cap: a memory-hog child dies with a clean
# allocation failure instead of OOM-killing the machine (which crashed the IDE).
ulimit -v "$BENCH_MEM_KB" 2>/dev/null || echo "warn: could not set ulimit -v" >&2
# Never let a child dump a multi-GB core file.
ulimit -c 0 2>/dev/null || true

echo "=== bench_bg start $(date -Iseconds) mode=$MODE mem=${BENCH_MEM_KB}KB wall=$BENCH_WALL ===" > "$BENCH_LOG"
echo "args: ${ARGS[*]}" >> "$BENCH_LOG"

# -u: unbuffered python so the log's last line always names the in-flight bench.
# timeout: wall-clock backstop. PYTHONUNBUFFERED belt-and-suspenders.
PYTHONUNBUFFERED=1 timeout --signal=KILL "$BENCH_WALL" \
    python3 -u run_benchmarks.py "${ARGS[@]}" >> "$BENCH_LOG" 2>&1
rc=$?
echo "=== bench_bg done $(date -Iseconds) rc=$rc ===" >> "$BENCH_LOG"
