# interface v2 & the three-hash ladder

**Status: decided spec.** This makes lamp.md §5.3, §5.6.6, §5.7.3, §5.7.5 real:
the `interface.jhi` v2 file (a serialized slice of typed HIR/MIR, not a flat
header) and the api/abi/object hash ladder. It depends on `caps.md` (the cap row
is in the interface and the abi hash) and `scope.md` (`PackageId.semantic_hash`
is the abi rung). Read both first.

---

## 0. Current reality vs. target

`src/interface.rs` today is a v1 header: `InterfaceFile { version, module,
functions: Vec<FnSig> }` over a closed `IType` enum — **no** ownership, **no**
effect/cap rows, **no** Perceus obligations, **no** hashing. lamp.md §5.3 demands
the opposite: enough serialized typed surface that a downstream compiler type-
checks a `use` *exactly as if it had the source*, plus a deterministic hash
ladder. This document specifies v2.

---

## 1. `interface.jhi` v2 — a serialized slice of typed HIR/MIR

The interface must carry, per public item:

- **generic signatures + HM-inferred constraints** — so a downstream `use`
  type-checks generics without the source;
- **value-semantics / borrow / `as` annotations** — so `tests/access_semantics.rs`
  invariants hold across the binary boundary (a binary cannot quietly relax
  ownership);
- **effect rows: error row + cap row** (`caps.md` §3.1) — so a binary cannot hide
  I/O; the cap row is verbatim the one the cap pass derived;
- **Perceus refcount obligations** on returned/consumed values (owned vs
  borrowed, drop/reuse glue) — so the caller's generated dup/drop is correct
  against pre-compiled MIR;
- **serialized canonical MIR** of public bodies eligible for cross-`.jnb`
  inlining/specialization.

### 1.1 The closed `IType` enum is replaced (decided)

The flat `IType` cannot express the full type system. **Decision:** replace it
with a serialized `types::Type` + ownership annotations — the compiler's real
type, not a parallel header type. The interface becomes a faithful projection of
the typed HIR's public surface, versioned (`interface.jhi` carries a format
version independent of the package version).

### 1.2 Layout authority — target-independent model (decided, feeds Q2)

The abi hash needs the compiler's *authoritative layout* (field order/size,
sret threshold, enum tag/niche). **Decision:** layout determinants are computed
by a **target-independent layout pass** that consumes a `TargetLayout`
descriptor (pointer size, alignment rules, niche availability) — *not* by
reading them back out of LLVM. The pass is pure (`TargetLayout × Type → Layout`)
so it is deterministic and reproducible without a backend. LLVM consumes the same
`Layout`; it is never the source of truth for the hash. This is the hinge that
makes Q2 resolvable cleanly (§3.4).

---

## 2. The three hashes — strictly separated

| Rung | Over | Computed on | Drives |
|------|------|-------------|--------|
| **api** | names + arity + nominal type identities only | canonical typed MIR (target-independent) | warn-only; never auto-coerce |
| **abi** | recursive Merkle closure of the full transitive type/ownership/effect graph of the *used surface* | canonical typed MIR **+ `TargetLayout`** (per-target) | coercion when boundary flag = `unify-ok`; promotion |
| **object** | final compiled object bytes | native object | store keying, link dedup, reproducible `--verify` |

The invariant (lamp.md §5.7.5): **semantic hashes (api/abi) decide
compatibility & promotion; the binary hash (object) decides artifact identity.**
The store (lamp.md §4) keys on `object`; the resolver and unification key on
`abi`/`api`.

---

## 3. The abi rung — the load-bearing one

### 3.1 Why it cannot be a shallow signature hash

`f(c: Config)` keeps the same **api** hash when `Config` silently gains a field
or widens `i32`→`i64`, yet that flips layout *and* Perceus glue. A shallow hash
would be a silent memory-corruption generator for a value-semantics language with
no boxing escape hatch. The abi hash is therefore a **recursive Merkle hash over
the entire reachable type graph**.

### 3.2 What goes into the Merkle closure

For every type reachable through the used surface:

- field layout, order, and size (from the §1.2 layout pass);
- the Perceus ownership protocol (owned vs borrowed fields, drop/reuse glue) —
  part of the ABI, not an afterthought;
- the **cap row and error row** (`caps.md`) — so *acquiring* a cap between v1 and
  v2 flips the abi hash at identical data layout (correct: gaining an effect is a
  behavioural/ABI change);
- calling-convention determinants (sret threshold, enum tag/niche layout).

### 3.3 The Merkle algorithm — pinned for reproducibility

The closure is a *graph*, not a tree (recursive types). The hash MUST be
byte-for-byte identical across two compilers of the same toolchain version, so
the algorithm is pinned:

1. **Collect** the used-surface type graph (§3.5).
2. **Condense to SCCs** (Tarjan). Each SCC is hashed as a unit so cycles are
   well-defined.
3. **Canonical order within an SCC:** sort nodes by a *structural* pre-key
   (kind tag, field count, sorted field-name hashes) to break the cycle
   deterministically; ties (true structural symmetry) are resolved by the
   canonical MIR serialization order, which is itself sorted/normalized.
4. **Hash bottom-up over the condensation DAG.** A node's hash = Blake3 of its
   own determinants (§3.2) ++ the hashes of its already-hashed dependency SCCs.
   Within an SCC, run a fixpoint with placeholder hashes until stable (monotone,
   finite ⇒ terminates), seeding placeholders from the §3 pre-key.
5. The package's `semantic_hash` (`scope.md` §1) is the root hash of its public
   surface under this scheme.

Two compilers agree because every ordering decision is structural, never
insertion-order or pointer-dependent.

### 3.4 Per-target abi hash — Q2 resolved

prereqs §5 Q2 flagged a real tension: §5.7.5 says abi/api are *target-
independent* (canonical MIR), yet the abi closure includes layout determinants
that are *target-dependent* (pointer size, alignment, niche availability differ
across `native`/`wasm32`).

**Decision:**

- The **api** rung stays **target-independent** (names/arity/nominal identities
  have no layout content).
- The **abi** rung is **per-target.** It is folded over canonical MIR **plus the
  `TargetLayout` descriptor** (§1.2). A `.jnb` therefore records `(target →
  abi_hash)` for every target it ships, and `interface.jhi` stores a small
  `abi_hashes: map<Target, Blake3>` rather than a single scalar.
- lamp.md §5.7.5's "target-independent" phrasing is corrected to apply **only to
  the api rung**; the abi rung is explicitly per-target. This is the honest model
  — an ABI *is* a per-target contract, and pretending otherwise would let a
  `wasm32` consumer reuse a `native` binary's layout, which is exactly the memory
  -corruption class §3.1 forbids.
- Promotion (`scope.md` §3) compares the abi hash **for the consumer's current
  target**; cross-target promotion is not attempted.

### 3.5 Used-surface hashing — the fast path (lamp.md §5.6.6)

The abi hash *for a dependency edge* is over only the subset of the dependency's
interface the consumer's MIR actually references. The compiler already computes
the import set during type-checking, so the used-surface hash is a deterministic
fold over exactly that set (a sub-graph of §3.2's closure, hashed by the same
§3.3 algorithm). This is what makes the binary fast-path fire under normal semver
evolution: a dependency can add unrelated public API (new abi hash for the *whole*
interface) while the consumer's *used-surface* hash is unchanged, so its edge
stays compatible and no source rebuild is triggered.

---

## 4. Resolver interaction — Q5 resolved

prereqs §5 Q5: a binary records `built X@1.2.0` as an MVS floor *and* pins an
abi/iface hash. These can interact confusingly.

**Decision: the `built` floor and the `iface` (used-surface abi) hash are two
independent constraints, checked independently, with distinct diagnostics.**

- **Constraint 1 — version floor (MVS).** The binary's `built X@1.2.0` raises the
  MVS floor for that edge to 1.2.0. MVS picks the maximum of all floors as usual.
- **Constraint 2 — abi-hash compatibility.** Independently, the resolved version's
  *used-surface abi hash* (§3.5) must match the hash the binary was built against,
  unless a source fallback recompiles the consumer against the new version.
- The resolver reports **which** constraint triggered a failure:
  - *floor*: "binary `A` requires `X ≥ 1.2.0` (built floor); resolved `1.1.0` from
    `B`'s `^1.0` — raise `B` or rebuild `A`."
  - *hash*: "binary `A` was built against `X` used-surface abi `7d1a…`; resolved
    `X@1.3.0` has abi `9f22…` — incompatible; rebuild `A` from source or pin `X`."
- Reaching lamp.md §5.6.4's hard error through *floors alone* (a binary's floor
  exceeds another binary's hash-pinned version with no source fallback) is a real,
  documented case; the diagnostic names *both* the floor and the conflicting
  hash-pinned edge so the operator sees the actual squeeze, not a generic
  "unsatisfiable."

---

## 5. Cross-toolchain stability — Q9 resolved

prereqs §5 Q9: the abi hash is computed by the compiler, so a compiler change to
layout, hashing order, or MIR canonicalization silently changes every abi hash,
invalidating every published binary's fast path.

**Decision: the abi-hash contract is stable within a *minor* toolchain series,
not across minors.**

- The toolchain version is split `MAJOR.MINOR.PATCH`. The abi-hash algorithm,
  the canonical MIR serialization, the layout pass, and the Merkle ordering are
  **frozen across PATCH releases** of one MINOR series. A patch upgrade therefore
  preserves every binary's fast path: hashes are bit-identical.
- A **MINOR** bump may change the algorithm (e.g. a layout fix). Such a change
  **bumps the `interface.jhi` format version** and is treated as: binaries built
  by an older minor have their abi hashes *re-derived on read* when possible
  (the new compiler re-hashes the serialized interface under the new scheme), and
  fall back to **source rebuild** when re-derivation is not faithful. The fallback
  is loud (a one-line "rebuilding `A` from source: toolchain minor changed the abi
  scheme") so it is never a silent mystery.
- The lock records the **toolchain version** that produced each binary (lamp.md
  §7 reproducibility). Comparing a consumer's abi pick against a binary's recorded
  `iface` is only trusted **within the same minor**; across minors it is re-
  derived or rebuilt. This is the cache-keyed-by-toolchain framing (lamp.md §5.5)
  made precise for the cross-version case.
- **Contract, stated for publishers:** "your published binary's fast path is
  guaranteed for any consumer on the *same toolchain minor*; a consumer on a newer
  minor may transparently rebuild you from source." This is the honest, operable
  promise — no claim of eternal hash stability, no silent breakage.

---

## 6. Canonical MIR serialization

The api/abi hashes fold over **canonical typed MIR**, so its serialization is
part of the contract:

- **Deterministic:** all collections sorted by a structural key (never hash-map
  iteration order); all interned ids rewritten to dense, definition-order indices
  per item; no addresses, relocations, opt-level, or target-triple noise (those
  belong only to the object rung).
- **Versioned:** a `canonical_mir_version` rides in `interface.jhi`; it is frozen
  across patch, bumped on minor (§5).
- This is the same artifact §3.5 hashes for the used surface and §3.3 walks for
  the Merkle closure — one canonical form, three folds over it.

---

## 7. The diagnostics ladder (lamp.md §5.7.3, restated)

- **object** match ⇒ unify silently (link dedup).
- **abi** match, object differ ⇒ *"compatible; behaviour may differ between X
  1.2.0 and 2.0.0"* (coercible if boundary flag permits).
- **api** match, abi differ ⇒ *"surface matches but the ABI contract changed;
  will not unify"* — never auto-coerce, warn only.

Each names both fully-qualified `PackageId`s (`scope.md` §1) and their hashes.

---

## 8. Cross-references

- `semantic_hash` field / promotion → `scope.md` §1, §3.
- Cap/error rows in the closure → `caps.md` §3, §7.
- Boundary flag gating abi-match coercion → `scope.md` §7, lamp.md §5.7.4.
- `built` floor in the lockfile → lamp.md §3.2, §3.3.
