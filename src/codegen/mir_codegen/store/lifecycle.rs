use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_store_destroy(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.emit_store_hard_delete(encoded_name, args)
    }

    pub(in crate::codegen) fn emit_store_restore(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (store_name, field_name, op, primary_pred, extra_specs) =
            Self::parse_encoded_filter(encoded_name)?;
        if args.is_empty() {
            return Ok(self.ctx.i64_type().const_int(0, false).into());
        }
        let (sd, st, rec_size, _fp) = self.setup_store_access(store_name)?;
        let fp = self.store_lock(store_name)?;
        self.txn_track_store(store_name, fp)?;
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

        let count = self.store_read_count(fp, rec_size, store_name)?;
        let buf = self.store_load_records(fp, count, rec_size)?;

        let deleted_idx = sd
            .fields
            .iter()
            .position(|f| f.name == "deleted")
            .ok_or_else(|| {
                format!("store '{store_name}' has no 'deleted' field (is it @simple?)")
            })?;

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let idx_ptr = self.entry_alloca(i64t.into(), "restore.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "restore.loop");
        let body_bb = self.ctx.append_basic_block(fv, "restore.body");
        let update_bb = self.ctx.append_basic_block(fv, "restore.update");
        let next_bb = self.ctx.append_basic_block(fv, "restore.next");
        let done_bb = self.ctx.append_basic_block(fv, "restore.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "restore.i")).into_int_value();
        let cmp =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "restore.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset =
            b!(self
                .bld
                .build_int_mul(idx, i64t.const_int(rec_size, false), "restore.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "restore.rec"))
        };

        let del_check_gep =
            b!(self
                .bld
                .build_struct_gep(st, rec_ptr, deleted_idx as u32, "restore.del.chk"));
        let del_check_val =
            b!(self.bld.build_load(i64t, del_check_gep, "restore.del.v")).into_int_value();
        let is_deleted = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            del_check_val,
            i64t.const_int(0, false),
            "restore.is_del"
        ));
        let filter_bb = self.ctx.append_basic_block(fv, "restore.filter");
        b!(self
            .bld
            .build_conditional_branch(is_deleted, filter_bb, next_bb));

        self.bld.position_at_end(filter_bb);
        let extras: Vec<(
            crate::ast::LogicalOp,
            usize,
            Type,
            crate::ast::BinOp,
            crate::ast::FilterPred,
            BasicValueEnum<'ctx>,
        )> = extra_specs
            .iter()
            .enumerate()
            .map(|(ei, (lop, efield, eop, epred))| {
                let (fi, ft) = sd
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .unwrap_or((0, Type::I64));
                let ev = self.val(args[1 + ei]);
                (*lop, fi, ft, *eop, *epred, ev)
            })
            .collect();
        let cond = self.eval_store_filter_pred(
            rec_ptr,
            st,
            field_idx,
            &field_ty,
            op,
            primary_pred,
            filter_val,
            &extras,
        )?;
        b!(self.bld.build_conditional_branch(cond, update_bb, next_bb));

        self.bld.position_at_end(update_bb);

        let del_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, deleted_idx as u32, "restore.del"));
        b!(self.bld.build_store(del_gep, i64t.const_int(0, false)));

        self.wal_write_update(store_name, rec_ptr, rec_size)?;
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "restore.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);

        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
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
        self.store_unlock(store_name, fp)?;

        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn store_field_offset(
        &self,
        sd: &hir::StoreDef,
        fname: &str,
    ) -> Option<u64> {
        let idx = sd.fields.iter().position(|f| f.name.as_str() == fname)?;
        let mut offset = 0u64;
        for f in &sd.fields[..idx] {
            let lty = self.store_field_llvm_ty(&f.ty);
            let fa = self.type_abi_align(lty);
            let fs = self.type_store_size(lty);
            offset = (offset + fa - 1) & !(fa - 1);
            offset += fs;
        }
        let lty = self.store_field_llvm_ty(&sd.fields[idx].ty);
        let align = self.type_abi_align(lty);
        offset = (offset + align - 1) & !(align - 1);
        Some(offset)
    }

    pub(in crate::codegen) fn store_deleted_offset(&self, sd: &hir::StoreDef) -> Option<u64> {
        self.store_field_offset(sd, "deleted")
    }

    pub(in crate::codegen) fn emit_store_compact(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

        let (sd, _st, _rec_size, _fp) = self.setup_store_access(store_name)?;

        let offset = match self.store_deleted_offset(&sd) {
            Some(o) => o,
            None => return Ok(i64t.const_int(0, false).into()),
        };

        let fp_global_name = format!("__store_{store_name}_fp");
        let fp_g = self
            .module
            .get_global(&fp_global_name)
            .ok_or_else(|| format!("no fp global for store '{store_name}'"))?;

        let path_lit = format!("{store_name}.store\0");
        let path_str = b!(self.bld.build_global_string_ptr(&path_lit, "cmp.path"));

        let compact_fn = crate::codegen::fn_or_die(&self.module, "jinn_store_compact");
        let reclaimed = self.call_result(b!(self.bld.build_call(
            compact_fn,
            &[
                fp_g.as_pointer_value().into(),
                path_str.as_pointer_value().into(),
                i64t.const_int(offset, false).into(),
            ],
            "cmp.n"
        )));

        let _ = ptr_ty;
        self.wal_checkpoint(store_name)?;
        Ok(reclaimed)
    }

    pub(in crate::codegen) fn emit_store_save(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (_sd, _st, _rec_size, fp) = self.setup_store_access(store_name)?;
        let wal = self.load_store_wal(store_name)?;
        let save_fn = crate::codegen::fn_or_die(&self.module, "jinn_store_save");
        b!(self.bld.build_call(save_fn, &[fp.into(), wal.into()], ""));
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }
}
