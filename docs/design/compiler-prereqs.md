# Compiler prerequisites for lamp

> **Status: decided spec; the capabilities pass has shipped, the rest has
> not.** Four pieces of compiler machinery that [`lamp.md`](lamp.md) assumes:
> capabilities as an effect pass (**live** — [145]/[149]/[159]: inferred rows
> over call-graph SCCs, extern-leaf classification, std apertures, path-scoped
> store effects, actor handlers in the fixpoint; residue at `E-2`/`E-3`),
> path-scoped package identity, interface v2 with the three-hash ladder, and
> config blocks — the latter three unimplemented. They are specified together
> because they depend on each other in that order — capabilities feed the
> interface, identity is rooted in the abi hash, and the manifest is a config
> block.

Read [`lamp.md`](lamp.md) first for what consumes all of this.

Current reality, stated plainly so the gap is visible:

- **Capability checking is live for `needs` upper bounds.** Effects are
  classified at the extern leaves (`src/cap_sites.rs`; an
  unclassified extern call or `syscall`/`asm` is `ffi.unsafe`), std-vetted
  aperture entries give `io.*`/`fs.*` path-scoped signatures, and the fixpoint
  in `src/typer/caps.rs` covers free functions, generics, type/impl methods,
  actor handlers, and store operations (path-scoped `fs` effects, [159]). What
  this design still adds beyond that: module/project capability ceilings and
  the manifest story.
- **Modules are merged by string prefixing.** `flatten_module` in
  `src/resolve.rs` rewrites `*name` → `module_name` and `Type` → `Module_Type`,
  after which there are no modules — just a flat mangled global namespace.
- **The interface file is a v1 header.** `src/interface.rs` holds
  `InterfaceFile { version, module, functions }` over a closed `IType` enum,
  with no ownership, no effect rows, no Perceus obligations, and no hashing.
  Reading it is off by default (`X-3`).
- **Config blocks do not exist.** The manifest is parsed ad hoc.

---

# Capabilities

A capability is a *static permission a function's body requires*. The compiler
**infers** the capability set of every function from what it actually does, by
union over its call graph. A declaration is an **upper bound the compiler
checks**, never the source of truth — exactly the shape of error effects, where
`! E` narrows and documents an inferred set rather than creating it. A pure
data-structure library declares nothing, infers the empty set, and is *proven*
incapable of I/O. The package manager never re-derives any of this; it reads the
finished row off the interface.

Capabilities are their **own pass**, not a merged `{errors, caps}` fixpoint.
They share the *shape* of error effects — inferred by default, annotation
narrows, SCC least-fixpoint over the call graph — but not their storage and not
one pass. Conflating the inference of two different kinds of fact about a
function made both harder to reason about.

## The lattice

The class set is **closed**:

```
fs.read <path?>     fs.write <path?>
net.client          net.server
process.spawn       env.read
clock               time
random              ffi.unsafe
state <region?>
```

A program cannot mint a new class. A new syscall surface maps onto an existing
class, or the set grows in a compiler release as a deliberate, reviewed event.
That closure is what makes `--deny` total: `lamp build --deny net.*` can only
miss a capability if the compiler invented one, which it cannot.

`fs.read`, `fs.write`, and `state` are parameterized, and the lattice order is
containment of the parameter:

```
fs.read './assets/img/x.png'  ⊑  fs.read './assets/img'  ⊑  fs.read './assets'  ⊑  fs.read
```

Path parameters are normalized — no `.`, `..`, or symlinks, project-relative —
before comparison. An absolute or escaping path is a *distinct, louder*
capability: `fs.read /etc` does not satisfy `fs.read './assets'`. Unscoped
`fs.read` is the top of its sub-lattice, so a bare `capabilities fs.read` is the
widest possible claim and lint warns when a scoped one would have sufficed.

Two operations, no meet: **inference joins** (a function's caps are the union of
its own primitive uses and everything it calls, as a least fixpoint over
call-graph SCCs), and **checking compares** (`derived ⊑ declared` at every
declaration site, `graph-union ⊑ project-ceiling` for the whole build).

## Declaration sites

Four sites, one rule everywhere: **inferred by default, the annotation is a
checked upper bound.** The syntax mirrors the `! E` error clause so there is one
mental model across both effect systems.

```jinn
*fetch url needs net.client

*save data needs fs.write './out'

class HttpClient needs net.client

capabilities net.client, fs.read './assets'    # module ceiling
```

Multiple capabilities are a comma list. Omitting `needs` means "infer, do not
check" — the common case. Writing it turns the inferred set into a checked
claim. `needs pure` (equivalently `needs ()`) is the affirmative "this function
is capability-free" assertion, and it is checked.

`needs` rather than `@caps(...)` because it reads as prose, sits in the
signature where an effect row belongs — so it reaches the interface naturally
rather than through a side channel — and is symmetric with `! E`. It is a
clause, not a decorator.

**Declarations only narrow.** From innermost out: a method's `needs` ⊑ its
class's `needs` ⊑ the module `capabilities` ⊑ the project ceiling. Each level is
checked against the inferred set at that level, and each outer level is checked
to contain the inner. A widening attempt is a precise error naming both levels.

## `state` — the keystone

`state` is what distinguishes a promotable (stateless) module from a
non-promotable one, and its soundness is the highest-risk claim in the whole
package design. The naive version — "promotion is free given the cap proof" — is
true only if `state` is reliably inferred for *every* source of process-wide
observable state: module-level bindings, store handles, actor mailboxes, the
scheduler, interners, lazily initialized caches, FFI globals. Without an
exhaustive, mechanically checked enumeration of those, unconditional promotion
can alias two modules that share a hidden singleton.

**Decision: promote only on an empty capability row.**

> Two instances are promotable iff their semantic hashes are equal **and** both
> capability rows are empty (`caps == ()`).

This is stricter than "no `state` cap", and it is still *free* — no separate
purity scan, because the row already says it. `clock`, `random`, `time`, and
`env.read` are distinct capabilities, not `state`: a module reading the clock is
stateless but not pure, so its *code* may be shared while its *identity* is
never collapsed. The trade is a little dedup on impure-but-stateless utilities
in exchange for an airtight proof. It can be revisited once the runtime
capability enumeration is certified exhaustive.

The proof rests on one enforced rule: every runtime surface a module
transitively touches must be capability-attributed, as a build error. That is
what upgrades "we scanned for globals" into "the table is exhaustive by
construction".

`state <region>` is parameterized by an interned region identity derived from
the declaring module's `PackageId`. Two instantiations of the same stateful
module passed distinct state get distinct regions and stay distinct **by
construction** — code dedup and state identity are orthogonal, and the region
parameter is what makes them so.

## Capabilities on function types

A capability can be *carried* by a value and discharged elsewhere: a closure
that uses the network, a function passed to a higher-order combinator, a message
sent to an actor. If derivation followed only direct calls, passing a
network-using function into a pure-looking `map` would **launder** the
capability.

**Decision: the capability row is part of the function type.**

```
fn(A) -> B ! E needs C        # E = error row, C = cap row, both on the type
```

Consequences, all forced: holding or storing a function whose type carries
`needs net.client` makes the holder's inferred capabilities include it at the
point the value can be called. A combinator that *calls* its function argument
unions that argument's row into its own; one that merely stores and returns it
carries the row in its return type, so the capability surfaces at the eventual
call site rather than vanishing. Actor sends carry capabilities through the
message and handler protocol the same way, so a pure-looking actor cannot
secretly do I/O without its type saying so. Higher-order purity is therefore
*parametric*: `map`'s own row is empty, and `map(f)`'s effective capabilities
are `f`'s.

The type system must carry effect rows on function types for the interface
anyway, so this adds no mechanism — only the discipline of always propagating
the row through function-typed values.

## FFI taint

`ffi.unsafe` symbols and anything imported from a binary object are **declared
ground truth**, not inferred, because the compiler cannot see through them. For
these, derivation switches to one-directional containment: the declared set is
taken as-is and unioned into every caller (it can only add, never subtract), and
`ffi_tainted` propagates transitively and **cannot be laundered** — a pure-Jinn
wrapper around an FFI call is still tainted. `--deny ffi.unsafe` is enforced by
refusing to discharge the taint, so the build fails with the taint's origin call
path.

A binary that declares fewer capabilities than it uses is its publisher's signed
lie, not a hole in inference. The taint marks the trust boundary; signing and
provenance are what back it.

---

# Path-scoped package identity

Today's string-prefix model cannot represent two instances of the same package,
a dependency scoped to its parent, or a version, owner-scope, or content hash
attached to a symbol — all of which are load-bearing for lamp. So identity
becomes a first-class compiler concept.

## `PackageId`

```
PackageId {
    name:          Symbol,      # the package's own name
    owner_scope:   ScopePath,   # the parent chain, e.g. foo:baz
    version:       SemVer,
    semantic_hash: Blake3,      # the abi-rung hash
}
```

`PackageId` is referenced by the typer, the capability pass, HIR, MIR, codegen,
and the interface, so it must be cheap to clone and hash. It is **interned**: a
`PkgId` is a `u32` handle into a side table in a new `src/pkgid.rs` — its own
interned domain, not crammed into `src/intern.rs`, whose concern is symbols.
`owner_scope` is likewise an interned dotted path rendered `foo:baz`, not a
`Vec<Symbol>`, which keeps `PackageId` copy-cheap and makes scope equality a
pointer compare.

Symbol mangling becomes `<pkgid_hash>_<module>_<name>`, where `pkgid_hash` is a
short collision-checked prefix of the semantic hash. Two promoted instances
share one `pkgid_hash` and therefore one symbol — **promotion *is* "they mangle
to the same name"** — and two non-promotable instances mangle distinctly, which
gives multi-version coexistence for free at the symbol level.

## Resolution is local

Every package instance is scoped to its immediate parent:

```
foo:bar              # foo's direct dependency bar
foo:baz:bar          # bar as seen by baz, which is foo's dependency
```

`use bar` inside `baz` resolves against **`baz`'s own manifest** to
`foo:baz:bar`; the parent's `use bar` resolves to `foo:bar`. The two are
distinct identities by construction, and resolution never consults a global
flattened set. This is npm's nested-resolution insight without npm's
flat-collision arbitration, and it is what makes resolution deterministic.

**The single-package fast path.** `prefix_module` is replaced, not extended. A
build with no manifest assigns the root `PackageId` — empty owner scope, version
`0.0.0` — to everything, and resolution behaves exactly as today. Packages pay
for scoping only when there is a manifest with `requires`, so nearly every small
program sees zero change.

## The unification pass

Path scoping would naively force one copy of a package per scope even when they
are byte-identical. The unification pass recovers global dedup as a
*provably-safe optimization* rather than baking it into resolution, using the
promotion predicate above: equal semantic hashes and both capability rows empty.

Promotion changes symbol count and binary size, never program meaning. For a
monomorphizing value-semantics backend it directly defeats the
version-count × generic-fan-out bloat that pure nested resolution would cause:
equal hash implies equal monomorphization implies one copy.

## The visibility ceiling

A consumer may reach a transitive dependency with a path import (`use baz/bar`),
binding `foo:baz:bar` directly. Reach-in is **open by default**: most of it is
harmless, and forcing every intermediary to enumerate every transitive
dependency it is willing to expose is per-edge bookkeeping on the wrong party —
`baz` does not necessarily know whether `bar` is a stable surface.

Consent therefore lives on the **target**, the one party that knows whether it
is a public surface or a private implementation detail. A package declares its
own ceiling in its own manifest: `public` (the default) or `internal` (visible
only within its own owner-scope subtree).

> A `use <path>` binding target `T` with owner scope `S_t`, requested by a
> consumer whose scope is `S_c`, is legal **iff** `T` is `public`, **or** `S_c`
> is within `S_t`'s subtree (`S_c == S_t`, or `S_t` is a prefix of `S_c`).

A `ScopePath` prefix compare — local, deterministic, monotone, no graph state.
An illegal reach-in errors naming the target's own manifest as the place to
relax the ceiling, and the consumer's scope as the offending site.

## Cross-version boundaries

When two scoped instances are **not** promotable and a value of one crosses into
the other, they are distinct nominal types and the call is **rejected** — the
sound default, made unambiguous by full qualification. The only open decision is
a one-bit policy knob:

- **`exclusive` (default)** — the dependency is private to its scope;
  cross-version coercion is forbidden. A scoped instance's choice of transitive
  version is *not* part of its public ABI.
- **`unify-ok`** — declared by the package that *exposes* the type in a public
  signature, this permits coercion across versions exactly when the **abi**
  hashes match.

Default-exclusive matters: silent unification on incidental hash agreement would
make a package's private choice of dependency part of its ABI, so a patch bump
inside it could flip whether a consumer compiles. The flag is set by the side
that exposes the type, never inferred.

Two incompatible majors may coexist deliberately (`use foo@1.2.0 as foo1`), and
the model supports it — distinct scoped identities, distinct abi hashes, never
promoted, capabilities unioned separately. The one hazard is coherence: if Jinn
grows protocols, both versions could implement the same trait for a shared type.
Until coherence is specified, two live versions that would produce overlapping
instances on a type they do not both own are rejected under a version-aware
orphan rule.

---

# Interface v2 and the three-hash ladder

## What `interface.jhi` must carry

Per public item:

- **generic signatures and inferred constraints**, so a downstream `use`
  type-checks generics without the source;
- **value-semantics, borrow, and modifier annotations**, so ownership invariants
  hold across the binary boundary — a binary cannot quietly relax ownership;
- **effect rows: the error row and the capability row**, so a binary cannot hide
  I/O;
- **Perceus refcount obligations** on returned and consumed values, so the
  caller's generated dup and drop are correct against pre-compiled MIR;
- **serialized canonical MIR** of public bodies eligible for cross-`.jnb`
  inlining and specialization.

The closed `IType` enum cannot express the type system and is **replaced** by a
serialized `types::Type` plus ownership annotations — the compiler's real type,
not a parallel header type. The interface becomes a faithful projection of the
typed HIR's public surface, versioned independently of the package version.

**Layout authority.** The abi hash needs authoritative layout — field order and
size, sret threshold, enum tag and niche. These are computed by a
**target-independent layout pass** consuming a `TargetLayout` descriptor, *not*
read back out of LLVM. The pass is pure (`TargetLayout × Type → Layout`), so it
is deterministic and reproducible without a backend. LLVM consumes the same
layout; it is never the source of truth for the hash.

## Three hashes, three jobs

| Rung | Over | Computed on | Drives |
| --- | --- | --- | --- |
| **api** | names, arity, nominal type identities only | canonical typed MIR (target-independent) | warn only; never auto-coerce |
| **abi** | recursive Merkle closure of the full transitive type, ownership, and effect graph of the used surface | canonical typed MIR **+ `TargetLayout`** (per target) | coercion when the boundary flag permits; promotion |
| **object** | final compiled object bytes | native object | store keying, link dedup, reproducible `--verify` |

The invariant: **semantic hashes (api, abi) decide compatibility and promotion;
the binary hash (object) decides artifact identity.** Conflating them breaks
both — a binary hash over object code rarely matches across builds, defeating
unification, and a semantic hash cannot dedup identical final objects.

## Why the abi rung cannot be a shallow signature hash

`f(c: Config)` keeps the same **api** hash when `Config` silently gains a field
or widens an `i32` to an `i64`, yet that flips both layout and Perceus glue. For
a value-semantics language with no boxing escape hatch, a shallow hash is a
silent memory-corruption generator. The abi hash is therefore a **recursive
Merkle hash over the entire reachable type graph**, covering, for every type
reachable through the used surface:

- field layout, order, and size;
- the Perceus ownership protocol — which fields are owned versus borrowed, drop
  and reuse glue — as part of the ABI, not an afterthought;
- the capability and error rows, so *acquiring* a capability between two
  versions flips the abi hash at identical data layout. That is correct: gaining
  an effect is a behavioural and ABI change;
- calling-convention determinants (sret threshold, enum tag and niche layout).

The abi rung is **per target**, because layout, sret threshold, and niche
availability are target-dependent. A `.jnb` records `(target → abi_hash)`;
pretending otherwise would let a `wasm32` consumer reuse a native layout.

## The Merkle algorithm, pinned

The closure is a *graph*, not a tree, and the hash must be byte-for-byte
identical across two compilers of the same toolchain version, so the algorithm
is fixed:

1. **Collect** the used-surface type graph.
2. **Condense to SCCs** (Tarjan), hashing each SCC as a unit so cycles are
   well-defined.
3. **Canonically order within an SCC** by a structural pre-key — kind tag, field
   count, sorted field-name hashes — breaking cycles deterministically; true
   structural symmetry is resolved by the canonical MIR serialization order,
   which is itself sorted and normalized.
4. **Hash bottom-up over the condensation DAG.** A node's hash is BLAKE3 of its
   own determinants concatenated with the hashes of its already-hashed
   dependency SCCs. Within an SCC, run a fixpoint with placeholder hashes seeded
   from the pre-key until stable — monotone and finite, so it terminates.
5. The package's `semantic_hash` is the root hash of its public surface.

Two compilers agree because every ordering decision is structural, never
insertion-order or pointer dependent.

The ladder drives diagnostics: object match means unify silently; abi match with
differing objects means "compatible; behaviour may differ between 1.2.0 and
2.0.0"; api match with differing abi means "surface matches but the ABI contract
changed; will not unify" — each naming both fully-qualified identities and their
hashes.

---

# Config blocks

The manifest is "a restricted Jinn dialect". Rather than growing a second
grammar, a **general declarative config-block construct** is parsed by the real
Jinn parser and desugars to ordinary Jinn values. `project.jn` is its first
consumer; the manifest restrictions become a *validation lint over the parsed
block*. No second config language, by construction.

## Grammar

A config block is an indentation-scoped block of entries, reusing class-body
indentation rules — no new lexer mode.

```
config-block   := block-head NEWLINE INDENT entry+ DEDENT
entry          := key ws clause-list NEWLINE
clause         := "is" value | connector value | nested-block
value          := literal | list | map | path | nested-record
connector      := "requires" | "from" | "path" | "git" | "rev"
                | "opt" | "with" | "enables" | "visibility"   # closed, schema-driven
```

The apparent ambiguity — `release is opt 3, debug-info false` mixes `is`
bindings, bare keywords, and comma lists — resolves on one structural rule:

> Inside a config block, an entry is **always** `key clause-list`. The leading
> token of an entry is always a key in binding position, and what follows is
> always a clause list, never an expression statement.

So `opt 3` is not a call and not a binary expression; it is a **connector
clause** whose keyword comes from a closed, schema-supplied set, and the
connector reading is active *only* in config-block entry position. Outside a
config block `opt` is an ordinary identifier. Commas separate clauses of one
entry; entries are newline-separated. A nested block is detected exactly as a
class body is: a key followed by newline and indent.

The parser is an LL(1) decision at entry start and at clause start, with no
backtracking, because config-block entry position is a **distinct grammatical
context** the parser enters at the block head and leaves at dedent.

Three shorthands, pinned: `key is value` (canonical), `key value` (sugar for the
same when the value is a single literal or list — the lint normalizes it), and
`key connector value, connector value` (which builds a record).

## Desugaring

A config block desugars to a nested record literal. The block head names the
field, entries become fields, clause lists become field values, nested blocks
become nested records:

```jinn
build
  toolchain is '^0.4'
  profiles
    release is opt 3, debug-info false
```

becomes

```jinn
build is {
  toolchain is '^0.4',
  profiles is { release is { opt is 3, debug-info is false } },
}
```

That right-hand side is already valid Jinn. There is exactly one evaluation
model — Jinn value construction — and one type model, the record's inferred type
checked against a schema.

## Schemas

A config block is validated against a declared record type. The manifest schema
is the first one, defined once and used to confirm that every key is known
(unknown key gives a precise diagnostic with the nearest valid key suggested),
that each value has the right type, and that the connectors used are legal in
that position — `with` is legal in a dependency clause but not in a profile.

Schemas are how the general construct stays safe: the grammar is open, and the
schema closes it per consumer. The same construct is available in ordinary `.jn`
source for store schemas, build configuration, and any declarative data.

## The restriction lint

The manifest is "parsed, never executed" because a *lint over the parsed block*
forbids the constructs that would execute — calls, control flow, `*name`
definitions — not because the parser is a separate, weaker thing. The `for each`
fleet form in `deployments` is a declarative fan-out over a literal list, not a
loop, and has no more execution power than the rest.
