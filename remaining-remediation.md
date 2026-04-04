# MIR Codegen — Remaining Remediation Items

Cross-referenced from three audit reports. Items already fixed are excluded.
Grouped by severity, then by file.

---

## CRITICAL

### 1. `val()` silently returns `i64(0)` for missing values
- **File**: `src/codegen/mir_codegen.rs` (~L1138)
- **Issue**: If a `ValueId` is not in `value_map`, `val()` prints a warning and returns `i64(0)`. This masks compiler bugs and causes silent miscompilation.
- **Fix**: Panic with a diagnostic message — a missing value is always a compiler bug.
- **Source**: Audit 1 #5, Audit 3 #19 (value_type fallback is related)

### 2. `value_type()` fallback to `Type::I64`
- **File**: `src/mir/lower.rs` (~L178)
- **Issue**: Returns `Type::I64` when a `ValueId` type can't be found. Silently produces wrong types.
- **Fix**: Panic or return `Option<Type>` with diagnostic.
- **Source**: Audit 1 #4, Audit 3 #19

### 3. `FieldSet` on non-pointer structs is a silent no-op
- **File**: `src/codegen/mir_codegen.rs` (~L666-695)
- **Issue**: `FieldSet` only handles `obj_val.is_pointer_value()`. For SSA struct values (the common case for local variables), it falls through and returns `void_val()`. Mutations like `pt.x = 5` are silently dropped.
- **Fix**: Use `build_insert_value` for struct values, or track the alloca origin of loaded values so FieldSet can GEP into the alloca.
- **Source**: Audit 1 #8, Audit 2 #2

### 4. `Load` returns `void_val()` for unknown variables
- **File**: `src/codegen/mir_codegen.rs` (~L536)
- **Issue**: Falls through to `Ok(void_val())` if the variable isn't in `var_allocs` or `find_var`. Masks bugs where variables are used before definition.
- **Fix**: Return an error or panic — a missing variable is always a compiler bug at this stage.
- **Source**: Audit 1 #9

### 5. `FieldGet` on unknown struct types falls back to index 0
- **File**: `src/codegen/mir_codegen.rs` (~L1554)
- **Issue**: When the struct type name isn't found, `emit_field_get` uses `build_extract_value(sv, 0, field)` regardless of which field was requested. Produces wrong values for any field other than the first.
- **Fix**: Error, or look up the field index from MIR type metadata.
- **Source**: Audit 1 #10

### 6. `ListComp` collection iteration binds loop index, not element
- **File**: `src/mir/lower.rs` (~L546)
- **Issue**: `self.var_map.insert(bind.clone(), idx)` — for `[x*2 for x in vec]`, `x` is bound to the index (0, 1, 2...) instead of the actual element (`vec[0]`, `vec[1]`, ...).
- **Fix**: Emit `let elem = Index(iter_val, idx)` and bind the element instead of the index.
- **Source**: Audit 2 #5, Audit 3 (ListComp deforestation)

### 7. Lambda capture variable collection incomplete
- **File**: `src/mir/lower.rs` (~L1718+)
- **Issue**: `collect_expr_var_refs_expr` doesn't traverse all expression types. Missing: `ExprKind::Select`, `ExprKind::MethodCall`, `ExprKind::Field`, `ExprKind::IfExpr` and others. Captures inside untouched expressions are silently dropped.
- **Fix**: Complete the visitor to handle all block-containing and reference-containing expression kinds.
- **Source**: Audit 1 #5 (first audit), Audit 3 (lambda capture)

### 8. `collect_assigned_vars_in_expr` only handles `Select`
- **File**: `src/mir/lower.rs` (~L1674-1684)
- **Issue**: Other block-containing expressions (`IfExpr`, `Block`, `Lambda`) are not traversed when collecting assigned variables. Variables assigned inside these sub-expressions in a loop may not be demoted to memory, causing SSA violations.
- **Fix**: Add cases for `IfExpr`, `Block`, `Lambda`, and other block-containing expression kinds.
- **Source**: Audit 1 #20, Audit 3

---

## HIGH

### 9. `compute_enum_payload_offset` uses hardcoded `idx * 8`
- **File**: `src/codegen/mir_codegen.rs` (~L1913-1923)
- **Issue**: Reads payload fields at `target_idx * 8` byte offsets. `VariantInit` writes them using actual type sizes with alignment. For fields smaller or larger than 8 bytes, read/write offsets mismatch.
- **Fix**: Use the same offset calculation as `VariantInit` — accumulate offsets based on actual field type sizes.
- **Source**: Audit 1 #11, Audit 2 #9

### 10. `IndexSet` on array values stores to local alloca — result lost
- **File**: `src/codegen/mir_codegen.rs` (~L738-755)
- **Issue**: For array-typed values (not pointers), `IndexSet` stores the base into a fresh alloca, GEPs into it, stores the new value, but never loads the modified array back. The mutation is lost.
- **Fix**: Load the modified array from the alloca and update the value map, or use the variable's existing alloca.
- **Source**: Audit 1 #13, Audit 3

### 11. `VariantInit` payload storage has no bounds validation
- **File**: `src/codegen/mir_codegen.rs` (~L593-620)
- **Issue**: Payload bytes are stored at computed offsets with no check that offsets don't overflow the payload area. Could cause buffer overflow for nested types or alignment issues.
- **Fix**: Calculate full payload size first, validate against allocated storage.
- **Source**: Audit 1 #25, Audit 3

### 12. `StoreDelete` lowering discards filter
- **File**: `src/mir/lower.rs` (~L1497)
- **Issue**: `_filter` parameter is ignored. Runtime call receives zero arguments. Delete operations can't know which records to remove.
- **Fix**: Lower the filter expression and pass it to the runtime call.
- **Source**: Audit 1 #30

### 13. `ChanCreate` ignores user-specified capacity
- **File**: `src/mir/lower.rs` (~L583) + `src/codegen/mir_codegen.rs`
- **Issue**: Capacity expression is evaluated then discarded (`let _c = ...`). `ChanCreate(Type)` doesn't carry capacity. Codegen hardcodes 64.
- **Fix**: Add capacity `ValueId` to `ChanCreate` instruction, thread through lowering and codegen.
- **Source**: Audit 1 #12 (first audit), Audit 2 #12

### 14. `SpawnActor` ignores constructor arguments
- **File**: `src/codegen/mir_codegen.rs` (~L990-992)
- **Issue**: `let _ = args;` — any constructor arguments passed to `SpawnActor` are silently discarded.
- **Fix**: Forward args to the actor initialization function.
- **Source**: Audit 1 #24, Audit 3

### 15. For-loop bodies don't check `current_block_has_terminator()` before `Goto`
- **File**: `src/mir/lower.rs` (range-for ~L1114, collection-for ~L1175)
- **Issue**: Unconditional `set_terminator(Goto(inc_bb))` after body. If the body ends with a `return` or `break`, this overwrites the terminator. `While` and `Loop` correctly check first.
- **Fix**: Add `if !self.current_block_has_terminator() { ... }` guard.
- **Source**: Audit 2 #17

---

## MODERATE

### 16. `emit_store_all` returns null pointer
- **File**: `src/codegen/mir_codegen.rs` (~L2697-2702)
- **Issue**: Returns `ptr_type.const_null()`. Any program using `store.all()` gets a null pointer.
- **Fix**: Implement store file reading and Vec construction, or emit a compile-time error if stores are used.
- **Source**: Audit 1 #1, Audit 2 #13

### 17. `emit_store_delete` is a stub (returns 0)
- **File**: `src/codegen/mir_codegen.rs` (~L2706-2711)
- **Issue**: Returns `i8.const_int(0)` without performing any deletion.
- **Fix**: Implement file I/O to filter records.
- **Source**: Audit 1 #2, Audit 2 #14

### 18. `emit_store_set` is a stub (returns 0)
- **File**: `src/codegen/mir_codegen.rs` (~L2714-2722)
- **Issue**: Returns `i8.const_int(0)` without modifying any record.
- **Fix**: Implement file I/O to update records.
- **Source**: Audit 1 #3, Audit 2 #14

### 19. `StrictCast` treated identically to `Cast`
- **File**: `src/mir/lower.rs` (~L394-397)
- **Issue**: `ExprKind::Cast` and `ExprKind::StrictCast` share the same match arm. No overflow check is emitted for strict casts.
- **Fix**: Add separate `StrictCast` MIR instruction with overflow checking, or emit `Cmp + Assert` after the cast.
- **Source**: Audit 2 #11

### 20. `Asm` statement lowered as no-op call
- **File**: `src/mir/lower.rs` (~L1487-1489)
- **Issue**: `_asm` content discarded. Lowered as bare `Call("__asm", vec![])` with no assembly content.
- **Fix**: Pass the assembly template and operands through to codegen, or use LLVM inline asm.
- **Source**: Audit 1 #31

### 21. `Transaction` body lowered without transactional semantics
- **File**: `src/mir/lower.rs` (~L1506-1508)
- **Issue**: Just `lower_block_stmts(body)` — no transaction begin/commit/rollback boundaries.
- **Fix**: Emit transaction boundary calls to the runtime.
- **Source**: Audit 1 #33

### 22. `MapInit` / `SetInit` return null when runtime not declared
- **File**: `src/codegen/mir_codegen.rs`
- **Issue**: When `jade_map_new` or `jade_set_new` functions aren't declared, codegen returns null pointer. Subsequent operations segfault.
- **Fix**: Ensure runtime is always declared when these instructions are present, or emit compile error.
- **Source**: Audit 1 #14

### 23. `VecLen` / `VecNew` return dummy values when runtime not declared
- **File**: `src/codegen/mir_codegen.rs`
- **Issue**: `VecLen` returns constant 0; `VecNew` returns null pointer when runtime functions unavailable.
- **Fix**: Error at compile time if vec operations are used without runtime.
- **Source**: Audit 1 #15, #16

### 24. `PQNew` and `DequeNew` both emit `SetInit`
- **File**: `src/mir/lower.rs`
- **Issue**: `PQNew` and `DequeNew` lower to `InstKind::SetInit`, which calls `jade_set_new`. They should use distinct runtime init functions.
- **Fix**: Add distinct MIR instructions or map to correct runtime functions.
- **Source**: Audit 1 #26

### 25. `Deref` on non-pointer values will panic
- **File**: `src/codegen/mir_codegen.rs`
- **Issue**: `Deref` calls `v.into_pointer_value()` unconditionally. If the MIR value isn't a pointer, this panics.
- **Fix**: Check `v.is_pointer_value()` first and return an error if not.
- **Source**: Audit 1 #24

### 26. LICM uses layout-order back-edges instead of dominance analysis
- **File**: `src/mir/opt.rs` (~L793-810)
- **Issue**: Loop detection assumes blocks are topologically ordered (`succ_idx <= i`). After merging/DCE this isn't guaranteed. False loop headers possible.
- **Fix**: Use proper dominance analysis or at minimum validate topological order.
- **Source**: Audit 1 #19 (strength reduction entry block), Audit 3 (LICM validation)

### 27. `CoroutineCreate` / `GeneratorCreate` body inlined into caller
- **File**: `src/mir/lower.rs` (~L800, ~L824)
- **Issue**: `self.lower_block_stmts(body)` inlines the coroutine/generator body into the caller as dead code. The body is separately extracted for actual codegen, so the inline residue is wasted.
- **Fix**: Skip `self.lower_block_stmts(body)` — only emit the `Call("__coro_create_...")` instruction.
- **Source**: Audit 1 #18 (CoroutineCreate), Audit 2 #23 (GeneratorCreate), Audit 3

### 28. `IfExpr` doesn't demote variables assigned in branches
- **File**: `src/mir/lower.rs` (~L351-380)
- **Issue**: `IfExpr` creates phi nodes but doesn't demote variables assigned inside branches. Variables assigned in branches aren't memory-backed, risking SSA violations.
- **Fix**: Apply same demotion logic as `Stmt::If`.
- **Source**: Audit 3

---

## LOW

### 29. GVN `Cmp` key doesn't normalize commutative comparisons
- **File**: `src/mir/opt.rs` (~L694)
- **Issue**: `Cmp(Eq, v1, v2)` and `Cmp(Eq, v2, v1)` produce different GVN keys. Missed optimization for `Eq`/`Ne`.
- **Fix**: Normalize by swapping operands if `l.0 > r.0` for `Eq` and `Ne`.
- **Source**: Audit 2 #16

### 30. Strength reduction `x * 0` doesn't check types
- **File**: `src/mir/opt.rs` (~L496)
- **Issue**: Always produces `IntConst(0)` regardless of instruction type. For i32/i8, the constant width disagrees.
- **Fix**: Use the instruction's type to create the correct-width zero constant.
- **Source**: Audit 1 #25, Audit 2 #25 (effectively safe because float 0 can't match `iconsts`, but type mismatch for small int types)

### 31. Unused `switch` binding in `emit_terminator`
- **File**: `src/codegen/mir_codegen.rs` (~L1124)
- **Issue**: `let switch = b!(...)` — result unused. Compiler warning.
- **Fix**: Change to `let _switch` or `let _ =`.
- **Source**: Audit 1 #23, Audit 3 #22

### 32. `Log` instruction `inst.ty` carries argument type, not result type
- **File**: `src/mir/lower.rs`
- **Issue**: `Log` is emitted with `arg_ty` as the instruction type but the result is void. Violates convention that `inst.ty = result type`.
- **Fix**: Use a separate field for the argument type, or document the exception.
- **Source**: Audit 1 #27

### 33. `SimFor` / `SimBlock` lowered as sequential execution
- **File**: `src/mir/lower.rs` (~L1511-1606)
- **Issue**: Parallel loops lowered identically to sequential. Parallelism semantics completely lost.
- **Fix**: Emit parallel execution primitives or mark the loop for later parallelization.
- **Source**: Audit 1 #32
- **Note**: Currently by-design — accepted limitation.

### 34. Redundant `constant_branch_elimination` pass
- **File**: `src/mir/opt.rs`
- **Issue**: Duplicates logic in `constant_fold` which also folds Branch on constant booleans.
- **Fix**: Consolidate (harmless but redundant work).
- **Source**: Audit 1 #34

### 35. `Ref` marked pure in `is_pure`
- **File**: `src/mir/opt.rs`
- **Issue**: `Ref` allocates an alloca and stores a value. While currently safe (GVN won't merge due to `gvn_key` returning `None`), it's conceptually wrong for DCE.
- **Fix**: Remove `Ref` and `Deref` from `is_pure` to be conservative.
- **Source**: Audit 2 #15

---

## ALREADY REMEDIATED (excluded from above)

The following findings from the audits have been verified as fixed:

1. **Cmp instruction carries operand type** — `Cmp(CmpOp, ValueId, ValueId, Type)` with 4th field
2. **emit_cmp uses operand type for signed/unsigned** — passes operand type, not Bool
3. **FieldStore instruction** — new `FieldStore(String, String, ValueId)` instruction added
4. **SelectArm has_default** — `SelectArm(Vec<ValueId>, bool)` with has_default flag
5. **VariantInit in is_pure** — added to DCE purity list
6. **Copy propagation removes dead Copies** — `bb.insts.retain(...)` removes resolved copies
7. **Phi simplification handles empty unique set** — `else if unique.is_empty() { continue; }`
8. **Constant folding uses wrapping arithmetic** — `wrapping_add/sub/mul/div/rem` for Add/Sub/Mul/Div/Mod
9. **GVN invalidates on FieldSet/FieldStore/IndexSet/Call** — proper cache invalidation
10. **Store-load forwarding invalidates on FieldStore** — removes var from `known`
11. **And/Or short-circuit evaluation** — branch+phi pattern implemented
12. **IndexSet handles Vec types** — header GEP + bounds check + store
13. **Float Exp (powf)** — calls `pow()` from libm
14. **DynDispatch implemented** — fat-pointer vtable lookup with indirect call
15. **Slice implemented** — dispatches to `emit_slice()`
16. **Enum tag FieldGet zero-extends i32→i64** — `build_int_z_extend`
17. **emit_select handles default arms** — `has_default` passed through
18. **Select variable demotion handles pre-existing vars** — `pre_existing` filter
19. **For-loop Branch terminators** — both explicit and implicit range paths fixed
