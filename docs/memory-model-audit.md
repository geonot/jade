# Jinn Memory Model — Adversarial Audit & State-of-the-Art Gap Analysis

> Companion to [`memory-model.md`](memory-model.md). That document is the
> contract; this one is the red-team. Every finding below was produced by
> compiling and running real programs against the current `jinnc`
> (`target/release/jinnc`), not by reading the spec. Findings are ranked,
> reproducible, and mapped to what a production-grade, compiler-burden
> (no-GC, no-RC, no-lifetime-syntax) memory system still requires.

## 1. Method

Adversarial probe programs were written against every M-rule and every
seam between rules: branch joins, loops, call-site aliasing, struct
literals, element projections, iteration-during-mutation, defer/return
interactions, channel sends, and task captures. Each probe either
compiled+ran or was rejected; the observed behavior was compared to the
contract in `memory-model.md`.

## 2. Confirmed holes

### H1 — CRITICAL: move into a struct literal is not tracked → double-free

```jinn
type Holder
    items as Vec of i64

*main
    v is vec(1, 2)
    h is Holder(items is v)   # moves v — but no tombstone is recorded
    log(v.length)             # compiles; crashes: free(): invalid pointer
```

Observed: compiles cleanly, aborts at runtime with `free(): invalid
pointer` (glibc heap corruption; core dump). M1 covers `b is a`; field
initializers in constructor literals (`Holder(items is v)`) evidently do
not route through `mark_var_moved`. Both `h.items` and `v` believe they
own one buffer; scope exit drops twice.

**Required**: a struct-literal field init whose value is an aggregate
variable must tombstone the source exactly like `b is a`. Same for enum
payload construction, vec literals of aggregates (`vec(v1, v2)`), and map
literals. Any *value-position aggregate variable* in any constructor
expression is a move.

### H2 — CRITICAL: same aggregate passed to two consuming parameters in one call

```jinn
*sink(v) returns Vec of i64
    return v

*combine(a, b) returns i64
    x is sink(a)
    y is sink(b)
    return x.length + y.length

*main
    v is vec(1, 2, 3)
    n is combine(v, v)     # compiles; two owners of one buffer
    log(n)                 # prints 6; double-drop at combine's scope exit
```

Observed: compiles and runs (exit 0 by allocator luck — the double free
is real but undetected at this size). The tombstone for a consuming
argument is recorded *after* the call statement, so the second `v` in the
same argument list never sees it. Sequential calls (`a is sink(v); b is
sink(v)`) are correctly rejected — only intra-statement duplication slips
through.

**Required**: per-call-site alias check — the same root place may not be
passed to two consuming positions (or a consuming and any other position)
within one call. This is a purely local, cheap check.

### H3 — HIGH: aliased mutable+shared borrow parameters are accepted

```jinn
*app(dst, src)                 # dst: BorrowMut, src: Borrowed
    for i in 0 to src.length
        dst.push(src.get(i))

*main
    v is vec(1, 2, 3)
    app(v, v)                  # same object as both params — accepted
```

Observed: compiles and runs. Because `Vec` is header-based, both params
alias one header and the program "works" — but it violates the
exclusivity the backend is entitled to assume. `set_ptr_param_attrs`
emits `noalias`/`readonly`/`nocapture` attributes; an aliased read-only
param that is mutated through the other alias is **undefined behavior at
the LLVM level** and can miscompile at `--opt 3` at any time. Perceus
reuse pairing (`vec_reuse_pairing`) similarly assumes unique ownership.
This is a latent miscompilation, not a theoretical nit.

**Required**: call-site exclusivity check (Swift's "law of exclusivity",
statically enforced): if any parameter of a call takes a mutable borrow of
place P, no other parameter may borrow P or any place overlapping P
(fields, elements). Overlap of distinct fields of one struct may be
permitted (disjointness proof); whole-object overlap must be rejected
with a diagnostic naming `copy`.

### H4 — HIGH: container element reads copy, not borrow — mutations silently lost

```jinn
*main
    grid is vec()
    grid.push(vec(1, 2))
    grid.get(0).push(3)          # mutation applied to a temporary copy
    log(grid.get(0).length)      # prints 2 — the push vanished
    grid.get(0).set(0, 99)
    log(grid.get(0).get(0))      # prints 1 — the set vanished
```

Also observed through a call boundary: `mutate(grid.get(0))` where
`mutate` does `row.set(0, 5555)` — the write is lost.

The contract (M5, §9.2) says element reads in expression position are
*borrows*. The implementation deep-copies. Consequences:

1. **Silent lost updates** — the worst failure mode a language can have:
   no diagnostic, no crash, wrong answer.
2. **Hidden O(n) deep copies** on every nested-container expression read,
   the exact "hidden cost" the model exists to prevent.

**Required**: pick one and enforce it everywhere:
- (a) implement true statement-scoped element borrows (correct per spec;
  needs H5's invalidation rules), or
- (b) keep copies but **reject mutation of an rvalue copy**
  (`grid.get(0).push(3)` = compile error "mutating a temporary copy;
  use `grid.get(0)` in expression position for reads, or take/mutate/put
  back") and never pass an element read to a `BorrowMut` parameter.

Silent-copy-and-discard must not survive in any resolution.

### H5 — HIGH: iterator invalidation is unchecked — hang / undefined iteration

```jinn
*main
    v is vec(1, 2, 3, 4)
    total is 0
    for x in v
        v.push(9)          # accepted; iterator sees live growth
        total is total + x
    log(total)             # never reached — infinite loop (timeout)
```

Observed: compiles; loops forever (the iterator chases the growing vec).
With a growth guard it terminates, proving iteration reads the live
buffer — so a mid-iteration realloc leaves the iterator's cached pointer
stale (use-after-free waiting for the right allocation pattern).

**Required**: `for x in v` takes an implicit *iteration borrow* of `v`
for the loop body: mutating calls on `v` (push/insert/remove/clear),
moves of `v`, and passing `v` to `BorrowMut`/consuming parameters inside
the body are compile errors. Escape hatches: `for x in copy v` (snapshot)
or index-based loops. This is the same statement-scoped-borrow machinery
extended to a block scope — no lifetime syntax needed.

### H6 — MEDIUM: conditionally-consuming callees over-tombstone (false positive)

```jinn
*maybe(v, flag as bool) returns i64
    if flag
        w is sink(v)       # consumes only on this path
        return w.length
    return 0

*main
    v is vec(1, 2)
    n is maybe(v, false)
    log(v.length)          # rejected: "use of moved value `v`"
```

Path-insensitive consuming inference makes `maybe` unconditionally
consuming. Sound but imprecise; the diagnostic ("whose parameter takes
ownership") does not say the consumption was conditional, so the user
cannot understand why a call that dynamically never consumes is an error.

**Required** (either): conditional-consume diagnostics ("consumed on the
path where `flag` is true at m.jn:3"), or path-splitting for the common
if/return shape. Minimum bar: the diagnostic must show the consuming
path.

### H7 — MEDIUM: consuming-method detection is name-based

`src/typer/consume_infer.rs` hardcodes `CONSUMING_METHODS = push, insert,
append, add, put, set, enqueue, send, …` — a *string* list. Two failure
directions:

- A user-defined method named `set`/`add` that does **not** store its
  argument gets its callers' arguments spuriously tombstoned.
- A user-defined method with any other name that **does** store its
  argument into `self` escapes detection → same double-free class as H1.

**Required**: consumingness must be derived from the method *body* (the
same escape scan already applied to free functions), with the name list
retained only for runtime-implemented builtins that have no body to scan.

### H8 — LOW: rejected-program ergonomics gaps

- `runtime error: vec index out of bounds` aborts with a core dump
  (SIGABRT) rather than a clean runtime-error exit. Panics should not
  dump core by default.
- No closure/anonymous-fn capture rules could be audited (`f is *() …`
  fails to parse); when closures land, they inherit the entire M8/M11
  capture problem and must be specified before implementation.

## 3. The case chart

Behavior of each operation per type category, as **observed** (✅ = works
per contract, ❌ = hole above, ⚠ = spec/impl divergence, — = n/a).

| # | Situation | Scalar | Value (String) | Aggregate (Vec/Map/struct-with-agg) | Resource |
|---|-----------|--------|--------------|--------------------------------------|----------|
| 1 | `b is a` | copy ✅ | deep copy ✅ | move + tombstone ✅ (M1) | move ✅ |
| 2 | read after move | — | — | rejected ✅ with reason+fix | rejected ✅ |
| 3 | reassign revives | — | — | ✅ (M2) | ✅ |
| 4 | move in one `if` branch, use after join | — | — | rejected ✅ (branch union) | ✅ |
| 5 | move inside loop body | — | — | rejected ✅ (`check_loop_body_moves`) | ✅ |
| 6 | `v is b.field` | copy ✅ | copy ✅ | partial move ✅ (M3); whole-struct read after → rejected ✅ | move ✅ |
| 7 | **constructor literal field init `T(f is v)`** | copy ✅ | copy ✅ | **❌ H1: move untracked → double-free** | untested — audit |
| 8 | bind element `x is v.get(i)` | copy ✅ | copy ✅ (verified independent) | rejected ✅ naming copy/take (M4) — Vec and Map | rejected |
| 9 | `take v.get(i)` | — | remove+own ✅ | remove+own ✅ | ✅ |
| 10 | **element read in expression position** | value ✅ | value ✅ | **⚠/❌ H4: deep copy, not borrow; mutations through it silently lost** | — |
| 11 | unannotated param, callee only reads/mutates | value ✅ | borrow ✅ | borrow, in-place mutation visible to caller ✅ (M6) | borrow |
| 12 | unannotated param, callee consumes (return/bind/store/send) | value ✅ | exempt ✅ | inferred consuming; caller use-after rejected ✅, incl. transitive + alias-chain | explicit only |
| 13 | consuming inference: store into struct-field of another param | — | — | detected ✅ (P21) | — |
| 14 | **consume on one path only** | — | — | **⚠ H6: whole param consuming; diagnostic omits the path** | — |
| 15 | **same variable to two params, either consuming** | ✅ safe | ✅ safe | **❌ H2 (two consuming, one call) / ❌ H3 (mut+shared alias, LLVM noalias UB)** | rejected? — audit |
| 16 | **user method that stores its arg (name not in list)** | — | — | **❌ H7: escapes inference** | — |
| 17 | return local / consumed param | ✅ | ✅ | transfer, single drop ✅ (M7) | ✅ |
| 18 | return borrowed param | — | — | rejected ✅ | — |
| 19 | **`for x in v` + mutate `v` in body** | — | — | **❌ H5: accepted; hang / stale iterator** | — |
| 20 | capture in `dispatch`/`sim for`/`spawn`/actor msg | copy ✅ | copy ✅ | move; 2nd capture or parent use rejected ✅ with alternatives (M8) | rejected ✅ |
| 21 | `send ch, v` then read `v` | copy ✅ | copy | rejected ✅ ("sends transfer ownership…") (M9) | rejected ✅ |
| 22 | `defer` reads v, then v moved | — | — | rejected ✅ (M10); defer+return-move ordering correct ✅ | ✅ |
| 23 | scope exit drops | none ✅ | own heap ✅ | exactly once, reverse order, post-defer ✅ | `*drop` once ✅ |
| 24 | closures capturing aggregates | — | — | **unimplemented/unparseable — unaudited risk** | — |
| 25 | generators capturing aggregates (M11) | — | — | **could not audit (syntax); conformance test needed** | — |

Score: the *single-owner dataflow core* (rows 1–6, 11–13, 17–23) is
solid — every probe matched the contract, with diagnostics that name the
fix. Every hole lives at a **projection or aliasing seam**: constructor
literals (H1), call-site aliasing (H2/H3), element projections (H4),
iteration borrows (H5). The pattern: moves of *variables* are tracked
perfectly; moves and borrows of *places* (fields-in-literals, elements,
call-argument overlap) are where the analysis loses the thread.

## 4. Problematic assumptions in the model itself

1. **"A borrow ends with its statement" is not compositional.** M5's
   statement-scoped borrow is the load-bearing simplification, but loops
   (H5) and multi-argument calls (H3) are *statements containing
   statements/subexpressions* where two borrows of one place coexist. The
   model needs an explicit **exclusivity axiom**: at any program point, a
   place has either one mutable borrow or any number of read borrows —
   and the checker must enforce it *within* statements, not just across
   them.
2. **Consuming inference makes call semantics a whole-program property.**
   A call site's meaning (move vs borrow) depends on the callee's body,
   transitively. Fine intra-module; corrosive across module/package
   boundaries — editing a library function's *body* can break downstream
   *callers* with no signature change. Interface hashing
   (`interface-hash.md`) must include inferred consumingness, and
   exported functions should be **required** to state `take`/`copy`
   explicitly (compiler suggests, `fmt` inserts).
3. **"No cycles are constructible" relies on there being no leaks by
   other means.** True for ownership cycles; but H1 shows the inverse
   failure (double-free), and an unkillable generator/actor holding
   aggregates is still a de-facto leak. The claim should be restated as
   an invariant the conformance suite *checks* (ASan/LSan CI over the
   whole corpus), not an axiom.
4. **Deep-copy `String` as a Value type is a hidden-cost cliff.** A
   struct with a `String` field deep-copies on every assignment; add one
   `Vec` field and the same struct silently changes category to move
   semantics. Category is *inferred from field types, transitively* —
   adding a field to a leaf struct can change assignment semantics of
   every struct that embeds it, three levels up, with no diagnostic.
   **Required**: a warning ("this change makes `Config` an aggregate:
   assignments now move") on category transitions, and a `@value` /
   `@aggregate` opt-in assertion so library authors can pin the category.
5. **The escape/Perceus layers trust the typer completely.** Correct
   division of labor, but there is no verifier: nothing checks that MIR
   drop placement preserves "exactly one drop per owner" after
   sinking/fusion/reuse. A MIR-level **drop-linearity verifier** (debug
   builds: every owner dropped exactly once on every path) would turn
   silent corruption into ICEs.

## 5. Use-case shortfalls (things users cannot express)

| Use case | Status today | What's needed |
|---|---|---|
| Doubly-linked list, parent pointers, graphs | Inexpressible (by design) | Blessed **arena/index idiom** in std (`Arena of T` with generational indices), documented as *the* pattern; optionally compiler-checked index types |
| Large read-only data shared across tasks (config, model weights) | Copy per task, or funnel through one actor | **`freeze v`**: one-way transition to a deeply-immutable value that may be *shared* across tasks without copy — no RC needed if frozen values live until program/scope end (region), or with a single atomic count *only* on frozen roots (the Channel/ActorRef exemption already sets this precedent) |
| Zero-copy parsing / sub-slices | Impossible: no storable views, element reads copy (H4) | **Second-class slices**: `v.slice(a, b)` usable in expression position and as parameters (never storable) — same statement/param-scoped discipline as borrows, so still no lifetimes |
| Returning a view into an argument (`first_word(s)`) | Must copy | Second-class returns are the one place views need *some* lifetime relation; cheapest sound rule: a returned view must derive from a single designated parameter (Hylo/Val's "remote parts", or Mojo's origin-inferred refs) |
| Struct that lends internal state (iterator objects, cursors) | Impossible | `*[]`-style **subscript/yield accessors** (Hylo): callee *yields* a borrow into a caller-provided block — borrows never reified, still no annotations |
| Buffer reuse across calls (`read_into(buf)`) | Works (BorrowMut param) ✅ | Keep; document as the sanctioned zero-alloc pattern |
| Self-referential structs | Inexpressible | Correctly out of scope; document |
| Caches/memo tables shared by callers | One owner + actor, or param-threading | Fine, but needs an std `Memo`/`Lazy` built on the actor runtime so users don't invent unsafe workarounds |

## 6. What a state-of-the-art "intelligent compiler" memory system requires

The design goal — burden on the compiler, not the coder (no arcane
syntax) and not the runtime (no RC/GC) — is the same star Hylo (Val),
Koka/Lean (Perceus), Swift (exclusivity + ownership), and Mojo are
steering by. Measured against that frontier, Jinn's remaining work, in
priority order:

**Tier 0 — soundness (ship-blockers)**
1. Close H1 (constructor-literal moves), H2 (intra-call double-consume),
   H3 (call-site exclusivity), H4 (element projection semantics), H5
   (iteration borrows). All are static checks over machinery that already
   exists (place-based tombstones + a call-site overlap check).
2. **Place-based, not variable-based, analysis.** Unify `moved_vars` /
   `moved_fields` / element rules into one *place* lattice
   (`root.field.elem…`) with overlap/disjointness queries. Every hole in
   §3 falls out of variable-granularity thinking; every fix falls out of
   place granularity. This is the single highest-leverage refactor.
3. **Exclusivity as a stated axiom** (§4.1) enforced intra-statement,
   intra-call, and across loop bodies. Only then are the LLVM `noalias`
   attributes and Perceus reuse actually justified — today they are
   wishes.
4. **Whole-corpus sanitizer CI**: every conformance test and every
   `apps/` program under ASan+LSan at `--opt 0` and `--opt 3`, plus a
   fuzzer that mutates ownership-relevant syntax. H1 would have been
   caught by the first run.
5. **MIR drop-linearity verifier** (debug): every path drops every owner
   exactly once; Perceus passes re-verified after each transform.

**Tier 1 — expressiveness without new user burden**
6. **Second-class references/slices** (Hylo-style): borrows usable in
   parameters, expression position, and yield-accessors — but never
   storable. Retains "no lifetime syntax" while unlocking zero-copy
   slices, lending iterators, and H4's honest fix.
7. **`freeze` + shared immutables** for cross-task read-mostly data;
   deep-immutability makes sharing race-free with at most one count on
   the root (or none, with region/scope-bounded lifetime via
   `together`-scoped sharing — structured concurrency already gives the
   scope).
8. **Arena/generational-index std type** as the blessed graph pattern.
9. **In-place reuse guarantees surfaced** (Koka's FBIP): a `@reuse`-style
   assertion or a `jinn explain drops` view showing where Perceus reused
   vs freed, so performance is inspectable, not folklore.

**Tier 2 — predictability and scale**
10. **Explicit ownership at public boundaries**: exported fns must
    declare `take`/`copy`/borrow; inference remains for internal code.
    Inferred consumingness goes into the interface hash either way.
11. **Category-transition diagnostics** (§4.4) and `@value`/`@aggregate`
    assertions.
12. **Path-qualified diagnostics** for conditional consumption (H6), and
    diagnostics that always show the *data-flow chain* (they already name
    move sites and fixes — best-in-class; keep that bar for the new
    checks).
13. **Ownership-aware IDE surface**: hover shows a binding's category,
    tier (Owned/Borrowed/BorrowMut), tombstone state, and drop site; the
    single most effective way to teach the model without syntax.
14. **A `jinn miri`-style checked interpreter** for the conformance
    corpus: executes MIR with full place/ownership metadata and traps any
    UAF/double-drop/lost-update the static layer misses — the oracle that
    keeps spec, checker, and codegen honest forever.

## 7. Bottom line

The core bet is sound and the core is *actually implemented*: variable-
level move tracking, branch/loop dataflow, consuming inference, task
isolation, and drop discipline all held up under attack, with diagnostics
that are genuinely excellent. What has not yet been built is the same
rigor for **places** — struct-literal fields, container elements, and
overlapping call arguments — and that is precisely where all five
serious holes (one crashing, two latent-UB, two silent-wrong-answer)
live. Close those with a place lattice + an exclusivity check, add
second-class references and `freeze` for the two big expressiveness gaps,
and back it with sanitizer CI plus a checked interpreter — and Jinn's
memory system is legitimately state-of-the-art: safer than RC (no cycles,
no leaks), faster than GC (no runtime), and lighter on the user than
Rust (no lifetimes, no `Box`/`Rc`/`Arc`, no annotations in application
code).
