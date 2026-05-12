//! Codegen for primitive type conversions (int/float/bool/string).

use inkwell::IntPredicate;
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

    pub(crate) fn compile_print(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("print() requires an argument".into());
        }
        let val = self.compile_expr(&args[0])?;
        let ty = &args[0].ty;
        self.emit_print(val, ty)?;
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    pub(crate) fn emit_log(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Type::Vec(elem_ty) = ty {
            self.emit_vec_format(val, elem_ty, true)?;
            return Ok(val);
        }
        // Struct types: dispatch to user-defined `<Name>_log` method if present,
        // otherwise emit the default `Name @ 0xADDR { field: value, ... }`
        // formatter. This is the "default log" semantics: every type prints
        // something useful out of the box; users may override by writing their
        // own `log` method on the type.
        if let Type::Struct(name, _) = ty {
            let log_method_name = format!("{name}_log");
            if let Some((fv, _, _)) = self.fns.get(&log_method_name).cloned() {
                let first_param_is_ptr = fv
                    .get_type()
                    .get_param_types()
                    .first()
                    .map(|t| t.is_pointer_type())
                    .unwrap_or(false);
                let arg: BasicValueEnum<'ctx> = if first_param_is_ptr && !val.is_pointer_value() {
                    let lty = self.llvm_ty(ty);
                    let tmp = self.entry_alloca(lty, "log.user.self");
                    b!(self.bld.build_store(tmp, val));
                    tmp.into()
                } else {
                    val
                };
                b!(self.bld.build_call(fv, &[arg.into()], "log.user"));
                return Ok(val);
            }
            let name_str = name.as_str();
            self.emit_struct_format(&name_str, val, ty, true)?;
            return Ok(val);
        }
        let printf = crate::codegen::fn_or_die(&self.module, "printf");
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

    pub(crate) fn emit_print(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Type::Vec(elem_ty) = ty {
            self.emit_vec_format(val, elem_ty, false)?;
            return Ok(val);
        }
        // Structs: nest the same default formatter (no trailing newline).
        // We do NOT dispatch to a user `<Name>_log` method here because
        // print is the lower-level building block and may be invoked from
        // contexts (e.g. vec element printing) where the user's log format
        // would inject newlines or be otherwise undesirable. The user
        // override applies at the top-level log() call.
        if let Type::Struct(name, _) = ty {
            let name_str = name.as_str();
            self.emit_struct_format(&name_str, val, ty, false)?;
            return Ok(val);
        }
        let printf = crate::codegen::fn_or_die(&self.module, "printf");
        let fmt = self.fmt_for_ty_no_newline(ty);
        let fs = b!(self.bld.build_global_string_ptr(fmt, "fmt.print"));
        if matches!(ty, Type::String) {
            let len = self.string_len(val)?.into_int_value();
            let len_i32 = b!(self
                .bld
                .build_int_truncate(len, self.ctx.i32_type(), "slen32"));
            let data = self.string_data(val)?;
            b!(self.bld.build_call(
                printf,
                &[fs.as_pointer_value().into(), len_i32.into(), data.into()],
                "print"
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
                .build_call(printf, &[fs.as_pointer_value().into(), print_val], "print"));
        }
        Ok(val)
    }

    /// Print a vector value as `[a, b, c]` (optionally trailing newline).
    /// Recurses through `emit_print` for elements so primitives, strings,
    /// and nested collections all format sensibly.
    pub(crate) fn emit_vec_format(
        &mut self,
        header_val: BasicValueEnum<'ctx>,
        elem_ty: &Type,
        newline: bool,
    ) -> Result<(), String> {
        let printf = crate::codegen::fn_or_die(&self.module, "printf");
        let header_ptr = header_val.into_pointer_value();
        let i64t = self.ctx.i64_type();

        // print "["
        let open_fmt = b!(self.bld.build_global_string_ptr("[", "vfmt.open"));
        b!(self
            .bld
            .build_call(printf, &[open_fmt.as_pointer_value().into()], "vfmt.opc"));

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let lty = self.llvm_ty(elem_ty);
        let fv = self.current_fn();

        let idx_ptr = self.entry_alloca(i64t.into(), "vfmt.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_zero()));

        let loop_bb = self.ctx.append_basic_block(fv, "vfmt.loop");
        let body_bb = self.ctx.append_basic_block(fv, "vfmt.body");
        let sep_bb = self.ctx.append_basic_block(fv, "vfmt.sep");
        let print_bb = self.ctx.append_basic_block(fv, "vfmt.print");
        let done_bb = self.ctx.append_basic_block(fv, "vfmt.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "vfmt.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, len, "vfmt.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let is_pos = b!(self.bld.build_int_compare(
            IntPredicate::SGT,
            idx,
            i64t.const_zero(),
            "vfmt.pos"
        ));
        b!(self
            .bld
            .build_conditional_branch(is_pos, sep_bb, print_bb));

        self.bld.position_at_end(sep_bb);
        let sep_fmt = b!(self.bld.build_global_string_ptr(", ", "vfmt.sep"));
        b!(self
            .bld
            .build_call(printf, &[sep_fmt.as_pointer_value().into()], "vfmt.sepc"));
        b!(self.bld.build_unconditional_branch(print_bb));

        self.bld.position_at_end(print_bb);
        let elem_gep = unsafe {
            b!(self
                .bld
                .build_gep(lty, data_ptr, &[idx], "vfmt.gep"))
        };
        let elem = b!(self.bld.build_load(lty, elem_gep, "vfmt.elem"));
        self.emit_print(elem, elem_ty)?;
        let next = b!(self.bld.build_int_nsw_add(
            idx,
            i64t.const_int(1, false),
            "vfmt.next"
        ));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let close_str = if newline { "]\n" } else { "]" };
        let close_fmt = b!(self.bld.build_global_string_ptr(close_str, "vfmt.close"));
        b!(self
            .bld
            .build_call(printf, &[close_fmt.as_pointer_value().into()], "vfmt.clc"));
        Ok(())
    }

    /// Default struct formatter: `Name @ 0xADDR { f1: v1, f2: v2 }` with an
    /// optional trailing newline. Recurses through `emit_print` for fields,
    /// so nested structs print inline (without their own newline) and
    /// primitives use their normal formatting.
    ///
    /// Address semantics: if `val` is already a pointer (typical for methods
    /// invoked via `obj.log()` where `self` is by-ptr) we use it directly;
    /// otherwise we spill the value to a stack alloca and print that
    /// address. For value-typed receivers this address is the temporary's
    /// stack slot, which is the only well-defined "address" such a value
    /// has at this program point.
    pub(crate) fn emit_struct_format(
        &mut self,
        name: &str,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
        newline: bool,
    ) -> Result<(), String> {
        let printf = crate::codegen::fn_or_die(&self.module, "printf");
        let fields = self
            .structs
            .get(&crate::intern::Symbol::intern(name))
            .cloned()
            .unwrap_or_default();

        // Compute address and the loadable struct value.
        let lty = self.llvm_ty(ty);
        let (addr, struct_val): (BasicValueEnum<'ctx>, BasicValueEnum<'ctx>) =
            if val.is_pointer_value() {
                let loaded = b!(self.bld.build_load(lty, val.into_pointer_value(), "log.load"));
                (val, loaded)
            } else {
                let tmp = self.entry_alloca(lty, "log.tmp");
                b!(self.bld.build_store(tmp, val));
                (tmp.into(), val)
            };

        // Header: "Name @ %p { " (or "Name @ %p {}\n" if no fields)
        if fields.is_empty() {
            let hdr = if newline {
                format!("{name} @ %p {{}}\n\0")
            } else {
                format!("{name} @ %p {{}}\0")
            };
            let hf = b!(self.bld.build_global_string_ptr(&hdr, "log.hdr"));
            b!(self.bld.build_call(
                printf,
                &[hf.as_pointer_value().into(), addr.into()],
                "log.hdrc"
            ));
            return Ok(());
        }
        let hdr = format!("{name} @ %p {{ \0");
        let hf = b!(self.bld.build_global_string_ptr(&hdr, "log.hdr"));
        b!(self.bld.build_call(
            printf,
            &[hf.as_pointer_value().into(), addr.into()],
            "log.hdrc"
        ));

        let sv = struct_val.into_struct_value();
        for (i, (fname, fty)) in fields.iter().enumerate() {
            let label = if i == 0 {
                format!("{fname}: \0")
            } else {
                format!(", {fname}: \0")
            };
            let lf = b!(self.bld.build_global_string_ptr(&label, "log.lbl"));
            b!(self.bld.build_call(
                printf,
                &[lf.as_pointer_value().into()],
                "log.lblc"
            ));
            let field_val = b!(self
                .bld
                .build_extract_value(sv, i as u32, "log.fld"));
            self.emit_print(field_val, fty)?;
        }
        let close_str = if newline { " }\n\0" } else { " }\0" };
        let cf = b!(self.bld.build_global_string_ptr(close_str, "log.close"));
        b!(self
            .bld
            .build_call(printf, &[cf.as_pointer_value().into()], "log.clc"));
        Ok(())
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

    pub(crate) fn fmt_for_ty_no_newline(&self, ty: &Type) -> &'static str {
        match ty {
            Type::I64 => "%ld",
            Type::I32 | Type::I16 | Type::I8 => "%d",
            Type::U64 => "%lu",
            Type::U32 | Type::U16 | Type::U8 => "%u",
            Type::F64 | Type::F32 => "%f",
            Type::Bool => "%d",
            Type::String => "%.*s",
            _ => "%ld",
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
            Type::Struct(name, _) => {
                let fn_name = format!("{name}_display");
                if let Some((fv, _, _)) = self.fns.get(&fn_name).cloned() {
                    let first_param_is_ptr = fv
                        .get_type()
                        .get_param_types()
                        .first()
                        .map(|t| t.is_pointer_type())
                        .unwrap_or(false);
                    let self_arg: BasicValueEnum = if first_param_is_ptr {
                        let tmp = self.entry_alloca(self.llvm_ty(&ty), "display.self");
                        b!(self.bld.build_store(tmp, val));
                        tmp.into()
                    } else {
                        val
                    };
                    let result = b!(self.bld.build_call(fv, &[self_arg.into()], "display.call"))
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

    pub(crate) fn int_to_string(
        &mut self,
        val: BasicValueEnum<'ctx>,
        unsigned: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let fmt_str = if unsigned { "%lu" } else { "%ld" };
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

        // Fast path for integer formatting: one snprintf into a fixed stack buffer,
        // then convert to SSO/heap string as needed. Avoids the length-probe snprintf.
        let snprintf = self.ensure_snprintf();
        let fmt = b!(self.bld.build_global_string_ptr(fmt_str, "ts.ifmt"));
        let cap = i64t.const_int(32, false); // enough for signed/unsigned 64-bit + NUL
        let buf_arr_ty = i8t.array_type(32);
        let buf_arr = self.entry_alloca(buf_arr_ty.into(), "ts.ibuf");
        let zero = i64t.const_int(0, false);
        let buf = unsafe {
            b!(self
                .bld
                .build_gep(buf_arr_ty, buf_arr, &[zero, zero], "ts.ibuf.ptr"))
        };
        let len_i32 = b!(self.bld.build_call(
            snprintf,
            &[
                buf.into(),
                cap.into(),
                fmt.as_pointer_value().into(),
                wide.into()
            ],
            "ts.ilen"
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void")
        .into_int_value();

        let len = b!(self.bld.build_int_s_extend(len_i32, i64t, "ts.ilen64"));
        self.finalize_string_sso(buf, len, false, "ts.i")
    }

    pub(crate) fn float_to_string(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = val.into_float_value();
        let wide: BasicValueEnum<'ctx> = if fv.get_type() == self.ctx.f32_type() {
            b!(self.bld.build_float_ext(fv, self.ctx.f64_type(), "fpext")).into()
        } else {
            fv.into()
        };
        self.snprintf_to_string("%g", &[wide.into()], "ts")
    }

    pub(crate) fn bool_to_string(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.current_fn();
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
