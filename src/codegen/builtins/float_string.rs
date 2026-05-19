use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn compile_float_method(
        &mut self,
        method: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let receiver = self.compile_expr(&args[0])?.into_float_value();
        let f64t = self.ctx.f64_type();
        let i64t = self.ctx.i64_type();

        match method {
            "sqrt" => {
                let f = self
                    .module
                    .get_function("llvm.sqrt.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.sqrt.f64",
                            f64t.fn_type(&[f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "sqrt"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "abs" => {
                let f = self
                    .module
                    .get_function("llvm.fabs.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.fabs.f64",
                            f64t.fn_type(&[f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "abs"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "floor" => {
                let f = self
                    .module
                    .get_function("llvm.floor.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.floor.f64",
                            f64t.fn_type(&[f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "floor"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "ceil" => {
                let f = self
                    .module
                    .get_function("llvm.ceil.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.ceil.f64",
                            f64t.fn_type(&[f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "ceil"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "round" => {
                let f = self
                    .module
                    .get_function("llvm.round.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.round.f64",
                            f64t.fn_type(&[f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "round"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "trunc" => {
                let f = self
                    .module
                    .get_function("llvm.trunc.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.trunc.f64",
                            f64t.fn_type(&[f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "trunc"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }

            "sin" => {
                let f = self.module.get_function("llvm.sin.f64").unwrap_or_else(|| {
                    self.module.add_function(
                        "llvm.sin.f64",
                        f64t.fn_type(&[f64t.into()], false),
                        None,
                    )
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "sin"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "cos" => {
                let f = self.module.get_function("llvm.cos.f64").unwrap_or_else(|| {
                    self.module.add_function(
                        "llvm.cos.f64",
                        f64t.fn_type(&[f64t.into()], false),
                        None,
                    )
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "cos"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "cbrt" => {
                let f = self.module.get_function(method).unwrap_or_else(|| {
                    self.module.add_function(
                        method,
                        f64t.fn_type(&[f64t.into()], false),
                        Some(Linkage::External),
                    )
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], method))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "exp" => {
                let f = self.module.get_function("llvm.exp.f64").unwrap_or_else(|| {
                    self.module.add_function(
                        "llvm.exp.f64",
                        f64t.fn_type(&[f64t.into()], false),
                        None,
                    )
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "exp"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "exp2" => {
                let f = self
                    .module
                    .get_function("llvm.exp2.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.exp2.f64",
                            f64t.fn_type(&[f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "exp2"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "ln" => {
                let f = self.module.get_function("llvm.log.f64").unwrap_or_else(|| {
                    self.module.add_function(
                        "llvm.log.f64",
                        f64t.fn_type(&[f64t.into()], false),
                        None,
                    )
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "ln"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "log2" => {
                let f = self
                    .module
                    .get_function("llvm.log2.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.log2.f64",
                            f64t.fn_type(&[f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "log2"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "log10" => {
                let f = self
                    .module
                    .get_function("llvm.log10.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.log10.f64",
                            f64t.fn_type(&[f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "log10"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void"))
            }
            "recip" => {
                let one = f64t.const_float(1.0);
                Ok(b!(self.bld.build_float_div(one, receiver, "recip")).into())
            }
            "signum" => {
                let zero = f64t.const_float(0.0);
                let neg_one = f64t.const_float(-1.0);
                let pos_one = f64t.const_float(1.0);
                let is_pos = b!(self.bld.build_float_compare(
                    inkwell::FloatPredicate::OGT,
                    receiver,
                    zero,
                    "pos"
                ));
                let is_neg = b!(self.bld.build_float_compare(
                    inkwell::FloatPredicate::OLT,
                    receiver,
                    zero,
                    "neg"
                ));
                let sel1 = b!(self.bld.build_select(
                    is_neg,
                    BasicValueEnum::FloatValue(neg_one),
                    BasicValueEnum::FloatValue(zero),
                    "s1",
                ))
                .into_float_value();
                Ok(b!(self.bld.build_select(
                    is_pos,
                    BasicValueEnum::FloatValue(pos_one),
                    BasicValueEnum::FloatValue(sel1),
                    "signum"
                ))
                .into())
            }

            "pow" => {
                if args.len() < 2 {
                    return Err("pow() requires 1 argument".into());
                }
                let exp = self.compile_expr(&args[1])?.into_float_value();
                let f = self.module.get_function("llvm.pow.f64").unwrap_or_else(|| {
                    self.module.add_function(
                        "llvm.pow.f64",
                        f64t.fn_type(&[f64t.into(), f64t.into()], false),
                        None,
                    )
                });
                Ok(b!(self
                    .bld
                    .build_call(f, &[receiver.into(), exp.into()], "pow"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void"))
            }
            "atan2" => {
                if args.len() < 2 {
                    return Err("atan2() requires 1 argument".into());
                }
                let other = self.compile_expr(&args[1])?.into_float_value();
                let f = self.module.get_function("atan2").unwrap_or_else(|| {
                    self.module.add_function(
                        "atan2",
                        f64t.fn_type(&[f64t.into(), f64t.into()], false),
                        Some(Linkage::External),
                    )
                });
                Ok(b!(self
                    .bld
                    .build_call(f, &[receiver.into(), other.into()], "atan2"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void"))
            }
            "copysign" => {
                if args.len() < 2 {
                    return Err("copysign() requires 1 argument".into());
                }
                let sign = self.compile_expr(&args[1])?.into_float_value();
                let f = self
                    .module
                    .get_function("llvm.copysign.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.copysign.f64",
                            f64t.fn_type(&[f64t.into(), f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self
                    .bld
                    .build_call(f, &[receiver.into(), sign.into()], "copysign"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void"))
            }
            "min" => {
                if args.len() < 2 {
                    return Err("min() requires 1 argument".into());
                }
                let other = self.compile_expr(&args[1])?.into_float_value();
                let f = self
                    .module
                    .get_function("llvm.minnum.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.minnum.f64",
                            f64t.fn_type(&[f64t.into(), f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self
                    .bld
                    .build_call(f, &[receiver.into(), other.into()], "fmin"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void"))
            }
            "max" => {
                if args.len() < 2 {
                    return Err("max() requires 1 argument".into());
                }
                let other = self.compile_expr(&args[1])?.into_float_value();
                let f = self
                    .module
                    .get_function("llvm.maxnum.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.maxnum.f64",
                            f64t.fn_type(&[f64t.into(), f64t.into()], false),
                            None,
                        )
                    });
                Ok(b!(self
                    .bld
                    .build_call(f, &[receiver.into(), other.into()], "fmax"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void"))
            }

            "is_nan" => {
                let result = b!(self.bld.build_float_compare(
                    inkwell::FloatPredicate::UNO,
                    receiver,
                    receiver,
                    "isnan"
                ));
                Ok(result.into())
            }
            "is_infinite" => {
                let abs_f = self
                    .module
                    .get_function("llvm.fabs.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.fabs.f64",
                            f64t.fn_type(&[f64t.into()], false),
                            None,
                        )
                    });
                let abs_val = b!(self.bld.build_call(abs_f, &[receiver.into()], "abs"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void")
                    .into_float_value();
                let inf = f64t.const_float(f64::INFINITY);
                let result = b!(self.bld.build_float_compare(
                    inkwell::FloatPredicate::OEQ,
                    abs_val,
                    inf,
                    "isinf"
                ));
                Ok(result.into())
            }
            "is_finite" => {
                let abs_f = self
                    .module
                    .get_function("llvm.fabs.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.fabs.f64",
                            f64t.fn_type(&[f64t.into()], false),
                            None,
                        )
                    });
                let abs_val = b!(self.bld.build_call(abs_f, &[receiver.into()], "abs"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void")
                    .into_float_value();
                let inf = f64t.const_float(f64::INFINITY);
                let not_inf = b!(self.bld.build_float_compare(
                    inkwell::FloatPredicate::ONE,
                    abs_val,
                    inf,
                    "notinf"
                ));
                let not_nan = b!(self.bld.build_float_compare(
                    inkwell::FloatPredicate::ORD,
                    receiver,
                    receiver,
                    "notnan"
                ));
                Ok(b!(self.bld.build_and(not_inf, not_nan, "isfinite")).into())
            }
            "clamp" => {
                if args.len() < 3 {
                    return Err("clamp() takes 2 arguments (lo, hi)".into());
                }
                let lo = self.compile_expr(&args[1])?.into_float_value();
                let hi = self.compile_expr(&args[2])?.into_float_value();
                let min_f = self
                    .module
                    .get_function("llvm.minnum.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.minnum.f64",
                            f64t.fn_type(&[f64t.into(), f64t.into()], false),
                            None,
                        )
                    });
                let max_f = self
                    .module
                    .get_function("llvm.maxnum.f64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.maxnum.f64",
                            f64t.fn_type(&[f64t.into(), f64t.into()], false),
                            None,
                        )
                    });
                let min_val =
                    b!(self
                        .bld
                        .build_call(min_f, &[receiver.into(), hi.into()], "clamp.min"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void")
                    .into_float_value();
                Ok(b!(self
                    .bld
                    .build_call(max_f, &[min_val.into(), lo.into()], "clamp.max"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void"))
            }
            "to_int" => Ok(b!(self.bld.build_float_to_signed_int(receiver, i64t, "ftoi")).into()),
            _ => Err(format!("unknown float method '{method}'")),
        }
    }

    pub(in crate::codegen) fn compile_string_from_raw(
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

    pub(in crate::codegen) fn compile_string_from_ptr(
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
            .expect("ICE: call returned void")
            .into_int_value();
        let size = b!(self
            .bld
            .build_int_nsw_add(len, i64t.const_int(1, false), "sfp.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "sfp.buf"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void");
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[buf.into(), ptr.into(), size.into()], ""));
        self.build_string(buf, len, size, "sfp")
    }
}
