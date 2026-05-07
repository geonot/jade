#![allow(unused_imports, unused_variables)]
use super::*;

impl<'ctx> Compiler<'ctx> {

    pub(in crate::codegen) fn compile_einsum_trace(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let a_ptr = self.compile_expr(&args[0])?.into_pointer_value();
        let n = match &args[0].ty {
            Type::NDArray(_, dims) if dims.len() == 2 && dims[0] == dims[1] => dims[0] as u64,
            _ => return Err("einsum trace requires square 2D NDArray".into()),
        };
        let i64t = self.ctx.i64_type();
        let f64t = self.ctx.f64_type();
        let fn_val = self.current_fn();

        let acc = self.entry_alloca(f64t.into(), "tr.acc");
        b!(self.bld.build_store(acc, f64t.const_float(0.0)));
        let iv = self.entry_alloca(i64t.into(), "tr.i");
        b!(self.bld.build_store(iv, i64t.const_zero()));

        let loop_bb = self.ctx.append_basic_block(fn_val, "tr.loop");
        let body_bb = self.ctx.append_basic_block(fn_val, "tr.body");
        let end_bb = self.ctx.append_basic_block(fn_val, "tr.end");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let i = b!(self.bld.build_load(i64t, iv, "i")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(
            IntPredicate::ULT,
            i,
            i64t.const_int(n, false),
            "tr.cmp"
        ));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));

        self.bld.position_at_end(body_bb);
        // A[i*n + i]
        let idx = b!(self
            .bld
            .build_int_mul(i, i64t.const_int(n, false), "tr.row"));
        let idx = b!(self.bld.build_int_add(idx, i, "tr.diag"));
        let ep = unsafe { b!(self.bld.build_gep(f64t, a_ptr, &[idx], "tr.ep")) };
        let val = b!(self.bld.build_load(f64t, ep, "tr.v")).into_float_value();
        let cur = b!(self.bld.build_load(f64t, acc, "tr.cur")).into_float_value();
        let sum = b!(self.bld.build_float_add(cur, val, "tr.sum"));
        b!(self.bld.build_store(acc, sum));
        let i_next = b!(self
            .bld
            .build_int_add(i, i64t.const_int(1, false), "i.next"));
        b!(self.bld.build_store(iv, i_next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(end_bb);
        Ok(b!(self.bld.build_load(f64t, acc, "tr.result")))
    }

    pub(in crate::codegen) fn compile_einsum_transpose(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let a_ptr = self.compile_expr(&args[0])?.into_pointer_value();
        let (m, n) = match &args[0].ty {
            Type::NDArray(_, dims) if dims.len() == 2 => (dims[0] as u64, dims[1] as u64),
            _ => return Err("einsum transpose requires 2D NDArray".into()),
        };
        let i64t = self.ctx.i64_type();
        let f64t = self.ctx.f64_type();
        let malloc = self.ensure_malloc();

        let total = m * n;
        let byte_size = i64t.const_int(total * 8, false);
        let result_ptr = b!(self.bld.build_call(malloc, &[byte_size.into()], "tp.ptr"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void")
            .into_pointer_value();

        let fn_val = self.current_fn();
        let iv = self.entry_alloca(i64t.into(), "tp.i");
        let jv = self.entry_alloca(i64t.into(), "tp.j");
        b!(self.bld.build_store(iv, i64t.const_zero()));

        let i_loop = self.ctx.append_basic_block(fn_val, "tp.i");
        let j_loop = self.ctx.append_basic_block(fn_val, "tp.j");
        let body = self.ctx.append_basic_block(fn_val, "tp.body");
        let j_end = self.ctx.append_basic_block(fn_val, "tp.j.end");
        let i_end = self.ctx.append_basic_block(fn_val, "tp.end");
        b!(self.bld.build_unconditional_branch(i_loop));

        self.bld.position_at_end(i_loop);
        let i = b!(self.bld.build_load(i64t, iv, "i")).into_int_value();
        let cmp_i = b!(self.bld.build_int_compare(
            IntPredicate::ULT,
            i,
            i64t.const_int(m, false),
            "tp.icmp"
        ));
        b!(self.bld.build_conditional_branch(cmp_i, j_loop, i_end));

        self.bld.position_at_end(j_loop);
        b!(self.bld.build_store(jv, i64t.const_zero()));
        let j_loop2 = self.ctx.append_basic_block(fn_val, "tp.j2");
        b!(self.bld.build_unconditional_branch(j_loop2));

        self.bld.position_at_end(j_loop2);
        let j = b!(self.bld.build_load(i64t, jv, "j")).into_int_value();
        let cmp_j = b!(self.bld.build_int_compare(
            IntPredicate::ULT,
            j,
            i64t.const_int(n, false),
            "tp.jcmp"
        ));
        b!(self.bld.build_conditional_branch(cmp_j, body, j_end));

        self.bld.position_at_end(body);
        // src[i*n + j] -> dst[j*m + i]
        let src_idx = b!(self.bld.build_int_mul(i, i64t.const_int(n, false), "s.row"));
        let src_idx = b!(self.bld.build_int_add(src_idx, j, "s.idx"));
        let src_ep = unsafe { b!(self.bld.build_gep(f64t, a_ptr, &[src_idx], "s.ep")) };
        let val = b!(self.bld.build_load(f64t, src_ep, "s.v")).into_float_value();
        let dst_idx = b!(self.bld.build_int_mul(j, i64t.const_int(m, false), "d.row"));
        let dst_idx = b!(self.bld.build_int_add(dst_idx, i, "d.idx"));
        let dst_ep = unsafe { b!(self.bld.build_gep(f64t, result_ptr, &[dst_idx], "d.ep")) };
        b!(self.bld.build_store(dst_ep, val));

        let j_next = b!(self
            .bld
            .build_int_add(j, i64t.const_int(1, false), "j.next"));
        b!(self.bld.build_store(jv, j_next));
        b!(self.bld.build_unconditional_branch(j_loop2));

        self.bld.position_at_end(j_end);
        let i_next = b!(self
            .bld
            .build_int_add(i, i64t.const_int(1, false), "i.next"));
        b!(self.bld.build_store(iv, i_next));
        b!(self.bld.build_unconditional_branch(i_loop));

        self.bld.position_at_end(i_end);
        Ok(result_ptr.into())
    }

    pub(in crate::codegen) fn compile_bit_intrinsic(
        &mut self,
        builtin: &hir::BuiltinFn,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let intrinsic = match builtin {
            hir::BuiltinFn::Popcount => "popcount",
            hir::BuiltinFn::Clz => "clz",
            hir::BuiltinFn::Ctz => "ctz",
            hir::BuiltinFn::RotateLeft => "rotate_left",
            hir::BuiltinFn::RotateRight => "rotate_right",
            hir::BuiltinFn::Bswap => "bswap",
            _ => return Err("not a bit intrinsic".into()),
        };
        if args.is_empty() {
            return Err(format!("{intrinsic}() requires at least one argument"));
        }
        let val = self.compile_expr(&args[0])?;
        let int_val = val.into_int_value();
        let bw = int_val.get_type().get_bit_width();
        let llvm_name = match intrinsic {
            "popcount" => format!("llvm.ctpop.i{bw}"),
            "clz" => format!("llvm.ctlz.i{bw}"),
            "ctz" => format!("llvm.cttz.i{bw}"),
            "rotate_left" => format!("llvm.fshl.i{bw}"),
            "rotate_right" => format!("llvm.fshr.i{bw}"),
            "bswap" => format!("llvm.bswap.i{bw}"),
            _ => unreachable!(),
        };
        let it = int_val.get_type();
        match intrinsic {
            "popcount" | "bswap" => {
                let ft = it.fn_type(&[it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                Ok(b!(self.bld.build_call(f, &[int_val.into()], intrinsic))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "clz" | "ctz" => {
                let false_val = self.ctx.bool_type().const_int(0, false);
                let ft = it.fn_type(&[it.into(), self.ctx.bool_type().into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                Ok(b!(self
                    .bld
                    .build_call(f, &[int_val.into(), false_val.into()], intrinsic))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void"))
            }
            "rotate_left" | "rotate_right" => {
                if args.len() < 2 {
                    return Err(format!("{intrinsic}() requires two arguments"));
                }
                let amt = self.compile_expr(&args[1])?.into_int_value();
                let ft = it.fn_type(&[it.into(), it.into(), it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                Ok(b!(self.bld.build_call(
                    f,
                    &[int_val.into(), int_val.into(), amt.into()],
                    intrinsic
                ))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void"))
            }
            _ => Err(format!("unknown intrinsic: {intrinsic}")),
        }
    }

    pub(in crate::codegen) fn compile_volatile_load(
        &mut self,
        ptr_val: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lty = self.llvm_ty(inner);
        let ptr = ptr_val.into_pointer_value();
        let load = b!(self.bld.build_load(lty, ptr, "vol.load"));
        load.as_instruction_value().expect("ICE: not an instruction")
            .set_volatile(true)
            .unwrap();
        Ok(load)
    }

    pub(in crate::codegen) fn compile_volatile_store(
        &mut self,
        ptr_val: BasicValueEnum<'ctx>,
        val: BasicValueEnum<'ctx>,
        _inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = ptr_val.into_pointer_value();
        let store = b!(self.bld.build_store(ptr, val));
        store.set_volatile(true).expect("ICE: set_volatile failed");
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn compile_wrapping_op(
        &mut self,
        builtin: &hir::BuiltinFn,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lhs = self.compile_expr(&args[0])?.into_int_value();
        let rhs = self.compile_expr(&args[1])?.into_int_value();
        Ok(match builtin {
            hir::BuiltinFn::WrappingAdd => b!(self.bld.build_int_add(lhs, rhs, "wrap.add")).into(),
            hir::BuiltinFn::WrappingSub => b!(self.bld.build_int_sub(lhs, rhs, "wrap.sub")).into(),
            hir::BuiltinFn::WrappingMul => b!(self.bld.build_int_mul(lhs, rhs, "wrap.mul")).into(),
            _ => unreachable!(),
        })
    }

    pub(in crate::codegen) fn compile_saturating_op(
        &mut self,
        builtin: &hir::BuiltinFn,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lhs = self.compile_expr(&args[0])?.into_int_value();
        let rhs = self.compile_expr(&args[1])?.into_int_value();
        let bw = lhs.get_type().get_bit_width();
        let signed = args[0].ty.is_signed();
        let (intrinsic_name, it) = match builtin {
            hir::BuiltinFn::SaturatingAdd if signed => {
                (format!("llvm.sadd.sat.i{bw}"), lhs.get_type())
            }
            hir::BuiltinFn::SaturatingAdd => (format!("llvm.uadd.sat.i{bw}"), lhs.get_type()),
            hir::BuiltinFn::SaturatingSub if signed => {
                (format!("llvm.ssub.sat.i{bw}"), lhs.get_type())
            }
            hir::BuiltinFn::SaturatingSub => (format!("llvm.usub.sat.i{bw}"), lhs.get_type()),
            hir::BuiltinFn::SaturatingMul => {
                return self.compile_saturating_mul(lhs, rhs, signed);
            }
            _ => unreachable!(),
        };
        let ft = it.fn_type(&[it.into(), it.into()], false);
        let f = self
            .module
            .get_function(&intrinsic_name)
            .unwrap_or_else(|| self.module.add_function(&intrinsic_name, ft, None));
        Ok(b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "sat"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void"))
    }

    pub(in crate::codegen) fn compile_saturating_mul(
        &mut self,
        lhs: inkwell::values::IntValue<'ctx>,
        rhs: inkwell::values::IntValue<'ctx>,
        signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.current_fn();
        let bw = lhs.get_type().get_bit_width();
        let it = lhs.get_type();
        let intrinsic = if signed {
            format!("llvm.smul.with.overflow.i{bw}")
        } else {
            format!("llvm.umul.with.overflow.i{bw}")
        };
        let overflow_ty = self
            .ctx
            .struct_type(&[it.into(), self.ctx.bool_type().into()], false);
        let ft = overflow_ty.fn_type(&[it.into(), it.into()], false);
        let f = self
            .module
            .get_function(&intrinsic)
            .unwrap_or_else(|| self.module.add_function(&intrinsic, ft, None));
        let result = b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "smul"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void");
        let val = b!(self
            .bld
            .build_extract_value(result.into_struct_value(), 0, "mul.val"))
        .into_int_value();
        let overflowed = b!(self
            .bld
            .build_extract_value(result.into_struct_value(), 1, "mul.of"))
        .into_int_value();

        let max_val = if signed {
            it.const_int((1u64 << (bw - 1)) - 1, false)
        } else {
            it.const_all_ones()
        };
        let clamped: BasicValueEnum = b!(self.bld.build_select::<BasicValueEnum, _>(
            overflowed,
            max_val.into(),
            val.into(),
            "sat.mul"
        ));

        let _ = fv;
        Ok(clamped)
    }

    pub(in crate::codegen) fn compile_checked_op(
        &mut self,
        builtin: &hir::BuiltinFn,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lhs = self.compile_expr(&args[0])?.into_int_value();
        let rhs = self.compile_expr(&args[1])?.into_int_value();
        let bw = lhs.get_type().get_bit_width();
        let it = lhs.get_type();
        let signed = args[0].ty.is_signed();
        let intrinsic = match builtin {
            hir::BuiltinFn::CheckedAdd if signed => format!("llvm.sadd.with.overflow.i{bw}"),
            hir::BuiltinFn::CheckedAdd => format!("llvm.uadd.with.overflow.i{bw}"),
            hir::BuiltinFn::CheckedSub if signed => format!("llvm.ssub.with.overflow.i{bw}"),
            hir::BuiltinFn::CheckedSub => format!("llvm.usub.with.overflow.i{bw}"),
            hir::BuiltinFn::CheckedMul if signed => format!("llvm.smul.with.overflow.i{bw}"),
            hir::BuiltinFn::CheckedMul => format!("llvm.umul.with.overflow.i{bw}"),
            _ => unreachable!(),
        };
        let overflow_ty = self
            .ctx
            .struct_type(&[it.into(), self.ctx.bool_type().into()], false);
        let ft = overflow_ty.fn_type(&[it.into(), it.into()], false);
        let f = self
            .module
            .get_function(&intrinsic)
            .unwrap_or_else(|| self.module.add_function(&intrinsic, ft, None));
        let result = b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "chk"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void");
        Ok(result)
    }

    pub(in crate::codegen) fn ensure_signal(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("signal").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i32t = self.ctx.i32_type();
            let ft = ptr_ty.fn_type(&[i32t.into(), ptr_ty.into()], false);
            self.module
                .add_function("signal", ft, Some(inkwell::module::Linkage::External))
        })
    }

    pub(in crate::codegen) fn ensure_raise(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("raise").unwrap_or_else(|| {
            let i32t = self.ctx.i32_type();
            let ft = i32t.fn_type(&[i32t.into()], false);
            self.module
                .add_function("raise", ft, Some(inkwell::module::Linkage::External))
        })
    }

    pub(in crate::codegen) fn compile_signal_handle(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let signum = self.compile_expr(&args[0])?;
        let handler = self.compile_expr(&args[1])?;
        let signal_fn = self.ensure_signal();
        let sig32 = b!(self.bld.build_int_truncate(
            signum.into_int_value(),
            self.ctx.i32_type(),
            "sig.trunc"
        ));
        b!(self
            .bld
            .build_call(signal_fn, &[sig32.into(), handler.into()], "sig"));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn compile_signal_raise(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let signum = self.compile_expr(&args[0])?;
        let raise_fn = self.ensure_raise();
        let sig32 = b!(self.bld.build_int_truncate(
            signum.into_int_value(),
            self.ctx.i32_type(),
            "sig.trunc"
        ));
        Ok(b!(self.bld.build_call(raise_fn, &[sig32.into()], "raise"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void"))
    }

    pub(in crate::codegen) fn compile_signal_ignore(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let signum = self.compile_expr(&args[0])?;
        let signal_fn = self.ensure_signal();
        let sig32 = b!(self.bld.build_int_truncate(
            signum.into_int_value(),
            self.ctx.i32_type(),
            "sig.trunc"
        ));
        let sig_ign = self
            .bld
            .build_int_to_ptr(
                self.ctx.i64_type().const_int(1, false),
                self.ctx.ptr_type(AddressSpace::default()),
                "sig.ign",
            )
            .unwrap();
        b!(self
            .bld
            .build_call(signal_fn, &[sig32.into(), sig_ign.into()], "sig.ign"));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn compile_assert(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let cond_expr = &args[0];
        let fv = self.current_fn();
        let cond_val = self.compile_expr(cond_expr)?;
        let cond = self.to_bool(cond_val);

        let pass_bb = self.ctx.append_basic_block(fv, "assert.pass");
        let fail_bb = self.ctx.append_basic_block(fv, "assert.fail");
        b!(self.bld.build_conditional_branch(cond, pass_bb, fail_bb));

        self.bld.position_at_end(fail_bb);
        let printf = crate::codegen::fn_or_die(&self.module, "printf");
        let line = cond_expr.span.line;
        // Use the descriptive message if provided
        let desc = if args.len() > 1 {
            if let hir::ExprKind::Str(ref s) = args[1].kind {
                s.clone()
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        let msg = if desc.is_empty() {
            format!("assertion failed at line {line}\n\0")
        } else {
            format!("assertion failed: {desc} (line {line})\n\0")
        };
        let gs = b!(self.bld.build_global_string_ptr(&msg, "assert.msg"));
        b!(self
            .bld
            .build_call(printf, &[gs.as_pointer_value().into()], ""));
        let exit_fn = self.module.get_function("exit").unwrap_or_else(|| {
            let i32t = self.ctx.i32_type();
            self.module.add_function(
                "exit",
                self.ctx.void_type().fn_type(&[i32t.into()], false),
                Some(Linkage::External),
            )
        });
        b!(self.bld.build_call(
            exit_fn,
            &[self.ctx.i32_type().const_int(1, false).into()],
            ""
        ));
        b!(self.bld.build_unreachable());

        self.bld.position_at_end(pass_bb);
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn compile_f64_intrinsic(
        &mut self,
        name: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let f64t = self.ctx.f64_type();
        let param_types: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> =
            (0..args.len()).map(|_| f64t.into()).collect();
        let f = self.module.get_function(name).unwrap_or_else(|| {
            self.module
                .add_function(name, f64t.fn_type(&param_types, false), None)
        });
        let compiled: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = args
            .iter()
            .map(|a| Ok(self.compile_expr(a)?.into_float_value().into()))
            .collect::<Result<_, String>>()?;
        Ok(b!(self.bld.build_call(f, &compiled, ""))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void"))
    }
}
