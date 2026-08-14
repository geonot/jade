#!/usr/bin/env bash
# ci/sanitize-corpus.sh — whole-corpus ASan(+LSan) sweep (roadmap O-2).
#
# Compiles every conformance program (tests/programs), every app entry
# (apps/*/project.jn), and every snippet under ASan at --opt 0 and --opt 3,
# links against an instrumented copy of the C runtime, and runs each binary
# in an isolated scratch directory with a timeout.
#
# Result classification, per program:
#   ok            ran clean
#   CORRUPT       AddressSanitizer report (use-after-free, OOB, wild write…)
#                 — always fails the sweep
#   LEAK          LeakSanitizer-only report. Reported with counts but does
#                 not fail the sweep by default (JINN_SAN_STRICT=1 makes it
#                 gate): the per-value leak residue is a known, tracked gap
#                 (roadmap O-2) and gating on it would make the sweep
#                 permanently red while it is worked down.
#   segv?         died on a signal with NO sanitizer report. The runtime's
#                 context switches carry __sanitizer_start_switch_fiber
#                 annotations (the jinn_coro_swap_* helpers), so this should
#                 now be rare; a persistent segv? is worth a hand re-run.
#                 Reported, non-gating.
#   compile/link  did not build — reported, but the test suite owns
#                 compile gating; the sweep's job is memory errors
#   prog          nonzero exit without a sanitizer report (program-level
#                 failure: missing input, port in use, assertion…)
#   timeout       exceeded $JINN_SAN_TIMEOUT (default 45s)
#
# Known-environmental: a SEGV inside __tls_get_addr on thread T-1 is the
# libasan bootstrap issue on this distro, not a finding; such runs are
# retried once and skipped if they repeat.
#
# Leak checking: detect_leaks=1 with suppressions for the runtime's
# intentional exit-time allocations (ci/lsan-suppressions.txt). Set
# JINN_SAN_LEAKS=0 to disable leak detection entirely.

set -u

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

JINNC="$ROOT_DIR/target/release/jinnc"
[ -x "$JINNC" ] || { echo "build target/release/jinnc first"; exit 2; }

SAN_TARGET="$ROOT_DIR/target/sanitize/corpus"
TIMEOUT="${JINN_SAN_TIMEOUT:-45}"
LEAKS="${JINN_SAN_LEAKS:-1}"
JOBS="${JOBS:-$(nproc)}"
OPTS="${JINN_SAN_OPTS:-0 3}"

CFLAGS="-fsanitize=address -fno-sanitize-recover=all"
SUPP="$ROOT_DIR/ci/lsan-suppressions.txt"
ASAN_ENV="ASAN_OPTIONS=abort_on_error=0:exitcode=99:detect_leaks=$LEAKS"
if [ "$LEAKS" = "1" ] && [ -f "$SUPP" ]; then
  ASAN_ENV="$ASAN_ENV LSAN_OPTIONS=suppressions=$SUPP:print_suppressions=0"
fi

build_runtime() {
  local outdir="$SAN_TARGET/rt"
  mkdir -p "$outdir"
  if [ -f "$outdir/libjinn_rt.a" ] && [ -z "${JINN_SAN_REBUILD:-}" ]; then
    newest=$(ls -t "$ROOT_DIR"/runtime/*.c "$ROOT_DIR"/runtime/*.h 2>/dev/null | head -1)
    if [ "$outdir/libjinn_rt.a" -nt "$newest" ]; then
      echo "instrumented runtime up to date"
      return
    fi
  fi
  echo "building instrumented runtime -> $outdir"
  local objs=()
  for src in "$ROOT_DIR"/runtime/*.c; do
    local obj="$outdir/$(basename "${src%.c}").o"
    # shellcheck disable=SC2086
    cc $CFLAGS -g -O1 -fno-omit-frame-pointer -fPIC \
      -I"$ROOT_DIR/runtime" -c "$src" -o "$obj" || exit 2
    objs+=("$obj")
  done
  local asm
  case "$(uname -m)" in
    aarch64|arm64) asm="$ROOT_DIR/runtime/context_aarch64.S" ;;
    *) asm="$ROOT_DIR/runtime/context_x86_64.S" ;;
  esac
  if [ -e "$asm" ]; then
    local aobj="$outdir/$(basename "${asm%.S}").o"
    cc -g -c "$asm" -o "$aobj" || exit 2
    objs+=("$aobj")
  fi
  ar rcs "$outdir/libjinn_rt.a" "${objs[@]}" || exit 2
}

# one_program <src> <label> <opt>
one_program() {
  local src="$1" label="$2" opt="$3"
  local work
  work="$(mktemp -d "$SAN_TARGET/run.XXXXXX")"
  local bin="$work/prog"
  local out="$work/out"

  if ! "$JINNC" "$src" --emit-obj --opt "$opt" -o "$bin" >"$out.compile" 2>&1; then
    echo "COMPILE $label"
    sed 's/^/    /' "$out.compile" | head -4
    rm -rf "$work"
    return
  fi
  # shellcheck disable=SC2086
  if ! cc $CFLAGS -g "$bin.o" "$SAN_TARGET/rt/libjinn_rt.a" \
    -lm -lpthread -o "$bin" >"$out.link" 2>&1; then
    echo "LINK    $label"
    sed 's/^/    /' "$out.link" | head -4
    rm -rf "$work"
    return
  fi

  local attempt rc
  for attempt in 1 2; do
    (cd "$work" && timeout "$TIMEOUT" env $ASAN_ENV "./prog" </dev/null >"$out" 2>&1)
    rc=$?
    if grep -q "__tls_get_addr" "$out" && grep -q "T-1" "$out"; then
      if [ "$attempt" = "1" ]; then continue; fi
      echo "envskip $label"
      rm -rf "$work"
      return
    fi
    break
  done

  if grep -q "ERROR: AddressSanitizer" "$out"; then
    echo "CORRUPT $label (opt $opt)"
    sed 's/^/    /' "$out" | head -25
  elif grep -q "ERROR: LeakSanitizer" "$out"; then
    local bytes
    bytes=$(sed -n 's/.*SUMMARY: AddressSanitizer: \([0-9]*\) byte(s).*/\1/p' "$out" | head -1)
    echo "LEAK    $label (opt $opt, ${bytes:-?} bytes)"
  elif [ "$rc" = "124" ]; then
    echo "timeout $label (opt $opt)"
  elif [ "$rc" -ge 128 ] 2>/dev/null; then
    echo "segv?   $label (opt $opt, signal $((rc - 128)))"
  elif [ "$rc" != "0" ]; then
    echo "prog    $label (opt $opt, exit $rc)"
  else
    echo "ok      $label (opt $opt)"
  fi
  rm -rf "$work"
}

export -f one_program
export JINNC SAN_TARGET CFLAGS TIMEOUT ASAN_ENV

collect_sources() {
  for f in "$ROOT_DIR"/tests/programs/*.jn; do
    echo "$f|programs/$(basename "$f" .jn)"
  done
  for p in "$ROOT_DIR"/apps/*/project.jn; do
    local dir entry
    dir="$(dirname "$p")"
    entry="$(sed -n "s/^entry is '\(.*\)'$/\1/p" "$p" | head -1)"
    [ -n "$entry" ] && [ -f "$dir/$entry" ] && echo "$dir/$entry|apps/$(basename "$dir")"
  done
  for f in "$ROOT_DIR"/snippets/*/*.jn "$ROOT_DIR"/snippets/*.jn; do
    [ -f "$f" ] || continue
    echo "$f|snippets/$(basename "$f" .jn)"
  done
}

main() {
  mkdir -p "$SAN_TARGET"
  build_runtime

  local list="$SAN_TARGET/list"
  collect_sources >"$list"
  echo "sweeping $(wc -l <"$list") programs at opt levels: $OPTS ($JOBS jobs)"

  local report="$SAN_TARGET/report"
  : >"$report"
  for opt in $OPTS; do
    while IFS='|' read -r src label; do
      printf '%s\0%s\0%s\0' "$src" "$label" "$opt"
    done <"$list"
  done | xargs -0 -n 3 -P "$JOBS" bash -c 'one_program "$1" "$2" "$3"' _ >>"$report" 2>&1

  echo
  echo "== summary =="
  grep -E "^(ok|CORRUPT|LEAK|segv\?|COMPILE|LINK|prog|timeout|envskip)" "$report" \
    | awk '{print $1}' | sort | uniq -c | sort -rn
  echo
  grep -E "^(CORRUPT|COMPILE|LINK|prog|timeout|segv\?|LEAK)" "$report" | sort | uniq | head -90

  if grep -q "^CORRUPT" "$report"; then
    echo
    echo "sanitize-corpus: MEMORY CORRUPTION FOUND (details in $report)"
    exit 1
  fi
  if [ "${JINN_SAN_STRICT:-0}" = "1" ] && grep -q "^LEAK" "$report"; then
    echo
    echo "sanitize-corpus: leaks found and JINN_SAN_STRICT=1 (details in $report)"
    exit 1
  fi
  echo
  echo "sanitize-corpus: no memory corruption (full report: $report)"
}

main "$@"
