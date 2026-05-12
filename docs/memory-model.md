# Jinn Memory Management: A Deep Dive

> How the Jinn compiler decides who allocates what, who frees it, and why
> the "classical Perceus wins" don't show up in our pipeline statistics —
> not because Perceus failed, but because the language was already paying
> a different (cheaper) bill.

This document explains the actual memory model of the language as
implemented in the `jinnc` codebase, the role of reference counting,
the semantics of move and borrow, where heap allocations come from,
where they go, and why the Perceus pass logs you see (`0 drops elided,
0 drops fused, 0 borrows promoted`) are *not* a sign of failure but a
direct consequence of a fundamentally different ownership discipline
than the one Perceus was originally designed for.

---

## 1. The Mental Model in One Paragraph

Jinn is **move-by-default for owned heap data, borrow-by-default for
parameters, with explicit `rc(...)` for shared ownership.** Every
owned heap object has exactly one live binding at any program point;
when that binding goes out of scope, the typer inserts a `Drop`, and
codegen lowers it to a deep destructor call. Parameters do not bump
refcounts because they are *borrowed* by the callee, not co-owned.
Reference counting only enters the picture when the user writes
`rc(x)` (and friends like `rc_retain` / `rc_release`); everything
                            ,,,,,,,,,,,move semantics than to Koka's reference-counted model — and that is
exactly why Perceus's headline optimizations have nothing to do here.

---

## 2. Who Allocates What

There are five kinds of heap residents the compiler emits code for:

| Construct | Layout | Allocated by | Freed by |
| --- | --- | --- | --- |
| `String` | `{ptr, len, cap}` 24-byte header + UTF-8 buffer | `string_new` / literal lowering | `drop_string` (frees buffer + header) |
| `Vec(T)` | `{ptr, len, cap}` 24-byte header + element buffer | `emit_vec_new` (`InstKind::VecNew`) | `drop_vec_deep` (drops elements, frees buffer + header) |
| `Map(K,V)` | runtime hashmap header | runtime `map_new` | `drop_map_deep` |
| `Rc(T)` | `{strong, weak, payload}` header + payload | `rc_alloc` (only when user writes `rc(...)`) | `rc_release` decrements; frees on `strong == 0` |
| `Coroutine(T)` | runtime stack + frame | `coroutine_spawn` | `coroutine_destroy` |

Stack values (i64, f64, bool, fixed structs/tuples by value, raw
pointers) are not in this table. They live in LLVM `alloca` slots
or SSA registers and are reclaimed by the function epilogue. The
typer's [`needs_drop`](../src/typer/lower/block.rs) function is the
single source of truth for what counts as "non-trivial":

```text
needs_drop(t) := matches!(t, String | Vec(_) | Map(_,_) | Rc(_) | Weak(_) | Coroutine(_))
```

If `needs_drop` returns false, no drop is ever emitted, no destructor
ever runs, no codegen path is taken — the value is bit-copied.

---

## 3. Who Frees What: The Drop Pipeline

The lifecycle of a heap-owned value travels through three IR layers:

1. **HIR**: at the end of every block, the typer walks the local
   scope and emits `hir::Stmt::Drop(var, ty)` for every binding
   that (a) `needs_drop`, (b) is not borrowed, and (c) has not
   moved into the block's tail expression or return.
2. **MIR**: `mir::lower::stmt` translates `Stmt::Drop` to
   `InstKind::Drop(value, ty)`. After MIR construction, the
   Perceus pass pipeline runs and may rewrite some drops into
   reuse-pairing slots, sink them, fuse them, or elide them.
3. **Codegen**: `mir_codegen::emit_inst` dispatches `Drop` to
   `drop_value`, which fans out by type to `drop_string`,
   `drop_vec_deep`, `drop_map_deep`, `rc_release`, etc. Each of
   these is a non-trivial LLVM IR sequence that may itself
   recursively drop element heaps (e.g. `Vec<String>` walks the
   buffer and drops each `String`).

The crucial property: **drops are scope-based, not refcount-based.**
A `Vec<I64>` going out of scope at a block's end results in exactly
one `Drop` instruction in MIR, which becomes exactly one `free` of
the data buffer plus one `free` of the header at runtime. There is
no per-element retain/release traffic and no atomic counter to
update. The cost of "owning a vec" is the cost of allocating and
freeing it — period.

---

## 4. Move-by-Default for Heap Owners

Consider:

```jinn
*main() returns i32
    s is "hello"      # s: String, allocated on heap
    t is s            # MOVE: t now owns the buffer; s is dead
    log(t)
    0
```

In HIR/typer terms, the assignment `t is s` is a **move**: the
ownership transfers and `s` is removed from the live-set. No
`Drop` for `s` is emitted because its memory has merely been
re-named — it now lives at `t`. Only `t` is dropped at end of
scope.

This is the same discipline as Rust without the borrow checker's
compile-time enforcement of "no aliased mutation". In Jinn the
guarantee is provided by the language being intentionally narrow:
there is no `&mut T` that aliases — mutation goes through
borrowed-mut parameters whose lifetime is bounded by the call.

There is no refcount bump on the move. There is no refcount bump
on `t = s`. There is *nothing for Perceus to elide*.

---

## 5. Borrow-by-Default for Parameters

```jinn
*sum(v: Vec<I64>) returns I64
    s is 0
    for x in v
        s is s + x
    s
```

The parameter `v` is **borrowed**, not co-owned. Concretely the
typer lowers `Type::Vec(I64)` parameters to `Type::Ptr(Vec(I64))`
in the callee's signature when the call site doesn't explicitly
hand off ownership. This means:

- The caller retains the responsibility to `Drop` the `Vec`.
- The callee never bumps a refcount on entry, never decrements one
  on exit.
- The callee's parameter does not appear in the callee's drop set
  at function exit.

This is the dual of move semantics: where a rebind moves ownership
along the assignment chain, a function call lends it for the
duration of the call. Together they ensure that **at every program
point, every heap-owned value has exactly one binding that is
responsible for its eventual `Drop`**.

Compare this to a refcounted language where every parameter pass
is `incref(p); ...; decref(p)`. Perceus's signature trick — eliding
`incref/decref` pairs when the analysis proves the callee doesn't
escape the value — is *literally inapplicable to Jinn parameters*
because no `incref`/`decref` was ever emitted.

---

## 6. Where Reference Counting Lives

`Rc(T)` exists, but it is **opt-in**. The user must write:

```jinn
shared is rc(big_thing)        # explicit Rc(BigThing) wrapper
clone  is rc_retain(shared)    # explicit refcount bump
rc_release(clone)              # explicit decrement (or scope drop does it)
```

The lowering for `rc(x)` is `BuiltinFn::RcAlloc`, which:

1. Allocates a `{strong: u32, weak: u32, payload: T}` header.
2. Initializes `strong = 1, weak = 0`.
3. Stores `x` in the payload (consuming `x` if it's heap-owned).

After that, an `Rc(T)` binding behaves like any other heap-owned
value: it gets a `Drop` at end of scope, lowered to `rc_release`,
which decrements `strong` and frees on zero.

**Rc parameters are also borrowed.** A function `*f(r: Rc<T>)` does
not bump on entry and does not decrement on exit. The callee sees
an unmodified `strong` count throughout. To take a real shared
copy across an actor boundary or into a closure, the user explicitly
calls `rc_retain`. This is the design decision that keeps the
runtime-RC code paths so quiet: they only execute when the program
explicitly opts into shared ownership.

---

## 7. Why the Perceus Stats Look Empty

Perceus, as published, optimizes a refcounted runtime by:

1. **Elision**: deleting `incref(x)` followed shortly by `decref(x)`
   when `x` is provably unused in between.
2. **Sinking / fusing**: pushing decrements past instructions that
   can't observe the refcount, then merging adjacent pairs.
3. **Borrow promotion**: replacing an owned reference with a
   borrowed one when the use does not need to extend ownership.
4. **Reuse**: pairing a final `decref` (which frees) with a
   subsequent allocation of the same shape, replacing both with
   an in-place rewrite of the soon-to-be-freed block.

The first three optimizations operate on `incref`/`decref`
instructions. **Jinn emits none implicitly.** `rc_retain` and
`rc_release` only appear in MIR when the user writes them or when
a drop falls on an `Rc` binding. There is therefore nothing to
elide, nothing to sink, nothing to fuse, and the borrow-promote
analysis has no candidate sites because owned uses are already
the minimum.

The fourth optimization — **reuse** — *does* apply, and that is
where Jinn now spends its Perceus budget. The recently landed
`vec_reuse_pairing` pass identifies the canonical loop pattern:

```jinn
while ...
    v is vec()        # alloc
    ...mutate v...
    # implicit Drop(v) at end of iteration
```

and pairs the iteration-end `Drop(Vec(T))` with the next iteration's
`vec()` literal. A pair becomes a "reuse slot": the drop stashes
the header (after deep-dropping elements but preserving `cap` and
the data buffer), and the matching alloc consumes that slot
instead of calling `malloc(24)` and growing a fresh buffer. After
the first iteration there are zero allocations and zero frees in
the body — exactly the Perceus paper's "in-place update" outcome,
but expressed in terms of *single-owner moves* rather than
*refcount-of-one*.

The `mir-perceus: ... 1 reuse pairs` line on a benchmark like
`/tmp/vecreuse.jn` is the visible result. The fact that the same
line reports `0 borrows promoted` is not a bug — it is the model
working as designed.

---

## 8. The "Are We Leaking?" Question

We are not. The audit (recorded in repo memory) verified:

- Every type for which `needs_drop` is true gets a scope `Drop` in
  HIR unless it has moved out of scope.
- Move analysis (`collect_moved_var_ids`) covers the constructs
  that legitimately consume a value — return / tail position,
  struct-init, tuple-init, bare variable references that are
  passed by-move at a call site that takes ownership.
- Function-call parameters that are borrowed do not produce a
  callee-side `Drop`; they produce a caller-side one when the
  caller's binding goes out of scope.
- The MIR Perceus passes never delete a `Drop` — they only
  *reassign* it to a reuse slot, which pairs it with a later
  allocation that will free the slot's contents on the next
  store or on `drain_reuse_slots` at function exit.
- `drain_reuse_slots` runs immediately before every `Return`,
  freeing whatever is still parked in any reuse slot. For Vec
  slots it frees both the buffer and the header.

The pipeline is self-balancing: every `malloc` and every
`rc_alloc` has a matching free reachable from every program exit
point. There is no path from "obj allocated" to "function
returns" that does not free obj exactly once.

---

## 9. What Optimizations *Are* Live

Given the design, the optimizations that actually pay off in
Jinn are different from Perceus's headline four:

1. **Reuse pairing** for `Vec` (live), `String` (next up), and
   in principle `Map` and `Rc(T)` of homogeneous shape.
2. **Drop sinking past pure code** — moving a `Drop` later in a
   block when the value isn't read, so the released memory is
   warm for whatever runs next. This is implemented and reported
   as `drops sunk`.
3. **Drop fusion** — collapsing back-to-back drops of the same
   value (a defensive pass; rare in practice).
4. **Drop elision** — when downstream control flow proves a value
   has no live use after a transfer, the redundant `Drop` is
   removed. Rare in Jinn because the typer already minimizes
   drops at HIR-construction time.

The result is a memory model that is predictable, single-owner,
and competitive with Rust's on the hot path, while remaining
compatible with explicit `Rc` for the genuinely-shared cases. The
runtime cost of "having garbage collection" is precisely zero
because there is no garbage collector — there is a typer that
knows where every heap pointer is born and where it dies.

---

## 10. TL;DR for the Reader Skipping to the Bottom

- Heap owners (`String`, `Vec`, `Map`, `Rc`, `Weak`, `Coroutine`)
  are tracked by ownership; everything else is bit-copied.
- Assignment is **move**; parameter passing is **borrow**.
- Refcounts only happen when the programmer writes `rc(...)`.
- Drops are emitted by the typer at scope boundaries and lowered
  to deep destructor calls in codegen.
- Perceus's `incref/decref` elision optimizations are *not* dead
  code — they are inapplicable, because the instructions they
  optimize are never emitted in the first place.
- Perceus's *reuse* optimization is alive and well: the
  `vec_reuse_pairing` pass turns loop bodies that allocate and
  drop a vec each iteration into in-place updates with zero
  steady-state malloc traffic.
- We are not leaking. The drain-on-return invariant guarantees
  every reuse slot is emptied before control leaves the function.

---

## 11. So Are the Perceus Passes Pointless?

No, but the honest answer is more nuanced than "they're great."

The pass pipeline as it stands consists of seven passes:
**use analysis**, **drop elision**, **drop sinking**, **drop fusion**,
**Rc reuse pairing**, **Vec reuse pairing**, and **borrow promotion**.
Of these:

- **Vec reuse pairing** is genuinely productive — it turns the
  iconic `while … v is vec() … end` loop into in-place updates.
  This is the optimization people picture when they hear "Perceus".
- **Drop sinking** has marginal value. It moves drops past
  read-only instructions to keep released memory warm; LLVM
  often makes this irrelevant after instcombine.
- **Drop fusion** is defensive — it collapses adjacent
  `Drop(x); Drop(x)` pairs that should never be emitted, and
  in practice never fires.
- **Drop elision** rarely fires because the typer already
  minimizes drops at HIR construction time.
- **Borrow promotion** is currently dormant. It targets implicit
  `incref(x)` / `decref(x)` traffic that this language never emits,
  so the analysis correctly reports "no candidates". If we ever
  introduce *implicit* `Rc` (see §15) it becomes the central
  optimization.
- **Rc reuse pairing** activates only when the user writes
  `rc_release(x)` followed by `rc(...)` of the same shape.
  Real programs hit this rarely.
- **Use analysis** is shared infrastructure feeding all of
  the above; the cost is one MIR walk per function.

So the *passes* aren't silly — but the *naming* is. They are
named for Perceus optimizations because the pipeline borrows
Perceus's reuse-token discipline. In a language with
move-by-default, what's running is closer to "scope-drop
optimization with reuse pairing," and most of the pass slots
are reserved for futures that may or may not arrive (implicit
Rc, finer escape analysis, etc.).

The decision to keep them all wired is deliberate: zero compile-
time cost when they don't fire, and they ship the framework
needed for future optimizations that *do* depend on richer
ownership annotations.

---

## 12. Why Don't Other Languages Use This Model?

They do — but with sharp edges sanded off in different ways.

| Language | Ownership | Borrow checking | RC | Notes |
|---|---|---|---|---|
| Rust | Affine, move-by-default | Compile-time, with lifetimes | Opt-in `Rc<T>` / `Arc<T>` | Strict, very safe, steep learning curve |
| C++ (modern) | Move-by-default | None enforced | `shared_ptr` | Easy to misuse; UAF is on you |
| Swift | Reference type by default | None | Implicit ARC | Slow refcount traffic; uniqueness checks |
| Koka | RC everywhere, Perceus-optimized | None | All-pervasive | Perceus's home turf |
| Vale | Generational refs, regions | Compile-time | None | Bleeding edge; complex |
| Hylo / Val | Mutable value semantics | Region-based | None | Closer to Jinn's spirit |
| Lobster | Move + automatic Rc fallback | Static analysis chooses | Inferred | The closest cousin |
| **Jinn** | **Move-by-default for heap, borrow-by-default for params** | **Lightweight verifier (no lifetimes)** | **Explicit `rc(…)`** | **Practical compromise** |

The genuinely uncommon thing Jinn does is *implicit borrowing of
parameters without surface-syntax lifetime annotations*. Most
languages either:

1. Demand annotations (Rust's `&'a T`),
2. Fall back to refcounting on every parameter pass (Swift, Koka),
3. Pretend everything is value-copied (functional purists), or
4. Leave it to the programmer (C/C++).

Jinn's choice — "borrowed unless explicitly handed off" — works
because the language doesn't (yet) expose general-purpose alias
mutation. Without `&mut T` aliasing concerns, the verifier can be
small and the user can ignore lifetimes. That simplification is
also Jinn's ceiling: the day someone wants two mutable references
into the same `Vec`, the model has to grow lifetimes or
runtime checks.

---

## 13. The `a is 1; b is a` Question (and Friends)

> Q: If assignment is move, doesn't `a is 1; b is a; print(a); print(b)`
> use `a` after it's been moved out?

**A: No, because `a` is `I64`, which is *trivially droppable*.**

The `OwnershipVerifier` (`src/ownership/mod.rs`) records a move
*only when* the type is non-trivially droppable:

```rust
if (state.ownership == Ownership::Owned || state.ownership == Ownership::BorrowMut)
    && !state.ty.is_trivially_droppable()
{
    s.moved = true;
}
```

`I64`, `F64`, `Bool`, fixed-size `Tuple` of trivials, raw pointers
— all bit-copied on assignment. The "move" terminology only
applies to heap owners. The example above is not a move; it is two
copies of a 64-bit integer, and both prints work.

The same example with strings is different:

```jinn
a is "hello"        # a: String (heap-owned)
b is a              # MOVE: b owns the buffer
log(a)              # ❌ ERROR: use of moved value `a`
log(b)              # OK
```

The verifier emits `DiagKind::UseAfterMove` for the third line.
This is the one place where Jinn's behavior visibly diverges from
"every language ever invented" and where it matches Rust.

The pragma to remember: **POD copies, heap moves, params borrow**.

### What about partial moves?

```jinn
pair is (mk_string(), 42)
first is pair.0     # moves the String out
log(pair.1)         # OK — i64 was bit-copied
log(pair.0)         # ❌ moved
```

Tuple/struct field moves are tracked at sub-field granularity
where possible; the verifier's coverage of nested patterns is
deliberately conservative — it errs on the side of "too
restrictive" rather than "missed UAF".

### What about reassignment?

```jinn
s is "first"
s is "second"       # implicit Drop("first") then store "second"
log(s)              # OK — s now owns "second"
```

Rebinding to the same name is *not* a move-then-use; the typer
inserts a `Drop` of the previous value at the assignment point.

---

## 14. Real-World Patterns Worked Through

### 14.1. Building a string from parts

```jinn
*greet(name: String) returns String
    msg is "hello, "
    msg = msg + name      # rebinding; old "hello, " dropped, new String allocated
    msg = msg + "!"
    msg                   # tail-position move into return slot
```

What allocates: `"hello, "` (literal), the concat result (×2),
the return value. What frees: each intermediate before the next
rebind. Caller of `greet` receives the moved-out `msg`.
**Optimization opportunity**: a future *string reuse* pass could
recycle `msg`'s buffer across the two concats — analogous to
the Vec reuse already shipped.

### 14.2. Looping over a borrowed Vec

```jinn
*sum(v: Vec<I64>) returns I64
    s is 0
    for x in v          # v is borrowed; iteration reads buffer
        s = s + x
    s
```

Zero allocation in the body. `v` is `Ptr(Vec(I64))` at the MIR
level — no incref, no decref. Caller still owns and will drop.

### 14.3. Filling a Vec then returning it

```jinn
*range(n: I64) returns Vec<I64>
    v is vec()
    i is 0
    while i < n
        v.push(i)
        i = i + 1
    v                   # tail-position move; not dropped here
```

`v` allocates the header on entry and grows the buffer as
`push` doubles capacity. The tail move means the typer skips
the `Drop(v)` and the caller becomes responsible. Reuse
pairing does not fire here (the move escapes the function).

### 14.4. Hash map of strings

```jinn
m is map()
m.set("a", 1)
m.set("b", 2)
total is m.get("a") + m.get("b")
log(total)
# implicit Drop(m): walks entries, drops each key String, frees buckets, frees header
```

Each `set` may allocate a key buffer (the string `"a"` is a
literal whose ownership transfers into the map). The deep drop
on `m` releases every key and every value's heap footprint.

### 14.5. Shared state across actors

```jinn
*main()
    state is rc(make_state())     # explicit Rc
    spawn worker(rc_retain(state))
    spawn worker(rc_retain(state))
    # implicit Drop(state) on the original = rc_release: count goes 3→2
    # workers each release on exit: 2→1→0 → free
```

This is when explicit `Rc` earns its keep. Without it there is no
way to give two actors ownership of the same heap object. The
codegen path here uses **atomic** refcount operations because
`Type::needs_atomic_rc()` is true for values that can cross
thread boundaries (actors, channels). Single-threaded `Rc` uses
non-atomic ops — a real perf win Jinn picks up automatically.

### 14.6. Tree / graph with cycles

```jinn
node Parent { children: Vec<Rc<Child>> }
node Child  { parent: Weak<Parent> }   # Weak breaks the cycle
```

Strong + Weak refs are the standard answer to cyclic structures,
exactly as in Rust/Swift/C++. `weak_upgrade()` returns
`Option<Rc<T>>` and the verifier requires you to check for
`none` before use (`DiagKind::WeakUpgradeWithoutCheck`).

### 14.7. Cache of computed values

```jinn
cache is map()
*fetch(key: String) returns Rc<Result>
    if cache.contains(key)
        cache.get(key)              # shares ownership: rc_retain implied? no — explicit
    else
        r is rc(compute(key))
        cache.set(key, rc_retain(r))
        r
```

Caches are the canonical case where explicit `Rc` is mandatory —
the cache itself owns one count, the caller owns another.

### 14.8. Producer / consumer over a channel

```jinn
ch is channel()
spawn producer(ch)
spawn consumer(ch)
```

Channels are runtime-managed and refcounted internally; the
language exposes them as values whose drop closes the channel
when no senders or receivers remain. This is the one place where
the runtime has its own GC-like discipline that's invisible to
the source program.

---

## 15. Do We Need Explicit Rc? Could We Infer It?

**Today:** yes, we need it. The escape analysis is intentionally
conservative; without `rc(...)` there is no way to say "share
this between two actors" or "stash this in a long-lived cache and
also keep using it". The compiler cannot prove safety without an
ownership annotation, so it forces the user's hand.

**Could we infer it?** In principle, yes. The literature points
two directions:

1. **Lobster-style inference**: do whole-program escape analysis;
   if a value provably has a single live owner at all times, leave
   it as move; otherwise wrap it in Rc automatically. Pros:
   ergonomic. Cons: opaque cost model — adding a `spawn` to your
   code can silently rewrite a hot path from move to RC.

2. **Polymorphic ownership**: parameterize functions over an
   ownership mode (`fn foo<O: Owned>(x: T@O)`). Pros: explicit
   and zero-cost. Cons: a second parameter system, more friction.

Jinn currently rejects both for a third position: **explicit
`rc(...)` is a single keystroke and makes the cost visible**.
This is the same answer Rust gives, and for the same reasons:
predictability beats convenience when memory is involved.

That said, two narrow inferences would be safe and helpful:

- **Auto-Arc for actor-bound values**: when the typer sees a
  value being captured by `spawn`, if the value's type is
  trivially shareable, wrap it in `Rc` automatically. Today the
  user sees a "captures owned value" error and has to add `rc(...)`
  manually.
- **Auto-Weak for self-referential closures**: detect when a
  closure captures something owned by its parent scope and would
  cycle, and downgrade to `Weak`.

Both are deferred until the use cases are common enough to
justify the loss of cost transparency.

### Alternatives to Rc altogether

For some workloads, `Rc` is the wrong tool entirely:

- **Arena allocation** for graphs that live and die together —
  one bulk free instead of N refcount decrements. Jinn doesn't
  have an arena type yet; it would be a worthwhile addition.
- **Index-based stores**: instead of `Rc<Node>`, store `Vec<Node>`
  and pass `NodeId` (a `u32` index). Move semantics work
  perfectly, no refcount, no cycles. Pattern is standard in
  game engines and ECS frameworks.
- **Persistent / functional data structures**: structural sharing
  via immutable nodes; one `Rc` per shared subtree, but path
  copies on update. Heavy but allocator-friendly.

A future direction is to provide ergonomic library support for
these patterns rather than encouraging `Rc` as the default escape
hatch.

---

## 16. Strengths, Weaknesses, Gaps

### Strengths

- **Predictable cost**: every allocation has a syntactic origin
  (`vec()`, `"…"`, `rc(…)`); every free has a syntactic scope
  end. No hidden GC pauses, no surprise refcount churn.
- **Single-owner discipline scales**: the verifier is small and
  the rules fit on one page. New users are not blocked by
  lifetime annotations.
- **Compatible with both stack and heap regimes**: trivial types
  are bit-copied at zero cost; heap types follow move semantics.
- **Atomic / non-atomic Rc auto-selection**: pay for atomicity
  only when the value can cross threads.
- **Reuse pairing actually fires**: the iconic Perceus loop
  optimization works in practice and is observable in
  benchmark stats.
- **No GC**: no stop-the-world, no tri-color anything, no
  finalizer pitfalls.

### Weaknesses

- **Aliased mutation is not first-class**: there is no `&mut T`
  that can be split or reborrowed. Patterns that need two
  mutable views into the same container must copy or restructure.
- **Conservative move analysis on nested patterns**: the
  verifier sometimes rejects programs that are actually safe
  because it can't see through deep tuple/struct destructures.
- **No region or arena types**: programs that want arena-style
  bulk freeing must roll their own.
- **Borrowed return is forbidden**: you cannot return `&T` from
  a function (`DiagKind::ReturnOfBorrowed`). To return shared
  state you must `rc_retain` and move out an `Rc<T>`.
- **Implicit Rc would break cost model**: any auto-promotion
  trades predictability for ergonomics, which the design
  currently refuses.

### Gaps (TODO surface)

- **String reuse pass** — analogous to the Vec one; not yet
  implemented but the framework is in place.
- **Map reuse pass** — much harder (variable bucket layout)
  but a worthwhile experiment.
- **Loop-invariant Drop hoisting** — drops inside loops that
  apply to invariant values should sink past the loop entirely.
- **Better escape analysis for actor / channel boundaries** —
  to enable auto-Rc inference where it is provably safe.
- **Region annotations as opt-in** — for the cases where users
  *do* want a region but can't get one today.

---

## 17. Is Jinn Sound?

To the best of the audit's reach, **yes** — with caveats:

1. **Single-owner invariant is preserved**: the typer emits one
   `Drop` per heap binding per scope exit; moves remove the
   binding from the live set; the ownership verifier rejects
   use-after-move at compile time.
2. **No double-free**: a `Drop` is only emitted on the binding
   that holds the unique heap pointer. After a move, the source
   is dead and contributes no `Drop`.
3. **No leaks on normal exit**: every heap allocation reaches a
   matching free along every control-flow path, including the
   reuse-slot drain before `Return`.
4. **Atomic RC for shared cases**: `needs_atomic_rc()` selects
   atomic ops when the value type can cross thread boundaries.
5. **Weak references gate access**: `weak_upgrade()` is
   mandatory before deref, enforced by the verifier.

Caveats and known limitations:

- **Panics / aborts skip user code but free OS memory**: the
  process exits and the kernel reclaims; no destructors fire on
  panic. This is a Rust-style decision (no two-phase teardown).
- **Channel drop order in actor systems**: there is a runtime
  invariant that all senders and receivers must be released
  before the channel's last refcount drops; violations would
  manifest as runtime asserts in the channel implementation,
  not silent corruption.
- **`unsafe` raw pointer ops bypass the verifier**: the
  language exposes raw pointer types for FFI; misusing them is
  on the programmer.
- **Cycle detection is opt-in via Weak**: a program that uses
  `Rc<Rc<…>>` cycles without `Weak` will leak. This is the same
  trade-off as Rust's `Rc` and Swift's strong references.

Soundness is therefore **structural** — it follows from the
design and is checked by the verifier — rather than guaranteed
by an external proof. The codebase ships a substantial test
suite (1,543 passing as of this writing) covering ownership
edge cases, but a formal mechanization is not in scope.

---

## 18. Closing Thought

The fact that the Perceus stat line reads
`0 drops elided, 0 drops sunk, 0 drops fused, 1 reuse pairs,
0 borrows promoted` is not a sign that the optimizer is broken.
It is a sign that the language has chosen *a different way to
be fast*: do less work in the first place by single-owner
moves, then capture the one Perceus optimization that survives
that choice (reuse pairing) where it actually applies (loops
that allocate-and-drop the same shape on every iteration).

The mistake would be to look at the empty stats and conclude
"Perceus doesn't help us, rip it out." The right reading is
"Perceus's *premise* (everything is RC) doesn't apply, but
Perceus's *deepest insight* (a final drop and a fresh
allocation are the same memory event) absolutely does." We
keep the framework, ship the win that does fire, and reserve
the rest of the pipeline for the day a future feature
(implicit Rc, dependent borrows, regions) makes them earn
their keep.
