use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn store_read_count(
        &mut self,
        fp: inkwell::values::PointerValue<'ctx>,
        rec_size: u64,
        store_name: &str,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        if self
            .module
            .get_function("jinn_store_read_count_checked")
            .is_none()
        {
            let ft = i64t.fn_type(&[ptr_ty.into(), i64t.into(), ptr_ty.into()], false);
            self.module.add_function(
                "jinn_store_read_count_checked",
                ft,
                Some(inkwell::module::Linkage::External),
            );
        }
        let path = format!("{store_name}.store\0");
        let path_str = b!(self.bld.build_global_string_ptr(&path, "rc.path"));
        let f = crate::codegen::fn_or_die(&self.module, "jinn_store_read_count_checked");
        let count = self.call_result(b!(self.bld.build_call(
            f,
            &[
                fp.into(),
                i64t.const_int(rec_size, false).into(),
                path_str.as_pointer_value().into(),
            ],
            "count"
        )));
        Ok(count.into_int_value())
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
