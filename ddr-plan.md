# Demand-Driven Resolution (DDR) — Lazy Constraint Graph Plan

## Goal
Replace the current batch-resolve phases (resolve_deferred_methods, resolve_deferred_fields,
resolve_trait_constrained_vars) with a demand-driven architecture where type queries trigger
resolution lazily. This enables incremental compilation, better error locality, and
future separate compilation with inference.

## Current Architecture

The typer pipeline in `lower.rs` runs 12 sequential phases:

```
1. declare types/traits/impls
2. declare fn signatures (TypeVars for unannotated params)
3. build SCC ordering
4. lower function bodies (produces deferred method/field entries)
5. resolve_deferred_methods()      ← batch
6. resolve_deferred_fields()       ← batch
7. resolve_trait_constrained_vars() ← batch
8. strict struct field check
9. resolve_all_types()             ← tree walk
10. mono pass
11. collect diagnostics
12. build final HIR
```

Steps 5–7 are batch passes that iterate over all deferred entries. Step 9 does a full
tree walk replacing all TypeVars with concrete types. This means the full program must
be lowered before any resolution happens, preventing incremental/demand-driven flow.

## Proposed Architecture

### Phase 1: Constraint Graph (Minor — est. 300 lines changed)

Replace the lists (`deferred_methods`, `deferred_fields`, `deferred_trait_vars`) with a
single `ConstraintGraph` that records pending constraints per TypeVar root:

```rust
struct PendingConstraint {
    kind: ConstraintKind,
    span: Span,
    expr_id: ExprId,
}

enum ConstraintKind {
    MethodCall { method: String, args: Vec<Type>, ret: Type },
    FieldAccess { field: String, result: Type },
    TraitBound { traits: Vec<String> },
}

struct ConstraintGraph {
    pending: HashMap<u32, Vec<PendingConstraint>>,  // TypeVar root → constraints
}
```

When a deferred method/field/trait is encountered during lowering, instead of pushing
to a list, register it in the constraint graph keyed by the unsolved TypeVar root.

### Phase 2: Demand Resolution Hooks (Minor — est. 200 lines changed)

Add a `try_resolve` callback in `InferCtx::unify`. When a TypeVar gets solved (bound
to a concrete type), check if that root has pending constraints in the graph and
resolve them immediately:

```rust
// In InferCtx::unify, after successfully binding a TypeVar:
fn on_var_solved(&mut self, root: u32, concrete_ty: &Type) {
    if let Some(pending) = self.constraint_graph.remove(&root) {
        for constraint in pending {
            self.resolve_constraint(constraint, concrete_ty);
        }
    }
}
```

This means methods resolve as soon as their receiver type becomes known, rather than
waiting for a batch pass. Benefits:
- Earlier error detection (error at the expression that caused the type to be known)
- No need for a separate resolve pass
- Incremental: only touched constraints get resolved

### Phase 3: Lazy Type Resolution (Moderate — est. 400 lines changed)

Replace the `resolve_all_types()` tree walk with lazy resolution. Instead of walking
the entire HIR tree to replace TypeVars, resolve them on-demand when:
- Codegen reads a type
- A diagnostic needs a concrete type
- An export boundary requires a fixed signature

This requires a `Resolved<Type>` wrapper or similar pattern where codegen calls
`infer_ctx.resolve(&ty)` at the point of use rather than pre-resolving everything.

### Phase 4: Cross-Module Signature Export (Moderate — est. 300 lines)

Currently Jade inlines all module declarations into one compilation unit. For true
separate compilation:

1. After lowering a module, export resolved function signatures as a `.jadei` interface file
2. When importing a module, read the `.jadei` file instead of re-parsing/lowering
3. Signatures contain only concrete types (all TypeVars resolved at export boundary)

Format (simple, human-readable):
```
# std/fmt.jadei — auto-generated
fn pad_left(String, i64, String) -> String
fn pad_right(String, i64, String) -> String
fn join(Vec<String>, String) -> String
fn repeat(String, i64) -> String
```

This enables O(n) compilation instead of O(n²) for n modules while preserving inference
within each module.

## Migration Strategy

Each phase is backward-compatible and independently testable:

1. **Phase 1** can coexist with the batch lists — build the graph in parallel, verify
   it produces identical results, then remove the lists.
2. **Phase 2** can be feature-gated — run demand resolution AND batch resolution,
   assert identical results, then remove batch.
3. **Phase 3** is a codegen refactor — change call sites to resolve lazily, verify
   outputs unchanged.
4. **Phase 4** is additive — new feature, no existing behavior changes.

## Complexity Assessment

| Phase | Lines Changed | Risk | Dependencies |
|-------|--------------|------|-------------|
| 1     | ~300         | Low  | None        |
| 2     | ~200         | Low  | Phase 1     |
| 3     | ~400         | Med  | Phase 2     |
| 4     | ~300         | Med  | Phase 3     |

Total: ~1,200 lines of changes, deliverable in 4 incremental PRs.

## Files Affected

- `src/typer/unify.rs` — Add constraint graph, on_var_solved hook
- `src/typer/lower.rs` — Register constraints instead of deferred lists, remove batch passes
- `src/typer/expr.rs` — Use constraint graph for method/field deferral
- `src/typer/resolve.rs` — Signature export for Phase 4
- `src/codegen/mod.rs` — Lazy resolution for Phase 3
- `src/main.rs` — Interface file loading for Phase 4

## Non-Goals

- Full dependent types or refinement types (out of scope)
- Parallel compilation (can be built on DDR later, but not part of this plan)
- Changing the surface syntax or type annotation format
