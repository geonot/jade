# MIR → HIR codegen helper call-site map

**Status:** generated for [CLEANUP.md](../../CLEANUP.md) section C.1, task 1
(May 2026). Read together with [docs/architecture.md](../architecture.md).

## Purpose

Section C.1 of the cleanup plan eliminates the HIR↔MIR codegen seam by inlining
or promoting every `MirCodegen → Compiler` (HIR codegen) helper call.  This
document is the prerequisite inventory: every method on `Compiler` that is
invoked from `src/codegen/mir_codegen/*.rs`, where it lives, what it does,
where it is called from, and how task C.1 will dispose of it.

## Scope

Two classes of cross-edge exist:

1. **LLVM-context field accesses** — `self.comp.bld`, `.ctx`, `.module`,
   `.cur_fn`, `.fns`, `.globals`. **1,222** occurrences across `mir_codegen/`.
   These are not "helper calls"; they are direct reads of LLVM state that
   conceptually belongs to a unified `Codegen`. Task C.1 step **7** (collapse
   `Compiler` and `MirCodegen` into one struct) makes them implicit `self.*`
   accesses. Not enumerated here.
2. **Helper method invocations** — `self.comp.<method>(...)`. **452**
   occurrences across **108** distinct methods. Enumerated below.

The May 2026 audit's coarse "15 call sites" referred to invocations into the
six designated HIR-era helper modules
(`expr.rs`, `stmt.rs`, `stores.rs`, `store_ops.rs`, `vec.rs`, `strings.rs`).
The exact count is **33 distinct helpers / 124 call sites**, broken down per
module below.

## Disposition legend

- **Inline** — used in ≤2 MIR call sites and trivial; copy the body inline at
  the call site, delete the helper. (Plan task 2.)
- **Promote-LLVM** — pure LLVM helper with no HIR-AST dependency; move to
  `src/codegen/llvm_util.rs` and let both sides call it. (Plan task 3.)
- **Promote-MIR** — codegen logic that conceptually belongs in MIR; move into
  `src/codegen/mir_codegen/{store,concurrency,…}.rs`. (Plan tasks 4 & 5.)
- **Stays-in-`Codegen`** — non-codegen-specific state plumbing (variable
  scope, debug info, target setup, declarations). Will live on the merged
  `Codegen` struct as plain `self.*` after task 7.

---

## Cross-edges into the six HIR-era modules (the "15-site" surface)

### `src/codegen/expr.rs` — 2 helpers, 4 call sites

| Helper | Sites | Purpose | Disposition |
| --- | ---: | --- | --- |
| `compile_expr` | 1 | Walk an `hir::Expr`. Single residual call from MIR is in [src/codegen/mir_codegen/mod.rs:1202](../../src/codegen/mir_codegen/mod.rs) (default-value initializers for global declarations — a HIR-level construct). | Inline as `compile_const_expr` already covers consts; for runtime defaults emit a small MIR initializer fragment (task 2). |
| `compile_str_literal` | 3 | Build a `String` value from a Rust `&str` (alloc + memcpy + descriptor). Sites: [mir_codegen/mod.rs:814](../../src/codegen/mir_codegen/mod.rs), [mir_codegen/helpers.rs:18](../../src/codegen/mir_codegen/helpers.rs), [mir_codegen/store.rs:142](../../src/codegen/mir_codegen/store.rs). | **Promote-LLVM** to `llvm_util::emit_str_literal` (task 3). |

### `src/codegen/stmt.rs` — 0 MIR call sites

The entire 628 LOC file is unreachable from `mir_codegen/`. Internal callers
are all inside HIR codegen modules themselves (`compile_block` is reached only
from `compile_expr`, recursive). Once task 2 inlines the residual
`compile_expr` call above, **the whole file becomes dead** — delete in task 6.

### `src/codegen/stores.rs` — 18 helpers, 64 call sites

This is the live core of HIR codegen still consumed by MIR.

| Helper | Sites | Purpose | Disposition |
| --- | ---: | --- | --- |
| `declare_store_runtime` | 2 | Forward-declare runtime entry points for store ops. Sites: mod.rs:305, mod.rs:315. | Stays-in-`Codegen` (declaration plumbing). |
| `declare_store` | 1 | Emit per-store globals for one `StoreDef`. Site: mod.rs:307. | Stays. |
| `load_kv_handle` | 6 | Load a `kv_handle*` global by store name. All sites in store_ext.rs. | **Promote-MIR** to `mir_codegen/store.rs` (task 4). |
| `load_vec_handle` | 3 | Same shape, vector index handle (store_ext.rs:461,521,578). | Promote-MIR. |
| `load_col_handle` | 2 | Columnar-field handle (store_ext.rs:961, store.rs:393). | Promote-MIR. |
| `load_bloom_handle` | 1 | Bloom-filter handle (store_ext.rs:601). | Promote-MIR. |
| `load_fts_handle` | 3 | FTS handle (store_ext.rs:641,667; store.rs:448). | Promote-MIR. |
| `load_store_idx` | 3 | Hash-index ptr by store/field (store.rs:192,363,577). | Promote-MIR. |
| `load_store_fp` | 9 | Open-store file-pointer load. Sites span store.rs and store_ext.rs. | Promote-MIR. |
| `load_store_ver` | 4 | Per-store version-counter ptr (store_ext.rs ×3, store.rs:2277). | Promote-MIR. |
| `gen_store_ensure_open` | 9 | Idempotent "open-once" function emitter. Pairs 1:1 with `load_store_fp`. | Promote-MIR. |
| `store_record_size` | 9 | Pure: byte size of a store's record layout. | **Promote-LLVM** (`llvm_util::store_record_size`). |
| `store_lock` / `store_unlock` | 5 / 6 | Emit advisory file-lock RT calls. | Promote-MIR (concurrency wrapper). |
| `gen_store_uuid` | 1 | Emit a UUID-build sequence (store.rs:138). | Inline (single use). |
| `gen_migration` | 1 | Compile a `MigrationDef` into an LLVM function (mod.rs:318). | Promote-MIR (lives next to declare-and-call site). |
| `idx_hash_field` | 3 | Hash a field value for index probe. | Promote-MIR. |
| `hash_store_field_from_gep` | 1 | Same, but from a GEP pointer (store_ext.rs:823). | Inline. |
| `ensure_time_fn` | 3 | Declare `clock_gettime` once (store.rs:100,1704,2319). | Promote-LLVM. |
| `wal_write_insert` | 1 | WAL append for INSERT (store.rs:465). | Promote-MIR. |
| `wal_write_delete` | 2 | WAL append for DELETE (store.rs:1789,2028). | Promote-MIR. |
| `wal_write_update` | 2 | WAL append for UPDATE (store.rs:2339,2922). | Promote-MIR. |
| `wal_checkpoint` | 1 | WAL fsync/checkpoint (store.rs:2973). | Promote-MIR. |

After this migration, `store_field_llvm_ty`, `load_store_wal`, `store_flock`,
`field_byte_size`, `field_has_index`, `field_is_unique`, `store_is_versioned`
in `stores.rs` become unreachable (no MIR consumer, no internal call from
helpers MIR still uses) and are deleted in task 6.

### `src/codegen/store_ops.rs` — 2 helpers, 26 call sites

| Helper | Sites | Purpose | Disposition |
| --- | ---: | --- | --- |
| `store_read_count` | 13 | Read row-count from a store handle. | Promote-MIR. |
| `store_load_records` | 13 | Read all records into a heap buffer. | Promote-MIR. |

The six high-level operations (`compile_store_insert`, `compile_store_count`,
`compile_store_query`, `compile_store_all`, `compile_store_delete`,
`compile_store_set`) have **no MIR callers**: they were re-implemented in
[mir_codegen/store.rs](../../src/codegen/mir_codegen/store.rs). They survive
only via internal cross-references inside `store_ops.rs`. **Whole file
deletes in task 6** after the two helpers above are promoted.

### `src/codegen/vec.rs` — 9 helpers, 16 call sites

| Helper | Sites | Purpose | Disposition |
| --- | ---: | --- | --- |
| `vec_header_type` | 8 | Pure LLVM struct type for `Vec<T>` header. | **Promote-LLVM**. |
| `vec_len` | 2 | Load `.len` field from header (mod.rs:873,926). | Promote-LLVM. |
| `vec_push_raw` | 1 | Push value with realloc-on-grow (mod.rs:878). | Inline / promote (last MIR caller). |
| `vec_pop` | 1 | Pop last (mod.rs:883). | Inline. |
| `vec_get_idx` | 1 | Indexed load (mod.rs:887). | Inline. |
| `vec_set_val` | 1 | Indexed store (mod.rs:896). | Inline. |
| `vec_remove_val` | 1 | Remove-at-index (mod.rs:903). | Inline. |
| `vec_clear` | 1 | Set len to 0 (mod.rs:907). | Inline. |
| `emit_vec_bounds_check` | 1 | Trap-on-OOB (mod.rs:1474). | Promote-LLVM. |
| `ensure_calloc` | 1 | Declare `calloc` once (store_ext.rs:734). | Promote-LLVM. |

After migration, all of `vec_data_and_len`, `vec_alloc_empty`, `vec_map`,
`vec_filter`, `vec_fold`, `vec_any_all`, `vec_find`, `vec_sum`,
`vec_take_skip`, `vec_zip`, `vec_chain`, `vec_enumerate`, `vec_flatten`,
`vec_contains`, `vec_reverse`, `vec_sort`, `vec_join`, `vec_map`,
`compile_vec_method`, `compile_vec_new`, `vec_push`, `vec_get`,
`array_contains`, `ensure_realloc`, `ensure_memmove`, `ensure_memset`,
`get_or_declare_trap` (≈ 28 helpers, ~1,500 LOC) — none of which MIR uses —
are dead and deleted in task 6.

### `src/codegen/strings.rs` — 4 helpers, 29 call sites

| Helper | Sites | Purpose | Disposition |
| --- | ---: | --- | --- |
| `string_len` | 12 | Load `.len` from a `String` value. | **Promote-LLVM**. |
| `string_data` | 11 | Load `.data` ptr from a `String` value. | Promote-LLVM. |
| `build_string` | 3 | Construct `String { ptr,len,cap }` triple from raw fields (intrinsics.rs:238,274,888). | Promote-LLVM. |
| `snprintf_to_string` | 3 | Format-via-`snprintf` then wrap in `String` (intrinsics.rs:175,188,208). | Promote-LLVM. |

The SSO-related helpers (`sso_branch`, `build_sso_result`,
`finalize_string_sso`, `compile_string_method`) are HIR-only and dead-removed
in task 6.

---

## All other `Compiler` methods called from `mir_codegen/`

Helpers that live outside the six designated modules but are still part of the
seam. Disposition is set by their nature, not by current location.

### Task 2 — Inline (single MIR site, trivial body)

| Helper | Defined in | Used by |
| --- | --- | --- |
| `compile_time_monotonic` | [src/codegen/fmt.rs](../../src/codegen/fmt.rs) | mir_codegen (1 site) |
| `compile_get_args` | [src/codegen/builtins.rs](../../src/codegen/builtins.rs) | mir_codegen (1 site) |
| `compile_const_expr` | [src/codegen/mod.rs](../../src/codegen/mod.rs) | mir_codegen (1 site) |
| `pop_debug_scope`, `finalize_debug` | mod.rs | mir_codegen (1 each) |

### Task 3 — Promote-LLVM (pure LLVM helpers, no HIR dep)

Move to `src/codegen/llvm_util.rs` and call from both sides.

| Helper | Origin | Sites |
| --- | --- | ---: |
| `entry_alloca` | mod.rs | 62 |
| `llvm_ty` | types.rs | 31 |
| `call_result` | mod.rs | 19 |
| `ensure_free`, `ensure_malloc`, `ensure_memcpy`, `ensure_calloc` | mod.rs / vec.rs | 14 / 11 / 5 / 1 |
| `set_tbaa`, `attr` | mod.rs | 6 / 5 |
| `mk_fn_type`, `tag_fn`, `closure_type` | mod.rs / types.rs | 3 / 3 / 2 |
| `type_store_size`, `default_val`, `string_type` | types.rs | 7 / 4 / 2 |
| `coerce_to_i64`, `zero_init` | coroutines.rs / mod.rs | 1 / 1 |

### Task 4 — Promote-MIR (codegen logic, MIR-only consumer)

Move into the appropriate `src/codegen/mir_codegen/` file.

- All `stores.rs` and `store_ops.rs` helpers tagged Promote-MIR above →
  [src/codegen/mir_codegen/store.rs](../../src/codegen/mir_codegen/store.rs)
  (where most equivalents already live).
- `eval_store_filter`, `load_store_record_as_jinn`, `copy_string_to_fixed_buf`
  from `store_filter.rs` → mir_codegen/store.rs.
- All `string_*` transform/op helpers
  (`string_concat`, `string_eq`, `string_split`, `string_case`, `string_trim`,
  `string_replace`, `string_repeat`, `string_find`, `string_starts_with`,
  `string_ends_with`, `string_contains`, `string_slice`, `string_char_at`)
  from `string_ops.rs` / `string_transform.rs` → new
  `mir_codegen/strings.rs` (or fold into helpers.rs).
- All `map_*` helpers from `map.rs`, `compile_map_new` →
  new `mir_codegen/map.rs`.
- `int_to_string`, `float_to_string`, `bool_to_string`, `emit_log` from
  `conversions.rs` → mir_codegen/helpers.rs.
- `find_var`, `set_var` (variable-scope plumbing) → fields of merged `Codegen`.
- `make_closure`, `fn_ref_wrapper` from `lambda.rs` →
  new `mir_codegen/closures.rs`.

### Task 5 — Promote-MIR (concurrency)

New file `src/codegen/mir_codegen/concurrency.rs`.

| Helper | Origin |
| --- | --- |
| `compile_actor_loop`, `compile_spawn`, `declare_actor`, `declare_actor_runtime` | actors.rs |
| `compile_coroutine_create`, `declare_gen_runtime` | coroutines.rs |
| `compile_supervisor` | mod.rs |

### Task 6 — Dead-code candidates after tasks 2–5

Scan each HIR-era module post-promotion; delete unreachable items.
Pre-deletion estimate (helpers with no MIR caller and only HIR-internal
chains of users):

| File | LOC | Likely fate |
| --- | ---: | --- |
| `expr.rs` | 1,796 | Delete (only `compile_str_literal` used externally; promote and delete). |
| `stmt.rs` | 628 | Delete entirely. |
| `store_ops.rs` | 882 | Delete entirely (helpers promoted, rest dead). |
| `vec.rs` | 1,830 | Delete the ~28 dead helpers above (~1,500 LOC); keep promoted utilities in `llvm_util`. |
| `strings.rs` | 419 | Delete the ~4 SSO helpers (~150 LOC). |
| `string_ops.rs` | 263 | Delete after promotion to mir_codegen/strings.rs. |
| `string_transform.rs` | 630 | Delete after promotion. |
| `stores.rs` | 1,708 | Trim to declarations-only after promotion (~300 LOC retained). |
| `store_filter.rs` | 459 | Delete after promotion. |
| `map.rs` | 610 | Delete after promotion. |
| `actors.rs` | 607 | Delete after promotion (concurrency.rs takes content). |
| `coroutines.rs` | 456 | Delete after promotion. |
| `lambda.rs` | 369 | Delete after promotion. |
| `conversions.rs` | 215 | Delete after promotion. |
| `fmt.rs` | 262 | Inline `compile_time_monotonic`; remainder is HIR-internal — delete. |
| **Net** | | **≈ −7,000 LOC** vs the C.0 baseline's 7,213 LOC of HIR helpers. |

### Task 7 — Struct merge

After tasks 2–6, the residual `Compiler` surface is just LLVM context fields
(`bld`, `ctx`, `module`, `cur_fn`, `fns`, `globals`) plus debug-info /
target-setup / declaration plumbing. Rename `MirCodegen → Codegen`, hoist the
LLVM fields directly onto it, and delete `Compiler`. All
`self.comp.<x>` reads become `self.<x>`. Acceptance grep
(`grep -r "self.comp" src/codegen/mir_codegen/`) returns nothing.

---

## Method × MIR-callsite census (full)

`grep -rhoE 'self\.comp\.[a-zA-Z_][a-zA-Z0-9_]*\(' src/codegen/mir_codegen/ | sort | uniq -c | sort -rn`

```
62  entry_alloca         31  llvm_ty              19  call_result
14  ensure_free          13  store_read_count     13  store_load_records
12  string_len           11  string_data          11  ensure_malloc
 9  store_record_size     9  load_store_fp         9  gen_store_ensure_open
 8  vec_header_type       7  type_store_size       6  store_unlock
 6  set_tbaa              6  load_kv_handle        5  store_lock
 5  load_store_record_as_jinn  5  ensure_memcpy   5  attr
 4  load_store_ver        4  int_to_string         4  default_val
 4  copy_string_to_fixed_buf
 3  tag_fn                3  string_trim           3  snprintf_to_string
 3  mk_fn_type            3  load_vec_handle       3  load_store_idx
 3  load_fts_handle       3  idx_hash_field        3  find_var
 3  eval_store_filter     3  ensure_time_fn        3  compile_str_literal
 3  build_string
 2  wal_write_update      2  wal_write_delete      2  vec_len
 2  string_type           2  string_split          2  string_char_at
 2  string_case           2  rc_retain             2  load_col_handle
 2  declare_store_runtime 2  declare_actor_runtime 2  closure_type
 1  zero_init             1  weak_upgrade          1  wal_write_insert
 1  wal_checkpoint        1  vec_set_val           1  vec_remove_val
 1  vec_push_raw          1  vec_pop               1  vec_get_idx
 1  vec_clear             1  string_starts_with    1  string_slice
 1  string_replace        1  string_repeat         1  string_find
 1  string_eq             1  string_ends_with      1  string_contains
 1  string_concat         1  setup_target          1  set_var
 1  rc_release            1  rc_deref              1  rc_alloc
 1  pop_debug_scope       1  map_set_val           1  map_remove_val
 1  map_has_val           1  map_get_val           1  map_clear
 1  make_closure          1  load_bloom_handle     1  hash_store_field_from_gep
 1  generate_vtables      1  gen_store_uuid        1  gen_migration
 1  fn_ref_wrapper        1  float_to_string       1  finalize_debug
 1  ensure_calloc         1  emit_vec_bounds_check 1  emit_log
 1  drop_value            1  declare_store         1  declare_method
 1  declare_jinn_runtime  1  declare_gen_runtime   1  declare_err_def
 1  declare_enum          1  declare_builtins      1  declare_actor
 1  compile_time_monotonic 1 compile_supervisor    1  compile_spawn
 1  compile_map_new       1  compile_get_args      1  compile_expr
 1  compile_coroutine_create 1 compile_const_expr  1  compile_actor_loop
 1  coerce_to_i64         1  bool_to_string
```

108 distinct helpers, 452 call sites total.

---

## Execution order recommendation for tasks 2–7

Order chosen to keep tests green at every commit and to minimise rebase pain.

1. **Task 3 first (Promote-LLVM).** Lowest risk: pure helpers, no semantic
   change, both call sites continue to compile. Largest count reduction
   (≈ 200 of the 452 call sites become parameterised `llvm_util::*` calls
   that no longer go through `self.comp`).
2. **Task 2 (Inline trivials).** Drops the awkward 1-site `compile_*` calls
   from `expr.rs` / `fmt.rs` / `builtins.rs` and detaches MIR from
   `compile_expr`.
3. **Task 4 (Store helpers → MIR).** The largest live cluster. Move
   `store_*`, `load_*`, `wal_*`, `gen_store_*`, `idx_hash_field`,
   `eval_store_filter`, `load_store_record_as_jinn` into
   `mir_codegen/store.rs` / `store_ext.rs`. Then delete `store_ops.rs`
   wholesale and most of `stores.rs` / `store_filter.rs`.
4. **Task 5 (Concurrency).** Create `mir_codegen/concurrency.rs`; move actors,
   coroutines, supervisors. Delete `actors.rs`, `coroutines.rs`.
5. **Task 6 (Dead-code sweep).** With MIR no longer reaching anything in
   `expr.rs`/`stmt.rs`/`store_ops.rs`/`map.rs`/`lambda.rs`/`string_*`/
   `conversions.rs`/`fmt.rs`/`vec.rs`, delete each module after a final
   `cargo check` confirms no live import. Expect ≈ −5,000 LOC.
6. **Task 7 (Struct merge).** Mechanical rename + field hoist. Apply with
   `cargo fmt` and `cargo clippy` to fix any stragglers. Verify acceptance
   greps:

   ```bash
   ! grep -rq 'self\.comp' src/codegen/mir_codegen/
   ! grep -rqi 'HIR codegen' src/
   ```

---

## Test baseline

Cleanup section preamble: **1565 passed / 0 failed** (release tests). Every
task above must preserve that count — the work is non-functional by design.
