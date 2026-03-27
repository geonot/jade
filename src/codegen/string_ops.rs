use inkwell::IntPredicate;
use inkwell::values::BasicValueEnum;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
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

        let len_eq = b!(self
            .bld
            .build_int_compare(IntPredicate::EQ, llen, rlen, "seq.leq"));
        let cmp_bb = self.ctx.append_basic_block(fv, "seq.cmp");
        let neq_bb = self.ctx.append_basic_block(fv, "seq.neq");
        let merge_bb = self.ctx.append_basic_block(fv, "seq.merge");
        b!(self.bld.build_conditional_branch(len_eq, cmp_bb, neq_bb));

        self.bld.position_at_end(cmp_bb);
        let cmp = b!(self.bld.build_call(
            memcmp,
            &[ldata.into(), rdata.into(), llen.into()],
            "seq.cmp"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        let eq = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            cmp,
            self.ctx.i32_type().const_int(0, false),
            "seq.eq"
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
        b!(self.bld.build_conditional_branch(fits, sso_bb, heap_bb));

        self.bld.position_at_end(sso_bb);
        let sso_out = self.entry_alloca(st.into(), "cat.sso");
        b!(self.bld.build_store(sso_out, st.const_zero()));
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[sso_out.into(), ldata.into(), llen.into()], ""));
        let sso_dst = unsafe { b!(self.bld.build_gep(i8t, sso_out, &[llen], "cat.dst")) };
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
        let idx = self.string_find(haystack, needle)?.into_int_value();
        let i64t = self.ctx.i64_type();
        let found = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            idx,
            i64t.const_int(u64::MAX, true),
            "sc.res"
        ));
        Ok(found.into())
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

        let data = self.string_data(s)?.into_pointer_value();
        let si = start.into_int_value();
        let ei = end.into_int_value();
        let new_len = b!(self.bld.build_int_nsw_sub(ei, si, "sl.len"));
        let src = unsafe { b!(self.bld.build_gep(i8t, data, &[si], "sl.src")) };

        self.finalize_string_sso(src, new_len, false, "sl")
    }
}
