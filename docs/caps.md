# capabilities — the effect pass that lamp reads

**Status: decided spec.** This is the language/compiler feature `docs/lamp.md`
§6.1, §5.5, §5.7.2 assume. It supersedes the "generalize the error-effect
machinery into one `{errors, caps}` fixpoint" lean in `docs/lamp-prereqs.md`
§2.4. Decision: **capabilities are their own pass.** Errors and capabilities are
different kinds of fact about a function and conflating their inference into one
record made both harder to reason about. They share *shape* (inferred-by-
default, annotation narrows, SCC least-fixpoint over the call graph) but not
*storage* and not *one pass*.

Read this before `scope.md` and `interface-hash.md`; both consume the cap row.

---

## 0. The one-paragraph model

A capability is a *static permission a function's body requires*. The compiler
**infers** the capability set of every function from what it actually does, by
union over its call graph. A declaration (`needs net.client`) is an **upper
bound the compiler checks**, never the source of truth — exactly the shape of
error effects (`! E` narrows and documents an inferred set, it does not create
it). A pure data-structure library declares nothing, infers the empty set, and
is *proven* incapable of I/O. The package manager never re-derives any of this;
it reads the finished cap row off the interface (`interface-hash.md` §3).

The cap row is `CapRow = { caps: CapSet, ffi_tainted: bool }`. It lives on the
typed signature next to (not inside) the error row.

---

## 1. The lattice

### 1.1 Capability classes (closed set)

```
fs.read  <path?>     fs.write <path?>
net.client           net.server
process.spawn        env.read
clock                time
random               ffi.unsafe
state <region?>
```

The set is **closed**: a program cannot mint a new capability class. New
syscall surfaces map onto an existing class or the class set grows in a compiler
release (a deliberate, reviewed event). This is what makes `--deny` total:
`lamp build --deny net.*` can only fail to cover a cap if the compiler invented
one, which it cannot.

### 1.2 Scoped capabilities and the containment order

`fs.read`, `fs.write`, and `state` are *parameterized*. The lattice order is
containment of the parameter:

```
fs.read './assets/img/x.png'  ⊑  fs.read './assets/img'  ⊑  fs.read './assets'  ⊑  fs.read   (unscoped = any path)
```

- Path parameters are **normalized** (`.`/`..`/symlink-free, project-relative)
  before comparison; an absolute or `..`-escaping path is itself a distinct,
  louder cap (`fs.read /etc` does not satisfy `fs.read './assets'`).
- Unscoped `fs.read` is the top of its sub-lattice (reads anywhere). A bare
  `capabilities fs.read` is therefore the *widest* claim, and lint warns when a
  scoped use would have sufficed.
- `state <region?>` is covered in §4; its parameter is a *type/region identity*,
  not a path.

### 1.3 The two lattice operations

- **Inference uses join (∪).** A function's inferred caps = union of its own
  primitive uses and the caps of everything it calls. The least-fixpoint over
  the call-graph SCCs (§3) computes this.
- **Checking uses ⊑.** `derived ⊑ declared` at every declaration site, and
  `graph-union ⊑ project-ceiling` for the whole build. Scoped containment means
  `fs.read './assets/x'` (derived) satisfies `fs.read './assets'` (declared).

There is no meet operation in the surface language; the lattice is used
join-and-compare only.

---

## 2. Declaration sites and syntax

Four sites, one rule everywhere: **inferred by default, the annotation is a
checked upper bound.** The syntax mirrors the error-effect `! E` clause so there
is exactly one mental model across both effect systems.

### 2.1 Function and method level — the `needs` clause

The cap annotation is a trailing `needs` clause on the signature, the cap-world
twin of the `! E` error clause:

```jinn
*fetch url needs net.client
    ...

*save data needs fs.write './out'
    ...
```

A method inside a class uses the same clause. `needs` with multiple caps is a
comma list: `needs net.client, fs.write './out'`. Omitting `needs` means
"infer, do not check" — the common 98% case. Writing `needs` turns the inferred
set into a checked claim: the compiler proves `derived ⊑ declared` and errors
precisely otherwise. `needs ()` (or `needs pure`) is the affirmative "this
function is capability-free" assertion, checked.

Rationale for `needs` over `@caps(...)`: it reads as prose (`*fetch url needs
net.client`), it sits in the signature where the effect row belongs (so it
reaches the interface naturally, not via a side channel), and it is symmetric
with `! E`. Annotations stay annotations; this is a clause, not a decorator.

### 2.2 Class / store level

A class annotates its *aggregate* ceiling — the union over its methods — with a
`needs` clause on the class header:

```jinn
class HttpClient needs net.client
    ...
```

The class-level clause is checked against the union of its methods' inferred
caps. A store (`@store`) is a class that additionally and automatically carries
`state` (§4); its `needs` clause declares the *non-state* caps its methods use
(e.g. a store backed by a file declares `fs.write`).

### 2.3 Module level — the ceiling

The top of a file declares the file's ceiling with the existing `capabilities`
block (lamp.md §6.1), unchanged in surface:

```jinn
capabilities net.client, fs.read './assets'
```

This is checked against the union of every function/class in the module. It is
the unit the manifest's project-wide ceiling (§6) is compared against.

### 2.4 Precedence and the narrowing discipline

Declarations **only narrow**, never widen. From innermost out: a method's
`needs` ⊑ its class's `needs` ⊑ the module `capabilities` ⊑ the project ceiling.
Each level is checked against the *inferred* set at that level, and each outer
level is checked to contain the inner. A widening attempt (method needs
`net.server` but class declares only `net.client`) is the precise error, naming
both levels. This is the same containment ladder as path scoping (§1.2), applied
to nesting.

---

## 3. The derivation pass

### 3.1 Where it sits — a separate pass

A new pass, `src/typer/caps.rs` (working name), runs **after** the main typer
and **after** `errset.rs`, over the same typed HIR. It does **not** share the
error-effect fixpoint. They are sequenced, not merged:

```
type-check → errset (error rows) → caps (cap rows) → interface emit
```

Reasons for separation (the decided departure from prereqs §2.4):

1. **Different fixpoint inputs.** Error effects propagate through `?`/`!` and
   handler scopes; caps propagate through *every* call plus primitive sites and
   function-typed values (§5). Merging forced one record to carry two unrelated
   propagation rules.
2. **Different failure surfaces.** An error-effect violation is "you didn't
   handle `E`"; a cap violation is "you used a power you didn't declare." Keeping
   the passes separate keeps the diagnostics single-purpose.
3. **The interface (`interface-hash.md`) stores them as sibling rows** anyway;
   one pass producing two unrelated outputs is a false economy.

They share *machinery* by both using the generic SCC least-fixpoint utility
(`src/typer/scc.rs`), parameterized over the lattice. That utility is the shared
code; the passes are distinct.

### 3.2 The fixpoint

Over the call-graph SCCs in reverse-topological order:

```
caps(f) = primitive_caps(f)
        ∪ ⋃ { caps(g) | g ∈ direct_callees(f) }
        ∪ ⋃ { effect_row(t) | t a function-typed value flowing through f }   (§5)
```

Recursive cycles (an SCC with >1 node, or a self-call) are solved by least-
fixpoint: start each node at its primitive caps, iterate the union to
stabilization. Monotone over a finite lattice ⇒ terminates.

### 3.3 The primitive cap-attribution table — single source of truth

Which surfaces *introduce* a cap is data, not scattered logic, following the
`src/store_decorators.rs` pattern. A const table `CAP_SITES` maps a runtime/std
symbol to the cap it introduces:

```
("std.net.connect",        net.client)
("std.net.listen",         net.server)
("std.fs.read_file",       fs.read <arg0 as path>)
("std.fs.write_file",      fs.write <arg0 as path>)
("std.process.spawn",      process.spawn)
("std.env.get",            env.read)
("std.time.now",           clock)
("std.random.next",        random)
...
```

- A path-scoped entry records *which argument* carries the path; when that
  argument is a string literal the derived cap is scoped to it, otherwise it is
  the unscoped top of the sub-lattice (a runtime-computed path = `fs.read`
  unscoped). Lint nudges toward literal paths where possible.
- **Q7 resolved — missing attribution is a build error.** Every runtime FFI
  entry point that touches a syscall MUST appear in `CAP_SITES` or be explicitly
  tagged `caps: ()`. A runtime symbol reachable from Jinn with *no* table entry
  is a **hard build error** ("untagged runtime surface `X`: add it to
  `CAP_SITES`"), never a silent empty set. This makes the table exhaustive by
  CI, closing the §5.5 FFI gap in the small.

### 3.4 The check

After derivation, for every declared site (§2): assert `derived ⊑ declared`
using the containment order (§1.2). On violation emit one diagnostic naming the
offending cap, the declaring site, and *one* concrete call path that introduces
it (the shortest path through the call graph — invaluable for "where did
`net.client` come from?").

---

## 4. `state` — the keystone, decided

`state` is the cap that distinguishes a promotable (stateless) module from a
non-promotable one (lamp.md §5.7.2). Its soundness is **Q1**, the single highest
-risk claim in the whole package design. Decision:

### 4.1 Promotion is opt-in, not free (Q1 resolved)

lamp.md §5.7.2 presents promotion as *unconditionally free* given the cap proof.
That is true **only if `state` is reliably inferred for every source of process-
wide observable state** — module-level `var`, `@store` handles, actor mailboxes,
the scheduler, interners, lazily-initialized caches, FFI globals, `clock` and
`random` sources. We do not yet have an exhaustive, mechanically-checked
enumeration of those. Until we do, an unconditional "free" promotion can alias
two modules that share a hidden singleton.

**Decision: promotion requires the module to be `state`-free *and* the compiler
to have proven cap-completeness for that module.** Concretely:

- A module is **promotable** iff its inferred cap row contains no `state` cap
  **and** every runtime surface it transitively touches is cap-attributed (§3.3,
  enforced as a build error). The §3.3 build-error rule is what upgrades the
  proof from "we scanned for globals" to "the table is exhaustive by
  construction." With that rule in force, `state`-freedom *is* a sound purity
  proof — there is no untagged runtime path through which hidden state could
  leak.
- `clock`, `random`, `time`, `env.read` are **distinct caps**, not `state`. A
  module reading the clock is not stateful but it is not pure either; it is
  non-promotable because its cap row is non-empty *and observable* — we promote
  only on `caps == ()` (empty), not merely on `state`-freedom. This is the
  conservative tightening: **promote iff the cap row is empty.** A clock-reading
  module with identical hash to another is still safe to share *code*, but we do
  not collapse identities unless the row is empty, removing the last class of
  surprise.

So §5.7.2's promotion predicate is amended to:

> `A` and `B` are promotable iff their semantic hashes are equal **and** both
> cap rows are **empty** (`caps == ()`).

This is stricter than "no `state` cap" and it is *still free* — no separate
purity scan, the cap row already says it. We trade a little dedup (clock-reading
utilities don't collapse) for an airtight proof. Promotion of non-empty-but-
`state`-free modules can be revisited once the runtime cap enumeration is
certified exhaustive; until then the empty-row rule is the honest contract.

### 4.2 `state` regions

`state <region>` is parameterized by an interned *region identity* derived from
the declaring module's path-scoped `PackageId` (`scope.md` §2). Two
instantiations of the same stateful module passed *distinct* state get distinct
region parameters and so stay distinct by construction — code may be shared,
state identity is not (lamp.md §5.7.2's orthogonality claim, now grounded in the
region parameter). A bare `state` (unscoped) is the top of its sub-lattice.

---

## 5. Caps on function types — Q6 resolved

A capability can be *carried* by a value and discharged elsewhere: a closure
that uses `net.client`, a function passed to a higher-order combinator, a
message sent to an actor. If derivation only followed direct calls, passing a
`net.client`-using `*f` into a pure-looking `map` would *launder* the cap. That
is a soundness hole identical in class to Q1 but for control flow.

**Decision: the cap row is part of the function type.** A function value's type
carries its effect rows (errors *and* caps), exactly as `interface-hash.md` §3.2
requires for the interface anyway:

```
fn(A) -> B ! E needs C        # E = error row, C = cap row, both on the type
```

Consequences, all forced and consistent:

- Holding or storing a `*f` whose type carries `needs net.client` makes the
  holder's inferred caps include `net.client` *at the point the value can be
  called*. A combinator that *calls* its function argument unions that argument's
  cap row into its own derived set (§3.2's third clause). A combinator that
  merely *stores and returns* it carries it in its return type's effect row, so
  the cap surfaces at the eventual call site, never laundered.
- **Actor sends** carry caps through the message/handler protocol the same way:
  the handler's cap row is part of the channel/actor type, so sending work that
  needs `net.client` to an actor makes the actor's surface declare it. A "pure-
  looking" actor cannot secretly do I/O in a handler without its type saying so.
- Higher-order purity is therefore *parametric*: `map`'s own cap row is empty;
  `map(f)`'s effective caps = `f`'s caps. `map` is promotable; a particular
  `map(networkFn)` call site is cap-tainted. Correct and ergonomic.

This requires the type system to carry effect rows on function types — which it
must do for §5.3's interface regardless, so there is no extra mechanism, only the
discipline of *always* propagating the row through function-typed values.

---

## 6. FFI taint — decided

`ffi.unsafe` and any symbol imported from a binary `object/` (lamp.md §5.5) are
**declared ground truth**, not inferred (the compiler cannot see through them).
For these, derivation switches to one-directional containment:

- The declared cap set of an FFI/binary symbol is taken as-is and unioned into
  every caller (it can only *add* caps, never subtract).
- `ffi_tainted = true` propagates transitively and **cannot be laundered away**
  by a dependent: a pure-Jinn wrapper around an FFI call is still `ffi_tainted`.
- `lamp build --deny ffi.unsafe` is enforced at compile time by refusing to
  discharge the taint — the effect cannot be removed, so the build fails with the
  taint's origin (§3.4-style call path), giving the §5.5 guarantee teeth.

A binary dependency that declares fewer caps than it actually uses is *its*
publisher's signed lie (§6.2), not a hole in inference — the taint marks the
trust boundary, and signing/provenance is what backs it.

---

## 7. What the package manager reads

lamp never runs cap inference. After a successful build the cap row of every
public function/class/module is serialized into `interface.jhi`
(`interface-hash.md` §3.2) and folded into the **abi** hash (§3.3 there: gaining
a cap flips the abi hash). lamp then:

- shows the graph-union of caps in `project.lock` and `lamp tree --caps`
  (per-output, rolled up to a one-line summary by default — Q8: `--verbose` for
  the full path-scoped breakdown to keep large graphs readable);
- enforces `--deny <cap>` by refusing any build whose union intersects the
  denied set, with the offending package + call path named;
- treats a dependency update that adds a cap as a one-line, reviewable lock diff.

The capability pass is the proof; lamp is the auditor that reads it.

---

## 8. Cross-references

- Promotion / empty-row rule → `scope.md` §3 (unification pass) and lamp.md
  §5.7.2 (predicate amended per §4.1).
- Cap row in the interface + abi hash → `interface-hash.md` §3.2, §3.3.
- `state` region identity ← `scope.md` §2 (`PackageId`).
- Manifest `capabilities` ceiling → `config-blocks.md` §4 (it is a config block).
