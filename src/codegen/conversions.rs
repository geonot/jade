use inkwell::AddressSpace;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_log(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
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
            } else if matches!(ty, Type::F32) {
                let fv = val.into_float_value();
                let f64t = self.ctx.f64_type();
                b!(self.bld.build_float_ext(fv, f64t, "fpext")).into()
            } else {
                val.into()
            };
            b!(self
                .bld
                .build_call(printf, &[fs.as_pointer_value().into(), print_val], "log"));
        }
        Ok(val)
    }

    pub(crate) fn fmt_for_ty(&self, ty: &Type) -> &'static str {
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

    pub(crate) fn compile_to_string(
        &mut self,
        expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
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
}
