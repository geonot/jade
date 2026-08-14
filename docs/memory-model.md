# Memory model — ownership, moves, and inferred borrows

The binding contract for every value in a Jinn program: **numbers and text
copy; containers move; reads borrow**. There is no garbage collector, no
refcount on any user path, no `Rc`/`Arc`/`Box`, and no lifetime syntax. Every
owned value has exactly one drop site, decided by the compiler.

This document is the contract the typer, the escape analysis, the drop
discipline, and codegen implement against. The implementation is wrong wherever
it disagrees with this file. Rules are numbered `M1`–`M11` and each is pinned by
a conformance test in `tests/memory_model.rs`, `tests/access_semantics.rs`, or
`tests/semantics_regression.rs`.

Known gaps between this contract and the implementation are tracked in
[`roadmap.md`](roadmap.md#memory-and-ownership). Since [146], moves and borrows
are tracked per **place** (`root.field.elem…`) with overlap and disjointness
queries: overlapping call arguments, iteration borrows of field places and
maps, and moves through projections are checked, and disjoint sibling places
stay independent. Element reads in expression position (`get`) copy by
contract; the zero-copy spelling is a view (§12).

## 1. Design pillars

Jinn is **value-semantics first**. The mental model is:

> Every binding is a value. Assignment, parameter passing, and field reads
> behave *as if* the value were copied.

The compiler then proves which copies are unobservable and turns them into
borrows, in-place mutation, or zero-cost moves. Three rules govern everything
below:

1. **Single ownership at any moment.** Every heap-backed value has exactly one
   owner. Aliases are statement-scoped borrows; no binding can hold one.
2. **No hidden refcount in the fast path.** Refcounted handles do not exist in
   the IR. Cross-thread sharing uses purpose-built atomic-refcounted primitives
   (`Channel`, `ActorRef`), not a generic `Arc of T`.
3. **The user writes intent; the compiler picks the tier.** A modifier (§7) is
   written only to override the default.

## 2. Type categories

Every type falls in exactly one category, and the category decides what `is`
does.

| Category | Members | `b is a` | Drop obligation |
| --- | --- | --- | --- |
| **Scalar** | `i8`–`u64`, `f32`/`f64`, `bool`, raw pointers (`%T`), enums with no heap payload | bit-copy; both live | none |
| **Value** | `String` (24-byte SSO handle), structs and enums containing only scalars and `String` | deep copy; both live, independent | each copy frees its own heap |
| **Aggregate** | `Vec of T`, `Map of K, V`, generators, coroutines, closures (all function-typed values, since [148]), and any struct or enum with an aggregate-typed field (transitively) | **move**; `a` is dead until reassigned | exactly one owner drops |
| **Resource** | any `@resource` type | move (linear; copies are rejected) | owner runs `*drop` once |

Category is inferred from field types, transitively: adding a `Vec` field to a
leaf struct changes the assignment semantics of every struct that embeds it.
Since [147] a type can pin its category — `type Point @value` / `type Bag
@aggregate` — and a definition whose fields contradict the assertion is a
compile error naming the field that flips it. Types without an assertion still
transition silently; the embedding-site diagnostic remains open (`O-5`).

## 3. Bindings and moves

### M1 — assignment moves an aggregate

```jinn
*main
    a is vec(1, 2, 3)
    b is a           # ownership moves to b
    b.push(4)
    log(b.length)    # 4
    0
```

`a` is a **tombstone** after `b is a`; reading it is a compile error:

```
error: use of moved value `a`
  --> m.jn:4:9
note: `a` moved here: `b is a` (m.jn:3:5) — aggregates move on assignment
help: to keep both values, clone explicitly: `b is copy a`
```

### M2 — reassignment revives

```jinn
*main
    a is vec(1)
    b is a           # a dead
    a is vec(2)      # a live again, owning a fresh vector
    log(a.length)    # 1
    0
```

An aggregate bind from a variable behaves exactly like `b is take a`, and
reassignment clears the tombstone the same way.

### M3 — binding a struct field is a partial move

```jinn
type Bag
    items as Vec of i64
    label as String

*main
    b is Bag(items is vec(1), label is 'x')
    v is b.items         # moves the field out
    log(b.label)         # ok — siblings unaffected
    log(v.length)
    0
```

Reading `b.items` after the move, or reading the *whole* struct while a field is
moved out, is a compile error:

```
error: use of moved field `b.items`
note: moved here: `v is b.items` (m.jn:7:10)
help: to keep the field, clone it: `v is copy b.items`
```

### M4 — binding a container element does not move

A container slot cannot be tombstoned — that would leave a hole — so binding an
element whose type is an **aggregate** is rejected rather than silently aliased
or silently cloned:

```
error: cannot bind aggregate element `grid.get(0)` — binding would alias the container's memory
help: clone it (`row is copy grid.get(0)`) or remove it (`row is take grid.get(0)`)
```

`take` on a container slot keeps its meaning: remove-and-own. Scalar and
`String` elements bind freely (`x is nums.get(0)` copies). Element reads in
**expression position** (`grid.get(0).length`) are borrows and stay legal — see
M5.

## 4. Reads and borrows

### M5 — reads borrow, and a borrow ends with its statement

Method calls, field reads, index reads, and argument passing **borrow**: no
copy, no move, no refcount. A borrow created inside a statement ends when the
statement completes. There is no way to store a borrow in a binding — M1, M3 and
M4 make every binding an owner — so borrows never outlive their source. That is
the entire lifetime story, and it is why no lifetime annotations exist.

```jinn
*main
    v is vec(1, 2, 3)
    log(v.length)        # borrow for the duration of the call
    v.push(4)            # exclusive borrow during the call
    0
```

The **exclusivity axiom** — at any program point a place has either one
mutable borrow or any number of read borrows — is enforced *within* statements
as well as across them ([146]): one call may not receive overlapping places
when a parameter mutates or consumes one of them, a `for` loop takes an
iteration borrow of the iterated *place* (a variable, a field like `s.items`,
a map, or an `Iter` source) that rejects overlapping mutation, moves, and
reassignment in the body — while leaving disjoint sibling places free — and a
parameter that was inferred borrowing cannot be moved out of (`take v`,
rebinding it into an owner, or capturing it in a constructor either makes the
parameter consuming or is a compile error). Two deliberate exceptions remain:
container-method argument reads (`v.set(i, v.get(j))`) evaluate before the
receiver's mutable borrow activates, and a mutating call argument must be a
whole variable — field and element reads pass copies, so passing one to a
mutating or consuming parameter is rejected outright rather than silently
losing the update.

### M6 — parameters borrow unless the callee consumes

An unannotated aggregate parameter defaults to a borrow; the caller keeps
ownership:

```jinn
*push_one(v)
    v.push(1)

*main
    xs is vec(1, 2, 3)
    push_one(xs)
    log(xs.length)   # 4 — xs still owned by main
    0
```

Binding a borrowed parameter to a local (`s is v`) does not mint an owner: the
new binding borrows too, so the callee can rename or restructure without
double-freeing the caller's value ([146] — this used to create a second owner
and free twice). `s is take v` on a borrowed parameter is a compile error that
suggests declaring the parameter `take`.

If the callee's body **consumes** the parameter — returns it, binds it, stores
it in something that outlives the call, sends it on a channel, moves it into
a task, captures it in a constructor (a struct literal, tuple, array, or
`vec(...)` takes ownership of its parts, [146]), or stores it into a field of
`self` — in either spelling, `self.data is x` or the idiomatic bare
`data is x` ([147]; the bare form used to slip past inference and double-free
at runtime) — the parameter is inferred **consuming**, and the call site moves
the argument exactly as `take` would:

```jinn
*ident(v) returns Vec of i64
    return v          # v escapes, so the parameter is consuming

*main
    a is vec(1, 2, 3)
    s is ident(a)     # a moves into the call
    log(s.length)     # 3 — one buffer, exactly one drop
    0
```

Reading `a` afterwards is a compile error that names the consuming call and the
line in the callee where the value escaped. Consumingness is inferred per
parameter, bottom-up over call-graph SCCs; explicit `take`/`copy` annotations
override inference and remain the vocabulary for exported APIs.

### M7 — returning transfers ownership

Returning a local, a consumed parameter, or a fresh expression transfers
ownership to the caller. The callee emits **no** drop for the returned value.
Returning a parameter that an explicit annotation forced to stay borrowed is an
error:

```
error: cannot return borrowed parameter `v`
help: take ownership: declare the parameter `v as take Vec of i64`, or return `copy v`
```

## 5. Concurrency

### M8 — an aggregate moves into at most one task

Capturing an aggregate in `dispatch`, `together`, `sim for`, a `spawn`
initializer, an actor message payload, or a channel `send` **moves** it into
that task or message. A second capture, or any later use in the parent, is a
use-after-move:

```
error: `shared` used after being moved into a concurrent task
  --> race.jn:11:20
note: `shared` moved into the task dispatched at race.jn:8-9; two tasks may not share one aggregate
help: give each task its own vector and merge the results over a channel,
      or let a single actor own the vector and send it messages
```

The diagnostic names the alternative on purpose: "use of moved value" alone
reads as a limitation rather than as a caught data race. Scalars and `String`s
copy into tasks freely. `@resource` values are rejected at task boundaries
outright.

### M9 — channels and actors transfer ownership

`send ch, v` moves `v` — the sender's binding tombstones — and `receive ch`
yields an owned value. An actor owns its state fields; message payloads move in
on send and are owned by the handler invocation. This is what "shared mutable
state is expressed by message passing" means mechanically: the data is never
shared, it is *relocated*.

## 6. Scope exit, `defer`, generators, closures

### M10 — one drop per owner, after `defer`

At scope exit, live owned aggregates drop exactly once, in reverse binding
order, **after** the scope's `defer` blocks run — so a `defer` may read anything
it could read at registration. Tombstoned bindings drop nothing. Moving a value
that a registered `defer` reads is a compile error:

```
error: cannot move `buf`: it is read by the `defer` registered at m.jn:4:5
help: move it before the defer is registered, or clone it into the defer
```

### M11 — generators own their captures

Creating a generator moves captured aggregates into its frame. The frame
outlives the creating statement, so borrowing would be unsound — the same
reasoning as M8. The frame's owner drops whatever it still holds. Yielded
aggregate values transfer ownership to the consumer of `next()`; yielded scalars
and strings copy. Since [149] the move-in is enforced: a generator's aggregate
arguments are inferred consuming (the frame outlives the call), so using the
original after creation is a use-after-move — previously the frame aliased the
caller's value and resuming after a consuming call read freed memory. Captures
still held by a generator dropped mid-suspension are not yet freed
(suspended-frame drops, `O-7`).

### M12 — closures capture exactly like tasks

Since [149], calling through a function-typed *parameter* inside a
`needs`-annotated function is a capability error: the callee's row cannot be
classified through a closure value yet, so the row derives the conservative
"indirect call" taint and the diagnostic names the introduction path. Lambda
*bodies* are scanned where they are written, so a closure's own effects are
always charged to its defining function.


Creating a closure (`f is |x| …` with free variables) classifies each captured
binding by category: scalars copy at creation, `String`s and value structs are
cloned into the environment, and aggregates **move** in — a later use of the
original is a use-after-move naming the capture site. There is no capture by
reference, ever. The closure value is itself an aggregate that owns its
environment: assigning it moves it, dropping it drops every capture, a
function-typed parameter borrows it for the call, and it moves into at most
one task (M8 applies unchanged). A borrowed parameter, a `@resource` value,
and a view (§12) cannot be captured; the diagnostics name the clone-first
escape hatch. Environments are never shared: capturing the same aggregate in
two closures is a double move.

## 7. Access modifiers and `@resource`

A binding, parameter, field, or loop binder may carry at most one access
modifier:

| Modifier | Meaning |
| --- | --- |
| *(none)* | The compiler picks the ownership tier from use (§8). |
| `copy` | Deep clone at the boundary; the consumer owns an independent value. |
| `take` | Move out of the source, or remove from a container slot. The source dies. |
| `const` | Rebind ban: the name may not appear on the left of `is` again in its scope. |

`copy` and `take` are about the **data flow**; `const` is about the **name
binding** and is orthogonal to ownership. A `const` aggregate that is moved
cannot be revived by M2, so its tombstone is permanent for that scope.

`ref` and `mut` are not surface keywords — the compiler chooses shared versus
exclusive aliasing from usage.

The modifier sits in **type position** (`name as take Type`), so a move is
driven by the callee's parameter rather than by a marker at the call site:

```jinn
*consume(s as take String)
    log(s.length)

*main
    name is 'alice'
    consume(name)
    # reading `name` here is a compile error: it was moved by an earlier `take`
    0
```

`copy x` is the escape hatch every M1/M3/M4 diagnostic names. `take` stays
meaningful for strings (which otherwise copy), for container slots (M4), and as
documentation of intent.

### `@resource` — linear types

`@resource` declares a linear type. Its values may never be implicitly
duplicated, may never cross a thread boundary (channel, actor send, spawn
capture), and have their `*drop` method invoked automatically at scope exit.
`copy` of a `@resource` value is a compile error.

```jinn
type File @resource
    handle as i64

    *drop
        if self.handle neq 0
            close_fd(self.handle)
            self.handle is 0
```

`@align(N)`, `@packed`, and `@strict` are layout attributes and have no bearing
on access semantics.

There is no `@atomic` type annotation. Two unrelated uses of the word "atomic"
remain and neither is about ownership: the `atomic x is value` binding, which
makes loads and stores of `x` sequentially consistent, and the `atomic_*` FFI
intrinsics over raw memory.

## 8. Ownership tiers

After type checking, every binding carries one of four ownership tiers in the
HIR. Tiers are an internal concept and are never written by the user.

| Tier | Lowering | When chosen |
| --- | --- | --- |
| `Owned` | Sole owner; responsible for the drop at scope exit. | Fresh values, constructor results, results of `take`. |
| `Borrowed` | Raw pointer alias. No refcount, no drop. | Read-only aliases whose live range is dominated by the source. |
| `BorrowMut` | Raw mutable alias. No refcount, no drop. | Exclusive alias for in-place update (`v.push` on the owner). |
| `Raw` | User-managed pointer (`Type::Ptr`). | FFI pointer types. |

Defaults:

- **POD types** (numerics, `bool`, small tuples of POD, pointers) are always
  `Owned` — a bit-copy *is* an independent value.
- **Heap-leaf containers** (`String`, `Vec`, `Map`, coroutines, generators)
  default to `Borrowed` for unannotated parameters: a caller should not lose
  ownership just by passing a vector to a helper.
- **User heap structs and enums** default to `Borrowed` for parameters iff they
  need drop, otherwise `Owned`.
- **Explicit modifiers always win.** `copy` → `Owned` with a clone, `take` →
  `Owned` with a move, `const` does not change the tier.

Only `Owned` bindings get drop glue. Parameter ownership is decided *after*
inference has resolved the parameter's type — deciding it earlier makes an
unannotated parameter whose type is still a variable default to `Owned`, and the
callee then emits drop glue for a value the caller still owns.

Escape analysis (`src/escape/`) classifies each binding as `T1`/`T2`/`T3` by
whether it escapes its scope, its thread, or neither; MIR and codegen consume
that to place drops and decide where in-place mutation is safe. Drop *placement*
(`src/drops/mir_drops.rs`, Perceus-style elision, sinking, fusion, reuse) is a
pure optimization over this model: it may elide or sink the one drop, never add
a second.

## 9. Cross-thread access

Tasks communicate through two primitives, both atomically refcounted inside the
runtime:

- **`Channel of T`** — bounded MPMC FIFO. `send`/`receive` move ownership.
- **`ActorRef of T`** — typed handle for sending messages to a spawned actor.

User code never wraps a struct in a "shared" mode; it sends the value. The
cross-thread rule is enforced in `enforce_cross_thread_safe` at channel-send
lowering, actor handler argument typing, and actor `spawn` init typing:

> A `@resource` value may not cross a thread boundary.

```
foo.jn:7:5: resource type `Handle` cannot cross thread boundaries (channel send)
```

A plain scalar is POD, copies freely, and crosses without issue. See
[`concurrency.md`](concurrency.md) for what the tasks themselves do.

## 10. Programs that must be rejected

| Program | Outcome | Diagnostic (lead line) |
| --- | --- | --- |
| `b is a; b.push(4); log(a.length)` | compile error | ``use of moved value `a` `` |
| two `dispatch` blocks over one `Vec` | compile error | ``  `shared` used after being moved into a concurrent task `` |
| use in the parent after a single task capture | compile error | same as above |
| bind of an aggregate element (`row is grid.get(0)`) | compile error | `cannot bind aggregate element` |
| whole-struct read while a field is moved out | compile error | ``use of partially moved value `b` `` |
| move of a value read by an earlier `defer` | compile error | ``cannot move `buf` `` |
| return of an explicitly-borrowed parameter | compile error | ``cannot return borrowed parameter `v` `` |
| `*ident(v) returns Vec of i64; return v` | **compiles**, runs clean, one drop | — |

## 11. Consequences, stated so they are not lost

- **No reference cycles are constructible.** Every aggregate has exactly one
  owner at any moment and no shared handles exist, so an ownership cycle cannot
  be expressed and cycle leaks are impossible by construction. This holds only
  as far as there are no leaks by other means, which is what the whole-corpus
  sanitizer sweep (`ci/sanitize-corpus.sh`, roadmap `O-2`) exists to check.
- **No refcount traffic exists on any path**, hot or cold. The only atomic
  refcounts are the runtime-internal `Channel` and `ActorRef` handles.
- **`jinn check` and `jinn build` agree.** The ownership analysis runs in both
  pipelines, so a program `check` passes must not corrupt memory when built.

## 12. Frozen values and second-class views ([148])

**`freeze x`** consumes an aggregate operand — a move through the same lattice
as every other move, with its own `MoveReason::Freeze` diagnostic — and
produces `Frozen of T`: representationally `T`, deeply immutable forever.
Freezability is structural (scalars, `String`, `Vec`, `Map`, structs/enums of
the same, recursively; `@resource` types, channels, actors, coroutines,
generators, functions, and views are rejected with the offending field path
named). Reads auto-deref: field and element reads, iteration, read-only
methods, and read-only parameters accept a frozen value unchanged. Every write
is a compile error at its natural chokepoint: mutating/consuming method calls
(builtin table and inferred user-method bits), assignment through a frozen
component, partial moves (`take fz.field`, aggregate field binds), and passing
to a parameter the mutation inference marks mutating or consuming — consuming
is rejected because a move into a mutable owner would thaw. `Frozen of T` in a
parameter or field position demands immutability at the boundary; `copy` of a
frozen place produces a fresh mutable value. Scope-shared multi-task capture
(`together` handing one frozen value to every `dispatch`) shipped in [149]:
frozen captures inside a `together` do not move — every `dispatch` shares the
one value by pointer, a `FrozenShare` borrow locks it against moves until the
`together` joins, and the owner (which must be bound outside the `together`)
keeps it afterwards and drops it once. Frozen values created *inside* the
`together` body still move into a single task (their drop would race the
join). Actor sends of frozen values are rejected with guidance unless the
handler declares `Frozen of ...` — handler write-inference is not yet
classified ([`design/freeze.md`](design/freeze.md), roadmap `O-8`).

**`View of T`** is a two-word borrowed window (`ptr + len`) created by
`xs.view(a, b)`, `xs.at_view(i)`, and `s.view(a, b)` (string bytes), or by
passing a whole `Vec`/array to a `View of T` parameter. Views are
*second-class*: they flow down (calls, expressions, loop bodies) and never
out — returning, storing in fields/containers/stores (declared *or*
inferred), sending, task capture, closure capture, and yielding are all
rejected at compile time. Since [149] a view may be **bound**: the bind
registers a borrow of the view's root in the same lattice stack iteration
borrows use, so mutating, moving, or reassigning the root while the view
lives is rejected with the view named, and the borrow dies with the view's
block. Views of temporaries cannot be bound; a bind from another view
inherits its root; view-typed parameters have no local root (the caller's
call-borrow covers them). `for x in xs.views()` is lending iteration —
the binder is a per-element view — and a field read through an element view
reads through the pointer, no element copy. Moving a field
out of a view is rejected; `.get` on value-category elements copies out
(clone for `String`). Read-only *method calls* through an element view pass
the element pointer as the receiver — the call operates on the original, and
mutating or consuming methods through a view are compile errors ([150]).
The rest of std's byte loops remain
([`design/second-class-refs.md`](design/second-class-refs.md), roadmap
`O-9`).

## 13. What this document does not cover

- MIR lowering of moves, drop hoisting, and field-tombstone tracking — see
  `src/mir/`.
- Perceus reuse-pairing heuristics — see `src/drops/mir_drops.rs`.
- LLVM parameter attributes (`nocapture`, `readonly`, `dereferenceable`) — see
  `set_ptr_param_attrs` in `src/codegen/support/runtime.rs`.

## 14. Implementation map

| Concern | File |
| --- | --- |
| Surface modifiers (`copy`/`take`/`const`) | `src/ast.rs` (`enum AccessMod`) |
| `@resource` parsing | `src/parser/decl/types.rs` (`parse_layout_attrs`) |
| Ownership tier selection | `src/typer/mod.rs` (`ownership_with_mod`, `param_ownership_with_mod`) |
| Statement-level tier overrides | `src/typer/stmt/dispatch.rs` |
| Move tombstones (variable and field) | `src/typer/lower/block.rs`, `src/typer/expr/ident.rs`, `src/typer/expr/access.rs` |
| Consuming-parameter inference | `src/typer/consume_infer.rs`, `src/typer/scc.rs` |
| HIR `Ownership` enum | `src/hir/mod.rs` |
| Cross-thread enforcement | `src/typer/mod.rs` (`enforce_cross_thread_safe`) |
| Escape analysis | `src/escape/mod.rs` |
| Drop emission | `src/codegen/drop/aggregates.rs` |
| Drop placement / Perceus | `src/drops/mir_drops.rs` |
| Drop verification (double-drop, use-after-drop, and since [147] the leak side: every `Vec`/`Map` allocation dropped or moved on every path to return) | `src/drops/verify.rs` (`JINN_MIR_VERIFY=0` opts out) |
| Category assertions (`@value`/`@aggregate`) | `src/typer/resolve.rs` (`check_category_assertion`) |
| `freeze` lowering, freezability, write rejection | `src/typer/expr/freeze.rs` |
| View methods, coercion, escape rejection | `src/typer/expr/views.rs`, `src/typer/call/method_call.rs`, `src/codegen/view.rs` |
| Closure capture classification | `src/typer/expr/lambda.rs`, `src/typer/lower/block.rs` (`mark_closure_captures`) |
| Closure environments (owned, cloned values, env drop fn) | `src/codegen/mir_codegen/helpers/runtime.rs` (`emit_closure_create`), `src/codegen/drop/mod.rs` (`drop_closure`) |
| Boundary ownership (`--lib` warnings, `.jni` `consumes`/`mutates` bits) | `src/typer/consume_infer.rs` (`boundary_ownership_warnings`), `src/interface.rs` |

## 15. Conformance

Every rule above is pinned by a test that compiles and runs a real program
through `jinnc`, or asserts a specific diagnostic lead line:

| Suite | Covers |
| --- | --- |
| `tests/memory_model.rs` | M1–M11 |
| `tests/access_semantics.rs` | modifier surface, `@resource` linearity, drop timing, tombstones |
| `tests/semantics_regression.rs` | the §10 rejection table |
| `tests/ownership_fuzz.rs` | randomized ownership-relevant programs |
| `tests/place_ownership.rs` | the [146] place lattice and the [147] closures: quaternary-arm and pipe moves, idiomatic field-store/field-write inference, category assertions, boundary warnings, `std/arena` |
| `tests/freeze.rs` | §12 frozen values: reads, every write rejection, freezability, `Frozen of T` boundaries |
| `tests/views.rs` | §12 views: creation, reads, bounds traps, every escape rejection |
| `tests/closure_captures.rs` | M12: capture classification, owned environments, closure moves |

Beyond the suites, `ci/sanitize-corpus.sh` compiles and runs the whole
executable corpus (conformance programs, apps, snippets) under ASan+LSan at
`--opt 0` and `--opt 3`, and `ci/fuzz-ownership.py` mutates ownership-relevant
syntax and asserts the compiler either rejects the mutant with a diagnostic or
the compiled result stays memory-safe (roadmap `O-2` tracks the leak tail).
