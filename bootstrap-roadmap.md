# Jade Self-Hosting Compiler: Analysis, Evaluation & Roadmap

## Executive Summary

This document analyzes the feasibility, requirements, and phased roadmap for
porting the Jade compiler from its current Rust implementation (~30K LOC) to Jade
itself, achieving **self-hosting**: a Jade compiler written in Jade, compiled by
itself.

**Verdict:** Jade is *surprisingly close* to being capable of self-hosting. The
language already has enums, pattern matching, recursion, structs, generics, file
I/O, strings, HashMap, Vec, and trait-based dispatch — the core building blocks
of a compiler. The test suite includes a working mini-compiler
(`compiler_pipeline.jade`) and a recursive-descent JSON parser, proving Jade can
express parser/evaluator patterns today. However, several focused language
additions and a substantial engineering effort are required.

---

## Part 1: Current State Assessment

### 1.1 What the Rust Compiler Does (30,417 LOC)

| Subsystem       | LOC   | Files | Role                                          |
|-----------------|-------|-------|-----------------------------------------------|
| **Typer**       | 7,465 | 11    | Type inference, unification, monomorphization  |
| **Codegen**     | 6,900 | 25    | LLVM IR generation via Inkwell                 |
| **Parser**      | 2,326 | 4     | Recursive descent, indentation-based           |
| **Lexer**       | 1,083 | 1     | 110+ tokens, string interpolation              |
| **HIR**         | 1,028 | 1     | Typed intermediate representation              |
| **Ownership**   | 700   | 1     | Move/borrow tracking                           |
| **Perceus**     | 629   | 3     | Reference counting analysis (9 phases)         |
| **Types**       | 500   | 1     | Type enum, utilities                           |
| **AST**         | 475   | 1     | Untyped syntax tree                            |
| **Diagnostics** | 400   | 1     | Error codes, labels, suggestions               |
| **Other**       | ~8.8K | —     | HIR validation, comptime, caching, main, lock  |

### 1.2 Compilation Pipeline

```
Source → Lexer → Parser → AST → Typer → HIR → Perceus → Ownership → Codegen → LLVM IR → Object → Link
```

The codegen backend uses **Inkwell** (Rust bindings to LLVM's C API). The
self-hosted compiler must choose a backend strategy (see §3.2).

### 1.3 Jade Language Features Available Today

**Strong foundations for a compiler:**
- [x] Enums with data (recursive, generic: `enum Ast` with `AAdd(Ast, Ast)`)
- [x] Pattern matching with destructuring, guards, or-patterns
- [x] Recursive functions (mutual recursion via SCC analysis)
- [x] Structs with methods
- [x] Generics (monomorphized)
- [x] Vec, Map (HashMap), Array
- [x] String ops: `length`, `char_at`, `slice`, `contains`, `find`, `split`, `replace`, `trim`
- [x] File I/O: `read_file`, `write_file` via `std/io`
- [x] First-class functions and closures
- [x] Trait system with dynamic dispatch (`DynTrait`)
- [x] Error types with `!` propagation
- [x] Pipe operator `~` for functional chains
- [x] FFI (`extern` declarations for C interop)
- [x] Modules (`use` imports)
- [x] String interpolation (`"{expr}"`)

**Already proven — `compiler_pipeline.jade` demonstrates:**
- Enum-based token and AST types
- Lexer with character-code comparisons
- Recursive descent parser with operator precedence
- Tree-walking evaluator via pattern matching

---

## Part 2: Gap Analysis

### 2.1 Language Gaps (Must Fix)

| # | Gap | Impact | Effort | Priority |
|---|-----|--------|--------|----------|
| **G1** | ~~**No char literals**~~ ✅ | Lexer must compare `ch equals 43` instead of `ch equals '+'`. Every character constant is a magic number. | S | P0 |
| **G2** | ~~**No Map iteration**~~ ✅ | Cannot `for k, v in map`. Symbol tables, scope maps, and every compiler pass that walks a map require this. | M | P0 |
| **G3** | ~~**No mutable struct fields via methods**~~ ✅ | Methods receive `self` by value. A `Parser` struct with a position cursor needs in-place mutation (advancing `pos`, pushing to `tokens`). Current pattern: return new state via tuples. | L | P1 |
| **G4** | **No `while let` / `if let`** | Must write full `match` blocks for simple optional checks. Verbose when unwrapping `Maybe` values returned from map lookups. | S | P1 |
| **G5** | ~~**No string builder**~~ ✅ | String concatenation is O(n²). Code generation emitting thousands of IR lines would be extremely slow with `result is result + line`. | M | P1 |
| **G6** | ~~**No multi-line string literals**~~ ✅ | Emitting code blocks (IR, assembly) requires escaping or concatenating many single-line strings. | S | P2 |
| **G7** | **No Set collection** | Would be useful for visited-node tracking, scope name deduplication, etc. Workaround: `Map of String, bool`. | S | P2 |
| **G8** | **No bitwise operations on enums / tag access** | Cannot inspect enum discriminant directly. Needed for compact IR encoding. | S | P3 |
| **G9** | **Closures capture by value only** | State-threading patterns require explicit struct passing. Functional compiler passes are possible but verbose. | M | P2 |

**Legend:** S = Small (< 1 week), M = Medium (1-3 weeks), L = Large (3+ weeks)

### 2.2 Standard Library Gaps

| # | Gap | Needed For | Workaround |
|---|-----|-----------|------------|
| **L1** | ~~`mkdir` / directory operations~~ ✅ | Output file organization | `extern` FFI to C `mkdir()` |
| **L2** | ~~`args` parsing / flag handling~~ ✅ | Compiler CLI (`--opt`, `--emit-ir`, etc.) | Build from `std/os.args` |
| **L3** | ~~`exit(code)` process termination~~ ✅ | Error abort | `extern` to C `exit()` |
| **L4** | ~~`stderr` output~~ ✅ | Diagnostic printing | FFI to `fprintf(stderr, ...)` |
| **L5** | ~~Formatted output to string~~ ✅ | IR generation, error messages | String interpolation covers most cases |

### 2.3 Architecture Decisions

#### The Backend Question

The current compiler generates LLVM IR through Inkwell (Rust bindings). The
self-hosted compiler has three viable backend options:

| Backend | Complexity | Performance | Bootstrap Path |
|---------|-----------|-------------|----------------|
| **A: LLVM C API via FFI** | High | Excellent (same as now) | Call LLVM-C functions via `extern` |
| **B: Emit LLVM IR text** | Medium | Good (invoke `llc` / `clang`) | Generate `.ll` files as strings |
| **C: Emit C code** | Low | Good (invoke `cc`) | Generate `.c` files as strings |

**Recommendation: Option B (LLVM IR text) for Phase 1, Option A for Phase 2.**

Rationale:
- Option C is easiest but requires mapping Jade's type system to C, handling
  struct layouts, and loses LLVM optimizations unless you pipe through clang
  anyway.
- Option B emits textual LLVM IR (`.ll` files), which `llc` or `clang` can
  compile. The current Rust codegen already conceptually constructs this IR — it
  just does so through API calls instead of text. This is a well-understood
  bootstrapping technique (used by Zig, among others).
- Option A (direct LLVM-C FFI) gives maximum control and performance but
  requires binding ~200 LLVM-C functions and managing opaque pointer types.
  Worth doing after the compiler is working.

---

## Part 3: Bootstrapping Strategy

### 3.1 The Three-Stage Bootstrap

```
┌─────────────────────────────────────────────────────────────────┐
│  Stage 0: "Genesis"                                             │
│  Rust compiler (jade-rs) compiles jade-in-jade compiler (jadec) │
│  Input: jadec.jade → Output: jadec binary (via jade-rs)         │
├─────────────────────────────────────────────────────────────────┤
│  Stage 1: "Self-Compile"                                        │
│  jadec binary compiles its own source code                      │
│  Input: jadec.jade → Output: jadec-stage1 binary (via jadec)    │
├─────────────────────────────────────────────────────────────────┤
│  Stage 2: "Verification"                                        │
│  jadec-stage1 compiles jadec.jade again                         │
│  Output must be bit-identical to jadec-stage1 (fixed point)     │
│  jadec-stage1 == jadec-stage2 → Compiler is self-hosting ✓      │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 What "Self-Hosting" Means in Practice

The self-hosted `jadec` must be able to:

1. **Read** Jade source files from disk
2. **Lex** them into tokens (110+ token types)
3. **Parse** tokens into an AST (indentation-aware, 12 declaration kinds)
4. **Type-check** the AST (unification, monomorphization, trait resolution)
5. **Lower** to HIR (typed IR with ownership annotations)
6. **Analyze** reference counts (Perceus)
7. **Generate** LLVM IR text (or call LLVM-C)
8. **Invoke** the system linker

That's the *minimum*. For practical use, it also needs diagnostics, the module
system, and optimization pass flags.

### 3.3 The Subset Strategy

Rather than porting all 30K LOC at once, bootstrap from a **language subset**:

**Jade-Core** (the subset the self-hosted compiler initially compiles):
- Integer types (`i64`), `bool`, `String`
- Functions, recursion, closures
- Enums with data, pattern matching
- Structs with methods
- `Vec`, `Map`
- `if`/`elif`/`else`, `while`, `for`, `loop`, `match`
- `use` imports, `extern` FFI
- Error types with `!`

**Excluded from Jade-Core initially:**
- Actors / channels / coroutines / select
- Persistent stores / queries
- Generics (monomorphize manually at first)
- Weak references
- Inline assembly
- String interpolation (use `fmt` functions)
- Comptime evaluation

This is practical because the compiler itself doesn't need actors, stores, or
coroutines.

---

## Part 4: Phased Implementation Roadmap

### Phase 0: Language Prerequisites (Pre-Bootstrap)
*Target: Enhance the Rust compiler so Jade can express a compiler*

#### 0.1 — Char Literals (G1) ✅
Add `'x'` syntax producing `i64` char codes. The lexer already handles single-
quoted strings; extend to single-character case returning `i64`.

```jade
# Before:
if ch equals 43     # '+'
# After:
if ch equals '+'
```

#### 0.2 — Map Iteration (G2) ✅
Add `for k, v in my_map` support. Requires:
- Iterator protocol impl for Map
- Key-value pair destructuring in for-loop desugaring

```jade
for name, ty in scope
    log("{name}: {ty}")
```

#### 0.3 — Mutable Self in Methods (G3) ✅
All methods now receive self by pointer by default, enabling in-place mutation:

```jade
type Parser
    tokens: Vec of Token
    pos: i64

    *advance self
        self.pos is self.pos + 1

    *current self
        self.tokens.get(self.pos)
```

This is critical — the parser, typer, and codegen all need mutable state.

#### 0.4 — StringBuilder Type (G5) ✅
Added `StringBuilder` to `std/strings.jade` that batches appends:

```jade
sb is StringBuilder()
sb.write("define i64 @main() {\n")
sb.write("  ret i64 0\n")
sb.write("}\n")
result is sb.to_string()
```

#### 0.5 — `if let` / `while let` (G4)
Sugar for option/result unwrapping:

```jade
if let Some(val) is map.get("key")
    use(val)
```

#### 0.6 — Multi-line Strings (G6) ✅
Triple-quoted strings for code generation:

```jade
ir is '''
    define i64 @main() {
        ret i64 0
    }
'''
```

---

### Phase 1: Lexer in Jade (~1,100 LOC equivalent)
*Port `src/lexer.rs` (1,083 lines)*

The lexer is the ideal starting point:
- Pure function: `String → Vec of Token`
- No dependencies on other compiler phases
- Already demonstrated in `compiler_pipeline.jade` at smaller scale
- Tests are easy: lex known input, compare token sequence

**Data structures needed:**

```jade
enum Token
    Int(i64)
    Float(f64)
    Str(String)
    Ident(String)
    Plus
    Minus
    Star
    Slash
    # ... 100+ more variants
    Indent
    Dedent
    Newline
    Eof

type Lexer
    source: String
    pos: i64
    line: i64
    col: i64
    indent_stack: Vec of i64
    tokens: Vec of Token
```

**Key challenge:** Indentation tracking. The Rust lexer maintains an indent stack
and emits `Indent`/`Dedent` tokens. This is straightforward in Jade with a
`Vec of i64` stack.

**Estimated Jade LOC:** ~800-1,000

---

### Phase 2: AST & Parser in Jade (~2,800 LOC equivalent)
*Port `src/ast.rs` (475 lines) + `src/parser/` (2,326 lines)*

#### 2.1 — AST Definition

```jade
enum Expr
    IntLit(i64)
    FloatLit(f64)
    StrLit(String)
    Ident(String)
    BinOp(Expr, Op, Expr)
    UnaryOp(Op, Expr)
    Call(Expr, Vec of Expr)
    MethodCall(Expr, String, Vec of Expr)
    FieldAccess(Expr, String)
    Index(Expr, Expr)
    If(Expr, Block, Vec of ElifClause, Block)
    Match(Expr, Vec of MatchArm)
    Lambda(Vec of Param, Expr)
    Tuple(Vec of Expr)
    Array(Vec of Expr)
    StructLit(String, Vec of FieldInit)
    Block(Vec of Stmt)
    Pipe(Expr, Expr)
    # ... more

enum Decl
    Fn(FnDef)
    Type(TypeDef)
    Enum(EnumDef)
    Extern(ExternDef)
    Use(UsePath)
    Trait(TraitDef)
    Impl(ImplDef)
    # ...
```

#### 2.2 — Parser

Jade's parser is already recursive descent — the same architecture translates
directly. The main complexity is:
- Indentation-sensitive parsing (Indent/Dedent tokens from lexer)
- Operator precedence (Pratt parsing or precedence climbing)
- Multi-clause function merging

**Estimated Jade LOC:** ~2,000-2,500

---

### Phase 3: Type System in Jade (~7,500 LOC equivalent)
*Port `src/types.rs` + `src/typer/` (7,465 lines)*

This is the largest and most complex phase:

#### 3.1 — Core Type Representation

```jade
enum Type
    I8
    I16
    I32
    I64
    U8
    U16
    U32
    U64
    F32
    F64
    Bool
    Str
    Void
    Vec(Type)
    Map(Type, Type)
    Array(Type, i64)
    Tuple(Vec of Type)
    Struct(String)
    Enum(String)
    Fn(Vec of Type, Type)
    Ptr(Type)
    Rc(Type)
    Param(String)
    TypeVar(i64)
    DynTrait(String)
```

#### 3.2 — Unification Engine

The current UnionFind-based unifier (~1,046 lines) needs:
- Union-Find with path compression (array-backed)
- Constraint tracking (Numeric, Integer, Float)
- Bidirectional type propagation

This is algorithmically complex but structurally straightforward in Jade —
it's mostly array/map operations.

#### 3.3 — Name Resolution & Monomorphization

- Scope stack (`Vec of Map of String, DefInfo`)
- Generic function instantiation (substitute type params, generate copies)
- Trait method resolution

**Estimated Jade LOC:** ~5,000-6,000

---

### Phase 4: HIR & Analysis Passes (~2,300 LOC equivalent)
*Port `src/hir.rs` + `src/hir_validate.rs` + `src/perceus/` + `src/ownership.rs`*

#### 4.1 — HIR Definition
Mirror of AST but with resolved types, DefIds, and ownership annotations.

#### 4.2 — Perceus (RC Analysis)
9-phase analysis producing drop/retain hints. Operates on HIR.

#### 4.3 — Ownership Tracking
Move/borrow analysis with diagnostic reporting.

**Estimated Jade LOC:** ~1,800-2,200

---

### Phase 5: Code Generation (~6,900 LOC equivalent)
*Port `src/codegen/` (25 files, 6,900 lines)*

#### Strategy: Emit LLVM IR as Text

Instead of calling LLVM APIs, emit `.ll` text files:

```jade
*emit_function(sb: StringBuilder, name: String, params: Vec of Param, body: Vec of HIRStmt)
    sb.write("define i64 @{name}(")
    for i from 0 to params.length
        if i > 0
            sb.write(", ")
        sb.write(llvm_type(params.get(i).ty))
        sb.write(" %{params.get(i).name}")
    sb.write(") {\n")
    sb.write("entry:\n")
    emit_body(sb, body)
    sb.write("}\n\n")
```

**Key components to emit:**
- Type declarations (`%struct.Name = type { ... }`)
- Function signatures and bodies
- Alloca / load / store / GEP instructions
- Branch / conditional branch / phi nodes
- Call instructions (direct + indirect)
- String constants as global byte arrays
- Vtable globals for dynamic dispatch

The current codegen has great structure — each `.rs` file maps to a logical unit
that becomes a Jade module.

**Estimated Jade LOC:** ~5,000-6,000

---

### Phase 6: Driver & Integration (~1,000 LOC equivalent)
*Port `src/main.rs` + `src/pkg.rs` + `src/cache.rs`*

- CLI argument parsing
- File reading and module resolution  
- Pipeline orchestration (lex → parse → type → codegen)
- Invoking `llc` and `cc` for final compilation
- Error reporting and exit codes

```jade
*main()
    args is os.args()
    if args.length < 2
        log("Usage: jadec <file.jade> [options]")
        exit(1)

    source is read_file(args.get(1))
    tokens is lex(source)
    ast is parse(tokens)
    hir is type_check(ast)
    ir is codegen(hir)
    write_file("output.ll", ir)
    system("llc -filetype=obj output.ll -o output.o")
    system("cc output.o -o output -ljade_rt -lpthread")
```

---

## Part 5: Effort Estimation

### Total Estimated Jade LOC

| Phase | Component | Est. Jade LOC | Rust LOC | Ratio |
|-------|-----------|---------------|----------|-------|
| 0 | Language prerequisites | 0 (Rust changes) | — | — |
| 1 | Lexer | 900 | 1,083 | 0.83× |
| 2 | AST + Parser | 2,300 | 2,801 | 0.82× |
| 3 | Type System | 5,500 | 7,465 | 0.74× |
| 4 | HIR + Analysis | 2,000 | 2,357 | 0.85× |
| 5 | Codegen (IR text) | 5,500 | 6,900 | 0.80× |
| 6 | Driver | 800 | 1,811 | 0.44× |
| **Total** | | **~17,000** | **30,417** | **0.56×** |

The ~0.56× ratio is expected: Jade's syntax is more concise than Rust (no
lifetime annotations, no borrow checker boilerplate, implicit returns,
indentation-based), and emitting IR text is simpler than calling LLVM APIs.

### Complexity Ranking

1. **Type system** (hardest) — Unification, generics, trait resolution.
   Algorithmically dense with many interacting rules.
2. **Codegen** — Large surface area (25 modules covering every language
   feature). Mechanically complex but each piece is straightforward.
3. **Parser** — Moderate. Jade's indentation sensitivity adds complexity but
   the recursive descent structure is well-understood.
4. **Lexer** — Easiest standalone component. Well-defined, testable in
   isolation.
5. **Analysis passes** — Moderate. Perceus is algorithmically interesting but
   smaller than type system.
6. **Driver** — Simplest. Glue code.

---

## Part 6: Risk Assessment

### High Risk

| Risk | Mitigation |
|------|------------|
| **Type system complexity** — Unification + monomorphization + trait dispatch is the hardest part of any compiler | Port in stages: basic types first, then generics, then traits. Keep Rust compiler as reference implementation. |
| **Performance of string-based codegen** — O(n²) string concat without StringBuilder | Implement StringBuilder before Phase 5. Or use a `Vec of String` and join at end. |
| **Recursive enum memory** — Deep ASTs may cause stack overflow | Current Jade uses heap-allocated enum variants (boxed). Verify stack depth handling. |
| **Bootstrap chicken-and-egg** — Changes to the self-hosted compiler require the Rust compiler to recompile | Maintain the Rust compiler as "Stage 0" until Stage 2 convergence is proven. Never delete it. |

### Medium Risk

| Risk | Mitigation |
|------|------------|
| **Missing LLVM IR coverage** — The Rust codegen uses 200+ Inkwell API calls. Textual IR must cover all of them. | Comprehensive test suite: compile every test program with both backends, compare output. |
| **Error reporting quality** — The Rust compiler has rich diagnostics. Jade's error types may be less expressive initially. | Accept simpler errors for bootstrap; enhance over time. |
| **Module system gaps** — The package resolver (`src/pkg.rs`) handles `jade.pkg` files, lockfiles, fetching. | Initially: simple file concatenation. Add full module resolution later. |

### Low Risk

| Risk | Mitigation |
|------|------------|
| **Lexer correctness** — Indentation logic is tricky | Extensive test suite already exists. Port tests alongside code. |
| **FFI compatibility** — LLVM IR calling conventions | Well-documented; the C runtime is already designed for external linkage. |

---

## Part 7: Milestone Timeline

### M0: Language Ready
- All P0 and P1 language gaps resolved (G1-G5)
- StringBuilder in stdlib
- Map iteration working
- Mutable self in methods

### M1: Lexer Self-Test
- Jade lexer can lex all existing test programs
- Output matches Rust lexer token-for-token
- Differential testing harness in place

### M2: Parser Self-Test
- Jade parser produces ASTs matching Rust parser
- All 71 test programs parse successfully
- AST pretty-printer for comparison

### M3: Type Checker MVP
- Basic types (integers, bool, string, void)
- Struct and enum type checking
- Function signatures and return type inference
- No generics yet (monomorphize by hand)

### M4: Type Checker Complete
- Unification engine ported
- Generics via monomorphization
- Trait resolution
- Full test suite passes

### M5: Codegen MVP
- Can compile `*main() log("hello")`
- Integer arithmetic, function calls
- String literals, basic control flow
- Produces working LLVM IR text

### M6: Codegen Complete
- All language features generating correct IR
- Enums, pattern matching, closures, collections
- Struct layout, vtable generation
- All test programs compile and produce correct output

### M7: Self-Compile (Stage 1)
- The Jade compiler compiles its own source code
- Resulting binary produces correct output for test programs

### M8: Fixed Point (Stage 2)
- Stage 1 compiler compiles itself
- Stage 2 output is identical to Stage 1
- **Jade is self-hosting** ✓

---

## Part 8: Architectural Recommendations

### 8.1 Project Structure

```
jadec/
    src/
        main.jade       # Driver, CLI
        lexer.jade      # Tokenization
        token.jade      # Token enum
        ast.jade        # AST types
        parser.jade     # Recursive descent parser
        types.jade      # Type enum and utilities
        hir.jade        # HIR types
        typer.jade      # Type checker core
        unify.jade      # Unification engine
        resolve.jade    # Name resolution
        mono.jade       # Monomorphization
        perceus.jade    # RC analysis
        ownership.jade  # Ownership tracking
        codegen.jade    # LLVM IR text emission
        emit.jade       # IR builder helpers
        diagnostic.jade # Error reporting
    std/
        io.jade
        fmt.jade
        os.jade
        # ... (shared with runtime)
```

### 8.2 Testing Strategy

**Differential testing:** For every phase, both compilers (Rust and Jade) must
produce identical output on the same input.

```
                    ┌─── jade-rs ──→ tokens₁ ──→ AST₁ ──→ HIR₁ ──→ IR₁
Source.jade ────┤                                              ↓
                    └─── jadec  ──→ tokens₂ ──→ AST₂ ──→ HIR₂ ──→ IR₂
                                                               ↓
                                              Compare: IR₁ ≡ IR₂ ?
```

Add `--dump-tokens`, `--dump-ast`, `--dump-hir` flags to both compilers for
phase-by-phase comparison.

### 8.3 Do NOT Port These Initially

| Component | Reason | When to Port |
|-----------|--------|-------------|
| Persistent stores (`stores.rs`, 749 LOC) | Not needed for a compiler | Never (or post-bootstrap) |
| Actor system (`actors.rs`) | Not needed for a compiler | Post-bootstrap |
| Coroutines / channels / select | Not needed for a compiler | Post-bootstrap |
| LSP server | Separate concern | Post-bootstrap |
| DWARF debug info | Nice-to-have, not essential | Post-M8 |
| Package manager (`pkg.rs`) | Simple `use` resolution suffices initially | Post-M8 |
| Comptime evaluation | Not needed for bootstrap | Post-M8 |

This cuts ~30% of the codebase out of scope for bootstrap.

### 8.4 When to Retire the Rust Compiler

**Never fully retire it.** Even after achieving self-hosting:

1. Keep it as the "Stage 0" bootstrap compiler for fresh builds
2. Use it as the reference implementation for correctness testing
3. Gradually reduce maintenance as the Jade compiler matures
4. Eventually: the Jade compiler can serve as its own Stage 0 (ship a
   pre-built binary), and the Rust version becomes archival

---

## Part 9: Quick Wins to Start Today

These changes to the **current Rust compiler** directly enable bootstrap work:

1. ~~**Add `--dump-tokens` flag**~~ ✅ — Serialize token stream to stdout for
   differential testing
2. ~~**Add `--dump-ast` flag**~~ ✅ — Pretty-print AST for parser comparison
3. ~~**Add char literal support**~~ ✅ (`'x'` → `i64`) — Makes lexer code readable
4. ~~**Add Map iteration**~~ ✅ — Required for nearly every compiler pass
5. ~~**Add `*method self` by-pointer**~~ ✅ — Required for stateful parser/typer
6. ~~**Create `std/strings.jade`**~~ ✅ with `StringBuilder` type

Each of these is independently valuable and moves toward self-hosting regardless
of timeline.

---

## Appendix A: Comparable Self-Hosting Compilers

| Language | Bootstrap LOC | Strategy | Time to Self-Host |
|----------|-------------|----------|-------------------|
| **Go** | ~50K | Transpile from C → Go | 2 years (2013-2015) |
| **Rust** | ~30K OCaml → Rust | OCaml bootstrap compiler | 3 years (2010-2013) |
| **Zig** | ~80K | C backend for bootstrap | 5+ years (ongoing) |
| **Nim** | ~20K | Pascal bootstrap → Nim | 2 years |
| **D** | ~40K | C backend initially | 3 years |

Jade's ~17K estimated LOC is on the smaller end, which is favorable.

## Appendix B: Jade Syntax Cheat Sheet for Compiler Code

```jade
# Enum with data
enum Token
    Int(i64)
    Plus
    Ident(String)

# Struct with methods
type Lexer
    src: String
    pos: i64

    *peek self
        if self.pos >= self.src.length
            return Eof
        self.src.char_at(self.pos)

    *advance self
        self.pos is self.pos + 1

# Pattern matching
*token_name(t: Token)
    match t
        Int(n) ? "int({n})"
        Plus ? "+"
        Ident(s) ? s

# Error handling
err LexError
    UnexpectedChar(i64, i64)   # char, line
    UnterminatedString(i64)

*lex_string(lexer: Lexer)
    # ... returns LexError on failure via `!`

# File I/O
use std.io
*main()
    source is read_file("input.jade")
    tokens is lex(source)
    # ...
```
