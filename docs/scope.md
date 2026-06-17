# package & module scopage — first-class identity

**Status: decided spec.** This makes lamp.md §5.7.1 (path-scoped identity) real
in the compiler. It replaces the `prefix_module` string-mangling model
(`src/resolve.rs`) with a first-class `PackageId` threaded from resolution
through HIR, MIR, and codegen. Read `caps.md` first (the cap row and `state`
region identity hang off `PackageId`).

---

## 0. The shape of the problem

Today modules are merged by string-prefixing: `prefix_module` rewrites `*name` →
`module_name` and `Type` → `Module_Type`, after which there are no modules, just
a flat mangled global namespace. That model **cannot represent**:

- two instances of the same package (`foo@1`, `foo@2`);
- a dependency scoped to its parent (`foo:baz:bar` vs `foo:bar`);
- a version, owner-scope, or content hash attached to a symbol.

Every one of those is load-bearing for lamp.md §5.7. So identity becomes a
first-class compiler concept.

---

## 1. `PackageId`

```
PackageId {
    name:          Symbol,        # the package's own name (bar)
    owner_scope:   ScopePath,     # the parent chain (foo:baz)
    version:       SemVer,
    semantic_hash: Blake3,        # the abi-rung hash (interface-hash.md §3.3)
}
```

### 1.1 Storage — interned handle + side table (decided)

`PackageId` is referenced by typer, caps, HIR, MIR, codegen, and interface; it
must be cheap to clone and hash. **Decision:** intern it. A `PkgId` is a `u32`
handle into a side table (`src/pkgid.rs`, a new module, *not* crammed into
`src/intern.rs` whose concern is symbols). Symbols stay symbols; `PkgId` is its
own interned domain. Everything downstream clones and hashes the `u32`.

### 1.2 `ScopePath` — interned dotted symbol (decided)

`owner_scope` is an interned dotted path rendered `foo:baz`, not a
`Vec<Symbol>`. Interning the whole path keeps `PackageId` `Copy`-cheap and makes
scope equality a pointer compare. Display renders the `:`-joined form; the empty
scope (the root project) renders as nothing (`bar`, not `:bar`).

### 1.3 Mangling

Symbol mangling becomes `<pkgid_hash>_<module>_<name>`, where `pkgid_hash` is a
short prefix of the `semantic_hash` (collision-checked at link). Two promoted
instances (caps.md §4.1: equal hash, empty cap rows) share one `pkgid_hash` and
therefore one symbol — promotion *is* "they mangle to the same name." Two non-
promotable instances mangle distinctly, giving the §5.7.6 multi-version
coexistence for free at the symbol level.

---

## 2. Path-scoped resolution

### 2.1 Local, deterministic, no global arbitration

`use bar` inside package `baz` resolves against **`baz`'s own manifest** to
`foo:baz:bar`. The parent's `use bar` resolves to `foo:bar`. The two are
distinct `PackageId`s by construction. Resolution is purely local: a package's
imports are resolved against *its* manifest's `requires`, never against a
global flattened set. This is npm's nested-resolution insight without npm's
flat-collision arbitration, and it is what makes the whole thing deterministic.

### 2.2 The single-package fast path (decided)

`prefix_module` is **replaced**, not extended. A non-package build (a bare
`.jn`, no `project.jn`) assigns the root `PackageId` (empty `owner_scope`,
version `0.0.0`, hash computed normally) to everything. Resolution then behaves
exactly as today — one identity namespace, no scoping cost. Packages only pay
for scoping when there is a manifest with `requires`. **98% of small programs
see zero change.**

### 2.3 `state` region identity

A stateful module's `state <region>` cap (caps.md §4.2) is parameterized by an
interned region derived from its `PackageId`. Two scoped instances of the same
stateful package therefore have distinct regions ⇒ distinct state by
construction, even when their code is byte-identical. `PackageId` is the root of
state identity; this is why caps.md hangs the region off it.

---

## 3. The unification pass

Path-scoping would naively force one copy of `bar` per scope. The unification
pass recovers global dedup as a *provably-safe optimization* (lamp.md §5.7.2),
now with caps.md §4.1's tightened, decided predicate:

> Two path-scoped instances `A` and `B` are **promoted** to one identity — owner
> -scope and version dropped from the symbol, one copy of code, one symbol set —
> **iff** their `semantic_hash`es are equal **and** both cap rows are **empty**
> (`caps == ()`).

The empty-row rule (not merely `state`-free) is the airtight version from
caps.md §4.1: a module that reads the clock is `state`-free but non-empty, so we
share its *code* but never collapse its *identity*. This removes the last class
of surprise (Q1) at the cost of a little dedup on impure-but-stateless utilities.

Promotion changes symbol count and binary size, never program meaning. For the
monomorphizing value-semantics backend it directly defeats version-count ×
generic-fan-out bloat: equal hash ⇒ equal monomorphization ⇒ one copy.

---

## 4. Visibility ceiling — reach-in, default-open

A consumer may reach a transitive dependency with a **path import** `use
baz/bar`, binding `foo:baz:bar` directly. Reach-in is **open by default**: most
of it is harmless, and forcing every intermediary (`baz`) to enumerate every
transitive dep it is willing to expose is per-edge bookkeeping on the wrong
party — `baz` does not necessarily know whether `bar` is a stable surface.

Consent therefore lives on the **target** (`bar`), the one party that knows
whether it is a public surface or a private implementation detail. `bar`
declares its own **visibility ceiling** in *its own* manifest:

- **`public` (default)** — no declaration; any consumer may path-import it.
- **`internal`** — visible only within `bar`'s own owner-scope subtree: its
  parent scope and that scope's descendants. A reach-in from a grandparent, a
  sibling, or an external package is a **hard error**.

This is `pub(in path)` / package-private applied at the package-identity layer,
not the symbol layer. It is a closed two-rung lattice (`public > internal`),
consistent with the cap/effect lattices.

### 4.1 The enforcement predicate (decided)

A `use <path>` binding target `T` (owner scope `S_t`) is requested by a consumer
whose own scope is `S_c`. The import is legal **iff**

> `T` is `public`  **or**  `S_c` is within `S_t`'s subtree
> (`S_c == S_t` or `S_t` is a prefix of `S_c`).

`S_t` is `T`'s `owner_scope` (e.g. `bar`'s is `foo:baz`), so "within the subtree"
means the consumer lives at `bar`'s parent (`baz`) or deeper. A `ScopePath`
prefix compare — local, deterministic, monotone, no graph state. An illegal
reach-in errors naming `T`'s own manifest as the place to relax the ceiling, and
the consumer's scope as the offending site.

---

## 5. Multi-version coexistence — Q4 resolved

lamp.md §5.7.6 admits two live majors via `use foo@1.2.0 as foo1` / `use
foo@2.0.0 as foo2`. The model supports the *identities* (`foo:foo1`, `foo:foo2`
distinct, distinct abi hashes, never promoted, caps unioned separately). The one
hazard is **coherence**: if Jinn grows protocols/typeclasses, `foo@1` and `foo@2`
could each `impl Show for SharedType`, creating two candidate instances at an
ambiguous use site. Coherence is unspecified and traits are not yet shipped.

**Decision: defer multi-version coexistence to post-traits; hard-reject it now.**

- The resolver **accepts** the `as foo1 / as foo2` *parse* (the surface is part
  of the language), but the build **rejects** any graph that actually
  instantiates two live majors of the same package, with a clear, honest
  diagnostic: *"multi-version coexistence of `foo` (1.2.0 and 2.0.0) is not yet
  supported; it is gated on coherence/traits (lamp.md §5.7.6). Pick one major."*
- This is choice (a) from prereqs §5 Q4: simplest and honest. We do not ship half
  a coherence design pinned to an absent feature. When traits land, the version-
  aware orphan rule is specified *then*, and the reject is lifted exactly where
  no overlapping instance exists.
- The `PackageId` model already represents both versions distinctly, so lifting
  the restriction later is additive — no rework, just removing the guard once the
  orphan rule exists.

---

## 6. Workspace flattening — `members`

`members` (lamp.md §2.4) flattens child-project outputs into one resolved DAG
with one identity namespace and one `project.lock` at the parent root. In the new
model:

- Each member is resolved with its own manifest (path-scoped, §2.1), then its
  outputs are inserted into the parent's identity namespace.
- Output names must be **globally unique across the tree**; a collision is a hard
  error naming *both* sites (file:line of each `provides`).
- All members resolve the **same external version per package** (one version per
  package across the tree, lamp.md §3.1) — the workspace eliminates the diamond
  hazard internally; the resolver enforces it during the parent-root resolution.
- A `src/workspace.rs` phase (lamp.md §12 phase 3) feeds the new identity model;
  it does **not** route through `prefix_module` (which is gone).
- Hierarchical → standalone promotion (lamp.md §2.4) is just publishing a member:
  its `PackageId` gains an owner-scope from the registry path instead of the
  parent tree, with no source change at the consumer beyond `requires planner` →
  `requires planner from <registry>`.

---

## 7. The boundary flag (lamp.md §5.7.4) — unchanged, restated for completeness

When two scoped instances are **not** promotable and a value of one crosses into
the other, they are distinct nominal types and the call is **rejected** (sound
default). The one policy knob is one bit, set by the *exposer*:

- **`exclusive` (default)** — dep is private to its scope; cross-version coercion
  forbidden; reject stands. A scoped instance's transitive-version choice is not
  part of its public ABI.
- **`unify-ok`** — declared by the package that exposes the type in a public
  signature; permits coercion across versions **exactly when the abi hashes
  match** (`interface-hash.md` §3.3). Never inferred — inferring it would make a
  package's private dep choice into part of its ABI.

This is consistent with the `PackageId` model: the flag rides on the exposed
type's `PackageId`, and abi-hash equality is the precondition the flag gates.

---

## 8. Cross-references

- `semantic_hash` / abi rung definition → `interface-hash.md` §3.3.
- Cap rows that gate promotion → `caps.md` §4.1 (empty-row rule).
- `visibility`, `requires`, `members` as manifest config blocks →
  `config-blocks.md` §4.
- Retiring `prefix_module` → `src/resolve.rs` (implementation note, phase 2 of
  lamp.md §12).
