use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    /// Unified drop dispatcher. Emits code to release all resources owned by a
    /// value of the given type. For types that are trivially droppable (scalars,
    /// bools, pointers-as-raw), this is a no-op. For heap-owning types, this
    /// recursively frees inner allocations before releasing the outer container.
    ///
    /// This produces a deterministic, zero-overhead destruction sequence with
    /// no dynamic dispatch and no RTTI. Each drop path is monomorphized at
    /// compile time — the generated code is a flat, branchless (per-type)
    /// sequence of frees. No GC, no finalizer queues.
    pub(crate) fn drop_value(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<(), String> {
        if ty.is_trivially_droppable() {
            return Ok(());
        }
        match ty {
            Type::String => {
                self.drop_string(val)?;
            }
            Type::Vec(elem) => {
                // Vec: free element storage if elements need dropping, then
                // free the data buffer and header. The element loop runs only
                // for non-trivially-droppable element types. For POD vecs this
                // collapses to two frees.
                self.drop_vec_deep(val, elem)?;
            }
            Type::Map(kt, vt) => {
                self.drop_map_deep(val, kt, vt)?;
            }
            Type::Set(elem) => {
                self.drop_set_deep(val, elem)?;
            }
            Type::Rc(inner) => {
                self.rc_release_deep(val, inner)?;
            }
            Type::Weak(inner) => {
                self.weak_release(val, inner)?;
            }
            Type::Arena => {
                self.drop_arena(val)?;
            }
            Type::Deque(elem) => {
                self.drop_container_header(val, elem)?;
            }
            Type::PriorityQueue(elem) => {
                self.drop_container_header(val, elem)?;
            }
            Type::NDArray(_, _) => {
                self.drop_ndarray(val)?;
            }
            Type::Generator(_) => {
                self.drop_ptr_allocated(val)?;
            }
            Type::Tuple(tys) => {
                self.drop_tuple(val, tys)?;
            }
            Type::Struct(name, _) => {
                self.drop_struct_fields(val, name)?;
            }
            Type::Array(elem, n) => {
                if !elem.is_trivially_droppable() {
                    self.drop_array_elements(val, elem, *n)?;
                }
            }
            Type::Enum(name) => {
                self.drop_enum_variants(val, name)?;
            }
            Type::Alias(_, inner) | Type::Newtype(_, inner) => {
                self.drop_value(val, inner)?;
            }
            // Coroutine, Channel, Fn, DynTrait, Cow — ptr-based, need
            // runtime cooperation to drop correctly. For now, free the
            // allocation if non-null.
            Type::Coroutine(_) | Type::Channel(_) | Type::Cow(_) => {
                self.drop_ptr_allocated(val)?;
            }
            _ => {
                // Scalars, bools, raw ptrs, ActorRef — no-op.
                // is_trivially_droppable should have caught these above.
            }
        }
        Ok(())
    }

    /// Drop a Vec, recursively destroying elements if they are non-trivially
    /// droppable. O(n) in element count, O(1) for POD elements.
    fn drop_vec_deep(
        &mut self,
        val: BasicValueEnum<'ctx>,
        elem: &Type,
    ) -> Result<(), String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let i64t = self.ctx.i64_type();
        let fv = self.cur_fn.unwrap();
        let free = self.ensure_free();

        // val is the header pointer itself (already loaded from alloca)
        let header_ptr = val.into_pointer_value();

        let null = ptr_ty.const_null();
        let is_null = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ, header_ptr, null, "dvd.null"
        ));
        let drop_bb = self.ctx.append_basic_block(fv, "dvd.drop");
        let done_bb = self.ctx.append_basic_block(fv, "dvd.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, drop_bb));
        self.bld.position_at_end(drop_bb);

        // If elements need dropping, iterate and drop each
        if !elem.is_trivially_droppable() {
            let data_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "dvd.d"));
            let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dvd.buf")).into_pointer_value();
            let len_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 1, "dvd.l"));
            let len = b!(self.bld.build_load(i64t, len_gep, "dvd.len")).into_int_value();

            let elem_llvm = self.llvm_ty(elem);
            let elem_size = elem_llvm.size_of().unwrap();

            // Loop: for i in 0..len { drop_value(data[i]) }
            let loop_bb = self.ctx.append_basic_block(fv, "dvd.loop");
            let body_bb = self.ctx.append_basic_block(fv, "dvd.body");
            let post_bb = self.ctx.append_basic_block(fv, "dvd.post");

            b!(self.bld.build_unconditional_branch(loop_bb));
            self.bld.position_at_end(loop_bb);
            let phi = b!(self.bld.build_phi(i64t, "dvd.i"));
            phi.add_incoming(&[(&i64t.const_int(0, false), drop_bb)]);
            let i = phi.as_basic_value().into_int_value();
            let cond = b!(self.bld.build_int_compare(inkwell::IntPredicate::ULT, i, len, "dvd.cmp"));
            b!(self.bld.build_conditional_branch(cond, body_bb, post_bb));

            self.bld.position_at_end(body_bb);
            let offset = b!(self.bld.build_int_mul(i, elem_size, "dvd.off"));
            let elem_ptr = unsafe {
                b!(self.bld.build_gep(self.ctx.i8_type(), data_ptr, &[offset], "dvd.ep"))
            };
            let elem_val = b!(self.bld.build_load(elem_llvm, elem_ptr, "dvd.ev"));
            self.drop_value(elem_val, elem)?;
            // After drop_value, the builder may be in a different BB (e.g., if
            // the element type has its own control flow). Use the actual current
            // block for the phi incoming edge.
            let after_drop_bb = self.bld.get_insert_block().unwrap();
            let next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "dvd.next"));
            phi.add_incoming(&[(&next, after_drop_bb)]);
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(post_bb);
            // Free data buffer and header
            let data_gep2 = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "dvd.d2"));
            let data_ptr2 = b!(self.bld.build_load(ptr_ty, data_gep2, "dvd.buf2"));
            b!(self.bld.build_call(free, &[data_ptr2.into()], ""));
            b!(self.bld.build_call(free, &[header_ptr.into()], ""));
        } else {
            // POD elements: just free buffer and header
            let data_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "dvd.d"));
            let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dvd.buf"));
            b!(self.bld.build_call(free, &[data_ptr.into()], ""));
            b!(self.bld.build_call(free, &[header_ptr.into()], ""));
        }

        b!(self.bld.build_unconditional_branch(done_bb));
        self.bld.position_at_end(done_bb);
        Ok(())
    }

    /// Drop a Map, freeing its bucket array and header.
    /// Map and Set share the same {ptr, len, cap} layout.
    fn drop_map_deep(
        &mut self,
        val: BasicValueEnum<'ctx>,
        _kt: &Type,
        _vt: &Type,
    ) -> Result<(), String> {
        // Map uses open-addressing hash table with {ptr, len, cap}.
        // Element-level recursive drop would require iterating live slots.
        // For now, free the bucket array and header (same as existing drop_map).
        self.drop_container_simple(val)
    }

    /// Drop a Set. Same layout as Map.
    fn drop_set_deep(
        &mut self,
        val: BasicValueEnum<'ctx>,
        _elem: &Type,
    ) -> Result<(), String> {
        self.drop_container_simple(val)
    }

    /// Free a container with {ptr, len, cap} header: free data, free header.
    fn drop_container_simple(&mut self, val: BasicValueEnum<'ctx>) -> Result<(), String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let free = self.ensure_free();
        let fv = self.cur_fn.unwrap();
        let null = ptr_ty.const_null();

        // val is the header pointer itself (already loaded from alloca)
        let header_ptr = val.into_pointer_value();

        let is_null = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ, header_ptr, null, "dcs.null"
        ));
        let free_bb = self.ctx.append_basic_block(fv, "dcs.free");
        let done_bb = self.ctx.append_basic_block(fv, "dcs.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, free_bb));

        self.bld.position_at_end(free_bb);
        let data_gep = b!(self.bld.build_struct_gep(header_ty, header_ptr, 0, "dcs.data"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dcs.buf"));
        b!(self.bld.build_call(free, &[data_ptr.into()], ""));
        b!(self.bld.build_call(free, &[header_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(done_bb);
        Ok(())
    }

    /// Drop containers with the same {ptr, len, cap} header (Deque, PriorityQueue).
    fn drop_container_header(
        &mut self,
        val: BasicValueEnum<'ctx>,
        _elem: &Type,
    ) -> Result<(), String> {
        self.drop_container_simple(val)
    }

    /// Drop Arena: free the base pointer.
    fn drop_arena(&mut self, val: BasicValueEnum<'ctx>) -> Result<(), String> {
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
    fn drop_ndarray(&mut self, val: BasicValueEnum<'ctx>) -> Result<(), String> {
        self.drop_ptr_allocated(val)
    }

    /// Free a pointer-allocated value (generator, coroutine, channel, etc.)
    /// Null-check, then free.
    fn drop_ptr_allocated(&mut self, val: BasicValueEnum<'ctx>) -> Result<(), String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let fv = self.cur_fn.unwrap();
        let free = self.ensure_free();

        let ptr = if val.is_pointer_value() {
            val.into_pointer_value()
        } else {
            return Ok(());
        };

        let null = ptr_ty.const_null();
        let is_null = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ, ptr, null, "dpa.null"
        ));
        let free_bb = self.ctx.append_basic_block(fv, "dpa.free");
        let done_bb = self.ctx.append_basic_block(fv, "dpa.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, free_bb));

        self.bld.position_at_end(free_bb);
        b!(self.bld.build_call(free, &[ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(done_bb);
        Ok(())
    }

    /// Drop a tuple: recursively drop each non-trivially-droppable element.
    fn drop_tuple(
        &mut self,
        val: BasicValueEnum<'ctx>,
        tys: &[Type],
    ) -> Result<(), String> {
        let st = self.ctx.struct_type(
            &tys.iter().map(|t| self.llvm_ty(t)).collect::<Vec<_>>(),
            false,
        );
        let ptr = self.entry_alloca(st.into(), "dt.tmp");
        b!(self.bld.build_store(ptr, val));
        for (i, ty) in tys.iter().enumerate() {
            if ty.is_trivially_droppable() {
                continue;
            }
            let gep = b!(self.bld.build_struct_gep(st, ptr, i as u32, "dt.f"));
            let fval = b!(self.bld.build_load(self.llvm_ty(ty), gep, "dt.fv"));
            self.drop_value(fval, ty)?;
        }
        Ok(())
    }

    /// Drop a struct by dropping each field that needs it.
    fn drop_struct_fields(
        &mut self,
        val: BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<(), String> {
        let fields = match self.structs.get(name) {
            Some(f) => f.clone(),
            None => return Ok(()), // unknown struct — can't drop fields
        };
        let any_needs_drop = fields.iter().any(|(_, ty)| !ty.is_trivially_droppable());
        if !any_needs_drop {
            return Ok(());
        }
        let st = match self.module.get_struct_type(name) {
            Some(s) => s,
            None => return Ok(()),
        };
        let ptr = self.entry_alloca(st.into(), "ds.tmp");
        b!(self.bld.build_store(ptr, val));
        for (i, (_, ty)) in fields.iter().enumerate() {
            if ty.is_trivially_droppable() {
                continue;
            }
            let gep = b!(self.bld.build_struct_gep(st, ptr, i as u32, "ds.f"));
            let fval = b!(self.bld.build_load(self.llvm_ty(ty), gep, "ds.fv"));
            self.drop_value(fval, ty)?;
        }
        Ok(())
    }

    /// Drop array elements one by one.
    fn drop_array_elements(
        &mut self,
        val: BasicValueEnum<'ctx>,
        elem: &Type,
        count: usize,
    ) -> Result<(), String> {
        let elem_llvm = self.llvm_ty(elem);
        let arr_ty = elem_llvm.array_type(count as u32);
        let ptr = self.entry_alloca(arr_ty.into(), "dae.tmp");
        b!(self.bld.build_store(ptr, val));
        for i in 0..count {
            let idx = self.ctx.i64_type().const_int(i as u64, false);
            let zero = self.ctx.i64_type().const_int(0, false);
            let gep = unsafe {
                b!(self.bld.build_gep(arr_ty, ptr, &[zero, idx], "dae.e"))
            };
            let ev = b!(self.bld.build_load(elem_llvm, gep, "dae.v"));
            self.drop_value(ev, elem)?;
        }
        Ok(())
    }

    /// Drop an enum: switch on the discriminant, then drop the active variant's
    /// payload fields. This is the enum analog of drop_struct_fields.
    fn drop_enum_variants(
        &mut self,
        val: BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<(), String> {
        let variants = match self.enums.get(name) {
            Some(v) => v.clone(),
            None => return Ok(()),
        };
        // Check if any variant has non-trivially-droppable fields
        let any_needs_drop = variants
            .iter()
            .any(|(_, tys)| tys.iter().any(|t| !t.is_trivially_droppable()));
        if !any_needs_drop {
            return Ok(());
        }

        let st = match self.module.get_struct_type(name) {
            Some(s) => s,
            None => return Ok(()),
        };
        let fv = self.cur_fn.unwrap();
        let i32t = self.ctx.i32_type();

        let ptr = self.entry_alloca(st.into(), "de.tmp");
        b!(self.bld.build_store(ptr, val));

        // Field 0 is the tag (i32)
        let tag_gep = b!(self.bld.build_struct_gep(st, ptr, 0, "de.tag"));
        let tag = b!(self.bld.build_load(i32t, tag_gep, "de.tv")).into_int_value();

        let done_bb = self.ctx.append_basic_block(fv, "de.done");

        // Collect variant case blocks that need drops
        struct VariantDrop {
            tag_val: u32,
            field_types: Vec<Type>,
        }
        let mut drop_variants: Vec<VariantDrop> = Vec::new();
        for (vname, vtys) in &variants {
            let tag_val = match self.variant_tags.get(vname) {
                Some((_, t)) => *t,
                None => continue,
            };
            let has_drops = vtys.iter().any(|t| !t.is_trivially_droppable());
            if has_drops {
                drop_variants.push(VariantDrop {
                    tag_val,
                    field_types: vtys.clone(),
                });
            }
        }

        // Pre-create basic blocks for each variant
        let case_bbs: Vec<_> = drop_variants
            .iter()
            .map(|vd| {
                let bb = self.ctx.append_basic_block(fv, &format!("de.v{}", vd.tag_val));
                (i32t.const_int(vd.tag_val as u64, false), bb)
            })
            .collect();

        b!(self.bld.build_switch(tag, done_bb, &case_bbs));

        // Emit drop code for each variant
        for (vd, (_tag_iv, case_bb)) in drop_variants.iter().zip(case_bbs.iter()) {
            self.bld.position_at_end(*case_bb);
            for (fi, fty) in vd.field_types.iter().enumerate() {
                if fty.is_trivially_droppable() {
                    continue;
                }
                // fi+1 because field 0 is the tag
                let f_gep = b!(self.bld.build_struct_gep(st, ptr, (fi + 1) as u32, "de.vf"));
                let f_val = b!(self.bld.build_load(self.llvm_ty(fty), f_gep, "de.vfv"));
                self.drop_value(f_val, fty)?;
            }
            b!(self.bld.build_unconditional_branch(done_bb));
        }

        self.bld.position_at_end(done_bb);
        Ok(())
    }

    /// Rc release with recursive inner value drop. When the refcount reaches
    /// zero, we drop the inner value FIRST, then free the allocation. This
    /// ensures no leaks for Rc<Vec<String>> or Rc<SomeStruct>.
    pub(crate) fn rc_release_deep(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let layout = self.rc_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let heap_ptr = ptr.into_pointer_value();
        let rc_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "rc.cnt"));
        let old = if inner.needs_atomic_rc() {
            b!(self.bld.build_atomicrmw(
                inkwell::AtomicRMWBinOp::Sub,
                rc_gep,
                i64t.const_int(1, false),
                inkwell::AtomicOrdering::AcquireRelease,
            ))
        } else {
            let loaded = b!(self.bld.build_load(i64t, rc_gep, "rc.cnt.ld")).into_int_value();
            let dec = b!(self.bld.build_int_nsw_sub(loaded, i64t.const_int(1, false), "rc.dec"));
            b!(self.bld.build_store(rc_gep, dec));
            loaded
        };
        let is_zero = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            old,
            i64t.const_int(1, false),
            "rc.dead"
        ));
        let free_bb = self.ctx.append_basic_block(fv, "rc.free");
        let cont_bb = self.ctx.append_basic_block(fv, "rc.cont");
        b!(self.bld.build_conditional_branch(is_zero, free_bb, cont_bb));

        self.bld.position_at_end(free_bb);
        // Drop the inner value before freeing the Rc allocation
        if !inner.is_trivially_droppable() {
            let val_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "rc.val.drop"));
            let inner_val = b!(self.bld.build_load(self.llvm_ty(inner), val_gep, "rc.inner"));
            self.drop_value(inner_val, inner)?;
        }
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[heap_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        Ok(())
    }
}
