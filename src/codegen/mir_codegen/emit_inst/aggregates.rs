//! MIR aggregate, field, index, cast, reference, and allocation emission.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_aggregate_memory_inst(
        &mut self,
        inst: &mir::Instruction,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        Ok(Some(
            (match &inst.kind {
                mir::InstKind::GlobalStore(name, val_id) => {
                    let v = self.val(*val_id);
                    if let Some((gv, ty)) = self.globals.get(name).cloned() {
                        let si = b!(self.bld.build_store(gv.as_pointer_value(), v));
                        self.set_tbaa(si, Compiler::tbaa_type_name(&ty));
                        Ok(self.ctx.i8_type().const_int(0, false).into())
                    } else {
                        Err(format!(
                            "GlobalStore to undefined global `{name}`"
                        ))
                    }
                }

                // ── Struct/Aggregate ──
                mir::InstKind::StructInit(name, fields) => {
                    let st = self
                        .module
                        .get_struct_type(&name.as_str())
                        .ok_or_else(|| format!("unknown struct `{name}`"))?;
                    let field_defs: Vec<(String, Type)> =
                        self.structs.get(name).cloned().unwrap_or_default();
                    let defaults = self.struct_defaults.get(name).cloned();
                    let mut agg: BasicValueEnum<'ctx> = st.const_zero().into();
                    // Track which field indices were explicitly provided.
                    let mut provided = std::collections::HashSet::new();
                    for (i, (fname, vid)) in fields.iter().enumerate() {
                        let v = self.val(*vid);
                        let idx = if fname.is_empty() {
                            provided.insert(i as u32);
                            i as u32
                        } else {
                            let pos = field_defs
                                .iter()
                                .position(|(n, _)| fname.with_str(|s| n == s))
                                .ok_or_else(|| {
                                    format!("struct `{name}` has no field `{fname}`")
                                })? as u32;
                            provided.insert(pos);
                            pos
                        };
                        let label = if fname.is_empty() {
                            field_defs.get(i).map(|s| s.0.as_str()).unwrap_or("field")
                        } else {
                            &fname.as_str()
                        };
                        agg =
                            b!(self
                                .bld
                                .build_insert_value(agg.into_struct_value(), v, idx, label))
                            .into_struct_value()
                            .into();
                    }
                    // Fill in defaults for missing fields.
                    for (i, (fname, fty)) in field_defs.iter().enumerate() {
                        let idx = i as u32;
                        if provided.contains(&idx) {
                            continue;
                        }
                        let val =
                            if let Some(def_expr) = defaults.as_ref().and_then(|d| d.get(fname)) {
                                self.compile_const_expr(def_expr)?
                            } else {
                                self.default_val(fty)
                            };
                        agg = b!(self.bld.build_insert_value(
                            agg.into_struct_value(),
                            val,
                            idx,
                            fname
                        ))
                        .into_struct_value()
                        .into();
                    }
                    // Materialize the aggregate into a stack alloca so that
                    // subsequent FieldSets and Call arguments share a single
                    // mutable backing store. value_map[dest] holds the pointer;
                    // val() will reload the current struct value lazily for
                    // any consumer that needs the value (e.g. Return).
                    if let Some(dest) = inst.dest {
                        let alloca = self.entry_alloca(st.into(), &name.as_str());
                        let _ = self.bld.build_store(alloca, agg);
                        self.self_allocs.insert(dest, alloca);
                        self.self_alloc_types.insert(dest, st.into());
                        return Ok(Some(alloca.into()));
                    }
                    Ok(agg)
                }
                mir::InstKind::VariantInit(enum_name, variant, tag, payload) => {
                    let enum_ty = self.llvm_ty(&Type::Enum(*enum_name));
                    let st = enum_ty.into_struct_type();
                    let i32t = self.ctx.i32_type();
                    let mut agg: BasicValueEnum<'ctx> = st.const_zero().into();
                    // Field 0 = tag.
                    agg = b!(self.bld.build_insert_value(
                        agg.into_struct_value(),
                        i32t.const_int(*tag as u64, false),
                        0,
                        "tag"
                    ))
                    .into_struct_value()
                    .into();
                    // Payload into field 1 (stored as a byte array, need to bitcast via alloca).
                    if !payload.is_empty() {
                        let alloca = self.entry_alloca(enum_ty, "variant.tmp");
                        b!(self.bld.build_store(alloca, agg));
                        let payload_gep = b!(self.bld.build_struct_gep(st, alloca, 1, "payload"));
                        // Look up variant field types for recursive-field detection.
                        let variant_field_types: Vec<Type> = self
                            .enums
                            .get(enum_name)
                            .and_then(|vs| vs.iter().find(|(vn, _)| variant.with_str(|s| vn == s)))
                            .map(|(_, ftys)| ftys.clone())
                            .unwrap_or_default();
                        // Store payload fields at proper byte offsets based on actual type sizes.
                        let mut byte_offset: u64 = 0;
                        for (i, vid) in payload.iter().enumerate() {
                            let v = self.val(*vid);
                            let is_rec = variant_field_types
                                .get(i)
                                .map(|fty| Compiler::is_recursive_field(fty, &enum_name.as_str()))
                                .unwrap_or(false);
                            let field_ptr = if byte_offset == 0 {
                                payload_gep
                            } else {
                                let offset_val = self.ctx.i64_type().const_int(byte_offset, false);
                                unsafe {
                                    b!(self.bld.build_gep(
                                        self.ctx.i8_type(),
                                        payload_gep,
                                        &[offset_val],
                                        "payload.elem"
                                    ))
                                }
                            };
                            if is_rec {
                                // Box the recursive field: malloc, store value, store pointer.
                                let actual_ty =
                                    self.llvm_ty(variant_field_types.get(i).unwrap_or(&Type::I64));
                                let size = self.type_store_size(actual_ty);
                                let malloc_fn = self.ensure_malloc();
                                let heap = b!(self.bld.build_call(
                                    malloc_fn,
                                    &[self.ctx.i64_type().const_int(size, false).into()],
                                    "box.alloc"
                                ))
                                .try_as_basic_value()
                                .basic()
                                .expect("ICE: call returned void")
                                .into_pointer_value();
                                b!(self.bld.build_store(heap, v));
                                b!(self.bld.build_store(field_ptr, heap));
                                byte_offset += 8;
                            } else {
                                b!(self.bld.build_store(field_ptr, v));
                                let type_size = v
                                    .get_type()
                                    .size_of()
                                    .map(|s| s.get_zero_extended_constant().unwrap_or(8))
                                    .unwrap_or(8);
                                byte_offset += (type_size + 7) & !7;
                            }
                        }
                        agg = b!(self.bld.build_load(enum_ty, alloca, "variant.loaded"));
                    }
                    Ok(agg)
                }
                mir::InstKind::ArrayInit(elems) => {
                    if elems.is_empty() {
                        let arr_ty = self.llvm_ty(&inst.ty);
                        return Ok(Some(arr_ty.const_zero()));
                    }
                    let elem_vals: Vec<BasicValueEnum<'ctx>> =
                        elems.iter().map(|v| self.val(*v)).collect();
                    let elem_ty = elem_vals[0].get_type();
                    let arr_ty = elem_ty.array_type(elems.len() as u32);
                    let alloca = self.entry_alloca(arr_ty.into(), "arr");
                    for (i, v) in elem_vals.iter().enumerate() {
                        let idx = self.ctx.i64_type().const_int(i as u64, false);
                        let zero = self.ctx.i64_type().const_int(0, false);
                        let ptr = unsafe {
                            b!(self.bld.build_gep(arr_ty, alloca, &[zero, idx], "arr.elem"))
                        };
                        b!(self.bld.build_store(ptr, *v));
                    }
                    Ok(b!(self.bld.build_load(arr_ty, alloca, "arr.val")).into())
                }

                // ── Field access ──
                mir::InstKind::FieldGet(obj, field) => {
                    self.emit_field_get(*obj, &field.as_str(), &inst.ty)
                }
                mir::InstKind::FieldSet(obj, field, val) => {
                    // If the object has a self_allocs entry, use the alloca pointer directly
                    // to avoid SSA domination issues with insertvalue across branches.
                    let obj_val = if let Some(alloca_ptr) = self.self_allocs.get(obj).copied() {
                        alloca_ptr.into()
                    } else {
                        self.val(*obj)
                    };
                    let v = self.val(*val);
                    if obj_val.is_pointer_value() {
                        // obj is a pointer to a struct (alloca).
                        // inst.ty carries the struct type from lowering.
                        let struct_name = self.struct_name_from_type(&inst.ty).or_else(|| {
                            // Also try var_allocs for the struct name.
                            self.var_allocs
                                .values()
                                .find(|(ptr, _)| *ptr == obj_val.into_pointer_value())
                                .and_then(|(_, ty)| match ty {
                                    Type::Struct(name, _) => Some(name.as_str()),
                                    _ => None,
                                })
                        });
                        if let Some(name) = &struct_name {
                            if let Some(st) = self.module.get_struct_type(name) {
                                let field_idx = self.field_index(name, &field.as_str());
                                let gep = b!(self.bld.build_struct_gep(
                                    st,
                                    obj_val.into_pointer_value(),
                                    field_idx,
                                    &field.as_str()
                                ));
                                b!(self.bld.build_store(gep, v));
                            }
                        }
                        // Return the pointer so MIR SSA chaining of field assignments
                        // continues to target the same struct (e.g. self.a is X; self.b is Y).
                        return Ok(Some(obj_val));
                    } else if obj_val.is_struct_value() {
                        // SSA struct value — use insert_value for immutable update.
                        let sv = obj_val.into_struct_value();
                        let struct_ty_name = sv
                            .get_type()
                            .get_name()
                            .map(|n| n.to_str().unwrap_or("").to_string());
                        if let Some(name) = &struct_ty_name {
                            let field_idx = self.field_index(name, &field.as_str());
                            let updated =
                                b!(self
                                    .bld
                                    .build_insert_value(sv, v, field_idx, &field.as_str()));
                            return Ok(Some(updated.into_struct_value().into()));
                        }
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::FieldStore(var_name, field, val) => {
                    // Direct field store into a named variable's alloca.
                    let v = self.val(*val);
                    if let Some((alloca, ty)) = self.var_allocs.get(var_name).cloned() {
                        let struct_name = self.struct_name_from_type(&ty);
                        if let Some(name) = &struct_name {
                            if let Some(st) = self.module.get_struct_type(name) {
                                let field_idx = self.field_index(name, &field.as_str());
                                let gep = b!(self.bld.build_struct_gep(
                                    st,
                                    alloca,
                                    field_idx,
                                    &field.as_str()
                                ));
                                b!(self.bld.build_store(gep, v));
                            }
                        }
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::FieldTombstone(var_name, field) => {
                    // P4 §5.2 partial-move tombstone: zero the field slot in
                    // the parent's alloca so a later drop sees null/zero and
                    // skips the (already-moved) heap allocation. All Jinn
                    // heap drops are null/zero-safe.
                    if let Some((alloca, ty)) = self.var_allocs.get(var_name).cloned() {
                        let struct_name = self.struct_name_from_type(&ty);
                        if let Some(name) = &struct_name {
                            if let Some(st) = self.module.get_struct_type(name) {
                                let field_idx = self.field_index(name, &field.as_str());
                                let field_ty = st.get_field_type_at_index(field_idx)
                                    .ok_or_else(|| format!(
                                        "ICE: field {} not found in struct {}",
                                        field.as_str(), name
                                    ))?;
                                let gep = b!(self.bld.build_struct_gep(
                                    st,
                                    alloca,
                                    field_idx,
                                    &field.as_str()
                                ));
                                let zero = field_ty.const_zero();
                                b!(self.bld.build_store(gep, zero));                            }
                        }
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }

                // ── Indexing ──
                mir::InstKind::Index(base, idx) | mir::InstKind::IndexUnchecked(base, idx) => {
                    let base_val = self.val(*base);
                    let idx_val = self.val(*idx);
                    let base_ty = self.value_types.get(base);
                    let unchecked = matches!(&inst.kind, mir::InstKind::IndexUnchecked(_, _));

                    // String indexing: get char at index (returns byte as i64)
                    if matches!(base_ty, Some(Type::String)) {
                        return Ok(Some((self.string_char_at(base_val, idx_val))?));
                    }

                    // For arrays: GEP into the array.
                    if base_val.get_type().is_array_type() {
                        let arr_ty = base_val.get_type().into_array_type();
                        let arr_len = arr_ty.len() as u64;
                        let i64t = self.ctx.i64_type();
                        let final_idx = if unchecked {
                            idx_val.into_int_value()
                        } else {
                            // Wrap negative indices: if idx < 0, idx = len + idx
                            let idx_int = idx_val.into_int_value();
                            let is_neg = b!(self.bld.build_int_compare(
                                inkwell::IntPredicate::SLT,
                                idx_int,
                                i64t.const_int(0, false),
                                "neg"
                            ));
                            let wrapped = b!(self.bld.build_int_nsw_add(
                                idx_int,
                                i64t.const_int(arr_len, false),
                                "wrap"
                            ));
                            b!(self.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                                .into_int_value()
                        };
                        let alloca = self.entry_alloca(arr_ty.into(), "idx.tmp");
                        b!(self.bld.build_store(alloca, base_val));
                        let zero = i64t.const_int(0, false);
                        let ptr = unsafe {
                            b!(self
                                .bld
                                .build_gep(arr_ty, alloca, &[zero, final_idx], "idx.ptr"))
                        };
                        let elem_ty = self.llvm_ty(&inst.ty);
                        Ok(b!(self.bld.build_load(elem_ty, ptr, "idx.val")))
                    } else if base_val.get_type().is_pointer_type() {
                        // Vec indexing: header is { ptr, len, cap }.
                        let header_ptr = base_val.into_pointer_value();
                        let header_ty = self.vec_header_type();
                        let elem_ty = self.llvm_ty(&inst.ty);
                        let i64t = self.ctx.i64_type();
                        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                        let ptr_gep = b!(self
                            .bld
                            .build_struct_gep(header_ty, header_ptr, 0, "vi.ptrp"));
                        let data_ptr = b!(self.bld.build_load(ptr_ty, ptr_gep, "vi.data"))
                            .into_pointer_value();
                        let final_idx = if unchecked {
                            idx_val.into_int_value()
                        } else {
                            let len_gep = b!(self
                                .bld
                                .build_struct_gep(header_ty, header_ptr, 1, "vi.lenp"));
                            let len =
                                b!(self.bld.build_load(i64t, len_gep, "vi.len")).into_int_value();
                            // Wrap negative indices: if idx < 0, idx = len + idx
                            let idx_int = idx_val.into_int_value();
                            let is_neg = b!(self.bld.build_int_compare(
                                inkwell::IntPredicate::SLT,
                                idx_int,
                                i64t.const_int(0, false),
                                "neg"
                            ));
                            let wrapped = b!(self.bld.build_int_nsw_add(idx_int, len, "wrap"));
                            let final_idx =
                                b!(self.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                                    .into_int_value();
                            self.emit_vec_bounds_check(final_idx, len)?;
                            final_idx
                        };
                        let elem_gep = unsafe {
                            b!(self
                                .bld
                                .build_gep(elem_ty, data_ptr, &[final_idx], "vi.egep"))
                        };
                        Ok(b!(self.bld.build_load(elem_ty, elem_gep, "vi.elem")))
                    } else if base_val.is_struct_value() {
                        // Tuple indexing: extract element from struct value.
                        if let Some(idx_const) =
                            idx_val.into_int_value().get_zero_extended_constant()
                        {
                            let elem = b!(self.bld.build_extract_value(
                                base_val.into_struct_value(),
                                idx_const as u32,
                                "tup.elem"
                            ));
                            Ok(elem)
                        } else {
                            // Dynamic index: store to alloca and GEP.
                            let st = base_val.get_type();
                            let alloca = self.entry_alloca(st, "tup.idx");
                            b!(self.bld.build_store(alloca, base_val));
                            let elem_ty = self.llvm_ty(&inst.ty);
                            let zero = self.ctx.i64_type().const_int(0, false);
                            let ptr = unsafe {
                                b!(self.bld.build_gep(
                                    st,
                                    alloca,
                                    &[zero, idx_val.into_int_value()],
                                    "tup.ptr"
                                ))
                            };
                            Ok(b!(self.bld.build_load(elem_ty, ptr, "tup.val")))
                        }
                    } else {
                        Ok(self.ctx.i8_type().const_int(0, false).into())
                    }
                }
                mir::InstKind::IndexSet(base, idx, val) => {
                    let base_val = self.val(*base);
                    let idx_val = self.val(*idx);
                    let v = self.val(*val);
                    if base_val.get_type().is_array_type() {
                        let arr_ty = base_val.get_type().into_array_type();
                        let arr_len = arr_ty.len() as u64;
                        let alloca = self.entry_alloca(arr_ty.into(), "idxset.tmp");
                        b!(self.bld.build_store(alloca, base_val));
                        let i64t = self.ctx.i64_type();
                        let zero = i64t.const_int(0, false);
                        // Wrap negative indices
                        let idx_int = idx_val.into_int_value();
                        let is_neg = b!(self.bld.build_int_compare(
                            inkwell::IntPredicate::SLT,
                            idx_int,
                            zero,
                            "neg"
                        ));
                        let wrapped = b!(self.bld.build_int_nsw_add(
                            idx_int,
                            i64t.const_int(arr_len, false),
                            "wrap"
                        ));
                        let final_idx = b!(self.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                            .into_int_value();
                        let ptr = unsafe {
                            b!(self
                                .bld
                                .build_gep(arr_ty, alloca, &[zero, final_idx], "idxset.ptr"))
                        };
                        b!(self.bld.build_store(ptr, v));
                        // Load the modified array back so the mutation is visible.
                        let updated = b!(self.bld.build_load(arr_ty, alloca, "idxset.updated"));
                        return Ok(Some(updated));
                    } else if base_val.get_type().is_pointer_type() {
                        // Vec: header is { ptr, len, cap }.
                        let header_ptr = base_val.into_pointer_value();
                        let header_ty = self.vec_header_type();
                        let elem_ty = v.get_type();
                        let i64t = self.ctx.i64_type();
                        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                        let ptr_gep = b!(self
                            .bld
                            .build_struct_gep(header_ty, header_ptr, 0, "vis.ptrp"));
                        let data_ptr = b!(self.bld.build_load(ptr_ty, ptr_gep, "vis.data"))
                            .into_pointer_value();
                        let len_gep = b!(self
                            .bld
                            .build_struct_gep(header_ty, header_ptr, 1, "vis.lenp"));
                        let len =
                            b!(self.bld.build_load(i64t, len_gep, "vis.len")).into_int_value();
                        // Wrap negative indices: `vec[-1]` → `vec[len-1]`.
                        let raw_idx = idx_val.into_int_value();
                        let zero_i = i64t.const_int(0, false);
                        let is_neg = b!(self.bld.build_int_compare(
                            inkwell::IntPredicate::SLT,
                            raw_idx,
                            zero_i,
                            "vis.neg"
                        ));
                        let wrapped =
                            b!(self.bld.build_int_nsw_add(raw_idx, len, "vis.wrap"));
                        let final_idx =
                            b!(self.bld.build_select(is_neg, wrapped, raw_idx, "vis.idx"))
                                .into_int_value();
                        self.emit_vec_bounds_check(final_idx, len)?;
                        let elem_gep = unsafe {
                            b!(self.bld.build_gep(
                                elem_ty,
                                data_ptr,
                                &[final_idx],
                                "vis.egep"
                            ))
                        };
                        b!(self.bld.build_store(elem_gep, v));
                        // Forward the vec pointer as the SSA result so subsequent
                        // uses (including Drop pairing in perceus) see the live value.
                        return Ok(Some(base_val));
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::IndexStore(var_name, idx, val) => {
                    // Direct index store into a named variable's alloca.
                    let idx_val = self.val(*idx);
                    let v = self.val(*val);
                    if let Some((alloca, ty)) = self.var_allocs.get(var_name).cloned() {
                        let llvm_ty = self.llvm_ty(&ty);
                        if llvm_ty.is_array_type() {
                            let arr_ty = llvm_ty.into_array_type();
                            let arr_len = arr_ty.len() as u64;
                            let i64t = self.ctx.i64_type();
                            let zero = i64t.const_int(0, false);
                            // Wrap negative indices
                            let idx_int = idx_val.into_int_value();
                            let is_neg = b!(self.bld.build_int_compare(
                                inkwell::IntPredicate::SLT,
                                idx_int,
                                zero,
                                "neg"
                            ));
                            let wrapped = b!(self.bld.build_int_nsw_add(
                                idx_int,
                                i64t.const_int(arr_len, false),
                                "wrap"
                            ));
                            let final_idx =
                                b!(self.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                                    .into_int_value();
                            let ptr = unsafe {
                                b!(self.bld.build_gep(
                                    arr_ty,
                                    alloca,
                                    &[zero, final_idx],
                                    "idxstore.ptr"
                                ))
                            };
                            b!(self.bld.build_store(ptr, v));
                        } else {
                            // Vec or other pointer-based type: load the header and index into data.
                            let header_ty = self.vec_header_type();
                            let i64t = self.ctx.i64_type();
                            let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                            let header_ptr = b!(self.bld.build_load(ptr_ty, alloca, "vis.hdr"))
                                .into_pointer_value();
                            let ptr_gep = b!(self
                                .bld
                                .build_struct_gep(header_ty, header_ptr, 0, "vis.ptrp"));
                            let data_ptr = b!(self.bld.build_load(ptr_ty, ptr_gep, "vis.data"))
                                .into_pointer_value();
                            let len_gep = b!(self
                                .bld
                                .build_struct_gep(header_ty, header_ptr, 1, "vis.lenp"));
                            let len =
                                b!(self.bld.build_load(i64t, len_gep, "vis.len")).into_int_value();
                            let elem_ty = v.get_type();
                            self.emit_vec_bounds_check(idx_val.into_int_value(), len)?;
                            let elem_gep = unsafe {
                                b!(self.bld.build_gep(
                                    elem_ty,
                                    data_ptr,
                                    &[idx_val.into_int_value()],
                                    "vis.egep"
                                ))
                            };
                            b!(self.bld.build_store(elem_gep, v));
                        }
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }

                // ── Cast / Ref / Deref ──
                mir::InstKind::Cast(val, target_ty) => {
                    let v = self.val(*val);
                    let target_llvm = self.llvm_ty(target_ty);
                    self.emit_cast(v, &inst.ty, target_ty, target_llvm)
                }
                mir::InstKind::StrictCast(val, target_ty) => {
                    let v = self.val(*val);
                    let target_llvm = self.llvm_ty(target_ty);
                    let casted = self.emit_cast(v, &inst.ty, target_ty, target_llvm)?;
                    // Validate: cast back and compare to original to detect overflow.
                    let source_llvm = v.get_type();
                    if v.is_int_value() && casted.is_int_value() {
                        let back = self.emit_cast(casted, target_ty, &inst.ty, source_llvm)?;
                        let eq = b!(self.bld.build_int_compare(
                            inkwell::IntPredicate::EQ,
                            v.into_int_value(),
                            back.into_int_value(),
                            "strict.eq"
                        ));
                        // If not equal, trap
                        let cur_fn = self.bld.get_insert_block().unwrap().get_parent().unwrap();
                        let ok_bb = self.ctx.append_basic_block(cur_fn, "strict.ok");
                        let trap_bb = self.ctx.append_basic_block(cur_fn, "strict.trap");
                        b!(self.bld.build_conditional_branch(eq, ok_bb, trap_bb));
                        self.bld.position_at_end(trap_bb);
                        if let Some(trap) = self.module.get_function("llvm.trap") {
                            b!(self.bld.build_call(trap, &[], ""));
                        }
                        b!(self.bld.build_unreachable());
                        self.bld.position_at_end(ok_bb);
                    }
                    Ok(casted)
                }
                mir::InstKind::Ref(val) => {
                    let v = self.val(*val);
                    let alloca = self.entry_alloca(v.get_type(), "ref");
                    b!(self.bld.build_store(alloca, v));
                    Ok(alloca.into())
                }
                mir::InstKind::Deref(val) => {
                    let v = self.val(*val);
                    if !v.is_pointer_value() {
                        return Err(format!("Deref on non-pointer value {:?}", val));
                    }
                    // R3.4.c: RC-family deref skips refcount header, reads
                    // payload via rc_deref. Rc, RcCell and Arc share the
                    // {strong, weak, payload} layout; only Arc<Mutex<T>>
                    // differs (extra mutex slot) and that case must go
                    // through mutex_lock/unlock, not raw Deref.
                    let val_ty = self.value_types.get(val).cloned();
                    if let Some(
                        Type::Rc(ref inner) | Type::RcCell(ref inner) | Type::Arc(ref inner),
                    ) = val_ty
                    {
                        return Ok(Some((self.rc_deref(v, inner))?));
                    }
                    let inner_ty = self.llvm_ty(&inst.ty);
                    Ok(b!(self.bld.build_load(
                        inner_ty,
                        v.into_pointer_value(),
                        "deref"
                    )))
                }

                // ── Memory / RC ──
                mir::InstKind::Alloc(val) => {
                    let v = self.val(*val);
                    let malloc = self.ensure_malloc();
                    let size = v
                        .get_type()
                        .size_of()
                        .unwrap_or(self.ctx.i64_type().const_int(8, false));
                    let ptr = b!(self.bld.build_call(malloc, &[size.into()], "alloc"))
                        .try_as_basic_value()
                        .basic()
                        .expect("ICE: call returned void");
                    b!(self.bld.build_store(ptr.into_pointer_value(), v));
                    Ok(ptr)
                }
                _ => return Ok(None),
            })?,
        ))
    }
}
