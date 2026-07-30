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

        let struct_ptr = if val.is_pointer_value() {
            val.into_pointer_value()
        } else {
            let p = self.entry_alloca(st.into(), "ds.tmp");
            b!(self.bld.build_store(p, val));
            p
        };

        if let Some(udf) = user_drop_fn {
            b!(self.bld.build_call(udf, &[struct_ptr.into()], ""));
            if !any_needs_drop {
                return Ok(());
            }
        }

        let drop_fn_name = format!("__drop_{}", name);

        if let Some(dfn) = self.module.get_function(&drop_fn_name) {
            b!(self.bld.build_call(dfn, &[struct_ptr.into()], ""));
            return Ok(());
        }

        let is_recursive = fields
            .iter()
            .any(|(_, ty)| Self::type_references_struct(ty, name));

        if !is_recursive {
            let ptr = struct_ptr;
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

        b!(self.bld.build_call(dfn, &[struct_ptr.into()], ""));
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

    /// Emit (once) and call `__drop_enum_<name>`: a function wrapping
    /// drop_enum_variants so recursive enums drop by RUNTIME recursion
    /// instead of infinite inline expansion.
    pub(in crate::codegen) fn call_enum_drop_fn(
        &mut self,
        val: inkwell::values::BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<(), String> {
        let fn_name = format!("__drop_enum_{name}");
        let fv = if let Some(f) = self.module.get_function(&fn_name) {
            f
        } else {
            let st = match self.module.get_struct_type(name) {
                Some(st) => st,
                None => return Ok(()), /* payload-less or undeclared: nothing to drop */
            };
            let ft = self.ctx.void_type().fn_type(&[st.into()], false);
            let f = self
                .module
                .add_function(&fn_name, ft, Some(inkwell::module::Linkage::Internal));
            self.tag_fn(f);
            let entry = self.ctx.append_basic_block(f, "entry");
            let old_fn = self.cur_fn;
            let old_bb = self.bld.get_insert_block();
            self.cur_fn = Some(f);
            self.bld.position_at_end(entry);
            let param = f.get_nth_param(0).expect("drop fn param");
            self.drop_enum_variants(param, name)?;
            b!(self.bld.build_return(None));
            self.cur_fn = old_fn;
            if let Some(bb) = old_bb {
                self.bld.position_at_end(bb);
            }
            f
        };
        b!(self.bld.build_call(fv, &[val.into()], ""));
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
            /* The enum's LLVM layout is {tag, [N x i8] payload}: fields live
             * at 8-aligned BYTE offsets inside member 1, exactly as the
             * constructor writes them. The old code struct_gep'd member
             * fi+1, which is out of range for any droppable field past the
             * first (the json_parser/linked_list ICE class, task 8-19) —
             * and recursive fields are boxed, so their slot holds a heap
             * pointer, which must be freed after its pointee drops. */
            let payload_gep = b!(self.bld.build_struct_gep(st, ptr, 1, "de.payload"));
            let mut byte_offset: u64 = 0;
            for fty in vd.field_types.iter() {
                let is_rec = Compiler::is_recursive_field(fty, name);
                let slot_size: u64 = if is_rec {
                    8
                } else {
                    self.type_store_size(self.llvm_ty(fty))
                };
                if fty.is_trivially_droppable() && !is_rec {
                    byte_offset += (slot_size + 7) & !7;
                    continue;
                }
                let f_ptr = if byte_offset == 0 {
                    payload_gep
                } else {
                    let off = self.ctx.i64_type().const_int(byte_offset, false);
                    unsafe {
                        b!(self.bld.build_gep(
                            self.ctx.i8_type(),
                            payload_gep,
                            &[off],
                            "de.vf"
                        ))
                    }
                };
                if is_rec {
                    let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                    let heap = b!(self.bld.build_load(ptr_ty, f_ptr, "de.box"))
                        .into_pointer_value();
                    let inner =
                        b!(self.bld.build_load(self.llvm_ty(fty), heap, "de.boxv"));
                    self.drop_value(inner, fty)?;
                    let free_fn = self.ensure_free();
                    b!(self.bld.build_call(free_fn, &[heap.into()], ""));
                } else {
                    let f_val = b!(self.bld.build_load(self.llvm_ty(fty), f_ptr, "de.vfv"));
                    self.drop_value(f_val, fty)?;
                }
                byte_offset += (slot_size + 7) & !7;
            }
            b!(self.bld.build_unconditional_branch(done_bb));
        }

        self.bld.position_at_end(done_bb);
        Ok(())
    }
}
