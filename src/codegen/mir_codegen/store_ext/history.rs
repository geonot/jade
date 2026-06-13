use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_store_history(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("history() requires sid argument".into());
        }
        let sid_val = self.val(args[0]).into_int_value();
        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();

        let ensure_fn_name = format!("__store_ensure_{store_name}");
        if let Some(ensure_fn) = self.module.get_function(&ensure_fn_name) {
            b!(self.bld.build_call(ensure_fn, &[], ""));
        } else {
            let ensure_fn = self.gen_store_ensure_open(&sd)?;
            b!(self.bld.build_call(ensure_fn, &[], ""));
        }

        let i64t = self.ctx.i64_type();
        let rec_size = self.store_record_size(&sd);

        let rec_name = format!("__store_{store_name}_rec");
        let rec_st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");

        let jinn_name = format!("__store_{store_name}");
        let jinn_st = self
            .module
            .get_struct_type(&jinn_name)
            .expect("ICE: struct type not declared");
        let jinn_size = self.type_store_size(jinn_st.into());

        let ver_fp = self.load_store_ver(store_name)?;
        let ver_count_fn = crate::codegen::fn_or_die(&self.module, "jinn_ver_count");
        let count = self
            .call_result(b!(self.bld.build_call(
                ver_count_fn,
                &[
                    ver_fp.into(),
                    sid_val.into(),
                    i64t.const_int(rec_size, false).into()
                ],
                "hist.cnt"
            )))
            .into_int_value();

        let one = i64t.const_int(1, false);
        let raw_total =
            b!(self
                .bld
                .build_int_mul(count, i64t.const_int(rec_size, false), "hist.raw_total"));
        let raw_alloc = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                raw_total,
                i64t.const_int(0, false),
                "hist.raw_isz"
            )),
            one,
            raw_total,
            "hist.raw_alloc"
        ))
        .into_int_value();
        let malloc_fn = self.ensure_malloc();
        let raw_buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[raw_alloc.into()],
                "hist.raw"
            )))
            .into_pointer_value();

        let ver_hist_fn = crate::codegen::fn_or_die(&self.module, "jinn_ver_history");
        let written = self
            .call_result(b!(self.bld.build_call(
                ver_hist_fn,
                &[
                    ver_fp.into(),
                    sid_val.into(),
                    raw_buf.into(),
                    i64t.const_int(rec_size, false).into(),
                    count.into()
                ],
                "hist.n"
            )))
            .into_int_value();

        let jinn_total =
            b!(self
                .bld
                .build_int_mul(written, i64t.const_int(jinn_size, false), "hist.jinn_total"));
        let jinn_alloc = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                jinn_total,
                i64t.const_int(0, false),
                "hist.jinn_isz"
            )),
            one,
            jinn_total,
            "hist.jinn_alloc"
        ))
        .into_int_value();
        let jinn_buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[jinn_alloc.into()],
                "hist.jn"
            )))
            .into_pointer_value();

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let idx_ptr = self.entry_alloca(i64t.into(), "hist.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "hist.loop");
        let body_bb = self.ctx.append_basic_block(fv, "hist.body");
        let done_bb = self.ctx.append_basic_block(fv, "hist.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "hist.i")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, idx, written, "hist.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let raw_off = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "hist.roff"));
        let raw_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), raw_buf, &[raw_off], "hist.rptr"))
        };
        let jinn_val = self.load_store_record_as_jinn(rec_st, raw_ptr, &sd)?;
        let jinn_off = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(jinn_size, false), "hist.joff"));
        let jinn_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), jinn_buf, &[jinn_off], "hist.jptr"))
        };
        b!(self.bld.build_store(jinn_ptr, jinn_val));
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "hist.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[raw_buf.into()], ""));

        let header_ty = self.vec_header_type();
        let header_size = self.type_store_size(header_ty.into());
        let header_buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[i64t.const_int(header_size, false).into()],
                "hist.hdr"
            )))
            .into_pointer_value();
        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_buf, 0, "hist.hdr.ptr"));
        b!(self.bld.build_store(ptr_gep, jinn_buf));
        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_buf, 1, "hist.hdr.len"));
        b!(self.bld.build_store(len_gep, written));
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_buf, 2, "hist.hdr.cap"));
        b!(self.bld.build_store(cap_gep, written));

        Ok(header_buf.into())
    }
}
