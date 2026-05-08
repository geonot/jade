//! Store destroy, restore, and save MIR codegen.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_store_destroy(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Physical removal — same as what delete used to do
        self.emit_store_hard_delete(encoded_name, args)
    }

    // ── StoreRestore: clear the deleted timestamp on soft-deleted records ──
    pub(in crate::codegen) fn emit_store_restore(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (store_name, field_name, op, extra_specs) = Self::parse_encoded_filter(encoded_name)?;
        if args.is_empty() {
            return Ok(self.ctx.i64_type().const_int(0, false).into());
        }
        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        self.store_lock(fp)?;
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

        // Find the 'deleted' field index
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

        // Only consider records that are actually soft-deleted
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
        b!(self.bld.build_conditional_branch(cond, update_bb, next_bb));

        self.bld.position_at_end(update_bb);
        // Set 'deleted' field to 0
        let del_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, deleted_idx as u32, "restore.del"));
        b!(self.bld.build_store(del_gep, i64t.const_int(0, false)));
        // WAL: log the restore as an update
        self.wal_write_update(store_name, rec_ptr, rec_size)?;
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "restore.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        // Write records back
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
        self.store_unlock(fp)?;

        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    // ── StoreSave: flush the store file ──
    pub(in crate::codegen) fn emit_store_save(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (_sd, _st, _rec_size, fp) = self.setup_store_access(store_name)?;
        let fflush_fn = crate::codegen::fn_or_die(&self.module, "fflush");
        b!(self.bld.build_call(fflush_fn, &[fp.into()], ""));
        // Checkpoint WAL on save
        self.wal_checkpoint(store_name)?;
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }
}
