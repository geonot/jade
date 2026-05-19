use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_store_query(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = encoded_name.splitn(3, "__").collect();
        if parts.len() < 3 || args.is_empty() {
            return Ok(self.ctx.i64_type().const_int(0, false).into());
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
        let i32t = self.ctx.i32_type();

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
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.value_map[&args[0]];

        let primary_field = &sd.fields[field_idx];
        let use_index = matches!(op, crate::ast::BinOp::Eq)
            && Compiler::field_has_index(primary_field)
            && extra_specs.is_empty();

        if use_index {
            let result_ptr = self.entry_alloca(st.into(), "qi.result");
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

            let idx_ptr = self.load_store_idx(store_name, field_name)?;
            let hash = self.idx_hash_field(filter_val, &field_ty)?;
            let lookup_fn = crate::codegen::fn_or_die(&self.module, "jinn_idx_lookup");
            let file_offset = self
                .call_result(b!(self.bld.build_call(
                    lookup_fn,
                    &[idx_ptr.into(), hash.into()],
                    "qi.off"
                )))
                .into_int_value();

            let fv_fn = self.cur_fn.expect("ICE: cur_fn not set");
            let found_bb = self.ctx.append_basic_block(fv_fn, "qi.found");
            let done_bb = self.ctx.append_basic_block(fv_fn, "qi.done");

            let not_found = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                file_offset,
                i64t.const_int(u64::MAX, false),
                "qi.miss"
            ));
            b!(self
                .bld
                .build_conditional_branch(not_found, done_bb, found_bb));

            self.bld.position_at_end(found_bb);
            let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
            b!(self.bld.build_call(
                fseek_fn,
                &[
                    fp.into(),
                    file_offset.into(),
                    i32t.const_int(0, false).into()
                ],
                ""
            ));
            let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");
            b!(self.bld.build_call(
                fread_fn,
                &[
                    result_ptr.into(),
                    i64t.const_int(rec_size, false).into(),
                    i64t.const_int(1, false).into(),
                    fp.into(),
                ],
                ""
            ));

            if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
                let del_gep =
                    b!(self
                        .bld
                        .build_struct_gep(st, result_ptr, del_idx as u32, "qi.del"));
                let del_val = b!(self.bld.build_load(i64t, del_gep, "qi.del.val")).into_int_value();
                let is_deleted = b!(self.bld.build_int_compare(
                    inkwell::IntPredicate::NE,
                    del_val,
                    i64t.const_int(0, false),
                    "qi.is_del"
                ));
                let copy_bb = self.ctx.append_basic_block(fv_fn, "qi.copy");

                let zero_bb = self.ctx.append_basic_block(fv_fn, "qi.zero");
                b!(self
                    .bld
                    .build_conditional_branch(is_deleted, zero_bb, copy_bb));

                self.bld.position_at_end(zero_bb);
                b!(self.bld.build_call(
                    memset_fn,
                    &[
                        result_ptr.into(),
                        i32t.const_int(0, false).into(),
                        i64t.const_int(rec_size, false).into()
                    ],
                    ""
                ));
                b!(self.bld.build_unconditional_branch(done_bb));

                self.bld.position_at_end(copy_bb);
            }

            b!(self.bld.build_unconditional_branch(done_bb));

            self.bld.position_at_end(done_bb);
            let result = self.load_store_record_as_jinn(st, result_ptr, &sd)?;
            return Ok(result);
        }

        let count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, count, rec_size)?;

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

        let fv_fn = self.cur_fn.expect("ICE: cur_fn not set");
        let loop_idx_ptr = self.entry_alloca(i64t.into(), "q.idx");
        b!(self.bld.build_store(loop_idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv_fn, "q.loop");
        let body_bb = self.ctx.append_basic_block(fv_fn, "q.body");
        let match_bb = self.ctx.append_basic_block(fv_fn, "q.match");
        let next_bb = self.ctx.append_basic_block(fv_fn, "q.next");
        let done_bb = self.ctx.append_basic_block(fv_fn, "q.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, loop_idx_ptr, "idx")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "q.cmp"));
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

        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "q.del"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "q.del.val")).into_int_value();
            let is_deleted = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "q.is_del"
            ));
            let filter_bb = self.ctx.append_basic_block(fv_fn, "q.filter");
            b!(self
                .bld
                .build_conditional_branch(is_deleted, next_bb, filter_bb));
            self.bld.position_at_end(filter_bb);
        }

        let cond = {
            let mut extras: Vec<(
                crate::ast::LogicalOp,
                usize,
                Type,
                crate::ast::BinOp,
                BasicValueEnum<'ctx>,
            )> = Vec::new();
            for (ei, (lop, efield, eop)) in extra_specs.iter().enumerate() {
                let (eidx, ety) = sd
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .ok_or_else(|| format!("unknown field '{efield}' in store '{store_name}'"))?;
                let eval = self.value_map[&args[ei + 1]];
                extras.push((*lop, eidx, ety, *eop, eval));
            }
            self.eval_store_filter(rec_ptr, st, field_idx, &field_ty, op, filter_val, &extras)?
        };
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
        b!(self.bld.build_store(loop_idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        let result = self.load_store_record_as_jinn(st, result_ptr, &sd)?;
        Ok(result)
    }

    pub(in crate::codegen) fn parse_store_op(s: &str) -> crate::ast::BinOp {
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

    pub(in crate::codegen) fn emit_store_count(
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
        let i32t = self.ctx.i32_type();

        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted");

        if deleted_idx.is_none() {
            let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
            b!(self.bld.build_call(
                fseek_fn,
                &[
                    fp.into(),
                    i64t.const_int(8, false).into(),
                    i32t.const_int(0, false).into()
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
                    fp.into()
                ],
                ""
            ));
            return Ok(b!(self.bld.build_load(i64t, count_buf, "count")));
        }

        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        let rec_size = self.store_record_size(&sd);
        let del_idx = deleted_idx.unwrap();

        let total_count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, total_count, rec_size)?;

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let live_ptr = self.entry_alloca(i64t.into(), "sc.live");
        b!(self.bld.build_store(live_ptr, i64t.const_int(0, false)));
        let idx_ptr = self.entry_alloca(i64t.into(), "sc.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "sc.loop");
        let body_bb = self.ctx.append_basic_block(fv, "sc.body");
        let inc_bb = self.ctx.append_basic_block(fv, "sc.inc");
        let next_bb = self.ctx.append_basic_block(fv, "sc.next");
        let done_bb = self.ctx.append_basic_block(fv, "sc.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "sc.i")).into_int_value();
        let cmp =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, total_count, "sc.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "sc.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "sc.rec"))
        };
        let del_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, del_idx as u32, "sc.del"));
        let del_val = b!(self.bld.build_load(i64t, del_gep, "sc.del.val")).into_int_value();
        let is_live = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            del_val,
            i64t.const_int(0, false),
            "sc.live_cmp"
        ));
        b!(self.bld.build_conditional_branch(is_live, inc_bb, next_bb));

        self.bld.position_at_end(inc_bb);
        let cur = b!(self.bld.build_load(i64t, live_ptr, "sc.cur")).into_int_value();
        let inc = b!(self
            .bld
            .build_int_add(cur, i64t.const_int(1, false), "sc.inc"));
        b!(self.bld.build_store(live_ptr, inc));
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "sc.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        Ok(b!(self.bld.build_load(i64t, live_ptr, "count")))
    }

    pub(in crate::codegen) fn emit_view_count(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = encoded_name.splitn(3, "__").collect();
        if parts.len() < 3 || args.is_empty() {
            return Ok(self.ctx.i64_type().const_int(0, false).into());
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
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.value_map[&args[0]];

        let count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, count, rec_size)?;

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let match_count_ptr = self.entry_alloca(i64t.into(), "vc.cnt");
        b!(self
            .bld
            .build_store(match_count_ptr, i64t.const_int(0, false)));
        let idx_ptr = self.entry_alloca(i64t.into(), "vc.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "vc.loop");
        let body_bb = self.ctx.append_basic_block(fv, "vc.body");
        let match_bb = self.ctx.append_basic_block(fv, "vc.match");
        let next_bb = self.ctx.append_basic_block(fv, "vc.next");
        let done_bb = self.ctx.append_basic_block(fv, "vc.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "vc.i")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "vc.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "vc.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "vc.rec"))
        };

        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "vc.del"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "vc.del.val")).into_int_value();
            let is_deleted = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "vc.is_del"
            ));
            let filter_bb = self.ctx.append_basic_block(fv, "vc.filter");
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
        let cond =
            self.eval_store_filter(rec_ptr, st, field_idx, &field_ty, op, filter_val, &extras)?;
        b!(self.bld.build_conditional_branch(cond, match_bb, next_bb));

        self.bld.position_at_end(match_bb);
        let cur = b!(self.bld.build_load(i64t, match_count_ptr, "vc.cur")).into_int_value();
        let inc = b!(self
            .bld
            .build_int_add(cur, i64t.const_int(1, false), "vc.inc"));
        b!(self.bld.build_store(match_count_ptr, inc));
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "vc.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        Ok(b!(self.bld.build_load(i64t, match_count_ptr, "vc.result")))
    }
}
