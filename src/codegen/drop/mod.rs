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
            Type::Set(elem) => {
                self.drop_set_deep(val, elem)?;
            }
            Type::Rc(inner) => {
                self.rc_release_deep(val, inner)?;
            }
            // R3.4.c: RcCell<T> shares Rc's non-atomic refcount and the
            // same deep-drop path on last release.
            Type::RcCell(inner) => {
                self.rc_release_deep(val, inner)?;
            }
            // R3.4.c: Arc<T> uses atomic refcount; share the deep-drop
            // path via the same helper (rc_release_deep already routes
            // on inner.needs_atomic_rc() — see src/codegen/rc.rs line
            // 165). For Arc<T> the inner type's atomicity may be false
            // (e.g. Arc<String>), so we must force atomic. The dedicated
            // arc_release_deep does that.
            Type::Arc(inner) => {
                self.arc_release_deep(val, inner)?;
            }
            // Mutex<T> standalone is meaningful only inside Arc<Mutex<T>>;
            // the outer Arc's release runs the inner Mutex drop. Reaching
            // this arm means a bare Mutex<T> is being dropped — should
            // never happen post-promotion.
            Type::Mutex(_) => {
                return Err(
                    "drop_value: bare Mutex<T> cannot be dropped; must live inside Arc<Mutex<T>>"
                        .to_string(),
                );
            }
            Type::Weak(inner) => {
                self.weak_release(val, inner)?;
            }
            Type::Arena => {
                self.drop_arena(val)?;
            }
            Type::Deque(elem) => {
                self.drop_container_header(val, elem)?;
            }
            Type::PriorityQueue(elem) => {
                self.drop_container_header(val, elem)?;
            }
            Type::NDArray(_, _) => {
                self.drop_ndarray(val)?;
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
            // Channel, Cow — ptr-based, free the allocation if non-null.
            Type::Channel(_) | Type::Cow(_) => {
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
