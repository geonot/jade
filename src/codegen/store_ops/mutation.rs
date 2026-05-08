//! High-level store delete and set HIR codegen.

use super::*;

impl<'ctx> Compiler<'ctx> {
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
        let st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        let rec_size = self.store_record_size(sd);

        let (fi, ft, fval, extras) = self.precompile_filter_values(filter, sd)?;

        let count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, count, rec_size)?;
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");

        // Rewind and truncate the file in-place (keeps the lock held)
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(0, false).into(),
                i32t.const_int(0, false).into() // SEEK_SET
            ],
            ""
        ));
        let fileno_fn = crate::codegen::fn_or_die(&self.module, "fileno");
        let fd = self
            .call_result(b!(self.bld.build_call(fileno_fn, &[fp.into()], "del.fd")))
            .into_int_value();
        // Declare ftruncate if needed
        let ftruncate_fn = self.module.get_function("ftruncate").unwrap_or_else(|| {
            let ft = i32t.fn_type(&[i32t.into(), i64t.into()], false);
            self.module
                .add_function("ftruncate", ft, Some(inkwell::module::Linkage::External))
        });
        b!(self.bld.build_call(
            ftruncate_fn,
            &[fd.into(), i64t.const_int(0, false).into()],
            ""
        ));
        let new_fp = fp;

        let fwrite_fn = crate::codegen::fn_or_die(&self.module, "fwrite");
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

        let fv_fn = self.current_fn();
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

        let fflush_fn = crate::codegen::fn_or_die(&self.module, "fflush");
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
        let st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        let rec_size = self.store_record_size(sd);

        let mut assign_vals = Vec::new();
        for (fname, fexpr) in assignments {
            let idx = sd.fields.iter().position(|f| f.name == *fname).unwrap();
            let val = self.compile_expr(fexpr)?;
            assign_vals.push((idx, fname.clone(), val));
        }

        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");

        let count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, count, rec_size)?;

        let (fi, ft, fval, extras) = self.precompile_filter_values(filter, sd)?;
        let fv = self.current_fn();
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
        let fwrite_fn = crate::codegen::fn_or_die(&self.module, "fwrite");
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

        let fflush_fn = crate::codegen::fn_or_die(&self.module, "fflush");
        b!(self.bld.build_call(fflush_fn, &[fp.into()], ""));

        self.store_unlock(fp)?;
        Ok(())
    }
}
