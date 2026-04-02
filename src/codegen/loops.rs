use inkwell::module::Linkage;
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
            if matches!(iter_ty, Type::Coroutine(_)) {
                return self.compile_for_coroutine(f);
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

    /// for x in gen — iterate over a generator/dispatch using direct context swap.
    /// Calls jade_gen_resume to get each value, breaks when done.
    fn compile_for_coroutine(
        &mut self,
        f: &hir::For,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        self.declare_gen_runtime();
        let fv = self.cur_fn.unwrap();
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();

        let gen_ptr = self.compile_expr(&f.iter)?.into_pointer_value();

        let loop_bb = self.ctx.append_basic_block(fv, "forgen.loop");
        let body_bb = self.ctx.append_basic_block(fv, "forgen.body");
        let end_bb = self.ctx.append_basic_block(fv, "forgen.end");

        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(loop_bb);

        // Resume the generator
        let gen_resume = self.module.get_function("jade_gen_resume").unwrap();
        b!(self
            .bld
            .build_call(gen_resume, &[gen_ptr.into()], ""));

        // Check if done
        let done_ptr = self.gen_field_ptr(gen_ptr, Self::GEN_DONE_OFF, "forgen.done")?;
        let done_val = b!(self.bld.build_load(i8t, done_ptr, "done")).into_int_value();
        let is_done = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            done_val,
            i8t.const_int(0, false),
            "is_done"
        ));
        b!(self.bld.build_conditional_branch(is_done, end_bb, body_bb));

        // Read the yielded value and bind it
        self.bld.position_at_end(body_bb);
        let value_ptr = self.gen_field_ptr(gen_ptr, Self::GEN_VALUE_OFF, "forgen.val")?;
        let value = b!(self.bld.build_load(i64t, value_ptr, "yielded"));

        // Clear has_value
        let hv_ptr = self.gen_field_ptr(gen_ptr, Self::GEN_HAS_VALUE_OFF, "forgen.hv")?;
        b!(self.bld.build_store(hv_ptr, i8t.const_int(0, false)));

        // Bind the loop variable
        let a = self.entry_alloca(i64t.into(), &f.bind);
        b!(self.bld.build_store(a, value));
        self.set_var(&f.bind, a, f.bind_ty.clone());

        self.loop_stack.push(super::LoopCtx {
            continue_bb: loop_bb,
            break_bb: end_bb,
        });
        self.compile_block(&f.body)?;
        self.loop_stack.pop();

        if self.no_term() {
            b!(self.bld.build_unconditional_branch(loop_bb));
        }

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
        // sim for: spawn each iteration as a coroutine, wait for all to finish.
        //
        // Layout of per-iteration arg struct passed to each coroutine:
        //   offset 0: iter_val  (i64)   — the loop variable value
        //   offset 8: counter   (*i64)  — pointer to shared atomic counter
        // Total: 16 bytes
        //
        // Counter is decremented (atomically) by each coroutine on completion.
        // Main code spins/yields until counter reaches 0.

        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let void = self.ctx.void_type();
        let fv = self.cur_fn.unwrap();

        // Compute range bounds
        let start_val = if f.end.is_some() {
            self.compile_expr(&f.iter)?.into_int_value()
        } else {
            i64t.const_int(0, false)
        };
        let end_val = if let Some(end) = &f.end {
            self.compile_expr(end)?.into_int_value()
        } else {
            self.compile_expr(&f.iter)?.into_int_value()
        };
        let step_val = if let Some(step) = &f.step {
            self.compile_expr(step)?.into_int_value()
        } else {
            i64t.const_int(1, false)
        };

        // Build iteration body function: void __sim_iter_N(void *arg)
        static SIM_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let sim_id = SIM_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let iter_fn_name = format!("__sim_iter_{sim_id}");
        let iter_fn_ty = void.fn_type(&[ptr.into()], false);
        let iter_fn = self
            .module
            .add_function(&iter_fn_name, iter_fn_ty, Some(Linkage::Internal));

        let saved_fn = self.cur_fn;
        let saved_bb = self.bld.get_insert_block();
        let saved_vars = std::mem::replace(&mut self.vars, vec![std::collections::HashMap::new()]);
        let saved_loop_stack = std::mem::replace(&mut self.loop_stack, Vec::new());

        self.cur_fn = Some(iter_fn);
        let entry = self.ctx.append_basic_block(iter_fn, "entry");
        self.bld.position_at_end(entry);

        let arg_ptr = iter_fn.get_first_param().unwrap().into_pointer_value();

        // Load iter_val from arg[0]
        let iter_val_ptr = arg_ptr; // offset 0
        let iter_val = b!(self.bld.build_load(i64t, iter_val_ptr, "iter_val"));

        // Load counter ptr from arg[8]
        let counter_ptr_ptr = unsafe {
            b!(self.bld.build_gep(
                self.ctx.i8_type(),
                arg_ptr,
                &[i64t.const_int(8, false)],
                "counter_pp"
            ))
        };
        let counter_ptr = b!(self.bld.build_load(ptr, counter_ptr_ptr, "counter_ptr")).into_pointer_value();

        // Set up the loop variable
        let lvar = self.entry_alloca(i64t.into(), &f.bind);
        b!(self.bld.build_store(lvar, iter_val));
        self.set_var(&f.bind, lvar, Type::I64);

        // Compile the loop body
        self.compile_block(&f.body)?;

        // Atomically decrement counter
        if self.no_term() {
            b!(self.bld.build_atomicrmw(
                inkwell::AtomicRMWBinOp::Sub,
                counter_ptr,
                i64t.const_int(1, false),
                inkwell::AtomicOrdering::AcquireRelease,
            ));
            // Free the arg struct
            let free_fn = self.module.get_function("free").unwrap();
            b!(self.bld.build_call(free_fn, &[arg_ptr.into()], ""));
            b!(self.bld.build_return(None));
        }

        // Restore caller context
        self.cur_fn = saved_fn;
        self.vars = saved_vars;
        self.loop_stack = saved_loop_stack;

        let bb = saved_bb.unwrap_or_else(|| self.ctx.append_basic_block(fv, "sim.after"));
        self.bld.position_at_end(bb);

        // Allocate atomic counter
        let counter_alloca = self.entry_alloca(i64t.into(), "sim.counter");
        b!(self.bld.build_store(counter_alloca, i64t.const_int(0, false)));

        let malloc_fn = self.ensure_malloc();
        let coro_create = self.module.get_function("jade_coro_create").unwrap();
        let sched_spawn = self.module.get_function("jade_sched_spawn").unwrap();

        // Spawn loop: for i in start..end step step
        let spawn_var = self.entry_alloca(i64t.into(), "sim.i");
        b!(self.bld.build_store(spawn_var, start_val));

        let spawn_cond = self.ctx.append_basic_block(fv, "sim.spawn.cond");
        let spawn_body = self.ctx.append_basic_block(fv, "sim.spawn.body");
        let spawn_inc = self.ctx.append_basic_block(fv, "sim.spawn.inc");
        let spawn_done = self.ctx.append_basic_block(fv, "sim.spawn.done");

        b!(self.bld.build_unconditional_branch(spawn_cond));
        self.bld.position_at_end(spawn_cond);
        let cur_i = b!(self.bld.build_load(i64t, spawn_var, "si")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(IntPredicate::SLT, cur_i, end_val, "sim.cmp"));
        b!(self.bld.build_conditional_branch(cmp, spawn_body, spawn_done));

        self.bld.position_at_end(spawn_body);

        // Increment counter atomically
        b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Add,
            counter_alloca,
            i64t.const_int(1, false),
            inkwell::AtomicOrdering::AcquireRelease,
        ));

        // Allocate arg struct (16 bytes: i64 iter_val, ptr counter)
        let arg_mem = b!(self.bld.build_call(
            malloc_fn,
            &[i64t.const_int(16, false).into()],
            "sim.arg"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        // Store iter_val at offset 0
        b!(self.bld.build_store(arg_mem, cur_i));
        // Store counter_ptr at offset 8
        let counter_field = unsafe {
            b!(self.bld.build_gep(
                self.ctx.i8_type(),
                arg_mem,
                &[i64t.const_int(8, false)],
                "sim.arg.cp"
            ))
        };
        b!(self.bld.build_store(counter_field, counter_alloca));

        // Create and spawn coroutine
        let coro = b!(self.bld.build_call(
            coro_create,
            &[
                iter_fn.as_global_value().as_pointer_value().into(),
                arg_mem.into(),
            ],
            "sim.coro"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap();
        b!(self.bld.build_call(sched_spawn, &[coro.into()], ""));

        b!(self.bld.build_unconditional_branch(spawn_inc));
        self.bld.position_at_end(spawn_inc);
        let cur_i = b!(self.bld.build_load(i64t, spawn_var, "si")).into_int_value();
        let next_i = b!(self.bld.build_int_nsw_add(cur_i, step_val, "sim.next"));
        b!(self.bld.build_store(spawn_var, next_i));
        b!(self.bld.build_unconditional_branch(spawn_cond));

        // Wait for all iterations to complete
        self.bld.position_at_end(spawn_done);
        let wait_cond = self.ctx.append_basic_block(fv, "sim.wait");
        let wait_done = self.ctx.append_basic_block(fv, "sim.done");
        b!(self.bld.build_unconditional_branch(wait_cond));

        self.bld.position_at_end(wait_cond);
        let remaining = b!(self.bld.build_load(i64t, counter_alloca, "sim.rem")).into_int_value();
        let all_done = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            remaining,
            i64t.const_int(0, false),
            "sim.alldone"
        ));
        let wait_yield = self.ctx.append_basic_block(fv, "sim.wait.yield");
        b!(self.bld.build_conditional_branch(all_done, wait_done, wait_yield));

        self.bld.position_at_end(wait_yield);
        let sched_yield = self.module.get_function("jade_sched_yield").unwrap();
        b!(self.bld.build_call(sched_yield, &[], ""));
        b!(self.bld.build_unconditional_branch(wait_cond));

        self.bld.position_at_end(wait_done);
        Ok(None)
    }

    pub(crate) fn compile_sim_block(
        &mut self,
        stmts: &[hir::Stmt],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        // sim block: spawn each statement as a coroutine, wait for all to finish.
        //
        // Each statement gets its own void(void*) wrapper function.
        // A shared atomic counter tracks how many are still running.

        if stmts.is_empty() {
            return Ok(None);
        }

        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let void = self.ctx.void_type();
        let fv = self.cur_fn.unwrap();

        static SIM_BLK_COUNTER: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let blk_id = SIM_BLK_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Build one wrapper function per statement
        let mut stmt_fns = Vec::new();
        for (i, stmt) in stmts.iter().enumerate() {
            let fn_name = format!("__sim_blk_{blk_id}_s{i}");
            let fn_ty = void.fn_type(&[ptr.into()], false);
            let wrapper = self
                .module
                .add_function(&fn_name, fn_ty, Some(Linkage::Internal));

            let saved_fn = self.cur_fn;
            let saved_bb = self.bld.get_insert_block();
            let saved_vars =
                std::mem::replace(&mut self.vars, vec![std::collections::HashMap::new()]);
            let saved_loop_stack = std::mem::replace(&mut self.loop_stack, Vec::new());

            self.cur_fn = Some(wrapper);
            let entry = self.ctx.append_basic_block(wrapper, "entry");
            self.bld.position_at_end(entry);

            let arg_ptr = wrapper.get_first_param().unwrap().into_pointer_value();

            // arg_ptr points to a single ptr: the counter
            let counter_ptr =
                b!(self.bld.build_load(ptr, arg_ptr, "counter_ptr")).into_pointer_value();

            // Compile the statement
            self.compile_stmt(stmt)?;

            // Atomically decrement counter
            if self.no_term() {
                b!(self.bld.build_atomicrmw(
                    inkwell::AtomicRMWBinOp::Sub,
                    counter_ptr,
                    i64t.const_int(1, false),
                    inkwell::AtomicOrdering::AcquireRelease,
                ));
                let free_fn = self.module.get_function("free").unwrap();
                b!(self.bld.build_call(free_fn, &[arg_ptr.into()], ""));
                b!(self.bld.build_return(None));
            }

            self.cur_fn = saved_fn;
            self.vars = saved_vars;
            self.loop_stack = saved_loop_stack;
            if let Some(bb) = saved_bb {
                self.bld.position_at_end(bb);
            }

            stmt_fns.push(wrapper);
        }

        // Back in the caller: allocate atomic counter, spawn all, wait
        let counter_alloca = self.entry_alloca(i64t.into(), "simb.counter");
        let n = stmts.len() as u64;
        b!(self.bld.build_store(counter_alloca, i64t.const_int(n, false)));

        let malloc_fn = self.ensure_malloc();
        let coro_create = self.module.get_function("jade_coro_create").unwrap();
        let sched_spawn = self.module.get_function("jade_sched_spawn").unwrap();

        for wrapper in &stmt_fns {
            // Allocate arg struct (8 bytes: just a pointer to counter)
            let arg_mem = b!(self.bld.build_call(
                malloc_fn,
                &[i64t.const_int(8, false).into()],
                "simb.arg"
            ))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();

            // Store counter_ptr
            b!(self.bld.build_store(arg_mem, counter_alloca));

            // Create and spawn
            let coro = b!(self.bld.build_call(
                coro_create,
                &[
                    wrapper.as_global_value().as_pointer_value().into(),
                    arg_mem.into(),
                ],
                "simb.coro"
            ))
            .try_as_basic_value()
            .basic()
            .unwrap();
            b!(self.bld.build_call(sched_spawn, &[coro.into()], ""));
        }

        // Wait for all statements to complete
        let wait_cond = self.ctx.append_basic_block(fv, "simb.wait");
        let wait_done = self.ctx.append_basic_block(fv, "simb.done");
        b!(self.bld.build_unconditional_branch(wait_cond));

        self.bld.position_at_end(wait_cond);
        let remaining =
            b!(self.bld.build_load(i64t, counter_alloca, "simb.rem")).into_int_value();
        let all_done = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            remaining,
            i64t.const_int(0, false),
            "simb.alldone"
        ));
        let wait_yield = self.ctx.append_basic_block(fv, "simb.wait.yield");
        b!(self.bld.build_conditional_branch(all_done, wait_done, wait_yield));

        self.bld.position_at_end(wait_yield);
        let sched_yield = self.module.get_function("jade_sched_yield").unwrap();
        b!(self.bld.build_call(sched_yield, &[], ""));
        b!(self.bld.build_unconditional_branch(wait_cond));

        self.bld.position_at_end(wait_done);
        Ok(None)
    }
}
