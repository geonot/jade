use inkwell::module::Linkage;
use inkwell::values::{BasicValue, BasicValueEnum};
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_builtin(
        &mut self,
        builtin: &hir::BuiltinFn,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match builtin {
            hir::BuiltinFn::ActorSpawn | hir::BuiltinFn::ActorSend => {
                Err("actor builtins are dispatched via ExprKind, not BuiltinFn".into())
            }
            hir::BuiltinFn::Log => self.compile_log(args),
            hir::BuiltinFn::ToString => {
                if args.len() != 1 {
                    return Err("to_string() takes 1 argument".into());
                }
                self.compile_to_string(&args[0])
            }
            hir::BuiltinFn::RcAlloc => {
                if args.len() != 1 {
                    return Err("rc() takes 1 argument".into());
                }
                let val = self.compile_expr(&args[0])?;
                self.rc_alloc(&args[0].ty, val)
            }
            hir::BuiltinFn::RcRetain => {
                if args.len() != 1 {
                    return Err("rc_retain() takes 1 argument".into());
                }
                let val = self.compile_expr(&args[0])?;
                if let Type::Rc(inner) = &args[0].ty {
                    self.rc_retain(val, inner)?;
                    Ok(val)
                } else {
                    Err("rc_retain: argument must be Rc type".into())
                }
            }
            hir::BuiltinFn::RcRelease => {
                if args.len() != 1 {
                    return Err("rc_release() takes 1 argument".into());
                }
                let val = self.compile_expr(&args[0])?;
                if let Type::Rc(inner) = &args[0].ty {
                    self.rc_release(val, inner)?;
                    Ok(self.ctx.i64_type().const_int(0, false).into())
                } else {
                    Err("rc_release: argument must be Rc type".into())
                }
            }
            hir::BuiltinFn::WeakDowngrade => {
                if args.len() != 1 {
                    return Err("weak() takes 1 argument (an rc value)".into());
                }
                let val = self.compile_expr(&args[0])?;
                if let Type::Rc(inner) = &args[0].ty {
                    self.weak_downgrade(val, inner)
                } else {
                    Err("weak(): argument must be rc type".into())
                }
            }
            hir::BuiltinFn::WeakUpgrade => {
                if args.len() != 1 {
                    return Err("weak_upgrade() takes 1 argument".into());
                }
                let val = self.compile_expr(&args[0])?;
                if let Type::Weak(inner) = &args[0].ty {
                    self.weak_upgrade(val, inner)
                } else {
                    Err("weak_upgrade(): argument must be weak type".into())
                }
            }
            hir::BuiltinFn::WeakAlloc => {
                Err("weak_alloc is internal — use weak() on an rc value".into())
            }
            hir::BuiltinFn::VolatileLoad => {
                if args.len() != 1 {
                    return Err("volatile_load() takes 1 argument".into());
                }
                let ptr_val = self.compile_expr(&args[0])?;
                if let Type::Ptr(inner) = &args[0].ty {
                    self.compile_volatile_load(ptr_val, inner)
                } else {
                    Err("volatile_load(): argument must be a pointer".into())
                }
            }
            hir::BuiltinFn::VolatileStore => {
                if args.len() != 2 {
                    return Err("volatile_store() takes 2 arguments (ptr, value)".into());
                }
                let ptr_val = self.compile_expr(&args[0])?;
                let val = self.compile_expr(&args[1])?;
                if let Type::Ptr(inner) = &args[0].ty {
                    self.compile_volatile_store(ptr_val, val, inner)
                } else {
                    Err("volatile_store(): first argument must be a pointer".into())
                }
            }
            hir::BuiltinFn::WrappingAdd
            | hir::BuiltinFn::WrappingSub
            | hir::BuiltinFn::WrappingMul => {
                if args.len() != 2 {
                    return Err("wrapping op takes 2 arguments".into());
                }
                self.compile_wrapping_op(builtin, args)
            }
            hir::BuiltinFn::SaturatingAdd
            | hir::BuiltinFn::SaturatingSub
            | hir::BuiltinFn::SaturatingMul => {
                if args.len() != 2 {
                    return Err("saturating op takes 2 arguments".into());
                }
                self.compile_saturating_op(builtin, args)
            }
            hir::BuiltinFn::CheckedAdd
            | hir::BuiltinFn::CheckedSub
            | hir::BuiltinFn::CheckedMul => {
                if args.len() != 2 {
                    return Err("checked op takes 2 arguments".into());
                }
                self.compile_checked_op(builtin, args)
            }
            hir::BuiltinFn::SignalHandle => {
                if args.len() != 2 {
                    return Err("signal_handle() takes 2 arguments (signum, handler)".into());
                }
                self.compile_signal_handle(args)
            }
            hir::BuiltinFn::SignalRaise => {
                if args.len() != 1 {
                    return Err("signal_raise() takes 1 argument".into());
                }
                self.compile_signal_raise(args)
            }
            hir::BuiltinFn::SignalIgnore => {
                if args.len() != 1 {
                    return Err("signal_ignore() takes 1 argument".into());
                }
                self.compile_signal_ignore(args)
            }
            hir::BuiltinFn::Popcount
            | hir::BuiltinFn::Clz
            | hir::BuiltinFn::Ctz
            | hir::BuiltinFn::RotateLeft
            | hir::BuiltinFn::RotateRight
            | hir::BuiltinFn::Bswap => self.compile_bit_intrinsic(builtin, args),
            hir::BuiltinFn::Assert => {
                if args.is_empty() {
                    return Err("assert requires a condition".into());
                }
                self.compile_assert(&args[0])
            }
            hir::BuiltinFn::Ln => self.compile_f64_intrinsic("llvm.log.f64", args),
            hir::BuiltinFn::Log2 => self.compile_f64_intrinsic("llvm.log2.f64", args),
            hir::BuiltinFn::Log10 => self.compile_f64_intrinsic("llvm.log10.f64", args),
            hir::BuiltinFn::Exp => self.compile_f64_intrinsic("llvm.exp.f64", args),
            hir::BuiltinFn::Exp2 => self.compile_f64_intrinsic("llvm.exp2.f64", args),
            hir::BuiltinFn::PowF => self.compile_f64_intrinsic2("llvm.pow.f64", args),
            hir::BuiltinFn::Copysign => self.compile_f64_intrinsic2("llvm.copysign.f64", args),
            hir::BuiltinFn::Fma => self.compile_f64_intrinsic3("llvm.fma.f64", args),
            hir::BuiltinFn::StringFromRaw => self.compile_string_from_raw(args),
            hir::BuiltinFn::StringFromPtr => self.compile_string_from_ptr(args),
            hir::BuiltinFn::GetArgs => self.compile_get_args(),
            hir::BuiltinFn::FmtFloat => self.compile_fmt_float(args),
            hir::BuiltinFn::FmtHex => self.compile_fmt_snprintf("%lx", args),
            hir::BuiltinFn::FmtOct => self.compile_fmt_snprintf("%lo", args),
            hir::BuiltinFn::FmtBin => self.compile_fmt_bin(args),
            hir::BuiltinFn::TimeMonotonic => self.compile_time_monotonic(),
            hir::BuiltinFn::SleepMs => self.compile_sleep_ms(args),
            hir::BuiltinFn::FileExists => self.compile_file_exists(args),
        }
    }

    fn compile_bit_intrinsic(
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
                    .unwrap())
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
                .unwrap())
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
                .unwrap())
            }
            _ => Err(format!("unknown intrinsic: {intrinsic}")),
        }
    }

    fn compile_volatile_load(
        &mut self,
        ptr_val: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lty = self.llvm_ty(inner);
        let ptr = ptr_val.into_pointer_value();
        let load = b!(self.bld.build_load(lty, ptr, "vol.load"));
        load.as_instruction_value()
            .unwrap()
            .set_volatile(true)
            .unwrap();
        Ok(load)
    }

    fn compile_volatile_store(
        &mut self,
        ptr_val: BasicValueEnum<'ctx>,
        val: BasicValueEnum<'ctx>,
        _inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = ptr_val.into_pointer_value();
        let store = b!(self.bld.build_store(ptr, val));
        store.set_volatile(true).unwrap();
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    fn compile_wrapping_op(
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

    fn compile_saturating_op(
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
            .unwrap())
    }

    fn compile_saturating_mul(
        &mut self,
        lhs: inkwell::values::IntValue<'ctx>,
        rhs: inkwell::values::IntValue<'ctx>,
        signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
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
            .unwrap();
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

    fn compile_checked_op(
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
            .unwrap();
        Ok(result)
    }

    fn ensure_signal(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("signal").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i32t = self.ctx.i32_type();
            let ft = ptr_ty.fn_type(&[i32t.into(), ptr_ty.into()], false);
            self.module
                .add_function("signal", ft, Some(inkwell::module::Linkage::External))
        })
    }

    fn ensure_raise(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("raise").unwrap_or_else(|| {
            let i32t = self.ctx.i32_type();
            let ft = i32t.fn_type(&[i32t.into()], false);
            self.module
                .add_function("raise", ft, Some(inkwell::module::Linkage::External))
        })
    }

    fn compile_signal_handle(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let signum = self.compile_expr(&args[0])?;
        let handler = self.compile_expr(&args[1])?;
        let signal_fn = self.ensure_signal();
        b!(self
            .bld
            .build_call(signal_fn, &[signum.into(), handler.into()], "sig"));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    fn compile_signal_raise(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let signum = self.compile_expr(&args[0])?;
        let raise_fn = self.ensure_raise();
        Ok(b!(self.bld.build_call(raise_fn, &[signum.into()], "raise"))
            .try_as_basic_value()
            .basic()
            .unwrap())
    }

    fn compile_signal_ignore(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let signum = self.compile_expr(&args[0])?;
        let signal_fn = self.ensure_signal();
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
            .build_call(signal_fn, &[signum.into(), sig_ign.into()], "sig.ign"));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    fn compile_assert(&mut self, cond_expr: &hir::Expr) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let cond_val = self.compile_expr(cond_expr)?;
        let cond = self.to_bool(cond_val);

        let pass_bb = self.ctx.append_basic_block(fv, "assert.pass");
        let fail_bb = self.ctx.append_basic_block(fv, "assert.fail");
        b!(self.bld.build_conditional_branch(cond, pass_bb, fail_bb));

        self.bld.position_at_end(fail_bb);
        let printf = self.module.get_function("printf").unwrap();
        let line = cond_expr.span.line;
        let msg = format!("assertion failed at line {line}\n\0");
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

    fn compile_f64_intrinsic(
        &mut self,
        name: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let f64t = self.ctx.f64_type();
        let f = self.module.get_function(name).unwrap_or_else(|| {
            self.module
                .add_function(name, f64t.fn_type(&[f64t.into()], false), None)
        });
        let v = self.compile_expr(&args[0])?.into_float_value();
        Ok(b!(self.bld.build_call(f, &[v.into()], ""))
            .try_as_basic_value()
            .basic()
            .unwrap())
    }

    fn compile_f64_intrinsic2(
        &mut self,
        name: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let f64t = self.ctx.f64_type();
        let f = self.module.get_function(name).unwrap_or_else(|| {
            self.module
                .add_function(name, f64t.fn_type(&[f64t.into(), f64t.into()], false), None)
        });
        let a = self.compile_expr(&args[0])?.into_float_value();
        let b_val = self.compile_expr(&args[1])?.into_float_value();
        Ok(b!(self.bld.build_call(f, &[a.into(), b_val.into()], ""))
            .try_as_basic_value()
            .basic()
            .unwrap())
    }

    fn compile_f64_intrinsic3(
        &mut self,
        name: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let f64t = self.ctx.f64_type();
        let f = self.module.get_function(name).unwrap_or_else(|| {
            self.module.add_function(
                name,
                f64t.fn_type(&[f64t.into(), f64t.into(), f64t.into()], false),
                None,
            )
        });
        let a = self.compile_expr(&args[0])?.into_float_value();
        let b_val = self.compile_expr(&args[1])?.into_float_value();
        let c = self.compile_expr(&args[2])?.into_float_value();
        Ok(b!(self
            .bld
            .build_call(f, &[a.into(), b_val.into(), c.into()], ""))
        .try_as_basic_value()
        .basic()
        .unwrap())
    }

    fn compile_string_from_raw(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(&args[0])?;
        let len = self.compile_expr(&args[1])?;
        let cap = if args.len() > 2 {
            self.compile_expr(&args[2])?
        } else {
            len
        };
        self.build_string(ptr, len, cap, "sfr")
    }

    fn compile_string_from_ptr(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(&args[0])?;
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let strlen = self.module.get_function("strlen").unwrap_or_else(|| {
            self.module.add_function(
                "strlen",
                i64t.fn_type(&[ptr_ty.into()], false),
                Some(Linkage::External),
            )
        });
        let len = b!(self.bld.build_call(strlen, &[ptr.into()], "sfp.len"))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let size = b!(self
            .bld
            .build_int_nsw_add(len, i64t.const_int(1, false), "sfp.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "sfp.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[buf.into(), ptr.into(), size.into()], ""));
        self.build_string(buf, len, size, "sfp")
    }

    fn compile_get_args(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let argc_g = self
            .module
            .get_global("__jade_argc")
            .ok_or("__jade_argc global not found")?;
        let argv_g = self
            .module
            .get_global("__jade_argv")
            .ok_or("__jade_argv global not found")?;
        let argc =
            b!(self.bld.build_load(i32t, argc_g.as_pointer_value(), "argc")).into_int_value();
        let argc64 = b!(self.bld.build_int_s_extend(argc, i64t, "argc64"));
        let argv = b!(self
            .bld
            .build_load(ptr_ty, argv_g.as_pointer_value(), "argv"))
        .into_pointer_value();

        let header_ptr = self.compile_vec_new(&[])?.into_pointer_value();
        let header_ty = self.vec_header_type();
        let st = self.string_type();
        let str_size: u64 = 24;

        let fv = self.cur_fn.unwrap();
        let loop_bb = self.ctx.append_basic_block(fv, "args.loop");
        let body_bb = self.ctx.append_basic_block(fv, "args.body");
        let done_bb = self.ctx.append_basic_block(fv, "args.done");
        let i_ptr = self.entry_alloca(i64t.into(), "args.i");
        b!(self.bld.build_store(i_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let i = b!(self.bld.build_load(i64t, i_ptr, "i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, i, argc64, "args.cond"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let arg_pp = unsafe { b!(self.bld.build_gep(ptr_ty, argv, &[i], "arg.pp")) };
        let arg_p = b!(self.bld.build_load(ptr_ty, arg_pp, "arg.p")).into_pointer_value();
        let strlen = self.module.get_function("strlen").unwrap_or_else(|| {
            self.module.add_function(
                "strlen",
                i64t.fn_type(&[ptr_ty.into()], false),
                Some(Linkage::External),
            )
        });
        let slen = b!(self.bld.build_call(strlen, &[arg_p.into()], "arg.len"))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let size = b!(self
            .bld
            .build_int_nsw_add(slen, i64t.const_int(1, false), "arg.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "arg.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[buf.into(), arg_p.into(), size.into()], ""));
        let s = self.build_string(buf, slen, size, "arg.s")?;

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "ga.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "ga.len")).into_int_value();
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "ga.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "ga.cap")).into_int_value();
        let needs_grow = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, len, cap, "ga.full"));
        let grow_bb = self.ctx.append_basic_block(fv, "ga.grow");
        let store_bb = self.ctx.append_basic_block(fv, "ga.store");
        b!(self
            .bld
            .build_conditional_branch(needs_grow, grow_bb, store_bb));

        self.bld.position_at_end(grow_bb);
        let doubled = b!(self
            .bld
            .build_int_nsw_mul(cap, i64t.const_int(2, false), "ga.dbl"));
        let new_cap_cmp = b!(self.bld.build_int_compare(
            IntPredicate::SGT,
            doubled,
            i64t.const_int(4, false),
            "ga.cmp"
        ));
        let new_cap =
            b!(self
                .bld
                .build_select(new_cap_cmp, doubled, i64t.const_int(4, false), "ga.nc"))
            .into_int_value();
        let new_size =
            b!(self
                .bld
                .build_int_nsw_mul(new_cap, i64t.const_int(str_size, false), "ga.ns"));
        let realloc = self.ensure_realloc();
        let data_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "ga.datap"));
        let old_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "ga.optr"));
        let new_ptr =
            b!(self
                .bld
                .build_call(realloc, &[old_ptr.into(), new_size.into()], "ga.nptr"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        b!(self.bld.build_store(data_gep, new_ptr));
        b!(self.bld.build_store(cap_gep, new_cap));
        b!(self.bld.build_unconditional_branch(store_bb));

        self.bld.position_at_end(store_bb);
        let data_gep2 = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "ga.dp2"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep2, "ga.dp")).into_pointer_value();
        let elem_gep = unsafe { b!(self.bld.build_gep(st, data_ptr, &[len], "ga.ep")) };
        b!(self.bld.build_store(elem_gep, s));
        let new_len = b!(self
            .bld
            .build_int_nsw_add(len, i64t.const_int(1, false), "ga.nl"));
        b!(self.bld.build_store(len_gep, new_len));

        let next = b!(self
            .bld
            .build_int_nsw_add(i, i64t.const_int(1, false), "args.next"));
        b!(self.bld.build_store(i_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(header_ptr.into())
    }
}
