# P1.7: Typer Subsystem Decomposition — Comprehensive Implementation Plan

## Objective
Decompose the monolithic `Typer` struct (42 fields, 15,464 lines across 11 files) into focused subsystems that own distinct responsibilities, enabling independent evolution, testing, and eventual parallelization.

## Current State

### File Breakdown (11 files, 15,464 total lines)
| File | Lines | Responsibility |
|------|-------|---------------|
| `mod.rs` | 1,840 | Core struct, `lower_program()`, type resolution, exhaustiveness |
| `builtins.rs` | 626 | Builtin function type tables, coercion rules |
| `call.rs` | 2,020 | Function/method/builtin call lowering |
| `expr.rs` | 2,381 | Expression lowering with bidirectional inference |
| `infer.rs` | 157 | AST type inference helpers |
| `lower.rs` | 3,213 | Definition lowering (fns, types, enums, actors, stores, impls) |
| `mono.rs` | 497 | Monomorphization (generic instantiation) |
| `resolve.rs` | 375 | Name pre-declaration (two-pass registration) |
| `scc.rs` | 331 | Strongly-connected component analysis for mutual recursion |
| `stmt.rs` | 981 | Statement lowering |
| `unify.rs` | 1,203 | Type unification engine (union-find, constraint solving) |

### Typer Struct (42 fields in mod.rs)
The struct conflates 5+ distinct concerns:
1. **Name resolution**: `scopes`, `fns`, `structs`, `enums`, `variant_tags`, `actors`, `externs`, `consts`, `globals`, `modules`
2. **Type inference**: `infer_ctx`, `deferred_methods`, `deferred_fields`, `deferred_quantified_vars`, `field_constraints`, `type_errors`
3. **Monomorphization**: `generic_fns`, `generic_enums`, `generic_types`, `mono_fns`, `mono_enums`, `mono_types`, `mono_depth`
4. **Trait system**: `traits`, `trait_impls`, `generic_bounds`, `trait_impl_type_args`, `assoc_types`, `trait_assoc_types`
5. **Definition context**: `methods`, `inferable_fns`, `fn_schemes`, `fn_param_names`, `fn_defaults`, `poly_lambda_asts`, `store_schemas`, `store_decorators`, `view_defs`
6. **Compilation state**: `next_id`, `source_dir`, `test_mode`, `debug_types`, `warnings`, `current_method_type`, `current_fn_ret_ty`, `unannotated_struct_fields`, `inferred_field_structs`

---

## Target Architecture

### Subsystem 1: `NameEnv` — Name Resolution Environment

**Owns**: All name→type/definition mappings that are populated during pre-declaration and consulted during lowering.

```rust
pub struct NameEnv {
    pub(crate) scopes: Vec<HashMap<Symbol, VarInfo>>,
    pub(crate) fns: IndexMap<Symbol, (DefId, Vec<Type>, Type)>,
    pub(crate) externs: IndexMap<Symbol, (DefId, Vec<Type>, Type)>,
    pub(crate) structs: IndexMap<Symbol, Vec<(Symbol, Type)>>,
    pub(crate) enums: IndexMap<Symbol, Vec<(Symbol, Vec<Type>)>>,
    pub(crate) variant_tags: IndexMap<Symbol, (Symbol, u32)>,
    pub(crate) actors: IndexMap<Symbol, (DefId, Vec<(Symbol, Type)>, Vec<(Symbol, Vec<Type>, u32)>)>,
    pub(crate) consts: IndexMap<Symbol, ast::Expr>,
    pub(crate) globals: IndexMap<Symbol, (ast::Expr, ast::Span)>,
    pub(crate) modules: HashSet<Symbol>,
    pub(crate) fn_param_names: IndexMap<Symbol, Vec<Symbol>>,
    pub(crate) fn_defaults: IndexMap<Symbol, Vec<Option<ast::Expr>>>,
}

impl NameEnv {
    pub fn push_scope(&mut self) { ... }
    pub fn pop_scope(&mut self) { ... }
    pub fn define_var(&mut self, name: Symbol, info: VarInfo) { ... }
    pub fn find_var(&self, name: &Symbol) -> Option<&VarInfo> { ... }
    pub fn find_fn(&self, name: &Symbol) -> Option<&(DefId, Vec<Type>, Type)> { ... }
    pub fn find_struct(&self, name: &Symbol) -> Option<&Vec<(Symbol, Type)>> { ... }
    pub fn find_enum(&self, name: &Symbol) -> Option<&Vec<(Symbol, Vec<Type>)>> { ... }
}
```

**File**: `src/typer/name_env.rs` (~300 lines)

**Migration**: Extract all scope/lookup methods from mod.rs. resolve.rs populates NameEnv directly.

---

### Subsystem 2: `InferEngine` — Type Inference & Unification

**Owns**: The constraint-based type inference machinery including unification, deferred resolution, and field constraints.

```rust
pub struct InferEngine {
    pub(crate) ctx: InferCtx,  // existing union-find
    pub(crate) deferred_methods: Vec<DeferredMethod>,
    pub(crate) deferred_fields: Vec<DeferredField>,
    pub(crate) deferred_quantified_vars: Vec<u32>,
    pub(crate) field_constraints: IndexMap<u32, Vec<(Symbol, Type)>>,
    pub(crate) type_errors: Vec<String>,
    pub(crate) unannotated_struct_fields: Vec<(Symbol, Symbol, Type, Span)>,
    pub(crate) inferred_field_structs: HashSet<Symbol>,
}

impl InferEngine {
    pub fn fresh_var(&mut self) -> Type { ... }
    pub fn unify(&mut self, a: &Type, b: &Type) -> Result<(), String> { ... }
    pub fn resolve(&self, ty: &Type) -> Type { ... }
    pub fn add_method_constraint(&mut self, dm: DeferredMethod) { ... }
    pub fn add_field_constraint(&mut self, df: DeferredField) { ... }
    pub fn solve_deferred(&mut self, names: &NameEnv) -> Vec<String> { ... }
}
```

**File**: `src/typer/infer_engine.rs` (~400 lines, wrapping existing unify.rs)

**Relationship**: `unify.rs` remains as the low-level union-find. `InferEngine` is the higher-level API that manages deferred constraints and resolution.

---

### Subsystem 3: `MonoCtx` — Monomorphization Context

**Owns**: Generic definition storage and instantiation tracking.

```rust
pub struct MonoCtx {
    pub(crate) generic_fns: IndexMap<Symbol, ast::Fn>,
    pub(crate) generic_enums: IndexMap<Symbol, ast::EnumDef>,
    pub(crate) generic_types: IndexMap<Symbol, ast::TypeDef>,
    pub(crate) inferable_fns: IndexMap<Symbol, ast::Fn>,
    pub(crate) fn_schemes: IndexMap<Symbol, (Vec<u32>, Vec<Type>, Type)>,
    pub(crate) poly_lambda_asts: IndexMap<Symbol, (Vec<ast::Param>, Option<Type>, ast::Block, Span)>,
    pub(crate) mono_depth: u32,
    pub(crate) mono_fns: Vec<hir::Fn>,
    pub(crate) mono_enums: Vec<hir::EnumDef>,
    pub(crate) mono_types: Vec<hir::TypeDef>,
}

impl MonoCtx {
    pub fn instantiate_fn(&mut self, name: &Symbol, type_args: &[Type],
                          names: &NameEnv, infer: &mut InferEngine) -> Symbol { ... }
    pub fn instantiate_enum(&mut self, name: &Symbol, type_args: &[Type]) -> Symbol { ... }
}
```

**File**: `src/typer/mono_ctx.rs` (~600 lines, absorbing current mono.rs)

---

### Subsystem 4: `TraitResolver` — Trait System

**Owns**: Trait definitions, implementations, bounds checking, associated types.

```rust
pub struct TraitResolver {
    pub(crate) traits: IndexMap<Symbol, Vec<TraitMethodSig>>,
    pub(crate) trait_impls: IndexMap<Symbol, Vec<Symbol>>,
    pub(crate) generic_bounds: IndexMap<Symbol, Vec<(Symbol, Vec<Symbol>)>>,
    pub(crate) trait_impl_type_args: IndexMap<(Symbol, Symbol), Vec<Type>>,
    pub(crate) assoc_types: IndexMap<(Symbol, Symbol), Type>,
    pub(crate) trait_assoc_types: IndexMap<Symbol, Vec<Symbol>>,
}

impl TraitResolver {
    pub fn declare_trait(&mut self, name: Symbol, methods: Vec<TraitMethodSig>) { ... }
    pub fn declare_impl(&mut self, trait_name: Symbol, type_name: Symbol) { ... }
    pub fn check_bounds(&self, ty: &Type, bound: &Symbol) -> bool { ... }
    pub fn resolve_assoc_type(&self, trait_name: &Symbol, type_name: &Symbol) -> Option<&Type> { ... }
    pub fn find_impl_for(&self, trait_name: &Symbol, ty: &Type) -> Option<Symbol> { ... }
}
```

**File**: `src/typer/trait_resolver.rs` (~500 lines)

---

### Subsystem 5: `DefinitionCtx` — Definition Metadata

**Owns**: Method registrations, store schemas, view definitions — metadata about user-defined constructs.

```rust
pub struct DefinitionCtx {
    pub(crate) methods: IndexMap<Symbol, Vec<ast::Fn>>,
    pub(crate) store_schemas: IndexMap<Symbol, Vec<(Symbol, Type)>>,
    pub(crate) store_decorators: IndexMap<Symbol, Vec<ast::StoreDecorator>>,
    pub(crate) view_defs: IndexMap<Symbol, (Symbol, Vec<ast::QueryClause>)>,
}
```

**File**: `src/typer/def_ctx.rs` (~100 lines — mostly data, methods in lower.rs)

---

### Revised Typer Struct

After extraction, the Typer becomes a thin coordinator:

```rust
pub struct Typer {
    pub(crate) next_id: u32,
    pub(crate) names: NameEnv,
    pub(crate) infer: InferEngine,
    pub(crate) mono: MonoCtx,
    pub(crate) traits: TraitResolver,
    pub(crate) defs: DefinitionCtx,
    // Compilation state (not extractable — thread through everything)
    pub(crate) source_dir: Option<PathBuf>,
    pub(crate) test_mode: bool,
    pub(crate) debug_types: bool,
    pub(crate) warnings: Vec<String>,
    pub(crate) current_method_type: Option<Symbol>,
    pub(crate) current_fn_ret_ty: Option<Type>,
}
```

**Result**: 42 flat fields → 5 subsystems + 7 state fields = clean separation of concerns.

---

## Migration Strategy

### Phase 1: Extract `NameEnv` (lowest coupling)
1. Create `src/typer/name_env.rs` with the struct and scope methods
2. Add `pub(crate) names: NameEnv` field to Typer
3. Replace all `self.scopes` → `self.names.scopes`, `self.fns` → `self.names.fns`, etc.
4. Move `push_scope`, `pop_scope`, `define_var`, `find_var` methods to NameEnv impl
5. Update all call sites: `self.push_scope()` → `self.names.push_scope()`
6. Run tests — must pass

### Phase 2: Extract `TraitResolver` (self-contained)
1. Create `src/typer/trait_resolver.rs`
2. Move trait-related fields and methods
3. Update `declare_impl_block` and trait resolution call sites
4. Run tests

### Phase 3: Extract `MonoCtx` (absorbs mono.rs)
1. Rename `mono.rs` → `mono_ctx.rs`, restructure as MonoCtx
2. Move generic definition storage from Typer to MonoCtx
3. `instantiate_fn` and `instantiate_enum` now take `&NameEnv` and `&mut InferEngine` params
4. Run tests

### Phase 4: Extract `InferEngine` (wraps unify.rs)
1. Create `src/typer/infer_engine.rs`
2. Move deferred constraint fields
3. Wrap `InferCtx` methods with higher-level API
4. Run tests

### Phase 5: Extract `DefinitionCtx` (trivial)
1. Create `src/typer/def_ctx.rs`
2. Move method/store/view fields
3. Run tests

### Phase 6: Clean up Typer struct
1. Remove all migrated fields from Typer
2. Add subsystem fields
3. Verify all 11 existing files compile
4. Run full test suite
5. Run benchmarks

---

## Self-referential Access Pattern

The main challenge is that many typer methods need access to multiple subsystems simultaneously. The solution:

### Pattern A: Method on Typer (coordinator)
```rust
impl Typer {
    fn lower_call(&mut self, ...) {
        // Can access self.names, self.infer, self.mono, self.traits freely
        let fn_info = self.names.find_fn(&name);
        let instantiated = self.mono.instantiate_fn(&name, &args, &self.names, &mut self.infer);
    }
}
```

### Pattern B: Free functions with explicit params
```rust
fn resolve_method(names: &NameEnv, traits: &TraitResolver, 
                  receiver_ty: &Type, method: &Symbol) -> Option<(Symbol, Vec<Type>, Type)> {
    // Pure lookup, no mutation
}
```

### Pattern C: Temporary borrows for subsystem methods
```rust
impl MonoCtx {
    pub fn instantiate_fn(&mut self, name: &Symbol, type_args: &[Type],
                          names: &NameEnv, infer: &mut InferEngine) -> Symbol {
        // MonoCtx mutates itself, reads NameEnv, mutates InferEngine
    }
}
```

**Rule**: Methods stay on Typer if they need ≥3 subsystems. Methods move to a subsystem if they primarily operate on that subsystem's data.

---

## File Structure After Migration

```
src/typer/
    mod.rs          (~800 lines)  — Typer struct, lower_program(), type resolution
    name_env.rs     (~300 lines)  — NameEnv: scopes, definitions, lookups
    infer_engine.rs (~400 lines)  — InferEngine: constraints, deferred resolution
    mono_ctx.rs     (~600 lines)  — MonoCtx: generic instantiation (replaces mono.rs)
    trait_resolver.rs (~500 lines) — TraitResolver: traits, impls, bounds
    def_ctx.rs      (~100 lines)  — DefinitionCtx: methods, stores, views
    builtins.rs     (626 lines)   — unchanged
    call.rs         (2,020 lines) — unchanged (uses self.names, self.infer, self.mono)
    expr.rs         (2,381 lines) — unchanged
    infer.rs        (157 lines)   — unchanged
    lower.rs        (3,213 lines) — unchanged
    resolve.rs      (375 lines)   — populates NameEnv + TraitResolver
    scc.rs          (331 lines)   — unchanged
    stmt.rs         (981 lines)   — unchanged
    unify.rs        (1,203 lines) — unchanged (low-level, wrapped by InferEngine)
```

**Total new code**: ~1,900 lines (subsystem structs + methods)
**Total deleted code**: ~1,200 lines (moved from Typer methods)
**Net change**: +700 lines, but complexity is distributed across focused modules

---

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Borrow checker fights with multiple `&mut` subsystems | Use coordinator pattern — Typer methods split borrows: `let (names, infer) = (&mut self.names, &mut self.infer)` |
| Performance regression from indirection | Subsystems are `pub(crate)` fields — no vtable, same memory layout, just namespacing |
| Massive PR size | Phase-by-phase extraction, each phase independently testable |
| Method signature churn | Keep methods on Typer initially, move to subsystems incrementally in later PRs |
| Cross-subsystem queries | Add convenience methods on Typer that delegate: `self.find_fn(name)` → `self.names.find_fn(name)` |

## Validation
- All 1,418 tests must pass after each phase
- Compilation benchmarks before/after (should be ≤1% regression)
- No public API changes (Typer is `pub` but fields are `pub(crate)`)
