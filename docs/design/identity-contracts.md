# Identity contracts

Status: normative. This document supersedes "The Merkle algorithm, pinned" in
`compiler-prereqs.md`. The SCC algorithm there iterated BLAKE3 over placeholder
hashes "until stable" and claimed the iteration was monotone and finite. It is
neither. Digest iteration has no lattice order. A finite state space only
guarantees that values eventually repeat, not that they reach a fixed point.
The replacement below hashes each node once, over a finite canonical
serialization, so it terminates by construction.

Implementation: `src/identity.rs` (`IdentityGraph`, `Rung`, `Edge`).

## Rungs

A digest is always computed for one rung. The rung is part of the BLAKE3
derive-key context, so digests from different rungs can never collide by
construction.

| Rung | Context | Determinants | Meaning of equality |
| --- | --- | --- | --- |
| `Nominal` | `jinn:identity:v1:nominal` | fully qualified name, owner scope, package identity | the same declared entity |
| `Api` | `jinn:identity:v1:api` | public names, signatures, access categories, error/capability rows | source-compatible surface |
| `Abi { target }` | `jinn:identity:v1:abi` + target triple | target layout (TargetData), field order, Perceus ownership/drop protocol, tag/niche layout, calling convention, error/capability rows | link-compatible on that target |
| `Object { target }` | `jinn:identity:v1:object` + target triple | everything in `Abi` plus the lowered bodies | interchangeable object code |

Target-dependent rungs must name a non-empty target. An empty target is an
error, not a wildcard.

Equal `Abi` does not mean equal behaviour. Two implementations with equal ABI
digests and equal capability purity must still not be merged. Only `Object`
equality licenses deduplication. Function addresses are not observable in
supported Jinn: the language has no function-to-integer conversion, and
pointer-to-integer casts in codegen are confined to runtime-internal boxing.
A function passed across FFI exposes its address to foreign code, so exported
or FFI-escaping functions are excluded from object deduplication. Any future
operation that exposes them must either exclude the
affected functions from object deduplication or be rejected.

## Graph model

The hashed closure is a directed graph:

- Each node carries a **label**. The label is the byte serialization of that
  node's own determinants for the rung. For identity-bearing nominal nodes
  (structs, enums, errors, actors, stores) the label includes the fully
  qualified name. Two different nominal types therefore never share a digest,
  even if their structure is identical.
- Each node carries an **ordered** list of out-edges. Order is semantic: field
  order, variant order and parameter order. Reordering edges changes the digest.
- An edge either targets a node in the same graph (`Edge::Node`) or carries an
  already-known digest of an external entity, such as a dependency package
  (`Edge::External`).

## Algorithm

1. Condense the graph into SCCs with an iterative Tarjan. The traversal uses an
   explicit stack, so depth is bounded by heap, not by the call stack. Tarjan
   emits SCCs in reverse topological order, so every successor SCC is complete
   before its predecessors.
2. For each node `r` in an SCC, compute its **rooted serialization**:
   - Assign local indices by breadth-first traversal from `r`, following
     out-edges in their declared order and restricted to the SCC. `r` gets
     index 0. The indices depend only on graph structure and edge order, never
     on insertion order, pointers or hash-map iteration.
   - Emit: rung context, then the length-prefixed target if the rung has one,
     then the node count. For each node in local-index order emit the
     length-prefixed label, the edge count, and each edge. An internal edge is
     tag `0` followed by the target's local index. An edge to an
     already-hashed SCC or an external digest is tag `1` followed by the
     32-byte digest.
   - The digest of `r` is BLAKE3 of that byte stream, computed once.
3. All integers are `u64` little-endian. All variable-length fields carry a
   length prefix, so the stream is prefix-free and label boundaries are
   unambiguous.

Every step is a single pass over finite data. Per node the cost is linear in
the size of its SCC, so the worst case is quadratic in the largest SCC. That is
acceptable: type SCCs are small in practice, and the bound is explicit.

## Properties (pinned by `identity::tests`)

- **Insertion independence.** Building the same logical graph in any node or
  insertion order yields the same per-node digests.
- **Process independence.** A pinned test vector reproduces the byte stream by
  hand. Nothing depends on addresses, `HashMap` order or randomized state.
- **Symmetry.** Nodes whose rooted serializations are isomorphic get equal
  digests. For example, the two nodes of a cycle `T -> T' -> T` with equal
  labels share a digest. This is correct. Such nodes are indistinguishable at
  this rung, and nominal distinction is expressed in labels, not recovered by
  tie-breaking.
- **Cycle shape.** A self-loop and a two-cycle with equal labels differ,
  because the node count and local indices are serialized.
- **Propagation.** Changing any reachable determinant, including one inside a
  cycle or behind it, changes every node that reaches it.
- **Compositionality.** An edge to an already-hashed node produces the same
  digest as an `External` edge carrying that node's digest. Packages can
  therefore be hashed separately and composed.
- **Fail-closed.** Dangling edges and empty targets are errors.

## Package `semantic_hash`

`pkgid::compute_semantic_hash` remains a source fingerprint (`v1`). It is not
the typed ABI digest. Completing its input manifest is task `17-3-3`.
Populating `IdentityGraph` from resolved HIR/MIR types for the `api`/`abi`/
`object` ladder in `interface.jhi` v2 is task `2-32-4`. That task must use this
module and must not define a second hashing scheme.

## Source manifests and package keys

Implementation: `src/source_manifest.rs`, `src/pkgid.rs`.

- A package's source input is a `SourceManifest`. The scan recurses through the
  package directory and skips dot-entries and `target`. Each `.jn` file is
  recorded by its UTF-8 relative path with `/` separators, its byte length, and
  the BLAKE3 hash of its contents. Entries are sorted by path. The digest
  (`jinn:source-manifest:v1`) length-prefixes every field, so moving bytes
  between files, renaming a file or editing a nested file all change the key.
- The scan fails closed. Symbolic links, unreadable directories or files,
  non-UTF-8 or control-character paths, and more than 65,536 sources are
  errors. They are never silently skipped or truncated.
- `compute_semantic_hash` (`jinn:pkgid:semantic_hash:v2`) takes the manifest
  digest rather than concatenated bytes. For the root package, the manifest
  covers exactly the resolved `use` closure.
- The package interner key is the full record: name, owner scope, version,
  semantic hash and visibility. Interning is per compiler process (a
  thread-local table) and is never persisted. Two records that differ in any
  field are distinct packages.
- Package symbol prefixes are the first 128 bits of the semantic hash (32 hex
  digits). Before linking, the driver checks the prefixes of every resolved
  package and rejects any collision between distinct packages with a
  diagnostic that names both. Collisions are not resolved by arbitration.
- The `jinn run` cache key is BLAKE3 over the canonical entry path, the
  compiler version and fingerprint, target, CPU, features, standalone mode, the
  entry source and the manifest of the entry's directory tree. The `.deps`
  sidecar is a `jinn-sources v1` manifest of the files the compiler actually
  resolved. A cached binary is reused only if that manifest parses, is
  non-empty and still matches on disk. If the tree cannot be scanned, the
  cache is bypassed.
