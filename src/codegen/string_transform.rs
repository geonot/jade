use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
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

        let nz = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            nlen,
            i64t.const_int(0, false),
            "sf.nz"
        ));
        let pre_check_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_conditional_branch(nz, found_bb, check_bb));

        self.bld.position_at_end(check_bb);
        let ok = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, hlen, nlen, "sf.ok"));
        b!(self.bld.build_conditional_branch(ok, loop_bb, nf_bb));

        self.bld.position_at_end(loop_bb);
        let phi_i = b!(self.bld.build_phi(i64t, "sf.i"));
        phi_i.add_incoming(&[(&i64t.const_int(0, false), check_bb)]);
        let i = phi_i.as_basic_value().into_int_value();
        let ptr = unsafe { b!(self.bld.build_gep(i8t, hdata, &[i], "sf.p")) };
        let cmp =
            b!(self
                .bld
                .build_call(memcmp, &[ptr.into(), ndata.into(), nlen.into()], "sf.cmp"))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let eq = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            cmp,
            self.ctx.i32_type().const_int(0, false),
            "sf.eq"
        ));
        let cont_bb = self.ctx.append_basic_block(fv, "sf.cont");
        b!(self.bld.build_conditional_branch(eq, found_bb, cont_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "sf.n"));
        let limit = b!(self.bld.build_int_nsw_sub(hlen, nlen, "sf.lim"));
        let lim1 = b!(self
            .bld
            .build_int_add(limit, i64t.const_int(1, false), "sf.l1"));
        let done = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, next, lim1, "sf.done"));
        phi_i.add_incoming(&[(&next, cont_bb)]);
        b!(self.bld.build_conditional_branch(done, nf_bb, loop_bb));

        self.bld.position_at_end(found_bb);
        let found_phi = b!(self.bld.build_phi(i64t, "sf.fi"));
        found_phi.add_incoming(&[(&i64t.const_int(0, false), pre_check_bb), (&i, loop_bb)]);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(nf_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(i64t, "sf.v"));
        phi.add_incoming(&[
            (&found_phi.as_basic_value(), found_bb),
            (&i64t.const_int(u64::MAX, true), nf_bb),
        ]);
        Ok(phi.as_basic_value())
    }

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

        let left_idx = if left {
            let loop_bb = self.ctx.append_basic_block(fv, "tl.loop");
            let done_bb = self.ctx.append_basic_block(fv, "tl.done");
            let entry_bb = self.bld.get_insert_block().unwrap();
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(loop_bb);
            let phi = b!(self.bld.build_phi(i64t, "tl.i"));
            phi.add_incoming(&[(&i64t.const_int(0, false), entry_bb)]);
            let i = phi.as_basic_value().into_int_value();
            let at_end = b!(self
                .bld
                .build_int_compare(IntPredicate::SGE, i, len, "tl.end"));
            let check_bb = self.ctx.append_basic_block(fv, "tl.chk");
            b!(self.bld.build_conditional_branch(at_end, done_bb, check_bb));

            self.bld.position_at_end(check_bb);
            let bp = unsafe { b!(self.bld.build_gep(i8t, data, &[i], "tl.bp")) };
            let byte = b!(self.bld.build_load(i8t, bp, "tl.b")).into_int_value();
            let is_space = self.is_whitespace(byte)?;
            let next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "tl.n"));
            phi.add_incoming(&[(&next, check_bb)]);
            b!(self
                .bld
                .build_conditional_branch(is_space, loop_bb, done_bb));

            self.bld.position_at_end(done_bb);
            let result = b!(self.bld.build_phi(i64t, "tl.v"));
            result.add_incoming(&[(&i, loop_bb), (&i, check_bb)]);
            result.as_basic_value().into_int_value()
        } else {
            i64t.const_int(0, false)
        };

        let right_idx = if right {
            let loop_bb = self.ctx.append_basic_block(fv, "tr.loop");
            let done_bb = self.ctx.append_basic_block(fv, "tr.done");
            let entry_bb = self.bld.get_insert_block().unwrap();
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(loop_bb);
            let phi = b!(self.bld.build_phi(i64t, "tr.i"));
            phi.add_incoming(&[(&len, entry_bb)]);
            let i = phi.as_basic_value().into_int_value();
            let at_start = b!(self
                .bld
                .build_int_compare(IntPredicate::SLE, i, left_idx, "tr.end"));
            let check_bb = self.ctx.append_basic_block(fv, "tr.chk");
            b!(self
                .bld
                .build_conditional_branch(at_start, done_bb, check_bb));

            self.bld.position_at_end(check_bb);
            let prev = b!(self
                .bld
                .build_int_nsw_sub(i, i64t.const_int(1, false), "tr.p"));
            let bp = unsafe { b!(self.bld.build_gep(i8t, data, &[prev], "tr.bp")) };
            let byte = b!(self.bld.build_load(i8t, bp, "tr.b")).into_int_value();
            let is_space = self.is_whitespace(byte)?;
            phi.add_incoming(&[(&prev, check_bb)]);
            b!(self
                .bld
                .build_conditional_branch(is_space, loop_bb, done_bb));

            self.bld.position_at_end(done_bb);
            let result = b!(self.bld.build_phi(i64t, "tr.v"));
            result.add_incoming(&[(&i, loop_bb), (&i, check_bb)]);
            result.as_basic_value().into_int_value()
        } else {
            len
        };

        self.string_slice(s, left_idx.into(), right_idx.into())
    }

    pub(crate) fn is_whitespace(
        &mut self,
        byte: inkwell::values::IntValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let i8t = self.ctx.i8_type();
        let sp = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            byte,
            i8t.const_int(b' ' as u64, false),
            "ws.sp"
        ));
        let tab = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            byte,
            i8t.const_int(b'\t' as u64, false),
            "ws.t"
        ));
        let nl = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            byte,
            i8t.const_int(b'\n' as u64, false),
            "ws.n"
        ));
        let cr = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            byte,
            i8t.const_int(b'\r' as u64, false),
            "ws.r"
        ));
        let a = b!(self.bld.build_or(sp, tab, "ws.a"));
        let b2 = b!(self.bld.build_or(nl, cr, "ws.b"));
        Ok(b!(self.bld.build_or(a, b2, "ws.v")))
    }

    pub(crate) fn string_case(
        &mut self,
        s: BasicValueEnum<'ctx>,
        upper: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let data = self.string_data(s)?.into_pointer_value();
        let len = self.string_len(s)?.into_int_value();

        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[len.into()], "sc.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();

        let loop_bb = self.ctx.append_basic_block(fv, "sc.loop");
        let body_bb = self.ctx.append_basic_block(fv, "sc.body");
        let done_bb = self.ctx.append_basic_block(fv, "sc.done");
        let entry_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let phi_i = b!(self.bld.build_phi(i64t, "sc.i"));
        phi_i.add_incoming(&[(&i64t.const_int(0, false), entry_bb)]);
        let i = phi_i.as_basic_value().into_int_value();
        let at_end = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, i, len, "sc.end"));
        b!(self.bld.build_conditional_branch(at_end, done_bb, body_bb));

        self.bld.position_at_end(body_bb);
        let src_p = unsafe { b!(self.bld.build_gep(i8t, data, &[i], "sc.sp")) };
        let byte = b!(self.bld.build_load(i8t, src_p, "sc.b")).into_int_value();

        let (lo, hi) = if upper { (b'a', b'z') } else { (b'A', b'Z') };
        let in_range_lo = b!(self.bld.build_int_compare(
            IntPredicate::UGE,
            byte,
            i8t.const_int(lo as u64, false),
            "sc.lo"
        ));
        let in_range_hi = b!(self.bld.build_int_compare(
            IntPredicate::ULE,
            byte,
            i8t.const_int(hi as u64, false),
            "sc.hi"
        ));
        let in_range = b!(self.bld.build_and(in_range_lo, in_range_hi, "sc.ir"));

        let diff = i8t.const_int((b'a' - b'A') as u64, false);
        let converted = if upper {
            b!(self.bld.build_int_nsw_sub(byte, diff, "sc.cv"))
        } else {
            b!(self.bld.build_int_add(byte, diff, "sc.cv"))
        };
        let out_byte =
            b!(self.bld.build_select(in_range, converted, byte, "sc.ob")).into_int_value();

        let dst_p = unsafe { b!(self.bld.build_gep(i8t, buf, &[i], "sc.dp")) };
        b!(self.bld.build_store(dst_p, out_byte));
        let next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "sc.n"));
        phi_i.add_incoming(&[(&next, body_bb)]);
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        self.finalize_string_sso(buf, len, true, "sc")
    }

    pub(crate) fn string_replace(
        &mut self,
        s: BasicValueEnum<'ctx>,
        old: BasicValueEnum<'ctx>,
        new: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let sdata = self.string_data(s)?.into_pointer_value();
        let slen = self.string_len(s)?.into_int_value();
        let odata = self.string_data(old)?.into_pointer_value();
        let olen = self.string_len(old)?.into_int_value();
        let ndata = self.string_data(new)?.into_pointer_value();
        let nlen = self.string_len(new)?.into_int_value();
        let memcmp = self.ensure_memcmp();
        let memcpy = self.ensure_memcpy();
        let malloc = self.ensure_malloc();

        let init_cap = b!(self
            .bld
            .build_int_nsw_mul(slen, i64t.const_int(2, false), "rep.ic"));
        let init_cap_min = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(
                IntPredicate::SGT,
                init_cap,
                i64t.const_int(64, false),
                "rep.cmp"
            )),
            init_cap,
            i64t.const_int(64, false),
            "rep.cap"
        ))
        .into_int_value();
        let buf = b!(self
            .bld
            .build_call(malloc, &[init_cap_min.into()], "rep.buf"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        let cond_bb = self.ctx.append_basic_block(fv, "rep.cond");
        let match_bb = self.ctx.append_basic_block(fv, "rep.match");
        let nomatch_bb = self.ctx.append_basic_block(fv, "rep.nm");
        let copy_new_bb = self.ctx.append_basic_block(fv, "rep.copy");
        let done_bb = self.ctx.append_basic_block(fv, "rep.done");

        let buf_alloca =
            self.entry_alloca(self.ctx.ptr_type(AddressSpace::default()).into(), "rep.ba");
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
        let at_end = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, si, slen, "rep.end"));
        b!(self.bld.build_conditional_branch(at_end, done_bb, match_bb));

        self.bld.position_at_end(match_bb);
        let remaining = b!(self.bld.build_int_nsw_sub(slen, si, "rep.rem"));
        let can_match =
            b!(self
                .bld
                .build_int_compare(IntPredicate::SGE, remaining, olen, "rep.cm"));
        let try_bb = self.ctx.append_basic_block(fv, "rep.try");
        b!(self
            .bld
            .build_conditional_branch(can_match, try_bb, nomatch_bb));

        self.bld.position_at_end(try_bb);
        let src_ptr = unsafe { b!(self.bld.build_gep(i8t, sdata, &[si], "rep.sp")) };
        let cmp = b!(self.bld.build_call(
            memcmp,
            &[src_ptr.into(), odata.into(), olen.into()],
            "rep.cmp"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        let is_match = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            cmp,
            self.ctx.i32_type().const_int(0, false),
            "rep.ism"
        ));
        b!(self
            .bld
            .build_conditional_branch(is_match, copy_new_bb, nomatch_bb));

        self.bld.position_at_end(copy_new_bb);
        let cur_buf_cn = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            buf_alloca,
            "rep.cb"
        ))
        .into_pointer_value();
        let dst_cn = unsafe { b!(self.bld.build_gep(i8t, cur_buf_cn, &[out_len], "rep.dst")) };
        b!(self
            .bld
            .build_call(memcpy, &[dst_cn.into(), ndata.into(), nlen.into()], ""));
        let new_si = b!(self.bld.build_int_add(si, olen, "rep.nsi"));
        let new_out = b!(self.bld.build_int_add(out_len, nlen, "rep.no"));
        phi_si.add_incoming(&[(&new_si, copy_new_bb)]);
        phi_out.add_incoming(&[(&new_out, copy_new_bb)]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(nomatch_bb);
        let cur_buf_nm = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            buf_alloca,
            "rep.cbnm"
        ))
        .into_pointer_value();
        let dst_nm = unsafe { b!(self.bld.build_gep(i8t, cur_buf_nm, &[out_len], "rep.dstnm")) };
        let src_byte = unsafe { b!(self.bld.build_gep(i8t, sdata, &[si], "rep.sb")) };
        let byte = b!(self.bld.build_load(i8t, src_byte, "rep.byte"));
        b!(self.bld.build_store(dst_nm, byte));
        let nm_si = b!(self
            .bld
            .build_int_add(si, i64t.const_int(1, false), "rep.nmsi"));
        let nm_out = b!(self
            .bld
            .build_int_add(out_len, i64t.const_int(1, false), "rep.nmo"));
        phi_si.add_incoming(&[(&nm_si, nomatch_bb)]);
        phi_out.add_incoming(&[(&nm_out, nomatch_bb)]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(done_bb);
        let final_buf = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            buf_alloca,
            "rep.fb"
        ))
        .into_pointer_value();
        self.finalize_string_sso(final_buf, out_len, true, "rep")
    }

    pub(crate) fn string_split(
        &mut self,
        s: BasicValueEnum<'ctx>,
        delim: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let sdata = self.string_data(s)?.into_pointer_value();
        let slen = self.string_len(s)?.into_int_value();
        let ddata = self.string_data(delim)?.into_pointer_value();
        let dlen = self.string_len(delim)?.into_int_value();
        let memcmp = self.ensure_memcmp();

        let vec = self.compile_vec_new(&[])?;
        let vec_ptr = vec.into_pointer_value();

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
        let at_end = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, si, slen, "spl.end"));
        b!(self.bld.build_conditional_branch(at_end, done_bb, match_bb));

        self.bld.position_at_end(match_bb);
        let remaining = b!(self.bld.build_int_nsw_sub(slen, si, "spl.rem"));
        let can_match =
            b!(self
                .bld
                .build_int_compare(IntPredicate::SGE, remaining, dlen, "spl.cm"));
        let try_bb = self.ctx.append_basic_block(fv, "spl.try");
        b!(self
            .bld
            .build_conditional_branch(can_match, try_bb, skip_bb));

        self.bld.position_at_end(try_bb);
        let sp = unsafe { b!(self.bld.build_gep(i8t, sdata, &[si], "spl.sp")) };
        let cmp =
            b!(self
                .bld
                .build_call(memcmp, &[sp.into(), ddata.into(), dlen.into()], "spl.cmp"))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let is_match = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            cmp,
            self.ctx.i32_type().const_int(0, false),
            "spl.ism"
        ));
        b!(self
            .bld
            .build_conditional_branch(is_match, push_bb, skip_bb));

        self.bld.position_at_end(push_bb);
        let slice = self.string_slice(s, start.into(), si.into())?;
        let elem_ty = crate::types::Type::String;
        let lty = self.llvm_ty(&elem_ty);
        let esz = self.type_store_size(lty);
        self.vec_push_raw(vec_ptr, slice, lty, esz)?;
        let new_si = b!(self.bld.build_int_add(si, dlen, "spl.nsi"));
        phi_si.add_incoming(&[(&new_si, self.bld.get_insert_block().unwrap())]);
        phi_start.add_incoming(&[(&new_si, self.bld.get_insert_block().unwrap())]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(skip_bb);
        let next_si = b!(self
            .bld
            .build_int_add(si, i64t.const_int(1, false), "spl.ns"));
        phi_si.add_incoming(&[(&next_si, skip_bb)]);
        phi_start.add_incoming(&[(&start, skip_bb)]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(done_bb);
        let final_slice = self.string_slice(s, start.into(), slen.into())?;
        self.vec_push_raw(vec_ptr, final_slice, lty, esz)?;

        Ok(vec_ptr.into())
    }

    pub(crate) fn drop_string(&mut self, val: BasicValueEnum<'ctx>) -> Result<(), String> {
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        let fv = self.cur_fn.unwrap();
        let st = self.string_type();
        let ptr = self.entry_alloca(st.into(), "ds.tmp");
        b!(self.bld.build_store(ptr, val));
        let tag_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, ptr, &[i64t.const_int(23, false)], "ds.tagp"))
        };
        let tag = b!(self.bld.build_load(i8t, tag_ptr, "ds.tag")).into_int_value();
        let masked = b!(self.bld.build_and(tag, i8t.const_int(0x80, false), "ds.hi"));
        let is_sso = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            masked,
            i8t.const_int(0, false),
            "ds.issso"
        ));
        let heap_bb = self.ctx.append_basic_block(fv, "ds.heap");
        let done_bb = self.ctx.append_basic_block(fv, "ds.done");
        b!(self.bld.build_conditional_branch(is_sso, done_bb, heap_bb));
        self.bld.position_at_end(heap_bb);
        let cap_gep = b!(self.bld.build_struct_gep(st, ptr, 2, "ds.capg"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "ds.cap")).into_int_value();
        let has_buf = b!(self.bld.build_int_compare(
            IntPredicate::UGT,
            cap,
            i64t.const_int(0, false),
            "ds.owned"
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
