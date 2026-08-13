# Second-class references and slices — design

> **Status: design.** Nothing here is implemented. This is `M-13` in
> [`../roadmap.md`](../roadmap.md#memory-and-ownership), and it is the honest
> fix for `M-4r` (element reads deep-copy) and the enabler for lending
> iterators and zero-copy sub-slices.

The premise stays fixed: **no lifetime syntax, ever.** Jinn's borrows today
are invisible because they are statement-scoped — a method call or field read
borrows for exactly one statement, so nothing needs a name. This design keeps
that property while extending borrows to three positions the language cannot
express today: a parameter that lends, an expression that views, and an
accessor that yields. The trick that makes lifetimes unnecessary is
*second-classness*: a view can flow **down** (into calls, into expressions,
into loop bodies) but never **out** (into a struct field, a container, a
return value, a channel, a task, or a captured environment). A value whose
every consumer sits below its producer on the stack needs no lifetime
annotation — the stack *is* the lifetime.

## Surface

No new keywords in user code. Views are inferred exactly where copies happen
today:

- **Sub-slice views.** `xs.view(a, b)` produces `View of T` — a borrowed
  window (`ptr + len`) into `xs`'s buffer. `s.view(a, b)` does the same for
  string bytes. The existing `slice` stays as the copying form.
- **Element views.** `xs.at_view(i)` and map `m.get_view(k)` produce a view of
  one element where `get` deep-copies (`M-4r`). Reading a scalar field from a
  view is free; calling a *read-only* method through it operates on the
  original, not a copy.
- **Lending parameters.** A parameter used only for reading already borrows;
  a `View of T` parameter extends this to windows: `sum(v as View of i64)`
  accepts a whole `Vec`, a sub-slice view, or an array — the callee cannot
  tell and cannot keep it.
- **Yield accessors.** `for x in xs.views()` binds `x` to a view per element —
  the lending-iterator shape that `Iter` cannot express today because `next`
  must return an owned value.

## Typing rules

`View of T` is a type but a restricted one. The compiler rejects, with the
place named in the diagnostic:

1. storing a view in a struct field, enum payload, container, or store;
2. returning a view or yielding it out of a generator whose consumer outlives
   the frame (`views()` is compiler-provided, not user-definable, until
   generators get their own audit — `M-16`);
3. sending a view across a channel, into an actor, or capturing it in a
   `dispatch`/`together`/`spawn` block;
4. rebinding a view to outlive its statement... except a plain `bind`, which
   is allowed and scope-checked: the view dies at the end of the enclosing
   block, and the borrowed root is locked (below) until then.

Rules 1–3 are one check: **a view type may not appear in any position the
escape analysis classifies as escaping.** The existing consuming-parameter
scan already computes exactly this; views reuse it with the polarity flipped
(escape of a view = error, not = consuming).

## Interaction with the place lattice

A live view is an entry in the same structure iteration borrows use since
[146] (`iter_borrowed`, one lattice: `src/typer/place.rs`): creating a view of
`xs.items` pushes the place `xs.items`; any overlapping mutation, move, or
consuming call while the view is live is rejected with the existing wording
("while ... is borrowed by the view bound at ..."). This is the exclusivity
axiom applied to a named borrow instead of an anonymous statement borrow —
no new analysis, a wider trigger.

## Representation and codegen

`View of T` is `{ptr, len}` by value — two words, trivially droppable, no
drop obligation, `T3` tier in the escape analysis. Creating a view is pointer
arithmetic; indexing through one is a bounds check against `len`. Because
views cannot escape, reallocation hazards reduce to the mutation-while-
borrowed rule above; there is no aliasing UB surface as long as rule 4's
root-locking holds.

## What this buys, concretely

- `M-4r` closes: nested-container reads stop paying hidden O(n) copies, and
  read-only method calls on element reads stop operating on silent copies.
- Zero-copy parsers (`csv`, `json`, `url`) — today every token is a `String`
  copy of a slice.
- Lending iteration over large elements without per-step copies.

## Sequencing

1. `View of T` type + creation methods + escape rejection (no lattice work).
2. Root-locking through the place lattice; bind-position views.
3. `views()` lending iteration.
4. std adoption sweep (`strings`, `csv`, `json`, `sort`) with benchmarks.

Each step is shippable alone; step 1 already unlocks lending parameters.
