use inkwell::module::Linkage;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum};
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
        let print_val: BasicMetadataValueEnum<'ctx> = if matches!(ty, Type::String) {
            self.string_data(val)?.into()
        } else if matches!(ty, Type::Bool) {
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
            Type::String => "%s\n",
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
}
