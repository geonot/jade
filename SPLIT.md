# SPLIT.md — C.2 Megafile Decomposition Plan

Operational, file-by-file plan to action **CLEANUP.md §C.2** ("Decompose the
megafiles") for every `src/` Rust file currently > 800 LOC, **excluding files
that are owned by the in-flight C.1 work** (HIR↔MIR codegen seam).

Files excluded from this plan because C.1 will rewrite, delete, or merge them:

- `src/codegen/mod.rs` (`Compiler` is being merged into `Codegen` — C.1 task #7)
- `src/codegen/builtins.rs` (HIR-era; C.1 task #6 may delete much of it)
- `src/codegen/vec.rs`, `src/codegen/expr.rs`, `src/codegen/stores.rs`,
  `src/codegen/loops.rs`, `src/codegen/store_ops.rs` (HIR-era helpers — C.1 §6)
- `src/codegen/mir_codegen/{mod,helpers,intrinsics,store,store_ext}.rs`
  (the new home for everything that survives C.1; their final shape is decided
  by C.1 tasks #4, #5, #7)

The remaining 18 files in scope (LOC at audit time, May 2026):

| # | File | LOC | Group |
| --- | --- | ---: | --- |
| 1 | `src/mir/lower.rs` | 3,488 | MIR |
| 2 | `src/typer/lower.rs` | 3,297 | Typer |
| 3 | `src/main.rs` | 2,497 | Driver |
| 4 | `src/typer/expr.rs` | 2,473 | Typer |
| 5 | `src/typer/call.rs` | 2,100 | Typer |
| 6 | `src/typer/mod.rs` | 1,860 | Typer (mostly tests) |
| 7 | `src/parser/expr.rs` | 1,826 | Parser |
| 8 | `src/mir/opt.rs` | 1,535 | MIR |
| 9 | `src/hir.rs` | 1,373 | IR types |
| 10 | `src/parser/stmt.rs` | 1,368 | Parser |
| 11 | `src/lexer.rs` | 1,295 | Lexer |
| 12 | `src/parser/decl.rs` | 1,226 | Parser |
| 13 | `src/typer/unify.rs` | 1,206 | Typer |
| 14 | `src/typer/stmt.rs` | 1,120 | Typer |
| 15 | `src/ownership.rs` | 1,048 | Analysis |
| 16 | `src/parser/mod.rs` | 1,045 | Parser |
| 17 | `src/comptime.rs` | 913 | IR pass |
| 18 | `src/perceus/uses.rs` | 858 | Analysis |

---

## 0. Big-picture strategy

### 0.1 Splitting axes (in priority order)

1. **AST/HIR/MIR node family** (Expr / Stmt / Pat / Decl / Type) — the largest
   files are giant `match` ladders over one of these enums; splitting by family
   produces files of comparable size, all sharing one `impl` block.
2. **Functional sub-domain** (e.g. "store", "actor", "loop", "filter") — used
   when a node family is itself thousands of lines.
3. **Public-API surface vs. internal helpers** — extract pure helper fns to a
   sibling `helpers.rs` so the main file reads as a thin dispatch layer.
4. **Tests** — every `#[cfg(test)] mod tests` containing > ~150 LOC moves to a
   `tests.rs` sibling. Many of the megafiles are 30–60% test code.

### 0.2 Mechanical pattern (use everywhere)

Every split follows this exact recipe to guarantee no behaviour change:

1. **Promote the file to a directory.** `git mv X.rs X/mod.rs` (or, for module
   roots that are already `mod.rs`, leave in place and add siblings).
2. **Add a top-of-file `//!` doc** on each new file naming purpose, primary
   types, and dispatch entry points (≤ 10 lines, per CLEANUP §C.9).
3. **Move `impl T { fn ... }` blocks unchanged** into siblings; Rust permits
   multiple `impl` blocks for the same type across files in the same crate, so
   `impl Typer { ... }` can be re-opened in `typer/expr.rs`, `typer/stmt.rs`, …
4. **Visibility:** functions called from sibling files become `pub(super)`
   (preferred) or `pub(crate)`. **Never** `pub`. This is a refactor invariant.
5. **Imports:** copy the original `use` block into the new file then run
   `cargo fix --allow-dirty --bin jade --tests` to prune unused imports.
6. **Compile** (`cargo check`) after every move, **before** moving the next
   block. Splits done in tiny commits avoid hour-long bisection later.
7. **Run the affected test slice** (`cargo test -p jade <module>`) and the full
   suite at the end of each megafile. The baseline is **1565 passed / 0
   failed**; any deviation aborts the split.
8. **Update `wc -l`** in the table at the end of each megafile in this file as
   you go.

### 0.3 Cross-cutting prerequisites (do once, up front)

| Task | Owner file | Why |
| --- | --- | --- |
| Add `pub(crate) use` re-exports if any module is imported by name from outside its parent | `lib.rs`, parent `mod.rs` | Avoids breaking external `use jade::typer::lower::X` paths used by `tests/`. |
| `grep -rn 'use crate::typer::lower::' src tests` etc. for each megafile **before** splitting | n/a | Records every external import so the new layout preserves them via re-exports. |
| Snapshot the test count + LOC | n/a | Acceptance gate: `cargo test --release` must report `1565 passed / 0 failed` at the end of every megafile split. |
| Snapshot the public symbol surface: `cargo public-api --simplified > /tmp/before.txt` | n/a | At the end of C.2 the diff must be empty (modulo doc-only changes). |

### 0.4 Suggested execution order (cheapest → riskiest)

This ordering front-loads the mechanical wins and defers the work that touches
the most other modules.

1. **Phase A — single-file, no cross-module impact** (low risk, parallelisable):
   - §11 `src/lexer.rs` (split tests off — drops to ~700 LOC alone)
   - §6 `src/typer/mod.rs` (extract tests — drops to ~440 LOC alone)
   - §16 `src/parser/mod.rs` (extract tests — drops to ~340 LOC alone)
   - §17 `src/comptime.rs`
   - §18 `src/perceus/uses.rs`
   - §15 `src/ownership.rs`
2. **Phase B — directory promotions** (medium risk):
   - §9 `src/hir.rs`
   - §8 `src/mir/opt.rs`
   - §7 `src/parser/expr.rs`, §10 `src/parser/stmt.rs`, §12 `src/parser/decl.rs`
   - §13 `src/typer/unify.rs`, §14 `src/typer/stmt.rs`
3. **Phase C — large `impl` ladders** (highest risk; do last, one at a time):
   - §3 `src/main.rs` → `src/driver/`
   - §4 `src/typer/expr.rs`
   - §5 `src/typer/call.rs`
   - §2 `src/typer/lower.rs`
   - §1 `src/mir/lower.rs`

> §1 (`mir/lower.rs`) and §2 (`typer/lower.rs`) bracket the change C.1 may make
> to MIR's interface. **Do them after the C.1 lowering boundary stabilises**;
> otherwise the file will be re-split twice.

### 0.5 Acceptance gates (re-state from CLEANUP §C.11)

After every megafile, and again at the end of C.2, the following must all hold:

```bash
# zero files > 800 LOC in src/
find src -name '*.rs' -not -path '*/target/*' | xargs wc -l \
  | awk '$1>800 && $2!="total"{c++} END{print c}'   # → 0

# tests unchanged
cargo test --release 2>&1 | grep -E "test result" \
  | awk '{p+=$4; f+=$6} END{printf "%d/%d\n", p, f}'  # → 1565/0

# clippy still clean
cargo clippy --release -- -D warnings                  # → exit 0

# public surface unchanged
cargo public-api --simplified | diff /tmp/before.txt -  # → empty
```

---

## 1. `src/mir/lower.rs` — 3,488 LOC → 7 files

**Role:** lowers `hir::Program` to `mir::Program` (SSA). The body is one giant
`Lowerer` impl with three monster methods: `lower_expr` (≈1,234 lines, line
271→1505), `lower_stmt` (≈1,493 lines, line 1513→3006), and a small tail of
function-level helpers.

### 1.1 Target layout (`src/mir/lower/`)

```
src/mir/
  lower.rs           ← DELETE (becomes lower/mod.rs)
  lower/
    mod.rs           ~250 LOC   — re-exports + Lowerer struct + lower_program
    ctx.rs           ~280 LOC   — Lowerer struct, new(), emit/emit_void/
                                  switch_to/set_terminator/value_type/
                                  new_value/new_block (lines 67–270)
    expr.rs          ~700 LOC   — lower_expr arms: literals, binop, unary,
                                  field, index, struct/variant, lambda,
                                  cast, ref/deref, slice
    expr_ctrl.rs     ~600 LOC   — lower_expr arms that build CFG: if-expr,
                                  match-expr, block-expr, ternary,
                                  short-circuit && / ||
    stmt.rs          ~500 LOC   — non-control statements: Let, Assign,
                                  FieldAssign (line 139), IndexAssign,
                                  Expr(_), Return, Break, Continue
    control.rs       ~700 LOC   — While, For, Loop, If, Match, Sim, Select
                                  (the CFG-heavy ladder inside lower_stmt)
    concurrency.rs   ~250 LOC   — Spawn, Send, Recv, ActorCreate,
                                  Supervise, Sleep, ChannelOps
    util.rs          ~250 LOC   — collect_assigned_vars*, collect_new_binds,
                                  demote_vars_to_memory,
                                  collect_expr_var_refs_*,
                                  lower_function, lower_binop, lower_unaryop
                                  (lines 3013–3488)
```

### 1.2 Step-by-step

1. `git mv src/mir/lower.rs src/mir/lower/mod.rs`.
2. In `src/mir/mod.rs`: no change (`pub mod lower;` continues to point at the
   directory). Verify with `cargo check`.
3. **Extract `ctx.rs`** first: cut lines 67–270 (`struct Lowerer` + the 12
   small helper methods) into `lower/ctx.rs`. In `mod.rs`, write
   `mod ctx; pub(super) use ctx::Lowerer;` and add a `use super::ctx::*;`
   inside the impl files.
4. **Extract `util.rs`**: cut lines 3013–3488 (the post-`lower_stmt` tail).
   These are free functions and `Lowerer::collect_*` helpers — pure, no
   behaviour change.
5. **Carve `lower_expr`**: open the 1,234-line `lower_expr` and bucket each
   `match` arm:
   - data construction + arithmetic / pure → `expr.rs`
   - control-flow expressions (if, match, &&, ||, block) → `expr_ctrl.rs`

   Mechanically: copy the entire `match` skeleton to both files, delete the
   irrelevant arms from each, and replace the original method with a thin
   dispatcher:
   ```rust
   fn lower_expr(&mut self, e: &hir::Expr) -> ValueId {
       match &e.kind {
           // control-flow first (delegate to lower_expr_ctrl)
           ExprKind::If(_) | ExprKind::Match(_) | ExprKind::Block(_)
           | ExprKind::And(_,_) | ExprKind::Or(_,_) | ExprKind::Ternary(_,_,_)
               => self.lower_expr_ctrl(e),
           _   => self.lower_expr_value(e),
       }
   }
   ```

   Rename the original to `lower_expr_value` in `expr.rs` and create a
   companion `lower_expr_ctrl` in `expr_ctrl.rs`. **No arm changes its body.**
6. **Carve `lower_stmt`** identically — split CFG-building arms (While, For,
   Loop, Match-stmt, Sim, Select) into `control.rs`; keep
   straight-line stmts in `stmt.rs`; concurrency primitives go to
   `concurrency.rs`.
7. After every file is added, `cargo check` → `cargo test mir`.

### 1.3 Risks / impact outside the file

- **Public API:** `lower_program` is the only public entry point
  (`src/mir/mod.rs` re-exports it via `pub mod lower;`). Re-export it from
  `mod.rs`: `pub use self::ctx::lower_program;` (or keep it in `mod.rs`
  itself). Confirmed callers: `src/main.rs` (`mir::lower::lower_program`),
  `src/codegen/mir_codegen/mod.rs`, `src/incr.rs`, several integration tests
  in `tests/`.
- **C.1 boundary:** `Lowerer::lower_field_assign` (line 139) reads HIR field
  types — leave its body untouched until C.1 finalises HIR field
  representation.
- **`mir::printer`** is unaffected.

---

## 2. `src/typer/lower.rs` — 3,297 LOC → 8 files

**Role:** turns `ast::Program` (post-resolve, post-infer) into `hir::Program`,
including deferred-method resolution and exhaustiveness checking. Single
`impl Typer` block with 47 free fns/methods.

### 2.1 Target layout (`src/typer/lower/`)

```
src/typer/
  lower.rs           ← DELETE (becomes lower/mod.rs)
  lower/
    mod.rs           ~250 LOC  — `lower_program` (top-level pipeline,
                                 lines 44–558) + the deferred-resolution
                                 calls; re-exports.
    deferred.rs      ~600 LOC  — resolve_deferred_methods (559),
                                 resolve_deferred_fields (792),
                                 resolve_trait_constrained_vars (894),
                                 reclassify_method_call (945),
                                 resolve_all_types (1113)
    display.rs       ~250 LOC  — auto_derive_display (1190),
                                 collect_display_usage*
                                 (1285, 1291, 1333)
    resolve.rs       ~500 LOC  — resolve_fn (1407), resolve_block (1415),
                                 resolve_stmt (1421), resolve_expr (1549),
                                 resolve_pat (1860), resolve_filter (1882)
    decl.rs          ~700 LOC  — lower_actor_def (1889), lower_store_def
                                 (2002), lower_impl_block (2071),
                                 lower_fn / lower_fn_deferred (2146/2156),
                                 lower_test_block + build_test_runner
                                 (2264, 2283),
                                 lower_type_def (2333), lower_method* (2388–
                                 2400), lower_enum_def (2501),
                                 lower_extern (2530), lower_err_def (2546),
                                 build_fn_scheme (2115), hir_tail_type (2089)
    iter.rs          ~400 LOC  — type_implements_trait (2567),
                                 iter_element_type (2574),
                                 desugar_for_iter (2600),
                                 desugar_for_map (2717)
    block.rs         ~250 LOC  — lower_block (2925), emit_scope_drops*,
                                 collect_moved_var_ids,
                                 collect_hir_var_ids_*, needs_drop
    exhaust.rs       ~250 LOC  — check_exhaustiveness (3164),
                                 find_missing_patterns (3206),
                                 flatten_or_pat (3287),
                                 type_references_name (line 13, free fn)
```

### 2.2 Step-by-step

1. `git mv src/typer/lower.rs src/typer/lower/mod.rs`.
2. In `src/typer/mod.rs`, the existing `mod lower;` continues to work; nothing
   to change there.
3. Extract `exhaust.rs` first (purely free fns at the bottom — easiest). Move
   the `pub(super) fn type_references_name` and the trio
   `check_exhaustiveness` / `find_missing_patterns` / `flatten_or_pat`.
4. Extract `iter.rs` — `desugar_for_iter` and `desugar_for_map` are 500 lines
   of fairly mechanical AST construction with one external dependency
   (`type_implements_trait`); move it with them.
5. Extract `block.rs`, then `decl.rs`, then `resolve.rs`, then `display.rs`,
   then `deferred.rs` in that order. Each new file starts with
   `impl super::super::Typer { ... }` (re-opening the impl across modules is
   legal since they live in the same crate).
6. Final `mod.rs` keeps only `lower_program` and the `mod` declarations.

### 2.3 Risks / impact

- **Test runner generation.** `build_test_runner` (2283) emits HIR for the
  `--test` driver; the generator is consumed by `src/main.rs` via the
  ordinary `lower_program` API — no API change.
- **Re-exports.** Search for `typer::lower::` outside `src/typer/`:
  ```
  grep -rn 'typer::lower::' src tests
  ```
  If any external code references a sub-symbol, mirror it from `lower/mod.rs`
  with `pub(crate) use`.
- **DDR / deferred-fields infrastructure** is currently coupled to fields on
  `Typer` itself (`deferred_methods`, `deferred_fields`); those fields stay in
  `typer/mod.rs`. The new files only re-open the impl; they do not move
  state.

---

## 3. `src/main.rs` — 2,497 LOC → 8 files in `src/driver/`

**Role:** binary entry point. Today it carries the CLI, `cargo`-equivalent
project commands (init/fetch/update/package/publish), the multi-file source
loader, undefined-reference walker, package resolver, and the
`compile_and_link` pipeline orchestrator. CLEANUP §C.2 mandates this drop to
≤ 500 LOC.

### 3.1 Target layout

```
src/
  main.rs            ~80 LOC   — `fn main()` only: parse args, dispatch to
                                 driver, set exit code.
  driver/
    mod.rs           ~120 LOC  — `pub fn run(cli: Cli) -> ExitCode`,
                                 dispatches to cmd_*; re-exports.
    cli.rs           ~200 LOC  — `clap` derive: Cli, Cmd; `die()`, `dirs_cache`,
                                 `find_project_root` (lines 28–202).
    project.rs       ~350 LOC  — ProjectConfig, from_file, set_field,
                                 project_config_to_package, run_git
                                 (lines 395–624).
    cmd_init.rs      ~50 LOC   — cmd_init (485).
    cmd_pkg.rs       ~250 LOC  — cmd_fetch (517), cmd_update (556),
                                 cmd_package (625), cmd_publish (681),
                                 load_packages (1657).
    sources.rs       ~600 LOC  — find_project_entry (746),
                                 merge_source_files (777), collect_jade_files
                                 (782 + 2062), EntityIndex + impl
                                 (868–1024), resolve_modules (238),
                                 resolve_implicit_imports (1556),
                                 should_import_decl (225), decl_name (203).
    undef.rs         ~530 LOC  — collect_undefined_refs (1025) and the
                                 walk_expr / walk_pat / walk_block / walk_stmt
                                 / walk_type family (1083–1555).
    pipeline.rs      ~250 LOC  — compile_and_link (1696) — the lex → parse →
                                 resolve → type → mir → codegen → link
                                 sequence. (`--emit` handling stays here for
                                 now; see §3.3.)
```

### 3.2 Step-by-step

1. `mkdir src/driver && touch src/driver/mod.rs`.
2. Add `mod driver;` to `main.rs` (do **not** add to `lib.rs` — driver is
   binary-private until justified).
3. Move `Cli` and `Cmd` first (CLI types are independent). The body of
   `fn main()` becomes:
   ```rust
   fn main() -> std::process::ExitCode {
       let cli = <driver::cli::Cli as clap::Parser>::parse();
       driver::run(cli)
   }
   ```
4. Move project commands one at a time (`cmd_init`, `cmd_fetch`, …). Each is
   self-contained; no shared state.
5. Move `EntityIndex` + the source loader (`merge_source_files`,
   `collect_jade_files`, `resolve_modules`) into `sources.rs`. **Note:**
   `collect_jade_files` is defined twice (lines 782 and 2062 — likely an
   oversight). De-duplicate while moving; leave a comment justifying
   the choice. Verify both call sites resolve to one definition before
   committing.
6. Move the `walk_*` undefined-ref machinery into `undef.rs`.
7. Move `compile_and_link` last; it pulls in nearly every other helper.

### 3.3 Future split (record but defer)

`pipeline.rs` is still 250 LOC and contains all `--emit-{llvm,mir,hir,obj}`
branches. After C.1, peel each emitter out into `driver/emit.rs` (CLEANUP
§C.2 task list). For C.2 this is optional; ≤ 500 LOC is the gate, and we
hit that.

### 3.4 Risks / impact

- **`bin/jadec` stays as-is.** Verify with `ls src/bin/` whether other binaries
  duplicate any of the moved code; if so they should switch to importing from
  `driver::*` (only possible if `driver/` is exposed via `lib.rs`).
- **Tests in `tests/`** that shell out to the `jade` binary are unaffected.
- **`cargo install` / package metadata:** no change to `Cargo.toml`.
- **Fragility:** `compile_and_link` reaches into nearly every crate module;
  do this move *after* every other helper has moved, and only when
  `cargo check` is clean.

---

## 4. `src/typer/expr.rs` — 2,473 LOC → 5 files

**Role:** `Typer::lower_expr_expected` (the workhorse) and a handful of
generic-instantiation helpers. The file is one `impl Typer` block dominated by
two huge methods: `lower_expr_expected` (line 17 → ~1,720) and
`lower_struct_or_variant` / `lower_struct_or_variant_with_typeargs`.

### 4.1 Target layout (`src/typer/expr/`)

```
src/typer/
  expr.rs            ← DELETE (becomes expr/mod.rs)
  expr/
    mod.rs           ~80 LOC   — re-exports; trivial wrapper for lower_expr
                                 (line 13) that delegates to lower_expr_expected.
    primary.rs       ~700 LOC  — lower_expr_expected arms for: literals,
                                 idents, binop, unary, cast, field, index,
                                 ternary, ref/deref, array/tuple lits, slice.
    collection.rs    ~300 LOC  — list comprehension, vec/map/set lits,
                                 spread, range, group of `Vec`-bound arms.
    control.rs       ~400 LOC  — if/match/block expression typing,
                                 short-circuit &&/||, try-as-expr, return-as-
                                 expr (where syntactically permitted).
    construct.rs     ~600 LOC  — lower_struct_or_variant +
                                 lower_struct_or_variant_with_typeargs
                                 (lines 1855–2333), including type-arg
                                 application.
    typeargs.rs      ~400 LOC  — collect_type_mapping (1722),
                                 substitute_type_params (1759),
                                 expr_to_type_args (1790),
                                 expr_to_single_type (1803),
                                 ident_to_type (1831),
                                 lower_lambda_with_expected (2334),
                                 maybe_coerce_to (2415).
```

### 4.2 Step-by-step

1. `git mv src/typer/expr.rs src/typer/expr/mod.rs`.
2. Extract `typeargs.rs` first (lines 1722–end are largely free of cross-
   references to the giant arm body).
3. Carve `lower_expr_expected` by **arm bucketing**, identical to §1
   technique: keep the master `match` shape in `mod.rs` (or in a `dispatch.rs`)
   delegating to per-bucket helpers `lower_primary` / `lower_collection` /
   `lower_control` / `lower_construct`. Each helper is a `pub(super) fn` on
   `Typer`.
4. `lower_expr` (line 13, the public 4-line wrapper) stays in `mod.rs`.

### 4.3 Risks / impact

- The HIR `ExprKind` enum is **frozen** for the duration of this split. If
  you must add a variant, do it in a separate PR before or after.
- `maybe_coerce_to` is called from sibling files (`stmt.rs`, `call.rs`).
  Promote to `pub(super)` and re-export from `expr/mod.rs` so call sites
  remain `self.maybe_coerce_to(...)`.

---

## 5. `src/typer/call.rs` — 2,100 LOC → 5 files

**Role:** call-site typing, named-args resolution, spread expansion, method
dispatch, pipe (`|>`) typing, monomorphization driver.

### 5.1 Target layout (`src/typer/call/`)

```
src/typer/
  call.rs            ← DELETE (becomes call/mod.rs)
  call/
    mod.rs           ~60 LOC   — re-exports + module declarations.
    args.rs          ~400 LOC  — resolve_named_args (16, free fn),
                                 expand_spread_args (83).
    fn_call.rs       ~700 LOC  — Typer::lower_call (163 → ~566). The free-
                                 function call path, including overload
                                 resolution and arity coercion.
    method_call.rs   ~700 LOC  — Typer::lower_method_call (567 → ~1870).
    pipe.rs          ~250 LOC  — Typer::lower_pipe (1871 → ~2036).
    mono.rs          ~120 LOC  — build_type_map (2037),
                                 monomorphize_call (2061).
```

### 5.2 Step-by-step

1. Move `mono.rs` first (smallest, fewest dependencies).
2. Move `args.rs` next (free functions).
3. `fn_call.rs` and `method_call.rs` are the meat; both reopen `impl Typer`.
   Move each as a single block. Inside each, internal helper methods
   (`resolve_overload_*`, etc.) move with them.
4. `pipe.rs` last.

### 5.3 Risks / impact

- `monomorphize_call` is reached from `lower.rs` (deferred resolution path).
  Ensure it remains importable as `super::call::monomorphize_call` or expose
  via `pub(crate) use` in `typer/mod.rs`.
- `resolve_named_args` is called by both `lower_call` and `lower_method_call`;
  declare it `pub(super)` in `args.rs`.

---

## 6. `src/typer/mod.rs` — 1,860 LOC → 2 files

**Role:** `Typer` struct + ~30 small accessor/mutator methods + **130 unit
tests**. The non-test surface is only ~440 LOC; the bulk is `mod tests`.

### 6.1 Target layout

```
src/typer/
  mod.rs             ~440 LOC  — header types (DeferredMethod/Field, VarInfo),
                                 Typer struct, Typer::new + setters,
                                 push_scope/pop_scope, define_var/find_var/
                                 update_var, fresh_id, generalize, the
                                 *_method_ret_ty helpers, free_type_vars_in_env,
                                 ownership_for_type. Lines 1–445.
  tests.rs           ~1,420 LOC — the existing #[cfg(test)] mod tests,
                                  unchanged. Wired in via `mod tests;` (the
                                  outer `#[cfg(test)] mod tests` becomes the
                                  whole file with `#![cfg(test)]` at top).
```

### 6.2 Step-by-step

1. Open the single `#[cfg(test)] mod tests { ... }` block (starts ~line 441,
   ends at EOF).
2. Cut the **inner** body (the contents between `{` and `}`) into a new
   `src/typer/tests.rs` with `#![cfg(test)]` as its first line.
3. Replace the original `mod tests { ... }` with `#[cfg(test)] mod tests;`.
4. Adjust imports: change `use super::*;` to `use crate::typer::*;` etc., or
   keep `use super::*;` (still resolves correctly when `mod tests;` lives in
   `mod.rs`).

### 6.3 Risks / impact

- Tests reference `pub(crate)` items from `typer/mod.rs`; visibility is
  unchanged because the new file is still `crate::typer::tests`.
- Watch for `parse(...)` / `type_check(...)` test helpers (lines 446–457):
  they are sibling tests' utilities and must remain in `tests.rs`.

---

## 7. `src/parser/expr.rs` — 1,826 LOC → 4 files

**Role:** Pratt-style expression parser plus several pure
"placeholder substitution" passes used by the lambda-shorthand desugaring.

### 7.1 Target layout (`src/parser/expr/`)

```
src/parser/
  expr.rs            ← DELETE (becomes expr/mod.rs)
  expr/
    mod.rs           ~120 LOC  — re-exports + parse_expr (91), parse_expr_inner
                                 (102), parse_pat (10), parse_single_pat (24).
    pratt.rs         ~700 LOC  — operator-precedence ladder: parse_ternary
                                 (110), parse_pipeline (163), parse_eq (203),
                                 parse_cmp (230), parse_exp (276),
                                 parse_unary (288), parse_postfix (323).
    primary.rs       ~700 LOC  — parse_primary (415), parse_literal_token
                                 (1093), parse_query_block (1128),
                                 parse_query_clause (1136), parse_builtin_call
                                 (1188), is_field_init (1204), is_named_arg
                                 (1210), parse_args (1216), parse_type (1246),
                                 ident_to_type (1309), parse_interp (1331).
    placeholder.rs   ~480 LOC  — contains_placeholder, replace_placeholder,
                                 contains_index_placeholder,
                                 replace_index_placeholder, plus all four
                                 *_in_block / *_in_stmt helpers
                                 (lines 1364 → end).
```

### 7.2 Step-by-step

1. Promote to directory.
2. **Extract `placeholder.rs` first** — it is a self-contained block of pure
   tree walks, no `Parser` state.
3. Pull `pratt.rs` and `primary.rs` apart by line range; each is a thin
   `impl Parser` extension.
4. `parse_pat` / `parse_single_pat` stay in `mod.rs` since they are tiny and
   shared.

### 7.3 Risks / impact

- `pub(super) fn contains_placeholder` etc. are imported by `parser/stmt.rs`
  (for lambda body desugaring). After the move, change to:
  ```rust
  use crate::parser::expr::placeholder::{contains_placeholder, ...};
  ```
  or expose them via `pub(super) use placeholder::*;` from `expr/mod.rs`.

---

## 8. `src/mir/opt.rs` — 1,535 LOC → 7 files

**Role:** the MIR optimization battery — 13 independent passes plus a small
range-analysis utility, all free functions taking `&mut Function`.

### 8.1 Target layout (`src/mir/opt/`)

```
src/mir/
  opt.rs             ← DELETE (becomes opt/mod.rs)
  opt/
    mod.rs           ~80 LOC   — pub enum OptLevel, pub fn optimize (the
                                 driver loop, line 23 → 59), pub use of every
                                 sub-pass.
    subst.rs         ~200 LOC  — subst_inst (60), subst_term (217), helpers.
    uses.rs          ~190 LOC  — collect_used (243), collect_inst_uses (259),
                                 collect_term_uses (404), is_pure (418),
                                 collect_inst_operands (1325).
    fold.rs          ~280 LOC  — ConstVal enum + impl (450), constant_fold
                                 (467), fold_binop (521), fold_cmp (562),
                                 fold_unary (585), fold_cast (595).
    scalar_passes.rs ~360 LOC  — copy_propagation (608), simplify_phis (668),
                                 dead_code_elimination (729),
                                 strength_reduction (748).
    memory_passes.rs ~150 LOC  — store_load_forwarding (874),
                                 redundant_store_elimination (951),
                                 constant_branch_elimination (997).
    cfg_passes.rs    ~370 LOC  — global_value_numbering (1023) + gvn_key
                                 (1078) + is_commutative (1107),
                                 branch_threading (1125),
                                 loop_invariant_code_motion (1177),
                                 merge_linear_blocks (1335),
                                 remove_unreachable_blocks (1419).
    ranges.rs        ~90 LOC   — IntRange struct + impl (1449), compute_ranges
                                 (1478).
```

### 8.2 Step-by-step

1. Extract by pass. Each pass is a free `pub fn ...(func: &mut Function) -> bool`
   with no shared state — splits are completely mechanical.
2. `mod.rs` keeps `OptLevel`, `optimize`, and `pub use` re-exports of every
   pass (preserves the existing `mir::opt::dead_code_elimination` import
   path).

### 8.3 Risks / impact

- External callers: `src/codegen/mir_codegen/mod.rs`, `src/main.rs`,
  `src/incr.rs`, and `tests/opt_*` use `mir::opt::*`. The `pub use` re-exports
  in `mod.rs` keep all these paths valid.
- `compute_ranges` returns `HashMap<ValueId, IntRange>`; tests in `tests/`
  reference `IntRange` directly — confirm the type stays `pub`.

---

## 9. `src/hir.rs` — 1,373 LOC → 3 files

**Role:** every HIR data type (Program, Fn, Stmt, Expr, ExprKind, Pat, …) plus
a 700-line debug pretty-printer.

### 9.1 Target layout (`src/hir/`)

```
src/
  hir.rs             ← DELETE (becomes hir/mod.rs)
  hir/
    mod.rs           ~700 LOC  — DefId, Ownership, Program, TraitImpl, Fn,
                                 Param, TypeDef/Field, EnumDef/Variant/VField,
                                 ExternFn, ErrDef/ErrVariant, ActorDef/
                                 HandlerDef, StoreField/StoreDef,
                                 SupervisorStrategy/SupervisorDef,
                                 StoreFilter/Cond, Stmt, SelectArm, Bind, Expr,
                                 ExprKind, BuiltinFn, CoercionKind, If, While,
                                 For, Loop, Match, Arm, Pat, FieldInit,
                                 AsmBlock, Global. Lines 1–629.
    print.rs         ~745 LOC  — pretty_print (630), PrettyPrinter struct +
                                 impl (639), all helpers: line, push, pop,
                                 program, extern_fn, type_def, enum_def,
                                 err_def, actor_def, store_def, trait_impl,
                                 fn_def, block, stmt, if_stmt, pat_str,
                                 expr_str. Lines 630→end.
```

This drops both files under 800 LOC.

### 9.2 Step-by-step

1. `git mv src/hir.rs src/hir/mod.rs`.
2. Cut `pretty_print` and the `PrettyPrinter` impl into `print.rs`. Add
   `mod print; pub use print::pretty_print;` to `mod.rs`.
3. `print.rs` needs only `use super::*;`.

### 9.3 Risks / impact

- The HIR types are referenced by every other module in `src/`. Their paths
  do **not** change (`crate::hir::Fn` → `crate::hir::Fn`). No external
  modifications needed.
- `pretty_print` is called from `src/main.rs` (`--emit-hir`) and tests; the
  `pub use` keeps `hir::pretty_print` working.

---

## 10. `src/parser/stmt.rs` — 1,368 LOC → 4 files

**Role:** statement-level parser. One huge `impl Parser` with a 380-line
`parse_stmt` (line 40) and a 380-line `parse_bind` (line 450); the rest are
medium-sized statement-form parsers.

### 10.1 Target layout (`src/parser/stmt/`)

```
src/parser/
  stmt.rs            ← DELETE (becomes stmt/mod.rs)
  stmt/
    mod.rs           ~80 LOC   — module declarations + re-exports;
                                 parse_block (13).
    dispatch.rs      ~420 LOC  — parse_stmt (40), aug_op (432), is_bind (381),
                                 is_tuple_bind (400), parse_tuple_bind (420).
    bind.rs          ~410 LOC  — parse_bind (450), complete_expr_after_pipeline
                                 (827), finish_bare_bangbang (881),
                                 finish_bare_handler_chain (924).
    control.rs       ~200 LOC  — parse_if (1024), parse_while (1053),
                                 parse_for (1065), parse_match (1109),
                                 parse_arm (1122), parse_asm_stmt (1146).
    store.rs         ~270 LOC  — parse_insert_stmt (1213), parse_insert_value
                                 (1234), parse_delete_stmt (1255),
                                 parse_set_stmt (1263), parse_transaction
                                 (1286), parse_store_filter (1294),
                                 parse_filter_op (1332), parse_destroy_stmt
                                 (1346), parse_restore_stmt (1354),
                                 parse_save_stmt (1362).
```

### 10.2 Step-by-step

1. Extract `store.rs` first — self-contained and at the bottom of the file.
2. Then `control.rs` (1024 → 1212) — also self-contained.
3. Split `parse_stmt` from `parse_bind` last; both are big and `parse_stmt`
   delegates to `parse_bind` and the others.

### 10.3 Risks / impact

- `parse_stmt` is the public entry point of this module. Re-export from
  `mod.rs` so `crate::parser::stmt::Parser::parse_stmt` resolves identically.
  (It's an `impl` method; only `Parser`'s import needs to be visible to
  callers.)

---

## 11. `src/lexer.rs` — 1,295 LOC → 2 files

**Role:** Token enum + `Lexer` impl + ~25 unit tests (lines 1075→end).

### 11.1 Target layout

```
src/
  lexer.rs           ~715 LOC  — Token enum (9 → 282), Display impl (143),
                                 Spanned, LexError, Lexer + every lex_* fn
                                 (293 → 1074).
  lexer_tests.rs     ~225 LOC  — the entire `#[cfg(test)] mod tests` body.
```

Or, equivalently, promote to `src/lexer/{mod.rs, tests.rs}`. Either works;
the flat-file form is simpler since `lexer.rs` is in scope and consumers
import via `crate::lexer::Token`.

### 11.2 Step-by-step

1. Cut `mod tests { ... }` to `src/lexer_tests.rs` with `#![cfg(test)]`.
2. In `lexer.rs` add `#[cfg(test)] #[path = "lexer_tests.rs"] mod tests;`.

### 11.3 Risks / impact

- Token enum is consumed by every parser file; layout unchanged.
- Recommended **follow-up (out of scope of this split):** generate the
  keyword table from a build-time include (CLEANUP §C.5 task 3). Track but do
  not block on it.

---

## 12. `src/parser/decl.rs` — 1,226 LOC → 4 files

**Role:** declaration parser — fns, types, enums, errs, actors, stores,
traits, impls, supervisors, migrations, views.

### 12.1 Target layout (`src/parser/decl/`)

```
src/parser/
  decl.rs            ← DELETE (becomes decl/mod.rs)
  decl/
    mod.rs           ~120 LOC  — Either<A,B> (9), parse_decl (62) — the
                                 top-level dispatcher.
    yield_scan.rs    ~30 LOC   — body_contains_yield (14),
                                 stmt_has_yield (18),
                                 expr_has_yield (37).
    fn.rs            ~250 LOC  — parse_fn_attrs (42), parse_type_params (154),
                                 parse_extern (188), parse_fn (229),
                                 parse_fn_param (291), parse_param (324).
    types.rs         ~200 LOC  — parse_type_def (348), parse_layout_attrs
                                 (384), parse_field (419), parse_enum_def
                                 (442), parse_variant (461), parse_vfield
                                 (491), parse_use_decl (507),
                                 parse_err_def (552), parse_test_block (584).
    actor_store.rs   ~430 LOC  — parse_actor_def (604), parse_handler (637),
                                 parse_store_def (680), parse_store_field
                                 (776), parse_migration_def (886),
                                 parse_view_def (957), parse_alter_op (980).
    trait_sup.rs     ~200 LOC  — parse_trait_def (1035), parse_trait_method
                                 (1068), parse_impl_block (1128),
                                 parse_supervisor_def (1178).
```

### 12.2 Notes

- `Either<A,B>` is a tiny private helper used by `parse_decl`; keep in
  `mod.rs`.
- `body_contains_yield` is also called from `src/parser/stmt.rs` for
  coroutine detection — confirm with `grep` and re-export accordingly.

---

## 13. `src/typer/unify.rs` — 1,206 LOC → 3 files

**Role:** the core unification engine + 27 unit tests (lines 940 → end).

### 13.1 Target layout (`src/typer/unify/`)

```
src/typer/
  unify.rs           ← DELETE (becomes unify/mod.rs)
  unify/
    mod.rs           ~520 LOC  — InferCtx + new (47), strict/default-warning
                                 plumbing, fresh_var family (132–158),
                                 constraint / merge_constraints / record_usage
                                 / constrain (160–273), find / union / unify_at /
                                 suggest_fix / origin_of / unify (274–542).
                                 Lines 1 → 564.
    resolve.rs       ~370 LOC  — type_to_impl_name (565), occurs_in (584),
                                 canonicalize_type (612), shallow_resolve (644),
                                 resolve / resolve_container_elem / resolve_core
                                 (658–848), instantiate (849), substitute (887),
                                 try_resolve (919). Lines 565 → 939.
    tests.rs         ~280 LOC  — every #[test] (940 → end).
```

### 13.2 Step-by-step

1. Promote to directory.
2. Extract `tests.rs` first (easy win — drops file under 1,000 LOC).
3. Extract `resolve.rs` — these methods all reopen `impl InferCtx` and form
   a coherent "type resolution" cluster.

### 13.3 Risks / impact

- `InferCtx` is referenced as `crate::typer::unify::InferCtx`; preserve the
  re-export.
- Test helpers in `tests.rs` may need `use super::*;` and also `use
  crate::types::*;` — confirm by `cargo test typer::unify`.

---

## 14. `src/typer/stmt.rs` — 1,120 LOC → 4 files

**Role:** statement-level type checking; the giant 700-line `lower_stmt`
arm-table plus store-filter machinery.

### 14.1 Target layout (`src/typer/stmt/`)

```
src/typer/
  stmt.rs            ← DELETE (becomes stmt/mod.rs)
  stmt/
    mod.rs           ~30 LOC   — module declarations + re-export.
    dispatch.rs      ~460 LOC  — Typer::lower_stmt (11 → 715). The arm
                                 dispatcher; arm bodies for non-control,
                                 non-store stmts inline here (Let, Assign,
                                 Return, Break, Continue, Expr).
    filter.rs        ~180 LOC  — lower_store_filter (716), expr_to_store_filter
                                 (761), flatten_filter_expr (793),
                                 merge_where_clauses (833).
    block.rs         ~280 LOC  — lower_block_no_scope (858), lower_if (886),
                                 collect_block_new_binds (936), lower_match
                                 (950), lower_pat (1004).
```

### 14.2 Notes

- `lower_stmt` may be too large for a single file even after this split. If
  the resulting `dispatch.rs` exceeds 800 LOC, further split control-flow
  arms (For, While, Loop) into a `control.rs` sibling using the same arm-
  bucketing technique as §1.

---

## 15. `src/ownership.rs` — 1,048 LOC → 3 files

**Role:** ownership / borrow verifier; 26 free fns + small impl + tests.

### 15.1 Target layout (`src/ownership/`)

```
src/
  ownership.rs       ← DELETE (becomes ownership/mod.rs)
  ownership/
    mod.rs           ~330 LOC  — OwnershipDiag (10), DiagKind (17), VarState
                                 (28), OwnershipVerifier (37) + impl::new (44)
                                 / verify (52) / verify_fn (76) /
                                 push_scope (707) / pop_scope (711) /
                                 define (715) / lookup / lookup_mut /
                                 check_use / record_borrow / record_move /
                                 check_return_borrows / extract_root_var.
                                 Lines 1–838 minus the verify_block/stmt/expr
                                 trio.
    verify.rs        ~580 LOC  — verify_block (97), verify_block_no_scope
                                 (105), verify_stmt (111), verify_expr (289),
                                 verify_pat (667). The big walk.
    walks.rs         ~165 LOC  — collect_var_ids_block / _stmt / _expr
                                 (lines 849 → 1011).
    tests.rs         ~40 LOC   — #[cfg(test)] mod tests (1012 → end).
```

### 15.2 Risks / impact

- Public surface: `OwnershipVerifier`, `OwnershipDiag`, `DiagKind` exported
  from `mod.rs`. Confirm with `grep -rn 'crate::ownership::'`.

---

## 16. `src/parser/mod.rs` — 1,045 LOC → 2 files

**Role:** Parser struct, top-level helpers (`peek`, `advance`, `error`,
indentation), the multi-clause-fn desugarer, and **~60 unit tests**
(lines 523 → end).

### 16.1 Target layout

```
src/parser/
  mod.rs             ~525 LOC  — ParseError (7), Parser struct (12),
                                 impl Parser (51 → ~345) + the two free
                                 desugaring fns (346, 399) and the
                                 helpers underneath. Lines 1 → 522.
  tests.rs           ~525 LOC  — the existing `#[cfg(test)] mod tests` body.
```

### 16.2 Step-by-step

1. Cut the `mod tests { ... }` body to `src/parser/tests.rs` with
   `#![cfg(test)]` at the top.
2. Replace original block with `#[cfg(test)] mod tests;` in `mod.rs`.

### 16.3 Risks / impact

- Test helpers (`fn parse(s: &str) -> ...`) move with the tests.
- The remaining `mod.rs` still pushes ~520 LOC. Acceptable (under 800).

---

## 17. `src/comptime.rs` — 913 LOC → 3 files

**Role:** compile-time evaluator + constant folder; mixes a "pure-fn detector"
and an interpreter with a bottom-up folder.

### 17.1 Target layout (`src/comptime/`)

```
src/
  comptime.rs        ← DELETE (becomes comptime/mod.rs)
  comptime/
    mod.rs           ~80 LOC   — pub fn fold_program (9) entry point + module
                                 declarations + ConstVal enum (139, shared).
    purity.rs        ~100 LOC  — is_pure_fn (42), is_pure_stmt (46),
                                 is_pure_expr (66).
    eval.rs          ~250 LOC  — try_eval_pure_call (95), to_expr (147),
                                 eval_block (158), eval_expr (205),
                                 eval_binop (258).
    fold.rs          ~480 LOC  — fold_block_with_fns (285), fold_stmt_with_fns
                                 (291), fold_expr_with_fns (377), fold_block
                                 (396), fold_stmt (402), fold_expr (488),
                                 try_fold (745), fold_binop (758),
                                 fold_int_op (779), fold_float_op (802),
                                 fold_unary (817), fold_ternary (827),
                                 fold_cast (835), fold_builtin (872),
                                 make (911).
```

### 17.2 Risks / impact

- `ConstVal` is referenced from both `eval.rs` and `fold.rs` — keep in
  `mod.rs` (or in a shared `value.rs`) and re-export.

---

## 18. `src/perceus/uses.rs` — 858 LOC → 3 files

**Role:** use-counting for the Perceus reference-counting pass.

### 18.1 Target layout (`src/perceus/uses/`)

```
src/perceus/
  uses.rs            ← DELETE (becomes uses/mod.rs)
  uses/
    mod.rs           ~30 LOC   — module declarations + impl PerceusPass
                                 (line 9 stub) + re-exports.
    block.rs         ~290 LOC  — count_uses_block (10), count_uses_stmt (16),
                                 count_uses_block_conservative (148).
    refs.rs          ~330 LOC  — collect_refs_block (160), collect_refs_stmt
                                 (166), collect_refs_expr (255).
    expr.rs          ~360 LOC  — count_uses_pat (498), count_uses_expr (530),
                                 count_uses_expr_escaping (847).
```

### 18.2 Notes

- This file contributes to Perceus pass selection (CLEANUP §C.4 R11). The
  split is safe regardless of which Perceus path wins; if the entire module
  is later archived under `_archive_perceus.rs`, the smaller files make
  removal easier.

---

## 19. Cross-cutting impact summary

After all 18 splits land, the changes outside the split files themselves are:

| Affected file | Change |
| --- | --- |
| `src/lib.rs` | None (every megafile is either a `mod.rs` for a directory of the same name, or remains a flat file). Verify after each split. |
| `src/typer/mod.rs` | Update the `mod expr; mod call; mod lower; mod stmt;` lines if any changed name (none should). |
| `src/codegen/mir_codegen/mod.rs` | Calls into `mir::lower::lower_program` — unchanged. |
| `src/main.rs` | Becomes ~80 LOC; the giant body lives in `src/driver/`. |
| `tests/*` | No changes — they import via the crate's public API which is preserved by `pub use` re-exports. |
| `Cargo.toml` | No changes. The `[[bin]]` section continues to point at `src/main.rs`. |
| `src/bin/*` | Audit for direct imports of moved symbols; expected to be unaffected. Run `grep -rn 'use jade::' src/bin/` once before starting and once after. |

---

## 20. Per-megafile LOC ledger (live, fill in as you go)

| File | Before | After (max single file) | Status |
| --- | ---: | ---: | --- |
| `src/mir/lower.rs` | 3,488 | _≤ 700_ | ⏳ |
| `src/typer/lower.rs` | 3,297 | _≤ 700_ | ⏳ |
| `src/main.rs` + `src/driver/` | 2,497 | _≤ 600_ | ⏳ |
| `src/typer/expr.rs` | 2,473 | _≤ 700_ | ⏳ |
| `src/typer/call.rs` | 2,100 | _≤ 700_ | ⏳ |
| `src/typer/mod.rs` | 1,860 | _≤ 440_ + tests file | ⏳ |
| `src/parser/expr.rs` | 1,826 | _≤ 700_ | ⏳ |
| `src/mir/opt.rs` | 1,535 | _≤ 370_ | ⏳ |
| `src/hir.rs` | 1,373 | _≤ 745_ | ⏳ |
| `src/parser/stmt.rs` | 1,368 | _≤ 420_ | ⏳ |
| `src/lexer.rs` | 1,295 | _≤ 715_ | ⏳ |
| `src/parser/decl.rs` | 1,226 | _≤ 430_ | ⏳ |
| `src/typer/unify.rs` | 1,206 | _≤ 520_ | ⏳ |
| `src/typer/stmt.rs` | 1,120 | _≤ 460_ | ⏳ |
| `src/ownership.rs` | 1,048 | _≤ 580_ | ⏳ |
| `src/parser/mod.rs` | 1,045 | _≤ 525_ | ⏳ |
| `src/comptime.rs` | 913 | _≤ 480_ | ⏳ |
| `src/perceus/uses.rs` | 858 | _≤ 360_ | ⏳ |

Acceptance: this column shows **0 files > 800 LOC** when C.2 is complete and
`cargo test --release` reports **1565 passed / 0 failed**.

---

## 21. Open questions / decisions to make before starting

1. **Tests-file naming convention.** Two reasonable patterns:
   - `foo.rs` + `foo_tests.rs` (sibling file with `#[path = ...]`).
   - `foo/mod.rs` + `foo/tests.rs` (directory).

   Pick one in `CONTRIBUTING.md` (CLEANUP §C.9 task 3) **before** §6, §11,
   §16. This plan assumes `foo/tests.rs` for files already promoted to
   directories, and the flat `lexer_tests.rs` form for `lexer.rs` so it does
   not need to become a directory.

2. **`pub(super) use` re-export ergonomics.** The plan uses `pub(super) use`
   for sibling-only access. Confirm clippy is OK (`unused_imports` shouldn't
   fire because of the re-export, but `redundant_imports` may — silence with
   `#[allow]` on the re-export itself).

3. **Sequencing relative to C.1.** §1 (`mir/lower.rs`) and §3
   (`main.rs::compile_and_link`) touch the codegen boundary. Recommend
   blocking these on C.1 task #4 (store-codegen migration) reaching at least
   "MIR-side store insert/query landed" — otherwise the file gets re-shaped
   twice. All other megafiles in this plan (15 of 18) can proceed in
   parallel with C.1.

4. **C.3 interaction.** Each split file should keep `.unwrap()` counts at or
   below the source file's pre-split number. After all splits, run the C.3
   `b!()` macro pass once across the new layout — easier on smaller files.
