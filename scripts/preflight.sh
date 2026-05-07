#!/usr/bin/env bash
# scripts/preflight.sh — Pre-release gate per ROADMAP §C.
# Block release on any non-green step.
set -euo pipefail

cd "$(dirname "$0")/.."

cyan() { printf "\033[36m▶ %s\033[0m\n" "$*"; }
red()  { printf "\033[31m✗ %s\033[0m\n" "$*"; }
green(){ printf "\033[32m✓ %s\033[0m\n" "$*"; }

cyan "1/7 cargo build --release"
cargo build --release 2>&1 | tail -3
green "build OK"

cyan "2/7 cargo test --release"
cargo test --release --quiet 2>&1 | tail -10
green "tests OK"

cyan "3/7 cargo fmt --check"
if cargo fmt --check; then green "fmt OK"; else red "fmt failed"; exit 1; fi

cyan "4/7 cargo clippy --release"
if cargo clippy --release --quiet -- -D warnings 2>&1 | tail -5; then
  green "clippy OK"
else
  red "clippy failed"; exit 1
fi

cyan "5/7 alpha smoke"
bash scripts/alpha_release_smoke.sh
green "smoke OK"

cyan "6/7 benchmarks (3 runs, quiet)"
python3 run_benchmarks.py --runs=3 --quiet 2>&1 | tail -20
green "bench OK"

cyan "7/7 wal crash recovery test"
cargo test --release --test wal_crash 2>&1 | tail -5
green "wal OK"

green "PREFLIGHT PASSED"
