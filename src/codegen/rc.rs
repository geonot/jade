//! Codegen for Perceus reference-counting ops (`rc.alloc`, `rc.retain`, `rc.release`).

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
                    self.ctx.i64_type().into(),
                    self.llvm_ty(inner),
                ],
                false,
            );
            st
        })
    }

    /// Weak refs share the Rc heap layout; this is a back-compat alias.
    pub(crate) fn weak_layout_ty(&self, inner: &Type) -> inkwell::types::StructType<'ctx> {
        self.rc_layout_ty(inner)
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
        // Perceus reuse: try to reuse a saved heap pointer instead of malloc.
        // The runtime alloca path may return a NULL pointer (slot was empty
        // at runtime), in which case we still need to malloc. We model this
        // with a `phi` over the (reuse-hit, malloc-fallback) branches.
        let heap_ptr = if let Some(reused) = self.try_consume_reuse_token(needed_bytes) {
            // Branch on null at runtime: if NULL, malloc; else use `reused`.
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
        let weak_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "rc.weak"));
        b!(self.bld.build_store(weak_gep, i64t.const_int(0, false)));
        let val_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 2, "rc.val"));
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

    /// Atomic-forced variant used by `Arc<T>` codegen. Layout-identical
    /// to `rc_retain` but always uses an atomic add regardless of
    /// `inner`'s natural atomicity. R3.4.b.
    #[allow(dead_code)]
    pub(crate) fn arc_retain(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        self.rc_retain_impl(ptr, inner, true)
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

    /// Atomic-forced variant used by `Arc<T>` codegen. R3.4.b.
    #[allow(dead_code)]
    pub(crate) fn arc_release(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        self.rc_release_impl(ptr, inner, true)
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
        // Strong refcount hit zero. Free only if no weak refs remain.
        let weak_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "rc.weak"));
        let weak = b!(self.bld.build_load(i64t, weak_gep, "rc.weak.ld")).into_int_value();
        let no_weak = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            weak,
            i64t.const_int(0, false),
            "rc.noweak"
        ));
        let free_bb = self.ctx.append_basic_block(fv, "rc.free");
        b!(self.bld.build_conditional_branch(no_weak, free_bb, cont_bb));
        self.bld.position_at_end(free_bb);
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
            .build_struct_gep(layout, ptr.into_pointer_value(), 2, "rc.val"));
        Ok(b!(self.bld.build_load(
            self.llvm_ty(inner),
            val_gep,
            "rc.load"
        )))
    }

    // weak_layout_ty defined above as alias of rc_layout_ty.

    pub(crate) fn weak_downgrade(
        &mut self,
        rc_ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let layout = self.weak_layout_ty(inner);
        let heap_ptr = rc_ptr.into_pointer_value();
        let weak_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "weak.cnt"));
        b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Add,
            weak_gep,
            self.ctx.i64_type().const_int(1, false),
            inkwell::AtomicOrdering::AcquireRelease,
        ));
        Ok(heap_ptr.into())
    }

    pub(crate) fn weak_upgrade(
        &mut self,
        weak_ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.current_fn();
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
        let fv = self.current_fn();
        let layout = self.weak_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let heap_ptr = ptr.into_pointer_value();
        let weak_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "weak.cnt"));
        let old_weak = b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Sub,
            weak_gep,
            i64t.const_int(1, false),
            inkwell::AtomicOrdering::AcquireRelease,
        ));
        let new_weak =
            b!(self
                .bld
                .build_int_nsw_sub(old_weak, i64t.const_int(1, false), "weak.dec"));

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
            new_weak,
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

    // ─────────────────────────────────────────────────────────────────
    // R3.4.b — Arc / Arc<Mutex> codegen emitters.
    //
    // Layout for `Arc<T>` is identical to `Rc<T>` (`{i64 strong, i64
    // weak, T payload}`); only the *operations* differ (atomic vs.
    // plain). Layout for `Arc<Mutex<T>>` adds a pthread_mutex_t slot
    // between the weak count and the payload.
    //
    // No caller invokes these yet — they will be wired by R3.4.d
    // (the promotion lowering pass) and R3.4.c (codegen dispatch on
    // Type::Arc / Type::Mutex). #[allow(dead_code)] silences the
    // unused warnings until then.
    // ─────────────────────────────────────────────────────────────────

    /// `Arc<T>` alloc — layout-identical to `rc_alloc`, but the
    /// initial refcount is published with release-ordering so that
    /// other threads that later observe the pointer see a fully
    /// initialized payload. Since `rc_alloc` stores the refcount
    /// before any potential publication, the publication ordering is
    /// the responsibility of whatever channel/send moves the pointer
    /// to another thread (already enforced by `runtime/channel.c`).
    #[allow(dead_code)]
    pub(crate) fn arc_alloc(
        &mut self,
        inner: &Type,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.rc_alloc(inner, val)
    }

    /// `Arc<T>` payload deref — layout-identical to `rc_deref`.
    /// The load itself is non-atomic; if the payload is mutable
    /// (`Arc<Mutex<T>>`) callers must wrap reads in `mutex_lock` /
    /// `mutex_unlock`.
    #[allow(dead_code)]
    pub(crate) fn arc_deref(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.rc_deref(ptr, inner)
    }

    /// Layout for `Arc<Mutex<T>>`:
    /// `{ i64 strong, i64 weak, pthread_mutex_t lock, T payload }`.
    /// The mutex slot is sized opaquely as `[i8 x N]` matching the
    /// platform's `sizeof(pthread_mutex_t)`. We can't query that at
    /// compile-time from LLVM, so we use a conservative 64-byte
    /// reservation (sufficient on Linux x86_64 = 40, aarch64 = 48,
    /// macOS = 56). The init/lock/unlock calls go through the platform
    /// pthread symbols (already linked by the scheduler).
    #[allow(dead_code)]
    pub(crate) fn arc_mutex_layout_ty(
        &self,
        inner: &Type,
    ) -> inkwell::types::StructType<'ctx> {
        let name = format!("ArcMutex_{inner}");
        self.module.get_struct_type(&name).unwrap_or_else(|| {
            let st = self.ctx.opaque_struct_type(&name);
            let mutex_slot = self.ctx.i8_type().array_type(64);
            st.set_body(
                &[
                    self.ctx.i64_type().into(),
                    self.ctx.i64_type().into(),
                    mutex_slot.into(),
                    self.llvm_ty(inner),
                ],
                false,
            );
            st
        })
    }

    /// Declare (or fetch) `int pthread_mutex_init(pthread_mutex_t*,
    /// const pthread_mutexattr_t*)`. Returns the FunctionValue.
    #[allow(dead_code)]
    pub(crate) fn ensure_pthread_mutex_init(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module
            .get_function("pthread_mutex_init")
            .unwrap_or_else(|| {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i32t = self.ctx.i32_type();
                let ft = i32t.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
                self.module.add_function(
                    "pthread_mutex_init",
                    ft,
                    Some(inkwell::module::Linkage::External),
                )
            })
    }

    /// Declare (or fetch) `int pthread_mutex_lock(pthread_mutex_t*)`.
    #[allow(dead_code)]
    pub(crate) fn ensure_pthread_mutex_lock(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module
            .get_function("pthread_mutex_lock")
            .unwrap_or_else(|| {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i32t = self.ctx.i32_type();
                let ft = i32t.fn_type(&[ptr_ty.into()], false);
                self.module.add_function(
                    "pthread_mutex_lock",
                    ft,
                    Some(inkwell::module::Linkage::External),
                )
            })
    }

    /// Declare (or fetch) `int pthread_mutex_unlock(pthread_mutex_t*)`.
    #[allow(dead_code)]
    pub(crate) fn ensure_pthread_mutex_unlock(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module
            .get_function("pthread_mutex_unlock")
            .unwrap_or_else(|| {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i32t = self.ctx.i32_type();
                let ft = i32t.fn_type(&[ptr_ty.into()], false);
                self.module.add_function(
                    "pthread_mutex_unlock",
                    ft,
                    Some(inkwell::module::Linkage::External),
                )
            })
    }

    /// Declare (or fetch) `int pthread_mutex_destroy(pthread_mutex_t*)`.
    #[allow(dead_code)]
    pub(crate) fn ensure_pthread_mutex_destroy(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module
            .get_function("pthread_mutex_destroy")
            .unwrap_or_else(|| {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i32t = self.ctx.i32_type();
                let ft = i32t.fn_type(&[ptr_ty.into()], false);
                self.module.add_function(
                    "pthread_mutex_destroy",
                    ft,
                    Some(inkwell::module::Linkage::External),
                )
            })
    }

    /// Emit `pthread_mutex_lock(&arc_ptr->lock)` for an
    /// `Arc<Mutex<T>>` value.
    #[allow(dead_code)]
    pub(crate) fn mutex_lock(
        &mut self,
        arc_ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        let layout = self.arc_mutex_layout_ty(inner);
        let heap_ptr = arc_ptr.into_pointer_value();
        let lock_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 2, "arc.mutex.lock"));
        let lock_fn = self.ensure_pthread_mutex_lock();
        b!(self.bld.build_call(lock_fn, &[lock_gep.into()], "mutex.lock"));
        Ok(())
    }

    /// Emit `pthread_mutex_unlock(&arc_ptr->lock)`.
    #[allow(dead_code)]
    pub(crate) fn mutex_unlock(
        &mut self,
        arc_ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<(), String> {
        let layout = self.arc_mutex_layout_ty(inner);
        let heap_ptr = arc_ptr.into_pointer_value();
        let lock_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 2, "arc.mutex.lock"));
        let unlock_fn = self.ensure_pthread_mutex_unlock();
        b!(self.bld
            .build_call(unlock_fn, &[lock_gep.into()], "mutex.unlock"));
        Ok(())
    }
}
