# Compiler internals

How `jinnc` is built, how it is laid out, how it is tested, and the decisions
that are easy to relearn the hard way. Language semantics live in
[`jinn.md`](jinn.md), [`memory-model.md`](memory-model.md),
[`concurrency.md`](concurrency.md), and [`error-effects.md`](error-effects.md).

## Building

```sh
cargo build --release        # incremental release build, ~2-4s warm
```

- A **fresh** build (after `cargo clean`) needs `LLVM_SYS_211_PREFIX=/usr/lib/llvm21`
  on Arch — `llvm-config` is not on `PATH`, and without it cargo fails with a
  cryptic llvm-sys/inkwell error. Warm builds do not need it.
- The toolchain is pinned (`rust-toolchain.toml`: 1.91.1), edition 2024. The
  release profile sets `incremental = true` deliberately; CI overrides it for
  published artifacts.
- `build.rs` compiles `runtime/*.c` plus a context-switch `.S` into
  `libjinn_rt.a` with warnings as errors, and probes pkg-config for optional
  OpenSSL, SQLite, and PCRE2 support. Those results are recorded at
  *compiler*-build time; a program needing an absent one fails at link with a
  specific message. `JINN_RT_DEBUG=1` builds the runtime `-O0 -g`.

Three binaries: `jinnc` (`src/main.rs`), `jinn` (byte-identical — it
`include!`s `main.rs`), and `jinnc-lsp` (`src/lsp/`).

## Pipeline

All compilation funnels through `src/driver/pipeline.rs::compile_and_link`:

```
lex (src/lexer/)
  → parse to AST (src/parser/, src/ast.rs)
  → module/package resolution (src/driver/sources/, src/resolve.rs)
  → typecheck + HIR lowering, one pass (src/typer/, ~20k LOC)
  → HIR validation (src/hir_validate.rs — hard fail)
  → comptime const folding (src/comptime/)
  → MIR lowering (src/mir/lower/)
  → MIR verify (JINN_MIR_VERIFY=0 to disable; failure = compiler bug)
  → MIR optimize (src/mir/opt/)
  → drop insertion + reuse analysis (src/drops/mir_drops.rs)
  → LLVM codegen (src/codegen/, ~21k LOC)
  → object emit + link via `cc` against libjinn_rt.a
```

**HIR** (`src/hir/`) is a fully typed, name-resolved tree keyed by `DefId`.
**MIR** (`src/mir/`) is an SSA-form CFG per function. `--emit-hir` and
`--emit-mir` print them.

Load-bearing design facts:

- **Ownership and move checking live inside the typer**, flow-sensitively, and
  are the single authority — a separate ownership pass existed and was deleted.
  Escape analysis (`src/escape/`) runs per function during HIR lowering,
  classifying bindings into tiers T1/T2/T3. Drop *placement* is MIR-level
  (`src/drops/`), producing Perceus-style reuse hints that codegen consumes.
- **Effect rows are inferred bottom-up over call-graph SCCs**
  (`src/typer/scc.rs`) as least fixpoints — twice, structurally identically:
  error effects (`src/typer/errset.rs`) and capabilities (`src/typer/caps.rs`,
  with the enum in `src/caps.rs` and the stdlib table in `src/cap_sites.rs`).
  Annotations like `needs net.client` are compiler-checked upper bounds, never
  the source of truth. The capability half of this is currently inert — see
  `C-1` in [`roadmap.md`](roadmap.md#effects-and-capabilities).
- **`src/comptime/` is constant folding of inferred-pure functions** (HIR→HIR),
  not user-facing metaprogramming.
- **MIR verify runs in release**, not just debug. It checks phi and edge types,
  which is what catches join points whose incoming values disagree.
- **`src/store_decorators.rs` is the single source of truth** for every store
  and field decorator: argument arity and type, mutual exclusions, numeric-only
  constraints. The parser validates names, argument shapes, and store-level
  exclusions; the typer validates field-type constraints once field types are
  known; the reference list in the docs is generated from the same table.

## Source trees — do not confuse them

| Tree | What it is |
| --- | --- |
| `src/` | The compiler, in Rust. |
| `runtime/` | The C runtime (~5k LOC) statically linked into every compiled program: coroutines and the work-stealing scheduler, actors, channels, `select`, structured-concurrency scopes, the persistent store engine (WAL, indexes, recovery, migrations), and the OS surface. Conventions — symbol naming, error-return shapes, shared helpers in `util.c` — are in [`../runtime/README.md`](../runtime/README.md); the codegen call sites for each `.c` file live in the matching `src/codegen/` file. |
| `std/` | The Jinn standard library. See [`stdlib.md`](stdlib.md). |
| `libjn/` | An aspirational C-stdlib-in-Jinn. Stub bodies; not part of `std` or the runtime. See [`design/libjn.md`](design/libjn.md). |
| `apps/` | 21 realistic multi-module programs. |
| `benchmarks/` | Benchmarks, plus `comparison/` C, Rust, and Python equivalents. |
| `snippets/` | 400 numbered single-feature programs. |
| `tests/programs/` | Language-surface programs driven by the corpus harnesses. |

## Incremental compilation

**There is none.** `src/incr.rs` was deleted. `src/cache.rs` is the *package*
cache despite the name; the only compile-time reuse is `.jni` interface files
(`src/interface.rs`), and reading those is off by default (`X-5`).

The deleted design is recorded here so the next attempt does not rebuild the
same broken shape. Its call sites only ever logged a dirty count,
`ArtifactCache::store` was never called outside its own unit test — so `lookup`
could never hit — and it could not have worked if wired up:

1. **Keys mixed in non-dependencies.** `function_cache_key` hashed every
   function's signature into every key, so any signature change anywhere dirtied
   everything. A usable key covers exactly the function's actual dependency set:
   the signatures it calls, the types it mentions, the constants it folds,
   discovered from the HIR rather than globally.
2. **Span-sensitive hashing.** `hash_stmt` hashed the `Debug` formatting of the
   statement, spans included, so inserting a blank line dirtied every function
   below it. Hash structure, never positions.
3. **No artifact store.** Nothing persisted object code keyed by those hashes,
   and the LLVM pipeline compiles the whole module monolithically. Per-function
   reuse needs per-function (or per-SCC) codegen units and a link step that
   composes cached objects.

**Bar to clear before building it at all:** compile times are ~64 ms for a small
file and ~1 s for an app. An incremental scheme must beat cold compiles on real
edits *including* its own hashing and IO overhead, and must be byte-for-byte
identical to a cold build (differential-tested), or it ships as a footgun.

## Testing

```sh
scripts/test.sh                       # full suite, parallel across binaries (~40s)
scripts/test.sh memory_model          # only binaries matching a substring
JOBS=2 scripts/test.sh                # cap concurrent binaries
INNER=4 scripts/test.sh               # per-binary --test-threads
cargo test --release                  # equivalent, slower; still fully supported
cargo test --release --test integration -- test_name   # one test in one suite
JINN_PROPTEST_CASES=64 scripts/test.sh # full property coverage (CI level)
```

`scripts/test.sh` builds the same binaries `cargo test` does and runs them with
the same assertions and exit semantics. The only difference is scheduling:
`cargo test` runs test *binaries* strictly one at a time, so every suite that
cannot saturate the box leaves cores idle. The script runs several concurrently
and prints the failing suites' logs.

### Why the suite costs what it costs

Nearly every test forks `jinnc` to compile and link a small program, so the
suite is dominated by process cost, not assertion logic. Measured on a
4-core/8-thread laptop for a two-line program:

| Phase | Cost |
| --- | --- |
| `jinnc` startup before any work | ~16–22 ms (mostly mapping the 157 MB `libLLVM.so`) |
| lex → parse → HIR → MIR → LLVM → object | ~8 ms |
| `cc` link (spawns `ld`) | ~35–50 ms |
| **total per compile-and-run test** | **~90 ms** |

Two consequences:

- **Concurrency is capped by cores, not by cleverness.** Past roughly `nproc`
  in-flight compiles, total CPU inflates and wall time gets *worse*.
- **The fastest test is the one that does not fork.** Prefer asserting on
  `--emit-hir`/`--emit-mir` output when a test is really about the frontend — it
  skips the linker, the largest single cost. Remember what that does and does
  not certify ([`tooling.md`](tooling.md#what---emit-hir-certifies)).

Switching linkers was evaluated and rejected: `mold` and `lld` are not present
on the reference machine, and `ld.gold` measured *slower* than the default `ld`.

### Rules for writing tests here

- **Corpus harnesses must fan out** via `tests/support/parallel.rs::par_map`,
  never loop serially inside one `#[test]`. A single test that loops over a
  corpus pins one core while the rest of the box idles. `par_map` preserves
  input order, so failure output stays deterministic. Resolve any sequential
  per-item state *before* the map, never inside it.
- **Property suites that fork the compiler must be sharded** across several
  `#[test]` functions so libtest can schedule them in parallel — see
  `ownership_fuzz.rs`, where 200 cases run as 8 shards of 25. Case counts come
  from `tests/support/cases.rs` (`JINN_PROPTEST_CASES`, with a small local
  default). Shrunk counterexamples are recorded in `.proptest-regressions` and
  replay at any case count, so a failure found by a soak run stays pinned.
- **Always run compiled test binaries with `.current_dir(<tempdir>)`.** A Jinn
  `store` resolves its `.store` and `.wal` files relative to the working
  directory and will otherwise litter the repo root. For ad-hoc runs by hand,
  use the gitignored `.data/` directory.

### Gates

```sh
cargo fmt --check
cargo clippy --release -- -D warnings   # zero-warning policy
scripts/preflight.sh                    # full release gate: build, tests, fmt,
                                        # clippy, smoke, benchmarks, WAL crash test
ci/sanitize.sh                          # ASan+UBSan then TSan sweeps (~2 min)
python3 run_benchmarks.py --bench=fib --runs=3
```

`ci/sanitize.sh` builds an instrumented runtime into its own target directory
and sweeps nine targeted programs, rather than doing a `cargo clean` plus two
full suite runs. It found a real data race in actor-drain code the day it was
rewritten. Its TSan half is not yet fully meaningful — coroutines migrate
between OS threads, so it needs `__tsan_switch_to_fiber` annotations (`N-5`).

**A gate certifies exactly what it runs.** A frontend-only gate certifies that
code type-checks; it says nothing about codegen, linking, or behaviour. When a
gate's name implies a broader claim than its body checks, fix the body or rename
the gate — that drift is how `apps/` came to be covered by no test at all while
two of its 21 projects had stopped compiling.

## Conventions

- **Source code carries no comments.** Every comment was deliberately stripped
  from the Rust, C, and Jinn sources (`scripts/strip_*_comments.py`). Do not
  reintroduce narration comments. Scripts, configs, and docs keep theirs.
- Commits are numbered `[N] summary`, each with a matching detailed entry at the
  top of `CHANGELOG.md` explaining what was measured or decided, and why.
- rustfmt `max_width` is 100. Clippy config is in `clippy.toml`; lint allow/deny
  lives in `src/lib.rs` and `src/main.rs`.
- Every fenced example in [`jinn.md`](jinn.md) is compiled by
  `tests/doc_examples.rs`. A doc edit that breaks an example fails the suite.
  Docs describe the language as implemented, with gaps stated inline and tracked
  in [`roadmap.md`](roadmap.md).
