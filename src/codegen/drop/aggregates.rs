//! Tuple, struct, array, enum, and reference-counted aggregate drop helpers.

use super::*;

impl<'ctx> Compiler<'ctx> {
    /// Drop a tuple: recursively drop each non-trivially-droppable element.
    pub(in crate::codegen::drop) fn drop_tuple(
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
    /// For recursive structs (e.g. Value containing Vec of Value), we
    /// generate a named drop function `__drop_<Name>` and call it to
    /// break infinite codegen recursion.
    pub(in crate::codegen::drop) fn drop_struct_fields(
        &mut self,
        val: BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<(), String> {
        // Auto-invoke user-defined `*drop` for `@resource` types
        // (docs/access-semantics.md §3, §4.1). The method is mangled as
        // `<TypeName>_drop` (see src/typer/lower/decl.rs) and takes `self`
        // by pointer. It runs BEFORE field cleanup so the method body can
        // still read fields like `self.fd`.
        let user_drop_fn = {
            let sym = crate::intern::Symbol::intern(name);
            let is_resource = self
                .struct_layouts
                .get(&sym)
                .map(|l| l.resource)
                .unwrap_or(false);
            if is_resource {
                self.module.get_function(&format!("{name}_drop"))
            } else {
                None
            }
        };

        let fields = match self.structs.get(name) {
            Some(f) => f.clone(),
            None => return Ok(()), // unknown struct — can't drop fields
        };
        let any_needs_drop = fields.iter().any(|(_, ty)| !ty.is_trivially_droppable());
        if user_drop_fn.is_none() && !any_needs_drop {
            return Ok(());
        }
        let st = match self.module.get_struct_type(name) {
            Some(s) => s,
            None => return Ok(()),
        };

        // If this is a @resource with a user *drop, fire it first then
        // continue with normal field cleanup. The struct is spilled to a
        // stack slot so we can pass `self` by pointer (matches the method
        // ABI emitted by typer/lower/decl.rs).
        if let Some(udf) = user_drop_fn {
            let ptr = self.entry_alloca(st.into(), "ds.udrop.tmp");
            b!(self.bld.build_store(ptr, val));
            b!(self.bld.build_call(udf, &[ptr.into()], ""));
            if !any_needs_drop {
                return Ok(());
            }
            // Re-load `val` from the slot — the user method may have mutated
            // fields (e.g. zeroed `fd` after close). Subsequent field-drop
            // logic uses the original `val`, which is fine: POD fields don't
            // care, and heap fields the user already owned should have been
            // dropped or moved out by the user method.
        }

        // Check if a recursive struct type needs an out-of-line drop function.
        let drop_fn_name = format!("__drop_{}", name);

        // If the drop function already exists, just call it.
        if let Some(dfn) = self.module.get_function(&drop_fn_name) {
            let ptr = self.entry_alloca(st.into(), "ds.tmp");
            b!(self.bld.build_store(ptr, val));
            b!(self.bld.build_call(dfn, &[ptr.into()], ""));
            return Ok(());
        }

        // Check if this struct has self-referencing fields (directly or via Vec/Map).
        let is_recursive = fields
            .iter()
            .any(|(_, ty)| Self::type_references_struct(ty, name));

        if !is_recursive {
            // Inline the drop as before.
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
            return Ok(());
        }

        // Recursive struct: generate __drop_<Name>(ptr) function and call it.
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let fn_ty = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
        let dfn = self.module.add_function(&drop_fn_name, fn_ty, None);

        // Save current state.
        let saved_fn = self.cur_fn;
        let saved_bb = self.bld.get_insert_block();

        self.cur_fn = Some(dfn);
        let entry = self.ctx.append_basic_block(dfn, "entry");
        self.bld.position_at_end(entry);

        let param_ptr = dfn
            .get_first_param()
            .expect("ICE: function has no first param")
            .into_pointer_value();
        for (i, (_, ty)) in fields.iter().enumerate() {
            if ty.is_trivially_droppable() {
                continue;
            }
            let gep = b!(self.bld.build_struct_gep(st, param_ptr, i as u32, "ds.f"));
            let fval = b!(self.bld.build_load(self.llvm_ty(ty), gep, "ds.fv"));
            self.drop_value(fval, ty)?;
        }
        b!(self.bld.build_return(None));

        // Restore state.
        self.cur_fn = saved_fn;
        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }

        // Now call the generated function.
        let ptr = self.entry_alloca(st.into(), "ds.tmp");
        b!(self.bld.build_store(ptr, val));
        b!(self.bld.build_call(dfn, &[ptr.into()], ""));
        Ok(())
    }

    /// Check if a type references a named struct (directly or nested in containers).
    fn type_references_struct(ty: &Type, name: &str) -> bool {
        match ty {
            Type::Struct(n, _) => n == name,
            Type::Vec(inner) => Self::type_references_struct(inner, name),
            Type::Map(k, v) => {
                Self::type_references_struct(k, name) || Self::type_references_struct(v, name)
            }
            Type::Tuple(tys) => tys.iter().any(|t| Self::type_references_struct(t, name)),
            Type::Rc(inner)
            | Type::Weak(inner) => Self::type_references_struct(inner, name),
            Type::Alias(_, inner) | Type::Newtype(_, inner) => {
                Self::type_references_struct(inner, name)
            }
            _ => false,
        }
    }

    /// Drop array elements one by one.
    pub(in crate::codegen::drop) fn drop_array_elements(
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
            let gep = unsafe { b!(self.bld.build_gep(arr_ty, ptr, &[zero, idx], "dae.e")) };
            let ev = b!(self.bld.build_load(elem_llvm, gep, "dae.v"));
            self.drop_value(ev, elem)?;
        }
        Ok(())
    }

    /// Drop an enum: switch on the discriminant, then drop the active variant's
    /// payload fields. This is the enum analog of drop_struct_fields.
    pub(in crate::codegen::drop) fn drop_enum_variants(
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
        let fv = self.current_fn();
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
                let bb = self
                    .ctx
                    .append_basic_block(fv, &format!("de.v{}", vd.tag_val));
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
        let fv = self.current_fn();
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
            let dec = b!(self
                .bld
                .build_int_nsw_sub(loaded, i64t.const_int(1, false), "rc.dec"));
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
        // Drop the inner value before freeing the Rc allocation. Payload is
        // at struct index 2 in the {strong (0), weak (1), payload (2)}
        // layout produced by rc_layout_ty (see src/codegen/rc.rs:12-26).
        // Previously this used index 1, which loaded the weak count as
        // garbage payload — corrupted free() for Rc<String> / Rc<Vec<_>>
        // / Rc<Struct-with-drop>.
        if !inner.is_trivially_droppable() {
            let val_gep = b!(self
                .bld
                .build_struct_gep(layout, heap_ptr, 2, "rc.val.drop"));
            let inner_val = b!(self
                .bld
                .build_load(self.llvm_ty(inner), val_gep, "rc.inner"));
            self.drop_value(inner_val, inner)?;
        }
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[heap_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        Ok(())
    }

    /// R3.4.c — Arc<T> release with recursive inner value drop. Mirrors
    /// `rc_release_deep` but unconditionally uses an atomic decrement
    /// (Arc is always atomic regardless of payload). For
    /// `Arc<Mutex<T>>` the inner drop will recurse into the Mutex arm
    /// of `drop_value`, which currently errors — R3.4.c.2 will add a
    /// dedicated `arc_mutex_release_deep` that destroys the lock and
    /// drops the payload directly without round-tripping the Mutex arm.
    #[allow(dead_code)]
    pub(crate) fn arc_release_deep(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        let fv = self.current_fn();
        let layout = self.rc_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let heap_ptr = ptr.into_pointer_value();
        let rc_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "arc.cnt"));
        let old = b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Sub,
            rc_gep,
            i64t.const_int(1, false),
            inkwell::AtomicOrdering::AcquireRelease,
        ));
        let is_zero = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            old,
            i64t.const_int(1, false),
            "arc.dead"
        ));
        let free_bb = self.ctx.append_basic_block(fv, "arc.free");
        let cont_bb = self.ctx.append_basic_block(fv, "arc.cont");
        b!(self.bld.build_conditional_branch(is_zero, free_bb, cont_bb));

        self.bld.position_at_end(free_bb);
        // Drop the inner value before freeing the Arc allocation. Use the
        // payload index (2) per the {strong, weak, payload} layout.
        if !inner.is_trivially_droppable() {
            let val_gep = b!(self
                .bld
                .build_struct_gep(layout, heap_ptr, 2, "arc.val.drop"));
            let inner_val = b!(self
                .bld
                .build_load(self.llvm_ty(inner), val_gep, "arc.inner"));
            self.drop_value(inner_val, inner)?;
        }
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[heap_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        Ok(())
    }
}
