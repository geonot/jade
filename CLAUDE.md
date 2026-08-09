# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Jinn is a compiled, statically typed language (indentation syntax, word operators, ownership-based memory management, no GC). This repo is the compiler `jinnc` (Rust + LLVM 21 via inkwell), a C runtime, the standard library, and the toolchain around them.

## Build

```bash
cargo build --release        # incremental release build, ~2-4s warm
```

- Fresh builds (after `cargo clean`) need `LLVM_SYS_211_PREFIX=/usr/lib/llvm21` on Arch — `llvm-config` is not on PATH, and without it cargo fails with a cryptic llvm-sys/inkwell error. Warm builds don't need it.
- Toolchain is pinned (rust-toolchain.toml: 1.91.1), edition 2024. The release profile is intentionally `incremental = true`; CI overrides it for published artifacts.
- `build.rs` compiles `runtime/*.c` + a context-switch `.S` into `libjinn_rt.a` with warnings-as-errors, and probes pkg-config for optional OpenSSL / SQLite / PCRE2 support (recorded at compiler-build time; programs needing an absent one fail at link with a specific message). `JINN_RT_DEBUG=1` builds the runtime `-O0 -g`.

## Test

```bash
scripts/test.sh                       # full suite, parallel across test binaries (~40s)
scripts/test.sh memory_model          # only test binaries matching a substring
cargo test --release --test integration -- test_name   # one test in one suite
JINN_PROPTEST_CASES=64 scripts/test.sh # full property-test coverage (CI/preflight level)
```

`scripts/test.sh` runs the same binaries as `cargo test --release` with the same exit semantics — it only fixes scheduling (cargo runs test binaries strictly serially). `JOBS=` caps concurrent binaries, `INNER=` sets per-binary `--test-threads`. Don't raise concurrency past ~nproc; it makes wall time worse (measured).

Test-writing rules that matter here (rationale in docs/internals.md):

- Nearly every test forks `jinnc` (~90ms per compile-and-run, dominated by process + `ld` cost). For frontend-only assertions, use `--emit-hir` / `--emit-mir` output instead of compiling and running — it skips the linker.
- Corpus harnesses must fan out via `tests/support/parallel.rs::par_map` (order-preserving), never loop serially inside one `#[test]`. Resolve any sequential per-file state *before* the map.
- Always run compiled test binaries with `.current_dir(<tempdir>)` — a Jinn `store` resolves its `.store`/`.wal` files relative to cwd and will otherwise litter the repo root. For ad-hoc runs of store-using programs by hand, use the gitignored `.data/` directory.
- Property suites read their case count from `tests/support/cases.rs` (`JINN_PROPTEST_CASES`, small local default).

## Lint, format, gates

```bash
cargo fmt --check
cargo clippy --release -- -D warnings   # zero-warning policy
scripts/preflight.sh                    # full release gate: build, tests, fmt, clippy, smoke, benchmarks, WAL crash test
python3 run_benchmarks.py --bench=fib --runs=3   # benchmark harness (Jinn vs C/Rust/Python)
ci/sanitize.sh                          # ASan+UBSan then TSan sweeps of the C runtime (minutes)
cargo test --release --test std_stable_subset    # "std gate": frontend-checks AND link-checks every std/ module
```

## Compiling Jinn programs

```bash
./target/release/jinnc file.jn -o prog   # `jinn` is the same binary under a friendlier name
./target/release/jinnc file.jn --emit-hir    # also --emit-mir, --emit-llvm, --lib, --opt N
```

Subcommands: `run`, `test`, `check`, `fmt`, `init`, `bind` (C-header → Jinn extern generator), plus package commands. `jinn run` keeps a hash-keyed binary cache.

## Architecture

All compilation funnels through `src/driver/pipeline.rs::compile_and_link`:

lex (`src/lexer/`) → parse to AST (`src/parser/`, `src/ast.rs`) → module/package resolution (`src/driver/sources/`, `src/resolve.rs`) → **typecheck + HIR lowering in one pass** (`src/typer/`, ~20k LOC) → HIR validation (`src/hir_validate.rs`, hard-fails) → comptime const folding (`src/comptime/`) → MIR lowering (`src/mir/lower/`) → MIR verify (runs in release too; `JINN_MIR_VERIFY=0` opts out; failure = compiler bug) → MIR optimize (`src/mir/opt/`) → drop insertion/reuse analysis (`src/drops/mir_drops.rs`) → LLVM codegen (`src/codegen/`, ~21k LOC) → object emit + link via `cc` against `libjinn_rt.a`.

- **HIR** (`src/hir/`): fully typed, name-resolved tree keyed by `DefId`. **MIR** (`src/mir/`): SSA-form CFG per function. `--emit-hir`/`--emit-mir` print them.
- **Ownership/move checking lives inside the typer** (flow-sensitive; the single authority — a separate ownership pass was deleted). Spec: docs/memory-model.md. Escape analysis (`src/escape/`) runs per-function during HIR lowering, classifying bindings into tiers T1/T2/T3. Drop *placement* is MIR-level (`src/drops/`), producing Perceus-style reuse hints consumed by codegen.
- **Effect rows are inferred bottom-up over call-graph SCCs** (`src/typer/scc.rs`) as least fixpoints — twice, structurally identically: error effects (`src/typer/errset.rs`) and capabilities (`src/typer/caps.rs`; enum in `src/caps.rs`, stdlib capability table in `src/cap_sites.rs`). Annotations like `needs net.client` are compiler-checked upper bounds, never the source of truth — but the capability half is currently inert (roadmap C-1); its design lives in docs/design/compiler-prereqs.md.
- **`src/comptime/` is constant folding of inferred-pure functions** (HIR→HIR), not user-facing metaprogramming.
- **There is no incremental compilation.** `src/incr.rs` was deleted; docs/internals.md carries the post-mortem and sets the bar for any retry. The only compile-time reuse is `.jni` interface files (`src/interface.rs`). `src/cache.rs` is the *package* cache, despite the name.
- Three binaries: `jinnc` (src/main.rs), `jinn` (byte-identical, `include!`s main.rs), `jinnc-lsp` (src/lsp/, JSON-RPC over stdio).

Layered source trees — don't confuse them:

- `runtime/` — C runtime (~5K LOC) statically linked into every compiled program: coroutines/work-stealing scheduler, actors/channels/select, the persistent store engine (WAL, indexes, recovery), OS surface. Conventions (symbol naming, error-return shapes, shared helpers in util.c) are in runtime/README.md; codegen call sites for each `.c` file live in the matching `src/codegen/` file.
- `std/` — the Jinn standard library (`use math`; function calls are module-qualified, types are global). The alpha-stable subset and its policy: docs/stdlib.md.
- `libjn/` — aspirational C-stdlib-in-Jinn; stub bodies, not part of std or the runtime (docs/design/libjn.md).
- `apps/` (21 realistic multi-module programs), `benchmarks/` (+ `comparison/` C/Rust/Python equivalents), `snippets/` (400 numbered single-feature programs), `tests/programs/` — the executable language-surface corpora.

## Conventions

- **Source code carries no comments** — every comment was deliberately stripped from Rust, C, and Jinn sources ([139]; `scripts/strip_*_comments.py`). Don't reintroduce narration comments; scripts, configs, and docs keep theirs.
- Commits are numbered `[N] summary`, each with a matching detailed entry at the top of CHANGELOG.md explaining what was measured/decided and why.
- docs/jinn.md is the language tour and **every fenced `jinn` example in it is compiled by tests/doc_examples.rs** — a doc edit that breaks an example fails the suite. Docs describe the language as implemented, with gaps stated inline and given an id in docs/roadmap.md, the single list of open work. docs/README.md maps the set; specs sit next to the tour, developer docs in docs/internals.md, unimplemented designs under docs/design/. There are no review, audit, or remediation documents — don't add any; file the item in docs/roadmap.md instead.
- rustfmt: max_width 100. Clippy config in clippy.toml; lint allow/deny lives in src/lib.rs and src/main.rs.
