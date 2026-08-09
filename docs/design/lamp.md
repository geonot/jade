# lamp — the package system

> **Status: design. Not implemented.** The shipped `jinnc` package commands
> (`fetch`, `update`, `build`, `package`, `publish`) are a git-tag prototype in
> `src/pkg.rs`, `src/lock.rs`, and `src/driver/cmd_pkg.rs`. This document
> specifies the target and supersedes that prototype. The compiler machinery it
> assumes — capabilities, path-scoped identity, interface v2, config blocks — is
> specified in [`compiler-prereqs.md`](compiler-prereqs.md), which should be
> read alongside it.

*A lamp is a small, dependable light you carry into unfamiliar code.* lamp is
Jinn's package manager, registry protocol, and supply-chain trust layer. It
exists to make depending on other people's code **boring, auditable, and
reproducible**.

Names fixed by this design and not negotiable downstream: manifest `project.jn`,
lockfile `project.lock`, binary library `.jnb`, deployment record
`project.deploy.lock`.

## Principles

1. **One tool.** There is `lamp`. Every verb — scaffold, build, add, lock,
   vendor, publish, sign, serve, file a bug, cut a changelog, deploy — is a
   `lamp` subcommand. `jinnc` stays a compiler; `lamp` drives it.
2. **One manifest, one lockfile, per project tree.** A project may produce many
   packages, but there is exactly one `project.jn` and one `project.lock` at its
   root, and nothing else is authoritative.
3. **Curated by default, open at the edges.** For any one facet there should be
   *one quintessential package*. Discovering a fourth redundant JSON library is
   a deliberate act, not the default.
4. **Trust is explicit and verifiable.** Every artifact is content-addressed and
   signed; every dependency declares the capabilities it needs; the resolver
   refuses to silently widen a package's capability set.
5. **Reproducible or it didn't happen.** Given a lockfile, a build on any
   machine at any time produces a bit-identical artifact.
6. **Small graphs.** A 1,400-package transitive closure is a bug report.
7. **Decentralized and federated.** The registry is a protocol, not a company.
8. **No ceremony in the common case.** `lamp add json` just works.

## Tiers

| Tier | Location | Maintained by | Trust |
| --- | --- | --- | --- |
| `std` | `std/*.jn` | core team, in-tree, versioned with the compiler | implicit, always present |
| `ext` | `ext/<name>/` | community, **vendored into the Jinn repo** under project review | curated, audited, signed by core release keys |
| community | external registry | anyone | signed by author, capability-gated |

`ext` is the destination that concentrates community effort: write a great
package, get it adopted, and it becomes *the* one. It versions independently but
is built and tested in Jinn CI against every compiler release, so it never
bit-rots. `lamp search` ranks `ext` above community results and labels them
canonical; `lamp nominate <pkg>` starts the community → `ext` promotion.

`libjn` is **not** a tier. It is the freestanding runtime and libc replacement
the compiler and `std` are built on top of — part of the toolchain, never
resolved through the dependency graph. Its layering (freestanding primitives →
allocator → syscall shims → scheduler) is what makes the freestanding output
kinds and substrate dials below possible.

## The manifest: `project.jn`

The manifest is Jinn source, parsed by the compiler's own lexer and parser using
the config-block construct in [`compiler-prereqs.md`](compiler-prereqs.md#config-blocks).
No second config language.

```jinn
project
  name is 'orchard'
  version is '1.4.0'
  authors is ['Ada <ada@example.org>']
  license is 'Apache-2.0'
  edition is '2026'

requires
  json                       # canonical std
  ext/uuid    is '^2.1'
  pomelo      is '~0.7.3' from 'lamp://reg.jinn.dev/pomelo'
  forked-zip  is git 'https://git.example/zip' rev 'a1b2c3d'

provides
  lib orchard-core is 'src/lib.jn'
  bin orchard-prune is 'src/prune.jn'
    requires orchard-core
  app orchard      is 'src/main.jn'
    requires orchard-core

members
  ./planner

dev-requires
  ext/quickcheck is '^1.0'

build
  toolchain is '>=1.9.0'
  targets   is ['native', 'wasm32']
  profiles
    release is opt 3, debug-info false
    dev     is opt 0, debug-info true

deployments
  dev   is local                            data './.run/dev'
  prod  is host 'deploy@orchard.example'    replicas 3
```

### The restricted sublanguage

The manifest is declarative and side-effect free, and that is enforced
structurally rather than by convention. Only the listed top-level blocks are
recognized; only `is`-bindings to literals and the closed connector set (`from`,
`path`, `git`, `rev`, `requires`, `with`, `enables`, `opt`, `visibility`, and the
deployment keywords) are allowed. No function calls, no control flow, no `*name`
definitions. The parser rejects anything else with a precise diagnostic.

**There is no `postinstall`, ever.** The dominant npm and PyPI attack vector is
closed by construction: the manifest is parsed, never executed, and builds run
only the declared toolchain on declared source.

### `provides` — many outputs from one project

An output is described on two orthogonal axes. The **kind** decides what is
produced; the **substrate** decides which runtime layers get linked.

| Kind | Has `main` | Substrate | Persistent stores | Packaged as | `lamp deploy` |
| --- | :-: | --- | :-: | --- | :-: |
| `lib` | no | hosted | may define store types, instantiates none | `.jnb`, content-addressed | no |
| `bin` | yes | hosted | none | single static native binary | no |
| `app` | yes | hosted | one or more | deploy bundle (binary + contract) | **yes** |
| `os` | boot entry | freestanding | n/a | bootable image (arch + loader) | no |
| `raw` | reset vector | freestanding | n/a | flat binary / ELF for flashing | no |

The **substrate dials** are opt-outs from the full hosted runtime, each
*inferred from the used surface* and optionally pinned:

- `runtimeless` — no runtime bring-up; the artifact provides its own entry.
  Implied by `os` and `raw`.
- `threadless` — no scheduler, actors, or channels. Inferred when the reachable
  closure spawns nothing and opens no channels.
- `heapless` — no dynamic allocation; value semantics over static and arena
  memory. The compiler proves this from ownership and escape analysis and
  *errors* rather than silently linking malloc if a heap path is reachable.

```jinn
provides
  bin probe   is 'src/probe.jn'  threadless
  os  kernel  is 'src/boot.jn'   for arch 'riscv64'
  raw blinky  is 'src/blinky.jn' for mcu 'rp2040' heapless
```

**Kinds are inferred, not declared.** No `main` ⇒ `lib`. `main`, hosted, no
persistent store ⇒ `bin`. `main`, hosted, one or more persistent stores ⇒ `app`.
A freestanding entry ⇒ `os`, or `raw` when a device target is named and
`heapless` holds. Writing the kind explicitly pins intent: a mismatch between
the declared kind and the inferred surface is a precise error. Pinning a dial
the inference already holds is a `--pedantic` lint; pinning one the code
violates is a located error ("`heapless` pinned but `Vec.push` at `boot.jn:42`
allocates").

Rules: output names share one namespace within the project and must be unique;
inter-output dependencies are explicit and acyclic (`requires <sibling>`, cycles
are a hard error naming the path); output-scoped external deps narrow the graph
so each binary's closure and capability union stay minimal; each output can be
built, run, and released selectively; an output may set its own `version`,
otherwise it inherits the project's.

### The `app` deploy contract

Building an `app` derives — from the same effect and store analysis that powers
capabilities — a contract embedded in the bundle: **store schemas** (a
fingerprint per persistent store, which is what makes migrations checkable
rather than hopeful), a **settings contract** (typed configuration keys with
types and defaults), the **capability ceiling**, and **environment requirements**
(an address to bind, a data directory, secrets *by name* — never by value).

Because the contract is derived it cannot drift from the code: an app that adds
a store, a setting, or a capability changes its contract, and a deploy to an
existing target surfaces that change as a reviewable diff.

### `members` — hierarchical projects

Each entry under `members` is a path to a child project with its own
`project.jn`. The parent includes its children's outputs into one resolved graph
and one lockfile at the parent root. Two composition modes, made explicit:

- **standalone referencing** — a `lib` is published and consumed by content
  address like any external dependency. Use across repo boundaries.
- **hierarchical referencing** — the parent flattens the member namespace into
  one DAG; intra-tree dependencies resolve by name and never hit the registry.
  Use within one repo.

A member is promoted from hierarchical to standalone simply by publishing it;
consumers switch from `requires planner` to `requires planner from <registry>`
with no source change. All members resolve the same external versions, which
eliminates the diamond hazard within a workspace.

### `features`

Optional, **additive-only** features gate code and dependencies without forking
packages. Because enabling one never removes API, the resolver can take the
union across the graph and still satisfy MVS. Subtractive features are forbidden
outright, which avoids npm's optional-dependency ambiguity and Cargo's feature
unification surprises.

## Versioning and resolution

SemVer `major.minor.patch[-pre][+build]`, with `^`, `~`, explicit ranges, and a
discouraged `*` that `lamp lint` warns on. Pre-releases are excluded from ranges
unless explicitly opted into. lamp resolves to a **single version per package**
across the entire project tree.

### MVS plus the lockfile

lamp uses **Minimal Version Selection**: pick the minimal version satisfying all
constraints. Reproducible selection logic, no surprise upgrades, and a natural
push toward small graphs. The division of labour is exact:

- **MVS decides which version.** A pure function of the manifests in the graph,
  deterministic and lockfile-independent.
- **The lockfile decides which bytes and whether to trust them.** A version
  number is not an integrity statement. MVS without a checksum database is
  insecure; the lock pins content hash and signature for the selected version.

Genuinely incompatible constraints **fail loudly** with the conflict path, never
silently duplicate. Deliberate coexistence of incompatible majors requires an
explicit `requires-isolated`, and each isolated copy is reported in the SBOM.

### `project.lock`

Machine-generated, committed, never hand-edited. Every entry is
content-addressed:

```
schema 2
toolchain 1.9.0

pomelo 0.7.4
  source  lamp://reg.jinn.dev/pomelo
  content blake3:2c26b46b68ffc68ff99b453c1d304134...
  sig     ed25519:author@example.org:8f1c...
  caps    [net.client]
  feats   [tls]
  deps    [ext/uuid, json]
```

`content` is the integrity anchor, verified before use. `caps` surfaces the
resolved capability set so a widening is a **visible diff in code review** —
this is the point of the whole design. A lock change that widens `caps`, changes
a signature, or adds an isolated duplicate is flagged by `lamp build` and
rendered specially by a provided git diff driver. Intra-tree member libraries
are recorded with `source path` and a content hash of the built `.jnb`, so even
local outputs participate in integrity checking.

## The content-addressed store

A per-machine store keyed by BLAKE3, in the Nix style:

```
~/.lamp/store/
  blake3-9f86.../           # immutable, hash-named, read-only
  index/                    # cached registry index snapshots
  keys/                     # trusted signer keys (TUF roots + log-anchored)
```

Immutable and deduplicated (two projects on the same hash share one entry),
verifiable (the store path *is* the hash), offline-first (a populated store plus
a lockfile builds with no network), vendorable (`lamp vendor` materializes the
closure into `./vendor/` for air-gapped CI), and garbage-collected (`lamp gc`
removes entries reachable from no committed lock — immutability makes this
safe). A working tree contains only `project.jn`, `project.lock`, and sources.

## `.jnb` — the binary library format

A `.jnb` is a sealed canonical archive carrying everything needed to type-check
against, link, and optionally debug a library **without its source**:

```
project.jn           # identity, version, caps, feature map
deps.descriptor      # dependency triples (range, iface-hash, built) — NO dep bytes
interface.jhi        # the public machine-checked surface
mir/<target>/        # serialized typed MIR — the linkable code
object/<target>/     # native object (optional, fast-link)
debug/<target>/      # DWARF + source map + MIR↔source spans
provenance.json      # toolchain version, source hash, build-env hash
sig                  # signature over the sealed archive
```

Three publish flavours: **source** (maximum transparency, slowest), **binary**
(interface + MIR + object), **binary-debug** (binary plus DWARF, so a downstream
developer can step into the library without upstream source).

A `.jnb` carries only its own code. The bytes of its dependencies live once in
the content-addressed store and are referenced, never embedded — this is the
deliberate rejection of the shaded-jar / vendor-into-binary anti-pattern, where
each artifact carries private copies, dedup dies, and CVEs become unpatchable.

`interface.jhi` is the crux of "integrate a binary blob": not a C-style textual
header but a serialized slice of typed HIR that lets the downstream compiler
type-check `use`s exactly as if it had the source — generic signatures,
ownership annotations, effect and capability rows, and Perceus refcount
obligations. See [`compiler-prereqs.md`](compiler-prereqs.md#interface-v2-and-the-three-hash-ladder).

Every distributable artifact is a **canonical archive**: deterministic, sorted,
normalized tar with fixed mtimes and permissions, so the same inputs always hash
identically.

### Interface stability and the FFI gap

A `.jnb` records its toolchain version and interface schema version. The
downstream compiler accepts it when both are in range, otherwise it falls back
to building from source or errors with a precise "rebuild needed" message. A
stale binary is never linked silently. There is no frozen cross-compiler binary
ABI: binaries are a *cache keyed by toolchain*; source is the source of truth.

Capability derivation works for pure Jinn, but `ffi.unsafe` calls and
pre-compiled objects can do anything the OS allows. That gap is closed honestly
rather than papered over: a `.jnb` containing `object/` or any unsafe FFI symbol
**must** declare `ffi.unsafe` and any concrete capabilities it grants through
it; the declared set is then taken as ground truth and unioned into every
consumer; the taint **cannot be laundered away** by a pure-Jinn wrapper; and
`lamp build --deny ffi.unsafe` refuses such a graph entirely.

### A binary's own transitive dependencies

The question on which any binary-library design lives or dies. Both prior
answers are bad: static bundling gives N private copies and unpatchable CVEs;
unconstrained late binding links whatever the consumer happens to pick. lamp
takes neither.

1. **A `.jnb`'s dependency edges are open, not frozen.** A binary publishes its
   dependencies as *version ranges with interface hashes*, not hard-pinned
   bytes, so they fold into the one global MVS solution. Single-version
   unification holds across the binary boundary.
2. **What a binary cannot do is recompile itself.** Its MIR and objects were
   monomorphized against recorded interface hashes. The resolver may pick any
   version in the declared range **only if** that version's interface hash
   matches what the binary was compiled against, or the binary ships source for
   a rebuild.
3. **Genuine incompatibility fails loudly**, reporting both binaries, both
   ranges, and both interface hashes — never a silent duplicate.

## Supply-chain security

Threat model: malicious or compromised transitive dependencies, typosquatting,
install-time code execution, dependency confusion, unsigned mutable registries,
key compromise.

**Capabilities.** Every package declares the ambient capabilities it needs; the
compiler *derives* the actual set from the code and the resolver rejects any
package whose code uses a capability it did not declare. A pure data-structure
library declares nothing and is *provably* incapable of network access —
verified, not promised. The consuming project sees the union in `project.lock`
and `lamp tree --caps`, per output. `lamp build --deny net.server` lets a
consumer hard-refuse a capability regardless of declaration, enforced at compile
time because the effect cannot be discharged.

**Signing and key transparency.** Every artifact is Ed25519-signed over its
content hash and manifest. `std` and `ext` are signed by core release keys
shipped with the toolchain. Plain trust-on-first-use is not enough — it has a
first-contact MITM weakness and no recovery from compromise — so the registry
maintains a TUF-style signed root with delegations plus an append-only
**transparency log**. A key is trusted on first use only if its binding is
witnessed in a log the client can independently verify and gossip; a new key for
an existing package requires a signature from the prior key or an explicit
`lamp trust` after showing log evidence. Revocation is a signed root entry:
clients refuse revoked keys for *new* resolutions while existing locks still
build, because immutability of published bytes is sacred.

**Provenance and SBOM.** `provenance.json` records the building toolchain, the
source content hash, and a hash of the declared build environment, enabling
independent rebuild verification. `lamp build` and `lamp publish` emit a
per-output SBOM enumerating every closure node with name, version, content hash,
signer, license, and capability set.

**Naming.** Canonical names (`std`, `ext/*`) are reserved and resolve in-toolchain
first, so a registry can never shadow `json`. The registry enforces a scoped
namespace. Member and output names never resolve from a registry, closing
intra-workspace confusion. `lamp add` warns on near-matches to canonical names
and on brand-new, low-download, single-author packages pulled transitively.

**Auditing.** `lamp audit` cross-references the closure against a signed
advisory database — itself a versioned, signed, mirror-able package — reporting
affected paths and the *minimal* version bump clearing each advisory.

## Reproducible builds

Given the lockfile and a pinned toolchain, `lamp build` is deterministic:
canonical archives make inputs hash-stable; the compiler runs in a normalized
environment (sorted env, fixed locale, fixed temp paths, remapped build paths,
no embedded timestamps); outputs are content-addressed. `lamp build --verify`
rebuilds and asserts the hash matches the lock or a published provenance record.
Vendored mode plus a pinned toolchain gives hermetic offline CI.

## The registry

A federated, content-addressed **protocol**, so the ecosystem cannot be captured
or centrally killed. A registry is anything speaking the versioned lamp registry
protocol over HTTPS; `lamp serve` turns any directory or object store into one.
The registry maps `name@version → content-hash + signed metadata`; the *bytes*
can come from any mirror holding the hash, so the registry is an index and trust
layer, not a sole host. The canonical index is a signed, Git-mirrorable Merkle
index anyone can clone. Publish authorization is scope-based via TUF delegation,
with no bearer-token model that grants silent publish if leaked. Because
everything is content-addressed and signed, the bytes layer can later be backed
by P2P transport with the signed index providing discovery and integrity.

The current git-tag publish and `git clone` fetch is a **valid degenerate
registry** — a git remote is a content-addressed mirror — so git stays
first-class as a transport while index, signing, capability, and content-hash
semantics layer on top. No flag day.

## Deployments

`lamp deploy <app> <target>` runs an instance of an `app` output in a named
environment. Targets are declared in the manifest and cover both the classic
environment ladder and per-tenant fleets:

```jinn
deployments
  tenant for each name in tenants
    is host 'deploy@{name}.orchard.app'
    settings tenant-id is name
```

Each target's live state is recorded in `project.deploy.lock` — the deployed
revision hash, version, timestamp, live store-schema fingerprints, settings
hash, capabilities, and signer. Because the deployed bundle is the same
content-addressed artifact the build produced, the revision hash *is* the proof
of what is running.

A target profile has four facets, each inferred with defaults, overridable per
target, and checked *before* a process is touched: **environment** (declared
external resources; secrets by name, never by value), **settings** (typed keys
checked against the app's contract, so a misconfigured target fails at deploy
rather than at 3 a.m.), **persistence** (per-target, so environments and tenants
get isolated state by construction), and **platform** (`local`, `host <ssh>`, or
a pluggable driver resolved and capability-gated like any dependency). Only
platform has an external adapter; the other three are pure data.

Deploy is a **single transaction with a plan-then-apply gate**: build and verify
under the committed lock, compute the diff against the deployment record —
binary revision, store schema fingerprints (a change requires a checked
migration), settings, capabilities — present it for confirmation, apply, and
roll back atomically on failure. Re-deploying the same revision is an idempotent
no-op. In a fleet rollout one tenant's failure rolls that tenant back while the
rest proceed.

## The maintainer surface

Issues and changelogs live in the same content-addressed, signed, federated
fabric as the code, so they survive, mirror, and verify rather than living in a
proprietary forge. Issues are append-only signed records under `issues/`, part
of the package, working offline. Changelogs are structured per-version entries;
`lamp release` seals the unreleased section and links bugs closed since the last
release, and security entries auto-feed the advisory database.

`lamp release` verifies a clean tree and version ordering, seals the changelog,
builds the requested flavours as canonical archives, computes and signs content
hashes, emits provenance and the SBOM, publishes, and appends to the
transparency log. `lamp yank` marks a version unusable for *new* resolutions
without deleting bytes.

**Interface-driven semver.** Because `interface.jhi` is a precise machine
surface, `lamp release` computes the minimum legal version bump by diffing the
new interface against the last published one — added API means minor, changed or
removed means major, internal-only means patch — and refuses a declared version
that undershoots it, showing the offending API diff. Accidental breaking changes
become hard to ship.

## Command surface

| Command | Does |
| --- | --- |
| `lamp new <name>` | Scaffold a project (`--bin`/`--lib`/`--workspace`). |
| `lamp add` / `remove <pkg>` | Add or drop a dependency; update the lock; show capability changes. |
| `lamp build [<output>]` | Resolve and compile via `jinnc`; verify hashes and signatures. |
| `lamp run <output>` / `test` / `bench` | Build, then run. |
| `lamp lock` / `update [pkg]` | Regenerate or bump the lock. |
| `lamp vendor` | Materialize the closure into `./vendor`. |
| `lamp tree [--caps] [--why <pkg>]` | Per-output graph, capabilities, paths. |
| `lamp audit [--fix]` | Advisory scan and minimal fix. |
| `lamp lint` | Manifest hygiene: `*` ranges, unused deps, license, member cycles. |
| `lamp search <q>` | Search, ranking `std` and `ext` first. |
| `lamp doc` | Build and serve docs from the interface and source. |
| `lamp release` / `yank` | Publish or withdraw. |
| `lamp deploy` / `rollback` / `targets` | Run, update, revert, and list app instances. |
| `lamp serve` | Run a registry. |
| `lamp gc` / `trust <key>` | Prune the store; acknowledge a new or rotated signer. |
| `lamp bug` / `bugs` / `changelog` / `notes` | Maintainer surface. |
| `lamp nominate <pkg>` | Start the community → `ext` promotion. |

## What this borrows, and what it refuses

- **Cargo** — one tool, one manifest, one lockfile, integrated test/bench/doc,
  workspaces (the basis for `members`). Adopted wholesale. Refused: maximal
  version resolution, `build.rs` arbitrary code execution, subtractive features.
- **Go modules** — MVS, checksum database, transparency, decentralization.
  Adopted. Refused: `/v2` major-version paths, the weak capability story.
- **npm and friends** — the cautionary tale. Tool fragmentation, deep unsigned
  mutable graphs, `postinstall`, typosquatting, dependency confusion. Rejected
  wholesale, and `ext` curation is the answer to the redundancy problem.
- **Nix / Guix** — content-addressed immutable store, hermetic reproducible
  builds, perfect dedup. Adopted, hidden behind boring commands.
- **Deno** — capability permissions, decentralization, signing. Adopted, but
  with a *declared, compiler-verified* manifest so refusal happens at compile
  time rather than at runtime.
- **Maven / Gradle** — coordinates, checksums, signing, binary artifacts,
  multi-module builds. Adopted. Refused: XML sprawl, and the shaded-fat-jar
  anti-pattern — binaries reference deduplicated store entries; bundling happens
  once at the application leaf, never per library.
- **Sigstore / SLSA / in-toto / TUF** — transparency logs, provenance, rebuild
  verification, delegated trust and revocation. Adopted, and they are what fix
  bare trust-on-first-use.
- **Hackage, Elm, OPAM, Pub** — curated registries and one-canonical-package
  culture (the basis for `ext`). Elm's compiler-computed semver bump is the
  model for interface-driven versioning.

## Implementation order

| Phase | Work |
| --- | --- |
| 1 | Split the `lamp` driver from `jinnc`; unify the verb set. |
| 2 | Manifest v2: multi-output `provides`, `members`, `capabilities`, `features`, sources, dev-deps, restricted sublanguage. |
| 3 | Output and member DAG: name unification, cycle detection, per-output scoping. |
| 4 | Content-addressed store, canonical archive, BLAKE3. |
| 5 | Lockfile v2 with content, signature, caps, features, deps; diff driver. |
| 6 | MVS resolver with conflict reporting; binary deps as ABI-pinned ranges. |
| 7 | Capability derivation from effects; resolver enforcement; FFI taint. |
| 8 | Signing, TUF roots, transparency log, revocation, provenance, SBOM. |
| 9 | `.jnb` writer and reader: interface v2, MIR/object pack, dependency descriptor, interface-driven semver diff. |
| 10 | Registry protocol and `lamp serve`; git transport bridge. |
| 11 | Reproducible-build normalization and `--verify`. |
| 12 | Maintainer surface: bugs, changelog, notes, release, yank. |
| 13 | Two-axis output model: kind and substrate-dial inference, layered link selection, freestanding targets, derived deploy contract. |
| 14 | Deployments: manifest block, deploy record, deploy transaction, `local` and `host` platforms, fleet rollout. |
| 15 | `ext/` tier wiring, curation CI, `nominate`, `gc`, `trust`. |

Each phase carries conformance tests in the Jinn style, plus end-to-end cases: a
multi-output project with a `members` child builds reproducibly; a `.jnb`
round-trips (build a lib, consume it as binary and binary-debug, confirm
type-checking is identical to source); tampered hashes, capability widening,
signature mismatch, revoked keys, and FFI taint are all rejected; two binaries
share one deduplicated dependency; a minor bump leaving the used-surface hash
stable links without rebuild while a signature-changing bump forces a source
rebuild; an irreducible major conflict is a hard error that resolves only via
explicit isolation; an `app` kind is inferred from an opened store and a
`bin`/`app` mismatch is rejected; a store-schema bump plans a checked migration
and blocks when the migration path is ill-typed; a `heapless` firmware errors
when a reachable path allocates and links no allocator when it does not.
