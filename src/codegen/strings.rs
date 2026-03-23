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
            _ => Err(format!("no method '{m}' on String")),
        }
    }

    pub(crate) fn string_eq(
        &mut self,
        l: BasicValueEnum<'ctx>,
        r: BasicValueEnum<'ctx>,
        negate: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let memcmp = self.ensure_memcmp();

        let llen = self.string_len(l)?.into_int_value();
        let rlen = self.string_len(r)?.into_int_value();
        let ldata = self.string_data(l)?.into_pointer_value();
        let rdata = self.string_data(r)?.into_pointer_value();

        // If lengths differ, strings aren't equal
        let len_eq = b!(self.bld.build_int_compare(IntPredicate::EQ, llen, rlen, "seq.leq"));
        let cmp_bb = self.ctx.append_basic_block(fv, "seq.cmp");
        let neq_bb = self.ctx.append_basic_block(fv, "seq.neq");
        let merge_bb = self.ctx.append_basic_block(fv, "seq.merge");
        b!(self.bld.build_conditional_branch(len_eq, cmp_bb, neq_bb));

        self.bld.position_at_end(cmp_bb);
        let cmp = b!(self.bld.build_call(memcmp, &[ldata.into(), rdata.into(), llen.into()], "seq.cmp"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let eq = b!(self.bld.build_int_compare(
            IntPredicate::EQ, cmp, self.ctx.i32_type().const_int(0, false), "seq.eq"
        ));
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(neq_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(self.ctx.bool_type(), "seq.v"));
        phi.add_incoming(&[
            (&eq, cmp_bb),
            (&self.ctx.bool_type().const_int(0, false), neq_bb),
        ]);
        let result = phi.as_basic_value().into_int_value();
        if negate {
            Ok(b!(self.bld.build_not(result, "seq.neg")).into())
        } else {
            Ok(result.into())
        }
    }

    pub(crate) fn string_concat(
        &mut self,
        l: BasicValueEnum<'ctx>,
        r: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        let st = self.string_type();
        let fv = self.cur_fn.unwrap();

        let llen = self.string_len(l)?.into_int_value();
        let rlen = self.string_len(r)?.into_int_value();
        let total = b!(self.bld.build_int_add(llen, rlen, "total"));
        let ldata = self.string_data(l)?.into_pointer_value();
        let rdata = self.string_data(r)?.into_pointer_value();

        let fits = b!(self.bld.build_int_compare(
            IntPredicate::ULE,
            total,
            i64t.const_int(23, false),
            "cat.fits"
        ));
        let sso_bb = self.ctx.append_basic_block(fv, "cat.sso");
        let heap_bb = self.ctx.append_basic_block(fv, "cat.heap");
        let merge_bb = self.ctx.append_basic_block(fv, "cat.merge");
        b!(self
            .bld
            .build_conditional_branch(fits, sso_bb, heap_bb));

        self.bld.position_at_end(sso_bb);
        let sso_out = self.entry_alloca(st.into(), "cat.sso");
        b!(self.bld.build_store(sso_out, st.const_zero()));
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[sso_out.into(), ldata.into(), llen.into()], ""));
        let sso_dst = unsafe {
            b!(self
                .bld
                .build_gep(i8t, sso_out, &[llen], "cat.dst"))
        };
        b!(self
            .bld
            .build_call(memcpy, &[sso_dst.into(), rdata.into(), rlen.into()], ""));
        let (sso_val, sso_exit) = self.build_sso_result(sso_out, total, merge_bb, "cat")?;

        self.bld.position_at_end(heap_bb);
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[total.into()], "buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        let memcpy2 = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy2, &[buf.into(), ldata.into(), llen.into()], ""));
        let dst = unsafe {
            b!(self
                .bld
                .build_gep(i8t, buf.into_pointer_value(), &[llen], "dst"))
        };
        b!(self
            .bld
            .build_call(memcpy2, &[dst.into(), rdata.into(), rlen.into()], ""));
        let heap_val = self.build_string(buf, total, total, "cat")?;
        let heap_exit = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(st, "cat.v"));
        phi.add_incoming(&[(&sso_val, sso_exit), (&heap_val, heap_exit)]);
        Ok(phi.as_basic_value())
    }

    pub(crate) fn string_contains(
        &mut self,
        haystack: BasicValueEnum<'ctx>,
        needle: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let hlen = self.string_len(haystack)?.into_int_value();
        let nlen = self.string_len(needle)?.into_int_value();
        let hdata = self.string_data(haystack)?.into_pointer_value();
        let ndata = self.string_data(needle)?.into_pointer_value();
        let memcmp = self.ensure_memcmp();

        let ne_zero =
            b!(self
                .bld
                .build_int_compare(IntPredicate::EQ, nlen, i64t.const_int(0, false), "nz"));
        let check_bb = self.ctx.append_basic_block(fv, "sc.check");
        let loop_bb = self.ctx.append_basic_block(fv, "sc.loop");
        let found_bb = self.ctx.append_basic_block(fv, "sc.found");
        let notfound_bb = self.ctx.append_basic_block(fv, "sc.nf");
        let merge_bb = self.ctx.append_basic_block(fv, "sc.merge");

        b!(self
            .bld
            .build_conditional_branch(ne_zero, found_bb, check_bb));

        self.bld.position_at_end(check_bb);
        let ok = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, hlen, nlen, "ok"));
        b!(self.bld.build_conditional_branch(ok, loop_bb, notfound_bb));

        self.bld.position_at_end(loop_bb);
        let phi_i = b!(self.bld.build_phi(i64t, "i"));
        phi_i.add_incoming(&[(&i64t.const_int(0, false), check_bb)]);
        let i = phi_i.as_basic_value().into_int_value();
        let ptr = unsafe { b!(self.bld.build_gep(self.ctx.i8_type(), hdata, &[i], "hp")) };
        let cmp = b!(self
            .bld
            .build_call(memcmp, &[ptr.into(), ndata.into(), nlen.into()], "cmp"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        let eq = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            cmp,
            self.ctx.i32_type().const_int(0, false),
            "eq"
        ));
        let cont_bb = self.ctx.append_basic_block(fv, "sc.cont");
        b!(self.bld.build_conditional_branch(eq, found_bb, cont_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "next"));
        let limit = b!(self.bld.build_int_nsw_sub(hlen, nlen, "lim"));
        let limit1 = b!(self
            .bld
            .build_int_add(limit, i64t.const_int(1, false), "lim1"));
        let done = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, next, limit1, "done"));
        phi_i.add_incoming(&[(&next, cont_bb)]);
        b!(self
            .bld
            .build_conditional_branch(done, notfound_bb, loop_bb));

        self.bld.position_at_end(found_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));
        self.bld.position_at_end(notfound_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(self.ctx.bool_type(), "sc.res"));
        phi.add_incoming(&[
            (&self.ctx.bool_type().const_int(1, false), found_bb),
            (&self.ctx.bool_type().const_int(0, false), notfound_bb),
        ]);
        Ok(phi.as_basic_value())
    }

    pub(crate) fn string_starts_with(
        &mut self,
        haystack: BasicValueEnum<'ctx>,
        part: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.string_affix_match(haystack, part, false)
    }

    pub(crate) fn string_ends_with(
        &mut self,
        haystack: BasicValueEnum<'ctx>,
        part: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.string_affix_match(haystack, part, true)
    }

    fn string_affix_match(
        &mut self,
        haystack: BasicValueEnum<'ctx>,
        part: BasicValueEnum<'ctx>,
        from_end: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let p = if from_end { "ew" } else { "sw" };
        let hlen = self.string_len(haystack)?.into_int_value();
        let plen = self.string_len(part)?.into_int_value();
        let hdata = self.string_data(haystack)?.into_pointer_value();
        let pdata = self.string_data(part)?.into_pointer_value();
        let memcmp = self.ensure_memcmp();

        let ok = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, hlen, plen, &format!("{p}.ok")));
        let cmp_bb = self.ctx.append_basic_block(fv, &format!("{p}.cmp"));
        let fail_bb = self.ctx.append_basic_block(fv, &format!("{p}.fail"));
        let merge_bb = self.ctx.append_basic_block(fv, &format!("{p}.merge"));
        b!(self.bld.build_conditional_branch(ok, cmp_bb, fail_bb));

        self.bld.position_at_end(cmp_bb);
        let hptr: inkwell::values::PointerValue<'ctx> = if from_end {
            let off = b!(self.bld.build_int_nsw_sub(hlen, plen, &format!("{p}.off")));
            unsafe {
                b!(self
                    .bld
                    .build_gep(self.ctx.i8_type(), hdata, &[off], &format!("{p}.ptr")))
            }
        } else {
            hdata
        };
        let cmp = b!(self.bld.build_call(
            memcmp,
            &[hptr.into(), pdata.into(), plen.into()],
            &format!("{p}.cmp")
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        let eq = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            cmp,
            self.ctx.i32_type().const_int(0, false),
            &format!("{p}.eq")
        ));
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(fail_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self
            .bld
            .build_phi(self.ctx.bool_type(), &format!("{p}.res")));
        phi.add_incoming(&[
            (&eq, cmp_bb),
            (&self.ctx.bool_type().const_int(0, false), fail_bb),
        ]);
        Ok(phi.as_basic_value())
    }

    pub(crate) fn string_char_at(
        &mut self,
        s: BasicValueEnum<'ctx>,
        idx: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let data = self.string_data(s)?.into_pointer_value();
        let i = idx.into_int_value();
        let ptr = unsafe { b!(self.bld.build_gep(self.ctx.i8_type(), data, &[i], "ca.ptr")) };
        let byte = b!(self.bld.build_load(self.ctx.i8_type(), ptr, "ca.byte"));
        Ok(b!(self
            .bld
            .build_int_z_extend(byte.into_int_value(), i64t, "ca.val"))
        .into())
    }

    pub(crate) fn string_slice(
        &mut self,
        s: BasicValueEnum<'ctx>,
        start: BasicValueEnum<'ctx>,
        end: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        let st = self.string_type();
        let fv = self.cur_fn.unwrap();

        let data = self.string_data(s)?.into_pointer_value();
        let si = start.into_int_value();
        let ei = end.into_int_value();
        let new_len = b!(self.bld.build_int_nsw_sub(ei, si, "sl.len"));
        let src = unsafe {
            b!(self
                .bld
                .build_gep(i8t, data, &[si], "sl.src"))
        };

        let fits = b!(self.bld.build_int_compare(
            IntPredicate::ULE,
            new_len,
            i64t.const_int(23, false),
            "sl.fits"
        ));
        let sso_bb = self.ctx.append_basic_block(fv, "sl.sso");
        let heap_bb = self.ctx.append_basic_block(fv, "sl.heap");
        let merge_bb = self.ctx.append_basic_block(fv, "sl.merge");
        b!(self
            .bld
            .build_conditional_branch(fits, sso_bb, heap_bb));

        self.bld.position_at_end(sso_bb);
        let sso_out = self.entry_alloca(st.into(), "sl.sso");
        b!(self.bld.build_store(sso_out, st.const_zero()));
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[sso_out.into(), src.into(), new_len.into()], ""));
        let (sso_val, sso_exit) = self.build_sso_result(sso_out, new_len, merge_bb, "sl")?;

        self.bld.position_at_end(heap_bb);
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[new_len.into()], "sl.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        let memcpy2 = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy2, &[buf.into(), src.into(), new_len.into()], ""));
        let heap_val = self.build_string(buf, new_len, new_len, "sl.val")?;
        let heap_exit = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(st, "sl.v"));
        phi.add_incoming(&[(&sso_val, sso_exit), (&heap_val, heap_exit)]);
        Ok(phi.as_basic_value())
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
        let sso_len = b!(self
            .bld
            .build_int_z_extend(sso_len_i8, i64t, "sso.len"));
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

    fn sso_branch(
        &mut self,
        val: BasicValueEnum<'ctx>,
        prefix: &str,
    ) -> Result<(
        inkwell::values::PointerValue<'ctx>,
        inkwell::values::IntValue<'ctx>,
        inkwell::basic_block::BasicBlock<'ctx>,
        inkwell::basic_block::BasicBlock<'ctx>,
        inkwell::basic_block::BasicBlock<'ctx>,
    ), String> {
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
        let masked = b!(self
            .bld
            .build_and(tag, i8t.const_int(0x80, false), "s.hi"));
        let is_sso = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            masked,
            i8t.const_int(0, false),
            "s.issso"
        ));
        let sso_bb = self.ctx.append_basic_block(fv, &format!("sso.{prefix}"));
        let heap_bb = self.ctx.append_basic_block(fv, &format!("heap.{prefix}"));
        let merge_bb = self.ctx.append_basic_block(fv, &format!("merge.{prefix}"));
        b!(self
            .bld
            .build_conditional_branch(is_sso, sso_bb, heap_bb));
        Ok((ptr, tag, sso_bb, heap_bb, merge_bb))
    }

    fn build_sso_result(
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
        let tag = b!(self
            .bld
            .build_or(len_i8, i8t.const_int(0x80, false), &format!("{prefix}.tag")));
        b!(self.bld.build_store(tag_ptr, tag));
        let val = b!(self
            .bld
            .build_load(st, alloca, &format!("{prefix}.ssov")));
        let exit_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));
        Ok((val, exit_bb))
    }

    // ── find: return index of needle in haystack, or -1 ────────────────

    pub(crate) fn string_find(
        &mut self,
        haystack: BasicValueEnum<'ctx>,
        needle: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let hlen = self.string_len(haystack)?.into_int_value();
        let nlen = self.string_len(needle)?.into_int_value();
        let hdata = self.string_data(haystack)?.into_pointer_value();
        let ndata = self.string_data(needle)?.into_pointer_value();
        let memcmp = self.ensure_memcmp();

        let check_bb = self.ctx.append_basic_block(fv, "sf.check");
        let loop_bb = self.ctx.append_basic_block(fv, "sf.loop");
        let found_bb = self.ctx.append_basic_block(fv, "sf.found");
        let nf_bb = self.ctx.append_basic_block(fv, "sf.nf");
        let merge_bb = self.ctx.append_basic_block(fv, "sf.merge");

        // Empty needle → found at 0
        let nz = b!(self.bld.build_int_compare(
            IntPredicate::EQ, nlen, i64t.const_int(0, false), "sf.nz"
        ));
        let pre_check_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_conditional_branch(nz, found_bb, check_bb));

        self.bld.position_at_end(check_bb);
        let ok = b!(self.bld.build_int_compare(IntPredicate::SGE, hlen, nlen, "sf.ok"));
        b!(self.bld.build_conditional_branch(ok, loop_bb, nf_bb));

        self.bld.position_at_end(loop_bb);
        let phi_i = b!(self.bld.build_phi(i64t, "sf.i"));
        phi_i.add_incoming(&[(&i64t.const_int(0, false), check_bb)]);
        let i = phi_i.as_basic_value().into_int_value();
        let ptr = unsafe { b!(self.bld.build_gep(i8t, hdata, &[i], "sf.p")) };
        let cmp = b!(self.bld.build_call(memcmp, &[ptr.into(), ndata.into(), nlen.into()], "sf.cmp"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let eq = b!(self.bld.build_int_compare(IntPredicate::EQ, cmp, self.ctx.i32_type().const_int(0, false), "sf.eq"));
        let cont_bb = self.ctx.append_basic_block(fv, "sf.cont");
        b!(self.bld.build_conditional_branch(eq, found_bb, cont_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "sf.n"));
        let limit = b!(self.bld.build_int_nsw_sub(hlen, nlen, "sf.lim"));
        let lim1 = b!(self.bld.build_int_add(limit, i64t.const_int(1, false), "sf.l1"));
        let done = b!(self.bld.build_int_compare(IntPredicate::SGE, next, lim1, "sf.done"));
        phi_i.add_incoming(&[(&next, cont_bb)]);
        b!(self.bld.build_conditional_branch(done, nf_bb, loop_bb));

        self.bld.position_at_end(found_bb);
        let found_phi = b!(self.bld.build_phi(i64t, "sf.fi"));
        found_phi.add_incoming(&[
            (&i64t.const_int(0, false), pre_check_bb),
            (&i, loop_bb),
        ]);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(nf_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(i64t, "sf.v"));
        phi.add_incoming(&[
            (&found_phi.as_basic_value(), found_bb),
            (&i64t.const_int(u64::MAX, true), nf_bb), // -1
        ]);
        Ok(phi.as_basic_value())
    }

    // ── trim: skip whitespace from left/right ──────────────────────────

    pub(crate) fn string_trim(
        &mut self,
        s: BasicValueEnum<'ctx>,
        left: bool,
        right: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let data = self.string_data(s)?.into_pointer_value();
        let len = self.string_len(s)?.into_int_value();

        // Find left bound
        let left_idx = if left {
            let loop_bb = self.ctx.append_basic_block(fv, "tl.loop");
            let done_bb = self.ctx.append_basic_block(fv, "tl.done");
            let entry_bb = self.bld.get_insert_block().unwrap();
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(loop_bb);
            let phi = b!(self.bld.build_phi(i64t, "tl.i"));
            phi.add_incoming(&[(&i64t.const_int(0, false), entry_bb)]);
            let i = phi.as_basic_value().into_int_value();
            let at_end = b!(self.bld.build_int_compare(IntPredicate::SGE, i, len, "tl.end"));
            let check_bb = self.ctx.append_basic_block(fv, "tl.chk");
            b!(self.bld.build_conditional_branch(at_end, done_bb, check_bb));

            self.bld.position_at_end(check_bb);
            let bp = unsafe { b!(self.bld.build_gep(i8t, data, &[i], "tl.bp")) };
            let byte = b!(self.bld.build_load(i8t, bp, "tl.b")).into_int_value();
            let is_space = self.is_whitespace(byte)?;
            let next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "tl.n"));
            phi.add_incoming(&[(&next, check_bb)]);
            b!(self.bld.build_conditional_branch(is_space, loop_bb, done_bb));

            self.bld.position_at_end(done_bb);
            let result = b!(self.bld.build_phi(i64t, "tl.v"));
            result.add_incoming(&[(&i, loop_bb), (&i, check_bb)]);
            result.as_basic_value().into_int_value()
        } else {
            i64t.const_int(0, false)
        };

        // Find right bound
        let right_idx = if right {
            let loop_bb = self.ctx.append_basic_block(fv, "tr.loop");
            let done_bb = self.ctx.append_basic_block(fv, "tr.done");
            let entry_bb = self.bld.get_insert_block().unwrap();
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(loop_bb);
            let phi = b!(self.bld.build_phi(i64t, "tr.i"));
            phi.add_incoming(&[(&len, entry_bb)]);
            let i = phi.as_basic_value().into_int_value();
            let at_start = b!(self.bld.build_int_compare(IntPredicate::SLE, i, left_idx, "tr.end"));
            let check_bb = self.ctx.append_basic_block(fv, "tr.chk");
            b!(self.bld.build_conditional_branch(at_start, done_bb, check_bb));

            self.bld.position_at_end(check_bb);
            let prev = b!(self.bld.build_int_nsw_sub(i, i64t.const_int(1, false), "tr.p"));
            let bp = unsafe { b!(self.bld.build_gep(i8t, data, &[prev], "tr.bp")) };
            let byte = b!(self.bld.build_load(i8t, bp, "tr.b")).into_int_value();
            let is_space = self.is_whitespace(byte)?;
            phi.add_incoming(&[(&prev, check_bb)]);
            b!(self.bld.build_conditional_branch(is_space, loop_bb, done_bb));

            self.bld.position_at_end(done_bb);
            let result = b!(self.bld.build_phi(i64t, "tr.v"));
            result.add_incoming(&[(&i, loop_bb), (&i, check_bb)]);
            result.as_basic_value().into_int_value()
        } else {
            len
        };

        // Build slice [left_idx..right_idx]
        self.string_slice(
            s,
            left_idx.into(),
            right_idx.into(),
        )
    }

    fn is_whitespace(&mut self, byte: inkwell::values::IntValue<'ctx>) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let i8t = self.ctx.i8_type();
        // space, tab, newline, carriage return
        let sp = b!(self.bld.build_int_compare(IntPredicate::EQ, byte, i8t.const_int(b' ' as u64, false), "ws.sp"));
        let tab = b!(self.bld.build_int_compare(IntPredicate::EQ, byte, i8t.const_int(b'\t' as u64, false), "ws.t"));
        let nl = b!(self.bld.build_int_compare(IntPredicate::EQ, byte, i8t.const_int(b'\n' as u64, false), "ws.n"));
        let cr = b!(self.bld.build_int_compare(IntPredicate::EQ, byte, i8t.const_int(b'\r' as u64, false), "ws.r"));
        let a = b!(self.bld.build_or(sp, tab, "ws.a"));
        let b2 = b!(self.bld.build_or(nl, cr, "ws.b"));
        Ok(b!(self.bld.build_or(a, b2, "ws.v")))
    }

    // ── to_upper / to_lower ────────────────────────────────────────────

    pub(crate) fn string_case(
        &mut self,
        s: BasicValueEnum<'ctx>,
        upper: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let st = self.string_type();
        let data = self.string_data(s)?.into_pointer_value();
        let len = self.string_len(s)?.into_int_value();

        // Allocate output buffer (always heap for simplicity — could optimize for SSO)
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[len.into()], "sc.buf"))
            .try_as_basic_value().basic().unwrap().into_pointer_value();

        // Loop: convert each byte
        let loop_bb = self.ctx.append_basic_block(fv, "sc.loop");
        let body_bb = self.ctx.append_basic_block(fv, "sc.body");
        let done_bb = self.ctx.append_basic_block(fv, "sc.done");
        let entry_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let phi_i = b!(self.bld.build_phi(i64t, "sc.i"));
        phi_i.add_incoming(&[(&i64t.const_int(0, false), entry_bb)]);
        let i = phi_i.as_basic_value().into_int_value();
        let at_end = b!(self.bld.build_int_compare(IntPredicate::SGE, i, len, "sc.end"));
        b!(self.bld.build_conditional_branch(at_end, done_bb, body_bb));

        self.bld.position_at_end(body_bb);
        let src_p = unsafe { b!(self.bld.build_gep(i8t, data, &[i], "sc.sp")) };
        let byte = b!(self.bld.build_load(i8t, src_p, "sc.b")).into_int_value();

        let (lo, hi) = if upper { (b'a', b'z') } else { (b'A', b'Z') };
        let in_range_lo = b!(self.bld.build_int_compare(
            IntPredicate::UGE, byte, i8t.const_int(lo as u64, false), "sc.lo"
        ));
        let in_range_hi = b!(self.bld.build_int_compare(
            IntPredicate::ULE, byte, i8t.const_int(hi as u64, false), "sc.hi"
        ));
        let in_range = b!(self.bld.build_and(in_range_lo, in_range_hi, "sc.ir"));

        let diff = if upper {
            i8t.const_int((b'a' - b'A') as u64, false)
        } else {
            i8t.const_int((b'a' - b'A') as u64, false)
        };
        let converted = if upper {
            b!(self.bld.build_int_nsw_sub(byte, diff, "sc.cv"))
        } else {
            b!(self.bld.build_int_add(byte, diff, "sc.cv"))
        };
        let out_byte = b!(self.bld.build_select(in_range, converted, byte, "sc.ob"))
            .into_int_value();

        let dst_p = unsafe { b!(self.bld.build_gep(i8t, buf, &[i], "sc.dp")) };
        b!(self.bld.build_store(dst_p, out_byte));
        let next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "sc.n"));
        phi_i.add_incoming(&[(&next, body_bb)]);
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        // Build SSO-aware string from buf + len
        let fits = b!(self.bld.build_int_compare(
            IntPredicate::ULE, len, i64t.const_int(23, false), "sc.fits"
        ));
        let sso_bb = self.ctx.append_basic_block(fv, "sc.sso");
        let heap_bb = self.ctx.append_basic_block(fv, "sc.heap");
        let merge_bb = self.ctx.append_basic_block(fv, "sc.merge");
        b!(self.bld.build_conditional_branch(fits, sso_bb, heap_bb));

        self.bld.position_at_end(sso_bb);
        let sso_out = self.entry_alloca(st.into(), "sc.sso");
        b!(self.bld.build_store(sso_out, st.const_zero()));
        let memcpy = self.ensure_memcpy();
        b!(self.bld.build_call(memcpy, &[sso_out.into(), buf.into(), len.into()], ""));
        let free = self.ensure_free();
        b!(self.bld.build_call(free, &[buf.into()], ""));
        let (sso_val, sso_exit) = self.build_sso_result(sso_out, len, merge_bb, "sc")?;

        self.bld.position_at_end(heap_bb);
        let heap_val = self.build_string(buf, len, len, "sc.hv")?;
        let heap_exit = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(st, "sc.v"));
        phi.add_incoming(&[(&sso_val, sso_exit), (&heap_val, heap_exit)]);
        Ok(phi.as_basic_value())
    }

    // ── replace(old, new) ──────────────────────────────────────────────

    pub(crate) fn string_replace(
        &mut self,
        s: BasicValueEnum<'ctx>,
        old: BasicValueEnum<'ctx>,
        new: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Strategy: scan for occurrences, build result buffer
        // Simple approach: use C strstr-like loop, accumulate into a Vec-like buffer
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let st = self.string_type();
        let sdata = self.string_data(s)?.into_pointer_value();
        let slen = self.string_len(s)?.into_int_value();
        let odata = self.string_data(old)?.into_pointer_value();
        let olen = self.string_len(old)?.into_int_value();
        let ndata = self.string_data(new)?.into_pointer_value();
        let nlen = self.string_len(new)?.into_int_value();
        let memcmp = self.ensure_memcmp();
        let memcpy = self.ensure_memcpy();
        let malloc = self.ensure_malloc();

        // Worst case: every char is a match, max output = slen * (nlen+1)
        // Start with slen * 2 buffer
        let init_cap = b!(self.bld.build_int_nsw_mul(slen, i64t.const_int(2, false), "rep.ic"));
        let init_cap_min = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(IntPredicate::SGT, init_cap, i64t.const_int(64, false), "rep.cmp")),
            init_cap, i64t.const_int(64, false), "rep.cap"
        )).into_int_value();
        let buf = b!(self.bld.build_call(malloc, &[init_cap_min.into()], "rep.buf"))
            .try_as_basic_value().basic().unwrap().into_pointer_value();

        let cond_bb = self.ctx.append_basic_block(fv, "rep.cond");
        let match_bb = self.ctx.append_basic_block(fv, "rep.match");
        let nomatch_bb = self.ctx.append_basic_block(fv, "rep.nm");
        let copy_new_bb = self.ctx.append_basic_block(fv, "rep.copy");
        let done_bb = self.ctx.append_basic_block(fv, "rep.done");

        // Store buf/cap/out_len in allocas so we can update them
        let buf_alloca = self.entry_alloca(self.ctx.ptr_type(AddressSpace::default()).into(), "rep.ba");
        b!(self.bld.build_store(buf_alloca, buf));
        let cap_alloca = self.entry_alloca(i64t.into(), "rep.ca");
        b!(self.bld.build_store(cap_alloca, init_cap_min));

        let entry_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(cond_bb);
        let phi_si = b!(self.bld.build_phi(i64t, "rep.si"));
        phi_si.add_incoming(&[(&i64t.const_int(0, false), entry_bb)]);
        let phi_out = b!(self.bld.build_phi(i64t, "rep.out"));
        phi_out.add_incoming(&[(&i64t.const_int(0, false), entry_bb)]);
        let si = phi_si.as_basic_value().into_int_value();
        let out_len = phi_out.as_basic_value().into_int_value();
        let at_end = b!(self.bld.build_int_compare(IntPredicate::SGE, si, slen, "rep.end"));
        b!(self.bld.build_conditional_branch(at_end, done_bb, match_bb));

        // Try to match old at position si
        self.bld.position_at_end(match_bb);
        let remaining = b!(self.bld.build_int_nsw_sub(slen, si, "rep.rem"));
        let can_match = b!(self.bld.build_int_compare(IntPredicate::SGE, remaining, olen, "rep.cm"));
        let try_bb = self.ctx.append_basic_block(fv, "rep.try");
        b!(self.bld.build_conditional_branch(can_match, try_bb, nomatch_bb));

        self.bld.position_at_end(try_bb);
        let src_ptr = unsafe { b!(self.bld.build_gep(i8t, sdata, &[si], "rep.sp")) };
        let cmp = b!(self.bld.build_call(memcmp, &[src_ptr.into(), odata.into(), olen.into()], "rep.cmp"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let is_match = b!(self.bld.build_int_compare(IntPredicate::EQ, cmp, self.ctx.i32_type().const_int(0, false), "rep.ism"));
        b!(self.bld.build_conditional_branch(is_match, copy_new_bb, nomatch_bb));

        // Copy new string into buf
        self.bld.position_at_end(copy_new_bb);
        let cur_buf_cn = b!(self.bld.build_load(self.ctx.ptr_type(AddressSpace::default()), buf_alloca, "rep.cb")).into_pointer_value();
        let dst_cn = unsafe { b!(self.bld.build_gep(i8t, cur_buf_cn, &[out_len], "rep.dst")) };
        b!(self.bld.build_call(memcpy, &[dst_cn.into(), ndata.into(), nlen.into()], ""));
        let new_si = b!(self.bld.build_int_add(si, olen, "rep.nsi"));
        let new_out = b!(self.bld.build_int_add(out_len, nlen, "rep.no"));
        phi_si.add_incoming(&[(&new_si, copy_new_bb)]);
        phi_out.add_incoming(&[(&new_out, copy_new_bb)]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        // No match — copy one byte
        self.bld.position_at_end(nomatch_bb);
        let cur_buf_nm = b!(self.bld.build_load(self.ctx.ptr_type(AddressSpace::default()), buf_alloca, "rep.cbnm")).into_pointer_value();
        let dst_nm = unsafe { b!(self.bld.build_gep(i8t, cur_buf_nm, &[out_len], "rep.dstnm")) };
        let src_byte = unsafe { b!(self.bld.build_gep(i8t, sdata, &[si], "rep.sb")) };
        let byte = b!(self.bld.build_load(i8t, src_byte, "rep.byte"));
        b!(self.bld.build_store(dst_nm, byte));
        let nm_si = b!(self.bld.build_int_add(si, i64t.const_int(1, false), "rep.nmsi"));
        let nm_out = b!(self.bld.build_int_add(out_len, i64t.const_int(1, false), "rep.nmo"));
        phi_si.add_incoming(&[(&nm_si, nomatch_bb)]);
        phi_out.add_incoming(&[(&nm_out, nomatch_bb)]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        // Done — build string from buf + out_len
        self.bld.position_at_end(done_bb);
        let final_buf = b!(self.bld.build_load(self.ctx.ptr_type(AddressSpace::default()), buf_alloca, "rep.fb")).into_pointer_value();
        let fits = b!(self.bld.build_int_compare(IntPredicate::ULE, out_len, i64t.const_int(23, false), "rep.fits"));
        let sso_bb = self.ctx.append_basic_block(fv, "rep.sso");
        let heap_bb = self.ctx.append_basic_block(fv, "rep.heap");
        let merge_bb = self.ctx.append_basic_block(fv, "rep.merge");
        b!(self.bld.build_conditional_branch(fits, sso_bb, heap_bb));

        self.bld.position_at_end(sso_bb);
        let sso_out = self.entry_alloca(st.into(), "rep.sso");
        b!(self.bld.build_store(sso_out, st.const_zero()));
        b!(self.bld.build_call(memcpy, &[sso_out.into(), final_buf.into(), out_len.into()], ""));
        let free = self.ensure_free();
        b!(self.bld.build_call(free, &[final_buf.into()], ""));
        let (sso_val, sso_exit) = self.build_sso_result(sso_out, out_len, merge_bb, "rep")?;

        self.bld.position_at_end(heap_bb);
        let heap_val = self.build_string(final_buf, out_len, out_len, "rep.hv")?;
        let heap_exit = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(st, "rep.v"));
        phi.add_incoming(&[(&sso_val, sso_exit), (&heap_val, heap_exit)]);
        Ok(phi.as_basic_value())
    }

    // ── split(delim) → Vec of String ───────────────────────────────────

    pub(crate) fn string_split(
        &mut self,
        s: BasicValueEnum<'ctx>,
        delim: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Create a Vec, scan for delimiters, push slices
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let sdata = self.string_data(s)?.into_pointer_value();
        let slen = self.string_len(s)?.into_int_value();
        let ddata = self.string_data(delim)?.into_pointer_value();
        let dlen = self.string_len(delim)?.into_int_value();
        let memcmp = self.ensure_memcmp();

        // Create empty vec
        let vec = self.compile_vec_new(&[])?;
        let vec_ptr = vec.into_pointer_value();

        // Store vec_ptr in alloca so push can modify it
        let cond_bb = self.ctx.append_basic_block(fv, "spl.cond");
        let match_bb = self.ctx.append_basic_block(fv, "spl.match");
        let push_bb = self.ctx.append_basic_block(fv, "spl.push");
        let skip_bb = self.ctx.append_basic_block(fv, "spl.skip");
        let done_bb = self.ctx.append_basic_block(fv, "spl.done");

        let entry_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(cond_bb);
        let phi_si = b!(self.bld.build_phi(i64t, "spl.si"));
        phi_si.add_incoming(&[(&i64t.const_int(0, false), entry_bb)]);
        let phi_start = b!(self.bld.build_phi(i64t, "spl.start"));
        phi_start.add_incoming(&[(&i64t.const_int(0, false), entry_bb)]);
        let si = phi_si.as_basic_value().into_int_value();
        let start = phi_start.as_basic_value().into_int_value();
        let at_end = b!(self.bld.build_int_compare(IntPredicate::SGE, si, slen, "spl.end"));
        b!(self.bld.build_conditional_branch(at_end, done_bb, match_bb));

        // Try matching delimiter at si
        self.bld.position_at_end(match_bb);
        let remaining = b!(self.bld.build_int_nsw_sub(slen, si, "spl.rem"));
        let can_match = b!(self.bld.build_int_compare(IntPredicate::SGE, remaining, dlen, "spl.cm"));
        let try_bb = self.ctx.append_basic_block(fv, "spl.try");
        b!(self.bld.build_conditional_branch(can_match, try_bb, skip_bb));

        self.bld.position_at_end(try_bb);
        let sp = unsafe { b!(self.bld.build_gep(i8t, sdata, &[si], "spl.sp")) };
        let cmp = b!(self.bld.build_call(memcmp, &[sp.into(), ddata.into(), dlen.into()], "spl.cmp"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let is_match = b!(self.bld.build_int_compare(IntPredicate::EQ, cmp, self.ctx.i32_type().const_int(0, false), "spl.ism"));
        b!(self.bld.build_conditional_branch(is_match, push_bb, skip_bb));

        // Push slice [start..si]
        self.bld.position_at_end(push_bb);
        let slice = self.string_slice(s, start.into(), si.into())?;
        // Build a temporary hir::Expr for push
        let elem_ty = crate::types::Type::String;
        let lty = self.llvm_ty(&elem_ty);
        let esz = self.type_store_size(lty);
        self.vec_push_raw(vec_ptr, slice, lty, esz)?;
        let new_si = b!(self.bld.build_int_add(si, dlen, "spl.nsi"));
        phi_si.add_incoming(&[(&new_si, self.bld.get_insert_block().unwrap())]);
        phi_start.add_incoming(&[(&new_si, self.bld.get_insert_block().unwrap())]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        // No match, advance
        self.bld.position_at_end(skip_bb);
        let next_si = b!(self.bld.build_int_add(si, i64t.const_int(1, false), "spl.ns"));
        phi_si.add_incoming(&[(&next_si, skip_bb)]);
        phi_start.add_incoming(&[(&start, skip_bb)]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        // Push final segment [start..slen]
        self.bld.position_at_end(done_bb);
        let final_slice = self.string_slice(s, start.into(), slen.into())?;
        self.vec_push_raw(vec_ptr, final_slice, lty, esz)?;

        Ok(vec_ptr.into())
    }

    /// Drop a String value — free heap buffer if not SSO inline and cap > 0.
    pub(crate) fn drop_string(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        let fv = self.cur_fn.unwrap();
        let st = self.string_type();
        let ptr = self.entry_alloca(st.into(), "ds.tmp");
        b!(self.bld.build_store(ptr, val));
        // Read tag byte at offset 23
        let tag_ptr = unsafe {
            b!(self.bld.build_gep(i8t, ptr, &[i64t.const_int(23, false)], "ds.tagp"))
        };
        let tag = b!(self.bld.build_load(i8t, tag_ptr, "ds.tag")).into_int_value();
        let masked = b!(self.bld.build_and(tag, i8t.const_int(0x80, false), "ds.hi"));
        let is_sso = b!(self.bld.build_int_compare(
            IntPredicate::NE, masked, i8t.const_int(0, false), "ds.issso"
        ));
        let heap_bb = self.ctx.append_basic_block(fv, "ds.heap");
        let done_bb = self.ctx.append_basic_block(fv, "ds.done");
        b!(self.bld.build_conditional_branch(is_sso, done_bb, heap_bb));
        // Heap case: only free if cap > 0 (cap == 0 means non-owned, e.g. global literal)
        self.bld.position_at_end(heap_bb);
        let cap_gep = b!(self.bld.build_struct_gep(st, ptr, 2, "ds.capg"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "ds.cap")).into_int_value();
        let has_buf = b!(self.bld.build_int_compare(
            IntPredicate::UGT, cap, i64t.const_int(0, false), "ds.owned"
        ));
        let free_bb = self.ctx.append_basic_block(fv, "ds.free");
        b!(self.bld.build_conditional_branch(has_buf, free_bb, done_bb));
        self.bld.position_at_end(free_bb);
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let heap_ptr_gep = b!(self.bld.build_struct_gep(st, ptr, 0, "ds.hptr"));
        let heap_ptr = b!(self.bld.build_load(ptr_ty, heap_ptr_gep, "ds.buf"));
        let free = self.ensure_free();
        b!(self.bld.build_call(free, &[heap_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(done_bb));
        self.bld.position_at_end(done_bb);
        Ok(())
    }
}
