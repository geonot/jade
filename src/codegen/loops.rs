use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_while(
        &mut self,
        w: &hir::While,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let cond_bb = self.ctx.append_basic_block(fv, "wh.cond");
        let body_bb = self.ctx.append_basic_block(fv, "wh.body");
        let end_bb = self.ctx.append_basic_block(fv, "wh.end");
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let wv = self.compile_expr(&w.cond)?;
        let c = self.to_bool(wv);
        b!(self.bld.build_conditional_branch(c, body_bb, end_bb));
        self.bld.position_at_end(body_bb);
        self.loop_stack.push(super::LoopCtx {
            continue_bb: cond_bb,
            break_bb: end_bb,
        });
        self.compile_block(&w.body)?;
        self.loop_stack.pop();
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(cond_bb));
        }
        self.bld.position_at_end(end_bb);
        Ok(None)
    }

    pub(crate) fn compile_for(
        &mut self,
        f: &hir::For,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        if f.end.is_none() && f.step.is_none() {
            if let Type::Array(ref elem_ty, len) = f.bind_ty {
                return self.compile_for_array(f, elem_ty, len);
            }
            let iter_ty = &f.iter.ty;
            if let Type::Array(elem_ty, len) = iter_ty {
                return self.compile_for_array(f, elem_ty, *len);
            }
            if let Type::Vec(ref elem_ty) = f.bind_ty {
                return self.compile_for_vec(f, elem_ty);
            }
            if let Type::Vec(elem_ty) = iter_ty {
                return self.compile_for_vec(f, elem_ty);
            }
            if matches!(iter_ty, Type::String)
                || matches!(f.bind_ty, Type::I64) && matches!(iter_ty, Type::String)
            {
                return self.compile_for_string(f);
            }
        }
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let start_val = if f.end.is_some() {
            self.compile_expr(&f.iter)?
        } else {
            i64t.const_int(0, false).into()
        };
        let end_val = if let Some(end) = &f.end {
            self.compile_expr(end)?
        } else {
            self.compile_expr(&f.iter)?
        };
        let step_val = if let Some(step) = &f.step {
            self.compile_expr(step)?
        } else {
            i64t.const_int(1, false).into()
        };
        let a = self.entry_alloca(i64t.into(), &f.bind);
        b!(self.bld.build_store(a, start_val));
        self.set_var(&f.bind, a, Type::I64);
        let cond_bb = self.ctx.append_basic_block(fv, "for.cond");
        let body_bb = self.ctx.append_basic_block(fv, "for.body");
        let inc_bb = self.ctx.append_basic_block(fv, "for.inc");
        let end_bb = self.ctx.append_basic_block(fv, "for.end");
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let cur = b!(self.bld.build_load(i64t, a, "i"));
        let cmp = b!(self.bld.build_int_compare(
            IntPredicate::SLT,
            cur.into_int_value(),
            end_val.into_int_value(),
            "for.cmp"
        ));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));
        self.bld.position_at_end(body_bb);
        self.loop_stack.push(super::LoopCtx {
            continue_bb: inc_bb,
            break_bb: end_bb,
        });
        self.compile_block(&f.body)?;
        self.loop_stack.pop();
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(inc_bb));
        }
        self.bld.position_at_end(inc_bb);
        let cur = b!(self.bld.build_load(i64t, a, "i"));
        let next =
            b!(self
                .bld
                .build_int_nsw_add(cur.into_int_value(), step_val.into_int_value(), "inc"));
        b!(self.bld.build_store(a, next));
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(end_bb);
        Ok(None)
    }

    fn compile_for_array(
        &mut self,
        f: &hir::For,
        elem_ty: &Type,
        len: usize,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let i64t = self.ctx.i64_type();
        let arr_ptr = match &f.iter.kind {
            hir::ExprKind::Var(_, n) => self
                .find_var(n)
                .map(|(p, _)| *p)
                .ok_or_else(|| format!("undefined: {n}"))?,
            _ => self.compile_expr(&f.iter)?.into_pointer_value(),
        };
        let lty = self.llvm_ty(elem_ty);
        let arr_ty = lty.array_type(len as u32);
        let count = i64t.const_int(len as u64, false);
        self.compile_for_indexed(f, elem_ty, arr_ptr, count, Some(arr_ty), "for")
    }

    fn compile_for_vec(
        &mut self,
        f: &hir::For,
        elem_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let i64t = self.ctx.i64_type();
        let vec_val = self.compile_expr(&f.iter)?;
        let vec_ptr = vec_val.into_pointer_value();
        let header_ty = self.vec_header_type();

        let len_gep = b!(self.bld.build_struct_gep(header_ty, vec_ptr, 1, "fv.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "fv.len")).into_int_value();
        let ptr_gep = b!(self.bld.build_struct_gep(header_ty, vec_ptr, 0, "fv.ptrp"));
        let data_ptr = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "fv.data"
        ))
        .into_pointer_value();

        self.compile_for_indexed(f, elem_ty, data_ptr, len, None, "fv")
    }

    fn compile_for_indexed(
        &mut self,
        f: &hir::For,
        elem_ty: &Type,
        data_ptr: inkwell::values::PointerValue<'ctx>,
        len: inkwell::values::IntValue<'ctx>,
        array_gep_ty: Option<inkwell::types::ArrayType<'ctx>>,
        prefix: &str,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);

        let idx_alloca = self.entry_alloca(i64t.into(), "__idx");
        b!(self.bld.build_store(idx_alloca, i64t.const_int(0, false)));
        let elem_alloca = self.entry_alloca(lty, &f.bind);
        self.set_var(&f.bind, elem_alloca, elem_ty.clone());

        let cond_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.cond"));
        let body_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.body"));
        let inc_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.inc"));
        let end_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.end"));
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(cond_bb);
        let idx = b!(self.bld.build_load(i64t, idx_alloca, "idx")).into_int_value();
        let cmp =
            b!(self
                .bld
                .build_int_compare(IntPredicate::ULT, idx, len, &format!("{prefix}.cmp")));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = if let Some(arr_ty) = array_gep_ty {
            unsafe {
                b!(self.bld.build_gep(
                    arr_ty,
                    data_ptr,
                    &[i64t.const_int(0, false), idx],
                    "elem.ptr"
                ))
            }
        } else {
            unsafe {
                b!(self
                    .bld
                    .build_gep(lty, data_ptr, &[idx], &format!("{prefix}.egep")))
            }
        };
        let elem = b!(self.bld.build_load(lty, elem_gep, "elem"));
        b!(self.bld.build_store(elem_alloca, elem));

        self.loop_stack.push(super::LoopCtx {
            continue_bb: inc_bb,
            break_bb: end_bb,
        });
        self.compile_block(&f.body)?;
        self.loop_stack.pop();
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(inc_bb));
        }

        self.bld.position_at_end(inc_bb);
        let idx = b!(self.bld.build_load(i64t, idx_alloca, "idx")).into_int_value();
        let next = b!(self
            .bld
            .build_int_nuw_add(idx, i64t.const_int(1, false), "inc"));
        b!(self.bld.build_store(idx_alloca, next));
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(end_bb);
        Ok(None)
    }

    fn compile_for_string(&mut self, f: &hir::For) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let i8t = self.ctx.i8_type();
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        let str_val = self.compile_expr(&f.iter)?;

        let str_ptr = self.string_data(str_val)?.into_pointer_value();
        let len = self.string_len(str_val)?.into_int_value();

        let idx_alloca = self.entry_alloca(i64t.into(), "__sidx");
        b!(self.bld.build_store(idx_alloca, i64t.const_int(0, false)));

        let cp_alloca = self.entry_alloca(i64t.into(), &f.bind);
        self.set_var(&f.bind, cp_alloca, crate::types::Type::I64);

        let cond_bb = self.ctx.append_basic_block(fv, "fs.cond");
        let body_bb = self.ctx.append_basic_block(fv, "fs.body");
        let end_bb = self.ctx.append_basic_block(fv, "fs.end");
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(cond_bb);
        let idx = b!(self.bld.build_load(i64t, idx_alloca, "idx")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(IntPredicate::ULT, idx, len, "fs.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));

        self.bld.position_at_end(body_bb);
        let idx = b!(self.bld.build_load(i64t, idx_alloca, "idx")).into_int_value();

        let byte_ptr = unsafe { b!(self.bld.build_gep(i8t, str_ptr, &[idx], "fs.bp")) };
        let b0 = b!(self.bld.build_load(i8t, byte_ptr, "fs.b0")).into_int_value();
        let b0_32 = b!(self.bld.build_int_z_extend(b0, i32t, "b0z"));

        let is_ascii = b!(self.bld.build_int_compare(
            IntPredicate::ULT,
            b0_32,
            i32t.const_int(0x80, false),
            "is_ascii"
        ));

        let ascii_bb = self.ctx.append_basic_block(fv, "fs.ascii");
        let multi_bb = self.ctx.append_basic_block(fv, "fs.multi");
        let merge_bb = self.ctx.append_basic_block(fv, "fs.merge");
        let trunc_bb = self.ctx.append_basic_block(fv, "fs.trunc");
        b!(self
            .bld
            .build_conditional_branch(is_ascii, ascii_bb, multi_bb));

        self.bld.position_at_end(trunc_bb);
        let cp_trunc = i64t.const_int(0xFFFD, false);
        let adv_trunc = i64t.const_int(1, false);
        b!(self.bld.build_unconditional_branch(merge_bb));
        let trunc_bb_end = self.bld.get_insert_block().unwrap();

        self.bld.position_at_end(ascii_bb);
        let cp_ascii = b!(self.bld.build_int_z_extend(b0_32, i64t, "cp_a"));
        let adv_ascii = i64t.const_int(1, false);
        b!(self.bld.build_unconditional_branch(merge_bb));
        let ascii_bb_end = self.bld.get_insert_block().unwrap();

        self.bld.position_at_end(multi_bb);
        let is_2byte = b!(self.bld.build_int_compare(
            IntPredicate::ULT,
            b0_32,
            i32t.const_int(0xE0, false),
            "is_2b"
        ));

        let two_bb = self.ctx.append_basic_block(fv, "fs.2b");
        let three_plus_bb = self.ctx.append_basic_block(fv, "fs.3p");
        b!(self
            .bld
            .build_conditional_branch(is_2byte, two_bb, three_plus_bb));

        self.bld.position_at_end(two_bb);
        let need_2 = b!(self
            .bld
            .build_int_nuw_add(idx, i64t.const_int(2, false), "n2"));
        let ok_2 = b!(self
            .bld
            .build_int_compare(IntPredicate::ULE, need_2, len, "ok2"));
        let two_ok_bb = self.ctx.append_basic_block(fv, "fs.2ok");
        b!(self.bld.build_conditional_branch(ok_2, two_ok_bb, trunc_bb));
        self.bld.position_at_end(two_ok_bb);
        let idx_plus1 = b!(self
            .bld
            .build_int_nuw_add(idx, i64t.const_int(1, false), "i1"));
        let bp1 = unsafe { b!(self.bld.build_gep(i8t, str_ptr, &[idx_plus1], "bp1")) };
        let b1_raw = b!(self.bld.build_load(i8t, bp1, "b1r")).into_int_value();
        let b1_32 = b!(self.bld.build_int_z_extend(b1_raw, i32t, "b1z"));
        let hi = b!(self
            .bld
            .build_and(b0_32, i32t.const_int(0x1F, false), "hi2"));
        let hi_s = b!(self
            .bld
            .build_left_shift(hi, i32t.const_int(6, false), "hi2s"));
        let lo = b!(self
            .bld
            .build_and(b1_32, i32t.const_int(0x3F, false), "lo2"));
        let cp_2b_32 = b!(self.bld.build_or(hi_s, lo, "cp2b"));
        let cp_2b = b!(self.bld.build_int_z_extend(cp_2b_32, i64t, "cp2"));
        let adv_2 = i64t.const_int(2, false);
        b!(self.bld.build_unconditional_branch(merge_bb));
        let two_bb_end = self.bld.get_insert_block().unwrap();

        self.bld.position_at_end(three_plus_bb);
        let is_3byte = b!(self.bld.build_int_compare(
            IntPredicate::ULT,
            b0_32,
            i32t.const_int(0xF0, false),
            "is_3b"
        ));
        let three_bb = self.ctx.append_basic_block(fv, "fs.3b");
        let four_bb = self.ctx.append_basic_block(fv, "fs.4b");
        b!(self
            .bld
            .build_conditional_branch(is_3byte, three_bb, four_bb));

        self.bld.position_at_end(three_bb);
        let need_3 = b!(self
            .bld
            .build_int_nuw_add(idx, i64t.const_int(3, false), "n3"));
        let ok_3 = b!(self
            .bld
            .build_int_compare(IntPredicate::ULE, need_3, len, "ok3"));
        let three_ok_bb = self.ctx.append_basic_block(fv, "fs.3ok");
        b!(self
            .bld
            .build_conditional_branch(ok_3, three_ok_bb, trunc_bb));
        self.bld.position_at_end(three_ok_bb);
        let idx_p1 = b!(self
            .bld
            .build_int_nuw_add(idx, i64t.const_int(1, false), "3i1"));
        let idx_p2 = b!(self
            .bld
            .build_int_nuw_add(idx, i64t.const_int(2, false), "3i2"));
        let bp3_1 = unsafe { b!(self.bld.build_gep(i8t, str_ptr, &[idx_p1], "3bp1")) };
        let bp3_2 = unsafe { b!(self.bld.build_gep(i8t, str_ptr, &[idx_p2], "3bp2")) };
        let b3_1 = b!(self.bld.build_load(i8t, bp3_1, "3b1")).into_int_value();
        let b3_2 = b!(self.bld.build_load(i8t, bp3_2, "3b2")).into_int_value();
        let b3_1_32 = b!(self.bld.build_int_z_extend(b3_1, i32t, "3b1z"));
        let b3_2_32 = b!(self.bld.build_int_z_extend(b3_2, i32t, "3b2z"));
        let h3 = b!(self.bld.build_and(b0_32, i32t.const_int(0x0F, false), "h3"));
        let h3s = b!(self
            .bld
            .build_left_shift(h3, i32t.const_int(12, false), "h3s"));
        let m3 = b!(self
            .bld
            .build_and(b3_1_32, i32t.const_int(0x3F, false), "m3"));
        let m3s = b!(self
            .bld
            .build_left_shift(m3, i32t.const_int(6, false), "m3s"));
        let l3 = b!(self
            .bld
            .build_and(b3_2_32, i32t.const_int(0x3F, false), "l3"));
        let cp3_1 = b!(self.bld.build_or(h3s, m3s, "cp3a"));
        let cp3_32 = b!(self.bld.build_or(cp3_1, l3, "cp3b"));
        let cp_3b = b!(self.bld.build_int_z_extend(cp3_32, i64t, "cp3"));
        let adv_3 = i64t.const_int(3, false);
        b!(self.bld.build_unconditional_branch(merge_bb));
        let three_bb_end = self.bld.get_insert_block().unwrap();

        self.bld.position_at_end(four_bb);
        let need_4 = b!(self
            .bld
            .build_int_nuw_add(idx, i64t.const_int(4, false), "n4"));
        let ok_4 = b!(self
            .bld
            .build_int_compare(IntPredicate::ULE, need_4, len, "ok4"));
        let four_ok_bb = self.ctx.append_basic_block(fv, "fs.4ok");
        b!(self
            .bld
            .build_conditional_branch(ok_4, four_ok_bb, trunc_bb));
        self.bld.position_at_end(four_ok_bb);
        let idx_p1 = b!(self
            .bld
            .build_int_nuw_add(idx, i64t.const_int(1, false), "4i1"));
        let idx_p2 = b!(self
            .bld
            .build_int_nuw_add(idx, i64t.const_int(2, false), "4i2"));
        let idx_p3 = b!(self
            .bld
            .build_int_nuw_add(idx, i64t.const_int(3, false), "4i3"));
        let bp4_1 = unsafe { b!(self.bld.build_gep(i8t, str_ptr, &[idx_p1], "4bp1")) };
        let bp4_2 = unsafe { b!(self.bld.build_gep(i8t, str_ptr, &[idx_p2], "4bp2")) };
        let bp4_3 = unsafe { b!(self.bld.build_gep(i8t, str_ptr, &[idx_p3], "4bp3")) };
        let b4_1 = b!(self.bld.build_load(i8t, bp4_1, "4b1")).into_int_value();
        let b4_2 = b!(self.bld.build_load(i8t, bp4_2, "4b2")).into_int_value();
        let b4_3 = b!(self.bld.build_load(i8t, bp4_3, "4b3")).into_int_value();
        let b4_1_32 = b!(self.bld.build_int_z_extend(b4_1, i32t, "4b1z"));
        let b4_2_32 = b!(self.bld.build_int_z_extend(b4_2, i32t, "4b2z"));
        let b4_3_32 = b!(self.bld.build_int_z_extend(b4_3, i32t, "4b3z"));
        let h4 = b!(self.bld.build_and(b0_32, i32t.const_int(0x07, false), "h4"));
        let h4s = b!(self
            .bld
            .build_left_shift(h4, i32t.const_int(18, false), "h4s"));
        let m4a = b!(self
            .bld
            .build_and(b4_1_32, i32t.const_int(0x3F, false), "m4a"));
        let m4as = b!(self
            .bld
            .build_left_shift(m4a, i32t.const_int(12, false), "m4as"));
        let m4b = b!(self
            .bld
            .build_and(b4_2_32, i32t.const_int(0x3F, false), "m4b"));
        let m4bs = b!(self
            .bld
            .build_left_shift(m4b, i32t.const_int(6, false), "m4bs"));
        let l4 = b!(self
            .bld
            .build_and(b4_3_32, i32t.const_int(0x3F, false), "l4"));
        let cp4_1 = b!(self.bld.build_or(h4s, m4as, "cp4a"));
        let cp4_2 = b!(self.bld.build_or(cp4_1, m4bs, "cp4b"));
        let cp4_32 = b!(self.bld.build_or(cp4_2, l4, "cp4c"));
        let cp_4b = b!(self.bld.build_int_z_extend(cp4_32, i64t, "cp4"));
        let adv_4 = i64t.const_int(4, false);
        b!(self.bld.build_unconditional_branch(merge_bb));
        let four_bb_end = self.bld.get_insert_block().unwrap();

        self.bld.position_at_end(merge_bb);
        let cp_phi = b!(self.bld.build_phi(i64t, "cp"));
        cp_phi.add_incoming(&[
            (&cp_ascii, ascii_bb_end),
            (&cp_2b, two_bb_end),
            (&cp_3b, three_bb_end),
            (&cp_4b, four_bb_end),
            (&cp_trunc, trunc_bb_end),
        ]);
        let adv_phi = b!(self.bld.build_phi(i64t, "adv"));
        adv_phi.add_incoming(&[
            (&adv_ascii, ascii_bb_end),
            (&adv_2, two_bb_end),
            (&adv_3, three_bb_end),
            (&adv_4, four_bb_end),
            (&adv_trunc, trunc_bb_end),
        ]);

        b!(self.bld.build_store(cp_alloca, cp_phi.as_basic_value()));

        let cur_idx = b!(self.bld.build_load(i64t, idx_alloca, "cidx")).into_int_value();
        let new_idx = b!(self.bld.build_int_nuw_add(
            cur_idx,
            adv_phi.as_basic_value().into_int_value(),
            "nidx"
        ));
        b!(self.bld.build_store(idx_alloca, new_idx));

        self.loop_stack.push(super::LoopCtx {
            continue_bb: cond_bb,
            break_bb: end_bb,
        });
        self.compile_block(&f.body)?;
        self.loop_stack.pop();
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(cond_bb));
        }

        self.bld.position_at_end(end_bb);
        Ok(None)
    }

    pub(crate) fn compile_loop(
        &mut self,
        l: &hir::Loop,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let body_bb = self.ctx.append_basic_block(fv, "loop");
        let end_bb = self.ctx.append_basic_block(fv, "loop.end");
        b!(self.bld.build_unconditional_branch(body_bb));
        self.bld.position_at_end(body_bb);
        self.loop_stack.push(super::LoopCtx {
            continue_bb: body_bb,
            break_bb: end_bb,
        });
        self.compile_block(&l.body)?;
        self.loop_stack.pop();
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(body_bb));
        }
        self.bld.position_at_end(end_bb);
        Ok(None)
    }

    pub(crate) fn compile_sim_for(
        &mut self,
        f: &hir::For,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        // sim for: currently compiled as a sequential for loop
        // A full implementation would spawn each iteration as a coroutine
        self.compile_for(f)
    }
}
