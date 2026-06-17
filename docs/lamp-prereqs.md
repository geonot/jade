# lamp — compiler prerequisites

**Status: design / planning (prereqs for task 2-19). This document does not
re-specify lamp; it specifies the deep compiler/language machinery lamp's
design (`docs/lamp.md`) silently assumes but that does not yet exist in
`jinnc`.** It is the bridge between the package-manager spec and the compiler.

Read `docs/lamp.md` first. Section references below (§5.6, §5.7, §6.1, …) point
into it.

> **Resolved.** The four workstreams and the §5 open-question register below are
> now turned into *decided* spec across four documents, in dependency order:
> 1. **`docs/caps.md`** — capabilities as a clean, **separate** pass (resolves
>    Q1, Q6, Q7; supersedes §2.4's "one merged `{errors, caps}` fixpoint" lean —
>    see §2.4 note).
> 2. **`docs/scope.md`** — `PackageId` & path-scoped identity (resolves Q4).
> 3. **`docs/interface-hash.md`** — interface v2 + three-hash ladder (resolves
>    Q2, Q5, Q9).
> 4. **`docs/config-blocks.md`** — config sugar as real compilable Jinn
>    (resolves Q3).
>
> This document is retained as the *rationale & current-reality* record; the
> decided semantics live in those four docs and the amended `docs/lamp.md`.

---

## 0. Why this document exists

`docs/lamp.md` is a complete *package-system* design. But four of its load-
bearing claims are not statements about a package manager at all — they are
statements about the **language and compiler**:

1. **"Resolution names everything by path"** (§5.7.1) — Jinn today flattens
   modules with a `module_name` string prefix (`src/resolve.rs::prefix_module`).
   There is no notion of a *scoped package identity* (`foo:baz:bar`), no version
   or owner-scope in a symbol, and `use` cannot disambiguate two versions of the
   same package. lamp's entire resolution model rests on an identity the
   compiler cannot currently represent.

2. **"Every package declares the capabilities it requires … the compiler
   *derives* the actual effect set from the code"** (§6.1). Jinn has an
   error-effect system (`docs/error-effects.md`) and nothing else. There is no
   capability lattice, no effect row carrying `net.client`/`fs.read`/…, no
   derivation pass, no declaration syntax at module/class/fn/method scope, and
   no derived-vs-declared containment check. Capabilities are referenced ~40
   times across lamp.md as if they exist; they do not.

3. **"The three-rung hash ladder … the abi hash is a recursive Merkle hash over
   the entire reachable type graph"** (§5.7.3, §5.6.6). Jinn has a v1
   `src/interface.rs` (a flat `.jhi`-precursor: function signatures, a closed
   `IType` enum, **no** ownership/effect/Perceus metadata, **no** hashing). The
   abi/api/object ladder, used-surface hashing, and the deep Merkle closure are
   entirely unbuilt — and they are the correctness anchor of the whole binary
   story.

4. **The manifest is "a restricted Jinn dialect"** (§2.1) with `project`,
   `requires`, `provides`, `build … profiles`, `features`, capability blocks,
   etc. The parser (`src/pkg.rs::Package::parse`) understands a small subset and
   hand-rolls it. There is no clean, general **config-block grammar** — the
   nested `build / profiles / release is opt 3, debug-info false` shape in
   lamp.md is not expressible by today's parser, and we do not want a bespoke
   ad-hoc reader. This is a *language* question (a little sugar), not just a
   pkg.rs question.

This document turns those four into concrete compiler workstreams, plus a
**§5 open-questions register** for the skeletons, anti-features, and usability
hazards still lurking in lamp.md.

---

## 1. Package / module scope (lamp.md §5.7.1, §2.4)

### 1.1 Current reality

- Modules are merged by **string prefixing**: `prefix_module` rewrites every top
  -level `*name` to `module_name` and every type to `Module_Type`
  (`src/resolve.rs`). After this pass there are no modules — just a flat global
  namespace with mangled names.
- `UseDecl` (`src/ast.rs:560`) names a module to splice in. There is no version,
  no owner-scope, no content identity attached to the import.
- Therefore the compiler **cannot represent** two instances of the same package
  (`foo@1` and `foo@2`), cannot scope a dependency to its parent
  (`foo:baz:bar`), and cannot attach a hash/version/owner to a symbol — all of
  which §5.7 requires.

### 1.2 Target

A first-class **package identity** threaded from resolution through HIR, MIR,
and codegen mangling:

```
PackageId { name: Symbol, owner_scope: ScopePath, version: SemVer, semantic_hash: Blake3 }
```

- **Path-scoped resolution.** `use bar` inside package `baz` resolves against
  `baz`'s manifest to `foo:baz:bar`; the parent's own `use bar` resolves to
  `foo:bar`. Distinct identities, purely local resolution (no global
  arbitration). Symbol mangling becomes `<pkgid-hash>_<module>_<name>` instead
  of today's flat `module_name`.
- **Visibility ceiling.** `use baz/bar` (path import) is open by default; the
  *target* `bar` may declare `visibility internal` in its own manifest, confining
  it to its owner-scope subtree. A reach-in from outside that subtree is a hard
  error (scope.md §4). Requires a `visibility` manifest/AST notion the compiler
  enforces at resolve via a `ScopePath` prefix check.
- **Multi-version coexistence.** `use foo@1.2.0 as foo1` / `use foo@2.0.0 as
  foo2` must produce two non-unifiable identities with separately-mangled
  symbols and separate cap unions (§5.7.6). The orphan/coherence guard (reject
  two live versions that would both `impl Show for SharedType`) lands here too —
  even before traits ship, the resolver must refuse the ambiguous case.
- **Workspace flattening.** `members` (§2.4) flattens child-project outputs into
  one DAG with one identity namespace; collisions are a hard error naming both
  sites. Needs `src/workspace.rs` (phase 3 in lamp.md §12) feeding the new
  identity model, not the string-prefix pass.

### 1.3 Decisions to make

- Does `PackageId` live in `src/intern.rs` (interned) or a new `src/pkgid.rs`?
  It is referenced by typer, HIR, MIR, codegen, interface — it must be cheap to
  clone and hash. **Lean: interned handle + side table.**
- How does the existing `prefix_module` retire? It cannot coexist with scoped
  identity. **Lean: replace it; keep a single-package fast path that assigns the
  root `PackageId` so non-package builds are unaffected.**
- Owner-scope representation: `ScopePath` as a `Vec<Symbol>` (`[foo, baz]`) vs.
  an interned dotted symbol. **Lean: interned, with a display that renders
  `foo:baz`.**

---

## 2. Capabilities, formalized (lamp.md §6.1, §5.5, §5.7.2)

This is the largest and most cross-cutting prerequisite, and it does triple duty
in lamp.md:

- **supply-chain** — the declared/derived cap set is the reviewable lockfile
  diff (§6.1);
- **unification safety** — "state is a capability, not an ambient" is the
  *keystone* that makes promoting provably-identical packages sound *for free*
  (§5.7.2);
- **abi identity** — the cap/effect signature is part of the abi Merkle hash, so
  gaining an effect is an ABI change (§5.7.3).

So capabilities are not a packaging feature bolted on at the end; they are a
**type-system feature** that the package manager *reads*.

### 2.1 The capability lattice

A closed set of capability classes (lamp.md §6.1), each possibly path/resource-
scoped:

```
fs.read <path>   fs.write <path>
net.client       net.server
process.spawn    env.read
clock            time
random           ffi.unsafe
state            # module-level mutable state (the §5.7.2 keystone)
```

- Capabilities form a lattice (a function's required set is the **union** of its
  body's; the project ceiling is a **superset** check). `state` is special: it
  is what distinguishes a promotable (stateless) module from a non-promotable
  one (§5.7.2). Decide whether `state` is one cap or parameterized by the state
  type.
- Path-scoped caps (`fs.read './assets'`) need a containment relation
  (`fs.read './assets/x'` ⊑ `fs.read './assets'`).

### 2.2 Declaration sites (the user's explicit requirement)

Capabilities must be declarable at **four** scopes, each with clean Jinn syntax
to design:

1. **Module level** (top of file) — the file's ceiling:
   ```jinn
   capabilities net.client, fs.read './assets'
   ```
2. **Class / store level** (annotation on the type):
   ```jinn
   @caps(net.client)
   class HttpClient
       ...
   ```
3. **Function level** (annotation):
   ```jinn
   @caps(fs.write './out')
   *save(data) ...
   ```
4. **Method level** (annotation on a method within a class).

Open syntax question: annotation form `@caps(...)` vs. a trailing clause
(`*save(data) needs fs.write` ) vs. an effect-row in the signature. Must be
idiomatic Jinn (implicit, inferred-by-default, annotation narrows — mirroring
the error-effect design where `! E` *narrows and documents* an inferred set).
**Lean: inferred by default; annotation is a checked upper bound, same shape as
error effects.** This keeps one mental model across both effect systems.

### 2.3 Derivation + checking pass

- A pass (typer or post-typer, alongside `src/typer/errset.rs`) **derives** the
  capability set of every function via the call graph (SCC least-fixpoint, like
  inferred fallibility): a fn's caps = union of its own primitive cap uses + the
  caps of everything it calls.
- **Primitive cap sites**: which stdlib/runtime surfaces *introduce* a cap
  (`std/net` → `net.client`, `std/fs` → `fs.read/write`, `ffi.unsafe` blocks →
  `ffi.unsafe`, module-level `var`/store → `state`). Needs a cap-attribution
  table — the single-source-of-truth pattern (cf. `src/store_decorators.rs`).
- **Check**: derived ⊑ declared (module/class/fn). Violation = precise
  diagnostic ("uses `net.client` but declares only `fs.read`").
- **FFI taint** (§5.5): `ffi.unsafe` and binary `object/` symbols are *declared
  ground truth*, derivation switches to one-directional containment, and the
  taint cannot be laundered away by a dependent. `lamp build --deny ffi.unsafe`
  must be enforceable at compile time (effect cannot be discharged).

### 2.4 Decisions to make

- Reuse the error-effect infrastructure (`errset.rs`, the SCC fixpoint in
  `src/typer/scc.rs`) or build a parallel cap-effect pass? ~~Lean: generalize
  the effect machinery to carry a `{ errors, caps }` effect record, one
  fixpoint, two payloads.~~ **DECIDED (`docs/caps.md` §3.1): a separate cap
  pass.** Errors and caps are different facts with different propagation rules
  (caps flow through *every* call + primitive sites + function-typed values;
  errors flow through `?`/`!`/handlers) and different diagnostics. They share
  only the generic SCC least-fixpoint *utility* (`src/typer/scc.rs`,
  parameterized over the lattice), and the interface stores them as sibling rows.
  One pass producing two unrelated outputs was a false economy.
- Where do caps live on the HIR/MIR fn? They must reach `interface.jhi` (§3) and
  the abi hash, so they belong on the typed signature, not a side channel.
- Is `state` inferred from module-level mutable bindings + `@store` handles
  reliably enough to make §5.7.2's "free" purity proof actually hold? This is
  the single highest-risk correctness claim in lamp.md — see §5 register Q1.

---

## 3. The interface file and the three-hash ladder (lamp.md §5.3, §5.6.6, §5.7.3)

### 3.1 Current reality

`src/interface.rs` is a v1 `.jhi`: `InterfaceFile { version, module, functions:
Vec<FnSig> }` over a closed `IType` enum. It carries **no** ownership metadata,
**no** effect/capability rows, **no** Perceus obligations, **no** generic
constraints, and is **not hashed**. It is a header, not the "serialized slice of
typed HIR" §5.3 demands.

### 3.2 Target: `interface.jhi` v2

Per §5.3, the interface must let a downstream compiler type-check `use`s
*exactly as if it had the source*:

- generic signatures + HM-inferred constraints;
- value-semantics / borrow / `as` annotations (so `tests/access_semantics.rs`
  invariants hold across the binary boundary);
- **effect rows + declared capability set** (§2) — a binary cannot hide I/O;
- **Perceus refcount obligations** on returned/consumed values, so the caller's
  generated dup/drop is correct against pre-compiled MIR;
- enough to keep cross-`.jnb` inlining/specialization possible (serialized MIR).

This is a real serialization of the typed HIR/MIR surface, not the flat `IType`
table. The closed `IType` enum must either grow to cover the full type system or
be replaced by a serialized `types::Type` + ownership annotations.

### 3.3 The three hashes (§5.7.3, §5.7.5)

Strictly separated — **semantic hashes decide compatibility/promotion; the
binary hash decides artifact identity** (§5.7.5):

| Rung | Over | Computed on | Drives |
|------|------|-------------|--------|
| **api** | names + arity + nominal type identities only | canonical typed MIR | warn-only; never auto-coerce |
| **abi** | **recursive Merkle closure** of the full transitive type/ownership/effect graph of the *used surface* | canonical typed MIR | coercion across versions when boundary flag = `unify-ok`; promotion |
| **object** | the final compiled object bytes | native object | store keying, link-time dedup, reproducible-build `--verify` |

The **abi rung is the load-bearing, correctness-critical one** and is explicitly
forbidden from being a shallow signature hash. It must be a **Merkle hash over
the entire reachable type graph (with cycle handling)** including:

- field layout, order, size of every reachable type;
- the Perceus ownership protocol (owned vs borrowed fields, drop/reuse glue);
- the capability/effect signature (so *acquiring* a cap between v1 and v2 flips
  the hash at identical data layout — correct: gaining an effect is an ABI
  change);
- calling-convention determinants (sret threshold, enum tag/niche layout).

**Used-surface hashing** (§5.6.6): the abi hash for a *dependency edge* is over
only the subset of the dependency's interface the consumer's MIR actually
references — the compiler already computes the import set during type-checking,
so the used-surface hash is a deterministic fold over exactly that set. This is
what makes the binary fast-path actually fire under normal semver evolution.

### 3.4 Decisions to make

- The Merkle closure needs the compiler's authoritative **layout** model. Does
  that live in codegen (LLVM-driven) or a target-independent layout pass? The
  abi hash must be **target-independent** (it is over canonical MIR, §5.7.5), so
  layout determinants must be computable without LLVM. **Open — see §5 Q2.**
- Cycle handling for recursive types (Merkle over a graph, not a tree): standard
  approach is to hash SCCs with a fixpoint/placeholder scheme. Pin the algorithm
  so two compilers agree byte-for-byte (reproducibility, §7).
- Canonical MIR serialization format + version. Must be stable and
  deterministic (sorted, normalized) — it is hashed.

---

## 4. Config-block syntax sugar for `project.jn` (lamp.md §2, §2.1)

### 4.1 Current reality

`Package::parse` (`src/pkg.rs`) hand-reads a small manifest. The richer nested
shapes in lamp.md — `build / profiles / release is opt 3, debug-info false`,
`provides / lib orchard-core is '...' / requires internal`, `features / postgres
is requires ext/pq` — are **not** expressible by today's parser.

### 4.2 The question: is this a language feature or a pkg.rs feature?

lamp.md §2.1 says the manifest is a *restricted Jinn dialect*. Two paths:

- **(a) Parse with the real Jinn parser, restrict semantically.** Requires the
  language to *have* a clean nested-config-block grammar — an indentation-based
  block that introduces a scope of `is`-bindings to literals plus a few keywords
  (`requires`, `from`, `git`, `rev`, `opt`, `with`, `enables`). This is the
  small **syntactic sugar** the user anticipated. It would also be reusable for
  other declarative blocks (store schemas, build config) — net language win.
- **(b) Keep a bespoke manifest reader in pkg.rs.** Faster to ship, but grows a
  second config language by accident — exactly what §2 swears off ("No second
  config language").

**Lean: (a).** Design a general **declarative block** construct: an
indentation-scoped block of `key is <literal | list | nested-block>` plus a
whitelisted set of declarative connector keywords, parsed by the real parser and
validated against a schema. The manifest becomes the first consumer; the schema
+ restriction (§2.1: no calls, no control flow, no `*name`) is enforced as a
*lint/validation pass* over the parsed block, yielding the precise diagnostics
§2.1 promises.

### 4.3 Decisions to make

- Exact grammar of a config block. Does it reuse class-body indentation rules?
  How are `release is opt 3, debug-info false` (multiple settings on one line)
  and `postgres is requires ext/pq, enables 'metrics'` parsed — as a comma
  list of mini-clauses? **Open — see §5 Q3.**
- Is the construct general (usable in normal `.jn` source) or manifest-only? A
  general construct is more valuable but a larger surface to stabilize. **Lean:
  general but initially only the manifest schema is wired; document it in
  `docs/stability.md` as provisional.**

---

## 5. Open questions / refinement / skeletons in lamp.md

A candid register of things in `docs/lamp.md` that are under-specified, risky,
or smell like anti-features. Each needs a decision before or during
implementation.

**Q1 — Is "state is a capability" actually free? (§5.7.2, the keystone).**
The soundness of the *entire* unification/promotion optimization rests on the
claim that a module with no `state` cap in its signature is *provably* free of
hidden singletons, so identical-hash modules collapse safely. But Jinn has
module-level mutable bindings, `@store` handles, actor mailboxes, the scheduler,
interners. Is `state` *reliably* inferred for **all** of these? A single hidden
ambient (e.g. a lazily-initialized cache, an FFI global, a `clock`/`random`
source not modeled as a cap) silently breaks the proof and lets unification
alias two things that must stay distinct. **This is the highest-risk claim in
the spec.** Action: enumerate every source of process-wide observable state in
the runtime + stdlib and prove each is cap-tagged, *or* downgrade promotion from
"free/unconditional" to "requires an explicit `@pure`/`@promotable` opt-in
verified by the cap pass." Recommend the conservative opt-in until the
enumeration is exhaustive.

**Q2 — Target-independent layout for the abi hash (§5.7.3 vs §5.7.5).**
§5.7.5 insists abi/api are folded over *target-independent* canonical MIR, yet
the abi hash must include "field layout, order, size … sret threshold, enum
tag/niche layout" — which are **target-dependent** (pointer size, alignment,
niche availability differ across `native`/`wasm32`). These two statements are in
tension. Either the abi hash is **per-target** (then a `.jnb` carries one abi
hash per target it ships, and §5.7.5's "target-independent" is wrong), or it
hashes a target-independent *layout intent* (field order + logical sizes) and
trusts the toolchain to lay out identically per target (then it is not a true
ABI hash for cross-target reuse). **Must be resolved before the hash is
implemented** — it changes the data model. Recommend: abi hash is **per-target**
and `interface.jhi` records `(target → abi-hash)`; drop the "target-independent"
phrasing for the abi rung (keep it only for the api rung).

**Q3 — Config-block grammar ambiguity (§2).** `release is opt 3, debug-info
false` and `metrics is requires ext/prom, enables 'metrics'` mix `is`-binding,
bare keywords, and comma lists in ways the current grammar does not define. Pin
the exact production and make sure it does not collide with expression-statement
parsing (e.g. is `opt 3` a call? a binary expr? a keyword-value pair?). Risk:
manifest parses differently from how a human reads it.

**Q4 — `requires-isolated` + coherence is a partial skeleton (§5.6.4,
§5.7.6).** The spec admits two live majors via `as foo1/as foo2` but then says
coherence is *unspecified* and lamp will "reject two live versions that would
produce overlapping instances … until coherence is specified." Traits/protocols
are themselves not yet shipped. So this feature is **half a design pinned to an
absent feature**. Action: either (a) explicitly defer multi-version coexistence
to post-traits and have the resolver hard-reject it now (simplest, honest), or
(b) specify the version-aware orphan rule concretely. Recommend (a) for the
prereq phase; document the deferral.

**Q5 — MVS vs. the lockfile's `built` floor interaction (§3.2 vs §5.6.2).** A
binary records `built X@1.2.0` as the MVS *floor* for its edge. If another
requester wants `^1.0` (floor 1.0.0) and the binary forces ≥1.2.0, MVS picks
1.2.0 — fine. But if the binary's `built` floor exceeds a *different* binary's
abi-hash-pinned version with no source fallback, you can reach the §5.6.4 hard
error through floors alone, not just range incompatibility. Confirm the resolver
treats the `built` floor and the `iface` hash as **two independent constraints**
and reports which one triggered. Under-documented; clarify the diagnostic.

**Q6 — Effect-derived caps vs. higher-order functions / actor sends.** Cap
derivation via the call graph (§2.3) is clean for direct calls. But Jinn has
first-class functions (`*name`), channels, and actor message sends — a cap can
be "carried" by a function value or a message and discharged elsewhere. Does the
derivation track caps through function-typed values and actor protocols, or does
passing a `net.client`-using closure to a pure-looking higher-order fn launder
the cap? **This is a soundness hole if unaddressed** — the same class of bug as
Q1 but for control flow rather than state. Action: caps on function *types*
(the effect row is part of the fn type), so a `*f` carrying `net.client` makes
its holder cap-tainted. Confirm the type system can carry effect rows on
function types (it must for §5.3's interface anyway).

**Q7 — `libjn` / runtime cap-tagging burden (§1.0, §5.5).** Every primitive cap
site lives in `libjn`/`std`/the C runtime. For derivation to be sound, **every**
syscall-touching runtime entry point must be cap-attributed, and the C runtime
(~6.4k LOC) is opaque to Jinn's effect inference. This is a large, error-prone
manual annotation surface (the FFI gap §5.5 in the small). Action: define the
cap-attribution table as the single source of truth (cf. `store_decorators.rs`),
make missing attribution on a runtime FFI symbol a **build error**, not a silent
"no caps."

**Q8 — Anti-feature check: per-output capability unions vs. ergonomics (§6.1).**
Showing per-output cap unions is good for audit but, combined with path-scoped
identity and isolated duplicates, `lamp tree --caps` output could become
unreadable on a real graph. Not a correctness issue — a usability one. Flag for
UX review once real graphs exist; ensure caps roll up to a one-line summary by
default with `--verbose` for the full breakdown.

**Q9 — Reproducibility of the Merkle hash across compiler versions (§7 vs §5.7).
** The abi hash is part of the lock and pins binary compatibility, but it is
computed by the compiler — a compiler change to layout, hashing order, or MIR
canonicalization silently changes every abi hash, invalidating every published
binary's fast path. The "cache keyed by toolchain" framing (§5.5) mostly covers
this, but the abi hash *crosses* toolchain versions when checking a consumer's
pick against a binary's recorded `iface`. Pin: is the abi hash guaranteed stable
across patch toolchain versions, or only within one? Document the contract; it
determines how often binaries silently fall back to source rebuilds.

---

## 6. Suggested build order

The four prereqs are not independent. Recommended sequencing:

1. **Capabilities first** (§2) — they are an input to the interface (§3) and the
   keystone for unification; resolving Q1/Q6/Q7 early de-risks everything else.
   Generalize the error-effect machinery to a `{ errors, caps }` effect record.
2. **Package/module scope** (§1) — the identity model that resolution, mangling,
   and the lock all need; replaces `prefix_module`.
3. **Interface v2 + hash ladder** (§3) — depends on caps (effect rows in the
   interface) and scope (identity in the symbols); resolve Q2/Q9 before coding
   the abi hash.
4. **Config-block sugar** (§4) — independent of the above; can proceed in
   parallel, lowest risk.

The §5 register questions (esp. Q1, Q2, Q6) should be answered as *amendments to
`docs/lamp.md`* before the corresponding code lands, so the spec and the
compiler never diverge.
