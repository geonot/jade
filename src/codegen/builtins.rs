use inkwell::module::Linkage;
use inkwell::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum};
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

    fn compile_log(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("log() requires an argument".into());
        }
        let val = self.compile_expr(&args[0])?;
        let ty = &args[0].ty;
        self.emit_log(val, ty)?;
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    pub(crate) fn emit_log(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let printf = self.module.get_function("printf").unwrap();
        let fmt = self.fmt_for_ty(ty);
        let fs = b!(self.bld.build_global_string_ptr(fmt, "fmt"));
        if matches!(ty, Type::String) {
            let len = self.string_len(val)?.into_int_value();
            let len_i32 = b!(self
                .bld
                .build_int_truncate(len, self.ctx.i32_type(), "slen32"));
            let data = self.string_data(val)?;
            b!(self.bld.build_call(
                printf,
                &[fs.as_pointer_value().into(), len_i32.into(), data.into()],
                "log"
            ));
        } else {
            let print_val: BasicMetadataValueEnum<'ctx> = if matches!(ty, Type::Bool) {
                let iv = val.into_int_value();
                let ext = if iv.get_type().get_bit_width() == 1 {
                    b!(self.bld.build_int_z_extend(iv, self.ctx.i32_type(), "bext"))
                } else {
                    iv
                };
                ext.into()
            } else {
                val.into()
            };
            b!(self
                .bld
                .build_call(printf, &[fs.as_pointer_value().into(), print_val], "log"));
        }
        Ok(val)
    }

    fn fmt_for_ty(&self, ty: &Type) -> &'static str {
        match ty {
            Type::I64 => "%ld\n",
            Type::I32 | Type::I16 | Type::I8 => "%d\n",
            Type::U64 => "%lu\n",
            Type::U32 | Type::U16 | Type::U8 => "%u\n",
            Type::F64 | Type::F32 => "%f\n",
            Type::Bool => "%d\n",
            Type::String => "%.*s\n",
            _ => "%ld\n",
        }
    }

    fn compile_to_string(&mut self, expr: &hir::Expr) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(expr)?;
        let ty = self.resolve_ty(expr.ty.clone());
        match &ty {
            Type::String => Ok(val),
            Type::I64 | Type::I32 | Type::I16 | Type::I8 => self.int_to_string(val, false),
            Type::U64 | Type::U32 | Type::U16 | Type::U8 => self.int_to_string(val, true),
            Type::F64 | Type::F32 => self.float_to_string(val),
            Type::Bool => self.bool_to_string(val),
            Type::Struct(name) => {
                let fn_name = format!("{name}_display");
                if let Some((fv, _, _)) = self.fns.get(&fn_name).cloned() {
                    let result = b!(self.bld.build_call(fv, &[val.into()], "display.call"))
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    Ok(result)
                } else {
                    self.int_to_string(val, false)
                }
            }
            _ => self.int_to_string(val, false),
        }
    }

    pub(crate) fn int_to_string(
        &mut self,
        val: BasicValueEnum<'ctx>,
        unsigned: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let fmt_str = if unsigned { "%lu" } else { "%ld" };
        let fmt = b!(self.bld.build_global_string_ptr(fmt_str, "ts.fmt"));
        let snprintf = self.ensure_snprintf();
        let iv = val.into_int_value();
        let wide: BasicValueEnum<'ctx> = if iv.get_type().get_bit_width() < 64 {
            if unsigned {
                b!(self.bld.build_int_z_extend(iv, i64t, "zext")).into()
            } else {
                b!(self.bld.build_int_s_extend(iv, i64t, "sext")).into()
            }
        } else {
            iv.into()
        };
        let null = ptr_ty.const_null();
        let len = b!(self.bld.build_call(
            snprintf,
            &[
                null.into(),
                i64t.const_int(0, false).into(),
                fmt.as_pointer_value().into(),
                wide.into()
            ],
            "ts.len"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        let len = b!(self.bld.build_int_s_extend(len, i64t, "ts.len64"));
        let size = b!(self
            .bld
            .build_int_nsw_add(len, i64t.const_int(1, false), "ts.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "ts.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        b!(self.bld.build_call(
            snprintf,
            &[
                buf.into(),
                size.into(),
                fmt.as_pointer_value().into(),
                wide.into()
            ],
            ""
        ));
        self.build_string(buf, len, size, "ts.val")
    }

    pub(crate) fn float_to_string(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let fmt = b!(self.bld.build_global_string_ptr("%g", "ts.ffmt"));
        let snprintf = self.ensure_snprintf();
        let fv = val.into_float_value();
        let f64t = self.ctx.f64_type();
        let wide: BasicMetadataValueEnum<'ctx> = if fv.get_type() == self.ctx.f32_type() {
            b!(self.bld.build_float_ext(fv, f64t, "fpext")).into()
        } else {
            fv.into()
        };
        let null = ptr_ty.const_null();
        let len = b!(self.bld.build_call(
            snprintf,
            &[
                null.into(),
                i64t.const_int(0, false).into(),
                fmt.as_pointer_value().into(),
                wide
            ],
            "ts.len"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        let len = b!(self.bld.build_int_s_extend(len, i64t, "ts.len64"));
        let size = b!(self
            .bld
            .build_int_nsw_add(len, i64t.const_int(1, false), "ts.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "ts.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        b!(self.bld.build_call(
            snprintf,
            &[buf.into(), size.into(), fmt.as_pointer_value().into(), wide],
            ""
        ));
        self.build_string(buf, len, size, "ts.val")
    }

    pub(crate) fn bool_to_string(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let true_str = b!(self.bld.build_global_string_ptr("true", "ts.true"));
        let false_str = b!(self.bld.build_global_string_ptr("false", "ts.false"));
        let cond = self.to_bool(val);
        let true_bb = self.ctx.append_basic_block(fv, "ts.t");
        let false_bb = self.ctx.append_basic_block(fv, "ts.f");
        let merge_bb = self.ctx.append_basic_block(fv, "ts.m");
        b!(self.bld.build_conditional_branch(cond, true_bb, false_bb));
        let i64t = self.ctx.i64_type();
        let zero = i64t.const_int(0, false);
        self.bld.position_at_end(true_bb);
        let tv = self.build_string(
            true_str.as_pointer_value(),
            i64t.const_int(4, false),
            zero,
            "ts.true",
        )?;
        b!(self.bld.build_unconditional_branch(merge_bb));
        self.bld.position_at_end(false_bb);
        let fv_val = self.build_string(
            false_str.as_pointer_value(),
            i64t.const_int(5, false),
            zero,
            "ts.false",
        )?;
        b!(self.bld.build_unconditional_branch(merge_bb));
        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(self.string_type(), "ts.res"));
        phi.add_incoming(&[(&tv, true_bb), (&fv_val, false_bb)]);
        Ok(phi.as_basic_value())
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

    pub(crate) fn rc_layout_ty(&self, inner: &Type) -> inkwell::types::StructType<'ctx> {
        let name = format!("Rc_{inner}");
        self.module.get_struct_type(&name).unwrap_or_else(|| {
            let st = self.ctx.opaque_struct_type(&name);
            st.set_body(&[self.ctx.i64_type().into(), self.llvm_ty(inner)], false);
            st
        })
    }

    pub(crate) fn rc_alloc(
        &mut self,
        inner: &Type,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let layout = self.rc_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let size = layout.size_of().unwrap();
        let malloc = self.ensure_malloc();
        let heap_ptr = b!(self.bld.build_call(malloc, &[size.into()], "rc.alloc"))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        let rc_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "rc.cnt"));
        b!(self.bld.build_store(rc_gep, i64t.const_int(1, false)));
        let val_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "rc.val"));
        b!(self.bld.build_store(val_gep, val));
        Ok(heap_ptr.into())
    }

    pub(crate) fn rc_retain(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        let layout = self.rc_layout_ty(inner);
        let rc_gep = b!(self
            .bld
            .build_struct_gep(layout, ptr.into_pointer_value(), 0, "rc.cnt"));
        // Atomic increment — safe across actor threads
        b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Add,
            rc_gep,
            self.ctx.i64_type().const_int(1, false),
            inkwell::AtomicOrdering::AcquireRelease,
        ));
        Ok(())
    }

    pub(crate) fn rc_release(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let layout = self.rc_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let heap_ptr = ptr.into_pointer_value();
        let rc_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "rc.cnt"));
        // Atomic decrement — returns the old value
        let old = b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Sub,
            rc_gep,
            i64t.const_int(1, false),
            inkwell::AtomicOrdering::AcquireRelease,
        ));
        // old == 1 means new count is 0 → free
        let is_zero = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            old,
            i64t.const_int(1, false),
            "rc.dead"
        ));
        let free_bb = self.ctx.append_basic_block(fv, "rc.free");
        let cont_bb = self.ctx.append_basic_block(fv, "rc.cont");
        b!(self.bld.build_conditional_branch(is_zero, free_bb, cont_bb));
        self.bld.position_at_end(free_bb);
        let free_fn = self.module.get_function("free").unwrap_or_else(|| {
            self.module.add_function(
                "free",
                self.ctx.void_type().fn_type(&[ptr_ty.into()], false),
                Some(Linkage::External),
            )
        });
        b!(self.bld.build_call(free_fn, &[heap_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(cont_bb));
        self.bld.position_at_end(cont_bb);
        Ok(())
    }

    pub(crate) fn rc_deref(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let layout = self.rc_layout_ty(inner);
        let val_gep = b!(self
            .bld
            .build_struct_gep(layout, ptr.into_pointer_value(), 1, "rc.val"));
        Ok(b!(self.bld.build_load(
            self.llvm_ty(inner),
            val_gep,
            "rc.load"
        )))
    }

    pub(crate) fn weak_layout_ty(&self, inner: &Type) -> inkwell::types::StructType<'ctx> {
        let name = format!("Weak_{inner}");
        self.module.get_struct_type(&name).unwrap_or_else(|| {
            let st = self.ctx.opaque_struct_type(&name);
            st.set_body(
                &[
                    self.ctx.i64_type().into(),
                    self.ctx.i64_type().into(),
                    self.llvm_ty(inner),
                ],
                false,
            );
            st
        })
    }

    pub(crate) fn weak_downgrade(
        &mut self,
        rc_ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let layout = self.weak_layout_ty(inner);
        let heap_ptr = rc_ptr.into_pointer_value();
        let weak_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "weak.cnt"));
        let old = b!(self.bld.build_load(self.ctx.i64_type(), weak_gep, "weak.old")).into_int_value();
        let new = b!(self.bld.build_int_nuw_add(
            old,
            self.ctx.i64_type().const_int(1, false),
            "weak.inc"
        ));
        b!(self.bld.build_store(weak_gep, new));
        Ok(heap_ptr.into())
    }

    pub(crate) fn weak_upgrade(
        &mut self,
        weak_ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let layout = self.weak_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let heap_ptr = weak_ptr.into_pointer_value();
        let strong_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "strong.cnt"));
        let strong = b!(self.bld.build_load(i64t, strong_gep, "strong")).into_int_value();
        let is_alive = b!(self.bld.build_int_compare(
            IntPredicate::SGT,
            strong,
            i64t.const_int(0, false),
            "alive"
        ));
        let alive_bb = self.ctx.append_basic_block(fv, "weak.alive");
        let dead_bb = self.ctx.append_basic_block(fv, "weak.dead");
        let merge_bb = self.ctx.append_basic_block(fv, "weak.merge");
        b!(self.bld.build_conditional_branch(is_alive, alive_bb, dead_bb));

        self.bld.position_at_end(alive_bb);
        let new_strong = b!(self.bld.build_int_nuw_add(
            strong,
            i64t.const_int(1, false),
            "strong.inc"
        ));
        b!(self.bld.build_store(strong_gep, new_strong));
        let alive_val: BasicValueEnum<'ctx> = heap_ptr.into();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(dead_bb);
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let null_val: BasicValueEnum<'ctx> = ptr_ty.const_null().into();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(ptr_ty, "weak.result"));
        phi.add_incoming(&[(&alive_val, alive_bb), (&null_val, dead_bb)]);
        Ok(phi.as_basic_value())
    }

    pub(crate) fn weak_release(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let layout = self.weak_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let heap_ptr = ptr.into_pointer_value();
        let weak_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "weak.cnt"));
        let old = b!(self.bld.build_load(i64t, weak_gep, "weak.old")).into_int_value();
        let new = b!(self.bld.build_int_nsw_sub(old, i64t.const_int(1, false), "weak.dec"));
        b!(self.bld.build_store(weak_gep, new));

        let strong_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "strong.cnt"));
        let strong = b!(self.bld.build_load(i64t, strong_gep, "strong")).into_int_value();
        let strong_zero = b!(self.bld.build_int_compare(
            IntPredicate::EQ, strong, i64t.const_int(0, false), "s.zero"
        ));
        let weak_zero = b!(self.bld.build_int_compare(
            IntPredicate::EQ, new, i64t.const_int(0, false), "w.zero"
        ));
        let both_zero = b!(self.bld.build_and(strong_zero, weak_zero, "both.zero"));

        let free_bb = self.ctx.append_basic_block(fv, "weak.free");
        let cont_bb = self.ctx.append_basic_block(fv, "weak.cont");
        b!(self.bld.build_conditional_branch(both_zero, free_bb, cont_bb));

        self.bld.position_at_end(free_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[heap_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        Ok(())
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
            hir::BuiltinFn::SaturatingAdd if signed => (format!("llvm.sadd.sat.i{bw}"), lhs.get_type()),
            hir::BuiltinFn::SaturatingAdd => (format!("llvm.uadd.sat.i{bw}"), lhs.get_type()),
            hir::BuiltinFn::SaturatingSub if signed => (format!("llvm.ssub.sat.i{bw}"), lhs.get_type()),
            hir::BuiltinFn::SaturatingSub => (format!("llvm.usub.sat.i{bw}"), lhs.get_type()),
            hir::BuiltinFn::SaturatingMul => {
                return self.compile_saturating_mul(lhs, rhs, signed);
            }
            _ => unreachable!(),
        };
        let ft = it.fn_type(&[it.into(), it.into()], false);
        let f = self.module.get_function(&intrinsic_name)
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
        let overflow_ty = self.ctx.struct_type(&[it.into(), self.ctx.bool_type().into()], false);
        let ft = overflow_ty.fn_type(&[it.into(), it.into()], false);
        let f = self.module.get_function(&intrinsic)
            .unwrap_or_else(|| self.module.add_function(&intrinsic, ft, None));
        let result = b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "smul"))
            .try_as_basic_value().basic().unwrap();
        let val = b!(self.bld.build_extract_value(result.into_struct_value(), 0, "mul.val")).into_int_value();
        let overflowed = b!(self.bld.build_extract_value(result.into_struct_value(), 1, "mul.of")).into_int_value();

        let max_val = if signed {
            it.const_int((1u64 << (bw - 1)) - 1, false)
        } else {
            it.const_all_ones()
        };
        let clamped: BasicValueEnum = b!(self.bld.build_select::<BasicValueEnum, _>(overflowed, max_val.into(), val.into(), "sat.mul"));

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
        let overflow_ty = self.ctx.struct_type(&[it.into(), self.ctx.bool_type().into()], false);
        let ft = overflow_ty.fn_type(&[it.into(), it.into()], false);
        let f = self.module.get_function(&intrinsic)
            .unwrap_or_else(|| self.module.add_function(&intrinsic, ft, None));
        let result = b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "chk"))
            .try_as_basic_value().basic().unwrap();
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
        b!(self.bld.build_call(
            signal_fn,
            &[signum.into(), handler.into()],
            "sig"
        ));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    fn compile_signal_raise(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
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
        b!(self.bld.build_call(
            signal_fn,
            &[signum.into(), sig_ign.into()],
            "sig.ign"
        ));
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
        b!(self.bld.build_call(printf, &[gs.as_pointer_value().into()], ""));
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

    // ── LLVM f64 intrinsics (1-arg, 2-arg, 3-arg) ──────────────────

    fn compile_f64_intrinsic(
        &mut self,
        name: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let f64t = self.ctx.f64_type();
        let f = self.module.get_function(name).unwrap_or_else(|| {
            self.module.add_function(name, f64t.fn_type(&[f64t.into()], false), None)
        });
        let v = self.compile_expr(&args[0])?.into_float_value();
        Ok(b!(self.bld.build_call(f, &[v.into()], "")).try_as_basic_value().basic().unwrap())
    }

    fn compile_f64_intrinsic2(
        &mut self,
        name: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let f64t = self.ctx.f64_type();
        let f = self.module.get_function(name).unwrap_or_else(|| {
            self.module.add_function(name, f64t.fn_type(&[f64t.into(), f64t.into()], false), None)
        });
        let a = self.compile_expr(&args[0])?.into_float_value();
        let b_val = self.compile_expr(&args[1])?.into_float_value();
        Ok(b!(self.bld.build_call(f, &[a.into(), b_val.into()], "")).try_as_basic_value().basic().unwrap())
    }

    fn compile_f64_intrinsic3(
        &mut self,
        name: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let f64t = self.ctx.f64_type();
        let f = self.module.get_function(name).unwrap_or_else(|| {
            self.module.add_function(name, f64t.fn_type(&[f64t.into(), f64t.into(), f64t.into()], false), None)
        });
        let a = self.compile_expr(&args[0])?.into_float_value();
        let b_val = self.compile_expr(&args[1])?.into_float_value();
        let c = self.compile_expr(&args[2])?.into_float_value();
        Ok(b!(self.bld.build_call(f, &[a.into(), b_val.into(), c.into()], "")).try_as_basic_value().basic().unwrap())
    }

    // ── String from raw/ptr ─────────────────────────────────────────

    fn compile_string_from_raw(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // __string_from_raw(ptr, len, cap) — takes ownership of ptr
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
        // __string_from_ptr(ptr) — copies strlen(ptr) bytes into new String
        let ptr = self.compile_expr(&args[0])?;
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let strlen = self.module.get_function("strlen").unwrap_or_else(|| {
            self.module.add_function("strlen", i64t.fn_type(&[ptr_ty.into()], false), Some(Linkage::External))
        });
        let len = b!(self.bld.build_call(strlen, &[ptr.into()], "sfp.len"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let size = b!(self.bld.build_int_nsw_add(len, i64t.const_int(1, false), "sfp.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "sfp.buf"))
            .try_as_basic_value().basic().unwrap();
        let memcpy = self.ensure_memcpy();
        b!(self.bld.build_call(memcpy, &[buf.into(), ptr.into(), size.into()], ""));
        self.build_string(buf, len, size, "sfp")
    }

    // ── GetArgs ─────────────────────────────────────────────────────

    fn compile_get_args(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let argc_g = self.module.get_global("__jade_argc")
            .ok_or("__jade_argc global not found")?;
        let argv_g = self.module.get_global("__jade_argv")
            .ok_or("__jade_argv global not found")?;
        let argc = b!(self.bld.build_load(i32t, argc_g.as_pointer_value(), "argc")).into_int_value();
        let argc64 = b!(self.bld.build_int_s_extend(argc, i64t, "argc64"));
        let argv = b!(self.bld.build_load(ptr_ty, argv_g.as_pointer_value(), "argv")).into_pointer_value();

        // Create empty Vec<String> using existing infrastructure
        let header_ptr = self.compile_vec_new(&[])?.into_pointer_value();
        let header_ty = self.vec_header_type();
        let st = self.string_type();
        let str_size: u64 = 24; // {ptr, len, cap}

        // Loop: for i in 0..argc
        let fv = self.cur_fn.unwrap();
        let loop_bb = self.ctx.append_basic_block(fv, "args.loop");
        let body_bb = self.ctx.append_basic_block(fv, "args.body");
        let done_bb = self.ctx.append_basic_block(fv, "args.done");
        let i_ptr = self.entry_alloca(i64t.into(), "args.i");
        b!(self.bld.build_store(i_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let i = b!(self.bld.build_load(i64t, i_ptr, "i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, i, argc64, "args.cond"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let arg_pp = unsafe { b!(self.bld.build_gep(ptr_ty, argv, &[i], "arg.pp")) };
        let arg_p = b!(self.bld.build_load(ptr_ty, arg_pp, "arg.p")).into_pointer_value();
        // Convert C string to Jade String via strlen + memcpy
        let strlen = self.module.get_function("strlen").unwrap_or_else(|| {
            self.module.add_function("strlen", i64t.fn_type(&[ptr_ty.into()], false), Some(Linkage::External))
        });
        let slen = b!(self.bld.build_call(strlen, &[arg_p.into()], "arg.len"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let size = b!(self.bld.build_int_nsw_add(slen, i64t.const_int(1, false), "arg.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "arg.buf"))
            .try_as_basic_value().basic().unwrap();
        let memcpy = self.ensure_memcpy();
        b!(self.bld.build_call(memcpy, &[buf.into(), arg_p.into(), size.into()], ""));
        let s = self.build_string(buf, slen, size, "arg.s")?;

        // Inline push: load len/cap, grow if needed, store
        let len_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 1, "ga.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "ga.len")).into_int_value();
        let cap_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 2, "ga.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "ga.cap")).into_int_value();
        let needs_grow = b!(self.bld.build_int_compare(IntPredicate::SGE, len, cap, "ga.full"));
        let grow_bb = self.ctx.append_basic_block(fv, "ga.grow");
        let store_bb = self.ctx.append_basic_block(fv, "ga.store");
        b!(self.bld.build_conditional_branch(needs_grow, grow_bb, store_bb));

        self.bld.position_at_end(grow_bb);
        let doubled = b!(self.bld.build_int_nsw_mul(cap, i64t.const_int(2, false), "ga.dbl"));
        let new_cap_cmp = b!(self.bld.build_int_compare(IntPredicate::SGT, doubled, i64t.const_int(4, false), "ga.cmp"));
        let new_cap = b!(self.bld.build_select(new_cap_cmp, doubled, i64t.const_int(4, false), "ga.nc")).into_int_value();
        let new_size = b!(self.bld.build_int_nsw_mul(new_cap, i64t.const_int(str_size, false), "ga.ns"));
        let realloc = self.ensure_realloc();
        let data_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "ga.datap"));
        let old_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "ga.optr"));
        let new_ptr = b!(self.bld.build_call(realloc, &[old_ptr.into(), new_size.into()], "ga.nptr"))
            .try_as_basic_value().basic().unwrap();
        b!(self.bld.build_store(data_gep, new_ptr));
        b!(self.bld.build_store(cap_gep, new_cap));
        b!(self.bld.build_unconditional_branch(store_bb));

        self.bld.position_at_end(store_bb);
        let data_gep2 = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "ga.dp2"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep2, "ga.dp")).into_pointer_value();
        let elem_gep = unsafe { b!(self.bld.build_gep(st, data_ptr, &[len], "ga.ep")) };
        b!(self.bld.build_store(elem_gep, s));
        let new_len = b!(self.bld.build_int_nsw_add(len, i64t.const_int(1, false), "ga.nl"));
        b!(self.bld.build_store(len_gep, new_len));

        let next = b!(self.bld.build_int_nsw_add(i, i64t.const_int(1, false), "args.next"));
        b!(self.bld.build_store(i_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(header_ptr.into())
    }

    // ── Formatting builtins ─────────────────────────────────────────

    fn compile_fmt_float(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // __fmt_float(x: f64, decimals: i64) -> String
        let x = self.compile_expr(&args[0])?.into_float_value();
        let decimals = self.compile_expr(&args[1])?.into_int_value();
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let snprintf = self.ensure_snprintf();

        // Build format string "%.*f" — precision from arg
        let fmt = b!(self.bld.build_global_string_ptr("%.*f", "ff.fmt"));
        // snprintf(NULL, 0, "%.*f", decimals, x) → len
        let null = ptr_ty.const_null();
        let dec_i32 = b!(self.bld.build_int_truncate(decimals, self.ctx.i32_type(), "dec32"));
        let len = b!(self.bld.build_call(snprintf, &[null.into(), i64t.const_int(0, false).into(), fmt.as_pointer_value().into(), dec_i32.into(), x.into()], "ff.len"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let len64 = b!(self.bld.build_int_s_extend(len, i64t, "ff.len64"));
        let size = b!(self.bld.build_int_nsw_add(len64, i64t.const_int(1, false), "ff.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "ff.buf"))
            .try_as_basic_value().basic().unwrap();
        b!(self.bld.build_call(snprintf, &[buf.into(), size.into(), fmt.as_pointer_value().into(), dec_i32.into(), x.into()], ""));
        self.build_string(buf, len64, size, "ff.s")
    }

    fn compile_fmt_snprintf(
        &mut self,
        fmt_str: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(&args[0])?.into_int_value();
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let snprintf = self.ensure_snprintf();
        let fmt = b!(self.bld.build_global_string_ptr(fmt_str, "fh.fmt"));
        let null = ptr_ty.const_null();
        let wide = if val.get_type().get_bit_width() < 64 {
            b!(self.bld.build_int_s_extend(val, i64t, "fw")).into()
        } else {
            val.into()
        };
        let len = b!(self.bld.build_call(snprintf, &[null.into(), i64t.const_int(0, false).into(), fmt.as_pointer_value().into(), wide], "fh.len"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let len64 = b!(self.bld.build_int_s_extend(len, i64t, "fh.len64"));
        let size = b!(self.bld.build_int_nsw_add(len64, i64t.const_int(1, false), "fh.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "fh.buf"))
            .try_as_basic_value().basic().unwrap();
        b!(self.bld.build_call(snprintf, &[buf.into(), size.into(), fmt.as_pointer_value().into(), wide], ""));
        self.build_string(buf, len64, size, "fh.s")
    }

    fn compile_fmt_bin(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Convert integer to binary string via bit extraction loop
        let val = self.compile_expr(&args[0])?.into_int_value();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let malloc = self.ensure_malloc();
        // Max 64 binary digits + null
        let buf = b!(self.bld.build_call(malloc, &[i64t.const_int(65, false).into()], "fb.buf"))
            .try_as_basic_value().basic().unwrap();
        let buf_ptr = buf.into_pointer_value();

        let fv = self.cur_fn.unwrap();
        let loop_bb = self.ctx.append_basic_block(fv, "fb.loop");
        let body_bb = self.ctx.append_basic_block(fv, "fb.body");
        let done_bb = self.ctx.append_basic_block(fv, "fb.done");

        let wide = if val.get_type().get_bit_width() < 64 {
            b!(self.bld.build_int_z_extend(val, i64t, "fb.w"))
        } else { val };

        // Find MSB position using CLZ
        let clz_name = "llvm.ctlz.i64";
        let clz = self.module.get_function(clz_name).unwrap_or_else(|| {
            let ft = i64t.fn_type(&[i64t.into(), self.ctx.bool_type().into()], false);
            self.module.add_function(clz_name, ft, None)
        });
        let lz = b!(self.bld.build_call(clz, &[wide.into(), self.ctx.bool_type().const_int(1, false).into()], "fb.lz"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        // nbits = max(64 - lz, 1) — at least "0"
        let raw_bits = b!(self.bld.build_int_nsw_sub(i64t.const_int(64, false), lz, "fb.nb"));
        let is_zero = b!(self.bld.build_int_compare(IntPredicate::EQ, wide, i64t.const_int(0, false), "fb.z"));
        let nbits = b!(self.bld.build_select(is_zero, i64t.const_int(1, false), raw_bits, "fb.bits")).into_int_value();

        let idx_ptr = self.entry_alloca(i64t.into(), "fb.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        let bit_ptr = self.entry_alloca(i64t.into(), "fb.bit");
        b!(self.bld.build_store(bit_ptr, b!(self.bld.build_int_nsw_sub(nbits, i64t.const_int(1, false), "fb.start"))));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "fb.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, nbits, "fb.cond"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let bit = b!(self.bld.build_load(i64t, bit_ptr, "fb.b")).into_int_value();
        let shifted = b!(self.bld.build_right_shift(wide, bit, false, "fb.sh"));
        let masked = b!(self.bld.build_and(shifted, i64t.const_int(1, false), "fb.m"));
        let ch = b!(self.bld.build_int_nsw_add(b!(self.bld.build_int_truncate(masked, i8t, "fb.trunc")), i8t.const_int(b'0' as u64, false), "fb.ch"));
        let dest = unsafe { b!(self.bld.build_gep(i8t, buf_ptr, &[idx], "fb.p")) };
        b!(self.bld.build_store(dest, ch));
        let next_idx = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "fb.ni"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        let next_bit = b!(self.bld.build_int_nsw_sub(bit, i64t.const_int(1, false), "fb.nb"));
        b!(self.bld.build_store(bit_ptr, next_bit));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        // Null-terminate
        let end = unsafe { b!(self.bld.build_gep(i8t, buf_ptr, &[nbits], "fb.end")) };
        b!(self.bld.build_store(end, i8t.const_int(0, false)));
        self.build_string(buf, nbits, b!(self.bld.build_int_nsw_add(nbits, i64t.const_int(1, false), "fb.cap")), "fb.s")
    }

    // ── Time builtins ───────────────────────────────────────────────

    fn compile_time_monotonic(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let f64t = self.ctx.f64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let clock_gettime = self.module.get_function("clock_gettime").unwrap_or_else(|| {
            self.module.add_function("clock_gettime", i32t.fn_type(&[i32t.into(), ptr_ty.into()], false), Some(Linkage::External))
        });

        // struct timespec { i64 tv_sec, i64 tv_nsec }
        let ts_ty = self.ctx.struct_type(&[i64t.into(), i64t.into()], false);
        let ts = self.entry_alloca(ts_ty.into(), "ts");
        // CLOCK_MONOTONIC = 1
        b!(self.bld.build_call(clock_gettime, &[i32t.const_int(1, false).into(), ts.into()], ""));
        let sec = b!(self.bld.build_load(i64t, b!(self.bld.build_struct_gep(ts_ty, ts, 0, "ts.sec")), "sec")).into_int_value();
        let nsec = b!(self.bld.build_load(i64t, b!(self.bld.build_struct_gep(ts_ty, ts, 1, "ts.nsec")), "nsec")).into_int_value();
        let sec_f = b!(self.bld.build_signed_int_to_float(sec, f64t, "secf"));
        let nsec_f = b!(self.bld.build_signed_int_to_float(nsec, f64t, "nsecf"));
        let billion = f64t.const_float(1_000_000_000.0);
        let ns_part = b!(self.bld.build_float_div(nsec_f, billion, "ns"));
        Ok(b!(self.bld.build_float_add(sec_f, ns_part, "mono")).into())
    }

    fn compile_sleep_ms(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ms = self.compile_expr(&args[0])?.into_int_value();
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        // nanosleep(&timespec, NULL)
        let nanosleep = self.module.get_function("nanosleep").unwrap_or_else(|| {
            self.module.add_function("nanosleep", i32t.fn_type(&[ptr_ty.into(), ptr_ty.into()], false), Some(Linkage::External))
        });
        let ts_ty = self.ctx.struct_type(&[i64t.into(), i64t.into()], false);
        let ts = self.entry_alloca(ts_ty.into(), "sl.ts");
        // sec = ms / 1000, nsec = (ms % 1000) * 1_000_000
        let sec = b!(self.bld.build_int_signed_div(ms, i64t.const_int(1000, false), "sl.sec"));
        let rem = b!(self.bld.build_int_signed_rem(ms, i64t.const_int(1000, false), "sl.rem"));
        let nsec = b!(self.bld.build_int_nsw_mul(rem, i64t.const_int(1_000_000, false), "sl.nsec"));
        let sec_p = b!(self.bld.build_struct_gep(ts_ty, ts, 0, "sl.secp"));
        b!(self.bld.build_store(sec_p, sec));
        let nsec_p = b!(self.bld.build_struct_gep(ts_ty, ts, 1, "sl.nsecp"));
        b!(self.bld.build_store(nsec_p, nsec));
        b!(self.bld.build_call(nanosleep, &[ts.into(), ptr_ty.const_null().into()], ""));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    fn compile_file_exists(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let s = self.compile_expr(&args[0])?;
        let data = self.string_data(s)?;
        let i32t = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let access = self.module.get_function("access").unwrap_or_else(|| {
            self.module.add_function("access", i32t.fn_type(&[ptr_ty.into(), i32t.into()], false), Some(Linkage::External))
        });
        // F_OK = 0
        let result = b!(self.bld.build_call(access, &[data.into(), i32t.const_int(0, false).into()], "fex"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let is_ok = b!(self.bld.build_int_compare(IntPredicate::EQ, result, i32t.const_int(0, false), "fex.ok"));
        Ok(is_ok.into())
    }
}
