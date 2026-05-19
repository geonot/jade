use super::*;

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
            hir::BuiltinFn::Print => self.compile_print(args),
            hir::BuiltinFn::ToString => {
                if args.len() != 1 {
                    return Err("to_string() takes 1 argument".into());
                }
                self.compile_to_string(&args[0])
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
            hir::BuiltinFn::SignalDefault => {
                if args.len() != 1 {
                    return Err("signal_default() takes 1 argument".into());
                }
                self.compile_signal_default(args)
            }
            hir::BuiltinFn::SignalKill => {
                if args.len() != 2 {
                    return Err("signal_kill() takes 2 arguments".into());
                }
                self.compile_signal_kill(args)
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
                self.compile_assert(args)
            }
            hir::BuiltinFn::Ln => self.compile_f64_intrinsic("llvm.log.f64", args),
            hir::BuiltinFn::Log2 => self.compile_f64_intrinsic("llvm.log2.f64", args),
            hir::BuiltinFn::Log10 => self.compile_f64_intrinsic("llvm.log10.f64", args),
            hir::BuiltinFn::Exp => self.compile_f64_intrinsic("llvm.exp.f64", args),
            hir::BuiltinFn::Exp2 => self.compile_f64_intrinsic("llvm.exp2.f64", args),
            hir::BuiltinFn::PowF => self.compile_f64_intrinsic("llvm.pow.f64", args),
            hir::BuiltinFn::Copysign => self.compile_f64_intrinsic("llvm.copysign.f64", args),
            hir::BuiltinFn::Fma => self.compile_f64_intrinsic("llvm.fma.f64", args),
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
            hir::BuiltinFn::AtomicLoad
            | hir::BuiltinFn::AtomicStore
            | hir::BuiltinFn::AtomicAdd
            | hir::BuiltinFn::AtomicSub
            | hir::BuiltinFn::AtomicCas
            | hir::BuiltinFn::CompTimeTypeOf
            | hir::BuiltinFn::CompTimeFieldsOf
            | hir::BuiltinFn::CompTimeSizeOf => Err(format!(
                "builtin {:?} should not appear in codegen",
                builtin
            )),
            hir::BuiltinFn::CharMethod(method) => self.compile_char_method(&method.as_str(), args),
            hir::BuiltinFn::Matmul => self.compile_matmul(args),
            hir::BuiltinFn::RegexMatch | hir::BuiltinFn::RegexFindAll => Err(format!(
                "builtin {:?} should be lowered to string methods",
                builtin
            )),
            hir::BuiltinFn::ConstantTimeEq => self.compile_constant_time_eq(args),
            hir::BuiltinFn::VecWithAlloc | hir::BuiltinFn::MapWithAlloc => {
                let _alloc = self.compile_expr(&args[0])?;
                let ptr_t = self.ctx.ptr_type(AddressSpace::default());
                let i64t = self.ctx.i64_type();
                let malloc = self.ensure_malloc();
                let size = i64t.const_int(32, false);
                let ptr = b!(self.bld.build_call(malloc, &[size.into()], "alloc_col"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void")
                    .into_pointer_value();
                let memset = self
                    .module
                    .get_function("llvm.memset.p0.i64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.memset.p0.i64",
                            self.ctx.void_type().fn_type(
                                &[
                                    ptr_t.into(),
                                    self.ctx.i8_type().into(),
                                    i64t.into(),
                                    self.ctx.bool_type().into(),
                                ],
                                false,
                            ),
                            None,
                        )
                    });
                b!(self.bld.build_call(
                    memset,
                    &[
                        ptr.into(),
                        self.ctx.i8_type().const_zero().into(),
                        size.into(),
                        self.ctx.bool_type().const_zero().into()
                    ],
                    "",
                ));
                Ok(ptr.into())
            }
            hir::BuiltinFn::GradFn => Err("GradFn not yet implemented in codegen".into()),
            hir::BuiltinFn::Einsum => Err("Einsum should be handled via ExprKind".into()),
            hir::BuiltinFn::Likely | hir::BuiltinFn::Unlikely => {
                if args.len() != 1 {
                    return Err("likely/unlikely takes exactly 1 boolean argument".into());
                }
                let cond = self.compile_expr(&args[0])?;
                let i1ty = self.ctx.bool_type();
                let ft = i1ty.fn_type(&[i1ty.into(), i1ty.into()], false);
                let expect_fn = self
                    .module
                    .get_function("llvm.expect.i1")
                    .unwrap_or_else(|| self.module.add_function("llvm.expect.i1", ft, None));
                let expected = match builtin {
                    hir::BuiltinFn::Likely => i1ty.const_int(1, false),
                    _ => i1ty.const_int(0, false),
                };
                let result =
                    b!(self
                        .bld
                        .build_call(expect_fn, &[cond.into(), expected.into()], "expect"));
                Ok(self.call_result(result))
            }
            hir::BuiltinFn::FloatMethod(method) => {
                self.compile_float_method(&method.as_str(), args)
            }
        }
    }

    pub(in crate::codegen) fn compile_constant_time_eq(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let a = self.compile_expr(&args[0])?;
        let b = self.compile_expr(&args[1])?;

        if args[0].ty == Type::String {
            let ptr_t = self.ctx.ptr_type(AddressSpace::default());
            let bool_t = self.ctx.bool_type();
            let fn_type = bool_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
            let func = self
                .module
                .get_function("__jinn_constant_time_eq")
                .unwrap_or_else(|| {
                    self.module
                        .add_function("__jinn_constant_time_eq", fn_type, None)
                });
            let result = b!(self.bld.build_call(func, &[a.into(), b.into()], "ct.eq"));
            Ok(self.call_result(result))
        } else {
            let i64t = self.ctx.i64_type();
            let av = a.into_int_value();
            let bv = b.into_int_value();
            let xor = b!(self.bld.build_xor(av, bv, "ct.xor"));
            let is_zero = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                xor,
                i64t.const_int(0, false),
                "ct.iszero"
            ));
            Ok(is_zero.into())
        }
    }

    pub(in crate::codegen) fn compile_matmul(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let _a_ptr = self.compile_expr(&args[0])?;
        let _b_ptr = self.compile_expr(&args[1])?;
        Err("matmul: NDArray type removed".into())
    }

    pub(crate) fn compile_einsum(
        &mut self,
        notation: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = notation.split("->").collect();
        let (inp_str, out_str) = match parts.as_slice() {
            [inp, out] => (*inp, *out),
            _ => return Err(format!("invalid einsum notation: {notation}")),
        };
        let inputs: Vec<&str> = inp_str.split(',').collect();

        if inputs.len() == 2 && args.len() == 2 {
            if inputs[0] == "ij" && inputs[1] == "jk" && out_str == "ik" {
                return self.compile_matmul(args);
            }

            if inputs[0] == "i" && inputs[1] == "i" && out_str.is_empty() {
                return self.compile_einsum_dot(args);
            }
        }

        if inputs.len() == 1 && args.len() == 1 && inputs[0] == "ii" && out_str.is_empty() {
            return self.compile_einsum_trace(args);
        }

        if inputs.len() == 1 && args.len() == 1 && inputs[0] == "ij" && out_str == "ji" {
            return self.compile_einsum_transpose(args);
        }
        Err(format!("unsupported einsum pattern: {notation}"))
    }

    pub(in crate::codegen) fn compile_einsum_dot(
        &mut self,
        _args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        Err("einsum dot: NDArray type removed".into())
    }
}
