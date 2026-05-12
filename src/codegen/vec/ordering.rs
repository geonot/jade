//! Vector flattening, membership, ordering, joining, and array containment helpers.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn vec_flatten(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // elem_ty should be Vec<inner>
        let inner_ty = match elem_ty {
            Type::Vec(inner) => inner.as_ref().clone(),
            _ => return Err("flatten requires Vec<Vec<T>>".into()),
        };
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let inner_lty = self.llvm_ty(&inner_ty);
        let inner_size = self.type_store_size(inner_lty);
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;
        let outer_idx_ptr = self.entry_alloca(i64t.into(), "flat.oidx");
        b!(self
            .bld
            .build_store(outer_idx_ptr, i64t.const_int(0, false)));

        let outer_loop = self.ctx.append_basic_block(fv, "flat.oloop");
        let outer_body = self.ctx.append_basic_block(fv, "flat.obody");
        let done_bb = self.ctx.append_basic_block(fv, "flat.done");
        b!(self.bld.build_unconditional_branch(outer_loop));

        self.bld.position_at_end(outer_loop);
        let oidx = b!(self.bld.build_load(i64t, outer_idx_ptr, "flat.oi")).into_int_value();
        let ocond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, oidx, len, "flat.oc"));
        b!(self
            .bld
            .build_conditional_branch(ocond, outer_body, done_bb));

        self.bld.position_at_end(outer_body);
        // Each element is a ptr to vec header
        let inner_gep = unsafe { b!(self.bld.build_gep(ptr_ty, data_ptr, &[oidx], "flat.ig")) };
        let inner_hdr = b!(self.bld.build_load(ptr_ty, inner_gep, "flat.ih")).into_pointer_value();
        let (inner_data, inner_len) = self.vec_data_and_len(inner_hdr)?;

        let inner_idx_ptr = self.entry_alloca(i64t.into(), "flat.iidx");
        b!(self
            .bld
            .build_store(inner_idx_ptr, i64t.const_int(0, false)));
        let inner_loop = self.ctx.append_basic_block(fv, "flat.iloop");
        let inner_body = self.ctx.append_basic_block(fv, "flat.ibody");
        let inner_done = self.ctx.append_basic_block(fv, "flat.idone");
        b!(self.bld.build_unconditional_branch(inner_loop));

        self.bld.position_at_end(inner_loop);
        let iidx = b!(self.bld.build_load(i64t, inner_idx_ptr, "flat.ii")).into_int_value();
        let icond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, iidx, inner_len, "flat.ic"));
        b!(self
            .bld
            .build_conditional_branch(icond, inner_body, inner_done));

        self.bld.position_at_end(inner_body);
        let elem_gep = unsafe {
            b!(self
                .bld
                .build_gep(inner_lty, inner_data, &[iidx], "flat.eg"))
        };
        let elem = b!(self.bld.build_load(inner_lty, elem_gep, "flat.e"));
        self.vec_push_raw(out_hdr, elem, inner_lty, inner_size)?;
        let inext = b!(self
            .bld
            .build_int_nsw_add(iidx, i64t.const_int(1, false), "flat.in"));
        b!(self.bld.build_store(inner_idx_ptr, inext));
        b!(self.bld.build_unconditional_branch(inner_loop));

        self.bld.position_at_end(inner_done);
        let onext = b!(self
            .bld
            .build_int_nsw_add(oidx, i64t.const_int(1, false), "flat.on"));
        b!(self.bld.build_store(outer_idx_ptr, onext));
        b!(self.bld.build_unconditional_branch(outer_loop));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    pub(in crate::codegen) fn vec_contains(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let needle = self.compile_expr(&args[0])?;
        let i64t = self.ctx.i64_type();
        let bool_ty = self.ctx.bool_type();
        let lty = self.llvm_ty(elem_ty);
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let result_ptr = self.entry_alloca(bool_ty.into(), "cont.res");
        b!(self
            .bld
            .build_store(result_ptr, bool_ty.const_int(0, false)));
        let idx_ptr = self.entry_alloca(i64t.into(), "cont.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "cont.loop");
        let body_bb = self.ctx.append_basic_block(fv, "cont.body");
        let done_bb = self.ctx.append_basic_block(fv, "cont.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "cont.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len, "cont.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "cont.gep")) };
        let elem = b!(self.bld.build_load(lty, gep, "cont.elem"));
        let eq = match elem_ty {
            Type::F64 => b!(self.bld.build_float_compare(
                inkwell::FloatPredicate::OEQ,
                elem.into_float_value(),
                needle.into_float_value(),
                "cont.eq"
            ))
            .into(),
            _ => b!(self.bld.build_int_compare(
                IntPredicate::EQ,
                elem.into_int_value(),
                needle.into_int_value(),
                "cont.eq"
            ))
            .into(),
        };
        let found_bb = self.ctx.append_basic_block(fv, "cont.found");
        let cont_bb = self.ctx.append_basic_block(fv, "cont.cont");
        b!(self.bld.build_conditional_branch(eq, found_bb, cont_bb));

        self.bld.position_at_end(found_bb);
        b!(self
            .bld
            .build_store(result_ptr, bool_ty.const_int(1, false)));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "cont.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(bool_ty, result_ptr, "cont.v")))
    }

    pub(in crate::codegen) fn vec_reverse(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;

        // Iterate from len-1 down to 0
        let idx_ptr = self.entry_alloca(i64t.into(), "rev.idx");
        let start = b!(self
            .bld
            .build_int_nsw_sub(len, i64t.const_int(1, false), "rev.start"));
        b!(self.bld.build_store(idx_ptr, start));

        let loop_bb = self.ctx.append_basic_block(fv, "rev.loop");
        let body_bb = self.ctx.append_basic_block(fv, "rev.body");
        let done_bb = self.ctx.append_basic_block(fv, "rev.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "rev.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(
            IntPredicate::SGE,
            idx,
            i64t.const_int(0, false),
            "rev.cmp"
        ));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "rev.gep")) };
        let elem = b!(self.bld.build_load(lty, gep, "rev.elem"));
        self.vec_push_raw(out_hdr, elem, lty, elem_size)?;
        let next = b!(self
            .bld
            .build_int_nsw_sub(idx, i64t.const_int(1, false), "rev.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    pub(in crate::codegen) fn vec_sort(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Simple insertion sort (copies to new vec first)
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        let fv = self.current_fn();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        // Copy to new vec first
        let out_hdr = self.vec_alloc_empty()?;
        let cp_idx = self.entry_alloca(i64t.into(), "sort.ci");
        b!(self.bld.build_store(cp_idx, i64t.const_int(0, false)));
        let cp_loop = self.ctx.append_basic_block(fv, "sort.cp");
        let cp_body = self.ctx.append_basic_block(fv, "sort.cpb");
        let cp_done = self.ctx.append_basic_block(fv, "sort.cpd");
        b!(self.bld.build_unconditional_branch(cp_loop));
        self.bld.position_at_end(cp_loop);
        let ci = b!(self.bld.build_load(i64t, cp_idx, "sort.ci")).into_int_value();
        let cc = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, ci, len, "sort.cc"));
        b!(self.bld.build_conditional_branch(cc, cp_body, cp_done));
        self.bld.position_at_end(cp_body);
        let gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[ci], "sort.cg")) };
        let elem = b!(self.bld.build_load(lty, gep, "sort.ce"));
        self.vec_push_raw(out_hdr, elem, lty, elem_size)?;
        let cn = b!(self
            .bld
            .build_int_nsw_add(ci, i64t.const_int(1, false), "sort.cn"));
        b!(self.bld.build_store(cp_idx, cn));
        b!(self.bld.build_unconditional_branch(cp_loop));

        self.bld.position_at_end(cp_done);
        // In-place insertion sort on out_hdr
        let header_ty = self.vec_header_type();
        let out_ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, out_hdr, 0, "sort.ptrp"));
        let out_data = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            out_ptr_gep,
            "sort.data"
        ))
        .into_pointer_value();

        // Fast path for primitive numeric vectors: delegate to C qsort-backed runtime.
        match elem_ty {
            Type::I64 => {
                let sort_fn = self
                    .module
                    .get_function("jinn_sort_i64")
                    .unwrap_or_else(|| {
                        let sig = self.ctx.void_type().fn_type(
                            &[
                                self.ctx.ptr_type(AddressSpace::default()).into(),
                                i64t.into(),
                            ],
                            false,
                        );
                        self.module
                            .add_function("jinn_sort_i64", sig, Some(Linkage::External))
                    });
                let cast = b!(self.bld.build_pointer_cast(
                    out_data,
                    self.ctx.ptr_type(AddressSpace::default()),
                    "sort.i64.cast"
                ));
                let _ =
                    b!(self
                        .bld
                        .build_call(sort_fn, &[cast.into(), len.into()], "sort.i64.fast"));
                return Ok(out_hdr.into());
            }
            Type::F64 => {
                let sort_fn = self
                    .module
                    .get_function("jinn_sort_f64")
                    .unwrap_or_else(|| {
                        let sig = self.ctx.void_type().fn_type(
                            &[
                                self.ctx.ptr_type(AddressSpace::default()).into(),
                                i64t.into(),
                            ],
                            false,
                        );
                        self.module
                            .add_function("jinn_sort_f64", sig, Some(Linkage::External))
                    });
                let cast = b!(self.bld.build_pointer_cast(
                    out_data,
                    self.ctx.ptr_type(AddressSpace::default()),
                    "sort.f64.cast"
                ));
                let _ =
                    b!(self
                        .bld
                        .build_call(sort_fn, &[cast.into(), len.into()], "sort.f64.fast"));
                return Ok(out_hdr.into());
            }
            _ => {}
        }

        let i_ptr = self.entry_alloca(i64t.into(), "sort.i");
        b!(self.bld.build_store(i_ptr, i64t.const_int(1, false)));
        let outer_loop = self.ctx.append_basic_block(fv, "sort.ol");
        let outer_body = self.ctx.append_basic_block(fv, "sort.ob");
        let sort_done = self.ctx.append_basic_block(fv, "sort.done");
        b!(self.bld.build_unconditional_branch(outer_loop));

        self.bld.position_at_end(outer_loop);
        let i = b!(self.bld.build_load(i64t, i_ptr, "sort.i")).into_int_value();
        let ic = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, i, len, "sort.ic"));
        b!(self.bld.build_conditional_branch(ic, outer_body, sort_done));

        self.bld.position_at_end(outer_body);
        let key_gep = unsafe { b!(self.bld.build_gep(lty, out_data, &[i], "sort.kg")) };
        let key = b!(self.bld.build_load(lty, key_gep, "sort.key"));
        let j_ptr = self.entry_alloca(i64t.into(), "sort.j");
        let j_start = b!(self
            .bld
            .build_int_nsw_sub(i, i64t.const_int(1, false), "sort.js"));
        b!(self.bld.build_store(j_ptr, j_start));

        let inner_loop = self.ctx.append_basic_block(fv, "sort.il");
        let inner_body = self.ctx.append_basic_block(fv, "sort.ib");
        let inner_done = self.ctx.append_basic_block(fv, "sort.id");
        b!(self.bld.build_unconditional_branch(inner_loop));

        self.bld.position_at_end(inner_loop);
        let j = b!(self.bld.build_load(i64t, j_ptr, "sort.j")).into_int_value();
        let jc = b!(self.bld.build_int_compare(
            IntPredicate::SGE,
            j,
            i64t.const_int(0, false),
            "sort.jc"
        ));
        b!(self
            .bld
            .build_conditional_branch(jc, inner_body, inner_done));

        self.bld.position_at_end(inner_body);
        let aj_gep = unsafe { b!(self.bld.build_gep(lty, out_data, &[j], "sort.ag")) };
        let aj = b!(self.bld.build_load(lty, aj_gep, "sort.aj"));
        let gt = match elem_ty {
            Type::F64 => b!(self.bld.build_float_compare(
                inkwell::FloatPredicate::OGT,
                aj.into_float_value(),
                key.into_float_value(),
                "sort.gt"
            ))
            .into(),
            _ => b!(self.bld.build_int_compare(
                IntPredicate::SGT,
                aj.into_int_value(),
                key.into_int_value(),
                "sort.gt"
            ))
            .into(),
        };
        let shift_bb = self.ctx.append_basic_block(fv, "sort.shift");
        b!(self.bld.build_conditional_branch(gt, shift_bb, inner_done));

        self.bld.position_at_end(shift_bb);
        // a[j+1] = a[j]
        let j1 = b!(self
            .bld
            .build_int_nsw_add(j, i64t.const_int(1, false), "sort.j1"));
        let dst_gep = unsafe { b!(self.bld.build_gep(lty, out_data, &[j1], "sort.dg")) };
        b!(self.bld.build_store(dst_gep, aj));
        let jn = b!(self
            .bld
            .build_int_nsw_sub(j, i64t.const_int(1, false), "sort.jn"));
        b!(self.bld.build_store(j_ptr, jn));
        b!(self.bld.build_unconditional_branch(inner_loop));

        self.bld.position_at_end(inner_done);
        // a[j+1] = key
        let j_final = b!(self.bld.build_load(i64t, j_ptr, "sort.jf")).into_int_value();
        let j1f = b!(self
            .bld
            .build_int_nsw_add(j_final, i64t.const_int(1, false), "sort.j1f"));
        let insert_gep = unsafe { b!(self.bld.build_gep(lty, out_data, &[j1f], "sort.ig")) };
        b!(self.bld.build_store(insert_gep, key));

        let in_ = b!(self
            .bld
            .build_int_nsw_add(i, i64t.const_int(1, false), "sort.in"));
        b!(self.bld.build_store(i_ptr, in_));
        b!(self.bld.build_unconditional_branch(outer_loop));

        self.bld.position_at_end(sort_done);
        Ok(out_hdr.into())
    }

    pub(in crate::codegen) fn vec_join(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("join() requires a separator argument".into());
        }
        let sep = self.compile_expr(&args[0])?;
        let fv = self.current_fn();
        let i64t = self.ctx.i64_type();
        let st = self.string_type();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        let empty_bb = self.ctx.append_basic_block(fv, "jn.empty");
        let start_bb = self.ctx.append_basic_block(fv, "jn.start");
        let merge_bb = self.ctx.append_basic_block(fv, "jn.merge");

        let is_empty = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::SLE,
            len,
            i64t.const_int(0, false),
            "jn.isempty"
        ));
        b!(self
            .bld
            .build_conditional_branch(is_empty, empty_bb, start_bb));

        // Empty vec → empty string
        self.bld.position_at_end(empty_bb);
        let empty_str = self.compile_str_literal("")?;
        let empty_exit = self.current_bb();
        b!(self.bld.build_unconditional_branch(merge_bb));

        // Non-empty: start with first element
        self.bld.position_at_end(start_bb);
        let first = b!(self.bld.build_load(st, data_ptr, "jn.first"));
        let cond_bb = self.ctx.append_basic_block(fv, "jn.cond");
        let body_bb = self.ctx.append_basic_block(fv, "jn.body");
        let done_bb = self.ctx.append_basic_block(fv, "jn.done");

        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(cond_bb);
        let phi_i = b!(self.bld.build_phi(i64t, "jn.i"));
        phi_i.add_incoming(&[(&i64t.const_int(1, false), start_bb)]);
        let phi_acc = b!(self.bld.build_phi(st, "jn.acc"));
        phi_acc.add_incoming(&[(&first, start_bb)]);
        let i = phi_i.as_basic_value().into_int_value();
        let acc = phi_acc.as_basic_value();
        let done = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::SGE, i, len, "jn.done"));
        b!(self.bld.build_conditional_branch(done, done_bb, body_bb));

        self.bld.position_at_end(body_bb);
        let elem_ptr = unsafe { b!(self.bld.build_gep(st, data_ptr, &[i], "jn.ep")) };
        let elem = b!(self.bld.build_load(st, elem_ptr, "jn.elem"));
        // acc = acc + sep + elem
        let with_sep = self.string_concat(acc, sep)?;
        let with_elem = self.string_concat(with_sep, elem)?;
        let next_i = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "jn.ni"));
        let body_exit = self.current_bb();
        phi_i.add_incoming(&[(&next_i, body_exit)]);
        phi_acc.add_incoming(&[(&with_elem, body_exit)]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(done_bb);
        let result = acc;
        let done_exit = self.current_bb();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(st, "jn.v"));
        phi.add_incoming(&[(&empty_str, empty_exit), (&result, done_exit)]);
        Ok(phi.as_basic_value())
    }

    /// Inline linear scan for `x in [a, b, c]` on fixed-size arrays.
    pub(in crate::codegen) fn array_contains(
        &mut self,
        obj: &hir::Expr,
        elem_ty: &Type,
        arr_len: usize,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let needle = self.compile_expr(&args[0])?;
        let obj_val = self.compile_expr(obj)?;
        let arr_ptr = obj_val.into_pointer_value();
        let lty = self.llvm_ty(elem_ty);
        let i64t = self.ctx.i64_type();
        let bool_ty = self.ctx.bool_type();
        let fv = self.current_fn();

        let result_ptr = self.entry_alloca(bool_ty.into(), "acont.res");
        b!(self
            .bld
            .build_store(result_ptr, bool_ty.const_int(0, false)));
        let idx_ptr = self.entry_alloca(i64t.into(), "acont.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        let len = i64t.const_int(arr_len as u64, false);
        let arr_ty = lty.array_type(arr_len as u32);

        let loop_bb = self.ctx.append_basic_block(fv, "acont.loop");
        let body_bb = self.ctx.append_basic_block(fv, "acont.body");
        let done_bb = self.ctx.append_basic_block(fv, "acont.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "acont.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len, "acont.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let gep = unsafe {
            b!(self.bld.build_gep(
                arr_ty,
                arr_ptr,
                &[i64t.const_int(0, false), idx],
                "acont.gep"
            ))
        };
        let elem = b!(self.bld.build_load(lty, gep, "acont.elem"));
        let eq = match elem_ty {
            Type::F64 => b!(self.bld.build_float_compare(
                inkwell::FloatPredicate::OEQ,
                elem.into_float_value(),
                needle.into_float_value(),
                "acont.eq"
            ))
            .into(),
            _ => b!(self.bld.build_int_compare(
                IntPredicate::EQ,
                elem.into_int_value(),
                needle.into_int_value(),
                "acont.eq"
            ))
            .into(),
        };
        let found_bb = self.ctx.append_basic_block(fv, "acont.found");
        let cont_bb = self.ctx.append_basic_block(fv, "acont.cont");
        b!(self.bld.build_conditional_branch(eq, found_bb, cont_bb));

        self.bld.position_at_end(found_bb);
        b!(self
            .bld
            .build_store(result_ptr, bool_ty.const_int(1, false)));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "acont.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(bool_ty, result_ptr, "acont.v")))
    }

    // ── MIR-friendly variants accepting precompiled values ──

    pub(in crate::codegen) fn vec_contains_v(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        needle: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let bool_ty = self.ctx.bool_type();
        let lty = self.llvm_ty(elem_ty);
        let fv = self.current_fn();
        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let result_ptr = self.entry_alloca(bool_ty.into(), "vc.res");
        b!(self
            .bld
            .build_store(result_ptr, bool_ty.const_int(0, false)));
        let idx_ptr = self.entry_alloca(i64t.into(), "vc.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        let loop_bb = self.ctx.append_basic_block(fv, "vc.loop");
        let body_bb = self.ctx.append_basic_block(fv, "vc.body");
        let done_bb = self.ctx.append_basic_block(fv, "vc.done");
        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "vc.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len, "vc.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));
        self.bld.position_at_end(body_bb);
        let gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "vc.gep")) };
        let elem = b!(self.bld.build_load(lty, gep, "vc.elem"));
        let eq = match elem_ty {
            Type::F64 | Type::F32 => b!(self.bld.build_float_compare(
                inkwell::FloatPredicate::OEQ,
                elem.into_float_value(),
                needle.into_float_value(),
                "vc.eq"
            ))
            .into(),
            Type::String => {
                // Use string equality (returns i1).
                self.string_eq(elem, needle, false)?.into_int_value()
            }
            _ => b!(self.bld.build_int_compare(
                IntPredicate::EQ,
                elem.into_int_value(),
                needle.into_int_value(),
                "vc.eq"
            )),
        };
        let found_bb = self.ctx.append_basic_block(fv, "vc.found");
        let cont_bb = self.ctx.append_basic_block(fv, "vc.cont");
        b!(self.bld.build_conditional_branch(eq, found_bb, cont_bb));
        self.bld.position_at_end(found_bb);
        b!(self
            .bld
            .build_store(result_ptr, bool_ty.const_int(1, false)));
        b!(self.bld.build_unconditional_branch(done_bb));
        self.bld.position_at_end(cont_bb);
        let next = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "vc.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(bool_ty, result_ptr, "vc.v")))
    }

    pub(in crate::codegen) fn vec_join_v(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        sep: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.current_fn();
        let i64t = self.ctx.i64_type();
        let st = self.string_type();
        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let empty_bb = self.ctx.append_basic_block(fv, "jn.empty");
        let start_bb = self.ctx.append_basic_block(fv, "jn.start");
        let merge_bb = self.ctx.append_basic_block(fv, "jn.merge");
        let is_empty = b!(self.bld.build_int_compare(
            IntPredicate::SLE,
            len,
            i64t.const_int(0, false),
            "jn.isempty"
        ));
        b!(self
            .bld
            .build_conditional_branch(is_empty, empty_bb, start_bb));
        self.bld.position_at_end(empty_bb);
        let empty_str = self.compile_str_literal("")?;
        let empty_exit = self.current_bb();
        b!(self.bld.build_unconditional_branch(merge_bb));
        self.bld.position_at_end(start_bb);
        let first = b!(self.bld.build_load(st, data_ptr, "jn.first"));
        let cond_bb = self.ctx.append_basic_block(fv, "jn.cond");
        let body_bb = self.ctx.append_basic_block(fv, "jn.body");
        let done_bb = self.ctx.append_basic_block(fv, "jn.done");
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let phi_i = b!(self.bld.build_phi(i64t, "jn.i"));
        phi_i.add_incoming(&[(&i64t.const_int(1, false), start_bb)]);
        let phi_acc = b!(self.bld.build_phi(st, "jn.acc"));
        phi_acc.add_incoming(&[(&first, start_bb)]);
        let i = phi_i.as_basic_value().into_int_value();
        let acc = phi_acc.as_basic_value();
        let done = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, i, len, "jn.done"));
        b!(self.bld.build_conditional_branch(done, done_bb, body_bb));
        self.bld.position_at_end(body_bb);
        let elem_ptr = unsafe { b!(self.bld.build_gep(st, data_ptr, &[i], "jn.ep")) };
        let elem = b!(self.bld.build_load(st, elem_ptr, "jn.elem"));
        let with_sep = self.string_concat(acc, sep)?;
        let with_elem = self.string_concat(with_sep, elem)?;
        let next_i = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "jn.ni"));
        let body_exit = self.current_bb();
        phi_i.add_incoming(&[(&next_i, body_exit)]);
        phi_acc.add_incoming(&[(&with_elem, body_exit)]);
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(done_bb);
        let result = acc;
        let done_exit = self.current_bb();
        b!(self.bld.build_unconditional_branch(merge_bb));
        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(st, "jn.v"));
        phi.add_incoming(&[(&empty_str, empty_exit), (&result, done_exit)]);
        Ok(phi.as_basic_value())
    }
}
