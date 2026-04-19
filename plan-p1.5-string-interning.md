# P1.5: String Interning Migration — Comprehensive Implementation Plan

## Objective
Replace all heap-allocated `String` identifiers throughout the compiler pipeline (Token, AST, HIR, Typer, Codegen) with `Symbol` handles from `src/intern.rs`. This eliminates millions of per-identifier allocations, reduces memory usage, and makes identifier comparison O(1) instead of O(n).

## Current State
- `src/intern.rs` exists with `Symbol` type backed by thread-local `lasso::Rodeo`
- `lasso = "0.7"` dependency in Cargo.toml
- `pub mod intern` declared in `lib.rs`
- **Not wired** into any pipeline stage

## Architecture

### Symbol Type (already implemented)
```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(Spur);  // 4 bytes, Copy, O(1) equality
```

### Thread Safety Consideration
Current design uses `thread_local! { RefCell<Rodeo> }`. This is single-threaded only. For future parallelism (parallel typer, parallel codegen), consider migrating to `lasso::ThreadedRodeo` with `Arc`. However, the current design is correct for the existing single-threaded pipeline and should be kept for now.

---

## Phase 1: Lexer — Token::Ident(String) → Token::Ident(Symbol)

### Changes Required

**`src/lexer.rs`:**
1. Add `use crate::intern::Symbol;`
2. Change `Token::Ident(String)` → `Token::Ident(Symbol)`
3. Change `Token::Str(String)` — **do NOT intern string literals** (they are values, not identifiers)
4. Update `lex_ident()` (around L1019): `Token::Ident(Symbol::intern(text))` instead of `Token::Ident(text.to_string())`
5. Update all test assertions that match `Token::Ident("x".into())` → `Token::Ident(Symbol::intern("x"))`

### Ripple Effects
- `keyword()` function: currently returns `Option<Token>` — no change needed (keywords are not Ident)
- `Token::PartialEq` derive works because `Symbol` derives `PartialEq`
- `Token::Debug` derive works because `Symbol` implements `Debug`

### Estimated scope: ~20 lines changed in lexer.rs, ~10 test lines

---

## Phase 2: Parser — AST String fields → Symbol

### AST Changes (`src/ast.rs`)

**Identifier fields to convert** (all become `Symbol`):
| Struct/Enum | Field | Current Type |
|-------------|-------|--------------|
| `Param` | `name` | `String` |
| `Fn` | `name` | `String` |
| `TypeDef` | `name` | `String` |
| `EnumDef` | `name` | `String` |
| `ActorDef` | `name` | `String` |
| `StoreDef` | `name` | `String` |
| `TraitDef` | `name` | `String` |
| `ImplBlock` | `trait_name`, `type_name` | `String` |
| `Decl::Use` | module path | `String` |
| `Expr::Ident` | name | `String` |
| `Expr::FieldAccess` | field | `String` |
| `Expr::MethodCall` | method | `String` |
| `Expr::StructLit` | name, field names | `String` |
| `Expr::EnumVariant` | enum_name, variant | `String` |
| `Stmt::Bind` | name | `String` |
| `Pat::Ident` | name | `String` |
| `Pat::Enum` | variant | `String` |

**Fields to NOT convert** (runtime values, not identifiers):
- `Expr::StringLit` — user-facing string value
- `Expr::Asm` — inline assembly text
- `Expr::Embed` — embedded code text
- Error messages, diagnostic strings

### Parser Changes (`src/parser/mod.rs`, `src/parser/expr.rs`)
- All `expect_ident()` calls return `Symbol` instead of `String`
- `parse_ident()` → returns `Symbol`
- All pattern matching on `Token::Ident(name)` → name is now `Symbol`
- String concatenation for qualified names: `format!("{}.{}", module, name)` → `Symbol::intern(&format!("{}.{}", module, name))`

### Key consideration: Qualified names
Module-prefixed names like `"math.sin"` are constructed via `format!()`. These must be interned at construction time: `Symbol::intern(&format!("{}.{}", mod_name, fn_name))`. The allocation for the format string is temporary and freed immediately — the Symbol stores only the interned handle.

### Estimated scope: ~200 lines in ast.rs, ~150 lines in parser/

---

## Phase 3: Typer — HashMap<String, _> → HashMap<Symbol, _>

### Typer Struct Fields (`src/typer/mod.rs`)
All 28+ IndexMap fields keyed by `String` become keyed by `Symbol`:
```rust
// Before:
pub(crate) fns: IndexMap<String, (DefId, Vec<Type>, Type)>,
// After:
pub(crate) fns: IndexMap<Symbol, (DefId, Vec<Type>, Type)>,
```

### Fields to convert (all IndexMap<String, _> keys):
`fns`, `structs`, `enums`, `variant_tags`, `generic_fns`, `generic_enums`, `generic_types`, `methods`, `actors`, `store_schemas`, `store_decorators`, `view_defs`, `traits`, `trait_impls`, `generic_bounds`, `consts`, `globals`, `inferable_fns`, `fn_schemes`, `fn_param_names`, `fn_defaults`, `externs`

### Fields with compound keys:
- `trait_impl_type_args: IndexMap<(String, String), Vec<Type>>` → `IndexMap<(Symbol, Symbol), Vec<Type>>`
- `assoc_types: IndexMap<(String, String), Type>` → `IndexMap<(Symbol, Symbol), Type>`

### Scope stack:
- `scopes: Vec<HashMap<String, VarInfo>>` → `Vec<HashMap<Symbol, VarInfo>>`

### VarInfo, DeferredMethod, DeferredField — string fields → Symbol

### All submodules:
- `resolve.rs`: all `.insert()`, `.get()`, `.contains_key()` calls use Symbol keys
- `mono.rs`: monomorphization name construction uses `Symbol::intern()`
- `unify.rs`: `InferCtx` trait_impls key type
- `expr.rs`, `stmt.rs`, `call.rs`, `lower.rs`, `builtins.rs`, `scc.rs`, `infer.rs`

### Estimated scope: ~500 lines across typer/

---

## Phase 4: HIR — String fields → Symbol

### HIR Changes (`src/hir.rs`)
| Struct | Field | Current |
|--------|-------|---------|
| `Fn` | `name` | `String` |
| `TypeDef` | `name` | `String` |
| `EnumDef` | `name` | `String` |
| `ActorDef` | `name` | `String` |
| `StoreDef` | `name` | `String` |
| `Stmt::Bind` | name | `String` |
| `Stmt::Drop` | name | `String` |
| `Expr::Var` | name | `String` |
| `Expr::Call` | fn_name | `String` |
| `Expr::MethodCall` | method | `String` |
| `Expr::FieldAccess` | field | `String` |
| `Expr::StructLit` | name, fields | `String` |
| `Expr::EnumVariant` | name, variant | `String` |

### Estimated scope: ~100 lines in hir.rs

---

## Phase 5: Codegen — IndexMap<String, _> → IndexMap<Symbol, _>

### Compiler struct (`src/codegen/mod.rs`)
All 13 IndexMap fields with String keys:
`fns`, `structs`, `struct_defaults`, `struct_layouts`, `enums`, `variant_tags`, `store_defs`, `actor_defs`, `vtables`, `trait_method_order`, `globals`, `reuse_tokens`, `vars` (inner maps)

### Variable scoping:
```rust
// Before:
vars: Vec<IndexMap<String, (PointerValue<'ctx>, Type)>>
// After:
vars: Vec<IndexMap<Symbol, (PointerValue<'ctx>, Type)>>
```

### All codegen submodules:
- `expr.rs`, `stmt.rs`, `decl.rs`, `call.rs`, `builtins.rs`, `strings.rs`, `stores.rs`, `collections.rs`, `actors.rs`, `lambda.rs`, `channels.rs`, `pattern_match.rs`
- Every `.get("name")` becomes `.get(&Symbol::intern("name"))` or better, uses a pre-interned constant

### LLVM name strings:
Many places pass `&name` to LLVM for function/variable naming. These need `.as_str()` or `.to_string()` at the LLVM boundary. The allocation here is unavoidable — LLVM needs C strings — but it happens only at emission time, not during lookups.

### Estimated scope: ~400 lines across codegen/

---

## Phase 6: MIR Pipeline

### MIR types (`src/mir/mod.rs`)
- Function names in MIR are `String` — convert to `Symbol`
- Variable names in MIR — convert to `Symbol`
- `src/mir/lower.rs`, `src/mir/opt.rs` — update accordingly

### MIR Codegen (`src/codegen/mir_codegen.rs`)
- `var_allocs: HashMap<String, _>` → `HashMap<Symbol, _>`
- `coro_bodies: HashMap<String, _>` → `HashMap<Symbol, _>`
- `actor_defs: HashMap<String, _>` → `HashMap<Symbol, _>`

### Estimated scope: ~200 lines across mir/

---

## Phase 7: Peripheral Systems

### Perceus (`src/perceus.rs`)
- UseInfo keyed by variable name → Symbol
- Drop insertion uses name strings → Symbol

### Ownership (`src/ownership.rs`)
- Variable tracking by name → Symbol

### HIR Validate (`src/hir_validate.rs`)
- DefId name lookups → Symbol

### Diagnostics (`src/diagnostic.rs`)
- Error messages `.to_string()` at the boundary — no Symbol changes needed

### Formatter (`src/fmt.rs`)
- Reads tokens, outputs text — Token::Ident(Symbol) → `.as_str()` at output

### Estimated scope: ~100 lines across peripheral files

---

## Migration Strategy

### Approach: Bottom-up, one phase per PR
1. **Phase 1** (Lexer): Smallest blast radius, validates the Symbol API
2. **Phase 2** (Parser/AST): Core data structures, large but mechanical
3. **Phase 3** (Typer): Largest change, most map key conversions
4. **Phase 4** (HIR): Bridges typer → codegen
5. **Phase 5** (Codegen): LLVM boundary requires `.as_str()` calls
6. **Phase 6** (MIR): Parallel pipeline
7. **Phase 7** (Peripheral): Cleanup pass

### Each phase must:
1. Compile with zero errors
2. Pass all 1,418 tests
3. Not change any observable behavior

### Performance validation:
- After Phase 7, run `python3 run_benchmarks.py --runs=5 --quiet`
- Expected: Compilation speed improvement (fewer allocations)
- Runtime performance should be unchanged (Symbol→str at LLVM boundary)

---

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Thread-local interner prevents future parallelism | Migrate to `ThreadedRodeo` when needed (API-compatible) |
| `.as_str()` returns `String` (allocates) | Add `with_str()` callback API: `sym.with_str(\|s\| ...)` to avoid allocation |
| Qualified name construction allocates for `format!()` | Unavoidable but temporary — interned result is cheap |
| `IndexMap<Symbol, _>` ordering | Symbol is `Ord` (by Spur index = insertion order) — deterministic |
| Serialization (cache.rs, interface files) | Implement `Serialize`/`Deserialize` for Symbol via `.as_str()` |

## Total Estimated Scope
~1,500 lines changed across ~30 files. Mechanical but high-volume. Each phase is independently testable.
