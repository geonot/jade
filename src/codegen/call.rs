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
            lp.extend(
                ptys.iter()
                    .map(|t| BasicMetadataTypeEnum::from(self.llvm_ty(t))),
            );
            let ft = self.mk_fn_type(ret.as_ref(), &lp, false);

            let mut a: Vec<BasicMetadataValueEnum<'ctx>> = vec![env_ptr.into()];
            a.extend(vals.iter().map(|v| BasicMetadataValueEnum::from(*v)));

            let csv = b!(self.bld.build_indirect_call(ft, fn_ptr, &a, "icall"));
            Ok(self.call_result(csv))
        } else {
            Err(format!("cannot call non-function type: {fn_ty}"))
        }
    }
}
