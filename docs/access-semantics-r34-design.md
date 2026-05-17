# R3.4 Design — Promotion Lowering for T2 / T3 Tiers

Status: **design**, not yet implemented.
Owner: R3.4 sprint (follows commit `fe1eb7e`).
Spec section: [docs/access-semantics-sprint.md §3.3 / §3.5](access-semantics-sprint.md)
Continues:  [docs/access-semantics-remediation.md](access-semantics-remediation.md)

## 1. Problem statement

The escape analyzer (`src/escape/mod.rs`) already classifies every
T1/T2/T3 alias binding. R3.3 lowered the **T1** case — Borrowed /
BorrowMut bindings become raw pointer aliases with the scope-exit
Drop suppressed and the MIR auto-clone elided.

R3.4 must lower the **T2** and **T3** cases:

| Escape tier | Mod | Resulting layout                       | Ownership tag (already present) |
|-------------|-----|----------------------------------------|----------------------------------|
| T2          | ref | `Rc<T>` (existing infrastructure)      | `Ownership::Rc`                  |
| T2          | mut | `Rc<Cell<T>>`                          | `Ownership::RcMut`               |
| T3          | ref | `Arc<T>`                               | `Ownership::Arc`                 |
| T3          | mut | `Arc<Mutex<T>>`                        | `Ownership::ArcMut`              |

The `Ownership` enum already encodes these. The missing pieces are:

1. **Type-level encoding** of the boxed layout (so function
   signatures, struct fields, returns survive the promotion).
2. **Runtime support** for `Cell`, `Arc`, and `Mutex` wrappers.
3. **Typer promotion pass** that rewrites a binding's *type* (not
   just its ownership tag) based on the escape tier.
4. **Codegen** for the new layouts.

## 2. Architectural decision — new Type variants vs. ownership-driven layout

### Option A — New `Type` variants

Extend `src/types.rs`:

```rust
pub enum Type {
    // ...
    RcCell(Box<Type>),   // Rc<Cell<T>>, single-threaded interior mut
    Arc(Box<Type>),      // Arc<T>, atomic-refcounted shared
    Mutex(Box<Type>),    // Mutex<T>, exclusive within Arc
}
```

Pros:
- Clean separation; the type system enforces correct usage at every
  call site (no "owned-but-secretly-boxed" footgun).
- `needs_atomic_rc` extends trivially.
- Matches the spec wording.

Cons:
- Touches 27 source files that pattern-match `Type` exhaustively
  (typer, MIR, codegen, formatter, mono, unify, perceus, incr,
  interface, undef).
- Increases the type-equality / unify surface (need new rules:
  `Rc<T>` and `RcCell<T>` are distinct; `Arc<T>` and `Rc<T>` are
  distinct).

### Option B — Ownership-driven layout

Keep `Type::Rc(T)` as the only refcounted variant; let the binding's
`Ownership` (RcMut / Arc / ArcMut) drive the *layout* at codegen.

Pros:
- Smaller diff; reuses existing `Type::Rc` machinery.

Cons:
- The "type" no longer fully describes the layout. Callees receiving
  a `Type::Rc<String>` value can't tell whether it's plain `Rc` or
  `Arc` — runtime ABI mismatch.
- Cross-function flow becomes load-bearing on the binding's
  `Ownership`, which today only travels with the *binding* not the
  *value*.
- Breaks the spec's stated mapping.

### **Decision: Option A.**

The cost is paid once; the type-system enforcement pays back forever.
This is consistent with the standing directive's "fix the architecture,
not the symptom".

## 3. Implementation plan — five commits

### Commit R3.4.a — additive `Type` variants

Touch list (additive only — no behavior change yet):

1. `src/types.rs`:
   - Add `RcCell(Box<Type>)`, `Arc(Box<Type>)`, `Mutex(Box<Type>)`.
   - Extend `default_ownership`:
     - `RcCell(_) -> Ownership::RcMut`
     - `Arc(_)    -> Ownership::Arc`
     - `Mutex(_)  -> Ownership::ArcMut`  *(only meaningful inside `Arc`)*
   - Extend `needs_atomic_rc`: `Arc | Mutex` → true; `RcCell` →
     recurse on inner.
   - Extend `Display`: `RcCell<T>`, `Arc<T>`, `Mutex<T>`.

2. Every exhaustive `match` on `Type` (file list at end of
   §3 below) gets a fall-through:
   ```rust
   Type::RcCell(inner) | Type::Arc(inner) | Type::Mutex(inner) => {
       // delegate to inner type for sizing/layout for now;
       // overridden in R3.4.c by codegen.
       self.sizeof(inner)
   }
   ```
   Until R3.4.c lands, treat the new variants as transparent
   wrappers so existing code keeps compiling and the test gate
   stays 921/921.

3. `src/typer/unify/mod.rs`: new wrappers don't unify with their
   inner type. Mirror existing `Type::Rc` rules.

4. `src/typer/mod.rs` / `src/typer/mono.rs`: variance and
   substitution rules — wrappers are invariant in `T` (same as
   `Type::Rc`).

5. **Gate**: bulk 921/921 unchanged.

### Commit R3.4.b — runtime layout & C helpers

New runtime files:

- `runtime/cell.c` — `__jinn_cell_alloc(size, align)`,
  `__jinn_cell_load(ptr, dst_size)`, `__jinn_cell_store(ptr, src,
  size)`. Layout: `{header; T payload;}` where the header is the
  existing `RcHeader` (refcount + drop fn ptr).
  *Single-threaded — no atomics needed; rely on Jinn's actor
  isolation invariant.*

- `runtime/atomic_rc.c` — `__jinn_arc_alloc(size, drop_fn)`,
  `__jinn_arc_clone(ptr)`, `__jinn_arc_drop(ptr)`. Reuse the
  atomic-RC paths already in `runtime/channel.c` / `runtime/actor.c`.
  Factor out the common header. **Audit**: confirm those modules can
  link against the new shared helpers without ABI break.

- `runtime/sync.c` — `__jinn_mutex_init(ptr)`,
  `__jinn_mutex_lock(ptr)`, `__jinn_mutex_unlock(ptr)`,
  `__jinn_mutex_destroy(ptr)`. Wrap pthread mutex (already linked
  by `runtime/pthread_glue.c`).

- `build.rs`: add new `.c` files to the compile list. Confirm with
  `cargo build --release` clean.

- **Gate**: bulk 921/921 still 921/921 (these helpers aren't called
  yet).

### Commit R3.4.c — codegen for new Type variants

Files (per the 27-file list):

- `src/codegen/types.rs`: LLVM layout for `RcCell<T>` (= `Rc<T>`
  layout — same `RcHeader` + payload), `Arc<T>` (= `Rc<T>` layout
  but with the atomic refcount header), `Mutex<T>`
  (= `pthread_mutex_t + T`).
- `src/codegen/clone/mod.rs`:
  - `RcCell` → bump rc (same as Rc).
  - `Arc` → atomic increment.
  - `Mutex` → not clonable directly (must be wrapped in Arc).
- `src/codegen/drop/mod.rs`:
  - `RcCell` → rc decrement; on zero, run inner drop.
  - `Arc` → atomic decrement; on zero, run inner drop.
  - `Mutex` → `pthread_mutex_destroy` then drop inner.
- `src/codegen/expr/access.rs`:
  - Read of `RcMut` binding (whose type is `Type::RcCell(T)`):
    `__jinn_cell_load`. Write: `__jinn_cell_store`.
  - Read of `Arc(T)`: deref RC payload.
  - Read of `ArcMut` binding (type `Arc<Mutex<T>>`):
    `__jinn_mutex_lock`, copy payload, `__jinn_mutex_unlock`. Write:
    lock → store → unlock.
- `src/codegen/mir_codegen/emit_inst/aggregates.rs`,
  `src/codegen/mir_codegen/emit_inst/collections.rs`,
  `src/codegen/support/module.rs`,
  `src/codegen/decl.rs`,
  `src/codegen/drop/aggregates.rs`,
  `src/codegen/builtins/dispatch_math.rs`:
  extend each `match` arm for `Type::Rc(_)` to also handle the new
  variants (usually delegating to the same path).

- `src/perceus/mir_perceus.rs` / `src/perceus/mod.rs`: Arc / RcCell
  / Mutex follow same dup/drop scheduling as Rc. The mutex acquire/
  release is *not* a clone — codegen handles it inline.

- `src/incr.rs`, `src/interface.rs`, `src/driver/undef.rs`: hash /
  serialize the new variants (mirror Rc handling).

- **Gate**: bulk 921/921 still 921/921 (no binding gets a new type
  until R3.4.d).

### Commit R3.4.d — promotion lowering pass (the real work)

New module: `src/typer/promote.rs` (or extend
`src/typer/lower/block.rs`). Runs **after** escape analysis,
**before** MIR lowering. Algorithm:

For each `Bind` in HIR:
1. Read its escape tier from `escape::Tier` (already attached).
2. If `Tier::T2`:
   - If `Ownership` ∈ {Borrowed, Owned, Rc}: keep type as
     `Rc<T>`, set ownership = `Rc`.
   - If `Ownership` ∈ {BorrowMut, RcMut}: rewrite type to
     `RcCell<T>`, set ownership = `RcMut`. Insert `__jinn_cell_alloc`
     at the binding site; rewrite the RHS expression to a
     `RcCellWrap(...)` IR node.
3. If `Tier::T3`:
   - If `Ownership` ∈ {Borrowed, Owned, Rc, Arc}: type → `Arc<T>`,
     ownership = `Arc`. Insert `__jinn_arc_alloc` wrap.
   - If `Ownership` ∈ {BorrowMut, RcMut, ArcMut}: type →
     `Arc<Mutex<T>>`, ownership = `ArcMut`. Insert
     `__jinn_arc_alloc(__jinn_mutex_init(...))` wrap.

For each `Use` of a promoted binding:
- Insert a `Deref` / `MutexLock` HIR node (or, more cleanly, leave
  the deref to codegen by tagging the use).

Cross-function flow:
- A promoted binding passed to a function call requires that the
  function parameter be re-typed to the promoted type. **Decision
  needed**: either (a) monomorphize functions per parameter
  promotion (expensive), or (b) require the user to annotate the
  parameter as `ref` / `mut` (matches the spec's `@resource` /
  `@atomic` story). Pick (b) — escape analysis stays *local* to
  the function; cross-function promotion is by signature.

Tests (new under `tests/programs/`):
- `access_ref_escape_to_rc.jn` — bind, return, verify Rc lowering.
- `access_mut_escape_to_rc_cell.jn` — bind mut, return, verify
  RcCell.
- `access_ref_cross_thread_to_arc.jn` — bind, send on channel,
  verify Arc.
- `access_mut_cross_thread_to_arc_mutex.jn` — bind mut, send on
  channel, verify Arc<Mutex>.

- **Gate**: bulk 921/921 + 4 new tests = 925/925.

### Commit R3.4.e — closure capture (spec §3.5)

`src/typer/expr/lambda.rs` + closure codegen path. An escaping
closure naming a borrowed binding promotes the borrow to T2 (Rc)
or T3 (Arc) per the same rules. Non-escaping closures (`map`,
`filter`, `for` body) capture by raw pointer (already work post-R3.3).

- **Gate**: bulk 925/925; add 2 closure-escape tests → 927/927.

## 4. Cross-cutting concerns

### 4.1 ABI stability

Promoted types must be ABI-distinct from their unboxed forms.
Functions exposed to FFI (`@extern`) must NOT have promoted
parameters — emit a typer error if escape analysis would promote
a parameter of an `@extern` function.

### 4.2 Interaction with `@resource` / `@atomic`

`@atomic T` is the user-driven counterpart of T3 promotion. A
binding to an `@atomic` value is *already* `Arc<T>` (per spec
§4.2). The escape pass should short-circuit: don't double-wrap.

### 4.3 Pretty-printing & error messages

Type errors involving promoted types must point to the *original*
binding, not the synthesized `Arc<Mutex<T>>` form. Keep a
`promotion_source: Option<Span>` on the new Type variants? Or
better: error reporter looks up the binding by span and reports
the *user-written* type, with a note "(promoted to Arc<Mutex<T>>
because crosses thread)".

## 5. Files touched (master list — for the R3.4 sprint)

```
src/types.rs                                            [R3.4.a]
src/typer/unify/mod.rs                                  [R3.4.a]
src/typer/mod.rs                                        [R3.4.a]
src/typer/mono.rs                                       [R3.4.a]
src/typer/builtins.rs                                   [R3.4.a]
src/typer/expr/mod.rs                                   [R3.4.a]
src/typer/expr/typeargs.rs                              [R3.4.a]
src/typer/lower/block.rs                                [R3.4.a]
src/typer/lower/resolve.rs                              [R3.4.a]
src/typer/stmt/dispatch.rs                              [R3.4.a]
src/typer/unify/resolve.rs                              [R3.4.a]
src/typer/tests.rs                                      [R3.4.a]

runtime/cell.c                                          [R3.4.b]  NEW
runtime/atomic_rc.c                                     [R3.4.b]  NEW
runtime/sync.c                                          [R3.4.b]  NEW
build.rs                                                [R3.4.b]

src/codegen/types.rs                                    [R3.4.c]
src/codegen/clone/mod.rs                                [R3.4.c]
src/codegen/drop/mod.rs                                 [R3.4.c]
src/codegen/drop/aggregates.rs                          [R3.4.c]
src/codegen/decl.rs                                     [R3.4.c]
src/codegen/expr/access.rs                              [R3.4.c]
src/codegen/builtins/dispatch_math.rs                   [R3.4.c]
src/codegen/mir_codegen/emit_inst/aggregates.rs         [R3.4.c]
src/codegen/mir_codegen/emit_inst/collections.rs        [R3.4.c]
src/codegen/support/module.rs                           [R3.4.c]
src/perceus/mir_perceus.rs                              [R3.4.c]
src/perceus/mod.rs                                      [R3.4.c]
src/incr.rs                                             [R3.4.c]
src/interface.rs                                        [R3.4.c]
src/driver/undef.rs                                     [R3.4.c]

src/typer/promote.rs                                    [R3.4.d]  NEW
src/escape/mod.rs                                       [R3.4.d]  (expose tier)
tests/programs/access_ref_escape_to_rc.jn               [R3.4.d]  NEW
tests/programs/access_mut_escape_to_rc_cell.jn          [R3.4.d]  NEW
tests/programs/access_ref_cross_thread_to_arc.jn        [R3.4.d]  NEW
tests/programs/access_mut_cross_thread_to_arc_mutex.jn  [R3.4.d]  NEW

src/typer/expr/lambda.rs                                [R3.4.e]
src/codegen/closure/...                                 [R3.4.e]
tests/programs/access_closure_escape_to_rc.jn           [R3.4.e]  NEW
tests/programs/access_closure_cross_thread_to_arc.jn    [R3.4.e]  NEW
```

## 6. Risks & open questions

1. **Cross-function promotion** — design says "by signature only".
   Confirm with the user before R3.4.d.
2. **Promotion of `Vec<String>` etc.** — does `Arc<Vec<String>>`
   need a custom drop that drops each String? The existing
   `Type::Rc(Vec<String>)` already handles this via the inner
   drop fn pointer — reuse the mechanism.
3. **Perceus interaction** — promoted bindings have RC semantics,
   so Perceus must dup/drop. Confirm the existing dup/drop scheduling
   handles a binding whose type changes mid-pipeline (it does —
   Perceus runs after promotion).
4. **Mutex poisoning** — pthread mutexes don't poison. If a Jinn
   thread aborts while holding the lock, the mutex stays locked.
   Acceptable for v1; revisit when panic recovery lands.

## 7. Done criteria

- All commits R3.4.a–e land green.
- Bulk tests 927/927.
- IR-snapshot tests confirm:
  - T2 ref binding produces `__jinn_rc_alloc` + `__jinn_rc_drop`.
  - T2 mut binding produces `__jinn_cell_alloc` + `__jinn_cell_*`.
  - T3 ref binding produces `__jinn_arc_alloc` + `__jinn_arc_drop`.
  - T3 mut binding produces `__jinn_arc_alloc` +
    `__jinn_mutex_lock`/`unlock`.
- After R3.4.e, R4 unblocks: `is_aliased_read_of_heap` can be
  deleted in `src/typer/stmt/dispatch.rs:24-53`, and the docs in
  `jinn.md`, `JINN.md`, `docs/access-semantics.md` updated to drop
  the legacy escape hatch.
