use super::*;
use inkwell::IntPredicate;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn try_handle_overflow_builtin(
        &mut self,
        name: &str,
        args: &[mir::ValueId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let builtin_name = match name.strip_prefix("__builtin_") {
            Some(n) => n,
            None => return Ok(None),
        };

        match builtin_name {
            "Bswap" | "Popcount" | "Clz" | "Ctz" | "RotateLeft" | "RotateRight" => {
                return self.try_handle_bit_builtin(builtin_name, args);
            }
            "PayloadMove" => return Ok(Some(self.val(args[1]))),
            "Likely" | "Unlikely" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let cond = self.val(args[0]);
                let i1ty = self.ctx.bool_type();
                let ft = i1ty.fn_type(&[i1ty.into(), i1ty.into()], false);
                let expect_fn = self
                    .module
                    .get_function("llvm.expect.i1")
                    .unwrap_or_else(|| self.module.add_function("llvm.expect.i1", ft, None));
                let expected = if builtin_name == "Likely" {
                    i1ty.const_int(1, false)
                } else {
                    i1ty.const_int(0, false)
                };
                let r =
                    b!(self
                        .bld
                        .build_call(expect_fn, &[cond.into(), expected.into()], "expect"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                return Ok(Some(r));
            }
            "PoolNew" => {
                if args.len() != 2 {
                    return Ok(None);
                }
                let obj_size = self.val(args[0]).into_int_value();
                let count = self.val(args[1]).into_int_value();
                let ptr_t = self.ctx.ptr_type(AddressSpace::default());
                let i64t = self.ctx.i64_type();
                let ft = ptr_t.fn_type(&[i64t.into(), i64t.into()], false);
                let func = self
                    .module
                    .get_function("jinn_pool_create")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "jinn_pool_create",
                            ft,
                            Some(inkwell::module::Linkage::External),
                        )
                    });
                let r = b!(self
                    .bld
                    .build_call(func, &[obj_size.into(), count.into()], "pool.new"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void");
                return Ok(Some(r));
            }
            "PoolAlloc" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let pool_ptr = self.val(args[0]).into_pointer_value();
                let ptr_t = self.ctx.ptr_type(AddressSpace::default());
                let ft = ptr_t.fn_type(&[ptr_t.into()], false);
                let func = self
                    .module
                    .get_function("jinn_pool_alloc")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "jinn_pool_alloc",
                            ft,
                            Some(inkwell::module::Linkage::External),
                        )
                    });
                let r = b!(self.bld.build_call(func, &[pool_ptr.into()], "pool.alloc"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                return Ok(Some(r));
            }
            "PoolFree" => {
                if args.len() != 2 {
                    return Ok(None);
                }
                let pool_ptr = self.val(args[0]).into_pointer_value();
                let obj_ptr = self.val(args[1]).into_pointer_value();
                let ptr_t = self.ctx.ptr_type(AddressSpace::default());
                let void_t = self.ctx.void_type();
                let ft = void_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
                let func = self
                    .module
                    .get_function("jinn_pool_free")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "jinn_pool_free",
                            ft,
                            Some(inkwell::module::Linkage::External),
                        )
                    });
                b!(self
                    .bld
                    .build_call(func, &[pool_ptr.into(), obj_ptr.into()], ""));
                return Ok(Some(self.ctx.i64_type().const_int(0, false).into()));
            }
            "PoolDestroy" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let pool_ptr = self.val(args[0]).into_pointer_value();
                let ptr_t = self.ctx.ptr_type(AddressSpace::default());
                let void_t = self.ctx.void_type();
                let ft = void_t.fn_type(&[ptr_t.into()], false);
                let func = self
                    .module
                    .get_function("jinn_pool_destroy")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "jinn_pool_destroy",
                            ft,
                            Some(inkwell::module::Linkage::External),
                        )
                    });
                b!(self.bld.build_call(func, &[pool_ptr.into()], ""));
                return Ok(Some(self.ctx.i64_type().const_int(0, false).into()));
            }
            "ToString" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let val = self.val(args[0]);
                let val_ty = self.value_types.get(&args[0]).cloned().unwrap_or(Type::I64);
                return Ok(Some(self.emit_to_string(val, &val_ty)?));
            }
            "Print" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let val = self.val(args[0]);
                let val_ty = self.value_types.get(&args[0]).cloned().unwrap_or(Type::I64);
                self.emit_print(val, &val_ty)?;
                return Ok(Some(self.ctx.i64_type().const_int(0, false).into()));
            }
            "FmtHex" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let val = self.val(args[0]).into_int_value();
                let i64t = self.ctx.i64_type();
                let wide = if val.get_type().get_bit_width() < 64 {
                    b!(self.bld.build_int_s_extend(val, i64t, "fw")).into()
                } else {
                    val.into()
                };
                return Ok(Some(self.snprintf_to_string("%lx", &[wide], "fh")?));
            }
            "FmtOct" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let val = self.val(args[0]).into_int_value();
                let i64t = self.ctx.i64_type();
                let wide = if val.get_type().get_bit_width() < 64 {
                    b!(self.bld.build_int_s_extend(val, i64t, "fw")).into()
                } else {
                    val.into()
                };
                return Ok(Some(self.snprintf_to_string("%lo", &[wide], "fo")?));
            }
            "FmtBin" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let val = self.val(args[0]).into_int_value();
                return Ok(Some(self.emit_fmt_bin(val)?));
            }
            "FmtFloat" => {
                if args.len() < 2 {
                    return Ok(None);
                }
                let x = self.val(args[0]).into_float_value();
                let decimals = self.val(args[1]).into_int_value();
                let dec_i32 =
                    b!(self
                        .bld
                        .build_int_truncate(decimals, self.ctx.i32_type(), "dec32"));
                return Ok(Some(self.snprintf_to_string(
                    "%.*f",
                    &[dec_i32.into(), x.into()],
                    "ff",
                )?));
            }
            "TimeMonotonic" => {
                return Ok(Some(self.compile_time_monotonic()?));
            }
            "SleepMs" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let ms = self.val(args[0]).into_int_value();
                return Ok(Some(self.emit_sleep_ms(ms)?));
            }
            "GetArgs" => {
                return Ok(Some(self.compile_get_args()?));
            }
            "StringFromRaw" => {
                if args.len() < 2 {
                    return Ok(None);
                }
                let ptr = self.val(args[0]);
                let len = self.val(args[1]).into_int_value();
                let i64t = self.ctx.i64_type();
                let size = b!(self
                    .bld
                    .build_int_nsw_add(len, i64t.const_int(1, false), "sfr.sz"));
                let malloc = self.ensure_malloc();
                let buf = b!(self.bld.build_call(malloc, &[size.into()], "sfr.buf"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                let memcpy = self.ensure_memcpy();
                b!(self
                    .bld
                    .build_call(memcpy, &[buf.into(), ptr.into(), len.into()], ""));
                let nul = unsafe {
                    b!(self.bld.build_gep(
                        self.ctx.i8_type(),
                        buf.into_pointer_value(),
                        &[len],
                        "sfr.nul"
                    ))
                };
                b!(self.bld.build_store(nul, self.ctx.i8_type().const_zero()));
                return Ok(Some(self.build_owned_string(buf, len, size, "sfr")?));
            }
            "StringFromPtr" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let ptr = self.val(args[0]);
                let i64t = self.ctx.i64_type();
                let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                let strlen = self.module.get_function("strlen").unwrap_or_else(|| {
                    self.module.add_function(
                        "strlen",
                        i64t.fn_type(&[ptr_ty.into()], false),
                        Some(inkwell::module::Linkage::External),
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
                return Ok(Some(self.build_owned_string(buf, len, size, "sfp")?));
            }
            "Chr" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let code = self.val(args[0]).into_int_value();
                let i8t = self.ctx.i8_type();
                let i64t = self.ctx.i64_type();

                let code64 = if code.get_type().get_bit_width() < 64 {
                    b!(self.bld.build_int_s_extend(code, i64t, "chr.code"))
                } else {
                    code
                };

                let size = i64t.const_int(5, false);
                let malloc = self.ensure_malloc();
                let buf = b!(self.bld.build_call(malloc, &[size.into()], "chr.buf"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                let bufp = buf.into_pointer_value();

                let enc = self
                    .module
                    .get_function("jinn_utf8_encode")
                    .unwrap_or_else(|| {
                        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                        let ft = i64t.fn_type(&[i64t.into(), ptr_ty.into()], false);
                        self.module.add_function(
                            "jinn_utf8_encode",
                            ft,
                            Some(inkwell::module::Linkage::External),
                        )
                    });
                let len = b!(self
                    .bld
                    .build_call(enc, &[code64.into(), bufp.into()], "chr.len"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_int_value();

                let nul = unsafe { b!(self.bld.build_gep(i8t, bufp, &[len], "chr.nul")) };
                b!(self.bld.build_store(nul, i8t.const_zero()));
                return Ok(Some(self.build_owned_string(buf, len, size, "chr")?));
            }
            "Byte" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let code = self.val(args[0]).into_int_value();
                let i8t = self.ctx.i8_type();
                let i64t = self.ctx.i64_type();

                let byte = if code.get_type().get_bit_width() > 8 {
                    b!(self.bld.build_int_truncate(code, i8t, "byte.byte"))
                } else {
                    code
                };

                let size = i64t.const_int(2, false);
                let malloc = self.ensure_malloc();
                let buf = b!(self.bld.build_call(malloc, &[size.into()], "byte.buf"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                let bufp = buf.into_pointer_value();
                b!(self.bld.build_store(bufp, byte));
                let p1 = unsafe {
                    b!(self
                        .bld
                        .build_gep(i8t, bufp, &[i64t.const_int(1, false)], "byte.p1"))
                };
                b!(self.bld.build_store(p1, i8t.const_zero()));
                return Ok(Some(self.build_owned_string(
                    buf,
                    i64t.const_int(1, false),
                    size,
                    "byte",
                )?));
            }
            "VolatileLoad" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let ptr = self.val(args[0]).into_pointer_value();
                let i64t = self.ctx.i64_type();
                let load = b!(self.bld.build_load(i64t, ptr, "vload"));
                load.as_instruction_value()
                    .expect("ICE: not an instruction")
                    .set_volatile(true)
                    .unwrap();
                return Ok(Some(load));
            }
            "VolatileStore" => {
                if args.len() < 2 {
                    return Ok(None);
                }
                let ptr = self.val(args[0]).into_pointer_value();
                let val = self.val(args[1]).into_int_value();
                let store_inst = b!(self.bld.build_store(ptr, val));
                store_inst
                    .set_volatile(true)
                    .expect("ICE: set_volatile failed");
                return Ok(Some(self.ctx.i64_type().const_int(0, false).into()));
            }
            "SignalHandle" => {
                if args.len() < 2 {
                    return Ok(None);
                }
                let signum = self.val(args[0]).into_int_value();
                let ptr_t = self.ctx.ptr_type(inkwell::AddressSpace::default());
                let i32t = self.ctx.i32_type();
                let ft = ptr_t.fn_type(&[i32t.into(), ptr_t.into()], false);
                let sig32 = b!(self.bld.build_int_truncate(signum, i32t, "sig32"));
                let handler_val = self.val(args[1]);
                let handler = match handler_val {
                    BasicValueEnum::PointerValue(ptr) => ptr,
                    BasicValueEnum::IntValue(int) => {
                        b!(self.bld.build_int_to_ptr(int, ptr_t, "handler_ptr"))
                    }
                    _ => return Err("signal_handle() handler must be a function pointer".into()),
                };
                let func = self.module.get_function("signal").unwrap_or_else(|| {
                    self.module
                        .add_function("signal", ft, Some(inkwell::module::Linkage::External))
                });
                b!(self
                    .bld
                    .build_call(func, &[sig32.into(), handler.into()], ""));
                return Ok(Some(self.ctx.i64_type().const_int(0, false).into()));
            }
            "SignalRaise" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let signum = self.val(args[0]).into_int_value();
                let i32t = self.ctx.i32_type();
                let ft = i32t.fn_type(&[i32t.into()], false);
                let sig32 = b!(self.bld.build_int_truncate(signum, i32t, "sig32"));
                let func = self.module.get_function("raise").unwrap_or_else(|| {
                    self.module
                        .add_function("raise", ft, Some(inkwell::module::Linkage::External))
                });
                let r = b!(self.bld.build_call(func, &[sig32.into()], "raise"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                return Ok(Some(r));
            }
            "SignalIgnore" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let signum = self.val(args[0]).into_int_value();
                let ptr_t = self.ctx.ptr_type(inkwell::AddressSpace::default());
                let i32t = self.ctx.i32_type();
                let ft = ptr_t.fn_type(&[i32t.into(), ptr_t.into()], false);
                let sig32 = b!(self.bld.build_int_truncate(signum, i32t, "sig32"));
                let sig_ign = b!(self.bld.build_int_to_ptr(
                    self.ctx.i64_type().const_int(1, false),
                    ptr_t,
                    "sig_ign"
                ));
                let func = self.module.get_function("signal").unwrap_or_else(|| {
                    self.module
                        .add_function("signal", ft, Some(inkwell::module::Linkage::External))
                });
                b!(self
                    .bld
                    .build_call(func, &[sig32.into(), sig_ign.into()], ""));
                return Ok(Some(self.ctx.i64_type().const_int(0, false).into()));
            }
            "SignalDefault" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let signum = self.val(args[0]).into_int_value();
                let ptr_t = self.ctx.ptr_type(inkwell::AddressSpace::default());
                let i32t = self.ctx.i32_type();
                let ft = ptr_t.fn_type(&[i32t.into(), ptr_t.into()], false);
                let sig32 = b!(self.bld.build_int_truncate(signum, i32t, "sig32"));
                let sig_dfl = b!(self.bld.build_int_to_ptr(
                    self.ctx.i64_type().const_int(0, false),
                    ptr_t,
                    "sig_dfl"
                ));
                let func = self.module.get_function("signal").unwrap_or_else(|| {
                    self.module
                        .add_function("signal", ft, Some(inkwell::module::Linkage::External))
                });
                b!(self
                    .bld
                    .build_call(func, &[sig32.into(), sig_dfl.into()], ""));
                return Ok(Some(self.ctx.i64_type().const_int(0, false).into()));
            }
            "SignalKill" => {
                if args.len() < 2 {
                    return Ok(None);
                }
                let pid = self.val(args[0]).into_int_value();
                let signum = self.val(args[1]).into_int_value();
                let i32t = self.ctx.i32_type();
                let ft = i32t.fn_type(&[i32t.into(), i32t.into()], false);
                let pid32 = b!(self.bld.build_int_truncate(pid, i32t, "pid32"));
                let sig32 = b!(self.bld.build_int_truncate(signum, i32t, "sig32"));
                let func = self.module.get_function("kill").unwrap_or_else(|| {
                    self.module
                        .add_function("kill", ft, Some(inkwell::module::Linkage::External))
                });
                let r = b!(self
                    .bld
                    .build_call(func, &[pid32.into(), sig32.into()], "kill"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void");
                return Ok(Some(r));
            }
            "Ln" | "Log2" | "Log10" | "Exp" | "Exp2" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let x = self.val(args[0]).into_float_value();
                let f64t = self.ctx.f64_type();
                let intrinsic = match builtin_name {
                    "Ln" => "llvm.log.f64",
                    "Log2" => "llvm.log2.f64",
                    "Log10" => "llvm.log10.f64",
                    "Exp" => "llvm.exp.f64",
                    "Exp2" => "llvm.exp2.f64",
                    _ => unreachable!(),
                };
                let ft = f64t.fn_type(&[f64t.into()], false);
                let func = self
                    .module
                    .get_function(intrinsic)
                    .unwrap_or_else(|| self.module.add_function(intrinsic, ft, None));
                let r = b!(self.bld.build_call(func, &[x.into()], "math"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                return Ok(Some(r));
            }
            "PowF" | "Copysign" => {
                if args.len() < 2 {
                    return Ok(None);
                }
                let x = self.val(args[0]).into_float_value();
                let y = self.val(args[1]).into_float_value();
                let f64t = self.ctx.f64_type();
                let intrinsic = match builtin_name {
                    "PowF" => "llvm.pow.f64",
                    "Copysign" => "llvm.copysign.f64",
                    _ => unreachable!(),
                };
                let ft = f64t.fn_type(&[f64t.into(), f64t.into()], false);
                let func = self
                    .module
                    .get_function(intrinsic)
                    .unwrap_or_else(|| self.module.add_function(intrinsic, ft, None));
                let r = b!(self.bld.build_call(func, &[x.into(), y.into()], "math"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                return Ok(Some(r));
            }
            "Fma" => {
                if args.len() < 3 {
                    return Ok(None);
                }
                let a = self.val(args[0]).into_float_value();
                let b_val = self.val(args[1]).into_float_value();
                let c = self.val(args[2]).into_float_value();
                let f64t = self.ctx.f64_type();
                let ft = f64t.fn_type(&[f64t.into(), f64t.into(), f64t.into()], false);
                let func = self
                    .module
                    .get_function("llvm.fma.f64")
                    .unwrap_or_else(|| self.module.add_function("llvm.fma.f64", ft, None));
                let r = b!(self
                    .bld
                    .build_call(func, &[a.into(), b_val.into(), c.into()], "fma"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void");
                return Ok(Some(r));
            }
            _ => {}
        }
        use crate::integer_arithmetic::{IntegerArithmetic, IntegerOp, OverflowMode};
        let Some(op) = IntegerArithmetic::from_mir(name) else {
            return Ok(None);
        };
        if args.len() != 2 {
            return Err(format!("ICE: {} requires two operands", op.source_name));
        }
        let ty = self
            .value_types
            .get(&args[0])
            .ok_or("ICE: missing integer operand type")?;
        if !ty.is_int() || self.value_types.get(&args[1]) != Some(ty) {
            return Err(format!("ICE: invalid {} operand types", op.source_name));
        }
        let signed = ty.is_signed();
        let lhs = self.val(args[0]).into_int_value();
        let rhs = self.val(args[1]).into_int_value();
        if op.mode == OverflowMode::Wrapping {
            let result = match op.op {
                IntegerOp::Add => b!(self.bld.build_int_add(lhs, rhs, "wrap.add")),
                IntegerOp::Sub => b!(self.bld.build_int_sub(lhs, rhs, "wrap.sub")),
                IntegerOp::Mul => b!(self.bld.build_int_mul(lhs, rhs, "wrap.mul")),
            };
            return Ok(Some(result.into()));
        }
        let pair = self.overflow_pair(op.op, lhs, rhs, signed)?;
        if op.mode == OverflowMode::Checked {
            return Ok(Some(pair.into()));
        }
        let value = b!(self.bld.build_extract_value(pair, 0, "sat.value")).into_int_value();
        let overflow = b!(self.bld.build_extract_value(pair, 1, "sat.overflow")).into_int_value();
        let it = lhs.get_type();
        let bound = if signed {
            let min = it.const_int(1u64 << (it.get_bit_width() - 1), false);
            let max = it.const_int((1u64 << (it.get_bit_width() - 1)) - 1, false);
            let negative = match op.op {
                IntegerOp::Mul => {
                    let signs = b!(self.bld.build_xor(lhs, rhs, "sat.signs"));
                    b!(self.bld.build_int_compare(
                        IntPredicate::SLT,
                        signs,
                        it.const_zero(),
                        "sat.negative"
                    ))
                }
                IntegerOp::Add | IntegerOp::Sub => b!(self.bld.build_int_compare(
                    IntPredicate::SLT,
                    lhs,
                    it.const_zero(),
                    "sat.negative"
                )),
            };
            b!(self.bld.build_select(negative, min, max, "sat.bound")).into_int_value()
        } else if matches!(op.op, IntegerOp::Sub) {
            it.const_zero()
        } else {
            it.const_all_ones()
        };
        Ok(Some(b!(self.bld.build_select(
            overflow,
            bound,
            value,
            "sat.result"
        ))))
    }
}
