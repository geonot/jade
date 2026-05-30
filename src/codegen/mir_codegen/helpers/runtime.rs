use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_closure_create(
        &mut self,
        fn_name: &str,
        captures: &[mir::ValueId],
        _result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let closure_ty = self.closure_type();

        let inner_fv = if let Some((fv, _, _)) = self.fns.get(fn_name).cloned() {
            Some(fv)
        } else {
            self.module.get_function(fn_name)
        };

        let cap_vals: Vec<BasicValueEnum<'ctx>> = captures.iter().map(|v| self.val(*v)).collect();
        let cap_tys: Vec<BasicTypeEnum<'ctx>> = cap_vals.iter().map(|v| v.get_type()).collect();

        let env_ptr = if !captures.is_empty() {
            let env_struct_ty = self.ctx.struct_type(&cap_tys, false);
            let env_size = env_struct_ty.size_of().expect("ICE: type has no size");
            let malloc = self.ensure_malloc();
            let ep = b!(self.bld.build_call(malloc, &[env_size.into()], "env.alloc"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_pointer_value();
            for (i, v) in cap_vals.iter().enumerate() {
                let gep = b!(self
                    .bld
                    .build_struct_gep(env_struct_ty, ep, i as u32, "env.field"));
                b!(self.bld.build_store(gep, *v));
            }
            ep
        } else {
            ptr_ty.const_null()
        };

        let wrapper_ptr = if let Some(ifv) = inner_fv {
            let wrapper_name = format!("{fn_name}.env_wrap");
            if let Some(w) = self.module.get_function(&wrapper_name) {
                w.as_global_value().as_pointer_value()
            } else {
                let inner_type = ifv.get_type();
                let inner_params = inner_type.get_param_types();
                let n_captures = captures.len();

                let declared_param_tys = &inner_params[n_captures..];
                let mut wrapper_params: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
                wrapper_params.extend(
                    declared_param_tys
                        .iter()
                        .map(|t| BasicMetadataTypeEnum::from(*t)),
                );
                let wrapper_ft = match inner_type.get_return_type() {
                    Some(ret) => ret.fn_type(&wrapper_params, false),
                    None => self.ctx.void_type().fn_type(&wrapper_params, false),
                };
                let wrapper_fv = self.module.add_function(
                    &wrapper_name,
                    wrapper_ft,
                    Some(inkwell::module::Linkage::Internal),
                );
                self.tag_fn(wrapper_fv);

                let saved_bb = self.bld.get_insert_block();
                let entry = self.ctx.append_basic_block(wrapper_fv, "entry");
                self.bld.position_at_end(entry);

                let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
                if n_captures > 0 {
                    let env_struct_ty = self.ctx.struct_type(&cap_tys, false);
                    let env_param = wrapper_fv
                        .get_nth_param(0)
                        .expect("ICE: missing param")
                        .into_pointer_value();
                    for i in 0..n_captures {
                        let gep = b!(self.bld.build_struct_gep(
                            env_struct_ty,
                            env_param,
                            i as u32,
                            "cap.gep"
                        ));
                        let load_ty: BasicTypeEnum<'ctx> = inner_params[i].try_into().unwrap();
                        let cap = b!(self.bld.build_load(load_ty, gep, "cap.load"));
                        call_args.push(cap.into());
                    }
                }

                for i in 0..declared_param_tys.len() {
                    let p = wrapper_fv.get_nth_param((i + 1) as u32).unwrap();
                    call_args.push(p.into());
                }

                let result = self.bld.build_call(ifv, &call_args, "lam.call").unwrap();
                match inner_type.get_return_type() {
                    Some(_) => {
                        let rv = self.call_result(result);
                        self.bld.build_return(Some(&rv)).unwrap();
                    }
                    None => {
                        self.bld.build_return(None).unwrap();
                    }
                }

                if let Some(bb) = saved_bb {
                    self.bld.position_at_end(bb);
                }
                wrapper_fv.as_global_value().as_pointer_value()
            }
        } else {
            ptr_ty.const_null()
        };

        let mut agg: BasicValueEnum<'ctx> = closure_ty.const_zero().into();
        agg =
            b!(self
                .bld
                .build_insert_value(agg.into_struct_value(), wrapper_ptr, 0, "closure.fn"))
            .into_struct_value()
            .into();
        agg = b!(self
            .bld
            .build_insert_value(agg.into_struct_value(), env_ptr, 1, "closure.env"))
        .into_struct_value()
        .into();
        Ok(agg)
    }

    pub(in crate::codegen) fn emit_chan_create(
        &mut self,
        elem_ty: &Type,
        cap: Option<&mir::ValueId>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        if let Some(fv) = self.module.get_function("jinn_chan_create") {
            let elem_size = self
                .llvm_ty(elem_ty)
                .size_of()
                .unwrap_or(i64t.const_int(8, false));
            let capacity = if let Some(cap_id) = cap {
                self.val(*cap_id).into_int_value()
            } else {
                i64t.const_int(64, false)
            };
            let csv = b!(self
                .bld
                .build_call(fv, &[elem_size.into(), capacity.into()], "chan"));
            Ok(self.call_result(csv))
        } else {
            Ok(ptr_ty.const_null().into())
        }
    }

    pub(in crate::codegen) fn emit_chan_send(
        &mut self,
        ch: mir::ValueId,
        val: mir::ValueId,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_val = self.val(ch);
        let v = self.val(val);
        if let Some(fv) = self.module.get_function("jinn_chan_send") {
            let alloca = self.entry_alloca(v.get_type(), "send.tmp");
            b!(self.bld.build_store(alloca, v));
            b!(self.bld.build_call(fv, &[ch_val.into(), alloca.into()], ""));
        }
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn emit_chan_recv(
        &mut self,
        ch: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_val = self.val(ch);
        if let Some(fv) = self.module.get_function("jinn_chan_recv") {
            let elem_llvm = self.llvm_ty(result_ty);
            let alloca = self.entry_alloca(elem_llvm, "recv.tmp");
            b!(self.bld.build_call(fv, &[ch_val.into(), alloca.into()], ""));
            Ok(b!(self.bld.build_load(elem_llvm, alloca, "recv.val")))
        } else {
            Ok(self.default_val(result_ty))
        }
    }

    pub(in crate::codegen) fn field_index(&self, struct_name: &str, field: &str) -> u32 {
        self.structs
            .get(struct_name)
            .and_then(|fields| fields.iter().position(|(n, _)| n == field))
            .unwrap_or(0) as u32
    }

    pub(in crate::codegen) fn struct_name_from_type(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Struct(name, _) => Some(name.as_str()),
            Type::Ptr(inner) => match inner.as_ref() {
                Type::Struct(name, _) => Some(name.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    pub(in crate::codegen) fn compute_enum_payload_offset(
        &self,
        enum_name: &str,
        target_idx: usize,
    ) -> u64 {
        if let Some(variants) = self.enums.get(enum_name) {
            for (_, field_types) in variants {
                if field_types.len() > target_idx {
                    let mut offset: u64 = 0;
                    for (i, fty) in field_types.iter().enumerate() {
                        if i == target_idx {
                            return offset;
                        }
                        let type_size = if Compiler::is_recursive_field(fty, enum_name) {
                            8
                        } else {
                            self.llvm_ty(fty)
                                .size_of()
                                .map(|s| s.get_zero_extended_constant().unwrap_or(8))
                                .unwrap_or(8)
                        };
                        offset += (type_size + 7) & !7;
                    }
                }
            }
        }
        (target_idx * 8) as u64
    }

    pub(in crate::codegen) fn emit_slice(
        &mut self,
        base: mir::ValueId,
        lo: mir::ValueId,
        hi: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let base_val = self.val(base);
        let lo_val = self.val(lo);
        let hi_val = self.val(hi);

        match result_ty {
            Type::Vec(_) => {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i64t = self.ctx.i64_type();
                let slice_fn = self
                    .module
                    .get_function("__jinn_vec_slice")
                    .unwrap_or_else(|| {
                        let ft = ptr_ty.fn_type(&[ptr_ty.into(), i64t.into(), i64t.into()], false);
                        self.module
                            .add_function("__jinn_vec_slice", ft, Some(Linkage::External))
                    });
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[base_val.into(), lo_val.into(), hi_val.into()],
                    "slice"
                ));
                Ok(self.call_result(result))
            }
            Type::String => {
                let st = self.llvm_ty(&Type::String);
                let i64t = self.ctx.i64_type();
                let slice_fn = self
                    .module
                    .get_function("__jinn_str_slice")
                    .unwrap_or_else(|| {
                        let ft = st.fn_type(&[st.into(), i64t.into(), i64t.into()], false);
                        self.module
                            .add_function("__jinn_str_slice", ft, Some(Linkage::External))
                    });
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[base_val.into(), lo_val.into(), hi_val.into()],
                    "str.slice"
                ));
                Ok(self.call_result(result))
            }
            _ => Ok(self.ctx.i8_type().const_int(0, false).into()),
        }
    }
}
