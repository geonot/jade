//! Codegen for value-clone glue.
//!
//! `clone_value(val, ty)` is the dual of `drop_value(val, ty)`: it produces a
//! fresh, independently-owned copy of `val`. Used by container readers like
//! `vec.get(i)` / `map.get(k)` so the bound local owns its value and can be
//! freely dropped, returned, or pushed into another container without aliasing
//! the source.
//!
//! Cost model:
//! - Trivial scalars / pointers / Rc-like cells: O(1) (value or RC bump).
//! - Strings: O(len) — sso path is just a 24-byte struct copy.
//! - Vec / Array of POD: one malloc + memcpy.
//! - Vec / Array of heap elements: one malloc + per-element clone loop.
//! - Struct / Tuple: per-field clone, generated inline.
//! - Containers we don't yet deep-clone (Map, Set, Deque, PriorityQueue,
//!   Enum, NDArray, Channel, Coroutine, Generator) report `false` from
//!   `is_value_clonable`. The typer marks bindings to those as `Borrowed`
//!   to suppress the drop, and rejects attempts to consume the alias.

use inkwell::AddressSpace;
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, PointerValue};

use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    /// Static predicate: does codegen know how to deep-clone this type?
    /// Trivially-droppable types (scalars, pointers, etc.) are always
    /// "clonable" because cloning is the identity (the value itself is the copy).
    /// Delegates to `Type::is_value_clonable` so the typer and codegen agree.
    pub(crate) fn is_value_clonable(ty: &Type) -> bool {
        ty.is_value_clonable()
    }

    /// Emit IR producing a fresh, independently-owned copy of `val` of type `ty`.
    /// Pre: `is_value_clonable(ty)` returns true.
    pub(crate) fn clone_value(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if ty.is_trivially_droppable() {
            return Ok(val);
        }
        match ty {
            Type::String => self.clone_string(val),
            Type::Vec(elem) => self.clone_vec(val, elem),
            Type::Array(elem, n) => self.clone_array(val, elem, *n),
            Type::Tuple(tys) => self.clone_tuple(val, tys),
            Type::Struct(name, _) => self.clone_struct(val, &name.as_str()),
            Type::Rc(_) | Type::Weak(_) => self.bump_ptr_rc(val),
            // R3.4.c: RcCell<T> shares Rc's non-atomic refcount layout
            // (single-threaded interior-mut alias). Clone = same bump.
            Type::RcCell(_) => self.bump_ptr_rc(val),
            // R3.4.c: Arc<T> uses an atomic refcount bump. Layout-
            // identical to Rc, but the operation must be atomic so
            // cross-thread aliases observe a consistent count.
            Type::Arc(_) => self.bump_ptr_arc(val),
            // Mutex<T> standalone is meaningful only inside Arc<Mutex<T>>;
            // the outer Arc carries the clone. Reaching this arm means a
            // bare Mutex value is being cloned, which the promotion pass
            // never emits.
            Type::Mutex(_) => Err(
                "clone_value: bare Mutex<T> is not clonable; must be wrapped in Arc<Mutex<T>>"
                    .to_string(),
            ),
            Type::Alias(_, inner) | Type::Newtype(_, inner) => self.clone_value(val, inner),
            other => Err(format!(
                "clone_value: unsupported type {:?} (caller should have checked is_value_clonable)",
                other
            )),
        }
    }

    /// Clone a String. Calls __jinn_str_clone in the runtime.
    /// SSO inline path is a 24-byte memcpy (no allocation); heap path mallocs
    /// and copies the buffer.
    fn clone_string(&mut self, val: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        // Pointer-based ABI: `void __jinn_str_clone(jinn_sso_t *out, const jinn_sso_t *src)`.
        // The struct is 24 bytes; System V AMD64 passes it via memory and LLVM's
        // implicit by-value lowering disagrees with clang's, so we marshal explicitly.
        let st = self.string_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let void_ty = self.ctx.void_type();
        let f = self.module.get_function("__jinn_str_clone").unwrap_or_else(|| {
            let ft = void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("__jinn_str_clone", ft, None)
        });
        self.needs_runtime = true;
        let out_slot = self.entry_alloca(st.into(), "str.clone.out");
        let in_slot = self.entry_alloca(st.into(), "str.clone.in");
        b!(self.bld.build_store(in_slot, val));
        b!(self
            .bld
            .build_call(f, &[out_slot.into(), in_slot.into()], "str.clone"));
        let ret = b!(self.bld.build_load(st, out_slot, "str.clone.val"));
        Ok(ret)
    }

    /// Clone a Vec. If the element type is trivially droppable, defer to the
    /// runtime helper __jinn_vec_clone_pod (single malloc + memcpy). Otherwise
    /// emit an IR loop that clones each element.
    fn clone_vec(
        &mut self,
        val: BasicValueEnum<'ctx>,
        elem: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let lty = self.llvm_ty(elem);
        let elem_size = self.type_store_size(lty);

        if elem.is_trivially_droppable() {
            let f = self
                .module
                .get_function("__jinn_vec_clone_pod")
                .unwrap_or_else(|| {
                    let ft = ptr_ty.fn_type(&[ptr_ty.into(), i64t.into()], false);
                    self.module
                        .add_function("__jinn_vec_clone_pod", ft, None)
                });
            self.needs_runtime = true;
            let elem_size_v = i64t.const_int(elem_size, false);
            let ret = b!(self.bld.build_call(
                f,
                &[val.into(), elem_size_v.into()],
                "vec.clone"
            ))
            .try_as_basic_value()
            .basic()
            .expect("ICE: __jinn_vec_clone_pod returned void");
            return Ok(ret);
        }

        // Deep-clone path: handle null source, allocate new header + buffer,
        // loop and clone each element.
        let fv = self.current_fn();
        let malloc = self.ensure_malloc();
        let src_ptr = val.into_pointer_value();
        let null = ptr_ty.const_null();

        let pre_bb = self.current_bb();
        let is_null = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            src_ptr,
            null,
            "vc.null"
        ));
        let alloc_bb = self.ctx.append_basic_block(fv, "vc.alloc");
        let join_bb = self.ctx.append_basic_block(fv, "vc.join");
        b!(self.bld.build_conditional_branch(is_null, join_bb, alloc_bb));

        // alloc_bb: allocate new header, copy len/cap, allocate new buffer if cap>0,
        // then loop-clone each element.
        self.bld.position_at_end(alloc_bb);
        let new_hdr = b!(self
            .bld
            .build_call(malloc, &[i64t.const_int(24, false).into()], "vc.hdr"))
        .try_as_basic_value()
        .basic()
        .expect("ICE: malloc returned void")
        .into_pointer_value();

        let src_len_gep = b!(self.bld.build_struct_gep(header_ty, src_ptr, 1, "vc.slenp"));
        let src_len = b!(self.bld.build_load(i64t, src_len_gep, "vc.slen")).into_int_value();
        let src_cap_gep = b!(self.bld.build_struct_gep(header_ty, src_ptr, 2, "vc.scapp"));
        let src_cap = b!(self.bld.build_load(i64t, src_cap_gep, "vc.scap")).into_int_value();
        let src_data_gep = b!(self.bld.build_struct_gep(header_ty, src_ptr, 0, "vc.sdatap"));
        let src_data =
            b!(self.bld.build_load(ptr_ty, src_data_gep, "vc.sdata")).into_pointer_value();

        let new_len_gep = b!(self.bld.build_struct_gep(header_ty, new_hdr, 1, "vc.dlenp"));
        b!(self.bld.build_store(new_len_gep, src_len));
        let new_cap_gep = b!(self.bld.build_struct_gep(header_ty, new_hdr, 2, "vc.dcapp"));
        b!(self.bld.build_store(new_cap_gep, src_cap));

        // Allocate new buffer if cap > 0.
        let cap_zero = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            src_cap,
            i64t.const_int(0, false),
            "vc.capz"
        ));
        let buf_bb = self.ctx.append_basic_block(fv, "vc.buf");
        let nobuf_bb = self.ctx.append_basic_block(fv, "vc.nobuf");
        b!(self
            .bld
            .build_conditional_branch(cap_zero, nobuf_bb, buf_bb));

        self.bld.position_at_end(nobuf_bb);
        let new_data_gep_z = b!(self.bld.build_struct_gep(header_ty, new_hdr, 0, "vc.ddatapz"));
        b!(self.bld.build_store(new_data_gep_z, null));
        b!(self.bld.build_unconditional_branch(join_bb));

        self.bld.position_at_end(buf_bb);
        let elem_size_v = i64t.const_int(elem_size, false);
        let buf_bytes = b!(self.bld.build_int_mul(src_cap, elem_size_v, "vc.bufsz"));
        let new_buf = b!(self.bld.build_call(malloc, &[buf_bytes.into()], "vc.buf"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: malloc returned void")
            .into_pointer_value();
        let new_data_gep = b!(self.bld.build_struct_gep(header_ty, new_hdr, 0, "vc.ddatap"));
        b!(self.bld.build_store(new_data_gep, new_buf));

        // Loop: for i in 0..len { dst[i] = clone(src[i]) }
        let loop_bb = self.ctx.append_basic_block(fv, "vc.loop");
        let body_bb = self.ctx.append_basic_block(fv, "vc.body");
        let post_bb = self.ctx.append_basic_block(fv, "vc.post");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let i_phi = b!(self.bld.build_phi(i64t, "vc.i"));
        i_phi.add_incoming(&[(&i64t.const_int(0, false), buf_bb)]);
        let i = i_phi.as_basic_value().into_int_value();
        let cont = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::ULT,
            i,
            src_len,
            "vc.ilt"
        ));
        b!(self.bld.build_conditional_branch(cont, body_bb, post_bb));

        self.bld.position_at_end(body_bb);
        let off = b!(self.bld.build_int_mul(i, elem_size_v, "vc.off"));
        let s_ep = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), src_data, &[off], "vc.sep"))
        };
        let d_ep = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), new_buf, &[off], "vc.dep"))
        };
        let s_v = b!(self.bld.build_load(lty, s_ep, "vc.sv"));
        let cloned = self.clone_value(s_v, elem)?;
        b!(self.bld.build_store(d_ep, cloned));
        let after = self.current_bb();
        let next = b!(self
            .bld
            .build_int_add(i, i64t.const_int(1, false), "vc.next"));
        i_phi.add_incoming(&[(&next, after)]);
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(post_bb);
        b!(self.bld.build_unconditional_branch(join_bb));

        // Join: phi over [null path, nobuf path, post path]
        self.bld.position_at_end(join_bb);
        let phi = b!(self.bld.build_phi(ptr_ty, "vc.r"));
        // For the null source we re-use src_ptr (= null) so dropping the
        // result is also a no-op.
        phi.add_incoming(&[
            (&src_ptr, pre_bb),
            (&new_hdr, nobuf_bb),
            (&new_hdr, post_bb),
        ]);
        Ok(phi.as_basic_value())
    }

    /// Clone a fixed-size Array. Allocates an alloca on the stack (matching
    /// existing array codegen) and clones each element.
    fn clone_array(
        &mut self,
        val: BasicValueEnum<'ctx>,
        elem: &Type,
        n: usize,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if elem.is_trivially_droppable() {
            // Aggregate copy is just a value-level copy.
            return Ok(val);
        }
        let lty = self.llvm_ty(elem);
        let arr_ty = lty.array_type(n as u32);
        let dst = self.entry_alloca(arr_ty.into(), "arr.clone");
        // Store source aggregate into a tmp so we can GEP into it.
        let src_slot = self.entry_alloca(arr_ty.into(), "arr.csrc");
        b!(self.bld.build_store(src_slot, val));
        let i64t = self.ctx.i64_type();
        for i in 0..n {
            let idx = i64t.const_int(i as u64, false);
            let s_ep = unsafe {
                b!(self.bld.build_gep(
                    arr_ty,
                    src_slot,
                    &[i64t.const_zero(), idx],
                    "arr.sep"
                ))
            };
            let d_ep = unsafe {
                b!(self.bld.build_gep(
                    arr_ty,
                    dst,
                    &[i64t.const_zero(), idx],
                    "arr.dep"
                ))
            };
            let s_v = b!(self.bld.build_load(lty, s_ep, "arr.sv"));
            let cloned = self.clone_value(s_v, elem)?;
            b!(self.bld.build_store(d_ep, cloned));
        }
        Ok(b!(self.bld.build_load(arr_ty, dst, "arr.cv")))
    }

    /// Clone a Tuple by cloning each field.
    fn clone_tuple(
        &mut self,
        val: BasicValueEnum<'ctx>,
        tys: &[Type],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if tys.iter().all(|t| t.is_trivially_droppable()) {
            return Ok(val);
        }
        let mut field_tys = Vec::with_capacity(tys.len());
        for t in tys {
            field_tys.push(self.llvm_ty(t));
        }
        let tuple_ty = self.ctx.struct_type(&field_tys, false);
        let src_slot = self.entry_alloca(tuple_ty.into(), "tup.csrc");
        b!(self.bld.build_store(src_slot, val));
        let dst_slot = self.entry_alloca(tuple_ty.into(), "tup.cdst");
        for (i, t) in tys.iter().enumerate() {
            let s_ep = b!(self.bld.build_struct_gep(tuple_ty, src_slot, i as u32, "tup.sep"));
            let d_ep = b!(self.bld.build_struct_gep(tuple_ty, dst_slot, i as u32, "tup.dep"));
            let s_v = b!(self.bld.build_load(field_tys[i], s_ep, "tup.sv"));
            let cloned = self.clone_value(s_v, t)?;
            b!(self.bld.build_store(d_ep, cloned));
        }
        Ok(b!(self.bld.build_load(tuple_ty, dst_slot, "tup.cv")))
    }

    /// Clone a Struct value by cloning each field. Looks up the schema by
    /// name. The result has the same struct type as the source.
    ///
    /// For self-referential struct types (e.g. `Value` with a `Vec of Value`
    /// field), the recursive inline emission would never terminate. In that
    /// case we materialize an out-of-line helper
    /// `__clone_<Name>(out_ptr, in_ptr)` and call it; the helper body uses the
    /// same recursive emission, but every nested clone of the same struct now
    /// resolves to a call back to the helper rather than another inline
    /// expansion. Mirrors `drop_struct_fields`.
    fn clone_struct(
        &mut self,
        val: BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fields = match self.structs.get(name).cloned() {
            Some(f) => f,
            None => return Ok(val), // unknown struct — treat as opaque
        };
        let trivial = fields.iter().all(|(_, t)| t.is_trivially_droppable());
        if trivial {
            return Ok(val);
        }
        let lty_st = match self.module.get_struct_type(name) {
            Some(s) => s,
            None => return Ok(val),
        };

        let clone_fn_name = format!("__clone_{}", name);

        // If the helper already exists (or this is a recursive re-entry while
        // we are mid-emit of it), just call it.
        if let Some(cfn) = self.module.get_function(&clone_fn_name) {
            let src_slot = self.entry_alloca(lty_st.into(), "stc.src");
            let dst_slot = self.entry_alloca(lty_st.into(), "stc.dst");
            b!(self.bld.build_store(src_slot, val));
            b!(self.bld.build_call(cfn, &[dst_slot.into(), src_slot.into()], ""));
            return Ok(b!(self.bld.build_load(lty_st, dst_slot, "stc.cv")));
        }

        let is_recursive = fields
            .iter()
            .any(|(_, ty)| Self::type_references_struct_for_clone(ty, name));

        if !is_recursive {
            // Inline the per-field clone.
            let src_slot = self.entry_alloca(lty_st.into(), "stc.src");
            b!(self.bld.build_store(src_slot, val));
            let dst_slot = self.entry_alloca(lty_st.into(), "stc.dst");
            for (i, (_, ft)) in fields.iter().enumerate() {
                let lf = self.llvm_ty(ft);
                let s_ep = b!(self.bld.build_struct_gep(lty_st, src_slot, i as u32, "stc.sep"));
                let d_ep = b!(self.bld.build_struct_gep(lty_st, dst_slot, i as u32, "stc.dep"));
                let s_v = b!(self.bld.build_load(lf, s_ep, "stc.sv"));
                let cloned = self.clone_value(s_v, ft)?;
                b!(self.bld.build_store(d_ep, cloned));
            }
            return Ok(b!(self.bld.build_load(lty_st, dst_slot, "stc.cv")));
        }

        // Recursive struct: materialize __clone_<Name>(out, in).
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let void_ty = self.ctx.void_type();
        let fn_ty = void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        let cfn = self.module.add_function(&clone_fn_name, fn_ty, None);

        // Save current emission state.
        let saved_fn = self.cur_fn;
        let saved_bb = self.bld.get_insert_block();

        self.cur_fn = Some(cfn);
        let entry = self.ctx.append_basic_block(cfn, "entry");
        self.bld.position_at_end(entry);

        let out_ptr = cfn
            .get_nth_param(0)
            .expect("ICE: clone helper missing out param")
            .into_pointer_value();
        let in_ptr = cfn
            .get_nth_param(1)
            .expect("ICE: clone helper missing in param")
            .into_pointer_value();

        for (i, (_, ft)) in fields.iter().enumerate() {
            let lf = self.llvm_ty(ft);
            let s_ep = b!(self.bld.build_struct_gep(lty_st, in_ptr, i as u32, "stc.sep"));
            let d_ep = b!(self.bld.build_struct_gep(lty_st, out_ptr, i as u32, "stc.dep"));
            let s_v = b!(self.bld.build_load(lf, s_ep, "stc.sv"));
            let cloned = self.clone_value(s_v, ft)?;
            b!(self.bld.build_store(d_ep, cloned));
        }
        b!(self.bld.build_return(None));

        // Restore caller state.
        self.cur_fn = saved_fn;
        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }

        // Call the freshly-built helper at the original call site.
        let src_slot = self.entry_alloca(lty_st.into(), "stc.src");
        let dst_slot = self.entry_alloca(lty_st.into(), "stc.dst");
        b!(self.bld.build_store(src_slot, val));
        b!(self.bld.build_call(cfn, &[dst_slot.into(), src_slot.into()], ""));
        Ok(b!(self.bld.build_load(lty_st, dst_slot, "stc.cv")))
    }

    /// Does `ty` reference (directly or via container) the named struct?
    /// Used to detect self-referential structs that need an out-of-line clone
    /// helper to bound codegen recursion. Mirrors
    /// `drop::aggregates::type_references_struct`.
    fn type_references_struct_for_clone(ty: &Type, name: &str) -> bool {
        match ty {
            Type::Struct(n, _) => n == name,
            Type::Vec(inner)
            | Type::Rc(inner)
            | Type::RcCell(inner)
            | Type::Arc(inner)
            | Type::Mutex(inner)
            | Type::Weak(inner)
            | Type::Array(inner, _)
            | Type::Ptr(inner) => Self::type_references_struct_for_clone(inner, name),
            Type::Map(k, v) => {
                Self::type_references_struct_for_clone(k, name)
                    || Self::type_references_struct_for_clone(v, name)
            }
            Type::Tuple(tys) => tys
                .iter()
                .any(|t| Self::type_references_struct_for_clone(t, name)),
            Type::Alias(_, inner) | Type::Newtype(_, inner) => {
                Self::type_references_struct_for_clone(inner, name)
            }
            _ => false,
        }
    }

    /// Bump the refcount of an Rc/Weak-style cell (first 8 bytes = i64 rc).
    /// Returns the same pointer (the alias is now legitimate, since rc was incremented).
    /// Null pointers are passed through unchanged.
    fn bump_ptr_rc(&mut self, val: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = val.into_pointer_value();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let null = ptr_ty.const_null();
        let is_null = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            ptr,
            null,
            "rc.null"
        ));
        let fv = self.current_fn();
        let bump_bb = self.ctx.append_basic_block(fv, "rc.bump");
        let done_bb = self.ctx.append_basic_block(fv, "rc.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, bump_bb));
        self.bld.position_at_end(bump_bb);
        let rc = b!(self.bld.build_load(i64t, ptr, "rc.cnt")).into_int_value();
        let inc = b!(self
            .bld
            .build_int_nuw_add(rc, i64t.const_int(1, false), "rc.inc"));
        b!(self.bld.build_store(ptr, inc));
        b!(self.bld.build_unconditional_branch(done_bb));
        self.bld.position_at_end(done_bb);
        Ok(val)
    }

    /// R3.4.c — atomic-bump variant for `Arc<T>` clone. Layout-identical
    /// to `bump_ptr_rc` but uses `atomicrmw add` with AcquireRelease
    /// ordering so that other threads observing the pointer see a
    /// consistent refcount.
    fn bump_ptr_arc(&mut self, val: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = val.into_pointer_value();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let null = ptr_ty.const_null();
        let is_null = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            ptr,
            null,
            "arc.null"
        ));
        let fv = self.current_fn();
        let bump_bb = self.ctx.append_basic_block(fv, "arc.bump");
        let done_bb = self.ctx.append_basic_block(fv, "arc.done");
        b!(self.bld.build_conditional_branch(is_null, done_bb, bump_bb));
        self.bld.position_at_end(bump_bb);
        b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Add,
            ptr,
            i64t.const_int(1, false),
            inkwell::AtomicOrdering::AcquireRelease,
        ));
        b!(self.bld.build_unconditional_branch(done_bb));
        self.bld.position_at_end(done_bb);
        Ok(val)
    }

    /// Aliased pointer placeholder for use in PHI nodes when a path is
    /// unreachable (parameter type matches).
    #[allow(dead_code)]
    fn null_ptr(&self) -> PointerValue<'ctx> {
        self.ctx.ptr_type(AddressSpace::default()).const_null()
    }
}
