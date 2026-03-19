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
            // Use %.*s with explicit length — no null-termination required
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
        match ty {
            Type::String => Ok(val),
            Type::I64 | Type::I32 | Type::I16 | Type::I8 => self.int_to_string(val, false),
            Type::U64 | Type::U32 | Type::U16 | Type::U8 => self.int_to_string(val, true),
            Type::F64 | Type::F32 => self.float_to_string(val),
            Type::Bool => self.bool_to_string(val),
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
        let old = b!(self.bld.build_load(self.ctx.i64_type(), rc_gep, "rc.old")).into_int_value();
        let new =
            b!(self
                .bld
                .build_int_nuw_add(old, self.ctx.i64_type().const_int(1, false), "rc.inc"));
        b!(self.bld.build_store(rc_gep, new));
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
        let old = b!(self.bld.build_load(i64t, rc_gep, "rc.old")).into_int_value();
        let new = b!(self
            .bld
            .build_int_nsw_sub(old, i64t.const_int(1, false), "rc.dec"));
        b!(self.bld.build_store(rc_gep, new));
        let is_zero = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            new,
            i64t.const_int(0, false),
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

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Weak references — cycle-breaking shared ownership
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //
    // Weak reference layout: { strong_count: i64, weak_count: i64, data: T }
    // - Downgrade: increments weak_count on the same RC allocation
    // - Upgrade: checks strong_count > 0, increments strong_count, returns rc
    // - Release: decrements weak_count, frees if both counts are zero
    //
    // The weak pointer points to the same allocation as the rc pointer.
    // This requires the rc layout to include a weak_count field:
    //   { strong: i64, weak: i64, data: T }

    pub(crate) fn weak_layout_ty(&self, inner: &Type) -> inkwell::types::StructType<'ctx> {
        let name = format!("Weak_{inner}");
        self.module.get_struct_type(&name).unwrap_or_else(|| {
            let st = self.ctx.opaque_struct_type(&name);
            st.set_body(
                &[
                    self.ctx.i64_type().into(), // strong count
                    self.ctx.i64_type().into(), // weak count
                    self.llvm_ty(inner),        // data
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
        // Increment the weak count on the rc allocation.
        // For now, weak shares the same pointer as rc but with a separate count.
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
        // Check if strong count > 0. If yes, increment strong and return pointer.
        // If strong == 0, the referent is dead — return null.
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

        // Alive: increment strong count and return pointer
        self.bld.position_at_end(alive_bb);
        let new_strong = b!(self.bld.build_int_nuw_add(
            strong,
            i64t.const_int(1, false),
            "strong.inc"
        ));
        b!(self.bld.build_store(strong_gep, new_strong));
        let alive_val: BasicValueEnum<'ctx> = heap_ptr.into();
        b!(self.bld.build_unconditional_branch(merge_bb));

        // Dead: return null pointer
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

        // Free only if both strong == 0 AND weak == 0
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

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Volatile reads/writes — hardware-observable memory operations
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

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

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Integer overflow control
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //
    // Default: nsw/nuw flags (UB on overflow — LLVM optimizes aggressively)
    // wrapping_*: two's complement wrap (no nsw/nuw flags)
    // saturating_*: clamp to type min/max via LLVM sadd.sat / uadd.sat intrinsics
    // checked_*: returns (result, overflowed) tuple via LLVM sadd.with.overflow

    fn compile_wrapping_op(
        &mut self,
        builtin: &hir::BuiltinFn,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lhs = self.compile_expr(&args[0])?.into_int_value();
        let rhs = self.compile_expr(&args[1])?.into_int_value();
        // Plain add/sub/mul without nsw/nuw flags — two's complement wrap
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
                // No direct LLVM intrinsic for saturating mul — emulate
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
        // Saturating mul: mul with overflow check, clamp if overflowed
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

        let _ = fv; // used for basic block context
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
        // Returns the LLVM struct { iN result, i1 overflowed } directly.
        // The Jade type system maps this to (iN, bool) tuple.
        Ok(result)
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Signal handling — POSIX signal(2) / raise(3) wrappers
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    fn ensure_signal(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("signal").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i32t = self.ctx.i32_type();
            // signal(int signum, void (*handler)(int)) -> void (*)(int)
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
        // SIG_IGN is typically 1 on POSIX systems
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
}
