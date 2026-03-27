use inkwell::values::BasicValueEnum;
use inkwell::IntPredicate;

use crate::hir;
use crate::types::Type;

use super::b;
use super::Compiler;

use super::stores::HEADER_SIZE;

impl<'ctx> Compiler<'ctx> {
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
        let memset_fn = self.module.get_function("memset").unwrap();
        b!(self.bld.build_call(
            memset_fn,
            &[
                rec_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(rec_size, false).into(),
            ],
            ""
        ));

        for (i, (field_def, val_expr)) in sd.fields.iter().zip(values.iter()).enumerate() {
            let gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, i as u32, &field_def.name));
            match &field_def.ty {
                Type::String => {
                    let val = self.compile_expr(val_expr)?;
                    self.copy_string_to_fixed_buf(val, gep)?;
                }
                _ => {
                    let val = self.compile_expr(val_expr)?;
                    b!(self.bld.build_store(gep, val));
                }
            }
        }

        let fseek_fn = self.module.get_function("fseek").unwrap();
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(0, false).into(),
                i32t.const_int(2, false).into(),
            ],
            ""
        ));

        let fwrite_fn = self.module.get_function("fwrite").unwrap();
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
        let fread_fn = self.module.get_function("fread").unwrap();
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

        let old_count = b!(self.bld.build_load(i64t, count_buf, "old.count"));
        let new_count = b!(self.bld.build_int_add(
            old_count.into_int_value(),
            i64t.const_int(1, false),
            "new.count"
        ));
        b!(self.bld.build_store(count_buf, new_count));

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

        let fflush_fn = self.module.get_function("fflush").unwrap();
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
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let fseek_fn = self.module.get_function("fseek").unwrap();
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));

        let count_buf = self.entry_alloca(i64t.into(), "store.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = self.module.get_function("fread").unwrap();
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

        let count = b!(self.bld.build_load(i64t, count_buf, "count"));
        Ok(count)
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
        let st = self.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.store_record_size(sd);

        let (fi, ft, fv, extras) = self.precompile_filter_values(filter, sd)?;

        let fseek_fn = self.module.get_function("fseek").unwrap();
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let count_buf = self.entry_alloca(i64t.into(), "q.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = self.module.get_function("fread").unwrap();
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
        let count = b!(self.bld.build_load(i64t, count_buf, "count")).into_int_value();

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(HEADER_SIZE, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));

        let total = b!(self
            .bld
            .build_int_mul(count, i64t.const_int(rec_size, false), "q.total"));
        let one = i64t.const_int(1, false);
        let alloc_size = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(
                IntPredicate::EQ,
                total,
                i64t.const_int(0, false),
                "q.isz"
            )),
            one,
            total,
            "q.alloc"
        ))
        .into_int_value();
        let malloc_fn = self.ensure_malloc();
        let buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[alloc_size.into()],
                "q.buf"
            )))
            .into_pointer_value();
        b!(self.bld.build_call(
            fread_fn,
            &[
                buf.into(),
                i64t.const_int(rec_size, false).into(),
                count.into(),
                fp.into()
            ],
            ""
        ));

        let result_ptr = self.entry_alloca(st.into(), "q.result");
        let memset_fn = self.module.get_function("memset").unwrap();
        b!(self.bld.build_call(
            memset_fn,
            &[
                result_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(rec_size, false).into()
            ],
            ""
        ));

        let fv_fn = self.cur_fn.unwrap();
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
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "q.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "q.rec"))
        };

        let cond = self.eval_store_filter(rec_ptr, st, fi, &ft, filter.op, fv, &extras)?;
        b!(self.bld.build_conditional_branch(cond, match_bb, next_bb));

        self.bld.position_at_end(match_bb);
        let memcpy_fn = self.ensure_memcpy();
        b!(self.bld.build_call(
            memcpy_fn,
            &[
                result_ptr.into(),
                rec_ptr.into(),
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
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        let result = self.load_store_record_as_jade(st, result_ptr, sd)?;
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
        let i32t = self.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let rec_st = self.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.store_record_size(sd);

        let jade_name = format!("__store_{store_name}");
        let jade_st = self.module.get_struct_type(&jade_name).unwrap();
        let jade_size = self.type_store_size(jade_st.into());

        let fseek_fn = self.module.get_function("fseek").unwrap();
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let count_buf = self.entry_alloca(i64t.into(), "all.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = self.module.get_function("fread").unwrap();
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
        let count = b!(self.bld.build_load(i64t, count_buf, "count")).into_int_value();

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(HEADER_SIZE, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));

        let raw_total =
            b!(self
                .bld
                .build_int_mul(count, i64t.const_int(rec_size, false), "all.raw_total"));
        let jade_total =
            b!(self
                .bld
                .build_int_mul(count, i64t.const_int(jade_size, false), "all.jade_total"));
        let malloc_fn = self.ensure_malloc();
        let raw_buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[raw_total.into()],
                "all.raw"
            )))
            .into_pointer_value();
        let jade_buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[jade_total.into()],
                "all.jade"
            )))
            .into_pointer_value();

        b!(self.bld.build_call(
            fread_fn,
            &[
                raw_buf.into(),
                i64t.const_int(rec_size, false).into(),
                count.into(),
                fp.into()
            ],
            ""
        ));

        let has_strings = sd.fields.iter().any(|f| matches!(f.ty, Type::String));

        if has_strings {
            let fv = self.cur_fn.unwrap();
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
            let jade_val = self.load_store_record_as_jade(rec_st, raw_ptr, sd)?;
            let jade_off =
                b!(self
                    .bld
                    .build_int_mul(idx, i64t.const_int(jade_size, false), "all.joff"));
            let jade_ptr = unsafe {
                b!(self
                    .bld
                    .build_gep(self.ctx.i8_type(), jade_buf, &[jade_off], "all.jptr"))
            };
            b!(self.bld.build_store(jade_ptr, jade_val));

            let next_idx = b!(self
                .bld
                .build_int_add(idx, i64t.const_int(1, false), "all.next"));
            b!(self.bld.build_store(idx_ptr, next_idx));
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(done_bb);
        } else {
            let memcpy_fn = self.ensure_memcpy();
            b!(self.bld.build_call(
                memcpy_fn,
                &[jade_buf.into(), raw_buf.into(), raw_total.into()],
                ""
            ));
        }

        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[raw_buf.into()], ""));

        Ok(jade_buf.into())
    }

    pub(crate) fn compile_store_delete(
        &mut self,
        store_name: &str,
        filter: &hir::StoreFilter,
        sd: &hir::StoreDef,
    ) -> Result<(), String> {
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));

        let fp = self.load_store_fp(store_name)?;
        self.store_lock(fp)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.store_record_size(sd);

        let (fi, ft, fval, extras) = self.precompile_filter_values(filter, sd)?;

        let fseek_fn = self.module.get_function("fseek").unwrap();
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let count_buf = self.entry_alloca(i64t.into(), "del.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = self.module.get_function("fread").unwrap();
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
        let count = b!(self.bld.build_load(i64t, count_buf, "count")).into_int_value();

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(HEADER_SIZE, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));

        let total = b!(self
            .bld
            .build_int_mul(count, i64t.const_int(rec_size, false), "del.total"));
        let malloc_fn = self.ensure_malloc();
        let buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[total.into()],
                "del.buf"
            )))
            .into_pointer_value();

        b!(self.bld.build_call(
            fread_fn,
            &[
                buf.into(),
                i64t.const_int(rec_size, false).into(),
                count.into(),
                fp.into()
            ],
            ""
        ));

        let fclose_fn = self.module.get_function("fclose").unwrap();
        b!(self.bld.build_call(fclose_fn, &[fp.into()], ""));

        let filename = format!("{store_name}.store\0");
        let file_str = b!(self.bld.build_global_string_ptr(&filename, "del.path"));
        let mode_wb = b!(self.bld.build_global_string_ptr("w+b\0", "del.mode"));
        let fopen_fn = self.module.get_function("fopen").unwrap();
        let new_fp = self
            .call_result(b!(self.bld.build_call(
                fopen_fn,
                &[
                    file_str.as_pointer_value().into(),
                    mode_wb.as_pointer_value().into()
                ],
                "del.fp"
            )))
            .into_pointer_value();

        let global_name = format!("__store_{store_name}_fp");
        let global = self.module.get_global(&global_name).unwrap();
        b!(self.bld.build_store(global.as_pointer_value(), new_fp));

        let fwrite_fn = self.module.get_function("fwrite").unwrap();
        let magic = b!(self.bld.build_global_string_ptr("JADESTR\0", "del.magic"));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                magic.as_pointer_value().into(),
                i64t.const_int(1, false).into(),
                i64t.const_int(8, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let new_count_ptr = self.entry_alloca(i64t.into(), "del.newcount");
        b!(self
            .bld
            .build_store(new_count_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                new_count_ptr.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let rec_size_ptr = self.entry_alloca(i64t.into(), "del.recsz");
        b!(self
            .bld
            .build_store(rec_size_ptr, i64t.const_int(rec_size, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                rec_size_ptr.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let fv_fn = self.cur_fn.unwrap();
        let idx_ptr = self.entry_alloca(i64t.into(), "del.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv_fn, "del.loop");
        let body_bb = self.ctx.append_basic_block(fv_fn, "del.body");
        let keep_bb = self.ctx.append_basic_block(fv_fn, "del.keep");
        let skip_bb = self.ctx.append_basic_block(fv_fn, "del.skip");
        let done_bb = self.ctx.append_basic_block(fv_fn, "del.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "del.i")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(IntPredicate::ULT, idx, count, "del.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "del.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "del.rec"))
        };

        let matches = self.eval_store_filter(rec_ptr, st, fi, &ft, filter.op, fval, &extras)?;
        b!(self.bld.build_conditional_branch(matches, skip_bb, keep_bb));

        self.bld.position_at_end(keep_bb);
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                rec_ptr.into(),
                i64t.const_int(rec_size, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));
        let kept = b!(self.bld.build_load(i64t, new_count_ptr, "kept")).into_int_value();
        let kept_inc = b!(self
            .bld
            .build_int_add(kept, i64t.const_int(1, false), "kept.inc"));
        b!(self.bld.build_store(new_count_ptr, kept_inc));
        b!(self.bld.build_unconditional_branch(skip_bb));

        self.bld.position_at_end(skip_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "del.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        b!(self.bld.build_call(
            fseek_fn,
            &[
                new_fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                new_count_ptr.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));

        let fflush_fn = self.module.get_function("fflush").unwrap();
        b!(self.bld.build_call(fflush_fn, &[new_fp.into()], ""));

        self.store_unlock(fp)?;
        Ok(())
    }

    pub(crate) fn compile_store_set(
        &mut self,
        store_name: &str,
        assignments: &[(String, hir::Expr)],
        filter: &hir::StoreFilter,
        sd: &hir::StoreDef,
    ) -> Result<(), String> {
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));

        let fp = self.load_store_fp(store_name)?;
        self.store_lock(fp)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.store_record_size(sd);

        let mut assign_vals = Vec::new();
        for (fname, fexpr) in assignments {
            let idx = sd.fields.iter().position(|f| f.name == *fname).unwrap();
            let val = self.compile_expr(fexpr)?;
            assign_vals.push((idx, fname.clone(), val));
        }

        let fseek_fn = self.module.get_function("fseek").unwrap();
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let count_buf = self.entry_alloca(i64t.into(), "set.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = self.module.get_function("fread").unwrap();
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
        let count = b!(self.bld.build_load(i64t, count_buf, "count")).into_int_value();

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(HEADER_SIZE, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));

        let total = b!(self
            .bld
            .build_int_mul(count, i64t.const_int(rec_size, false), "set.total"));
        let malloc_fn = self.ensure_malloc();
        let buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[total.into()],
                "set.buf"
            )))
            .into_pointer_value();
        b!(self.bld.build_call(
            fread_fn,
            &[
                buf.into(),
                i64t.const_int(rec_size, false).into(),
                count.into(),
                fp.into()
            ],
            ""
        ));

        let (fi, ft, fval, extras) = self.precompile_filter_values(filter, sd)?;
        let fv = self.cur_fn.unwrap();
        let idx_ptr = self.entry_alloca(i64t.into(), "set.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "set.loop");
        let body_bb = self.ctx.append_basic_block(fv, "set.body");
        let update_bb = self.ctx.append_basic_block(fv, "set.update");
        let next_bb = self.ctx.append_basic_block(fv, "set.next");
        let done_bb = self.ctx.append_basic_block(fv, "set.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "set.i")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(IntPredicate::ULT, idx, count, "set.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "set.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "set.rec"))
        };
        let matches = self.eval_store_filter(rec_ptr, st, fi, &ft, filter.op, fval, &extras)?;
        b!(self
            .bld
            .build_conditional_branch(matches, update_bb, next_bb));

        self.bld.position_at_end(update_bb);
        for (field_idx, _fname, val) in &assign_vals {
            let fty = &sd.fields[*field_idx].ty;
            let gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, *field_idx as u32, "set.assign"));
            match fty {
                Type::String => {
                    self.copy_string_to_fixed_buf(*val, gep)?;
                }
                _ => {
                    b!(self.bld.build_store(gep, *val));
                }
            }
        }
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "set.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(HEADER_SIZE, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let fwrite_fn = self.module.get_function("fwrite").unwrap();
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                buf.into(),
                i64t.const_int(rec_size, false).into(),
                count.into(),
                fp.into()
            ],
            ""
        ));

        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));

        let fflush_fn = self.module.get_function("fflush").unwrap();
        b!(self.bld.build_call(fflush_fn, &[fp.into()], ""));

        self.store_unlock(fp)?;
        Ok(())
    }
}
