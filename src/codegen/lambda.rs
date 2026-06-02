use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType};
use inkwell::values::BasicValueEnum;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn make_closure(
        &mut self,
        fn_ptr: inkwell::values::PointerValue<'ctx>,
        env_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ct = self.closure_type();
        let mut sv = ct.const_zero();
        sv = b!(self.bld.build_insert_value(sv, fn_ptr, 0, "cl.fn")).into_struct_value();
        sv = b!(self.bld.build_insert_value(sv, env_ptr, 1, "cl.env")).into_struct_value();
        Ok(sv.into())
    }

    pub(crate) fn fn_ref_wrapper(
        &mut self,
        fv: inkwell::values::FunctionValue<'ctx>,
    ) -> inkwell::values::PointerValue<'ctx> {
        let wrapper_name = format!("{}.cl_wrap", fv.get_name().to_str().unwrap_or("fn"));

        if let Some(w) = self.module.get_function(&wrapper_name) {
            return w.as_global_value().as_pointer_value();
        }
        let original_type = fv.get_type();
        let original_params = original_type.get_param_types();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let mut wrapper_params: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
        wrapper_params.extend(
            original_params
                .iter()
                .map(|t| BasicMetadataTypeEnum::from(*t)),
        );
        let wrapper_ft = match original_type.get_return_type() {
            Some(ret) => ret.fn_type(&wrapper_params, false),
            None => self.ctx.void_type().fn_type(&wrapper_params, false),
        };
        let wrapper_fv =
            self.module
                .add_function(&wrapper_name, wrapper_ft, Some(Linkage::Internal));
        wrapper_fv.add_attribute(
            inkwell::attributes::AttributeLoc::Function,
            self.attr("nounwind"),
        );
        wrapper_fv.add_attribute(
            inkwell::attributes::AttributeLoc::Function,
            self.attr("alwaysinline"),
        );

        let saved_bb = self.bld.get_insert_block();
        let entry = self.ctx.append_basic_block(wrapper_fv, "entry");
        self.bld.position_at_end(entry);

        let args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = (1..wrapper_fv
            .count_params())
            .map(|i| wrapper_fv.get_nth_param(i).unwrap().into())
            .collect();
        let result = self.bld.build_call(fv, &args, "wrap").unwrap();
        match original_type.get_return_type() {
            Some(_) => {
                self.bld
                    .build_return(Some(&self.call_result(result)))
                    .unwrap();
            }
            None => {
                self.bld.build_return(None).unwrap();
            }
        };

        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }
        wrapper_fv.as_global_value().as_pointer_value()
    }

}
