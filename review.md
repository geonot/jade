---

# Jade Compiler — Rigorous Code Review

## Executive Summary

Jade is a ~60KLOC compiled language with an impressively complete pipeline: lexer → parser → AST → typer (Hindley-Milner with union-find) → HIR → Perceus reference-counting optimization → ownership verification → LLVM IR codegen, producing native binaries at 0.94× Clang -O3. For what appears to be substantially the work of one engineer, the breadth is remarkable — concurrency runtime, actors, channels, coroutines, persistent stores, trait objects, SIMD, SSO strings, pattern matching, monomorphization, SCC-based mutual recursion.

**However, the codebase has accrued significant structural debt that will increasingly impede correctness and scalability.** The Typer is a 38-field god object with interleaved concerns. The `Token` enum carries owned `String` values, creating millions of unnecessary heap allocations during lexing. There is no string interning anywhere — identifier comparison is O(n) throughout. The AST and HIR are allocated via standard `Box`/`Vec` with no arena, meaning pathological fragmentation at scale. Several `panic!()` calls in the typer will crash the compiler on valid-but-unusual inputs rather than producing errors. The Map/Set drop path silently leaks element-level resources. The codegen has hundreds of `.unwrap()` calls that should be `?` propagation.

**Viability: High for a research/personal language. Medium for production.** The core architecture is sound and the performance results are real. But the technical debt is compounding, and several of the issues below will become harder — not easier — to fix as the codebase grows.

---

## 1. Critical Flaws (Severity: High)

### 1.1. Map/Set Drop Silently Leaks Non-Trivial Element Resources

In drop.rs, `drop_map_deep` and `drop_set_deep` call `drop_container_simple` which frees the bucket array and header. **But they never iterate live slots to drop keys/values:**

```rust
fn drop_map_deep(
    &mut self, val: BasicValueEnum<'ctx>,
    _kt: &Type, _vt: &Type,  // ← UNUSED
) -> Result<(), String> {
    self.drop_container_simple(val)  // just frees header + bucket ptr
}
```

If a `Map of str, Vec of str` is dropped, every `String` key and every `Vec<String>` value **leaks**. This is a silent memory leak for any non-trivially-droppable key/value type. The `Vec` drop path correctly iterates elements (drop.rs), proving the pattern is known — it was just never implemented for Map/Set.

**Impact:** Memory leak proportional to map size × element complexity. No crash, no warning — just unbounded growth.

### 1.2. `panic!()` in Typer ICEs the Compiler on Reachable Paths

In mod.rs:
```rust
panic!("expected bind");
```

And mod.rs:
```rust
panic!("expected Fn types from instantiation");
```

And mod.rs:
```rust
panic!("expected Fn type for f, got {:?}", b.ty);
```

These are not behind `debug_assert!`. They are live `panic!()` in the typer that will abort the compiler with a stack trace if a user writes code that triggers an unexpected internal state. A compiler must never panic on user input. Every one of these should be `return Err(...)` with a helpful error message.

### 1.3. Unresolved `TypeVar` and `Type::Param` Silently Degrade to `i64` in Codegen

In types.rs:
```rust
Type::TypeVar(v) => {
    debug_assert!(false, "ICE: unresolved TypeVar({v}) reached codegen");
    eprintln!("warning: unresolved TypeVar({v}) reached codegen — defaulting to i64");
    self.ctx.i64_type().into()
}
```

And types.rs for `Type::Param`. These **silently produce wrong code** in release builds (`debug_assert!` is stripped). A `TypeVar` that leaks to codegen means the unification engine failed to resolve, and the generated program will miscompile — interpreting pointers as integers, structs as scalars, etc. This should be a hard error, not a warning.

### 1.4. `entry_alloca` Creates a New Builder Per Call

In mod.rs:
```rust
pub(crate) fn entry_alloca(&self, ty: BasicTypeEnum<'ctx>, name: &str) -> PointerValue<'ctx> {
    let entry = self.cur_fn.unwrap().get_first_basic_block().unwrap();
    let tmp = self.ctx.create_builder();  // ← NEW BUILDER EVERY CALL
    match entry.get_first_instruction() {
        Some(inst) => tmp.position_before(&inst),
        None => tmp.position_at_end(entry),
    }
    tmp.build_alloca(ty, name).unwrap()
}
```

This function is called hundreds of times per function — every variable bind, every temporary, every loop index. Each call creates an `inkwell::Builder` (which wraps an `LLVMBuilderRef` — a heap allocation + LLVM internal state). The builder is then immediately dropped. **This is a significant codegen performance bottleneck.** Store a dedicated alloca builder in `Compiler` and reuse it.

---

## 2. Performance Bottlenecks (Severity: Medium)

### 2.1. No String Interning — O(n) Identifier Comparison Everywhere

The `Token::Ident(String)` variant carries an owned, heap-allocated `String`. Every identifier occurrence (`x`, `main`, `i64`) allocates a new `String` on the heap. The lexer's `KEYWORDS` map does `HashMap<&str, Token>` lookup followed by `.cloned()` — which clones the _value_ Token, but identifiers still produce fresh `String`s.

This flows through the entire pipeline:
- Parser: `self.ident()` returns `String`, matched with `==` (byte comparison)
- Typer: `HashMap<String, ...>` for `fns`, `structs`, `enums`, etc. — every lookup hashes and compares the full string
- Codegen: `HashMap<String, ...>` for `vars`, `fns`, `structs` — same

**For a 10K-line program, this means hundreds of thousands of redundant heap allocations and O(n) string comparisons.** An interning table (mapping strings to `u32` symbols) would reduce all identifier ops to integer comparison and eliminate most allocations. This is standard practice in every production compiler (rustc's `Symbol`, V8's `InternedString`, etc.).

### 2.2. `Token::clone()` Is Expensive by Design

The `Token` enum derives `Clone`, but `Token::Str(String)` and `Token::Ident(String)` require heap allocation to clone. `Parser::peek()` calls `self.tok[self.pos].token.clone()` — **every single `peek()` clones the token**, including any carried String. Since `peek()` is called in a tight loop for every parse decision, this is a major allocator pressure point.

In mod.rs:
```rust
fn peek(&self) -> Token {
    if self.pos < self.tok.len() {
        self.tok[self.pos].token.clone()  // ← clones String heap data
    } else {
        Token::Eof
    }
}
```

**Fix:** Return `&Token` instead of `Token`. This is a one-line change that eliminates the clone, but it requires adjusting callers to borrow rather than own.

### 2.3. AST/HIR Nodes Use `Box<Expr>` Everywhere — No Arena Allocation

Every `Expr::BinOp(Box<Expr>, BinOp, Box<Expr>, Span)` allocates 2 separate heap nodes. A 1000-expression function creates 2000+ scattered heap allocations. The AST is built once, traversed multiple times, and dropped all at once — the textbook use case for an arena allocator (`bumpalo`, `typed-arena`).

**Impact:** Cache-unfriendly traversal patterns, allocator fragmentation, slower drop. For the current codebase size this is tolerable, but it will become the dominant cost for large programs.

### 2.4. `Type` Enum Is 72+ Bytes and Cloned Pervasively

The `Type` enum has 39 variants, many carrying `Box<Type>`, `Vec<Type>`, or `String`. A rough size estimate: the largest variant (`Fn(Vec<Type>, Box<Type>)`) is ~48 bytes, plus the discriminant, padding, etc.

Throughout the typer, `Type` values are `.clone()`d constantly — every unification, every method return type lookup, every variable resolution. The grep for `clone()` in `typer/mod.rs` shows 16+ clone sites in just the method resolver.

**Mitigation:** Intern `Type` values behind indices into a type table (similar to how `TypeVar(u32)` already works). Or use `Rc<Type>` for shared subtrees to make clones O(1).

### 2.5. Typer `Struct` Has 38 Fields — God Object

The `Typer` struct in mod.rs has **38 fields**:
```rust
pub struct Typer {
    pub(crate) next_id: u32,
    pub(crate) scopes: Vec<HashMap<String, VarInfo>>,
    pub(crate) fns: HashMap<String, (DefId, Vec<Type>, Type)>,
    pub(crate) structs: ...
    pub(crate) enums: ...
    pub(crate) generic_fns: ...
    pub(crate) generic_enums: ...
    pub(crate) generic_types: ...
    pub(crate) methods: ...
    pub(crate) mono_fns: ...
    // ... 28 more fields ...
}
```

This is a classic god object. It contains name resolution state, inference state, monomorphization state, diagnostics, deferred resolution queues, and type-scheme caches all in one struct. This makes reasoning about lifetimes impossible, introduces hidden coupling between passes, and makes it very difficult to parallelize any phase.

---

## 3. Design & Maintainability Issues (Severity: Low to Medium)

### 3.1. MIR Pipeline Is Built But Disconnected

From the repo notes and code: the MIR pipeline (lower, optimize, perceus) exists and has 10 SSA optimization passes, but HIR codegen is the default path. MIR codegen exists as an opt-in `--mir-codegen` flag, but the MIR path has no test coverage in the CI test suite. This means:
- ~4000 lines of code (mir/lower.rs + mir/opt.rs + codegen/mir_codegen.rs) are effectively dead
- Any HIR-level refactoring must be duplicated in MIR
- The two codegen paths can silently diverge

**Decision needed:** Either commit to MIR as the primary codegen input (and delete HIR codegen) or delete MIR entirely. Maintaining two parallel codegen backends is unsustainable.

### 3.2. Exhaustive `compile_expr` Match Arm Is 80+ Arms

The `compile_expr` method in expr.rs is an 80+ arm match statement. This is fine at 30 variants but becomes an obstacle at 80+. There is no visitor pattern, no dispatch table —  just a monolithic match. Same issue in the typer's `lower_expr`.

### 3.3. Reserve Keyword Pollution

The `KEYWORDS` table in lexer.rs reserves **80+ keywords**: `grad`, `einsum`, `contract`, `deque`, `syscall`, `build`, `supervisor`, etc. Many of these are aspirational features. Every reserved keyword is a name users cannot use for variables/functions. For a language targeting clarity ("reads like pseudocode"), having 80+ reserved words — many domain-specific — is excessive.

### 3.4. `Span` Does Not Track File Origin

`Span` is `{ start: usize, end: usize, line: u32, col: u32 }` — no file identifier. When modules are imported via `use`, all spans are relative to their source file, but there's no way to determine _which_ file a span belongs to. Error messages from imported modules will show wrong file context.

### 3.5. Incomplete Error Recovery in Parser

The parser has a `synchronize()` method that skips to the next declaration boundary (good), and it accumulates up to 20 errors before bailing (good). **But** the typer has no error recovery — the first type error returns `Err(String)` and aborts the entire compilation. For a production compiler, the typer should accumulate errors and continue lowering to report multiple issues.

### 3.6. HIR Validator Is Too Permissive

The `HirValidator` in hir_validate.rs has `check_top_level_def` which detects duplicate DefIds... but the detection branch is empty:
```rust
if let Some(prev) = self.fn_defs.insert(id.0, span) {
    if prev.line != span.line {
        // Same DefId at different locations — only warn if not a duplicate extern
    }
}
```

The comment says "only warn if not a duplicate extern" but **nothing happens**. Either implement the check or remove the dead code.

---

## 4. Praise

### 4.1. Perceus Reference-Counting Optimization Is Genuinely Sophisticated

The Perceus implementation across perceus is a standout. It includes drop elision, reuse analysis, borrow-to-move promotion, speculative reuse, drop fusion, FBIP (functional-but-in-place) detection, tail-reuse, and pool hints. The analysis passes are compositional and clearly separated. The `UseInfo` struct is lean (6 fields). The stats tracking is precise. This is genuine compiler research-level work — most languages with reference counting (Swift, Python, Lobster) implement far fewer optimizations.

### 4.2. Runtime Concurrency Design Is Production-Quality

The C runtime in runtime is impressively engineered:
- M:N work-stealing scheduler with Chase-Lev deques
- Platform-specific context switch in hand-written assembly (x86_64 and AArch64)
- Spinlock-based synchronization (avoiding glibc futex portability issues)
- Spin-before-park idle strategy with tuned parameters (40 spin iterations, 100μs timedwait)
- Stack caching (up to 64 coroutine stacks) to avoid mmap/munmap churn
- Proper lock-through-context-swap protocol (`held_chan_lock` + `last_action`) to prevent wake-before-save races

This is not toy concurrency — it handles real edge cases that most language runtimes get wrong.

---

## 5. Refactoring Action Plan (Priority Order)

### P0: Correctness (Do First)
1. **Replace all `panic!()` in typer with `return Err(...)`** — grep for `panic!` in typer, replace with error returns. ~4 sites. (1 hour)
2. **Make `TypeVar`/`Type::Param` reaching codegen a hard error** — change the `eprintln` + fallback in `llvm_ty()` to `return Err(...)`. This will surface latent monomorphization bugs. (30 min)
3. **Implement Map/Set element-level drop** — Port the loop pattern from `drop_vec_deep` to `drop_map_deep`/`drop_set_deep`, iterating live slots. (2-4 hours)

### P1: Performance (Do Second)
4. **Return `&Token` from `peek()`** — Change signature to `fn peek(&self) -> &Token`, audit all callers. Eliminates per-token heap clones in the parser. (2-3 hours)
5. **Cache the alloca builder in `Compiler`** — Add a `alloca_bld: Builder<'ctx>` field, initialize once in `new()`, reuse in `entry_alloca`. (30 min)
6. **Introduce string interning** — Build a `StringInterner` (or use `lasso`/`string_interner` crate) in the lexer, thread `Symbol` IDs through AST/HIR. This is a large refactor but the single highest-leverage performance improvement. (Days)

### P2: Architecture (Do Third)
7. **Split `Typer` into sub-structs** — Extract `InferState`, `NameResolution`, `MonomorphizationState`, `DiagnosticAccumulator` as separate structs composed into `Typer`. (Half day)
8. **Decide on MIR** — Either wire MIR as the sole codegen input (deleting ~2000 lines of HIR codegen) or remove MIR entirely (deleting ~4000 lines of MIR code). The dual-path is maintenance liability. (Decision + execution: 1-2 days)
9. **Add file ID to `Span`** — Change `Span` to include a `file_id: u16`, add a `FileMap` to the compilation context. Required for accurate multi-file diagnostics. (Half day)

### P3: Cleanup (Do When Convenient)
10. **Audit keyword table** — Remove aspirational keywords that have no parser/typer support. Reserve them only when the feature ships.
11. **Add error accumulation to typer** — Replace `Result<..., String>` with a diagnostic accumulator, continue after non-fatal errors.
12. **Fix the dead HIR validator branch** — Either implement the duplicate-DefId warning or delete the empty conditional. 