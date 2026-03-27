# Jade Type Inference System — Comprehensive Analysis

## Executive Summary

Jade's type inference system is a **substantially complete implementation of Hindley-Milner (HM) type inference**, extended with constraint-based numeric defaulting, structural field inference, deferred resolution, and bidirectional type propagation. The implementation spans **9,478 lines** across 10 source files with **1,135 automated tests** (25 unification, 101 typer unit, 4 SCC, 245 integration, 760 bulk).

The system achieves **Level 1 inference** — local + inter-procedural inference via let-generalization and scheme instantiation — and is competitive with OCaml and Haskell in the function-body case while exceeding Rust and Go in annotation-freedom for numeric code and struct field access patterns.

Of **536 total type annotations** across 102 sample/test/std `.jade` files, **288 are structurally required** (extern FFI, data type definitions) and **248 are potentially inferable** under a more aggressive inference regime.

---

## Table of Contents

1. [Theoretical Foundation](#1-theoretical-foundation)
2. [Implementation Architecture](#2-implementation-architecture)
3. [What Is Implemented](#3-what-is-implemented)
4. [Where Annotations Are Required](#4-where-annotations-are-required)
5. [HM Completeness Assessment](#5-hm-completeness-assessment)
6. [Test Coverage Analysis](#6-test-coverage-analysis)
7. [Shortcomings & Gaps](#7-shortcomings--gaps)
8. [Language Comparison](#8-language-comparison)
9. [Level 2 Inference Analysis](#9-level-2-inference-analysis)
10. [Remediation Strategy](#10-remediation-strategy)

---

## 1. Theoretical Foundation

### 1.1 Classical HM

Hindley-Milner type inference (Damas & Milner, 1982) provides **complete and decidable** type inference for the simply-typed lambda calculus with let-polymorphism. The core judgment is:

$$\Gamma \vdash e : \tau$$

with generalization:

$$\text{Gen}(\Gamma, \tau) = \forall \bar{\alpha}.\,\tau \quad \text{where } \bar{\alpha} = \text{ftv}(\tau) \setminus \text{ftv}(\Gamma)$$

The key operations are:

| Operation | Mathematical Form | Jade Implementation |
|---|---|---|
| Unification | $\mathcal{U}(\tau_1, \tau_2) = \sigma$ | `InferCtx::unify()` — union-find with path compression |
| Instantiation | $\text{Inst}(\forall \bar{\alpha}.\,\tau) = [\bar{\alpha} \mapsto \bar{\beta}]\tau$ | `InferCtx::instantiate()` — fresh TypeVars per call site |
| Generalization | $\text{Gen}(\Gamma, \tau) = \forall (\text{ftv}(\tau) \setminus \text{ftv}(\Gamma)).\,\tau$ | `Typer::generalize()` — env-subtraction based |
| Occurs check | $\alpha \notin \text{ftv}(\tau)$ | `InferCtx::occurs_in()` — prevents infinite types |

### 1.2 Extensions Beyond Classical HM

Jade extends classical HM with:

1. **Constraint-based numeric typing** — TypeVars carry `TypeConstraint` (None, Numeric, Integer, Float, Trait) that refine during unification
2. **Bidirectional propagation** — Expected types flow downward through `lower_expr_expected`
3. **Deferred resolution** — Method calls and field accesses on unsolved TypeVars are deferred and re-checked post-lowering
4. **Structural field inference** — TypeVars accumulate `field_constraints` from field access, resolved to unique matching structs
5. **Trait-guided resolution (R4.1)** — Unsolved TypeVars with Trait constraints are resolved to the unique implementing type
6. **Value restriction** — Only syntactic values (Lambda, Ident, Struct, Array/Tuple of values) can be generalized

---

## 2. Implementation Architecture

### 2.1 File Layout

| File | Lines | Purpose |
|---|---|---|
| `types.rs` | 224 | Type enum (30 variants), Scheme struct |
| `typer/unify.rs` | 1,133 | InferCtx: union-find, unify, resolve, instantiate, substitute |
| `typer/mod.rs` | 1,552 | Typer struct, generalize(), is_syntactic_value(), scope management |
| `typer/lower.rs` | 2,117 | AST→HIR pipeline, deferred resolution, scheme building |
| `typer/expr.rs` | 2,550 | Expression lowering, bidirectional inference, call resolution |
| `typer/stmt.rs` | 581 | Statement lowering, let-generalization, match arm unification |
| `typer/mono.rs` | 432 | Monomorphization, substitute_type, normalize_inferable_fn |
| `typer/scc.rs` | 274 | Tarjan's SCC for topological function ordering |
| `typer/resolve.rs` | 373 | Signature declaration, TypeVar allocation for unannotated params |
| `typer/infer.rs` | 242 | Coroutine yield inference, literal_type, int coercion |

### 2.2 Lowering Pipeline

The complete inference pipeline in `lower_program()`:

```
Phase 1: register_prelude_types()       — Option<T>, Result<T,E>, Iter
Phase 2: declare_fn_sig() for all fns   — TypeVars for unannotated params/returns
Phase 3: declare_type_def(), enums      — TypeVars for unannotated struct fields
Phase 3A: infer_param_types()           — self param type for standalone methods
Phase 4: SCC ordering via Tarjan        — topological sort of function bodies
Phase 5: lower_fn()/lower_fn_deferred() — type-check bodies in SCC order
Phase 6: build_fn_scheme()              — Gen(Γ,τ) for inferable functions
Phase 7: resolve_deferred_methods()     — re-check method calls on solved TypeVars
Phase 8: resolve_deferred_fields()      — structural field inference → struct type
Phase 9: resolve_trait_constrained_vars — single-implementor trait resolution
Phase 10: strict struct field check     — error on unsolved field TypeVars
Phase 11: resolve_all_types()           — walk HIR, resolve every TypeVar to concrete
Phase 12: emit warnings + errors        — strict mode enforcement
```

### 2.3 Union-Find Engine

The `InferCtx` implements a **ranked union-find with path compression**:

- **Path compression** in `find()`: flattens chains to root for $O(\alpha(n))$ amortized
- **Rank-based union**: shorter tree is merged into taller tree
- **Constraint merge** during union: more specific constraint wins (Integer > Numeric > None; Float > Numeric > None; Integer + Float = error)
- **Constraint origin tracking**: each TypeVar records where its constraint came from (span + reason string) for error messages

### 2.4 Defaulting Hierarchy

When `resolve()` encounters an unsolved TypeVar:

| Constraint | Default | Strict Mode |
|---|---|---|
| Integer | `i64` | Silent (principled) |
| Float | `f64` | Silent (principled) |
| Numeric | `i64` | Warning (ambiguous) |
| None | `i64` | **Error** (unconstrained) |
| Trait(traits) | — | **Error** (unresolved) |

Three strictness levels:
- **Strict** (default): Integer/Float default silently; Numeric warns; None errors
- **Lenient** (`--lenient`): All default silently
- **Pedantic**: Even Integer/Float defaults produce errors

---

## 3. What Is Implemented

### 3.1 Core HM Operations ✓

| Feature | Status | Location |
|---|---|---|
| TypeVar allocation | ✓ | `fresh_var()`, `fresh_integer_var()`, `fresh_float_var()`, `fresh_numeric_var()` |
| Unification | ✓ | `unify()` with structural recursion over Fn, Vec, Array, Map, Tuple, Ptr, Rc, etc. |
| Occurs check | ✓ | `occurs_in()` prevents Type::TypeVar(α) ~ τ when α ∈ ftv(τ) |
| Let-generalization | ✓ | `generalize()` in `mod.rs` — ftv(τ) \ ftv(Γ) |
| Scheme instantiation | ✓ | `instantiate()` — fresh TypeVars replace quantified vars |
| Value restriction | ✓ | `is_syntactic_value()` — Lambda, Ident, Struct, Array/Tuple of values |
| Env-based scoping | ✓ | `scopes: Vec<HashMap<String, VarInfo>>` with push/pop |

### 3.2 Extended Features ✓

| Feature | Status | Description |
|---|---|---|
| Numeric constraints | ✓ | TypeConstraint enum with lattice-based merging |
| Bidirectional inference | ✓ | `lower_expr_expected(expr, Option<&Type>)` propagates expected types |
| SCC ordering | ✓ | Tarjan's algorithm for mutually recursive functions |
| Deferred methods | ✓ | Re-resolve method calls after full lowering |
| Deferred fields | ✓ | Re-resolve field access after full lowering |
| Structural field inference | ✓ | `field_constraints` map: TypeVar → Vec<(field, type)> → unique struct |
| Trait-guided resolution | ✓ | Post-lowering: unsolved TypeVar + Trait → unique implementor |
| Monomorphization | ✓ | Generic fns with Type::Param + inferable fns via scheme→normalize→mono |
| Int literal coercion | ✓ | Expected type propagation for integer width selection |
| Float literal coercion | ✓ | Expected type propagation for float width selection |
| Quantified var exemption | ✓ | Scheme-quantified vars exempt from strict-mode errors at definition |

### 3.3 Inference Capability Summary

What Jade can infer **without any annotations**:

- **Variable bindings**: `x is 42` → x : i64 (Integer constraint → default)
- **Return types**: Inferred from tail expression and return statements
- **Function params**: When used in arithmetic, comparison, method calls, or passed to typed callees
- **Lambda params**: From expected `Fn` type context (bidirectional)
- **Struct types from field access**: `x.name` + `x.age` → unique struct with both fields
- **Enum types from match patterns**: At call sites, passing `Add(i, i+1)` constrains the param type
- **Generic function instantiation**: `identity(42)` → instantiate ∀α.α→α with fresh β, unify with i64
- **Container element types**: `vec(1, 2, 3)` → Vec of i64 from element unification

---

## 4. Where Annotations Are Required

### 4.1 Taxonomy of Required Annotations

From analysis of 536 annotations across 102 `.jade` files:

| Category | Count | % | Why Required |
|---|---|---|---|
| Extern FFI boundaries | 83 | 15.5% | No body to infer from; ABI contract |
| Struct field definitions | 76 | 14.2% | Nominal type system demands field types |
| Enum variant payloads | 80 | 14.9% | Variant constructors need payload types |
| Store field types | 23 | 4.3% | Schema definition (like struct fields) |
| Higher-order Fn params | 11 | 2.1% | `f: (i64) -> i64` — Fn type cannot be inferred without call |
| Actor field types | 7 | 1.3% | Actor state definition |
| Handler param types | 8 | 1.5% | Message protocol interface |
| **Subtotal REQUIRED** | **288** | **53.7%** | |
| Numeric function params | ~100 | 18.7% | Inferable from arithmetic usage |
| String function params | ~44 | 8.2% | Inferable from method dispatch |
| Custom type params | ~48 | 9.0% | Inferable from field access or match |
| Lambda params | ~19 | 3.5% | Inferable from HOF context |
| Variable bindings | ~12 | 2.2% | Inferable from initializer |
| Width-specific annotations | ~15 | 2.8% | Non-default width (i32, u8, etc.) |
| Return type annotations | ~10 | 1.9% | Inferable from body |
| **Subtotal INFERABLE** | **~248** | **46.3%** | |

### 4.2 Fine-Grained Classification

**Annotations that ARE inferable today** (inference engine supports it):
- Function params used in integer arithmetic: `*fib(n) → if n < 2 ...` → n gets Integer constraint → i64
- Function params used in float arithmetic: `*degrees(radians) → radians * 180.0 / PI` → Float constraint → f64
- Return types: Always inferred from tail expression
- Variable bindings with initializers: `x is 42` never needs `: i64`
- Struct params used via field access: `*dot(a, b) → a.x * b.x + a.y * b.y` → structural inference

**Annotations that CANNOT be inferred today** (engine limitation):
- Non-default numeric width: `m: i32` when i64 would be default — no syntax for "I want i32 specifically"
- Higher-order function parameter types: `f: (i64) -> i64` — Fn type deduced only from call in body
- String params without method calls: `name: String` passed through without `.length`/`.char_at`
- Extern declarations: FFI boundary — always required
- Data type definitions: Struct fields, enum variants, actor fields — always required

---

## 5. HM Completeness Assessment

### 5.1 Score: HM-Complete for Core, HM-Extended for Practice

| HM Feature | Present | Quality |
|---|---|---|
| Principal types | ✓ | Unification produces most general unifier |
| ∀-quantification | ✓ | Scheme struct with quantified TypeVar ids |
| Let-polymorphism | ✓ | `generalize()` with env-subtraction |
| Instantiation | ✓ | Fresh TypeVars per call site |
| Occurs check | ✓ | Prevents infinite types |
| Value restriction | ✓ | Prevents unsound generalization |
| Rank-1 polymorphism | ✓ | ∀ at outermost only (let-bindings) |

**Missing from full Damas-Milner**:

| Feature | Status | Impact |
|---|---|---|
| Rank-2+ polymorphism | ✗ | Cannot pass polymorphic functions as args that stay polymorphic |
| Impredicative polymorphism | ✗ | Cannot instantiate ∀ with another ∀ type |
| Type classes / qualified types | Partial | Trait constraints exist but no Wadler-Blott dictionary passing |
| Kind system | ✗ | No higher-kinded types (no `* → *` kinds) |
| Row polymorphism | ✗ | Structural field inference is ad hoc, not row-based |
| GADTs | ✗ | No type refinement from pattern matching |

### 5.2 Verdict

**Jade IS "HM" in the classical sense**: it implements Algorithm J with union-find unification, let-generalization, scheme instantiation, and the value restriction. Programs that are well-typed under HM will be correctly inferred by Jade.

**Jade exceeds HM** with constraint-based numeric typing, bidirectional propagation, structural field inference, and deferred resolution — features that make annotation-free programming practical for systems code.

**Jade falls short of System F**: no higher-rank types, no impredicative polymorphism, no type-class dictionary passing.

---

## 6. Test Coverage Analysis

### 6.1 Test Inventory

| Test Suite | Count | Scope |
|---|---|---|
| `typer/unify.rs` unit tests | 25 | Unification, constraints, occurs check, defaulting |
| `typer/mod.rs` unit tests | 101 | Typing rules, generalization, scheme creation, errors |
| `typer/scc.rs` unit tests | 4 | Tarjan's SCC correctness |
| `tests/integration.rs` | 245 | End-to-end compile+run, compile-fail |
| `tests/bulk_tests.rs` | 760 | End-to-end compile+run, strict mode, edge cases |
| **Total** | **1,135** | |

### 6.2 Coverage by Inference Feature

| Feature | Unit Tests | Integration Tests | Assessment |
|---|---|---|---|
| Int literal typing | ✓ | ✓ | Adequate |
| Float literal typing | ✓ | ✓ | Adequate |
| Bool typing | ✓ | ✓ | Adequate |
| String typing | ✓ | ✓ | Adequate |
| Binary operator inference | ✓ | ✓ | Adequate |
| Comparison → Bool | ✓ | ✓ | Adequate |
| Function call inference | ✓ | ✓ | Adequate |
| Let-generalization | ✓ | ✓ | Good |
| Scheme instantiation | ✓ | ✓ | Good |
| Value restriction | ✓ | — | Adequate (unit only) |
| Generic monomorphization | ✓ | ✓ | Good |
| Unannotated param inference | ✓ | ✓ | Good |
| Lambda typing | ✓ | ✓ | Good |
| Bidirectional inference | ✓ | ✓ | Adequate |
| Struct field inference | ✓ | ✓ | Adequate |
| Deferred method resolution | ✓ | ✓ | Adequate |
| Deferred field resolution | ✓ | ✓ | Adequate |
| Trait-guided resolution | ✓ | — | Minimal |
| SCC ordering | ✓ | — | Adequate (unit only) |
| Strict mode enforcement | ✓ | ✓ | Good |
| Integer/Float/Numeric defaults | ✓ | ✓ | Good |
| Occurs check | ✓ | — | Adequate (unit only) |
| Constraint merging | ✓ | — | Adequate (unit only) |
| No-TypeVar-leak guarantee | ✓ | — | Adequate (unit only) |

### 6.3 Coverage Gaps

1. **Mutual recursion inference**: SCC unit tests exist but no integration test exercises mutual recursion with unannotated params
2. **Poly lambda re-lowering**: Unit tests verify scheme creation but no integration test for polymorphic let-bound lambdas called at multiple types
3. **Trait-guided resolution**: Only unit-level coverage; no end-to-end test
4. **Negative testing for unsound generalization**: Value restriction is unit-tested but no integration test verifies that generalizing a mutation-containing let binding is rejected
5. **Edge cases for structural inference**: When multiple structs share field names, the disambiguation logic is tested only cursorily

---

## 7. Shortcomings & Gaps

### 7.1 Inference Limitations

**S1: Struct field types require annotation in strict mode**

In strict mode (the default), unannotated struct fields that are never constrained by usage produce errors. While fields used in constructors ARE inferred, the current design requires explicit annotation in the type definition:

```jade
type Vec3
    x: i64    # Cannot write: x
    y: i64    # Without usage, TypeVar stays unsolved
    z: i64
```

*Theoretical fix*: Infer field types from constructor call sites across the module.

**S2: Higher-order function parameter types**

Function types (`(i64) -> i64`) as parameter types cannot be inferred from the body alone because the function type structure (arity, param types, return type) must be known before the body can be checked:

```jade
*apply(f: (i64) -> i64, x: i64)    # f's type cannot be omitted
    f(x)
```

*Theoretical fix*: Create a TypeVar with Fn-constraint (`?1 : ?2 → ?3`) from the call pattern `f(x)`, then unify at call sites.

**S3: No width inference for non-default integers**

When a function needs `i32` specifically (e.g., for FFI), the annotation cannot be elided because the inference engine defaults `Integer` to `i64`:

```jade
*ackermann(m: i32, n: i32)    # Would default to i64 without annotation
```

*Theoretical fix*: Propagate width requirements from call sites (e.g., if called from an `extern` that expects `i32`).

**S4: No row polymorphism for structural typing**

The structural field inference is ad hoc: it collects field constraints and finds a unique matching struct. But it cannot express "any struct with field `.x` of type i64" as a polymorphic type. This means:

```jade
*get_x(s)      # Works if exactly ONE struct has field x
    s.x

*get_x(s)      # FAILS if multiple structs have field x
    s.x
```

*Theoretical fix*: Row polymorphism (e.g., `{x : i64 | ρ}`) would generalize over struct shapes.

**S5: No higher-rank types**

Cannot pass a polymorphic function as an argument and have it remain polymorphic:

```jade
# Cannot express: apply_to_both : (∀α. α → α) → (i64, String) → (i64, String)
```

This is inherent to Rank-1 HM and requires System F or Rank-N extensions.

**S6: Enum type inference from match patterns does not work for function params**

While match patterns contain variant names that identify enum types, the current system does NOT infer enum types for function parameters from match patterns in the function body. When `*eval(op)` contains `match op { Add(a,b) ? ... }`, the param `op` defaults to `I64` instead of resolving to `Op`. This is because match-pattern-based type resolution happens after the param type has already been defaulted.

*Verified by test*: `*eval(op)` with `match op { Add(a,b) ? ... }` produces error `non-exhaustive match on I64: missing _` instead of inferring `Op`.

*Fix*: During match lowering, if the scrutinee is a TypeVar and the match arms contain enum variant patterns, unify the scrutinee TypeVar with the enum type.

### 7.2 Implementation Quality Issues

**Q1: Repeated code paths for call resolution**

`lower_call` in `expr.rs` has **four** separate call-resolution paths: scheme-based, generic, auto-monomorphization fallback, and direct-fn. This creates maintenance burden and potential for inconsistent behavior.

**Q2: Deferred resolution is a second pass, not demand-driven**

Deferred methods/fields are resolved in a separate phase after all lowering. A demand-driven approach (re-resolve lazily when needed) could handle cascading dependencies more robustly.

**Q3: Monomorphization depth limit is a hard constant (64)**

The depth limit prevents infinite monomorphization but is arbitrary. A gas-based or type-size-based limit would be more principled.

---

## 8. Language Comparison

### 8.1 Classification Framework

**Level 0**: No inference — all types annotated (C, Java pre-10).
**Level 1**: Local + let-polymorphic inference — function bodies inferred, signatures sometimes inferred (HM languages, Jade).
**Level 2**: Whole-program inference — all types inferred including function signatures across module boundaries (no production language achieves this for systems code).

### 8.2 Detailed Comparison

| Language | Inference Level | Param Annotations | Return Annotations | Local Vars | Generics | Width Inference |
|---|---|---|---|---|---|---|
| **Jade** | **1+** | **Optional** | **Optional** | **Never needed** | **Implicit** | **No** (defaults i64/f64) |
| **Rust** | 1 | **Required** | **Required** | Never needed | Explicit `<T>` | N/A (no default width) |
| **Haskell** | 1+ | Optional | Optional | Never needed | Implicit | N/A (Integer typeclass) |
| **OCaml** | 1+ | Optional | Optional | Never needed | Implicit | N/A (single int type) |
| **TypeScript** | 1 | Optional* | Optional* | Contextual | Explicit | N/A |
| **Go** | 0.5 | Required | Required | `:=` inferred | No generics (1.18+: explicit) | N/A |
| **Python** | 0 (dynamic) | None needed | None needed | None needed | None needed | N/A |
| **Swift** | 1 | Required | Required | Inferred | Explicit | N/A |
| **Kotlin** | 1 | Required | Optional | Inferred | Explicit | N/A |
| **Scala** | 1 | Required | Optional | Inferred | Explicit | N/A |
| **Zig** | 0.5 | Required | Required | Inferred | `anytype` | N/A |
| **C** | 0 | Required | Required | Required | N/A | N/A |

*TypeScript's "optional" requires `strict` mode to be off; in strict mode many annotations are effectively required.

### 8.3 Jade vs. Key Competitors

**Jade vs. Rust**: Jade is strictly more powerful in inference. Rust's "no principal types for lifetime-annotated functions" limitation forces explicit signatures. Jade infers both params and returns, including polymorphic generalization. Rust requires `fn foo<T>(x: T) -> T`; Jade infers `*foo(x) → x` as `∀α. α → α`.

**Jade vs. Haskell**: Similar inference power (both HM-based). Haskell has type classes (qualified types) that Jade's traits approximate but do not fully replicate. Haskell's `Num` class subsumes Jade's Integer/Float/Numeric constraints. Haskell has higher-rank types via `RankNTypes` extension; Jade does not. However, Jade's structural field inference has no Haskell equivalent (Haskell requires fully qualified record access or lens libraries).

**Jade vs. OCaml**: Very similar architecturally. both use union-find HM. OCaml has row polymorphism for objects (not records); Jade has structural field inference (similar spirit, less general). OCaml has functors (higher-order modules); Jade does not. OCaml has a single integer type (`int`), avoiding width ambiguity entirely.

**Jade vs. Go**: Jade is dramatically more powerful. Go requires all function signatures to be explicitly annotated. Go's type inference is limited to `x := expr` for local variables. Go lacks generics (prior to 1.18) and even with generics, requires explicit type parameter declarations.

**Jade vs. Python**: Python has zero type annotations required (dynamic typing). Jade aims for the same annotation-freedom but with static type safety. Currently Jade achieves this for integer/float arithmetic and many struct/enum patterns, but still requires annotations at FFI boundaries and data definitions. In terms of ergonomics, Jade approaches Python for "scripting-style" code while providing C-level performance.

### 8.4 Unique Jade Advantages

1. **Implicit generics**: `*identity(x) → x` is automatically generalized to a polymorphic scheme — no `<T>` syntax needed anywhere
2. **Structural struct inference**: Field access patterns resolve to struct types without annotation
3. **Constraint-based numeric typing**: `x + 1` constrains x to Integer, `x + 1.0` constrains to Float — more ergonomic than explicit numeric types
4. **Deferred resolution**: Method calls on not-yet-solved types are automatically re-checked — enables forward-reference-like patterns
5. **SCC-ordered inference**: Mutual recursion "just works" without forward declarations

---

## 9. Level 2 Inference Analysis

### 9.1 Definition

**Level 2 inference** (whole-program type inference) means: given a complete program, infer ALL types — including function parameter types, return types, struct field types, and generic instantiations — without ANY annotations except at true system boundaries (FFI).

### 9.2 The State of the Art

**No production systems-level language achieves Level 2 inference.** The closest approaches:

| System | Approach | Limitation |
|---|---|---|
| **MLton** (whole-program SML) | Defunctorize + monomorphize entire program | Closed-world; no separate compilation |
| **Crystal** | Global type inference for method args | Slow compilation; no separate compilation |
| **Nim** | Limited inter-procedural inference | Requires annotations in many cases |
| **Julia** | Runtime type specialization | JIT-based, not ahead-of-time |
| **Typed Racket** | Occurrence typing | Requires annotations at module boundaries |

The fundamental theoretical obstacle is the **open-world assumption**: if functions can be called from unknown contexts (libraries, plugins, FFI), their parameter types cannot be uniquely determined. Level 2 requires a **closed-world** assumption.

### 9.3 What Jade Already Has Toward Level 2

Jade's existing machinery provides a strong foundation:

1. **InferCtx unification engine**: Can propagate constraints across function boundaries today
2. **SCC ordering**: Already handles mutual recursion — the hardest inter-procedural case
3. **Scheme-based polymorphism**: Functions are generalized and instantiated at call sites
4. **Structural inference**: Field access already resolves struct types without annotation
5. **Deferred resolution**: Already implements a form of demand-driven re-analysis

### 9.4 What Level 2 Requires

**R1: Inter-module constraint propagation**

Currently, function signatures are finalized within a single compilation unit. Level 2 requires propagating constraints across module boundaries:

```jade
# module_a.jade
*foo(x)
    x + 1    # x : Integer constraint (not yet resolved)

# module_b.jade
import module_a
*bar()
    foo(42)  # foo's x should unify with i64 here
```

Implementation: Emit "constraint summaries" for each function (param constraints + return constraint) as part of module metadata. When importing, load constraints and propagate.

**R2: Demand-driven analysis**

Instead of resolving all TypeVars in a fixed phase order, use a **demand-driven** (lazy) approach:
- When a TypeVar is needed (e.g., for codegen), trace back through constraint edges to find all usage sites
- Collect constraints lazily, unify, and resolve
- Handles cascading dependencies naturally

**R3: Fn-type inference from call patterns**

Currently `f: (i64) -> i64` requires annotation. To infer this:
- When `f(x)` appears in a body, create arity+param TypeVars: `?f : (?a) → ?r`
- Unify `?a` with `x`'s type and `?r` with the call expression's expected type
- At call sites, the caller's argument types constrain `?a`

**R4: Struct field type inference from constructors**

Currently strict mode requires struct field annotations. To infer:
- Collect all constructor call sites: `Vec3(x is 1, y is 2, z is 3)`
- Unify field TypeVars with constructor argument types
- If all fields are constrained, the struct is fully typed

**R5: Width propagation from FFI boundaries**

When a function is eventually called by an `extern` that expects `i32`, propagate the width requirement backward through the call chain.

### 9.5 Level 2 Strategy Outline

**Phase A: Fn-type inference (Medium effort)**

Extend `lower_expr` for `Call` on an Ident that is a local variable: if the variable's type is a TypeVar, create a Fn-constraint `(?p1, ?p2, ...) → ?r` from the call arity and unify.

Estimated scope: ~200 lines in expr.rs + stmt.rs.

**Phase B: Constructor-driven struct field inference (Medium effort)**

Remove the strict-mode requirement for struct field annotations. Instead, collect all constructor calls for each struct, unify field types, and error only if fields remain unsolved after full lowering.

Estimated scope: ~100 lines in lower.rs + resolve.rs.

**Phase C: Width propagation (High effort)**

Introduce width constraints: instead of just `Integer`, track `IntegerWithWidth(i32)` as a constraint. When a function's return value feeds into an i32 context, propagate backward.

Estimated scope: ~300 lines across unify.rs + expr.rs + resolve.rs.

**Phase D: Inter-module constraint export (High effort)**

Add a serialization format for constraint summaries. During compilation, emit `.jadec` constraint files alongside object code. When importing, load and apply constraints.

Estimated scope: ~500 lines new module + changes to lower.rs.

**Phase E: Demand-driven resolution (Very high effort)**

Replace the fixed-phase resolution with a lazy constraint graph. This is a fundamental architectural change.

Estimated scope: ~1000+ lines, significant refactoring.

### 9.6 Prioritized Roadmap

| Priority | Feature | Impact | Effort | Dependency |
|---|---|---|---|---|
| **P0** | Fn-type inference from call patterns | Removes ~11 HOF annotations | Medium | None |
| **P1** | Constructor-driven struct field inference | Removes ~76 struct field annotations requirement | Medium | None |
| **P2** | Width propagation | Removes ~15 width-specific annotations | High | None |
| **P3** | Inter-module constraints | Enables separate compilation with inference | High | P0, P1 |
| **P4** | Demand-driven resolution | Enables true Level 2 | Very High | P3 |

---

## 10. Remediation Strategy

### 10.1 Immediate Actions (Current State Polish)

1. **Add integration tests for mutual recursion inference**: Verify SCC-ordered inference works end-to-end with unannotated params
2. **Add integration tests for trait-guided resolution**: Cover the R4.1 path with end-to-end compile+run tests
3. **Add integration tests for poly lambda re-lowering**: Verify polymorphic let-bound lambdas work at multiple call types
4. **Add negative test for value restriction**: Verify that non-value let bindings get monomorphic schemes
5. **Consolidate call-resolution paths**: The four paths in `lower_call` should be unified into a single dispatch

### 10.2 Short-Term Improvements

6. **Fn-type inference from call patterns (P0)**: Enable `*apply(f, x) → f(x)` to infer `f : (?a) → ?r`
7. **Constructor-driven struct field inference (P1)**: Remove annotation requirement for struct fields that are constrained by constructors
8. **Width propagation (P2)**: Propagate specific integer widths backward from typed context

### 10.3 Long-Term Architecture

9. **Inter-module constraint export (P3)**: Enable cross-module inference
10. **Demand-driven resolution (P4)**: Replace fixed-phase resolution with lazy constraint graph

---

## Appendix A: Annotation Audit Summary

### A.1 Files with Potentially Inferable Annotations

**Standard Library** (58 annotations):
- `std/math.jade`: 26 × `f64` params — all constrained by float arithmetic or LLVM intrinsics
- `std/fmt.jade`: 6 — `f64` from float formatting, `String` from string methods
- `std/io.jade`: 9 — `String` from `fopen`, string methods
- `std/sort.jade`: 7 — `Vec of i64` from element comparisons
- `std/path.jade`: 5 — `String` from string method dispatch
- `std/os.jade`: 3 — `String` from raw-pointer coercion
- `std/rand.jade`: 1 — `u64` from bitwise operations (width annotation needed)
- `std/time.jade`: 1 — `f64` from float subtraction

**Benchmarks** (23 annotations):
- `nbody.jade`: 6 × `i64` params — integer arithmetic
- `array_ops.jade`: 5 × `i64` params — addition
- `hof_pipeline.jade`: 3 — `i64` params in simple arithmetic functions
- `ackermann.jade`: 2 × `i32` — width-specific (NOT inferable without width propagation)
- `spectral_norm.jade`: 2 × `i64` — arithmetic
- `struct_ops.jade`: 2 × `Vec3` — field access inference
- `closure_capture.jade`: 2 — integer arithmetic
- `enum_dispatch.jade`: 1 × `Op` — match pattern inference

**Tests** (167 annotations):
- Distributed across 22 test program files, majority are custom type params (inferable from field access/match patterns) and numeric params (inferable from arithmetic).

### A.2 Why Not Remove Them Now

While the inference engine CAN handle most of these cases, removing annotations requires:
1. **Compilation testing**: Each removal must be verified by `cargo test`
2. **Semantic preservation**: Removing `i32` annotations would change behavior (default to `i64`)
3. **Documentation value**: Some annotations serve as documentation for readers
4. **Std library stability**: Standard library files may be imported by user code that depends on specific types

The annotations identified as "inferable" should be treated as a **compatibility matrix** — they demonstrate where the inference engine already has the power to work without help, and represent the target for annotation-free programming in future Jade evolution.

### A.3 Annotations Successfully Removed (Verified by Compilation + Execution)

The following annotation removals were made and verified against the full test suite (1,005 tests passing):

| File | Change | Verified |
|---|---|---|
| `benchmarks/spectral_norm.jade` | `*a_elem(i: i64, j: i64)` → `*a_elem(i, j)` | Output: `243125000000` ✓ |
| `benchmarks/nbody.jade` | 6× `i64` param annotations removed from `dist_sq` | Output: `-399999000` ✓ |
| `benchmarks/struct_ops.jade` | `*dot(a: Vec3, b: Vec3)` → `*dot(a, b)` (field inference) | Output: `3876420019754212736` ✓ |
| `benchmarks/array_ops.jade` | 5× `i64` param annotations removed from `sum_arr` | Output: `250000075000000` ✓ |

### A.4 Annotations That Cannot Be Removed (Verified by Testing)

| File | Annotation | Reason |
|---|---|---|
| `benchmarks/enum_dispatch.jade` | `op: Op` | Match-based enum inference not implemented for function params |
| `benchmarks/hof_pipeline.jade` | `x: i64` on `double`/`add_one` | Functions used as values (not called) need annotations |
| `benchmarks/ackermann.jade` | `m: i32, n: i32` | Width-specific (i32 vs i64) — would change semantics |

---

## Appendix B: Formal Inference Rules

### B.1 Jade Typing Judgments

$$\frac{x : \sigma \in \Gamma \quad \tau = \text{Inst}(\sigma)}{\Gamma \vdash x : \tau} \quad \text{[Var]}$$

$$\frac{\Gamma, x : \tau_1 \vdash e : \tau_2}{\Gamma \vdash \lambda x.\, e : \tau_1 \to \tau_2} \quad \text{[Abs]}$$

$$\frac{\Gamma \vdash e_1 : \tau_1 \to \tau_2 \quad \Gamma \vdash e_2 : \tau_1}{\Gamma \vdash e_1 \; e_2 : \tau_2} \quad \text{[App]}$$

$$\frac{\Gamma \vdash e_1 : \tau_1 \quad \text{syntactic\_value}(e_1) \quad \sigma = \text{Gen}(\Gamma, \tau_1) \quad \Gamma, x : \sigma \vdash e_2 : \tau_2}{\Gamma \vdash \texttt{let}\; x = e_1 \;\texttt{in}\; e_2 : \tau_2} \quad \text{[Let]}$$

### B.2 Jade-Specific Extensions

$$\frac{n \in \mathbb{Z} \quad \alpha \;\text{fresh} \quad \text{constrain}(\alpha, \texttt{Integer})}{\Gamma \vdash n : \alpha} \quad \text{[IntLit]}$$

$$\frac{n \in \mathbb{R} \quad \alpha \;\text{fresh} \quad \text{constrain}(\alpha, \texttt{Float})}{\Gamma \vdash n : \alpha} \quad \text{[FloatLit]}$$

$$\frac{\Gamma \vdash e : \alpha \quad \text{field\_constraints}(\alpha) \supseteq \{(f_1, \tau_1), \ldots\} \quad S \;\text{unique struct with fields } f_1, \ldots}{\Gamma \vdash e.f_i : \tau_i \quad \mathcal{U}(\alpha, S)} \quad \text{[FieldInfer]}$$

$$\frac{\Gamma \vdash e : \alpha \quad \text{constraint}(\alpha) = \texttt{Trait}(T) \quad |\text{impls}(T)| = 1 = \{S\}}{\mathcal{U}(\alpha, S)} \quad \text{[TraitResolve]}$$

---

*Report generated from source analysis of Jade compiler v0.5.x (9,478 lines of type system code, 1,135 tests, 102 sample programs).*
