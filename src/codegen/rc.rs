

use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn rc_layout_ty(&self, inner: &Type) -> inkwell::types::StructType<'ctx> {
        let name = format!("Rc_{inner}");
        self.module.get_struct_type(&name).unwrap_or_else(|| {
            let st = self.ctx.opaque_struct_type(&name);
            st.set_body(
                &[
                    self.ctx.i64_type().into(),
                    self.llvm_ty(inner),
                ],
                false,
            );
            st
        })
    }

    pub(crate) fn rc_alloc(
        &mut self,
        inner: &Type,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let layout = self.rc_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let size = layout.size_of().expect("ICE: type has no size");
        let needed_bytes = size.get_zero_extended_constant().unwrap_or(8);




        let heap_ptr = if let Some(reused) = self.try_consume_reuse_token(needed_bytes) {

            let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
            let is_null = b!(self.bld.build_is_null(reused, "rc.alloc.reuse.null"));
            let fv = self.current_fn();
            let malloc_bb = self.ctx.append_basic_block(fv, "rc.alloc.malloc");
            let cont_bb = self.ctx.append_basic_block(fv, "rc.alloc.cont");
            let entry_bb = self
                .bld
                .get_insert_block()
                .expect("builder has no insert block");
            b!(self
                .bld
                .build_conditional_branch(is_null, malloc_bb, cont_bb));
            self.bld.position_at_end(malloc_bb);
            let malloc = self.ensure_malloc();
            let m = b!(self.bld.build_call(malloc, &[size.into()], "rc.alloc"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_pointer_value();
            b!(self.bld.build_unconditional_branch(cont_bb));
            self.bld.position_at_end(cont_bb);
            let phi = b!(self.bld.build_phi(ptr_ty, "rc.alloc.phi"));
            phi.add_incoming(&[(&m, malloc_bb), (&reused, entry_bb)]);
            phi.as_basic_value().into_pointer_value()
        } else {
            let malloc = self.ensure_malloc();
            b!(self.bld.build_call(malloc, &[size.into()], "rc.alloc"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_pointer_value()
        };
        let rc_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "rc.cnt"));
        b!(self.bld.build_store(rc_gep, i64t.const_int(1, false)));
        let val_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "rc.val"));
        b!(self.bld.build_store(val_gep, val));
        Ok(heap_ptr.into())
    }

    pub(crate) fn rc_retain(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        self.rc_retain_impl(ptr, inner, inner.needs_atomic_rc())
    }




    fn rc_retain_impl(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
        atomic: bool,
    ) -> Result<(), String> {
        let layout = self.rc_layout_ty(inner);
        let rc_gep = b!(self
            .bld
            .build_struct_gep(layout, ptr.into_pointer_value(), 0, "rc.cnt"));
        if atomic {
            b!(self.bld.build_atomicrmw(
                inkwell::AtomicRMWBinOp::Add,
                rc_gep,
                self.ctx.i64_type().const_int(1, false),
                inkwell::AtomicOrdering::AcquireRelease,
            ));
        } else {
            let i64t = self.ctx.i64_type();
            let old = b!(self.bld.build_load(i64t, rc_gep, "rc.cnt.ld")).into_int_value();
            let inc = b!(self
                .bld
                .build_int_nuw_add(old, i64t.const_int(1, false), "rc.inc"));
            b!(self.bld.build_store(rc_gep, inc));
        }
        Ok(())
    }

    pub(crate) fn rc_release(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        self.rc_release_impl(ptr, inner, inner.needs_atomic_rc())
    }


    fn rc_release_impl(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
        atomic: bool,
    ) -> Result<(), String> {
        let fv = self.current_fn();
        let layout = self.rc_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let heap_ptr = ptr.into_pointer_value();
        let rc_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "rc.cnt"));
        let old = if atomic {
            b!(self.bld.build_atomicrmw(
                inkwell::AtomicRMWBinOp::Sub,
                rc_gep,
                i64t.const_int(1, false),
                inkwell::AtomicOrdering::AcquireRelease,
            ))
        } else {
            let loaded = b!(self.bld.build_load(i64t, rc_gep, "rc.cnt.ld")).into_int_value();
            let dec = b!(self
                .bld
                .build_int_nsw_sub(loaded, i64t.const_int(1, false), "rc.dec"));
            b!(self.bld.build_store(rc_gep, dec));
            loaded
        };
        let is_zero = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            old,
            i64t.const_int(1, false),
            "rc.dead"
        ));
        let dead_bb = self.ctx.append_basic_block(fv, "rc.dead");
        let cont_bb = self.ctx.append_basic_block(fv, "rc.cont");
        b!(self.bld.build_conditional_branch(is_zero, dead_bb, cont_bb));
        self.bld.position_at_end(dead_bb);

        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[heap_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(cont_bb));
        self.bld.position_at_end(cont_bb);
        Ok(())
    }

    pub(crate) fn rc_deref(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let layout = self.rc_layout_ty(inner);
        let val_gep = b!(self
            .bld
            .build_struct_gep(layout, ptr.into_pointer_value(), 1, "rc.val"));
        let loaded = b!(self.bld.build_load(self.llvm_ty(inner), val_gep, "rc.load"));






        if inner.is_trivially_droppable() {
            Ok(loaded)
        } else if Self::is_value_clonable(inner) {
            self.clone_value(loaded, inner)
        } else {
            Err(format!(
                "rc_deref: inner type {inner:?} is neither trivially droppable nor clonable; \
                 `@` on Rc<{inner:?}> would alias and double-free"
            ))
        }
    }


















































}
