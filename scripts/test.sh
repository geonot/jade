#!/usr/bin/env bash
# scripts/test.sh — fast test runner.
#
# `cargo test` runs test *binaries* strictly one at a time, so a suite that
# leaves cores idle stalls the whole run. This builds every target once, then
# runs the binaries concurrently, largest-first, each with its own thread
# budget. Same binaries, same assertions, same exit semantics.
#
#   scripts/test.sh                 # everything
#   scripts/test.sh bulk            # only targets matching "bulk"
#   JOBS=4 scripts/test.sh          # cap concurrent binaries
set -uo pipefail

cd "$(dirname "$0")/.."

FILTER="${1:-}"
NPROC=$(nproc 2>/dev/null || echo 4)
JOBS="${JOBS:-$(( NPROC / 2 ))}"
INNER="${INNER:-2}"

cargo test --release --no-run 2>&1 | grep -E "^(error|warning: unused)" && exit 1
mapfile -t BINS < <(
  cargo test --release --no-run --message-format=json 2>/dev/null |
    python3 -c '
import json,sys
for line in sys.stdin:
    try: m=json.loads(line)
    except Exception: continue
    if m.get("profile",{}).get("test") and m.get("executable"):
        print(m["executable"])
'
)

[ "${#BINS[@]}" -eq 0 ] && { echo "no test binaries"; exit 1; }

if [ -n "$FILTER" ]; then
  KEEP=(); for b in "${BINS[@]}"; do [[ "$b" == *"$FILTER"* ]] && KEEP+=("$b"); done
  BINS=("${KEEP[@]}")
  [ "${#BINS[@]}" -eq 0 ] && { echo "no test binary matches '$FILTER'"; exit 1; }
fi

# Largest binaries first: longest-job-first packs the tail tightly.
mapfile -t BINS < <(ls -S "${BINS[@]}" 2>/dev/null || printf '%s\n' "${BINS[@]}")

OUT=$(mktemp -d); trap 'rm -rf "$OUT"' EXIT
START=$(date +%s%N)

# Every test forks jinnc (which maps a 157MB libLLVM) and then cc. The win
# here is purely filling cores that `cargo test` leaves idle while it walks
# test binaries one at a time -- not oversubscribing them: past ~nproc
# concurrent compiles, total CPU inflates and wall time gets worse.
# Keep the compiler-hash suffix in the log name: `src/lib.rs` and `src/main.rs`
# both produce a binary basenamed `jinnc`, and collapsing them to one log file
# silently drops a whole suite's result (and could mask its failure).
run_one() {
  local b="$1"
  local name; name=$(basename "$b")
  "$b" --test-threads="$INNER" >"$OUT/$name.log" 2>&1
  echo $? >"$OUT/$name.rc"
}

for b in "${BINS[@]}"; do
  while [ "$(jobs -rp | wc -l)" -ge "$JOBS" ]; do wait -n 2>/dev/null || true; done
  run_one "$b" &
done
wait

fail=0
for f in "$OUT"/*.rc; do
  name=$(basename "$f" .rc)
  rc=$(cat "$f")
  if [ "$rc" != "0" ]; then
    fail=1
    printf "\033[31m✗ %s\033[0m\n" "$(echo "$name" | sed "s/-[0-9a-f]*$//")"
    grep -E "^(test .* FAILED|failures:|---- )" -A2 "$OUT/$name.log" | head -40
    tail -5 "$OUT/$name.log"
  fi
done

TOTAL=$(grep -ho "[0-9]* passed" "$OUT"/*.log | awk '{s+=$1} END{print s}')
MS=$(( ($(date +%s%N) - START) / 1000000 ))
if [ "$fail" = 0 ]; then
  printf "\033[32m✓ %s tests passed in %s.%03ds (%d binaries, %d-way)\033[0m\n" \
    "$TOTAL" "$((MS/1000))" "$((MS%1000))" "${#BINS[@]}" "$JOBS"
else
  printf "\033[31mFAILED in %s.%03ds\033[0m\n" "$((MS/1000))" "$((MS%1000))"
fi
exit $fail
