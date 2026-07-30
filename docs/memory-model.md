# Jinn Memory Model — aggregates, moves, and inferred borrows

> The binding contract for decision **D1**
> ([`remediation-2026-07.md`](remediation-2026-07.md) §0): heap aggregates
> are **move-on-assign with compiler-inferred borrows**. This document is
> what tasks 8-6 (drop discipline), 8-7 (one flow-sensitive ownership
> analysis), and 8-8 (cross-task aliasing rejection) implement against; the
> implementation is wrong wherever it disagrees with this file. It refines
> [`access-semantics.md`](access-semantics.md) — the modifier surface
> (`copy`/`take`/`const`), the HIR tiers, and `@resource` linearity are
> unchanged — and corrects it where the review showed its claims did not
> hold (§9).
>
> Status: **contract, being enforced.** Landed: 8-6 (M6 inferred consuming
> parameters, M7 return transfer, single-drop accounting through nested
> scopes — §3.1 runs clean and its use-after-move companion is rejected);
> 8-7 (M1 move-on-assign, M2 revive, M3 field partial move, M4 element-bind
> rejection, M9 channel-send tombstone, M10 defer-read protection — the
> typer's flow-sensitive analysis is the single authority, `src/ownership/`
> is deleted, and `jinn check`/`build` agree); 8-8 (M8 — aggregates move
> into at most one task: `dispatch` capture, `sim for` bodies, actor
> message payloads, and `spawn` initializers all enforced, with the
> `copy`-capture snapshot pattern recognized). Every §7 row is now
> enforced; the review pins in `tests/review_2026_07.rs` are flipped.

## 1. Type categories

Every type falls in exactly one category; the category decides what `is`
does.

| Category | Members | `b is a` | drop obligation |
| --- | --- | --- | --- |
| **Scalar** | `i8..u64`, `f32/f64`, `bool`, raw pointers (`%T`), function values, enums with no heap payload | bit-copy; both live | none |
| **Value** | `String` (24-byte SSO handle), structs/enums containing only scalars and `String` | deep copy; both live, independent | each copy frees its own heap |
| **Aggregate** | `Vec of T`, `Map of K, V`, generators/coroutines, and any struct or enum with an aggregate-typed field (transitively) | **move**; `a` is dead until reassigned | exactly one owner drops |
| **Resource** | any `@resource` type | move (linear; copies are already rejected) | owner runs `*drop` once |

The rule of thumb the user needs: **numbers and text copy; containers
move.** Nothing else in this table is new — strings already deep-copy, and
`@resource` already moves. D1 makes `Vec`/`Map`/aggregate structs behave
like resources with an implicit, inferred discipline instead of like
untracked shared pointers.

## 2. Bindings

### M1 — assignment moves an aggregate *(example `m1_move_on_assign`)*

```jinn
*main
    a is vec(1, 2, 3)
    b is a           # ownership moves to b
    b.push(4)
    log(b.length)    # 4
```

`a` is a **tombstone** after `b is a`. Reading it is a compile error:

```jinn
*main
    a is vec(1, 2, 3)
    b is a
    log(a.length)    # error
```

```
error: use of moved value `a`
  --> m.jn:4:9
note: `a` moved here: `b is a` (m.jn:3:5) — aggregates move on assignment
help: to keep both values, clone explicitly: `b is copy a`
```

This is the §3.3 review program: it must **fail to compile** with exactly
this diagnostic class. Today it compiles and aliases (pinned in
`tests/review_2026_07.rs::review_3_3_vec_assignment_aliases`).

### M2 — reassignment revives *(example `m2_reassign_revives`)*

```jinn
*main
    a is vec(1)
    b is a           # a dead
    a is vec(2)      # a live again, owning a fresh vector
    log(a.length)    # 1 — ok
```

Identical to the existing `take` tombstone rule
(access-semantics.md §4.2); D1 reuses that machinery — an aggregate bind
from a variable behaves exactly like `b is take a`.

### M3 — binding a struct field is a partial move *(example `m3_field_move`)*

```jinn
type Bag
    items as Vec of i64
    label as String

*main
    b is Bag(items is vec(1), label is 'x')
    v is b.items       # moves the field out (as `take b.items` does today)
    log(b.label)       # ok — siblings unaffected
    log(b.items.length)  # error
```

```
error: use of moved field `b.items`
note: moved here: `v is b.items` (m.jn:7:10)
help: to keep the field, clone it: `v is copy b.items`
```

Reading the *whole* struct (`c is b`, passing `b` somewhere) while a field
is moved out is also an error, matching the existing field-tombstone rule.

### M4 — binding a container element does not move *(example `m4_element_bind`)*

A container slot cannot be tombstoned (that would leave a hole), so a bind
of an element read whose type is an **aggregate** is rejected rather than
silently aliased or silently cloned:

```jinn
*main
    grid is vec()
    grid.push(vec(1, 2))
    row is grid.get(0)      # error — element is Vec of i64
```

```
error: cannot bind aggregate element `grid.get(0)` — binding would alias the container's memory
help: clone it (`row is copy grid.get(0)`) or remove it (`row is take grid.get(0)`)
```

`take` on a container slot keeps its existing meaning: remove-and-own.
Scalar and `String` elements bind freely (`x is nums.get(0)` copies).
Element reads in **expression position** (`grid.get(0).length`,
`log(grid.get(0))`) are borrows and stay legal — see M5.

## 3. Reads and borrows

### M5 — reads borrow; a borrow ends with its statement *(example `m5_reads_borrow`)*

Method calls, field reads, index reads, and argument passing **borrow**:
no copy, no move, no refcount. A borrow created inside a statement ends
when the statement completes; there is no way to store a borrow in a
binding (M1/M3/M4 make every binding an owner), so borrows never outlive
their source. That is the whole lifetime story, and it is why no lifetime
annotations exist.

```jinn
*main
    v is vec(1, 2, 3)
    log(v.length)        # borrow for the duration of the call
    total is v.sum()     # borrow during the call; total owns a scalar
    v.push(4)            # exclusive borrow during the call
```

### M6 — parameters borrow unless the callee consumes *(examples `m6_borrow_param`, `m6_consuming_param`)*

An unannotated aggregate parameter defaults to a borrow — the caller keeps
ownership (unchanged from access-semantics.md §4.1, pinned by
`vec_param_borrows_and_mutates_in_place`):

```jinn
*push_one(v)
    v.push(1)

*main
    xs is vec(1, 2, 3)
    push_one(xs)
    log(xs.length)   # 4 — xs still owned by main
```

But if the callee's body **consumes** the parameter — returns it, binds it
(M1), stores it in a struct/container that outlives the call, sends it on
a channel, or moves it into a task — the parameter is inferred as
**consuming**, and the call site moves the argument exactly as `take`
would:

```jinn
*ident(v) returns Vec of i64
    return v          # v escapes → parameter inferred consuming

*main
    a is vec(1, 2, 3)
    s is ident(a)     # a moves into the call
    log(s.length)     # 3 — s owns the one buffer; exactly one drop
    log(a.length)     # would be: error: use of moved value `a`
                      #   note: `a` moved into `ident` here (m.jn:6:16),
                      #         whose parameter `v` is consuming
                      #         (returned at m.jn:2:12)
```

The first six lines are the §3.1 review program: it must **compile and run
cleanly, printing 3**, under ASan, at `--opt 0` and `--opt 3` (today it
double-frees; pinned as `review_3_1_returning_vec_parameter_corrupts_heap`).
Consumingness is inferred per parameter from the callee body, computed
bottom-up over the call graph (SCCs conservatively treat in-cycle calls as
consuming only if any member consumes). Explicit `take`/`copy` annotations
override inference and remain the vocabulary for exported library APIs.

### M7 — returning transfers ownership *(example `m7_return_transfers`)*

Returning a local, a consumed parameter, or a fresh expression transfers
ownership to the caller. The callee emits **no** drop for the returned
value; the caller's binding is the sole owner. A `return` of a *borrowed*
parameter that inference could not make consuming (only possible with an
explicit non-consuming annotation) is a compile error:

```
error: cannot return borrowed parameter `v`
help: take ownership: declare the parameter `v as take Vec of i64`, or return `copy v`
```

## 4. Concurrency

### M8 — an aggregate moves into at most one task *(example `m8_cross_task`)*

Capturing an aggregate in `dispatch`, `together`, `sim for`, a `spawn`
initializer, an actor message payload, or a channel `send` **moves** it
into that task/message. A second capture — or any later use in the parent
— is a use-after-move:

```jinn
*pusher(v, base)
    for i in 0 to 20000
        v.push(base + i)

*main
    shared is vec()
    together
        dispatch
            pusher(shared, 0)        # shared moves into this task
        dispatch
            pusher(shared, 1000000)  # error
```

```
error: `shared` used after being moved into a concurrent task
  --> race.jn:11:20
note: `shared` moved into the task dispatched at race.jn:8-9; two tasks may not share one aggregate
help: give each task its own vector and merge the results over a channel,
      or let a single actor own the vector and send it messages
```

This is the §3.2 review program: it must **fail to compile** with this
diagnostic (today it corrupts the allocator; pinned as
`review_3_2_cross_task_shared_vec_races`). The diagnostic must name the
alternative — "use of moved value" alone reads as a limitation instead of
a caught race. The equivalent correct programs (per-task vectors merged
over a channel; an actor owning the vector) compile and run.

Scalars and `String`s copy into tasks freely. `@resource` values are
already rejected at task boundaries (`enforce_cross_thread_safe`), which
stays.

### M9 — channels and actors transfer ownership *(example `m9_channel_transfer`)*

`send ch, v` moves `v` (sender's binding tombstones); `receive ch` yields
an owned value. An actor owns its state fields; message payloads move in
on send and are owned by the handler invocation. This is what "shared
mutable state is expressed by message passing" means mechanically: the
data is never shared, it is *relocated*.

## 5. Scope exit, `defer`, generators

### M10 — one drop per owner, after `defer` *(example `m10_defer_order`)*

At scope exit, live owned aggregates drop exactly once, in reverse binding
order, **after** the scope's `defer` blocks run (a `defer` may therefore
read any binding it could read at registration). Tombstoned bindings drop
nothing. Moving a value that a registered `defer` reads is a compile
error:

```
error: cannot move `buf`: it is read by the `defer` registered at m.jn:4:5
help: move it before the defer is registered, or clone it into the defer
```

### M11 — generators own their captures *(example `m11_generator_captures`)*

Creating a generator moves captured aggregates into its frame (the frame
outlives the creating statement, so borrowing is unsound — same reasoning
as M8). The frame's owner drops whatever the frame still holds when it is
dropped. `yield`ed aggregate values transfer ownership to the consumer of
`next()`; yielded scalars/strings copy.

## 6. Interaction with explicit modifiers

- `copy x` — deep clone; always legal on non-`@resource` values; the
  escape hatch every M1/M3/M4 diagnostic names.
- `take x` / `take x.f` / `take v.get(i)` — explicit move / field move /
  slot removal. M1 and M3 make the implicit forms behave identically for
  aggregates; `take` remains meaningful for strings (which otherwise
  copy), for container slots (M4), and for documentation of intent.
- `const x` — rebind ban only; orthogonal (but note a `const` aggregate
  that is moved cannot be revived by M2, so the tombstone is permanent for
  that scope — the diagnostic should say so).

## 7. Programs that must be rejected

Each entry names its pinned test in `tests/review_2026_07.rs` (flipped
from observed-bad to required-good by the owning task) or its conformance
test to be added by 8-6..8-8.

| Program | Outcome required | Diagnostic (lead line) | Owner |
| --- | --- | --- | --- |
| §3.3 `b is a; b.push(4); log(a.length)` | compile error | `use of moved value \`a\`` | 8-7 |
| §3.2 two `dispatch` over one `Vec` | compile error | `\`shared\` used after being moved into a concurrent task` | 8-8 |
| §3.1 `*ident(v) returns …; return v` | **compiles**, runs clean, prints 3, one drop | — | 8-6 |
| bind of aggregate element `row is grid.get(0)` | compile error | `cannot bind aggregate element` | 8-7 |
| whole-struct read while field moved out | compile error | `use of partially moved value \`b\`` | 8-7 (exists for `take`; extend to M3) |
| move of a value read by an earlier `defer` | compile error | `cannot move \`buf\`` | 8-7 |
| return of an explicitly-borrowed param | compile error | `cannot return borrowed parameter \`v\`` | 8-6 |
| use in parent after task capture (`log(shared.length)` after a single `dispatch pusher(shared, 0)`) | compile error | same as M8 | 8-8 |

## 8. Consequences, stated so they are not lost

- **No reference cycles are constructible.** Every aggregate has exactly
  one owner at any moment and no shared handles exist, so an ownership
  cycle cannot be expressed; cycle leaks are impossible by construction
  (this restates access-semantics.md §1 and review §3.4's one upside).
- **No refcount traffic exists on any path**, hot or cold. The only
  atomic refcounts in the system remain the runtime-internal
  `Channel`/`ActorRef` handles.
- **`jinn check` and `jinn build` agree.** The ownership analysis runs in
  both pipelines (8-7); a program `check` passes must not corrupt memory
  when built.
- Drop *placement* (Perceus elision/sinking/fusion/reuse in
  `src/perceus/mir_perceus.rs`) remains a pure optimization over this
  model: it may elide or sink the one drop, never add a second.

## 9. Corrections to `access-semantics.md`

Recorded per the 8-5 DoD review-for-contradictions:

1. §1 "Aliases are read-only views with a known, statically bounded
   lifetime" — was **false** in the implementation for `Vec`/`Map` binds
   (review §3.3). This document makes it true by construction: no binding
   can hold an alias (M1/M3/M4); only statement-scoped borrows alias.
2. §4.1 "Container reads … return a `Borrowed` view aliased to the
   container slot" — narrowed by M4: true in expression position only;
   *binding* an aggregate element is a compile error naming `copy`/`take`.
3. §4.2's move-tombstone paragraph described `take`-only tombstones; M1
   and M3 extend the same tombstones to all aggregate binds. The
   snapshot/restore/union dataflow it references is the machinery 8-7
   builds the single analysis on.

## 10. Conformance

Every M-rule above gets a compile-or-run conformance test in
`tests/memory_model.rs` (extended by 8-7/8-8 as each rule becomes
enforceable; M6/M7 and the nested-scope drop discipline are pinned there
now), following the `tests/access_semantics.rs` pattern: real programs
through `jinnc`, asserting either exact runtime output or the diagnostic
lead line. The §7 table is the checklist; the review pins in
`tests/review_2026_07.rs` flip as their owners land.
