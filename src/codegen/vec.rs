use inkwell::module::Linkage;
use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

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
        .unwrap()
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
                .unwrap()
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
                if args.len() < 2 { return Err("set() requires index and value".into()); }
                let idx = self.compile_expr(&args[0])?.into_int_value();
                let val = self.compile_expr(&args[1])?;
                self.vec_set_val(header_ptr, &elem_ty, idx, val)
            }
            "remove" => {
                if args.is_empty() { return Err("remove() requires an index".into()); }
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

    fn vec_push(
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
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let fv = self.cur_fn.unwrap();

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
            IntPredicate::SGT,
            doubled,
            i64t.const_int(4, false),
            "vpr.cmp"
        ));
        let new_cap =
            b!(self
                .bld
                .build_select(new_cap_cmp, doubled, i64t.const_int(4, false), "vpr.nc"))
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
            .unwrap();
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

    fn vec_get(
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
        let fv = self.cur_fn.unwrap();
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

    fn get_or_declare_trap(&self) -> inkwell::values::FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("llvm.trap") {
            return f;
        }
        let ft = self.ctx.void_type().fn_type(&[], false);
        self.module.add_function("llvm.trap", ft, None)
    }

    /// Helper: load vec data_ptr and len from header
    fn vec_data_and_len(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<(inkwell::values::PointerValue<'ctx>, inkwell::values::IntValue<'ctx>), String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let ptr_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "vdl.ptrp"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, ptr_gep, "vdl.data")).into_pointer_value();
        let len_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 1, "vdl.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "vdl.len")).into_int_value();
        Ok((data_ptr, len))
    }

    /// Helper: allocate a new empty vec header, returns header_ptr
    fn vec_alloc_empty(&mut self) -> Result<inkwell::values::PointerValue<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let malloc = self.ensure_malloc();
        let header_ptr = b!(self.bld.build_call(malloc, &[i64t.const_int(24, false).into()], "vn.hdr"))
            .try_as_basic_value().basic().unwrap().into_pointer_value();
        let ptr_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "vn.ptr"));
        b!(self.bld.build_store(ptr_gep, ptr_ty.const_null()));
        let len_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 1, "vn.len"));
        b!(self.bld.build_store(len_gep, i64t.const_int(0, false)));
        let cap_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 2, "vn.cap"));
        b!(self.bld.build_store(cap_gep, i64t.const_int(0, false)));
        Ok(header_ptr)
    }

    fn vec_map(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_val = self.compile_expr(&args[0])?;
        let fn_ty = &args[0].ty;
        let out_elem_ty = match fn_ty {
            Type::Fn(_, ret) => ret.as_ref().clone(),
            _ => return Err("map callback must be a function".into()),
        };
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let out_lty = self.llvm_ty(&out_elem_ty);
        let out_elem_size = self.type_store_size(out_lty);
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;

        let idx_ptr = self.entry_alloca(i64t.into(), "map.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "map.loop");
        let body_bb = self.ctx.append_basic_block(fv, "map.body");
        let done_bb = self.ctx.append_basic_block(fv, "map.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "map.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, len, "map.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "map.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "map.elem"));
        let mapped = self.indirect_call_vals(fn_val, fn_ty, &[elem])?;
        self.vec_push_raw(out_hdr, mapped, out_lty, out_elem_size)?;
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "map.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    fn vec_filter(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_val = self.compile_expr(&args[0])?;
        let fn_ty = &args[0].ty;
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;

        let idx_ptr = self.entry_alloca(i64t.into(), "filt.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "filt.loop");
        let body_bb = self.ctx.append_basic_block(fv, "filt.body");
        let push_bb = self.ctx.append_basic_block(fv, "filt.push");
        let cont_bb = self.ctx.append_basic_block(fv, "filt.cont");
        let done_bb = self.ctx.append_basic_block(fv, "filt.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "filt.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, len, "filt.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "filt.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "filt.elem"));
        let pred = self.indirect_call_vals(fn_val, fn_ty, &[elem])?.into_int_value();
        let pred_bool = b!(self.bld.build_int_compare(IntPredicate::NE, pred, self.ctx.bool_type().const_int(0, false), "filt.bool"));
        b!(self.bld.build_conditional_branch(pred_bool, push_bb, cont_bb));

        self.bld.position_at_end(push_bb);
        // Reload elem since we may need it fresh
        let elem2 = b!(self.bld.build_load(lty, elem_gep, "filt.elem2"));
        self.vec_push_raw(out_hdr, elem2, lty, elem_size)?;
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "filt.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    fn vec_fold(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let init_val = self.compile_expr(&args[0])?;
        let fn_val = self.compile_expr(&args[1])?;
        let fn_ty = &args[1].ty;
        let acc_lty = init_val.get_type();
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        let acc_ptr = self.entry_alloca(acc_lty, "fold.acc");
        b!(self.bld.build_store(acc_ptr, init_val));
        let idx_ptr = self.entry_alloca(i64t.into(), "fold.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "fold.loop");
        let body_bb = self.ctx.append_basic_block(fv, "fold.body");
        let done_bb = self.ctx.append_basic_block(fv, "fold.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "fold.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, len, "fold.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let cur_acc = b!(self.bld.build_load(acc_lty, acc_ptr, "fold.cur"));
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "fold.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "fold.elem"));
        let new_acc = self.indirect_call_vals(fn_val, fn_ty, &[cur_acc, elem])?;
        b!(self.bld.build_store(acc_ptr, new_acc));
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "fold.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(acc_lty, acc_ptr, "fold.result")))
    }

    fn vec_any_all(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
        is_any: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_val = self.compile_expr(&args[0])?;
        let fn_ty = &args[0].ty;
        let i64t = self.ctx.i64_type();
        let bool_ty = self.ctx.bool_type();
        let lty = self.llvm_ty(elem_ty);
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        // For any: start false, short-circuit on true
        // For all: start true, short-circuit on false
        let init = if is_any { 0u64 } else { 1u64 };
        let result_ptr = self.entry_alloca(bool_ty.into(), "aa.res");
        b!(self.bld.build_store(result_ptr, bool_ty.const_int(init, false)));
        let idx_ptr = self.entry_alloca(i64t.into(), "aa.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "aa.loop");
        let body_bb = self.ctx.append_basic_block(fv, "aa.body");
        let done_bb = self.ctx.append_basic_block(fv, "aa.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "aa.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, len, "aa.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "aa.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "aa.elem"));
        let pred = self.indirect_call_vals(fn_val, fn_ty, &[elem])?.into_int_value();
        let pred_bool = b!(self.bld.build_int_compare(IntPredicate::NE, pred, bool_ty.const_int(0, false), "aa.pb"));
        if is_any {
            // If true found, set result=true and exit
            let found_bb = self.ctx.append_basic_block(fv, "aa.found");
            let cont_bb = self.ctx.append_basic_block(fv, "aa.cont");
            b!(self.bld.build_conditional_branch(pred_bool, found_bb, cont_bb));
            self.bld.position_at_end(found_bb);
            b!(self.bld.build_store(result_ptr, bool_ty.const_int(1, false)));
            b!(self.bld.build_unconditional_branch(done_bb));
            self.bld.position_at_end(cont_bb);
        } else {
            // If false found, set result=false and exit
            let fail_bb = self.ctx.append_basic_block(fv, "aa.fail");
            let cont_bb = self.ctx.append_basic_block(fv, "aa.cont");
            b!(self.bld.build_conditional_branch(pred_bool, cont_bb, fail_bb));
            self.bld.position_at_end(fail_bb);
            b!(self.bld.build_store(result_ptr, bool_ty.const_int(0, false)));
            b!(self.bld.build_unconditional_branch(done_bb));
            self.bld.position_at_end(cont_bb);
        }
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "aa.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(bool_ty, result_ptr, "aa.v")))
    }

    fn vec_find(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_val = self.compile_expr(&args[0])?;
        let fn_ty = &args[0].ty;
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let bool_ty = self.ctx.bool_type();
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        let result_ptr = self.entry_alloca(lty, "find.res");
        let idx_ptr = self.entry_alloca(i64t.into(), "find.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "find.loop");
        let body_bb = self.ctx.append_basic_block(fv, "find.body");
        let done_bb = self.ctx.append_basic_block(fv, "find.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "find.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, len, "find.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "find.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "find.elem"));
        let pred = self.indirect_call_vals(fn_val, fn_ty, &[elem])?.into_int_value();
        let pred_bool = b!(self.bld.build_int_compare(IntPredicate::NE, pred, bool_ty.const_int(0, false), "find.pb"));
        let found_bb = self.ctx.append_basic_block(fv, "find.found");
        let cont_bb = self.ctx.append_basic_block(fv, "find.cont");
        b!(self.bld.build_conditional_branch(pred_bool, found_bb, cont_bb));

        self.bld.position_at_end(found_bb);
        let elem2 = b!(self.bld.build_load(lty, elem_gep, "find.elem2"));
        b!(self.bld.build_store(result_ptr, elem2));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "find.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(lty, result_ptr, "find.v")))
    }

    fn vec_sum(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        let acc_ptr = self.entry_alloca(lty, "sum.acc");
        let zero: BasicValueEnum<'ctx> = match elem_ty {
            Type::F64 => self.ctx.f64_type().const_float(0.0).into(),
            _ => i64t.const_int(0, false).into(),
        };
        b!(self.bld.build_store(acc_ptr, zero));
        let idx_ptr = self.entry_alloca(i64t.into(), "sum.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "sum.loop");
        let body_bb = self.ctx.append_basic_block(fv, "sum.body");
        let done_bb = self.ctx.append_basic_block(fv, "sum.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "sum.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, len, "sum.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "sum.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "sum.elem"));
        let cur = b!(self.bld.build_load(lty, acc_ptr, "sum.cur"));
        let new_val: BasicValueEnum<'ctx> = match elem_ty {
            Type::F64 => b!(self.bld.build_float_add(cur.into_float_value(), elem.into_float_value(), "sum.add")).into(),
            _ => b!(self.bld.build_int_nsw_add(cur.into_int_value(), elem.into_int_value(), "sum.add")).into(),
        };
        b!(self.bld.build_store(acc_ptr, new_val));
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "sum.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(lty, acc_ptr, "sum.v")))
    }

    fn vec_take_skip(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
        is_take: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let n_val = self.compile_expr(&args[0])?.into_int_value();
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;

        // For take: iterate 0..min(n, len)
        // For skip: iterate n..len
        let (start, end) = if is_take {
            let min_bb = self.ctx.append_basic_block(fv, "ts.min");
            let use_n_bb = self.ctx.append_basic_block(fv, "ts.usen");
            let use_len_bb = self.ctx.append_basic_block(fv, "ts.uselen");
            let cmp = b!(self.bld.build_int_compare(IntPredicate::SLT, n_val, len, "ts.cmp"));
            b!(self.bld.build_conditional_branch(cmp, use_n_bb, use_len_bb));
            self.bld.position_at_end(use_n_bb);
            b!(self.bld.build_unconditional_branch(min_bb));
            self.bld.position_at_end(use_len_bb);
            b!(self.bld.build_unconditional_branch(min_bb));
            self.bld.position_at_end(min_bb);
            let phi = b!(self.bld.build_phi(i64t, "ts.end"));
            phi.add_incoming(&[(&n_val, use_n_bb), (&len, use_len_bb)]);
            (i64t.const_int(0, false), phi.as_basic_value().into_int_value())
        } else {
            (n_val, len)
        };

        let idx_ptr = self.entry_alloca(i64t.into(), "ts.idx");
        b!(self.bld.build_store(idx_ptr, start));

        let loop_bb = self.ctx.append_basic_block(fv, "ts.loop");
        let body_bb = self.ctx.append_basic_block(fv, "ts.body");
        let done_bb = self.ctx.append_basic_block(fv, "ts.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "ts.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, end, "ts.cmp2"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "ts.gep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "ts.elem"));
        self.vec_push_raw(out_hdr, elem, lty, elem_size)?;
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "ts.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    fn vec_zip(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let other_val = self.compile_expr(&args[0])?.into_pointer_value();
        let other_elem_ty = match &args[0].ty {
            Type::Vec(et) => et.as_ref().clone(),
            _ => return Err("zip argument must be Vec".into()),
        };
        let i64t = self.ctx.i64_type();
        let lty_a = self.llvm_ty(elem_ty);
        let lty_b = self.llvm_ty(&other_elem_ty);
        // Tuple type: (A, B)
        let tuple_lty = self.ctx.struct_type(&[lty_a.into(), lty_b.into()], false);
        let tuple_size = self.type_store_size(tuple_lty.into());
        let fv = self.cur_fn.unwrap();

        let (data_a, len_a) = self.vec_data_and_len(header_ptr)?;
        let (data_b, len_b) = self.vec_data_and_len(other_val)?;

        // min(len_a, len_b)
        let min_bb = self.ctx.append_basic_block(fv, "zip.min");
        let use_a_bb = self.ctx.append_basic_block(fv, "zip.usea");
        let use_b_bb = self.ctx.append_basic_block(fv, "zip.useb");
        let cmp = b!(self.bld.build_int_compare(IntPredicate::SLT, len_a, len_b, "zip.cmp"));
        b!(self.bld.build_conditional_branch(cmp, use_a_bb, use_b_bb));
        self.bld.position_at_end(use_a_bb);
        b!(self.bld.build_unconditional_branch(min_bb));
        self.bld.position_at_end(use_b_bb);
        b!(self.bld.build_unconditional_branch(min_bb));
        self.bld.position_at_end(min_bb);
        let phi = b!(self.bld.build_phi(i64t, "zip.len"));
        phi.add_incoming(&[(&len_a, use_a_bb), (&len_b, use_b_bb)]);
        let min_len = phi.as_basic_value().into_int_value();

        let out_hdr = self.vec_alloc_empty()?;
        let idx_ptr = self.entry_alloca(i64t.into(), "zip.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "zip.loop");
        let body_bb = self.ctx.append_basic_block(fv, "zip.body");
        let done_bb = self.ctx.append_basic_block(fv, "zip.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "zip.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, min_len, "zip.cmp2"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let a_gep = unsafe { b!(self.bld.build_gep(lty_a, data_a, &[idx], "zip.a")) };
        let a_val = b!(self.bld.build_load(lty_a, a_gep, "zip.av"));
        let b_gep = unsafe { b!(self.bld.build_gep(lty_b, data_b, &[idx], "zip.b")) };
        let b_val = b!(self.bld.build_load(lty_b, b_gep, "zip.bv"));
        // Build tuple
        let mut tup = tuple_lty.get_undef();
        tup = b!(self.bld.build_insert_value(tup, a_val, 0, "zip.t0")).into_struct_value();
        tup = b!(self.bld.build_insert_value(tup, b_val, 1, "zip.t1")).into_struct_value();
        self.vec_push_raw(out_hdr, tup.into(), tuple_lty.into(), tuple_size)?;
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "zip.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    fn vec_chain(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let other_val = self.compile_expr(&args[0])?.into_pointer_value();
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        let fv = self.cur_fn.unwrap();

        let out_hdr = self.vec_alloc_empty()?;

        // Copy first vec
        let (data_a, len_a) = self.vec_data_and_len(header_ptr)?;
        let idx_ptr = self.entry_alloca(i64t.into(), "chn.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        let loop1 = self.ctx.append_basic_block(fv, "chn.l1");
        let body1 = self.ctx.append_basic_block(fv, "chn.b1");
        let mid = self.ctx.append_basic_block(fv, "chn.mid");
        b!(self.bld.build_unconditional_branch(loop1));
        self.bld.position_at_end(loop1);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "chn.i1")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, len_a, "chn.c1"));
        b!(self.bld.build_conditional_branch(cond, body1, mid));
        self.bld.position_at_end(body1);
        let gep = unsafe { b!(self.bld.build_gep(lty, data_a, &[idx], "chn.g1")) };
        let elem = b!(self.bld.build_load(lty, gep, "chn.e1"));
        self.vec_push_raw(out_hdr, elem, lty, elem_size)?;
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "chn.n1"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop1));

        // Copy second vec
        self.bld.position_at_end(mid);
        let (data_b, len_b) = self.vec_data_and_len(other_val)?;
        let idx_ptr2 = self.entry_alloca(i64t.into(), "chn.idx2");
        b!(self.bld.build_store(idx_ptr2, i64t.const_int(0, false)));
        let loop2 = self.ctx.append_basic_block(fv, "chn.l2");
        let body2 = self.ctx.append_basic_block(fv, "chn.b2");
        let done = self.ctx.append_basic_block(fv, "chn.done");
        b!(self.bld.build_unconditional_branch(loop2));
        self.bld.position_at_end(loop2);
        let idx2 = b!(self.bld.build_load(i64t, idx_ptr2, "chn.i2")).into_int_value();
        let cond2 = b!(self.bld.build_int_compare(IntPredicate::SLT, idx2, len_b, "chn.c2"));
        b!(self.bld.build_conditional_branch(cond2, body2, done));
        self.bld.position_at_end(body2);
        let gep2 = unsafe { b!(self.bld.build_gep(lty, data_b, &[idx2], "chn.g2")) };
        let elem2 = b!(self.bld.build_load(lty, gep2, "chn.e2"));
        self.vec_push_raw(out_hdr, elem2, lty, elem_size)?;
        let next2 = b!(self.bld.build_int_nsw_add(idx2, i64t.const_int(1, false), "chn.n2"));
        b!(self.bld.build_store(idx_ptr2, next2));
        b!(self.bld.build_unconditional_branch(loop2));

        self.bld.position_at_end(done);
        Ok(out_hdr.into())
    }

    fn vec_enumerate(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let tuple_lty = self.ctx.struct_type(&[i64t.into(), lty.into()], false);
        let tuple_size = self.type_store_size(tuple_lty.into());
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;
        let idx_ptr = self.entry_alloca(i64t.into(), "enum.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "enum.loop");
        let body_bb = self.ctx.append_basic_block(fv, "enum.body");
        let done_bb = self.ctx.append_basic_block(fv, "enum.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "enum.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, len, "enum.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "enum.gep")) };
        let elem = b!(self.bld.build_load(lty, gep, "enum.elem"));
        let mut tup = tuple_lty.get_undef();
        tup = b!(self.bld.build_insert_value(tup, idx, 0, "enum.t0")).into_struct_value();
        tup = b!(self.bld.build_insert_value(tup, elem, 1, "enum.t1")).into_struct_value();
        self.vec_push_raw(out_hdr, tup.into(), tuple_lty.into(), tuple_size)?;
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "enum.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    fn vec_flatten(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // elem_ty should be Vec<inner>
        let inner_ty = match elem_ty {
            Type::Vec(inner) => inner.as_ref().clone(),
            _ => return Err("flatten requires Vec<Vec<T>>".into()),
        };
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let inner_lty = self.llvm_ty(&inner_ty);
        let inner_size = self.type_store_size(inner_lty);
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;
        let outer_idx_ptr = self.entry_alloca(i64t.into(), "flat.oidx");
        b!(self.bld.build_store(outer_idx_ptr, i64t.const_int(0, false)));

        let outer_loop = self.ctx.append_basic_block(fv, "flat.oloop");
        let outer_body = self.ctx.append_basic_block(fv, "flat.obody");
        let done_bb = self.ctx.append_basic_block(fv, "flat.done");
        b!(self.bld.build_unconditional_branch(outer_loop));

        self.bld.position_at_end(outer_loop);
        let oidx = b!(self.bld.build_load(i64t, outer_idx_ptr, "flat.oi")).into_int_value();
        let ocond = b!(self.bld.build_int_compare(IntPredicate::SLT, oidx, len, "flat.oc"));
        b!(self.bld.build_conditional_branch(ocond, outer_body, done_bb));

        self.bld.position_at_end(outer_body);
        // Each element is a ptr to vec header
        let inner_gep = unsafe { b!(self.bld.build_gep(ptr_ty, data_ptr, &[oidx], "flat.ig")) };
        let inner_hdr = b!(self.bld.build_load(ptr_ty, inner_gep, "flat.ih")).into_pointer_value();
        let (inner_data, inner_len) = self.vec_data_and_len(inner_hdr)?;

        let inner_idx_ptr = self.entry_alloca(i64t.into(), "flat.iidx");
        b!(self.bld.build_store(inner_idx_ptr, i64t.const_int(0, false)));
        let inner_loop = self.ctx.append_basic_block(fv, "flat.iloop");
        let inner_body = self.ctx.append_basic_block(fv, "flat.ibody");
        let inner_done = self.ctx.append_basic_block(fv, "flat.idone");
        b!(self.bld.build_unconditional_branch(inner_loop));

        self.bld.position_at_end(inner_loop);
        let iidx = b!(self.bld.build_load(i64t, inner_idx_ptr, "flat.ii")).into_int_value();
        let icond = b!(self.bld.build_int_compare(IntPredicate::SLT, iidx, inner_len, "flat.ic"));
        b!(self.bld.build_conditional_branch(icond, inner_body, inner_done));

        self.bld.position_at_end(inner_body);
        let elem_gep = unsafe { b!(self.bld.build_gep(inner_lty, inner_data, &[iidx], "flat.eg")) };
        let elem = b!(self.bld.build_load(inner_lty, elem_gep, "flat.e"));
        self.vec_push_raw(out_hdr, elem, inner_lty, inner_size)?;
        let inext = b!(self.bld.build_int_nsw_add(iidx, i64t.const_int(1, false), "flat.in"));
        b!(self.bld.build_store(inner_idx_ptr, inext));
        b!(self.bld.build_unconditional_branch(inner_loop));

        self.bld.position_at_end(inner_done);
        let onext = b!(self.bld.build_int_nsw_add(oidx, i64t.const_int(1, false), "flat.on"));
        b!(self.bld.build_store(outer_idx_ptr, onext));
        b!(self.bld.build_unconditional_branch(outer_loop));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    fn vec_contains(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let needle = self.compile_expr(&args[0])?;
        let i64t = self.ctx.i64_type();
        let bool_ty = self.ctx.bool_type();
        let lty = self.llvm_ty(elem_ty);
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let result_ptr = self.entry_alloca(bool_ty.into(), "cont.res");
        b!(self.bld.build_store(result_ptr, bool_ty.const_int(0, false)));
        let idx_ptr = self.entry_alloca(i64t.into(), "cont.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "cont.loop");
        let body_bb = self.ctx.append_basic_block(fv, "cont.body");
        let done_bb = self.ctx.append_basic_block(fv, "cont.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "cont.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, len, "cont.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "cont.gep")) };
        let elem = b!(self.bld.build_load(lty, gep, "cont.elem"));
        let eq = match elem_ty {
            Type::F64 => {
                b!(self.bld.build_float_compare(
                    inkwell::FloatPredicate::OEQ,
                    elem.into_float_value(),
                    needle.into_float_value(),
                    "cont.eq"
                )).into()
            }
            _ => {
                b!(self.bld.build_int_compare(
                    IntPredicate::EQ,
                    elem.into_int_value(),
                    needle.into_int_value(),
                    "cont.eq"
                )).into()
            }
        };
        let found_bb = self.ctx.append_basic_block(fv, "cont.found");
        let cont_bb = self.ctx.append_basic_block(fv, "cont.cont");
        b!(self.bld.build_conditional_branch(eq, found_bb, cont_bb));

        self.bld.position_at_end(found_bb);
        b!(self.bld.build_store(result_ptr, bool_ty.const_int(1, false)));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "cont.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(bool_ty, result_ptr, "cont.v")))
    }

    fn vec_reverse(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;
        let out_hdr = self.vec_alloc_empty()?;

        // Iterate from len-1 down to 0
        let idx_ptr = self.entry_alloca(i64t.into(), "rev.idx");
        let start = b!(self.bld.build_int_nsw_sub(len, i64t.const_int(1, false), "rev.start"));
        b!(self.bld.build_store(idx_ptr, start));

        let loop_bb = self.ctx.append_basic_block(fv, "rev.loop");
        let body_bb = self.ctx.append_basic_block(fv, "rev.body");
        let done_bb = self.ctx.append_basic_block(fv, "rev.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "rev.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SGE, idx, i64t.const_int(0, false), "rev.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "rev.gep")) };
        let elem = b!(self.bld.build_load(lty, gep, "rev.elem"));
        self.vec_push_raw(out_hdr, elem, lty, elem_size)?;
        let next = b!(self.bld.build_int_nsw_sub(idx, i64t.const_int(1, false), "rev.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(out_hdr.into())
    }

    fn vec_sort(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Simple insertion sort (copies to new vec first)
        let i64t = self.ctx.i64_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        let fv = self.cur_fn.unwrap();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        // Copy to new vec first
        let out_hdr = self.vec_alloc_empty()?;
        let cp_idx = self.entry_alloca(i64t.into(), "sort.ci");
        b!(self.bld.build_store(cp_idx, i64t.const_int(0, false)));
        let cp_loop = self.ctx.append_basic_block(fv, "sort.cp");
        let cp_body = self.ctx.append_basic_block(fv, "sort.cpb");
        let cp_done = self.ctx.append_basic_block(fv, "sort.cpd");
        b!(self.bld.build_unconditional_branch(cp_loop));
        self.bld.position_at_end(cp_loop);
        let ci = b!(self.bld.build_load(i64t, cp_idx, "sort.ci")).into_int_value();
        let cc = b!(self.bld.build_int_compare(IntPredicate::SLT, ci, len, "sort.cc"));
        b!(self.bld.build_conditional_branch(cc, cp_body, cp_done));
        self.bld.position_at_end(cp_body);
        let gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[ci], "sort.cg")) };
        let elem = b!(self.bld.build_load(lty, gep, "sort.ce"));
        self.vec_push_raw(out_hdr, elem, lty, elem_size)?;
        let cn = b!(self.bld.build_int_nsw_add(ci, i64t.const_int(1, false), "sort.cn"));
        b!(self.bld.build_store(cp_idx, cn));
        b!(self.bld.build_unconditional_branch(cp_loop));

        self.bld.position_at_end(cp_done);
        // In-place insertion sort on out_hdr
        let header_ty = self.vec_header_type();
        let out_ptr_gep = b!(self.bld.build_struct_gep(header_ty, out_hdr, 0, "sort.ptrp"));
        let out_data = b!(self.bld.build_load(self.ctx.ptr_type(AddressSpace::default()), out_ptr_gep, "sort.data")).into_pointer_value();

        let i_ptr = self.entry_alloca(i64t.into(), "sort.i");
        b!(self.bld.build_store(i_ptr, i64t.const_int(1, false)));
        let outer_loop = self.ctx.append_basic_block(fv, "sort.ol");
        let outer_body = self.ctx.append_basic_block(fv, "sort.ob");
        let sort_done = self.ctx.append_basic_block(fv, "sort.done");
        b!(self.bld.build_unconditional_branch(outer_loop));

        self.bld.position_at_end(outer_loop);
        let i = b!(self.bld.build_load(i64t, i_ptr, "sort.i")).into_int_value();
        let ic = b!(self.bld.build_int_compare(IntPredicate::SLT, i, len, "sort.ic"));
        b!(self.bld.build_conditional_branch(ic, outer_body, sort_done));

        self.bld.position_at_end(outer_body);
        let key_gep = unsafe { b!(self.bld.build_gep(lty, out_data, &[i], "sort.kg")) };
        let key = b!(self.bld.build_load(lty, key_gep, "sort.key"));
        let j_ptr = self.entry_alloca(i64t.into(), "sort.j");
        let j_start = b!(self.bld.build_int_nsw_sub(i, i64t.const_int(1, false), "sort.js"));
        b!(self.bld.build_store(j_ptr, j_start));

        let inner_loop = self.ctx.append_basic_block(fv, "sort.il");
        let inner_body = self.ctx.append_basic_block(fv, "sort.ib");
        let inner_done = self.ctx.append_basic_block(fv, "sort.id");
        b!(self.bld.build_unconditional_branch(inner_loop));

        self.bld.position_at_end(inner_loop);
        let j = b!(self.bld.build_load(i64t, j_ptr, "sort.j")).into_int_value();
        let jc = b!(self.bld.build_int_compare(IntPredicate::SGE, j, i64t.const_int(0, false), "sort.jc"));
        b!(self.bld.build_conditional_branch(jc, inner_body, inner_done));

        self.bld.position_at_end(inner_body);
        let aj_gep = unsafe { b!(self.bld.build_gep(lty, out_data, &[j], "sort.ag")) };
        let aj = b!(self.bld.build_load(lty, aj_gep, "sort.aj"));
        let gt = match elem_ty {
            Type::F64 => b!(self.bld.build_float_compare(
                inkwell::FloatPredicate::OGT, aj.into_float_value(), key.into_float_value(), "sort.gt"
            )).into(),
            _ => b!(self.bld.build_int_compare(IntPredicate::SGT, aj.into_int_value(), key.into_int_value(), "sort.gt")).into(),
        };
        let shift_bb = self.ctx.append_basic_block(fv, "sort.shift");
        b!(self.bld.build_conditional_branch(gt, shift_bb, inner_done));

        self.bld.position_at_end(shift_bb);
        // a[j+1] = a[j]
        let j1 = b!(self.bld.build_int_nsw_add(j, i64t.const_int(1, false), "sort.j1"));
        let dst_gep = unsafe { b!(self.bld.build_gep(lty, out_data, &[j1], "sort.dg")) };
        b!(self.bld.build_store(dst_gep, aj));
        let jn = b!(self.bld.build_int_nsw_sub(j, i64t.const_int(1, false), "sort.jn"));
        b!(self.bld.build_store(j_ptr, jn));
        b!(self.bld.build_unconditional_branch(inner_loop));

        self.bld.position_at_end(inner_done);
        // a[j+1] = key
        let j_final = b!(self.bld.build_load(i64t, j_ptr, "sort.jf")).into_int_value();
        let j1f = b!(self.bld.build_int_nsw_add(j_final, i64t.const_int(1, false), "sort.j1f"));
        let insert_gep = unsafe { b!(self.bld.build_gep(lty, out_data, &[j1f], "sort.ig")) };
        b!(self.bld.build_store(insert_gep, key));

        let in_ = b!(self.bld.build_int_nsw_add(i, i64t.const_int(1, false), "sort.in"));
        b!(self.bld.build_store(i_ptr, in_));
        b!(self.bld.build_unconditional_branch(outer_loop));

        self.bld.position_at_end(sort_done);
        Ok(out_hdr.into())
    }

    pub(crate) fn drop_vec(
        &mut self,
        header_alloca: inkwell::values::PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let free = self.ensure_free();
        let fv = self.cur_fn.unwrap();
        let null = ptr_ty.const_null();
        let header_ptr =
            b!(self.bld.build_load(ptr_ty, header_alloca, "dv.hdr")).into_pointer_value();
        let is_null =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::EQ, header_ptr, null, "dv.null"));
        let free_bb = self.ctx.append_basic_block(fv, "dv.free");
        let done_bb = self.ctx.append_basic_block(fv, "dv.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, free_bb));
        self.bld.position_at_end(free_bb);
        let data_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "dv.data"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dv.buf"));
        b!(self.bld.build_call(free, &[data_ptr.into()], ""));
        b!(self.bld.build_call(free, &[header_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(done_bb));
        self.bld.position_at_end(done_bb);
        Ok(())
    }

    fn vec_join(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("join() requires a separator argument".into());
        }
        let sep = self.compile_expr(&args[0])?;
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let st = self.string_type();

        let (data_ptr, len) = self.vec_data_and_len(header_ptr)?;

        let empty_bb = self.ctx.append_basic_block(fv, "jn.empty");
        let start_bb = self.ctx.append_basic_block(fv, "jn.start");
        let merge_bb = self.ctx.append_basic_block(fv, "jn.merge");

        let is_empty = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::SLE,
            len,
            i64t.const_int(0, false),
            "jn.isempty"
        ));
        b!(self.bld.build_conditional_branch(is_empty, empty_bb, start_bb));

        // Empty vec → empty string
        self.bld.position_at_end(empty_bb);
        let empty_str = self.compile_str_literal("")?;
        let empty_exit = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));

        // Non-empty: start with first element
        self.bld.position_at_end(start_bb);
        let first = b!(self.bld.build_load(st, data_ptr, "jn.first"));
        let cond_bb = self.ctx.append_basic_block(fv, "jn.cond");
        let body_bb = self.ctx.append_basic_block(fv, "jn.body");
        let done_bb = self.ctx.append_basic_block(fv, "jn.done");

        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(cond_bb);
        let phi_i = b!(self.bld.build_phi(i64t, "jn.i"));
        phi_i.add_incoming(&[(&i64t.const_int(1, false), start_bb)]);
        let phi_acc = b!(self.bld.build_phi(st, "jn.acc"));
        phi_acc.add_incoming(&[(&first, start_bb)]);
        let i = phi_i.as_basic_value().into_int_value();
        let acc = phi_acc.as_basic_value();
        let done = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::SGE,
            i,
            len,
            "jn.done"
        ));
        b!(self.bld.build_conditional_branch(done, done_bb, body_bb));

        self.bld.position_at_end(body_bb);
        let elem_ptr = unsafe {
            b!(self
                .bld
                .build_gep(st, data_ptr, &[i], "jn.ep"))
        };
        let elem = b!(self.bld.build_load(st, elem_ptr, "jn.elem"));
        // acc = acc + sep + elem
        let with_sep = self.string_concat(acc, sep)?;
        let with_elem = self.string_concat(with_sep, elem)?;
        let next_i = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "jn.ni"));
        let body_exit = self.bld.get_insert_block().unwrap();
        phi_i.add_incoming(&[(&next_i, body_exit)]);
        phi_acc.add_incoming(&[(&with_elem, body_exit)]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(done_bb);
        let result = acc;
        let done_exit = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(st, "jn.v"));
        phi.add_incoming(&[(&empty_str, empty_exit), (&result, done_exit)]);
        Ok(phi.as_basic_value())
    }

    /// Inline linear scan for `x in [a, b, c]` on fixed-size arrays.
    fn array_contains(
        &mut self,
        obj: &hir::Expr,
        elem_ty: &Type,
        arr_len: usize,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let needle = self.compile_expr(&args[0])?;
        let obj_val = self.compile_expr(obj)?;
        let arr_ptr = obj_val.into_pointer_value();
        let lty = self.llvm_ty(elem_ty);
        let i64t = self.ctx.i64_type();
        let bool_ty = self.ctx.bool_type();
        let fv = self.cur_fn.unwrap();

        let result_ptr = self.entry_alloca(bool_ty.into(), "acont.res");
        b!(self.bld.build_store(result_ptr, bool_ty.const_int(0, false)));
        let idx_ptr = self.entry_alloca(i64t.into(), "acont.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        let len = i64t.const_int(arr_len as u64, false);
        let arr_ty = lty.array_type(arr_len as u32);

        let loop_bb = self.ctx.append_basic_block(fv, "acont.loop");
        let body_bb = self.ctx.append_basic_block(fv, "acont.body");
        let done_bb = self.ctx.append_basic_block(fv, "acont.done");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "acont.i")).into_int_value();
        let cond = b!(self.bld.build_int_compare(IntPredicate::SLT, idx, len, "acont.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let gep = unsafe {
            b!(self.bld.build_gep(
                arr_ty,
                arr_ptr,
                &[i64t.const_int(0, false), idx],
                "acont.gep"
            ))
        };
        let elem = b!(self.bld.build_load(lty, gep, "acont.elem"));
        let eq = match elem_ty {
            Type::F64 => b!(self.bld.build_float_compare(
                inkwell::FloatPredicate::OEQ,
                elem.into_float_value(),
                needle.into_float_value(),
                "acont.eq"
            ))
            .into(),
            _ => b!(self.bld.build_int_compare(
                IntPredicate::EQ,
                elem.into_int_value(),
                needle.into_int_value(),
                "acont.eq"
            ))
            .into(),
        };
        let found_bb = self.ctx.append_basic_block(fv, "acont.found");
        let cont_bb = self.ctx.append_basic_block(fv, "acont.cont");
        b!(self.bld.build_conditional_branch(eq, found_bb, cont_bb));

        self.bld.position_at_end(found_bb);
        b!(self.bld.build_store(result_ptr, bool_ty.const_int(1, false)));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "acont.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(b!(self.bld.build_load(bool_ty, result_ptr, "acont.v")))
    }
}
