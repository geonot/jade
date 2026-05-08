//! Store/view read-all MIR codegen.

use super::*;

impl<'ctx> Compiler<'ctx> {
    /// Emit a view all: iterate source store, collect all records matching filter.
    pub(in crate::codegen) fn emit_view_all(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = encoded_name.splitn(3, "__").collect();
        if parts.len() < 3 || args.is_empty() {
            return Ok(self
                .ctx
                .ptr_type(inkwell::AddressSpace::default())
                .const_null()
                .into());
        }
        let store_name = parts[0];
        let field_name = parts[1];
        let remainder = parts[2];
        let segments: Vec<&str> = remainder.split("__").collect();
        let op = Self::parse_store_op(segments[0]);

        let mut extra_specs: Vec<(crate::ast::LogicalOp, &str, crate::ast::BinOp)> = Vec::new();
        let mut i = 1;
        while i + 2 < segments.len() {
            let lop = match segments[i] {
                "and" => crate::ast::LogicalOp::And,
                "or" => crate::ast::LogicalOp::Or,
                _ => {
                    i += 1;
                    continue;
                }
            };
            let efield = segments[i + 1];
            let eop = Self::parse_store_op(segments[i + 2]);
            extra_specs.push((lop, efield, eop));
            i += 3;
        }

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

        let rec_name = format!("__store_{store_name}_rec");
        let rec_st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        let rec_size = self.store_record_size(&sd);

        let jinn_name = format!("__store_{store_name}");
        let jinn_st = self
            .module
            .get_struct_type(&jinn_name)
            .expect("ICE: struct type not declared");
        let jinn_size = self.type_store_size(jinn_st.into());

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.value_map[&args[0]];

        let count = self.store_read_count(fp)?;
        let raw_buf = self.store_load_records(fp, count, rec_size)?;

        // Allocate max-capacity output buffer (worst case all records match)
        let one = i64t.const_int(1, false);
        let jinn_total =
            b!(self
                .bld
                .build_int_mul(count, i64t.const_int(jinn_size, false), "va.total"));
        let jinn_alloc = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                jinn_total,
                i64t.const_int(0, false),
                "va.isz"
            )),
            one,
            jinn_total,
            "va.alloc"
        ))
        .into_int_value();
        let malloc_fn = self.ensure_malloc();
        let jinn_buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[jinn_alloc.into()],
                "va.buf"
            )))
            .into_pointer_value();

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let idx_ptr = self.entry_alloca(i64t.into(), "va.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        let out_ptr = self.entry_alloca(i64t.into(), "va.out");
        b!(self.bld.build_store(out_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "va.loop");
        let body_bb = self.ctx.append_basic_block(fv, "va.body");
        let copy_bb = self.ctx.append_basic_block(fv, "va.copy");
        let next_bb = self.ctx.append_basic_block(fv, "va.next");
        let done_bb = self.ctx.append_basic_block(fv, "va.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "va.i")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "va.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let raw_off = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "va.roff"));
        let raw_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), raw_buf, &[raw_off], "va.rptr"))
        };

        // Skip soft-deleted records
        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep = b!(self
                .bld
                .build_struct_gep(rec_st, raw_ptr, del_idx as u32, "va.del"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "va.del.val")).into_int_value();
            let is_deleted = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "va.is_del"
            ));
            let filter_bb = self.ctx.append_basic_block(fv, "va.filter");
            b!(self
                .bld
                .build_conditional_branch(is_deleted, next_bb, filter_bb));
            self.bld.position_at_end(filter_bb);
        }

        // Apply filter
        let extras: Vec<(
            crate::ast::LogicalOp,
            usize,
            Type,
            crate::ast::BinOp,
            BasicValueEnum<'ctx>,
        )> = extra_specs
            .iter()
            .enumerate()
            .map(|(ei, (lop, efield, eop))| {
                let (eidx, ety) = sd
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .unwrap();
                let eval = self.value_map[&args[ei + 1]];
                (*lop, eidx, ety, *eop, eval)
            })
            .collect();
        let cond = self.eval_store_filter(
            raw_ptr, rec_st, field_idx, &field_ty, op, filter_val, &extras,
        )?;
        b!(self.bld.build_conditional_branch(cond, copy_bb, next_bb));

        // Copy matching record
        self.bld.position_at_end(copy_bb);
        let out_idx = b!(self.bld.build_load(i64t, out_ptr, "va.oi")).into_int_value();
        let jinn_val = self.load_store_record_as_jinn(rec_st, raw_ptr, &sd)?;
        let jinn_off =
            b!(self
                .bld
                .build_int_mul(out_idx, i64t.const_int(jinn_size, false), "va.joff"));
        let jinn_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), jinn_buf, &[jinn_off], "va.jptr"))
        };
        b!(self.bld.build_store(jinn_ptr, jinn_val));
        let next_out = b!(self
            .bld
            .build_int_add(out_idx, i64t.const_int(1, false), "va.oinc"));
        b!(self.bld.build_store(out_ptr, next_out));
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_i = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "va.next"));
        b!(self.bld.build_store(idx_ptr, next_i));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[raw_buf.into()], ""));
        let final_count = b!(self.bld.build_load(i64t, out_ptr, "va.count")).into_int_value();

        // Build vec header: {ptr, len, cap}
        let vec_ty = self.ctx.struct_type(
            &[
                self.ctx.ptr_type(inkwell::AddressSpace::default()).into(),
                i64t.into(),
                i64t.into(),
            ],
            false,
        );
        let vec_ptr = self.entry_alloca(vec_ty.into(), "va.vec");
        let ptr_gep = b!(self.bld.build_struct_gep(vec_ty, vec_ptr, 0, "va.vec.ptr"));
        b!(self.bld.build_store(ptr_gep, jinn_buf));
        let len_gep = b!(self.bld.build_struct_gep(vec_ty, vec_ptr, 1, "va.vec.len"));
        b!(self.bld.build_store(len_gep, final_count));
        let cap_gep = b!(self.bld.build_struct_gep(vec_ty, vec_ptr, 2, "va.vec.cap"));
        b!(self.bld.build_store(cap_gep, count));

        Ok(b!(self.bld.build_load(vec_ty, vec_ptr, "va.result")))
    }

    pub(in crate::codegen) fn emit_store_all(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
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

        let rec_name = format!("__store_{store_name}_rec");
        let rec_st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        let rec_size = self.store_record_size(&sd);

        let jinn_name = format!("__store_{store_name}");
        let jinn_st = self
            .module
            .get_struct_type(&jinn_name)
            .expect("ICE: struct type not declared");
        let jinn_size = self.type_store_size(jinn_st.into());

        let count = self.store_read_count(fp)?;
        let raw_buf = self.store_load_records(fp, count, rec_size)?;

        let jinn_total =
            b!(self
                .bld
                .build_int_mul(count, i64t.const_int(jinn_size, false), "all.jinn_total"));
        let one = i64t.const_int(1, false);
        let jinn_alloc = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                jinn_total,
                i64t.const_int(0, false),
                "all.jinn_isz"
            )),
            one,
            jinn_total,
            "all.jinn_alloc"
        ))
        .into_int_value();
        let malloc_fn = self.ensure_malloc();
        let jinn_buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[jinn_alloc.into()],
                "all.jn"
            )))
            .into_pointer_value();

        let has_strings = sd.fields.iter().any(|f| matches!(f.ty, Type::String));
        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted");

        if has_strings || deleted_idx.is_some() {
            // Need a loop: either for string conversion or soft-delete filtering (or both)
            let fv = self.cur_fn.expect("ICE: cur_fn not set");
            let idx_ptr = self.entry_alloca(i64t.into(), "all.idx");
            b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

            // Separate output counter for soft-delete filtering
            let out_ptr = self.entry_alloca(i64t.into(), "all.out");
            b!(self.bld.build_store(out_ptr, i64t.const_int(0, false)));

            let loop_bb = self.ctx.append_basic_block(fv, "all.loop");
            let body_bb = self.ctx.append_basic_block(fv, "all.body");
            let copy_bb = self.ctx.append_basic_block(fv, "all.copy");
            let next_bb = self.ctx.append_basic_block(fv, "all.next");
            let done_bb = self.ctx.append_basic_block(fv, "all.done");

            b!(self.bld.build_unconditional_branch(loop_bb));
            self.bld.position_at_end(loop_bb);
            let idx = b!(self.bld.build_load(i64t, idx_ptr, "all.i")).into_int_value();
            let cmp =
                b!(self
                    .bld
                    .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "all.cmp"));
            b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

            self.bld.position_at_end(body_bb);
            let raw_off =
                b!(self
                    .bld
                    .build_int_mul(idx, i64t.const_int(rec_size, false), "all.roff"));
            let raw_ptr = unsafe {
                b!(self
                    .bld
                    .build_gep(self.ctx.i8_type(), raw_buf, &[raw_off], "all.rptr"))
            };

            // Skip soft-deleted records
            if let Some(del_idx) = deleted_idx {
                let del_gep =
                    b!(self
                        .bld
                        .build_struct_gep(rec_st, raw_ptr, del_idx as u32, "all.del"));
                let del_val =
                    b!(self.bld.build_load(i64t, del_gep, "all.del.val")).into_int_value();
                let is_deleted = b!(self.bld.build_int_compare(
                    inkwell::IntPredicate::NE,
                    del_val,
                    i64t.const_int(0, false),
                    "all.is_del"
                ));
                b!(self
                    .bld
                    .build_conditional_branch(is_deleted, next_bb, copy_bb));
            } else {
                b!(self.bld.build_unconditional_branch(copy_bb));
            }

            self.bld.position_at_end(copy_bb);
            let out_idx = b!(self.bld.build_load(i64t, out_ptr, "all.oi")).into_int_value();

            if has_strings {
                let jinn_val = self.load_store_record_as_jinn(rec_st, raw_ptr, &sd)?;
                let jinn_off = b!(self.bld.build_int_mul(
                    out_idx,
                    i64t.const_int(jinn_size, false),
                    "all.joff"
                ));
                let jinn_ptr = unsafe {
                    b!(self
                        .bld
                        .build_gep(self.ctx.i8_type(), jinn_buf, &[jinn_off], "all.jptr"))
                };
                b!(self.bld.build_store(jinn_ptr, jinn_val));
            } else {
                let src_off =
                    b!(self
                        .bld
                        .build_int_mul(idx, i64t.const_int(rec_size, false), "all.soff"));
                let src_ptr = unsafe {
                    b!(self
                        .bld
                        .build_gep(self.ctx.i8_type(), raw_buf, &[src_off], "all.src"))
                };
                let dst_off = b!(self.bld.build_int_mul(
                    out_idx,
                    i64t.const_int(rec_size, false),
                    "all.doff"
                ));
                let dst_ptr = unsafe {
                    b!(self
                        .bld
                        .build_gep(self.ctx.i8_type(), jinn_buf, &[dst_off], "all.dst"))
                };
                let memcpy_fn = self.ensure_memcpy();
                b!(self.bld.build_call(
                    memcpy_fn,
                    &[
                        dst_ptr.into(),
                        src_ptr.into(),
                        i64t.const_int(rec_size, false).into()
                    ],
                    ""
                ));
            }

            let next_out =
                b!(self
                    .bld
                    .build_int_add(out_idx, i64t.const_int(1, false), "all.onext"));
            b!(self.bld.build_store(out_ptr, next_out));
            b!(self.bld.build_unconditional_branch(next_bb));

            self.bld.position_at_end(next_bb);
            let next_idx = b!(self
                .bld
                .build_int_add(idx, i64t.const_int(1, false), "all.next"));
            b!(self.bld.build_store(idx_ptr, next_idx));
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(done_bb);
        } else {
            // Simple store, no strings, no deleted field: memcpy
            let total =
                b!(self
                    .bld
                    .build_int_mul(count, i64t.const_int(rec_size, false), "all.total"));
            let memcpy_fn = self.ensure_memcpy();
            b!(self.bld.build_call(
                memcpy_fn,
                &[jinn_buf.into(), raw_buf.into(), total.into()],
                ""
            ));
        }

        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[raw_buf.into()], ""));

        Ok(jinn_buf.into())
    }
}
