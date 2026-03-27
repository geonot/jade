use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
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
