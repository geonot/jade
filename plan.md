## Phase 0.2.0 — Wire the Engine (Critical)

| Priority | Item | Design Decision | Effort |
|----------|------|-----------------|--------|
| P0 | Wire Perceus to codegen | Emit free() / rc_release() calls from Stmt::Drop. Use elide_drops to skip trivial drops. Use reuse_candidates to replace malloc with reuse. | Large |
| P0 | Exhaustiveness checking | Add check_exhaustive(pats: &[Pat], ty: &Type) -> Result<(), Vec<Pat>> in typer. Return missing patterns as errors. | Medium |
| P1 | Validate HIR | Add a pass between typer and codegen: check DefId uniqueness, type consistency, scope validity. Catch bugs before they become LLVM errors. | Medium |
| P1 | Split typer | Extract name resolution into its own pass. Move monomorphization to a separate module. Reduce Typer struct to <10 fields. | Large |

## Phase 0.3.0 — Iterator Protocol & Generics

| Priority | Item | Design Decision | Effort |
|----------|------|-----------------|--------|
| P0 | Iterator trait | trait Iter of T with *next() -> Option of T. Desugar for x in coll to loop { match coll.next() ... }. | Medium |
| P0 | Trait bounds on generics | *max of T: Ord(a: T, b: T) -> T. Reject monomorphization if bound not satisfied. | Medium |
| P1 | Associated types | trait Iter with type Item. Needed for map/filter/zip chains. | Medium |
| P2 | String iteration | for ch in str iterates codepoints. Needs UTF-8 decoder. | Small |

## Phase 0.4.0 — Concurrency Done Right

| Priority | Item | Design Decision | Effort |
|----------|------|-----------------|--------|
| P0 | Stackful coroutines | Replace pthread-based dispatch with setjmp/longjmp or platform-specific context switch. 8KB initial stacks, growable. | Large |
| P0 | Typed channels | channel of T with send/receive. Bounded (ring buffer) or unbounded (linked list). | Medium |
| P1 | M:N scheduler | Thread pool with work-stealing deques. Coroutines suspended on channel ops yield to scheduler. | Large |
| P1 | Migrate actors to scheduler | Replace per-actor pthreads with coroutines on the scheduler. Same API, 1000× less overhead. | Medium |
| P2 | Select | select over multiple channels. Compile to a multi-wait on the scheduler. | Medium |

## Phase 0.5.0 — Ecosystem

| Priority | Item | Design Decision | Effort |
|----------|------|-----------------|--------|
| P0 | Package manager | File-based, git-backed. use pkg.module fetches from registry. Lockfile for reproducibility. | Large |
| P0 | Standard library | std.io, std.os, std.math, std.fmt. Written in Jade + extern FFI. | Large |
| P1 | LSP | Reuse typer for diagnostics. Document symbols, go-to-definition, hover types. | Large |
| P2 | Compile-time evaluation | comptime blocks evaluated during compilation. Enables config, code generation, static assertions. | Large |