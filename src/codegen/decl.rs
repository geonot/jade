//! Codegen for top-level declarations: functions, globals, type metadata.

use inkwell::attributes::AttributeLoc;
use inkwell::module::Linkage;
use inkwell::types::BasicMetadataTypeEnum;

use inkwell::AddressSpace;

use crate::hir;
use crate::types::Type;

use super::Compiler;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn declare_builtins(&mut self) {
        let i32t = self.ctx.i32_type();
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let pf = self
            .module
            .add_function("printf", i32t.fn_type(&[ptr.into()], true), None);
        pf.add_attribute(AttributeLoc::Function, self.attr("nounwind"));
        pf.add_attribute(AttributeLoc::Function, self.attr("nofree"));
        let pc = self
            .module
            .add_function("putchar", i32t.fn_type(&[i32t.into()], false), None);
        pc.add_attribute(AttributeLoc::Function, self.attr("nounwind"));
        pc.add_attribute(AttributeLoc::Function, self.attr("nofree"));
    }

    pub(crate) fn declare_method(&mut self, _type_name: &str, m: &hir::Fn) -> Result<(), String> {
        let method_name = m.name.clone();
        let ptys: Vec<Type> = m.params.iter().map(|p| p.ty.clone()).collect();
        let ret = m.ret.clone();
        let lp: Vec<BasicMetadataTypeEnum<'ctx>> =
            ptys.iter().map(|t| self.llvm_ty(t).into()).collect();
        let ft = self.mk_fn_type(&ret, &lp, false);
        let fv = self.module.add_function(&method_name.as_str(), ft, None);
        self.tag_fn(fv);
        fv.set_linkage(Linkage::Internal);
        for (i, p) in m.params.iter().enumerate() {
            let loc = AttributeLoc::Param(i as u32);
            fv.add_attribute(loc, self.attr("noundef"));
            self.tag_param_ownership(fv, loc, &p.ownership, &p.ty);
        }
        self.fns.insert(method_name, (fv, ptys, ret));
        Ok(())
    }

    pub(crate) fn declare_enum(&mut self, ed: &hir::EnumDef) -> Result<(), String> {
        let variants: Vec<(String, Vec<Type>, u32)> = ed
            .variants
            .iter()
            .map(|v| {
                let ftys: Vec<Type> = v.fields.iter().map(|f| f.ty.clone()).collect();
                (v.name.as_str(), ftys, v.tag)
            })
            .collect();
        self.declare_tagged_union(&ed.name.as_str(), &variants)
    }

    pub(crate) fn declare_err_def(&mut self, ed: &hir::ErrDef) -> Result<(), String> {
        let variants: Vec<(String, Vec<Type>, u32)> = ed
            .variants
            .iter()
            .map(|v| (v.name.as_str(), v.fields.clone(), v.tag))
            .collect();
        self.declare_tagged_union(&ed.name.as_str(), &variants)
    }

    fn declare_tagged_union(
        &mut self,
        name: &str,
        variants: &[(String, Vec<Type>, u32)],
    ) -> Result<(), String> {
        let i32t = self.ctx.i32_type();
        let mut resolved = Vec::new();
        let mut max_payload = 0usize;
        for (vname, ftys, tag) in variants {
            let mut payload_bytes: usize = 0;
            for t in ftys {
                let size = if Self::is_recursive_field(t, name) {
                    8
                } else {
                    self.type_store_size(self.llvm_ty(t)) as usize
                };
                payload_bytes += (size + 7) & !7;
            }
            max_payload = max_payload.max(payload_bytes);
            self.variant_tags
                .insert(vname.clone().into(), (name.into(), *tag));
            resolved.push((vname.clone(), ftys.clone()));
        }

        if max_payload == 0 {
            let st = self.ctx.opaque_struct_type(name);
            st.set_body(&[i32t.into()], false);
            self.enums.insert(name.into(), resolved);
            return Ok(());
        }

        if variants.len() == 2 {
            let empty_idx = variants.iter().position(|(_, fs, _)| fs.is_empty());
            let payload_idx = variants.iter().position(|(_, fs, _)| fs.len() == 1);
            if let (Some(_ei), Some(pi)) = (empty_idx, payload_idx) {
                let field_ty = &variants[pi].1[0];
                let is_ptr_like = matches!(
                    field_ty,
                    Type::String
                        | Type::Weak(_)
                        | Type::Fn(_, _)
                ) || matches!(field_ty, Type::Struct(_, _) | Type::Enum(_))
                    && !Self::is_recursive_field(field_ty, name);
                if is_ptr_like {
                    let ptr = self.ctx.ptr_type(inkwell::AddressSpace::default());
                    let st = self.ctx.opaque_struct_type(name);
                    st.set_body(&[ptr.into()], false);
                    self.enums.insert(name.into(), resolved);
                    return Ok(());
                }
            }
        }

        let payload_ty = self.ctx.i8_type().array_type(max_payload as u32);
        let st = self.ctx.opaque_struct_type(name);
        st.set_body(&[i32t.into(), payload_ty.into()], false);
        self.enums.insert(name.into(), resolved);
        Ok(())
    }
}
