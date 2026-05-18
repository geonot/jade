//! Argument, arena, pool, and char builtin helpers.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_get_args(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let argc_g = self.module.get_global("__jinn_argc").unwrap_or_else(|| {
            let g = self.module.add_global(i32t, None, "__jinn_argc");
            g.set_initializer(&i32t.const_int(0, false));
            g
        });
        let argv_g = self.module.get_global("__jinn_argv").unwrap_or_else(|| {
            let g = self.module.add_global(ptr_ty, None, "__jinn_argv");
            g.set_initializer(&ptr_ty.const_null());
            g
        });
        let argc =
            b!(self.bld.build_load(i32t, argc_g.as_pointer_value(), "argc")).into_int_value();
        let argc64 = b!(self.bld.build_int_s_extend(argc, i64t, "argc64"));
        let argv = b!(self
            .bld
            .build_load(ptr_ty, argv_g.as_pointer_value(), "argv"))
        .into_pointer_value();

        let header_ptr = self.compile_vec_new(&[])?.into_pointer_value();
        let header_ty = self.vec_header_type();
        let st = self.string_type();
        let str_size: u64 = 24;

        let fv = self.current_fn();
        let loop_bb = self.ctx.append_basic_block(fv, "args.loop");
        let body_bb = self.ctx.append_basic_block(fv, "args.body");
        let done_bb = self.ctx.append_basic_block(fv, "args.done");
        let i_ptr = self.entry_alloca(i64t.into(), "args.i");
        b!(self.bld.build_store(i_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let i = b!(self.bld.build_load(i64t, i_ptr, "i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, i, argc64, "args.cond"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let arg_pp = unsafe { b!(self.bld.build_gep(ptr_ty, argv, &[i], "arg.pp")) };
        let arg_p = b!(self.bld.build_load(ptr_ty, arg_pp, "arg.p")).into_pointer_value();
        let strlen = self.module.get_function("strlen").unwrap_or_else(|| {
            self.module.add_function(
                "strlen",
                i64t.fn_type(&[ptr_ty.into()], false),
                Some(Linkage::External),
            )
        });
        let slen = b!(self.bld.build_call(strlen, &[arg_p.into()], "arg.len"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void")
            .into_int_value();
        let size = b!(self
            .bld
            .build_int_nsw_add(slen, i64t.const_int(1, false), "arg.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "arg.buf"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void");
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[buf.into(), arg_p.into(), size.into()], ""));
        let s = self.build_string(buf, slen, size, "arg.s")?;

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "ga.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "ga.len")).into_int_value();
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "ga.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "ga.cap")).into_int_value();
        let needs_grow = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, len, cap, "ga.full"));
        let grow_bb = self.ctx.append_basic_block(fv, "ga.grow");
        let store_bb = self.ctx.append_basic_block(fv, "ga.store");
        b!(self
            .bld
            .build_conditional_branch(needs_grow, grow_bb, store_bb));

        self.bld.position_at_end(grow_bb);
        let doubled = b!(self
            .bld
            .build_int_nsw_mul(cap, i64t.const_int(2, false), "ga.dbl"));
        let new_cap_cmp = b!(self.bld.build_int_compare(
            IntPredicate::SGT,
            doubled,
            i64t.const_int(4, false),
            "ga.cmp"
        ));
        let new_cap =
            b!(self
                .bld
                .build_select(new_cap_cmp, doubled, i64t.const_int(4, false), "ga.nc"))
            .into_int_value();
        let new_size =
            b!(self
                .bld
                .build_int_nsw_mul(new_cap, i64t.const_int(str_size, false), "ga.ns"));
        let realloc = self.ensure_realloc();
        let data_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "ga.datap"));
        let old_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "ga.optr"));
        let new_ptr =
            b!(self
                .bld
                .build_call(realloc, &[old_ptr.into(), new_size.into()], "ga.nptr"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void");
        b!(self.bld.build_store(data_gep, new_ptr));
        b!(self.bld.build_store(cap_gep, new_cap));
        b!(self.bld.build_unconditional_branch(store_bb));

        self.bld.position_at_end(store_bb);
        let data_gep2 = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "ga.dp2"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep2, "ga.dp")).into_pointer_value();
        let elem_gep = unsafe { b!(self.bld.build_gep(st, data_ptr, &[len], "ga.ep")) };
        b!(self.bld.build_store(elem_gep, s));
        let new_len = b!(self
            .bld
            .build_int_nsw_add(len, i64t.const_int(1, false), "ga.nl"));
        b!(self.bld.build_store(len_gep, new_len));

        let next = b!(self
            .bld
            .build_int_nsw_add(i, i64t.const_int(1, false), "args.next"));
        b!(self.bld.build_store(i_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(header_ptr.into())
    }

    pub(in crate::codegen) fn compile_char_method(
        &mut self,
        method: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let char_val = self.compile_expr(&args[0])?.into_int_value();
        let i64t = self.ctx.i64_type();
        let _bool_t = self.ctx.bool_type();

        match method {
            "to_code" => Ok(char_val.into()),
            "is_digit" => {
                // 0x30..=0x39 ('0'..='9')
                let ge = b!(self.bld.build_int_compare(
                    IntPredicate::SGE,
                    char_val,
                    i64t.const_int(0x30, false),
                    "ch.ge0"
                ));
                let le = b!(self.bld.build_int_compare(
                    IntPredicate::SLE,
                    char_val,
                    i64t.const_int(0x39, false),
                    "ch.le9"
                ));
                let result = b!(self.bld.build_and(ge, le, "ch.isdigit"));
                Ok(result.into())
            }
            "is_alpha" => {
                // A-Z (0x41..=0x5A) or a-z (0x61..=0x7A)
                let ge_a = b!(self.bld.build_int_compare(
                    IntPredicate::SGE,
                    char_val,
                    i64t.const_int(0x41, false),
                    "ch.geA"
                ));
                let le_z = b!(self.bld.build_int_compare(
                    IntPredicate::SLE,
                    char_val,
                    i64t.const_int(0x5A, false),
                    "ch.leZ"
                ));
                let upper = b!(self.bld.build_and(ge_a, le_z, "ch.isupper"));
                let ge_la = b!(self.bld.build_int_compare(
                    IntPredicate::SGE,
                    char_val,
                    i64t.const_int(0x61, false),
                    "ch.gea"
                ));
                let le_lz = b!(self.bld.build_int_compare(
                    IntPredicate::SLE,
                    char_val,
                    i64t.const_int(0x7A, false),
                    "ch.lez"
                ));
                let lower = b!(self.bld.build_and(ge_la, le_lz, "ch.islower"));
                let result = b!(self.bld.build_or(upper, lower, "ch.isalpha"));
                Ok(result.into())
            }
            "is_alphanumeric" => {
                // Combination of is_alpha and is_digit
                let ge_0 = b!(self.bld.build_int_compare(
                    IntPredicate::SGE,
                    char_val,
                    i64t.const_int(0x30, false),
                    "ch.ge0"
                ));
                let le_9 = b!(self.bld.build_int_compare(
                    IntPredicate::SLE,
                    char_val,
                    i64t.const_int(0x39, false),
                    "ch.le9"
                ));
                let digit = b!(self.bld.build_and(ge_0, le_9, "ch.dig"));
                let ge_a = b!(self.bld.build_int_compare(
                    IntPredicate::SGE,
                    char_val,
                    i64t.const_int(0x41, false),
                    "ch.geA"
                ));
                let le_z = b!(self.bld.build_int_compare(
                    IntPredicate::SLE,
                    char_val,
                    i64t.const_int(0x5A, false),
                    "ch.leZ"
                ));
                let upper = b!(self.bld.build_and(ge_a, le_z, "ch.up"));
                let ge_la = b!(self.bld.build_int_compare(
                    IntPredicate::SGE,
                    char_val,
                    i64t.const_int(0x61, false),
                    "ch.gea"
                ));
                let le_lz = b!(self.bld.build_int_compare(
                    IntPredicate::SLE,
                    char_val,
                    i64t.const_int(0x7A, false),
                    "ch.lez"
                ));
                let lower = b!(self.bld.build_and(ge_la, le_lz, "ch.lo"));
                let alpha = b!(self.bld.build_or(upper, lower, "ch.al"));
                let result = b!(self.bld.build_or(digit, alpha, "ch.alnum"));
                Ok(result.into())
            }
            "is_upper" => {
                let ge = b!(self.bld.build_int_compare(
                    IntPredicate::SGE,
                    char_val,
                    i64t.const_int(0x41, false),
                    "ch.geA"
                ));
                let le = b!(self.bld.build_int_compare(
                    IntPredicate::SLE,
                    char_val,
                    i64t.const_int(0x5A, false),
                    "ch.leZ"
                ));
                Ok(b!(self.bld.build_and(ge, le, "ch.isupper")).into())
            }
            "is_lower" => {
                let ge = b!(self.bld.build_int_compare(
                    IntPredicate::SGE,
                    char_val,
                    i64t.const_int(0x61, false),
                    "ch.gea"
                ));
                let le = b!(self.bld.build_int_compare(
                    IntPredicate::SLE,
                    char_val,
                    i64t.const_int(0x7A, false),
                    "ch.lez"
                ));
                Ok(b!(self.bld.build_and(ge, le, "ch.islower")).into())
            }
            "is_whitespace" => {
                // space(0x20), tab(0x09), newline(0x0A), carriage return(0x0D)
                let is_sp = b!(self.bld.build_int_compare(
                    IntPredicate::EQ,
                    char_val,
                    i64t.const_int(0x20, false),
                    "ch.sp"
                ));
                let is_tab = b!(self.bld.build_int_compare(
                    IntPredicate::EQ,
                    char_val,
                    i64t.const_int(0x09, false),
                    "ch.tab"
                ));
                let is_nl = b!(self.bld.build_int_compare(
                    IntPredicate::EQ,
                    char_val,
                    i64t.const_int(0x0A, false),
                    "ch.nl"
                ));
                let is_cr = b!(self.bld.build_int_compare(
                    IntPredicate::EQ,
                    char_val,
                    i64t.const_int(0x0D, false),
                    "ch.cr"
                ));
                let t1 = b!(self.bld.build_or(is_sp, is_tab, "ch.ws1"));
                let t2 = b!(self.bld.build_or(is_nl, is_cr, "ch.ws2"));
                Ok(b!(self.bld.build_or(t1, t2, "ch.isws")).into())
            }
            "to_upper" => {
                // If lowercase (0x61..=0x7A), subtract 0x20
                let ge = b!(self.bld.build_int_compare(
                    IntPredicate::SGE,
                    char_val,
                    i64t.const_int(0x61, false),
                    "ch.gea"
                ));
                let le = b!(self.bld.build_int_compare(
                    IntPredicate::SLE,
                    char_val,
                    i64t.const_int(0x7A, false),
                    "ch.lez"
                ));
                let is_lower = b!(self.bld.build_and(ge, le, "ch.islo"));
                let upper =
                    b!(self
                        .bld
                        .build_int_nsw_sub(char_val, i64t.const_int(0x20, false), "ch.toU"));
                Ok(b!(self
                    .bld
                    .build_select(is_lower, upper, char_val, "ch.toupper"))
                .into())
            }
            "to_lower" => {
                // If uppercase (0x41..=0x5A), add 0x20
                let ge = b!(self.bld.build_int_compare(
                    IntPredicate::SGE,
                    char_val,
                    i64t.const_int(0x41, false),
                    "ch.geA"
                ));
                let le = b!(self.bld.build_int_compare(
                    IntPredicate::SLE,
                    char_val,
                    i64t.const_int(0x5A, false),
                    "ch.leZ"
                ));
                let is_upper = b!(self.bld.build_and(ge, le, "ch.isup"));
                let lower =
                    b!(self
                        .bld
                        .build_int_add(char_val, i64t.const_int(0x20, false), "ch.toL"));
                Ok(b!(self
                    .bld
                    .build_select(is_upper, lower, char_val, "ch.tolower"))
                .into())
            }
            "to_float" => {
                let f64t = self.ctx.f64_type();
                let result = b!(self.bld.build_signed_int_to_float(char_val, f64t, "i2f"));
                Ok(result.into())
            }
            "abs" => {
                // x < 0 ? -x : x
                let neg = b!(self.bld.build_int_neg(char_val, "int.neg"));
                let is_neg = b!(self.bld.build_int_compare(
                    IntPredicate::SLT,
                    char_val,
                    i64t.const_zero(),
                    "int.isneg"
                ));
                Ok(b!(self.bld.build_select(is_neg, neg, char_val, "int.abs")).into())
            }
            "min" => {
                if args.len() < 2 {
                    return Err("min() takes 1 argument".into());
                }
                let other = self.compile_expr(&args[1])?.into_int_value();
                let cmp =
                    b!(self
                        .bld
                        .build_int_compare(IntPredicate::SLT, char_val, other, "int.lt"));
                Ok(b!(self.bld.build_select(cmp, char_val, other, "int.min")).into())
            }
            "max" => {
                if args.len() < 2 {
                    return Err("max() takes 1 argument".into());
                }
                let other = self.compile_expr(&args[1])?.into_int_value();
                let cmp =
                    b!(self
                        .bld
                        .build_int_compare(IntPredicate::SGT, char_val, other, "int.gt"));
                Ok(b!(self.bld.build_select(cmp, char_val, other, "int.max")).into())
            }
            "clamp" => {
                if args.len() < 3 {
                    return Err("clamp() takes 2 arguments (lo, hi)".into());
                }
                let lo = self.compile_expr(&args[1])?.into_int_value();
                let hi = self.compile_expr(&args[2])?.into_int_value();
                // max(lo, min(x, hi))
                let cmp_hi =
                    b!(self
                        .bld
                        .build_int_compare(IntPredicate::SLT, char_val, hi, "clamp.lthi"));
                let min_val =
                    b!(self.bld.build_select(cmp_hi, char_val, hi, "clamp.min")).into_int_value();
                let cmp_lo =
                    b!(self
                        .bld
                        .build_int_compare(IntPredicate::SGT, min_val, lo, "clamp.gtlo"));
                Ok(b!(self.bld.build_select(cmp_lo, min_val, lo, "clamp.max")).into())
            }
            "to_str" => self.compile_to_string(&args[0]),
            _ => Err(format!("unknown char method '{method}'")),
        }
    }
}
