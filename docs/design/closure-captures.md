# Closure and generator capture rules — specification

> **Status: step 1 implemented ([148]).** When this was specified ([147]) it
> assumed capturing closures did not parse; probing showed they parsed *and
> ran* — capturing the enclosing frame by alias, with a use-after-free on any
> capture the frame invalidated. [148] made the rules below real: captures
> classify by category (scalars copy, `String`s clone, aggregates move through
> the place lattice), the closure value is an aggregate owning its
> environment, and environments carry their own drop function. Pinned by
> `tests/closure_captures.rs`. [149] added the caps edge (step 2: calling
> through a function-typed parameter inside a `needs`-annotated function
> derives a conservative indirect-call taint with the introduction path
> named) and the creation half of step 3 (a generator's aggregate arguments
> are inferred consuming, so the frame owns them — the aliasing SIGSEGV is a
> use-after-move error now). Remaining: suspended-frame drops for generators
> dropped mid-iteration, by-view capture (step 4), and per-iteration closure
> temporaries in loops, which still leak their environments — tracked as
> `O-7` in [`../roadmap.md`](../roadmap.md#memory-and-ownership). Lambda
> syntax is `|x| x * 2`; a capturing closure is any lambda with free
> variables.

## The rule, in one sentence

**A closure captures exactly like a task captures**: values copy, aggregates
move, and there is no capture by reference — because a closure, like a task,
is a value whose call site is unknowable, so anything it holds must be owned
by it.

This is deliberately the most conservative point in the design space. Every
relaxation below is listed with what it would need; none is required for
closures to ship.

## Capture semantics

At closure creation (`f is |…| body`), every free variable of `body` is
classified by the same category rules as assignment:

- **scalars and `String`** — copied into the environment. The original stays
  usable. Mutating the copy inside the closure does not affect the original
  (and vice versa); the mutation inference treats the environment slot like a
  local.
- **aggregates** (`Vec`, `Map`, structs/enums containing one) — **moved**
  into the environment. The original is tombstoned through the same place
  lattice as any move (`MoveReason::ClosureCapture`, naming the closure bind
  and the capture site), so use-after-capture is the existing use-after-move
  diagnostic. `copy x` at the capture site captures a clone instead.
- **second-class views** ([`second-class-refs.md`](second-class-refs.md)) —
  **rejected**. A view may not outlive its statement through an environment;
  the diagnostic points at the captured view and suggests capturing the
  owner or a copy.
- **frozen values** ([`freeze.md`](freeze.md)) — captured by pointer iff the
  closure itself provably does not escape the frozen value's scope, i.e. the
  closure is subject to the same joined-before-scope-exit escape check as a
  task capture. Otherwise moved like an aggregate.

The environment is a struct; the closure value is `{fn ptr, env ptr}` and is
itself an **aggregate** (it owns its environment): assigning a closure moves
it, dropping it drops the environment, sending it to a task moves it with all
captures — no shared environments, ever. Two closures never alias one
environment; capturing the same aggregate in two closures is a double-move
and already an error under the lattice.

## Mutation from within

A closure may mutate its owned captures freely — they are its locals. It
cannot mutate the enclosing function's variables, because it never holds
them: what looks like "mutating a captured variable" in other languages is,
under move-capture, mutating a value the closure owns and the outer scope
has already lost. The diagnostic for the common mistake (capture, mutate,
then read the original expecting the change) falls out of the move: the
outer read is a use-after-move that names the capture.

Shared mutable state across a closure boundary is what actors are for; the
docs pattern points there rather than at cells or refcounts, which do not
exist and are not planned.

## Generators

A generator's frame is a closure environment plus a resume point, so the
same rules apply verbatim: creation classifies free variables, aggregates
move into the frame, the frame is single-owner, and dropping a suspended
generator drops its captures at the suspension point (the drop-obligation
verifier's leak side must count suspended frames — extend `O-2`'s dataflow
across yield edges before generators ship). `yield` of a view is rejected
(second-classness); `yield` of an aggregate moves it out of the frame.

## Cross-task interaction

Because closures are aggregates with owned environments, the existing task
isolation rule needs no special case: a closure moves into at most one task.
The capture-time move already guaranteed the environment shares nothing with
the spawning frame. `needs`/capability inference must treat an indirect call
through a closure-typed value as the join of all closures whose creation
sites flow to it (the caps pass's name-bucket strategy, applied to closure
literals); until that flow analysis exists, calling a closure inside a
`needs`-annotated function derives the conservative top row and the
diagnostic says why.

## Relaxations, priced

| Relaxation | What it needs |
| --- | --- |
| by-view capture for non-escaping closures | escape proof that the closure dies in-frame — the second-class rules give exactly this; do it *after* views ship |
| capturing a slot the closure and frame share | reference cells or refcounts — rejected by the memory model's charter |
| environment sharing between closures | refcounts — same rejection |

## Sequencing

1. Parse capturing closures; classification + move-capture through the place
   lattice; closure value as aggregate. (Blocked only on parser work.)
2. Escape/caps integration: closure-typed call edges in the caps fixpoint.
3. Generators: frame captures + suspended-frame drops (`O-2` extension).
4. By-view capture for provably in-frame closures, now that views exist.
