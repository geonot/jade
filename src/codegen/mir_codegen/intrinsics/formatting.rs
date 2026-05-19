use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn try_handle_bit_builtin(
        &mut self,
        name: &str,
        args: &[mir::ValueId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        if args.is_empty() {
            return Ok(None);
        }
        let val = self.val(args[0]).into_int_value();
        let bw = val.get_type().get_bit_width();
        let it = val.get_type();
        match name {
            "Bswap" => {
                let llvm_name = format!("llvm.bswap.i{bw}");
                let ft = it.fn_type(&[it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r = b!(self.bld.build_call(f, &[val.into()], "bswap"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                Ok(Some(r))
            }
            "Popcount" => {
                let llvm_name = format!("llvm.ctpop.i{bw}");
                let ft = it.fn_type(&[it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r = b!(self.bld.build_call(f, &[val.into()], "popcount"))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void");
                Ok(Some(r))
            }
            "Clz" => {
                let llvm_name = format!("llvm.ctlz.i{bw}");
                let false_val = self.ctx.bool_type().const_int(0, false);
                let ft = it.fn_type(&[it.into(), self.ctx.bool_type().into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r = b!(self
                    .bld
                    .build_call(f, &[val.into(), false_val.into()], "clz"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void");
                Ok(Some(r))
            }
            "Ctz" => {
                let llvm_name = format!("llvm.cttz.i{bw}");
                let false_val = self.ctx.bool_type().const_int(0, false);
                let ft = it.fn_type(&[it.into(), self.ctx.bool_type().into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r = b!(self
                    .bld
                    .build_call(f, &[val.into(), false_val.into()], "ctz"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void");
                Ok(Some(r))
            }
            "RotateLeft" => {
                if args.len() < 2 {
                    return Ok(None);
                }
                let amt = self.val(args[1]).into_int_value();
                let llvm_name = format!("llvm.fshl.i{bw}");
                let ft = it.fn_type(&[it.into(), it.into(), it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r = b!(self
                    .bld
                    .build_call(f, &[val.into(), val.into(), amt.into()], "rotl"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void");
                Ok(Some(r))
            }
            "RotateRight" => {
                if args.len() < 2 {
                    return Ok(None);
                }
                let amt = self.val(args[1]).into_int_value();
                let llvm_name = format!("llvm.fshr.i{bw}");
                let ft = it.fn_type(&[it.into(), it.into(), it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                let r = b!(self
                    .bld
                    .build_call(f, &[val.into(), val.into(), amt.into()], "rotr"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void");
                Ok(Some(r))
            }
            _ => Ok(None),
        }
    }

    pub(in crate::codegen) fn emit_to_string(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            Type::String => Ok(val),
            Type::I64 | Type::I32 | Type::I16 | Type::I8 => self.int_to_string(val, false),
            Type::U64 | Type::U32 | Type::U16 | Type::U8 => self.int_to_string(val, true),
            Type::F64 | Type::F32 => self.float_to_string(val),
            Type::Bool => self.bool_to_string(val),
            Type::Struct(name, _) => {
                let fn_name = format!("{name}_display");
                if let Some((fv, _, _)) = self.fns.get(&fn_name).cloned() {
                    let first_param_is_ptr = fv
                        .get_type()
                        .get_param_types()
                        .first()
                        .map(|t| t.is_pointer_type())
                        .unwrap_or(false);
                    let self_arg: BasicValueEnum<'ctx> =
                        if first_param_is_ptr && !val.is_pointer_value() {
                            let tmp = self.entry_alloca(val.get_type(), "display.self");
                            b!(self.bld.build_store(tmp, val));
                            tmp.into()
                        } else {
                            val
                        };
                    let result = b!(self.bld.build_call(fv, &[self_arg.into()], "display.call"))
                        .try_as_basic_value()
                        .basic()
                        .expect("ICE: call returned void");
                    Ok(result)
                } else {
                    self.int_to_string(val, false)
                }
            }
            _ => self.int_to_string(val, false),
        }
    }

    pub(in crate::codegen) fn emit_fmt_bin(
        &mut self,
        val: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let malloc = self.ensure_malloc();
        let buf = b!(self
            .bld
            .build_call(malloc, &[i64t.const_int(65, false).into()], "fb.buf"))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void");
        let buf_ptr = buf.into_pointer_value();

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
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
        .expect("ICE: call returned void")
        .into_int_value();
        let raw_bits = b!(self
            .bld
            .build_int_nsw_sub(i64t.const_int(64, false), lz, "fb.nb"));
        let is_zero = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
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
        let cond =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::SLT, idx, nbits, "fb.cond"));
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
            .build_int_nsw_sub(bit, i64t.const_int(1, false), "fb.nb2"));
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

    pub(in crate::codegen) fn emit_sleep_ms(
        &mut self,
        ms: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let nanosleep = self.module.get_function("nanosleep").unwrap_or_else(|| {
            self.module.add_function(
                "nanosleep",
                i32t.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
                Some(inkwell::module::Linkage::External),
            )
        });
        let ts_ty = self.ctx.struct_type(&[i64t.into(), i64t.into()], false);
        let ts = self.entry_alloca(ts_ty.into(), "sleep.ts");
        let secs = b!(self
            .bld
            .build_int_unsigned_div(ms, i64t.const_int(1000, false), "sleep.s"));
        let ns = b!(self
            .bld
            .build_int_unsigned_rem(ms, i64t.const_int(1000, false), "sleep.rem"));
        let ns_full = b!(self
            .bld
            .build_int_mul(ns, i64t.const_int(1_000_000, false), "sleep.ns"));
        let s_ptr = b!(self.bld.build_struct_gep(ts_ty, ts, 0, "sleep.sp"));
        b!(self.bld.build_store(s_ptr, secs));
        let n_ptr = b!(self.bld.build_struct_gep(ts_ty, ts, 1, "sleep.np"));
        b!(self.bld.build_store(n_ptr, ns_full));
        let null = ptr_ty.const_null();
        b!(self
            .bld
            .build_call(nanosleep, &[ts.into(), null.into()], ""));
        Ok(i64t.const_int(0, false).into())
    }
}
