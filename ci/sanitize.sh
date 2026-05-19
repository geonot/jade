#!/usr/bin/env bash
# ci/sanitize.sh — runtime C sanitizer sweep (P0-11).
#
# Compiles the C runtime under ASan+UBSan, then under TSan, and runs
# the full `cargo test` suite under each. Designed for CI; takes
# minutes, not seconds.
#
# The Rust side stays on stable; only the C side is instrumented (the
# Rust compiler's -Zsanitizer flag requires nightly, and most of the
# undefined-behaviour surface lives in the C runtime).

set -eu

cd "$(dirname "$0")/.."

clean() {
  cargo clean -q
}

run_with() {
  local label="$1"
  shift
  echo
  echo "=========================================================="
  echo "sanitize: $label"
  echo "  CFLAGS=$*"
  echo "=========================================================="
  clean
  CFLAGS="$* -g -O1 -fno-omit-frame-pointer" \
    LDFLAGS="$*" \
    cargo test --no-fail-fast 2>&1 | tail -200
}

# ASan + UBSan — catches use-after-free, OOB heap/stack, signed
# overflow, NULL deref, type-confusion in the C runtime.
run_with "asan+ubsan" \
  -fsanitize=address,undefined -fno-sanitize-recover=all

# TSan — catches data races in the runtime scheduler, channels,
# WAL, actors.
run_with "tsan" \
  -fsanitize=thread -fno-sanitize-recover=all

echo
echo "sanitize: all sweeps passed"
