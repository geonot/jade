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
                self.drop_struct_fields(val, &name.as_str())?;
            }
            Type::Array(elem, n) => {
                if !elem.is_trivially_droppable() {
                    self.drop_array_elements(val, elem, *n)?;
                }
            }
            Type::Enum(name) => {
                self.drop_enum_variants(val, &name.as_str())?;
            }
            Type::Alias(_, inner) | Type::Newtype(_, inner) => {
                self.drop_value(val, inner)?;
            }

            Type::Coroutine(_) => {
                self.drop_generator(val)?;
            }

            Type::Channel(_) => {
                self.drop_ptr_allocated(val)?;
            }
            _ => {}
        }
        Ok(())
    }
}
