//! Extended store codegen: KV, graph, time-series, vector, bloom, FTS, distinct, aggregation, and versioning operations.

use super::super::Compiler;
use super::super::b;
use crate::mir;
use inkwell::values::BasicValueEnum;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_kv_set(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.len() < 2 {
            return Err("kv_set requires key and value args".into());
        }
        let kv = self.load_kv_handle(store_name)?;
        let key_val = self.value_map[&args[0]];
        let val_val = self.value_map[&args[1]];

        let key_data = self.string_data(key_val)?;
        let key_len = self.string_len(key_val)?;

        let set_fn = crate::codegen::fn_or_die(&self.module, "jade_kv_set");
        b!(self.bld.build_call(
            set_fn,
            &[kv.into(), key_data.into(), key_len.into(), val_val.into()],
            ""
        ));
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn emit_kv_get(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("kv_get requires key arg".into());
        }
        let kv = self.load_kv_handle(store_name)?;
        let key_val = self.value_map[&args[0]];

        let key_data = self.string_data(key_val)?;
        let key_len = self.string_len(key_val)?;

        let get_fn = crate::codegen::fn_or_die(&self.module, "jade_kv_get");
        let result = self
            .call_result(b!(self.bld.build_call(
                get_fn,
                &[kv.into(), key_data.into(), key_len.into()],
                "kv.val"
            )))
            .into_int_value();
        Ok(result.into())
    }

    pub(in crate::codegen) fn emit_kv_has(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("kv_has requires key arg".into());
        }
        let kv = self.load_kv_handle(store_name)?;
        let key_val = self.value_map[&args[0]];

        let key_data = self.string_data(key_val)?;
        let key_len = self.string_len(key_val)?;

        let has_fn = crate::codegen::fn_or_die(&self.module, "jade_kv_has");
        let result = self
            .call_result(b!(self.bld.build_call(
                has_fn,
                &[kv.into(), key_data.into(), key_len.into()],
                "kv.has"
            )))
            .into_int_value();
        // Convert i32 result to i1 bool
        let bool_val = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            result,
            self.ctx.i32_type().const_int(0, false),
            "kv.has.bool"
        ));
        Ok(bool_val.into())
    }

    pub(in crate::codegen) fn emit_kv_del(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("kv_del requires key arg".into());
        }
        let kv = self.load_kv_handle(store_name)?;
        let key_val = self.value_map[&args[0]];

        let key_data = self.string_data(key_val)?;
        let key_len = self.string_len(key_val)?;

        let del_fn = crate::codegen::fn_or_die(&self.module, "jade_kv_del");
        b!(self
            .bld
            .build_call(del_fn, &[kv.into(), key_data.into(), key_len.into()], ""));
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn emit_kv_incr(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.len() < 2 {
            return Err("kv_incr requires key and delta args".into());
        }
        let kv = self.load_kv_handle(store_name)?;
        let key_val = self.value_map[&args[0]];
        let delta_val = self.value_map[&args[1]];

        let key_data = self.string_data(key_val)?;
        let key_len = self.string_len(key_val)?;

        let incr_fn = crate::codegen::fn_or_die(&self.module, "jade_kv_incr");
        b!(self.bld.build_call(
            incr_fn,
            &[kv.into(), key_data.into(), key_len.into(), delta_val.into()],
            ""
        ));
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn emit_kv_count(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let kv = self.load_kv_handle(store_name)?;
        let count_fn = crate::codegen::fn_or_die(&self.module, "jade_kv_count");
        let result = self
            .call_result(b!(self.bld.build_call(count_fn, &[kv.into()], "kv.cnt")))
            .into_int_value();
        Ok(result.into())
    }

    // ── @graph store codegen ────────────────────────────────────────

    /// Graph query: count edges where field[0] or field[1] == node_value.
    /// .from(node) matches first user field (src), .to(node) matches second user field (dst).
    pub(in crate::codegen) fn emit_graph_query(
        &mut self,
        store_name: &str,
        direction: &str, // "from" or "to"
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err(format!("graph.{direction}() requires a node argument"));
        }
        let (sd, _st, rec_size, fp) = self.setup_store_access(store_name)?;
        let node_val = self.value_map[&args[0]];

        // Skip builtin fields to find user fields
        let builtin_names = [
            "sid",
            "uuid",
            "hash",
            "created",
            "updated",
            "deleted",
            "__version",
        ];
        let user_fields: Vec<(usize, &crate::hir::StoreField)> = sd
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| !builtin_names.contains(&&*f.name.as_str()))
            .collect();
        // .from() → first user field, .to() → second user field
        let target_idx = if direction == "from" { 0usize } else { 1 };
        if target_idx >= user_fields.len() {
            return Err(format!(
                "@graph store '{store_name}' needs at least 2 user fields (src, dst)"
            ));
        }
        let (field_idx, field_def) = user_fields[target_idx];
        let field_ty = field_def.ty.clone();

        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        // fseek to after header
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(24, false).into(), // HEADER_SIZE
                i32t.const_int(0, false).into(),  // SEEK_SET
            ],
            ""
        ));

        // Read record count from header: fseek(0), fread count, fseek back
        let _ftell_fn = crate::codegen::fn_or_die(&self.module, "ftell");
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");

        // First read count from offset 8
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));
        let count_alloca = self.entry_alloca(i64t.into(), "g.cnt");
        b!(self.bld.build_call(
            fread_fn,
            &[
                count_alloca.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));
        let total = b!(self.bld.build_load(i64t, count_alloca, "g.total")).into_int_value();

        // Seek back to data start
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(24, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));

        // Allocate one record buffer
        let rec_name = format!("__store_{store_name}_rec");
        let rec_st = self
            .module
            .get_struct_type(&rec_name)
            .ok_or_else(|| format!("no record type for '{store_name}'"))?;
        let rec_buf = self.entry_alloca(rec_st.into(), "g.rec");

        let rec_size_val = i64t.const_int(rec_size, false);
        let match_count = self.entry_alloca(i64t.into(), "g.matches");
        b!(self.bld.build_store(match_count, i64t.const_int(0, false)));

        // Loop through records
        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let loop_bb = self.ctx.append_basic_block(fv, "g.loop");
        let body_bb = self.ctx.append_basic_block(fv, "g.body");
        let inc_bb = self.ctx.append_basic_block(fv, "g.inc");
        let done_bb = self.ctx.append_basic_block(fv, "g.done");

        let idx = self.entry_alloca(i64t.into(), "g.idx");
        b!(self.bld.build_store(idx, i64t.const_int(0, false)));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let cur_idx = b!(self.bld.build_load(i64t, idx, "g.i")).into_int_value();
        let cond =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, cur_idx, total, "g.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        b!(self.bld.build_call(
            fread_fn,
            &[
                rec_buf.into(),
                rec_size_val.into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));

        // Load the field value and compare with node_val
        let field_ptr = b!(self
            .bld
            .build_struct_gep(rec_st, rec_buf, field_idx as u32, "g.fp"));
        let cmp_result = match &field_ty {
            crate::types::Type::I64 => {
                let fval = b!(self.bld.build_load(i64t, field_ptr, "g.fv")).into_int_value();
                b!(self.bld.build_int_compare(
                    inkwell::IntPredicate::EQ,
                    fval,
                    node_val.into_int_value(),
                    "g.eq"
                ))
            }
            crate::types::Type::String => {
                // Compare fixed-size string buffer
                let memcmp_fn = crate::codegen::fn_or_die(&self.module, "memcmp");
                let node_data = self.string_data(node_val)?;
                let node_len = self.string_len(node_val)?;
                // Load stored string length (first 8 bytes of the 256-byte buffer)
                let stored_len =
                    b!(self.bld.build_load(i64t, field_ptr, "g.slen")).into_int_value();
                // Compare lengths first
                let len_eq = b!(self.bld.build_int_compare(
                    inkwell::IntPredicate::EQ,
                    stored_len,
                    node_len.into_int_value(),
                    "g.leq"
                ));
                // If lengths match, compare data
                let _ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                let data_ptr = unsafe {
                    b!(self.bld.build_gep(
                        self.ctx.i8_type(),
                        field_ptr,
                        &[i64t.const_int(8, false)],
                        "g.sdp"
                    ))
                };
                let cmp_val = self
                    .call_result(b!(self.bld.build_call(
                        memcmp_fn,
                        &[data_ptr.into(), node_data.into(), node_len.into()],
                        "g.cmp"
                    )))
                    .into_int_value();
                let data_eq = b!(self.bld.build_int_compare(
                    inkwell::IntPredicate::EQ,
                    cmp_val,
                    i32t.const_int(0, false),
                    "g.deq"
                ));
                b!(self.bld.build_and(len_eq, data_eq, "g.match"))
            }
            _ => {
                return Err(format!(
                    "graph field type {:?} not supported for comparison",
                    field_ty
                ));
            }
        };

        // If match, increment count
        let match_bb = self.ctx.append_basic_block(fv, "g.matched");
        b!(self
            .bld
            .build_conditional_branch(cmp_result, match_bb, inc_bb));

        self.bld.position_at_end(match_bb);
        let cur_count = b!(self.bld.build_load(i64t, match_count, "g.mc")).into_int_value();
        let new_count = b!(self
            .bld
            .build_int_add(cur_count, i64t.const_int(1, false), "g.mc1"));
        b!(self.bld.build_store(match_count, new_count));
        b!(self.bld.build_unconditional_branch(inc_bb));

        self.bld.position_at_end(inc_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(cur_idx, i64t.const_int(1, false), "g.ni"));
        b!(self.bld.build_store(idx, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let result = b!(self.bld.build_load(i64t, match_count, "g.result")).into_int_value();
        Ok(result.into())
    }

    // ── @timeseries store codegen ───────────────────────────────────

    /// Return the count of records in a timeseries store (latest = highest index).
    pub(in crate::codegen) fn emit_ts_latest(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (_sd, _st, _rec_size, fp) = self.setup_store_access(store_name)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");

        // Read count from header offset 8
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));
        let count_alloca = self.entry_alloca(i64t.into(), "ts.cnt_buf");
        b!(self.bld.build_call(
            fread_fn,
            &[
                count_alloca.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));
        let count = b!(self.bld.build_load(i64t, count_alloca, "ts.count")).into_int_value();
        Ok(count.into())
    }

    // ── @vector store codegen ───────────────────────────────────

    /// Emit vec.nearest(query_array, k) — calls jade_vec_nearest with stack-allocated buffers.
    /// Returns the count of results found (up to k). For now we return just the count.
    pub(in crate::codegen) fn emit_vec_nearest(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Look up the vector dimensions from store decorators
        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();
        let dims = sd
            .decorators
            .iter()
            .find_map(|d| match d {
                crate::ast::StoreDecorator::Vector(n) => Some(*n),
                _ => None,
            })
            .ok_or_else(|| format!("store '{store_name}' is not @vector"))?;

        let i64t = self.ctx.i64_type();
        let _f64t = self.ctx.f64_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

        // Load the vector handle (lazy open)
        let vec_handle = self.load_vec_handle(store_name, dims)?;

        // args[0] = query vector — could be PointerValue (Jade Vec) or ArrayValue (literal)
        let arg_val = self.val(args[0]);
        let query_ptr = if arg_val.is_pointer_value() {
            let header_ty = self.vec_header_type();
            let gep =
                b!(self
                    .bld
                    .build_struct_gep(header_ty, arg_val.into_pointer_value(), 0, "vn.dp"));
            b!(self.bld.build_load(ptr_ty, gep, "vn.data")).into_pointer_value()
        } else {
            let alloca = self.entry_alloca(arg_val.get_type(), "vn.arr");
            b!(self.bld.build_store(alloca, arg_val));
            alloca
        };

        let k_val = self.val(args[1]).into_int_value();

        // Allocate output indices buffer on stack (k * sizeof(i64))
        let out_indices = b!(self.bld.build_array_alloca(i64t, k_val, "vec.out"));

        // Call jade_vec_nearest(handle, query_ptr, k, out_indices) -> count
        let nearest_fn = crate::codegen::fn_or_die(&self.module, "jade_vec_nearest");
        let result = self.call_result(b!(self.bld.build_call(
            nearest_fn,
            &[
                vec_handle.into(),
                query_ptr.into(),
                k_val.into(),
                out_indices.into()
            ],
            "vec.found"
        )));
        Ok(result)
    }

    /// Emit vec.insert(vec_array) — insert a vector into the store, returns count.
    pub(in crate::codegen) fn emit_vec_insert(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();
        let dims = sd
            .decorators
            .iter()
            .find_map(|d| match d {
                crate::ast::StoreDecorator::Vector(n) => Some(*n),
                _ => None,
            })
            .ok_or_else(|| format!("store '{store_name}' is not @vector"))?;

        let vec_handle = self.load_vec_handle(store_name, dims)?;
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

        // args[0] is the vector data — could be:
        // 1. A PointerValue to a Jade Vec header {ptr, len, cap} — extract field 0
        // 2. An ArrayValue [N x double] from a literal — alloca + store, pass pointer
        let arg_val = self.val(args[0]);
        let data_ptr = if arg_val.is_pointer_value() {
            // Jade Vec: extract data pointer from header field 0
            let header_ty = self.vec_header_type();
            let gep =
                b!(self
                    .bld
                    .build_struct_gep(header_ty, arg_val.into_pointer_value(), 0, "vi.dp"));
            b!(self.bld.build_load(ptr_ty, gep, "vi.data")).into_pointer_value()
        } else {
            // LLVM array value: alloca, store, pass pointer to the alloca
            let alloca = self.entry_alloca(arg_val.get_type(), "vi.arr");
            b!(self.bld.build_store(alloca, arg_val));
            alloca
        };

        let insert_fn = crate::codegen::fn_or_die(&self.module, "jade_vec_insert");
        b!(self
            .bld
            .build_call(insert_fn, &[vec_handle.into(), data_ptr.into()], ""));

        // Return the new count
        let count_fn = crate::codegen::fn_or_die(&self.module, "jade_vec_count");
        let count = self.call_result(b!(self.bld.build_call(
            count_fn,
            &[vec_handle.into()],
            "vi.cnt"
        )));
        Ok(count)
    }

    /// Emit vec.count() — return the number of vectors in the store.
    pub(in crate::codegen) fn emit_vec_count(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();
        let dims = sd
            .decorators
            .iter()
            .find_map(|d| match d {
                crate::ast::StoreDecorator::Vector(n) => Some(*n),
                _ => None,
            })
            .ok_or_else(|| format!("store '{store_name}' is not @vector"))?;

        let vec_handle = self.load_vec_handle(store_name, dims)?;
        let count_fn = crate::codegen::fn_or_die(&self.module, "jade_vec_count");
        let count = self.call_result(b!(self.bld.build_call(
            count_fn,
            &[vec_handle.into()],
            "vc.cnt"
        )));
        Ok(count)
    }

    pub(in crate::codegen) fn emit_bloom_test(
        &mut self,
        rest: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // rest = {store_name}_{field_name}
        let parts: Vec<&str> = rest.splitn(2, '_').collect();
        if parts.len() < 2 || args.is_empty() {
            return Err(format!("malformed bloom_test name: {rest}"));
        }
        let store_name = parts[0];
        let field_name = parts[1];

        let bloom = self.load_bloom_handle(store_name, field_name, 10000)?;
        let val = self.val(args[0]);

        let test_fn = self.module.get_function("jade_bloom_test_i64").unwrap();
        let result = self
            .call_result(b!(self.bld.build_call(
                test_fn,
                &[bloom.into(), val.into()],
                "bloom.res"
            )))
            .into_int_value();

        // Convert i64 result to i1 bool
        let bool_val = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            result,
            self.ctx.i64_type().const_int(0, false),
            "bloom.bool"
        ));
        Ok(bool_val.into())
    }

    pub(in crate::codegen) fn emit_fts_search(
        &mut self,
        rest: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // rest = {store_name}_{field_name}
        let parts: Vec<&str> = rest.splitn(2, '_').collect();
        if parts.len() < 2 || args.is_empty() {
            return Err(format!("malformed fts_search name: {rest}"));
        }
        let store_name = parts[0];
        let field_name = parts[1];

        let fts = self.load_fts_handle(store_name, field_name)?;
        let query_val = self.val(args[0]);

        let query_data = self.string_data(query_val)?;
        let query_len = self.string_len(query_val)?;

        let search_fn = crate::codegen::fn_or_die(&self.module, "jade_fts_search_n");
        let count = self
            .call_result(b!(self.bld.build_call(
                search_fn,
                &[fts.into(), query_data.into(), query_len.into()],
                "fts.res"
            )))
            .into_int_value();
        Ok(count.into())
    }
}

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_fts_count(
        &mut self,
        rest: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = rest.splitn(2, '_').collect();
        if parts.len() < 2 {
            return Err(format!("malformed fts_count name: {rest}"));
        }
        let store_name = parts[0];
        let field_name = parts[1];

        let fts = self.load_fts_handle(store_name, field_name)?;
        let count_fn = self.module.get_function("jade_fts_posting_count").unwrap();
        let count = self
            .call_result(b!(self.bld.build_call(count_fn, &[fts.into()], "fts.cnt")))
            .into_int_value();
        Ok(count.into())
    }

    /// Emit distinct(field) — returns count of distinct values for a field.
    /// Uses a simple hash-set approach: hash each field value, track in a
    /// dynamically allocated bitset/array, and count unique hashes.
    pub(in crate::codegen) fn emit_store_distinct(
        &mut self,
        rest: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = rest.splitn(2, "__").collect();
        if parts.len() < 2 {
            return Err(format!("malformed store distinct name: {rest}"));
        }
        let store_name = parts[0];
        let field_name = parts[1];

        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();

        let ensure_fn_name = format!("__store_ensure_{store_name}");
        if let Some(ensure_fn) = self.module.get_function(&ensure_fn_name) {
            b!(self.bld.build_call(ensure_fn, &[], ""));
        } else {
            let ensure_fn = self.gen_store_ensure_open(&sd)?;
            b!(self.bld.build_call(ensure_fn, &[], ""));
        }

        let fp = self.load_store_fp(store_name)?;
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        let rec_size = self.store_record_size(&sd);

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("no field '{field_name}' in store '{store_name}'"))?;

        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted");

        let total_count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, total_count, rec_size)?;

        // Allocate a hash table: open-addressing with linear probing.
        // Capacity = total_count * 4 + 16 (low load factor for O(1) amortized probe).
        // Sentinel: 0 = empty slot. We store (hash | 1) to avoid storing 0.
        let calloc_fn = self.ensure_calloc();
        let cap = b!(self.bld.build_int_add(
            b!(self
                .bld
                .build_int_mul(total_count, i64t.const_int(4, false), "dist.cap.mul")),
            i64t.const_int(16, false),
            "dist.cap"
        ));
        let hash_tbl = self
            .call_result(b!(self.bld.build_call(
                calloc_fn,
                &[cap.into(), i64t.const_int(8, false).into()],
                "dist.tbl"
            )))
            .into_pointer_value();

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let idx_ptr = self.entry_alloca(i64t.into(), "dist.idx");
        let uniq_ptr = self.entry_alloca(i64t.into(), "dist.uniq");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_store(uniq_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "dist.loop");
        let body_bb = self.ctx.append_basic_block(fv, "dist.body");
        let check_bb = self.ctx.append_basic_block(fv, "dist.check");
        let next_bb = self.ctx.append_basic_block(fv, "dist.next");
        let done_bb = self.ctx.append_basic_block(fv, "dist.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        // Loop condition
        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "dist.i")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::ULT,
            idx,
            total_count,
            "dist.cmp"
        ));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        // Body: skip deleted, compute hash
        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "dist.off"));
        let rec_ptr = unsafe { b!(self.bld.build_gep(i8t, buf, &[offset], "dist.rec")) };

        if let Some(del_idx) = deleted_idx {
            let del_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "dist.del"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "dist.del.val")).into_int_value();
            let is_live = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                del_val,
                i64t.const_int(0, false),
                "dist.live"
            ));
            b!(self
                .bld
                .build_conditional_branch(is_live, check_bb, next_bb));
        } else {
            b!(self.bld.build_unconditional_branch(check_bb));
        }

        // Check: hash the field value, see if we've seen it
        self.bld.position_at_end(check_bb);
        let field_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, field_idx as u32, "dist.fld"));

        // Load field and compute hash
        let hash = self.hash_store_field_from_gep(field_gep, &field_ty)?;

        // Open-addressing hash table probe: O(1) amortized dedup check.
        // marked_h = hash | 1  (ensure non-zero; 0 is the empty sentinel)
        let marked_h = b!(self
            .bld
            .build_or(hash, i64t.const_int(1, false), "dist.marked"));

        // initial slot = marked_h % cap  (unsigned remainder)
        let slot_ptr = self.entry_alloca(i64t.into(), "dist.slot");
        let init_slot = b!(self.bld.build_int_unsigned_rem(marked_h, cap, "dist.islot"));
        b!(self.bld.build_store(slot_ptr, init_slot));

        let probe_bb = self.ctx.append_basic_block(fv, "dist.probe");
        let add_bb = self.ctx.append_basic_block(fv, "dist.add");

        b!(self.bld.build_unconditional_branch(probe_bb));

        // Probe loop: check table[slot]
        self.bld.position_at_end(probe_bb);
        let slot = b!(self.bld.build_load(i64t, slot_ptr, "dist.s")).into_int_value();
        let entry_ptr = unsafe { b!(self.bld.build_gep(i64t, hash_tbl, &[slot], "dist.ep")) };
        let entry_val = b!(self.bld.build_load(i64t, entry_ptr, "dist.ev")).into_int_value();

        // If slot is empty (0), this hash is new → add it
        let is_empty = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            entry_val,
            i64t.const_int(0, false),
            "dist.empty"
        ));
        let match_bb = self.ctx.append_basic_block(fv, "dist.match");
        b!(self
            .bld
            .build_conditional_branch(is_empty, add_bb, match_bb));

        // Check if existing entry matches our hash → duplicate, skip
        self.bld.position_at_end(match_bb);
        let is_dup = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            entry_val,
            marked_h,
            "dist.dup"
        ));
        let advance_bb = self.ctx.append_basic_block(fv, "dist.advance");
        b!(self
            .bld
            .build_conditional_branch(is_dup, next_bb, advance_bb));

        // Collision: advance to next slot = (slot + 1) % cap
        self.bld.position_at_end(advance_bb);
        let next_slot = b!(self
            .bld
            .build_int_add(slot, i64t.const_int(1, false), "dist.ns"));
        let wrapped = b!(self.bld.build_int_unsigned_rem(next_slot, cap, "dist.wrap"));
        b!(self.bld.build_store(slot_ptr, wrapped));
        b!(self.bld.build_unconditional_branch(probe_bb));

        // Add new unique hash: store marked_h in table[slot], increment count
        self.bld.position_at_end(add_bb);
        // Reload slot (it's in slot_ptr, still valid from probe_bb → add_bb path)
        let add_slot = b!(self.bld.build_load(i64t, slot_ptr, "dist.as")).into_int_value();
        let add_entry = unsafe { b!(self.bld.build_gep(i64t, hash_tbl, &[add_slot], "dist.ae")) };
        b!(self.bld.build_store(add_entry, marked_h));
        let uc = b!(self.bld.build_load(i64t, uniq_ptr, "dist.uc2")).into_int_value();
        let new_uc = b!(self
            .bld
            .build_int_add(uc, i64t.const_int(1, false), "dist.ucinc"));
        b!(self.bld.build_store(uniq_ptr, new_uc));
        b!(self.bld.build_unconditional_branch(next_bb));

        // Next
        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "dist.ni"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        // Done
        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        b!(self.bld.build_call(free_fn, &[hash_tbl.into()], ""));

        let result = b!(self.bld.build_load(i64t, uniq_ptr, "dist.result")).into_int_value();
        Ok(result.into())
    }

    /// Emit aggregation: sum, avg, min, max over a numeric field.
    /// `rest` is "{store_name}__{field_name}", `op` is "sum"|"avg"|"min"|"max".
    pub(in crate::codegen) fn emit_store_agg(
        &mut self,
        rest: &str,
        op: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = rest.splitn(2, "__").collect();
        if parts.len() < 2 {
            return Err(format!("malformed store agg name: {rest}"));
        }
        let store_name = parts[0];
        let field_name = parts[1];

        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();

        // @column fast path: use column files for vectorized aggregation on i64
        let is_column = sd
            .decorators
            .iter()
            .any(|d| *d == crate::ast::StoreDecorator::Column);
        if is_column && (op == "sum" || op == "min" || op == "max") {
            let field_ty = sd
                .fields
                .iter()
                .find(|f| f.name == field_name)
                .map(|f| f.ty.clone());
            if let Some(ref fty) = field_ty {
                let norm = crate::codegen::store_filter::normalize_store_field_type(fty);
                let is_float = matches!(norm, crate::types::Type::F64 | crate::types::Type::F32);
                if !is_float {
                    let col = self.load_col_handle(store_name, field_name, 8)?;
                    let fn_name = format!("jade_col_{op}_i64");
                    let col_fn = self.module.get_function(&fn_name).unwrap();
                    let result = self
                        .call_result(b!(self.bld.build_call(col_fn, &[col.into()], "col.agg")))
                        .into_int_value();
                    return Ok(result.into());
                }
            }
        }

        let ensure_fn_name = format!("__store_ensure_{store_name}");
        if let Some(ensure_fn) = self.module.get_function(&ensure_fn_name) {
            b!(self.bld.build_call(ensure_fn, &[], ""));
        } else {
            let ensure_fn = self.gen_store_ensure_open(&sd)?;
            b!(self.bld.build_call(ensure_fn, &[], ""));
        }

        let fp = self.load_store_fp(store_name)?;
        let i64t = self.ctx.i64_type();
        let f64t = self.ctx.f64_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        let rec_size = self.store_record_size(&sd);

        // Determine field index and whether field is float
        let (field_idx, is_float) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| {
                let norm = crate::codegen::store_filter::normalize_store_field_type(&f.ty);
                (
                    i,
                    matches!(norm, crate::types::Type::F64 | crate::types::Type::F32),
                )
            })
            .ok_or_else(|| format!("no field '{field_name}' in store '{store_name}'"))?;

        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted");

        let total_count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, total_count, rec_size)?;

        let fv = self.cur_fn.expect("ICE: cur_fn not set");

        // Accumulators — use f64 for float fields, i64 for integer fields
        let acc_ptr = if is_float {
            self.entry_alloca(f64t.into(), "agg.acc")
        } else {
            self.entry_alloca(i64t.into(), "agg.acc")
        };
        let cnt_ptr = self.entry_alloca(i64t.into(), "agg.cnt");
        let idx_ptr = self.entry_alloca(i64t.into(), "agg.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_store(cnt_ptr, i64t.const_int(0, false)));
        // Initialize acc based on op and type
        if is_float {
            let init_acc = match op {
                "sum" | "avg" => f64t.const_float(0.0),
                "min" => f64t.const_float(f64::MAX),
                "max" => f64t.const_float(f64::MIN),
                _ => f64t.const_float(0.0),
            };
            b!(self.bld.build_store(acc_ptr, init_acc));
        } else {
            let init_acc = match op {
                "sum" | "avg" => i64t.const_int(0, false),
                "min" => i64t.const_int(i64::MAX as u64, false),
                "max" => i64t.const_int(i64::MIN as u64, true),
                _ => i64t.const_int(0, false),
            };
            b!(self.bld.build_store(acc_ptr, init_acc));
        }

        let loop_bb = self.ctx.append_basic_block(fv, "agg.loop");
        let body_bb = self.ctx.append_basic_block(fv, "agg.body");
        let accum_bb = self.ctx.append_basic_block(fv, "agg.accum");
        let next_bb = self.ctx.append_basic_block(fv, "agg.next");
        let done_bb = self.ctx.append_basic_block(fv, "agg.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        // Loop condition
        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "agg.i")).into_int_value();
        let cmp =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, total_count, "agg.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        // Body: check deleted, load field
        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "agg.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "agg.rec"))
        };

        // Skip deleted records
        if let Some(del_idx) = deleted_idx {
            let del_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "agg.del"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "agg.del.val")).into_int_value();
            let is_live = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                del_val,
                i64t.const_int(0, false),
                "agg.live"
            ));
            b!(self
                .bld
                .build_conditional_branch(is_live, accum_bb, next_bb));
        } else {
            b!(self.bld.build_unconditional_branch(accum_bb));
        }

        // Accumulate
        self.bld.position_at_end(accum_bb);
        let field_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, field_idx as u32, "agg.fld"));

        if is_float {
            let field_val = b!(self.bld.build_load(f64t, field_gep, "agg.fval")).into_float_value();
            let cur_acc = b!(self.bld.build_load(f64t, acc_ptr, "agg.fcur")).into_float_value();
            let new_acc = match op {
                "sum" | "avg" => {
                    b!(self.bld.build_float_add(cur_acc, field_val, "agg.fadd"))
                }
                "min" => {
                    let lt = b!(self.bld.build_float_compare(
                        inkwell::FloatPredicate::OLT,
                        field_val,
                        cur_acc,
                        "agg.flt"
                    ));
                    let sel = b!(self.bld.build_select(lt, field_val, cur_acc, "agg.fmin"));
                    sel.into_float_value()
                }
                "max" => {
                    let gt = b!(self.bld.build_float_compare(
                        inkwell::FloatPredicate::OGT,
                        field_val,
                        cur_acc,
                        "agg.fgt"
                    ));
                    let sel = b!(self.bld.build_select(gt, field_val, cur_acc, "agg.fmax"));
                    sel.into_float_value()
                }
                _ => cur_acc,
            };
            b!(self.bld.build_store(acc_ptr, new_acc));
        } else {
            let field_val = b!(self.bld.build_load(i64t, field_gep, "agg.val")).into_int_value();
            let cur_acc = b!(self.bld.build_load(i64t, acc_ptr, "agg.cur")).into_int_value();
            let new_acc = match op {
                "sum" | "avg" => {
                    b!(self.bld.build_int_add(cur_acc, field_val, "agg.add"))
                }
                "min" => {
                    let lt = b!(self.bld.build_int_compare(
                        inkwell::IntPredicate::SLT,
                        field_val,
                        cur_acc,
                        "agg.lt"
                    ));
                    b!(self
                        .bld
                        .build_select::<inkwell::values::IntValue, inkwell::values::IntValue>(
                            lt,
                            field_val.into(),
                            cur_acc.into(),
                            "agg.min"
                        ))
                    .into_int_value()
                }
                "max" => {
                    let gt = b!(self.bld.build_int_compare(
                        inkwell::IntPredicate::SGT,
                        field_val,
                        cur_acc,
                        "agg.gt"
                    ));
                    b!(self
                        .bld
                        .build_select::<inkwell::values::IntValue, inkwell::values::IntValue>(
                            gt,
                            field_val.into(),
                            cur_acc.into(),
                            "agg.max"
                        ))
                    .into_int_value()
                }
                _ => cur_acc,
            };
            b!(self.bld.build_store(acc_ptr, new_acc));
        }

        // Increment count for avg
        if op == "avg" {
            let cur_cnt = b!(self.bld.build_load(i64t, cnt_ptr, "agg.ccnt")).into_int_value();
            let new_cnt = b!(self
                .bld
                .build_int_add(cur_cnt, i64t.const_int(1, false), "agg.cinc"));
            b!(self.bld.build_store(cnt_ptr, new_cnt));
        }

        b!(self.bld.build_unconditional_branch(next_bb));

        // Next iteration
        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "agg.next_i"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        // Done
        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));

        if is_float {
            let result = b!(self.bld.build_load(f64t, acc_ptr, "agg.fresult")).into_float_value();
            if op == "avg" {
                let cnt = b!(self.bld.build_load(i64t, cnt_ptr, "agg.fcnt")).into_int_value();
                let cnt_f = b!(self.bld.build_signed_int_to_float(cnt, f64t, "agg.cf"));
                let avg = b!(self.bld.build_float_div(result, cnt_f, "agg.favg"));
                Ok(avg.into())
            } else {
                Ok(result.into())
            }
        } else {
            let result = b!(self.bld.build_load(i64t, acc_ptr, "agg.result")).into_int_value();
            if op == "avg" {
                let cnt = b!(self.bld.build_load(i64t, cnt_ptr, "agg.fcnt")).into_int_value();
                let sum_f = b!(self.bld.build_signed_int_to_float(result, f64t, "agg.sf"));
                let cnt_f = b!(self.bld.build_signed_int_to_float(cnt, f64t, "agg.cf"));
                let avg = b!(self.bld.build_float_div(sum_f, cnt_f, "agg.avg"));
                Ok(avg.into())
            } else {
                Ok(result.into())
            }
        }
    }

    /// Emit version_count(sid) for a @versioned store.
    pub(in crate::codegen) fn emit_store_version_count(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("version_count() requires sid argument".into());
        }
        let sid_val = self.val(args[0]).into_int_value();
        let (sd, _st, rec_size, _fp) = self.setup_store_access(store_name)?;
        let i64t = self.ctx.i64_type();

        let ver_fp = self.load_store_ver(store_name)?;
        let ver_count_fn = crate::codegen::fn_or_die(&self.module, "jade_ver_count");
        let count = self
            .call_result(b!(self.bld.build_call(
                ver_count_fn,
                &[
                    ver_fp.into(),
                    sid_val.into(),
                    i64t.const_int(rec_size, false).into()
                ],
                "ver.cnt"
            )))
            .into_int_value();
        // Add 1 for the current version in the main store
        let _ = sd; // used for setup_store_access
        let total = b!(self
            .bld
            .build_int_add(count, i64t.const_int(1, false), "ver.total"));
        Ok(total.into())
    }

    /// Emit at_version(sid, version) for a @versioned store.
    /// Returns 1 if found, 0 if not. Side effect: prints/logs the record.
    /// For now: returns 1/0 (found/not-found) as i64.
    pub(in crate::codegen) fn emit_store_at_version(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.len() < 2 {
            return Err("at_version() requires (sid, version) arguments".into());
        }
        let sid_val = self.val(args[0]).into_int_value();
        let ver_val = self.val(args[1]).into_int_value();
        let (_sd, _st, rec_size, _fp) = self.setup_store_access(store_name)?;
        let i64t = self.ctx.i64_type();
        let _ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

        let ver_fp = self.load_store_ver(store_name)?;

        // Allocate buffer for the record
        let malloc_fn = self.ensure_malloc();
        let out_buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[i64t.const_int(rec_size, false).into()],
                "ver.buf"
            )))
            .into_pointer_value();

        let ver_at_fn = crate::codegen::fn_or_die(&self.module, "jade_ver_at");
        let found = self
            .call_result(b!(self.bld.build_call(
                ver_at_fn,
                &[
                    ver_fp.into(),
                    sid_val.into(),
                    ver_val.into(),
                    out_buf.into(),
                    i64t.const_int(rec_size, false).into()
                ],
                "ver.found"
            )))
            .into_int_value();

        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[out_buf.into()], ""));

        Ok(found.into())
    }
}

impl<'ctx> Compiler<'ctx> {
    /// Emit history(sid) for a @versioned store.
    /// Returns the number of version entries found.
    pub(in crate::codegen) fn emit_store_history(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("history() requires sid argument".into());
        }
        let sid_val = self.val(args[0]).into_int_value();
        let (_sd, _st, rec_size, _fp) = self.setup_store_access(store_name)?;
        let i64t = self.ctx.i64_type();

        let ver_fp = self.load_store_ver(store_name)?;
        let ver_count_fn = crate::codegen::fn_or_die(&self.module, "jade_ver_count");
        let count = self
            .call_result(b!(self.bld.build_call(
                ver_count_fn,
                &[
                    ver_fp.into(),
                    sid_val.into(),
                    i64t.const_int(rec_size, false).into()
                ],
                "hist.cnt"
            )))
            .into_int_value();
        Ok(count.into())
    }
}
