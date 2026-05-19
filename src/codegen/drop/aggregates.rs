use super::*;

impl<'ctx> Compiler<'ctx> {
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

    pub(in crate::codegen::drop) fn drop_struct_fields(
        &mut self,
        val: BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<(), String> {
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
            None => return Ok(()),
        };
        let any_needs_drop = fields.iter().any(|(_, ty)| !ty.is_trivially_droppable());
        if user_drop_fn.is_none() && !any_needs_drop {
            return Ok(());
        }
        let st = match self.module.get_struct_type(name) {
            Some(s) => s,
            None => return Ok(()),
        };

        if let Some(udf) = user_drop_fn {
            let ptr = self.entry_alloca(st.into(), "ds.udrop.tmp");
            b!(self.bld.build_store(ptr, val));
            b!(self.bld.build_call(udf, &[ptr.into()], ""));
            if !any_needs_drop {
                return Ok(());
            }
        }

        let drop_fn_name = format!("__drop_{}", name);

        if let Some(dfn) = self.module.get_function(&drop_fn_name) {
            let ptr = self.entry_alloca(st.into(), "ds.tmp");
            b!(self.bld.build_store(ptr, val));
            b!(self.bld.build_call(dfn, &[ptr.into()], ""));
            return Ok(());
        }

        let is_recursive = fields
            .iter()
            .any(|(_, ty)| Self::type_references_struct(ty, name));

        if !is_recursive {
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

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let fn_ty = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
        let dfn = self.module.add_function(&drop_fn_name, fn_ty, None);

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

        self.cur_fn = saved_fn;
        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }

        let ptr = self.entry_alloca(st.into(), "ds.tmp");
        b!(self.bld.build_store(ptr, val));
        b!(self.bld.build_call(dfn, &[ptr.into()], ""));
        Ok(())
    }

    fn type_references_struct(ty: &Type, name: &str) -> bool {
        match ty {
            Type::Struct(n, _) => n == name,
            Type::Vec(inner) => Self::type_references_struct(inner, name),
            Type::Map(k, v) => {
                Self::type_references_struct(k, name) || Self::type_references_struct(v, name)
            }
            Type::Tuple(tys) => tys.iter().any(|t| Self::type_references_struct(t, name)),
            Type::Alias(_, inner) | Type::Newtype(_, inner) => {
                Self::type_references_struct(inner, name)
            }
            _ => false,
        }
    }

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

    pub(in crate::codegen::drop) fn drop_enum_variants(
        &mut self,
        val: BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<(), String> {
        let variants = match self.enums.get(name) {
            Some(v) => v.clone(),
            None => return Ok(()),
        };

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

        let tag_gep = b!(self.bld.build_struct_gep(st, ptr, 0, "de.tag"));
        let tag = b!(self.bld.build_load(i32t, tag_gep, "de.tv")).into_int_value();

        let done_bb = self.ctx.append_basic_block(fv, "de.done");

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

        for (vd, (_tag_iv, case_bb)) in drop_variants.iter().zip(case_bbs.iter()) {
            self.bld.position_at_end(*case_bb);
            for (fi, fty) in vd.field_types.iter().enumerate() {
                if fty.is_trivially_droppable() {
                    continue;
                }

                let f_gep = b!(self.bld.build_struct_gep(st, ptr, (fi + 1) as u32, "de.vf"));
                let f_val = b!(self.bld.build_load(self.llvm_ty(fty), f_gep, "de.vfv"));
                self.drop_value(f_val, fty)?;
            }
            b!(self.bld.build_unconditional_branch(done_bb));
        }

        self.bld.position_at_end(done_bb);
        Ok(())
    }
}
