//! Container and runtime-object drop helpers.

use super::*;

impl<'ctx> Compiler<'ctx> {
    /// Drop a Vec, recursively destroying elements if they are non-trivially
    /// droppable. O(n) in element count, O(1) for POD elements.
    pub(in crate::codegen::drop) fn drop_vec_deep(
        &mut self,
        val: BasicValueEnum<'ctx>,
        elem: &Type,
    ) -> Result<(), String> {
        self.drop_vec_impl(val, elem, true)
    }

    /// Drop only the *elements* of a Vec — frees per-element heap if elements
    /// are non-trivially-droppable, but **leaves the data buffer and header
    /// allocations intact** so they can be recycled by Perceus reuse pairing.
    /// The caller is responsible for stashing or freeing the header pointer.
    pub(crate) fn drop_vec_elements_only(
        &mut self,
        val: BasicValueEnum<'ctx>,
        elem: &Type,
    ) -> Result<(), String> {
        self.drop_vec_impl(val, elem, false)
    }

    fn drop_vec_impl(
        &mut self,
        val: BasicValueEnum<'ctx>,
        elem: &Type,
        free_storage: bool,
    ) -> Result<(), String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let i64t = self.ctx.i64_type();
        let fv = self.current_fn();
        let free = self.ensure_free();

        // val is the header pointer itself (already loaded from alloca)
        let header_ptr = val.into_pointer_value();

        let null = ptr_ty.const_null();
        let is_null =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::EQ, header_ptr, null, "dvd.null"));
        let drop_bb = self.ctx.append_basic_block(fv, "dvd.drop");
        let done_bb = self.ctx.append_basic_block(fv, "dvd.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, drop_bb));
        self.bld.position_at_end(drop_bb);

        // If elements need dropping, iterate and drop each
        if !elem.is_trivially_droppable() {
            let data_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "dvd.d"));
            let data_ptr =
                b!(self.bld.build_load(ptr_ty, data_gep, "dvd.buf")).into_pointer_value();
            let len_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 1, "dvd.l"));
            let len = b!(self.bld.build_load(i64t, len_gep, "dvd.len")).into_int_value();

            let elem_llvm = self.llvm_ty(elem);
            let elem_size = elem_llvm.size_of().expect("ICE: type has no size");

            // Loop: for i in 0..len { drop_value(data[i]) }
            let loop_bb = self.ctx.append_basic_block(fv, "dvd.loop");
            let body_bb = self.ctx.append_basic_block(fv, "dvd.body");
            let post_bb = self.ctx.append_basic_block(fv, "dvd.post");

            b!(self.bld.build_unconditional_branch(loop_bb));
            self.bld.position_at_end(loop_bb);
            let phi = b!(self.bld.build_phi(i64t, "dvd.i"));
            phi.add_incoming(&[(&i64t.const_int(0, false), drop_bb)]);
            let i = phi.as_basic_value().into_int_value();
            let cond =
                b!(self
                    .bld
                    .build_int_compare(inkwell::IntPredicate::ULT, i, len, "dvd.cmp"));
            b!(self.bld.build_conditional_branch(cond, body_bb, post_bb));

            self.bld.position_at_end(body_bb);
            let offset = b!(self.bld.build_int_mul(i, elem_size, "dvd.off"));
            let elem_ptr = unsafe {
                b!(self
                    .bld
                    .build_gep(self.ctx.i8_type(), data_ptr, &[offset], "dvd.ep"))
            };
            let elem_val = b!(self.bld.build_load(elem_llvm, elem_ptr, "dvd.ev"));
            self.drop_value(elem_val, elem)?;
            // After drop_value, the builder may be in a different BB (e.g., if
            // the element type has its own control flow). Use the actual current
            // block for the phi incoming edge.
            let after_drop_bb = self.current_bb();
            let next = b!(self
                .bld
                .build_int_add(i, i64t.const_int(1, false), "dvd.next"));
            phi.add_incoming(&[(&next, after_drop_bb)]);
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(post_bb);
            // Free data buffer and header
            if free_storage {
                let data_gep2 = b!(self
                    .bld
                    .build_struct_gep(header_ty, header_ptr, 0, "dvd.d2"));
                let data_ptr2 = b!(self.bld.build_load(ptr_ty, data_gep2, "dvd.buf2"));
                b!(self.bld.build_call(free, &[data_ptr2.into()], ""));
                b!(self.bld.build_call(free, &[header_ptr.into()], ""));
            }
        } else {
            // POD elements: just free buffer and header
            if free_storage {
                let data_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "dvd.d"));
                let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dvd.buf"));
                b!(self.bld.build_call(free, &[data_ptr.into()], ""));
                b!(self.bld.build_call(free, &[header_ptr.into()], ""));
            }
        }

        b!(self.bld.build_unconditional_branch(done_bb));
        self.bld.position_at_end(done_bb);
        Ok(())
    }

    /// Drop a Map, recursively destroying keys and values if they are non-trivially
    /// droppable. Iterates all capacity slots, checking the occupancy marker at
    /// bucket offset 40. Bucket layout: [8B hash][24B key][8B value][1B occ][7B pad].
    pub(in crate::codegen::drop) fn drop_map_deep(
        &mut self,
        val: BasicValueEnum<'ctx>,
        kt: &Type,
        vt: &Type,
    ) -> Result<(), String> {
        if kt.is_trivially_droppable() && vt.is_trivially_droppable() {
            return self.drop_container_simple(val);
        }

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let fv = self.current_fn();
        let free = self.ensure_free();
        let null = ptr_ty.const_null();

        let header_ptr = val.into_pointer_value();
        let is_null =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::EQ, header_ptr, null, "dmd.null"));
        let drop_bb = self.ctx.append_basic_block(fv, "dmd.drop");
        let done_bb = self.ctx.append_basic_block(fv, "dmd.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, drop_bb));
        self.bld.position_at_end(drop_bb);

        let data_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "dmd.d"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dmd.buf")).into_pointer_value();
        let cap_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 2, "dmd.c"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "dmd.cap")).into_int_value();

        let bucket_size = i64t.const_int(48, false);

        let loop_bb = self.ctx.append_basic_block(fv, "dmd.loop");
        let check_bb = self.ctx.append_basic_block(fv, "dmd.check");
        let body_bb = self.ctx.append_basic_block(fv, "dmd.body");
        let inc_bb = self.ctx.append_basic_block(fv, "dmd.inc");
        let post_bb = self.ctx.append_basic_block(fv, "dmd.post");

        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(loop_bb);
        let phi = b!(self.bld.build_phi(i64t, "dmd.i"));
        phi.add_incoming(&[(&i64t.const_int(0, false), drop_bb)]);
        let i = phi.as_basic_value().into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, i, cap, "dmd.cmp"));
        b!(self.bld.build_conditional_branch(cond, check_bb, post_bb));

        // Check occupancy marker at bucket offset 40
        self.bld.position_at_end(check_bb);
        let offset = b!(self.bld.build_int_mul(i, bucket_size, "dmd.off"));
        let _entry_ptr = unsafe { b!(self.bld.build_gep(i8t, data_ptr, &[offset], "dmd.ep")) };
        let occ_off = b!(self
            .bld
            .build_int_add(offset, i64t.const_int(40, false), "dmd.ooff"));
        let occ_ptr = unsafe { b!(self.bld.build_gep(i8t, data_ptr, &[occ_off], "dmd.ocp")) };
        let occ = b!(self.bld.build_load(i8t, occ_ptr, "dmd.occ")).into_int_value();
        let is_occupied = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            occ,
            i8t.const_int(0, false),
            "dmd.occ_ne"
        ));
        b!(self
            .bld
            .build_conditional_branch(is_occupied, body_bb, inc_bb));

        // Drop key (at offset 8) and value (at offset 32) if non-trivially-droppable
        self.bld.position_at_end(body_bb);
        if !kt.is_trivially_droppable() {
            let key_off = b!(self
                .bld
                .build_int_add(offset, i64t.const_int(8, false), "dmd.koff"));
            let key_ptr = unsafe { b!(self.bld.build_gep(i8t, data_ptr, &[key_off], "dmd.kp")) };
            let key_llvm = self.llvm_ty(kt);
            let key_val = b!(self.bld.build_load(key_llvm, key_ptr, "dmd.kv"));
            self.drop_value(key_val, kt)?;
        }
        if !vt.is_trivially_droppable() {
            let val_off = b!(self
                .bld
                .build_int_add(offset, i64t.const_int(32, false), "dmd.voff"));
            let val_ptr = unsafe { b!(self.bld.build_gep(i8t, data_ptr, &[val_off], "dmd.vp")) };
            let val_llvm = self.llvm_ty(vt);
            let val_val = b!(self.bld.build_load(val_llvm, val_ptr, "dmd.vv"));
            self.drop_value(val_val, vt)?;
        }
        let after_drop_bb = self.current_bb();
        b!(self.bld.build_unconditional_branch(inc_bb));

        self.bld.position_at_end(inc_bb);
        let inc_phi = b!(self.bld.build_phi(i64t, "dmd.iphi"));
        inc_phi.add_incoming(&[(&i, check_bb), (&i, after_drop_bb)]);
        let next = b!(self.bld.build_int_add(
            inc_phi.as_basic_value().into_int_value(),
            i64t.const_int(1, false),
            "dmd.next"
        ));
        phi.add_incoming(&[(&next, inc_bb)]);
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(post_bb);
        b!(self.bld.build_call(free, &[data_ptr.into()], ""));
        b!(self.bld.build_call(free, &[header_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(done_bb));
        self.bld.position_at_end(done_bb);
        Ok(())
    }

    /// Drop a Set, recursively destroying elements if they are non-trivially
    /// droppable. Same bucket layout as Map: 48-byte entries, occupancy at offset 40,
    /// element at offset 8.
    pub(in crate::codegen::drop) fn drop_set_deep(
        &mut self,
        val: BasicValueEnum<'ctx>,
        elem: &Type,
    ) -> Result<(), String> {
        if elem.is_trivially_droppable() {
            return self.drop_container_simple(val);
        }

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let fv = self.current_fn();
        let free = self.ensure_free();
        let null = ptr_ty.const_null();

        let header_ptr = val.into_pointer_value();
        let is_null =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::EQ, header_ptr, null, "dsd.null"));
        let drop_bb = self.ctx.append_basic_block(fv, "dsd.drop");
        let done_bb = self.ctx.append_basic_block(fv, "dsd.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, drop_bb));
        self.bld.position_at_end(drop_bb);

        let data_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "dsd.d"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dsd.buf")).into_pointer_value();
        let cap_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 2, "dsd.c"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "dsd.cap")).into_int_value();

        let bucket_size = i64t.const_int(48, false);

        let loop_bb = self.ctx.append_basic_block(fv, "dsd.loop");
        let check_bb = self.ctx.append_basic_block(fv, "dsd.check");
        let body_bb = self.ctx.append_basic_block(fv, "dsd.body");
        let inc_bb = self.ctx.append_basic_block(fv, "dsd.inc");
        let post_bb = self.ctx.append_basic_block(fv, "dsd.post");

        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(loop_bb);
        let phi = b!(self.bld.build_phi(i64t, "dsd.i"));
        phi.add_incoming(&[(&i64t.const_int(0, false), drop_bb)]);
        let i = phi.as_basic_value().into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, i, cap, "dsd.cmp"));
        b!(self.bld.build_conditional_branch(cond, check_bb, post_bb));

        self.bld.position_at_end(check_bb);
        let offset = b!(self.bld.build_int_mul(i, bucket_size, "dsd.off"));
        let occ_off = b!(self
            .bld
            .build_int_add(offset, i64t.const_int(40, false), "dsd.ooff"));
        let occ_ptr = unsafe { b!(self.bld.build_gep(i8t, data_ptr, &[occ_off], "dsd.ocp")) };
        let occ = b!(self.bld.build_load(i8t, occ_ptr, "dsd.occ")).into_int_value();
        let is_occupied = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            occ,
            i8t.const_int(0, false),
            "dsd.occ_ne"
        ));
        b!(self
            .bld
            .build_conditional_branch(is_occupied, body_bb, inc_bb));

        self.bld.position_at_end(body_bb);
        let elem_off = b!(self
            .bld
            .build_int_add(offset, i64t.const_int(8, false), "dsd.eoff"));
        let elem_ptr = unsafe { b!(self.bld.build_gep(i8t, data_ptr, &[elem_off], "dsd.ep")) };
        let elem_llvm = self.llvm_ty(elem);
        let elem_val = b!(self.bld.build_load(elem_llvm, elem_ptr, "dsd.ev"));
        self.drop_value(elem_val, elem)?;
        let after_drop_bb = self.current_bb();
        b!(self.bld.build_unconditional_branch(inc_bb));

        self.bld.position_at_end(inc_bb);
        let inc_phi = b!(self.bld.build_phi(i64t, "dsd.iphi"));
        inc_phi.add_incoming(&[(&i, check_bb), (&i, after_drop_bb)]);
        let next = b!(self.bld.build_int_add(
            inc_phi.as_basic_value().into_int_value(),
            i64t.const_int(1, false),
            "dsd.next"
        ));
        phi.add_incoming(&[(&next, inc_bb)]);
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(post_bb);
        b!(self.bld.build_call(free, &[data_ptr.into()], ""));
        b!(self.bld.build_call(free, &[header_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(done_bb));
        self.bld.position_at_end(done_bb);
        Ok(())
    }

    /// Free a container with {ptr, len, cap} header: free data, free header.
    pub(in crate::codegen::drop) fn drop_container_simple(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let free = self.ensure_free();
        let fv = self.current_fn();
        let null = ptr_ty.const_null();

        // val is the header pointer itself (already loaded from alloca)
        let header_ptr = val.into_pointer_value();

        let is_null =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::EQ, header_ptr, null, "dcs.null"));
        let free_bb = self.ctx.append_basic_block(fv, "dcs.free");
        let done_bb = self.ctx.append_basic_block(fv, "dcs.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, free_bb));

        self.bld.position_at_end(free_bb);
        let data_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "dcs.data"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dcs.buf"));
        b!(self.bld.build_call(free, &[data_ptr.into()], ""));
        b!(self.bld.build_call(free, &[header_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(done_bb);
        Ok(())
    }

    /// Drop containers with the same {ptr, len, cap} header (Deque, PriorityQueue).
    pub(in crate::codegen::drop) fn drop_container_header(
        &mut self,
        val: BasicValueEnum<'ctx>,
        _elem: &Type,
    ) -> Result<(), String> {
        self.drop_container_simple(val)
    }

    /// Drop Arena: free the base pointer.
    pub(in crate::codegen::drop) fn drop_arena(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        let arena_ty = self.arena_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let ptr = val.into_pointer_value();
        let base_gep = b!(self.bld.build_struct_gep(arena_ty, ptr, 0, "da.base.p"));
        let base = b!(self.bld.build_load(ptr_ty, base_gep, "da.base"));
        let free = self.ensure_free();
        b!(self.bld.build_call(free, &[base.into()], ""));
        Ok(())
    }

    /// Drop NDArray: free the data buffer (raw malloc'd).
    pub(in crate::codegen::drop) fn drop_ndarray(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        self.drop_ptr_allocated(val)
    }

    /// Free a pointer-allocated value (generator, coroutine, channel, etc.)
    /// Null-check, then free.
    pub(in crate::codegen::drop) fn drop_ptr_allocated(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let fv = self.current_fn();
        let free = self.ensure_free();

        let ptr = if val.is_pointer_value() {
            val.into_pointer_value()
        } else {
            return Ok(());
        };

        let null = ptr_ty.const_null();
        let is_null =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::EQ, ptr, null, "dpa.null"));
        let free_bb = self.ctx.append_basic_block(fv, "dpa.free");
        let done_bb = self.ctx.append_basic_block(fv, "dpa.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, free_bb));

        self.bld.position_at_end(free_bb);
        b!(self.bld.build_call(free, &[ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(done_bb);
        Ok(())
    }

    /// Drop a generator: calls jinn_gen_destroy to free coroutine stack + gen block.
    pub(in crate::codegen::drop) fn drop_generator(
        &mut self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let fv = self.current_fn();

        let ptr = if val.is_pointer_value() {
            val.into_pointer_value()
        } else {
            return Ok(());
        };

        self.declare_gen_runtime();
        let gen_destroy = crate::codegen::fn_or_die(&self.module, "jinn_gen_destroy");

        let null = ptr_ty.const_null();
        let is_null =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::EQ, ptr, null, "dg.null"));
        let free_bb = self.ctx.append_basic_block(fv, "dg.free");
        let done_bb = self.ctx.append_basic_block(fv, "dg.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, free_bb));

        self.bld.position_at_end(free_bb);
        b!(self.bld.build_call(gen_destroy, &[ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(done_bb);
        Ok(())
    }
}
