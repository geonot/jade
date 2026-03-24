
# Jade Type Inference System: Complete Technical Analysis & Remediation Plan

## 1. Architecture Overview

The Jade type inference system spans ~8,200 lines across 10 files. It operates in a **multi-phase pipeline**:

| Phase | File | Role |
|-------|------|------|
| 1. Registration | resolve.rs | Registers all declarations, assigns `TypeVar`s to unannotated positions |
| 2. Heuristic seeding | resolve.rs | `infer_param_types()` — AST heuristic pre-seeds return types |
| 3. Lowering + Unification | lower.rs, expr.rs, stmt.rs | Walks AST → HIR, emitting `unify_at()` constraints |
| 4. Resolution | lower.rs | `resolve_all_types()` — walks entire HIR, replaces `TypeVar`s with concrete types |
| 5. Monomorphization | mono.rs | Instantiates generics with concrete types |

The core constraint solver lives in unify.rs — an `InferCtx` implementing **union-find with path compression and ranked union** for type variable equivalence classes.

---

## 2. What Currently Works (Strengths)

### 2.1 Union-Find Unification Engine (`InferCtx`)
- **Well-implemented**: Path-compressed union-find with ranked union (unify.rs)
- **Occurs check**: Prevents infinite types (unify.rs)
- **Structural unification**: Correctly recurses through `Array`, `Vec`, `Map`, `Tuple`, `Fn`, `Ptr`, `Rc`, `Weak`, `Channel`, `Coroutine` (unify.rs)
- **Constraint provenance**: Records origin spans/reasons for diagnostics (unify.rs)
- **Type constraint classes**: Distinguishes `Numeric`/`Integer`/`Float`/`None` and merges correctly (unify.rs)
- **Transitive resolution**: Works correctly, well-tested

### 2.2 Forward Inference (What "Just Works")
- **Literals**: `42` → `I64`, `3.14` → `F64`, `"hi"` → `String`, `true` → `Bool`
- **Variable bindings**: `x is 42` → `x: I64`, `x is "hello"` → `x: String`
- **Struct constructors**: `Point(x is 1, y is 2)` → `Point`
- **Enum constructors**: `Red` → `Color`, `Some(42)` → `Option_i64`
- **Binary operators**: Comparison → `Bool`, `+` propagates operand type
- **Field access**: `p.x` → field type from struct schema
- **Array literals**: Element types unified
- **Monomorphization**: Generic functions instantiated based on call-site argument types
- **Bidirectional expected types**: `lower_expr_expected()` propagates expected types downward into lambdas, arrays, ternaries, blocks

### 2.3 Annotation-Free Features Already Working
Programs like `fib(n)` work without any annotations — the function is treated as generic, monomorphized at call site with `I64`.

---

## 3. Critical Deficiencies (Rigorous Analysis)

### **D1: Dual Inference Systems — AST Heuristic vs. Unification (FUNDAMENTAL)**

The system has **two entirely separate type inference mechanisms** that operate in tension:

1. **`expr_ty_ast()` / `infer_ret_ast()`** — infer.rs: A separate, incomplete, AST-walking type estimator that doesn't use unification at all. It's a **pattern-matching heuristic** that guesses types by walking the AST without building constraints.

2. **`InferCtx` (union-find unification)** — unify.rs: The real constraint solver, used during HIR lowering.

**The problem**: `expr_ty_ast()` is called *before* lowering to seed function signatures, but it operates without unification context. Its results are often wrong or overly conservative:

- infer.rs: Unknown identifiers default to `I64`
- infer.rs: Failed generic call resolution defaults to `I64`
- infer.rs: Array indexing of unknown type → `I64`
- infer.rs: Lambda params without annotations → `I64`
- infer.rs: Pipe with non-Fn right side → `I64`
- infer.rs: Unknown method → `I64`

The `I64` fallback permeates the system. It's the "give up" type.

**Impact**: Functions using complex expressions (method chains, generic calls, closures) get their return types wrong during Phase 2 seeding, and the Phase 3 unification may not correct them because the wrong seed is already committed.

### **D2: No Backward Inference for Parameter Types**

The comment in resolve.rs explicitly states:

```
// Phase 3: Eliminate the dual inference system.
// The iterative body-driven and call-site heuristic loops are removed.
// TypeVars in function signatures survive into the lowering pass where
// proper unification via unify_at() at call sites will solve them.
```

This is the **aspiration** but the **reality** is that call-site unification only works in one direction: the call site constrains the `TypeVar` in the parameter position, but **the function body was already lowered with the TypeVar unsolved**. The body expressions were typed using the unsolved `TypeVar`, and those types never get re-evaluated after the `TypeVar` is solved.

**Missing**: There is no **backward pass** — once the function body is lowered, the constraint solutions from call sites don't flow back into the body's type decisions. The `resolve_all_types()` pass handles mechanical `TypeVar` → concrete substitution, but doesn't re-check type consistency.

### **D3: Single-Pass Function Processing (No Fixed-Point)**

Functions are lowered in **declaration order** (lower.rs). If function `A` calls function `B` defined later, `B`'s signature may still contain `TypeVar`s when `A` is lowered. There is **no iterative fixed-point** to handle mutual recursion or out-of-order dependencies.

The generic function mechanism (monomorphize-on-demand) partially papers over this for explicitly generic functions, but not for non-generic functions with unresolved types.

### **D4: `Type::Inferred` is a Dead Concept**

`Type::Inferred` exists in the type enum (types.rs) but is handled inconsistently:
- In unification: `(Type::Inferred, _) | (_, Type::Inferred) => Ok(())` — silently accepts any type (unify.rs)
- In resolution: `Type::Inferred => Type::I64` — defaults to `I64` (unify.rs)
- In normalization: converted to fresh `TypeVar` (resolve.rs)

This creates a hole: any position typed as `Inferred` silently accepts everything during unification (it's like a wildcard `_` in pattern matching), then gets resolved to `I64`. This is **unsound** — it can mask genuine type errors.

### **D5: No Numeric Literal Polymorphism**

Integer literals are hard-coded to `I64` (expr.rs), float literals to `F64` (expr.rs). The `fresh_numeric_var()` method exists (unify.rs) and `TypeConstraint` classes exist (`Numeric`, `Integer`, `Float`) but are **never used** during literal lowering.

This means `x: i32 = 42` requires an explicit cast or annotation — the `42` is always `I64`, never a polymorphic numeric literal that could unify with `I32`.

### **D6: No Return Type Inference from Multiple Return Points**

`refine_ret_from_body()` (infer.rs) uses `pick_better_ret()` which is a heuristic:
```rust
if *current == Type::Void
    || (*current == Type::I64 && *candidate != Type::I64 && *candidate != Type::Void)
{
    *current = candidate.clone();
}
```

This is **not** a greatest-common-type or unification — it simply picks the first "interesting" type it encounters. If a function has multiple return paths returning different types, no join/meet/unification is computed.

### **D7: Generic Functions Use `I64` as Default Type Parameter**

In monomorphization (mono.rs):
```rust
for tp in &gf.type_params {
    type_map.entry(tp.clone()).or_insert(Type::I64);
}
```

If the call-site argument doesn't have an explicit type annotation, and the heuristic `expr_ty_ast()` returns `I64` (its fallback), then the generic is always instantiated with `I64` regardless of the actual runtime type.

### **D8: Method Call Type Inference Relies on AST Heuristic**

In `lower_method_call()` (expr.rs):
```rust
let obj_ty = self.expr_ty_ast(obj);
```

The object type is computed using the heuristic `expr_ty_ast()` **before lowering the object expression**. If the object's type is complex (e.g., result of a generic call, a chain of method calls), this heuristic will return `I64`, and method dispatch will fail or produce wrong types.

### **D9: Lambda Parameter Inference Only One Level Deep**

Lambda parameters without annotations get their types from:
1. Expected type context (if the lambda is passed to a typed parameter) — works
2. Otherwise: `self.infer_ctx.fresh_var()` — creates a TypeVar

But the TypeVar for lambda params is only solved if the lambda is **immediately** passed to a function with known parameter types. If the lambda is stored in a variable and called later, or if the expected type itself contains TypeVars, the param TypeVar stays unsolved and defaults to `I64`.

### **D10: No Constraint Propagation Through Data Structures**

If you create a `Vec` and push elements of different types into it, or if you want the element type of a `Vec` to propagate backward to constrain earlier expressions, this doesn't happen. Each container creation site fixes its element type from the first element or from a fresh TypeVar, with no global constraint propagation.

### **D11: `Bool` Coercion in `UnaryOp::Not` is Wrong for Bitwise**

expr.rs: `UnaryOp::Not` always produces `Type::Bool`, but `Not` should produce the operand's type for bitwise-not on integers. This is a type inference correctness issue.

### **D12: No Flow-Sensitive Typing**

The type system has no notion of narrowing types through conditionals. After `if x is Some(v)`, `v` is correctly bound in the match pattern, but there's no mechanism to narrow `x` itself from `Option` to `Some` in the then-branch outside of pattern matching.

---

## 4. What State-of-the-Art Research Recommends

### 4.1 Hindley-Milner with Let-Polymorphism
Jade's system is **not** HM. It has no `let`-polymorphism (no generalization step), no rigid type schemes, no instantiation discipline. Functions without explicit type params are treated as monomorphic, and the "generic" mechanism is really just template-style monomorphization from explicit `type_params`.

**What Jade needs**: Algorithm W or Algorithm J for HM inference with proper let-generalization.

### 4.2 Bidirectional Type Checking (Pierce & Turner 2000, Dunfield & Krishnaswami 2021)
Jade has **partial** bidirectional typing: `lower_expr_expected()` passes expected types downward. But it's incomplete — it only works for a few expression forms (lambdas, arrays, ternaries, blocks, if-expressions). It doesn't propagate through binary operations, method calls, or function call return positions.

**What Jade needs**: Systematic bidirectional checking where every expression form has both a "check" mode (expected type pushed down) and a "synth" mode (type synthesized upward).

### 4.3 Local Type Inference (Pierce & Turner 2000)
The gold standard for practical type inference in languages with subtyping. Jade doesn't have subtyping, but the idea of constraining types locally at each expression (rather than globally) applies.

### 4.4 Constraint-Based Inference with Deferred Resolution
Languages like Kotlin, Swift, and Rust use constraint-based systems where type constraints are collected first, then solved globally. Jade's `InferCtx` is already a constraint solver — but constraints are emitted and solved **incrementally** during lowering rather than being collected first and solved later.

### 4.5 Row Polymorphism (for struct inference)
Would allow inferring struct field types without annotations: `f(x) = x.name` would work for any struct with a `name` field. Jade doesn't have this — all struct field accesses require knowing the concrete struct type.

---

## 5. Remediation Plan

### Phase 1: Eliminate the Dual System (Correctness Foundation)

**Goal**: Remove `expr_ty_ast()` / `infer_ret_ast()` entirely. All type inference goes through `InferCtx`.

#### 1.1 — Numeric Literal Polymorphism
- In `lower_expr()` for `Int(n)`, emit `self.infer_ctx.fresh_numeric_var()` (with `TypeConstraint::Integer`) instead of hard-coded `I64`
- For `Float(n)`, emit `fresh_var()` with `TypeConstraint::Float` instead of hard-coded `F64`
- In `resolve()`, default unsolved `Integer` → `I64`, `Float` → `F64` (this already exists)
- **Result**: `x: i32 = 42` works without casts; arithmetic preserves type correctly

#### 1.2 — Replace `expr_ty_ast()` in Method Dispatch
- In `lower_method_call()`, lower the object expression **first** to get its HIR type, then dispatch based on that
- Similarly in `lower_call()` for generic dispatch
- Remove all calls to `expr_ty_ast()` from the lowering path

#### 1.3 — Function Signature Registration with TypeVars Only
- In `declare_fn_sig()`: assign `TypeVar` to **every** unannotated position (params and return). Remove the AST heuristic seeding entirely.
- Remove `infer_param_types()` — it's the largest vestige of the dual system
- **Result**: All inference flows through unification

#### 1.4 — Kill `Type::Inferred`
- Replace every `Type::Inferred` creation with `self.infer_ctx.fresh_var()`
- Remove the `(Type::Inferred, _) => Ok(())` case from unification
- **Result**: No more type-safety holes

### Phase 2: Multi-Pass Inference (Completeness)

#### 2.1 — Two-Pass Function Lowering
- **Pass 1 (Collection)**: Register all function signatures with TypeVars. Lower all function **bodies** to HIR. During lowering, call-site arguments emit `unify_at()` constraints on the callee's parameter TypeVars.
- **Pass 2 (Resolution)**: `resolve_all_types()` replaces all TypeVars. Validate type consistency.
- This naturally handles out-of-order function definitions.

#### 2.2 — Return Type Unification (Not Heuristic)
- When lowering a function body, create a `ret_var: TypeVar` 
- Every `return expr` and every tail expression unifies its type with `ret_var`
- Multiple return paths get unified together — the constraint solver finds the consistent type or reports an error
- Remove `refine_ret_from_body()` and `pick_better_ret()` — they're heuristics

#### 2.3 — Fixed-Point for Mutually Recursive Functions
- After the first lowering pass, check if any function's return TypeVar was solved by call-site constraints in other functions
- If solved types changed, re-lower affected functions (with a depth limit)
- Most code won't need this — it only triggers for mutual recursion without annotations

### Phase 3: Systematic Bidirectional Checking

#### 3.1 — Expected Type Propagation for All Expression Forms
Extend `lower_expr_expected()` to propagate through:
- **Binary operations**: `x + y` where `x: i32` → expect `y: i32`
- **Function call arguments**: already done for direct calls, extend to method calls and indirect calls
- **Match arms**: the match result type pushes down into each arm
- **If/elif/else branches**: the expected type pushes into all branches
- **Assignments**: `x = expr` where `x: T` → expect `expr: T`

#### 3.2 — Lambda Inference from Context
When a lambda is assigned to a variable or passed to a function:
- The expected `Fn(params, ret)` type flows down to supply parameter types
- Already partially working; extend to cover all positions (e.g., stored in containers, returned from functions)

### Phase 4: Improved Diagnostics & Robustness

#### 4.1 — Rich Error Messages with Constraint Origins
- Already have `ConstraintOrigin` — extend to show the **chain** of constraints that led to a type error
- When two TypeVars are unified and later one gets a conflicting concrete type, show both constraint origins

#### 4.2 — Unsolved TypeVar Warnings
- After `resolve_all_types()`, any TypeVar that defaulted to `I64` without being constrained should emit a warning
- This catches cases where the compiler silently guessed wrong

#### 4.3 — `BitNot` Operand Type Fix
- Fix `UnaryOp::Not` to return the operand type when the operand is an integer, `Bool` only when the operand is `Bool`

### Phase 5: Advanced Features (Future)

#### 5.1 — Let-Generalization
For functions like `*identity(x) x` — instead of treating all unannotated params as generic, implement proper let-generalization: the function is generalized over unconstrained TypeVars in its body, and instantiated fresh at each call site.

#### 5.2 — Flow-Sensitive Narrowing
After `match x { Some(v) => ... }`, narrow `x` to `Some` within the branch. After `if x != nil`, narrow `x` to non-nil.

#### 5.3 — Struct Field Inference
Allow structs with unannotated fields to have their field types inferred from usage (constructor calls, field access patterns).

---

## 6. Priority Matrix

| Item | Impact | Effort | Soundness Fix? | Annotation Reduction |
|------|--------|--------|----------------|---------------------|
| 1.1 Numeric polymorphism | High | Low | Yes | Medium |
| 1.2 Kill `expr_ty_ast()` in lowering | Critical | Medium | Yes | High |
| 1.3 Pure TypeVar signatures | Critical | Medium | Yes | High |
| 1.4 Kill `Type::Inferred` | High | Low | Yes | None |
| 2.1 Two-pass lowering | Critical | High | Yes | Very High |
| 2.2 Return type unification | High | Medium | Yes | High |
| 2.3 Fixed-point recursion | Medium | Medium | Yes | Medium |
| 3.1 Full bidirectional | High | High | No | Very High |
| 3.2 Lambda context inference | Medium | Low | No | Medium |
| 4.1 Rich diagnostics | Medium | Low | No | None |
| 4.2 Unsolved TypeVar warnings | Medium | Low | No (DX) | None |
| 4.3 BitNot fix | Low | Trivial | Yes | None |

## 7. Implementation Order

1. **Phase 1.4** (Kill `Type::Inferred`) — trivial, immediate soundness improvement
2. **Phase 1.1** (Numeric polymorphism) — low effort, enables `i32`/`u8` code without casts
3. **Phase 4.3** (BitNot fix) — trivial correctness fix
4. **Phase 1.2 + 1.3** (Kill dual system) — the big structural change: remove `expr_ty_ast()` from the lowering path, assign pure TypeVars in registration
5. **Phase 2.2** (Return type unification) — flows naturally from Phase 1.3
6. **Phase 2.1** (Two-pass lowering) — enables out-of-order definitions
7. **Phase 3.1 + 3.2** (Bidirectional) — polish pass for maximum annotation elimination
8. **Phase 2.3** (Fixed-point) — only after everything else works
9. **Phase 4.1 + 4.2** (Diagnostics) — DX improvement

**After Phases 1-3, the following annotations become unnecessary**:
- Function return types (inferred from body)
- Function parameter types when called with concrete arguments
- Variable binding types (always inferred)
- Lambda parameter types when context supplies them
- Numeric literal types (resolved by context)

**Annotations that will still be required** (correctly, by design):
- `extern` function signatures (FFI boundary)
- Struct/enum field types when no default is provided
- Ambiguous cases where a function is never called (no constraint source)
- Explicit generic type parameters for complex polymorphism