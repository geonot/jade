use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn compile_strict_cast(
        &mut self,
        expr: &hir::Expr,
        target: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(expr)?;
        let src = &expr.ty;
        if src.is_int() && target.is_int() {
            let dst = self.llvm_ty(target);
            let (sb, db) = (src.bits(), target.bits());
            if sb > db {
                let truncated = b!(self.bld.build_int_truncate(
                    val.into_int_value(),
                    dst.into_int_type(),
                    "strict.trunc"
                ));
                let extended = if src.is_signed() {
                    b!(self.bld.build_int_s_extend(
                        truncated,
                        val.into_int_value().get_type(),
                        "strict.ext"
                    ))
                } else {
                    b!(self.bld.build_int_z_extend(
                        truncated,
                        val.into_int_value().get_type(),
                        "strict.ext"
                    ))
                };
                let ok = b!(self.bld.build_int_compare(
                    IntPredicate::EQ,
                    val.into_int_value(),
                    extended,
                    "strict.ok"
                ));
                let fv = self.current_fn();
                let pass_bb = self.ctx.append_basic_block(fv, "strict.pass");
                let fail_bb = self.ctx.append_basic_block(fv, "strict.fail");
                b!(self.bld.build_conditional_branch(ok, pass_bb, fail_bb));
                self.bld.position_at_end(fail_bb);
                self.emit_trap("strict cast: value out of range");
                self.bld.position_at_end(pass_bb);
                return Ok(truncated.into());
            } else if sb < db {
                return Ok(if src.is_signed() {
                    b!(self.bld.build_int_s_extend(
                        val.into_int_value(),
                        dst.into_int_type(),
                        "strict.sext"
                    ))
                    .into()
                } else {
                    b!(self.bld.build_int_z_extend(
                        val.into_int_value(),
                        dst.into_int_type(),
                        "strict.zext"
                    ))
                    .into()
                });
            }
            return Ok(val);
        }

        if src.is_float() && target.is_int() {
            let fv = self.current_fn();
            let float_val = val.into_float_value();
            let dst_int_ty = self.llvm_ty(target).into_int_type();
            let src_float_ty = float_val.get_type();

            let is_nan = b!(self.bld.build_float_compare(
                FloatPredicate::UNO,
                float_val,
                float_val,
                "strict.isnan"
            ));
            let nan_fail_bb = self.ctx.append_basic_block(fv, "strict.nan_fail");
            let nan_pass_bb = self.ctx.append_basic_block(fv, "strict.nan_pass");
            b!(self
                .bld
                .build_conditional_branch(is_nan, nan_fail_bb, nan_pass_bb));
            self.bld.position_at_end(nan_fail_bb);
            self.emit_trap("strict cast: cannot convert NaN to integer");
            self.bld.position_at_end(nan_pass_bb);

            let int_val = if target.is_signed() {
                b!(self
                    .bld
                    .build_float_to_signed_int(float_val, dst_int_ty, "strict.fptosi"))
            } else {
                b!(self
                    .bld
                    .build_float_to_unsigned_int(float_val, dst_int_ty, "strict.fptoui"))
            };

            let roundtrip = if target.is_signed() {
                b!(self
                    .bld
                    .build_signed_int_to_float(int_val, src_float_ty, "strict.sitofp"))
            } else {
                b!(self
                    .bld
                    .build_unsigned_int_to_float(int_val, src_float_ty, "strict.uitofp"))
            };
            let ok = b!(self.bld.build_float_compare(
                FloatPredicate::OEQ,
                float_val,
                roundtrip,
                "strict.roundtrip_ok"
            ));
            let pass_bb = self.ctx.append_basic_block(fv, "strict.fti_pass");
            let fail_bb = self.ctx.append_basic_block(fv, "strict.fti_fail");
            b!(self.bld.build_conditional_branch(ok, pass_bb, fail_bb));
            self.bld.position_at_end(fail_bb);
            self.emit_trap("strict cast: float value out of integer range");
            self.bld.position_at_end(pass_bb);
            return Ok(int_val.into());
        }

        self.compile_cast(expr, target)
    }

    pub(in crate::codegen) fn compile_as_format(
        &mut self,
        expr: &hir::Expr,
        fmt: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match fmt {
            "json" => self.compile_as_json(expr),
            _ => self.compile_to_string(expr),
        }
    }

    pub(in crate::codegen) fn compile_as_json(
        &mut self,
        expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ty = self.resolve_ty(expr.ty.clone());
        match &ty {
            Type::Struct(name, _) => {
                let fields = self.structs.get(name).cloned().unwrap_or_default();
                let val = self.compile_expr(expr)?;

                let mut result = self.compile_str_literal("{")?;
                let struct_ty = self
                    .module
                    .get_struct_type(&name.as_str())
                    .ok_or_else(|| format!("unknown struct type: {name}"))?;
                for (i, (fname, fty)) in fields.iter().enumerate() {
                    if i > 0 {
                        let comma = self.compile_str_literal(", ")?;
                        result = self.string_concat(result, comma)?;
                    }

                    let key_str = self.compile_str_literal(&format!("\"{fname}\": "))?;
                    result = self.string_concat(result, key_str)?;

                    let field_val = if val.is_pointer_value() {
                        let fgep = b!(self.bld.build_struct_gep(
                            struct_ty,
                            val.into_pointer_value(),
                            i as u32,
                            "json.fgep"
                        ));
                        b!(self.bld.build_load(self.llvm_ty(fty), fgep, "json.fld"))
                    } else {
                        b!(self.bld.build_extract_value(
                            val.into_struct_value(),
                            i as u32,
                            "json.fld"
                        ))
                    };

                    let fval_str = match fty {
                        Type::String => {
                            let q = self.compile_str_literal("\"")?;
                            let s = self.string_concat(q.clone(), field_val)?;
                            self.string_concat(s, q)?
                        }
                        Type::I64 | Type::I32 | Type::I16 | Type::I8 => {
                            self.int_to_string(field_val, false)?
                        }
                        Type::U64 | Type::U32 | Type::U16 | Type::U8 => {
                            self.int_to_string(field_val, true)?
                        }
                        Type::F64 | Type::F32 => self.float_to_string(field_val)?,
                        Type::Bool => self.bool_to_string(field_val)?,
                        _ => self.int_to_string(field_val, false)?,
                    };
                    result = self.string_concat(result, fval_str)?;
                }
                let close = self.compile_str_literal("}")?;
                self.string_concat(result, close)
            }
            Type::String => {
                let q = self.compile_str_literal("\"")?;
                let val = self.compile_expr(expr)?;
                let s = self.string_concat(q.clone(), val)?;
                self.string_concat(s, q)
            }
            _ => self.compile_to_string(expr),
        }
    }

    pub(in crate::codegen) fn compile_atomic_load(
        &mut self,
        ptr_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let i64t = self.ctx.i64_type();
        let load = b!(self
            .bld
            .build_load(i64t, ptr.into_pointer_value(), "atomic.load"));

        load.as_instruction_value()
            .expect("ICE: not an instruction")
            .set_atomic_ordering(inkwell::AtomicOrdering::SequentiallyConsistent)
            .map_err(|_| "failed to set atomic ordering".to_string())?;
        Ok(load)
    }

    pub(in crate::codegen) fn compile_atomic_store(
        &mut self,
        ptr_expr: &hir::Expr,
        val_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let val = self.compile_expr(val_expr)?;
        let store = b!(self.bld.build_store(ptr.into_pointer_value(), val));
        store
            .set_atomic_ordering(inkwell::AtomicOrdering::SequentiallyConsistent)
            .map_err(|_| "failed to set atomic ordering".to_string())?;
        Ok(self.ctx.i64_type().const_zero().into())
    }

    pub(in crate::codegen) fn compile_atomic_add(
        &mut self,
        ptr_expr: &hir::Expr,
        val_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let val = self.compile_expr(val_expr)?;
        let old = b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Add,
            ptr.into_pointer_value(),
            val.into_int_value(),
            inkwell::AtomicOrdering::SequentiallyConsistent,
        ));
        Ok(old.into())
    }

    pub(in crate::codegen) fn compile_atomic_sub(
        &mut self,
        ptr_expr: &hir::Expr,
        val_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let val = self.compile_expr(val_expr)?;
        let old = b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Sub,
            ptr.into_pointer_value(),
            val.into_int_value(),
            inkwell::AtomicOrdering::SequentiallyConsistent,
        ));
        Ok(old.into())
    }

    pub(in crate::codegen) fn compile_atomic_cas(
        &mut self,
        ptr_expr: &hir::Expr,
        expected_expr: &hir::Expr,
        new_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let expected = self.compile_expr(expected_expr)?;
        let new_val = self.compile_expr(new_expr)?;
        let cas = b!(self.bld.build_cmpxchg(
            ptr.into_pointer_value(),
            expected.into_int_value(),
            new_val.into_int_value(),
            inkwell::AtomicOrdering::SequentiallyConsistent,
            inkwell::AtomicOrdering::SequentiallyConsistent,
        ));

        let old = b!(self.bld.build_extract_value(cas, 0, "cas.old"));
        Ok(old)
    }

    pub(in crate::codegen) fn compile_slice(
        &mut self,
        obj: &hir::Expr,
        start: &hir::Expr,
        end: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.compile_expr(obj)?;
        let start_val = self.compile_expr(start)?;
        let end_val = self.compile_expr(end)?;
        match &obj.ty {
            Type::Vec(elem_ty) => {
                let lty = self.llvm_ty(elem_ty);
                let elem_size = self.type_store_size(lty);
                let i64t = self.ctx.i64_type();
                let slice_fn = self
                    .module
                    .get_function("__jinn_vec_slice")
                    .unwrap_or_else(|| {
                        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                        let ft = ptr_ty.fn_type(
                            &[ptr_ty.into(), i64t.into(), i64t.into(), i64t.into()],
                            false,
                        );
                        self.module.add_function(
                            "__jinn_vec_slice",
                            ft,
                            Some(inkwell::module::Linkage::External),
                        )
                    });
                let esz = i64t.const_int(elem_size, false);
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[obj_val.into(), start_val.into(), end_val.into(), esz.into()],
                    "slice"
                ));
                Ok(result
                    .try_as_basic_value()
                    .basic()
                    .unwrap_or_else(|| self.ctx.i64_type().const_zero().into()))
            }
            Type::String => {
                let slice_fn = self
                    .module
                    .get_function("__jinn_str_slice")
                    .unwrap_or_else(|| {
                        let st = self.string_type();
                        let i64t = self.ctx.i64_type();
                        let ft = st.fn_type(&[st.into(), i64t.into(), i64t.into()], false);
                        self.module.add_function(
                            "__jinn_str_slice",
                            ft,
                            Some(inkwell::module::Linkage::External),
                        )
                    });
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[obj_val.into(), start_val.into(), end_val.into()],
                    "str.slice"
                ));
                Ok(result
                    .try_as_basic_value()
                    .basic()
                    .unwrap_or_else(|| self.ctx.i64_type().const_zero().into()))
            }
            _ => Err(format!("slice not supported for type: {}", &obj.ty)),
        }
    }

    pub(in crate::codegen) fn compile_grad(
        &mut self,
        inner: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let f_closure = self.compile_expr(inner)?;

        let f64t = self.ctx.f64_type();
        let ptr_t = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let grad_ft = f64t.fn_type(&[ptr_t.into(), f64t.into()], false);
        let grad_fn = self.module.add_function("__grad_wrapper", grad_ft, None);

        let saved_bb = self.bld.get_insert_block();
        let saved_fn = self.cur_fn;
        let entry = self.ctx.append_basic_block(grad_fn, "entry");
        self.bld.position_at_end(entry);
        self.cur_fn = Some(grad_fn);

        let env_arg = grad_fn
            .get_nth_param(0)
            .expect("ICE: missing param")
            .into_pointer_value();
        let x = grad_fn
            .get_nth_param(1)
            .expect("ICE: missing param")
            .into_float_value();

        let cl_ty = self.closure_type();
        let orig_cl = b!(self.bld.build_load(cl_ty, env_arg, "orig.cl")).into_struct_value();
        let orig_fn = b!(self.bld.build_extract_value(orig_cl, 0, "orig.fn")).into_pointer_value();
        let orig_env = b!(self.bld.build_extract_value(orig_cl, 1, "orig.env"));

        let inner_ft = f64t.fn_type(&[ptr_t.into(), f64t.into()], false);

        let h = f64t.const_float(1e-8);
        let two_h = f64t.const_float(2e-8);

        let x_plus = b!(self.bld.build_float_add(x, h, "xp"));

        let x_minus = b!(self.bld.build_float_sub(x, h, "xm"));

        let fp = b!(self.bld.build_indirect_call(
            inner_ft,
            orig_fn,
            &[orig_env.into(), x_plus.into()],
            "fp"
        ));
        let fp_val = self.call_result(fp).into_float_value();

        let fm = b!(self.bld.build_indirect_call(
            inner_ft,
            orig_fn,
            &[orig_env.into(), x_minus.into()],
            "fm"
        ));
        let fm_val = self.call_result(fm).into_float_value();

        let diff = b!(self.bld.build_float_sub(fp_val, fm_val, "diff"));
        let grad_val = b!(self.bld.build_float_div(diff, two_h, "grad"));
        b!(self.bld.build_return(Some(&grad_val)));

        self.cur_fn = saved_fn;
        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }

        let cl_alloc = self.entry_alloca(cl_ty.into(), "grad.env");
        b!(self.bld.build_store(cl_alloc, f_closure));

        self.make_closure(grad_fn.as_global_value().as_pointer_value(), cl_alloc)
    }
}
