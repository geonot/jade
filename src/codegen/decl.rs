use inkwell::AddressSpace;
use inkwell::attributes::AttributeLoc;

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
        let mut max_payload = 0u64;
        for (vname, ftys, tag) in variants {
            max_payload = max_payload.max(self.variant_layout(name, ftys).size);
            self.variant_tags
                .insert(vname.clone().into(), (name.into(), *tag));
            resolved.push((vname.clone(), ftys.clone()));
        }

        let st = self
            .module
            .get_struct_type(name)
            .unwrap_or_else(|| self.ctx.opaque_struct_type(name));

        if max_payload == 0 {
            st.set_body(&[i32t.into()], false);
            self.enums.insert(name.into(), resolved);
            return Ok(());
        }

        let payload_words = max_payload.div_ceil(8);
        let payload_words = u32::try_from(payload_words)
            .map_err(|_| format!("native enum payload exceeds layout limit: {name}"))?;
        let payload_ty = self.ctx.i64_type().array_type(payload_words);
        st.set_body(&[i32t.into(), payload_ty.into()], false);
        self.enums.insert(name.into(), resolved);
        Ok(())
    }
}
