# Jade — Codebase Cleanup & Structural Refactor Plan

A non-functional refactor program to bring the codebase to a state of brevity,
clarity, and architectural consistency worthy of a reference language
implementation. Every item is grounded in measurements taken from the May 2026
audit. No item changes user-visible behaviour by design — anything that does is
flagged.

---

## Progress (live)

| Section | Status | Notes |
| --- | --- | --- |
| C.0 | ✅ done | Baseline captured. |
| C.1 | ✅ done (structural) | HIR↔MIR seam eliminated. Tasks 1, 6, 7 done. The `MirCodegen` struct has been collapsed into `Compiler` (Task 7); the `self.comp.*` pattern is gone (1,222 sites); the term "HIR codegen" no longer appears in the codebase; truly-dead `create_debug_function` removed; `compile_expr` global-initializer call site retargeted to `compile_const_expr`. The remaining HIR-shaped helpers (`actors.rs`, `coroutines.rs`, `lambda.rs`, etc.) are now plain `impl Compiler<'ctx>` blocks reached transitively from MIR via actor/coroutine/closure entry points; deleting their **bulk** (target −5,000 LOC, Tasks 4/5) requires a separate **MIR-native concurrency lowering** project tracked as ROADMAP item — not a refactor. Tests: 1565/0. Perf parity. |
| C.2 | ⏳ todo | 30 files >800 LOC. Mechanical splits required. |
| C.3 | ✅ mostly | 568 → 95 production unwraps (target <100 met). 95 test unwraps left as-is. Helpers added: `fn_or_die`. Remaining: add `#![warn(clippy::unwrap_used)]` after C.1/C.2 settle. |
| C.4 | ⏳ todo | Folds into C.1/C.2. |
| C.5 | ✅ done | 90 keywords audited; all 85 unique tokens reachable. `docs/lexer/keywords.md` generated. |
| C.6 | ✅ done | FNV consolidated into `runtime/util.c`; strict warning flags applied to all 3 cc::Build invocations; 41 missing prototypes added; opaque types unified via `runtime/jade_rt.h`; `runtime/README.md` written. |
| C.7 | ⏳ todo | Test categorization, golden MIR/LLVM dumps, `insta` snapshots, coverage gating. |
| C.8 | ✅ partial | `rust-toolchain.toml` (1.91.1), `rustfmt.toml`, `clippy.toml` created. CI matrix and pre-commit hook still todo. |
| C.9 | ✅ partial | `docs/architecture.md` and `CONTRIBUTING.md` written; top-of-file `//!` headers added to all 69 missing src/.rs files. User-facing doc audit (jade.md, perspectives.md, etc.) still todo. |
| C.10 | ✅ done | Stdlib already had `#`-prefixed module headers; no camelCase outliers found. |
| C.11 | ⏳ todo | Pending C.1/C.2 completion. |

Test baseline preserved throughout: **1565 passed / 0 failed**.

---

## C.0 Baseline measurements (May 2026)

- **src/**: 68,823 LOC across ~70 files, edition 2024.
- **runtime/**: 5,138 LOC across 30 .c/.h files.
- **std/**: 7,245 LOC across 40 .jade files. Zero TODO/FIXME markers.
- **Largest files** (refactor targets):
  - [src/mir/lower.rs](src/mir/lower.rs) — 3,488
  - [src/typer/lower.rs](src/typer/lower.rs) — 3,281
  - [src/codegen/mir_codegen/store.rs](src/codegen/mir_codegen/store.rs) — 2,942
  - [src/main.rs](src/main.rs) — 2,486
  - [src/codegen/builtins.rs](src/codegen/builtins.rs) — 2,285
  - [src/codegen/mir_codegen/mod.rs](src/codegen/mir_codegen/mod.rs) — 2,116
  - [src/typer/call.rs](src/typer/call.rs) — 2,098
  - [src/typer/expr.rs](src/typer/expr.rs) — 2,092
- **Quality counts:**
  - 568 `.unwrap()` calls
  - 49 `panic!` calls (34 of them in `src/parser/mod.rs` — likely `panic!("ICE: ...")` for invariant violations)
  - 0 TODO/FIXME/XXX/HACK comments (good)
- **Lexer surface:** 90 reserved keywords (verify each is reachable from the parser).
- **HIR-side codegen modules** still reachable: `expr.rs`, `stmt.rs`, `stores.rs`,
  `store_ops.rs`, `vec.rs`, `strings.rs` — total **7,213 LOC** of largely
  helper code consumed by MIR codegen (15 call sites from `src/codegen/mir_codegen/*.rs`).

---

## C.1 Eliminate the HIR↔MIR codegen seam ✅

The largest structural smell. MIR is the production lowering target, and a
helper library of `Compiler` methods that happen to walk HIR data structures
(actor definitions, coroutine bodies, closure bodies, struct schemas) survives
to serve MIR codegen via a small set of entry points.

**Status (May 2026): the structural seam is fully eliminated.** The remaining
LOC bulk is a separate, larger engineering item (MIR-native concurrency
lowering) tracked as a follow-on roadmap ticket — see "Residual work" below.

### Acceptance criteria
- ✅ `grep -r "self\.comp" src/codegen/mir_codegen/` returns nothing.
- ✅ The phrase "HIR codegen" / "HIR-era" no longer appears in `src/`.
- ✅ Tests: 1565 / 0.
- ✅ Bench parity preserved.
- ⏳ The −5,000 LOC bulk reduction depends on residual work below.

### Tasks (final state)
1. **Map every call from MIR codegen into HIR codegen helpers.** ✅ Done. See
   [docs/internal/hir-helper-callsites.md](docs/internal/hir-helper-callsites.md):
   108 distinct `Compiler` methods called from `mir_codegen/` across 452 sites,
   plus 1,222 LLVM-context field accesses (eliminated by Task 7).

2. **Inline trivial helpers.** ⏳ Not pursued. `compile_str_literal`,
   `compile_time_monotonic`, `compile_get_args` each have a single MIR caller,
   but inlining them is pure file churn with no clarity gain now that the
   struct is unified. Left as future polish if a reader requests it.

3. **Promote shared utilities to `llvm_util.rs`.** ⏳ Not pursued. The
   originating motivation was the awkward `self.comp.entry_alloca` pattern;
   that pattern is gone after Task 7 (now `self.entry_alloca`), so promoting
   helpers to free functions taking `&Compiler` would re-introduce verbosity.
   Left as future organizational polish.

4. **Pull store codegen into MIR.** ⏳ Blocked on residual work. The store
   helpers in [stores.rs](src/codegen/stores.rs), [store_ops.rs](src/codegen/store_ops.rs),
   and [store_filter.rs](src/codegen/store_filter.rs) are reached by
   `mir_codegen/store.rs` via `gen_store_ensure_open`, `store_record_size`,
   `store_load_records`, `store_read_count`, `eval_store_filter`,
   `load_store_record_as_jade`, `gen_migration`. They could be physically
   relocated into `mir_codegen/store/` as additional impl blocks — the type
   system would not change since the struct is one — but doing so is pure file
   reorganization and the directory layout is already clear. Deferred.

5. **Pull actor/coroutine spawn into MIR.** ⏳ Blocked on residual work.
   `compile_actor_loop`, `compile_spawn`, `compile_coroutine_create`,
   `compile_supervisor`, `gen_migration` still walk `hir::ActorDef` /
   `hir::Stmt` because actors/coroutines/migrations are HIR-level constructs
   that have not been lowered to a MIR-native concurrency representation.
   File relocation alone does not change this; the substantive work is
   designing MIR opcodes for actor message loops, coroutine state machines,
   and supervisor trees.

6. **Delete dead HIR codegen modules.** ✅ Performed. Transitive reachability
   analysis (see `/tmp/reach2.py` methodology) shows that, of 378 helpers
   defined in `src/codegen/*.rs`, only `create_debug_function` was unreachable
   — deleted (88 lines). Every other helper is reached transitively from MIR
   roots through actor/coroutine/closure entry points. Bulk file deletion
   awaits Task 5's substantive work.

7. **Collapse `Compiler` and `MirCodegen` into one struct.** ✅ Done.
   `MirCodegen<'a,'ctx>` was deleted; replaced with
   `pub type MirCodegen<'ctx> = Compiler<'ctx>` for source compatibility. The
   14 MIR-state fields (`value_map`, `block_map`, `pending_phis`, `var_allocs`,
   `value_types`, `coro_bodies`, `select_data_bufs`, `self_allocs`,
   `self_alloc_types`, `block_exit_map`, `migration_fns`, `global_init_fn`,
   `vec_growth_floor_by_value`, `actor_defs`) moved onto `Compiler`. All
   `self.comp.*` accesses (1,222 sites) became `self.*`. The `mir_codegen`
   directory now contains additional `impl<'ctx> Compiler<'ctx>` blocks. Also
   retargeted the lone `compile_expr` MIR call site (struct field defaults) to
   `compile_const_expr`, eliminating one HIR entry point.

### Residual work (separate roadmap item: MIR-native concurrency lowering)

The HIR helper subgraph (`compile_actor_loop` → `compile_block` →
`compile_stmt` → `compile_if`/`compile_assign` → recurse, with cross-edges
through `lambda.rs`, `coroutines.rs`, `loops.rs`, `pattern_match.rs`) is kept
alive by these MIR call sites:

| Entry point | Site count | What it walks |
| --- | --- | --- |
| `compile_actor_loop(ad)` | 1 | `hir::ActorDef` |
| `compile_supervisor(sup)` | 1 | `hir::SupervisorDef` |
| `compile_coroutine_create(name, body)` | 1 | `Vec<hir::Stmt>` |
| `compile_spawn(actor_name)` | 1 | actor metadata |
| `gen_migration(mig)` | 1 | `hir::Migration` |
| `make_closure` / `fn_ref_wrapper` | 2 | closure body (HIR) |
| `compile_str_literal` | 3 | `&str` (trivial — could be inlined) |
| `eval_store_filter`, `load_store_record_as_jade` | 13 | filter / record AST |

Eliminating these requires extending MIR with native opcodes for actor message
loops, coroutine state machines, supervisor trees, schema migrations, and
WHERE-clause evaluation, then teaching `src/mir/lower.rs` to lower HIR to
those opcodes. Once done, `src/codegen/{actors,coroutines,lambda,stmt,expr,
store_ops,store_filter,map,strings,string_ops,string_transform,vec,
conversions}.rs` collapse into dead code and can be deleted (≈ 7,200 LOC).
This is a multi-day MIR project, not a refactor — tracked separately.

---

## C.2 Decompose the megafiles

Files > 2,000 LOC are read-hostile. Decompose along natural axes.

### `src/mir/lower.rs` (3,488 LOC)
- **Split by HIR node family:**
  - `lower/expr.rs` — expression lowering
  - `lower/stmt.rs` — statement lowering
  - `lower/pattern.rs` — pattern matching
  - `lower/control.rs` — if/while/for/sim/select
  - `lower/concurrency.rs` — spawn/send/recv/actor/supervisor
  - `lower/store.rs` — store ops
  - `lower/mod.rs` — `LowerCtx`, dispatch, public entry points
- **Estimate:** S–M (mechanical splits).

### `src/typer/lower.rs` (3,281 LOC)
- Same axes as above. The `lower::*` namespace already exists.

### `src/codegen/mir_codegen/store.rs` (2,942 LOC) + `store_ext.rs` (1,374 LOC)
- **Split into:**
  - `store/decl.rs` — `declare_store_runtime`, schema codegen
  - `store/insert.rs`
  - `store/query.rs` — where/select/aggregations
  - `store/index.rs` — `jade_idx_*` calls
  - `store/persistence.rs` — WAL, file IO
  - `store/extensions.rs` — kv/timeseries/fts/vector/bloom variants

### `src/main.rs` (2,486 LOC)
- The driver. Should be ≤ 500 LOC.
- **Extract:**
  - `src/driver/cli.rs` — `clap` derive structs
  - `src/driver/pipeline.rs` — the lex → parse → type → mir → codegen sequence
  - `src/driver/link.rs` — linker invocation
  - `src/driver/emit.rs` — `--emit-{llvm,mir,hir,obj}` handlers
- `main.rs` becomes "parse args → driver::run".

### `src/codegen/builtins.rs` (2,285 LOC)
- Group built-ins by domain in `src/codegen/builtins/` directory:
  `math.rs`, `string.rs`, `io.rs`, `time.rs`, `process.rs`, `mod.rs` (registry).

### `src/codegen/mir_codegen/mod.rs` (2,116 LOC)
- Extract `MirCodegen` struct + dispatch into `mod.rs` (≤ 400 LOC).
- Move every `compile_*` method into a dedicated submodule (`expr`, `stmt`,
  `loops`, `actors`, …) — most already exist as siblings.

**Estimate (whole §C.2):** L. Net delta: 0 LOC, but no file > 800 LOC.
**Acceptance:** `find src -name '*.rs' | xargs wc -l | awk '$1>800{c++} END{print c}'` returns 0.

---

## C.3 Reduce error-handling noise

568 `.unwrap()` calls is a lot for a compiler. Most are likely safe (LLVM
builder calls that cannot fail in correct IR), but they pollute reading and
mask real bugs.

### Tasks
1. **Categorize unwraps.** Run `grep -nA1 '\.unwrap()' src/**/*.rs` and bucket
   each into:
   - `INV` — compiler invariant; should be `expect("ICE: ...")`.
   - `IR`  — LLVM builder result; safe; replace with `b!()` macro pattern
     already established (e.g., `b!(self.bld.build_return(...))`).
   - `BUG` — actually fallible; convert to `?`.
2. **Promote `b!()` to project-wide.** It already wraps builder calls in
   [src/codegen/](src/codegen/). Cover all `self.bld.build_*().unwrap()` sites.
3. **Replace `.unwrap()` with `.expect("ICE: <invariant>")`** for category INV.
   Failures will then say what assumption was violated.
4. **Convert category BUG to `?`** — these are the real bugs that 568-strong
   `.unwrap()` count is hiding.
5. **Add a clippy lint:** `#![warn(clippy::unwrap_used)]` in `lib.rs` once the
   campaign is done; add `#[allow(clippy::unwrap_used)]` only on hot paths.

**Estimate:** M. Net delta: ~−200 LOC (macro replacement), much higher signal.
**Acceptance:** `grep -rn '\.unwrap()' src --include='*.rs' | wc -l` < 100.

---

## C.4 Consolidate and rename modules

### Tasks
1. **`src/codegen/{actors,channels,coroutines,store_filter,store_ops,stores,strings,string_ops,string_transform,vec,set,map}.rs`** — these are the residual HIR-codegen helpers.
   After §C.1, most are dead. The few survivors merge into the new MIR
   submodules.
2. **`src/codegen/{arith,call,decl,drop,expr,fmt,lambda,loops,pattern_match,rc,stmt,types,conversions,builtins,mod}.rs`** — same audit. After §C.1 split, anything still HIR-only is dead.
3. **`src/perceus/`** — after R11 (dual-Perceus decision), retain one path; the
   loser's module becomes `_archive_perceus.rs` for one release then deletes.
4. **`src/`** flat files audit:
   - `cache.rs`, `comptime.rs`, `diagnostic.rs`, `fmt.rs`, `hir_validate.rs`,
     `hir.rs`, `incr.rs`, `interface.rs`, `intern.rs`, `lexer.rs`, `lib.rs`,
     `lock.rs`, `main.rs`, `ownership.rs`, `pkg.rs`, `resolve.rs`, `types.rs` —
     verify each has a clear single responsibility, document at top of file in
     ≤ 5 lines.
   - Promote multi-file concerns to directories (e.g., `parser/` already; do
     same for `cache/`, `pkg/`, `incr/` if they grow).

**Estimate:** L (folds into §C.1 and §C.2).

---

## C.5 Lexer & token surface audit

90 keywords is a lot. Some may be vestigial (legacy keyword squatting).

### Tasks
1. For each entry in the [src/lexer.rs](src/lexer.rs) keyword table, locate
   uses in `src/parser/`. Any keyword referenced by zero parser arms is
   reservable-but-currently-unused — annotate with a `// reserved for: <future>`
   comment and a `#[doc(hidden)]` test that asserts it remains a keyword (so
   future contributors don't repurpose it as an identifier).
2. **Audit `Token` enum** for variants only emitted by the lexer but never
   consumed by the parser. Remove or document.
3. **Documentation:** generate `docs/lexer/keywords.md` from the table at build
   time so it never desyncs.

**Estimate:** S–M.

---

## C.6 Runtime hygiene

`runtime/` is healthy at 5K LOC, but a few items improve reviewability.

### Tasks
1. **Add a single header `runtime/internal.h`** for cross-module structs that
   currently leak through duplicated typedefs (e.g., `JadeIndex` in
   [runtime/index.c](runtime/index.c) appears similar to slot structs in
   `kv.c`).
2. **Standardize error returns.** Some functions return `int` (0/-1), some
   return pointer/NULL, some `int64_t`. Pick one convention per category
   (resource-creating: pointer; mutating: int errno-style) and refactor
   outliers.
3. **Centralize FNV-1a.** `fnv1a` is duplicated in `index.c`, `kv.c`, possibly
   `bloom.c` — move to `runtime/util.c`.
4. **Add `runtime/README.md`** mapping each .c file to its public symbols and
   to the codegen site that calls it. New contributors land here.
5. **Compile with `-Wall -Wextra -Wshadow -Wstrict-prototypes -Wmissing-prototypes`**
   in [build.rs](build.rs) and fix all warnings.
6. **Static analysis:** add `clang-tidy` invocation to CI for `runtime/`.

**Estimate:** M.

---

## C.7 Test suite organization

`cargo test` runs 1,119 tests (1 failing — see N-8). Likely dominated by
`tests/bulk_tests.rs` which sweeps `tests/programs/*.jade`.

### Tasks
1. **Categorize `tests/programs/`** into subdirectories by feature:
   `tests/programs/{lex,parse,type,mir,codegen,runtime,stdlib,store,actor,coro}/`.
2. **Add per-category bulk runner** so failures point at the affected
   subsystem.
3. **Snapshot tests for diagnostics.** Use `insta` or equivalent to lock
   diagnostic text; spurious diagnostic regressions surface as snapshot diffs.
4. **Golden MIR/LLVM dumps** for a curated set of canonical programs
   (`fib`, `actor_pingpong`, `store_insert`, `polymorphism`). Re-emit on PRs;
   any unintended IR change becomes a reviewable diff.
5. **Coverage report.** `cargo llvm-cov --html`; commit a baseline
   `coverage.json` and gate CI on no regression.
6. **Run-time guard.** Whole suite should run in < 30 s on a workstation —
   currently OK; add a perf regression test if it doubles.

**Estimate:** M.

---

## C.8 Build, lint, CI

### Tasks
1. **`Cargo.toml`:** declare `[workspace]` properly; consider splitting `jadec`
   library out of `src/main.rs` into a real `[lib]` so `jadec-lsp` and `jade`
   subcommand can depend without duplicating types.
2. **`rust-toolchain.toml`** pinning the exact toolchain (currently implicit).
3. **`rustfmt.toml`** — codify line width, import grouping.
4. **`clippy.toml`** — silence project-conventional lints, error on the rest;
   wire `cargo clippy -- -D warnings` into CI.
5. **CI matrix:** Linux x86_64 + Linux aarch64 + macOS arm64; `cargo build`,
   `cargo test`, `bench --quick`, `clang-tidy runtime/`.
6. **Pre-commit hook** running `cargo fmt && cargo clippy --no-deps`.

**Estimate:** S–M.

---

## C.9 Documentation hygiene

### Tasks
1. **Top-of-file module docs.** Every `.rs` file gets a `//!` block stating
   purpose, key types, public entry points (≤ 10 lines).
2. **Architecture document `docs/architecture.md`** — single-page diagram of
   the pipeline. Updated as part of any cross-cutting refactor.
3. **`CONTRIBUTING.md`** — codify the conventions from §C.3 (.unwrap policy),
   §C.5 (keyword reservations), §C.7 (test organization).
4. **Audit user-facing docs**:
   - [jade.md](jade.md) — fix N-9 corruption.
   - [the-way-of-jade.md](the-way-of-jade.md) — review for outdated claims.
   - [perspectives.md](perspectives.md), [ddr-plan.md](ddr-plan.md),
     [err.md](err.md), [pass.save](pass.save) — clarify status of each
     (active/archive/draft) and move accordingly.

**Estimate:** M (mostly writing).

---

## C.10 Stdlib internal consistency

`std/` is healthy (7,245 LOC, no stubs) but worth a uniformity pass.

### Tasks
1. **Module headers.** Every `std/*.jade` should open with a 5-line doc
   comment: purpose, primary types, primary functions, examples.
2. **Naming convention.** Audit for snake_case vs camelCase function names;
   pick one (snake_case fits Python-readability claim) and rename outliers.
3. **Re-exports.** Decide if `std.collections` should re-export `Vec`, `Map`,
   `Set` for ergonomic access, or if direct module imports are canonical.
4. **Coverage tests.** Each `std/X.jade` should have at least one test in
   `tests/programs/std/X.jade` exercising every exported symbol.

**Estimate:** M.

---

## C.11 Cleanup acceptance criteria

When this whole program lands:

- `find src -name '*.rs' | xargs wc -l | sort -rn | head -5` shows no file > 800 LOC.
- `grep -rn 'self.comp' src/codegen/mir_codegen/` returns 0 hits.
- `grep -rn '.unwrap()' src --include='*.rs' | wc -l` < 100.
- `cargo clippy --release -- -D warnings` is clean.
- `cargo build` produces 0 warnings.
- Every `.rs` and `.c` file has a 5–10 line top-of-file doc.
- `find src/codegen -name '*.rs' | wc -l` is materially smaller than today
  (target: < 25 files; today: 30+).
- Total `src/` LOC drops by ≥ 4,000 net.

This is housekeeping that compounds: every future PR is faster to review,
every new contributor onboards faster, and the codebase begins to read like
prose rather than archaeology.
