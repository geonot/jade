use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;

use super::b;
use super::Compiler;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_fmt_float(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let x = self.compile_expr(&args[0])?.into_float_value();
        let decimals = self.compile_expr(&args[1])?.into_int_value();
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let snprintf = self.ensure_snprintf();

        let fmt = b!(self.bld.build_global_string_ptr("%.*f", "ff.fmt"));
        let null = ptr_ty.const_null();
        let dec_i32 = b!(self
            .bld
            .build_int_truncate(decimals, self.ctx.i32_type(), "dec32"));
        let len = b!(self.bld.build_call(
            snprintf,
            &[
                null.into(),
                i64t.const_int(0, false).into(),
                fmt.as_pointer_value().into(),
                dec_i32.into(),
                x.into()
            ],
            "ff.len"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        let len64 = b!(self.bld.build_int_s_extend(len, i64t, "ff.len64"));
        let size = b!(self
            .bld
            .build_int_nsw_add(len64, i64t.const_int(1, false), "ff.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "ff.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        b!(self.bld.build_call(
            snprintf,
            &[
                buf.into(),
                size.into(),
                fmt.as_pointer_value().into(),
                dec_i32.into(),
                x.into()
            ],
            ""
        ));
        self.build_string(buf, len64, size, "ff.s")
    }

    pub(crate) fn compile_fmt_snprintf(
        &mut self,
        fmt_str: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(&args[0])?.into_int_value();
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let snprintf = self.ensure_snprintf();
        let fmt = b!(self.bld.build_global_string_ptr(fmt_str, "fh.fmt"));
        let null = ptr_ty.const_null();
        let wide = if val.get_type().get_bit_width() < 64 {
            b!(self.bld.build_int_s_extend(val, i64t, "fw")).into()
        } else {
            val.into()
        };
        let len = b!(self.bld.build_call(
            snprintf,
            &[
                null.into(),
                i64t.const_int(0, false).into(),
                fmt.as_pointer_value().into(),
                wide
            ],
            "fh.len"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        let len64 = b!(self.bld.build_int_s_extend(len, i64t, "fh.len64"));
        let size = b!(self
            .bld
            .build_int_nsw_add(len64, i64t.const_int(1, false), "fh.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "fh.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        b!(self.bld.build_call(
            snprintf,
            &[buf.into(), size.into(), fmt.as_pointer_value().into(), wide],
            ""
        ));
        self.build_string(buf, len64, size, "fh.s")
    }

    pub(crate) fn compile_fmt_bin(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(&args[0])?.into_int_value();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let malloc = self.ensure_malloc();
        let buf = b!(self
            .bld
            .build_call(malloc, &[i64t.const_int(65, false).into()], "fb.buf"))
        .try_as_basic_value()
        .basic()
        .unwrap();
        let buf_ptr = buf.into_pointer_value();

        let fv = self.cur_fn.unwrap();
        let loop_bb = self.ctx.append_basic_block(fv, "fb.loop");
        let body_bb = self.ctx.append_basic_block(fv, "fb.body");
        let done_bb = self.ctx.append_basic_block(fv, "fb.done");

        let wide = if val.get_type().get_bit_width() < 64 {
            b!(self.bld.build_int_z_extend(val, i64t, "fb.w"))
        } else {
            val
        };

        let clz_name = "llvm.ctlz.i64";
        let clz = self.module.get_function(clz_name).unwrap_or_else(|| {
            let ft = i64t.fn_type(&[i64t.into(), self.ctx.bool_type().into()], false);
            self.module.add_function(clz_name, ft, None)
        });
        let lz = b!(self.bld.build_call(
            clz,
            &[wide.into(), self.ctx.bool_type().const_int(1, false).into()],
            "fb.lz"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        let raw_bits = b!(self
            .bld
            .build_int_nsw_sub(i64t.const_int(64, false), lz, "fb.nb"));
        let is_zero = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            wide,
            i64t.const_int(0, false),
            "fb.z"
        ));
        let nbits =
            b!(self
                .bld
                .build_select(is_zero, i64t.const_int(1, false), raw_bits, "fb.bits"))
            .into_int_value();

        let idx_ptr = self.entry_alloca(i64t.into(), "fb.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        let bit_ptr = self.entry_alloca(i64t.into(), "fb.bit");
        b!(self.bld.build_store(
            bit_ptr,
            b!(self
                .bld
                .build_int_nsw_sub(nbits, i64t.const_int(1, false), "fb.start"))
        ));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "fb.i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, nbits, "fb.cond"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let bit = b!(self.bld.build_load(i64t, bit_ptr, "fb.b")).into_int_value();
        let shifted = b!(self.bld.build_right_shift(wide, bit, false, "fb.sh"));
        let masked = b!(self
            .bld
            .build_and(shifted, i64t.const_int(1, false), "fb.m"));
        let ch = b!(self.bld.build_int_nsw_add(
            b!(self.bld.build_int_truncate(masked, i8t, "fb.trunc")),
            i8t.const_int(b'0' as u64, false),
            "fb.ch"
        ));
        let dest = unsafe { b!(self.bld.build_gep(i8t, buf_ptr, &[idx], "fb.p")) };
        b!(self.bld.build_store(dest, ch));
        let next_idx = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "fb.ni"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        let next_bit = b!(self
            .bld
            .build_int_nsw_sub(bit, i64t.const_int(1, false), "fb.nb"));
        b!(self.bld.build_store(bit_ptr, next_bit));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let end = unsafe { b!(self.bld.build_gep(i8t, buf_ptr, &[nbits], "fb.end")) };
        b!(self.bld.build_store(end, i8t.const_int(0, false)));
        self.build_string(
            buf,
            nbits,
            b!(self
                .bld
                .build_int_nsw_add(nbits, i64t.const_int(1, false), "fb.cap")),
            "fb.s",
        )
    }

    pub(crate) fn compile_time_monotonic(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let f64t = self.ctx.f64_type();
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
        let sec_f = b!(self.bld.build_signed_int_to_float(sec, f64t, "secf"));
        let nsec_f = b!(self.bld.build_signed_int_to_float(nsec, f64t, "nsecf"));
        let billion = f64t.const_float(1_000_000_000.0);
        let ns_part = b!(self.bld.build_float_div(nsec_f, billion, "ns"));
        Ok(b!(self.bld.build_float_add(sec_f, ns_part, "mono")).into())
    }

    pub(crate) fn compile_sleep_ms(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let ms = self.compile_expr(&args[0])?.into_int_value();
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
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    pub(crate) fn compile_file_exists(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let s = self.compile_expr(&args[0])?;
        let data = self.string_data(s)?;
        let i32t = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let access = self.module.get_function("access").unwrap_or_else(|| {
            self.module.add_function(
                "access",
                i32t.fn_type(&[ptr_ty.into(), i32t.into()], false),
                Some(Linkage::External),
            )
        });
        let result = b!(self.bld.build_call(
            access,
            &[data.into(), i32t.const_int(0, false).into()],
            "fex"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        let is_ok = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            result,
            i32t.const_int(0, false),
            "fex.ok"
        ));
        Ok(is_ok.into())
    }
}
