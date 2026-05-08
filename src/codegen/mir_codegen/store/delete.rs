//! Soft and hard store delete MIR codegen.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_store_delete(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // For non-@simple stores: soft-delete by setting deleted timestamp
        // For @simple stores: fall back to hard delete
        let (store_name, _, _, _) = Self::parse_encoded_filter(encoded_name)?;
        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();
        let is_simple = sd
            .decorators
            .iter()
            .any(|d| *d == crate::ast::StoreDecorator::Simple);

        if is_simple || sd.fields.iter().all(|f| f.name != "deleted") {
            return self.emit_store_hard_delete(encoded_name, args);
        }

        // Soft delete: set deleted = time() on matching records
        let (store_name, field_name, op, extra_specs) = Self::parse_encoded_filter(encoded_name)?;
        if args.is_empty() {
            return Ok(self.ctx.i64_type().const_int(0, false).into());
        }
        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        self.store_lock(fp)?;

        // @before_delete hook
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::BeforeDelete(fname) = dec {
                if let Some(hook_fn) = self.module.get_function(&fname.as_str()) {
                    b!(self.bld.build_call(hook_fn, &[], ""));
                }
            }
        }

        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted").unwrap();

        let filter_val = self.val(args[0]);

        let count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, count, rec_size)?;

        // Get current time for the deleted timestamp
        self.ensure_time_fn();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let time_fn = crate::codegen::fn_or_die(&self.module, "time");
        let now = self
            .call_result(b!(self.bld.build_call(
                time_fn,
                &[ptr_ty.const_null().into()],
                "del.now"
            )))
            .into_int_value();

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let idx_ptr = self.entry_alloca(i64t.into(), "sdel.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "sdel.loop");
        let body_bb = self.ctx.append_basic_block(fv, "sdel.body");
        let mark_bb = self.ctx.append_basic_block(fv, "sdel.mark");
        let next_bb = self.ctx.append_basic_block(fv, "sdel.next");
        let done_bb = self.ctx.append_basic_block(fv, "sdel.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "sdel.i")).into_int_value();
        let cmp =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "sdel.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "sdel.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "sdel.rec"))
        };
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
                let (fi, ft) = sd
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .unwrap_or((0, Type::I64));
                let ev = self.val(args[1 + ei]);
                (*lop, fi, ft, *eop, ev)
            })
            .collect();
        let cond =
            self.eval_store_filter(rec_ptr, st, field_idx, &field_ty, op, filter_val, &extras)?;
        b!(self.bld.build_conditional_branch(cond, mark_bb, next_bb));

        self.bld.position_at_end(mark_bb);
        let del_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, deleted_idx as u32, "sdel.del"));
        b!(self.bld.build_store(del_gep, now));
        // WAL: log the soft-delete
        self.wal_write_delete(store_name, rec_ptr, rec_size)?;
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "sdel.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        // Write updated records back
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");

        // @after_delete hook
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::AfterDelete(fname) = dec {
                if let Some(hook_fn) = self.module.get_function(&fname.as_str()) {
                    b!(self.bld.build_call(hook_fn, &[], ""));
                }
            }
        }

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(crate::codegen::stores::HEADER_SIZE, false)
                    .into(),
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

        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn emit_store_hard_delete(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (store_name, field_name, primary_op, extra_conds) =
            Self::parse_encoded_filter(encoded_name)?;
        if args.is_empty() {
            return Ok(self.ctx.i64_type().const_int(0, false).into());
        }

        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        self.store_lock(fp)?;

        // @before_delete hook
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::BeforeDelete(fname) = dec {
                if let Some(hook_fn) = self.module.get_function(&fname.as_str()) {
                    b!(self.bld.build_call(hook_fn, &[], ""));
                }
            }
        }

        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.val(args[0]);

        let count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, count, rec_size)?;

        // Rewrite file: close and reopen in w+b mode
        let fclose_fn = crate::codegen::fn_or_die(&self.module, "fclose");
        b!(self.bld.build_call(fclose_fn, &[fp.into()], ""));

        let filename = format!("{store_name}.store\0");
        let file_str = b!(self.bld.build_global_string_ptr(&filename, "del.path"));
        let mode_wb = b!(self.bld.build_global_string_ptr("w+b\0", "del.mode"));
        let fopen_fn = crate::codegen::fn_or_die(&self.module, "fopen");
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

        // Write header: magic + count placeholder + rec_size
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

        // Loop: keep records that DON'T match the filter
        let fv_fn = self.cur_fn.expect("ICE: cur_fn not set");
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
            .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "del.cmp"));
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

        let matches = {
            let extras: Vec<(
                crate::ast::LogicalOp,
                usize,
                Type,
                crate::ast::BinOp,
                BasicValueEnum<'ctx>,
            )> = extra_conds
                .iter()
                .enumerate()
                .map(|(ei, (lop, fname, cop))| {
                    let (fi, ft) = sd
                        .fields
                        .iter()
                        .enumerate()
                        .find(|(_, f)| f.name == *fname)
                        .map(|(i, f)| (i, f.ty.clone()))
                        .unwrap_or((0, Type::I64));
                    let ev = self.val(args[1 + ei]);
                    (*lop, fi, ft, *cop, ev)
                })
                .collect();
            self.eval_store_filter(
                rec_ptr, st, field_idx, &field_ty, primary_op, filter_val, &extras,
            )?
        };
        let del_hook_bb = self.ctx.append_basic_block(fv_fn, "del.hook");
        b!(self
            .bld
            .build_conditional_branch(matches, del_hook_bb, keep_bb));

        // @after_delete hook — only fires for records being deleted
        self.bld.position_at_end(del_hook_bb);
        // WAL: log the hard-delete
        self.wal_write_delete(store_name, rec_ptr, rec_size)?;
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::AfterDelete(fname) = dec {
                if let Some(hook_fn) = self.module.get_function(&fname.as_str()) {
                    b!(self.bld.build_call(hook_fn, &[], ""));
                }
            }
        }
        b!(self.bld.build_unconditional_branch(skip_bb));

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
        // Update count in header
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
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
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }
}
