use super::*;

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
        let count_fn = self.module.get_function("jinn_fts_posting_count").unwrap();
        let count = self
            .call_result(b!(self.bld.build_call(count_fn, &[fts.into()], "fts.cnt")))
            .into_int_value();
        Ok(count.into())
    }

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

        let header_ty = self.vec_header_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let malloc_fn = self.ensure_malloc();
        let result_vec = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[i64t.const_int(24, false).into()],
                "dist.vec"
            )))
            .into_pointer_value();
        let v_dgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 0, "dist.vec.d"));
        b!(self.bld.build_store(v_dgep, ptr_ty.const_null()));
        let v_lgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 1, "dist.vec.l"));
        b!(self.bld.build_store(v_lgep, i64t.const_int(0, false)));
        let v_cgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 2, "dist.vec.c"));
        b!(self.bld.build_store(v_cgep, i64t.const_int(0, false)));
        let elem_lty = self.llvm_ty(&field_ty);
        let elem_size = self.type_store_size(elem_lty);

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

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "dist.i")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::ULT,
            idx,
            total_count,
            "dist.cmp"
        ));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

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

        self.bld.position_at_end(check_bb);
        let field_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, field_idx as u32, "dist.fld"));

        let hash = self.hash_store_field_from_gep(field_gep, &field_ty)?;

        let marked_h = b!(self
            .bld
            .build_or(hash, i64t.const_int(1, false), "dist.marked"));

        let slot_ptr = self.entry_alloca(i64t.into(), "dist.slot");
        let init_slot = b!(self.bld.build_int_unsigned_rem(marked_h, cap, "dist.islot"));
        b!(self.bld.build_store(slot_ptr, init_slot));

        let probe_bb = self.ctx.append_basic_block(fv, "dist.probe");
        let add_bb = self.ctx.append_basic_block(fv, "dist.add");

        b!(self.bld.build_unconditional_branch(probe_bb));

        self.bld.position_at_end(probe_bb);
        let slot = b!(self.bld.build_load(i64t, slot_ptr, "dist.s")).into_int_value();
        let entry_ptr = unsafe { b!(self.bld.build_gep(i64t, hash_tbl, &[slot], "dist.ep")) };
        let entry_val = b!(self.bld.build_load(i64t, entry_ptr, "dist.ev")).into_int_value();

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

        self.bld.position_at_end(advance_bb);
        let next_slot = b!(self
            .bld
            .build_int_add(slot, i64t.const_int(1, false), "dist.ns"));
        let wrapped = b!(self.bld.build_int_unsigned_rem(next_slot, cap, "dist.wrap"));
        b!(self.bld.build_store(slot_ptr, wrapped));
        b!(self.bld.build_unconditional_branch(probe_bb));

        self.bld.position_at_end(add_bb);

        let add_slot = b!(self.bld.build_load(i64t, slot_ptr, "dist.as")).into_int_value();
        let add_entry = unsafe { b!(self.bld.build_gep(i64t, hash_tbl, &[add_slot], "dist.ae")) };
        b!(self.bld.build_store(add_entry, marked_h));
        let uc = b!(self.bld.build_load(i64t, uniq_ptr, "dist.uc2")).into_int_value();
        let new_uc = b!(self
            .bld
            .build_int_add(uc, i64t.const_int(1, false), "dist.ucinc"));
        b!(self.bld.build_store(uniq_ptr, new_uc));

        let elem_val = match crate::codegen::store_filter::normalize_store_field_type(&field_ty) {
            crate::types::Type::String => self.read_string_from_fixed_buf(field_gep)?,
            ref nty => {
                let lty = self.llvm_ty(nty);
                b!(self.bld.build_load(lty, field_gep, "dist.elem"))
            }
        };
        self.vec_push_raw(result_vec, elem_val, elem_lty, elem_size)?;

        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "dist.ni"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        b!(self.bld.build_call(free_fn, &[hash_tbl.into()], ""));

        Ok(result_vec.into())
    }

    pub(in crate::codegen) fn emit_store_group(
        &mut self,
        rest: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = rest.splitn(4, "__").collect();
        if parts.len() < 4 {
            return Err(format!("malformed store group name: {rest}"));
        }
        let store_name = parts[0];
        let key_name = parts[1];
        let agg = parts[2];
        let val_name = parts[3];

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
        let f64t = self.ctx.f64_type();
        let i8t = self.ctx.i8_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        let rec_size = self.store_record_size(&sd);

        let (key_idx, key_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == key_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("no field '{key_name}' in store '{store_name}'"))?;
        let key_norm = crate::codegen::store_filter::normalize_store_field_type(&key_ty);

        let is_count = agg == "count";
        let (val_idx, val_is_float) = if is_count {
            (0usize, false)
        } else {
            sd.fields
                .iter()
                .enumerate()
                .find(|(_, f)| f.name == val_name)
                .map(|(i, f)| {
                    let norm = crate::codegen::store_filter::normalize_store_field_type(&f.ty);
                    (
                        i,
                        matches!(norm, crate::types::Type::F64 | crate::types::Type::F32),
                    )
                })
                .ok_or_else(|| format!("no field '{val_name}' in store '{store_name}'"))?
        };
        let result_float = !is_count && (val_is_float || agg == "avg");

        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted");

        let total_count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, total_count, rec_size)?;

        let calloc_fn = self.ensure_calloc();
        let cap = b!(self.bld.build_int_add(
            b!(self
                .bld
                .build_int_mul(total_count, i64t.const_int(4, false), "grp.cap.mul")),
            i64t.const_int(16, false),
            "grp.cap"
        ));
        let hash_tbl = self
            .call_result(b!(self.bld.build_call(
                calloc_fn,
                &[cap.into(), i64t.const_int(8, false).into()],
                "grp.tbl"
            )))
            .into_pointer_value();
        let rec_of = self
            .call_result(b!(self.bld.build_call(
                calloc_fn,
                &[cap.into(), i64t.const_int(8, false).into()],
                "grp.recof"
            )))
            .into_pointer_value();
        let acc_arr = self
            .call_result(b!(self.bld.build_call(
                calloc_fn,
                &[cap.into(), i64t.const_int(8, false).into()],
                "grp.acc"
            )))
            .into_pointer_value();
        let cnt_arr = self
            .call_result(b!(self.bld.build_call(
                calloc_fn,
                &[cap.into(), i64t.const_int(8, false).into()],
                "grp.cnt"
            )))
            .into_pointer_value();

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let idx_ptr = self.entry_alloca(i64t.into(), "grp.idx");
        let ngrp_ptr = self.entry_alloca(i64t.into(), "grp.n");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_store(ngrp_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "grp.loop");
        let body_bb = self.ctx.append_basic_block(fv, "grp.body");
        let live_bb = self.ctx.append_basic_block(fv, "grp.live");
        let next_bb = self.ctx.append_basic_block(fv, "grp.next");
        let done_bb = self.ctx.append_basic_block(fv, "grp.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "grp.i")).into_int_value();
        let cmp =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, total_count, "grp.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "grp.off"));
        let rec_ptr = unsafe { b!(self.bld.build_gep(i8t, buf, &[offset], "grp.rec")) };

        if let Some(del_idx) = deleted_idx {
            let del_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "grp.del"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "grp.del.val")).into_int_value();
            let is_live = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                del_val,
                i64t.const_int(0, false),
                "grp.islive"
            ));
            b!(self.bld.build_conditional_branch(is_live, live_bb, next_bb));
        } else {
            b!(self.bld.build_unconditional_branch(live_bb));
        }

        self.bld.position_at_end(live_bb);
        let key_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, key_idx as u32, "grp.key"));
        let hash = self.hash_store_field_from_gep(key_gep, &key_ty)?;
        let marked_h = b!(self
            .bld
            .build_or(hash, i64t.const_int(1, false), "grp.marked"));

        let slot_ptr = self.entry_alloca(i64t.into(), "grp.slot");
        let init_slot = b!(self.bld.build_int_unsigned_rem(marked_h, cap, "grp.islot"));
        b!(self.bld.build_store(slot_ptr, init_slot));

        let probe_bb = self.ctx.append_basic_block(fv, "grp.probe");
        let newgrp_bb = self.ctx.append_basic_block(fv, "grp.new");
        let accum_bb = self.ctx.append_basic_block(fv, "grp.accum");
        b!(self.bld.build_unconditional_branch(probe_bb));

        self.bld.position_at_end(probe_bb);
        let slot = b!(self.bld.build_load(i64t, slot_ptr, "grp.s")).into_int_value();
        let entry_ptr = unsafe { b!(self.bld.build_gep(i64t, hash_tbl, &[slot], "grp.ep")) };
        let entry_val = b!(self.bld.build_load(i64t, entry_ptr, "grp.ev")).into_int_value();
        let is_empty = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            entry_val,
            i64t.const_int(0, false),
            "grp.empty"
        ));
        let probe_match_bb = self.ctx.append_basic_block(fv, "grp.pmatch");
        b!(self
            .bld
            .build_conditional_branch(is_empty, newgrp_bb, probe_match_bb));

        self.bld.position_at_end(probe_match_bb);
        let is_match = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            entry_val,
            marked_h,
            "grp.hmatch"
        ));
        let advance_bb = self.ctx.append_basic_block(fv, "grp.advance");
        b!(self
            .bld
            .build_conditional_branch(is_match, accum_bb, advance_bb));

        self.bld.position_at_end(advance_bb);
        let next_slot = b!(self
            .bld
            .build_int_add(slot, i64t.const_int(1, false), "grp.ns"));
        let wrapped = b!(self.bld.build_int_unsigned_rem(next_slot, cap, "grp.wrap"));
        b!(self.bld.build_store(slot_ptr, wrapped));
        b!(self.bld.build_unconditional_branch(probe_bb));

        self.bld.position_at_end(newgrp_bb);
        b!(self.bld.build_store(entry_ptr, marked_h));
        let recof_slot = unsafe { b!(self.bld.build_gep(i64t, rec_of, &[slot], "grp.rofs")) };
        b!(self.bld.build_store(recof_slot, idx));
        let acc_init_slot = unsafe { b!(self.bld.build_gep(i64t, acc_arr, &[slot], "grp.ais")) };
        if result_float {
            let zero = f64t.const_float(0.0);
            let bits = b!(self.bld.build_bit_cast(zero, i64t, "grp.zbits"));
            b!(self.bld.build_store(acc_init_slot, bits));
        } else {
            b!(self
                .bld
                .build_store(acc_init_slot, i64t.const_int(0, false)));
        }
        let ng = b!(self.bld.build_load(i64t, ngrp_ptr, "grp.ngl")).into_int_value();
        let ng1 = b!(self
            .bld
            .build_int_add(ng, i64t.const_int(1, false), "grp.ng1"));
        b!(self.bld.build_store(ngrp_ptr, ng1));
        b!(self.bld.build_unconditional_branch(accum_bb));

        self.bld.position_at_end(accum_bb);
        let acc_slot = unsafe { b!(self.bld.build_gep(i64t, acc_arr, &[slot], "grp.acs")) };
        let cnt_slot = unsafe { b!(self.bld.build_gep(i64t, cnt_arr, &[slot], "grp.cns")) };
        let cur_cnt = b!(self.bld.build_load(i64t, cnt_slot, "grp.ccnt")).into_int_value();
        let new_cnt = b!(self
            .bld
            .build_int_add(cur_cnt, i64t.const_int(1, false), "grp.cntinc"));
        b!(self.bld.build_store(cnt_slot, new_cnt));

        if !is_count {
            let val_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, val_idx as u32, "grp.vgep"));
            if result_float {
                let v = if val_is_float {
                    b!(self.bld.build_load(f64t, val_gep, "grp.vf")).into_float_value()
                } else {
                    let iv = b!(self.bld.build_load(i64t, val_gep, "grp.vi")).into_int_value();
                    b!(self.bld.build_signed_int_to_float(iv, f64t, "grp.vi2f"))
                };
                let cur_bits = b!(self.bld.build_load(i64t, acc_slot, "grp.acb")).into_int_value();
                let cur = b!(self.bld.build_bit_cast(cur_bits, f64t, "grp.acf")).into_float_value();
                let is_first = b!(self.bld.build_int_compare(
                    inkwell::IntPredicate::EQ,
                    cur_cnt,
                    i64t.const_int(0, false),
                    "grp.first"
                ));
                let combined = match agg {
                    "min" => {
                        let lt = b!(self.bld.build_float_compare(
                            inkwell::FloatPredicate::OLT,
                            v,
                            cur,
                            "grp.fmin"
                        ));
                        b!(self.bld.build_select(lt, v, cur, "grp.fminsel")).into_float_value()
                    }
                    "max" => {
                        let gt = b!(self.bld.build_float_compare(
                            inkwell::FloatPredicate::OGT,
                            v,
                            cur,
                            "grp.fmax"
                        ));
                        b!(self.bld.build_select(gt, v, cur, "grp.fmaxsel")).into_float_value()
                    }
                    _ => b!(self.bld.build_float_add(cur, v, "grp.fadd")),
                };
                let result = if matches!(agg, "min" | "max") {
                    b!(self.bld.build_select(is_first, v, combined, "grp.firstsel"))
                        .into_float_value()
                } else {
                    combined
                };
                let rbits = b!(self.bld.build_bit_cast(result, i64t, "grp.rbits"));
                b!(self.bld.build_store(acc_slot, rbits));
            } else {
                let v = b!(self.bld.build_load(i64t, val_gep, "grp.vi")).into_int_value();
                let cur = b!(self.bld.build_load(i64t, acc_slot, "grp.aci")).into_int_value();
                let is_first = b!(self.bld.build_int_compare(
                    inkwell::IntPredicate::EQ,
                    cur_cnt,
                    i64t.const_int(0, false),
                    "grp.ifirst"
                ));
                let combined = match agg {
                    "min" => {
                        let lt = b!(self.bld.build_int_compare(
                            inkwell::IntPredicate::SLT,
                            v,
                            cur,
                            "grp.imin"
                        ));
                        b!(self.bld.build_select(lt, v, cur, "grp.iminsel")).into_int_value()
                    }
                    "max" => {
                        let gt = b!(self.bld.build_int_compare(
                            inkwell::IntPredicate::SGT,
                            v,
                            cur,
                            "grp.imax"
                        ));
                        b!(self.bld.build_select(gt, v, cur, "grp.imaxsel")).into_int_value()
                    }
                    _ => b!(self.bld.build_int_add(cur, v, "grp.iadd")),
                };
                let result = if matches!(agg, "min" | "max") {
                    b!(self
                        .bld
                        .build_select(is_first, v, combined, "grp.ifirstsel"))
                    .into_int_value()
                } else {
                    combined
                };
                b!(self.bld.build_store(acc_slot, result));
            }
        }
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "grp.ni"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);

        let key_lty = self.llvm_ty(&key_norm);
        let val_lty: inkwell::types::BasicTypeEnum<'ctx> = if result_float {
            f64t.into()
        } else {
            i64t.into()
        };
        let tuple_ty = self.ctx.struct_type(&[key_lty, val_lty], false);
        let tuple_size = self.type_store_size(tuple_ty.into());

        let header_ty = self.vec_header_type();
        let malloc_fn = self.ensure_malloc();
        let result_vec = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[i64t.const_int(24, false).into()],
                "grp.vec"
            )))
            .into_pointer_value();
        let v_dgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 0, "grp.vec.d"));
        b!(self.bld.build_store(v_dgep, ptr_ty.const_null()));
        let v_lgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 1, "grp.vec.l"));
        b!(self.bld.build_store(v_lgep, i64t.const_int(0, false)));
        let v_cgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 2, "grp.vec.c"));
        b!(self.bld.build_store(v_cgep, i64t.const_int(0, false)));

        let sidx_ptr = self.entry_alloca(i64t.into(), "grp.sidx");
        b!(self.bld.build_store(sidx_ptr, i64t.const_int(0, false)));

        let oloop_bb = self.ctx.append_basic_block(fv, "grp.oloop");
        let obody_bb = self.ctx.append_basic_block(fv, "grp.obody");
        let oused_bb = self.ctx.append_basic_block(fv, "grp.oused");
        let onext_bb = self.ctx.append_basic_block(fv, "grp.onext");
        let odone_bb = self.ctx.append_basic_block(fv, "grp.odone");

        b!(self.bld.build_unconditional_branch(oloop_bb));

        self.bld.position_at_end(oloop_bb);
        let sidx = b!(self.bld.build_load(i64t, sidx_ptr, "grp.osi")).into_int_value();
        let ocmp =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, sidx, cap, "grp.ocmp"));
        b!(self.bld.build_conditional_branch(ocmp, obody_bb, odone_bb));

        self.bld.position_at_end(obody_bb);
        let oentry_ptr = unsafe { b!(self.bld.build_gep(i64t, hash_tbl, &[sidx], "grp.oep")) };
        let oentry = b!(self.bld.build_load(i64t, oentry_ptr, "grp.oev")).into_int_value();
        let oused = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            oentry,
            i64t.const_int(0, false),
            "grp.oused"
        ));
        b!(self.bld.build_conditional_branch(oused, oused_bb, onext_bb));

        self.bld.position_at_end(oused_bb);
        let recof_ld = unsafe { b!(self.bld.build_gep(i64t, rec_of, &[sidx], "grp.orof")) };
        let rec_i = b!(self.bld.build_load(i64t, recof_ld, "grp.oreci")).into_int_value();
        let roffset =
            b!(self
                .bld
                .build_int_mul(rec_i, i64t.const_int(rec_size, false), "grp.ooff"));
        let orec_ptr = unsafe { b!(self.bld.build_gep(i8t, buf, &[roffset], "grp.orec")) };
        let okey_gep = b!(self
            .bld
            .build_struct_gep(st, orec_ptr, key_idx as u32, "grp.okey"));
        let key_val = match key_norm {
            crate::types::Type::String => self.read_string_from_fixed_buf(okey_gep)?,
            ref nty => {
                let lty = self.llvm_ty(nty);
                b!(self.bld.build_load(lty, okey_gep, "grp.okv"))
            }
        };

        let acc_ld = unsafe { b!(self.bld.build_gep(i64t, acc_arr, &[sidx], "grp.oacc")) };
        let cnt_ld = unsafe { b!(self.bld.build_gep(i64t, cnt_arr, &[sidx], "grp.ocnt")) };
        let cnt_val = b!(self.bld.build_load(i64t, cnt_ld, "grp.ocntv")).into_int_value();
        let out_val: BasicValueEnum<'ctx> = if is_count {
            cnt_val.into()
        } else if result_float {
            let abits = b!(self.bld.build_load(i64t, acc_ld, "grp.oab")).into_int_value();
            let af = b!(self.bld.build_bit_cast(abits, f64t, "grp.oaf")).into_float_value();
            if agg == "avg" {
                let cntf = b!(self
                    .bld
                    .build_signed_int_to_float(cnt_val, f64t, "grp.cntf"));
                b!(self.bld.build_float_div(af, cntf, "grp.avg")).into()
            } else {
                af.into()
            }
        } else {
            let av = b!(self.bld.build_load(i64t, acc_ld, "grp.oai")).into_int_value();
            av.into()
        };

        let undef = tuple_ty.get_undef();
        let with0 =
            b!(self.bld.build_insert_value(undef, key_val, 0, "grp.ins0")).into_struct_value();
        let tuple_val =
            b!(self.bld.build_insert_value(with0, out_val, 1, "grp.ins1")).into_struct_value();
        self.vec_push_raw(result_vec, tuple_val.into(), tuple_ty.into(), tuple_size)?;
        b!(self.bld.build_unconditional_branch(onext_bb));

        self.bld.position_at_end(onext_bb);
        let osnext = b!(self
            .bld
            .build_int_add(sidx, i64t.const_int(1, false), "grp.osn"));
        b!(self.bld.build_store(sidx_ptr, osnext));
        b!(self.bld.build_unconditional_branch(oloop_bb));

        self.bld.position_at_end(odone_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        b!(self.bld.build_call(free_fn, &[hash_tbl.into()], ""));
        b!(self.bld.build_call(free_fn, &[rec_of.into()], ""));
        b!(self.bld.build_call(free_fn, &[acc_arr.into()], ""));
        b!(self.bld.build_call(free_fn, &[cnt_arr.into()], ""));

        Ok(result_vec.into())
    }

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

        let is_column = sd.decorators.contains(&crate::ast::StoreDecorator::Column);
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
                    let fn_name = format!("jinn_col_{op}_i64");
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

        let acc_ptr = if is_float {
            self.entry_alloca(f64t.into(), "agg.acc")
        } else {
            self.entry_alloca(i64t.into(), "agg.acc")
        };
        let cnt_ptr = self.entry_alloca(i64t.into(), "agg.cnt");
        let idx_ptr = self.entry_alloca(i64t.into(), "agg.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_store(cnt_ptr, i64t.const_int(0, false)));

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

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "agg.i")).into_int_value();
        let cmp =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, total_count, "agg.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "agg.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "agg.rec"))
        };

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
                            lt, field_val, cur_acc, "agg.min"
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
                            gt, field_val, cur_acc, "agg.max"
                        ))
                    .into_int_value()
                }
                _ => cur_acc,
            };
            b!(self.bld.build_store(acc_ptr, new_acc));
        }

        if op == "avg" {
            let cur_cnt = b!(self.bld.build_load(i64t, cnt_ptr, "agg.ccnt")).into_int_value();
            let new_cnt = b!(self
                .bld
                .build_int_add(cur_cnt, i64t.const_int(1, false), "agg.cinc"));
            b!(self.bld.build_store(cnt_ptr, new_cnt));
        }

        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "agg.next_i"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

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
        let ver_count_fn = crate::codegen::fn_or_die(&self.module, "jinn_ver_count");
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

        let _ = sd;
        let total = b!(self
            .bld
            .build_int_add(count, i64t.const_int(1, false), "ver.total"));
        Ok(total.into())
    }

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

        let malloc_fn = self.ensure_malloc();
        let out_buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[i64t.const_int(rec_size, false).into()],
                "ver.buf"
            )))
            .into_pointer_value();

        let ver_at_fn = crate::codegen::fn_or_die(&self.module, "jinn_ver_at");
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

    pub(in crate::codegen) fn emit_store_query_group(
        &mut self,
        rest: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let segs: Vec<&str> = rest.split("__").collect();
        if segs.len() < 5 {
            return Err(format!("malformed store qgroup name: {rest}"));
        }
        let store_name = segs[0];
        let key_name = segs[1];
        let n_aggs: usize = segs[2]
            .parse()
            .map_err(|_| format!("malformed store qgroup name: {rest}"))?;
        let mut pos = 3;
        let mut agg_specs: Vec<(&str, &str)> = Vec::new();
        for _ in 0..n_aggs {
            if pos + 1 >= segs.len() {
                return Err(format!("malformed store qgroup name: {rest}"));
            }
            agg_specs.push((segs[pos], segs[pos + 1]));
            pos += 2;
        }
        let filter_spec = if pos < segs.len() {
            if segs[pos] != "where" || pos + 2 >= segs.len() {
                return Err(format!("malformed store qgroup name: {rest}"));
            }
            let pfield = segs[pos + 1];
            let (pop, ppred) = Self::parse_store_pred(segs[pos + 2]);
            pos += 3;
            let mut extras: Vec<(
                crate::ast::LogicalOp,
                &str,
                crate::ast::BinOp,
                crate::ast::FilterPred,
            )> = Vec::new();
            while pos + 2 < segs.len() {
                let lop = match segs[pos] {
                    "and" => crate::ast::LogicalOp::And,
                    "or" => crate::ast::LogicalOp::Or,
                    _ => return Err(format!("malformed store qgroup name: {rest}")),
                };
                let (eop, epred) = Self::parse_store_pred(segs[pos + 2]);
                extras.push((lop, segs[pos + 1], eop, epred));
                pos += 3;
            }
            Some((pfield, pop, ppred, extras))
        } else {
            None
        };

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
        let f64t = self.ctx.f64_type();
        let i8t = self.ctx.i8_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        let rec_size = self.store_record_size(&sd);

        let field_of = |fname: &str| -> Result<(usize, crate::types::Type), String> {
            sd.fields
                .iter()
                .enumerate()
                .find(|(_, f)| f.name == fname)
                .map(|(i, f)| (i, f.ty.clone()))
                .ok_or_else(|| format!("no field '{fname}' in store '{store_name}'"))
        };
        let (key_idx, key_ty) = field_of(key_name)?;
        let key_norm = crate::codegen::store_filter::normalize_store_field_type(&key_ty);

        struct AggInfo {
            agg: String,
            val_idx: usize,
            val_is_float: bool,
            result_float: bool,
            is_count: bool,
        }
        let mut aggs: Vec<AggInfo> = Vec::new();
        for (agg, val_name) in &agg_specs {
            let is_count = *agg == "count";
            let (val_idx, val_is_float) = if is_count {
                (0usize, false)
            } else {
                let (i, ty) = field_of(val_name)?;
                let norm = crate::codegen::store_filter::normalize_store_field_type(&ty);
                (
                    i,
                    matches!(norm, crate::types::Type::F64 | crate::types::Type::F32),
                )
            };
            aggs.push(AggInfo {
                agg: agg.to_string(),
                val_idx,
                val_is_float,
                result_float: !is_count && (val_is_float || *agg == "avg"),
                is_count,
            });
        }

        let filter = match &filter_spec {
            Some((pfield, pop, ppred, extra_specs)) => {
                if args.len() < 1 + extra_specs.len() {
                    return Err(format!("malformed store qgroup args: {rest}"));
                }
                let (pidx, pty) = field_of(pfield)?;
                let pval = self.value_map[&args[0]];
                let mut extras = Vec::new();
                for (i, (lop, efield, eop, epred)) in extra_specs.iter().enumerate() {
                    let (eidx, ety) = field_of(efield)?;
                    extras.push((*lop, eidx, ety, *eop, *epred, self.value_map[&args[i + 1]]));
                }
                Some((pidx, pty, *pop, *ppred, pval, extras))
            }
            None => None,
        };

        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted");

        let total_count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, total_count, rec_size)?;

        let calloc_fn = self.ensure_calloc();
        let cap = b!(self.bld.build_int_add(
            b!(self
                .bld
                .build_int_mul(total_count, i64t.const_int(4, false), "qg.cap.mul")),
            i64t.const_int(16, false),
            "qg.cap"
        ));
        let mut arrays = Vec::new();
        for name in ["qg.tbl", "qg.recof", "qg.cnt"] {
            arrays.push(
                self.call_result(b!(self.bld.build_call(
                    calloc_fn,
                    &[cap.into(), i64t.const_int(8, false).into()],
                    name
                )))
                .into_pointer_value(),
            );
        }
        let hash_tbl = arrays[0];
        let rec_of = arrays[1];
        let cnt_arr = arrays[2];
        let mut acc_arrs = Vec::new();
        for i in 0..aggs.len() {
            acc_arrs.push(
                self.call_result(b!(self.bld.build_call(
                    calloc_fn,
                    &[cap.into(), i64t.const_int(8, false).into()],
                    &format!("qg.acc{i}")
                )))
                .into_pointer_value(),
            );
        }

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let idx_ptr = self.entry_alloca(i64t.into(), "qg.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "qg.loop");
        let body_bb = self.ctx.append_basic_block(fv, "qg.body");
        let live_bb = self.ctx.append_basic_block(fv, "qg.live");
        let match_bb = self.ctx.append_basic_block(fv, "qg.match");
        let next_bb = self.ctx.append_basic_block(fv, "qg.next");
        let done_bb = self.ctx.append_basic_block(fv, "qg.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "qg.i")).into_int_value();
        let cmp =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, total_count, "qg.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "qg.off"));
        let rec_ptr = unsafe { b!(self.bld.build_gep(i8t, buf, &[offset], "qg.rec")) };

        if let Some(del_idx) = deleted_idx {
            let del_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "qg.del"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "qg.del.val")).into_int_value();
            let is_live = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                del_val,
                i64t.const_int(0, false),
                "qg.islive"
            ));
            b!(self.bld.build_conditional_branch(is_live, live_bb, next_bb));
        } else {
            b!(self.bld.build_unconditional_branch(live_bb));
        }

        self.bld.position_at_end(live_bb);
        if let Some((pidx, pty, pop, ppred, pval, extras)) = &filter {
            let matched =
                self.eval_store_filter_pred(rec_ptr, st, *pidx, pty, *pop, *ppred, *pval, extras)?;
            b!(self
                .bld
                .build_conditional_branch(matched, match_bb, next_bb));
        } else {
            b!(self.bld.build_unconditional_branch(match_bb));
        }

        self.bld.position_at_end(match_bb);
        let key_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, key_idx as u32, "qg.key"));
        let hash = self.hash_store_field_from_gep(key_gep, &key_ty)?;
        let marked_h = b!(self
            .bld
            .build_or(hash, i64t.const_int(1, false), "qg.marked"));

        let slot_ptr = self.entry_alloca(i64t.into(), "qg.slot");
        let init_slot = b!(self.bld.build_int_unsigned_rem(marked_h, cap, "qg.islot"));
        b!(self.bld.build_store(slot_ptr, init_slot));

        let probe_bb = self.ctx.append_basic_block(fv, "qg.probe");
        let newgrp_bb = self.ctx.append_basic_block(fv, "qg.new");
        let accum_bb = self.ctx.append_basic_block(fv, "qg.accum");
        b!(self.bld.build_unconditional_branch(probe_bb));

        self.bld.position_at_end(probe_bb);
        let slot = b!(self.bld.build_load(i64t, slot_ptr, "qg.s")).into_int_value();
        let entry_ptr = unsafe { b!(self.bld.build_gep(i64t, hash_tbl, &[slot], "qg.ep")) };
        let entry_val = b!(self.bld.build_load(i64t, entry_ptr, "qg.ev")).into_int_value();
        let is_empty = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            entry_val,
            i64t.const_int(0, false),
            "qg.empty"
        ));
        let probe_match_bb = self.ctx.append_basic_block(fv, "qg.pmatch");
        b!(self
            .bld
            .build_conditional_branch(is_empty, newgrp_bb, probe_match_bb));

        self.bld.position_at_end(probe_match_bb);
        let is_match = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            entry_val,
            marked_h,
            "qg.hmatch"
        ));
        let advance_bb = self.ctx.append_basic_block(fv, "qg.advance");
        b!(self
            .bld
            .build_conditional_branch(is_match, accum_bb, advance_bb));

        self.bld.position_at_end(advance_bb);
        let next_slot = b!(self
            .bld
            .build_int_add(slot, i64t.const_int(1, false), "qg.ns"));
        let wrapped = b!(self.bld.build_int_unsigned_rem(next_slot, cap, "qg.wrap"));
        b!(self.bld.build_store(slot_ptr, wrapped));
        b!(self.bld.build_unconditional_branch(probe_bb));

        self.bld.position_at_end(newgrp_bb);
        b!(self.bld.build_store(entry_ptr, marked_h));
        let recof_slot = unsafe { b!(self.bld.build_gep(i64t, rec_of, &[slot], "qg.rofs")) };
        b!(self.bld.build_store(recof_slot, idx));
        for (i, agg) in aggs.iter().enumerate() {
            let acc_init_slot =
                unsafe { b!(self.bld.build_gep(i64t, acc_arrs[i], &[slot], "qg.ais")) };
            if agg.result_float {
                let zero = f64t.const_float(0.0);
                let bits = b!(self.bld.build_bit_cast(zero, i64t, "qg.zbits"));
                b!(self.bld.build_store(acc_init_slot, bits));
            } else {
                b!(self
                    .bld
                    .build_store(acc_init_slot, i64t.const_int(0, false)));
            }
        }
        b!(self.bld.build_unconditional_branch(accum_bb));

        self.bld.position_at_end(accum_bb);
        let cnt_slot = unsafe { b!(self.bld.build_gep(i64t, cnt_arr, &[slot], "qg.cns")) };
        let cur_cnt = b!(self.bld.build_load(i64t, cnt_slot, "qg.ccnt")).into_int_value();
        let new_cnt = b!(self
            .bld
            .build_int_add(cur_cnt, i64t.const_int(1, false), "qg.cntinc"));
        b!(self.bld.build_store(cnt_slot, new_cnt));

        for (i, agg) in aggs.iter().enumerate() {
            if agg.is_count {
                continue;
            }
            let acc_slot = unsafe { b!(self.bld.build_gep(i64t, acc_arrs[i], &[slot], "qg.acs")) };
            let val_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, agg.val_idx as u32, "qg.vgep"));
            let is_first = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                cur_cnt,
                i64t.const_int(0, false),
                "qg.first"
            ));
            if agg.result_float {
                let v = if agg.val_is_float {
                    b!(self.bld.build_load(f64t, val_gep, "qg.vf")).into_float_value()
                } else {
                    let iv = b!(self.bld.build_load(i64t, val_gep, "qg.vi")).into_int_value();
                    b!(self.bld.build_signed_int_to_float(iv, f64t, "qg.vi2f"))
                };
                let cur_bits = b!(self.bld.build_load(i64t, acc_slot, "qg.acb")).into_int_value();
                let cur = b!(self.bld.build_bit_cast(cur_bits, f64t, "qg.acf")).into_float_value();
                let combined = match agg.agg.as_str() {
                    "min" => {
                        let lt = b!(self.bld.build_float_compare(
                            inkwell::FloatPredicate::OLT,
                            v,
                            cur,
                            "qg.fmin"
                        ));
                        b!(self.bld.build_select(lt, v, cur, "qg.fminsel")).into_float_value()
                    }
                    "max" => {
                        let gt = b!(self.bld.build_float_compare(
                            inkwell::FloatPredicate::OGT,
                            v,
                            cur,
                            "qg.fmax"
                        ));
                        b!(self.bld.build_select(gt, v, cur, "qg.fmaxsel")).into_float_value()
                    }
                    _ => b!(self.bld.build_float_add(cur, v, "qg.fadd")),
                };
                let result = if matches!(agg.agg.as_str(), "min" | "max") {
                    b!(self.bld.build_select(is_first, v, combined, "qg.firstsel"))
                        .into_float_value()
                } else {
                    combined
                };
                let rbits = b!(self.bld.build_bit_cast(result, i64t, "qg.rbits"));
                b!(self.bld.build_store(acc_slot, rbits));
            } else {
                let v = b!(self.bld.build_load(i64t, val_gep, "qg.vi")).into_int_value();
                let cur = b!(self.bld.build_load(i64t, acc_slot, "qg.aci")).into_int_value();
                let combined = match agg.agg.as_str() {
                    "min" => {
                        let lt = b!(self.bld.build_int_compare(
                            inkwell::IntPredicate::SLT,
                            v,
                            cur,
                            "qg.imin"
                        ));
                        b!(self.bld.build_select(lt, v, cur, "qg.iminsel")).into_int_value()
                    }
                    "max" => {
                        let gt = b!(self.bld.build_int_compare(
                            inkwell::IntPredicate::SGT,
                            v,
                            cur,
                            "qg.imax"
                        ));
                        b!(self.bld.build_select(gt, v, cur, "qg.imaxsel")).into_int_value()
                    }
                    _ => b!(self.bld.build_int_add(cur, v, "qg.iadd")),
                };
                let result = if matches!(agg.agg.as_str(), "min" | "max") {
                    b!(self.bld.build_select(is_first, v, combined, "qg.ifirstsel"))
                        .into_int_value()
                } else {
                    combined
                };
                b!(self.bld.build_store(acc_slot, result));
            }
        }
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "qg.ni"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);

        let key_lty = self.llvm_ty(&key_norm);
        let mut elem_ltys: Vec<inkwell::types::BasicTypeEnum<'ctx>> = vec![key_lty];
        for agg in &aggs {
            if agg.result_float {
                elem_ltys.push(f64t.into());
            } else {
                elem_ltys.push(i64t.into());
            }
        }
        let tuple_ty = self.ctx.struct_type(&elem_ltys, false);
        let tuple_size = self.type_store_size(tuple_ty.into());

        let header_ty = self.vec_header_type();
        let malloc_fn = self.ensure_malloc();
        let result_vec = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[i64t.const_int(24, false).into()],
                "qg.vec"
            )))
            .into_pointer_value();
        let v_dgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 0, "qg.vec.d"));
        b!(self.bld.build_store(v_dgep, ptr_ty.const_null()));
        let v_lgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 1, "qg.vec.l"));
        b!(self.bld.build_store(v_lgep, i64t.const_int(0, false)));
        let v_cgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 2, "qg.vec.c"));
        b!(self.bld.build_store(v_cgep, i64t.const_int(0, false)));

        let sidx_ptr = self.entry_alloca(i64t.into(), "qg.sidx");
        b!(self.bld.build_store(sidx_ptr, i64t.const_int(0, false)));

        let oloop_bb = self.ctx.append_basic_block(fv, "qg.oloop");
        let obody_bb = self.ctx.append_basic_block(fv, "qg.obody");
        let oused_bb = self.ctx.append_basic_block(fv, "qg.oused");
        let onext_bb = self.ctx.append_basic_block(fv, "qg.onext");
        let odone_bb = self.ctx.append_basic_block(fv, "qg.odone");

        b!(self.bld.build_unconditional_branch(oloop_bb));

        self.bld.position_at_end(oloop_bb);
        let sidx = b!(self.bld.build_load(i64t, sidx_ptr, "qg.osi")).into_int_value();
        let ocmp = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, sidx, cap, "qg.ocmp"));
        b!(self.bld.build_conditional_branch(ocmp, obody_bb, odone_bb));

        self.bld.position_at_end(obody_bb);
        let oentry_ptr = unsafe { b!(self.bld.build_gep(i64t, hash_tbl, &[sidx], "qg.oep")) };
        let oentry = b!(self.bld.build_load(i64t, oentry_ptr, "qg.oev")).into_int_value();
        let oused = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            oentry,
            i64t.const_int(0, false),
            "qg.oused"
        ));
        b!(self.bld.build_conditional_branch(oused, oused_bb, onext_bb));

        self.bld.position_at_end(oused_bb);
        let recof_ld = unsafe { b!(self.bld.build_gep(i64t, rec_of, &[sidx], "qg.orof")) };
        let rec_i = b!(self.bld.build_load(i64t, recof_ld, "qg.oreci")).into_int_value();
        let roffset = b!(self
            .bld
            .build_int_mul(rec_i, i64t.const_int(rec_size, false), "qg.ooff"));
        let orec_ptr = unsafe { b!(self.bld.build_gep(i8t, buf, &[roffset], "qg.orec")) };
        let okey_gep = b!(self
            .bld
            .build_struct_gep(st, orec_ptr, key_idx as u32, "qg.okey"));
        let key_val = match key_norm {
            crate::types::Type::String => self.read_string_from_fixed_buf(okey_gep)?,
            ref nty => {
                let lty = self.llvm_ty(nty);
                b!(self.bld.build_load(lty, okey_gep, "qg.okv"))
            }
        };

        let cnt_ld = unsafe { b!(self.bld.build_gep(i64t, cnt_arr, &[sidx], "qg.ocnt")) };
        let cnt_val = b!(self.bld.build_load(i64t, cnt_ld, "qg.ocntv")).into_int_value();

        let mut tuple_val = tuple_ty.get_undef();
        tuple_val = b!(self
            .bld
            .build_insert_value(tuple_val, key_val, 0, "qg.ins0"))
        .into_struct_value();
        for (i, agg) in aggs.iter().enumerate() {
            let acc_ld = unsafe { b!(self.bld.build_gep(i64t, acc_arrs[i], &[sidx], "qg.oacc")) };
            let out_val: BasicValueEnum<'ctx> = if agg.is_count {
                cnt_val.into()
            } else if agg.result_float {
                let abits = b!(self.bld.build_load(i64t, acc_ld, "qg.oab")).into_int_value();
                let af = b!(self.bld.build_bit_cast(abits, f64t, "qg.oaf")).into_float_value();
                if agg.agg == "avg" {
                    let cntf = b!(self.bld.build_signed_int_to_float(cnt_val, f64t, "qg.cntf"));
                    b!(self.bld.build_float_div(af, cntf, "qg.avg")).into()
                } else {
                    af.into()
                }
            } else {
                b!(self.bld.build_load(i64t, acc_ld, "qg.oai"))
            };
            tuple_val =
                b!(self
                    .bld
                    .build_insert_value(tuple_val, out_val, (i + 1) as u32, "qg.ins"))
                .into_struct_value();
        }
        self.vec_push_raw(result_vec, tuple_val.into(), tuple_ty.into(), tuple_size)?;
        b!(self.bld.build_unconditional_branch(onext_bb));

        self.bld.position_at_end(onext_bb);
        let osnext = b!(self
            .bld
            .build_int_add(sidx, i64t.const_int(1, false), "qg.osn"));
        b!(self.bld.build_store(sidx_ptr, osnext));
        b!(self.bld.build_unconditional_branch(oloop_bb));

        self.bld.position_at_end(odone_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        b!(self.bld.build_call(free_fn, &[hash_tbl.into()], ""));
        b!(self.bld.build_call(free_fn, &[rec_of.into()], ""));
        for acc in &acc_arrs {
            b!(self.bld.build_call(free_fn, &[(*acc).into()], ""));
        }
        b!(self.bld.build_call(free_fn, &[cnt_arr.into()], ""));

        Ok(result_vec.into())
    }
}
