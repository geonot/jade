//! Store update, lookup, first, and existence MIR codegen.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_store_set(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Name format: {store_name}__{field}__{op}[__and/or__{f2}__{op2}]*__fields_{f1}_{f2}_...
        // Split out the __fields_ suffix first.
        let (filter_part, fields_part) = if let Some(pos) = encoded_name.find("__fields_") {
            (&encoded_name[..pos], &encoded_name[pos + 9..]) // skip "__fields_"
        } else {
            return Err(format!(
                "malformed store.set name '{encoded_name}'"
            ));
        };

        let field_names: Vec<&str> = fields_part.split('_').collect();

        let (store_name, filter_field, primary_op, extra_conds) =
            Self::parse_encoded_filter(filter_part)?;
        if args.is_empty() {
            return Ok(self.ctx.i64_type().const_int(0, false).into());
        }
        let extra_count = extra_conds.len();

        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        self.store_lock(fp)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == filter_field)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{filter_field}' in store '{store_name}'"))?;

        let filter_val = self.val(args[0]);
        // args[1..1+extra_count] are extra filter vals
        // args[1+extra_count..] are the field assignment values
        let assign_start = 1 + extra_count;

        // Pre-gather field assignment values
        let mut assign_vals: Vec<(usize, &str, BasicValueEnum<'ctx>)> = Vec::new();
        for (i, fname) in field_names.iter().enumerate() {
            let arg_idx = assign_start + i;
            if arg_idx >= args.len() {
                break;
            }
            let val = self.val(args[arg_idx]);
            let field_pos = sd
                .fields
                .iter()
                .position(|f| f.name == *fname)
                .ok_or_else(|| format!("unknown field '{fname}' in store '{store_name}'"))?;
            assign_vals.push((field_pos, fname, val));
        }

        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");

        let count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, count, rec_size)?;

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
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
            .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "set.cmp"));
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

        // Skip soft-deleted records
        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "set.del"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "set.del.val")).into_int_value();
            let is_deleted = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "set.is_del"
            ));
            let filter_bb = self.ctx.append_basic_block(fv, "set.filter");
            b!(self
                .bld
                .build_conditional_branch(is_deleted, next_bb, filter_bb));
            self.bld.position_at_end(filter_bb);
        }

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
        b!(self
            .bld
            .build_conditional_branch(matches, update_bb, next_bb));

        self.bld.position_at_end(update_bb);

        // Phase 7: If @versioned, save old record to versions file and increment __version
        let is_versioned = Compiler::store_is_versioned(&sd);
        if is_versioned {
            // Read the record's current sid and __version
            let sid_idx = sd.fields.iter().position(|f| f.name == "sid");
            let ver_idx = sd.fields.iter().position(|f| f.name == "__version");
            if let (Some(si), Some(vi)) = (sid_idx, ver_idx) {
                let sid_gep = b!(self
                    .bld
                    .build_struct_gep(st, rec_ptr, si as u32, "ver.sid.gep"));
                let sid_val = b!(self.bld.build_load(i64t, sid_gep, "ver.sid")).into_int_value();
                let ver_gep = b!(self
                    .bld
                    .build_struct_gep(st, rec_ptr, vi as u32, "ver.ver.gep"));
                let old_ver = b!(self.bld.build_load(i64t, ver_gep, "ver.old")).into_int_value();

                // Save old record to versions file
                let ver_fp = self.load_store_ver(store_name)?;
                let ver_append_fn = crate::codegen::fn_or_die(&self.module, "jinn_ver_append");
                b!(self.bld.build_call(
                    ver_append_fn,
                    &[
                        ver_fp.into(),
                        sid_val.into(),
                        old_ver.into(),
                        rec_ptr.into(),
                        i64t.const_int(rec_size, false).into()
                    ],
                    ""
                ));

                // Increment __version
                let new_ver =
                    b!(self
                        .bld
                        .build_int_add(old_ver, i64t.const_int(1, false), "ver.new"));
                b!(self.bld.build_store(ver_gep, new_ver));
            }
        }

        for (fpos, _fname, val) in &assign_vals {
            let fty = &sd.fields[*fpos].ty;
            let gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, *fpos as u32, "set.assign"));
            match fty {
                Type::String => {
                    self.copy_string_to_fixed_buf(*val, gep)?;
                }
                _ => {
                    b!(self.bld.build_store(gep, *val));
                }
            }
        }

        // Update the `updated` timestamp on modified records
        if let Some(upd_idx) = sd.fields.iter().position(|f| f.name == "updated") {
            self.ensure_time_fn();
            let time_fn = crate::codegen::fn_or_die(&self.module, "time");
            let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
            let now = self
                .call_result(b!(self.bld.build_call(
                    time_fn,
                    &[ptr_ty.const_null().into()],
                    "set.now"
                )))
                .into_int_value();
            let upd_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, upd_idx as u32, "set.upd"));
            b!(self.bld.build_store(upd_gep, now));
        }

        // WAL: log the update
        self.wal_write_update(store_name, rec_ptr, rec_size)?;

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

    /// Parse a filter op string back to a BinOp.
    pub(in crate::codegen) fn parse_filter_op(s: &str) -> crate::ast::BinOp {
        match s {
            "eq" => crate::ast::BinOp::Eq,
            "ne" => crate::ast::BinOp::Ne,
            "lt" => crate::ast::BinOp::Lt,
            "le" => crate::ast::BinOp::Le,
            "gt" => crate::ast::BinOp::Gt,
            "ge" => crate::ast::BinOp::Ge,
            _ => crate::ast::BinOp::Eq,
        }
    }

    /// Parse the encoded filter name into (store_name, field, op, extra_conds).
    pub(in crate::codegen) fn parse_encoded_filter(
        encoded: &str,
    ) -> Result<
        (
            &str,
            &str,
            crate::ast::BinOp,
            Vec<(crate::ast::LogicalOp, String, crate::ast::BinOp)>,
        ),
        String,
    > {
        let parts: Vec<&str> = encoded.splitn(3, "__").collect();
        if parts.len() < 3 {
            return Err(format!("malformed encoded filter: '{encoded}'"));
        }
        let store_name = parts[0];
        let field_name = parts[1];
        let remainder = parts[2];
        let segments: Vec<&str> = remainder.split("__").collect();
        let op = Self::parse_filter_op(segments[0]);

        let mut extra: Vec<(crate::ast::LogicalOp, String, crate::ast::BinOp)> = Vec::new();
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
            let efield = segments[i + 1].to_string();
            let eop = Self::parse_filter_op(segments[i + 2]);
            extra.push((lop, efield, eop));
            i += 3;
        }
        Ok((store_name, field_name, op, extra))
    }

    /// Common helper to set up store access: ensure open, load_fp, get sd + rec type + rec size.
    pub(in crate::codegen) fn setup_store_access(
        &mut self,
        store_name: &str,
    ) -> Result<
        (
            hir::StoreDef,
            inkwell::types::StructType<'ctx>,
            u64,
            PointerValue<'ctx>,
        ),
        String,
    > {
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
        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .module
            .get_struct_type(&rec_name)
            .ok_or_else(|| format!("no store rec struct '{rec_name}'"))?;
        let rec_size = self.store_record_size(&sd);
        Ok((sd, st, rec_size, fp))
    }

    // ── StoreGet: lookup by sid (i64) ──
    pub(in crate::codegen) fn emit_store_get(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Ok(self.ctx.i64_type().const_int(0, false).into());
        }
        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let sid_val = self.val(args[0]).into_int_value();

        let count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, count, rec_size)?;

        let result_ptr = self.entry_alloca(st.into(), "get.result");
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

        // sid is always the first field (index 0) for non-@simple stores
        // Find sid index
        let sid_idx = sd.fields.iter().position(|f| f.name == "sid").unwrap_or(0);

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let idx_ptr = self.entry_alloca(i64t.into(), "get.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "get.loop");
        let body_bb = self.ctx.append_basic_block(fv, "get.body");
        let match_bb = self.ctx.append_basic_block(fv, "get.match");
        let next_bb = self.ctx.append_basic_block(fv, "get.next");
        let done_bb = self.ctx.append_basic_block(fv, "get.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "get.i")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "get.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "get.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "get.rec"))
        };

        // Skip soft-deleted records
        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "get.del"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "get.del.val")).into_int_value();
            let is_deleted = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "get.is_del"
            ));
            let check_bb = self.ctx.append_basic_block(fv, "get.check");
            b!(self
                .bld
                .build_conditional_branch(is_deleted, next_bb, check_bb));
            self.bld.position_at_end(check_bb);
        }

        let rec_sid_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, sid_idx as u32, "get.sid"));
        let rec_sid = b!(self.bld.build_load(i64t, rec_sid_gep, "get.sid.val")).into_int_value();
        let match_cond =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::EQ, rec_sid, sid_val, "get.eq"));
        b!(self
            .bld
            .build_conditional_branch(match_cond, match_bb, next_bb));

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
            .build_int_add(idx, i64t.const_int(1, false), "get.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        let result = self.load_store_record_as_jinn(st, result_ptr, &sd)?;
        Ok(result)
    }

    // ── StoreFirst: like query but returns first match ──
    pub(in crate::codegen) fn emit_store_first(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Same as emit_store_query — the query already returns first match
        self.emit_store_query(encoded_name, args)
    }

    // ── StoreExists: returns bool (1 if match found, 0 otherwise) ──
    pub(in crate::codegen) fn emit_store_exists(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (store_name, field_name, op, extra_specs) = Self::parse_encoded_filter(encoded_name)?;
        if args.is_empty() {
            return Ok(self.ctx.bool_type().const_int(0, false).into());
        }
        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        let i64t = self.ctx.i64_type();

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.value_map[&args[0]];

        let count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, count, rec_size)?;

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let found_ptr = self.entry_alloca(self.ctx.bool_type().into(), "exists.found");
        b!(self
            .bld
            .build_store(found_ptr, self.ctx.bool_type().const_int(0, false)));
        let idx_ptr = self.entry_alloca(i64t.into(), "exists.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "exists.loop");
        let body_bb = self.ctx.append_basic_block(fv, "exists.body");
        let match_bb = self.ctx.append_basic_block(fv, "exists.match");
        let next_bb = self.ctx.append_basic_block(fv, "exists.next");
        let done_bb = self.ctx.append_basic_block(fv, "exists.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "exists.i")).into_int_value();
        let cmp =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "exists.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "exists.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "exists.rec"))
        };

        // Skip soft-deleted records
        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "ex.del"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "ex.del.val")).into_int_value();
            let is_deleted = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "ex.is_del"
            ));
            let filter_bb = self.ctx.append_basic_block(fv, "exists.filter");
            b!(self
                .bld
                .build_conditional_branch(is_deleted, next_bb, filter_bb));
            self.bld.position_at_end(filter_bb);
        }

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
                let ev = self.value_map[&args[ei + 1]];
                (*lop, fi, ft, *eop, ev)
            })
            .collect();
        let cond =
            self.eval_store_filter(rec_ptr, st, field_idx, &field_ty, op, filter_val, &extras)?;
        b!(self.bld.build_conditional_branch(cond, match_bb, next_bb));

        self.bld.position_at_end(match_bb);
        b!(self
            .bld
            .build_store(found_ptr, self.ctx.bool_type().const_int(1, false)));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "exists.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        let result = b!(self
            .bld
            .build_load(self.ctx.bool_type(), found_ptr, "exists.result"));
        Ok(result)
    }

    // ── StoreDestroy: hard delete (physically remove matching records) ──
}
