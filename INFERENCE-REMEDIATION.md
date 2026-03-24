# Jade Type Inference System — Complete Scientific Audit & Remediation Plan

**Date**: March 24, 2026  
**Scope**: All type inference, constraint solving, default resolution, annotation requirements  
**Baseline**: 930 tests passing (114 unit + 571 bulk + 245 integration)  
**Goal**: Zero mandatory type annotations — fully inferred programs  
**Codebase**: ~6,361 lines in typer module, ~483 lines in unification engine, ~184 lines type representation

---

## 1. Formal Foundations

### 1.1 Type Inference Theory Taxonomy

Type inference systems exist on a spectrum of power. Every design point trades expressiveness for decidability, inference completeness, or performance:

| System | Annotations | Decidable | Complete | Example Languages |
|--------|-------------|-----------|----------|-------------------|
| **Hindley-Milner (HM)** | Zero (rank-1) | ✅ | ✅ | ML, Haskell 98, Elm |
| **Local Type Inference** | Partial | ✅ | ❌ | Scala 2, C# |
| **Bidirectional** | Strategic | ✅ | ❌ | GHC, TypeScript |
| **HM(X) / Qualified** | Zero (constrained) | ✅ | ✅ | Haskell + typeclasses |
| **System F** | All polymorphic | ❌ | ❌ | — |
| **Algebraic Subtyping (MLsub)** | Zero | ✅ | ✅ | MLsub, simple-sub |
| **OutsideIn(X)** | Stratified | ✅ | ✅ | GHC modern |

**Jade's current position**: A hybrid between *ad-hoc heuristic inference* and *partial bidirectional typing*. It has a unification engine (Union-Find in `unify.rs`) but uses it inconsistently. The system is not grounded in any single formal type inference algorithm.

### 1.2 The Gold Standard for Jade's Design Goals

For Jade's goal of "zero annotations", the appropriate theoretical target is:

> **Bidirectional HM with constraint-based solving and defaulting rules**

This combines:
1. **Hindley-Milner W/J algorithm** for principal types on non-generic code
2. **Bidirectional checking** for literals, lambdas, and expected-type propagation
3. **Constraint generation** during lowering (not AST heuristics)
4. **TypeVar unification** via Union-Find (already implemented)
5. **Post-inference defaulting** for genuinely ambiguous variables
6. **Monomorphization** for generics (already working)

---

## 2. Current Implementation Analysis

### 2.1 Architecture Overview

**Complete Pipeline** (in execution order):

```
1. register_prelude_types()         — Option, Result, Iter (resolve.rs)
2. declare_*_sig() loop             — Register all top-level signatures
   ├─ Unannotated params → fresh_var() (TypeVar)
   ├─ Annotated params → concrete types
   ├─ Unannotated returns → infer_ret_ast() (AST walk heuristic)  ← PROBLEM
   └─ main() → forced I32
3. declare_impl_block()             — Trait impls, method signatures
4. infer_param_types()              — Fixed-point iteration (max 8 rounds):  ← PROBLEM
   ├─ Body-driven: constraint_from_expr() heuristic walk
   ├─ Call-site-driven: call_site_expr() heuristic walk
   ├─ Return re-inference: infer_ret_ast_with_params()
   └─ FINAL RESOLVE: all remaining TypeVar/Inferred → resolve() → I64  ← CRITICAL
5. lower_*() pass                   — AST → HIR with bidirectional lowering
   ├─ lower_expr_expected(expr, expected) — checking mode
   ├─ lower_call() — argument/param unify
   └─ unify_at() used at ~20 call sites
6. resolve_all_types()              — Walk entire HIR, resolve all TypeVars → concrete
```

### 2.2 Type Representation (types.rs — 184 lines)

```rust
enum Type {         // 31 variants
    I8, I16, I32, I64, U8, U16, U32, U64, F32, F64,  // Numeric (10)
    Bool, Void, String,                                 // Primitives (3)
    Array(Box<Type>, usize), Vec(Box<Type>),           // Collections (2)
    Map(Box<Type>, Box<Type>), Tuple(Vec<Type>),       // Collections (2)
    Struct(String), Enum(String),                       // Named (2)
    Fn(Vec<Type>, Box<Type>),                          // Function (1)
    Param(String),                                      // Generic param (1)
    Ptr(Box<Type>), Rc(Box<Type>), Weak(Box<Type>),   // Pointers (3)
    ActorRef(String), Coroutine(Box<Type>),            // Concurrency (2)
    Channel(Box<Type>), DynTrait(String),              // Concurrency/Traits (2)
    Inferred,                                           // Legacy sentinel (1)
    TypeVar(u32),                                       // Unification variable (1)
}
```

**Assessment**: Adequate. `TypeVar(u32)` is properly implemented. `Type::Inferred` is a legacy remnant that should be eliminated (subsumed by TypeVar).

### 2.3 Unification Engine (unify.rs — 483 lines)

**Status: IMPLEMENTED and SOUND**

The `InferCtx` is a proper Union-Find with:

| Feature | Status | Details |
|---------|--------|---------|
| `fresh_var()` | ✅ | Allocates `TypeVar(u32)`, O(1) |
| `unify(a, b)` | ✅ | Structural unification, recursive |
| `unify_at(a, b, span, reason)` | ✅ | With constraint provenance tracking |
| Occurs check | ✅ | Prevents `α = Vec<α>` infinite types |
| Path compression | ✅ | `find()` with path halving |
| Rank-based union | ✅ | Balanced tree, near-O(α(n)) |
| Structural recursion | ✅ | Array, Vec, Map, Tuple, Fn, Ptr, Rc, Weak, Channel, Coroutine |
| `resolve(ty)` | ⚠️ | Deep resolution, but unsolved → I64 |
| `try_resolve(ty)` | ✅ | Deep resolution, unsolved → None |
| `shallow_resolve(ty)` | ✅ | One-step resolution |
| `origin_of(ty)` | ✅ | Constraint provenance lookup |
| `Type::Inferred` wildcard | ⚠️ | Unifies with anything silently |

**Soundness properties verified by 18 unit tests**:
- ✅ Identity: `unify(a, a)` succeeds
- ✅ Symmetry: `unify(a, b)` equivalent to `unify(b, a)`
- ✅ Transitivity: `unify(a,b); unify(b,c) ⟹ resolve(a) == resolve(c)`
- ✅ Structural: `unify(Vec<α>, Vec<F64>) ⟹ α = F64`
- ✅ Occurs check: `unify(α, Vec<α>)` → error
- ✅ Concrete mismatch: `unify(I64, String)` → error
- ✅ Arity mismatch: `unify(Tuple(A), Tuple(A,B))` → error

**Critical defect**: `resolve()` defaults unsolved TypeVars to `I64`. This is a **lossy operation** that destroys information. It should be separated into (1) resolution and (2) defaulting, where defaulting uses context-aware rules.

### 2.4 Bidirectional Checking (expr.rs — 1742 lines, stmt.rs — 498 lines)

**Status: PARTIALLY IMPLEMENTED**

`lower_expr_expected(expr, expected: Option<&Type>)` is the central dispatch. Expected type propagation:

| Context | Expected Propagated? | Mechanism | Quality |
|---------|---------------------|-----------|---------|
| Lambda parameters | ✅ | `Fn(ptys, ret)` decomposition | Sound |
| Lambda return type | ✅ | `Fn(_, ret)` decomposition | Sound |
| Array elements | ✅ | `Array(et, _)` decomposition | Sound |
| Function call args | ✅ | `param_tys.get(i)` | Sound |
| Return statements | ✅ | `lower_stmt(Ret)` passes `ret_ty` | Sound |
| Bind with annotation | ✅ | `b.ty` unify with value | Sound |
| Assignment LHS=RHS | ✅ | `unify_at(ht.ty, hv.ty)` | Sound |
| Ternary then/else | ✅ | `unify_at(ht.ty, he.ty)` | Sound |
| If-expression branches | ✅ | `unify_at(ty, e.ty)` | Sound |
| Channel send | ✅ | `unify_at(elem_ty, hval.ty)` | Sound |
| Struct literal fields | ✅ | `unify_at(declared, fi.value.ty)` | Sound |
| Select arm channels | ✅ | `unify_at(elem_ty, hv.ty)` | Sound |
| Vec literal elements | ⚠️ | First element forward only | Incomplete |
| **Match arm bodies** | ❌ | No expected type propagation | Missing |
| **Block as expression** | ❌ | Lowered with `Type::Void` expected | Missing |
| **Pipe RHS** | ❌ | No backward type flow | Missing |
| **Index backward** | ❌ | No constraint on container from result | Missing |
| **Map keys** | ❌ | Hardcoded to `String` | Missing |
| **For-each into block body** | ❌ | Body doesn't know iteration type context | Missing |

### 2.5 Pre-Lowering Heuristic Inference (resolve.rs — 823 lines)

**Status: IMPLEMENTED but DUPLICATIVE and HARMFUL**

`infer_param_types()` runs BEFORE lowering and does a fixed-point iteration (max 8 rounds) with two heuristic mechanisms. This is the **core architectural problem**.

#### Mechanism 1: Body-Driven (`constraint_from_expr`)

Walks a function's AST body looking for usage patterns that reveal parameter types:

```
Call patterns:     f(param) where f has known param types → constrain
Method patterns:   param.contains() → constrain to String
BinOp patterns:    param + known_type → constrain
```

Coverage: ~12 expression forms, ~5 method name heuristics.

**Limitation**: This is pattern matching on AST shape. It misses:
- Chains: `f(g(param))` only constrains from `g`, not `f`
- Assignments: `x is param; f(x)` doesn't flow back to param
- Control flow: constraints in only-sometimes-executed branches
- User-defined methods: only hardcoded method names recognized

#### Mechanism 2: Call-Site-Driven (`call_site_expr`)

Walks ALL function bodies looking for calls to functions with unsolved params:

```
For each call f(arg1, arg2, ...):
    If f's param[i] is unsolved AND arg[i] has known type:
        Resolve f's param[i] to arg[i]'s type
```

**Critical**: Uses `expr_ty_ast()` to compute argument types — NOT the full lowering engine. This means intermediate computations don't contribute.

#### The Destructive Final Step

After the fixed-point loop, `infer_param_types()` does:

```rust
// Lines 420-432 of resolve.rs
for k in keys {
    let entry = self.fns.get_mut(&k).unwrap();
    for ty in &mut entry.1 {
        if matches!(ty, Type::Inferred | Type::TypeVar(_)) {
            *ty = self.infer_ctx.resolve(ty);  // UNSOLVED → I64 !!!
        }
    }
    if entry.2.has_type_var() || entry.2 == Type::Inferred {
        entry.2 = self.infer_ctx.resolve(&entry.2);  // UNSOLVED → I64 !!!
    }
}
```

**This is the single most damaging operation in the entire type system.** It replaces every unsolved TypeVar with I64 BEFORE the real lowering pass even begins. The lowering pass (which has proper unification) can never recover these lost TypeVars.

### 2.6 AST-Level Type Synthesis (infer.rs — 655 lines)

**Status: SOUND for what it does, SEVERELY LIMITED**

`expr_ty_ast()` is a parallel, simpler type system running on raw ASTs. It has:
- Direct synthesis for all literal types
- Variable lookup (from current scope only)
- Call return from registered `self.fns`
- Generic return inference with `infer_depth` guard (max 4)
- Method return type tables (hardcoded for String, struct methods)
- **30+ `Type::I64` fallbacks** for unknown cases

`infer_ret_ast()` walks function body collecting return/tail types. Uses `pick_better_ret()`:
```rust
fn pick_better_ret(current: &mut Type, candidate: &Type) {
    if *current == Type::Void
        || (*current == Type::I64 && *candidate != Type::I64 && *candidate != Type::Void) {
        *current = candidate.clone();
    }
}
```

This is a **heuristic preference ordering**: Void < I64 < anything else. First non-trivial type wins.

**Problems**:
- Conflicting branches: if-then returns String, if-else returns Bool → picks whichever is seen first
- No unification between branches
- Recursive functions hit depth limit and return I64
- Parameters are unknown at this point, so body walk has limited information

### 2.7 Return Type Inference

**TWO COMPETING MECHANISMS**:

1. **AST-based** (`infer_ret_ast()` in `declare_fn_sig()`): Pre-lowering, heuristic
2. **HIR-based** (`refine_ret_from_body()`): Post-lowering per-function, also heuristic

Neither uses unification. Both use the `pick_better_ret()` preference order.

### 2.8 Monomorphization (mono.rs — 394 lines)

**Status: COMPLETE and CORRECT**

The monomorphization system handles generics well:
- Type-parameter substitution
- Name mangling for monomorphized instances
- Recursive monomorphization with depth guard (64)
- Generic enum support (Option, Result)
- Trait-bound checking

**One I64 fallback**: `p.ty.clone().unwrap_or(Type::I64)` at line 159. Should use `Type::Param`.

### 2.9 Post-Lowering Resolution (lower.rs — 1421 lines)

**Status: COMPLETE and CORRECT**

`resolve_all_types()` walks the entire HIR tree and calls `self.infer_ctx.resolve()` on every type. This is the proper resolution pass. It recursively visits:
- All functions (params, return, body)
- All type definitions (fields, methods)
- All enum definitions (variant fields)
- All externs, error definitions, actors, stores, trait impls
- Every expression, statement, pattern in the HIR

**This pass is correct.** The problem is that by the time it runs, most TypeVars have already been prematurely resolved to I64 by `infer_param_types()`.

---

## 3. Systematic I64 Fallback Catalog

### 3.1 Complete Enumeration

Every site in the typer where the compiler produces `Type::I64` as a fallback or default:

#### CRITICAL — Premature resolution (2 sites)

| # | Location | Code | Impact |
|---|----------|------|--------|
| C1 | `unify.rs:244` | `resolve()` unsolved TypeVar → I64 | All unsolved types become I64 |
| C2 | `unify.rs:260` | `resolve()` Type::Inferred → I64 | Legacy sentinel resolved to I64 |

#### HIGH — In lowering, affects type correctness (14 sites)

| # | Location | Code | Should Be |
|---|----------|------|-----------|
| H1 | `expr.rs:78` | None literal → I64 | fresh_var() or bottom |
| H2 | `expr.rs:143` | Unknown ident → I64 | error |
| H3 | `expr.rs:206-214` | Unknown struct field → I64 | error |
| H4 | `expr.rs:230` | Tuple index `.first()` → I64 | computed index type |
| H5 | `expr.rs:375` | Deref non-pointer → I64 | error |
| H6 | `expr.rs:389` | ListComp bind from non-array → I64 | fresh_var() |
| H7 | `expr.rs:1238` | Indirect call non-fn → I64 | error |
| H8 | `expr.rs:1382` | Unknown method → I64 | error |
| H9 | `expr.rs:1515` | Pipe non-fn RHS → I64 | error |
| H10 | `expr.rs:1585` | Pipe computed → I64 | error |
| H11 | `stmt.rs:76` | TupleBind non-tuple → vec[I64] | error |
| H12 | `stmt.rs:82` | TupleBind OOB → I64 | error |
| H13 | `stmt.rs:146` | Unknown iterable → I64 | error |
| H14 | `stmt.rs:456` | Pattern field OOB → I64 | error |

#### MODERATE — In pre-lowering heuristics (12 sites)

| # | Location | Code | Notes |
|---|----------|------|-------|
| M1 | `infer.rs:26` | Unknown ident → I64 | Acceptable for heuristic |
| M2 | `infer.rs:96` | Unknown call return → I64 | Acceptable for heuristic |
| M3 | `infer.rs:115` | Unknown field → I64 | Acceptable for heuristic |
| M4 | `infer.rs:121` | Array elem → I64 | Acceptable for heuristic |
| M5-M12 | `infer.rs:131-203` | Various AST fallbacks | Acceptable for heuristic |

#### LOW — In monomorphization (3 sites)

| # | Location | Code | Should Be |
|---|----------|------|-----------|
| L1 | `mono.rs:159` | Generic param base → I64 | Type::Param(name) |
| L2 | `mono.rs:321` | Unfilled type param → I64 | error or warning |
| L3 | `mono.rs:344` | Unfilled type param → I64 | error or warning |

#### LEGITIMATE — Correct I64 (5 sites)

| # | Location | Code | Why Correct |
|---|----------|------|-------------|
| OK1 | `expr.rs:54` | Int literal → I64 | Integer literals ARE I64 |
| OK2 | `expr.rs:357` | Placeholder → I64 | Pipe placeholder semantics |
| OK3 | `stmt.rs:131` | Range for bind → I64 | Range iteration IS I64 |
| OK4 | `lower.rs:65` | Store field unwrap → I64 | Store fields default I64 |
| OK5 | `lower.rs:692` | Lower fn param → I64 | ... but should be from sig |

**Total**: 2 CRITICAL + 14 HIGH + 12 MODERATE + 3 LOW + 5 LEGITIMATE = **36 sites**

### 3.2 Impact Assessment

The **2 CRITICAL sites** in `resolve()` are the root cause. They are called:
- At end of `infer_param_types()` → destroys all param TypeVars
- In `resolve_all_types()` → final resolution (this call is correct in principle but defaults are wrong)

If the critical sites used smarter defaulting, **many HIGH sites would become unreachable** because TypeVars would be properly constrained by the time resolution happens.

---

## 4. Where Annotations Are Currently Required

### 4.1 Annotation Classification

| Scenario | Annotation Needed? | Root Cause |
|----------|-------------------|------------|
| Function params with known call-sites | NO (usually) | Pre-pass infers from callers |
| Function params only forwarded | **YES** | No concrete usage constraint |
| Function return when obvious | NO | tail/return expression inferred |
| Function return with branch mismatch | **YES** | pick_better_ret heuristic fails |
| Struct fields | **YES** (no default) | Only default values infer type |
| Actor handler params | **YES** (usually) | Async: no direct call-site |
| Lambda params in callbacks | SOMETIMES | Expected type propagation partial |
| Generic function bodies | NO | Monomorphization handles |
| Extern functions | **YES** (by design) | FFI requires explicit types |
| Variable bindings | NO | RHS type flows to LHS |
| For-loop variables | NO | Collection element type |
| Match bindings | NO | Subject type propagates |

### 4.2 Estimated Current Coverage

| Mechanism | Contribution | Notes |
|-----------|-------------|-------|
| Literal synthesis | ~15% | All literals typed |
| RHS → LHS binding flow | ~15% | `x is expr` |
| Explicit annotations | ~10% | User provides |
| Monomorphization | ~5% | Full generic support |
| Pre-pass body inference | ~5% | Heuristic, limited |
| Pre-pass call-site inference | ~3% | Forward pass |
| AST return inference | ~3% | Heuristic |
| Expected type propagation | ~2% | Lambda, array, arg |
| **Total** | **~58%** | |

---

## 5. Cutting-Edge Research Comparison

### 5.1 Relevant Papers and Systems

| System | Year | Key Innovation | Relevance to Jade |
|--------|------|---------------|-------------------|
| **Algorithm W** (Damas-Milner) | 1982 | Principal type inference | Foundation for constraint solving |
| **Algorithm J** (optimized W) | 1982 | Efficient substitution | Implementation technique |
| **Bidirectional Typing** (Pierce-Turner) | 2000 | Synth ↑ / Check ↓ modes | Framework for expected type propagation |
| **Local Type Inference** (Odersky) | 2001 | Annotation-minimal OOP | Less ambitious than Jade's goal |
| **OutsideIn(X)** (Vytiniotis) | 2011 | Constraint solver separation | Architecture pattern |
| **MLsub** (Dolan) | 2017 | Principal types + subtyping | Elegant but complex |
| **Complete/Easy Bidirectional** (Dunfield-Krishnaswami) | 2021 | Higher-rank + completeness | State of the art |
| **simple-sub** (Parreaux) | 2020 | Simplified MLsub | Practical implementation guide |
| **Luau** (Roblox) | 2021 | Gradual HM in production | Real-world type inference |
| **TypeScript** | ongoing | Structural + contextual | Expected-type propagation patterns |

### 5.2 Key Theoretical Results

**Theorem (Damas-Milner, 1982)**: For the simply-typed lambda calculus with let-polymorphism (rank-1), there exists an algorithm that:
1. Always terminates
2. Infers the most general (principal) type
3. Requires **zero** type annotations

**Corollary**: Jade, being monomorphized (effectively rank-0/1), falls within this decidability class. Zero annotations is theoretically achievable for the core language.

**Theorem (Pierce-Turner, 2000)**: Bidirectional type checking can be made complete for rank-1 types by systematic use of synthesis and checking modes.

**Implication**: Jade should implement systematic bidirectional checking, which guarantees that every rank-1 expression can be typed without annotation.

### 5.3 What Jade Should Adopt

1. **From HM**: Constraint generation during traversal, solve via unification ← **already have the engine**
2. **From Bidirectional**: Systematic synth/check modes ← **partially have, need completion**
3. **From OutsideIn(X)**: Separation of constraint generation and solving ← **architecturally needed**
4. **From TypeScript**: Contextual typing for callbacks/lambdas ← **partially have**
5. **From Luau**: Gradual defaulting for unconstrained vars ← **need smart defaults**

---

## 6. Ten Gaps Between Current State and Zero Annotations

### Gap 1: PREMATURE TYPEVAR RESOLUTION (CRITICAL)

`infer_param_types()` resolves ALL TypeVars to concrete (defaulting unsolved → I64) BEFORE the main lowering pass runs. The lowering pass has proper unification but can never correct prematurely-resolved types.

**Mathematical formulation**: Let σ be the unifier computed by the lowering pass. Let σ₀ be the premature resolution. The system computes σ ∘ σ₀ instead of σ. Since σ₀ maps unsolved variables to I64, and σ cannot override a concrete type, the result σ ∘ σ₀ loses all information from σ about those variables.

### Gap 2: DUAL INFERENCE SYSTEMS (CRITICAL)

Two separate, incompatible type inference systems compete:
- **AST-level** (`expr_ty_ast`, `infer_ret_ast`): No unification, heuristic
- **HIR-level** (`lower_expr_expected`, `InferCtx`): Proper unification

The AST system runs first, produces inferior results, and the HIR system inherits them.

### Gap 3: HEURISTIC RETURN TYPE INFERENCE (HIGH)

`pick_better_ret()` has arbitrary preference: Void < I64 < anything. First non-trivial type wins. Should use unification to verify all return paths agree.

### Gap 4: INCOMPLETE EXPECTED-TYPE PROPAGATION (HIGH)

Missing in: match arms, block expressions, pipe chains, index backward constraints.

### Gap 5: NO NUMERIC LITERAL POLYMORPHISM (MODERATE)

`42` is always I64. Should be polymorphic: `42 : α where Num(α)`, defaulting to I64 if unconstrained.

### Gap 6: STRUCT FIELD INFERENCE LIMITED (MODERATE)

Fields without defaults get TypeVars but these are unified only within function scope, not cross-function.

### Gap 7: MAP KEY ASSUMPTION (LOW-MODERATE)

`map()` forces `String` keys. Should be `fresh_var()`.

### Gap 8: ACTOR HANDLER PARAM INFERENCE (LOW)

No call-site for handler params (async messages). But `send()` expressions provide constraints that could flow.

### Gap 9: RECURSIVE FUNCTION DEPTH LIMIT (LOW)

`infer_depth < 4` guard prevents deep recursive inference. With proper TypeVars and unification, recursion is handled naturally.

### Gap 10: TYPE::INFERRED LEGACY (LOW)

`Type::Inferred` should be replaced entirely with `TypeVar`. Having two "unknown" markers causes inconsistency.

---

## 7. Remediation Plan — 10 Phases

### PHASE 1: Keep TypeVars Alive Through Lowering

**Objective**: Remove the premature resolution in `infer_param_types()`.

**Changes**:
- In `infer_param_types()` (resolve.rs:420-432): Remove the final loop that calls `self.infer_ctx.resolve()` on all function signature types
- Keep TypeVars alive in `self.fns` entries
- TypeVars will be solved during `lower_call()` → `unify_at(param_ty, arg.ty)` in the lowering pass
- Final resolution happens only in `resolve_all_types()`

**Files**: `src/typer/resolve.rs`  
**Risk**: Medium — some code paths may not handle TypeVars in signatures  
**Verification**: All 930 tests must pass  
**Estimated annotation-free gain**: +7% → ~65%

### PHASE 2: Return Type as TypeVar

**Objective**: Eliminate heuristic return type inference in `declare_fn_sig()`.

**Changes**:
- In `declare_fn_sig()`: if no explicit return type, use `self.infer_ctx.fresh_var()` instead of `infer_ret_ast()`
- In `declare_method_sig()` and `declare_method_sig_by_ptr()`: same
- `lower_fn()` body lowering already unifies return stmt types with `ret_ty`
- Remove `pick_better_ret()` — unification replaces it
- Remove `refine_ret_from_body()` — no longer needed

**Files**: `src/typer/resolve.rs`, `src/typer/lower.rs`  
**Risk**: Medium — return type must be solved before callers need it  
**Dependency**: Phase 1  
**Estimated annotation-free gain**: +7% → ~72%

### PHASE 3: Eliminate Dual Inference System

**Objective**: Remove the `infer_param_types()` pre-pass entirely.

**Changes**:
- Delete `infer_param_types()` and all its helper methods (~200 lines in resolve.rs)
- Delete `constraint_from_expr()`, `constraint_from_body()`, `call_site_expr()`, `call_site_stmt()`
- Keep `expr_ty_ast()` only for: monomorphization type-map, diagnostics
- In `declare_fn_sig()`: all params without annotations → `fresh_var()`, returns → `fresh_var()`
- In `lower_program()`: remove `self.infer_param_types(prog)` call

**Files**: `src/typer/resolve.rs`, `src/typer/lower.rs`  
**Risk**: High — removes ~300 lines of inference code  
**Dependency**: Phases 1, 2  
**Estimated annotation-free gain**: +6% → ~78%

### PHASE 4: Complete Bidirectional Propagation

**Objective**: Propagate expected types into all remaining sub-expression forms.

**Sub-tasks**:

**4a. Match expression arms**: Pass expected type into arm body lowering
```rust
// In lower_match: arm body gets expected type from match expression context
let body = self.lower_block_with_expected(&a.body, ret_ty, expected)?;
```

**4b. Block tail expression**: `lower_block()` accepts optional expected type, propagates to last expression
```rust
fn lower_block_expected(&mut self, block: &Block, ret_ty: &Type, 
                        expected: Option<&Type>) -> Result<hir::Block, String>
```

**4c. Pipe expression**: Return type of pipe chain flows backward into lambda synthesis

**4d. For-each body**: Element type available in body scope for further inference

**4e. Let-bound expected**: If a later usage constrains a variable, the constraint flows back through unification (already works with TypeVars from Phase 1)

**Files**: `src/typer/expr.rs`, `src/typer/stmt.rs`  
**Risk**: Medium — each sub-task independent and testable  
**Dependency**: Phases 1-3  
**Estimated annotation-free gain**: +5% → ~83%

### PHASE 5: Numeric Literal TypeVars

**Objective**: Integer literals get polymorphic numeric types.

**Changes**:
- Add `InferCtx::fresh_numeric_var()` that marks the TypeVar as numeric
- `lower_expr(Int(n))` → `TypeVar` with numeric tag, not `I64`
- During unification: numeric TypeVar unifies only with numeric types
- During resolution: unsolved numeric TypeVar defaults to `I64`
- Float literals: similar, default to `F64`

**Implementation**:
```rust
// In InferCtx:
constraints: Vec<TypeConstraint>,  // new field

enum TypeConstraint {
    None,
    Numeric,    // I8-U64, F32-F64
    Integer,    // I8-U64 only
    Float,      // F32-F64 only
}
```

**Files**: `src/typer/unify.rs`, `src/typer/expr.rs`  
**Risk**: Medium — affects all arithmetic code  
**Dependency**: Phase 1  
**Estimated annotation-free gain**: +3% → ~86%

### PHASE 6: Smart Defaulting Rules

**Objective**: Replace blind `unsolved → I64` with context-aware defaults.

**Rules** (in priority order):
1. Numeric-constrained TypeVar → I64 (integer) or F64 (float)
2. TypeVar with constraint origins → follow dominant constraint
3. TypeVar in function parameter position → emit "cannot infer, add annotation" error
4. TypeVar in struct field → emit error
5. TypeVar with zero constraints → I64 with warning (opt-in via flag)

**Implementation**:
```rust
fn resolve_with_defaults(&mut self, ty: &Type) -> Result<Type, String> {
    match ty {
        Type::TypeVar(v) => {
            let root = self.find(*v);
            if let Some(resolved) = self.types[root as usize].clone() {
                self.resolve_with_defaults(&resolved)
            } else {
                match self.constraints[root as usize] {
                    TypeConstraint::Numeric => Ok(Type::I64),
                    TypeConstraint::Float => Ok(Type::F64),
                    _ => Err(format!("cannot infer type for ?{root}"))
                }
            }
        }
        // ... recurse structurally
    }
}
```

**Files**: `src/typer/unify.rs`, `src/typer/lower.rs`  
**Risk**: Low — orthogonal to other phases  
**Dependency**: All previous phases  
**Estimated annotation-free gain**: +2% → ~88%

### PHASE 7: Cross-Function Constraint Sharing

**Objective**: Ensure TypeVars in function signatures are shared through Union-Find.

**Verification**: After Phase 1, when `f()` calls `g(x)`:
1. `declare_fn_sig(g)` creates TypeVar(42) for `g`'s param
2. `self.fns["g"] = (id, [TypeVar(42)], retvar)`
3. During `lower_fn(f)`, `lower_call(g, args)` does `unify_at(TypeVar(42), arg.ty)`
4. TypeVar(42) is now solved
5. When `lower_fn(g)` runs later, `self.fns["g"]`'s param is already solved

**Key**: TypeVar(u32) is an index into InferCtx's arrays. Cloning TypeVar(42) creates another reference to the SAME Union-Find entry. So this should work automatically.

**What to verify**: That `self.fns.get("g").1[0]` returns `TypeVar(42)` (not a cloned+resolved copy).

**Files**: Verification only  
**Risk**: Low  
**Dependency**: Phase 1  
**Estimated annotation-free gain**: +2% → ~90%

### PHASE 8: Eliminate Remaining I64 Fallbacks

**Objective**: Replace 14 HIGH priority I64 fallbacks with `fresh_var()` or proper errors.

**Changes** (ordered by impact):

| Site | Current | Fix |
|------|---------|-----|
| `expr.rs:78` (None) | I64 | fresh_var() |
| `expr.rs:143` (unknown ident) | I64 | error("undefined") |
| `expr.rs:206-214` (unknown field) | I64 | error("no field") |
| `expr.rs:375` (bad deref) | I64 | error("cannot deref") |
| `expr.rs:1238` (non-fn call) | I64 | error |
| `expr.rs:1382` (unknown method) | I64 | error |
| `stmt.rs:76` (non-tuple bind) | vec[I64] | error |
| `stmt.rs:456` (pat field OOB) | I64 | error |
| `stmt.rs:476-488` (pat types) | I64 | fresh_var() |
| `mono.rs:159` | I64 | Type::Param(name) |
| `mono.rs:321,344` | I64 | error |

**Files**: `src/typer/expr.rs`, `src/typer/stmt.rs`, `src/typer/mono.rs`  
**Risk**: Low — each change isolated  
**Dependency**: Phases 1-3 (many become unreachable)  
**Estimated annotation-free gain**: +1% → ~91%

### PHASE 9: Map Key and Miscellaneous Inference

**Objective**: Fix remaining edge cases.

**Changes**:
- `map()` → `Map(fresh_var(), fresh_var())`
- Actor handler params constrained from `send()` expressions
- `Type::Inferred` replaced with `TypeVar` globally
- String iteration type → `U8` or character type (not I64)

**Files**: `src/typer/expr.rs`, `src/types.rs`  
**Risk**: Low  
**Estimated annotation-free gain**: +1% → ~92%

### PHASE 10: Diagnostics and Provenance

**Objective**: When inference fails, explain why clearly.

**Changes**:
- All `unify()` calls → `unify_at()` with span and reason
- On unification error: show constraint provenance chain
- On unsolved TypeVar: show where introduced, what was tried
- On wrong default: suggest minimal annotation

**Files**: `src/typer/unify.rs`, all lowering files  
**Risk**: Very low  

---

## 8. Implementation Priority Graph

```
                    ┌─── Phase 5 (numeric literals) ───────┐
                    │                                       │
Phase 1 ──→ Phase 2 ──→ Phase 3 ──→ Phase 4 ──→ Phase 6 ──→ Phase 10
(keep TVars) (ret TVars) (remove pre) (bidir)    (defaults)  (diagnostics)
    │                                    ↑
    ├─── Phase 7 (cross-fn verify) ─────┘
    │
    └─── Phase 9 (map/misc) ──→ Phase 8 (eliminate I64)
```

**Critical path**: 1 → 2 → 3 → 4 (delivers 78% → 83%)  
**Full path**: All 10 phases (delivers ~92%)

### Expected Coverage Over Phases

| Phase | Cumulative Coverage | Annotation Reduction |
|-------|-------------------|---------------------|
| Current | ~58% | baseline |
| 1 (keep TypeVars) | ~65% | params auto-solved from call-sites |
| 2 (return TypeVars) | ~72% | returns auto-solved from body |
| 3 (remove pre-pass) | ~78% | eliminates heuristic interference |
| 4 (bidirectional) | ~83% | match/block/pipe propagation |
| 5 (numeric) | ~86% | literal polymorphism |
| 6 (defaults) | ~88% | smarter unsolved handling |
| 7 (cross-fn) | ~90% | inter-procedural flow |
| 8 (fallbacks) | ~91% | error not wrong type |
| 9-10 (misc) | ~92% | edge cases, diagnostics |

**Remaining 8%**: Genuinely ambiguous cases (forwarded-only params, unannotated struct fields without defaults, actor handlers without matching sends). These require annotations by semantic necessity.

---

## 9. Validation Strategy

### 9.1 Phase Validation

Each phase must:
1. ✅ All 930 existing tests pass
2. ✅ New tests exercise the specific inference improvement
3. ✅ No TypeVars escape to codegen (check in `resolve_all_types`)
4. ✅ Run `--debug-types` on benchmark programs to verify constraint solving

### 9.2 Annotation Reduction Metric

Create `tests/programs/inference_test.jade` with programs that currently require annotations. After each phase, attempt to remove annotations and verify correctness.

### 9.3 Regression Canaries

Programs that are sensitive to type inference correctness:
- `neural_net.jade` — f64 arithmetic throughout
- `crypto.jade` — i64 bit operations
- `ecs.jade` — struct methods with mixed types
- `data_structures.jade` — generics and containers
- `string_processing.jade` — String inference critical

### 9.4 Property-Based Invariants

After each phase, verify:
1. **Completeness**: No TypeVar appears in final HIR
2. **Soundness**: All `unify_at()` calls succeed or produce diagnostic
3. **Monotonicity**: Adding an annotation never changes inferred types of OTHER expressions
4. **Stability**: Same program produces same types across compilations

---

## 10. Files to Modify (Complete Map)

| File | Lines | Phases | Nature of Changes |
|------|-------|--------|-------------------|
| `src/typer/resolve.rs` | 823 | 1,2,3,7 | Major: remove pre-pass, modify signature declarations |
| `src/typer/unify.rs` | 483 | 5,6 | Moderate: add constraints, smart defaulting |
| `src/typer/expr.rs` | 1742 | 4,8 | Moderate: complete bidirectional, fix fallbacks |
| `src/typer/stmt.rs` | 498 | 4,8 | Moderate: block expected type, fix fallbacks |
| `src/typer/lower.rs` | 1421 | 2,4 | Moderate: return TypeVar, block expected type |
| `src/typer/infer.rs` | 655 | 3,8 | Major: reduce role, fix fallbacks |
| `src/typer/mono.rs` | 394 | 8 | Minor: fix generic param defaults |
| `src/typer/mod.rs` | 345 | 1 | Minor: keep TypeVars in signatures |
| `src/types.rs` | 184 | 5,9 | Minor: possibly add TypeVar metadata |

---

## 11. Conclusion

Jade's type inference has a **sound unification engine** and **partial bidirectional checking** — the hard algorithmic parts are in place. The system is hamstrung by an **architectural problem**: a legacy heuristic pre-pass that prematurely resolves TypeVars before the real inference engine runs.

The remediation is primarily **engineering, not research**:
1. Stop killing TypeVars early (Phase 1)
2. Use TypeVars for return types (Phase 2)
3. Let the unification engine do its job (Phase 3)
4. Complete the bidirectional framework (Phase 4)

No novel type theory is needed. The work uses well-understood algorithms from 40+ years of research (HM, bidirectional, Union-Find). Production languages (Haskell, OCaml, TypeScript, Kotlin) have proven these techniques at scale.

The critical path is **4 phases** to reach ~83% annotation-free coverage. The full 10-phase plan reaches ~92%, with the remaining 8% being cases where annotations carry genuine semantic information.
