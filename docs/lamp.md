# lamp — the Jinn package system

**Status: design (task 2-19). Specifies the target; supersedes the
git-tag/`jinn.pkg` prototype in `src/pkg.rs`, `src/lock.rs`,
`src/driver/cmd_pkg.rs`.**

> *A lamp is a small, dependable light you carry into unfamiliar code.* lamp
> is Jinn's package manager, registry protocol, and supply-chain trust layer.
> It exists to make depending on other people's code **boring, auditable, and
> reproducible** — the opposite of the sprawling, mutable, unsigned dependency
> graphs that defined the 2015–2025 supply-chain-attack era.

This document is the canonical design. It is long on purpose: package
management is where most languages accrete decades of incoherent tooling. We
specify the whole thing once, coherently, so we never grow an `npm`/`yarn`/
`pnpm`/`bun`/`npx` zoo.

---

## 0. Principles

These derive directly from the Jinn ethos (implicitness, inference, an
intelligent compiler doing the heavy lifting, the programmer expressing
intent) and from a clear-eyed reading of what every prior system got right and
wrong.

1. **One tool.** There is `lamp`. Not `lamp` + `lampx` + `lamp-audit` + a
   third-party resolver. Every verb a developer or maintainer needs —
   scaffold, build, add, lock, vendor, publish, sign, serve, file a bug, cut a
   changelog — is a `lamp` subcommand. The compiler (`jinnc`) stays a compiler;
   `lamp` drives it.
2. **One manifest, one lockfile.** A project is described by `project.jn`
   (human-authored, Jinn syntax) and pinned by `jinn.lock` (machine-authored,
   never hand-edited). Nothing else is authoritative.
3. **Curated by default, open at the edges.** Functionality lives in tiers:
   `std` → `libjn` → `ext` → community. For any one facet there should be
   **one quintessential package** — standards-conformant, minimal, complete,
   idiomatic. lamp actively patronizes the canonical package and makes
   discovering a fourth redundant JSON library a deliberate act, not the
   default.
4. **Trust is explicit and verifiable.** Every artifact is content-addressed
   and signed. Every dependency declares the **capabilities** it needs
   (filesystem, network, process, env, unsafe FFI). The resolver refuses to
   silently widen a package's capability set. Supply-chain safety is a
   first-class feature, not a bolt-on `audit` command.
5. **Reproducible or it didn't happen.** Given a `jinn.lock`, a build on any
   machine at any time produces a bit-identical artifact (modulo declared,
   pinned toolchain). The lockfile records content hashes, not just versions.
6. **Small graphs.** lamp's defaults, economics, and UX all push toward
   *fewer, deeper* dependencies rather than *more, shallower* ones. We treat a
   1,400-package transitive closure as a bug report, not a Tuesday.
7. **Decentralized, federated, durable.** The registry is a protocol, not a
   company. Anyone can `lamp serve`. The canonical index is mirror-able,
   content-addressed, and survives any single host disappearing.
8. **No ceremony in the common case.** `lamp add json` Just Works. Locking,
   hashing, capability inspection, and SBOM emission happen automatically and
   silently unless something is wrong.

---

## 1. The package tiers

Jinn ships functionality in four concentric rings of decreasing curation and
increasing openness. lamp understands all four and treats them uniformly at
the call site — `use json` is `use json` regardless of tier — but applies
different trust and resolution rules.

| Tier        | Location          | Who maintains it             | Versioning            | Trust |
|-------------|-------------------|------------------------------|-----------------------|-------|
| `std`       | `std/*.jn`        | core team, in-tree           | tied to compiler      | implicit, always available |
| `libjn`     | `libjn/*.jn`      | core team, in-tree           | tied to compiler      | implicit, low-level/libc surface |
| `ext`       | `ext/<name>/`     | community, **vendored** into the Jinn repo under project purview | independent semver, reviewed | curated, audited, signed by core |
| community   | external registry | anyone                       | independent semver    | signed by author; capability-gated |

### 1.1 `std` and `libjn` (unchanged)

These are the language's batteries. They are *not* packages you add; they are
always present and version-locked to the compiler that built them. `use math`,
`use json`, `use http` resolve here first. This is deliberate: the standard
library is the answer to "what is the one true X" for the most common Xs.

### 1.2 `ext` — the curated extended library (new)

`ext` is the **innovation of this spec**. It is the bridge between "in the
language" and "out on the open registry." An `ext` package:

- Has its source **vendored into the Jinn repository** under `ext/<name>/`,
  brought under the project's review, license, and CI purview.
- Has passed a contribution review: it is the *quintessential* implementation
  of its facet (e.g. `ext/toml`, `ext/uuid`, `ext/zip`, `ext/sqlite`,
  `ext/markdown`), is standards-conformant, minimal-but-complete, idiomatic
  Jinn, and carries a conformance test suite in the Jinn style.
- Versions independently of the compiler (its own semver), but is **built and
  tested in the Jinn CI** against every compiler release, so it never bit-rots.
- Is signed by the core release key and distributed both in-tree and via the
  registry. Adding it is `lamp add ext/zip` and resolution is instant and
  offline-capable (it's already in your toolchain distribution or one fetch
  away from the canonical mirror).

**Why `ext` exists.** It gives the community a *destination*. Instead of N
competing JSON/UUID/HTTP-client packages on an open registry (the npm failure
mode), there is a clear social and technical gradient: write a great package →
get it adopted into `ext` → it becomes *the* one. This concentrates effort,
reduces the attack surface (fewer, reviewed, signed artifacts), and answers
principle 3. lamp's search ranks `ext` above community results and labels them
**canonical**.

### 1.3 Community packages

Everything else. Published by anyone to any `lamp serve` registry, addressed by
name + content hash, signed by the author, and gated by the capability system
(§6). These are first-class — Jinn is not a walled garden — but they carry the
weakest implicit trust and the strongest explicit checks.

**Promotion path.** A community package that becomes the de-facto standard for
its facet is a candidate for `ext` adoption. The path is documented and
deliberate; lamp surfaces it (`lamp nominate <pkg>`), turning "popular" into
"curated and signed."

---

## 2. The manifest: `project.jn`

The manifest is **Jinn source**, parsed by the compiler's own lexer/parser
(consistent with the prototype in `Package::from_project_file`). No second
config language (no TOML/YAML/JSON manifest). This is the
intelligent-compiler convention: you already know Jinn syntax; you do not
learn a manifest DSL.

```jinn
package
  name is 'orchard'
  version is '1.4.0'
  authors is ['Ada <ada@example.org>']
  license is 'Apache-2.0'
  summary is 'A pruning planner for fruit trees.'
  edition is '2026'

requires
  json                       # canonical std — pinned to compiler
  ext/uuid    is '^2.1'      # curated ext, caret range
  pomelo      is '~0.7.3' from 'lamp://reg.jinn.dev/pomelo'
  internal    is path './crates/internal'
  forked-zip  is git 'https://git.example/zip' rev 'a1b2c3d'

provides
  bin orchard is 'source/main.jn'      # produces an executable
  lib orchard is 'source/lib.jn'       # produces a .jnlib

dev-requires
  ext/quickcheck is '^1.0'             # only for `lamp test`

build
  toolchain is '>=1.9.0'               # compiler version constraint
  targets is ['native', 'wasm32']
```

### 2.1 Manifest semantics

- **`requires`** lists *direct* dependencies. Bare name (`json`) means "the
  canonical package of this name," resolved std → libjn → ext → registry. A
  source qualifier (`from`, `path`, `git`) overrides the default registry.
- **Version constraints** use the SemVer grammar of §3. A bare canonical
  dependency with no version pins to the compiler/toolchain (std, libjn) or to
  the latest `ext` validated for this toolchain.
- **`provides`** declares build outputs: `bin` (native executable), `lib`
  (a `.jnlib`, §5), or both. This replaces the prototype's implicit
  `source/main.jn` assumption.
- **`dev-requires`** are present only for `lamp test`/`lamp bench`; they never
  enter a consumer's graph.
- **`build`** pins the toolchain range and target list, feeding the
  reproducibility guarantee (§7).
- The manifest is **declarative and side-effect free.** It is parsed, not
  executed — there is no `postinstall` script, ever (this is the single
  largest npm attack vector; we close it by construction, §6.4).

---

## 3. Versioning and resolution

### 3.1 SemVer (extends `src/pkg.rs`)

lamp uses semantic versioning `major.minor.patch[-pre][+build]`. The prototype
`SemVer` (major/minor/patch only) is extended with pre-release and build
metadata. Constraint grammar:

| Form        | Meaning                                  |
|-------------|------------------------------------------|
| `1.4.0`     | exact                                    |
| `^1.4.0`    | `>=1.4.0, <2.0.0` (compatible)           |
| `~1.4.0`    | `>=1.4.0, <1.5.0` (patch-level)          |
| `>=1.2, <2` | explicit range                           |
| `*`         | any (discouraged; `lamp lint` warns)     |

Pre-release versions are excluded from ranges unless explicitly opted into
(`^1.4.0-rc`), matching the principle of least surprise.

### 3.2 The resolver

lamp resolves to a **single version per package** in the default graph
(unification), preferring the *minimal version satisfying all constraints*
(MVS, the Go approach) over *maximal* (the Cargo/npm approach). Rationale:

- **MVS is reproducible without a lockfile** and minimizes surprise upgrades.
- It makes the lockfile a *cache and integrity record*, not a *decision
  record*, which simplifies the trust story.
- It naturally pushes toward small, slow-moving graphs (principle 6).

When constraints are genuinely incompatible, lamp **fails loudly** with the
conflict path (`A requires X ^1, B requires X ^2`) rather than silently
duplicating. Duplication of incompatible majors is allowed only behind an
explicit `requires-isolated` opt-in, and each isolated copy is reported in the
SBOM. We refuse the npm default of a sprawling deduplicated/un-deduplicated
`node_modules` lottery.

### 3.3 The lockfile: `jinn.lock`

Machine-generated, committed to VCS, never hand-edited (extends `src/lock.rs`).
Every entry is **content-addressed**:

```
# jinn.lock — auto-generated, do not edit
schema 1
toolchain 1.9.0

ext/uuid 2.1.4
  source  lamp://reg.jinn.dev/ext/uuid
  content blake3:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
  sig     ed25519:core@jinn.dev:3a7bd3e2360...
  caps    []
  deps    [json]

pomelo 0.7.4
  source  lamp://reg.jinn.dev/pomelo
  content blake3:2c26b46b68ffc68ff99b453c1d304134...
  sig     ed25519:author@example.org:8f1c...
  caps    [net.client]
  deps    [ext/uuid, json]
```

Key fields beyond the prototype's `name url version commit`:

- **`content`** — BLAKE3 hash of the canonical package archive (§5.4). This is
  the integrity anchor; `lamp build` verifies it before use.
- **`sig`** — detached signature over the content hash + manifest (§6.2).
- **`caps`** — the resolved capability set (§6.1), surfaced in the lock so a
  capability widening is a *visible diff* in code review.
- **`deps`** — transitive edges, enabling offline graph reconstruction.

A lockfile change that widens `caps` or changes `sig` (key rotation/new
signer) is flagged by `lamp build` and shown specially in `git diff` via a
provided diff driver. The lockfile is the supply-chain review surface.

---

## 4. The cache and store (content-addressed)

lamp maintains a per-machine **content-addressed store** (extends
`src/cache.rs`), keyed by BLAKE3 content hash, à la Nix:

```
~/.lamp/store/
  blake3-9f86.../           # immutable, hash-named, read-only
    project.jn
    source/...
    jnlib/...               # compiled artifacts (§5)
  index/                    # cached registry index snapshots
  keys/                     # trusted signer keys (TOFU + core roots)
```

Properties:

- **Immutable & deduplicated.** Two projects depending on the same hash share
  one store entry. No per-project `node_modules` copy explosion.
- **Verifiable.** Store path *is* the hash. Tampering changes the path.
- **Offline-first.** A populated store + lockfile builds with no network.
- **Vendorable.** `lamp vendor` materializes the closure into `./vendor/` for
  air-gapped / fully-pinned builds; the manifest can require `vendored` mode so
  CI never reaches the network.

A project's working tree never contains dependency source by default; it
contains only `project.jn` and `jinn.lock`. Builds read from the store.

---

## 5. Binary library format: `.jnlib`

> *"Is it possible to have a binary blob that can be integrated into a Jinn
> project?"* — Yes. This section specifies it.

A `.jnlib` is Jinn's **distributable compiled library**: a single
content-addressed archive carrying everything needed to type-check against,
link, and optionally debug a library **without its source**.

### 5.1 Why a binary format

- **Closed-source distribution** for vendors who cannot ship `.jn` source.
- **Faster builds**: skip re-compiling stable dependencies from source.
- **Reproducibility + provenance**: a signed `.jnlib` pins exactly what was
  built, by which toolchain, from which source hash.
- It must remain *integratable*: a downstream project links it as if it were a
  source dependency — same `use`, same inference, same Perceus ownership rules.

### 5.2 Structure

A `.jnlib` is a sealed archive (the same canonical archive format as §5.4)
containing:

```
manifest.jn          # package identity, version, caps, deps
interface.jhi        # "Jinn header interface" — see §5.3
mir/<target>/        # serialized typed MIR per target (the linkable code)
object/<target>/     # native object/static-lib per target (optional, fast-link)
debug/<target>/      # DWARF + source map + MIR↔source spans (debug variant)
provenance.json      # toolchain version, source content hash, build env hash
sig                  # signature over the whole sealed archive
```

Three publish flavors (matching the requested `source`, `binary`,
`binary-debug`):

- **source** — `.jn` files only; downstream compiles them. Maximum
  transparency, slowest, most auditable.
- **binary** — `interface.jhi` + `mir/` + `object/`, no source, no debug.
  Closed, fast.
- **binary-debug** — binary **plus** `debug/`, so a downstream developer can
  step into the library and get meaningful stack traces / source lines without
  the upstream source tree.

### 5.3 The interface file: `interface.jhi`

The crux of "integrate a binary blob." `interface.jhi` is the **public,
machine-checked surface** of the library: every exported type, function
signature, trait/protocol, store schema, effect/capability annotation, and —
critically — the **ownership and effect metadata** the Jinn type system and
Perceus pass require.

It is *not* C-style textual headers. It is a serialized slice of the typed HIR
that lets the downstream compiler type-check `use`s of the library **exactly as
if it had the source**, including:

- Generic signatures and HM-inferred constraints.
- Value-semantics / borrow / `as` annotations (so access-semantics rules,
  `tests/access_semantics.rs`, hold across the boundary).
- Effect rows and the declared capability set (§6) — a binary dependency
  cannot *hide* that it does network I/O; the interface carries the effect.
- Perceus refcount obligations on returned/consumed values, so the caller's
  generated drops/dups are correct against pre-compiled MIR.

Because the MIR is serialized and versioned, cross-`.jnlib` inlining and
specialization remain possible (the compiler can choose to inline from `mir/`
rather than treat the object as opaque), preserving Jinn's performance model.

### 5.4 Canonical archive & content addressing

Every distributable artifact (source pkg, `.jnlib`) is a **canonical
archive**: a deterministic, sorted, normalized tar (fixed mtimes, fixed perms,
sorted entries, no platform noise) so the same inputs always hash identically.
The BLAKE3 of the canonical archive is the package's content address (§3.3,
§4). Reproducibility (§7) depends on this determinism.

### 5.5 ABI / interface stability

`.jnlib` carries the **toolchain version** it was built with and an
**interface schema version**. The downstream compiler:

- Accepts a `.jnlib` whose toolchain is within the project's `build.toolchain`
  range and whose interface schema it understands.
- Otherwise **falls back to building from source** (if the source flavor is
  available) or errors with a precise "rebuild needed" message. We never link
  a stale/mismatched binary silently.

There is no promise of a frozen cross-compiler binary ABI — that is a trap
(C++ pain). Instead: binaries are a *cache keyed by toolchain*, and source
remains the source of truth. This keeps the language free to evolve its MIR.

---

## 6. Supply-chain security (the core of lamp)

This is where lamp earns its name. The threat model is the modern one:
malicious or compromised transitive dependencies, typosquatting, install-time
code execution, dependency confusion, and unsigned mutable registries.

### 6.1 Capabilities / effects manifest

Every package declares the **ambient capabilities** it requires, expressed
through the same effect system Jinn already uses (`docs/error-effects.md`):

```jinn
package
  name is 'pomelo'
  ...
  capabilities
    net.client          # may open outbound sockets
    fs.read './assets'  # may read a scoped path
```

Capability classes: `fs.read`/`fs.write` (path-scoped), `net.client`/
`net.server`, `process.spawn`, `env.read`, `clock`, `random`, `ffi.unsafe`,
`time`. The compiler *derives* the actual effect set from the code and the
resolver **rejects any package whose code uses a capability it did not
declare**. A pure data-structure library declares nothing and is provably
incapable of touching the network — verified, not promised.

The consuming project sees the **union** of its graph's capabilities in
`jinn.lock` (`caps`) and in `lamp tree --caps`. A dependency update that adds
`process.spawn` is a one-line, reviewable lock diff — the single most
important supply-chain signal, made impossible to miss.

`lamp build --deny net.server` (etc.) lets a consumer hard-refuse capabilities
regardless of declaration, enforced at compile time (the effect cannot be
discharged), giving defense in depth.

### 6.2 Signing & provenance

- Every published artifact is **signed** (Ed25519) over its canonical content
  hash + manifest. `ext` and `std`/`libjn` are signed by **core release
  keys** shipped with the toolchain (a root of trust you already installed).
- Community packages are signed by author keys under **TOFU** (trust-on-first-
  use, like SSH): the first version pins the key; a later version signed by a
  *different* key requires explicit `lamp trust` acknowledgement. This defeats
  account-takeover republishing.
- `provenance.json` records the building toolchain, source content hash, and a
  hash of the (declared, minimal) build environment, enabling **independent
  rebuild verification** (a third party rebuilds and confirms the same content
  hash — SLSA-style).
- Optional **transparency log**: registries SHOULD append every published
  (name, version, content-hash, signer) tuple to an append-only Merkle log
  (Sigstore/`rekor`-style) that clients can audit and gossip, so a registry
  cannot serve different bytes to different clients undetected.

### 6.3 SBOM

`lamp build` and `lamp publish` emit a **Software Bill of Materials**
(`jinn.sbom`, SPDX/CycloneDX-compatible export available) enumerating every
node in the closure with name, version, content hash, signer, license, and
capability set. This is a first-class output, not an afterthought.

### 6.4 No install-time code execution — by construction

There are **no install/postinstall/prepublish lifecycle scripts**. The
manifest is parsed, never executed. Builds run only the declared compiler
toolchain on declared source. Build-time codegen (if ever needed) is a
*declared, sandboxed, capability-gated* build step, not arbitrary shell. This
single decision eliminates the dominant npm/PyPI attack vector outright.

### 6.5 Naming, typosquatting, dependency confusion

- Canonical names (`std`, `libjn`, `ext/*`) are **reserved** and resolve
  in-toolchain first; a registry can never shadow `json` or `ext/uuid`. This
  closes dependency-confusion (the "internal name pulled from public registry"
  attack) by construction for the curated tiers.
- The registry enforces a **namespace** model (`@scope/name`) so authors own a
  scope; unscoped names are reserved for `ext`-promoted packages.
- `lamp add` warns on Levenshtein-near matches to popular/canonical names
  (typosquat heuristic) and on brand-new, low-download, single-author packages
  pulled transitively.

### 6.6 Auditing

`lamp audit` cross-references the closure against a signed **advisory
database** (itself a versioned, signed, mirror-able package). It reports
affected paths and the *minimal* version bump that clears each advisory, and
can `lamp audit --fix` to compute it. Advisories are content-addressed and
gossiped like packages, so the security feed is as decentralized as the
registry.

---

## 7. Reproducible builds

Given `jinn.lock` + the pinned toolchain, `lamp build` is deterministic:

- Canonical archives (§5.4) make inputs hash-stable.
- The compiler is run in a **normalized environment** (sorted env, fixed
  locale, fixed temp paths, no embedded absolute paths or timestamps in
  output; build paths remapped).
- Output artifacts are themselves content-addressed; two clean builds of the
  same lock on different machines produce byte-identical `.jnlib`/binaries.
- `lamp build --verify` rebuilds and asserts the content hash matches the lock
  / a published provenance record — the consumer-side half of §6.2.

`vendored` mode (§4) plus a pinned toolchain gives fully hermetic, offline,
reproducible CI.

---

## 8. The registry: a protocol, not a company

lamp's registry is a **federated, content-addressed protocol** so the
ecosystem cannot be captured or centrally killed.

### 8.1 Model

- A registry is anything that speaks the **lamp registry protocol** over
  `lamp://` (HTTPS transport). `lamp serve` turns any directory or object
  store into one.
- Packages are **content-addressed**; the registry maps
  `name@version → content-hash + signed metadata`. The *bytes* can come from
  any mirror that has the hash — the registry is an index + trust layer, not a
  sole bytes-host. This is what makes it decentralized: hashes are portable.
- The **canonical index** (`reg.jinn.dev`) is itself a signed, versioned,
  Git-mirrorable data structure (a Merkle index). Anyone can clone and mirror
  it; clients can fall back to mirrors transparently. No single point of
  failure.
- **Federation**: a project's `project.jn` / global config lists trusted
  registries in order; resolution walks them. An organization runs a private
  registry that *overlays* the public one for internal packages — with
  dependency-confusion protection from §6.5.

### 8.2 Decentralized / P2P aspirations (forward-looking)

Because everything is content-addressed and signed, the bytes layer can be
backed by content-addressed P2P transport (IPFS-style CIDs / BitTorrent-style
swarms) with the signed Merkle index providing discovery and integrity. The
canonical host becomes a *seeder and indexer*, not a *gatekeeper*. This is a
later milestone, but the data model is designed for it now: nothing in the
protocol assumes a single trusted server holds the bytes.

### 8.3 The prototype migration

The current implementation (git-tag publish, `git clone` fetch in
`cmd_publish`/`Cache::resolve`) is a **valid degenerate registry**: a git remote
*is* a content-addressed mirror (commit hashes), and `git`-source dependencies
remain first-class (`from git ... rev ...`). lamp keeps git as a transport while
layering the index, signing, capability, and content-hash semantics on top. No
flag day.

---

## 9. The maintainer surface: bugs, changelogs, releases

A package is more than code. lamp brings issue tracking and release notes into
the **same content-addressed, signed, federated fabric** — so they survive,
mirror, and verify like the code does, rather than living in a proprietary
forge.

### 9.1 Bug tracking

- Issues live in-repo under a structured `issues/` directory (append-only,
  content-addressed records: id, reporter, signed body, status, labels). They
  are *part of the package*, travel with mirrors, and work offline.
- `lamp bug "title"` files one (signed by reporter key); `lamp bugs` lists;
  `lamp bug show <id>`; maintainers `lamp bug close <id> --fixed-in 1.4.1`,
  which **links the fix to a release** and propagates into the changelog and
  the advisory DB if security-relevant.
- Because issues are signed records, a federated tracker can aggregate them
  across mirrors without a central server, and a maintainer's "close" is
  verifiable.

### 9.2 Changelogs & patch notes

- A package carries `CHANGELOG` as structured, per-version entries
  (`added`/`changed`/`fixed`/`security`/`removed`, Keep-a-Changelog shape).
- `lamp changelog add fixed "..."` appends to the *unreleased* section;
  `lamp release 1.4.1` (§9.3) seals the unreleased section into a versioned
  entry, stamps the date, and links closed bugs.
- `lamp notes 1.4.1` renders human patch notes; `lamp notes --since 1.2.0`
  aggregates across versions. Security entries auto-feed the advisory DB.

### 9.3 The release flow (replaces `cmd_publish`)

`lamp release <version>` is the maintainer's one command. It:

1. Verifies a clean tree and that `version` matches (and is `>` the last).
2. Seals the changelog's unreleased section; links bugs closed since the last
   release.
3. Builds the requested flavors (`--source`, `--binary`, `--binary-debug`,
   default `source`), each as a canonical archive (§5.4).
4. Computes content hashes, signs them and the manifest, emits `provenance`
   and the SBOM.
5. Publishes to the configured registry (and pushes the git tag, preserving the
   prototype's behavior) and appends to the transparency log.
6. Prints the consumer `requires` line, exactly like the prototype, but with the
   content hash and signer shown.

`lamp yank <version>` marks a version unusable for *new* resolutions without
deleting bytes (existing locks still build — immutability of published bytes is
sacred; we never rewrite history, only advise against it).

---

## 10. Scaffolding & developer ergonomics

One coherent verb set. No `init` vs `new` vs `create` confusion.

| Command | Does |
|---------|------|
| `lamp new <name>` | scaffold a new project (`--bin`/`--lib`/`--workspace`) |
| `lamp add <pkg>` | add a dep, resolve, update lock, show added caps |
| `lamp remove <pkg>` | drop a dep, prune lock |
| `lamp build` | resolve (if needed) + compile via `jinnc`; verifies hashes/sigs |
| `lamp run` / `lamp test` / `lamp bench` | build then run/test/bench |
| `lamp lock` / `lamp update [pkg]` | regenerate / bump lock (MVS) |
| `lamp vendor` | materialize closure into `./vendor` |
| `lamp tree [--caps] [--why <pkg>]` | dependency graph, capabilities, paths |
| `lamp audit [--fix]` | advisory scan + minimal fix |
| `lamp lint` | manifest hygiene (`*` ranges, unused deps, license) |
| `lamp search <q>` | search; ranks std/libjn/ext (canonical) first |
| `lamp doc` | build & serve docs from `.jhi` + source |
| `lamp release` / `lamp yank` | §9.3 |
| `lamp serve` | run a registry (§8) |
| `lamp bug` / `lamp bugs` / `lamp changelog` / `lamp notes` | §9 |
| `lamp nominate <pkg>` | start the community→`ext` promotion (§1.2) |

Scaffolds produce a *minimal* `project.jn`, a `source/` tree, a `tests/` tree
wired to `lamp test`, a starter `CHANGELOG`, and a `.gitignore` (store/vendor
artifacts ignored, `jinn.lock` committed).

---

## 11. What we learned from prior systems

A deliberate audit, so the choices above are grounded, not aesthetic.

- **Cargo (Rust)** — gold standard for UX: one tool, one manifest, one
  lockfile, integrated test/bench/doc. *We adopt all of this.* Weakness:
  maximal-version resolution causes churn; `build.rs` is an arbitrary-code-
  execution vector. *We choose MVS and forbid build scripts.*
- **Go modules** — MVS gives reproducibility without a lockfile and small
  graphs; `GOPROXY` + `sumdb` is an excellent content-integrity + transparency
  model; no central registry (URL-addressed). *We adopt MVS, checksum DB
  (transparency log), and decentralization.* Weakness: ergonomics around major
  versions (`/v2` in path) are clumsy; weaker capability story. *We keep
  semver ranges and add capabilities.*
- **npm/yarn/pnpm/bun (JS)** — the cautionary tale: tool fragmentation, deep
  unsigned mutable graphs, `postinstall` scripts, typosquatting, dependency
  confusion, left-pad fragility. *We reject tool sprawl, install scripts,
  mutable publishes, and unsigned artifacts; we add curation (`ext`) to fight
  redundancy.*
- **Nix / Guix** — content-addressed immutable store, hermetic reproducible
  builds, perfect dedup. *We adopt the store and reproducibility model.*
  Weakness: steep UX. *We hide it behind boring `lamp` commands.*
- **Deno** — capability-based permissions (`--allow-net`), URL imports, no
  central registry, code signing direction. *We adopt capabilities and
  decentralization*, but choose a *declared* capability manifest (verified by
  the compiler's effect system) over purely run-time flags, so refusal happens
  at *compile* time.
- **Maven/Gradle (JVM)** — coordinates + checksums + signing (GPG) + binary
  artifacts (`.jar`) are mature. *We take binary artifacts (`.jnlib`),
  checksums, and signing.* Weakness: XML/Groovy config sprawl, slow resolution,
  diamond/version-conflict pain. *We use Jinn-native manifests and MVS.*
- **Sigstore / SLSA / in-toto (research/industry)** — keyless-ish signing,
  transparency logs, provenance attestation, reproducible-build verification.
  *We adopt transparency logs, provenance, and rebuild verification.*
- **Hackage/Stack (Haskell), Elm, OPAM (OCaml), Pub (Dart)** — curated/
  reviewed registries and "one canonical package" cultures (Elm especially)
  show that *curation reduces redundancy and improves quality.* *This is the
  intellectual basis for `ext`.* Elm's enforced semver (the compiler computes
  the required version bump from the API diff) is a model we adopt for
  `.jhi` interfaces: **lamp can compute the correct semver bump by diffing the
  published interface**, making accidental breaking changes hard.

---

## 12. Implementation roadmap (from the prototype)

The prototype already ships the skeleton; lamp is its principled completion.

| Phase | Work | Touches |
|-------|------|---------|
| 1 | Split `lamp` driver from `jinnc`; unify the verb set (§10) | `src/driver/`, new `src/bin/lamp.rs` |
| 2 | Manifest v2 (`provides`, `capabilities`, sources, dev-deps) | `src/pkg.rs` |
| 3 | Content-addressed store + canonical archive + BLAKE3 | `src/cache.rs`, new `src/store.rs` |
| 4 | Lockfile v2 (content/sig/caps/deps); diff driver | `src/lock.rs` |
| 5 | MVS resolver + conflict reporting | `src/cache.rs` resolve |
| 6 | Capability derivation from effect system; resolver enforcement | typer/effects + `src/pkg.rs` |
| 7 | Signing, TOFU keystore, provenance, SBOM | new `src/trust.rs` |
| 8 | `.jnlib` writer/reader: `interface.jhi` (HIR slice), MIR/object pack | new `src/jnlib/`, codegen, typer |
| 9 | Registry protocol + `lamp serve`; git transport bridge | new `src/registry/` |
| 10 | Reproducible-build normalization + `--verify` | driver/codegen |
| 11 | Maintainer surface: bugs, changelog, notes, release/yank | new `src/maint/` |
| 12 | `ext/` tier wiring, curation CI, `lamp nominate`, transparency log | repo CI, `ext/` |

Each phase ships with conformance tests in the Jinn style: a two-package
workspace builds reproducibly (the original task-2-19 definition of done),
plus a tampered-hash test, a capability-widening test, a signature-mismatch
test, and a `.jnlib` round-trip (build a lib, consume it as binary and as
binary-debug, confirm identical type-checking to source).

---

## 13. Summary

lamp is **one tool**, **one manifest**, **one lockfile**, and **one
content-addressed, signed, federated store**. It makes the safe, reproducible,
auditable path the *default* and the only ergonomic one. It introduces `ext` to
concentrate community effort into single canonical packages, a `.jnlib` binary
format that integrates as cleanly as source while supporting closed and
debuggable distribution, a capability system that makes a dependency's powers a
reviewable lockfile diff, and a maintainer surface (bugs, changelogs, releases)
that lives in the same durable fabric as the code. It learns from Cargo's UX,
Go's MVS and transparency, Nix's store, Deno's capabilities, Elm/Hackage
curation, and Sigstore/SLSA provenance — while refusing JS-land's tool sprawl
and install-time code execution. It is the package manager Jinn's ethos
demands: the compiler does the heavy lifting; the programmer expresses intent;
trust is explicit, verifiable, and boring.
