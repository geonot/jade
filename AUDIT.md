# Jade Compiler — Comprehensive Audit Report

**Scope:** Full 7-phase audit of the `jadec` compiler (16,655 LOC Rust)  
**Codebase version:** `v0.0.0` (Cargo.toml)  
**Date:** 2025  
**Build status at start of audit:** BROKEN (stores.rs:886 merge artifact) — **fixed during audit**  
**Test status after fix:** 215/215 passing, 0 warnings

---qqqqqqqqqqqqqqqqqqq

## Table of Contents

1. [Phase 1 — Front-End (Lexer, Parser, Semantic Analysis)](#phase-1--front-end)
2. [Phase 2 — IR Generation (AST → HIR Lowering)](#phase-2--ir-generation)
3. [Phase 3 — Optimizations (Perceus RC, Ownership)](#phase-3--optimizations)
4. [Phase 4 — Back-End (Code Generation, Instruction Selection)](#phase-4--back-end)
5. [Phase 5 — Runtime & Toolchain Integration](#phase-5--runtime--toolchain-integration)
6. [Phase 6 — Security & Robustness](#phase-6--security--robustness)
7. [Phase 7 — Maintainability & Documentation](#phase-7--maintainability--documentation)
8. [Prioritized Findings Summary](#prioritized-findings-summary)
9. [Recommended Action Plan](#recommended-action-plan)

---

## Phase 1 — Front-End

### Lexer (`lexer.rs` — 1,051 lines)

**Architecture:** Single-pass character-at-a-time lexer with indentation tracking (Python-style INDENT/DEDENT). 68 keywords, string interpolation via `{expr}`, raw strings, escape sequences, hex/binary/octal/scientific number literals.

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| L-01 | **Medium** | **No multi-line comments.** Only `#` single-line comments. Makes it awkward to comment out blocks of code. | Add `#{ ... }#` or similar block comment syntax. |
| L-02 | **Low** | **No multi-line strings.** No triple-quote or heredoc syntax for embedding large text. | Consider `"""..."""` or indent-based multi-line strings. |
| L-03 | **Low** | **ASCII-only identifiers.** `is_ascii_alphabetic()` / `is_ascii_digit()` rejects all Unicode identifiers. This is fine for now but limits international adoption. | Document as a deliberate choice or relax to Unicode XID_Start/XID_Continue. |
| L-04 | **Info** | **Tabs are rejected.** `handle_indent()` returns `Err("tabs are not allowed")`. Good design choice — eliminates mixed-indent ambiguity. | No action needed. Correct. |
| L-05 | **Low** | **String interpolation creates a recursive Lexer.** `lex_string()` spawns an inner `Lexer` for `{expr}` interpolation. Stack depth is unbounded for deeply nested interpolations like `"{"{"{x}"}"}"`. | Add a nesting depth limit (e.g., 8). |

### Parser (`parser.rs` — 2,486 lines)

**Architecture:** Recursive-descent with a `binop!` macro for precedence climbing. 12-level precedence chain. Post-pass `desugar_multi_clause_fns()` merges pattern-directed function overloads into if/elif chains.

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| P-01 | **Medium** | **`desugar_multi_clause_fns()` has O(n²) pattern matching.** Each clause compares parameter lists to find matching overloads. For modules with many small functions this is unlikely to matter, but for generated code with hundreds of clauses it could. | Acceptable for now. Profile if parse time becomes an issue. |
| P-02 | **Medium** | **No error recovery.** Parser returns `Err(String)` on the first error and stops. User sees one error per compilation. | Implement synchronization points (e.g., skip to next `fn`/`type` on error) and collect multiple diagnostics. |
| P-03 | **Low** | **`for` loop uses `in` keyword in parser** (`Token::In`), but other documentation references `from`/`to` syntax. Potential spec/implementation mismatch for range-based for. | Audit docs vs. implementation and unify. |
| P-04 | **Info** | **Store filter parsing supports compound filters** (`and`/`or` with `LogicalOp`). Well-structured. | No action needed. |

### Semantic Analysis / Type Checking (`typer.rs` — 3,079 lines)

**Architecture:** The `Typer` performs AST → HIR lowering with name resolution, type inference, and generic monomorphization in a single pass. Scoped variable resolution via `Vec<HashMap<String, (DefId, Type)>>`.

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| T-01 | **High** | **Fallback to `I64` on type inference failure.** `expr_ty_ast()` returns `Type::I64` as a default for ~15 branches (unknown call return, unknown field, unknown method). This silently hides type errors and can cause miscompilation. | Return `Type::Inferred` or `Type::Error` sentinel type and propagate errors. |
| T-02 | **Medium** | **Generic monomorphization can infinite-loop.** Recursive generic instantiation (e.g., `fn f<T>(x: T) → f<Array<T>>(...)`) has no depth limit. | Add a recursion depth limit (e.g., 64) on `mangle_generic` / `mono_*` calls. |
| T-03 | **Medium** | **No occurs check for type inference.** If Jade gains more sophisticated type inference (e.g., Hindley-Milner), the current system has no occurs check to prevent infinite types. Not currently exploitable but will be when inference is extended. | Note for future work. |
| T-04 | **Low** | **`Type::Inferred` exists but is never resolved to a concrete type.** It acts as another synonym for I64 in practice. | Either implement proper inference resolution or remove the variant. |
| T-05 | **Low** | **`effective_type_params()` auto-infers type params from untyped parameters.** Clever feature but undocumented and could surprise users. | Document this behavior in the language spec. |

### Diagnostics (`diagnostic.rs` — 197 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| D-01 | **High** | **Diagnostic system is defined but barely used.** Most errors across lexer, parser, and typer are plain `String` via `die()`, `Err(format!(...))`, or `eprintln!`. The structured `Diagnostic` type with error codes, labels, notes, and suggestions is only used in ~5 places (match exhaustiveness, ownership warnings). | Migrate all error sites to use `Diagnostic`. This is the single highest-leverage improvement for user experience. |
| D-02 | **Medium** | **Error codes are defined but most are unused.** `ErrorCode` has ranges E001–E799 and W001–W299, but the majority are never emitted. | Either implement error codes at each emission site or trim the enum. |

---

## Phase 2 — IR Generation

### AST (`ast.rs` — 428 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| A-01 | **Info** | Well-structured. 9 `Decl` variants, 18 `Stmt` variants, 28 `Expr` variants. Clean separation. `Span` captures all source locations. | No action needed. |

### HIR (`hir.rs` — 402 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| H-01 | **Info** | Clean design. Adds `DefId` to all bindings, 31 `ExprKind` variants, 6 `Ownership` variants. Nicely typed. | No action needed. |
| H-02 | **Low** | **`DefId` is `usize`.** No namespace isolation — a DefId from one module can collide with another. Fine for single-file compilation but will break with incremental/parallel compilation. | Wrap in a newtype with module context if multi-module compilation becomes a goal. |

### AST → HIR Lowering

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| IR-01 | **Medium** | **Coercion insertion is ad-hoc.** `coerce_binop_operands()` handles Int↔Float, Float↔Float, and Int↔Int width coercions. But coercion for function call arguments is limited to String→ptr for extern calls (`coerced_args()`). No general implicit coercion framework. | Introduce a coercion matrix or trait-like mechanism if implicit conversions become more complex. |
| IR-02 | **Low** | **Return type inference falls back to `Void` or `I64`.** `infer_ret_ast()` checks only the last statement. Multi-path returns (e.g., `if/else` with different tail expressions) aren't verified for type consistency. | Add return type unification across all return paths. |

---

## Phase 3 — Optimizations

### Perceus Reference Counting (`perceus.rs` — 1,184 lines)

**Architecture:** Sophisticated Perceus-style RC optimization pass producing `PerceusHints`. Implements drop elision, reuse analysis, borrow-to-move promotion, speculative reuse, last-use tracking, drop fusion, FBIP (functional-but-in-place) detection, and tail reuse.

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| RC-01 | **Info** | **Impressive optimization suite.** 8 distinct optimization strategies in a single pass. Stats tracking for diagnostics. Well-designed. | No action — this is a strength of the compiler. |
| RC-02 | **Medium** | **Borrow-to-move promotion is always safe only for single-use bindings.** The check `info.use_count == 1 && !info.escapes && !info.borrowed` is correct. However, the `escapes` flag is set conservatively for lambda captures — any variable referenced in a lambda is marked as escaping even if the lambda doesn't outlive the scope. | Consider more precise escape analysis (e.g., non-escaping closures). |
| RC-03 | **Low** | **Drop fusion heuristic is simple.** Fuses consecutive drops of the same type. Could be extended to fuse drops within the same basic block even if separated by trivial statements. | Minor optimization opportunity. |

### Ownership Verification (`ownership.rs` — 638 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| O-01 | **Medium** | **Ownership diagnostics are warnings, not errors.** `OwnershipVerifier` detects use-after-move, double borrow, etc., but only emits diagnostics that don't halt compilation. Code with use-after-move will compile and potentially crash at runtime. | Make critical ownership violations (UseAfterMove, DoubleMutableBorrow) hard errors. Keep WeakUpgradeWithoutCheck as a warning. |
| O-02 | **Low** | **No tracking through struct fields.** Moving `s.field` doesn't invalidate `s` or other fields of `s`. | Track partial moves for structs if ownership becomes stricter. |
| O-03 | **Low** | **Ownership analysis doesn't track through match arms.** A variable consumed in one arm but not another isn't flagged. | Handle divergent consumption in match analysis. |

---

## Phase 4 — Back-End

### Code Generation — Core (`codegen/mod.rs` — 462 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| CG-01 | **Medium** | **`DWARFSourceLanguage::C` used as placeholder** for debug info. Debuggers (GDB, LLDB) will interpret Jade programs as C, causing confusing variable inspection and step behavior. | Register a custom DWARF language ID or use `DW_LANG_lo_user`+N. |
| CG-02 | **Low** | **`filename` field in `Compiler` struct is written but never read.** Dead code. | Remove the field. |
| CG-03 | **Info** | **Function attribute tagging is thorough.** `nounwind`, `nosync`, `nofree`, `willreturn`, `norecurse` are correctly applied based on program analysis (`fn_may_recurse()`). | Well done. |
| CG-04 | **Info** | **LLVM pass manager configuration is solid.** Vectorization and loop unrolling enabled at O2+, correct use of `PassBuilderOptions`. | No action needed. |

### Code Generation — Types (`codegen/types.rs` — 360 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| CT-01 | **Medium** | **Manual `type_store_size()` and `type_abi_align()` instead of LLVM DataLayout.** The manual calculations duplicate logic that LLVM's `DataLayout::getTypeAllocSize()` provides. Any mismatch (e.g., platform-specific alignment) would cause silent memory corruption in stores/actors/enums. | Use `TargetData::get_store_size_of_type()` and `TargetData::get_abi_alignment_of_type()` from inkwell when available. |
| CT-02 | **Low** | **String type is a 3-field struct `{ptr, i64, i64}` but SSO uses the first 23 bytes as inline storage.** This is a union-over-struct trick. The code is correct but the layout relies on the struct being exactly 24 bytes. Any LLVM padding change would silently break SSO. | Add a compile-time assertion (`debug_assert!` on `type_store_size(string_type()) == 24`). |

### Code Generation — Declarations (`codegen/decl.rs` — 247 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| CD-01 | **Info** | **Linkage model is correct.** Internal linkage for non-main functions; External for `main` and lib-mode exports. | No action needed. |
| CD-02 | **Low** | **Tagged union (enum) payload uses `[N x i8]` with manual GEP.** Correct but fragile — relies on the manual size calculation matching the actual largest variant. | Consider generating a proper LLVM union type or validating sizes at compile time. |

### Code Generation — Statements (`codegen/stmt.rs` — 906 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| CS-01 | **Info** | **Drop handler properly respects Perceus hints.** Checks `elide_drops`, `reuse_candidates`, `speculative_reuse`, `borrow_to_move` before emitting free/rc_release/weak_release. | Good integration with the optimization pass. |
| CS-02 | **Medium** | **Match exhaustiveness check emits errors via `Err(diag.render(...))`.** The error string contains ANSI-colored source snippets from `render()`. This means the error message includes escape codes that may not display correctly in all environments. | Separate error rendering from error reporting. Return a `Diagnostic` and let the caller render. |
| CS-03 | **Low** | **`compile_asm()` doesn't validate constraint strings.** Arbitrary inline assembly constraints from user source are passed directly to LLVM. Malformed constraints cause LLVM assertions, not user-friendly errors. | Validate constraint syntax before passing to LLVM, or catch LLVM errors gracefully. |

### Code Generation — Expressions (`codegen/expr.rs` — 1,203 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| CE-01 | **Medium** | **Integer arithmetic uses NSW/NUW flags unconditionally.** `build_int_nsw_add`, `build_int_nuw_add` etc. make signed/unsigned overflow undefined behavior in LLVM. This means integer overflow in Jade programs is UB, not wrapping. | Document this clearly. Consider making default arithmetic wrapping (like Go/Zig) and offering `checked_*`/`wrapping_*` as explicit opts. Users already have `wrapping_add` etc. builtins — but the default `+` operator silently has UB on overflow. |
| CE-02 | **Medium** | **Division by zero is not checked.** `build_int_signed_div` / `build_int_unsigned_div` with a zero divisor is UB in LLVM IR. The generated code will crash with SIGFPE on x86 but behavior is target-dependent. | Insert a zero-check before division, or document that division by zero is UB. |
| CE-03 | **Low** | **`compile_field()` returns `Err("cannot access field on rvalue")`.** This prevents chained field access like `get_point().x`. | Support GEP from rvalues by storing to a temporary alloca. |
| CE-04 | **Low** | **Lambda capture uses global variables.** `compile_lambda()` lifts captured values to LLVM globals (`module.add_global`). This is correct for single-threaded code but causes data races if lambdas are used across actor threads. | Use closure environments (struct passed as extra arg) instead of globals, or document the thread-safety limitation. |
| CE-05 | **Low** | **List comprehension has a hardcoded max of 1024 elements.** `let max_size = 1024u64;` stack-allocates the result array. Exceeding this silently corrupts the stack. | Heap-allocate or add bounds checking. |

### Code Generation — Calls (`codegen/call.rs` — 151 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| CC-01 | **Info** | **Clean dispatch.** Direct calls try module function first, fall back to indirect call. Pipe compilation is straightforward. | No action needed. |

### Code Generation — Stores (`codegen/stores.rs` — 1,418 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| ST-01 | **Critical** | **Build-breaking merge artifact at line 886.** `eval_store_filter` was called with 9 arguments instead of 7 due to a corrupted duplicate `er.op, fval, &extras`. | **FIXED during this audit.** |
| ST-02 | **High** | **No file locking.** Store files are opened with `fopen("r+b")` / `fopen("w+b")` without any advisory or mandatory locking. Concurrent processes writing to the same `.store` file will corrupt data. | Use `flock()` or `fcntl()` advisory locks around store operations. |
| ST-03 | **Medium** | **Fixed 256-byte string buffer.** String fields in stores use `[256 x i8]` fixed buffers. Strings longer than 248 bytes (256 minus 8 for length prefix) are silently truncated. No error or warning. | Either grow the buffer dynamically, emit a compilation error for stores with string fields, or add runtime truncation warnings. |
| ST-04 | **Medium** | **No crash recovery.** The delete operation writes to a new file then renames. If the process crashes mid-write, data can be lost. No journaling or write-ahead log. | Consider fsync + atomic rename pattern, or document durability limitations. |
| ST-05 | **Low** | **`eval_store_filter()` string comparison uses `memcmp()`.** This is byte-comparison only — no locale-aware or UTF-8-aware collation. | Document that string filtering is byte-level. |

### Code Generation — Actors (`codegen/actors.rs` — 636 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| AC-01 | **Medium** | **Fixed ring buffer capacity (256).** `MAILBOX_CAP = 256`. If a producer sends faster than a consumer processes, the sender blocks on `pthread_cond_wait`. No backpressure signaling, no dynamic resize. | Fine for most use cases. Consider making capacity configurable per-actor or adding a warning when the mailbox is frequently full (runtime diagnostic). |
| AC-02 | **Medium** | **Threads are detached (`pthread_detach`).** Spawned actor threads are fire-and-forget. The main thread has no way to join/wait for actors to finish. If `main` returns while actors are running, the process exits and actors are killed mid-message. | Add a `join` or `await` mechanism for actor completion. |
| AC-03 | **Medium** | **Actor state fields default to zero-init via `memset`.** The comment acknowledges "defaults would need to be compiled into the init section in a production impl." User-specified default values for actor state fields are silently ignored. | Compile field initializers into the actor state setup after malloc. |
| AC-04 | **Low** | **No graceful shutdown.** No poison-pill or stop message. The `alive` flag (field 8) is set to 1 at spawn but never set to 0. The actor loop runs forever. | Add a built-in `@stop` handler that sets alive=0 and breaks the loop. |
| AC-05 | **Info** | **Unlock-before-signal optimization.** The send path unlocks the mutex before signaling the condvar, reducing "hurry up and wait" contention. Good practice. | No action needed. |

### Code Generation — Strings (`codegen/strings.rs` — 502 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| SS-01 | **Info** | **SSO implementation is correct and efficient.** 23-byte inline threshold with tag byte at offset 23. Proper SSO/heap bifurcation in all string operations (concat, slice, len, data). | Well-implemented. |
| SS-02 | **Low** | **`string_contains()` uses naive O(n·m) search.** Loops over every position and calls `memcmp`. | For large strings, consider Boyer-Moore or `memmem()`. Low priority — most strings are short. |
| SS-03 | **Low** | **`char_at()` returns raw bytes, not Unicode codepoints.** Works correctly for ASCII but will return individual UTF-8 bytes for multi-byte characters. | Document as byte-indexing, or add a `codepoint_at()` method. |

### Code Generation — Builtins (`codegen/builtins.rs` — 907 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| BU-01 | **Info** | **Rich builtin set.** RC lifecycle (`rc`/`rc_retain`/`rc_release`/`weak`/`weak_upgrade`), volatile load/store, wrapping/saturating/checked arithmetic, signal handling, bit intrinsics (popcount, clz, ctz, bswap, rotl, rotr), assert. | Comprehensive. |
| BU-02 | **Medium** | **`rc_release()` free is not thread-safe.** The refcount decrement and free check is: load → sub → store → compare → free. This is a non-atomic sequence. If an Rc value is shared across actors, concurrent releases can double-free. | Use `atomicrmw sub` + `icmp eq 0` for the refcount path. Alternatively, document that Rc is not thread-safe (acceptable if actors use message-passing only). |
| BU-03 | **Low** | **`rc_retain()` increment is non-atomic.** Same thread-safety concern as BU-02, but retain is less likely to cause corruption (increment is monotonic). | Same recommendation as BU-02. |
| BU-04 | **Low** | **`weak_upgrade()` returns null pointer on dead upgrade.** The caller must check for null. No automatic option/result wrapping. | Consider returning a tagged enum (Some/None) for type safety. |
| BU-05 | **Info** | **`compile_assert()` prints line number and calls `exit(1)`.** Clean trap implementation with proper `unreachable` after the exit call. | No action needed. |

---

## Phase 5 — Runtime & Toolchain Integration

### CLI & Pipeline (`main.rs` — 236 lines)

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| RT-01 | **Medium** | **Module resolution (`resolve_modules`) is recursive with no cycle detection.** `use foo` → reads `foo.jade` → which may `use bar` → which may `use foo`. Infinite recursion crashes the compiler. | Add a `visited: HashSet<PathBuf>` to prevent import cycles. |
| RT-02 | **Low** | **Object file emitted then linked via external `cc`.** The command is `cc output.o -o output -lm -lpthread`. No control over which C compiler is used, no cross-compilation support, no Windows linker support. | Add `--linker` flag and document platform requirements. |
| RT-03 | **Low** | **`--test` mode compiles and runs all `test` blocks.** Good feature. But test output is mixed with program output — no TAP, JUnit, or structured test result format. | Consider structured test output for CI integration. |
| RT-04 | **Info** | **`--emit-ir` and `--emit-obj` work correctly.** Useful for debugging. | No action needed. |

### Build System

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| BS-01 | **Info** | **Cargo workspace with single crate.** Clean setup. LLVM 21 via inkwell 0.8, clap 4.5, thiserror 1.0. | No action needed. |
| BS-02 | **Low** | **`anyhow` dependency declared but not used.** Cargo.toml lists `anyhow = "1.0"` but no `use anyhow` anywhere in source. | Remove unused dependency. |

---

## Phase 6 — Security & Robustness

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| S-01 | **High** | **Integer overflow is UB.** Default `+`, `-`, `*` emit NSW/NUW instructions. Overflowing signed addition, for example, is undefined behavior in LLVM and can be exploited by the optimizer to delete code. | This is the most dangerous correctness issue. Either: (a) use wrapping ops by default and provide `checked_*` for UB-free overflow detection, or (b) emit overflow checks (LLVM `llvm.sadd.with.overflow`) in debug mode. |
| S-02 | **High** | **Division by zero is UB.** See CE-02. On x86, this triggers SIGFPE. On other architectures, behavior is undefined. | Insert a zero-check before every division/modulo, at least in debug builds. |
| S-03 | **Medium** | **No bounds checking on array access.** `compile_index()` emits raw GEP without validating the index is within `[0, len)`. Out-of-bounds access is UB. | Insert bounds check before GEP. Can be elided by LLVM at higher optimization levels if provably safe. |
| S-04 | **Medium** | **`compile_syscall()` allows arbitrary syscalls.** Any Jade program can invoke `syscall(number, ...)` to make raw Linux system calls. This is a powerful but dangerous capability. | Document the security implications. Consider gating behind an `--unsafe` flag if Jade aims for any sandboxing. |
| S-05 | **Medium** | **Inline assembly (`asm`) has no restrictions.** Any Jade program can embed arbitrary x86 assembly. Combined with syscalls, a Jade program has full system access. | Same as S-04 — document or gate behind `--unsafe`. |
| S-06 | **Medium** | **Lambda globals cause data races.** CE-04 revisited from a security angle: if a lambda captures a mutable variable and is sent to an actor, two threads can concurrently read/write the same global. | Use proper closure environments or disallow sending lambdas across actor boundaries. |
| S-07 | **Low** | **Store file path comes from source code.** The `.store` filename is programmer-specified in the Jade source. Malicious or careless paths (e.g., `store "/etc/passwd"`) could overwrite system files. | This is expected for a systems language. Document as programmer responsibility. |
| S-08 | **Low** | **`signal_handle()` accepts arbitrary function pointers.** Passing a non-signal-safe function as a signal handler is undefined behavior per POSIX. | Document signal handler constraints. |

---

## Phase 7 — Maintainability & Documentation

| ID | Severity | Finding | Recommendation |
|----|----------|---------|----------------|
| M-01 | **Medium** | **No `cargo clippy` in CI or development workflow evident.** The code compiles with 0 warnings, which is good. But clippy would catch additional issues (redundant clones, needless borrows, etc.). | Add `cargo clippy -- -D warnings` to CI. |
| M-02 | **Medium** | **Test coverage gaps.** 215 tests cover basic functionality well, but no tests for: nested enum matching, Or/Range patterns, mutable closure captures, complex ErrReturn chains, store CRUD operations, actor messaging, inline assembly, signal handling, checked/saturating/wrapping arithmetic, string methods beyond basic use. | Add targeted tests for each untested feature. |
| M-03 | **Low** | **Monolithic `typer.rs` (3,079 lines).** This is the largest file and performs name resolution, type checking, coercion, lowering, and monomorphization. Hard to navigate and reason about. | Consider splitting into `resolve.rs`, `infer.rs`, `lower.rs`, and `mono.rs`. |
| M-04 | **Low** | **`stores.rs` (1,418 lines) mixes file I/O codegen with filter evaluation.** | Consider splitting filter evaluation into its own module. |
| M-05 | **Low** | **Error messages use `die()` and `format!()`.** See D-01. This duplicates the concern from a maintainability angle: adding new error messages requires manually formatting strings rather than using a structured system. | Central error catalog with codes and templates. |
| M-06 | **Info** | **Good module structure overall.** `codegen/` split into 8 focused files (mod, types, decl, stmt, expr, call, actors, stores, strings, builtins) is clean. | Keep this structure as the codebase grows. |

---

## Prioritized Findings Summary

### Critical (1)
| ID | Component | Issue |
|----|-----------|-------|
| ST-01 | stores.rs | ~~Build-breaking merge artifact~~ **FIXED** |

### High (4)
| ID | Component | Issue |
|----|-----------|-------|
| S-01 | expr.rs | Integer overflow is UB (NSW/NUW flags on default arithmetic) |
| S-02 | expr.rs | Division by zero is UB (no zero-check) |
| D-01 | diagnostic.rs | Structured diagnostic system defined but barely used |
| T-01 | typer.rs | Silent fallback to I64 on type inference failure |

### Medium (19)
| ID | Component | Issue |
|----|-----------|-------|
| ST-02 | stores.rs | No file locking on store operations |
| BU-02 | builtins.rs | Rc refcount operations are non-atomic (thread-unsafe) |
| CE-01 | expr.rs | Default arithmetic has undefined overflow behavior |
| CE-02 | expr.rs | Division by zero not guarded |
| CE-04 | expr.rs | Lambda captures via globals cause data races |
| S-03 | expr.rs | No bounds checking on array access |
| S-04 | expr.rs | Arbitrary syscall access |
| S-05 | stmt.rs | Arbitrary inline assembly access |
| S-06 | expr.rs | Lambda globals + actors = data races |
| O-01 | ownership.rs | Ownership violations are warnings, not errors |
| RT-01 | main.rs | Module import cycle detection missing |
| AC-02 | actors.rs | Detached threads — no join/await |
| AC-03 | actors.rs | Actor state defaults silently ignored |
| CG-01 | mod.rs | DWARF language set to C |
| CT-01 | types.rs | Manual type size/align instead of LLVM DataLayout |
| ST-03 | stores.rs | 256-byte fixed string buffer truncation |
| ST-04 | stores.rs | No crash recovery for store writes |
| P-02 | parser.rs | No parse error recovery |
| L-01 | lexer.rs | No multi-line comments |

### Low (24)
Various quality-of-life, documentation, and robustness improvements as detailed in sections above.

---

## Recommended Action Plan

**Priority 1 — Safety-critical (address before any production use):**
1. **Make integer overflow defined** — either wrapping by default or insert overflow checks in debug mode
2. **Insert division-by-zero checks** — at least in debug builds (`--opt 0`)
3. **Add array bounds checking** — runtime check before GEP, elidable at higher opt levels
4. **Fix lambda capture mechanism** — use closure structs instead of globals to prevent data races
5. **Make ownership violation errors block compilation** — UseAfterMove and DoubleMutableBorrow should be hard errors

**Priority 2 — Correctness:**
6. **Add module import cycle detection** — `HashSet<PathBuf>` in `resolve_modules`
7. **Fix type inference fallbacks** — replace `Type::I64` defaults with error propagation
8. **Make Rc operations atomic** — or document that Rc is single-threaded only and add `Arc` for actors

**Priority 3 — Robustness:**
9. **Migrate error messages to Diagnostic system** — high-leverage UX improvement
10. **Add file locking to stores** — prevent data corruption from concurrent access
11. **Add DWARF language identifier** — correct debug info for Jade
12. **Use LLVM DataLayout for type sizes** — prevent platform-specific miscompilation

**Priority 4 — Polish:**
13. Add multi-line comments
14. Parse error recovery for multiple diagnostics
15. SSO size assertion
16. Actor graceful shutdown mechanism
17. Split `typer.rs` into sub-modules
18. Remove unused `anyhow` dependency and `filename` field
19. Add clippy to development workflow
20. Expand test coverage to uncovered features
