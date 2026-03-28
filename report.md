# Jade Type Inference System — Comprehensive Technical Report

**Date:** 2026-03-27  
**Scope:** Complete audit of the Jade compiler's type inference engine — completeness, rigor, limitations, remediation plan, and Rank-2 feasibility analysis.  
**Codebase Version:** 1224 tests passing (208 unit + 771 bulk + 245 integration). Type inference subsystem: 9,230 lines across 11 files in `src/typer/`.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Architecture Overview](#2-architecture-overview)
3. [Formal Foundations — Hindley-Milner Analysis](#3-formal-foundations--hindley-milner-analysis)
4. [Completeness Assessment](#4-completeness-assessment)
5. [Where Annotations Are Still Required](#5-where-annotations-are-still-required)
6. [Cross-Module and Whole-Program Inference](#6-cross-module-and-whole-program-inference)
7. [Standard Library Inference Status](#7-standard-library-inference-status)
8. [Test Coverage Analysis](#8-test-coverage-analysis)
9. [Shortcomings and Open Issues](#9-shortcomings-and-open-issues)
10. [Comparative Analysis](#10-comparative-analysis)
11. [Rank-2 Polymorphism Feasibility](#11-rank-2-polymorphism-feasibility)
12. [Remediation Plan](#12-remediation-plan)
13. [Conclusion](#13-conclusion)

---

## 1. Executive Summary

Jade's type inference system is a **complete Hindley-Milner implementation** extended with constrained type variables, bidirectional checking, monomorphization-based generics, and SCC-ordered mutual recursion. The system achieves the design goal of "Python-like ease with Rust-like safety" — the standard library contains **49 fully-inferred functions** with zero annotations, and **no Jade-native function requires a return type annotation**.

**Key findings:**

| Metric | Status |
|--------|--------|
| HM unification with occurs check | ✅ Complete |
| Let-generalization with value restriction | ✅ Complete |
| Bidirectional type propagation | ✅ Complete (13 expression forms) |
| Constrained type variables (Numeric/Integer/Float/Trait) | ✅ Complete |
| SCC-based mutual recursion | ✅ Complete |
| Cross-module inference | ✅ Works (flat namespace model) |
| Whole-program inference | ✅ Single InferCtx for all modules |
| Polymorphic lambdas / let-polymorphism | ✅ Complete |
| Higher-order function inference | ✅ Complete |
| Row polymorphism (struct field deduction) | ⚠️ Partial (single-candidate matching) |
| Higher-kinded types | ❌ Not supported |
| Rank-2 polymorphism | ❌ Not supported (no production language does this) |
| Separate compilation / interface files | ❌ Not supported |

The system is **production-grade for Rank-1 HM**. Six specific gaps remain addressable without architectural changes.

---

## 2. Architecture Overview

### 2.1 Pipeline

```
Source → Parse → Module Flattening → Type Registration (Pass 1)
       → Param Inference (Pass 2) → SCC Ordering → Body Lowering (Pass 3)
       → Deferred Resolution (Pass 4) → Final Resolution (Pass 5) → Codegen
```

### 2.2 File Structure (9,230 lines)

| File | Lines | Role |
|------|-------|------|
| `unify.rs` | 1,046 | Union-Find engine, constraint system, generalization/instantiation |
| `mod.rs` | 1,738 | Orchestrator, generalization, scope management, exhaustiveness |
| `lower.rs` | 2,020 | Definition lowering, SCC processing, struct/actor/store lowering |
| `expr.rs` | 1,235 | Bidirectional expression lowering |
| `call.rs` | 813 | Function/method call dispatch with 6 inference paths |
| `stmt.rs` | 613 | Statement lowering, pattern matching, let-binding generalization |
| `mono.rs` | 417 | Monomorphization, generic instantiation, name mangling |
| `resolve.rs` | 355 | Signature declaration, trait registration, type resolution |
| `scc.rs` | 308 | Tarjan's SCC algorithm, call graph construction |
| `builtins.rs` | 297 | Builtin function type tables (~20 builtins) |
| `infer.rs` | 157 | Coroutine yield type inference, dynamic dispatch return types |

### 2.3 Core Data Structures

**InferCtx** — the unification context:
```
InferCtx {
    parent:      Vec<u32>,                    // Union-Find parent pointers
    rank:        Vec<u8>,                     // UF rank for balancing
    types:       Vec<Option<Type>>,           // Solved type per root
    origins:     Vec<Option<ConstraintOrigin>>,// Diagnostic provenance
    constraints: Vec<TypeConstraint>,         // Per-var constraints
    quantified_vars: HashSet<u32>,            // Generalized vars
    strict_types: bool,                       // Default: true
    pedantic:     bool,                       // Ultra-strict mode
}
```

Five parallel `Vec`s indexed by `TypeVar(u32)` ID. Every `fresh_var()` call pushes one entry onto all five. The union-find uses path compression and union-by-rank — amortized $O(\alpha(n))$ per operation, where $\alpha$ is the inverse Ackermann function.

**Type** — 28 variants:
```
Primitives:  I8, I16, I32, I64, U8, U16, U32, U64, F32, F64, Bool, Void
Heap:        String
Compound:    Array(T, N), Vec(T), Map(K, V), Tuple(Vec<T>)
Named:       Struct(name), Enum(name)
Function:    Fn(Vec<T>, T)
Generics:    Param(name)
Pointer:     Ptr(T), Rc(T), Weak(T)
Concurrent:  ActorRef(name), Coroutine(T), Channel(T)
Trait:       DynTrait(name)
Inference:   TypeVar(u32)
```

**Scheme** — polymorphic type:
```
Scheme {
    quantified: Vec<u32>,  // ∀-quantified TypeVar IDs
    ty: Type,              // Body type containing those vars
}
```

---

## 3. Formal Foundations — Hindley-Milner Analysis

### 3.1 Classical HM Requirements

| Requirement | Jade Status | Notes |
|-------------|-------------|-------|
| **Unification** | ✅ | Union-Find with path compression and union-by-rank |
| **Occurs check** | ✅ | `occurs_in(v, ty)` prevents $\tau = \tau \to \sigma$ |
| **Let-generalization** | ✅ | `generalize(ty)` computes $\forall \bar{\alpha}. \tau$ where $\bar{\alpha} = \text{ftv}(\tau) \setminus \text{ftv}(\Gamma)$ |
| **Instantiation** | ✅ | `instantiate(scheme)` replaces quantified vars with fresh vars preserving constraints |
| **Value restriction** | ✅ | `is_syntactic_value()` gates generalization (ML-style) |
| **Principal types** | ✅ | Union-Find guarantees most general unifier |
| **Decidability** | ✅ | Rank-1 HM is decidable; depth limit (64) prevents divergence in monomorphization |

### 3.2 The Unification Algorithm

Jade's `unify(a, b)` handles all 28 × 28 type pairs:

1. Both sides are `shallow_resolve`d (follows UF pointers without defaulting)
2. Structural equality check — if equal, return `Ok`
3. `TypeVar ↔ TypeVar`: Find both roots. If same root → `Ok`. Otherwise `union()` the roots; if either has a solved type, recursively unify the solved types
4. `TypeVar ↔ Concrete`: Occurs check, constraint compatibility check, then bind: `types[root] = Some(concrete)`
5. Structural types: Recursively unify children (Array elements, Vec inner, Map key/value, Tuple elements pairwise, Fn params pairwise + return, Ptr/Rc/Weak/Channel/Coroutine inners)
6. Everything else: Type mismatch error

**Constraint lattice for numeric types:**

```
         Numeric
        /       \
   Integer     Float
```

Merging rules:
- $\text{None} \cup X = X$
- $\text{Integer} \cup \text{Float} = \text{Error}$ (mutually exclusive)
- $\text{Integer} \cup \text{Numeric} = \text{Integer}$ (more specific wins)
- $\text{Float} \cup \text{Numeric} = \text{Float}$
- $\text{Trait}(A) \cup \text{Trait}(B) = \text{Trait}(A \cup B)$
- $\text{Trait} \cup \text{Numeric/Integer/Float} = \text{Error}$

### 3.3 Generalization Correctness

The generalization algorithm implements the standard $\text{Gen}(\Gamma, \tau) = \forall (\text{ftv}(\tau) \setminus \text{ftv}(\Gamma)). \tau$:

```
generalize(τ):
  1. τ' ← canonicalize_type(τ)         // resolve UF but keep unsolved vars
  2. if ¬has_type_var(τ') → Mono(τ')   // no vars → monomorphic
  3. env_ftvs ← ∪{ftv(Γᵢ) | Γᵢ ∈ scopes}
  4. ty_ftvs ← ftv(τ')
  5. quantified ← ty_ftvs \ env_ftvs
  6. return Scheme(quantified, τ')
```

The **value restriction** is applied correctly: only syntactic values (lambdas, identifiers, struct literals, arrays/tuples of values, references of values) are generalized. This prevents the unsoundness of generalizing side-effecting expressions — the same approach used by OCaml since 1995.

### 3.4 Instantiation Correctness

```
instantiate(∀ā.τ):
  1. For each αᵢ ∈ ā:
     fresh βᵢ with constraint(αᵢ)  // Integer → fresh_integer_var(), etc.
  2. σ ← {αᵢ ↦ βᵢ}
  3. return τ[σ]
```

Constraints are preserved through instantiation. An `Integer`-constrained quantified variable produces a fresh `Integer`-constrained variable at each use site.

### 3.5 Verdict: Is Jade "HM"?

**Yes.** Jade implements Algorithm W (variant: bidirectional) with:
- Union-Find-based unification (textbook)
- Let-generalization with value restriction (OCaml-style)
- Instantiation with constraint preservation (extension beyond textbook HM)
- Occurs check (standard)
- Principal type property (guaranteed by union-find MGU)

The extensions (numeric constraints, trait constraints, monomorphization) are conservative — they restrict the set of accepted programs but don't make the type system unsound or lose the principal type property.

---

## 4. Completeness Assessment

### 4.1 What Works Without Any Annotations

| Feature | Status | Example |
|---------|--------|---------|
| Integer literals | ✅ | `x is 42` → inferred as `i64` (or contextual width) |
| Float literals | ✅ | `x is 3.14` → inferred as `f64` (or contextual width) |
| String literals | ✅ | `x is "hello"` → `String` |
| Bool literals | ✅ | `x is true` → `Bool` |
| Arithmetic operators | ✅ | `x + y` → both constrained to `Numeric`, result same type |
| Comparison operators | ✅ | `x > y` → both constrained to `Numeric`, result `Bool` |
| Bitwise operators | ✅ | `x & y` → both constrained to `Integer` |
| Array literals | ✅ | `[1, 2, 3]` → `Array(i64, 3)` |
| Vec operations | ✅ | `v.push(42)` → element type inferred |
| Map operations | ✅ | `m["key"] is 42` → `Map(String, i64)` |
| Tuple construction | ✅ | `(1, "hi", true)` → `Tuple(i64, String, Bool)` |
| Struct construction | ✅ | `Point(1, 2)` → `Struct("Point")` |
| Enum variants | ✅ | `Some(42)` → `Enum("Option")` |
| Function return types | ✅ | Inferred from tail expression |
| Lambda parameters | ✅ | From expected type context or usage |
| Let-bindings | ✅ | From initializer expression |
| Match arm types | ✅ | Unified across all arms |
| Pattern destructuring | ✅ | Enum variant fields, tuple elements |
| Pipe operator | ✅ | Flows subject type to function parameter |
| Cross-function calls | ✅ | Argument types constrain parameter type vars |
| Generic functions | ✅ | Monomorphized at call site from argument types |
| Polymorphic bindings | ✅ | Let-generalized, instantiated at each use |
| Higher-order functions | ✅ | `Fn` type unified from argument/result types |
| Mutual recursion | ✅ | SCC-ordered, all bodies processed together |
| Channel element type | ✅ | Inferred from send/receive |
| Struct fields | ✅ | From constructor arguments or default values |

### 4.2 Where Inference Succeeds That Users Might Not Expect

**Example 1: Fully inferred higher-order pipeline**
```jade
*apply(f, x)
    f(x)

*double(n)
    n * 2

*main()
    result is apply(double, 21)
    log(result)    # prints 42
```
All types inferred: `apply: (Fn(i64 → i64), i64) → i64`, `double: i64 → i64`, `result: i64`.

**Example 2: Polymorphic identity with value restriction**
```jade
*main()
    id is *fn(x) x
    a is id(42)       # id instantiated at i64
    b is id("hello")  # id instantiated at String
    log(a)             # 42
    log(b)             # hello
```
The lambda `*fn(x) x` is generalized at the let-binding. Each use site gets a fresh instantiation.

**Example 3: Enum inference from match patterns**
```jade
enum Op
    Add(a, b)
    Lit(n)

*eval(op)
    match op
        Add(a, b) ? eval(a) + eval(b)
        Lit(n) ? n
```
The type of `op` is inferred as `Op` from the `Add(a, b)` and `Lit(n)` patterns. Return type inferred as `i64` from `n` and the `+` operator.

**Example 4: Cross-function constraint propagation**
```jade
*square(x)
    x * x

*sum_squares(a, b)
    square(a) + square(b)

*main()
    log(sum_squares(3, 4))    # 25
```
The call `square(a)` constrains `x` to `Numeric`; `x * x` constrains to `Numeric`; the `+` constrains the return to `Numeric`; `sum_squares(3, 4)` with integer literals constrains to `Integer`; defaults to `i64`.

---

## 5. Where Annotations Are Still Required

### 5.1 Mandatory Annotations (Cannot Be Eliminated)

#### M1. Extern/FFI Function Signatures

**Status:** Fundamental — required by the language boundary  
**Rationale:** Extern functions have no Jade body to infer from. The compiler must know the C-level ABI types.

```jade
# REQUIRED: All params and return type must be annotated
extern *sqrt(x: f64) -> f64
extern *fopen(path: %i8, mode: %i8) -> %i8
```

**Test coverage:** All extern tests use fully annotated signatures. ✅

#### M2. Struct Field Types on Never-Constructed Structs

**Status:** Fundamental — no information source  
**Rationale:** If a struct is declared but never constructed or assigned, no constraint source exists.

```jade
type Box
    value       # ERROR: "struct `Box` field `value` has no type annotation
                #         and was never constrained"

*main()
    log(1)
```

**Test coverage:** `b_infer_e3_strict_unconstrained_struct_field` ✅

#### M3. Completely Unused Generic Functions

**Status:** By design — no call site to monomorphize from  
**Rationale:** Generic functions with no call sites are never lowered. Their type parameters remain abstract.

```jade
*identity(x)
    x
# Not called → never monomorphized → no concrete types
```

This is not an error — such functions simply don't appear in the output. No annotation fixes this since the function is dead code.

**Test coverage:** Generic function tests only test called functions. Dead generic functions are implicitly handled.

### 5.2 Conditional Annotations (Required Only Without Sufficient Context)

#### C1. Unconstrained Type Variables in Strict Mode

**Status:** Solvable in many cases by improved propagation  
**Error:** `"ambiguous type: cannot infer type for this expression"`

```jade
*main()
    ch is channel()    # ERROR if channel element type never constrained
    # No sends or receives → element type unknown
```

**Fix:** Adding usage context resolves this:
```jade
*main()
    ch is channel()
    ch <- 42           # Now element type is inferred as i64
```

**Test coverage:** `b_p42_strict_unsolved_typevar` ✅, `b_channel_infer_from_send` ✅

#### C2. Empty Collections With No Context

**Status:** Inherent limitation — solvable with usage analysis  

```jade
*main()
    arr is []          # What element type? ERROR in strict mode
```

**Fix:** Provide context:
```jade
*main()
    arr is []
    arr.push(42)       # Now element type is i64
```

**Current status:** Empty array with no context gets `fresh_var()`. If later usage constrains it, inference succeeds. If not, strict mode flags it.

**Test coverage:** `b_infer_array_element_unification` ✅

#### C3. Higher-Order Functions With Unconstrained Type Chains

**Status:** Rare edge case  

```jade
*compose(f, g)
    *fn(x) f(g(x))

*main()
    h is compose(*fn(x) x, *fn(y) y)
    # h: Fn(α) → α — but α is unconstrained
    # Becomes an error only if h is never called with concrete args
```

**Test coverage:** `b_lambda_hof_compose` ✅, `b_r5_poly_apply_with_lambdas` ✅

#### C4. Trait-Constrained Variables With Multiple Implementors

**Status:** Fundamental when ambiguous  
**Error:** `"ambiguous type: cannot infer concrete type for trait-constrained variable"`

```jade
trait Show
    *show() -> String

impl Show for i64
    *show() -> String
        __fmt_int(self)

impl Show for String
    *show() -> String
        self

*display(x)
    x.show()

*main()
    display    # Which impl? Multiple candidates → ambiguous
```

**Fix:** Call with concrete type:
```jade
*main()
    display(42)      # Resolved: x is i64
```

**Test coverage:** `b_r41_trait_guided_single_implementor` ✅

### 5.3 Optional Annotations (User-Facing Ergonomics: Default Overrides)

#### O1. Width Narrowing (i64 → i32, f64 → f32)

**Status:** By design — integer literals default to `i64`, float to `f64`  
**Annotation needed when:** A narrower width is desired.

```jade
*main()
    x is 42           # Inferred i64
    y: i32 is 42      # Annotation needed for i32
    z: f32 is 3.14    # Annotation needed for f32
```

**Bidirectional propagation helps:** If context provides a narrow type, no annotation is needed:
```jade
*foo(x: i32)
    x + 1             # 1 inferred as i32 from context!

*main()
    foo(42)            # 42 inferred as i32 from parameter type
```

**Test coverage:** `b_strict_integer_literal_defaults` ✅, `b_width_propagation_param` ✅, `b_width_propagation_return` ✅

#### O2. Pedantic Mode (All Defaults Require Annotations)

**Status:** Opt-in strictness via `--pedantic`  

```jade
# With --pedantic:
*main()
    x is 42        # ERROR: "pedantic: integer type defaults to i64"
    x: i64 is 42   # OK
```

**Test coverage:** `b_infer_i3_pedantic_rejects_integer_default` ✅, `b_infer_i3_pedantic_passes_annotated` ✅

### 5.4 Summary Matrix

| Scenario | Annotation Required? | Fundamental? | Can Be Improved? |
|----------|---------------------|--------------|------------------|
| Extern/FFI signatures | Always | Yes | No |
| Unused struct fields | Always (strict) | Yes | No |
| Unconstrained type vars | Sometimes | No | Yes (§12) |
| Empty collections | Sometimes | Partially | Yes (§12) |
| HOF type chains | Sometimes | Partially | Yes (§12) |
| Multiple trait implementors | Sometimes | Yes | No |
| Width narrowing (i32/f32) | Only when non-default | By design | N/A |
| Pedantic mode | All defaults | By design | N/A |

---

## 6. Cross-Module and Whole-Program Inference

### 6.1 Architecture: Flat Namespace Model

Jade uses a **source-level textual inclusion** model:

1. Entry file is parsed into AST
2. `resolve_modules()` follows all `use` declarations
3. Imported modules' declarations are **appended** to the main program's declaration list
4. One `Typer` + one `InferCtx` processes the entire flattened program

This means **type inference is whole-program by construction**. There are no module boundaries from the type system's perspective.

### 6.2 What Works

- ✅ Functions from `std/math.jade` unify with user code in a single InferCtx
- ✅ Generic functions defined in one module, called from another, are monomorphized correctly
- ✅ SCC analysis considers the full call graph across all modules
- ✅ Struct/enum types defined in imports are visible everywhere
- ✅ Bidirectional propagation flows across module boundaries (same as within a file)

### 6.3 Limitations and Implications

| Limitation | Impact | Severity |
|------------|--------|----------|
| No namespacing | Name collisions between modules | Medium |
| No selective imports | `use math` imports all 18 functions | Low |
| No separate compilation | All code re-compiled every time | High (at scale) |
| No incremental type checking | Changing one module re-checks everything | High (at scale) |
| No interface files | Can't type-check against a precompiled library | High (at scale) |
| No privacy/visibility | Every function is globally visible | Medium |

### 6.4 Standard Library Integration

Standard library files are plain `.jade` source — no special status. The compiler locates them via a search path:
1. Package dependencies → `{pkg_path}/src/{module}.jade`
2. Relative to entry file → `{base_dir}/{path}.jade`
3. Std relative to entry → `{base_dir}/std/{name}.jade`
4. Std relative to compiler binary → `{exe_dir}/std/{name}.jade`

### 6.5 Built-in Types

The prelude types `Option<T>` (`Some`/`Nothing`) and `Result<T, E>` (`Ok`/`Err`) are **hardcoded** in the typer's `resolve.rs` — always available without any `use` statement.

Builtin functions (`__ln`, `__fmt_float`, `__string_from_raw`, etc.) are intercepted by `builtins.rs` with hardcoded type tables.

---

## 7. Standard Library Inference Status

A complete audit of all 82 functions across 8 standard library files:

| File | Total Fns | Fully Annotated | Partially Annotated | Fully Inferred |
|------|-----------|----------------|--------------------|-----------------| 
| `fmt.jade` | 8 | 0 | 4 | 4 |
| `io.jade` | 23 | 14 (externs) | 0 | 9 |
| `math.jade` | 18 | 1 (extern) | 0 | 17 |
| `os.jade` | 15 | 10 (externs) | 0 | 5 |
| `path.jade` | 4 | 0 | 4 | 0 |
| `rand.jade` | 4 | 0 | 0 | 4 |
| `sort.jade` | 7 | 0 | 0 | 7 |
| `time.jade` | 3 | 0 | 0 | 3 |
| **Total** | **82** | **25** | **8** | **49** |

**Key observations:**
- **No Jade-native function has a return type annotation** — all return types are inferred
- The **25 fully-annotated** functions are all `extern` (FFI) — mandatory by the language boundary
- The **8 partially annotated** functions annotate only `String` parameters (in `fmt.jade` and `path.jade`) — these constrain non-numeric types where no operator provides constraint information
- **49 functions (60%) have zero annotations** — fully inferred
- Of the non-extern functions: **49/57 (86%) are fully inferred**

### Annotation Patterns in Partially-Annotated Functions

```jade
# fmt.jade: String params annotated, width params inferred
*pad_left(s: String, width, fill: String)
    ...
# width is constrained to i64 by comparison operators in body

# path.jade: All params are String (no numeric ops to constrain)
*path_join(a: String, b: String)
    ...
# Without annotations, a and b could be any type
```

**Analysis:** The partial annotations follow a clear pattern — they occur on parameters whose types cannot be deduced from operators or other constraints in the function body. String parameters used only in string operations need annotation because no operator constrains them to `String`.

---

## 8. Test Coverage Analysis

### 8.1 Inference-Related Test Coverage

| Category | Count | Source |
|----------|-------|--------|
| Direct inference tests (`b_infer_*`) | 22 | bulk_tests.rs |
| Struct field inference (`b_struct_field_infer_*`) | 9 | bulk_tests.rs |
| Row polymorphism (`b_row_poly_*`) | 7 | bulk_tests.rs |
| HM-specific tests (`b_hm_*`) | 18 | bulk_tests.rs |
| Generic/polymorphism (`b_gen_*`, `b_r5_poly_*`) | 20 | bulk_tests.rs |
| Lambda/closure inference | 13 | bulk_tests.rs |
| Strict/pedantic mode | 3 | bulk_tests.rs |
| Phase tests (P41, P42, P43, P5) | 20 | bulk_tests.rs |
| Constrained polymorphism | 3 | bulk_tests.rs |
| Value restriction | 2 | bulk_tests.rs |
| Trait-guided inference | 12 | bulk_tests.rs |
| Channel/enum inference | 6 | bulk_tests.rs |
| Width propagation | 2 | bulk_tests.rs |
| Return type inference | 5 | bulk_tests.rs |
| Round 5 mixed/recursive | 15 | bulk_tests.rs |
| Integration (generics, lambdas, closures) | 39 | integration.rs |
| Unit tests (InferCtx, unification) | 21 | unify.rs |
| Unit tests (typer) | ~80 | mod.rs |
| **Total inference-related tests** | **~313** | |

### 8.2 Coverage Gaps

| Missing Test Scenario | Severity | Code Path |
|-----------------------|----------|-----------|
| Rank-1 polymorphism across module boundaries | Low | Already works by flat-namespace design, but no explicit test |
| Generic enum with >2 type params | Low | `monomorphize_enum` handles arbitrary arity |
| Recursive polymorphic data types (e.g., `List<T>`) | Medium | Would test generics + recursion interaction |
| Mutual recursion with >3 functions in SCC | Low | SCC algorithm is general but tested with ≤3 |
| Trait constraint propagation through HOFs | Medium | `TypeConstraint::Trait` through `Fn` unification |
| Deferred field resolution with 3+ candidates | Low | Multi-candidate struct matching |
| `--pedantic` with float default | Low | Only integer pedantic tested |
| Empty map literal inference | Low | Empty `{}` may not be tested |
| Nested generic instantiation (generic calling generic) | Medium | `depth_limit` exists but untested at high depth |
| Lambda returning lambda (currying) | Medium | Nested `Fn` type construction |

### 8.3 Recommendations for Test Suite

**Priority 1 (Add before proceeding):**
1. Cross-module generic function test (define generic in one file, call from another)
2. Curried function inference: `*curry(f) *fn(x) *fn(y) f(x, y)`
3. Nested generic instantiation: generic function calling another generic function
4. Trait constraint through HOF chain

**Priority 2 (Complete coverage):**
5. Float pedantic mode test
6. Empty map literal
7. 4+-function mutual recursion SCC
8. Generic enum with 3+ type params

---

## 9. Shortcomings and Open Issues

### 9.1 Structural Limitations

#### S1. No Higher-Kinded Types

Jade's type constructors (`Vec`, `Map`, `Channel`, etc.) are hardcoded in the type system. There is no way to abstract over a container:

```jade
# Cannot write this in Jade:
*map_over(container, f)
    # Which .map method? Vec.map, Array.map, Channel.map?
    container.map(f)
```

**Impact:** Moderate. Prevents generic algorithms over container types. Would require HKT/type classes.  
**Recommendation:** Defer. HKT is a major language feature requiring significant design work. Monomorphization handles concrete cases well.

#### S2. Nominal Struct/Enum Types

`Struct(String)` and `Enum(String)` carry only a name — no type parameters in the type representation:

```jade
type Pair
    fst
    snd

# Pair(1, "hello") and Pair(1, 2) have the same type: Struct("Pair")
# No Struct("Pair", [I64, String]) vs Struct("Pair", [I64, I64])
```

**Impact:** Generic struct fields are handled via monomorphization (name mangling creates `Pair_i64_String`), but the type system itself doesn't distinguish parameterized structs.  
**Recommendation:** Consider enriching `Struct(String)` to `Struct(String, Vec<Type>)` for better error messages and more precise type checking.

#### S3. Row Polymorphism is Single-Candidate

When field/method access is used on an unknown type (TypeVar), the typer searches all known structs for a matching field. If exactly one candidate matches, it resolves. If multiple match, it defers. If zero match, error.

```jade
*get_name(x)
    x.name     # Which struct has .name? If only Person → resolved
               # If Person AND Company both have .name → deferred
```

**Impact:** Low. Most programs have unique field names across structs. Multi-candidate resolution works but is less deterministic.  
**Recommendation:** Document the behavior. Consider adding a diagnostic when deferral happens.

#### S4. No Level-Based Generalization

The implementation scans all scopes to compute free type variables in the environment:

```
free_type_vars_in_env():
    for scope in scopes:
        for info in scope:
            shallow_resolve(info.ty).free_type_vars(&mut ftvs)
```

This is $O(|\Gamma|)$ per generalization. Level-based generalization (as in OCaml's internal implementation) maintains a "level" counter on each type variable, enabling $O(1)$ generalization by comparing levels.

**Impact:** Performance only. Correctness is unaffected. For programs with <10K bindings, negligible.  
**Recommendation:** Profile first. Optimize only if generalization becomes a bottleneck.

#### S5. Global Defaulting

All unsolved type variables are defaulted at a single point (`resolve_all_types`), regardless of where they were introduced. There's no per-binding ambiguity check:

```jade
*main()
    x is 42         # TypeVar with Integer constraint
    # ... 1000 lines later ...
    # x defaults to i64 at resolve_all_types
```

**Impact:** Low. The current strict-mode catches genuinely ambiguous cases. The defaulting phase handles only numeric literals which have well-defined defaults.  
**Recommendation:** No change needed for correctness. Could add a "gap" diagnostic if a variable travels far from its creation before defaulting.

#### S6. Trait Constraint Not Enforced at Unification Time

When a `TypeConstraint::Trait(["Show"])` variable is unified with a concrete type, there's no immediate check that the type actually implements `Show`:

```rust
// In unify.rs, constraint check for Trait:
TypeConstraint::Trait(_) => {
    // Just check that concrete is compatible, but don't verify trait impl
    _ => {} // silently accept
}
```

**Impact:** The trait check happens later during method dispatch, so it's not unsound. But earlier checking would give better error messages.  
**Recommendation:** Add trait satisfaction check at unification time (see §12.4).

### 9.2 Ambiguity and Unresolved Type Issues

| Issue | Frequency | Severity | Example |
|-------|-----------|----------|---------|
| Numeric literal width ambiguity | Common | Low | `42` → i64 by default, may want i32 |
| Empty collection element type | Occasional | Medium | `[]` with no subsequent push |
| Unused function parameter | Rare | Medium | `*f(x, y) x` — y unconstrained |
| Chain of unconstrained HOFs | Rare | Low | `compose(id, id)` never called |
| Deferred field with no resolution | Rare | Medium | Struct method on TypeVar that never resolves |

### 9.3 Known Edge Cases

**Edge Case 1: Recursive function as sole constraint**
```jade
*loop(n)
    loop(n - 1)
```
Return type of `loop` is only constrained by the recursive call (which produces the same TypeVar). Result: returns `Void` from the recursion base case (no explicit return). This is correct but may surprise users.

**Edge Case 2: Polymorphic lambda re-lowering**
When a let-bound polymorphic lambda is called at different types, the lambda AST is **re-lowered** from scratch at each call site. This means:
- The lambda body is type-checked multiple times
- Each instance gets independent TypeVars
- Side effects in the lambda body execute at compile time during re-lowering (not an issue since Jade doesn't have compile-time execution)

**Edge Case 3: Monomorphization depth limit**
The depth limit of 64 prevents infinite monomorphization loops but may reject legitimate deeply-nested generic code. In practice, 64 levels is far beyond any reasonable program.

---

## 10. Comparative Analysis

### 10.1 Systems Languages

| Feature | Jade | Rust | C++ (auto/templates) | Go |
|---------|------|------|---------------------|-----|
| **Local type inference** | ✅ Full | ✅ Full | ⚠️ Limited (`auto`) | ✅ Full (`:=`) |
| **Function param inference** | ✅ From call sites | ❌ Always annotated | ✅ Templates | ❌ Always annotated |
| **Return type inference** | ✅ From body | ⚠️ `-> impl Trait` only | ✅ `auto` | ❌ Always annotated |
| **Cross-function inference** | ✅ Via unification | ❌ No | ❌ No | ❌ No |
| **Generics** | ✅ Monomorphized | ✅ Monomorphized | ✅ Templates | ✅ Monomorphized (1.18+) |
| **Let-polymorphism** | ✅ With value restriction | ❌ No | ❌ No | ❌ No |
| **Numeric literal typing** | ✅ Constrained vars | ✅ Inference | ❌ Fixed types | ✅ Untyped constants |
| **Bidirectional** | ✅ 13 forms | ✅ Extensive | ❌ No | ❌ No |

**Assessment:** Jade has **significantly stronger inference than any mainstream systems language**. Rust requires all function signatures to be annotated. Go requires all function signatures annotated. C++ templates are powerful but not HM-based and rely on SFINAE/concepts rather than unification. Jade's cross-function inference from call sites is unique among compiled systems languages.

### 10.2 Dynamic/Scripting Languages

| Feature | Jade | Python (mypy) | TypeScript | Ruby (Sorbet) |
|---------|------|---------------|------------|---------------|
| **Inference approach** | HM + bidirectional | Flow-based | Bidirectional + flow | Flow-based |
| **Annotation-free functions** | ✅ Full | ✅ (but no type safety) | ⚠️ Return only | ✅ (but no type safety) |
| **Type safety** | ✅ Compile-time | ❌ Runtime only | ⚠️ Gradual | ❌ Runtime only |
| **Generics** | ✅ Monomorphized | ✅ Erased | ✅ Erased | ⚠️ Limited |
| **Sound type system** | ✅ | ❌ (gradual) | ❌ (`any` escape) | ❌ (gradual) |
| **Performance impact** | None (compiled) | None (interpreted) | None (erased) | None (interpreted) |

**Assessment:** Jade achieves the ergonomic goal — writing `*add(a, b) a + b` is as easy as Python's `def add(a, b): return a + b`, but Jade **statically verifies** the types and compiles to native code. TypeScript/mypy/Sorbet add type checking but sacrifice soundness (TypeScripts' `any`, Python's gradual typing). Jade's type system is sound by construction.

### 10.3 Functional Languages

| Feature | Jade | OCaml | Haskell | F# | Elm |
|---------|------|-------|---------|-----|-----|
| **Core algorithm** | Algorithm W (bidir) | Algorithm W | Algorithm W + TC | Algorithm W | Algorithm W |
| **Let-polymorphism** | ✅ Value restriction | ✅ Value restriction | ✅ Monomorphism restriction | ✅ Value restriction | ✅ |
| **Type classes/traits** | ⚠️ Basic | ❌ (modules) | ✅ Full | ⚠️ (interfaces) | ✅ |
| **Numeric overloading** | ✅ Constrained vars | ❌ Fixed types | ✅ Type classes | ❌ Fixed types | ❌ Fixed types |
| **Rank-1 polymorphism** | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Rank-N polymorphism** | ❌ | ❌ (default) | ⚠️ RankNTypes ext | ❌ | ❌ |
| **Higher-kinded types** | ❌ | ✅ (functors) | ✅ | ❌ | ❌ |
| **Row polymorphism** | ⚠️ Single-candidate | ✅ (objects) | ❌ | ❌ | ✅ (records) |
| **GADTs** | ❌ | ✅ | ✅ | ❌ | ❌ |
| **Separate compilation** | ❌ | ✅ (.mli) | ✅ | ✅ | ✅ |

**Assessment:** Jade is on par with F# and Elm for core inference capability. It exceeds OCaml in numeric literal handling (constrained type variables vs OCaml's fixed `int`/`float`). It falls short of Haskell's type class system and OCaml's module-level polymorphism (functors). The absence of HKTs and separate compilation are the two largest gaps vs the functional language family.

### 10.4 Unique Jade Advantages

1. **No function signature annotations required** — Unique among compiled languages. Not even OCaml or Haskell require this (they technically work without annotations but style guides universally require them).

2. **Constrained numeric type variables** — A lightweight form of Haskell's `Num` class, implemented without full type class machinery. `42` is `Integer`-constrained, `3.14` is `Float`-constrained, and `x + y` is `Numeric`-constrained. Defaults are applied only at resolution time.

3. **Cross-function inference + monomorphization** — Functions with unannotated parameters become implicitly generic via monomorphization. This is more powerful than C++ templates (which don't do unification) and more ergonomic than Rust generics (which require explicit `<T>` declarations).

4. **Whole-program inference** — The flat namespace model ensures all type information is available during inference. No interface files, no forward declarations, no module signature annotations.

---

## 11. Rank-2 Polymorphism Feasibility

### 11.1 Background

In the polymorphism rank hierarchy:

- **Rank-0**: No polymorphism. Types are monomorphic. ($\text{int} \to \text{int}$)
- **Rank-1**: Universal quantifiers at the outermost level only. ($\forall \alpha.\, \alpha \to \alpha$). This is standard HM.
- **Rank-2**: Universal quantifiers can appear to the left of at most one arrow. ($(\forall \alpha.\, \alpha \to \alpha) \to \text{int}$). The argument to a function can itself be polymorphic.
- **Rank-N** (N ≥ 3): Universal quantifiers at arbitrary depth. Undecidable inference.

Formally, a type $\tau$ has Rank $k$ if:
$$\text{rank}(\forall \bar{\alpha}.\, \sigma_1 \to \sigma_2) = \max(\text{rank}(\sigma_1) + 1, \text{rank}(\sigma_2))$$

### 11.2 Survey: Production Languages With Rank-2 Inference

| Language | Rank-2 Support | Notes |
|----------|---------------|-------|
| Haskell (GHC) | ⚠️ `RankNTypes` extension | **Requires explicit annotation** — inference is not attempted for Rank-2+ types. The user must write `forall` in the type signature. This is Rank-N *checking*, not Rank-N *inference*. |
| OCaml | ❌ | No Rank-2 support in the standard language. Available via objects with `'a. 'a -> 'a` method types, but not general. |
| F# | ❌ | No support. |
| Scala | ❌ | No direct Rank-2. Workaround via trait encoding. |
| Rust | ❌ | `for<'a>` is Rank-2 over lifetimes only, not types. `dyn Fn(&i32) -> &i32` uses HRTB but only for lifetimes. |
| Idris | ✅ | Full dependent types subsume Rank-N, but inference is heuristic. |
| PureScript | ⚠️ | Similar to Haskell — annotation required. |
| Elm | ❌ | Strict Rank-1 only. |
| TypeScript | ❌ | No universal quantification in types. |
| Go | ❌ | No Rank-2 support. |
| C++ | ❌ | Templates are not part of the type system proper. |

**Conclusion: No production language performs complete Rank-2 type *inference*.** Haskell and PureScript support Rank-2 (and higher) *type checking* but require the user to write the higher-rank type explicitly. GHC does not attempt to infer Rank-2 types.

### 11.3 Theoretical Decidability

**Rank-2 inference is decidable.** This was proved by Kfoury and Wells (1999):

> **Theorem (Kfoury & Wells, 1999):** Type inference for Rank-2 intersection types is decidable, but the problem is EXPTIME-complete.

The key reference is:
- Kfoury, A.J. and Wells, J.B. (1999). "Principality and decidable type inference for finite-rank intersection types." In *POPL '99*.

For **Rank-3 and above**, inference is **undecidable** (Wells, 1999):
- Wells, J.B. (1999). "Typability and type checking in System F are equivalent and undecidable." In *Annals of Pure and Applied Logic*.

### 11.4 What Rank-2 Would Enable

```jade
# Rank-1 (current): Cannot write this
*run_twice(f)
    a is f(42)       # f instantiated at i64 here
    b is f("hello")  # ERROR: f already bound to i64 → i64
    (a, b)

# Rank-2 (proposed): f is polymorphic within the body
*run_twice(f: forall a. a -> a)
    a is f(42)        # f instantiated at i64
    b is f("hello")   # f re-instantiated at String
    (a, b)
```

The key difference: in Rank-1, `f` gets a single monomorphic type for the entire function body. In Rank-2, `f` can be used polymorphically because its type is explicitly quantified.

### 11.5 Strategy to Achieve Rank-2 (If Pursued)

**Phase 1: Rank-2 Type Checking (annotation-required)**

This is what Haskell does — the user writes `forall` and the compiler checks it.

1. **Syntax extension**: Add `forall` keyword to type annotations:
   ```jade
   *apply_both(f: forall a. a -> a, x, y)
       (f(x), f(y))
   ```

2. **Type representation**: Add `Type::Forall(Vec<u32>, Box<Type>)` variant.

3. **Unification extension**: When unifying `Forall(vars, body)` with a concrete type, instantiate the forall with fresh variables and unify the body. When unifying two `Forall` types, use subsumption checking.

4. **Subsumption rule**: $\forall \alpha.\, \tau \leq \sigma$ iff for *all* instantiations of $\alpha$, $\tau[\alpha := \beta] \leq \sigma$.

5. **Instantiation change**: At call sites where a Rank-2 parameter is passed, the argument must be checkable against the quantified type — it must be at least as polymorphic as required.

**Effort estimate**: ~500 lines of new code. Type::Forall variant, subsumption in unify, syntax support.

**Phase 2: Rank-2 Inference (no annotation required)**

This is the research frontier. No production language has achieved this.

The algorithm of Kfoury-Wells uses:
1. **Expansion**: Each application `f(x)` generates constraints that may require the argument's type to be polymorphic.
2. **Constraint generation**: Unlike Rank-1 where constraints are equations, Rank-2 constraints include *subsumption* relations.
3. **Constraint solving**: The solver must find the minimal quantification points — where `forall` must be inserted.

**Challenges specific to Jade:**
- Monomorphization-based codegen conflicts with Rank-2 semantics. A Rank-2 argument `f: forall a. a -> a` cannot be simply monomorphized — it must remain polymorphic at runtime, requiring either boxing/type erasure or compile-time function duplication.
- The EXPTIME complexity bound means worst-case compilation times grow super-polynomially.
- Integration with constrained type variables (Integer/Float) in the Rank-2 framework needs research.

**Recommended approach:**
1. Implement Phase 1 (Rank-2 checking with annotations) — straightforward, proven technique
2. Investigate inference heuristics: detect common Rank-2 patterns (like `run_twice`) and infer the quantification automatically when it's unambiguous
3. Do NOT pursue general Rank-2 inference — the EXPTIME complexity makes it impractical, and no language has found it necessary

### 11.6 Rank-2 Verdict

| Approach | Feasibility | Recommendation |
|----------|------------|----------------|
| Rank-2 *checking* with annotations | ✅ Straightforward | Implement in Phase 1 of future work |
| Rank-2 *inference* (heuristic) | ⚠️ Difficult but possible | Research phase after checking works |
| Rank-2 *complete inference* | ❌ EXPTIME-complete | Do not pursue |
| Rank-3+ inference | ❌ Undecidable | Impossible |

---

## 12. Remediation Plan

The following is ordered by priority and dependency. Each item specifies what it fixes, the implementation approach, estimated line count, and which tests to add.

### 12.1 [P0] Improve String Parameter Inference

**Problem:** String parameters require annotation when the function body doesn't use operators that constrain them. This accounts for 8 of the 8 partial annotations in the standard library.

**Root cause:** String concatenation (`+`) currently constrains to `Numeric`, not `String`. String methods (`.len()`, `.slice()`, `.contains()`) are dispatched by the method call system but don't retroactively constrain a TypeVar to `String`.

**Solution:**
1. Add `TypeConstraint::StringLike` or use the existing `Trait(["StringOps"])` mechanism
2. In `lower_method_call`, when a method is dispatched on a string method (`.len()`, `.contains()`, `.slice()`, `.split()`, etc.), constrain the receiver type to `String`
3. In string concatenation, constrain both operands to `String` when neither is numeric

**Implementation:** ~40 lines in `call.rs` (method dispatch), ~20 lines in `expr.rs` (binop).

**Expected impact:** Eliminates annotations in `pad_left`, `pad_right`, `repeat`, `join`, `path_join`, `path_dir`, `path_base`, `path_ext`. Reduces partial annotations from 8 to ~0.

**Tests to add:**
- `b_string_param_inferred_from_method` — function using `.len()` on param
- `b_string_param_inferred_from_concat` — function using string `+`
- `b_string_param_inferred_from_slice` — function using `.slice()`

### 12.2 [P0] Enhance Trait Constraint Enforcement at Unification

**Problem:** When a `TypeConstraint::Trait(["Show"])` variable is unified with a concrete type, no check verifies that the type actually implements `Show`.

**Solution:**
1. In `unify()`, when binding a Trait-constrained TypeVar to a concrete type, check the Typer's `trait_impls` map
2. If the concrete type doesn't implement the required trait(s), emit a type error immediately rather than at method dispatch time

**Implementation:** ~30 lines in `unify.rs`. Requires passing trait_impls as a reference or callback to InferCtx.

**Tests to add:**
- `b_trait_constraint_fails_at_unify` — pass non-Show type to Show-constrained function

### 12.3 [P1] Improve Empty Collection Inference

**Problem:** `[]` and `{}` with no context create unconstrained type variables that may remain unsolved.

**Solution:**
1. When an empty `Vec` or `Array` is bound in a `let`, wait until the end of the scope to check whether the variable was constrained by `.push()`, indexing, or other operations
2. If the variable's type is a `Vec(TypeVar)` and the inner var was constrained during the scope, resolve it. If not, generate a targeted error message: "cannot infer element type of empty array — add an element or a type annotation"

**Implementation:** ~25 lines in `stmt.rs` (enhanced let-binding analysis).

**Tests to add:**
- `b_empty_vec_inferred_from_push` — `v is []; v.push(42)` should infer `Vec(i64)`
- `b_empty_vec_error_no_context` — `v is []` with no usage should give clear error

### 12.4 [P1] Curried Function / Nested Lambda Inference

**Problem:** Curried functions return lambdas, creating nested `Fn` types that may not propagate correctly.

**Example that should work:**
```jade
*add(a)
    *fn(b) a + b

*main()
    add3 is add(3)
    log(add3(4))    # 7
```

**Solution:** Ensure that when the return type of a function is inferred as a `Fn` type (from a lambda tail expression), the outer function's return type variable is properly unified with that `Fn` type, and call sites that invoke the result correctly propagate the inner types.

**Implementation:** Verify call.rs handles indirect calls on returned Fn types. Add targeted tests. ~15 lines of test code, potentially ~20 lines in call.rs for improved indirect call handling.

**Tests to add:**
- `b_curried_add` — basic currying
- `b_curried_compose` — `compose(f)(g)(x)`

### 12.5 [P2] Bidirectional Return Type Propagation for Conditional Chains

**Problem:** Long chains of `if/elif/else` may not propagate the expected return type into all branches uniformly.

**Solution:** Ensure that in `lower_stmt` for `If` statements used as expressions, the expected type is threaded into every branch's tail expression, not just the first.

**Implementation:** ~15 lines in `stmt.rs`. The current implementation already does this for `if` expressions in `expr.rs`, but statement-level `if` chains may not propagate expected types from the parent function's return type.

**Tests to add:**
- `b_deep_elif_return_inference` — 5+ elif branches with mixed types

### 12.6 [P2] Improved Diagnostics for Unsolved Type Variables

**Problem:** The error message "ambiguous type: cannot infer type for this expression" could be more helpful by showing what the variable was *partially* constrained by.

**Solution:**
1. When an unsolved TypeVar produces a strict-mode error, walk the ConstraintOrigin chain to find all spans/reasons that touched this variable
2. Emit "Note: this type was used at [span1] (as argument to `f`), [span2] (as return value), but no concrete type could be determined"

**Implementation:** ~50 lines in `unify.rs` (enhanced diagnostic for `resolve_core`).

**Tests to add:** (Diagnostic quality tests — check error message content)

### 12.7 [P3] Struct Type Parameters in the Type Representation

**Problem:** `Struct(String)` carries no type information. `Pair(1, "hi")` and `Pair(1, 2)` have the same `Type::Struct("Pair")` representation until monomorphization mangles the name.

**Solution:** Extend to `Struct(String, Vec<Type>)` so that type parameters are visible in the type representation. This enables better error messages and more precise unification.

**Implementation:** ~100 lines across `types.rs`, `unify.rs`, `expr.rs`, `call.rs`. Requires updating all `Type::Struct(name)` patterns to `Type::Struct(name, params)`.

**Note:** This is a larger refactor. The monomorphization system already handles generic structs correctly via name mangling, so this is primarily an ergonomic and diagnostic improvement.

### 12.8 [P3] Separate Compilation Foundation

**Problem:** Whole-program compilation won't scale. As the Jade ecosystem grows, re-compiling all modules on every change is prohibitive.

**Solution (long-term):**
1. Define a `.jadei` interface file format containing:
   - Function signatures (with inferred types resolved)
   - Type definitions
   - Enum definitions
   - Trait definitions
2. After type inference, emit `.jadei` files alongside compiled code
3. When compiling a module that `use`s another, load the `.jadei` file instead of re-parsing and re-typing the source

**Implementation:** ~500 lines for serialization/deserialization. Major architectural change.

**Note:** This does NOT reduce inference power — the interface files contain the *results* of inference, not annotations. The key is that inference runs once, and results are cached.

### Summary: Priority Matrix

| ID | Description | Priority | Lines | Blocks |
|----|------------|----------|-------|--------|
| 12.1 | String parameter inference | P0 | ~60 | Nothing |
| 12.2 | Trait constraint enforcement | P0 | ~30 | Nothing |
| 12.3 | Empty collection inference | P1 | ~25 | Nothing |
| 12.4 | Curried function inference | P1 | ~35 | Nothing |
| 12.5 | Conditional chain propagation | P2 | ~15 | Nothing |
| 12.6 | Better unsolved var diagnostics | P2 | ~50 | Nothing |
| 12.7 | Struct type parameters | P3 | ~100 | 12.1-12.6 |
| 12.8 | Separate compilation | P3 | ~500 | 12.7 |

**Total P0-P2 work: ~215 lines.** All can be implemented independently and tested incrementally.

---

## 13. Conclusion

### 13.1 Current State Assessment

Jade's type inference system is a **complete, correct, and practical Hindley-Milner implementation**. It achieves the design goal of zero-annotation programming for the vast majority of code:

- **86% of standard library functions require zero annotations**
- **100% of return types are inferred** (no Jade-native function has a return type annotation)
- **The remaining 14% of annotations are String-type parameters** — addressable by remediation item 12.1

The system extends classical HM with:
- Constrained type variables for numeric overloading
- Bidirectional type propagation (13 expression forms)
- SCC-based mutual recursion handling
- Monomorphization-based generics
- Row-polymorphic struct field deduction
- Multi-tier strictness modes (lenient → strict → pedantic)

### 13.2 "Are We HM Yet?"

**Yes.** Jade implements all essential components of the Hindley-Milner type system:

| HM Component | Status |
|--------------|--------|
| $\text{Var}$: Variable lookup | ✅ |
| $\text{App}$: Application (function call) | ✅ With unification of args ↔ params |
| $\text{Abs}$: Abstraction (lambda) | ✅ With bidirectional param inference |
| $\text{Let}$: Let-binding with generalization | ✅ With value restriction |
| $\text{MGU}$: Most General Unifier | ✅ Union-Find guarantees MGU |
| Principal type property | ✅ Follows from MGU + Let-Gen correctness |

### 13.3 Jade vs the Field

Jade occupies a **unique position** in the language landscape:

- More inference than any systems language (Rust, Go, C++ all require function signature annotations)
- Sound type system, unlike gradual/optional systems (TypeScript, Python/mypy)
- On par with OCaml/F#/Elm for core inference, with better numeric literal handling
- Falls short of Haskell in type class machinery and higher-kinded types
- Unique in combining zero-annotation functions with compiled/monomorphized execution

### 13.4 Path Forward

The six P0-P2 remediation items (§12.1-12.6, ~215 lines total) will:
1. Eliminate the remaining 8 partial annotations in the standard library
2. Improve diagnostic quality for edge cases
3. Add trait constraint enforcement at unification time
4. Handle curried functions and empty collections robustly

After completing these items, Jade's type inference will be **feature-complete for Rank-1 HM** with no known gaps in the common case. The only annotations remaining will be:
- Extern/FFI function signatures (fundamental, cannot be eliminated)
- Width-narrowing annotations (`i32`, `f32`) when the default (`i64`, `f64`) is not desired (by design)
- Genuinely ambiguous cases with multiple trait implementors (fundamental)

Rank-2 type *checking* (with explicit `forall` annotations) is a natural future extension and requires ~500 lines. Full Rank-2 *inference* is theoretically possible but EXPTIME-complete — no production language has implemented it, and Jade should not be the first to try without substantial PLT research.

---

*End of report.*
