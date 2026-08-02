use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_time_monotonic(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let clock_gettime = self
            .module
            .get_function("clock_gettime")
            .unwrap_or_else(|| {
                self.module.add_function(
                    "clock_gettime",
                    i32t.fn_type(&[i32t.into(), ptr_ty.into()], false),
                    Some(Linkage::External),
                )
            });

        let ts_ty = self.ctx.struct_type(&[i64t.into(), i64t.into()], false);
        let ts = self.entry_alloca(ts_ty.into(), "ts");
        b!(self.bld.build_call(
            clock_gettime,
            &[i32t.const_int(1, false).into(), ts.into()],
            ""
        ));
        let sec = b!(self.bld.build_load(
            i64t,
            b!(self.bld.build_struct_gep(ts_ty, ts, 0, "ts.sec")),
            "sec"
        ))
        .into_int_value();
        let nsec = b!(self.bld.build_load(
            i64t,
            b!(self.bld.build_struct_gep(ts_ty, ts, 1, "ts.nsec")),
            "nsec"
        ))
        .into_int_value();
        let billion = i64t.const_int(1_000_000_000, false);
        let sec_ns = b!(self.bld.build_int_nsw_mul(sec, billion, "sec_ns"));
        Ok(b!(self.bld.build_int_nsw_add(sec_ns, nsec, "mono")).into())
    }

    pub(crate) fn emit_sleep_ms_val(
        &mut self,
        ms: inkwell::values::IntValue<'ctx>,
    ) -> Result<(), String> {
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let nanosleep = self.module.get_function("nanosleep").unwrap_or_else(|| {
            self.module.add_function(
                "nanosleep",
                i32t.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
                Some(Linkage::External),
            )
        });
        let ts_ty = self.ctx.struct_type(&[i64t.into(), i64t.into()], false);
        let ts = self.entry_alloca(ts_ty.into(), "sl.ts");
        let sec = b!(self
            .bld
            .build_int_signed_div(ms, i64t.const_int(1000, false), "sl.sec"));
        let rem = b!(self
            .bld
            .build_int_signed_rem(ms, i64t.const_int(1000, false), "sl.rem"));
        let nsec = b!(self
            .bld
            .build_int_nsw_mul(rem, i64t.const_int(1_000_000, false), "sl.nsec"));
        let sec_p = b!(self.bld.build_struct_gep(ts_ty, ts, 0, "sl.secp"));
        b!(self.bld.build_store(sec_p, sec));
        let nsec_p = b!(self.bld.build_struct_gep(ts_ty, ts, 1, "sl.nsecp"));
        b!(self.bld.build_store(nsec_p, nsec));
        b!(self
            .bld
            .build_call(nanosleep, &[ts.into(), ptr_ty.const_null().into()], ""));
        Ok(())
    }
}
