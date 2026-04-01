# Jade Language — Exhaustive Compiler & Runtime Analysis

---

## 1. Executive Summary & Paradigmatic Identity

### Language Identity

Jade is a **multi-paradigm systems language** fusing imperative/procedural control flow with functional features (pattern matching, algebraic data types, closures, pipelines, list comprehensions), actor-based concurrency (Erlang-model), and structured data persistence (`store` blocks). Its syntactic ethos is **"clear English over opaque symbols"** — natural-language keywords (`is`, `equals`, `returns`, `of`, `from`, `to`) replace conventional sigils, yielding pseudocode-like readability. It is neither purely functional nor object-oriented; it occupies a niche analogous to Rust's structural-typing-plus-traits model but with far less ceremony and a deliberate eschewing of `unsafe` blocks.

### Design Philosophy

Jade targets **native-speed systems programming with low cognitive overhead**. Its primary architectural bets:

1. **Zero-cost abstractions** via Perceus reference counting with compile-time drop elision rather than tracing GC.
2. **Concurrency as a first-class citizen** — stackful coroutines, typed channels, M:N work-stealing scheduler, actors — all surfaced through dedicated syntax, not library-level wrappers.
3. **Compile-time evaluation** — a constant-folding pass (comptime.rs) interprets pure functions at compile time.
4. **Pragmatic persistence** — built-in `store` blocks for file-backed structured data, compiled directly to I/O syscall sequences.

### Architecture Overview

**AOT (Ahead-of-Time) native compilation** via LLVM:

```
Source (.jade)
  → Lexer        (indentation-sensitive tokenizer)
  → Parser       (recursive descent + precedence climbing)
  → AST          (untyped, surface-level)
  → Typer        (type inference, monomorphization, HIR lowering)
  → HIR          (typed, DefId-indexed, exhaustiveness-checked)
  → Comptime     (constant folding of pure functions)
  → Perceus      (reference counting optimization: 9 analysis phases)
  → Ownership    (static move/borrow verification)
  → Codegen      (LLVM IR via inkwell)
  → LLVM passes  (O0–O3 via PassBuilder)
  → Object file  (native machine code)
  → Linker       (cc, linking libjade_rt for concurrency)
```

Total implementation: ~46K LOC Rust (compiler) + ~2K LOC C (runtime) + ~2K LOC assembly (context switch).

---

## 2. Frontend Pipeline (Lexical & Syntactic Analysis)

### Lexer (lexer.rs)

**Strategy**: Hand-written, single-pass, byte-level scanner operating on `&[u8]` with explicit position/line/column tracking. **1,246 lines**.

**Indentation sensitivity**: Python-style INDENT/DEDENT token synthesis via an explicit indent stack (`indents: Vec<u32>`). The `handle_indent()` method compares leading whitespace against the stack top and emits INDENT or DEDENT pseudo-tokens. **Tabs are rejected** (`b'\t' => return self.err("tabs are not allowed; use spaces")`), enforcing spaces-only formatting.

**Token representation** (lexer.rs): A flat `enum Token` with ~130 variants. Literals carry values inline (`Int(i64)`, `Float(f64)`, `Str(String)`, `CharLit(i64)`). Keywords are resolved via a `keyword()` function containing ~80 string-match arms — a **linear-search dispatch table**, not a hash map. This is a minor optimization opportunity but immaterial at typical source sizes.

**Trivia handling**: Comments are `#` line-comments, silently consumed by `skip_line()`. No block comments. Whitespace (spaces) are consumed during tokenization. Newlines are significant — the lexer collapses consecutive newlines into a single `Token::Newline` via the `nl` flag.

**String interpolation**: Handled at the lexer level via re-entrant lexing. When `{` is encountered inside a `'...'` string, the lexer:
1. Emits a partial `Str` token + `InterpStart`
2. Scans for the matching `}` tracking brace depth
3. Instantiates a **new `Lexer`** on the inner substring (`lex_all()`)
4. Splices the inner tokens into the pending queue
5. Emits `InterpEnd`

A recursion guard limits nesting to depth 8 (`interp_depth >= 8`). This is a clean, if non-standard, approach that avoids grammar complexity at the parser level.

**Number literals**: Full support for hex (`0xFF`), binary (`0b1010`), octal (`0o77`), scientific notation (`2.5e-10`), and underscore separators (`1_000_000`).

**Character literals**: `:c` syntax (colon followed by a single character), with escape sequences (`:\ n`, `:\ 0`, etc.) — stored as `i64` for direct interoperability with integer operations, following the C tradition.

### Parser (parser)

**Algorithm**: **Recursive descent** combined with **precedence climbing** (via the `binop!` macro). **2,754 lines** across three files: mod.rs (846), expr.rs (1,255), stmt.rs (653).

**Precedence hierarchy** (lowest to highest):

| Level | Operators | Method |
|-------|-----------|--------|
| 0 | `? !` (ternary) | `parse_ternary` |
| 1 | `~` (pipeline) | `parse_pipeline` |
| 2 | `or` | `parse_or` (binop macro) |
| 3 | `xor` | `parse_xor` |
| 4 | `and` | `parse_and` |
| 5 | `equals`/`neq` | `parse_eq` |
| 6 | `<` `>` `<=` `>=` `in` | `parse_cmp` (chained comparisons) |
| 7 | `\|` (bitwise OR) | `parse_bitor` |
| 8 | `^` (bitwise XOR) | `parse_bitxor` |
| 9 | `&` (bitwise AND) | `parse_bitand` |
| 10 | `<<` `>>` | `parse_shift` |
| 11 | `+` `-` | `parse_add` |
| 12 | `*` `/` `%`/`mod` | `parse_mul` |
| 13 | `**` (exponentiation, right-assoc) | `parse_exp` |
| 14 | Unary: `-`, `not`, `~`, `@` (deref) | `parse_unary` |
| 15 | Postfix: `.field`, `[idx]`, `(args)`, `as` | `parse_postfix` |

The `binop!` macro (mod.rs) is elegant — it generates a left-recursive loop over token-to-operator mappings, calling the next-higher-precedence parser for each operand. This is essentially a manually unrolled Pratt parser, with each precedence level being a distinct function rather than using a numeric binding power table.

**Notable feature**: Chained comparisons (`a < b < c` → `(a < b) and (b < c)`) are desugared in-parser at `parse_cmp()`, cloning the middle operand. This creates a hidden double-evaluation risk if `b` has side effects, though in practice Jade's expression semantics are largely pure.

**Pipeline operator** (`~`): The parser handles placeholder synthesis — `val ~ $ * 2` is desugared to `val ~ *fn(__ph) __ph * 2` via `replace_placeholder()`. Named-call pipelines pass through unmodified for the typer.

**Pattern-directed function clauses**: `*fib(0) is 0` / `*fib(n) is n * fib(n-1)` — parser accumulates same-name `Decl::Fn` items, then `desugar_multi_clause_fns()` merges them into a single function with an `IfExpr` dispatch chain testing literal parameters.

### Error Handling (Frontend)

**Basic synchronization**: The parser uses `ParseError` with line/column reporting. There is **no panic-mode recovery** — a single parse error aborts compilation. The `expect()` method produces `"expected {t}, got {actual}"` messages. Error granularity is function-level; there is no error-recovery to continue past a malformed declaration and parse subsequent decls.

This is a pragmatic choice for a young language but limits IDE integration (LSP) to first-error-only reporting. The lsp directory exists in the tree, implying ongoing work.

---

## 3. Semantic Analysis & Type System

### AST Design (ast.rs)

**565 lines.** The AST is **weakly typed** — types are `Option<Type>` on parameters and fields, allowing inference. Key structural properties:

- `Decl` is a flat sum type with 15 variants (functions, types, enums, externs, actors, stores, traits, impls, consts, type aliases, newtypes, etc.)
- `Expr` has 50+ variants, including domain-specific nodes: `Spawn`, `Send`, `Receive`, `ChannelCreate`, `Select`, `StoreQuery`, `Grad`, `Einsum`, `SIMDLit`, `NDArray`
- `Span` is a lightweight 12-byte value type: `{ start: usize, end: usize, line: u32, col: u32 }`
- `Pat` supports wildcards, identifiers, literals, constructors, or-patterns, ranges, tuples, and arrays

The AST carries **no `DefId`s** — name resolution is deferred entirely to the typer.

### Type System (types.rs, typer)

**Type representation**: `enum Type` with 38 variants covering:
- Fixed-width integers: `I8`–`I64`, `U8`–`U64`
- Floating point: `F32`, `F64`
- Compound: `Array(Box<Type>, usize)`, `Vec(Box<Type>)`, `Map(Box<Type>, Box<Type>)`, `Tuple(Vec<Type>)`, `Set`, `Deque`, `PriorityQueue`
- Nominal: `Struct(String, Vec<Type>)`, `Enum(String)`
- Higher-kinded: `Fn(Vec<Type>, Box<Type>)`, `Ptr(Box<Type>)`, `Rc(Box<Type>)`, `Weak(Box<Type>)`, `Channel(Box<Type>)`, `Coroutine(Box<Type>)`, `Generator(Box<Type>)`
- Inference: `TypeVar(u32)`, `Param(String)`
- Type-level: `DynTrait(String)`, `Arena`, `Alias`, `Newtype`, `Cow`, `SIMD`, `NDArray`

**Type inference**: A **UnionFind-based unification engine** (unify.rs, 1,199 lines) implementing a constrained variant of Hindley-Milner:

- **`InferCtx`** maintains parallel arrays: `parent`, `rank`, `types`, `origins`, `constraints`, `usage_sites`
- **TypeConstraints**: `None | Numeric | Addable | Integer | Float | Trait(Vec<String>)` — these are first-class constraint lattice elements merged during unification
- **`fresh_var()`** allocates a new unsolved `TypeVar(u32)` with `TypeConstraint::None`
- **`fresh_integer_var()`** / **`fresh_float_var()`** create constrained variables
- **`unify()`** implements structural unification with occurs check (`occurs_in()`), constraint satisfaction, and recursive type walking (arrays, tuples, functions, pointers, channels, etc.)
- **`shallow_resolve()`** follows the union-find chain but preserves unsolved `TypeVar`s; **`resolve()`** kills unsolved vars with defaults (`Float-constrained → F64, else → I64`)

**Bidirectional type checking**: `lower_expr_expected()` passes expected types downward; `unify_at()` propagates upward. The system is **not fully bidirectional** in the ML/Pierce sense — expected types are hints for lambdas and if-expressions, not full propagation through all expression forms.

**Monomorphization** (mono.rs, 491 lines): Generic functions with `Type::Param` parameters are instantiated at call sites via `substitute_type()` + name mangling (`format!("{name}_{ty}")`) + fresh HIR emission. Depth-limited to prevent infinite instantiation (`mono_depth: u32`). Generic enums and structs follow the same pattern.

**Trait system**: Traits declare method signatures with optional default implementations. Impl blocks map trait methods to types. Associated types are supported (`assoc_types` maps). Trait constraints on TypeVars are verified during unification by checking `trait_impls` in `InferCtx`. Dynamic dispatch via **fat pointers** (`{data_ptr, vtable_ptr}`) with thunk-based vtable generation.

**SCC analysis** (scc.rs, 308 lines): Tarjan's algorithm for **detecting mutually recursive function groups** — enables correct TypeVar resolution for co-recursive functions that would otherwise circularly depend on each other's return types.

### Name Resolution & Scoping

**Lexical scoping** via `scopes: Vec<HashMap<String, VarInfo>>` in `Typer`. `push_scope()` / `pop_scope()` form a stack; `find_var()` searches from innermost to outermost. **No separate resolution pass** — resolution happens inline during HIR lowering.

**Two-pass declaration**: `lower_program()` first pre-declares **all** top-level names (functions, types, enums, externs, actors, stores, traits, constants) so that forward references work. Then it lowers bodies in a second pass. Impl blocks are processed between these phases.

**Closures**: Lambda captures are identified at codegen time (lambda.rs) by scanning the body for referenced identifiers not in parameter scope or global function scope. Captured values are materialized via **thread-local globals** (`set_thread_local(true)`) — this is a closure-conversion strategy that avoids heap allocation for the closure environment but limits closures to single-threaded use. This is an unusual choice with **thread-safety implications**.

---

## 4. Middle-End (Lowering, IR, & Optimizations)

### Lowering Process (AST → HIR)

The typer performs **combined type checking and HIR lowering** in a single pass per function body. This is the heaviest component: ~8,000 lines across mod.rs (1,794), lower.rs (2,536), expr.rs (1,497), stmt.rs (664), call.rs (1,132), builtins.rs (484).

Key transformations:
- **DefId assignment**: Every binding, parameter, and function receives a unique `DefId(u32)` for identity tracking through perceus and ownership.
- **Coercion insertion**: Numeric widening/truncating, int↔float, bool→int coercions are inserted as explicit `ExprKind::Coerce(expr, CoercionKind)` nodes.
- **Iterator desugaring**: `for x in coll` → `loop { match iter.next() { Some(x) → body, Nothing → break } }`
- **Method resolution**: `obj.method(args)` → `ExprKind::Method(obj, type_name, method_name, args)` or specialized nodes (`StringMethod`, `VecMethod`, `MapMethod`)
- **Builtin resolution**: ~70 builtin functions (see `BuiltinFn` enum in hir.rs) are recognized and lowered to `ExprKind::Builtin(BuiltinFn, args)`, bypassing function call overhead.
- **Exhaustiveness checking**: Match expressions are verified for completeness against enum variants, boolean values, or-patterns, and guards.
- **Type resolution**: After all functions are lowered, `resolve_all_types()` walks the entire HIR tree replacing all remaining `TypeVar`s with their resolved concrete types.

### Intermediate Representation (HIR)

The HIR (hir.rs, 1,154 lines) is a **typed, DefId-indexed AST** — not SSA, not CPS, not a register-based IR. It is structurally isomorphic to the AST but with all types resolved and all names resolved to `DefId`s. Key differences from AST:

- All `Option<Type>` become concrete `Type`
- `ExprKind` replaces `Expr` variants, separating expression node from type+span metadata
- Explicit `Stmt::Drop(DefId, String, Type, Span)` nodes inserted by perceus
- `Coerce` nodes for implicit conversions
- Distinct expression kinds for different call targets (`Call`, `IndirectCall`, `Builtin`, `Method`, `DynDispatch`)

**Architectural note**: The HIR is **not** in SSA form. The codegen must handle mutable variables, reassignment, and aliasing directly via LLVM `alloca` + `store`/`load`. This is the standard strategy for languages that compile to LLVM — the LLVM mem2reg pass promotes most allocas to SSA registers anyway.

### HIR Validation (hir_validate.rs)

A post-lowering verification pass checking:
- DefId uniqueness (no duplicate definitions)
- Unreachable code detection (statements after return/break/continue)
- Function signature consistency

### Comptime Evaluation (comptime.rs)

A **constant-folding interpreter** operating on the HIR:
- Identifies pure functions (no side effects — only arithmetic, conditionals, recursion)
- For calls to pure functions with all-constant arguments: interprets the body in a `HashMap<DefId, ConstVal>` environment
- Recursion depth limited to 100 steps
- Supports `Int`, `Float`, `Bool` constant values; `BinOp` and `UnaryOp` arithmetic

### Perceus Reference Counting Optimization (perceus)

**1,516 lines** across mod.rs, analysis.rs, uses.rs.

Perceus is Jade's implementation of the **Perceus** reference counting optimization framework (Reinking et al., ICFP 2021). Nine analysis phases produce `PerceusHints`:

1. **Drop specialization** (`analyze_drop_specialization`): Trivially droppable types (scalars, scalar arrays, tuples of scalars, ptrs, ActorRef, Channel) have their drops elided entirely.
2. **Reuse analysis** (`analyze_reuse`): Single-use Rc values whose allocation can be reused by a later allocation of compatible layout.
3. **Borrow promotion** (`promote_borrows`): Single-use borrowed values promoted to moves.
4. **Last-use tracking** (`analyze_last_use`): Records the last use point of each binding for deferred drop insertion.
5. **FBIP analysis** (`analyze_fbip`): Functional-but-in-place — detects match-and-reconstruct patterns where the destructed value's memory can be reused.
6. **Tail reuse** (`analyze_tail_reuse`): Parameter that is dropped in a tail-recursive call whose allocation can be reused for the recursive call's allocation.
7. **Drop fusion** (`analyze_drop_fusion`): Consecutive drops in the same block fused into a single operation.
8. **Speculative reuse** (`analyze_speculative_reuse`): Like reuse analysis but for speculative paths (if/match branches).

`PerceusHints` is passed to the codegen as a separate struct, not embedded in the HIR. The codegen consults it during `Stmt::Drop` compilation.

---

## 5. Backend & Code Generation

### Target Generation

Jade emits **LLVM IR** via the [inkwell](https://github.com/TheDan64/inkwell) Rust bindings (wrapping LLVM 21 C API). The codegen module is **13,442 lines** across 26 files.

The compilation flow:

1. **`compile_program()`**: Declares all types/enums/externs/functions (forward declarations), then generates vtables, compiles actor loops, then compiles all function bodies.
2. **`emit_object()`**: Runs LLVM's `PassBuilder` optimization pipeline (`default<O0..O3>`), then emits a native `.o` file via `TargetMachine::write_to_file()`.
3. **Linking**: Invoked externally via `cc` (system C compiler), linking `-lm` and optionally `-ljade_rt -lpthread` for concurrency.

### LLVM IR Generation Patterns

- **All variables are `alloca`d** via `entry_alloca()` — placed at the function entry block for LLVM's mem2reg to handle: `let tmp = self.ctx.create_builder(); match entry.get_first_instruction() { Some(inst) => tmp.position_before(&inst), None => tmp.position_at_end(entry) }; tmp.build_alloca(ty, name)`
- **Structs** use `build_struct_gep` for field access
- **Enums** use a tagged-union layout: `{ i32 tag, [max_payload x i8] }` with `usize` alignment
- **Strings** use a **24-byte SSO layout**: `{ ptr data, i64 len, i64 cap }` where tag byte at offset 23 discriminates inline (≤23 bytes, bit7=1) vs heap (bit7=0)
- **Vec/Map/Set**: Opaque pointer types backed by malloc'd buffers with explicit header structs
- **Dyn trait**: Fat pointer `{ data: ptr, vtable: ptr }`, vtable is a global array of function pointers with thunks for ABI adaptation

### Register Allocation & Instruction Selection

Delegated entirely to LLVM. Jade's codegen emits unoptimized LLVM IR and relies on the LLVM pass pipeline for:
- mem2reg (alloca→register promotion)
- Instruction selection (SelectionDAG)
- Register allocation (greedy allocator at O2+)
- Loop vectorization, SLP vectorization (enabled at O2+)
- Function merging (enabled at O3)

### Calling Conventions

Standard **C calling convention** (System V AMD64 ABI on x86-64, AAPCS64 on AArch64). No custom calling conventions. Functions tagged with `nounwind`, `nosync`, `nofree`, `mustprogress`, `willreturn`, `norecurse` attributes for LLVM optimization benefits. `main()` is wrapped with a `__jade_user_main()` indirection to inject scheduler init/run/shutdown calls for concurrency support.

Foreign functions via `extern` use C linkage. String parameters are coerced from Jade's 24-byte struct to raw `ptr` at call boundaries.

---

## 6. Memory Model & Execution Semantics

### Allocation Strategy

**Stack**: All local variables are `alloca`'d at function entry. Scalars, fixed-size arrays, tuples, and struct values live on the stack. LLVM's mem2reg promotes most to registers.

**Heap**: `Vec`, `Map`, `Set`, `Deque`, `PriorityQueue`, `Rc<T>`, `Arena` — allocated via `malloc()`. Strings ≤23 bytes use inline SSO storage (stack/register); longer strings heap-allocate.

**Arena**: `Arena(cap)` allocates a single `malloc(cap)`-byte buffer; `arena.alloc(size)` bumps an offset pointer with no deallocation until `arena.reset()`.

### Data Semantics

**Pass-by-value** is the default. Structs, tuples, arrays, and strings are passed by value (LLVM struct copy). The `%expr` / `Ref` syntax produces a pointer (`Type::Ptr(I8)`, always erased to `i8*`). Method `self` is by-value for regular methods, by-pointer (`*self`) for mutating methods.

**Copy semantics**: There is no implicit `Clone` trait. Struct copies are bitwise. For heap-backed types (Vec, Map, String (heap)), a copy creates a shallow duplicate — **aliasing safety relies on Perceus and the ownership verifier** to insert drops and prevent use-after-free.

### Pointers & References

- `%x` → `Expr::Ref` → `ExprKind::Ref` → raw pointer (`Type::Ptr(I8)`)
- `@p` → `Expr::Deref` → `ExprKind::Deref` → typed load from pointer
- No pointer arithmetic in the language syntax (available via `asm` blocks)
- `Rc(T)` → heap-allocated `{ i64 refcount, T value }`, `rc_retain()` uses `atomicrmw add`, `rc_release()` uses `atomicrmw sub` + conditional free
- `Weak(T)` → companion to `Rc`, with `upgrade`/`downgrade` semantics
- **No lifetime annotations** — the ownership verifier catches use-after-move and double-borrow statically, but does not enforce borrow lifetimes à la Rust

### Memory Management & Lifetimes

**Hybrid approach**:

1. **Perceus drop insertion** (compile-time): The typer's `emit_scope_drops()` inserts `Stmt::Drop` at scope boundaries. Perceus optimizes these away for trivially-droppable types and reuses allocations where possible.

2. **Reference counting** (`Rc<T>`): Atomic refcount (`atomicrmw` ops) for shared ownership. The drop handler checks `rc_release()` → conditional `free()`.

3. **Static ownership verification** (ownership.rs, 755 lines): A **post-HIR verification pass** (not a type-system enforcement) that detects:
   - Use-after-move
   - Double mutable borrow
   - Move of borrowed value
   - Invalid Rc dereference
   - Return of borrowed value
   - Weak upgrade without check

   This produces warnings/errors but **hard errors abort compilation only for severe violations** (UAF, double-mut-borrow). Weak-upgrade-without-check is a warning.

4. **No garbage collector**: No tracing GC at runtime. All deallocation is deterministic via scope drops + refcounting.

### Concurrency

**M:N green thread model** with work-stealing:

- **Runtime** (runtime): ~2,000 lines of C implementing:
  - **Stackful coroutines** (coro.c): 64KB mmap'd stacks with guard pages, platform-specific context switch in assembly (context_x86_64.S, context_aarch64.S)
  - **M:N scheduler** (sched.c): N worker threads (one per CPU by default), each with a **Chase-Lev work-stealing deque**. Idle workers steal FIFO from random peers. Parking via `pthread_cond_timedwait` (1ms timeout for timer checks).
  - **Typed channels** (channel.c): Bounded MPMC ring buffer. Fast-path: atomic CAS on head/tail. Slow-path: park coroutine on wait queue, scheduler-integrated resume.
  - **Select** (select.c): Go-style multiplexing across multiple channels.
  - **Timers** (timer.c): `clock_gettime(CLOCK_MONOTONIC)`-based deadline tracking.

- **Actors**: Erlang-model with bounded MPSC mailbox (pthread mutex+condvar), one coroutine per actor. Backpressure on full mailbox. Message format: `{ i32 tag, [max_payload x i8] }`.

- **Atomics**: `AtomicLoad`, `AtomicStore`, `AtomicAdd`, `AtomicSub`, `AtomicCas` — compiled to LLVM atomic instructions with `SeqCst` ordering.

- **Memory visibility**: Sequential consistency for atomics. No relaxed ordering options exposed. Channel operations provide happens-before guarantees through the lock-based slow path.

---

## 7. Language Syntax, Ergonomics, & Capabilities

### Syntax Aesthetics

Jade's syntax is **Python-like indentation** with **English keywords**:

```jade
*fib(n)
    if n < 2
        return n
    return fib(n - 1) + fib(n - 2)

*main
    log(fib(40))
```

- Functions: `*name(params)` — the `*` prefix is distinctive and terse
- Binding: `x is 42` (not `let x = 42`)
- Comparison: `equals` / `neq` / `lt` / `gt` / `lte` / `gte`
- Pipeline: `val ~ transform` (using `~`)
- Type annotations: `x as i64` (context-dependent — `as` is also a cast)
- Return types: `*fn(x as i64) returns i64`
- Modulo: `mod` keyword (not `%`)
- Parens optional on no-arg functions and calls
- `for i from 0 to 10 by 2`
- `unless` / `until` as inverted `if` / `while`

The "English first" philosophy creates readable code but increases lexer keyword count (~80 keywords) and creates occasional context-sensitivity.

### Control Flow

- **If/elif/else**: Standard, plus `unless` (inverted condition)
- **While/loop/for**: `while cond`, `loop` (infinite), `for x in collection`, `for i from start to end by step`, `until` (inverted while)
- **Match**: Exhaustiveness-checked pattern matching with constructors, or-patterns, range patterns, guard clauses, tuple/array destructuring
- **`yield`**: Returns a value from a loop (loop-as-expression)
- **Error handling**: `err` definitions (tagged unions), `ErrReturn` statement for early-exit propagation (similar to Rust's `?` operator semantics)
- **No exceptions/try-catch**: Errors are values, consistent with Rust/Haskell philosophy

### First-Class Constructs

- **Functions**: First-class values; `%fn_name` captures a function pointer. Lambdas: `*fn(x) expr`
- **Closures**: Capture by value, materialized via thread-local globals at codegen
- **Traits**: Declaration with associated types, default methods, impl blocks
- **Dynamic dispatch**: `dyn TraitName` creates fat pointers with vtable indirection
- **Generics**: Parametric polymorphism via monomorphization — no dictionary passing
- **Actors**: First-class `actor` blocks with `spawn`, `send`, `receive`, `supervisor`
- **Channels**: `channel of Type(capacity)`, `send ch, value`, `receive ch`
- **Coroutines/Generators**: `yield` in function bodies creates stackful coroutines
- **List comprehensions**: `[expr for x in coll if cond]`
- **Stores**: Persistent file-backed typed records with query/filter/transaction support

---

## 8. Runtime & Ecosystem

### Runtime Overhead

- **No GC pauses**: Deterministic deallocation via scope-based drops + Rc
- **Coroutine overhead**: 64KB mmap'd stack per coroutine + guard page (standard for green threads)
- **Channel overhead**: Lock-free fast path, mutex slow path. Bounded ring buffer eliminates unbounded memory growth.
- **Actor mailbox**: Single mutex + two condvars per actor. Ring buffer (capacity 256) with backpressure.
- **String SSO**: 24-byte inline representation avoids heap allocation for short strings (≤23 bytes covers ~95% of typical string values)
- **Atomic refcounting**: `atomicrmw` on each retain/release — cache-coherence cost on multi-core but no stop-the-world

### Standard Library (std)

10 modules in pure Jade:
- fmt.jade: String formatting
- io.jade: File I/O
- math.jade: Math functions (wrapping LLVM intrinsics)
- os.jade: OS interface
- path.jade: Path manipulation
- rand.jade: Random number generation
- regex.jade: Regular expressions
- sort.jade: Sorting algorithms
- strings.jade: String utilities
- time.jade: Time operations

**Module resolution**: `use` declarations → file path lookup (relative to source, std, executable directory, `CARGO_MANIFEST_DIR`). Interface caching via `.jadei` files for pre-compiled declarations.

### Tooling

- **Package manager**: `jade.pkg` + `jade.lock` + `cache` system with SemVer resolution (pkg.rs, cache.rs, lock.rs)
- **Formatter**: `jadec fmt` (fmt.rs, 535 lines) — code formatter with consistent style enforcement
- **LSP**: lsp directory exists (structure unclear from listing)
- **Tree-sitter grammar**: tree-sitter-jade for editor integration
- **VS Code extension**: vscode-jade with syntax highlighting
- **Test framework**: Built-in `test` blocks compiled with `--test` flag
- **C header binding generator**: `jadec bind header.h` (bind.rs, 440 lines)
- **Interface emission**: `--emit-interface` generates `.jadei` files for separate compilation
- **Debug info**: DWARF via `--debug`/`-g` (debug locations per-statement, function scopes)
- **Benchmarking**: run_benchmarks.py with cross-language comparison (C, Rust, Python)

---

## 9. Critical Expert Evaluation

### Strengths

1. **Performance parity with C**: Benchmark aggregate shows Jade at 0.94× Clang `-O3` across 19 benchmarks. This is exceptional for a language with automatic memory management, closures, and dynamic dispatch.

2. **Perceus integration**: The 9-phase Perceus analysis is one of the most complete implementations outside of Koka itself. Drop elision, reuse analysis, FBIP, tail reuse, drop fusion, and speculative reuse provide genuine reference-counting optimization beyond naive `retain`/`release`.

3. **Pragmatic type inference**: The UnionFind-based `InferCtx` with constraint lattice (`Numeric`, `Integer`, `Float`, `Addable`, `Trait`) is well-engineered. The separation of `shallow_resolve()` (preserved-unsolved) vs `resolve()` (defaulting) prevents premature monomorphization.

4. **Concurrency runtime**: The M:N work-stealing scheduler with Chase-Lev deques, guard-paged coroutine stacks, and lock-free channel fast paths is production-grade systems engineering. Platform-specific assembly context switch (x86-64 + AArch64) eliminates `setjmp`/`longjmp` overhead.

5. **No `unsafe`**: The language's "sharp" philosophy makes all operations available without escape hatches, maintaining consistent safety guarantees.

6. **SSO strings**: The 24-byte SSO layout is cache-friendly and eliminates heap allocation for the common case.

7. **Comptime evaluation**: Constant-folding of pure functions at compile time is a straightforward but effective optimization.

### Weaknesses & Bottlenecks

1. **Closure capture via thread-local globals** (lambda.rs): Captured variables are copied into thread-local globals, then the lambda function reads from these globals. This is **fundamentally unsafe in concurrent contexts** — if a closure is sent to another thread (e.g., via channel or actor message), the captured values will be read from the wrong thread's TLS or, worse, from zero-initialized memory. A heap-allocated environment struct (closure conversion) is the standard approach.

2. **No parser error recovery**: A single syntax error aborts compilation. For IDE integration (LSP), this severely limits the user experience. Panic-mode recovery with synchronization tokens (e.g., `DEDENT`, `*`, `type`, `enum`) is a standard improvement.

3. **Linear keyword lookup**: `keyword()` in the lexer is an 80-arm `match` statement. While Rust's compiler will likely optimize this to a lookup table, an explicit `phf` or `HashMap` would guarantee O(1) lookup.

4. **Monomorphization explosion risk**: `mono_depth` limits recursion but doesn't limit total instantiation count. A type like `Vec of Vec of Vec of ...` with deeply nested generics could produce code bloat. No sharing of monomorphized code between identical instantiations produced via different paths.

5. **Ownership verifier is advisory, not enforcing**: The ownership pass runs after HIR lowering and produces diagnostics, but only hard-errors on severe violations. The type system itself does not encode move semantics or borrow lifetimes. This means **some use-after-move or aliasing bugs may slip through** to runtime. The verifier is a best-effort static analysis, not a soundness guarantee.

6. **HIR is not in SSA form**: The codegen operates on a tree-structured HIR and relies entirely on LLVM's mem2reg for register promotion. This means Jade-level optimizations (beyond Perceus and comptime) are difficult to implement without first lowering to an SSA or CPS IR.

7. **`Stmt::Drop` codegen is partially wired**: The repo memory notes that `Stmt::Drop` is partially a no-op for non-Rc types. For `String`, `Vec`, `Map` — the drop should call `free()` on the heap buffer, but the `ensure_free()` function is noted as dead code. This implies **potential memory leaks for non-Rc heap types** when Perceus cannot elide the drop.

8. **String interpolation re-entrancy**: Creating a new `Lexer` instance per interpolation expression works but creates allocation overhead proportional to interpolation count. A state-machine-based approach would avoid allocations.

### Safety

**Memory safety**: The combination of Perceus drop insertion + ownership verification provides **strong but not watertight** memory safety:
- **UAF**: Caught by ownership verifier in most cases, but the verifier is not sound (tree-walking heuristic, not formal proof)
- **Double-free**: Prevented by DefId-based drop tracking — each binding drops exactly once
- **Buffer overflows**: Array indexing is **not bounds-checked at runtime** (LLVM GEP is unchecked). Vec indexing has bounds checks in the built-in `compile_index` codegen. Raw arrays (`Type::Array`) use unchecked GEP.
- **Null dereference**: Raw pointers (`%x`) can be null — no null-safety for `Ptr` types. `Rc`/`Weak` use checked operations.
- **Thread safety**: The TLS closure capture strategy is unsound for concurrent use. Channel/actor communication is safe (runtime-mediated).

**Type safety**: The type system is **structurally sound** for monomorphic code — unification prevents type confusion. However:
- `TypeVar` fallback to `I64` when unsolved means some ambiguous programs type-check but with surprising semantics
- `Type::Param` falls through to `i64` in codegen — unresolved generic parameters silently compile as 64-bit integers
- `StrictCast` (`strict` keyword) bypasses normal coercion rules — potential for reinterpretation UB

### Performance

**Execution speed**: The 0.94× Clang `-O3` benchmark result is credible given the architecture:
- LLVM's optimization pipeline (O3) applies identical passes to Jade's IR as it would to Clang's
- Jade's lack of bounds checking and GC removes overhead sources
- The main performance tax is Rc atomics and scope-based drops for non-trivially-droppable types
- SSO strings eliminate allocation for short strings
- Perceus drop elision removes most trivial drops

**Compilation speed**: Multi-pass architecture (lex → parse → type → HIR → comptime → perceus → ownership → codegen → LLVM optimize → link) with no parallelism or incremental compilation. For large projects, expect compilation time dominated by LLVM's backend (O3 is slow). The `.jadei` interface caching helps with module-level incrementality.

**Critical O(n²) concern**: The `analyze_reuse()` function (analysis.rs) uses a nested loop over Rc bindings: `for i in 0..rc_bindings.len() { for j in (i+1)..rc_bindings.len() { ... } }`. For functions with many Rc bindings, this is O(n²). In practice, n is typically small, but pathological cases (generated code, macro-expanded patterns) could trigger quadratic slowdown.

---

**Bottom line**: Jade is an **ambitious, well-executed systems language** at approximately v0.2 maturity. Its standout achievement is integrating Perceus reference counting, a work-stealing M:N runtime, and LLVM codegen into a cohesive system that achieves near-C performance. The primary architectural debts are the TLS closure capture strategy (unsafe for concurrency), the advisory nature of the ownership verifier (not a soundness guarantee), and the lack of an SSA-form middle IR for Jade-level optimizations. The language design is coherent and readable, with a clear identity distinct from Rust's complexity.