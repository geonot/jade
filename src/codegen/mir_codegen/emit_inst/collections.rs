//! MIR ownership, drop, copy, slice, and collection instruction emission.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_ownership_collection_inst(
        &mut self,
        inst: &mir::Instruction,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        Ok(Some(
            (match &inst.kind {
                mir::InstKind::Drop(val, ty) => {
                    let v = self.val(*val);
                    // Perceus reuse: stash the heap pointer for the matching
                    // alloc site to pick up via `current_reuse_slots`. Rc and
                    // RcCell share `rc_alloc` layout so reuse is safe across
                    // them; Arc<T> (non-Mutex) also shares the layout but
                    // Arc<Mutex<T>> does NOT (extra mutex slot), so Arc reuse
                    // is gated separately in R3.4.d when promotion runs.
                    if matches!(ty, Type::Rc(_) | Type::RcCell(_)) && v.is_pointer_value() {
                        if self.try_save_reuse_slot(*val, v.into_pointer_value()) {
                            return Ok(Some(self.ctx.i8_type().const_int(0, false).into()));
                        }
                    }
                    // Perceus Vec reuse: deep-drop elements, then stash the
                    // header (with its data buffer attached) for the matching
                    // empty `VecNew` to pick up.
                    if let Type::Vec(elem) = ty {
                        if v.is_pointer_value()
                            && self
                                .current_perceus_meta
                                .reuse_save
                                .get(val)
                                .map(|s| self.current_perceus_meta.vec_slots.contains(s))
                                .unwrap_or(false)
                        {
                            self.drop_vec_elements_only(v, elem)?;
                            self.try_save_vec_slot(*val, v.into_pointer_value());
                            return Ok(Some(self.ctx.i8_type().const_int(0, false).into()));
                        }
                    }
                    self.drop_value(v, ty)?;
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::DropMany(items) => {
                    // Fused drop run: emit the per-value drops back-to-back so
                    // LLVM sees an unbroken sequence of free() calls and can
                    // batch them. We do NOT save reuse slots inside a fused
                    // run (the perceus pass excludes save-slot drops from
                    // fusion runs by construction).
                    for (val, ty) in items {
                        let v = self.val(*val);
                        self.drop_value(v, ty)?;
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::RcInc(val) => {
                    let v = self.val(*val);
                    match &inst.ty {
                        // R3.4.c: Rc / RcCell share non-atomic refcount.
                        Type::Rc(inner) | Type::RcCell(inner) => {
                            self.rc_retain(v, inner)?;
                        }
                        // R3.4.c: Arc forces atomic refcount bump.
                        Type::Arc(inner) => {
                            self.arc_retain(v, inner)?;
                        }
                        _ => {}
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::RcDec(val) => {
                    let v = self.val(*val);
                    match &inst.ty {
                        Type::Rc(inner) | Type::RcCell(inner) => {
                            self.rc_release(v, inner)?;
                        }
                        Type::Arc(inner) => {
                            self.arc_release(v, inner)?;
                        }
                        _ => {}
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::RcNew(val, inner_ty) => {
                    let v = self.val(*val);
                    self.current_alloc_dest = inst.dest;
                    // R3.4.c: allocator dispatch based on inst.ty wrapper.
                    let r = match &inst.ty {
                        Type::Arc(_) => self.arc_alloc(inner_ty, v),
                        // Rc / RcCell share the rc_alloc layout.
                        _ => self.rc_alloc(inner_ty, v),
                    };
                    self.current_alloc_dest = None;
                    r
                }
                mir::InstKind::RcClone(val) => {
                    let v = self.val(*val);
                    match &inst.ty {
                        Type::Rc(inner) | Type::RcCell(inner) => {
                            self.rc_retain(v, inner)?;
                        }
                        Type::Arc(inner) => {
                            self.arc_retain(v, inner)?;
                        }
                        _ => {}
                    }
                    Ok(v)
                }
                mir::InstKind::WeakUpgrade(val) => {
                    let v = self.val(*val);
                    if let Type::Weak(inner) | Type::Rc(inner) = &inst.ty {
                        self.weak_upgrade(v, inner)
                    } else {
                        Ok(v)
                    }
                }
                mir::InstKind::WeakDowngrade(val) => {
                    let v = self.val(*val);
                    if let Type::Weak(inner) = &inst.ty {
                        self.weak_downgrade(v, inner)
                    } else {
                        Ok(v)
                    }
                }

                // ── Copy ──
                mir::InstKind::Copy(val) => Ok(self.val(*val)),

                // ── Clone (deep heap clone for auto-copy / explicit copy modifier) ──
                mir::InstKind::Clone(val, ty) => {
                    let v = self.val(*val);
                    if !Self::is_value_clonable(ty) || ty.is_trivially_droppable() {
                        Ok(v)
                    } else {
                        self.clone_value(v, ty).map(|c| c)
                    }
                }

                // ── Slice ──
                mir::InstKind::Slice(base, lo, hi) => self.emit_slice(*base, *lo, *hi, &inst.ty),

                // ── Collections ──
                mir::InstKind::VecNew(elems) => {
                    self.current_alloc_dest = inst.dest;
                    let r = self.emit_vec_new(elems, &inst.ty);
                    self.current_alloc_dest = None;
                    r
                }
                mir::InstKind::VecPush(vec, elem) => {
                    let vec_val = self.val(*vec).into_pointer_value();
                    let elem_val = self.val(*elem);
                    let lty = elem_val.get_type();
                    let elem_size = self.type_store_size(lty);
                    let growth_floor = self
                        .vec_growth_floor_by_value
                        .get(vec)
                        .copied()
                        .unwrap_or(self.empty_vec_growth_floor);
                    self.vec_push_raw_with_floor(vec_val, elem_val, lty, elem_size, growth_floor)?;
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::VecLen(vec) => {
                    let vec_val = self.val(*vec);
                    let i64t = self.ctx.i64_type();
                    let vec_ty = self.value_types.get(vec);
                    if matches!(vec_ty, Some(Type::String)) || vec_val.is_struct_value() {
                        // String: struct { ptr, len, cap }; len is field 1.
                        self.string_len(vec_val)
                    } else if vec_val.is_pointer_value() {
                        // Vec (heap-allocated): ptr to {ptr, i64, i64}; len is field 1.
                        let header_ty = self.vec_header_type();
                        let header_ptr = vec_val.into_pointer_value();
                        let len_gep = b!(self
                            .bld
                            .build_struct_gep(header_ty, header_ptr, 1, "vl.len"));
                        Ok(b!(self.bld.build_load(i64t, len_gep, "vl.v")))
                    } else if vec_val.is_array_value() {
                        // Fixed-size array: length is known at compile time.
                        let arr_len = vec_val.into_array_value().get_type().len();
                        Ok(i64t.const_int(arr_len as u64, false).into())
                    } else {
                        Err("VecLen on non-vec/array value".into())
                    }
                }
                mir::InstKind::MapInit => self.compile_map_new(),
                mir::InstKind::SetInit => {
                    if let Some(fv) = self.module.get_function("jinn_set_new") {
                        let csv = b!(self.bld.build_call(fv, &[], "set"));
                        Ok(self.call_result(csv))
                    } else {
                        Err(
                        "SetInit used but jinn_set_new runtime function not declared"
                            .into(),
                    )
                    }
                }
                mir::InstKind::PQInit => {
                    if let Some(fv) = self.module.get_function("jinn_pq_new") {
                        let csv = b!(self.bld.build_call(fv, &[], "pq"));
                        Ok(self.call_result(csv))
                    } else {
                        Err(
                        "PQInit used but jinn_pq_new runtime function not declared"
                            .into(),
                    )
                    }
                }
                mir::InstKind::DequeInit => {
                    if let Some(fv) = self.module.get_function("jinn_deque_new") {
                        let csv = b!(self.bld.build_call(fv, &[], "deque"));
                        Ok(self.call_result(csv))
                    } else {
                        Err("DequeInit used but jinn_deque_new runtime function not declared".into())
                    }
                }

                // ── Closures ──,
                _ => return Ok(None),
            })?,
        ))
    }
}
