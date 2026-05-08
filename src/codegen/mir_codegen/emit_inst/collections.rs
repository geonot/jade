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
                    self.drop_value(v, ty)?;
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::RcInc(val) => {
                    let v = self.val(*val);
                    if let Type::Rc(inner) = &inst.ty {
                        self.rc_retain(v, inner)?;
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::RcDec(val) => {
                    let v = self.val(*val);
                    if let Type::Rc(inner) = &inst.ty {
                        self.rc_release(v, inner)?;
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::RcNew(val, inner_ty) => {
                    let v = self.val(*val);
                    self.rc_alloc(inner_ty, v)
                }
                mir::InstKind::RcClone(val) => {
                    let v = self.val(*val);
                    if let Type::Rc(inner) = &inst.ty {
                        self.rc_retain(v, inner)?;
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

                // ── Copy ──
                mir::InstKind::Copy(val) => Ok(self.val(*val)),

                // ── Slice ──
                mir::InstKind::Slice(base, lo, hi) => self.emit_slice(*base, *lo, *hi, &inst.ty),

                // ── Collections ──
                mir::InstKind::VecNew(elems) => self.emit_vec_new(elems, &inst.ty),
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
                        Err("mir_codegen: VecLen on non-vec/array value".into())
                    }
                }
                mir::InstKind::MapInit => self.compile_map_new(),
                mir::InstKind::SetInit => {
                    if let Some(fv) = self.module.get_function("jinn_set_new") {
                        let csv = b!(self.bld.build_call(fv, &[], "set"));
                        Ok(self.call_result(csv))
                    } else {
                        Err(
                        "mir_codegen: SetInit used but jinn_set_new runtime function not declared"
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
                        "mir_codegen: PQInit used but jinn_pq_new runtime function not declared"
                            .into(),
                    )
                    }
                }
                mir::InstKind::DequeInit => {
                    if let Some(fv) = self.module.get_function("jinn_deque_new") {
                        let csv = b!(self.bld.build_call(fv, &[], "deque"));
                        Ok(self.call_result(csv))
                    } else {
                        Err("mir_codegen: DequeInit used but jinn_deque_new runtime function not declared".into())
                    }
                }

                // ── Closures ──,
                _ => return Ok(None),
            })?,
        ))
    }
}
