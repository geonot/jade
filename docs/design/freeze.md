# `freeze` and shared immutables — design

> **Status: step 1 implemented ([148]).** The `freeze` expression,
> `Frozen of T`, the structural freezability check, auto-deref reads, and the
> full write rejection are in the compiler and pinned by `tests/freeze.rs`.
> The multi-capture exception (every `dispatch` in a `together` sharing one
> frozen value — step 2) and std adoption (step 3) remain design — tracked as
> `M-14r` in [`../roadmap.md`](../roadmap.md#memory-and-ownership). Until
> step 2 lands, a frozen value moves into at most one task, like any
> aggregate.

Large read-only data — configuration, lookup tables, model weights — must
today be copied into every task that reads it or funnelled through one actor.
Both defeat the point: the copy costs memory and startup time; the actor
serializes reads that have no reason to serialize. The missing primitive is a
**one-way transition to deep immutability** that makes sharing across tasks
safe *because nothing can ever write again*, with the sharing bounded by a
scope so no reference counting is needed.

## Surface

```
cfg is load_config()
frozen is freeze cfg
together
    dispatch worker(frozen, 1)
    dispatch worker(frozen, 2)
```

`freeze x` consumes `x` (a move, checked like any move — `x` is tombstoned)
and produces a `Frozen of T`. A `Frozen of T`:

- is passed to any number of tasks **within a scope** without copying —
  `together` is the scope supplier; `dispatch` blocks inside it may all
  capture the same frozen value, the one exception to "an aggregate moves
  into at most one task";
- supports every read `T` supports — field reads, element reads, iteration,
  read-only methods — through the same syntax (auto-deref in receiver and
  read positions);
- rejects, at compile time, every write: mutating methods, field assignment,
  element assignment, passing to a parameter the mutation inference marks
  mutating, `take` of any place under it, and `freeze`-then-send to an actor
  handler that writes.

There is no `thaw`. One-way is what makes the model checkable without
tracking readers: a frozen value can never race because there is no state to
race on and no transition back.

## Why no refcount is needed

The scope rule carries the whole weight: a frozen value is created in some
frame, and tasks that share it are joined **before that frame exits** —
`together` already guarantees children are joined at block end, and
`dispatch` outside a `together` joins at function exit. The owner therefore
strictly outlives every reader, and one drop site (the owner's scope end)
frees it. This is the same reasoning that lets second-class references
([`second-class-refs.md`](second-class-refs.md)) skip lifetimes: consumers
sit below the producer on the (task-join) stack. The two designs share the
enforcement point — escape analysis classifies the frozen value's captures,
and the typer rejects any capture whose task is not joined within the
producing scope (a `spawn` that outlives the frame, storing the frozen value
in an actor field, sending it over a channel that escapes).

## Typing and representation

`Frozen of T` is a distinct type, not a flag on `T` — the write rejection is
then ordinary method/assignment resolution (no flow analysis), and APIs can
demand immutability by taking `Frozen of T`. Representation is `T`'s
representation unchanged; `freeze` is a no-op at runtime (a move). Deep
immutability is enforced structurally: `freeze` requires `T` to be built of
freezable parts — scalars, `String`, `Vec`, `Map`, structs and enums of the
same, recursively. `@resource` types, channels, actor refs, coroutines, and
open stores are not freezable, and the diagnostic names the offending field
path.

Task capture codegen passes the pointer, not a copy — the single change on
the runtime side. The scheduler needs no awareness: joined-before-scope-exit
is a compile-time property.

## Diagnostics

- write attempt: "`cfg` is frozen: `set_debug` mutates its receiver, and a
  frozen value can never be written; mutate before the `freeze`, or rebuild
  a new value and freeze that".
- escape attempt: "`frozen` is shared with a task that may outlive the scope
  that owns it; move the `freeze` up to a scope that outlives the task, or
  give the task its own copy".

## Sequencing

1. `Frozen of T` type, `freeze` expression, structural freezability check,
   write rejection. Shippable alone — already useful single-task for
   API-enforced immutability.
2. Multi-capture exception for joined tasks (`together` first, function-exit
   `dispatch` second) with the escape check.
3. std adoption: `config`-style loaders return frozen values; docs pattern
   for model-weight sharing.
