# Access Semantics — Implementation Sprint

> Companion to [docs/access-semantics.md](access-semantics.md). That document
> establishes *what* we're building and *why*. This document is the
> *how* and *in what order*: file lists, test gates, acceptance
> criteria, and the explicit invariants each phase must preserve.

This is the master sprint plan. Each phase is independently shippable
(green build, green tests). Phases are ordered so the build and test
suite stay passing on every commit; later phases depend on the IR /
analysis introduced in earlier phases. Do not parallelize phases.

---

## 0. Ground truth and standing invariants

These hold at every commit in every phase:

| Invariant                                 | Verified by                                         |
|-------------------------------------------|-----------------------------------------------------|
| `cargo build --release` succeeds          | CI build step                                       |
| `cargo test --release --lib` passes       | 216+ lib tests                                      |
| `cargo test --release --test bulk_tests`  | 886+ bulk tests; **must never regress**             |
| `valgrind /tmp/split` is clean            | spot-check after String/container changes           |
| No new clippy errors                      | `cargo clippy --release -- -D warnings` (eventually)|
| Benchmark deltas tracked                  | `python run_benchmarks.py --compare` before/after   |

### Standing rules

- **Root-cause discipline (max-mode).** No symptom patches. If a fix
  in phase N reveals that phase N−1 made the wrong call, walk back
  and re-do N−1.
- **No silent regressions in semantics.** Every behavior change that
  affects user code goes through the test corpus first. Add a new
  bulk test for every new behavior.
- **No new `unimplemented!()`** in code paths the new tests will hit.
- **Surface-syntax decisions are frozen.** The keywords are `copy`,
  `ref`, `mut`, `take`. The annotations are `@resource`, `@atomic`,
  `@weakable`. Any pressure to add more goes to a follow-up doc, not
  this sprint.

---

## 1. Surface design recap (frozen)

For ease of reference; the canonical source is
[docs/access-semantics.md §4](access-semantics.md#4-proposal-for-jinn).

### Modifiers (binding RHS, parameter type, `for` binder)

| Modifier | Meaning                                                          | Source-side after | Consumer mutability |
|----------|------------------------------------------------------------------|-------------------|---------------------|
| (none)   | Inference picks per §4.3 table                                   | varies            | varies              |
| `copy`   | Deep clone (POD: bitcopy; heap: full clone)                      | unchanged         | independent owner   |
| `ref`    | Shared read-only alias; T1 borrow / T2 Rc / T3 Arc               | aliased           | read-only           |
| `mut`    | Exclusive mutable alias; T1 `&mut` / T2 Rc<Cell> / T3 Arc<Mutex> | aliased exclusively | read-write       |
| `take`   | Move out (or remove from container slot)                         | gone / tombstoned | sole owner          |

`ref` and `mut` are mutually exclusive. `copy` and `take` are
mutually exclusive with each other and with `ref`/`mut`.

### Type annotations (on `type` declarations)

| Annotation  | Meaning                                                                  |
|-------------|--------------------------------------------------------------------------|
| `@resource` | Linear; never copies; auto-`*drop` at owning scope's end                 |
| `@atomic`   | Cross-thread aliasing; `ref` lowers to Arc, `mut` to Arc<Mutex>          |
| `@weakable` | (Optional, only with `@atomic`) Type may be referenced via `weak ref`    |

Reuses the existing `@`-attribute machinery (`@inline`, `@hot`,
`@packed`, etc. — see [src/ast.rs](src/ast.rs#L371)).

### Three lowering tiers for `ref` and `mut`

| Tier | Trigger                                                            | `ref` lowers to | `mut` lowers to    |
|------|--------------------------------------------------------------------|-----------------|--------------------|
| T1   | Alias contained in source's lifetime; no escape                    | raw `*T`        | raw `*mut T`       |
| T2   | Escapes (return, struct field, escaping closure); single-threaded  | `Rc<T>`         | `Rc<Cell<T>>`      |
| T3   | Crosses thread boundary, OR source type is `@atomic`               | `Arc<T>`        | `Arc<Mutex<T>>`    |

Cross-thread boundary = channel send, actor message, `spawn` capture,
store-write, any function call where the parameter is a `Channel<T>`
or `ActorRef<T>`.

---

## 2. Phase 1 — Surface & HIR plumbing

**Objective:** parse, AST-represent, and HIR-propagate the four
modifiers and the three new type annotations. **No semantic changes
yet** — defaults stay the same. After this phase, parsing the new
syntax produces correct HIR but the typer still picks ownership the
old way.

### 2.1 Lexer

- [src/lexer/mod.rs](src/lexer/mod.rs) — register four contextual
  keywords. These are *not* reserved identifiers; they're recognized
  as keywords only when they appear in modifier position. The lexer
  emits them as `Token::Ident` and the parser disambiguates by
  position. This matches how `take` already works as a query clause.
- Verify the `@` token (`Token::At`) is unchanged and continues to
  drive attribute parsing.

### 2.2 AST nodes

- [src/ast.rs](src/ast.rs):
  - Add `enum AccessMod { Copy, Ref, Mut, Take }` (with a
    `Display`/`Debug` impl).
  - Add `pub access_mod: Option<AccessMod>` to:
    - `struct Bind` (binding statement: `x is …`)
    - `struct Param` (function parameter: `name as T`)
    - `struct ForLoop` (for `for COPY x in xs`)
    - `struct Field` (struct/type field declaration: `name as ref T`)
  - Extend `LayoutAttrs` with `pub resource: bool, pub atomic: bool,
    pub weakable: bool`. Rename to `TyAttrs` and add a doc comment
    explaining it's the catch-all for `@`-attributes on `type`
    decls. Add migration shim if necessary.

### 2.3 Parser

- [src/parser/stmt/bind.rs](src/parser/stmt/bind.rs):
  parse optional `copy`/`ref`/`mut`/`take` after `is` and before the
  expression. `take`/`ref`/`mut`/`copy` followed by `(` is *not* a
  modifier (it's the call expression `ref(…)`); the modifier form
  requires the keyword to be immediately followed by an expression
  start that is *not* a paren-call of that name.
- [src/parser/expr/primary.rs](src/parser/expr/primary.rs)
  `parse_type` — when the next token is a modifier keyword, consume
  it and store on the AST `Param`/`Field`. Order: `as ref T`,
  `as mut T`, `as copy T`, `as take T`.
- [src/parser/stmt/control.rs](src/parser/stmt/control.rs)
  (or wherever `for` is parsed): allow the modifier between `for`
  and the binder: `for ref x in xs`.
- [src/parser/decl/types.rs](src/parser/decl/types.rs):
  recognize `@resource`, `@atomic`, `@weakable` in the existing
  `parse_type_attrs` (or equivalent) for `type` decls. Same place
  that already handles `@packed`/`@strict`/`@align`.

### 2.4 HIR plumbing

- [src/hir/mod.rs](src/hir/mod.rs):
  - Add `pub access_mod: Option<hir::AccessMod>` to `hir::Bind`,
    `hir::Param`, `hir::ForLoop`.
  - Mirror `hir::AccessMod` from AST.
  - Add `pub attrs: TyAttrs` to `hir::TypeDef` (replacing the
    existing `layout: LayoutAttrs` field path; LayoutAttrs becomes
    a subset).
- [src/typer/lower/decl.rs](src/typer/lower/decl.rs) and the bind /
  param / for-loop lowerers — copy `access_mod` from AST to HIR.
- For now, the typer **ignores** `access_mod` when computing
  ownership. The new field is present and threaded but inert.

### 2.5 Tests (P1 acceptance)

- **Unit:** parser tests for each new keyword position. New file
  `src/parser/tests/access_mods.rs` with cases for binding, param,
  for-loop, and field positions.
- **Bulk:** add `tests/programs/access_mod_parse.jn` exercising each
  modifier — compile must succeed, runtime behavior should be
  observationally identical to today's (no semantic change).
- **Annotation:** `tests/programs/type_annotations.jn` with a
  `@resource` and `@atomic` struct declared but used in a way that
  doesn't exercise the new semantics. Compile must succeed.

### 2.6 Exit criteria (P1)

- 886/886 bulk tests green; 216/216 lib tests green.
- New parser tests green.
- Grep for `access_mod`: present in `ast`, `hir`, `parser`,
  `typer/lower`; absent (still TODO) in `typer/stmt/dispatch.rs`
  ownership decisions and codegen.
- One pass through `cargo doc --no-deps` for sanity.

---

## 3. Phase 2 — Escape analysis & tiered lowering

**Objective:** make `ref` and `mut` actually mean something. Implement
the local escape analysis and the three-tier lowering. Wire
container readers (`.get`, `.peek`, `.front`, `.back`) and `for`
loops to default-`ref` per the §4.3 inference table.

### 3.1 New module: escape analysis

- New file: [src/escape/mod.rs](src/escape/mod.rs).
- API: `pub fn analyze_fn(f: &hir::Fn, structs: &StructTable) -> EscapeInfo`.
  `EscapeInfo` maps each `DefId` whose RHS produced an alias to one
  of `Tier::T1 | Tier::T2 | Tier::T3 | Tier::AutoCopy`.
- Algorithm:
  1. Walk HIR forward. For each `Bind { access_mod: Some(Ref|Mut), .. }`
     or for each implicit alias (default-`ref` from inference), find
     all uses of the bound `DefId`.
  2. Mark T1 if every use is contained in a scope that does not
     outlive the *source's* scope. Use the existing scope-stack from
     `typer/lower/block.rs`.
  3. Escalate to T2 if any use is: returned, stored in a struct field,
     stored in a container, captured by a closure, or assigned to a
     binding with a longer lifetime.
  4. Escalate to T3 if any use is: passed to a `Channel.send`,
     sent to an `ActorRef`, captured by a `spawn`, OR the source
     type is `@atomic`. T3 is sticky (T3 → T3 always).
  5. For default-`ref` bindings, if the analysis cannot prove T1 and
     the user wrote no modifier, allow auto-promotion to T2 silently
     (T3 if cross-thread). Under `--strict-borrow`, escalation emits
     a warning.
- Uses (and may extend) the existing
  [src/ownership/mod.rs](src/ownership/mod.rs) — that module already
  tracks ownership states post-Perceus. Integrate; don't duplicate.

### 3.2 Typer integration

- [src/typer/stmt/dispatch.rs](src/typer/stmt/dispatch.rs) `lower_stmt`
  Bind arm:
  - Remove the `is_aliased_read_of_heap` heuristic (the new escape
    analysis subsumes it).
  - If `b.access_mod == Some(Ref)`: set `Ownership::Borrowed`,
    annotate the binding with `tier: T1` initially; the escape pass
    will rewrite.
  - If `b.access_mod == Some(Mut)`: set `Ownership::BorrowMut`,
    `tier: T1`.
  - If `b.access_mod == Some(Copy)`: set `Ownership::Owned` and
    flag the binding for codegen to insert a clone. (Already the
    current behavior for clonable types when the heuristic fires.)
  - If `b.access_mod == Some(Take)`: set `Ownership::Owned`, lower
    the RHS as a move-out (container slot tombstone or field move).
  - If `None`: apply the §4.3 default — for container reads on heap
    elements, set `Ownership::Borrowed` (T1 initial); for POD, keep
    `Owned` (copy is free).
- [src/typer/lower/iter.rs](src/typer/lower/iter.rs) — for loops:
  same logic for the iteration binder.
- [src/typer/lower/decl.rs](src/typer/lower/decl.rs) — function
  parameters: respect explicit `ref`/`mut`/`copy`/`take`.

### 3.3 Codegen for T1 / T2 / T3

- T1 (`ref` static borrow): codegen emits a raw pointer load. The
  binding's slot holds a `ptr` to the source's storage. No
  refcount, no drop at scope exit.
- T1 (`mut` exclusive borrow): same, raw pointer; writes go through
  the pointer.
- T2 (`ref`): lower to existing `Rc<T>` (HIR `Type::Rc(inner)`).
  Use existing Rc bump / drop / weakening infrastructure.
- T2 (`mut`): wrap inner in a single-threaded interior-mut cell. New
  runtime helper: `runtime/cell.c` providing
  `__jinn_cell_alloc`/`__jinn_cell_borrow_mut`/`__jinn_cell_release`.
  Type: `Rc<Cell<T>>` (new HIR type `Type::RcCell(inner)`).
- T3 (`ref`): lower to new `Type::Arc(inner)`. Runtime: atomic
  refcount via `__jinn_arc_clone`/`__jinn_arc_drop`. Reuse the
  atomic-RC paths already present for `Channel` / `ActorRef` —
  see [src/types.rs:160-185](src/types.rs#L160-L185) (`needs_atomic_rc`).
- T3 (`mut`): `Type::Arc(Type::Mutex(inner))`. Runtime mutex glue:
  `runtime/sync.c` `__jinn_mutex_lock`/`__jinn_mutex_unlock` (may
  reuse pthread mutex which is already linked).

### 3.4 Container readers default to `ref`

- [src/codegen/vec/core.rs](src/codegen/vec/core.rs) `vec_get_idx`:
  return a borrow when the caller is a default-`ref` binding (the
  typer drives this via the binding's `Ownership`/`tier`). Drop the
  clone call when the binding is borrowed.
- [src/codegen/map.rs](src/codegen/map.rs): same for `map.get`.
- [src/codegen/set.rs](src/codegen/set.rs): same for `set.peek`.
- Deque/PQ `front`/`back`/`peek_min`/`peek_max`: same.
- `for x in xs`: the loop emits a per-iteration borrow load,
  not a clone load. Mutating loops (`for mut x`) emit a writable
  pointer load.

### 3.5 Closure capture revisited

- [src/typer/expr/lambda.rs](src/typer/expr/lambda.rs) and the
  closure codegen path: an escaping closure that names a borrowed
  binding must capture by promoting the borrow to T2 (Rc) or T3
  (Arc per cross-thread rules). Non-escaping closures (`map`,
  `filter`, `for` body) can capture by raw pointer.

### 3.6 Tests (P2 acceptance)

- **New bulk tests** in `tests/programs/`:
  - `access_ref_borrow_basic.jn` — `x is ref v.get(0)` followed by a
    read; verify no clone in IR.
  - `access_ref_escape_to_rc.jn` — same but the binding is returned;
    verify Rc lowering.
  - `access_ref_cross_thread_to_arc.jn` — borrowed value sent on a
    channel; verify Arc lowering.
  - `access_mut_borrow_basic.jn` — `mut x is v.get(0); x.field = 1`;
    verify mutation visible in v.
  - `access_for_default_ref.jn` — `for x in vec_of_strings`; verify
    no per-iteration clone in IR (was a perf bug).
  - `access_take_field.jn` — `s is take obj.field`; verify field
    tombstone or last-use partial-move.
- **Existing bulk tests must all still pass**: 886/886. Behavior
  changes are limited to making borrows where there were silent
  clones; user-observable output is identical.
- **Benchmark gate:** run `python run_benchmarks.py`; the 6
  pre-existing regressions (struct_ops, spectral_norm, sim_for,
  tight_loop, store_perf, vec_grow) should *improve* under the new
  default-`ref` semantics. Document deltas in
  [benchmarks/results.json](benchmarks/results.json).

### 3.7 Exit criteria (P2)

- 886/886 bulk + 216/216 lib green.
- 6+ new bulk tests covering ref/mut/take/auto-escape.
- IR inspection: `vec.get` no longer emits `clone_value` calls
  unless the binding is `copy` or escapes.
- Benchmarks: all six pre-existing regressions resolved or improved
  toward baseline.

---

## 4. Phase 3 — `@resource` and `@atomic` semantics

**Objective:** lift the type-level annotations from "parsed and
inert" (Phase 1) to "fully enforced". Wire stdlib resource types.

### 4.1 `@resource` enforcement

- Typer: when a binding's RHS has a `@resource` type and the binding
  has `access_mod == Some(Copy)`, emit a hard error: "cannot copy
  value of resource type `T`; use `ref`, `mut`, or `take`".
- Typer: when a binding's RHS is a `@resource` and there's no
  modifier, the default is **move**, never the implicit-clone path,
  regardless of `is_value_clonable`.
- Codegen: if a `*drop` method is defined on the type, call it at
  scope exit of the owning binding. Lower as `__drop_<TypeName>`
  helper (mirrors existing struct-drop pattern from
  [src/codegen/drop/aggregates.rs](src/codegen/drop/aggregates.rs)).
  The helper signature is `void(ptr self)`; called from the same
  scope-drop emission point as today's struct drops.
- Cross-thread: a `@resource` that is not also `@atomic` cannot be
  sent on a channel or to an actor. Compile error: "resource type
  `T` is not `@atomic`; cannot send across threads".

### 4.2 `@atomic` enforcement

- Typer: `@atomic` types pin the `ref` tier to T3 regardless of
  local escape analysis. `mut` pins to T3 with `Mutex` wrapping.
- Codegen: lower the type's storage with an embedded atomic
  refcount header (`AtomicU64` at offset 0). Use C11 atomics in the
  runtime helpers. New runtime: `runtime/atomic_rc.c`.
- `@atomic` types may be both ref'd (alias) and moved (transfer).
  Move just hands over the Arc with a refcount of 1 (no atomic
  bump needed for transfer; only for sharing).
- `@weakable` (sub-option): allocates a separate weak-count slot
  alongside the strong count. Without `@weakable`, `weak ref` of
  this type is a compile error.

### 4.3 Stdlib audit

Annotate the following stdlib types in [std/*.jn](std/) and
[libjn/*.jn](libjn/):

| Type            | Annotation         | Rationale                                  |
|-----------------|--------------------|--------------------------------------------|
| `Socket`        | `@resource`        | OS FD; close-once                          |
| `File`          | `@resource`        | OS FD                                      |
| `Pipe`          | `@resource`        | OS FD                                      |
| `Process`       | `@resource`        | OS PID                                     |
| `Mutex`         | `@resource @atomic`| pthread_mutex; shared use across threads   |
| `RwLock`        | `@resource @atomic`| same                                       |
| `Channel`       | `@resource @atomic`| already cross-thread; ARC already         |
| `ActorRef`      | `@atomic`          | already ARC                                |
| `Coroutine`     | `@resource`        | linear; cannot run twice                   |
| `Generator`     | `@resource`        | linear                                     |
| `DbConnection`  | `@resource`        | (if/when present)                          |
| `TlsContext`    | `@resource @atomic`| shared session, must close                 |

Each annotation may require adding a `*drop` method to the type if
one wasn't there before (most resource types already have explicit
`close`/`free` methods — call those from `*drop`).

### 4.4 Tests (P3 acceptance)

- `tests/programs/resource_no_copy.jn` — must fail compilation with
  the expected error message.
- `tests/programs/resource_drop_runs.jn` — define a `@resource`
  type with `*drop` that increments a global counter; verify
  counter at end-of-scope.
- `tests/programs/atomic_arc_lowering.jn` — `@atomic` type shared
  across two coroutines; runtime check that refcount goes up/down
  correctly via atomic ops (use the `--debug-refcount` build mode).
- `tests/programs/resource_cross_thread_rejected.jn` — try to
  send a non-`@atomic` `@resource` on a channel; must fail.
- All existing bulk tests still pass; any stdlib resource types
  may need callers updated to use `ref` instead of moving.

### 4.5 Exit criteria (P3)

- All annotations from §4.3 applied; build green; bulk green.
- New negative tests produce the documented error messages.
- Refcount audit on `@atomic` types: valgrind clean, no leaks.

---

## 5. Phase 4 — Field-access auto-copy and Perceus partial-move

**Objective:** make `x.field` do the right thing per §4.6. Promote
the Perceus pass to track last-use of struct values so partial-moves
out of fields are zero-cost where possible.

### 5.1 Auto-copy on field-access escape

- [src/typer/expr/field.rs](src/typer/expr/field.rs) (or equivalent):
  when typing `x.field` where `field` has a heap type:
  1. The expression's *immediate* type is a T1 borrow (raw pointer to
     the field's storage in `x`).
  2. The escape analysis from Phase 2 examines the parent expression
     /  binding. If the borrow escapes (assigned to a longer-lived
     binding, returned, sent, stored), the typer inserts an implicit
     `copy` at the field-access boundary — a synthetic
     `clone_value(field_load, field_ty)`.
  3. If the user wrote `ref`/`mut`/`take` explicitly, honor that and
     skip the auto-copy.

### 5.2 Perceus partial-move

- [src/perceus/mir_perceus.rs](src/perceus/mir_perceus.rs):
  - For a `FieldGet`/`FieldRead` whose result is the `take` target of
    a `Bind`, mark the parent struct's `DefId` as "field K consumed".
  - If the struct is itself dropped at the end of the current MIR
    function and no use of field K survives the `take`, the drop
    helper for that struct must skip field K's drop slot.
  - Implementation: emit a per-call drop helper variant where the
    consumed field is skipped, OR pass a bitmask flag to a
    generic drop helper. Bitmask is simpler when only a small
    number of types have partial-moves; per-variant is faster
    runtime but more codegen churn.
  - Start with the per-variant approach for the 1–2 stdlib types
    that need it (`Vec.pop` and friends already do this informally);
    generalize only if benchmarks demand it.
- If Perceus cannot prove last-use, fall back to the auto-copy from
  §5.1. Never emit unsound code.

### 5.3 Tests (P4 acceptance)

- `tests/programs/field_auto_copy.jn` — `s is x.field` where `s` is
  returned; verify (via IR inspection) a clone was inserted.
- `tests/programs/field_short_lived_borrow.jn` — `if x.field == 0 …`;
  verify no clone (raw load).
- `tests/programs/field_take_partial_move.jn` — `s is take x.field`
  where `x` is also consumed; verify no clone and no double-drop.
- `tests/programs/field_take_persistent_parent.jn` — `s is take
  x.field` where `x` is *not* consumed; verify either the bitmask
  drop-skip works OR the typer rejects with a clear error
  ("partial move out of `x.field` while `x` is still live; use
  `copy` or restructure").

### 5.4 Exit criteria (P4)

- 886+ bulk green; new tests green.
- Valgrind: no leaks, no double-frees on any field-take test.
- IR check: at least one stdlib pop-shaped function (`Vec.pop`,
  `Deque.pop_front`) compiles without a clone in the hot path.

---

## 6. Phase 5 — Stores & smart rows

**Objective:** persisted-state correctness. Make store-row mutation
go through the store, not through user-owned copies.

### 6.1 `Row<T>` type

- New HIR / surface type `Row<T>` representing a handle to a
  store-managed row. Two constructors:
  - `store.where(…).first()` → `Row<T>` (mutable, write-through).
  - `store.where(…).snapshot()` → `T` (value snapshot; mutation
    does nothing).
- `Row<T>` field access reads through the store's backing storage
  (one indirection per read; behind the scenes, the store may keep
  a hot cache or a row index).
- `Row<T>::update(f: fn(&mut T))` opens a write transaction on the
  store, applies `f` to a mutable view, commits.
- `Row<T>` is `@resource` (cannot be copied; must be released or
  moved into an `update` call).
- Direct field mutation `row.field = …` is parsed as a sugar for
  `row.update { it.field = … }`.

### 6.2 Stdlib store API changes

- [src/typer/lower/store.rs](src/typer/lower/store.rs) (or wherever
  store-query lowering lives): change `query(…).first()` return
  type to `Row<T>`.
- Add `snapshot()` to all `Query` types.
- Audit existing store tests; any code that relied on the old
  "first() returns owned T" semantics is updated by either calling
  `.snapshot()` (read-only) or `.update { … }` (mutate).

### 6.3 Tests (P5 acceptance)

- `tests/programs/store_row_write_through.jn` — two coroutines each
  mutate the same row via `update`; runtime invariant: both writes
  visible in the persisted state, no lost update.
- `tests/programs/store_row_mutation_via_field.jn` — `row.email =
  "x"` followed by re-query; verify the persisted state.
- `tests/programs/store_snapshot_is_copy.jn` — `u is
  q.snapshot()`; mutating `u.email` does NOT update the store.
- Existing store bulk tests adapted to new API; all must pass.

### 6.4 Exit criteria (P5)

- All bulk tests pass (count may exceed 886+ due to new tests).
- The `apps/bank_ledger` app (which uses stores heavily) compiles
  unchanged or with minimal `.snapshot()` annotations.
- No remaining bare-`T`-from-mutable-query call sites in the
  stdlib or examples.

---

## 7. Phase 6 — Cleanup, deprecation, docs

**Objective:** remove dead paths, finalize docs, ship migration
tooling.

### 7.1 Code cleanup

- Delete `Typer::is_aliased_read_of_heap` (
  [src/typer/stmt/dispatch.rs](src/typer/stmt/dispatch.rs#L16))
  and all call sites. The escape analysis from Phase 2 subsumes
  it.
- Audit `Compiler::clone_value` call sites:
  ([src/codegen/clone/mod.rs](src/codegen/clone/mod.rs)):
  keep only paths reachable from `copy`-modifier bindings, the
  auto-copy boundary in field access, and the auto-promote
  fallback. Remove "clone on get" paths that are now borrows.
- Audit `Ownership::Borrowed` usages — confirm the new tier field
  is the sole source of truth; remove redundant checks.

### 7.2 Deprecate surface `Rc<T>` / `Weak<T>`

- Lexer/parser: continue to accept `Rc T` and `Weak T` syntax for
  backward compatibility; emit a deprecation warning suggesting
  `ref T` / `weak ref T`.
- Migration codemod `scripts/migrate-rc-to-ref.py` (or a `jinn
  migrate` subcommand in the compiler driver): mechanically
  rewrites `Rc T` → `ref T`, `Weak T` → `weak ref T` in `.jn`
  source.

### 7.3 Docs

- Update [jinn.md](jinn.md) §"Principles" point 3 (Borrowing is
  free) and point 4 (Sharing is inferred) to reference the new
  surface. Add a new short section "Access modifiers" linking to
  this document.
- Update [docs/memory-model.md](docs/memory-model.md) (if it
  doesn't already match) with the three tiers and the new
  annotations.
- Write a user-facing migration guide:
  `docs/access-semantics-migration.md`. Three audiences:
  1. Code that compiled and just gets faster — no action.
  2. Code that hit the `.get()`-escape path — guide on `copy` vs
     `ref` vs restructure.
  3. Code that mutated store rows directly — guide on
     `.update { … }` vs `.snapshot()`.
- Update [JINN.md](JINN.md) (project status) to record the
  language version bump.

### 7.4 Exit criteria (P6)

- `is_aliased_read_of_heap` no longer exists in the tree.
- `cargo doc --no-deps` green; rustdoc has no dead links.
- The migration guide compiles all three of its example transcripts.
- All apps in [apps/](apps/) build and run.
- All examples in [examples/](examples/) build and run.
- Tag a language version: `jinn 0.6` (or whatever the next major
  bump is).

---

## 8. Risks and contingencies

| Risk                                                | Mitigation                                                 |
|-----------------------------------------------------|------------------------------------------------------------|
| Escape analysis is wrong → unsound borrows          | Default to auto-promote (T2/T3) on any uncertainty.        |
| `@atomic` reveals latent thread-safety bugs in stdlib | Run thread sanitizer (`-fsanitize=thread`) after P3.    |
| Codegen churn breaks benchmarks                     | Run benchmark suite at the end of every phase, not just P2.|
| Surface ambiguity with `take`/`copy` as method names | Modifier position is disambiguated by parser look-ahead; tests cover both. |
| User code that relied on silent clones              | Auto-promotion to T2 keeps it working at a small cost; migration guide documents the explicit alternatives. |
| Per-field drop bitmask balloons struct sizes        | Use per-variant drop helpers (compile-time, no runtime cost) until benchmarks force a different choice. |
| LLVM IR for tiered lowering bloats binary size      | Measure after P2; if >10% increase, factor common bodies into runtime helpers. |

---

## 9. Out of scope

These belong to follow-up sprints, **not this one**:

- Full Rust-style lifetime variables in signatures.
- Async/await syntax for the new ref tiers.
- A `Send`/`Sync` trait system (the cross-thread check is
  type-driven today; trait-driven later if needed).
- COW-aware `mut` (mutating a Tier-2 `mut` with refcount>1
  clones the buffer): worth doing, but a separate sprint with
  its own benchmark gates.
- Region inference (Tofte–Talpin–style). The escape analysis here
  is intentionally local-only.
- IDE integration (`vscode-jinn`) for surfacing tier decisions.
  Likely a tooltip in a later cycle.

---

## 10. Tracking & sign-off

Each phase is a single git tag and a short post-phase note in
`/memories/repo/access_semantics_sprint_log.md` summarising:
- What landed.
- Test counts (lib, bulk, new).
- Benchmark deltas (focus on the six existing regressions).
- Any deviation from this plan and why.

When all six phases are complete and §0 invariants hold for the
combined work, this sprint is closed and the language version is
bumped per §7.3.
