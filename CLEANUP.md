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
| C.1 | ⏳ in progress | HIR↔MIR seam. Tasks 1 (call-site map) ✅ and 7 (struct merge) ✅ done — `MirCodegen` collapsed into `Compiler`, the `self.comp.*` pattern eliminated entirely, 1565/0 tests, perf parity (collatz/fib/tight_loop 0.94×–1.01× J/C). Tasks 4/5/6 require **substantive re-implementation** of MIR entry points (compile_actor_loop, compile_spawn, compile_coroutine_create, gen_migration, compile_expr-for-globals, …) — deferred. Tasks 2/3 (inline trivials, promote pure LLVM helpers) remain as polish. |
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

## C.1 Eliminate the HIR↔MIR codegen seam

The largest structural smell. MIR is the production lowering target, but
~7K LOC of HIR-era codegen survives as a "helper library" with 15 cross-edges.
This makes the type signatures of `MirCodegen` and `Compiler` mutually entangled
and forces every reader to context-switch between two data models.

### Tasks
1. **Map every call from MIR codegen into HIR codegen helpers.** ✅ Done
   (May 2026). See [docs/internal/hir-helper-callsites.md](docs/internal/hir-helper-callsites.md):
   108 distinct `Compiler` methods called from `mir_codegen/` across 452
   sites; per-helper purpose, defining file, exact call sites, and
   disposition (Inline / Promote-LLVM / Promote-MIR / Stays) recorded.
   The audit's coarse "15 sites" referred to invocations into the six
   designated HIR-era modules — exact count there is **33 helpers / 124
   sites**. Plus **1,222** LLVM-context field accesses
   (`self.comp.{bld,ctx,module,cur_fn,fns,globals}`) that vanish under task
   7's struct merge.
2. **Inline trivial helpers.** `compile_str_literal`, `compile_time_monotonic`,
   `compile_get_args` — each used in exactly one MIR call site. Move the body
   into the caller.
3. **Promote shared utilities.** `vec_header_type`, `closure_type`,
   `ensure_malloc`, `entry_alloca`, `llvm_ty`, `type_store_size`,
   `string_concat`, `string_eq`, `string_len`, `string_data`, `tag_fn`,
   `call_result` are pure LLVM helpers with no HIR dependency. Move them out
   of `Compiler` into `src/codegen/llvm_util.rs` (or similar) and let both
   sides call them.
4. **Pull store codegen into MIR.** `compile_store_insert`, `compile_store_query`
   in [src/codegen/stores.rs](src/codegen/stores.rs) — port to
   [src/codegen/mir_codegen/store.rs](src/codegen/mir_codegen/store.rs) where
   most of it has already been re-implemented anyway. Delete the originals.
5. **Pull actor/coroutine spawn into MIR.** `compile_actor_loop`,
   `compile_spawn`, `compile_coroutine_create`, `compile_supervisor` are
   called from MIR. Move them to a new `src/codegen/mir_codegen/concurrency.rs`.
6. **Delete dead HIR codegen modules.** After (4) and (5), audit
   `src/codegen/{expr,stmt,stores,store_ops}.rs` for residual public API.
   Anything not called from MIR codegen is dead — delete it. Target: drop
   3,000–5,000 LOC.
7. **Collapse `Compiler` and `MirCodegen` into one struct.** ✅ Done (May 2026).
   `MirCodegen<'a,'ctx>` deleted; replaced with `pub type MirCodegen<'ctx> = Compiler<'ctx>`
   for source compatibility. The 14 MIR-state fields (`value_map`, `block_map`,
   `pending_phis`, `var_allocs`, `value_types`, `coro_bodies`,
   `select_data_bufs`, `self_allocs`, `self_alloc_types`, `block_exit_map`,
   `migration_fns`, `global_init_fn`, `vec_growth_floor_by_value`, `actor_defs`)
   moved onto `Compiler`. All `self.comp.*` accesses (1,222 sites) became
   `self.*`. The `mir_codegen` directory now contains additional
   `impl<'ctx> Compiler<'ctx>` blocks. Tests: 1565/0. Perf: parity preserved.

**Estimate:** L. Net delta achieved by Task 7: 0 LOC (mechanical merge). Tasks 4/5/6 require substantive re-implementation (see "Remaining work" below) before the −5,000 LOC target can be met.

**Remaining work (Tasks 4/5/6).** The HIR helper subgraph is deeply self-recursive (`compile_expr` → `compile_block` → `compile_stmt` → `compile_if`/`compile_assign`/`compile_asm` → recurse, with cross-edges through `lambda.rs`, `coroutines.rs`, `loops.rs`, `pattern_match.rs`). MIR codegen still drives into the entry helpers — `compile_expr` (global initializers), `compile_str_literal`, `compile_actor_loop`, `compile_spawn`, `compile_coroutine_create`, `compile_supervisor`, `gen_migration`, `eval_store_filter`, `load_store_record_as_jade`, plus various `string_*` / `vec_*` utilities. Removing them is **not mechanical** — each entry point must be reimplemented in MIR-native style:

- Re-emit global default-value initializers from MIR instead of recursing through HIR `compile_expr`.
- Lift actor/coroutine/supervisor codegen to a new `src/codegen/mir_codegen/concurrency.rs` that consumes MIR (today it walks `hir::ActorDef`/`hir::Stmt`).
- Migrate `gen_migration` and `eval_store_filter` similarly.

Once the entry points are MIR-native, `src/codegen/{actors,coroutines,lambda,stmt,expr,store_ops,store_filter,map,strings,string_ops,string_transform,vec,conversions}.rs` can be deleted (≈ 7,200 LOC). Until that work is scheduled, the seam is gone (Task 7) but the helper bulk remains.

**Acceptance (revised):** ✅ `grep -r "self\.comp" src/codegen/mir_codegen/` returns nothing. ⏳ The phrase "HIR codegen" still appears in module headers; remove during the deletion pass.

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
