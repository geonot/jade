//! Higher-order vector transform and reduction helpers.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn vec_map(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_val = self.compile_expr(&args[0])?;
        let fn_ty = &args[0].ty;
        let out_elem_ty = match fn_ty {
            Type::Fn(_, ret) => ret.as_ref().clone(),
            _ => return Err("map callback must be a function".into()),
        };
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let out_lty = self.llvm_ty(&out_elem_ty);
        let out_elem_size = self.type_store_size(out_lty);
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;

        let idx_ptr = self.entry_alloca(i64t.into(), "map.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "map.loop");
        let body_bb = self.ctx.append_basic_block(fv, "map.body");
        let done_bb = self.ctx.append_basic_block(fv, "map.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "map.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len, "map.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "map.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "map.elem"));
        let mapped = self.indirect_call_vals(fn_val, fn_ty, &[elem])?;
        self.vec_push_raw(out_hdr, mapped, out_lty, out_elem_size)?;
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "map.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    pub(in crate::codegen) fn vec_filter(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_val = self.compile_expr(&args[0])?;
        let fn_ty = &args[0].ty;
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;

        let idx_ptr = self.entry_alloca(i64t.into(), "filt.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "filt.loop");
        let body_bb = self.ctx.append_basic_block(fv, "filt.body");
        let push_bb = self.ctx.append_basic_block(fv, "filt.push");
        let cont_bb = self.ctx.append_basic_block(fv, "filt.cont");
        let done_bb = self.ctx.append_basic_block(fv, "filt.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "filt.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len, "filt.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "filt.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "filt.elem"));
        let pred = self
            .indirect_call_vals(fn_val, fn_ty, &[elem])?
            .into_int_value();
        let pred_bool = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            pred,
            self.ctx.bool_type().const_int(0, false),
            "filt.bool"
        ));
        b!(self
            .bld
            .build_conditional_branch(pred_bool, push_bb, cont_bb));

        self.bld.position_at_end(push_bb);
        // Reload elem since we may need it fresh
        let elem2 = b!(self.bld.build_load(lty, elem_gep, "filt.elem2"));
        self.vec_push_raw(out_hdr, elem2, lty, elem_size)?;
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "filt.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    pub(in crate::codegen) fn vec_fold(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let init_val = self.compile_expr(&args[0])?;
        let fn_val = self.compile_expr(&args[1])?;
        let fn_ty = &args[1].ty;
        let acc_lty = init_val.get_type();
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        let acc_ptr = self.entry_alloca(acc_lty, "fold.acc");
        b!(self.bld.build_store(acc_ptr, init_val));
        let idx_ptr = self.entry_alloca(i64t.into(), "fold.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "fold.loop");
        let body_bb = self.ctx.append_basic_block(fv, "fold.body");
        let done_bb = self.ctx.append_basic_block(fv, "fold.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "fold.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len, "fold.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let cur_acc = b!(self.bld.build_load(acc_lty, acc_ptr, "fold.cur"));
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "fold.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "fold.elem"));
        let new_acc = self.indirect_call_vals(fn_val, fn_ty, &[cur_acc, elem])?;
        b!(self.bld.build_store(acc_ptr, new_acc));
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "fold.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(acc_lty, acc_ptr, "fold.result")))
    }

    pub(in crate::codegen) fn vec_any_all(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
        is_any: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_val = self.compile_expr(&args[0])?;
        let fn_ty = &args[0].ty;
        let i64t = self.ctx.i64_type();
        let bool_ty = self.ctx.bool_type();
        let lty = self.llvm_ty(elem_ty);
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        // For any: start false, short-circuit on true
        // For all: start true, short-circuit on false
        let init = if is_any { 0u64 } else { 1u64 };
        let result_ptr = self.entry_alloca(bool_ty.into(), "aa.res");
        b!(self
            .bld
            .build_store(result_ptr, bool_ty.const_int(init, false)));
        let idx_ptr = self.entry_alloca(i64t.into(), "aa.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "aa.loop");
        let body_bb = self.ctx.append_basic_block(fv, "aa.body");
        let done_bb = self.ctx.append_basic_block(fv, "aa.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "aa.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len, "aa.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "aa.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "aa.elem"));
        let pred = self
            .indirect_call_vals(fn_val, fn_ty, &[elem])?
            .into_int_value();
        let pred_bool = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            pred,
            bool_ty.const_int(0, false),
            "aa.pb"
        ));
        if is_any {
            // If true found, set result=true and exit
            let found_bb = self.ctx.append_basic_block(fv, "aa.found");
            let cont_bb = self.ctx.append_basic_block(fv, "aa.cont");
            b!(self
                .bld
                .build_conditional_branch(pred_bool, found_bb, cont_bb));
            self.bld.position_at_end(found_bb);
            b!(self
                .bld
                .build_store(result_ptr, bool_ty.const_int(1, false)));
            b!(self.bld.build_unconditional_branch(done_bb));
            self.bld.position_at_end(cont_bb);
        } else {
            // If false found, set result=false and exit
            let fail_bb = self.ctx.append_basic_block(fv, "aa.fail");
            let cont_bb = self.ctx.append_basic_block(fv, "aa.cont");
            b!(self
                .bld
                .build_conditional_branch(pred_bool, cont_bb, fail_bb));
            self.bld.position_at_end(fail_bb);
            b!(self
                .bld
                .build_store(result_ptr, bool_ty.const_int(0, false)));
            b!(self.bld.build_unconditional_branch(done_bb));
            self.bld.position_at_end(cont_bb);
        }
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "aa.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(bool_ty, result_ptr, "aa.v")))
    }

    pub(in crate::codegen) fn vec_find(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_val = self.compile_expr(&args[0])?;
        let fn_ty = &args[0].ty;
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let bool_ty = self.ctx.bool_type();
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        let result_ptr = self.entry_alloca(lty, "find.res");
        let idx_ptr = self.entry_alloca(i64t.into(), "find.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "find.loop");
        let body_bb = self.ctx.append_basic_block(fv, "find.body");
        let done_bb = self.ctx.append_basic_block(fv, "find.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "find.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len, "find.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "find.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "find.elem"));
        let pred = self
            .indirect_call_vals(fn_val, fn_ty, &[elem])?
            .into_int_value();
        let pred_bool = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            pred,
            bool_ty.const_int(0, false),
            "find.pb"
        ));
        let found_bb = self.ctx.append_basic_block(fv, "find.found");
        let cont_bb = self.ctx.append_basic_block(fv, "find.cont");
        b!(self
            .bld
            .build_conditional_branch(pred_bool, found_bb, cont_bb));

        self.bld.position_at_end(found_bb);
        let elem2 = b!(self.bld.build_load(lty, elem_gep, "find.elem2"));
        b!(self.bld.build_store(result_ptr, elem2));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "find.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(lty, result_ptr, "find.v")))
    }

    pub(in crate::codegen) fn vec_sum(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        let acc_ptr = self.entry_alloca(lty, "sum.acc");
        let zero: BasicValueEnum<'ctx> = match elem_ty {
            Type::F64 => self.ctx.f64_type().const_float(0.0).into(),
            _ => i64t.const_int(0, false).into(),
        };
        b!(self.bld.build_store(acc_ptr, zero));
        let idx_ptr = self.entry_alloca(i64t.into(), "sum.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "sum.loop");
        let body_bb = self.ctx.append_basic_block(fv, "sum.body");
        let done_bb = self.ctx.append_basic_block(fv, "sum.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "sum.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len, "sum.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "sum.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "sum.elem"));
        let cur = b!(self.bld.build_load(lty, acc_ptr, "sum.cur"));
        let new_val: BasicValueEnum<'ctx> = match elem_ty {
            Type::F64 => b!(self.bld.build_float_add(
                cur.into_float_value(),
                elem.into_float_value(),
                "sum.add"
            ))
            .into(),
            _ => b!(self.bld.build_int_nsw_add(
                cur.into_int_value(),
                elem.into_int_value(),
                "sum.add"
            ))
            .into(),
        };
        b!(self.bld.build_store(acc_ptr, new_val));
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "sum.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(lty, acc_ptr, "sum.v")))
    }

    pub(in crate::codegen) fn vec_take_skip(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
        is_take: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let n_val = self.compile_expr(&args[0])?.into_int_value();
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;

        // For take: iterate 0..min(n, len)
        // For skip: iterate n..len
        let (start, end) = if is_take {
            let min_bb = self.ctx.append_basic_block(fv, "ts.min");
            let use_n_bb = self.ctx.append_basic_block(fv, "ts.usen");
            let use_len_bb = self.ctx.append_basic_block(fv, "ts.uselen");
            let cmp = b!(self
                .bld
                .build_int_compare(IntPredicate::SLT, n_val, len, "ts.cmp"));
            b!(self.bld.build_conditional_branch(cmp, use_n_bb, use_len_bb));
            self.bld.position_at_end(use_n_bb);
            b!(self.bld.build_unconditional_branch(min_bb));
            self.bld.position_at_end(use_len_bb);
            b!(self.bld.build_unconditional_branch(min_bb));
            self.bld.position_at_end(min_bb);
            let phi = b!(self.bld.build_phi(i64t, "ts.end"));
            phi.add_incoming(&[(&n_val, use_n_bb), (&len, use_len_bb)]);
            (
                i64t.const_int(0, false),
                phi.as_basic_value().into_int_value(),
            )
        } else {
            (n_val, len)
        };

        let idx_ptr = self.entry_alloca(i64t.into(), "ts.idx");
        b!(self.bld.build_store(idx_ptr, start));

        let loop_bb = self.ctx.append_basic_block(fv, "ts.loop");
        let body_bb = self.ctx.append_basic_block(fv, "ts.body");
        let done_bb = self.ctx.append_basic_block(fv, "ts.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "ts.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, end, "ts.cmp2"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "ts.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "ts.elem"));
        self.vec_push_raw(out_hdr, elem, lty, elem_size)?;
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "ts.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    pub(in crate::codegen) fn vec_zip(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let other_val = self.compile_expr(&args[0])?.into_pointer_value();
        let other_elem_ty = match &args[0].ty {
            Type::Vec(et) => et.as_ref().clone(),
            _ => return Err("zip argument must be Vec".into()),
        };
        let i64t = self.ctx.i64_type();
        let lty_a = self.llvm_ty(elem_ty);
        let lty_b = self.llvm_ty(&other_elem_ty);
        // Tuple type: (A, B)
        let tuple_lty = self.ctx.struct_type(&[lty_a.into(), lty_b.into()], false);
        let tuple_size = self.type_store_size(tuple_lty.into());
        let fv = self.current_fn();

        let (data_a, len_a) = self.vec_data_and_len(header_ptr)?;
        let (data_b, len_b) = self.vec_data_and_len(other_val)?;

        // min(len_a, len_b)
        let min_bb = self.ctx.append_basic_block(fv, "zip.min");
        let use_a_bb = self.ctx.append_basic_block(fv, "zip.usea");
        let use_b_bb = self.ctx.append_basic_block(fv, "zip.useb");
        let cmp = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, len_a, len_b, "zip.cmp"));
        b!(self.bld.build_conditional_branch(cmp, use_a_bb, use_b_bb));
        self.bld.position_at_end(use_a_bb);
        b!(self.bld.build_unconditional_branch(min_bb));
        self.bld.position_at_end(use_b_bb);
        b!(self.bld.build_unconditional_branch(min_bb));
        self.bld.position_at_end(min_bb);
        let phi = b!(self.bld.build_phi(i64t, "zip.len"));
        phi.add_incoming(&[(&len_a, use_a_bb), (&len_b, use_b_bb)]);
        let min_len = phi.as_basic_value().into_int_value();

        let out_hdr = self.vec_alloc_empty()?;
        let idx_ptr = self.entry_alloca(i64t.into(), "zip.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "zip.loop");
        let body_bb = self.ctx.append_basic_block(fv, "zip.body");
        let done_bb = self.ctx.append_basic_block(fv, "zip.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "zip.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, min_len, "zip.cmp2"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let a_gep = unsafe { b!(self.bld.build_gep(lty_a, data_a, &[idx], "zip.a")) };
        let a_val = b!(self.bld.build_load(lty_a, a_gep, "zip.av"));
        let b_gep = unsafe { b!(self.bld.build_gep(lty_b, data_b, &[idx], "zip.b")) };
        let b_val = b!(self.bld.build_load(lty_b, b_gep, "zip.bv"));
        // Build tuple
        let mut tup = tuple_lty.get_undef();
        tup = b!(self.bld.build_insert_value(tup, a_val, 0, "zip.t0")).into_struct_value();
        tup = b!(self.bld.build_insert_value(tup, b_val, 1, "zip.t1")).into_struct_value();
        self.vec_push_raw(out_hdr, tup.into(), tuple_lty.into(), tuple_size)?;
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "zip.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    pub(in crate::codegen) fn vec_chain(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let other_val = self.compile_expr(&args[0])?.into_pointer_value();
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        let fv = self.current_fn();

        let out_hdr = self.vec_alloc_empty()?;

        // Copy first vec
        let (data_a, len_a) = self.vec_data_and_len(header_ptr)?;
        let idx_ptr = self.entry_alloca(i64t.into(), "chn.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        let loop1 = self.ctx.append_basic_block(fv, "chn.l1");
        let body1 = self.ctx.append_basic_block(fv, "chn.b1");
        let mid = self.ctx.append_basic_block(fv, "chn.mid");
        b!(self.bld.build_unconditional_branch(loop1));
        self.bld.position_at_end(loop1);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "chn.i1")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len_a, "chn.c1"));
        b!(self.bld.build_conditional_branch(cond, body1, mid));
        self.bld.position_at_end(body1);
        let gep = unsafe { b!(self.bld.build_gep(lty, data_a, &[idx], "chn.g1")) };
        let elem = b!(self.bld.build_load(lty, gep, "chn.e1"));
        self.vec_push_raw(out_hdr, elem, lty, elem_size)?;
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "chn.n1"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop1));

        // Copy second vec
        self.bld.position_at_end(mid);
        let (data_b, len_b) = self.vec_data_and_len(other_val)?;
        let idx_ptr2 = self.entry_alloca(i64t.into(), "chn.idx2");
        b!(self.bld.build_store(idx_ptr2, i64t.const_int(0, false)));
        let loop2 = self.ctx.append_basic_block(fv, "chn.l2");
        let body2 = self.ctx.append_basic_block(fv, "chn.b2");
        let done = self.ctx.append_basic_block(fv, "chn.done");
        b!(self.bld.build_unconditional_branch(loop2));
        self.bld.position_at_end(loop2);
        let idx2 = b!(self.bld.build_load(i64t, idx_ptr2, "chn.i2")).into_int_value();
        let cond2 = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx2, len_b, "chn.c2"));
        b!(self.bld.build_conditional_branch(cond2, body2, done));
        self.bld.position_at_end(body2);
        let gep2 = unsafe { b!(self.bld.build_gep(lty, data_b, &[idx2], "chn.g2")) };
        let elem2 = b!(self.bld.build_load(lty, gep2, "chn.e2"));
        self.vec_push_raw(out_hdr, elem2, lty, elem_size)?;
        let next2 = b!(self
            .bld
            .build_int_nsw_add(idx2, i64t.const_int(1, false), "chn.n2"));
        b!(self.bld.build_store(idx_ptr2, next2));
        b!(self.bld.build_unconditional_branch(loop2));

        self.bld.position_at_end(done);
        Ok(out_hdr.into())
    }

    pub(in crate::codegen) fn vec_enumerate(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let tuple_lty = self.ctx.struct_type(&[i64t.into(), lty.into()], false);
        let tuple_size = self.type_store_size(tuple_lty.into());
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;
        let idx_ptr = self.entry_alloca(i64t.into(), "enum.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "enum.loop");
        let body_bb = self.ctx.append_basic_block(fv, "enum.body");
        let done_bb = self.ctx.append_basic_block(fv, "enum.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "enum.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len, "enum.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "enum.gep")) };
        let elem = b!(self.bld.build_load(lty, gep, "enum.elem"));
        let mut tup = tuple_lty.get_undef();
        tup = b!(self.bld.build_insert_value(tup, idx, 0, "enum.t0")).into_struct_value();
        tup = b!(self.bld.build_insert_value(tup, elem, 1, "enum.t1")).into_struct_value();
        self.vec_push_raw(out_hdr, tup.into(), tuple_lty.into(), tuple_size)?;
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "enum.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }
}
