//! Codegen for destructor / drop-glue insertion.

use inkwell::AddressSpace;
use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;

use crate::types::Type;

use super::Compiler;
use super::b;

mod aggregates;
mod containers;

impl<'ctx> Compiler<'ctx> {
    /// Unified drop dispatcher. Emits code to release all resources owned by a
    /// value of the given type. For types that are trivially droppable (scalars,
    /// bools, pointers-as-raw), this is a no-op. For heap-owning types, this
    /// recursively frees inner allocations before releasing the outer container.
    ///
    /// This produces a deterministic, zero-overhead destruction sequence with
    /// no dynamic dispatch and no RTTI. Each drop path is monomorphized at
    /// compile time — the generated code is a flat, branchless (per-type)
    /// sequence of frees. No GC, no finalizer queues.
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
                // Vec: free element storage if elements need dropping, then
                // free the data buffer and header. The element loop runs only
                // for non-trivially-droppable element types. For POD vecs this
                // collapses to two frees.
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
            // Coroutine — needs jinn_gen_destroy to free both coroutine stack and gen block
            Type::Coroutine(_) => {
                self.drop_generator(val)?;
            }
            // Channel — ptr-based, free the allocation if non-null.
            Type::Channel(_) => {
                self.drop_ptr_allocated(val)?;
            }
            _ => {
                // Scalars, bools, raw ptrs, ActorRef — no-op.
                // is_trivially_droppable should have caught these above.
            }
        }
        Ok(())
    }
}
