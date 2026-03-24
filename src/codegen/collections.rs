use inkwell::module::Linkage;
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
            _ => Type::I64,
        };
        let obj_val = self.compile_expr(obj)?;
        let header_ptr = obj_val.into_pointer_value();

        match method {
            "push" => self.vec_push(header_ptr, &elem_ty, args),
            "pop" => self.vec_pop(header_ptr, &elem_ty),
            "len" => self.vec_len(header_ptr),
            "get" => self.vec_get(header_ptr, &elem_ty, args),
            "set" => self.vec_set(header_ptr, &elem_ty, args),
            "remove" => self.vec_remove(header_ptr, &elem_ty, args),
            "clear" => self.vec_clear(header_ptr),
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
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let lty = self.llvm_ty(elem_ty);
        let elem_size = self.type_store_size(lty);
        let fv = self.cur_fn.unwrap();

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vp.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "vp.len")).into_int_value();
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "vp.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "vp.cap")).into_int_value();

        let needs_grow = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, len, cap, "vp.full"));
        let grow_bb = self.ctx.append_basic_block(fv, "vp.grow");
        let store_bb = self.ctx.append_basic_block(fv, "vp.store");
        b!(self
            .bld
            .build_conditional_branch(needs_grow, grow_bb, store_bb));

        self.bld.position_at_end(grow_bb);
        let doubled = b!(self
            .bld
            .build_int_nsw_mul(cap, i64t.const_int(2, false), "vp.dbl"));
        let new_cap_cmp = b!(self.bld.build_int_compare(
            IntPredicate::SGT,
            doubled,
            i64t.const_int(4, false),
            "vp.cmp"
        ));
        let new_cap =
            b!(self
                .bld
                .build_select(new_cap_cmp, doubled, i64t.const_int(4, false), "vp.nc"))
            .into_int_value();
        let new_size =
            b!(self
                .bld
                .build_int_nsw_mul(new_cap, i64t.const_int(elem_size, false), "vp.ns"));
        let realloc = self.ensure_realloc();
        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "vp.ptrp"));
        let old_ptr = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "vp.optr"
        ));
        let new_ptr =
            b!(self
                .bld
                .build_call(realloc, &[old_ptr.into(), new_size.into()], "vp.nptr"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        b!(self.bld.build_store(ptr_gep, new_ptr));
        b!(self.bld.build_store(cap_gep, new_cap));
        b!(self.bld.build_unconditional_branch(store_bb));

        self.bld.position_at_end(store_bb);
        let ptr_gep2 = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "vp.ptrp2"));
        let data_ptr = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep2,
            "vp.data"
        ))
        .into_pointer_value();
        let len2 = b!(self.bld.build_load(i64t, len_gep, "vp.len2")).into_int_value();
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[len2], "vp.egep")) };
        b!(self.bld.build_store(elem_gep, val));

        let new_len = b!(self
            .bld
            .build_int_nsw_add(len2, i64t.const_int(1, false), "vp.nl"));
        b!(self.bld.build_store(len_gep, new_len));

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

    fn vec_pop(
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

    fn vec_set(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.len() < 2 {
            return Err("set() requires index and value".into());
        }
        let idx = self.compile_expr(&args[0])?.into_int_value();
        let val = self.compile_expr(&args[1])?;
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

    fn vec_remove(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        elem_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("remove() requires an index".into());
        }
        let idx = self.compile_expr(&args[0])?.into_int_value();
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

    fn vec_clear(
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

    fn emit_vec_bounds_check(
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

    pub(crate) fn compile_map_new(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let malloc = self.ensure_malloc();

        let header_size = i64t.const_int(24, false);
        let header_ptr = b!(self
            .bld
            .build_call(malloc, &[header_size.into()], "map.hdr"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        let init_cap = 16u64;
        let entry_size = 48u64;
        let calloc = self.ensure_calloc();
        let buf = b!(self.bld.build_call(
            calloc,
            &[
                i64t.const_int(init_cap, false).into(),
                i64t.const_int(entry_size, false).into()
            ],
            "map.buf"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "map.ptr"));
        b!(self.bld.build_store(ptr_gep, buf));

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "map.len"));
        b!(self.bld.build_store(len_gep, i64t.const_int(0, false)));

        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "map.cap"));
        b!(self
            .bld
            .build_store(cap_gep, i64t.const_int(init_cap, false)));

        Ok(header_ptr.into())
    }

    pub(crate) fn compile_map_method(
        &mut self,
        obj: &hir::Expr,
        method: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.compile_expr(obj)?;
        let header_ptr = obj_val.into_pointer_value();

        match method {
            "len" => self.vec_len(header_ptr),
            "set" => self.map_set(header_ptr, obj, args),
            "get" => self.map_get(header_ptr, obj, args),
            "has" => self.map_has(header_ptr, obj, args),
            "remove" => self.map_remove(header_ptr, obj, args),
            "clear" => self.map_clear(header_ptr),
            _ => Err(format!("no method '{method}' on Map")),
        }
    }

    fn map_set(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        _obj: &hir::Expr,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.len() < 2 {
            return Err("map.set() requires key and value".into());
        }
        let key_val = self.compile_expr(&args[0])?;
        let val_val = self.compile_expr(&args[1])?;
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let fv = self.cur_fn.unwrap();

        let hash = self.fnv_hash_string(key_val)?;

        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "ms.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "ms.cap")).into_int_value();

        let mask = b!(self
            .bld
            .build_int_nsw_sub(cap, i64t.const_int(1, false), "ms.mask"));
        let start_idx = b!(self.bld.build_and(hash, mask, "ms.idx"));

        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "ms.ptrp"));
        let entries = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "ms.entries"
        ))
        .into_pointer_value();

        let entry_size = i64t.const_int(48, false);
        let i8t = self.ctx.i8_type();

        let loop_bb = self.ctx.append_basic_block(fv, "ms.loop");
        let found_bb = self.ctx.append_basic_block(fv, "ms.found");
        let empty_bb = self.ctx.append_basic_block(fv, "ms.empty");
        let done_bb = self.ctx.append_basic_block(fv, "ms.done");
        let pre_loop_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let phi_idx = b!(self.bld.build_phi(i64t, "ms.i"));
        phi_idx.add_incoming(&[(&start_idx, pre_loop_bb)]);
        let idx = phi_idx.as_basic_value().into_int_value();

        let byte_off = b!(self.bld.build_int_nsw_mul(idx, entry_size, "ms.off"));
        let entry_ptr = unsafe { b!(self.bld.build_gep(i8t, entries, &[byte_off], "ms.eptr")) };

        let occ_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(40, false)], "ms.occp"))
        };
        let occ = b!(self.bld.build_load(i8t, occ_ptr, "ms.occ")).into_int_value();
        let is_occupied = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            occ,
            i8t.const_int(0, false),
            "ms.isocc"
        ));
        b!(self
            .bld
            .build_conditional_branch(is_occupied, found_bb, empty_bb));

        self.bld.position_at_end(found_bb);
        let stored_hash_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(0, false)], "ms.shp"))
        };
        let stored_hash = b!(self.bld.build_load(i64t, stored_hash_ptr, "ms.sh")).into_int_value();
        let hash_eq = b!(self
            .bld
            .build_int_compare(IntPredicate::EQ, stored_hash, hash, "ms.heq"));

        let overwrite_bb = self.ctx.append_basic_block(fv, "ms.overwrite");
        let next_bb = self.ctx.append_basic_block(fv, "ms.next");
        b!(self
            .bld
            .build_conditional_branch(hash_eq, overwrite_bb, next_bb));

        self.bld.position_at_end(overwrite_bb);
        let val_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(32, false)], "ms.vp"))
        };
        b!(self.bld.build_store(val_ptr, val_val));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "ms.ni"));
        let wrapped = b!(self.bld.build_and(next_idx, mask, "ms.wi"));
        phi_idx.add_incoming(&[(&wrapped, next_bb)]);
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(empty_bb);
        let hash_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(0, false)], "ms.hp2"))
        };
        b!(self.bld.build_store(hash_ptr, hash));
        let key_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(8, false)], "ms.kp"))
        };
        let memcpy = self.ensure_memcpy();
        let key_alloca = self.entry_alloca(self.string_type().into(), "ms.ktmp");
        b!(self.bld.build_store(key_alloca, key_val));
        b!(self.bld.build_call(
            memcpy,
            &[
                key_ptr.into(),
                key_alloca.into(),
                i64t.const_int(24, false).into()
            ],
            ""
        ));
        let val_ptr2 = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(32, false)], "ms.vp2"))
        };
        b!(self.bld.build_store(val_ptr2, val_val));
        b!(self.bld.build_store(occ_ptr, i8t.const_int(1, false)));
        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "ms.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "ms.len")).into_int_value();
        let new_len = b!(self
            .bld
            .build_int_nsw_add(len, i64t.const_int(1, false), "ms.nl"));
        b!(self.bld.build_store(len_gep, new_len));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(done_bb);
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    fn map_get(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        _obj: &hir::Expr,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("map.get() requires a key".into());
        }
        let key_val = self.compile_expr(&args[0])?;
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let header_ty = self.vec_header_type();
        let fv = self.cur_fn.unwrap();

        let hash = self.fnv_hash_string(key_val)?;
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "mg.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "mg.cap")).into_int_value();
        let mask = b!(self
            .bld
            .build_int_nsw_sub(cap, i64t.const_int(1, false), "mg.mask"));
        let start_idx = b!(self.bld.build_and(hash, mask, "mg.idx"));

        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "mg.ptrp"));
        let entries = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "mg.entries"
        ))
        .into_pointer_value();

        let entry_size = i64t.const_int(48, false);
        let loop_bb = self.ctx.append_basic_block(fv, "mg.loop");
        let check_bb = self.ctx.append_basic_block(fv, "mg.check");
        let found_bb = self.ctx.append_basic_block(fv, "mg.found");
        let notfound_bb = self.ctx.append_basic_block(fv, "mg.nf");
        let next_bb = self.ctx.append_basic_block(fv, "mg.next");
        let merge_bb = self.ctx.append_basic_block(fv, "mg.merge");

        let pre_loop_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(loop_bb);
        let phi_idx = b!(self.bld.build_phi(i64t, "mg.i"));
        phi_idx.add_incoming(&[(&start_idx, pre_loop_bb)]);
        let idx = phi_idx.as_basic_value().into_int_value();

        let byte_off = b!(self.bld.build_int_nsw_mul(idx, entry_size, "mg.off"));
        let entry_ptr = unsafe { b!(self.bld.build_gep(i8t, entries, &[byte_off], "mg.eptr")) };
        let occ_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(40, false)], "mg.occp"))
        };
        let occ = b!(self.bld.build_load(i8t, occ_ptr, "mg.occ")).into_int_value();
        let is_occ =
            b!(self
                .bld
                .build_int_compare(IntPredicate::NE, occ, i8t.const_int(0, false), "mg.io"));
        b!(self
            .bld
            .build_conditional_branch(is_occ, check_bb, notfound_bb));

        self.bld.position_at_end(check_bb);
        let stored_hash_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(0, false)], "mg.shp"))
        };
        let stored_hash = b!(self.bld.build_load(i64t, stored_hash_ptr, "mg.sh")).into_int_value();
        let hash_eq = b!(self
            .bld
            .build_int_compare(IntPredicate::EQ, stored_hash, hash, "mg.heq"));
        b!(self
            .bld
            .build_conditional_branch(hash_eq, found_bb, next_bb));

        self.bld.position_at_end(found_bb);
        let val_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(32, false)], "mg.vp"))
        };
        let found_val = b!(self.bld.build_load(i64t, val_ptr, "mg.fv"));
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(notfound_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "mg.ni"));
        let wrapped = b!(self.bld.build_and(next_idx, mask, "mg.wi"));
        phi_idx.add_incoming(&[(&wrapped, next_bb)]);
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(i64t, "mg.v"));
        phi.add_incoming(&[
            (&found_val, found_bb),
            (&i64t.const_int(0, false), notfound_bb),
        ]);
        Ok(phi.as_basic_value())
    }

    fn map_has(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        _obj: &hir::Expr,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("map.has() requires a key".into());
        }
        let key_val = self.compile_expr(&args[0])?;
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let header_ty = self.vec_header_type();
        let fv = self.cur_fn.unwrap();

        let hash = self.fnv_hash_string(key_val)?;
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "mh.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "mh.cap")).into_int_value();
        let mask = b!(self
            .bld
            .build_int_nsw_sub(cap, i64t.const_int(1, false), "mh.mask"));
        let start_idx = b!(self.bld.build_and(hash, mask, "mh.idx"));
        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "mh.ptrp"));
        let entries = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "mh.entries"
        ))
        .into_pointer_value();
        let entry_size = i64t.const_int(48, false);

        let loop_bb = self.ctx.append_basic_block(fv, "mh.loop");
        let check_bb = self.ctx.append_basic_block(fv, "mh.check");
        let found_bb = self.ctx.append_basic_block(fv, "mh.found");
        let nf_bb = self.ctx.append_basic_block(fv, "mh.nf");
        let next_bb = self.ctx.append_basic_block(fv, "mh.next");
        let merge_bb = self.ctx.append_basic_block(fv, "mh.merge");

        let pre_loop_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(loop_bb);
        let phi_idx = b!(self.bld.build_phi(i64t, "mh.i"));
        phi_idx.add_incoming(&[(&start_idx, pre_loop_bb)]);
        let idx = phi_idx.as_basic_value().into_int_value();
        let byte_off = b!(self.bld.build_int_nsw_mul(idx, entry_size, "mh.off"));
        let entry_ptr = unsafe { b!(self.bld.build_gep(i8t, entries, &[byte_off], "mh.eptr")) };
        let occ_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(40, false)], "mh.occp"))
        };
        let occ = b!(self.bld.build_load(i8t, occ_ptr, "mh.occ")).into_int_value();
        let is_occ =
            b!(self
                .bld
                .build_int_compare(IntPredicate::NE, occ, i8t.const_int(0, false), "mh.io"));
        b!(self.bld.build_conditional_branch(is_occ, check_bb, nf_bb));

        self.bld.position_at_end(check_bb);
        let shp = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(0, false)], "mh.shp"))
        };
        let sh = b!(self.bld.build_load(i64t, shp, "mh.sh")).into_int_value();
        let heq = b!(self
            .bld
            .build_int_compare(IntPredicate::EQ, sh, hash, "mh.heq"));
        b!(self.bld.build_conditional_branch(heq, found_bb, next_bb));

        self.bld.position_at_end(found_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));
        self.bld.position_at_end(nf_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));
        self.bld.position_at_end(next_bb);
        let ni = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "mh.ni"));
        let wi = b!(self.bld.build_and(ni, mask, "mh.wi"));
        phi_idx.add_incoming(&[(&wi, next_bb)]);
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(merge_bb);
        let bool_t = self.ctx.bool_type();
        let phi = b!(self.bld.build_phi(bool_t, "mh.v"));
        phi.add_incoming(&[
            (&bool_t.const_int(1, false), found_bb),
            (&bool_t.const_int(0, false), nf_bb),
        ]);
        Ok(phi.as_basic_value())
    }

    fn map_remove(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        _obj: &hir::Expr,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("map.remove() requires a key".into());
        }
        let key_val = self.compile_expr(&args[0])?;
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let header_ty = self.vec_header_type();
        let fv = self.cur_fn.unwrap();

        let hash = self.fnv_hash_string(key_val)?;
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "mr.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "mr.cap")).into_int_value();
        let mask = b!(self
            .bld
            .build_int_nsw_sub(cap, i64t.const_int(1, false), "mr.mask"));
        let start_idx = b!(self.bld.build_and(hash, mask, "mr.idx"));
        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "mr.ptrp"));
        let entries = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "mr.entries"
        ))
        .into_pointer_value();
        let entry_size = i64t.const_int(48, false);

        let loop_bb = self.ctx.append_basic_block(fv, "mr.loop");
        let check_bb = self.ctx.append_basic_block(fv, "mr.check");
        let found_bb = self.ctx.append_basic_block(fv, "mr.found");
        let nf_bb = self.ctx.append_basic_block(fv, "mr.nf");
        let next_bb = self.ctx.append_basic_block(fv, "mr.next");

        let pre_loop_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(loop_bb);
        let phi_idx = b!(self.bld.build_phi(i64t, "mr.i"));
        phi_idx.add_incoming(&[(&start_idx, pre_loop_bb)]);
        let idx = phi_idx.as_basic_value().into_int_value();
        let byte_off = b!(self.bld.build_int_nsw_mul(idx, entry_size, "mr.off"));
        let entry_ptr = unsafe { b!(self.bld.build_gep(i8t, entries, &[byte_off], "mr.eptr")) };
        let occ_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(40, false)], "mr.occp"))
        };
        let occ = b!(self.bld.build_load(i8t, occ_ptr, "mr.occ")).into_int_value();
        let is_occ =
            b!(self
                .bld
                .build_int_compare(IntPredicate::NE, occ, i8t.const_int(0, false), "mr.io"));
        b!(self.bld.build_conditional_branch(is_occ, check_bb, nf_bb));

        self.bld.position_at_end(check_bb);
        let shp = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(0, false)], "mr.shp"))
        };
        let sh = b!(self.bld.build_load(i64t, shp, "mr.sh")).into_int_value();
        let heq = b!(self
            .bld
            .build_int_compare(IntPredicate::EQ, sh, hash, "mr.heq"));
        b!(self.bld.build_conditional_branch(heq, found_bb, next_bb));

        self.bld.position_at_end(found_bb);
        b!(self.bld.build_store(occ_ptr, i8t.const_int(0, false)));
        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "mr.lenp"));
        let cur_len = b!(self.bld.build_load(i64t, len_gep, "mr.len")).into_int_value();
        let new_len = b!(self
            .bld
            .build_int_nsw_sub(cur_len, i64t.const_int(1, false), "mr.nl"));
        b!(self.bld.build_store(len_gep, new_len));
        b!(self.bld.build_unconditional_branch(nf_bb));

        self.bld.position_at_end(next_bb);
        let ni = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "mr.ni"));
        let wi = b!(self.bld.build_and(ni, mask, "mr.wi"));
        phi_idx.add_incoming(&[(&wi, next_bb)]);
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(nf_bb);
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    fn map_clear(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();

        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "mc.ptrp"));
        let entries = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "mc.entries"
        ))
        .into_pointer_value();
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "mc.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "mc.cap")).into_int_value();
        let total = b!(self
            .bld
            .build_int_nsw_mul(cap, i64t.const_int(48, false), "mc.total"));
        let memset = self.ensure_memset();
        let zero_i32 = self.ctx.i32_type().const_int(0, false);
        b!(self
            .bld
            .build_call(memset, &[entries.into(), zero_i32.into(), total.into()], ""));

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "mc.lenp"));
        b!(self.bld.build_store(len_gep, i64t.const_int(0, false)));

        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    fn fnv_hash_string(
        &mut self,
        str_val: BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let fv = self.cur_fn.unwrap();

        let data = self.string_data(str_val)?.into_pointer_value();
        let len = self.string_len(str_val)?.into_int_value();

        let basis = i64t.const_int(0xcbf29ce484222325, false);
        let prime = i64t.const_int(0x00000100000001B3, false);

        let cond_bb = self.ctx.append_basic_block(fv, "fnv.cond");
        let body_bb = self.ctx.append_basic_block(fv, "fnv.body");
        let done_bb = self.ctx.append_basic_block(fv, "fnv.done");
        let entry_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(cond_bb);
        let phi_hash = b!(self.bld.build_phi(i64t, "fnv.h"));
        phi_hash.add_incoming(&[(&basis, entry_bb)]);
        let phi_i = b!(self.bld.build_phi(i64t, "fnv.i"));
        phi_i.add_incoming(&[(&i64t.const_int(0, false), entry_bb)]);
        let cur_i = phi_i.as_basic_value().into_int_value();
        let cur_hash = phi_hash.as_basic_value().into_int_value();
        let done = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, cur_i, len, "fnv.done"));
        b!(self.bld.build_conditional_branch(done, done_bb, body_bb));

        self.bld.position_at_end(body_bb);
        let byte_ptr = unsafe { b!(self.bld.build_gep(i8t, data, &[cur_i], "fnv.bp")) };
        let byte = b!(self.bld.build_load(i8t, byte_ptr, "fnv.byte")).into_int_value();
        let ext = b!(self.bld.build_int_z_extend(byte, i64t, "fnv.ext"));
        let xored = b!(self.bld.build_xor(cur_hash, ext, "fnv.xor"));
        let mulled = b!(self.bld.build_int_nsw_mul(xored, prime, "fnv.mul"));
        let next_i = b!(self
            .bld
            .build_int_nsw_add(cur_i, i64t.const_int(1, false), "fnv.ni"));
        phi_hash.add_incoming(&[(&mulled, body_bb)]);
        phi_i.add_incoming(&[(&next_i, body_bb)]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(done_bb);
        Ok(phi_hash.as_basic_value().into_int_value())
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

    pub(crate) fn drop_map(
        &mut self,
        header_alloca: inkwell::values::PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let free = self.ensure_free();
        let fv = self.cur_fn.unwrap();
        let null = ptr_ty.const_null();
        let header_ptr =
            b!(self.bld.build_load(ptr_ty, header_alloca, "dm.hdr")).into_pointer_value();
        let is_null =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::EQ, header_ptr, null, "dm.null"));
        let free_bb = self.ctx.append_basic_block(fv, "dm.free");
        let done_bb = self.ctx.append_basic_block(fv, "dm.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, free_bb));
        self.bld.position_at_end(free_bb);
        let data_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "dm.data"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dm.buf"));
        b!(self.bld.build_call(free, &[data_ptr.into()], ""));
        b!(self.bld.build_call(free, &[header_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(done_bb));
        self.bld.position_at_end(done_bb);
        Ok(())
    }
}
