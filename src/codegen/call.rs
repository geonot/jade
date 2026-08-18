use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum};

use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn indirect_call_vals(
        &mut self,
        closure_val: BasicValueEnum<'ctx>,
        fn_ty: &Type,
        vals: &[BasicValueEnum<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Type::Fn(ptys, ret) = fn_ty {
            let sv = closure_val.into_struct_value();
            let fn_ptr = b!(self.bld.build_extract_value(sv, 0, "cl.fn")).into_pointer_value();
            let env_ptr = b!(self.bld.build_extract_value(sv, 1, "cl.env"));

            let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
            let mut lp: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
            let mut a: Vec<BasicMetadataValueEnum<'ctx>> = vec![env_ptr.into()];
            for (i, v) in vals.iter().enumerate() {
                match ptys.get(i) {
                    Some(t @ (Type::Struct(_, _) | Type::Tuple(_) | Type::Enum(_))) => {
                        lp.push(ptr_ty.into());
                        if v.is_pointer_value() {
                            a.push((*v).into());
                        } else {
                            let slot = self.entry_alloca(self.llvm_ty(t), "icall.spill");
                            b!(self.bld.build_store(slot, *v));
                            a.push(slot.into());
                        }
                    }
                    Some(t) => {
                        lp.push(BasicMetadataTypeEnum::from(self.llvm_ty(t)));
                        a.push((*v).into());
                    }
                    None => {
                        lp.push(v.get_type().into());
                        a.push((*v).into());
                    }
                }
            }
            let ft = self.mk_fn_type(ret.as_ref(), &lp, false);

            let csv = b!(self.bld.build_indirect_call(ft, fn_ptr, &a, "icall"));
            Ok(self.call_result(csv))
        } else {
            Err(format!("cannot call non-function type: {fn_ty}"))
        }
    }
}
