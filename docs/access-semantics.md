# Access Semantics in Jinn: Copy, Reference, Borrow, Move

> A design analysis of how values flow between containers, variables,
> functions, and stores — what Jinn does today, what other languages do,
> what we should do.

---

## 0. Why this document exists

The trigger was a concrete implementation choice in the compiler. When you
write:

```jinn
msg is queue.get(0)
```

what *is* `msg`?

- A **copy** of the element (Jinn today, after the recent value-clone work)?
- A **reference** into the queue's storage?
- A **borrow** that prevents the queue from being mutated while `msg` lives?
- The element itself, **removed** from the queue?

Jinn currently picks (1) — `.get()` deep-clones clonable types and borrows
opaque ones. That choice is fine for `Vec of i64` and even `Vec of String`,
but it falls apart for the use cases the user raised:

1. **Aliasing handles.** A `Vec of Socket` where iterating means *using* each
   socket. A copy of a socket handle is a second OS-level reference to the
   same FD; closing one closes both. Logical aliasing is the *intent*.

2. **Mailbox / queue pop.** `let msg = mailbox.pop()` — the message must be
   removed from the queue, not copied. The mailbox shouldn't still hold it.

3. **Store query results.** `users is db.query("…")` — these aren't owned by
   the local; they're rows from a persistent backing store. Mutating a copy
   and persisting it competes with anyone else doing the same → lost
   updates, write skew, the usual.

4. **Read-only inspection.** `if vec.get(i) equals target …` doesn't need to
   own anything; cloning a 4KB struct for a comparison is wasteful.

So we need to make access intent *explicit in the model* (even if not always
explicit in syntax) and let the compiler enforce/optimize accordingly. That
means revisiting how every "place where a value moves" works:

- Variable binding (`is`, `=`, shadow)
- Function call (arg passing) and return
- Container insert (`.push`, `.set`, `.insert`)
- Container read (`.get`, `.peek`, `.front`, `.back`)
- Container remove (`.pop`, `.remove`, `.take`)
- Iteration (`for x in xs`)
- Pattern matching binding
- Field access (`s.field`)
- Closure capture
- Channel/actor message send
- Store reads and writes

This document goes through all of those for Jinn today, surveys six
language families, names four canonical access semantics, and proposes a
design.

---

## 1. Jinn today: inventory of access semantics

Jinn's stated principles (jinn.md):

> 2. Ownership is default. One owner per value. Compiler inserts drops statically.
> 3. Borrowing is free. Read access borrows a reference — zero runtime cost.
> 4. Sharing is inferred. The compiler determines when values need shared
>    ownership and inserts reference counting automatically.

So the *intent* is Rust-style affine ownership + auto-inferred Rc. The
*implementation* has diverged in places. Here's the actual state:

### 1.1 The `Ownership` enum (src/hir/mod.rs)

```rust
pub enum Ownership {
    Owned,       // value owns its storage; dropped at scope exit
    Borrowed,    // aliases storage owned elsewhere; NOT dropped
    BorrowMut,   // exclusive borrow; rare in practice today
    Rc,          // refcounted shared pointer
    Weak,        // weak refcounted pointer
    Raw,         // %ptr — unmanaged
}
```

All six modes exist in HIR. Only `Owned`, `Borrowed`, `Rc`, `Weak`, `Raw`
are routinely produced by the typer. `BorrowMut` is essentially unused.

### 1.2 Variable binding (`x is expr`)

In [src/typer/stmt/dispatch.rs](src/typer/stmt/dispatch.rs):

1. Type the RHS.
2. Compute default ownership from the type:
   - `Rc(T)` → `Rc`
   - `Ptr(T)` → `Raw`
   - everything else → `Owned`
3. If RHS is an `is_aliased_read_of_heap` (i.e. `container.get/peek/front/back`)
   AND the type is *not* `is_value_clonable`, override to `Borrowed` so we
   don't double-free.
4. Otherwise: `Owned`. Codegen emits a deep clone where needed (e.g. inside
   `vec.get` for `Vec of String` now goes through `clone_value`).

This means `x is queue.get(0)` is:
- A **clone** for clonable element types (String, struct of clonable, Vec of
  clonable, nested transitively).
- A **borrow** for non-clonable types (Map, Set, Deque, PQ, NDArray, Enum,
  Channel, Coroutine, Generator).

Both behaviors are silent. The user can't tell which they got.

### 1.3 Function parameters

Parameters are typed (`name as Type`). The `ownership` field on each
parameter is set by `ownership_for_type`:

- `Rc(T)` → `Rc` (caller-side bump; argument is an RC bump-aliased pointer)
- `Ptr(T)` → `Raw`
- anything else → `Owned` — i.e. arguments are *moved* into the callee.

Consequences:
- Passing a `Vec` to a function transfers ownership. The caller can't use it
  after the call unless the callee returns it.
- For value-clonable types we currently *do not* auto-clone at the call
  site, so this is a true move. (Compare 1.2: bindings get a clone, calls
  get a move. Inconsistent.)
- `Rc` types are an implicit alias — the compiler does the refcount.

There is no surface syntax for borrowed parameters. Methods get an implicit
`self` that is passed by pointer for mutation (`*` prefix), or by value for
read-only methods.

### 1.4 Method receivers (`self`)

Methods on `type Foo` use a leading `*` to denote mutation:

```jinn
type Counter
    n as i64

    *inc                 # mutating: receives &mut self
        self.n = self.n + 1

    *value returns i64   # non-mutating: but still `*` prefix today
        self.n
```

The `*` prefix on the method name is actually overloaded with "this is a
method" syntax — it doesn't currently encode read vs mutate. Internally the
compiler can choose pointer-pass when needed. The actor/store equivalents
have similar conventions (`*handler`, `*query`).

### 1.5 Container insert

`vec.push(x)`, `map.set(k, v)`, `deque.push_back(x)`, `set.insert(x)`,
`pq.push(x)`: all consume `x` (move semantics). For trivially droppable
types this is a value-copy; for heap types this is an ownership transfer
into the container.

Once `x` has been pushed, it should not be used again. The compiler does
not currently warn on use-after-push of a moved heap value (this is a
known gap — Perceus + drop-tracking would catch it).

### 1.6 Container read

The behavior implemented this week:

| Method                         | Behavior                              |
|--------------------------------|---------------------------------------|
| `vec.get(i)`, `array[i]`       | Deep clone if clonable, else borrow   |
| `map.get(k)`                   | i64-only fast path; pending for others |
| `set.peek` / `peek_min/max`    | Borrow (no clone hook yet)            |
| `deque.front/back`             | Borrow (no clone hook yet)            |
| `pq.peek`                      | Borrow                                |
| `vec[i] = expr` (set)          | Drops old value, moves new in         |
| `for x in vec`                 | x is owned **a clone of** each elem   |

The for-loop case is particularly subtle: `for x in xs` clones every
element. That's correct for safety but disastrous for performance if `xs`
is `Vec of HugeStruct`.

### 1.7 Container remove

`.pop()`, `.remove(i)`, etc. — these are *true moves out*: the element is
removed from the container, ownership transfers to the caller, and the
container's slot is left empty (or shifted). Implementation-wise this is a
memcpy out + length adjustment. No clone.

This is the cleanest part of the system today.

### 1.8 Field access

`x.field` for a value-typed struct copies the field's value out (it's a
load from the struct's memory). For heap-typed fields (`String`,
container, `Rc`), this is a *raw alias* — there is no automatic clone or
RC bump on field read. This is unsound if the alias outlives the struct,
which is one of the latent bugs we keep finding.

### 1.9 Pattern match binding

`when x is Some(v) …` binds `v` to the payload, ownership transferred from
the matched value (`x` is consumed). For non-exhaustive matches with
read-only intent, this also moves — there is no `ref v` syntax today.

### 1.10 Closure capture

Closures capture by ownership: heap types are moved into the closure
environment, primitives are copied. There is no by-reference capture form
yet. `Rc` types get a refcount bump as expected.

### 1.11 Channel / actor send

Sending a value over a channel or to an actor is a *move*. The runtime
copies the bytes; the sender forfeits ownership. This is required for
correctness — the receiver may be on another thread. Non-`Send`-equivalent
types (raw pointers into the sender's stack, etc.) are theoretically
unsound; we don't yet have a `Send` marker but it's implicit-by-type
today.

### 1.12 Store reads

`store.users.where(…).first()` — currently each row is materialized as an
owned value (rows are copied out of the store's storage). Mutating that
row does *not* update the store; you'd need to `store.users.update(row)`.
This is the right semantics (rows are values, not references), but it is
also implicit and unsigned — a user who writes `u.email = "new@…"` on a
query result and expects it to persist is in for a bad time.

### 1.13 Summary table

| Site                          | Semantics today                         | Heap correctness   |
|-------------------------------|-----------------------------------------|--------------------|
| `x is expr`                   | Move (clone if from container)          | OK (recent fix)    |
| `f(arg)`                      | Move                                    | OK                 |
| `x.field`                     | Load (raw alias for heap fields)        | **Latent UAF**     |
| `vec.push(x)`                 | Move                                    | OK                 |
| `vec.get(i)`                  | Clone (if clonable) / Borrow            | OK after fix       |
| `vec.pop()`                   | Move out                                | OK                 |
| `for x in vec`                | Clone per iteration                     | Correct, slow      |
| `match … { Some(v) … }`       | Move                                    | OK                 |
| `closure { … x … }`           | Move-capture                            | OK                 |
| `chan.send(x)`                | Move                                    | OK                 |
| Store query result            | Materialize as owned value              | OK, but surprising |

The diagonal between **"reasonable default"** and **"what the user
actually wanted"** is widest in: container read, for-loop iteration, field
access, store query.

---

## 2. How other languages handle this

Six families. Each gets: model name, what `x = container[i]` means, what
function call does, the cost model, and the tradeoff.

### 2.1 C / C++

**Model:** explicit. Every type has a value form (`T`), a pointer form
(`T*`), and a reference form (`T&` in C++).

- `T x = v[i];` — copy-constructs `x` from `v[i]`. Cost: O(sizeof(T))
  plus deep work if `T` has a custom copy ctor.
- `T& x = v[i];` — `x` is a reference; aliases `v[i]`. No cost, no
  ownership. Dangling if `v` reallocates.
- `T* x = &v[i];` — same, pointer form.
- `T x = std::move(v[i]);` — moves out, leaves `v[i]` in valid-but-empty
  state (rvalue-reference + move ctor).
- Function call: by default by-value (copy); programmer can opt into
  `const T&`, `T&`, `T*`, `T&&`.

**Tradeoff:** maximum control, maximum footgun. Aliasing rules are
*advisory* (UB if violated). Compiler can't help much.

### 2.2 Rust

**Model:** affine ownership + borrow checker + lifetimes.

- `let x = v[i];` — error if `T: !Copy`. Compiler refuses to silently
  copy a heap value out of an indexed location.
- `let x = &v[i];` — shared borrow, lifetime tied to `v`. Multiple
  shared borrows OK; no mutation of `v` while any live.
- `let x = &mut v[i];` — exclusive borrow. Excludes all other borrows.
- `let x = v[i].clone();` — explicit copy, allowed for `T: Clone`.
- `let x = std::mem::take(&mut v[i]);` — move out, leaves `Default::default()`.
- `let x = v.remove(i);` — move out, shifts.
- `for x in &v` (borrow) vs `for x in v` (consume).

Function args: `fn f(x: T)` moves; `fn f(x: &T)` borrows; `fn f(x: &mut T)`
borrows mutably. Programmer writes which.

**Tradeoff:** zero-cost, statically safe, but the user *must* understand
borrows and lifetimes. The mental tax is real — `&self` vs `self`,
"cannot borrow as mutable more than once," etc. The cost of getting the
defaults right is paid by the programmer.

### 2.3 Swift

**Model:** value types (structs) + reference types (classes) + COW.

- Structs (`struct Point { … }`) are value types. `let p2 = points[0]`
  copies. COW is implemented in the standard collections (`Array`,
  `Dictionary`, `Set`, `String`) so the copy is cheap until mutation.
- Classes (`class User`) are reference types. `let u = users[0]` gets a
  refcounted reference (ARC). Multiple owners; mutation visible to all
  aliases.
- `inout` parameters for mutating function args: `func swap(_ a: inout T, _ b: inout T)`.
- `borrow`/`consuming` keywords (recent additions, Swift 5.9+) make
  parameter ownership explicit, though defaults remain copy/ARC-bump.

**Tradeoff:** safety without the syntactic cost of Rust. ARC overhead is
real (atomic ops); COW saves memory but adds an RC check on every
mutation. "Is this a struct or a class?" determines aliasing — easy to
get wrong because the syntax for use is identical.

### 2.4 Go

**Model:** value types + pointers + GC.

- `x := v[i]` always copies. Slices are pointer/len/cap triples so
  `slice = otherSlice` is a *shared view* into the same backing array —
  the canonical Go footgun.
- Pointers (`*T`) are explicit. `x := &v[i]` gives you an interior
  pointer. GC keeps the backing array alive.
- Map index returns a value-typed copy; you cannot take `&m[k]`.
- Channels: send is a copy; for pointers, the pointer itself is what's
  copied (so aliasing across goroutines is on the programmer).

**Tradeoff:** GC removes the "drop / borrow" concerns; everything is
either a copy or a pointer. Aliasing semantics are clear at the type
level (T vs *T) but slices break the rule and everyone trips on them
once.

### 2.5 Python / JavaScript / Ruby / most dynamic languages

**Model:** all values are references; assignment is pointer copy.

- `x = arr[0]` binds `x` to the same object the array's slot points to.
  Mutating `x.field = …` is visible through `arr[0]`.
- "Copy" is opt-in (`copy.copy`, `copy.deepcopy`, spread `{…obj}`,
  `Array.from`).
- Function call is reference-pass for objects; primitives are
  pass-by-value (in Python/JS small ints are interned).
- No ownership at all; GC handles lifetimes.

**Tradeoff:** trivial to use, hostile to reasoning. Performance is
GC-bound. Aliasing bugs are common but rarely catastrophic (no UAF).

### 2.6 Functional languages (Haskell / OCaml / Erlang / Clojure)

**Model:** immutable by construction. The question dissolves.

- `let x = head xs` — there's no question of copy vs reference because
  `xs` cannot change.
- Updates produce new values; the runtime uses persistent data
  structures (HAMTs, finger trees) so updates are O(log n) with sharing.
- Aliasing is unobservable.

**Tradeoff:** the cleanest semantic story. Costs: log-factor on every
update, no in-place mutation, requires committing to the immutable
paradigm. Erlang/BEAM specifically copy values across processes, which
is exactly what gives them their fault-isolation guarantees.

### 2.7 Lessons distilled

1. **Languages that succeed pick a default and make it consistent.**
   Rust = borrow. Swift class = ARC. Go = copy + explicit pointer.
   Python = reference. Haskell = irrelevant.
2. **Two-tier type systems (value vs reference) are pleasant but
   confusing.** Swift's struct-vs-class is the classic example; users
   ship bugs when they assume class-like aliasing of a struct or vice
   versa.
3. **Explicit syntax for non-default access pays for itself.** Rust's
   `&`, `&mut`, `.clone()`; Go's `&` and `*`; Swift's `inout`. Where
   it's missing (Python, Ruby) you get bugs you have to debug at
   runtime.
4. **Iteration is where defaults bite hardest.** `for x in xs` is the
   single most common access pattern; if the default does the wrong
   thing 80% of the time, the user pays 80% of the time.

---

## 3. The four canonical access semantics

Stripping down to the minimal vocabulary:

| Name      | Producer keeps the value? | Consumer can mutate? | Consumer is independent? |
|-----------|---------------------------|----------------------|--------------------------|
| **Copy**  | Yes                       | Yes (own copy)       | Yes — divergent state    |
| **Share** | Yes                       | Yes (visible to all) | No — shared state        |
| **Borrow**| Yes                       | Yes/No (declared)    | No — limited lifetime    |
| **Move**  | No                        | Yes                  | Yes — original gone      |

That's the whole story. The four user cases the user raised map onto
these:

| User case                             | Right semantics |
|---------------------------------------|-----------------|
| `Vec of i64` summed in a loop         | Copy            |
| `Vec of HugeStruct` inspected         | Borrow          |
| `Vec of Socket` iterated for IO       | Share (or Borrow) |
| Mailbox `pop()`                       | Move            |
| Query result mutated and persisted    | **Don't** — go through the store |

The fifth case is interesting: "don't" is a valid answer. You shouldn't
get a *thing* from a store at all; you should get a handle that knows
how to write back through the store API. That's a store-design issue,
not an access-semantics issue.

### Why not just "borrow" everywhere?

Borrowing is great when the borrower is short-lived. It's painful when
the borrower needs to outlive the source (returning a borrow from a
function, storing a borrow in a struct field, capturing in a closure
that lives on a different thread, etc.). Rust solves this with
lifetimes; the cost is a learning curve.

### Why not just "share" everywhere (ARC)?

Atomic refcount ops are ~10ns each. Hot loops can spend more time
bumping refcounts than doing work. Cycles need a separate solution
(weak references or a cycle collector). Lock-free safe aliasing
requires a memory model commitment.

### Why not just "copy" everywhere?

Wrong for resources (sockets, file handles, GPU buffers). Wrong for
intent (you wanted aliasing). Quadratic-or-worse in iteration when
elements are large. Surprising for users coming from Python/JS.

### Why not just "move" everywhere?

You can only read a value once. Trivial expressions like
`if x > 0 { use(x) } else { use(x) }` need a duplicate or an explicit
`x.clone()`. The user-experience tax is too high.

**Conclusion: all four are necessary. The question is what's *default***
and *how the others are spelled.*

---

## 4. Proposal for Jinn

### 4.1 Three guiding principles

1. **The default does the safe, predictable thing at every access site.**
   We will not silently clone a 4KB struct because the user wrote
   `for x in v`. We will not silently alias a heap pointer because the
   user wrote `x.field`.
2. **Non-default access has syntax — but only one shape per behavior.**
   Not Rust's `&`/`&mut`/`Box`/`Rc`/`RefCell` zoo. Maybe four or five
   total surface forms.
3. **Inference picks the cheapest correct option** unless the user
   asked for something specific. A read-only `for` over a `Vec` borrows;
   one that mutates element fields borrows mutably; one whose body
   forks ownership (sends to a channel, stores in another container)
   copies or moves as needed.

### 4.2 Surface syntax (resolved)

Four optional access modifiers, all spelled as plain words. They apply
in the same positions: binding RHS, parameter type, `for`-loop binder.

| Modifier | Meaning                                                              |
|----------|----------------------------------------------------------------------|
| (none)   | Inference picks per §4.3                                             |
| `copy`   | Independent deep clone; consumer owns its own value                  |
| `ref`    | Shared **read-only** alias; compiler picks borrow / Rc / Arc tier   |
| `mut`    | Exclusive **mutable** alias; compiler picks &mut / Rc<Cell> / Arc<Mutex> tier |
| `take`   | Move out of the source                                               |

`ref` and `mut` are the only mutation-vs-read split; we deliberately
do **not** introduce `borrow` / `borrow mut` as separate words —
`mut` carries the mutation intent and the compiler decides the
lowering tier (see §4.4). `ref` and `mut` are mutually exclusive.

Binding:

```jinn
x is queue.get(0)         # default: see §4.3
x is copy queue.get(0)    # deep clone
x is ref queue.get(0)     # shared alias (read-only)
x is mut queue.get(0)     # exclusive mutable alias
x is take queue.pop()     # move out
```

Function parameters:

```jinn
*process(msg as Message)        # move (consume)
*inspect(msg as ref Message)    # shared read alias
*update(msg as mut Message)     # exclusive mutable alias
*log(msg as copy Message)       # caller keeps its value; callee owns a clone
```

Container iteration:

```jinn
for x in xs              # default per §4.3 (usually ref for heap, copy for POD)
for copy x in xs         # clone each element
for ref x in xs          # explicit shared alias per iter
for mut x in xs          # exclusive mutable alias per iter
for take x in xs         # consume xs; each x is owned
```

### 4.3 Default-inference rules

The compiler's choice when no modifier is present. Bolded entries are
changes from today.

| Site                                | POD          | Heap, clonable    | Heap, not clonable | `@resource`    | `@atomic`         |
|-------------------------------------|--------------|-------------------|--------------------|----------------|-------------------|
| `let x = c.get(i)`                  | copy         | **ref (T1)**      | ref (T1)           | ref (T1)       | ref (T3 = Arc)    |
| `let x = c.pop()`                   | move         | move              | move               | move           | move              |
| `for x in c`                        | copy         | **ref (T1)**      | ref (T1)           | ref (T1)       | ref (T3)          |
| `c.push(x)`                         | move-as-copy | move              | move               | move           | move (Arc bump)   |
| `f(x)` where `f(p as T)`            | copy         | move              | move               | move           | move              |
| `f(x)` where `f(p as ref T)`        | (use copy)   | ref (T1/T2)       | ref (T1/T2)        | ref (T1/T2)    | ref (T3)          |
| `f(x)` where `f(p as mut T)`        | (use copy)   | mut (T1/T2)       | mut (T1/T2)        | mut (T1/T2)    | mut (T3 = Arc<Mutex>) |
| `x.field`                           | copy         | **ref then auto-copy on escape** (§4.6) | same  | ref            | ref (T3)          |
| `match … bind v`                    | copy         | move (exhaustive) | move               | move           | move              |
| `closure { … x … }`                  | copy         | move-capture      | move-capture       | move-capture   | ref-capture (T3)  |
| `chan.send(x)`                      | move (wire copy) | move          | error              | move           | ref-send (T3)     |

The critical defaults:

- **Container reads (`.get`, `.peek`, `.front`, `.back`, `.first`,
  `.last`) and `for` loops borrow at Tier 1** when the consumer is
  contained in the source's lifetime. They fall back to the higher
  tiers (or to copy, if the user wrote `copy`) when escape is
  detected. They never silently deep-clone.
- **`@atomic` types always go through Arc.** A user who annotates a
  type with `@atomic` is opting in to atomic refcounting at every
  access site.
- **`@resource` types never copy.** Default binding is move; the
  compiler emits an error rather than synthesizing a clone.
- POD (trivially droppable) types continue to copy by default —
  there is no win to be had from borrowing an `i64`.

### 4.4 Tiered lowering of `ref` and `mut`

`ref` and `mut` are *intent*. The compiler picks the cheapest correct
lowering. There are three tiers; the user only writes the intent, never
the tier.

**`ref` (shared read-only alias) tiers:**

| Tier | Condition                                            | Lowering                |
|------|------------------------------------------------------|-------------------------|
| T1   | Use is contained in source's lifetime; no escape     | Static borrow (raw ptr) |
| T2   | Escapes scope/return/struct/closure; single-threaded | `Rc<T>` refcount bump   |
| T3   | Crosses a thread boundary (channel send, actor msg, `spawn` capture, or type is `@atomic`) | `Arc<T>` atomic bump |

**`mut` (exclusive mutable alias) tiers:**

| Tier | Condition                                            | Lowering                |
|------|------------------------------------------------------|-------------------------|
| T1   | Contained, exclusive — no other live `ref`/`mut`     | Static `&mut` (raw ptr) |
| T2   | Escapes; single-threaded                             | `Rc<Cell<T>>`-style (interior mutability, single-threaded) |
| T3   | Crosses thread boundary OR `@atomic` type            | `Arc<Mutex<T>>`-style (atomic + lock) |

The escape analysis is *local* (per-function). It does not require
lifetime variables in signatures. The rules:

- A T1 alias is valid only within the smallest scope that contains
  both its introduction and its source's last use.
- T1 may not be: returned, stored in a heap-owned field, captured by
  an escaping closure, or sent on a channel.
- Multiple T1 `ref`s of the same source are fine simultaneously.
  A T1 `mut` excludes all other access to its source for its scope.
- If any of the T1 conditions fail, the compiler silently promotes
  to T2 (or T3 if a thread boundary is crossed). No error, no
  warning by default — the user wrote `ref`, they got an alias.
  `--strict-borrow` can promote the silent escalation to a warning.

This is essentially Swift's model: "borrow if you can, ARC if you
must." Jinn's existing auto-Rc inference already does most of the
T2 work; this proposal just makes the intent visible at the surface.

**On the "rc>1 + mutated → atomic" intuition:** an Rc is upgraded to
Arc statically (at the type-flow level), not at runtime. If a `ref`
value *could* cross a thread boundary anywhere in its program-wide
flow, the compiler picks Arc up front. Refcount-aware copy-on-write
is a separate, runtime mechanism for `String` and `Vec` and applies
orthogonally — mutating a Tier-2/3 `mut` of a COW-able heap value
clones the backing buffer if the refcount is >1 at the moment of
the write. That gives the "transparent unaliasing" behavior users
expect from Swift `Array` / Jinn's strings without any explicit
opt-in.

### 4.5 Handling the four user cases

```jinn
# 1. Vec of i64 — copy is correct, cheap, and default.
total is 0
for n in numbers         # copy of each i64
    total = total + n

# 2. Vec of Socket — iteration borrows; the IO uses the borrow.
for s in sockets         # borrow each — no FD duplication
    s.read_into(buf)     # method takes &Socket

# 3. Mailbox.pop — explicit consumer, explicit move out.
msg is mailbox.pop()     # default of pop = move; no copy

# 4. Store query result — store API returns Row<T>, a smart handle.
#    No `.field = …` mutation; only `row.update { it.email = … }`,
#    which goes through the store's write path.
row is users.where(active: true).first()
users.update(row) { it.last_seen = now() }
```

### 4.6 Field access (resolved: short-lived borrow with auto-copy on escape)

`x.field` is a *short-lived borrow* of the field, valid for the
immediate expression. Most uses (`if x.field == …`, `print(x.field)`,
`y is x.field + 1`) consume the borrow within the same statement and
compile to a plain load.

If the borrow would escape its expression — e.g. `s is x.address`
where `s` is then stored, returned, or sent elsewhere — the compiler
auto-copies (deep clone) at the boundary. The user wrote no modifier,
so they get a safe, independent value.

Explicit modifiers override:
- `s is ref x.address` — alias (Tier-1 borrow if possible, else Rc/Arc).
- `s is take x.address` — move the field out (see below).
- `s is copy x.address` — explicit clone (same as the auto-copy default,
  but documents intent).

**On move-out (`take x.field`):** partial move out of a field is sound
but requires the compiler to track which fields of `x` are still
live, so `x`'s drop glue skips the moved field. Two implementation
options:

1. **Per-field liveness bitmask on the parent.** Adds a small runtime
   tag (one bit per non-POD field) and skips fields whose bit is
   clear in the drop helper. Always works.
2. **Static "last use of `x`" check (Perceus-driven).** If `x` itself
   is dead after the field move, we can move out and consume the
   rest of `x` without dropping the moved slot. Zero runtime cost.

We ship (2) first (it's a clean extension of the Perceus pass we
already run) and fall back to auto-copy when (2) can't prove last-use.
Option (1) is a future possibility if real code wants partial moves
out of long-lived parents.

### 4.7 Type-level annotations: `@resource` and `@atomic`

Two annotations attach to `type` declarations and affect every
binding/parameter/iteration that touches values of that type.

```jinn
@resource
type Socket
    fd as i32

    *close
        # called automatically when the last owner drops
        c.close(self.fd)

@atomic
type EventBus
    listeners as Vec of Listener
```

**`@resource`** — linear-by-default. For values of this type:
- Default binding is **move** (not copy, regardless of size).
- `copy` modifier is a compile error.
- `ref` Tier-1 borrow is allowed; Tier-2 (Rc) is allowed; Tier-3
  (Arc) is allowed only if the type is *also* `@atomic`.
- Channel/actor send moves the resource (transfers ownership to the
  receiver). A resource cannot be `ref`-sent.
- A `*drop` method on the type, if present, is invoked automatically
  at the end of the owning binding's scope (replaces ad-hoc cleanup).

**`@atomic`** — concurrent-aliasable. For values of this type:
- `ref` lowers to Arc (Tier-3) by default, even without a visible
  thread boundary. Use this for types you *intend* to share across
  threads.
- `mut` lowers to `Arc<Mutex<T>>`. Mutation acquires the lock.
- Channel/actor sends are `ref`-pass (just an Arc bump on the wire),
  no deep copy required.
- Cycle handling: a `@atomic` type may also be marked `@weakable`
  to expose weak references; otherwise cycles are the user's
  responsibility (as today).

The combination `@resource @atomic` is valid and means a
"shareable resource" (think: a connection pool, a logger). Multiple
threads can hold an Arc to it; the wrapped resource is freed and its
`*drop` runs when the last Arc is released.

### 4.8 What about Rc / Weak today?

The `Rc(T)` / `Weak(T)` HIR types stay. They become the compiler's
internal lowering targets for `ref` Tier-2 and the cycle-breaking
escape hatch. **Users stop writing them at the surface.** Existing
code that names `Rc(T)` continues to parse and compile; the type is
deprecated but not removed. Migration tooling converts surface `Rc`
bindings to `ref`.

`weak` survives as a keyword for the rare case of explicit
cycle-breaking: `parent as weak ref Parent`.

---

## 5. Sprint plan

The detailed phase-by-phase sprint, with file lists, test checkpoints,
and acceptance criteria, lives in
[docs/access-semantics-sprint.md](access-semantics-sprint.md). Summary:

1. **P1 — Surface & HIR plumbing.** Lex/parse `copy`/`ref`/`mut`/`take`
   modifiers, `@resource`/`@atomic` annotations, weak ref form.
   Extend `Ownership` enum, add `TyAttrs` to type decls.
2. **P2 — Escape analysis & tiered lowering.** Implement local escape
   analysis. Lower `ref`/`mut` to T1/T2/T3 per §4.4. Wire stdlib
   container readers to default-`ref`.
3. **P3 — Resource & atomic semantics.** Implement `@resource` (no
   copy, auto-`*drop`) and `@atomic` (Arc/Mutex lowering). Annotate
   stdlib `Socket`/`File`/etc.
4. **P4 — Field-access auto-copy + Perceus partial-move.** Boundary
   auto-copy for `x.field` escapes; partial-move on proven last-use.
5. **P5 — Stores & smart rows.** `query(…)` returns `Row<T>`;
   `snapshot()` returns plain `T`. Bare-row mutation is an error.
6. **P6 — Cleanup & docs.** Remove `is_aliased_read_of_heap`
   heuristic, prune unused clone paths, update `jinn.md`,
   write user-facing migration guide.

### Compatibility

Most existing Jinn programs compile unchanged:
- POD types behave identically (copy is copy).
- `.pop()` and `.remove()` were already moves.
- Iteration that doesn't mutate or escape compiles to a Tier-1
  borrow — observationally equivalent to today's clone, just faster.

The one breaking class of programs: code that relied on `.get()`
returning an independent owned heap value and then escaped that value
out of the borrowing scope. These now get either auto-promotion to
Tier-2 (Rc — silent, faster) or an explicit `copy` requirement under
`--strict-borrow`. A `jinn migrate` codemod is part of P6.

---

## 6. Resolved decisions

Decisions made in response to user review (see also §4.2, §4.4, §4.6,
§4.7):

1. **Four keywords: `copy`, `ref`, `mut`, `take`.** Plain words, no
   sigils. `copy` covers both bitcopy and deep clone — the compiler
   picks. Future tooling can surface "this `copy` is O(n)" as a hint
   for perf-sensitive code, but we don't burden the surface with
   `clone` vs `copy`.
2. **No separate `borrow` keyword.** `ref` (read alias) and `mut`
   (mutable alias) cover the same ground; the compiler decides the
   tier (T1 static borrow / T2 Rc / T3 Arc) per §4.4. `mut` replaces
   the strawman `borrow mut`.
3. **Function-level markers are *user-written intent*, not inferred.**
   `*f(x as ref T)` is a guarantee to the caller ("I will not consume
   x"). Inference can promote `mut` → `ref` if the body never writes,
   but it cannot infer ownership-direction; the signature is the
   contract.
4. **Iteration mutation uses `mut`.** `for mut x in xs { x.field = …}`.
   Symmetric with binding and parameters. No sigil overload.
5. **Stores hand back `Row<T>` smart handles for mutable queries.**
   Read-only `query(…).snapshot()` returns plain `T`. Bare `T` from a
   mutable query becomes a compile error during Phase 5 of the sprint.
6. **`take` is a general expression modifier.** Valid wherever the
   compiler can prove (or insert) the source-side erasure: `take xs[i]`,
   `take x.field`, `take queue.front`, `take map[k]`. For container
   slots it lowers to the container's existing remove/take method or
   inserts a per-slot tombstone. `.pop()` is shorthand for
   `take queue.front`.
7. **`@atomic` is the user-visible concurrency switch.** Cross-thread
   flow detection still drives auto-promotion for non-`@atomic` types,
   but the annotation lets users *declare* that a type is meant to be
   shared, making Arc the default lowering instead of an inferred
   fallback. `needs_atomic_rc` keeps its current type-traversal job;
   the annotation just adds another input.
8. **`@resource` formalises linear types.** No copy, no Tier-3 share
   without `@atomic`, automatic `*drop`. Stdlib types `Socket`,
   `File`, `Pipe`, `Mutex`, `Channel`, `Coroutine`, `Generator`,
   `Process` all gain `@resource`.
9. **Field access auto-copies on escape (§4.6).** Partial-move via
   Perceus last-use is a Phase-3 optimization, not part of the
   semantic surface.

There are no remaining open questions for the surface design.
Implementation-detail questions (e.g. exact wire format for `Arc`
refcounts on `@atomic` types, runtime layout of `Rc<Cell<T>>` for
Tier-2 `mut`) are captured in the sprint plan ([docs/access-semantics-sprint.md](access-semantics-sprint.md)).

---

## 7. Recommendation

Adopt the design in §4 in full, executed in the phases of §5 (detailed
in [docs/access-semantics-sprint.md](access-semantics-sprint.md)). The
benefits:

- **Correctness for the user's four cases.** Sockets borrow, mailboxes
  move, stores hand back smart handles, large structs aren't silently
  cloned in `for` loops.
- **Performance recovery.** The 6 benchmark regressions we're chasing
  almost all involve extra drops/clones at sites that should be
  borrows.
- **Self-documenting code.** A function signature with `ref` says
  "I won't take this from you." A binding with `take` says "this is
  gone now." A loop with `copy` says "I'm forking ownership."
- **Smaller surface than Rust.** No lifetime annotations in
  signatures, no `'a` syntax, no `Box`/`Rc`/`RefCell` zoo. Four
  modifier keywords (`copy`, `ref`, `mut`, `take`), two type
  annotations (`@resource`, `@atomic`) — and most code uses none of
  them.

The cost is real but bounded: a language version bump, the sprint
work in §5, and one breaking class (escape of `.get()` results
auto-promotes to Rc, or errors under `--strict-borrow`).

What we have today is a halfway design: ownership-by-default, with
silent clones bolted on at the points where the half-design hurt the
most. The user is right to flag it. The fix is to commit to the full
model.
