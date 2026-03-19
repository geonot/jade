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
        let llen = self.string_len(l)?.into_int_value();
        let rlen = self.string_len(r)?.into_int_value();
        let total = b!(self.bld.build_int_add(llen, rlen, "total"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[total.into()], "buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        let ldata = self.string_data(l)?;
        let rdata = self.string_data(r)?;
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[buf.into(), ldata.into(), llen.into()], ""));
        let dst = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf.into_pointer_value(), &[llen], "dst"))
        };
        b!(self
            .bld
            .build_call(memcpy, &[dst.into(), rdata.into(), rlen.into()], ""));
        self.build_string(buf, total, total, "cat")
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
        let data = self.string_data(s)?.into_pointer_value();
        let si = start.into_int_value();
        let ei = end.into_int_value();
        let new_len = b!(self.bld.build_int_nsw_sub(ei, si, "sl.len"));
        let src = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), data, &[si], "sl.src"))
        };
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[new_len.into()], "sl.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[buf.into(), src.into(), new_len.into()], ""));
        self.build_string(buf, new_len, new_len, "sl.val")
    }

    pub(crate) fn string_len(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let st = self.string_type();
        let ptr = self.entry_alloca(st.into(), "s.tmp");
        b!(self.bld.build_store(ptr, val));
        let lp = b!(self.bld.build_struct_gep(st, ptr, 1, "s.len"));
        Ok(b!(self.bld.build_load(self.ctx.i64_type(), lp, "len")))
    }

    pub(crate) fn string_data(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let st = self.string_type();
        let ptr = self.entry_alloca(st.into(), "s.tmp");
        b!(self.bld.build_store(ptr, val));
        let dp = b!(self.bld.build_struct_gep(st, ptr, 0, "s.data"));
        Ok(b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            dp,
            "data"
        )))
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
