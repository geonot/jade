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
            _ => Err(format!("no method '{m}' on String")),
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

        // Branch: SSO if total <= 23, else heap
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

        // SSO path
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
        let tag_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, sso_out, &[i64t.const_int(23, false)], "cat.tagp"))
        };
        let total_i8 = b!(self
            .bld
            .build_int_truncate(total, i8t, "cat.l8"));
        let tag = b!(self
            .bld
            .build_or(total_i8, i8t.const_int(0x80, false), "cat.tag"));
        b!(self.bld.build_store(tag_ptr, tag));
        let sso_val = b!(self.bld.build_load(st, sso_out, "cat.ssov"));
        let sso_exit = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));

        // Heap path
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

        // Merge
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

        // Branch: SSO if new_len <= 23, else heap
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

        // SSO path
        self.bld.position_at_end(sso_bb);
        let sso_out = self.entry_alloca(st.into(), "sl.sso");
        b!(self.bld.build_store(sso_out, st.const_zero()));
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[sso_out.into(), src.into(), new_len.into()], ""));
        let tag_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, sso_out, &[i64t.const_int(23, false)], "sl.tagp"))
        };
        let len_i8 = b!(self
            .bld
            .build_int_truncate(new_len, i8t, "sl.l8"));
        let tag = b!(self
            .bld
            .build_or(len_i8, i8t.const_int(0x80, false), "sl.tag"));
        b!(self.bld.build_store(tag_ptr, tag));
        let sso_val = b!(self.bld.build_load(st, sso_out, "sl.ssov"));
        let sso_exit = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));

        // Heap path
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

        // Merge
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
        let fv = self.cur_fn.unwrap();

        let ptr = self.entry_alloca(st.into(), "s.tmp");
        b!(self.bld.build_store(ptr, val));

        // SSO tag at byte 23: bit 7 = 1 means inline
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

        let sso_bb = self.ctx.append_basic_block(fv, "sso.len");
        let heap_bb = self.ctx.append_basic_block(fv, "heap.len");
        let merge_bb = self.ctx.append_basic_block(fv, "merge.len");
        b!(self
            .bld
            .build_conditional_branch(is_sso, sso_bb, heap_bb));

        // SSO: length = tag & 0x7F
        self.bld.position_at_end(sso_bb);
        let sso_len_i8 = b!(self
            .bld
            .build_and(tag, i8t.const_int(0x7F, false), "sso.l8"));
        let sso_len = b!(self
            .bld
            .build_int_z_extend(sso_len_i8, i64t, "sso.len"));
        b!(self.bld.build_unconditional_branch(merge_bb));

        // Heap: length = field 1
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
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let fv = self.cur_fn.unwrap();

        let ptr = self.entry_alloca(st.into(), "s.tmp");
        b!(self.bld.build_store(ptr, val));

        // SSO tag at byte 23: bit 7 = 1 means inline
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

        let sso_bb = self.ctx.append_basic_block(fv, "sso.data");
        let heap_bb = self.ctx.append_basic_block(fv, "heap.data");
        let merge_bb = self.ctx.append_basic_block(fv, "merge.data");
        b!(self
            .bld
            .build_conditional_branch(is_sso, sso_bb, heap_bb));

        // SSO: data starts at byte 0 of the struct alloca
        self.bld.position_at_end(sso_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        // Heap: data = field 0 (pointer)
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
}
