//! High-level store read, insert, count, query, and all HIR codegen.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn store_read_count(
        &mut self,
        fp: inkwell::values::PointerValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));
        let count_buf = self.entry_alloca(i64t.into(), "sc.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");
        b!(self.bld.build_call(
            fread_fn,
            &[
                count_buf.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));
        Ok(b!(self.bld.build_load(i64t, count_buf, "count")).into_int_value())
    }

    pub(crate) fn store_load_records(
        &mut self,
        fp: inkwell::values::PointerValue<'ctx>,
        count: inkwell::values::IntValue<'ctx>,
        rec_size: u64,
    ) -> Result<inkwell::values::PointerValue<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(HEADER_SIZE, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));
        let total = b!(self
            .bld
            .build_int_mul(count, i64t.const_int(rec_size, false), "sl.total"));
        let one = i64t.const_int(1, false);
        let alloc_size = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(
                IntPredicate::EQ,
                total,
                i64t.const_int(0, false),
                "sl.isz"
            )),
            one,
            total,
            "sl.alloc"
        ))
        .into_int_value();
        let malloc_fn = self.ensure_malloc();
        let buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[alloc_size.into()],
                "sl.buf"
            )))
            .into_pointer_value();
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");
        b!(self.bld.build_call(
            fread_fn,
            &[
                buf.into(),
                i64t.const_int(rec_size, false).into(),
                count.into(),
                fp.into(),
            ],
            ""
        ));
        Ok(buf)
    }

    pub(crate) fn compile_store_insert(
        &mut self,
        store_name: &str,
        values: &[hir::Expr],
        sd: &hir::StoreDef,
    ) -> Result<(), String> {
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));

        let fp = self.load_store_fp(store_name)?;
        self.store_lock(fp)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .module
            .get_struct_type(&rec_name)
            .ok_or_else(|| format!("no store rec struct '{rec_name}'"))?;

        let rec_ptr = self.entry_alloca(st.into(), "store.rec");
        let rec_size = self.store_record_size(sd);
        let memset_fn = crate::codegen::fn_or_die(&self.module, "memset");
        b!(self.bld.build_call(
            memset_fn,
            &[
                rec_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(rec_size, false).into(),
            ],
            ""
        ));

        // Read current count for sid assignment
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");
        let count_buf = self.entry_alloca(i64t.into(), "ins.count");
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        b!(self.bld.build_call(
            fread_fn,
            &[
                count_buf.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into()
            ],
            ""
        ));
        let old_count = b!(self.bld.build_load(i64t, count_buf, "old.count")).into_int_value();
        let new_sid = b!(self
            .bld
            .build_int_add(old_count, i64t.const_int(1, false), "new.sid"));

        // Get current time via time(NULL)
        self.ensure_time_fn();
        let time_fn = crate::codegen::fn_or_die(&self.module, "time");
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let now = self
            .call_result(b!(self.bld.build_call(
                time_fn,
                &[ptr_ty.const_null().into()],
                "now"
            )))
            .into_int_value();

        // Determine which fields are built-in vs user-defined
        let builtin_names = [
            "sid",
            "uuid",
            "hash",
            "created",
            "updated",
            "deleted",
            "__version",
        ];
        let mut user_val_idx = 0usize;
        for (i, field_def) in sd.fields.iter().enumerate() {
            let gep =
                b!(self
                    .bld
                    .build_struct_gep(st, rec_ptr, i as u32, &field_def.name.as_str()));
            if builtin_names.contains(&&*field_def.name.as_str()) {
                // Auto-populate built-in fields
                match &*field_def.name.as_str() {
                    "sid" => {
                        b!(self.bld.build_store(gep, new_sid));
                    }
                    "uuid" => {
                        // Generate a simple UUID-like string from sid + time
                        let uuid_str = self.gen_store_uuid(new_sid, now)?;
                        self.copy_string_to_fixed_buf(uuid_str, gep)?;
                    }
                    "hash" => {
                        // Placeholder empty hash — will be recomputed after all fields set
                        let empty = self.compile_str_literal("")?;
                        self.copy_string_to_fixed_buf(empty, gep)?;
                    }
                    "created" | "updated" => {
                        b!(self.bld.build_store(gep, now));
                    }
                    "deleted" => {
                        b!(self.bld.build_store(gep, i64t.const_int(0, false)));
                    }
                    _ => {}
                }
            } else {
                // User-defined field
                if user_val_idx < values.len() {
                    match &field_def.ty {
                        Type::String => {
                            let val = self.compile_expr(&values[user_val_idx])?;
                            self.copy_string_to_fixed_buf(val, gep)?;
                        }
                        _ => {
                            let val = self.compile_expr(&values[user_val_idx])?;
                            b!(self.bld.build_store(gep, val));
                        }
                    }
                    user_val_idx += 1;
                }
            }
        }

        // R13: amortize file growth in 64KiB chunks via posix ftruncate.
        // ftruncate may extend the file beyond the logical end (count*rec_size
        // + header), so we must seek to the *logical* end based on old_count
        // rather than SEEK_END which would land in the zero-padded reserved
        // region after a prior reserve.
        let reserve_fn = self
            .module
            .get_function("jinn_store_reserve")
            .unwrap_or_else(|| {
                let void_ty = self.ctx.void_type();
                let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                let ft = void_ty.fn_type(&[ptr_ty.into(), i64t.into(), i64t.into()], false);
                self.module.add_function(
                    "jinn_store_reserve",
                    ft,
                    Some(inkwell::module::Linkage::External),
                )
            });
        b!(self.bld.build_call(
            reserve_fn,
            &[
                fp.into(),
                old_count.into(),
                i64t.const_int(rec_size, false).into(),
            ],
            ""
        ));

        // Seek to logical end = 8 + old_count * rec_size.
        let logical_end_off = b!(self.bld.build_int_nsw_mul(
            old_count,
            i64t.const_int(rec_size, false),
            "ins.logoff"
        ));
        let logical_end =
            b!(self
                .bld
                .build_int_nsw_add(logical_end_off, i64t.const_int(8, false), "ins.logend"));
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                logical_end.into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));

        let fwrite_fn = crate::codegen::fn_or_die(&self.module, "fwrite");
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                rec_ptr.into(),
                i64t.const_int(rec_size, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));

        let count_buf = self.entry_alloca(i64t.into(), "count.buf");
        b!(self.bld.build_store(count_buf, new_sid)); // new_sid = old_count + 1 = new count
        let fwrite_fn = crate::codegen::fn_or_die(&self.module, "fwrite");

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                count_buf.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));

        let fflush_fn = crate::codegen::fn_or_die(&self.module, "fflush");
        b!(self.bld.build_call(fflush_fn, &[fp.into()], ""));

        self.store_unlock(fp)?;
        Ok(())
    }

    pub(crate) fn compile_store_count(
        &mut self,
        store_name: &str,
        sd: &hir::StoreDef,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));
        let fp = self.load_store_fp(store_name)?;
        let count = self.store_read_count(fp)?;
        Ok(count.into())
    }

    pub(crate) fn compile_store_query(
        &mut self,
        store_name: &str,
        filter: &hir::StoreFilter,
        sd: &hir::StoreDef,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));

        let fp = self.load_store_fp(store_name)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        let rec_size = self.store_record_size(sd);

        let (fi, ft, fv, extras) = self.precompile_filter_values(filter, sd)?;

        let count = self.store_read_count(fp)?;

        // Seek past header to first record
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(HEADER_SIZE, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));

        // Allocate a single-record buffer for streaming reads
        let malloc_fn = self.ensure_malloc();
        let rec_buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[i64t.const_int(rec_size, false).into()],
                "q.recbuf"
            )))
            .into_pointer_value();

        let result_ptr = self.entry_alloca(st.into(), "q.result");
        let memset_fn = crate::codegen::fn_or_die(&self.module, "memset");
        b!(self.bld.build_call(
            memset_fn,
            &[
                result_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(rec_size, false).into()
            ],
            ""
        ));

        let fv_fn = self.current_fn();
        let idx_ptr = self.entry_alloca(i64t.into(), "q.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv_fn, "q.loop");
        let body_bb = self.ctx.append_basic_block(fv_fn, "q.body");
        let match_bb = self.ctx.append_basic_block(fv_fn, "q.match");
        let next_bb = self.ctx.append_basic_block(fv_fn, "q.next");
        let done_bb = self.ctx.append_basic_block(fv_fn, "q.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "idx")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(IntPredicate::ULT, idx, count, "q.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        // Read one record from file (fread advances file position)
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");
        b!(self.bld.build_call(
            fread_fn,
            &[
                rec_buf.into(),
                i64t.const_int(rec_size, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));

        let cond = self.eval_store_filter(rec_buf, st, fi, &ft, filter.op, fv, &extras)?;
        b!(self.bld.build_conditional_branch(cond, match_bb, next_bb));

        self.bld.position_at_end(match_bb);
        let memcpy_fn = self.ensure_memcpy();
        b!(self.bld.build_call(
            memcpy_fn,
            &[
                result_ptr.into(),
                rec_buf.into(),
                i64t.const_int(rec_size, false).into()
            ],
            ""
        ));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "q.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[rec_buf.into()], ""));
        let result = self.load_store_record_as_jinn(st, result_ptr, sd)?;
        Ok(result)
    }

    pub(crate) fn compile_store_all(
        &mut self,
        store_name: &str,
        sd: &hir::StoreDef,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));

        let fp = self.load_store_fp(store_name)?;
        let i64t = self.ctx.i64_type();

        let rec_name = format!("__store_{store_name}_rec");
        let rec_st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        let rec_size = self.store_record_size(sd);

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
                IntPredicate::EQ,
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

        if has_strings {
            let fv = self.current_fn();
            let idx_ptr = self.entry_alloca(i64t.into(), "all.idx");
            b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

            let loop_bb = self.ctx.append_basic_block(fv, "all.loop");
            let body_bb = self.ctx.append_basic_block(fv, "all.body");
            let done_bb = self.ctx.append_basic_block(fv, "all.done");

            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(loop_bb);
            let idx = b!(self.bld.build_load(i64t, idx_ptr, "all.i")).into_int_value();
            let cmp = b!(self
                .bld
                .build_int_compare(IntPredicate::ULT, idx, count, "all.cmp"));
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
            let jinn_val = self.load_store_record_as_jinn(rec_st, raw_ptr, sd)?;
            let jinn_off =
                b!(self
                    .bld
                    .build_int_mul(idx, i64t.const_int(jinn_size, false), "all.joff"));
            let jinn_ptr = unsafe {
                b!(self
                    .bld
                    .build_gep(self.ctx.i8_type(), jinn_buf, &[jinn_off], "all.jptr"))
            };
            b!(self.bld.build_store(jinn_ptr, jinn_val));

            let next_idx = b!(self
                .bld
                .build_int_add(idx, i64t.const_int(1, false), "all.next"));
            b!(self.bld.build_store(idx_ptr, next_idx));
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(done_bb);
        } else {
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
