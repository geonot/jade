use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::types::Type;

use super::b;
use super::Compiler;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn rc_layout_ty(&self, inner: &Type) -> inkwell::types::StructType<'ctx> {
        let name = format!("Rc_{inner}");
        self.module.get_struct_type(&name).unwrap_or_else(|| {
            let st = self.ctx.opaque_struct_type(&name);
            st.set_body(&[self.ctx.i64_type().into(), self.llvm_ty(inner)], false);
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
        let size = layout.size_of().unwrap();
        let malloc = self.ensure_malloc();
        let heap_ptr = b!(self.bld.build_call(malloc, &[size.into()], "rc.alloc"))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
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
        let layout = self.rc_layout_ty(inner);
        let rc_gep = b!(self
            .bld
            .build_struct_gep(layout, ptr.into_pointer_value(), 0, "rc.cnt"));
        b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Add,
            rc_gep,
            self.ctx.i64_type().const_int(1, false),
            inkwell::AtomicOrdering::AcquireRelease,
        ));
        Ok(())
    }

    pub(crate) fn rc_release(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let layout = self.rc_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let heap_ptr = ptr.into_pointer_value();
        let rc_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "rc.cnt"));
        let old = b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Sub,
            rc_gep,
            i64t.const_int(1, false),
            inkwell::AtomicOrdering::AcquireRelease,
        ));
        let is_zero = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            old,
            i64t.const_int(1, false),
            "rc.dead"
        ));
        let free_bb = self.ctx.append_basic_block(fv, "rc.free");
        let cont_bb = self.ctx.append_basic_block(fv, "rc.cont");
        b!(self.bld.build_conditional_branch(is_zero, free_bb, cont_bb));
        self.bld.position_at_end(free_bb);
        let free_fn = self.module.get_function("free").unwrap_or_else(|| {
            self.module.add_function(
                "free",
                self.ctx.void_type().fn_type(&[ptr_ty.into()], false),
                Some(Linkage::External),
            )
        });
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
        Ok(b!(self.bld.build_load(
            self.llvm_ty(inner),
            val_gep,
            "rc.load"
        )))
    }

    pub(crate) fn weak_layout_ty(&self, inner: &Type) -> inkwell::types::StructType<'ctx> {
        let name = format!("Weak_{inner}");
        self.module.get_struct_type(&name).unwrap_or_else(|| {
            let st = self.ctx.opaque_struct_type(&name);
            st.set_body(
                &[
                    self.ctx.i64_type().into(),
                    self.ctx.i64_type().into(),
                    self.llvm_ty(inner),
                ],
                false,
            );
            st
        })
    }

    pub(crate) fn weak_downgrade(
        &mut self,
        rc_ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let layout = self.weak_layout_ty(inner);
        let heap_ptr = rc_ptr.into_pointer_value();
        let weak_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "weak.cnt"));
        let old = b!(self
            .bld
            .build_load(self.ctx.i64_type(), weak_gep, "weak.old"))
        .into_int_value();
        let new = b!(self.bld.build_int_nuw_add(
            old,
            self.ctx.i64_type().const_int(1, false),
            "weak.inc"
        ));
        b!(self.bld.build_store(weak_gep, new));
        Ok(heap_ptr.into())
    }

    pub(crate) fn weak_upgrade(
        &mut self,
        weak_ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let layout = self.weak_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let heap_ptr = weak_ptr.into_pointer_value();
        let strong_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "strong.cnt"));
        let strong = b!(self.bld.build_load(i64t, strong_gep, "strong")).into_int_value();
        let is_alive = b!(self.bld.build_int_compare(
            IntPredicate::SGT,
            strong,
            i64t.const_int(0, false),
            "alive"
        ));
        let alive_bb = self.ctx.append_basic_block(fv, "weak.alive");
        let dead_bb = self.ctx.append_basic_block(fv, "weak.dead");
        let merge_bb = self.ctx.append_basic_block(fv, "weak.merge");
        b!(self
            .bld
            .build_conditional_branch(is_alive, alive_bb, dead_bb));

        self.bld.position_at_end(alive_bb);
        let new_strong =
            b!(self
                .bld
                .build_int_nuw_add(strong, i64t.const_int(1, false), "strong.inc"));
        b!(self.bld.build_store(strong_gep, new_strong));
        let alive_val: BasicValueEnum<'ctx> = heap_ptr.into();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(dead_bb);
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let null_val: BasicValueEnum<'ctx> = ptr_ty.const_null().into();
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(ptr_ty, "weak.result"));
        phi.add_incoming(&[(&alive_val, alive_bb), (&null_val, dead_bb)]);
        Ok(phi.as_basic_value())
    }

    pub(crate) fn weak_release(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let layout = self.weak_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let heap_ptr = ptr.into_pointer_value();
        let weak_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "weak.cnt"));
        let old = b!(self.bld.build_load(i64t, weak_gep, "weak.old")).into_int_value();
        let new = b!(self
            .bld
            .build_int_nsw_sub(old, i64t.const_int(1, false), "weak.dec"));
        b!(self.bld.build_store(weak_gep, new));

        let strong_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "strong.cnt"));
        let strong = b!(self.bld.build_load(i64t, strong_gep, "strong")).into_int_value();
        let strong_zero = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            strong,
            i64t.const_int(0, false),
            "s.zero"
        ));
        let weak_zero = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            new,
            i64t.const_int(0, false),
            "w.zero"
        ));
        let both_zero = b!(self.bld.build_and(strong_zero, weak_zero, "both.zero"));

        let free_bb = self.ctx.append_basic_block(fv, "weak.free");
        let cont_bb = self.ctx.append_basic_block(fv, "weak.cont");
        b!(self
            .bld
            .build_conditional_branch(both_zero, free_bb, cont_bb));

        self.bld.position_at_end(free_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[heap_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        Ok(())
    }
}
