# Jade Type Inference — Deep Audit & Remediation Report

**Date:** Session 2 (State-of-the-Art Inference Pass)  
**Baseline:** 908 tests (102 unit + 561 bulk + 245 integration), 0 warnings  
**Final:** 930 tests (114 unit + 571 bulk + 245 integration), 0 warnings  
**Files modified:** 4 (`src/typer/unify.rs`, `src/typer/mod.rs`, `src/typer/resolve.rs`, `tests/bulk_tests.rs`)

---

## Executive Summary

This second-pass audit targeted Jade's type inference engine to bring it to state-of-the-art quality for a compiled systems language. The audit identified **16 critical gaps** in the inference system and resolved all of them. The core issue was that Jade had the architectural foundation for Hindley-Milner-class inference (TypeVar + UnionFind + bidirectional propagation) but the wiring was incomplete — many language constructs silently bypassed unification or fell back to `Type::I64`.

---

## Architecture (Post-Remediation)

```
AST ─→ Typer (mod.rs) ─→ HIR
         │
         ├─ InferCtx (unify.rs): UnionFind with path compression + union-by-rank
         │   ├─ fresh_var() → TypeVar(u32)
         │   ├─ unify(a, b) → structural unification with occurs check
         │   ├─ unify_at(a, b, span, ctx) → unify + record origin for diagnostics
         │   ├─ resolve(ty) → deep substitution (TypeVar → concrete)
         │   └─ shallow_resolve(ty) → one-step substitution
         │
         ├─ Bidirectional propagation: lower_expr_expected(expr, Option<&Type>)
         │   ├─ Lambda params ← expected fn type
         │   ├─ Array elements ← expected array element type
         │   ├─ Return values ← function return type
         │   └─ Bind values ← annotation type
         │
         └─ Resolution (resolve.rs): Pre-declaration + param type inference
             ├─ Multi-call-site unification for untyped params
             └─ Proper unify+resolve (not overwrite)
```

---

## Findings & Fixes

### CRITICAL: Unification Engine

| # | Finding | Severity | Fix |
|---|---------|----------|-----|
| 1 | `unify()` catch-all `_ => Ok(())` silently accepted mismatches between ANY concrete types (e.g., `Bool` unified with `Str`) | **Critical** | Changed to `Err(format!("type mismatch: ..."))` |

### HIGH: Missing Constraint Sites

| # | Finding | Severity | Fix |
|---|---------|----------|-----|
| 2 | `Assign` stmt: no unification between target and value types | High | Added `unify_at(&target.ty, &value.ty, span, "assignment")` + coercion |
| 3 | `Ret` stmt: return value not unified with function return type | High | Added `unify_at(&value.ty, ret_ty, span, "return value")` + bidirectional expected type |
| 4 | Ternary (`? !`): branches not unified | High | Added `unify_at(&then.ty, &else.ty, span, "ternary branches")` |
| 5 | If-expression: else/elif branches not unified with then branch | High | Added `unify_at` for each alternative branch |
| 6 | Array literal: elements not unified with each other | High | Added pairwise `unify_at` across all elements |
| 7 | Channel send: sent value not unified with channel element type | High | Added `unify_at(&elem_ty, &value.ty, span, "channel send")` |
| 8 | Select arm send: sent value not unified with channel type | High | Added `unify_at(&elem_ty, &value.ty, span, "select send")` |
| 9 | Indirect call args: arguments not unified with parameter types | High | Added `unify_at(param_ty, &arg.ty, span, "indirect call argument")` |

### MEDIUM: I64 Fallback Elimination

| # | Finding | Severity | Fix |
|---|---------|----------|-----|
| 10 | Lambda params without expected type defaulted to `Type::I64` | Medium | Changed to `self.infer_ctx.fresh_var()` |
| 11 | `vec()` constructor element type defaulted to `I64` on empty | Medium | Changed to `fresh_var()` + element unification |
| 12 | Map constructor value type defaulted to `I64` | Medium | Changed to `fresh_var()` |
| 13 | Index expression fallback on unknown container was `I64` | Medium | Changed to `fresh_var()` |

### MEDIUM: Resolution Pass Bugs

| # | Finding | Severity | Fix |
|---|---------|----------|-----|
| 14 | `infer_param_types` double-write: overwrote unified TypeVar with raw type, orphaning the unified result | Medium | Changed `entry.1[slot] = ty` to `entry.1[slot] = self.infer_ctx.resolve(&entry.1[slot])` (2 sites) |

### MEDIUM: Bidirectional Propagation Gaps

| # | Finding | Severity | Fix |
|---|---------|----------|-----|
| 15 | Array literals: no expected element type propagation | Medium | Extract element type from `Array(et, _)` and pass to each element via `lower_expr_expected` |
| 16 | Return statements: no expected type propagation | Medium | Pass function return type as expected to `lower_expr_expected` |

### ANALYZED — No Change Needed

| # | Area | Rationale |
|---|------|-----------|
| A | `mono.rs` I64 defaults | Monomorphization requires concrete types for LLVM codegen; defaults are correct at this stage |
| B | Bind annotation checking | Parser doesn't currently emit `ty: Some(...)` on `Bind` AST nodes; wiring in typer deferred to when parser supports it |
| C | `diagnostic.rs` | Structured diagnostics work correctly with `unify_at` origin tracking |

---

## Test Coverage Added

### Unit Tests (12 new in `unify.rs`)
- `test_concrete_mismatch_errors` — verifies `I64 ≠ Bool` produces error
- `test_concrete_same_ok` — verifies `I64 = I64` succeeds
- `test_structural_vec_mismatch` — verifies `Vec<I64> ≠ Vec<Bool>`
- `test_tuple_arity_mismatch` — verifies `(I64, I64) ≠ (I64,)`
- `test_tuple_unify_with_vars` — verifies `(TypeVar, I64)` unifies with `(Bool, I64)`
- `test_map_unify` — verifies `Map<TypeVar, I64>` unifies with `Map<Str, I64>`
- `test_channel_unify` — verifies `Channel<TypeVar>` unifies with `Channel<I64>`
- `test_fn_arity_mismatch` — verifies fn with 2 params ≠ fn with 1 param
- `test_array_length_mismatch` — verifies `[I64; 3] ≠ [I64; 5]`
- `test_deeply_nested_unification` — verifies `Vec<Map<TypeVar, I64>>` resolves correctly
- `test_unify_at_records_origin` — verifies span/context tracking
- `test_try_resolve_unsolved` — verifies unsolved TypeVars return `None`

### Integration Tests (10 new in `bulk_tests.rs`)
- `b_infer_assign_propagates` — assignment unification
- `b_infer_ternary_branches` — ternary type unification
- `b_infer_array_element_unification` — array element consistency
- `b_infer_return_type` — return type inference from body
- `b_infer_lambda_from_context` — lambda param types from call site
- `b_infer_bind_simple` — bind infers type from value
- `b_infer_vec_element_type` — vec element type unification
- `b_infer_nested_lambda` — nested lambda type propagation
- `b_infer_struct_field_from_literal` — struct field types from literal
- `b_infer_if_expr_type` — ternary expression type

---

## Comparison with State-of-the-Art

| Feature | Rust | Swift | Kotlin | **Jade (now)** |
|---------|------|-------|--------|----------------|
| TypeVar + UnionFind | ✓ | ✓ | ✓ | **✓** |
| Concrete mismatch errors | ✓ | ✓ | ✓ | **✓** (was broken) |
| Bidirectional propagation | ✓ | ✓ | ✓ | **✓** (lambdas, arrays, returns) |
| Returns unified with signature | ✓ | ✓ | ✓ | **✓** (was missing) |
| Branch unification (if/match) | ✓ | ✓ | ✓ | **✓** (was missing) |
| Assignment constraints | ✓ | ✓ | ✓ | **✓** (was missing) |
| Occurs check | ✓ | ✓ | ✓ | **✓** |
| Multi-call-site param inference | partial | ✗ | ✗ | **✓** |
| Numeric type lattice | ✓ | ✓ | ✓ | ✗ (future) |
| Flow-sensitive narrowing | partial | ✓ | ✓ | ✗ (future) |
| Trait-based inference | ✓ | ✓ | ✓ | partial |
| Error recovery + continuity | ✓ | ✓ | ✓ | ✗ (future) |

---

## Remaining Opportunities (Future Work)

1. **Numeric type lattice** — Infer `i32` vs `i64` vs `f64` from context rather than defaulting all to `i64`
2. **Flow-sensitive narrowing** — After `if x is Foo`, narrow `x` to `Foo` in the then branch
3. **Bind type annotation parsing** — Parser needs to support `x: i64 is 42` syntax
4. **Error recovery** — Continue inference after first error to report multiple diagnostics
5. **Trait-based inference** — Infer types from trait method signatures at call sites
6. **Closure syntax** — Add `|x| expr` lightweight closure syntax (currently only `*fn(x: T) -> T expr`)

---

## Final State

- **930 tests**: 114 unit + 571 bulk + 245 integration — **all passing**
- **0 compiler warnings**
- **0 regressions** from baseline
- **22 new tests** validating inference improvements
