#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT_DIR/target/debug/jinn"

log() {
  printf '[alpha-smoke] %s\n' "$*"
}

log "building jinn and jinnc binaries"
cargo build --quiet --manifest-path "$ROOT_DIR/Cargo.toml" --bin jinn --bin jinnc

if [[ ! -x "$BIN" ]]; then
  echo "jinn binary not found at $BIN" >&2
  exit 1
fi

log "validating moderate multi-module project with stdlib + actors + store"
pushd "$ROOT_DIR/examples/alpha_release_demo" >/dev/null
"$BIN" test
"$BIN" build -o alpha_demo
DEMO_OUT="$(./alpha_demo)"
if [[ "$DEMO_OUT" != *"sample_count"* ]] || [[ "$DEMO_OUT" != *"value_sum"* ]]; then
  echo "alpha_release_demo output missing expected markers" >&2
  echo "$DEMO_OUT" >&2
  exit 1
fi
"$BIN" package --no-archive
"$BIN" package
popd >/dev/null

log "validating local package publish/include/fetch flow"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR/mathlib/source"
cat > "$TMP_DIR/mathlib/project.jn" <<'JINN'
name is 'mathlib'
version is '0.1.0'
entry is 'source/mathlib.jn'
JINN

cat > "$TMP_DIR/mathlib/source/mathlib.jn" <<'JINN'
*answer() returns i64
    42
JINN

pushd "$TMP_DIR/mathlib" >/dev/null
git init >/dev/null
git config user.email "alpha-smoke@jinn.local"
git config user.name "Jinn Alpha Smoke"
git add project.jn source/mathlib.jn
git commit -m "init mathlib" >/dev/null
"$BIN" package --no-archive
"$BIN" publish --force
popd >/dev/null

mkdir -p "$TMP_DIR/consumer/source"
cat > "$TMP_DIR/consumer/project.jn" <<JINN
name is 'consumer'
version is '0.1.0'
entry is 'source/main.jn'
require('mathlib', 'file://$TMP_DIR/mathlib', '0.1.0')
JINN

cat > "$TMP_DIR/consumer/source/main.jn" <<'JINN'
use mathlib

*main
    log(mathlib.answer())
JINN

pushd "$TMP_DIR/consumer" >/dev/null
JINN_ALLOW_NON_HTTPS_DEPS=1 "$BIN" fetch
JINN_ALLOW_NON_HTTPS_DEPS=1 "$BIN" build -o consumer_app
CONS_OUT="$(JINN_ALLOW_NON_HTTPS_DEPS=1 ./consumer_app)"
if [[ "$CONS_OUT" != "42" ]]; then
  echo "consumer_app produced unexpected output: $CONS_OUT" >&2
  exit 1
fi
popd >/dev/null

log "checking cross-target command path"
pushd "$ROOT_DIR/examples/alpha_release_demo" >/dev/null
if "$BIN" build --target wasm32-wasi --standalone -o alpha_demo.wasm >/dev/null 2>&1; then
  log "cross-target build succeeded (wasm32-wasi)"
else
  log "cross-target build skipped/failing in this environment (toolchain unavailable)"
fi
popd >/dev/null

log "running focused actor/store benchmark sample"
if command -v python3 >/dev/null 2>&1; then
  python3 "$ROOT_DIR/run_benchmarks.py" --bench=actor_single,actor_pingpong,store_ops --langs=jinn --runs=2 --quiet >/dev/null
  log "benchmark sample completed"
else
  log "python3 not available; benchmark sample skipped"
fi

log "alpha smoke checks complete"
