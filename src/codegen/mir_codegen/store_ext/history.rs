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
        let (_sd, _st, rec_size, _fp) = self.setup_store_access(store_name)?;
        let i64t = self.ctx.i64_type();

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
        Ok(count.into())
    }
}
