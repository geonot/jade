use super::*;

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
                let len = self.val(args[1]);
                let cap = if args.len() > 2 {
                    self.val(args[2])
                } else {
                    len
                };
                return Ok(Some(self.build_string(ptr, len, cap, "sfr")?));
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
                return Ok(Some(self.build_string(buf, len, size, "sfp")?));
            }
            "Chr" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let code = self.val(args[0]).into_int_value();
                let i8t = self.ctx.i8_type();
                let i64t = self.ctx.i64_type();
                // Narrow the code to a single byte (inverse of `String.char_at`).
                let byte = if code.get_type().get_bit_width() > 8 {
                    b!(self.bld.build_int_truncate(code, i8t, "chr.byte"))
                } else {
                    code
                };
                // Heap buffer: one data byte plus a trailing NUL for C interop.
                let size = i64t.const_int(2, false);
                let malloc = self.ensure_malloc();
                let buf = b!(self.bld.build_call(malloc, &[size.into()], "chr.buf"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                let bufp = buf.into_pointer_value();
                b!(self.bld.build_store(bufp, byte));
                let p1 = unsafe {
                    b!(self
                        .bld
                        .build_gep(i8t, bufp, &[i64t.const_int(1, false)], "chr.p1"))
                };
                b!(self.bld.build_store(p1, i8t.const_zero()));
                return Ok(Some(self.build_string(
                    buf,
                    i64t.const_int(1, false),
                    size,
                    "chr",
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
        if args.len() != 2 {
            return Ok(None);
        }
        let lhs = self.val(args[0]).into_int_value();
        let rhs = self.val(args[1]).into_int_value();
        let result = match builtin_name {
            "WrappingAdd" => b!(self.bld.build_int_add(lhs, rhs, "wrap.add")),
            "WrappingSub" => b!(self.bld.build_int_sub(lhs, rhs, "wrap.sub")),
            "WrappingMul" => b!(self.bld.build_int_mul(lhs, rhs, "wrap.mul")),

            "SaturatingAdd" => {
                let bw = lhs.get_type().get_bit_width();
                let name = format!("llvm.sadd.sat.i{bw}");
                let ft = lhs
                    .get_type()
                    .fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self
                    .module
                    .get_function(&name)
                    .unwrap_or_else(|| self.module.add_function(&name, ft, None));
                b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "sat.add"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void")
                    .into_int_value()
            }
            "SaturatingSub" => {
                let bw = lhs.get_type().get_bit_width();
                let name = format!("llvm.ssub.sat.i{bw}");
                let ft = lhs
                    .get_type()
                    .fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self
                    .module
                    .get_function(&name)
                    .unwrap_or_else(|| self.module.add_function(&name, ft, None));
                b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "sat.sub"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void")
                    .into_int_value()
            }
            "SaturatingMul" => {
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.smul.with.overflow.i{bw}");
                let ovf_ty = self
                    .ctx
                    .struct_type(&[lhs.get_type().into(), self.ctx.bool_type().into()], false);
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self
                    .module
                    .get_function(&intr)
                    .unwrap_or_else(|| self.module.add_function(&intr, ft, None));
                let r = b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "smul"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void")
                    .into_struct_value();
                let val = b!(self.bld.build_extract_value(r, 0, "smul.val")).into_int_value();
                let ovf = b!(self.bld.build_extract_value(r, 1, "smul.ovf")).into_int_value();
                let max_val = lhs.get_type().const_int(i64::MAX as u64, false);
                b!(self.bld.build_select(ovf, max_val, val, "sat.mul")).into_int_value()
            }

            "CheckedAdd" => {
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.sadd.with.overflow.i{bw}");
                let ovf_ty = self
                    .ctx
                    .struct_type(&[lhs.get_type().into(), self.ctx.bool_type().into()], false);
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self
                    .module
                    .get_function(&intr)
                    .unwrap_or_else(|| self.module.add_function(&intr, ft, None));
                let r = b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "cadd"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void")
                    .into_struct_value();

                b!(self.bld.build_extract_value(r, 0, "cadd.val")).into_int_value()
            }
            "CheckedSub" => {
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.ssub.with.overflow.i{bw}");
                let ovf_ty = self
                    .ctx
                    .struct_type(&[lhs.get_type().into(), self.ctx.bool_type().into()], false);
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self
                    .module
                    .get_function(&intr)
                    .unwrap_or_else(|| self.module.add_function(&intr, ft, None));
                let r = b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "csub"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void")
                    .into_struct_value();
                b!(self.bld.build_extract_value(r, 0, "csub.val")).into_int_value()
            }
            "CheckedMul" => {
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.smul.with.overflow.i{bw}");
                let ovf_ty = self
                    .ctx
                    .struct_type(&[lhs.get_type().into(), self.ctx.bool_type().into()], false);
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self
                    .module
                    .get_function(&intr)
                    .unwrap_or_else(|| self.module.add_function(&intr, ft, None));
                let r = b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "cmul"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void")
                    .into_struct_value();
                b!(self.bld.build_extract_value(r, 0, "cmul.val")).into_int_value()
            }
            _ => return Ok(None),
        };
        Ok(Some(result.into()))
    }
}
