# Jade Roadmap

**Maintainer:** Rome
**Updated:** July 2026

---

## Current Status

| Metric | Value |
|--------|-------|
| Tests | 659 passing (85 unit + 376 bulk + 198 integration) |
| Source | ~14,000 LOC |
| Performance | J       ade = 0.72× Clang · 0.67× Rust · 84× Python across 15 benchmarks |
| Phase | 0.0.0-rc → 0.0.2 systems (in progress) |

### What works today

**Core:** i8–u64 integers, f32/f64 floats, booleans, void, strings (literal + interpolation + methods), fixed arrays, tuples, structs (with methods), enums (tagged unions), full pattern matching with destructuring, generics via `of` (monomorphized), error definitions with `!` return, augmented assignment (`+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`).

**Control flow:** if/elif/else (statement + expression), ternary `?`/`!`, while, for-in range with step, loop, break/continue with values, match, implicit return.

**Functions:** Higher-order, lambdas (`*fn`), closures (by-value capture), pipeline `~` with `$` placeholder, `do`...`end` blocks, default parameters, multiple return, recursion.

**Memory:** Perceus reference counting (9 optimization passes: drop elision, reuse analysis, borrow elision, last-use detection, FBIP, tail reuse, drop fusion, speculative reuse, scoped ownership), inferred RC allocation, weak references (explicit downgrade/upgrade), ownership verification with move/borrow tracking.

**Systems:** Extern functions (C FFI with variadic), syscall infrastructure (x86_64), inline assembly, raw pointers, type casting with `as`, volatile reads/writes, signal handling (POSIX), integer overflow control (wrapping/saturating/checked).

**Diagnostics:** Structured diagnostics with error codes (E001–E799, W001+), labeled spans, suggestions, multi-severity (error/warning/info).

**Debugging:** DWARF debug info via `--debug` / `-g` (compile unit, subprograms, source locations). Compatible with lldb/gdb.

**Codegen:** Integer literal coercion, call/return coercion, enum field coercion, recursive enum boxing (malloc-based indirection), LLVM function attributes (nounwind, nosync, nofree, mustprogress, willreturn, noundef), internal linkage, nsw/nuw flags, square-and-multiply integer exponentiation, zext for booleans, width-correct printf formats, if/else value propagation via phi nodes.

**Tooling:** VS Code extension (syntax highlighting), tree-sitter grammar, 4-language benchmark runner (Jade/C/Rust/Python) with historic tracking.

---

## Versioning

```
0.0.0   Genesis         Core language. Compiles, runs, matches C.
0.0.1   Functions       Closures, iterators, standard Option/Result.
0.0.2   Systems         Volatile, unsafe model, memory layout attrs.
0.1.0   Abstractions    Traits, actors, custom allocators, SIMD.
0.2.0   Persistence     Stores, transactions, query engine.
0.3.0   Concurrency     Async/await, channels, thread pools.
1.0.0   Stable          Backward-compatible. Production-ready.
```

---

## Design Decisions

### 1. Reference Counting: Inferred, Not Explicit

**Decision:** RC wrapping is inferred by the compiler. No explicit `rc` annotation.

**Rationale:** Jade's Perceus pass already analyzes the entire program's ownership flow. The compiler knows exactly when a value is shared (multiple live references) and when it's unique. Requiring the programmer to annotate `rc` is busywork — the compiler has strictly more information than the programmer in this domain.

**However:** An explicit `rc()` builtin exists for the rare case where the programmer needs to force RC wrapping for FFI interop or to share a value across closures where inference hasn't caught it. Think of it as a hint, not a requirement. The compiler will warn if an explicit `rc()` is applied to a value that was already going to be RC'd.

**Why not Rust's approach:** Rust's `Rc<T>` / `Arc<T>` is a library type because Rust can't infer sharing across function boundaries without whole-program analysis. Jade does whole-program analysis (monomorphized generics, full HIR pass), so it can infer.

### 2. Weak References: Explicit, With Warnings

**Decision:** Weak references are always explicit: `weak(rc_val)` to downgrade, `weak_upgrade(w)` to upgrade. The ownership verifier warns when weak refs are used directly without upgrading first.

**Rationale:** Cycles are a design decision. The programmer must decide which edge breaks the cycle — auto-detecting this is undecidable in general. However, the compiler will:
- Detect potential reference cycles in the type graph at compile time
- Warn when weak refs are accessed without `weak_upgrade()` + nil check
- Track weak ref liveness through the ownership verifier

**Implementation:** `Type::Weak(Box<Type>)`. Weak layout extends the RC allocation: `{ strong_count: i64, weak_count: i64, data: T }`. When strong count hits zero, data is dropped but the allocation survives until weak count also hits zero. `weak_upgrade()` returns the RC or none (nil pointer) if the strong count is zero.

### 3. Perceus: Champion's Inventory

Nine optimization passes, all wired to codegen:

| Pass | Effect | Codegen Impact |
|------|--------|----------------|
| **Drop elision** | Removes drops for last-use values | Skips `free()` call entirely |
| **Reuse analysis** | Identifies allocations that can reuse a recently-freed object | Codegen checks `reuse_candidates` in Drop handler, skips free |
| **Borrow elision** | Promotes borrows to moves when original isn't used after | Codegen checks `borrow_to_move`, skips RC increment |
| **Last-use detection** | Tracks final use of every binding | Enables drop elision |
| **FBIP** (Functional-But-In-Place) | Detects `new(field: old.field)` patterns | Enables in-place update without alloc |
| **Tail reuse** | Reuse in tail-call position | Same-size allocation recycling |
| **Drop fusion** | Merges adjacent drops | Single free for multiple dead values |
| **Speculative reuse** | Heuristic reuse for hot paths | Reduces allocator pressure |
| **Scoped ownership** | Tracks ownership epochs per scope | Enables scope-exit batch drops |

**Stats tracked:** drops_elided, reuse_sites, borrows_promoted, fbip_sites, tail_reuse_sites, speculative_reuse_sites, drops_fused, last_use_tracked, total_bindings_analyzed. Printed to stderr when nonzero.

### 4. String Type: SSO Design

**Current:** `{ ptr: *u8, len: i64, cap: i64 }` = 24 bytes. Literals point to globals (cap=0, no heap alloc). Concat/slice malloc.

**SSO Design (next step):**
- Same 24-byte struct, two modes:
  - **Heap:** `{ ptr, len, cap }` with high bit of cap = 1
  - **Inline:** `{ data: [u8; 23], tag }` where tag = length (0–23, high bit = 0)
- Every string access branches on the tag bit
- Eliminates malloc for strings ≤ 23 bytes (most strings in practice)
- `to_string()` on small ints, short identifiers, etc. → zero heap alloc

**Stack opportunities identified:**
- String literals: already zero-alloc (global constant pointer) ✅
- Short computed strings: can use alloca instead of malloc when they don't escape
- Format buffers for interpolation: can alloca for known-bounded results
- String method results (contains, starts_with): no allocation ✅

### 5. Debug Info: DWARF via lldb/gdb

**Decision:** Use DWARF debug info, leverage lldb/gdb. No custom debugger.

**Rationale:** DWARF is the universal standard. lldb and gdb are battle-tested with decades of development. Building a custom debugger would be a massive investment with no benefit over DWARF — the investment should go into making DWARF output excellent instead.

**Implementation:**
- `--debug` / `-g` CLI flag enables DWARF emission
- `DebugInfoBuilder` creates `DICompileUnit` with `DWARFSourceLanguage::C` (closest ABI match)
- Per-function `DISubprogram` with source location
- `set_debug_location(line, col)` on each instruction
- `-g` passed through to linker
- Compatible with `lldb ./binary`, `gdb ./binary`, VS Code debug extension

### 6. No Unsafe Blocks — Jade is Sharp

**Decision:** No `unsafe { }` blocks. All operations are available everywhere.

**Rationale:** Jade's philosophy is "sharp tools for sharp programmers." The language doesn't pretend dangerous operations don't exist by gating them behind special syntax. Instead:
- Volatile reads/writes are builtins: `volatile_load(ptr)`, `volatile_store(ptr, val)`
- Raw pointer arithmetic is always available
- FFI calls are always available
- Signal handling is always available

**Safety model:** The compiler warns (not errors) when it detects potentially dangerous patterns:
- Using a weak ref without upgrading first → warning
- Returning a reference to a local → error
- Use after move → error
- Double mutable borrow → error

The distinction: ownership violations are errors (they're bugs). Hardware-level operations are available without ceremony (they're tools).

### 7. Memory Layout Annotations

**Current spec:** `@layout is STRICT`, `@layout is PACKED`, `@align is N`

**Improved syntax:**
```jade
type CStruct @strict      # C-compatible field ordering
    magic: u32
    flags: u16

type Pixel @packed         # No padding between fields
    r: u8
    g: u8
    b: u8

type CacheLine @align(64)  # Force alignment
    data: [u8; 64]

type NetPacket @packed @strict @align(4)   # Combinable
    header: u32
    payload: [u8; 1024]
```

The `@strict` / `@packed` / `@align(N)` form is shorter, consistent with existing `@` decorator syntax, and composable. Replaces the verbose `@layout is STRICT` form.

### 8. Integer Overflow Control

**Default:** Trap on overflow (nsw/nuw flags → undefined behavior in release, trap in debug).

**Explicit control via builtins:**
| Builtin | Behavior | LLVM |
|---------|----------|------|
| `wrapping_add(a, b)` | Wraps at bit width | Plain `add` (no nsw/nuw) |
| `wrapping_sub(a, b)` | Wraps at bit width | Plain `sub` |
| `wrapping_mul(a, b)` | Wraps at bit width | Plain `mul` |
| `saturating_add(a, b)` | Clamps to min/max | `llvm.sadd.sat` / `llvm.uadd.sat` |
| `saturating_sub(a, b)` | Clamps to min/max | `llvm.ssub.sat` / `llvm.usub.sat` |
| `saturating_mul(a, b)` | Overflow check + clamp | `llvm.smul.with.overflow` + select |
| `checked_add(a, b)` | Returns `(result, overflowed)` | `llvm.sadd.with.overflow` |
| `checked_sub(a, b)` | Returns `(result, overflowed)` | `llvm.ssub.with.overflow` |
| `checked_mul(a, b)` | Returns `(result, overflowed)` | `llvm.smul.with.overflow` |

User-defined functions with the same name take priority over builtins.

### 9. Signal Handling

**Builtins:**
- `signal_handle(signum, handler_fn)` — Register a signal handler (wraps POSIX `signal(2)`)
- `signal_raise(signum)` → i32 — Raise a signal (wraps `raise(3)`)
- `signal_ignore(signum)` — Ignore a signal (sets handler to `SIG_IGN`)

**Future:** Consider `sigaction(2)` wrapper for more control (sa_mask, SA_RESTART, etc.).

---

## 0.0.0-rc: Remaining

- [x] Exhaustive pattern matching with diagnostics
- [ ] Heap-allocated String type with SSO *(design complete, implementation pending)*
- [x] Reference counting (`rc` wrapper, precise retain/release) — inferred
- [x] Weak references (`weak` type, upgrade/downgrade)
- [x] Perceus optimizations — 9 passes, all wired to codegen
- [ ] Separate compilation (per-module LLVM IR, linking)
- [x] Structured diagnostics (error codes, spans, suggestions)

## 0.0.1: Functions — Remaining

- [ ] Protocol-based iterators, for-loop desugaring over iterables
- [ ] Standard `Option` and `Result` types
- [ ] Standard collections (Vec, Map)

## 0.0.2: Systems — complete

- [x] Volatile reads/writes — `volatile_load()`, `volatile_store()` builtins
- [x] Sharp safety model — no `unsafe` blocks, warnings for dangerous patterns
- [ ] Memory layout: `@strict`, `@packed`, `@align(N)` *(syntax designed, parser pending)*
- [x] Integer overflow control (wrapping, saturating, checked)
- [x] Signal handling (handle, raise, ignore)
- [x] DWARF debug info (`--debug` / `-g`)

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

15-benchmark suite. Jade vs C (gcc -O3) vs Rust (rustc -C opt-level=3) vs Python 3.

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
