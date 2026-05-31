# Jinn Access Semantics

> Authoritative reference for how Jinn binds, passes, shares, mutates, and
> destroys values. Supersedes the prior `access-semantics*.md`,
> `architecture.md`, and `memory-model.md` documents (all deleted).

## 1. Design pillars

Jinn is **value-semantics first**. The default user mental model is:

> "Every binding is a value. Assignment, parameter passing, and field reads
> behave _as if_ the value were copied."

The compiler then proves which copies are unobservable and turns them into
borrows, in-place mutation, or zero-cost moves. There is **no `Rc`, no
`Arc`, no `Box`, and     no `&` lifetime syntax** in the surface language. The
runtime has **no garbage collector**. Heap management is explicit and
deterministic: every owned value has exactly one drop site, decided by
escape analysis and Perceus-style use counting.

Three rules govern everything below:

1. **Single ownership at any moment.** Every heap-backed value has exactly
   one owner. Aliases are read-only views with a known, statically bounded
   lifetime.
2. **No hidden refcount in the fast path.** Refcounted handles do not exist
   in the IR. Cross-thread sharing uses purpose-built atomic-refcounted
   primitives (`Channel`, `ActorRef`) — not a generic `Arc<T>`.
3. **The user writes intent; the compiler picks the tier.** When the
   compiler can pick an aliasing strategy that preserves value semantics, it
   does so silently. The user only writes a modifier (§2) when they want to
   override the default.

## 2. Surface syntax — access modifiers

A binding, parameter, field, or for-loop binder may be prefixed with at
most one access modifier:

| Modifier  | Meaning                                                                   |
| --------- | ------------------------------------------------------------------------- |
| _(none)_  | Compiler picks the ownership tier from use (see §4).                      |
| `copy`    | Deep clone at the boundary; consumer owns an independent value.           |
| `take`    | Move out of the source (or remove from a container slot). Source dies.    |
| `const`   | Rebind ban: `x` cannot appear on the LHS of `is` again in its scope.      |

`copy` and `take` are **about the data flow**. `const` is **about the name
binding** — orthogonal to ownership, equivalent to "this identifier is
single-assignment in its scope". The value itself still moves/borrows/copies
per the ordinary rules.

`ref` and `mut` are **not** surface keywords. The compiler chooses shared
vs. exclusive aliasing automatically based on usage.

Source of truth: [src/ast.rs](src/ast.rs#L382) (`enum AccessMod`).

## 3. Type annotations

Two `@`-prefixed annotations affect access semantics:

- **`@resource`** — declares a linear type. Values may never be implicitly
  duplicated, may never cross thread boundaries (channels, actor sends,
  spawn captures), and have their `*drop` method invoked automatically at
  scope exit. Attempting to `copy` a `@resource` value is a compile error.
  Examples: `File` (in `libjn/stdio.jn`), `Socket`, any hand-rolled
  RAII handle.
- **`@align(N)` / `@packed` / `@strict`** — layout/representation. Not
  access-semantics, listed only for completeness.

Source of truth: [src/ast.rs](src/ast.rs#L411) (`struct LayoutAttrs`),
[src/parser/decl/types.rs](src/parser/decl/types.rs#L48)
(`parse_layout_attrs`).

> The historical `@atomic` annotation has been removed. Cross-thread
> sharing is provided by the purpose-built `Channel<T>` and `ActorRef<T>`
> primitives, both of which carry their own atomic refcount in the
> runtime. User code does not opt structs into "shared mutability"; it
> sends values through a channel or to an actor.

## 4. Ownership tiers (HIR)

After type checking, every binding carries one of four ownership tiers in
the HIR. These tiers are an internal IR concept — they are never written
by the user.

| Tier         | Lowering                                            | When chosen                                                                          |
| ------------ | --------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `Owned`      | Sole owner. Responsible for `*drop` at scope exit.  | Default for fresh values, results of constructors, results of `take`.                |
| `Borrowed`   | Raw pointer alias. No refcount, no drop.            | Read-only aliases of heap values whose live range is dominated by the source.        |
| `BorrowMut`  | Raw mut pointer alias. No refcount, no drop.        | Exclusive mutable alias for in-place updates (e.g. `vec.push` on the owner).         |
| `Raw`        | Raw user-managed pointer (`Type::Ptr`).             | FFI pointer types.                                                                   |

Source of truth: [src/hir/mod.rs](src/hir/mod.rs#L15) (`enum Ownership`).

The selection logic lives in
[src/typer/mod.rs](src/typer/mod.rs) at `ownership_with_mod` and
`param_ownership_with_mod`, with statement-level overrides in
[src/typer/stmt/dispatch.rs](src/typer/stmt/dispatch.rs).

### 4.1 Default rules

- **POD-shaped types** (numerics, bool, small tuples of POD, pointers) are
  always `Owned`. A bit-copy _is_ an independent value.
- **Heap-leaf containers** (`String`, `Vec<T>`, `Map<K,V>`, `Coroutine`,
  `Generator`) default to **`Borrowed`** for unannotated function
  parameters (see `type_param_default_borrows` in
  [src/typer/mod.rs](src/typer/mod.rs)). Rationale: callers should not lose
  ownership just by passing a vector to a helper.
- **User-defined heap structs/enums** default to `Borrowed` for parameters
  iff they need drop, otherwise `Owned`.
- **Container reads** (`v.get(i)`, `m[k]`) that do not deep-clone return a
  `Borrowed` view aliased to the container slot. The container must
  outlive the binding.
- **Explicit modifiers always win.** `copy` → `Owned` (with a clone),
  `take` → `Owned` (with a move), `const` → does not change the tier.

### 4.2 Drop discipline

- Only `Owned` bindings get drop glue. `Borrowed`/`BorrowMut`/`Raw`
  bindings are skipped at scope exit.
- A `@resource` value's `*drop` method is invoked automatically when its
  `Owned` binding goes out of scope. Explicit `.shut()` calls are
  idempotent (the handle is zeroed after first close).
- `take` ends the source binding's scope early — the source must not be
  read after the move. The typer enforces this with a per-scope **move
  tombstone**: an explicit `take` (a `take` binding, or passing a value to
  a `take` parameter) tombstones the source for the rest of the scope, and
  any later read is a compile error. Tombstones are tracked at two
  granularities — whole variables (`moved_vars`) and individual struct
  fields (`moved_fields`) — and both flow through `if`/`match`/loop
  branches via the shared snapshot/restore/union dataflow. Reassigning the
  variable (or field) clears its tombstone and makes it live again.

## 5. Cross-thread access

Threads in Jinn communicate through two primitives:

- **`Channel<T>`** — bounded MPMC FIFO. `send`/`recv` perform a
  move-with-transfer-of-ownership. The channel itself is reference-counted
  atomically by the runtime.
- **`ActorRef<T>`** — typed handle for sending messages to a spawned
  actor. Same atomic-refcount story.

Both are atomic by construction; user code never wraps a struct in
"shared" mode. The cross-thread safety rule is:

> A `@resource` value may not cross a thread boundary.

This is enforced in [src/typer/mod.rs](src/typer/mod.rs) at
`enforce_cross_thread_safe`, called from:

- channel `send` lowering,
- actor handler argument typing,
- actor `spawn` init typing.

Diagnostic:

```
foo.jn:7:5: resource type `Handle` cannot cross thread boundaries (channel send)
```

The escape analyser ([src/escape/mod.rs](src/escape/mod.rs)) classifies
each binding into one of three tiers (`T1`/`T2`/`T3`) based on whether it
escapes its source scope, its thread, or neither. The MIR/codegen passes
consume this information to decide where to insert drops and where in-place
mutation is safe.

## 6. Examples

### 6.1 Copy-on-pass for a vector

```jinn
*push_one(v as Vec of i64)
    v.push(1)        # ok: parameter is BorrowMut by default

*main()
    xs is [1, 2, 3]
    push_one(xs)
    log(xs.len())    # 4 — caller still owns xs
```

The parameter defaults to a borrow; the caller's vector is mutated
in-place. No reference counting, no copy.

### 6.2 Linear resource with `*drop`

```jinn
type File @resource
    handle as i64

    *drop
        if self.handle != 0
            *close(self.handle)
            self.handle is 0

*write_then_die()
    f is File(handle is *open("x.txt"))
    f.write("hi")
    # f goes out of scope → *drop runs → fd closed deterministically
```

### 6.3 Explicit `take` to give up ownership

The access modifier sits in **type position** (`name as take Type`), so the
move is driven by the callee's parameter, not by a marker at the call site:

```jinn
*consume(s as take String)
    log(s.len())

*main()
    name is "alice"
    consume(name)
    # log(name.len()) here is a compile error: `name` was moved out
    # by an earlier `take`; reassign `name` before reading it
```

The same tombstone applies to a `take` binding (`b is take a`) and to a
moved-out struct field (`b is take a.items`, after which reading `a.items`
is rejected until it is reassigned).

### 6.4 Cross-thread (allowed)

```jinn
*main()
    ch is channel<i32>(8)
    spawn worker(ch)
    send ch, 42
```

A plain `i32` is POD, copies freely, and crosses threads without issue.

### 6.5 Cross-thread (rejected)

```jinn
type Handle @resource
    fd as i32

*main()
    ch is channel<Handle>(1)
    h is Handle(fd is 3)
    send ch, h        # error: resource type `Handle` cannot cross thread
                      #        boundaries (channel send)
```

## 7. `atomic` keyword (variable binding) vs `atomic_*` builtins

Two unrelated uses of the word "atomic" remain in the language. They are
**not** about access tiers, and they are not affected by anything above.

- **`atomic x is value` variable binding** — declares that loads and stores
  of `x` are sequentially consistent atomic operations. Surface keyword
  reserved in the lexer. Implementation lives in
  [src/parser/stmt/dispatch.rs](src/parser/stmt/dispatch.rs) and the typer.
- **`atomic_*` FFI builtins** — `atomic_load`, `atomic_store`,
  `atomic_compare_exchange`, etc. These are direct intrinsics over raw
  memory used by `libjn/stdatomic.jn` and runtime primitives.

Neither has any connection to the (now-removed) `@atomic` type
annotation.

## 8. Implementation map

| Concern                                | File                                                                            |
| -------------------------------------- | ------------------------------------------------------------------------------- |
| Surface modifiers (`copy`/`take`/`const`) | [src/ast.rs](src/ast.rs#L382)                                                |
| `@resource` parsing                    | [src/parser/decl/types.rs](src/parser/decl/types.rs#L48)                        |
| Ownership tier selection               | [src/typer/mod.rs](src/typer/mod.rs)                                            |
| Statement-level ownership overrides    | [src/typer/stmt/dispatch.rs](src/typer/stmt/dispatch.rs)                        |
| Move tombstones (whole-var + field)    | [src/typer/lower/block.rs](src/typer/lower/block.rs) (`record_take_moves_in_stmt`), [src/typer/expr/ident.rs](src/typer/expr/ident.rs), [src/typer/expr/access.rs](src/typer/expr/access.rs) |
| HIR `Ownership` enum                   | [src/hir/mod.rs](src/hir/mod.rs#L15)                                            |
| Cross-thread enforcement               | [src/typer/mod.rs](src/typer/mod.rs) (`enforce_cross_thread_safe`)              |
| Escape analysis                        | [src/escape/mod.rs](src/escape/mod.rs)                                          |
| Drop emission                          | [src/codegen/drop/aggregates.rs](src/codegen/drop/aggregates.rs)                |
| Perceus use-counting pass              | [src/perceus/mir_perceus.rs](src/perceus/mir_perceus.rs)                        |

## 9. What this document deliberately does not cover

- Detailed MIR lowering of moves (`InstKind::Move`, drop hoisting,
  field-tombstone tracking) — see source.
- The Perceus heuristics for `Vec` reuse pairing — see
  [src/perceus/mir_perceus.rs](src/perceus/mir_perceus.rs).
- LLVM-level parameter attributes (`nocapture`, `readonly`,
  `dereferenceable`) — see
  [src/codegen/support/runtime.rs](src/codegen/support/runtime.rs)
  (`set_ptr_param_attrs`).
- Actor scheduler internals — see [runtime/actor.c](runtime/actor.c).

## 10. Conformance tests

Every rule in this document is pinned by an executable conformance test in
[tests/access_semantics.rs](tests/access_semantics.rs). Each test compiles
and runs a real `.jn` program through `jinnc` (or asserts a specific
compile-time diagnostic), so the contract above cannot silently drift from
the implementation.

| Test                                      | Rule pinned                                                                 |
| ----------------------------------------- | --------------------------------------------------------------------------- |
| `vec_param_borrows_and_mutates_in_place`  | Heap params borrow by default; callee mutates the caller's value in place (§6.1). |
| `vec_copy_param_is_independent`           | `copy` parameter is an independent value; callee mutations never reach the caller (§2, §4.1). |
| `string_param_borrows_caller_retains`     | A borrowed `String` parameter leaves the caller owning it (§3). |
| `string_take_moves_use_after_is_error`    | Whole-variable use-after-`take` is a compile error (§4.2, §6.3). |
| `struct_field_take_preserves_siblings`    | `take` of one struct field leaves the siblings intact and readable. |
| `struct_copy_is_deep_independent`         | `copy` of a struct deep-clones nested heap fields (§2). |
| `enum_payload_value_semantics`            | Enum payloads obey value semantics. |
| `resource_copy_is_compile_error`          | `copy` of a `@resource` type is rejected — resources are linear (§4.1). |
| `resource_drop_runs_once_at_scope_exit`   | `*drop` runs exactly once, deterministically, at scope exit (§4.2, §6.2). |
| `take_then_reassign_is_ok`                | Reassigning a moved-out variable clears its tombstone (§4.2). |
| `field_take_use_after_is_error`           | Reading a struct field after `take` is a compile error (§4.2, §6.3). |

Run them with:

```sh
cargo test --test access_semantics
```
