use inkwell::AddressSpace;
use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;

use crate::types::Type;

use super::Compiler;
use super::b;

mod aggregates;
mod containers;

impl<'ctx> Compiler<'ctx> {
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
                self.drop_vec_deep(val, elem)?;
            }
            Type::Map(kt, vt) => {
                self.drop_map_deep(val, kt, vt)?;
            }
            Type::Generator(_) => {
                self.drop_ptr_allocated(val)?;
            }
            Type::Tuple(tys) => {
                self.drop_tuple(val, tys)?;
            }
            Type::Struct(name, _) => {
                if self.enums.contains_key(&name.as_str()) {
                    self.call_enum_drop_fn(val, &name.as_str())?;
                } else {
                    self.drop_struct_fields(val, &name.as_str())?;
                }
            }
            Type::Row(name) => {
                self.drop_struct_fields(val, &format!("__store_{}", name.as_str()))?;
            }
            Type::Array(elem, n) => {
                if !elem.is_trivially_droppable() {
                    self.drop_array_elements(val, elem, *n)?;
                }
            }
            Type::Enum(name) => {
                self.call_enum_drop_fn(val, &name.as_str())?;
            }
            Type::Alias(_, inner) | Type::Newtype(_, inner) | Type::Frozen(inner) => {
                self.drop_value(val, inner)?;
            }

            Type::Coroutine(_) => {
                self.drop_generator(val)?;
            }

            Type::Channel(_) => {
                if val.is_pointer_value() {
                    let release = self
                        .module
                        .get_function("jinn_chan_release")
                        .unwrap_or_else(|| {
                            let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                            let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
                            self.module.add_function(
                                "jinn_chan_release",
                                ft,
                                Some(inkwell::module::Linkage::External),
                            )
                        });
                    self.needs_runtime = true;
                    b!(self.bld.build_call(release, &[val.into()], "ch.release"));
                }
            }
            Type::Fn(_, _) => {
                self.drop_closure(val)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn drop_closure(&mut self, val: BasicValueEnum<'ctx>) -> Result<(), String> {
        if !val.is_struct_value() {
            return Ok(());
        }
        let sv = val.into_struct_value();
        let env = b!(self.bld.build_extract_value(sv, 1, "cld.env")).into_pointer_value();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let is_null = b!(self.bld.build_is_null(env, "cld.isnull"));
        let fv = self.current_fn();
        let drop_bb = self.ctx.append_basic_block(fv, "cld.drop");
        let done_bb = self.ctx.append_basic_block(fv, "cld.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, drop_bb));
        self.bld.position_at_end(drop_bb);
        let drop_fn_ptr = b!(self.bld.build_load(ptr_ty, env, "cld.dropfn")).into_pointer_value();
        let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
        b!(self
            .bld
            .build_indirect_call(ft, drop_fn_ptr, &[env.into()], "cld.call"));
        b!(self.bld.build_unconditional_branch(done_bb));
        self.bld.position_at_end(done_bb);
        Ok(())
    }
}
