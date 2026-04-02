use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_string_method(
        &mut self,
        obj: &hir::Expr,
        m: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sv = self.compile_expr(obj)?;
        match m {
            "contains" | "starts_with" | "ends_with" | "char_at" => {
                if args.len() != 1 {
                    return Err(format!("{m}() takes 1 argument"));
                }
                let a = self.compile_expr(&args[0])?;
                match m {
                    "contains" => self.string_contains(sv, a),
                    "starts_with" => self.string_starts_with(sv, a),
                    "ends_with" => self.string_ends_with(sv, a),
                    _ => self.string_char_at(sv, a),
                }
            }
            "slice" => {
                if args.len() != 2 {
                    return Err("slice() takes 2 arguments (start, end)".into());
                }
                let start = self.compile_expr(&args[0])?;
                let end = self.compile_expr(&args[1])?;
                self.string_slice(sv, start, end)
            }
            "length" | "len" => self.string_len(sv),
            "find" => {
                if args.len() != 1 {
                    return Err("find() takes 1 argument".into());
                }
                let a = self.compile_expr(&args[0])?;
                self.string_find(sv, a)
            }
            "trim" => self.string_trim(sv, true, true),
            "trim_left" => self.string_trim(sv, true, false),
            "trim_right" => self.string_trim(sv, false, true),
            "to_upper" => self.string_case(sv, true),
            "to_lower" => self.string_case(sv, false),
            "replace" => {
                if args.len() != 2 {
                    return Err("replace() takes 2 arguments (old, new)".into());
                }
                let old = self.compile_expr(&args[0])?;
                let new = self.compile_expr(&args[1])?;
                self.string_replace(sv, old, new)
            }
            "split" => {
                if args.len() != 1 {
                    return Err("split() takes 1 argument".into());
                }
                let delim = self.compile_expr(&args[0])?;
                self.string_split(sv, delim)
            }
            "lines" => {
                let newline = self.compile_str_literal("\n")?;
                self.string_split(sv, newline)
            }
            "repeat" => {
                if args.len() != 1 {
                    return Err("repeat() takes 1 argument".into());
                }
                let count = self.compile_expr(&args[0])?;
                self.string_repeat(sv, count)
            }
            "is_empty" => {
                let len = self.string_len(sv)?.into_int_value();
                let i64t = self.ctx.i64_type();
                let cmp = b!(self.bld.build_int_compare(
                    IntPredicate::EQ,
                    len,
                    i64t.const_int(0, false),
                    "isempty"
                ));
                let i1t = self.ctx.bool_type();
                let ext = b!(self.bld.build_int_z_extend(cmp, i1t, "isempty.ext"));
                Ok(ext.into())
            }
            "matches" => {
                if args.len() != 1 {
                    return Err("matches() takes 1 argument (pattern)".into());
                }
                let pattern = self.compile_expr(&args[0])?;
                let ptr_t = self.ctx.ptr_type(AddressSpace::default());
                let bool_t = self.ctx.bool_type();
                let fn_type = bool_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
                let func = self
                    .module
                    .get_function("__jade_regex_match")
                    .unwrap_or_else(|| self.module.add_function("__jade_regex_match", fn_type, None));
                let result = b!(self.bld.build_call(func, &[sv.into(), pattern.into()], "re.match"));
                Ok(result.try_as_basic_value().basic().unwrap())
            }
            "find_all" => {
                if args.len() != 1 {
                    return Err("find_all() takes 1 argument (pattern)".into());
                }
                let pattern = self.compile_expr(&args[0])?;
                let ptr_t = self.ctx.ptr_type(AddressSpace::default());
                let fn_type = ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
                let func = self
                    .module
                    .get_function("__jade_regex_find_all")
                    .unwrap_or_else(|| self.module.add_function("__jade_regex_find_all", fn_type, None));
                let result = b!(self.bld.build_call(func, &[sv.into(), pattern.into()], "re.findall"));
                Ok(result.try_as_basic_value().basic().unwrap())
            }
            "replace_re" => {
                if args.len() != 2 {
                    return Err("replace_re() takes 2 arguments (pattern, replacement)".into());
                }
                let pattern = self.compile_expr(&args[0])?;
                let replacement = self.compile_expr(&args[1])?;
                let ptr_t = self.ctx.ptr_type(AddressSpace::default());
                let fn_type = ptr_t.fn_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into()], false);
                let func = self
                    .module
                    .get_function("__jade_regex_replace")
                    .unwrap_or_else(|| self.module.add_function("__jade_regex_replace", fn_type, None));
                let result = b!(self.bld.build_call(func, &[sv.into(), pattern.into(), replacement.into()], "re.replace"));
                Ok(result.try_as_basic_value().basic().unwrap())
            }
            _ => {
                Err(format!("no method '{m}' on String"))
            }
        }
    }

    pub(crate) fn string_len(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let st = self.string_type();
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();

        let (ptr, tag, sso_bb, heap_bb, merge_bb) = self.sso_branch(val, "len")?;

        self.bld.position_at_end(sso_bb);
        let sso_len_i8 = b!(self
            .bld
            .build_and(tag, i8t.const_int(0x7F, false), "sso.l8"));
        let sso_len = b!(self.bld.build_int_z_extend(sso_len_i8, i64t, "sso.len"));
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(heap_bb);
        let lp = b!(self.bld.build_struct_gep(st, ptr, 1, "s.lenp"));
        let heap_len = b!(self.bld.build_load(i64t, lp, "heap.len")).into_int_value();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(i64t, "len"));
        phi.add_incoming(&[(&sso_len, sso_bb), (&heap_len, heap_bb)]);
        Ok(phi.as_basic_value())
    }

    pub(crate) fn string_data(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let st = self.string_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let (ptr, _, sso_bb, heap_bb, merge_bb) = self.sso_branch(val, "data")?;

        self.bld.position_at_end(sso_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(heap_bb);
        let dp = b!(self.bld.build_struct_gep(st, ptr, 0, "s.datap"));
        let heap_data = b!(self.bld.build_load(ptr_ty, dp, "heap.data")).into_pointer_value();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(ptr_ty, "data"));
        phi.add_incoming(&[(&ptr, sso_bb), (&heap_data, heap_bb)]);
        Ok(phi.as_basic_value())
    }

    pub(crate) fn build_string(
        &mut self,
        data: impl Into<BasicValueEnum<'ctx>>,
        len: impl Into<BasicValueEnum<'ctx>>,
        cap: impl Into<BasicValueEnum<'ctx>>,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let st = self.string_type();
        let out = self.entry_alloca(st.into(), name);
        let dp = b!(self.bld.build_struct_gep(st, out, 0, "s.data"));
        b!(self.bld.build_store(dp, data.into()));
        let lp = b!(self.bld.build_struct_gep(st, out, 1, "s.len"));
        b!(self.bld.build_store(lp, len.into()));
        let cp = b!(self.bld.build_struct_gep(st, out, 2, "s.cap"));
        b!(self.bld.build_store(cp, cap.into()));
        Ok(b!(self.bld.build_load(st, out, name)))
    }

    pub(crate) fn sso_branch(
        &mut self,
        val: BasicValueEnum<'ctx>,
        prefix: &str,
    ) -> Result<
        (
            inkwell::values::PointerValue<'ctx>,
            inkwell::values::IntValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
        ),
        String,
    > {
        let st = self.string_type();
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        let fv = self.cur_fn.unwrap();
        let ptr = self.entry_alloca(st.into(), "s.tmp");
        b!(self.bld.build_store(ptr, val));
        let tag_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, ptr, &[i64t.const_int(23, false)], "s.tagp"))
        };
        let tag = b!(self.bld.build_load(i8t, tag_ptr, "s.tag")).into_int_value();
        let masked = b!(self.bld.build_and(tag, i8t.const_int(0x80, false), "s.hi"));
        let is_sso = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            masked,
            i8t.const_int(0, false),
            "s.issso"
        ));
        let sso_bb = self.ctx.append_basic_block(fv, &format!("sso.{prefix}"));
        let heap_bb = self.ctx.append_basic_block(fv, &format!("heap.{prefix}"));
        let merge_bb = self.ctx.append_basic_block(fv, &format!("merge.{prefix}"));
        b!(self.bld.build_conditional_branch(is_sso, sso_bb, heap_bb));
        Ok((ptr, tag, sso_bb, heap_bb, merge_bb))
    }

    pub(crate) fn build_sso_result(
        &mut self,
        alloca: inkwell::values::PointerValue<'ctx>,
        len: inkwell::values::IntValue<'ctx>,
        merge_bb: inkwell::basic_block::BasicBlock<'ctx>,
        prefix: &str,
    ) -> Result<(BasicValueEnum<'ctx>, inkwell::basic_block::BasicBlock<'ctx>), String> {
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        let st = self.string_type();
        let tag_ptr = unsafe {
            b!(self.bld.build_gep(
                i8t,
                alloca,
                &[i64t.const_int(23, false)],
                &format!("{prefix}.tagp")
            ))
        };
        let len_i8 = b!(self
            .bld
            .build_int_truncate(len, i8t, &format!("{prefix}.l8")));
        let tag =
            b!(self
                .bld
                .build_or(len_i8, i8t.const_int(0x80, false), &format!("{prefix}.tag")));
        b!(self.bld.build_store(tag_ptr, tag));
        let val = b!(self.bld.build_load(st, alloca, &format!("{prefix}.ssov")));
        let exit_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));
        Ok((val, exit_bb))
    }

    pub(crate) fn finalize_string_sso(
        &mut self,
        src: inkwell::values::PointerValue<'ctx>,
        len: inkwell::values::IntValue<'ctx>,
        owns_buffer: bool,
        prefix: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let st = self.string_type();
        let fv = self.cur_fn.unwrap();

        let fits = b!(self.bld.build_int_compare(
            IntPredicate::ULE,
            len,
            i64t.const_int(23, false),
            &format!("{prefix}.fits")
        ));
        let sso_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.sso"));
        let heap_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.heap"));
        let merge_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.merge"));
        b!(self.bld.build_conditional_branch(fits, sso_bb, heap_bb));

        self.bld.position_at_end(sso_bb);
        let sso_out = self.entry_alloca(st.into(), &format!("{prefix}.sso"));
        b!(self.bld.build_store(sso_out, st.const_zero()));
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[sso_out.into(), src.into(), len.into()], ""));
        if owns_buffer {
            let free = self.ensure_free();
            b!(self.bld.build_call(free, &[src.into()], ""));
        }
        let (sso_val, sso_exit) = self.build_sso_result(sso_out, len, merge_bb, prefix)?;

        self.bld.position_at_end(heap_bb);
        let heap_buf = if owns_buffer {
            src
        } else {
            let malloc = self.ensure_malloc();
            let buf = b!(self
                .bld
                .build_call(malloc, &[len.into()], &format!("{prefix}.buf")))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
            b!(self
                .bld
                .build_call(memcpy, &[buf.into(), src.into(), len.into()], ""));
            buf
        };
        let heap_val = self.build_string(heap_buf, len, len, &format!("{prefix}.hv"))?;
        let heap_exit = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(st, &format!("{prefix}.v")));
        phi.add_incoming(&[(&sso_val, sso_exit), (&heap_val, heap_exit)]);
        Ok(phi.as_basic_value())
    }

    pub(crate) fn snprintf_to_string(
        &mut self,
        fmt_str: &str,
        args: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
        prefix: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let snprintf = self.ensure_snprintf();
        let fmt = b!(self
            .bld
            .build_global_string_ptr(fmt_str, &format!("{prefix}.fmt")));

        let null = ptr_ty.const_null();
        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = vec![
            null.into(),
            i64t.const_int(0, false).into(),
            fmt.as_pointer_value().into(),
        ];
        call_args.extend_from_slice(args);

        let len = b!(self
            .bld
            .build_call(snprintf, &call_args, &format!("{prefix}.len")))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        let len = b!(self
            .bld
            .build_int_s_extend(len, i64t, &format!("{prefix}.len64")));
        let size =
            b!(self
                .bld
                .build_int_nsw_add(len, i64t.const_int(1, false), &format!("{prefix}.sz")));
        let malloc = self.ensure_malloc();
        let buf = b!(self
            .bld
            .build_call(malloc, &[size.into()], &format!("{prefix}.buf")))
        .try_as_basic_value()
        .basic()
        .unwrap();

        let mut call_args2: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            vec![buf.into(), size.into(), fmt.as_pointer_value().into()];
        call_args2.extend_from_slice(args);
        b!(self.bld.build_call(snprintf, &call_args2, ""));

        self.build_string(buf, len, size, &format!("{prefix}.s"))
    }
}
