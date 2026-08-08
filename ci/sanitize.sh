#!/usr/bin/env bash
# ci/sanitize.sh — runtime C sanitizer sweep.
#
# Builds an instrumented copy of the C runtime into its own target
# directory, then compiles and runs a targeted set of Jinn programs
# against it under ASan+UBSan and then TSan.
#
# The previous version ran `cargo clean` and then the whole `cargo test`
# suite twice. That destroyed the shared release build every developer
# and every other gate depends on, and took about an hour, so in practice
# nobody ran it — which is why the memory findings in the 2026-08-06
# alpha review rested on crash signatures rather than an instrumented
# sweep. Keeping this cheap enough to run is the point.
#
# Only the C side is instrumented: the Rust compiler's -Zsanitizer needs
# nightly, and the undefined-behaviour surface lives in the C runtime.

set -eu

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

SAN_TARGET="$ROOT_DIR/target/sanitize"
PROGRAMS_DIR="${JINN_SAN_PROGRAMS:-$ROOT_DIR/tests/programs}"
# Programs chosen to exercise the subsystems with the most C: the
# scheduler, channels, actors, scopes, and the store's WAL and indexes.
DEFAULT_PROGRAMS="
actors_basic
actors_multi
actors_stress
dispatch_multi
dispatch_fib
store_basic
store_index
store_delete
store_persist
"

fail=0

build_runtime() {
  local label="$1"
  local flags="$2"
  local outdir="$SAN_TARGET/$label"
  mkdir -p "$outdir"
  echo "  building instrumented runtime -> $outdir"
  local objs=()
  for src in "$ROOT_DIR"/runtime/*.c; do
    local obj="$outdir/$(basename "${src%.c}").o"
    # shellcheck disable=SC2086
    cc $flags -g -O1 -fno-omit-frame-pointer -fPIC \
      -I"$ROOT_DIR/runtime" -c "$src" -o "$obj"
    objs+=("$obj")
  done
  # One context-switch implementation per architecture, matching build.rs.
  local asm
  case "$(uname -m)" in
    aarch64|arm64) asm="$ROOT_DIR/runtime/context_aarch64.S" ;;
    *) asm="$ROOT_DIR/runtime/context_x86_64.S" ;;
  esac
  if [ -e "$asm" ]; then
    local aobj="$outdir/$(basename "${asm%.S}").o"
    cc -g -c "$asm" -o "$aobj"
    objs+=("$aobj")
  fi
  ar rcs "$outdir/libjinn_rt.a" "${objs[@]}"
}

sweep() {
  local label="$1"
  local flags="$2"
  local rtopts="$3"
  local outdir="$SAN_TARGET/$label"

  echo
  echo "=========================================================="
  echo "sanitize: $label"
  echo "  CFLAGS=$flags"
  echo "=========================================================="

  build_runtime "$label" "$flags"

  local work
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' RETURN

  local programs="${JINN_SAN_PROGRAM_LIST:-$DEFAULT_PROGRAMS}"
  for name in $programs; do
    local src="$PROGRAMS_DIR/$name.jn"
    if [ ! -f "$src" ]; then
      echo "  skip $name (no such program)"
      continue
    fi
    # Emit an object for the Jinn program, then link it against the
    # instrumented archive ourselves — the driver would otherwise link
    # the uninstrumented one from the normal build.
    if ! "$ROOT_DIR/target/release/jinnc" "$src" --emit-obj \
      -o "$work/$name" >"$work/$name.compile" 2>&1; then
      echo "  FAIL $name (compile)"
      sed 's/^/    /' "$work/$name.compile"
      fail=1
      continue
    fi
    # shellcheck disable=SC2086
    if ! cc $flags -g "$work/$name.o" "$outdir/libjinn_rt.a" \
      -lm -lpthread -o "$work/$name.bin" >"$work/$name.link" 2>&1; then
      echo "  FAIL $name (link)"
      sed 's/^/    /' "$work/$name.link"
      fail=1
      continue
    fi
    if (cd "$work" && env $rtopts "./$name.bin" >"$name.out" 2>&1); then
      echo "  ok   $name"
    else
      echo "  FAIL $name (run)"
      sed 's/^/    /' "$work/$name.out"
      fail=1
    fi
  done
}

# ASan + UBSan — use-after-free, out-of-bounds heap/stack, signed
# overflow, NULL deref.
#
# detect_leaks is off: the runtime intentionally leaves some allocations
# to process exit, and leak reports would drown the real signal.
sweep "asan-ubsan" \
  "-fsanitize=address,undefined -fno-sanitize-recover=all" \
  "ASAN_OPTIONS=detect_leaks=0:abort_on_error=1 UBSAN_OPTIONS=print_stacktrace=1"

# TSan — data races in the scheduler, channels, WAL, actors.
#
# Caveat worth knowing before you trust a clean run: TSan does not
# understand the coroutine context switch in runtime/sched.c. Without
# __tsan_switch_to_fiber annotations it reports races across what are
# really the same logical thread of control. Treat findings here as
# leads, and prefer the ASan sweep as the gating one until those
# annotations exist.
sweep "tsan" \
  "-fsanitize=thread -fno-sanitize-recover=all" \
  "TSAN_OPTIONS=halt_on_error=1"

echo
if [ "$fail" -ne 0 ]; then
  echo "sanitize: FAILURES above"
  exit 1
fi
echo "sanitize: all sweeps passed"
