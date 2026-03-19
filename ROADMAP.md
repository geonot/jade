# Jade Roadmap

**Maintainer:** Rome
**Updated:** July 2026

---

## Current Status

| Metric | Value |
|--------|-------|
| Tests | 639 passing (85 unit + 376 bulk + 178 integration) |
| Source | 12,484 LOC (codegen 3,888 · typer 2,504 · parser 1,686 · perceus 1,611 · lexer 984 · ownership 675 · hir 421 · ast 327 · main 198 · types 106 · diagnostic 72) |
| Tests LOC | 3,930 (bulk 2,387 · integration 1,543) |
| Performance | Jade = 0.72× Clang · 0.67× Rust · 84× Python across 15 benchmarks |
| Phase | 0.0.0-perceus (Phases A–D complete) |

### What works today

**Core:** i8–u64 integers, f32/f64 floats, booleans, void, strings (literal + interpolation + methods), fixed arrays, tuples, structs (with methods), enums (tagged unions), full pattern matching with destructuring, generics via `of` (monomorphized), error definitions with `!` return, augmented assignment (`+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`).

**Control flow:** if/elif/else (statement + expression), ternary `?`/`!`, while, for-in range with step, loop, break/continue with values, match, implicit return.

**Functions:** Higher-order, lambdas (`*fn`), closures (by-value capture), pipeline `~` with `$` placeholder, `do`...`end` blocks, default parameters, multiple return, recursion.

**Systems:** Extern functions (C FFI with variadic), syscall infrastructure (x86_64), inline assembly, raw pointers, type casting with `as`.

**Codegen:** Integer literal coercion, call/return coercion, enum field coercion, recursive enum boxing (malloc-based indirection), LLVM function attributes (nounwind, nosync, nofree, mustprogress, willreturn, noundef), internal linkage, nsw/nuw flags, square-and-multiply integer exponentiation, zext for booleans, width-correct printf formats, if/else value propagation via phi nodes.

**Tooling:** VS Code extension (syntax highlighting), tree-sitter grammar, 4-language benchmark runner (Jade/C/Rust/Python) with historic tracking.

---

## Versioning

```
0.0.0   Genesis         Core language. Compiles, runs, matches C.
0.0.1   Functions       Closures, iterators, standard Option/Result.
0.0.2   Systems         Volatile, unsafe blocks, memory layout attrs.
0.1.0   Abstractions    Traits, actors, custom allocators, SIMD.
0.2.0   Persistence     Stores, transactions, query engine.
0.3.0   Concurrency     Async/await, channels, thread pools.
1.0.0   Stable          Backward-compatible. Production-ready.
```

---

## 0.0.0-rc: Remaining

- [ ] Exhaustive pattern matching with diagnostics
- [ ] Heap-allocated String type with SSO and interpolation
- [ ] Reference counting (`rc` wrapper, precise retain/release)
- [ ] Weak references (`weak` type, upgrade/downgrade)
- [ ] Perceus optimizations (borrow elision, drop specialization, reuse)
- [ ] Separate compilation (per-module LLVM IR, linking)
- [ ] Structured diagnostics (error codes, spans, suggestions)

## 0.0.1: Functions — Remaining

- [ ] Protocol-based iterators, for-loop desugaring over iterables
- [ ] Standard `Option` and `Result` types
- [ ] Standard collections (Vec, Map)

## 0.0.2: Systems — Remaining

- [ ] Volatile reads/writes
- [ ] `unsafe` block full semantics
- [ ] Memory layout: `@repr(c)`, `@packed`, `@align(N)`
- [ ] Integer overflow control (wrapping, saturating, checked)
- [ ] Signal handling

---

## Phase 1: Abstractions (0.1.0)

- [ ] Traits: definition, impl, method dispatch, bounds
- [ ] Operator overloading via traits (Add, Mul, Eq, Ord, Display)
- [ ] Dynamic dispatch: `dyn Trait` (opt-in)
- [ ] Actors: `@` handlers, bounded mailboxes, supervision
- [ ] Custom allocators (arena, pool, bump)
- [ ] SIMD intrinsics
- [ ] Thread spawning

## Phase 2: Persistence & Async (0.2.0–0.3.0)

- [ ] Persistent stores, WAL, ACID transactions, queries
- [ ] Async/await (stackless coroutines via LLVM)
- [ ] Typed channels, work-stealing thread pool
- [ ] Atomic types, lock-free data structures
- [ ] Async I/O (io_uring / kqueue)

## Phase 3: Ecosystem (0.4.0+)

- [ ] LSP (hover, goto-def, completion, diagnostics)
- [ ] Formatter, package manager, doc generator
- [ ] Standard library (collections, I/O, networking, HTTP, JSON, crypto, time)
- [ ] DWARF debug info
- [ ] Cross-compilation (x86_64, aarch64, RISC-V, WASM)
- [ ] PGO, incremental/parallel compilation
- [ ] Self-hosted compiler

## 1.0.0: Stability

- [ ] Language spec finalized
- [ ] Backward compatibility guarantee
- [ ] Edition system
- [ ] Security audit
- [ ] Three production applications as proof points

---

## Benchmarks

12-benchmark suite. Jade vs C (gcc -O3) vs Rust (rustc -C opt-level=3) vs Python 3.

```
python3 run_benchmarks.py --opt=3 --runs=5 --save=<tag>
python3 run_benchmarks.py --opt=all                        # O0–O3 comparison
python3 run_benchmarks.py --langs=jade,rust                # specific languages
```

Benchmark gate: **Jade total ≤ 1.1× of Clang -O3 across full suite.** ✅ Achieved.

Historic results stored in `benchmarks/history.json`. Current results in `benchmarks/results.json`.

---

## Research Foundation

| Paper | Relevance |
|-------|-----------|
| Perceus (Reinking et al., 2021) | Core memory management |
| Counting Immutable Beans (Ullrich & de Moura, 2020) | RC optimization |
| Frame-Limited Reuse (Lorenzen & Leijen, 2023) | Bounded memory reuse |
| Complete & Easy Bidirectional Typechecking (Dunfield, 2021) | Type inference |
| Algebraic Subtyping (Dolan & Mycroft, 2017) | Future trait hierarchies |
| Mojo (Lattner et al., 2023) | Ownership without lifetimes |
| Koka Effect System (Leijen, 2023) | Future effect typing |
| LLVM (Lattner & Adve, 2004) | Backend |
| Rust (Matsakis & Klock, 2014) | Ownership model |
| Swift ARC (Apple, 2014) | Automatic RC |

---

*Start simple. Build up. Never stop.*
