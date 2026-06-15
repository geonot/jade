# lamp — the Jinn package system

**Status: design (task 2-19, rev 2). Specifies the target; supersedes the
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

**Names fixed by this revision.** Manifest: `project.jn`. Lockfile:
`project.lock`. Binary library: `.jnb`. These are not negotiable downstream.

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
2. **One manifest, one lockfile, per project tree.** A project is described by
   `project.jn` (human-authored, Jinn syntax) and pinned by `project.lock`
   (machine-authored, never hand-edited). A project may *produce many
   packages* (§2.4), but there is exactly one manifest and one lockfile at its
   root. Nothing else is authoritative.
3. **Curated by default, open at the edges.** Functionality lives in tiers:
   `std` → `ext` → community. For any one facet there should be **one
   quintessential package** — standards-conformant, minimal, complete,
   idiomatic. lamp actively patronizes the canonical package and makes
   discovering a fourth redundant JSON library a deliberate act, not the
   default. (`libjn` is *not* a tier — see §1.0.)
4. **Trust is explicit and verifiable.** Every artifact is content-addressed
   and signed. Every dependency declares the **capabilities** it needs
   (filesystem, network, process, env, unsafe FFI). The resolver refuses to
   silently widen a package's capability set. Supply-chain safety is a
   first-class feature, not a bolt-on `audit` command.
5. **Reproducible or it didn't happen.** Given a `project.lock`, a build on any
   machine at any time produces a bit-identical artifact (modulo the declared,
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

Jinn ships functionality in concentric rings of decreasing curation and
increasing openness. lamp treats them uniformly at the call site — `use json`
is `use json` regardless of tier — but applies different trust and resolution
rules.

| Tier        | Location          | Who maintains it             | Versioning            | Trust |
|-------------|-------------------|------------------------------|-----------------------|-------|
| `std`       | `std/*.jn`        | core team, in-tree           | tied to compiler      | implicit, always available |
| `ext`       | `ext/<name>/`     | community, **vendored** into the Jinn repo under project purview | independent semver, reviewed | curated, audited, signed by core |
| community   | external registry | anyone                       | independent semver    | signed by author; capability-gated |

### 1.0 `libjn` is not a packaging tier

`libjn` is Jinn's **freestanding runtime / libc replacement** — the low-level
substrate (allocator, syscall shims, string/mem primitives, the C runtime
surface) that the compiler and `std` are built *on top of*. It is orthogonal
to lamp: you never `lamp add libjn`, it has no manifest, and it is not
resolved through the dependency graph. It ships and versions with the
toolchain like the code generator does. The previous draft mistakenly listed
it as a curation tier; this revision removes that. Where this spec speaks of
"the toolchain," `libjn` is part of the toolchain. Because `libjn` is layered
(freestanding primitives → allocator → OS/syscall shims → scheduler), it is
also what makes the freestanding output kinds (`os`, `raw`) and the substrate
dials (`runtimeless`/`threadless`/`heapless`, §2.3.1) possible: lamp links only
the layers the artifact's used surface actually reaches.

### 1.1 `std`

The language's batteries. Not packages you add; always present and
version-locked to the compiler that built them. `use math`, `use json`,
`use http` resolve here first. This is deliberate: the standard library is the
answer to "what is the one true X" for the most common Xs.

### 1.2 `ext` — the curated extended library

`ext` is the bridge between "in the language" and "out on the open registry."
An `ext` package:

- Has its source **vendored into the Jinn repository** under `ext/<name>/`,
  brought under the project's review, license, and CI purview.
- Has passed a contribution review: it is the *quintessential* implementation
  of its facet (e.g. `ext/toml`, `ext/uuid`, `ext/zip`, `ext/sqlite`,
  `ext/markdown`), standards-conformant, minimal-but-complete, idiomatic Jinn,
  with a conformance test suite in the Jinn style.
- Versions independently of the compiler (its own semver), but is **built and
  tested in the Jinn CI** against every compiler release, so it never bit-rots.
- Is signed by the core release key and distributed both in-tree and via the
  registry. Adding it is `lamp add ext/zip`; resolution is instant and
  offline-capable.

**Why `ext` exists.** It gives the community a *destination*. Instead of N
competing JSON/UUID/HTTP-client packages (the npm failure mode), there is a
clear social and technical gradient: write a great package → get it adopted
into `ext` → it becomes *the* one. This concentrates effort, reduces attack
surface, and answers principle 3. lamp's search ranks `ext` above community
results and labels them **canonical**.

### 1.3 Community packages

Everything else. Published by anyone to any `lamp serve` registry, addressed by
scoped name + content hash, signed by the author, gated by capabilities (§6).
First-class — Jinn is not a walled garden — but carrying the weakest implicit
trust and the strongest explicit checks.

**Promotion path.** A community package that becomes the de-facto standard for
its facet is a candidate for `ext` adoption (`lamp nominate <pkg>`), turning
"popular" into "curated and signed."

---

## 2. The manifest: `project.jn`

The manifest is **Jinn source**, parsed by the compiler's own lexer/parser
(consistent with `Package::from_project_file`). No second config language. This
is the intelligent-compiler convention: you already know Jinn syntax; you do
not learn a manifest DSL.

```jinn
project
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
  forked-zip  is git 'https://git.example/zip' rev 'a1b2c3d'

provides
  lib orchard-core is 'src/lib.jn'         # a .jnb library
    requires internal                      # output-scoped deps (§2.3)
  bin orchard-prune is 'src/prune.jn'      # a run-and-exit CLI
    requires orchard-core                  # one output depends on another
  app orchard      is 'src/main.jn'        # a deployable application (§2.3.1)
    requires orchard-core                  # kind usually inferred; pinned here

members
  ./planner                                # child project (§2.4)
  ./crates/internal

dev-requires
  ext/quickcheck is '^1.0'                 # only for `lamp test`

build
  toolchain is '>=1.9.0'                   # compiler version constraint
  targets   is ['native', 'wasm32']
  profiles
    release is opt 3, debug-info false
    dev     is opt 0, debug-info true

deployments                                # where instances of `app` run (§9)
  dev   is local                  data './.run/dev'
  prod  is host 'deploy@orchard.example'  replicas 3
```

### 2.1 The restricted manifest sublanguage

The manifest is **declarative and side-effect free.** To make "parsed, never
executed" precise (§6.4), the manifest is a *restricted Jinn dialect*: only
the top-level blocks `project`, `requires`, `provides`, `members`,
`dev-requires`, `build`, `capabilities`, `features`, `deployments`, and
`[target]` overrides are recognized; only `is`-bindings to **literals** (string, int, bool, list)
and the dependency-source keywords (`from`, `path`, `git`, `rev`) are allowed.
The `deployments` block adds a small, equally-declarative keyword set
(`local`, `host`, `data`, `replicas`, `settings`, and the `for each … in`
fleet form — §9.2), and `provides` admits the substrate dials
(`runtimeless`/`threadless`/`heapless`) and freestanding targets
(`for arch '…'`, `for mcu '…'`, §2.3.1) — still binding only to literals and
names; none has more execution power than the rest. No function calls, no control flow (the
`for each` form is a declarative fan-out over a literal list, not a loop), no
`*name` definitions. The parser rejects
anything else with a precise diagnostic. There is no `postinstall`, ever — we
close npm's single largest attack vector by construction.

### 2.2 Top-level keys

- **`project`** — identity of the *project root*: name, version, license,
  edition, summary, authors. `name`/`version` here are the defaults inherited
  by every `provides` output that does not override them.
- **`requires`** — *root-shared* direct dependencies, visible to every output
  unless an output narrows its own set (§2.3). Bare name (`json`) means "the
  canonical package of this name," resolved std → ext → registry. A source
  qualifier (`from`, `path`, `git`) overrides the default registry.
- **`dev-requires`** — present only for `lamp test`/`lamp bench`; never enter a
  consumer's graph.
- **`build`** — pins the toolchain range, target list, and named build
  **profiles** (`opt`, `debug-info`, `lto`, …), feeding reproducibility (§7).
- **`capabilities`** — the *project-wide ceiling* (§6.1). Individual outputs
  may declare narrower sets.
- **`deployments`** — named **targets** for `app` outputs: the environments or
  per-tenant instances an application is deployed to, each a profile of
  environment, settings, persistence, and platform (§9). Present only for
  projects that ship an `app`; ignored entirely for pure `lib`/`bin` projects.

### 2.3 `provides` — multiple outputs from one project

A single project produces **one or more named outputs**. This is the
multi-package/multi-executable requirement made first-class. An output is
described on **two orthogonal axes** (§2.3.1), and together they tell lamp
*how to package, link, and run* it without further questions:

- the **kind** — *what the artifact is*;
- the **substrate** — *what runtime it assumes underneath*.

There are **five kinds**, the first three *hosted* (they assume Jinn's full
runtime — allocator, scheduler, OS) and the last two *freestanding* (no OS
beneath them):

- `lib <name> is '<entry>.jn'` — a `.jnb` **library** (§5): an interface plus
  packed MIR/object, publishable to a registry and consumable by other
  projects. Has no `main`. Packaged as a `.jnb` (§5.2) and content-addressed.
- `bin <name> is '<entry>.jn'` — a plain **executable**: a self-contained
  native artifact with a `main`, meant to be *run and exit* (a CLI, a build
  tool, a one-shot). It owns no managed persistence or lifecycle; lamp packages
  it as a single statically-linked native binary and nothing more.
- `app <name> is '<entry>.jn'` — an **application**: a long-lived, *deployable*
  unit with a `main` that owns persistent `@store`s (§2.3.1), settings, and an
  environment, and is the target of `lamp deploy` (§9). An `app` is the only
  kind that carries a deployment contract (store schemas, settings keys,
  capability ceiling, env requirements). lamp packages it as a *deploy bundle*
  (§9.1): the native binary plus a manifest of the store schema fingerprints,
  settings contract, and required capabilities it was built against.
- `os <name> is '<entry>.jn'` — a freestanding **operating-system image**: the
  artifact *is* the OS. There is no kernel beneath it; it owns the hardware,
  provides its own boot entry, and makes no syscalls (it *implements* them).
  lamp packages it as a bootable image (with a target machine/arch and a
  loader), linking only the freestanding slice of `libjn` (§1.0) — no hosted
  syscall layer, no OS scheduler unless the image brings its own.
- `raw <name> is '<entry>.jn'` — **bare-metal / embedded firmware**: a
  freestanding artifact for a fixed device with no OS and usually no heap. A
  `raw` output exposes a raw entry vector (reset handler / `_start`), targets a
  specific MCU/arch, and links the absolute minimum: value-semantics code,
  Perceus refcounting over static/arena memory, and only the `libjn` primitives
  it actually touches. Packaged as a flat binary / ELF for flashing.

Rules:

1. **Output names share a single namespace** within the project and must be
   unique. They are *distinct from* the `project.name` (which names the
   publishable unit / repo identity). An output may share the project name
   (`bin orchard`) or not (`lib orchard-core`).
2. **Inter-output dependencies are explicit and acyclic.** An output lists
   `requires <other-output>` to depend on a sibling `lib`. lamp builds a DAG
   over outputs and external deps; cycles are a hard error with the cycle path.
   This is *standalone referencing*: `orchard-prune` references `orchard-core`
   by name, nothing implicit.
3. **Output-scoped external deps** narrow the graph: a dep listed under an
   output is visible only to that output and its dependents, keeping each
   binary's closure (and capability union) minimal. Root `requires` are the
   shared baseline.
4. **Selective build/publish.** `lamp build orchard-prune`, `lamp run
   orchard`, `lamp release lib orchard-core@1.4.0` operate on a named output.
   `lamp build` with no target builds all outputs. Each `lib` is published as
   its own `.jnb` with its own content hash and (optionally) its own version.
5. **Per-output versions.** An output may set `version is '...'`; otherwise it
   inherits `project.version`. This lets one repo ship, e.g., a stable
   `orchard-core 2.x` library and a fast-moving `orchard 1.x` CLI.

#### 2.3.1 The two axes: kind and substrate

A kind is not bureaucracy; it is the single fact that lets lamp decide the
artifact shape, the link set, the lifecycle, and the trust surface without
asking. The **kind** decides *what is produced*; the **substrate** decides
*which layers of the runtime are linked in*. They are orthogonal: a `bin` can
be threadless, an `app` is fully hosted, an `os` is freestanding-by-definition.

**Axis 1 — kind.**

| Kind  | Has `main` | Substrate (default) | Persistent `@store`s | Packaged as | Lifecycle | `lamp deploy` |
|-------|:----------:|---------------------|:--------------------:|-------------|-----------|:-------------:|
| `lib` | no         | hosted              | no (may *define* store types, instantiates none) | `.jnb` (interface + pack), content-addressed | consumed at build time | no |
| `bin` | yes        | hosted              | none used            | single static native binary | run and exit | no |
| `app` | yes        | hosted              | one or more          | deploy bundle (binary + deploy contract) | long-lived, stateful, updatable in place | **yes** |
| `os`  | yes (boot entry) | freestanding   | n/a (owns the device) | bootable image (arch + loader) | owns the machine | no |
| `raw` | yes (reset vector) | freestanding | n/a                | flat binary / ELF for flashing | owns the device | no |

**Axis 2 — substrate dials.** The substrate is the set of `libjn` runtime
layers an artifact assumes. Rather than a fixed menu, it is expressed as
opt-outs from the full hosted runtime — and each is **inferred from the used
surface**, then optionally pinned:

- `runtimeless` — no Jinn runtime bring-up (no global init, no managed entry).
  The artifact provides its own entry and gets only the language primitives it
  calls. Implied by `os`/`raw`.
- `threadless` — no scheduler, no actors/channels (docs/concurrency.md). Perceus
  refcounting still works; the program is single-threaded. Inferred when the
  reachable closure spawns no actors and opens no channels.
- `heapless` (a.k.a. allocator-less) — no dynamic allocation. Value-semantics
  code over static and arena memory only; the allocator layer is not linked.
  Inferred when the closure performs no heap allocation. The compiler proves
  this from ownership/escape analysis and *errors* (rather than silently
  linking malloc) if a heap path is reachable under a pinned `heapless`.

The substrate is written inline after the entry, reading as prose:

```jinn
provides
  bin probe   is 'src/probe.jn'  threadless          # CLI, no scheduler linked
  os  kernel  is 'src/boot.jn'   for arch 'riscv64'   # freestanding, runtimeless
  raw blinky  is 'src/blinky.jn' for mcu 'rp2040' heapless   # firmware, no heap
```

`os` implies `runtimeless` and "no syscalls" (it provides them); `raw` implies
`runtimeless` and usually `heapless`. Pinning a dial that the inference already
holds is redundant (a `--pedantic` lint); pinning one the code *violates* is a
precise, located error (e.g. "`heapless` pinned but `Vec.push` at `boot.jn:42`
allocates").

**Inference, not declaration.** You rarely write either axis by hand. lamp
infers the kind from the entry's surface, matching the Jinn ethos (the compiler
does the heavy lifting):

- no `main` ⇒ `lib`;
- `main`, hosted, instantiates **no** persistent `@store` ⇒ `bin`;
- `main`, hosted, instantiates **one or more** persistent `@store`s (task 2-31
  store decorators) ⇒ `app`;
- a freestanding entry (declares `for arch`/`for mcu`, or pins `runtimeless`,
  or defines a boot/reset entry) ⇒ `os` or `raw` — `raw` when a device target
  is named and `heapless` holds, `os` otherwise.

Writing the kind explicitly (`app orchard is '...'`) is allowed and serves two
purposes: it *pins* intent (a mismatch between the declared kind and the
inferred surface is a precise error — e.g. declaring `bin` for an entry that
opens a `@store`, or `os` for an entry that makes hosted syscalls), and it lets
a stateless service that *will* grow persistence be deployable from day one.
`lamp lint` flags the redundant case (a declared kind/dial equal to the
inferred one) only with `--pedantic`. Only `lib`/`bin`/`app` are publishable to
the registry; only `app` is deployable (§9); `os`/`raw` are built and flashed,
not deployed or published.

**The deploy contract.** Building an `app` derives — from the same effect/store
analysis that powers capabilities (§6.1) — a *deployment contract* embedded in
the bundle:

- **store schemas** — for every persistent `@store` the app opens, its
  schema fingerprint (the schema-fingerprinted descriptor; task 2-31-9). This
  is what makes migrations checkable (§9.3) rather than hopeful.
- **settings contract** — the typed configuration keys the app reads from its
  environment/config, with their types and defaults, derived from the
  `config`/settings surface (docs/config-blocks.md). A target that fails to
  supply a required, default-less setting is rejected *before* a process
  starts.
- **capability ceiling** — the capability union (§6.1) the binary needs;
  carried into the deploy so a target environment can refuse, sandbox, or audit
  it exactly as the resolver does at build time.
- **env requirements** — declared external resources (a network address to
  bind, a data directory, secrets by name — never by value). These are
  *requirements*, satisfied per-target (§9.2), not baked in.

Because the contract is *derived*, it cannot drift from the code: an `app` that
adds a store, a setting, or a capability changes its contract, and a deploy to
an existing target surfaces that change as a reviewable diff (§9.3) — the same
discipline lamp already applies to dependency capabilities.

### 2.4 `members` — hierarchical projects (parent includes children)

For larger codebases a project may be a **parent that includes child
projects** (the workspace model, but coherent):

- Each entry under `members` is a path to a **child project** that has its own
  `project.jn`. The parent *includes* its children's outputs into one resolved
  graph and one lockfile at the parent root.
- **Hierarchical referencing.** Within the tree, any output can `requires`
  another member's `lib` by name — the parent flattens the member namespace
  into one DAG. A child built standalone (run `lamp build` inside `./planner`)
  uses only its own subtree.
- **Single lock, shared store.** The parent owns the one `project.lock`;
  members do not carry their own lockfiles when built under the parent (they
  may carry one only for standalone development, and the parent's lock wins
  when present). All members resolve the same external dependency versions
  (one version per package across the whole tree — §3.1), eliminating the
  diamond hazard within a workspace.
- **Two composition modes, made explicit:**
  - *standalone referencing* — a `lib` is published and consumed by content
    address like any external dep; the consumer need not know it came from a
    sibling. Use across team/repo boundaries.
  - *hierarchical referencing* — parent includes children; intra-tree deps
    resolve by name against member outputs, never hitting the registry. Use
    within one repo. A member can be promoted from hierarchical to standalone
    simply by publishing it; consumers switch from `requires planner` (member)
    to `requires planner from <registry>` with no source change.

`members` and `provides` compose: a member is itself a project that `provides`
outputs. The parent's effective output set is the union, with names required to
be globally unique across the tree (collisions are a hard error naming both
sites).

### 2.5 `features` — conditional compilation

Optional, additive **features** gate code and dependencies without forking
packages:

```jinn
features
  default is ['json']
  postgres is requires ext/pq
  metrics  is requires ext/prom, enables 'metrics'
```

Features are *additive only* (enabling one never removes API), so the resolver
can take the union across the graph and still satisfy MVS (§3). A dependency is
requested with its features: `pomelo is '^0.7' with [tls]`. Features feed the
`#when feature` compile-time gate in the source. This avoids npm's "optional
dependency" ambiguity and Cargo's occasional feature-unification surprises by
*forbidding* subtractive features outright.

---

## 3. Versioning and resolution

### 3.1 SemVer

lamp uses `major.minor.patch[-pre][+build]`. The prototype `SemVer`
(major/minor/patch) is extended with pre-release and build metadata.

| Form        | Meaning                                  |
|-------------|------------------------------------------|
| `1.4.0`     | exact                                    |
| `^1.4.0`    | `>=1.4.0, <2.0.0` (compatible)           |
| `~1.4.0`    | `>=1.4.0, <1.5.0` (patch-level)          |
| `>=1.2, <2` | explicit range                           |
| `*`         | any (discouraged; `lamp lint` warns)     |

Pre-releases are excluded from ranges unless explicitly opted into
(`^1.4.0-rc`). lamp resolves to a **single version per package** across the
entire project tree (unification across all outputs and members).

### 3.2 The resolver — MVS, reconciled with the lockfile

lamp uses **Minimal Version Selection** (the Go approach): pick the *minimal*
version satisfying all constraints. Rationale: reproducible *selection logic*
without surprise upgrades, and a natural push toward small, slow-moving graphs.

The previous draft implied MVS makes the lockfile redundant. It does not, and
this revision states the division of labor cleanly:

- **MVS decides *which version*.** This is a pure function of the manifests in
  the graph — deterministic, lockfile-independent.
- **The lockfile decides *which bytes* and *whether to trust them*.** A version
  number is not an integrity statement; the lock pins the **content hash** and
  **signature** for the selected version (§3.3). MVS without a checksum DB is
  insecure (Go learned this; hence `sumdb`). So: MVS for selection, lockfile +
  transparency log for integrity. No contradiction.

When constraints are genuinely incompatible, lamp **fails loudly** with the
conflict path (`A requires X ^1, B requires X ^2`) rather than silently
duplicating. Duplication of incompatible majors is allowed only behind an
explicit `requires-isolated` opt-in (§3.4), and each isolated copy is reported
in the SBOM.

### 3.3 The lockfile: `project.lock`

Machine-generated, committed to VCS, never hand-edited (extends `src/lock.rs`).
Every entry is **content-addressed**:

```
# project.lock — auto-generated, do not edit
schema 2
toolchain 1.9.0

ext/uuid 2.1.4
  source  lamp://reg.jinn.dev/ext/uuid
  content blake3:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
  sig     ed25519:core@jinn.dev:3a7bd3e2360...
  caps    []
  feats   []
  deps    [json]

pomelo 0.7.4
  source  lamp://reg.jinn.dev/pomelo
  content blake3:2c26b46b68ffc68ff99b453c1d304134...
  sig     ed25519:author@example.org:8f1c...
  caps    [net.client]
  feats   [tls]
  deps    [ext/uuid, json]
```

Beyond the prototype's `name url version commit`:

- **`content`** — BLAKE3 of the canonical package archive (§5.4). The integrity
  anchor; `lamp build` verifies it before use.
- **`sig`** — detached signature over the content hash + manifest (§6.2).
- **`caps`** — resolved capability set (§6.1), surfaced so a widening is a
  *visible diff* in code review.
- **`feats`** — the resolved (unioned) feature set, so the build is exact.
- **`deps`** — transitive edges, enabling offline graph reconstruction.

A lock change that widens `caps`, changes `sig` (key rotation/new signer), or
adds a `requires-isolated` duplicate is flagged by `lamp build` and rendered
specially in `git diff` via a provided diff driver. The lockfile is the
supply-chain review surface. Intra-tree member `lib`s (§2.4 hierarchical mode)
are recorded with `source path` and a content hash of the built `.jnb`, so even
local outputs participate in integrity checking.

### 3.4 Binary deps and the resolution boundary

A `.jnb` consumed in `binary`/`binary-debug` flavor (§5) carries *already
compiled* code. This raises the question the rest of this spec must answer
precisely: **when A is a binary and A requires X@1, Y@2, where do X and Y come
from, and at which versions?** Naively treating a binary as an opaque "leaf
pinned to its published versions" is wrong — it silently re-introduces the
diamond/duplicate-version problem that §3.1's single-version unification exists
to kill. The full mechanism is specified in **§5.6**; the resolution-level
contract is:

1. **A `.jnb`'s dependency edges are open, not frozen, by default.** A binary
   publishes its deps as *version ranges with interface hashes* (§5.6.2), not
   as hard-pinned bytes. The consumer's resolver folds those ranges into the
   one global MVS solution, so X resolves to **one** version across source and
   binary consumers alike. Single-version unification (§3.1) holds across the
   binary boundary.
2. **What a binary *cannot* do is recompile itself.** Its `mir/`/`object/` were
   monomorphized against the interface hashes it recorded. The resolver may
   pick any X in A's declared range **only if** that X's `interface.jhi` hash
   matches what A was compiled against (an *ABI-compatible* pick), or A ships
   source for a rebuild. §5.6.3 makes this check exact.
3. **Genuine incompatibility still fails loudly.** If two binaries were
   compiled against interface-incompatible majors of a third dependency and no
   single version satisfies both interface hashes, that is a hard error
   reporting both binaries, both ranges, and both interface hashes — never a
   silent duplicate. Deliberate coexistence requires `requires-isolated`
   (§5.6.4), and each isolated copy is reported in the SBOM.

---

## 4. The cache and store (content-addressed)

lamp maintains a per-machine **content-addressed store** (extends
`src/cache.rs`), keyed by BLAKE3, à la Nix:

```
~/.lamp/store/
  blake3-9f86.../           # immutable, hash-named, read-only
    project.jn
    src/...
    jnb/...                 # compiled artifacts (§5)
  index/                    # cached registry index snapshots
  keys/                     # trusted signer keys (TUF roots + log-anchored)
```

Properties:

- **Immutable & deduplicated.** Two projects (or two members of one tree) on
  the same hash share one entry. No per-project copy explosion.
- **Verifiable.** Store path *is* the hash. Tampering changes the path.
- **Offline-first.** A populated store + lockfile builds with no network.
- **Vendorable.** `lamp vendor` materializes the closure into `./vendor/` for
  air-gapped builds; the manifest can require `vendored` mode so CI never
  reaches the network.
- **GC'd.** `lamp gc` removes store entries reachable from no committed lock on
  the machine; immutability makes this safe.

A working tree contains only `project.jn` and `project.lock` (plus members'
sources); builds read deps from the store.

---

## 5. Binary library format: `.jnb`

> *"Is it possible to have a binary blob that can be integrated into a Jinn
> project?"* — Yes. This section specifies it. **The extension is `.jnb`**
> (Jinn Binary).

A `.jnb` is Jinn's **distributable compiled library**: a single
content-addressed archive carrying everything needed to type-check against,
link, and optionally debug a library **without its source**.

### 5.1 Why a binary format

- **Closed-source distribution** for vendors who cannot ship `.jn` source.
- **Faster builds**: skip recompiling stable dependencies from source.
- **Reproducibility + provenance**: a signed `.jnb` pins exactly what was
  built, by which toolchain, from which source hash.
- It must remain *integratable*: a downstream project links it as if it were a
  source dependency — same `use`, same inference, same Perceus ownership rules.

### 5.2 Structure

A `.jnb` is a sealed canonical archive (§5.4) containing:

```
project.jn           # package identity, version, caps, feature map
deps.descriptor      # dependency triples (range, iface-hash, built) — §5.6.2; NO dep bytes
interface.jhi        # "Jinn header interface" — see §5.3
mir/<target>/        # serialized typed MIR per target (the linkable code)
object/<target>/     # native object/static-lib per target (optional, fast-link)
debug/<target>/      # DWARF + source map + MIR↔source spans (debug variant)
provenance.json      # toolchain version, source content hash, build env hash
sig                  # signature over the whole sealed archive
```

A `.jnb` carries **only its own code**; the bytes of X and Y live once in the
content-addressed store and are referenced, never embedded (§5.6).

Three publish flavors:

- **source** — `.jn` only; downstream compiles. Max transparency, slowest.
- **binary** — `interface.jhi` + `mir/` + `object/`, no source, no debug.
- **binary-debug** — binary **plus** `debug/`, so a downstream developer can
  step into the library and get meaningful traces without the upstream source.

A `.jnb` is **target-scoped**: it records exactly which `targets` it contains.
Consuming for a target not present triggers source rebuild (if available) or a
precise error (§5.5).

### 5.3 The interface file: `interface.jhi`

The crux of "integrate a binary blob." `interface.jhi` is the **public,
machine-checked surface**: every exported type, function signature,
trait/protocol, store schema, effect/capability annotation, and — critically —
the **ownership and effect metadata** the type system and Perceus require.

Not C-style textual headers — a serialized slice of the typed HIR that lets the
downstream compiler type-check `use`s **exactly as if it had the source**,
including:

- Generic signatures and HM-inferred constraints.
- Value-semantics / borrow / `as` annotations (so `tests/access_semantics.rs`
  invariants hold across the boundary).
- Effect rows and the declared capability set (§6): a binary dependency cannot
  *hide* network I/O; the interface carries the effect.
- Perceus refcount obligations on returned/consumed values, so the caller's
  generated drops/dups are correct against pre-compiled MIR.

Serialized MIR keeps cross-`.jnb` inlining/specialization possible, preserving
Jinn's performance model.

### 5.4 Canonical archive & content addressing

Every distributable artifact (source pkg, `.jnb`) is a **canonical archive**: a
deterministic, sorted, normalized tar (fixed mtimes, fixed perms, sorted
entries, no platform noise) so the same inputs always hash identically. The
BLAKE3 of the canonical archive is the package's content address.

### 5.5 Interface stability — and the FFI integrity gap, closed

`.jnb` carries the **toolchain version** and an **interface schema version**.
The downstream compiler accepts a `.jnb` whose toolchain is within the
project's `build.toolchain` range and whose schema it understands; otherwise it
**falls back to building from source** (if available) or errors with a precise
"rebuild needed" message. We never link a stale/mismatched binary silently.
There is no frozen cross-compiler binary ABI (the C++ trap): binaries are a
*cache keyed by toolchain*; source is the source of truth.

**The FFI gap.** Capability derivation from effects (§6.1) works for pure Jinn,
but `ffi.unsafe` calls and pre-compiled objects in a binary `.jnb` can do
anything the OS allows — the compiler cannot *derive* their true effects. We
close this honestly rather than pretend:

1. A `.jnb` containing `object/` or any `ffi.unsafe` symbol **must** declare
   `ffi.unsafe` (and any concrete caps it grants through FFI) in its manifest;
   the interface marks the tainted symbols.
2. The derived-vs-declared check (§6.1) becomes a *one-directional containment*
   for binaries: the declared set is taken as ground truth, and any output that
   `requires` an `ffi.unsafe` binary **inherits that taint** in its own
   capability union — it cannot be laundered away.
3. `lamp build --deny ffi.unsafe` refuses such a graph entirely. This gives the
   consumer a hard, auditable boundary: trusting a binary's FFI is an explicit,
   logged decision, never silent.

### 5.6 Transitive dependencies of a binary — the resolution model

This is the question on which any binary-library design lives or dies: **A is a
`.jnb`; A requires X@1 and Y@2. Does A bundle X and Y inside itself, fetch them
by manifest at the consumer's build, or something in between?** Every prior
system answers badly in one of two ways — *static bundling* (each binary
carries private copies; you get N copies of X, code bloat, and no security
patch propagation, the Maven shaded-jar / Go-vendor-into-binary failure) or
*unconstrained late binding* (the binary names a range and prays the consumer
picks something link-compatible, the C/C++ shared-object hell). lamp takes a
third path that the content-addressed store and `interface.jhi` make possible.

#### 5.6.1 Principle: a binary references its deps, it does not contain them

**A `.jnb` never embeds the bytes of its dependencies.** It is *not* a fat
archive. A `.jnb` for A contains only A's own `interface.jhi`, A's own
`mir/`/`object/`, and a **dependency descriptor** naming X and Y. X and Y are
resolved into the *same* content-addressed store (§4) as everything else and
**deduplicated globally**. If three binaries and your own source all depend on
the same X@1.2.0, there is exactly **one** `blake3-…/X` store entry, used by
all four. This is precisely the user's option (c) — *packed-by-reference,
independent, deduplicated* — chosen deliberately over fat bundling (a) and over
naive by-manifest refetch (b), while subsuming the good parts of both: like (b)
the bytes are fetched/located through the resolver, but like (a) the build is
fully pinned and offline-capable because the descriptor records exact content
hashes, not just names.

The fat-bundle option is rejected outright: it defeats the store's dedup,
multiplies the attack surface, makes a CVE in X unpatchable without rebuilding
every binary that bundled it, and inflates the SBOM with hidden copies. The
only exception is `lamp vendor`/static-final-link for air-gapped *application*
delivery (§5.6.5), which is an explicit end-of-line packaging step, never the
library format.

#### 5.6.2 The binary dependency descriptor

Inside A's `.jnb`, the `project.jn` `deps` are recorded not as bare names but as
**triples** — `(range, interface-hash, compiled-against-version)`:

```
deps
  X  range '^1.0'  iface blake3:7d1a…  built 1.2.0
  Y  range '^2.3'  iface blake3:0fe4…  built 2.4.1
```

- **`range`** — the semver range A's *author* declared. Feeds the consumer's
  MVS exactly like a source dep's range. A binary is a first-class participant
  in the one global resolution, not a leaf.
- **`iface`** — the BLAKE3 of the `interface.jhi` of the *exact* X that A's
  `mir/object` were compiled and monomorphized against. This is the ABI anchor.
- **`built`** — the concrete version A was built against, for diagnostics and as
  the MVS *floor* for this edge (A cannot link against an X older than the one
  it was compiled with).

The descriptor is part of the signed, canonical archive (§5.4), so A's declared
dependency surface is tamper-evident and travels with the bytes.

#### 5.6.3 How the consumer resolves a binary's deps — ABI-pinned MVS

When A is in the graph, the resolver:

1. **Folds A's `range`s into the global MVS solution.** X's selected version is
   the MVS minimum satisfying *all* requesters — A's source siblings, other
   binaries, and A — exactly as for source. Result: one X version, tree-wide
   (§3.1 preserved).
2. **Checks ABI compatibility of the pick against A's `iface` hash.** Two
   outcomes:
   - **Interface-hash match** (the common case under semver — a patch/minor
     bump that adds API but does not change the symbols A uses keeps A's used
     surface hash-stable; see §5.6.6 on *surface hashing*): A's pre-compiled
     `mir/object` links directly against the selected X. No rebuild. This is the
     fast path and the whole point of binaries.
   - **Interface-hash mismatch** (X's selected version changed the surface A was
     built against): A's compiled code is **stale** for this X. lamp then, in
     order: (a) if A shipped `source` flavor, **rebuilds A from source** against
     the selected X — binaries silently degrade to source builds rather than
     link garbage; (b) else if a different in-range X exists whose interface
     hash *does* match A's `iface`, the resolver may select that X for A's
     subgraph (the narrow, recorded use of per-edge pinning); (c) else **hard
     error**: "binary A was built against X interface `7d1a…`; no in-range X
     provides it and A ships no source — rebuild A or relax the constraint,"
     naming every party.
3. **Records the outcome in `project.lock`.** The lock gains a `via` field on
   binary-introduced edges so the review surface shows *why* an X version is
   present and which binary's ABI pinned it (§5.6.7).

The crucial invariant: **lamp never links a binary against a dependency whose
interface differs from the one it was compiled against.** A version number is
not an ABI promise; the `interface.jhi` hash is. This closes the C/C++ "right
soname, wrong layout" class of bugs by construction.

#### 5.6.4 When a single version genuinely cannot satisfy everyone

If binary A needs X interface `7d1a…` and binary B needs X interface `0fe4…`
and these belong to incompatible majors, MVS cannot unify them and neither A nor
B ships source. This is a real, irreducible conflict. lamp's response, in
priority order:

1. **Report it as a hard error** with the full picture: both binaries, both
   ranges, both interface hashes, and the import paths — the §3.4 contract.
2. **Offer the explicit escape hatch `requires-isolated X`** in the manifest.
   This admits *two* X store entries into the build, each linked only into the
   subgraph that demanded it, with symbol namespacing so they never collide at
   link time. Both copies appear in the SBOM and in `lamp tree --caps` with an
   `[isolated]` marker; their capability sets union into the consumer
   separately. This is the *only* path to version duplication, and it is loud,
   deliberate, and auditable — the antithesis of npm's silent nested
   `node_modules`.
3. **Surface the cure**: `lamp audit` reports which binary, if rebuilt against
   the other's X, would collapse the isolation, and `lamp tree --why X` shows
   the divergence so a maintainer can push for a source release or an aligned
   bump upstream.

#### 5.6.5 Application final-link vs. library distribution

The "do binaries pack their deps" question has *two* legitimate answers
depending on the artifact's role, and lamp keeps them strictly separate:

- **A `lib` `.jnb` (library)** — *always* references deps (§5.6.1). It is a node
  in someone else's graph; bundling would be antisocial. This is the default and
  the only library form.
- **A `bin` (final application)** — at the *end* of the line, where the artifact
  is delivered to run, not to be depended upon, `lamp build` (and `lamp vendor`
  / `--static`) performs a **whole-program final link**: the resolved, unified,
  deduplicated closure is linked into one executable (or a vendored tree). Here
  the deps *are* packed — but this happens **once, at the leaf, after global
  unification**, so there is still exactly one X in the binary, chosen by the
  one MVS solution. Packing at the application boundary is fine; packing at
  every library boundary is the bug.

This is the clean resolution of the user's framing: libraries are
*packed-by-reference and deduplicated*; applications are *packed-by-value, once,
after unification*. The store guarantees the value packed is the same bytes the
references pointed at.

#### 5.6.6 Surface hashing — making the fast path actually fast

If the `iface` hash were over A's *entire* dependency interface, every trivial
addition to X would invalidate every binary using X and force mass rebuilds —
the fast path would rarely fire. So the `iface` hash is computed over the
**used surface**: the subset of X's `interface.jhi` that A's MIR actually
references (the symbols, types, monomorphizations, effect rows, and Perceus
obligations A links against), canonicalized and hashed. Adding unrelated API to
X leaves A's used-surface hash unchanged → fast path holds. Changing a signature
A *uses* flips the hash → A is correctly rebuilt or pinned. The compiler already
computes A's import set during type-checking against `interface.jhi` (§5.3); the
used-surface hash is a deterministic fold over exactly that set. This makes
binary reuse robust under normal semver evolution while staying sound.

#### 5.6.7 What the lockfile records (closing the loop with §3.3)

The §3.3 lock entry gains, for any edge introduced or constrained by a binary:

```
X 1.2.0
  source  lamp://reg.jinn.dev/X
  content blake3:…                     # the one shared store entry
  iface   blake3:7d1a…                 # used-surface hash the build linked against
  via     [A (binary, built 1.2.0), my-cli (source)]   # who requires it & how
  pin     abi                          # 'abi' = held by a binary's iface; 'isolated' if duplicated
```

So a reviewer sees, in one diff, that X is shared (not bundled), exactly which
binary's ABI pins it, and whether any duplication (`isolated`) entered the
graph. A future X bump that would break A's ABI shows up as a lock change that
either rebuilds A (if source) or requires an explicit decision — never a silent
relink. Transitive dependency handling is thus, like everything else in lamp, a
**reviewable, content-addressed, deduplicated, single-version-by-default**
property of the lockfile.

---

### 5.7 Path-scoped identity, hash unification, and the compatibility ladder

§5.6 resolves *which version* of a dependency enters the graph. §5.7 specifies
*how every package is named internally*, *when two packages may be proven the
same and collapsed*, and *what happens at a type boundary when they are not*.
The three layers are strictly separated so that each stays simple:

1. **Resolution names everything by path (always).** Deterministic, local.
2. **Unification collapses provably-identical packages (an optimization).**
   Hash-driven, safe-by-construction, never changes program meaning.
3. **The boundary flag decides cross-version coercion (a one-bit policy).**
   Default-exclusive, set by the exposer.

#### 5.7.1 The fully-qualified name

Every package instance has a canonical internal identity that is **scoped to its
immediate parent**:

```
foo:bar              # foo's direct dependency bar
foo:baz:bar          # bar as seen by baz, which is foo's dependency
```

`baz`'s `use bar` resolves against **`baz`'s** manifest — `foo:baz:bar` — with
no global negotiation. `foo`'s own `use bar` resolves to `foo:bar`. The two are
distinct identities by construction; resolution is purely local and therefore
deterministic (npm's nested insight, without npm's flat-collision arbitration).

The full reference carries version, owner-scope, and the **BLAKE3 semantic
hash** (§5.7.3):

```
bar[blake3:7d1a…]{foo:baz}(1.2.0)
```

A consumer may reach a transitive dependency explicitly with a **path import**,
`use baz/bar`, which binds `foo:baz:bar` directly. This is allowed only when
`baz` **re-exports** `bar` (a manifest `reexport bar` declaration); an
un-re-exported reach-in is a hard error, because a private implementation detail
of `baz` (which `bar` it happens to use) must not silently become part of
`foo`'s build — the npm reach-in fragility, closed by requiring consent.

#### 5.7.2 The unification pass — promote provably-identical packages

Path-scoping would, naively, force one copy of `bar` per scope even when they are
byte-identical. The **unification pass** recovers global dedup (§5.6.1) *as a
provably-safe optimization* rather than baking it into resolution:

> Two path-scoped instances `A` and `B` may be **promoted** to a single global
> entity — owner-scope and version dropped from the symbol, one copy of the
> code, one set of symbols — **iff** they satisfy the *promotion predicate*.

**Promotion predicate** (decided — `docs/caps.md` §4.1, tightened from the
original "no state cap" to "empty cap row" to make the proof airtight; this is
the resolution of prereqs Q1). `A` and `B` are promotable iff:

- **(identity)** their **semantic hashes are equal** (§5.7.3) — same code,
  same transitive layout, same ownership/Perceus glue, same effect rows; **and**
- **(purity)** both **capability rows are empty** (`caps == ()`, `docs/caps.md`
  §3.1) — not merely state-free. A clock/random-reading module is state-free but
  non-empty, so its *code* may be shared but its *identity* is never collapsed.
  Promoting only on the empty row removes the last class of aliasing surprise.

The purity clause is the keystone, and it is *free* because state is a
capability, not an ambient: a module that owns process-wide mutable state
(a registry, an interner, a `@store` handle, a scheduler hook) carries that
state as a declared cap threaded through its signature. Therefore:

- A **stateless** module (no state-cap in its signature) is *provably* free of
  hidden singletons. Identical hash ⇒ identical, observable-behaviour-preserving
  ⇒ unconditionally safe to collapse to one symbol. No heuristic "scan for
  globals" pass is needed — the cap system *is* the proof.
- A **stateful** module never accidentally aliases under unification: even if its
  *code* hash is identical and the code is shared, its *state* is per-cap, so two
  instantiations passed distinct cap instances stay distinct **by construction**.
  Code dedup and state identity are orthogonal; the cap makes them so for free.

Promotion is a pure optimization: it changes symbol count and binary size, never
program meaning. For the monomorphizing, value-semantics backend this directly
defeats the version-count × generic-fan-out code-bloat that pure nested
resolution would cause (identical semantic hash ⇒ identical monomorphization ⇒
one copy).

#### 5.7.3 The compatibility ladder — three hashes, not one

A version number is a human claim; the hash is the truth (§5.6.3 already pins
this for `iface`). §5.7 generalizes it into a **three-rung ladder**, computed
over canonical, deterministic input (the **typed, monomorphized MIR** — *not*
raw object code, which carries addresses, relocations, opt-level and
target-triple noise and would almost never match; see §5.7.5):

| Rung | Hash | What it covers | Equality means | Default action |
|------|------|----------------|----------------|----------------|
| **object** | binary hash of final object | code + layout + codegen | bit-identical artifact | link-time dedup (§5.7.5) |
| **abi** | Merkle hash of the **full transitive type/ownership/effect closure** of the used surface | memory layout, field order/size, Perceus owned-vs-borrowed obligations, **cap/effect rows**, calling convention of every type reachable through the signature | the ABI contract is identical; code may differ | **coercible** across versions when the boundary flag permits (§5.7.4) |
| **api** | hash of names + arity + nominal type identities only | the call *shape* | the surface looks the same; the contract may not | **never auto-coerce**; warn only |

The **abi** rung is the load-bearing, correctness-critical one, and it is why a
shallow signature hash is *forbidden*: `f(c: Config)` keeps the same `api` hash
when `Config` silently gains a field or widens an `i32`→`i64`, yet that flips
both layout and Perceus glue. The abi hash is therefore a **recursive Merkle
hash over the entire reachable type graph** (with cycle handling), including:

- field layout, order, and size of every reachable type;
- the Perceus ownership protocol (which fields are owned vs borrowed, drop/reuse
  glue) — *part of the ABI, not an afterthought*;
- the **capability/effect signature** — so a function that *acquires* a state
  cap between v1 and v2 hashes differently even at identical data layout, which
  is exactly right: gaining an effect *is* a behavioural/ABI change;
- calling-convention determinants (sret threshold, enum tag/niche layout).

Hashing only immediate param/return *identities* would make the abi rung a
silent memory-corruption generator for a value-semantics language with no boxing
escape hatch; the Merkle closure is mandatory.

The ladder drives diagnostics: object-match ⇒ unify silently; abi-match,
object-differ ⇒ *"compatible; behaviour may differ between X 1.2.0 and 2.0.0"*;
api-match, abi-differ ⇒ *"surface matches but the ABI contract changed; will not
unify"* — each naming both fully-qualified identities and their hashes.

#### 5.7.4 The boundary flag — default exclusive, set by the exposer

When two scoped instances are **not** promotable and a value of one crosses into
the other (`foo:baz:bar:Config` handed to `foo:bar:bar`'s `configure`), they are
**distinct nominal types** and the call is **rejected** — the sound default,
made unambiguous by full qualification. The *only* open decision is the policy
knob, which is one bit:

- **`exclusive` (default)** — the dep is private to its scope; cross-version
  coercion is forbidden; the reject stands. A scoped instance's choice of
  transitive version is *not* part of its public ABI.
- **`unify-ok`** — declared **by the package that exposes the type in a public
  signature**, this permits the compiler to coerce across versions *exactly when
  the **abi** hashes match* (§5.7.3). The boundary call then succeeds against one
  shared layout.

Default-exclusive matters: silent unification on incidental hash collision would
make `baz`'s private choice of `bar` into part of `baz`'s ABI — a patch bump
inside `baz` could start or stop unifying with `foo`'s `bar` and flip whether
`foo` compiles. The flag is therefore set by the side that *exposes* the type,
never inferred. This is the same public/private distinction §5.6.4's
`requires-isolated` gestures at, now stated as an affirmative, exposer-owned
opt-in rather than an escape hatch.

#### 5.7.5 Two hashes, two jobs: semantic vs binary

The **api** rung is *semantic and target-independent* — folded over the
**canonical typed MIR**, the same artifact §5.6.6 hashes for the used surface.
The **abi** rung is *semantic but **per-target*** (decided — `docs/interface-
hash.md` §3.4, resolving prereqs Q2): it folds the canonical MIR **plus the
target's `TargetLayout`**, because field layout, sret threshold, and niche
availability are target-dependent. A `.jnb` records `(target → abi_hash)`; an ABI
is a per-target contract and pretending otherwise would let a `wasm32` consumer
reuse a `native` layout. Both rungs The **object** rung is
a *binary* hash over the final compiled object, used solely for link-time dedup
and the reproducible-build check (§7). Conflating them breaks both: a binary hash
over object code rarely matches across builds (defeating unification), and a
semantic hash cannot dedup identical final objects. lamp keeps them distinct:
**semantic hash decides compatibility and promotion; binary hash decides
artifact identity.** The store (§4) keys on the binary hash; the resolver and
unification pass key on the semantic hash.

#### 5.7.6 The extreme case, and why it is sound

Two incompatible majors may coexist deliberately:

```jinn
use foo@1.2.0 as foo1
use foo@2.0.0 as foo2
```

This is admitted because the model already supports it: `foo1` and `foo2` are
`foo:foo1`-scoped and `foo:foo2`-scoped distinct identities with distinct abi
hashes, never promoted, their caps unioned separately into the consumer
(§6.1), both shown `[isolated]` in `lamp tree --caps` (§5.6.4). The one hazard is
**coherence**: if Jinn grows protocols/typeclasses, `foo@1` and `foo@2` could
each `impl Show for SharedType`, and a use site with a `SharedType` of ambiguous
provenance has two candidate instances. Until coherence is specified, lamp
**rejects** two live versions that would produce overlapping instances on a type
they do not both own (the orphan rule, version-aware). `as foo1/as foo2` is
permitted only when no such overlap exists.

---

## 6. Supply-chain security (the core of lamp)

Threat model: malicious/compromised transitive deps, typosquatting,
install-time code execution, dependency confusion, unsigned mutable registries,
key compromise.

### 6.1 Capabilities / effects manifest

Every package declares the **ambient capabilities** it requires. Capabilities
are a **separate effect pass** (decided — `docs/caps.md`, *not* a merged
`{errors, caps}` fixpoint); they share the inferred-by-default / annotation-
narrows *shape* of error effects but have their own pass, lattice, and storage.
The full capability spec — lattice, the `needs` clause at fn/method/class/module
sites, the `CAP_SITES` single-source attribution table, caps on function types,
and FFI taint — lives in `docs/caps.md`. The surface:

```jinn
capabilities
  net.client          # may open outbound sockets
  fs.read './assets'  # may read a scoped path
```

Classes: `fs.read`/`fs.write` (path-scoped), `net.client`/`net.server`,
`process.spawn`, `env.read`, `clock`, `random`, `ffi.unsafe`, `time`. The
compiler *derives* the actual effect set from the code and the resolver
**rejects any package whose code uses a capability it did not declare** (for
pure Jinn; for FFI/binary see §5.5). A pure data-structure library declares
nothing and is provably incapable of network access — verified, not promised.

The consuming project sees the **union** of its graph's capabilities in
`project.lock` (`caps`) and in `lamp tree --caps`. Per-output unions are shown
separately so a CLI's powers are distinguished from a library's. A dependency
update adding `process.spawn` is a one-line, reviewable lock diff.

`lamp build --deny net.server` (etc.) lets a consumer hard-refuse capabilities
regardless of declaration, enforced at compile time (the effect cannot be
discharged), giving defense in depth.

### 6.2 Signing, provenance, and key transparency

- Every published artifact is **signed** (Ed25519) over its canonical content
  hash + manifest. `ext` and `std` are signed by **core release keys** shipped
  with the toolchain (a root of trust you already installed).
- **Key model — beyond bare TOFU.** The previous draft's plain TOFU has a known
  weakness (first-contact MITM, no recovery from compromise). We strengthen it:
  the registry maintains a **TUF-style signed root + delegations** and a
  **transparency log** (below). A package's signer key is *anchored in the
  log*; a client trusts a key on first use **only if** its binding is witnessed
  in the append-only log it can independently verify and gossip. A new key for
  an existing package requires either (a) a signature from the prior key
  (rotation), or (b) explicit `lamp trust` after showing the log evidence —
  defeating silent account-takeover republishing.
- **Revocation.** A compromised key is revoked via a signed entry in the root
  metadata; clients refuse artifacts signed by revoked keys for *new*
  resolutions while existing locks still build (immutability of bytes is
  sacred). `lamp audit` surfaces locked deps signed by since-revoked keys.
- `provenance.json` records building toolchain, source content hash, and a hash
  of the (declared, minimal) build environment, enabling **independent rebuild
  verification** (SLSA-style).
- **Transparency log**: registries MUST append every published
  (name, version, content-hash, signer) tuple to an append-only Merkle log
  (Sigstore/`rekor`-style) clients audit and gossip, so a registry cannot serve
  different bytes to different clients undetected.

### 6.3 SBOM

`lamp build`/`lamp publish` emit a **Software Bill of Materials**
(`project.sbom`, SPDX/CycloneDX export available) enumerating every closure node
with name, version, content hash, signer, license, and capability set — per
output, since each `bin`/`lib` may have a distinct closure. First-class output.

### 6.4 No install-time code execution — by construction

**No** install/postinstall/prepublish lifecycle scripts. The manifest is the
restricted sublanguage of §2.1 — parsed, never executed. Builds run only the
declared compiler toolchain on declared source. Build-time codegen (if ever
needed) is a *declared, sandboxed, capability-gated* build step, not arbitrary
shell. This eliminates the dominant npm/PyPI attack vector outright.

### 6.5 Naming, typosquatting, dependency confusion

- Canonical names (`std`, `ext/*`) are **reserved** and resolve in-toolchain
  first; a registry can never shadow `json` or `ext/uuid`. Closes
  dependency-confusion for curated tiers by construction.
- The registry enforces a **namespace** model (`@scope/name`); authors own a
  scope; unscoped names are reserved for `ext`-promoted packages.
- **Member/output names never resolve from a registry.** Within a project tree
  (§2.4), `requires planner` binds to the member output; lamp will not silently
  substitute a public `planner` — closing intra-workspace dependency confusion.
- `lamp add` warns on Levenshtein-near matches to popular/canonical names and on
  brand-new, low-download, single-author packages pulled transitively.

### 6.6 Auditing

`lamp audit` cross-references the closure against a signed **advisory database**
(itself a versioned, signed, mirror-able package). It reports affected paths and
the *minimal* version bump clearing each advisory, and `lamp audit --fix`
computes it. Advisories are content-addressed and gossiped like packages, so the
feed is as decentralized as the registry.

---

## 7. Reproducible builds

Given `project.lock` + pinned toolchain, `lamp build` is deterministic:

- Canonical archives (§5.4) make inputs hash-stable.
- The compiler runs in a **normalized environment** (sorted env, fixed locale,
  fixed temp paths, no embedded absolute paths/timestamps; build paths
  remapped).
- Output artifacts are themselves content-addressed; two clean builds of the
  same lock on different machines produce byte-identical `.jnb`/binaries per
  target/profile.
- `lamp build --verify` rebuilds and asserts the content hash matches the lock /
  a published provenance record — the consumer-side half of §6.2.

`vendored` mode (§4) plus a pinned toolchain gives fully hermetic, offline,
reproducible CI.

---

## 8. The registry: a protocol, not a company

A **federated, content-addressed protocol** so the ecosystem cannot be captured
or centrally killed.

### 8.1 Model

- A registry is anything that speaks the **lamp registry protocol** (versioned;
  the client sends a protocol version, the server advertises its range) over
  `lamp://` (HTTPS transport). `lamp serve` turns any directory or object store
  into one.
- Packages are **content-addressed**; the registry maps
  `name@version → content-hash + signed metadata`. The *bytes* can come from any
  mirror holding the hash — the registry is an index + trust layer, not a sole
  bytes-host. Hashes are portable; that is what makes it decentralized.
- The **canonical index** (`reg.jinn.dev`) is a signed, versioned,
  Git-mirrorable Merkle index. Anyone can clone and mirror it; clients fall back
  to mirrors transparently. No single point of failure.
- **Publish authorization** is scope-based: a scope's TUF delegation lists the
  keys allowed to publish under it; a `release` is accepted only if signed by a
  delegated key and appended to the transparency log. There is no
  password/token bearer model that, if leaked, grants silent publish.
- **Federation**: `project.jn` / global config lists trusted registries in
  order; resolution walks them. An organization runs a private registry that
  *overlays* the public one, with §6.5 confusion protection.

### 8.2 Decentralized / P2P (forward-looking)

Because everything is content-addressed and signed, the bytes layer can be
backed by content-addressed P2P transport (IPFS-style CIDs / swarms) with the
signed Merkle index providing discovery and integrity. The canonical host
becomes a *seeder and indexer*, not a *gatekeeper*. Later milestone; the data
model is designed for it now — nothing assumes one trusted server holds bytes.

### 8.3 Prototype migration

The current git-tag publish / `git clone` fetch is a **valid degenerate
registry**: a git remote *is* a content-addressed mirror (commit hashes), and
`git`-source deps stay first-class (`from git ... rev ...`). lamp keeps git as a
transport while layering index, signing, capability, and content-hash semantics
on top. No flag day.

---

## 9. Deployments — running your application instances

Everything above gets a *library* or an *executable* built, signed, and into a
store. Deployment is the other half of an application's life: taking an `app`
output (§2.3) and **running an instance of it in an environment**, then keeping
that instance current as the code, its stores, and its settings evolve. This is
the operator's surface, and lamp owns it with the same discipline it brings to
dependencies: derived contracts, content addressing, reproducibility, and
reviewable diffs — no ceremony in the common case.

The shape is deliberately small:

```
lamp deploy <application> <target>
```

`<application>` is an `app` output of the project (the only deployable kind —
§2.3.1). `<target>` is a named **deployment target**: a place an instance runs,
with a profile of environment, settings, persistence location, and a platform.
Targets are *named*, and the names are yours. The two canonical shapes are the
classic environment ladder and per-tenant instances:

```jinn
# environment ladder
deployments
  dev     is local                       data './.run/dev'
  stage   is host 'deploy@stage.internal'
  prod    is host 'deploy@orchard.example'  replicas 3

# per-tenant instances of the same app
deployments
  tenant for each name in tenants         # user1, user2, user3, …
    is host 'deploy@{name}.orchard.app'
    settings tenant-id is name
```

Both forms produce the same thing: a set of named targets, each a fully
specified place to run an instance of an `app`. `lamp deploy orchard prod`,
`lamp deploy orchard tenant/user2`. With no target, lamp lists targets and the
deployed revision of each.

### 9.1 What a deployment *is* — the deployed-state record

A target's live state is itself content-addressed and recorded, so a deployment
is auditable and reversible rather than a fire-and-forget `scp`. For each
target lamp keeps a **deployment record** (committed alongside `project.lock`,
human-readable, machine-authored — `project.deploy.lock`):

```jinn
# project.deploy.lock — auto-generated, do not edit
target prod
  app          orchard
  revision     blake3:9f3c…             # exact app bundle hash deployed
  version      1.4.0
  deployed-at  2026-06-14T22:10:03Z
  store-schemas
    accounts   blake3:11ab…             # schema fingerprint live at target
    audit-log  blake3:77de…
  settings-hash blake3:c0ffee…          # hash of the resolved settings set
  capabilities [net:bind:443, fs:rw:/var/lib/orchard]
  signer       blake3:keyA…
```

The record is the join point for every deploy operation: the **binary update**
diffs `revision`; **store migrations** diff `store-schemas`; **settings**
changes diff `settings-hash`; **capability** changes diff `capabilities` — each
shown and confirmed before anything mutates. Because the deployed bundle is the
same content-addressed `.jnb`/native artifact the build produced (§5, §7), the
revision hash *is* the proof of what is running: reproducible build → identical
hash → "the running instance is exactly this commit," verifiable after the
fact.

### 9.2 The target profile — environment, settings, persistence, platform

A target satisfies the app's **deploy contract** (§2.3.1). Four facets, each
inferred-with-defaults, overridable per target, and checked *before* a process
is touched:

- **environment** — declared external resources the app requires: an address to
  bind, a data directory, named secrets (by *name*; lamp never stores secret
  *values* — it records that the target must supply `db-password`, and binds it
  from the platform's secret source at launch). A target missing a required,
  default-less resource is rejected with the exact unmet requirement.
- **settings** — the typed configuration keys from the settings contract.
  Defaults come from the app; a target overrides any key (`settings log-level
  is 'debug'`). Types are checked against the contract, so a misconfigured
  target fails at `lamp deploy`, not at 3 a.m. in production.
- **persistence** — where the app's `@store`s live for *this* instance
  (`data './run/dev'`, a host path, an object-store URL). Persistence is
  per-target, so `dev` and `prod` and each tenant get isolated state by
  construction; the store schema fingerprints in the record pin what shape that
  state is in.
- **platform** — how the instance is launched and supervised: `local` (a
  managed local process), `host <ssh-target>` (push bundle + supervise over a
  transport), or a pluggable platform driver (container image, systemd unit,
  cloud target). The platform is the *only* facet with an external adapter; the
  other three are pure data. lamp ships `local` and `host`; everything else is
  a driver resolved like any dependency, capability-gated.

All four are declared in the `deployments` block of `project.jn` (restricted
sublanguage, §2.1 — declarative, no execution) and resolved against the app's
derived contract. A target that under- or over-specifies the contract is a hard
error naming the offending key, *before* deployment begins.

### 9.3 The deploy transaction — binary update, store migrations, settings

`lamp deploy <app> <target>` is a **single transaction with a plan-then-apply
gate**, modeled on the resolver's reviewable-diff philosophy:

1. **Build & verify.** Build the `app` for the target's platform under the
   committed lock (reproducible, §7); compute the bundle revision hash; verify
   signatures and the capability set.
2. **Compute the deploy diff** against the target's deployment record (§9.1):
   - *binary*: old revision → new revision (and the source/dep diff behind it);
   - *stores*: for each `@store`, old schema fingerprint → new. A changed
     fingerprint requires a **migration** (below);
   - *settings*: added/removed/changed keys, with new required-without-default
     keys flagged as **blocking** until the target supplies them;
   - *capabilities*: any *widening* is flagged exactly as the resolver flags a
     dependency widening (§6.1) — a deploy that needs a new capability at a
     target is an explicit acknowledgement, never silent.
3. **Plan store migrations.** A changed store schema fingerprint is resolved
   against the app's **migrations** — ordered, content-addressed schema
   transforms the app declares for its stores (each keyed by `from`→`to`
   fingerprint, so the path from the *target's live* schema to the *new* schema
   is computed, not assumed). Migrations are checked at *compile* time against
   the store schema (task 2-31-9 fingerprints), so an unreachable or
   ill-typed migration is a build error, not a runtime corruption. lamp shows
   the migration path and whether it is online (no downtime), offline (instance
   paused), or destructive (requires `--allow-destructive` and a snapshot).
4. **Gate.** The full diff — binary, migrations, settings, capabilities — is
   printed and confirmed (`--yes` to skip in CI). Nothing has mutated yet.
5. **Apply, atomically and reversibly.** lamp snapshots the target's store
   state (cheap, content-addressed), runs migrations, stages the new bundle,
   cuts over, and only then updates the deployment record. A failure at any
   step rolls back to the snapshot and the prior record. `lamp rollback <app>
   <target>` restores the previous recorded revision and the matching store
   snapshot in one step — the deployment record makes "what was running an hour
   ago" an exact, restorable fact.

Because every input is content-addressed and the record pins the live schema,
*re-running the same deploy is a no-op* (the diff is empty) — deploys are
idempotent, which is the property that makes them safe to put in CI and safe to
retry.

### 9.4 Many targets, one app — fleets and per-tenant rollout

The `for each` target form (above) makes a *fleet* of identical-shaped
instances first-class. `lamp deploy orchard tenant` (no specific tenant) plans
the diff once and applies it across the fleet with a configurable strategy:
`--rolling` (one at a time, halt on first failure — the default), `--canary
<target>` (deploy to one, observe, then proceed), or `--all-at-once`. Each
instance keeps its own deployment record and its own isolated persistence, so a
migration that succeeds on `user1` and fails on `user2` leaves `user2` rolled
back and the fleet partially updated — reported precisely, never silently
divergent. Per-target settings (`tenant-id is name`) differentiate instances
without forking the app.

### 9.5 Why deployment belongs in lamp

A deployment is just another reproducible, content-addressed, capability-bounded
artifact transition — the exact problem lamp already solves for dependencies,
turned outward at runtime. Putting it in the one tool means the bundle you
deploy is provably the bundle you built (§7), the stores you migrate are
checked against the schemas you compiled (task 2-31-9), the settings you set are
type-checked against the contract you derived (§2.3.1), and the capabilities you
grant a running instance are the same reviewable set the resolver vetted at
build time (§6.1). Deployment is not a separate ops product bolted on; it is the
runtime face of the same trust fabric — boring, verifiable, and idempotent by
construction.

---

## 10. The maintainer surface: bugs, changelogs, releases

A package is more than code. lamp brings issue tracking and release notes into
the **same content-addressed, signed, federated fabric** — so they survive,
mirror, and verify like the code, rather than living in a proprietary forge.

### 10.1 Bug tracking

- Issues live in-repo under structured `issues/` (append-only, content-addressed
  records: id, reporter, signed body, status, labels). Part of the package;
  travel with mirrors; work offline.
- `lamp bug "title"` files one (signed); `lamp bugs` lists; `lamp bug show <id>`;
  maintainers `lamp bug close <id> --fixed-in 1.4.1`, linking the fix to a
  release, propagating to the changelog and (if security) the advisory DB.
- Signed records let a federated tracker aggregate across mirrors with no
  central server; a maintainer's "close" is verifiable.

### 10.2 Changelogs & patch notes

- `CHANGELOG` as structured per-version entries
  (`added`/`changed`/`fixed`/`security`/`removed`, Keep-a-Changelog shape).
- `lamp changelog add fixed "..."` appends to the *unreleased* section;
  `lamp release` seals it; `lamp notes 1.4.1` / `--since 1.2.0` render/aggregate
  human notes. Security entries auto-feed the advisory DB.

### 10.3 The release flow (replaces `cmd_publish`)

`lamp release [<lib|bin> <name>][@<version>]` is the maintainer's one command.
With no output named, it releases every publishable `lib` in the project at the
project version. It:

1. Verifies a clean tree and that the relevant `version` matches (and is `>` the
   last release of that output).
2. Seals the changelog's unreleased section; links bugs closed since last
   release.
3. Builds the requested flavors (`--source`/`--binary`/`--binary-debug`,
   default `source`) per target/profile, each a canonical archive (§5.4).
4. Computes content hashes, signs them and the manifest, emits `provenance` and
   the SBOM.
5. Publishes to the configured registry (and pushes a git tag namespaced by
   output, e.g. `orchard-core/v1.4.0`, so multiple outputs from one repo tag
   cleanly) and appends to the transparency log.
6. Prints the consumer `requires` line with content hash and signer shown.

`lamp yank <name>@<version>` marks a version unusable for *new* resolutions
without deleting bytes (existing locks still build — published bytes are
immutable; we advise against, never rewrite history).

### 10.4 Interface-driven semver (Elm-style)

Because `interface.jhi` is a precise machine surface, `lamp release` **computes
the minimum legal semver bump by diffing the new interface against the last
published one**: added API → minor, changed/removed → major, internal-only →
patch. If the maintainer's declared `version` undershoots the computed minimum,
`lamp release` refuses with the offending API diff. Accidental breaking changes
become hard to ship. This applies per `lib` output independently.

---

## 11. Scaffolding & developer ergonomics

One coherent verb set. No `init` vs `new` vs `create` confusion.

| Command | Does |
|---------|------|
| `lamp new <name>` | scaffold a project (`--bin`/`--lib`/`--workspace`) |
| `lamp add <pkg>` | add a dep, resolve, update lock, show added caps |
| `lamp remove <pkg>` | drop a dep, prune lock |
| `lamp build [<output>]` | resolve + compile via `jinnc`; verifies hashes/sigs; all outputs if none named |
| `lamp run <output>` / `lamp test` / `lamp bench` | build then run/test/bench |
| `lamp lock` / `lamp update [pkg]` | regenerate / bump lock (MVS) |
| `lamp vendor` | materialize closure into `./vendor` |
| `lamp tree [--caps] [--why <pkg>] [--output <name>]` | per-output graph, caps, paths |
| `lamp audit [--fix]` | advisory scan + minimal fix |
| `lamp lint` | manifest hygiene (`*` ranges, unused deps, license, member cycles) |
| `lamp search <q>` | search; ranks std/ext (canonical) first |
| `lamp doc` | build & serve docs from `.jhi` + source |
| `lamp release [<output>][@ver]` / `lamp yank <name>@ver` | §10.3 |
| `lamp deploy <app> <target>` / `lamp rollback <app> <target>` | run/update/revert an app instance (§9) |
| `lamp targets [<app>]` | list deployment targets and their live revision (§9.1) |
| `lamp serve` | run a registry (§8) |
| `lamp gc` | prune unreachable store entries (§4) |
| `lamp trust <key>` | acknowledge a new/rotated signer (§6.2) |
| `lamp bug` / `lamp bugs` / `lamp changelog` / `lamp notes` | §10 |
| `lamp nominate <pkg>` | start the community→`ext` promotion (§1.2) |

Scaffolds produce a *minimal* `project.jn`, a `src/` tree, a `tests/` tree wired
to `lamp test`, a starter `CHANGELOG`, and a `.gitignore` (store/vendor ignored,
`project.lock` committed). `--workspace` scaffolds a parent with a `members`
block and one child.

---

## 12. What we learned from prior systems

- **Cargo (Rust)** — gold standard UX: one tool, one manifest, one lockfile,
  integrated test/bench/doc, **workspaces** (the basis for our `members`). *We
  adopt all of this.* Weakness: maximal-version resolution causes churn;
  `build.rs` is an arbitrary-code-execution vector; feature unification can
  surprise. *We choose MVS, forbid build scripts, and make features additive
  only.*
- **Go modules** — MVS, `GOPROXY` + `sumdb` (content integrity + transparency),
  no central registry. *We adopt MVS, checksum DB / transparency log, and
  decentralization.* Weakness: clumsy `/v2` major-version paths, weak capability
  story. *We keep semver ranges and add capabilities.*
- **npm/yarn/pnpm/bun (JS)** — the cautionary tale: tool fragmentation, deep
  unsigned mutable graphs, `postinstall`, typosquatting, dependency confusion,
  left-pad. *We reject all of it and add `ext` curation.*
- **Nix / Guix** — content-addressed immutable store, hermetic reproducible
  builds, perfect dedup. *We adopt the store + reproducibility.* Weakness: steep
  UX. *We hide it behind boring `lamp` commands.*
- **Deno** — capability permissions, URL imports, decentralization, signing
  direction. *We adopt capabilities + decentralization* but choose a *declared,
  compiler-verified* capability manifest so refusal is at compile time.
- **Maven/Gradle (JVM)** — coordinates + checksums + GPG signing + binary
  artifacts (`.jar`); also multi-module reactor builds (kin to `members`). *We
  take binary artifacts (`.jnb`), checksums, signing, and multi-output builds.*
  Weakness: XML/Groovy sprawl, slow resolution, diamond pain, and the
  **shaded/fat-jar** anti-pattern that bundles private dep copies into each
  artifact (dedup death, unpatchable CVEs). *We use Jinn-native manifests + MVS +
  single-version unification, and binaries reference deduplicated store entries
  rather than bundling them (§5.6); bundling happens once at the application
  leaf, never per library.*
- **Sigstore / SLSA / in-toto / TUF** — keyless-ish signing, transparency logs,
  provenance, rebuild verification, **delegated trust + revocation**. *We adopt
  transparency logs, provenance, rebuild verification, and TUF-style roots to
  fix bare TOFU.*
- **Hackage/Stack, Elm, OPAM, Pub** — curated registries and "one canonical
  package" culture (the basis for `ext`). Elm's **compiler-computed semver
  bump** is the model for §10.4.

---

## 13. Implementation roadmap (from the prototype)

| Phase | Work | Touches |
|-------|------|---------|
| 1 | Split `lamp` driver from `jinnc`; unify the verb set (§11) | `src/driver/`, new `src/bin/lamp.rs` |
| 2 | Manifest v2 (`provides` multi-output, `members`, `capabilities`, `features`, sources, dev-deps, restricted sublanguage §2.1) | `src/pkg.rs` |
| 3 | Output/member DAG: name unification, cycle detection, per-output scoping (§2.3–2.4) | `src/pkg.rs`, new `src/workspace.rs` |
| 4 | Content-addressed store + canonical archive + BLAKE3 | `src/cache.rs`, new `src/store.rs` |
| 5 | Lockfile v2 → `project.lock` (content/sig/caps/feats/deps); diff driver | `src/lock.rs` |
| 6 | MVS resolver + conflict reporting; binary deps as ABI-pinned ranges, used-surface hashing, `requires-isolated` (§3.4, §5.6) | `src/cache.rs` resolve |
| 7 | Capability derivation from effects; resolver enforcement; FFI taint (§5.5) | typer/effects + `src/pkg.rs` |
| 8 | Signing, TUF roots + transparency log + revocation, provenance, SBOM | new `src/trust.rs` |
| 9 | `.jnb` writer/reader: `interface.jhi` (HIR slice), MIR/object pack, `deps.descriptor` (triples), used-surface hash, semver diff (§5.6, §10.4) | new `src/jnb/`, codegen, typer |
| 10 | Registry protocol (versioned) + `lamp serve`; git transport bridge | new `src/registry/` |
| 11 | Reproducible-build normalization + `--verify` | driver/codegen |
| 12 | Maintainer surface: bugs, changelog, notes, release/yank (per-output) | new `src/maint/` |
| 13 | Output two-axis model (§2.3.1): kind inference (`lib`/`bin`/`app`/`os`/`raw`) + substrate-dial inference (`runtimeless`/`threadless`/`heapless`) from used surface, layered `libjn` link selection, freestanding `for arch`/`for mcu` targets; derived `app` deploy contract (store-schema fingerprints, settings contract, caps, env reqs) | typer/effects, escape analysis, `src/pkg.rs`, `libjn` link layers |
| 14 | Deployments: `deployments` manifest block, `project.deploy.lock` record, deploy transaction (build/diff/migrate/gate/apply/rollback), `local`+`host` platforms, fleet/per-tenant rollout (§9) | new `src/deploy/`, `local`/`host` drivers, store-migration check (task 2-31-9) |
| 15 | `ext/` tier wiring, curation CI, `lamp nominate`, `lamp gc`, `lamp trust` | repo CI, `ext/` |

**Conformance tests** (Jinn style) per phase, plus end-to-end: a multi-output
project (`bin` + `lib` + a `members` child) builds reproducibly; one binary
references a sibling `lib` (standalone) and one references a member `lib`
(hierarchical); tampered-hash, capability-widening, signature-mismatch, revoked
-key, and FFI-taint rejection tests; a `.jnb` round-trip (build a lib, consume
it as binary and as binary-debug, confirm type-checking identical to source);
and the §5.6 transitive cases — two binaries sharing one deduplicated X (single
store entry, single lock entry); a minor X bump leaving the used-surface hash
stable so binaries link without rebuild; a signature-changing X bump forcing a
source rebuild of the binary; an irreducible incompatible-major conflict
producing a hard error, then resolving via `requires-isolated` with both copies
SBOM-reported; and an application final-link packing the unified closure into a
single deduplicated executable. Deployment cases: an `app` whose kind is
inferred from an opened `@store` (and a `bin`/`app` mismatch rejected); a deploy
to a fresh target creating its `project.deploy.lock` record; a re-deploy of the
same revision being a no-op (idempotent empty diff); a store-schema bump
planning a checked migration and being blocked when the migration path is
ill-typed; a settings change introducing a required default-less key blocking
until supplied; a capability widening surfaced as an explicit acknowledgement; a
failed migration rolling back to the snapshot and prior record; and a
per-tenant `for each` fleet rollout where one tenant's failure rolls that
tenant back while the rest proceed, reported precisely. Output two-axis cases: a
`bin` inferred `threadless` with the linked artifact confirmed free of
scheduler symbols; a `heapless` `raw` firmware that *errors* when a reachable
path allocates (`Vec.push`) and links no allocator when it does not; an `os`
image inferred freestanding from a boot entry and rejected when it makes a
hosted syscall; and a pinned dial equal to the inferred one warning only under
`--pedantic`.

---

## 14. Summary

lamp is **one tool**, **one manifest (`project.jn`)**, **one lockfile
(`project.lock`)**, and **one content-addressed, signed, federated store**. A
single project can **produce many outputs** described on two orthogonal axes —
a *kind* (`lib` a `.jnb` library, `bin` a run-and-exit executable, `app` a
long-lived deployable stateful application, `os` a freestanding bootable image,
`raw` bare-metal firmware) and a *substrate* (the `libjn` runtime layers it
assumes, opted out of via `runtimeless`/`threadless`/`heapless`) — both
inferred from the entry's surface so the output *determines its own packaging,
link set, and lifecycle* without ceremony. Outputs reference each other
either *standalone* (by content address, across repos) or *hierarchically*
(parent includes children, by name within the tree, never touching the
registry). It makes the safe, reproducible, auditable
path the *default* and the only ergonomic one. It introduces `ext` to
concentrate community effort into single canonical packages (with `libjn` kept
where it belongs — the libc-replacement runtime, not a packaging tier), a
`.jnb` binary format that integrates as cleanly as source while supporting
closed and debuggable distribution — and that handles its *own* transitive
dependencies by reference, not by bundling: a binary's deps live once in the
shared content-addressed store, fold into the one global MVS solution
(single-version unification holds across the binary boundary), and are linked
only when the consumer's selected version matches the used-surface interface
hash the binary was compiled against — degrading to a source rebuild, or a loud
`requires-isolated` decision, never a silent duplicate (§5.6). It adds a
capability system — with the FFI gap closed honestly — that makes a dependency's
powers a reviewable lockfile diff,
TUF-grade signing with a transparency log and revocation, interface-driven
semver, and a maintainer surface that lives in the same durable fabric as the
code. It carries that same fabric outward to runtime: `lamp deploy <app>
<target>` runs and updates application instances — environment ladders or
per-tenant fleets — as a single reproducible, idempotent transaction over a
content-addressed deployment record, with binary updates, compile-checked store
migrations, type-checked settings, and capability grants all surfaced as a
reviewable diff and rolled back atomically on failure. It learns from Cargo's UX and workspaces, Go's MVS and transparency,
Nix's store, Deno's capabilities, Maven's multi-module builds, Elm/Hackage
curation, and Sigstore/SLSA/TUF provenance — while refusing JS-land's tool
sprawl and install-time code execution. It is the package manager Jinn's ethos
demands: the compiler does the heavy lifting; the programmer expresses intent;
trust is explicit, verifiable, and boring.
