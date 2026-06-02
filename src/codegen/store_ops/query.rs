use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn store_read_count(
        &mut self,
        fp: inkwell::values::PointerValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));
        let count_buf = self.entry_alloca(i64t.into(), "sc.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");
        b!(self.bld.build_call(
            fread_fn,
            &[
                count_buf.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));
        Ok(b!(self.bld.build_load(i64t, count_buf, "count")).into_int_value())
    }

    pub(crate) fn store_load_records(
        &mut self,
        fp: inkwell::values::PointerValue<'ctx>,
        count: inkwell::values::IntValue<'ctx>,
        rec_size: u64,
    ) -> Result<inkwell::values::PointerValue<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(HEADER_SIZE, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));
        let total = b!(self
            .bld
            .build_int_mul(count, i64t.const_int(rec_size, false), "sl.total"));
        let one = i64t.const_int(1, false);
        let alloc_size = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(
                IntPredicate::EQ,
                total,
                i64t.const_int(0, false),
                "sl.isz"
            )),
            one,
            total,
            "sl.alloc"
        ))
        .into_int_value();
        let malloc_fn = self.ensure_malloc();
        let buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[alloc_size.into()],
                "sl.buf"
            )))
            .into_pointer_value();
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");
        b!(self.bld.build_call(
            fread_fn,
            &[
                buf.into(),
                i64t.const_int(rec_size, false).into(),
                count.into(),
                fp.into(),
            ],
            ""
        ));
        Ok(buf)
    }

}
