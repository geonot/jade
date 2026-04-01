# Jade Compiler — Remediation Plan

Fourteen weaknesses, analyzed against the current codebase state (March 2026, 25.3K LOC, 753 tests passing). Each section states the problem, root cause, proposed fix, risk/complexity, and sequencing constraints.

---

## 1. TLS Closure Capture — Unsafe for Concurrency

**Root cause:** `compile_lambda` (codegen/lambda.rs L33-49) captures variables by storing them into TLS globals (`g.set_thread_local(true)`), then loading them back inside the lambda body. Closures are therefore not first-class values; they cannot cross threads, and recursive/nested closures with the same capture name silently clobber each other.

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 1.1 | Define a closure representation as a fat pointer `{ fn_ptr, env_ptr }` where `env_ptr` is a heap-allocated struct containing captured values. | codegen/lambda.rs, codegen/types.rs |
| 1.2 | At the call site that creates a closure, `malloc` an env struct, store each captured value into it, and construct the fat pointer. | codegen/lambda.rs |
| 1.3 | Inside the lambda body, receive `env_ptr` as an extra hidden first parameter. Load captures from the env struct instead of TLS globals. | codegen/lambda.rs |
| 1.4 | Adjust all indirect call sites (`compile_call` for function-typed values) to pass the env pointer from the fat-pointer pair. | codegen/expr.rs |
| 1.5 | Integrate env lifetime with Perceus: insert a `Drop` for the env when the last reference to the closure dies. If the closure is `Rc`-wrapped, the env piggybacks on the Rc allocation. | codegen/lambda.rs, perceus/ |
| 1.6 | Remove all TLS global creation from `compile_lambda`. | codegen/lambda.rs |
| 1.7 | Add tests: closure passed to another thread via actor/channel, nested closures capturing overlapping names, recursive closure. | tests/ |

**Risk:** High — touches every closure call path. Must update the calling convention for function-typed values globally.  
**Sequence:** Can start independently; must finish before any concurrency correctness claims.

---

## 2. Ownership Verifier — Advisory, No Soundness Guarantee

**Root cause:** `ownership.rs` walks the HIR and reports diagnostics, but the findings don't block codegen. The five "hard error" kinds (`UseAfterMove`, `DoubleMutableBorrow`, etc.) do call `die()` in `main.rs` L759-791, but only after all diagnostics are collected — and the verifier itself has incomplete coverage (doesn't track through closures, channels, or Rc inner values).

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 2.1 | Audit the verifier's match arms against every HIR expression/statement variant. List uncovered variants. | ownership.rs |
| 2.2 | Add tracking for closure captures: when a closure captures `&mut x`, mark `x` as mutably borrowed for the closure's lifetime. | ownership.rs |
| 2.3 | Add tracking for channel sends: sending a value constitutes a move. | ownership.rs |
| 2.4 | Add tracking through Rc: `Rc::deref_mut` should require exclusive ownership proof. | ownership.rs |
| 2.5 | Make ownership errors *gate* codegen: if any hard-error diagnostic is emitted, return an error before entering codegen. (Currently wired in main.rs but verify no silent swallowing path exists.) | main.rs |
| 2.6 | Add comprehensive test coverage for each `DiagKind` variant, including cross-function and cross-scope cases. | tests/ |
| 2.7 | Long-term: consider a formal borrow-checker model (region-based) if Jade wants Rust-level guarantees. Design document first. | — |

**Risk:** Medium — expanding the verifier is incremental; the soundness guarantee requires proving coverage is exhaustive, which is hard to do partially.  
**Sequence:** Steps 2.1-2.4 are independent of other items. Step 2.5 depends on error handling being correct in main.rs.

---

## 3. No SSA-Form IR for Jade-Level Optimizations

**Root cause:** The compiler pipeline is AST → HIR → LLVM IR. There is no Jade-owned SSA-form IR between HIR and LLVM. Perceus operates on the HIR, which is a tree, not a graph — limiting the optimizations it can perform (no dataflow analysis, no DCE, no constant propagation at the Jade level).

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 3.1 | Design a MIR (Mid-level IR) in SSA form: basic blocks, phi nodes, explicit control flow graph. Document the representation. | New: src/mir.rs |
| 3.2 | Implement HIR → MIR lowering. Each HIR function becomes a CFG of basic blocks. Variables become SSA values. | New: src/mir/lower.rs |
| 3.3 | Port Perceus analysis from HIR-tree-walking to MIR-CFG-walking. This unblocks proper liveness analysis (needed for correct drop insertion). | perceus/, mir/ |
| 3.4 | Implement basic MIR passes: dead code elimination, constant folding, copy propagation. | New: src/mir/opt.rs |
| 3.5 | Replace the codegen input from HIR to MIR: `MIR → LLVM IR` instead of `HIR → LLVM IR`. | codegen/ |
| 3.6 | Retain HIR for type checking and semantic analysis; MIR becomes the codegen handoff point. | — |

**Risk:** Very high — this is a large architecture change that touches every part of codegen.  
**Sequence:** This is a multi-milestone project. Steps 3.1-3.2 can proceed in parallel with all other items. Step 3.5 is a flag day that requires coordinated migration of all codegen. Consider a feature flag to run both paths initially.

---

## 4. TypeVar Fallback to I64

**Root cause:** `default_quantified_vars` (typer/unify.rs L84-94) defaults all unsolved type variables to `I64` (or `F64` for float-constrained vars). Multiple call sites in lower.rs also use `unwrap_or(Type::I64)`. This means type inference failures are silent — a variable that should be `String` or a struct type becomes `i64` with no warning (unless `--warn-inferred-defaults` is passed, which defaults to off).

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 4.1 | Change the default for `--warn-inferred-defaults` to **on**. Users who rely on implicit i64 defaulting can opt out with `--no-warn-inferred-defaults`. | main.rs, typer/lower.rs |
| 4.2 | Introduce an `--error-on-inferred-defaults` flag that promotes these warnings to hard errors. This becomes the path toward requiring explicit type annotations where inference fails. | main.rs, typer/lower.rs |
| 4.3 | Audit every `unwrap_or(Type::I64)` in lower.rs (lines 75, 1509, 1556, 2032). For each, determine if the context permits a more specific default or if an error should be emitted instead. | typer/lower.rs |
| 4.4 | For numeric literals without suffix, I64 default is correct. For non-literal contexts (function parameters, return types, struct fields), emit a diagnostic instead of silently defaulting. | typer/unify.rs, typer/lower.rs |
| 4.5 | Add tests: function with unresolved generic param should warn/error, numeric literal should still default silently. | tests/ |

**Risk:** Low-medium — mainly diagnostic plumbing. Risk of breaking programs that rely on silent defaulting, mitigated by the opt-out flag.  
**Sequence:** Independent; can proceed in parallel with all other items.

---

## 5. Type::Param Fall-Through to i64

**Root cause:** `llvm_ty()` (codegen/types.rs L56) maps `Type::Param(_)` → `i64`. This arm should be unreachable — by codegen time, all type parameters should be monomorphized. Its existence silently masks monomorphization bugs.

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 5.1 | Change `Type::Param(_) => self.ctx.i64_type().into()` to `Type::Param(name) => panic!("unresolved type param '{}' at codegen", name)`. | codegen/types.rs |
| 5.2 | Do the same for `Type::TypeVar(_)` on line 24. | codegen/types.rs |
| 5.3 | Run the full test suite; any panics reveal monomorphization gaps that need fixing upstream in the typer. | — |
| 5.4 | Fix any upstream typer gaps discovered in 5.3. | typer/ |

**Risk:** Low — this is a correctness tightening. Any breakage reveals pre-existing bugs.  
**Sequence:** Should be done *before* adding new generic features, to catch regressions early.

---

## 6. StrictCast Bypasses Normal Coercion Rules

**Root cause:** `compile_strict_cast` (codegen/expr.rs L1123-1187) only validates narrowing integer→integer casts (truncate + re-extend + compare). It falls back to `compile_cast` for anything else, which means `as strict` on float→int or pointer→int silently uses normal (unsafe) coercion. The stated intent is: "force an error if a strict cast was truncated."

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 6.1 | Document the intended semantics of `as strict` for each type pair: int→int, float→int, int→float, ptr→int, struct→struct. | jade.md or lang-roadmap.md |
| 6.2 | For **float→int**: add overflow checking — convert, convert back, compare. Trap if the round-trip doesn't match (NaN, infinity, out-of-range). | codegen/expr.rs |
| 6.3 | For **ptr casts** and **struct casts**: decide whether `as strict` should be a hard error (disallowed) or should have specific semantics. Currently falls through silently. | codegen/expr.rs |
| 6.4 | For **int→float** with precision loss (e.g., large i64 → f64): optionally add a round-trip check. Lower priority since precision loss is expected behavior for most users. | codegen/expr.rs |
| 6.5 | Add tests for each type pair with boundary values (I64_MAX, NaN, overflow). | tests/ |

**Risk:** Low — additive; doesn't change existing cast behavior, only extends strict checking.  
**Sequence:** Independent.

---

## 7. analyze_reuse Is O(n²)

**Root cause:** `analyze_reuse` (perceus/analysis.rs L24-65) compares every Rc binding against every later Rc binding in a nested loop. With `n` Rc bindings in one block, this is O(n²).

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 7.1 | Group Rc bindings by their layout size in a `HashMap<usize, Vec<BindingId>>`. | perceus/analysis.rs |
| 7.2 | For each new allocation, look up same-size-class candidates in O(1) amortized. Within a size class, take the first match (FIFO). | perceus/analysis.rs |
| 7.3 | Result: O(n) amortized for the common case (few distinct size classes), O(n·k) where k is the max bucket size worst case. | — |
| 7.4 | Benchmark before/after with a program that has many Rc allocations in one scope (e.g., building a large linked list). | benchmarks/ |

**Risk:** Low — self-contained change within perceus.  
**Sequence:** Independent.

---

## 8. No Parallelized or Incremental Compilation

**Root cause:** The compiler processes a single file in a single thread: lex → parse → lower → type-check → codegen → link. There is no module-level parallelism and no caching of intermediate artifacts.

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 8.1 | **Incremental compilation (lower effort, higher payoff first):** Hash each function's HIR and cache the generated LLVM IR (bitcode). On recompile, skip unchanged functions. | New: src/cache.rs (exists but may need expansion) |
| 8.2 | Define a cache key: `(function_name, hash(hir_subtree), hash(dependency_signatures))`. Invalidate if any dependency's signature changes. | cache.rs |
| 8.3 | **Parallel codegen:** After type-checking, the function table is immutable. Partition functions across N threads, each with its own LLVM context/module. Link modules at the end. | codegen/mod.rs, main.rs |
| 8.4 | Use `rayon` or `std::thread::scope` for the parallel dispatch. LLVM contexts are thread-safe when distinct. | Cargo.toml, codegen/ |
| 8.5 | **Parallel parsing (lower priority):** Only matters for multi-file projects. Parse each file independently, merge ASTs. | parser/, main.rs |
| 8.6 | Measure compilation time on a representative project to establish baselines before optimization. | — |

**Risk:** Medium — LLVM module merging has subtleties (name conflicts, global declarations). Incremental compilation requires careful invalidation.  
**Sequence:** Step 8.1-8.2 (incremental) can start immediately and delivers value independently. Step 8.3-8.4 (parallel) is a larger effort.

---

## 9. Rc Atomics and Scope-Based Drops for Hard-to-Drop Types

**Root cause:** All `Rc` operations use `AtomicRMWBinOp` with `AcquireRelease` ordering (rc.rs L49-53, L68-72), even for single-threaded code. Atomic RMW is 10-20× slower than plain load/add/store on x86. Additionally, `rc_release` only `free()`s the Rc header — it doesn't recursively drop the inner value.

### What makes a type "hard to drop"

A type is hard to drop when it requires non-trivial cleanup:
- **Nested heap types:** `Rc<Vec<String>>` — freeing the Rc doesn't free the Vec's buffer or the Strings inside it.
- **Recursive types:** `Rc<Node>` where `Node` contains `Rc<Node>` — requires cycle-aware reference counting or weak refs.
- **Types with external resources:** File handles, sockets, channels — require cleanup beyond `free()`.
- **Enum variants with mixed drop needs:** An enum where one variant holds heap data and another doesn't.

### Plan

| Step | Work | Files |
|------|------|-------|
| 9.1 | **Non-atomic fast path:** Add a `needs_atomic_rc(ty: &Type) -> bool` check. If a type never escapes to another thread (not sent through a channel, not captured by an actor closure), use plain i64 load/add/store instead of atomicRMW. | codegen/rc.rs |
| 9.2 | **Escape analysis:** In the ownership verifier or a new pass, track whether an Rc-typed variable is ever sent to a channel, captured by an actor, or stored in a global. Annotate the HIR/perceus hints with `local_only: bool`. | ownership.rs or perceus/ |
| 9.3 | **Recursive inner drop:** When `rc_release` frees, generate a call to the inner type's destructor before freeing. For `Rc<Vec<String>>`: call `drop_vec` on the inner value, which itself calls `drop_string` on each element, then `free` the Rc header. | codegen/rc.rs, codegen/stmt.rs |
| 9.4 | Implement a `drop_value(val, ty)` helper that dispatches to the correct destructor for any type. Use this in both `Stmt::Drop` and `rc_release`. | codegen/stmt.rs or codegen/drop.rs (new) |
| 9.5 | **Scope-based drop optimization:** When Perceus can prove an Rc's refcount never exceeds 1 (single-owner), elide the refcount entirely and use scope-based `free()` — the Rc degenerates to a `Box`. | perceus/analysis.rs |
| 9.6 | Benchmark Rc-heavy programs (e.g., linked list construction/traversal) before and after. | benchmarks/ |

**Alternatives considered:**
- *Biased reference counting* (fast thread-local path + slow atomic fallback): more complex but optimal. Deferred to later.
- *Epoch-based reclamation*: overkill for Jade's current use cases.
- *Tracing GC for cycles*: would solve the cycle problem but adds latency unpredictability. Not recommended for Jade.

**Risk:** Medium — the non-atomic fast path is safe if escape analysis is correct. Recursive drops are essential for correctness.  
**Sequence:** Step 9.3-9.4 (recursive drop) is a correctness fix and should be prioritized. Steps 9.1-9.2 (non-atomic fast path) are a performance optimization.

---

## 10. Raw Arrays and Raw Pointers — Safety

**Root cause:** Arrays are stack-allocated with `build_gep` for indexing, and bounds checking exists for fixed-size arrays (codegen/expr.rs L714). However: (a) NDArray uses raw `malloc`'d buffers with no bounds checking, (b) Vec data access through `build_gep` has no bounds check in the inner loop, (c) the slice fallback for non-Vec/non-String types returns the original value unchanged (a no-op bug).

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 10.1 | **Slice fallback bug:** The `_ =>` arm in slice codegen (expr.rs L1319) should emit an error, not silently return the input. Change to `return Err(...)`. | codegen/expr.rs |
| 10.2 | **NDArray bounds checking:** Add bounds check calls before NDArray element access, gated by a `--bounds-check` flag (default: on in debug, off in release). | codegen/expr.rs |
| 10.3 | **Vec bounds checking:** The Vec indexing path should call `emit_bounds_check` using the Vec's length field. Currently relies on LLVM's `build_gep` which doesn't check. | codegen/vec.rs or codegen/expr.rs |
| 10.4 | **Compile-time flag:** `--release` or `--unchecked` to strip all bounds checks for production builds. | main.rs, codegen/ |
| 10.5 | **Unsafe block syntax (long-term):** Consider an `unsafe` keyword for raw pointer operations, making the rest of Jade's surface memory-safe by construction. Design document first. | — |

**Risk:** Low for steps 10.1-10.3 (bug fixes). Medium for 10.4-10.5 (design decisions).  
**Sequence:** Step 10.1 is a simple bug fix — do immediately. Steps 10.2-10.3 can proceed independently.

---

## 11. Parser Error Recovery — Single Error Aborts Compilation

**Root cause:** Every parse function returns `Result<T, ParseError>` and propagates errors via `?`. The first error exits the parser entirely. `main.rs` calls `parse_program().unwrap_or_else(|e| die(...))`.

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 11.1 | Change `ParseError` to accumulate errors: add a `Vec<ParseError>` to the parser struct. | parser/mod.rs |
| 11.2 | Add a `synchronize()` method: on error, skip tokens until a synchronization point (newline at indent level 0, `*` for a new function, `type`/`enum` for a new type, or EOF). | parser/mod.rs |
| 11.3 | Wrap fallible parse paths (parse_fn, parse_type_def, etc.) in a `try_parse_or_sync` helper: attempt to parse, if it fails, push the error and synchronize. | parser/mod.rs |
| 11.4 | At the end of `parse_program`, return all collected errors (if any) instead of just the first one. | parser/mod.rs |
| 11.5 | Update `main.rs` to display all parse errors, with colored spans and counts. | main.rs, diagnostic.rs |
| 11.6 | Add a cap (e.g., 20 errors) to avoid flooding the terminal. | parser/mod.rs |
| 11.7 | Add tests: file with 3 independent syntax errors should report all 3. | tests/ |

**Risk:** Medium — error recovery can produce cascading false-positive errors if synchronization is too aggressive or too lax. Careful choice of sync points is key.  
**Sequence:** Independent of all other items. High UX value.

---

## 12. Use PHF or HashMap for keyword() Instead of 80-Arm Match

**Root cause:** `keyword()` (lexer.rs L300-398) is a ~80-arm `match` on string slices. While rustc compiles string matches reasonably, a perfect hash function is faster for large match sets and eliminates the linear scan.

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 12.1 | Add `phf` crate to `Cargo.toml` (build dependency). | Cargo.toml |
| 12.2 | Replace the `keyword()` body with a `phf_map!` macro invocation mapping `&str → Token`. | lexer.rs |
| 12.3 | Alternatively, use a `HashMap<&'static str, Token>` constructed once with `LazyLock::new` — simpler, no new dependency, still O(1). | lexer.rs |
| 12.4 | Benchmark lexing time on a large source file before and after. | — |

**Risk:** Very low — drop-in replacement.  
**Sequence:** Independent. Quick win.

---

## 13. Stmt::Drop Partially Wired / ensure_free() Dead Code / Memory Leaks

**Root cause:** `Stmt::Drop` (codegen/stmt.rs L137-206) only handles `String`, `Vec`, `Map`, `Rc`, `Weak`, and `Arena`. The `_ => {}` catch-all silently leaks all other heap types: `Set`, `Deque`, `Channel`, `NDArray`, `Generator`, and any user-defined struct containing heap-allocated fields. `ensure_free()` is defined (codegen/mod.rs L558) but never called in normal drop paths — only `Rc`/`Arena` use `free()` directly.

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 13.1 | **Implement `drop_value(val, ty)` dispatcher:** A single function that handles drop for every type, including recursive field drops for structs. | New: codegen/drop.rs or extend codegen/stmt.rs |
| 13.2 | Add drop implementations for missing types: `Set` (free bucket array + drop elements), `Deque` (free ring buffer + drop elements), `Channel` (free buffer, condition vars), `NDArray` (free data buffer), `Generator` (free stack/state). | codegen/drop.rs |
| 13.3 | **Struct drops:** For each field of a struct, recursively call `drop_value`. Skip Copy types (integers, floats, bools). | codegen/drop.rs |
| 13.4 | **Enum drops:** Switch on the discriminant tag, drop the active variant's fields. | codegen/drop.rs |
| 13.5 | Wire `ensure_free()` into the new `drop_value` as the final `free()` call for heap-allocated types. | codegen/drop.rs |
| 13.6 | Replace the `_ => {}` catch-all in `Stmt::Drop` with a call to `drop_value`. | codegen/stmt.rs |
| 13.7 | Add tests: create and scope-drop a `Set`, `Deque`, `Channel`, struct with String field. Run under Valgrind/AddressSanitizer to verify no leaks. | tests/ |

**Risk:** Medium — recursive drop for deeply nested types needs cycle detection (or rely on the ownership verifier to prevent cycles). Getting this wrong causes double-free.  
**Sequence:** This is a correctness item and should be prioritized. Step 13.1 is the foundation; blocks 13.2-13.6. Closely related to item 9 (Rc recursive drops).

---

## 14. Lexer Creates New Instance Per Interpolation

**Root cause:** String interpolation (lexer.rs L855-900) creates `let mut inner_lexer = Lexer::new(inner_str)` for each `{}` expression inside a string. This allocates a new `Vec<Token>`, copies span state, and discards it after lexing. For strings with many interpolations, this creates allocation churn. Additionally, span information for interpolated expressions is incorrect (inner lexer starts at line 1, col 1).

**Plan:**

| Step | Work | Files |
|------|------|-------|
| 14.1 | **State-machine approach:** Instead of creating a new Lexer, add a `mode` field to the existing Lexer (`enum LexMode { Normal, StringInterp { depth: u8 } }`). When `{` is encountered inside a string, push the mode and continue lexing in expression mode until the matching `}`. | lexer.rs |
| 14.2 | Use a mode stack (`Vec<LexMode>`) to handle nested interpolations (`"outer {inner_fn("nested {x}")} end"`). | lexer.rs |
| 14.3 | Emit structured tokens: `Token::InterpStart`, then expression tokens, then `Token::InterpEnd`. This preserves the token stream's linear structure. | lexer.rs, ast.rs |
| 14.4 | **Span correctness:** Since the same lexer handles interpolations, line/col tracking is continuous and correct. No span fixup needed. | lexer.rs |
| 14.5 | Remove the `Lexer::new(inner_str)` allocation path. | lexer.rs |
| 14.6 | Add tests: string with 100 interpolations, nested interpolations, interpolation containing a string literal. | tests/ |

**Risk:** Medium — changing the lexer's state machine affects token generation for all string handling. Careful testing needed.  
**Sequence:** Independent.

---

## Execution Priority

Ordered by impact × urgency:

| Priority | Items | Rationale |
|----------|-------|-----------|
| **P0 — Correctness** | 13 (Drop leaks), 5 (Type::Param panic), 9.3-9.4 (Rc recursive drop), 10.1 (slice bug) | These are active bugs causing memory leaks or silent miscompilation. |
| **P1 — Safety** | 1 (Closure capture), 2 (Ownership verifier), 4 (TypeVar warn-by-default) | Unsound concurrency behavior and silent type errors. |
| **P2 — UX** | 11 (Parser error recovery), 6 (StrictCast spec), 12 (keyword PHF) | Quality-of-life improvements for users. |
| **P3 — Performance** | 7 (analyze_reuse), 9.1-9.2 (non-atomic Rc), 14 (lexer interpolation) | Performance optimizations with measurable but non-blocking impact. |
| **P4 — Architecture** | 3 (SSA MIR), 8 (parallel/incremental compilation) | Large-scale improvements that unblock future optimization work. |

### Dependency Graph

```
5 (Param panic) ─────── standalone, do first
10.1 (slice bug) ────── standalone, do first
13 (Drop wiring) ──┬── foundation for 9.3/9.4
                   └── 9.3 (Rc recursive drop) uses drop_value from 13.1
4 (TypeVar warn) ───── standalone
1 (Closures) ──────── blocks: safe concurrency, actor migration
2 (Ownership) ─────── benefits from 1 (closure tracking)
11 (Error recovery) ── standalone, high UX value
3 (MIR) ───────────── long-term, blocks: advanced Perceus, parallelism
8 (Parallel) ──────── benefits from 3 (MIR provides clean module boundaries)
```
