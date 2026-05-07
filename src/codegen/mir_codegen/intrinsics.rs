//! Builtin and intrinsic codegen: overflow arithmetic, bit operations, string formatting, and sleep.

use inkwell::AddressSpace;
use inkwell::values::{BasicValue, BasicValueEnum};
use crate::mir;
use crate::types::Type;
use super::super::b;
use super::super::Compiler;

impl<'ctx> Compiler<'ctx> {
    /// Handle overflow builtins that MIR lowered as `__builtin_WrappingAdd` etc.
    pub(super) fn try_handle_overflow_builtin(
        &mut self,
        name: &str,
        args: &[mir::ValueId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let builtin_name = match name.strip_prefix("__builtin_") {
            Some(n) => n,
            None => return Ok(None),
        };
        // ── Bit intrinsics (1 arg) ──
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
                let r = b!(self.bld.build_call(
                    expect_fn,
                    &[cond.into(), expected.into()],
                    "expect"
                ))
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
                    .get_function("jade_pool_create")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "jade_pool_create",
                            ft,
                            Some(inkwell::module::Linkage::External),
                        )
                    });
                let r = b!(self.bld.build_call(
                    func,
                    &[obj_size.into(), count.into()],
                    "pool.new"
                ))
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
                    .get_function("jade_pool_alloc")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "jade_pool_alloc",
                            ft,
                            Some(inkwell::module::Linkage::External),
                        )
                    });
                let r = b!(self
                    .bld
                    .build_call(func, &[pool_ptr.into()], "pool.alloc"))
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
                    .get_function("jade_pool_free")
                    .unwrap_or_else(|| {
                        self.module
                            .add_function("jade_pool_free", ft, Some(inkwell::module::Linkage::External))
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
                    .get_function("jade_pool_destroy")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "jade_pool_destroy",
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
                let dec_i32 = b!(self.bld.build_int_truncate(
                    decimals,
                    self.ctx.i32_type(),
                    "dec32"
                ));
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
                let size =
                    b!(self
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
            "VolatileLoad" => {
                if args.is_empty() {
                    return Ok(None);
                }
                let ptr = self.val(args[0]).into_pointer_value();
                let i64t = self.ctx.i64_type();
                let load = b!(self.bld.build_load(i64t, ptr, "vload"));
                load.as_instruction_value().expect("ICE: not an instruction")
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
                store_inst.set_volatile(true).expect("ICE: set_volatile failed");
                return Ok(Some(self.ctx.i64_type().const_int(0, false).into()));
            }
            "SignalHandle" => {
                if args.len() < 2 {
                    return Ok(None);
                }
                let signum = self.val(args[0]).into_int_value();
                let handler = self.val(args[1]).into_pointer_value();
                let ptr_t = self.ctx.ptr_type(inkwell::AddressSpace::default());
                let i32t = self.ctx.i32_type();
                let ft = ptr_t.fn_type(&[i32t.into(), ptr_t.into()], false);
                let sig32 = b!(self.bld.build_int_truncate(signum, i32t, "sig32"));
                let func = self.module.get_function("signal").unwrap_or_else(|| {
                    self.module.add_function(
                        "signal",
                        ft,
                        Some(inkwell::module::Linkage::External),
                    )
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
                    self.module.add_function(
                        "raise",
                        ft,
                        Some(inkwell::module::Linkage::External),
                    )
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
                )); // SIG_IGN = 1
                let func = self.module.get_function("signal").unwrap_or_else(|| {
                    self.module.add_function(
                        "signal",
                        ft,
                        Some(inkwell::module::Linkage::External),
                    )
                });
                b!(self
                    .bld
                    .build_call(func, &[sig32.into(), sig_ign.into()], ""));
                return Ok(Some(self.ctx.i64_type().const_int(0, false).into()));
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
                let r = b!(self
                    .bld
                    .build_call(func, &[x.into(), y.into()], "math"))
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
                let r =
                    b!(self
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
            // Wrapping ops — just normal LLVM int arithmetic (wraps naturally)
            "WrappingAdd" => b!(self.bld.build_int_add(lhs, rhs, "wrap.add")),
            "WrappingSub" => b!(self.bld.build_int_sub(lhs, rhs, "wrap.sub")),
            "WrappingMul" => b!(self.bld.build_int_mul(lhs, rhs, "wrap.mul")),
            // Saturating ops — use LLVM intrinsics
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
                b!(self
                    .bld
                    .build_call(f, &[lhs.into(), rhs.into()], "sat.add"))
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
                b!(self
                    .bld
                    .build_call(f, &[lhs.into(), rhs.into()], "sat.sub"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_int_value()
            }
            "SaturatingMul" => {
                // No LLVM intrinsic for sat mul; use checked mul + select
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.smul.with.overflow.i{bw}");
                let ovf_ty = self.ctx.struct_type(
                    &[lhs.get_type().into(), self.ctx.bool_type().into()],
                    false,
                );
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self
                    .module
                    .get_function(&intr)
                    .unwrap_or_else(|| self.module.add_function(&intr, ft, None));
                let r = b!(self
                    .bld
                    .build_call(f, &[lhs.into(), rhs.into()], "smul"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_struct_value();
                let val = b!(self.bld.build_extract_value(r, 0, "smul.val")).into_int_value();
                let ovf = b!(self.bld.build_extract_value(r, 1, "smul.ovf")).into_int_value();
                let max_val = lhs.get_type().const_int(i64::MAX as u64, false);
                b!(self.bld.build_select(ovf, max_val, val, "sat.mul")).into_int_value()
            }
            // Checked ops — return {value, overflow_flag}
            "CheckedAdd" => {
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.sadd.with.overflow.i{bw}");
                let ovf_ty = self.ctx.struct_type(
                    &[lhs.get_type().into(), self.ctx.bool_type().into()],
                    false,
                );
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self
                    .module
                    .get_function(&intr)
                    .unwrap_or_else(|| self.module.add_function(&intr, ft, None));
                let r = b!(self
                    .bld
                    .build_call(f, &[lhs.into(), rhs.into()], "cadd"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_struct_value();
                // Return just the value; overflow info is in the struct
                b!(self.bld.build_extract_value(r, 0, "cadd.val")).into_int_value()
            }
            "CheckedSub" => {
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.ssub.with.overflow.i{bw}");
                let ovf_ty = self.ctx.struct_type(
                    &[lhs.get_type().into(), self.ctx.bool_type().into()],
                    false,
                );
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self
                    .module
                    .get_function(&intr)
                    .unwrap_or_else(|| self.module.add_function(&intr, ft, None));
                let r = b!(self
                    .bld
                    .build_call(f, &[lhs.into(), rhs.into()], "csub"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_struct_value();
                b!(self.bld.build_extract_value(r, 0, "csub.val")).into_int_value()
            }
            "CheckedMul" => {
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.smul.with.overflow.i{bw}");
                let ovf_ty = self.ctx.struct_type(
                    &[lhs.get_type().into(), self.ctx.bool_type().into()],
                    false,
                );
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self
                    .module
                    .get_function(&intr)
                    .unwrap_or_else(|| self.module.add_function(&intr, ft, None));
                let r = b!(self
                    .bld
                    .build_call(f, &[lhs.into(), rhs.into()], "cmul"))
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

    /// Handle bit intrinsics: bswap, popcount, clz, ctz, rotate_left, rotate_right.
    pub(super) fn try_handle_bit_builtin(
        &mut self,
        name: &str,
        args: &[mir::ValueId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        if args.is_empty() {
            return Ok(None);
        }
        let val = self.val(args[0]).into_int_value();
        let bw = val.get_type().get_bit_width();
        let it = val.get_type();
        match name {
            "Bswap" => {
                let llvm_name = format!("llvm.bswap.i{bw}");
                let ft = it.fn_type(&[it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r = b!(self.bld.build_call(f, &[val.into()], "bswap"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                Ok(Some(r))
            }
            "Popcount" => {
                let llvm_name = format!("llvm.ctpop.i{bw}");
                let ft = it.fn_type(&[it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r = b!(self.bld.build_call(f, &[val.into()], "popcount"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                Ok(Some(r))
            }
            "Clz" => {
                let llvm_name = format!("llvm.ctlz.i{bw}");
                let false_val = self.ctx.bool_type().const_int(0, false);
                let ft = it.fn_type(&[it.into(), self.ctx.bool_type().into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r = b!(self
                    .bld
                    .build_call(f, &[val.into(), false_val.into()], "clz"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void");
                Ok(Some(r))
            }
            "Ctz" => {
                let llvm_name = format!("llvm.cttz.i{bw}");
                let false_val = self.ctx.bool_type().const_int(0, false);
                let ft = it.fn_type(&[it.into(), self.ctx.bool_type().into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r = b!(self
                    .bld
                    .build_call(f, &[val.into(), false_val.into()], "ctz"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void");
                Ok(Some(r))
            }
            "RotateLeft" => {
                if args.len() < 2 {
                    return Ok(None);
                }
                let amt = self.val(args[1]).into_int_value();
                let llvm_name = format!("llvm.fshl.i{bw}");
                let ft = it.fn_type(&[it.into(), it.into(), it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r =
                    b!(self
                        .bld
                        .build_call(f, &[val.into(), val.into(), amt.into()], "rotl"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                Ok(Some(r))
            }
            "RotateRight" => {
                if args.len() < 2 {
                    return Ok(None);
                }
                let amt = self.val(args[1]).into_int_value();
                let llvm_name = format!("llvm.fshr.i{bw}");
                let ft = it.fn_type(&[it.into(), it.into(), it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r =
                    b!(self
                        .bld
                        .build_call(f, &[val.into(), val.into(), amt.into()], "rotr"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                Ok(Some(r))
            }
            _ => Ok(None),
        }
    }

    /// Convert a value to a String, matching the sibling `compile_to_string` helper.
    pub(super) fn emit_to_string(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            Type::String => Ok(val),
            Type::I64 | Type::I32 | Type::I16 | Type::I8 => self.int_to_string(val, false),
            Type::U64 | Type::U32 | Type::U16 | Type::U8 => self.int_to_string(val, true),
            Type::F64 | Type::F32 => self.float_to_string(val),
            Type::Bool => self.bool_to_string(val),
            Type::Struct(name, _) => {
                let fn_name = format!("{name}_display");
                if let Some((fv, _, _)) = self.fns.get(&fn_name).cloned() {
                    let first_param_is_ptr = fv
                        .get_type()
                        .get_param_types()
                        .first()
                        .map(|t| t.is_pointer_type())
                        .unwrap_or(false);
                    let self_arg: BasicValueEnum<'ctx> =
                        if first_param_is_ptr && !val.is_pointer_value() {
                            let tmp = self.entry_alloca(val.get_type(), "display.self");
                            b!(self.bld.build_store(tmp, val));
                            tmp.into()
                        } else {
                            val
                        };
                    let result =
                        b!(self
                            .bld
                            .build_call(fv, &[self_arg.into()], "display.call"))
                        .try_as_basic_value()
                        .basic()
                        .expect("ICE: call returned void");
                    Ok(result)
                } else {
                    self.int_to_string(val, false)
                }
            }
            _ => self.int_to_string(val, false),
        }
    }

    pub(super) fn emit_fmt_bin(
        &mut self,
        val: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let malloc = self.ensure_malloc();
        let buf =
            b!(self
                .bld
                .build_call(malloc, &[i64t.const_int(65, false).into()], "fb.buf"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void");
        let buf_ptr = buf.into_pointer_value();

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let loop_bb = self.ctx.append_basic_block(fv, "fb.loop");
        let body_bb = self.ctx.append_basic_block(fv, "fb.body");
        let done_bb = self.ctx.append_basic_block(fv, "fb.done");

        let wide = if val.get_type().get_bit_width() < 64 {
            b!(self.bld.build_int_z_extend(val, i64t, "fb.w"))
        } else {
            val
        };

        let clz_name = "llvm.ctlz.i64";
        let clz = self.module.get_function(clz_name).unwrap_or_else(|| {
            let ft = i64t.fn_type(&[i64t.into(), self.ctx.bool_type().into()], false);
            self.module.add_function(clz_name, ft, None)
        });
        let lz = b!(self.bld.build_call(
            clz,
            &[
                wide.into(),
                self.ctx.bool_type().const_int(1, false).into()
            ],
            "fb.lz"
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void")
        .into_int_value();
        let raw_bits = b!(self
            .bld
            .build_int_nsw_sub(i64t.const_int(64, false), lz, "fb.nb"));
        let is_zero = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            wide,
            i64t.const_int(0, false),
            "fb.z"
        ));
        let nbits =
            b!(self
                .bld
                .build_select(is_zero, i64t.const_int(1, false), raw_bits, "fb.bits"))
            .into_int_value();

        let idx_ptr = self.entry_alloca(i64t.into(), "fb.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        let bit_ptr = self.entry_alloca(i64t.into(), "fb.bit");
        b!(self.bld.build_store(
            bit_ptr,
            b!(self
                .bld
                .build_int_nsw_sub(nbits, i64t.const_int(1, false), "fb.start"))
        ));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "fb.i")).into_int_value();
        let cond =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::SLT, idx, nbits, "fb.cond"));
        b!(self
            .bld
            .build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let bit = b!(self.bld.build_load(i64t, bit_ptr, "fb.b")).into_int_value();
        let shifted = b!(self.bld.build_right_shift(wide, bit, false, "fb.sh"));
        let masked = b!(self
            .bld
            .build_and(shifted, i64t.const_int(1, false), "fb.m"));
        let ch = b!(self.bld.build_int_nsw_add(
            b!(self.bld.build_int_truncate(masked, i8t, "fb.trunc")),
            i8t.const_int(b'0' as u64, false),
            "fb.ch"
        ));
        let dest = unsafe { b!(self.bld.build_gep(i8t, buf_ptr, &[idx], "fb.p")) };
        b!(self.bld.build_store(dest, ch));
        let next_idx = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "fb.ni"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        let next_bit = b!(self
            .bld
            .build_int_nsw_sub(bit, i64t.const_int(1, false), "fb.nb2"));
        b!(self.bld.build_store(bit_ptr, next_bit));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let end = unsafe { b!(self.bld.build_gep(i8t, buf_ptr, &[nbits], "fb.end")) };
        b!(self.bld.build_store(end, i8t.const_int(0, false)));
        self.build_string(
            buf,
            nbits,
            b!(self
                .bld
                .build_int_nsw_add(nbits, i64t.const_int(1, false), "fb.cap")),
            "fb.s",
        )
    }

    pub(super) fn emit_sleep_ms(
        &mut self,
        ms: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let nanosleep = self
            .module
            .get_function("nanosleep")
            .unwrap_or_else(|| {
                self.module.add_function(
                    "nanosleep",
                    i32t.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
                    Some(inkwell::module::Linkage::External),
                )
            });
        let ts_ty = self
            .ctx
            .struct_type(&[i64t.into(), i64t.into()], false);
        let ts = self.entry_alloca(ts_ty.into(), "sleep.ts");
        let secs =
            b!(self
                .bld
                .build_int_unsigned_div(ms, i64t.const_int(1000, false), "sleep.s"));
        let ns =
            b!(self
                .bld
                .build_int_unsigned_rem(ms, i64t.const_int(1000, false), "sleep.rem"));
        let ns_full =
            b!(self
                .bld
                .build_int_mul(ns, i64t.const_int(1_000_000, false), "sleep.ns"));
        let s_ptr = b!(self.bld.build_struct_gep(ts_ty, ts, 0, "sleep.sp"));
        b!(self.bld.build_store(s_ptr, secs));
        let n_ptr = b!(self.bld.build_struct_gep(ts_ty, ts, 1, "sleep.np"));
        b!(self.bld.build_store(n_ptr, ns_full));
        let null = ptr_ty.const_null();
        b!(self
            .bld
            .build_call(nanosleep, &[ts.into(), null.into()], ""));
        Ok(i64t.const_int(0, false).into())
    }
}
