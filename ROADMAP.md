# Jade Roadmap

**Maintainer:** Rome
**Updated:** March 2026

---

## Current Status

| Metric | Value |
|--------|-------|
| Tests | 798 passing (91 unit + 478 bulk + 229 integration) |
| Source | ~15K LOC (src) · ~6K LOC (tests) |
| Performance | Jade = 0.94× Clang -O3 across 19 benchmarks (aggregate median) |
| Phase | 0.1.0 — Abstractions |

### What works today

**Core:** i8–u64 integers, f32/f64 floats, booleans, void, strings (literal + interpolation + methods + SSO), fixed arrays, tuples, structs (with methods), enums (tagged unions), full pattern matching with destructuring, guards (`when`), or-patterns (`or`), and range patterns (`to`), generics via `of` (monomorphized), error definitions with `!` return, augmented assignment.

**Control flow:** if/elif/else (statement + expression), ternary `?`/`!`, while, for-from range with step, for..in (arrays + Vec), loop, yield, break/continue, match with guards, implicit return.

**Functions:** Higher-order, lambdas (`*fn`), closures (by-value capture), pipeline `~` with `$` placeholder, default parameters, multiple return, recursion. Parentheses optional on definitions and calls. Pattern-directed function clauses with literal parameters. Inline body syntax.

**Types & Methods:** Struct methods (inline in type block), extension methods (`impl Type`), trait methods (`impl Trait for Type`). Operator overloading (eq/neq/add/sub/mul/div/lt/gt/le/ge via trait dispatch, Display via `to_string`). Static dispatch via name-mangling. Dynamic dispatch via `dyn Trait` (fat pointer + vtable).

**Collections:** `Vec of T` (growable array, `{ptr, len, cap}`, push/pop/get/set/remove/clear, for..in iteration). `Map of K, V` (open-addressing hash map, FNV-1a, set/get/has/remove/len/clear).

**Strings:** SSO (24-byte inline ≤23 chars), concat, interpolation, contains, starts_with, ends_with, char_at, slice, length, find, trim/trim_left/trim_right, to_upper/to_lower, replace, split, equality (`equals`/`isnt`).

**Memory:** Perceus reference counting (9 optimization passes), inferred RC allocation, weak references, ownership verification with move/borrow tracking.

**Systems:** Extern functions (C FFI with variadic), syscall infrastructure (x86_64), inline assembly, raw pointers, type casting with `as`, volatile reads/writes, signal handling (POSIX), integer overflow control (wrapping/saturating/checked).

**Persistence:** Typed stores (`store` keyword), insert/delete/count/all/where/set operations, transactions, AND/OR compound filters, flat binary file format.

**Diagnostics:** Structured diagnostics with error codes, labeled spans, suggestions.

**Actors:** Erlang-style actors with bounded MPSC mailbox (ring buffer), pthread-based threads, `spawn`/`send`/`dispatch`/`receive` primitives, backpressure on full mailbox.

**Debugging:** DWARF debug info (`--debug` / `-g`), compatible with lldb/gdb.

**Tooling:** VS Code extension, tree-sitter grammar, 4-language benchmark runner with historic tracking. `--emit-hir` (structured HIR dump), `--emit-ir` (unoptimized LLVM IR), `--emit-llvm` (optimized LLVM IR).

---

## Language Assessment

### What's solid (no work needed)

- **Type inference** — Full HM + bidirectional. Functions, variables, return types all inferred. Zero type annotations required for typical programs.
- **Codegen** — Matches or beats Clang -O3 across most benchmarks (0.94× aggregate). Integer coercion, phi-node propagation, function attributes all wired.
- **Perceus RC** — 9 optimization passes, drop elision, reuse analysis, speculative reuse. No GC. No cycles.
- **Pattern matching** — Exhaustiveness checking, enum destructuring, literal patterns, guards. Fully wired through codegen.
- **FFI** — Extern declarations, variadic calls, string-to-ptr coercion, mixed-width integer passing.
- **Syntax** — Pipeline `~`, ternary `?`/`!`, `is` bindings, paren-optional calls, indentation structure. Clean and stable.

### What needs work before 0.1.0

All items completed. Moving to 0.2.0/0.3.0 targets.

| Gap | Impact | Status |
|-----|--------|--------|
| `Option`/`Result` unboxing | Fieldless → tag-only, nullable pointer optimization | **Done** |
| Array/tuple destructuring | Destructuring in match patterns | **Done** |
| Dynamic dispatch (`dyn Trait`) | Fat pointer + vtable thunks | **Done** |
| Iterator protocol | Protocol-based iteration for custom types | Planned |

### What we're NOT doing yet

- Custom allocators, SIMD — Phase 1 (0.1.0)
- Async/concurrency — Phase 2 (0.3.0), design below
- LSP, package manager, standard library — Phase 3 (0.4.0+)

---

## Versioning

```
0.0.0   Genesis         Core language. Compiles, runs, matches C.
0.0.1   Collections     Vec, Map, iterators, Option/Result zero-cost.
0.1.0   Abstractions    Traits, operator overloading, extension methods.
0.2.0   Persistence     Stores, transactions, query engine.
0.3.0   Concurrency     Structured concurrency, channels, coroutines.
1.0.0   Stable          Backward-compatible. Production-ready.
```

---

## 0.0.0-rc: Completed

- [x] Pattern matching with guards (`when` clause)
- [x] Extension methods (`impl Type`)
- [x] Remove defer (confusing semantics)
- [x] Exhaustive pattern matching with diagnostics
- [x] Reference counting (inferred, Perceus)
- [x] Weak references
- [x] Structured diagnostics
- [x] Or-patterns (`A or B ? ...`) — parser + typer + codegen
- [x] Range patterns (`1 to 10 ? ...`) — parser + typer + codegen

## 0.0.1: Collections & Iteration — Completed

- [x] `Vec of T` — Growable array (heap-allocated, `{ptr, len, cap}`, push/pop/get/set/remove/clear)
- [x] `Map of K, V` — Hash map (open addressing, FNV-1a, set/get/has/remove/len/clear)
- [x] `for..in` iteration over arrays and Vec
- [x] String methods (find, trim, trim_left, trim_right, replace, to_upper, to_lower, split)
- [x] String equality (`equals`/`isnt`) — memcmp with length pre-check
- [x] Struct operator overloading (eq/neq via `TypeName_eq` trait dispatch)
- [x] `dispatch` keyword (alias for `send` in actor system)
- [x] Array/tuple destructuring in patterns
- [x] `Option`/`Result` zero-cost representation (see design below)

## 0.1.0: Abstractions

- [x] Traits: definition, impl, method dispatch
- [x] Operator overloading via traits (Eq)
- [x] Additional operator overloading (Add, Mul, Ord, Display)
- [x] Dynamic dispatch: `dyn Trait` (opt-in)

## 0.2.0: Persistence (mostly done)

- [x] Persistent stores, transactions, query engine
- [ ] WAL, ACID guarantees for transactions
- [ ] Query optimization

## 0.3.0: Concurrency (see design below)

- [ ] Goroutine-style coroutines with structured scoping
- [ ] Typed channels (bounded, unbounded)
- [ ] Select over channels
- [ ] Thread pool with work-stealing
- [ ] Async I/O (io_uring / kqueue)

---

## Design Decisions

### 1. Reference Counting: Inferred, Not Explicit

RC wrapping is inferred by the compiler. No explicit `rc` annotation. Jade does whole-program analysis (monomorphized generics, full HIR pass), so it can infer sharing across function boundaries. An explicit `rc()` builtin exists as a hint for FFI interop.

### 2. Weak References: Explicit, With Warnings

`weak(rc_val)` to downgrade, `weak_upgrade(w)` to upgrade. The ownership verifier warns when weak refs are used without upgrading first. Cycles are a design decision — the programmer must decide which edge breaks the cycle.

### 3. Perceus: Nine Passes

| Pass | Effect |
|------|--------|
| Drop elision | Removes drops for last-use values |
| Reuse analysis | Reuses recently-freed objects |
| Borrow elision | Promotes borrows to moves when original isn't used after |
| Last-use detection | Tracks final use of every binding |
| FBIP | Detects `new(field: old.field)` patterns for in-place update |
| Tail reuse | Same-size allocation recycling in tail position |
| Drop fusion | Merges adjacent drops |
| Speculative reuse | Heuristic reuse for hot paths |
| Scoped ownership | Scope-exit batch drops |

### 4. String SSO

24-byte struct, two modes: heap `{ptr, len, cap}` with high bit of byte 23 = 0, inline `{data: [u8; 23], tag}` with high bit = 1 and length in low 7 bits. Eliminates malloc for strings ≤ 23 bytes.

### 5. No Unsafe Blocks — Sharp Tools

No `unsafe { }` gating. Volatile, raw pointers, FFI, signals all available everywhere. Ownership violations are errors. Hardware-level operations are tools.

### 6. Option/Result: Zero-Cost Without Boxing

**Problem:** `Option of i64` is currently an enum, which means tag + payload in the same struct. For small types (i64, bool, pointers), this wastes space and adds branch overhead for wrapping/unwrapping.

**Design:** The compiler should optimize known-representation enums:

| Pattern | Optimization | Representation |
|---------|-------------|----------------|
| `Option of T` where T is pointer-like | Nullable pointer — null = Nothing | `T` (same size) |
| `Option of i64` | Tagged representation — use a sentinel or extra byte | `{i64, i8}` (9 bytes) |
| `Result of T, E` | Tagged union (current behavior) | Already optimal |
| Fieldless enum (`enum Dir: N, S, E, W`) | Integer tag only | `i32` |

The key insight: the compiler already knows the full type of every `Option` and `Result` instantiation at monomorphization time. It can pick the optimal representation per-instantiation without the programmer thinking about it. No boxing, no heap allocation, no casting — the type system handles it.

**Implementation path:** Add a `layout_enum` pass in codegen that inspects monomorphized enum types and selects compact layouts. Pattern match codegen already uses tag-based dispatch — just needs to handle the nullable-pointer and sentinel cases.

### 7. Concurrency: Structured Coroutines + Channels

**Philosophy:** No async/await. No callback hell. No colored functions. Instead: goroutine-style lightweight coroutines with structured lifetimes, communicating via typed channels.

**Why not async/await:**
- Colors every function in the call chain (viral annotation)
- Requires a runtime with an event loop
- Splits the ecosystem into sync/async halves
- Complex cancellation semantics

**Why not raw threads:**
- Too heavy for fine-grained concurrency (1 OS thread per task = bad)
- No structured lifetime — fire-and-forget leads to resource leaks

**The Jade model:**

```jade
# Spawn a coroutine — like Go's goroutines but with structured scope
*main
    ch is channel of i64

    go
        # This runs concurrently
        for i from 0 to 10
            ch <- i      # send to channel
        close ch

    # Receive from channel
    for val in ch
        log val

    # Channel is closed, loop ends
```

**Core primitives:**

| Primitive | Syntax | Semantics |
|-----------|--------|-----------|
| `go` block | `go\n    body` | Spawn a coroutine. Structured — parent waits for all children at scope exit. |
| `channel of T` | `ch is channel of i64` | Typed, bounded (default cap=0 = synchronous). `channel of i64, 16` for buffered. |
| Send | `ch <- value` | Blocks if channel full. |
| Receive | `val is <- ch` | Blocks if channel empty. Returns `Nothing` when closed. |
| Select | `select\n    <- ch1 ? ...\n    <- ch2 ? ...` | Wait on multiple channels, run first ready arm. Like Go's `select`. |
| Close | `close ch` | Signal no more sends. Receivers get `Nothing` after drain. |

**Implementation strategy:**
1. **Stackful coroutines** — Each `go` block gets its own small stack (8KB default, growable). Context switch is a register save/restore (~20ns). No LLVM coroutine intrinsics needed — just `setjmp`/`longjmp` style switching.
2. **M:N scheduling** — M coroutines on N OS threads. Work-stealing scheduler. Coroutines that block on channels yield to the scheduler.
3. **Structured concurrency** — A `go` block's lifetime is bounded by its enclosing scope. When the scope exits, it waits for all spawned coroutines to complete (or cancels them). No orphaned goroutines.
4. **Async I/O under the hood** — File/network operations suspend the coroutine and register with io_uring (Linux) / kqueue (macOS). The programmer writes sequential code; the runtime handles non-blocking I/O transparently.

**Key difference from Go:** Structured scoping. Go's goroutines are fire-and-forget, which leads to leak bugs. Jade's `go` blocks are scoped — you can't accidentally leave a coroutine running after its parent exits.

**Key difference from Rust:** No colored functions. A function that sends on a channel looks the same as one that doesn't. No `async fn`, no `.await`, no `Pin<Box<dyn Future>>`. The scheduler handles suspension transparently.

**What makes this work for Jade:**
- Perceus RC is already single-threaded. Cross-coroutine sharing uses `rc` (non-atomic within same thread) or `arc` (atomic across threads).
- Channels are the communication mechanism — no shared mutable state by default.
- The compiler knows at compile time which values cross coroutine boundaries (closure captures in `go` blocks) and can insert the right RC flavor automatically.

---

## Benchmarks

15-benchmark suite. Jade vs C (gcc -O3) vs Rust (rustc -C opt-level=3) vs Python 3.

```
python3 run_benchmarks.py --opt=3 --runs=5 --save=<tag>
python3 run_benchmarks.py --opt=all
python3 run_benchmarks.py --langs=jade,rust
```

Gate: **Jade total ≤ 1.1× of Clang -O3 across full suite.** Achieved.

---

## What's Needed Before Standard Library Work

The core language is in good shape. The remaining gaps before shifting focus to stdlib + ecosystem:

1. **Vec + Map** — Can't write real programs without growable collections. These should be compiler-known types (not library types) so codegen can optimize layout and bounds checking.

2. **Iterators** — `for..in` over Vec, Map, ranges, strings. Needs a simple protocol: `*next() -> Option of T`. The compiler desugars `for x in coll` to `loop { match coll.next() { Some(x) ? body; Nothing ? break } }`.

3. **Trait operator overloading** — `+`, `==`, `<`, `to_string` on user types. Without this, structs are second-class. Extension methods are done; traits are the remaining piece.

4. **String stdlib** — split, trim, replace, find, starts_with, ends_with, parse. These are builtins, not library functions, so they get wired into the string method dispatch.

Once these four are in place, the language is self-sufficient for building its own stdlib and tooling.

---

## Research Foundation

| Paper | Relevance |
|-------|-----------|
| Perceus (Reinking et al., 2021) | Core memory management |
| Counting Immutable Beans (Ullrich & de Moura, 2020) | RC optimization |
| Frame-Limited Reuse (Lorenzen & Leijen, 2023) | Bounded memory reuse |
| Complete & Easy Bidirectional Typechecking (Dunfield, 2021) | Type inference |
| Algebraic Subtyping (Dolan & Mycroft, 2017) | Future trait hierarchies |

---

*Start simple. Build up. Never stop.*
                                                                                          