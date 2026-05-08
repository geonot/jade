//! Vector allocation, element access, mutation, bounds, and runtime allocation helpers.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn vec_header_type(&self) -> inkwell::types::StructType<'ctx> {
        self.module
            .get_struct_type("__vec_header")
            .unwrap_or_else(|| {
                let st = self.ctx.opaque_struct_type("__vec_header");
                st.set_body(
                    &[
                        self.ctx.ptr_type(AddressSpace::default()).into(),
                        self.ctx.i64_type().into(),
                        self.ctx.i64_type().into(),
                    ],
                    false,
                );
                st
            })
    }

    pub(crate) fn compile_vec_new(
        &mut self,
        elems: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let malloc = self.ensure_malloc();

        let header_size = i64t.const_int(24, false);
        let header_ptr = b!(self
            .bld
            .build_call(malloc, &[header_size.into()], "vec.hdr"))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void")
        .into_pointer_value();

        let n = elems.len();
        let cap = if n == 0 {
            0u64
        } else {
            n.next_power_of_two() as u64
        };

        if n > 0 {
            let elem_ty = &elems[0].ty;
            let lty = self.llvm_ty(elem_ty);
            let elem_size = self.type_store_size(lty);
            let buf_size = i64t.const_int(cap * elem_size, false);
            let buf = b!(self.bld.build_call(malloc, &[buf_size.into()], "vec.buf"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_pointer_value();

            for (i, e) in elems.iter().enumerate() {
                let val = self.compile_expr(e)?;
                let gep = unsafe {
                    b!(self
                        .bld
                        .build_gep(lty, buf, &[i64t.const_int(i as u64, false)], "vec.elem"))
                };
                b!(self.bld.build_store(gep, val));
            }

            let ptr_gep = b!(self
                .bld
                .build_struct_gep(header_ty, header_ptr, 0, "vec.ptr"));
            b!(self.bld.build_store(ptr_gep, buf));
        } else {
            let ptr_gep = b!(self
                .bld
                .build_struct_gep(header_ty, header_ptr, 0, "vec.ptr"));
            b!(self.bld.build_store(ptr_gep, ptr_ty.const_null()));
        }

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vec.len"));
        b!(self
            .bld
            .build_store(len_gep, i64t.const_int(n as u64, false)));

        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "vec.cap"));
        b!(self.bld.build_store(cap_gep, i64t.const_int(cap, false)));

        Ok(header_ptr.into())
    }

    pub(crate) fn compile_vec_method(
        &mut self,
        obj: &hir::Expr,
        method: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let elem_ty = match &obj.ty {
            Type::Vec(et) => *et.clone(),
            Type::Array(et, _) => *et.clone(),
            _ => Type::I64,
        };

        // Fixed-size array: inline linear scan for contains, len returns constant
        if let Type::Array(_, arr_len) = &obj.ty {
            let arr_len = *arr_len;
            match method {
                "contains" => return self.array_contains(obj, &elem_ty, arr_len, args),
                "len" => {
                    return Ok(self.ctx.i64_type().const_int(arr_len as u64, false).into());
                }
                _ => {}
            }
        }

        let obj_val = self.compile_expr(obj)?;
        let header_ptr = obj_val.into_pointer_value();

        match method {
            "push" => self.vec_push(header_ptr, &elem_ty, args),
            "pop" => self.vec_pop(header_ptr, &elem_ty),
            "len" => self.vec_len(header_ptr),
            "get" => self.vec_get(header_ptr, &elem_ty, args),
            "set" => {
                if args.len() < 2 {
                    return Err("set() requires index and value".into());
                }
                let idx = self.compile_expr(&args[0])?.into_int_value();
                let val = self.compile_expr(&args[1])?;
                self.vec_set_val(header_ptr, &elem_ty, idx, val)
            }
            "remove" => {
                if args.is_empty() {
                    return Err("remove() requires an index".into());
                }
                let idx = self.compile_expr(&args[0])?.into_int_value();
                self.vec_remove_val(header_ptr, &elem_ty, idx)
            }
            "clear" => self.vec_clear(header_ptr),
            "map" => self.vec_map(header_ptr, &elem_ty, args),
            "filter" => self.vec_filter(header_ptr, &elem_ty, args),
            "fold" => self.vec_fold(header_ptr, &elem_ty, args),
            "any" => self.vec_any_all(header_ptr, &elem_ty, args, true),
            "all" => self.vec_any_all(header_ptr, &elem_ty, args, false),
            "find" => self.vec_find(header_ptr, &elem_ty, args),
            "count" => self.vec_len(header_ptr),
            "sum" => self.vec_sum(header_ptr, &elem_ty),
            "take" => self.vec_take_skip(header_ptr, &elem_ty, args, true),
            "skip" => self.vec_take_skip(header_ptr, &elem_ty, args, false),
            "zip" => self.vec_zip(header_ptr, &elem_ty, args),
            "chain" => self.vec_chain(header_ptr, &elem_ty, args),
            "enumerate" => self.vec_enumerate(header_ptr, &elem_ty),
            "flatten" => self.vec_flatten(header_ptr, &elem_ty),
            "contains" => self.vec_contains(header_ptr, &elem_ty, args),
            "reverse" => self.vec_reverse(header_ptr, &elem_ty),
            "sort" => self.vec_sort(header_ptr, &elem_ty),
            "join" => self.vec_join(header_ptr, args),
            "collect" => Ok(obj_val), // collect is identity on Vec
            _ => Err(format!("no method '{method}' on Vec")),
        }
    }

    pub(crate) fn vec_len(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let header_ty = self.vec_header_type();
        let i64t = self.ctx.i64_type();
        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vl.len"));
        Ok(b!(self.bld.build_load(i64t, len_gep, "vl.v")))
    }

    pub(in crate::codegen) fn vec_push(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("push() requires an argument".into());
        }
        let val = self.compile_expr(&args[0])?;
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        self.vec_push_raw(header_ptr, val, lty, elem_size)?;
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(crate) fn vec_push_raw(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        val: BasicValueEnum<'ctx>,
        lty: inkwell::types::BasicTypeEnum<'ctx>,
        elem_size: u64,
    ) -> Result<(), String> {
        self.vec_push_raw_with_floor(header_ptr, val, lty, elem_size, self.empty_vec_growth_floor)
    }

    pub(crate) fn vec_push_raw_with_floor(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        val: BasicValueEnum<'ctx>,
        lty: inkwell::types::BasicTypeEnum<'ctx>,
        elem_size: u64,
        growth_floor: u64,
    ) -> Result<(), String> {
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let fv = self.current_fn();
        let growth_floor = growth_floor.clamp(16, 128).next_power_of_two();

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vpr.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "vpr.len")).into_int_value();
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "vpr.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "vpr.cap")).into_int_value();

        let needs_grow = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, len, cap, "vpr.full"));
        let grow_bb = self.ctx.append_basic_block(fv, "vpr.grow");
        let store_bb = self.ctx.append_basic_block(fv, "vpr.store");
        b!(self
            .bld
            .build_conditional_branch(needs_grow, grow_bb, store_bb));

        self.bld.position_at_end(grow_bb);
        let doubled = b!(self
            .bld
            .build_int_nsw_mul(cap, i64t.const_int(2, false), "vpr.dbl"));
        let new_cap_cmp = b!(self.bld.build_int_compare(
            IntPredicate::SGE,
            doubled,
            i64t.const_int(growth_floor, false),
            "vpr.cmp"
        ));
        let new_cap = b!(self.bld.build_select(
            new_cap_cmp,
            doubled,
            i64t.const_int(growth_floor, false),
            "vpr.nc",
        ))
        .into_int_value();
        let new_size =
            b!(self
                .bld
                .build_int_nsw_mul(new_cap, i64t.const_int(elem_size, false), "vpr.ns"));
        let realloc = self.ensure_realloc();
        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "vpr.ptrp"));
        let old_ptr = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "vpr.optr"
        ));
        let new_ptr =
            b!(self
                .bld
                .build_call(realloc, &[old_ptr.into(), new_size.into()], "vpr.nptr"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void");
        b!(self.bld.build_store(ptr_gep, new_ptr));
        b!(self.bld.build_store(cap_gep, new_cap));
        b!(self.bld.build_unconditional_branch(store_bb));

        self.bld.position_at_end(store_bb);
        let ptr_gep2 = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "vpr.ptrp2"));
        let data_ptr = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep2,
            "vpr.data"
        ))
        .into_pointer_value();
        let len2 = b!(self.bld.build_load(i64t, len_gep, "vpr.len2")).into_int_value();
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[len2], "vpr.egep")) };
        b!(self.bld.build_store(elem_gep, val));
        let new_len = b!(self
            .bld
            .build_int_nsw_add(len2, i64t.const_int(1, false), "vpr.nl"));
        b!(self.bld.build_store(len_gep, new_len));
        Ok(())
    }

    pub(crate) fn vec_pop(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let lty = self.llvm_ty(elem_ty);

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vpop.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "vpop.len")).into_int_value();

        // Bounds check: trap if len == 0
        let fv = self.current_fn();
        let is_nonzero = b!(self.bld.build_int_compare(
            IntPredicate::SGT,
            len,
            i64t.const_int(0, false),
            "vpop.nz"
        ));
        let ok_bb = self.ctx.append_basic_block(fv, "vpop.ok");
        let fail_bb = self.ctx.append_basic_block(fv, "vpop.fail");
        b!(self
            .bld
            .build_conditional_branch(is_nonzero, ok_bb, fail_bb));
        self.bld.position_at_end(fail_bb);
        let trap = self.get_or_declare_trap();
        b!(self.bld.build_call(trap, &[], ""));
        b!(self.bld.build_unreachable());
        self.bld.position_at_end(ok_bb);

        let new_len = b!(self
            .bld
            .build_int_nsw_sub(len, i64t.const_int(1, false), "vpop.nl"));
        b!(self.bld.build_store(len_gep, new_len));

        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "vpop.ptrp"));
        let data_ptr = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "vpop.data"
        ))
        .into_pointer_value();
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[new_len], "vpop.egep")) };
        Ok(b!(self.bld.build_load(lty, elem_gep, "vpop.v")))
    }

    pub(in crate::codegen) fn vec_get(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("get() requires an index".into());
        }
        let idx = self.compile_expr(&args[0])?.into_int_value();
        self.vec_get_idx(header_ptr, elem_ty, idx)
    }

    pub(crate) fn vec_get_idx(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        idx: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let lty = self.llvm_ty(elem_ty);

        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "vg.ptrp"));
        let data_ptr = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "vg.data"
        ))
        .into_pointer_value();

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vg.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "vg.len")).into_int_value();
        self.emit_vec_bounds_check(idx, len)?;

        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "vg.egep")) };
        Ok(b!(self.bld.build_load(lty, elem_gep, "vg.v")))
    }

    pub(crate) fn vec_set_val(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        idx: inkwell::values::IntValue<'ctx>,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let lty = self.llvm_ty(elem_ty);

        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "vs.ptrp"));
        let data_ptr = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "vs.data"
        ))
        .into_pointer_value();

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vs.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "vs.len")).into_int_value();
        self.emit_vec_bounds_check(idx, len)?;

        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "vs.egep")) };
        b!(self.bld.build_store(elem_gep, val));
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(crate) fn vec_remove_val(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        idx: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);

        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "vr.ptrp"));
        let data_ptr = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "vr.data"
        ))
        .into_pointer_value();

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vr.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "vr.len")).into_int_value();
        self.emit_vec_bounds_check(idx, len)?;

        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "vr.egep")) };
        let removed = b!(self.bld.build_load(lty, elem_gep, "vr.v"));

        let next_idx = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "vr.ni"));
        let src = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[next_idx], "vr.src")) };
        let count = b!(self.bld.build_int_nsw_sub(len, next_idx, "vr.cnt"));
        let bytes =
            b!(self
                .bld
                .build_int_nsw_mul(count, i64t.const_int(elem_size, false), "vr.bytes"));
        let memmove = self.ensure_memmove();
        b!(self
            .bld
            .build_call(memmove, &[elem_gep.into(), src.into(), bytes.into()], ""));

        let new_len = b!(self
            .bld
            .build_int_nsw_sub(len, i64t.const_int(1, false), "vr.nl"));
        b!(self.bld.build_store(len_gep, new_len));

        Ok(removed)
    }

    pub(crate) fn vec_clear(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vc.lenp"));
        b!(self.bld.build_store(len_gep, i64t.const_int(0, false)));
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(crate) fn emit_vec_bounds_check(
        &mut self,
        idx: inkwell::values::IntValue<'ctx>,
        len: inkwell::values::IntValue<'ctx>,
    ) -> Result<(), String> {
        let fv = self.current_fn();
        let ok = b!(self
            .bld
            .build_int_compare(IntPredicate::ULT, idx, len, "vbc.ok"));
        let ok_bb = self.ctx.append_basic_block(fv, "vbc.ok");
        let fail_bb = self.ctx.append_basic_block(fv, "vbc.fail");
        b!(self.bld.build_conditional_branch(ok, ok_bb, fail_bb));

        self.bld.position_at_end(fail_bb);
        let trap = self.get_or_declare_trap();
        b!(self.bld.build_call(trap, &[], ""));
        b!(self.bld.build_unreachable());

        self.bld.position_at_end(ok_bb);
        Ok(())
    }

    pub(crate) fn ensure_realloc(&self) -> inkwell::values::FunctionValue<'ctx> {
        let name = "realloc";
        if let Some(f) = self.module.get_function(name) {
            return f;
        }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let ft = ptr_ty.fn_type(&[ptr_ty.into(), i64t.into()], false);
        self.module.add_function(name, ft, Some(Linkage::External))
    }

    pub(crate) fn ensure_calloc(&self) -> inkwell::values::FunctionValue<'ctx> {
        let name = "calloc";
        if let Some(f) = self.module.get_function(name) {
            return f;
        }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let ft = ptr_ty.fn_type(&[i64t.into(), i64t.into()], false);
        self.module.add_function(name, ft, Some(Linkage::External))
    }

    pub(crate) fn ensure_memmove(&self) -> inkwell::values::FunctionValue<'ctx> {
        let name = "memmove";
        if let Some(f) = self.module.get_function(name) {
            return f;
        }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64t.into()], false);
        self.module.add_function(name, ft, Some(Linkage::External))
    }

    pub(crate) fn ensure_memset(&self) -> inkwell::values::FunctionValue<'ctx> {
        let name = "memset";
        if let Some(f) = self.module.get_function(name) {
            return f;
        }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let ft = ptr_ty.fn_type(
            &[ptr_ty.into(), self.ctx.i32_type().into(), i64t.into()],
            false,
        );
        self.module.add_function(name, ft, Some(Linkage::External))
    }

    pub(in crate::codegen) fn get_or_declare_trap(&self) -> inkwell::values::FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("llvm.trap") {
            return f;
        }
        let ft = self.ctx.void_type().fn_type(&[], false);
        self.module.add_function("llvm.trap", ft, None)
    }

    /// Helper: load vec data_ptr and len from header
    pub(in crate::codegen) fn vec_data_and_len(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<
        (
            inkwell::values::PointerValue<'ctx>,
            inkwell::values::IntValue<'ctx>,
        ),
        String,
    > {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "vdl.ptrp"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, ptr_gep, "vdl.data")).into_pointer_value();
        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vdl.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "vdl.len")).into_int_value();
        Ok((data_ptr, len))
    }

    /// Helper: allocate a new empty vec header, returns header_ptr
    pub(in crate::codegen) fn vec_alloc_empty(
        &mut self,
    ) -> Result<inkwell::values::PointerValue<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let malloc = self.ensure_malloc();
        let header_ptr =
            b!(self
                .bld
                .build_call(malloc, &[i64t.const_int(24, false).into()], "vn.hdr"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void")
            .into_pointer_value();
        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "vn.ptr"));
        b!(self.bld.build_store(ptr_gep, ptr_ty.const_null()));
        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vn.len"));
        b!(self.bld.build_store(len_gep, i64t.const_int(0, false)));
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "vn.cap"));
        b!(self.bld.build_store(cap_gep, i64t.const_int(0, false)));
        Ok(header_ptr)
    }
}
