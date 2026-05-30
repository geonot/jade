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

        let header_ptr = self.vec_alloc_empty()?;
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
}
