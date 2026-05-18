//! MIR scalar, comparison, cast, field, and vector value helpers.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_string_const(
        &mut self,
        s: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_str_literal(s)
    }

    pub(in crate::codegen) fn emit_binop(
        &mut self,
        op: mir::BinOp,
        lhs: mir::ValueId,
        rhs: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Check for struct operator overload dispatch (e.g. Vec2 + Vec2 → Vec2.add(other)).
        if let Some(Type::Struct(name, _)) = self.value_types.get(&lhs) {
            let method = match op {
                mir::BinOp::Add => Some("add"),
                mir::BinOp::Sub => Some("sub"),
                mir::BinOp::Mul => Some("mul"),
                mir::BinOp::Div => Some("div"),
                _ => None,
            };
            if let Some(method_name) = method {
                let fn_name = format!("{name}_{method_name}");
                if let Some((fv, _, _)) = self.fns.get(&fn_name).cloned() {
                    let l = self.val(lhs);
                    let r = self.val(rhs);
                    let ptypes: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = fv
                        .get_type()
                        .get_param_types()
                        .into_iter()
                        .map(|t| t.into())
                        .collect();
                    let coerced = self.coerce_call_args(&[l, r], &[lhs, rhs], &ptypes);
                    let csv = b!(self
                        .bld
                        .build_call(fv, &coerced, &format!("{method_name}.call")));
                    return Ok(self.call_result(csv));
                }
            }
        }

        // String concatenation: String + String.
        if matches!(op, mir::BinOp::Add) && matches!(result_ty, Type::String) {
            let l = self.val(lhs);
            let r = self.val(rhs);
            return self.string_concat(l, r);
        }

        let l = self.val(lhs);
        let r = self.val(rhs);

        if result_ty.is_float() {
            let lf = l.into_float_value();
            let rf = r.into_float_value();
            let res = match op {
                mir::BinOp::Add => b!(self.bld.build_float_add(lf, rf, "fadd")),
                mir::BinOp::Sub => b!(self.bld.build_float_sub(lf, rf, "fsub")),
                mir::BinOp::Mul => b!(self.bld.build_float_mul(lf, rf, "fmul")),
                mir::BinOp::Div => b!(self.bld.build_float_div(lf, rf, "fdiv")),
                mir::BinOp::Mod => b!(self.bld.build_float_rem(lf, rf, "fmod")),
                mir::BinOp::Exp => {
                    let f64t = self.ctx.f64_type();
                    let pow = self.module.get_function("pow").unwrap_or_else(|| {
                        let ft = f64t.fn_type(&[f64t.into(), f64t.into()], false);
                        self.module.add_function("pow", ft, Some(Linkage::External))
                    });
                    let result = b!(self.bld.build_call(pow, &[lf.into(), rf.into()], "pow"));
                    return Ok(self.call_result(result));
                }
                _ => return Err(format!("unsupported float binop {op:?}")),
            };
            Ok(res.into())
        } else if l.is_pointer_value()
            && r.is_int_value()
            && matches!(op, mir::BinOp::Add | mir::BinOp::Sub)
        {
            // Pointer arithmetic: ptr + int or ptr - int
            let ptr = l.into_pointer_value();
            let idx = r.into_int_value();
            let i8_ty = self.ctx.i8_type();
            let offset = if matches!(op, mir::BinOp::Sub) {
                b!(self.bld.build_int_neg(idx, "neg"))
            } else {
                idx
            };
            let res = unsafe { b!(self.bld.build_gep(i8_ty, ptr, &[offset], "ptradd")) };
            Ok(res.into())
        } else {
            let mut li = l.into_int_value();
            let mut ri = r.into_int_value();
            // Auto-widen mismatched integer widths to the wider operand.
            // Required because MIR currently lets `i32 << i64` reach codegen
            // unaltered (e.g. `i * 2` where `i: i32` and `2: i64`); LLVM
            // rejects this with "operands not of the same type".
            let lw = li.get_type().get_bit_width();
            let rw = ri.get_type().get_bit_width();
            if lw != rw {
                if lw < rw {
                    let signed = result_ty.is_signed();
                    li = if signed {
                        b!(self.bld.build_int_s_extend(li, ri.get_type(), "sext"))
                    } else {
                        b!(self.bld.build_int_z_extend(li, ri.get_type(), "zext"))
                    };
                } else {
                    let signed = result_ty.is_signed();
                    ri = if signed {
                        b!(self.bld.build_int_s_extend(ri, li.get_type(), "sext"))
                    } else {
                        b!(self.bld.build_int_z_extend(ri, li.get_type(), "zext"))
                    };
                }
            }
            let res = match op {
                mir::BinOp::Add => b!(self.bld.build_int_add(li, ri, "add")),
                mir::BinOp::Sub => b!(self.bld.build_int_sub(li, ri, "sub")),
                mir::BinOp::Mul => b!(self.bld.build_int_mul(li, ri, "mul")),
                mir::BinOp::Div => {
                    if result_ty.is_signed() {
                        b!(self.bld.build_int_signed_div(li, ri, "sdiv"))
                    } else {
                        b!(self.bld.build_int_unsigned_div(li, ri, "udiv"))
                    }
                }
                mir::BinOp::Mod => {
                    if result_ty.is_signed() {
                        b!(self.bld.build_int_signed_rem(li, ri, "srem"))
                    } else {
                        b!(self.bld.build_int_unsigned_rem(li, ri, "urem"))
                    }
                }
                mir::BinOp::BitAnd => b!(self.bld.build_and(li, ri, "and")),
                mir::BinOp::BitOr => b!(self.bld.build_or(li, ri, "or")),
                mir::BinOp::BitXor => b!(self.bld.build_xor(li, ri, "xor")),
                mir::BinOp::Shl => b!(self.bld.build_left_shift(li, ri, "shl")),
                mir::BinOp::Shr => {
                    b!(self
                        .bld
                        .build_right_shift(li, ri, result_ty.is_signed(), "shr"))
                }
                mir::BinOp::Ushr => {
                    b!(self.bld.build_right_shift(li, ri, false, "ushr"))
                }
                mir::BinOp::And => b!(self.bld.build_and(li, ri, "land")),
                mir::BinOp::Or => b!(self.bld.build_or(li, ri, "lor")),
                mir::BinOp::Exp => {
                    // Exponentiation: use llvm.powi intrinsic or loop.
                    // For now, cast to float, call pow, cast back.
                    let f64t = self.ctx.f64_type();
                    let lf = b!(self.bld.build_signed_int_to_float(li, f64t, "exp.l"));
                    let rf = b!(self.bld.build_signed_int_to_float(ri, f64t, "exp.r"));
                    let pow = self.module.get_function("pow").unwrap_or_else(|| {
                        let ft = f64t.fn_type(&[f64t.into(), f64t.into()], false);
                        self.module.add_function("pow", ft, Some(Linkage::External))
                    });
                    let result = b!(self.bld.build_call(pow, &[lf.into(), rf.into()], "pow"));
                    let fv = self.call_result(result).into_float_value();
                    let iv = b!(self
                        .bld
                        .build_float_to_signed_int(fv, li.get_type(), "exp.i"));
                    return Ok(iv.into());
                }
            };
            Ok(res.into())
        }
    }

    pub(in crate::codegen) fn emit_unary(
        &mut self,
        op: mir::UnaryOp,
        val: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let v = self.val(val);
        match op {
            mir::UnaryOp::Neg => {
                if result_ty.is_float() {
                    Ok(b!(self.bld.build_float_neg(v.into_float_value(), "fneg")).into())
                } else {
                    let zero = v.into_int_value().get_type().const_int(0, false);
                    Ok(b!(self.bld.build_int_sub(zero, v.into_int_value(), "neg")).into())
                }
            }
            mir::UnaryOp::Not => Ok(b!(self.bld.build_not(v.into_int_value(), "not")).into()),
            mir::UnaryOp::BitNot => Ok(b!(self.bld.build_not(v.into_int_value(), "bitnot")).into()),
        }
    }

    pub(in crate::codegen) fn emit_cmp(
        &mut self,
        op: mir::CmpOp,
        lhs: mir::ValueId,
        rhs: mir::ValueId,
        operand_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Check for struct operator overload dispatch for comparisons.
        if let Some(Type::Struct(name, _)) = self.value_types.get(&lhs) {
            let method = match op {
                mir::CmpOp::Lt => Some("less"),
                mir::CmpOp::Gt => Some("greater"),
                mir::CmpOp::Le => Some("less_eq"),
                mir::CmpOp::Ge => Some("greater_eq"),
                mir::CmpOp::Eq => Some("equal"),
                mir::CmpOp::Ne => Some("equal"), // call equal then negate
            };
            if let Some(method_name) = method {
                let fn_name = format!("{name}_{method_name}");
                if let Some((fv, _, _)) = self.fns.get(&fn_name).cloned() {
                    let l = self.val(lhs);
                    let r = self.val(rhs);
                    let ptypes: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = fv
                        .get_type()
                        .get_param_types()
                        .into_iter()
                        .map(|t| t.into())
                        .collect();
                    let coerced = self.coerce_call_args(&[l, r], &[lhs, rhs], &ptypes);
                    let csv = b!(self.bld.build_call(fv, &coerced, "cmp.call"));
                    let result = self.call_result(csv);
                    return if matches!(op, mir::CmpOp::Ne) {
                        Ok(b!(self.bld.build_not(result.into_int_value(), "neq")).into())
                    } else {
                        Ok(result)
                    };
                }
            }
        }

        let l = self.val(lhs);
        let r = self.val(rhs);

        // String comparison: delegate to Compiler::string_eq which uses memcmp.
        let is_string_type = matches!(operand_ty, Type::String)
            || matches!(operand_ty, Type::Struct(n, _) if n == "String");
        if l.is_struct_value() && is_string_type {
            let negate = matches!(op, mir::CmpOp::Ne);
            return self.string_eq(l, r, negate);
        }

        // Determine comparison mode from the actual LLVM value type, not
        // from inst.ty (which is Bool — the result type, not operand type).
        if l.get_type().is_float_type() {
            let pred = match op {
                mir::CmpOp::Eq => inkwell::FloatPredicate::OEQ,
                mir::CmpOp::Ne => inkwell::FloatPredicate::ONE,
                mir::CmpOp::Lt => inkwell::FloatPredicate::OLT,
                mir::CmpOp::Gt => inkwell::FloatPredicate::OGT,
                mir::CmpOp::Le => inkwell::FloatPredicate::OLE,
                mir::CmpOp::Ge => inkwell::FloatPredicate::OGE,
            };
            Ok(b!(self.bld.build_float_compare(
                pred,
                l.into_float_value(),
                r.into_float_value(),
                "fcmp"
            ))
            .into())
        } else {
            // Use unsigned predicates for unsigned operand types, signed otherwise.
            let is_unsigned = matches!(operand_ty, Type::U8 | Type::U16 | Type::U32 | Type::U64);
            let pred = match (op, is_unsigned) {
                (mir::CmpOp::Eq, _) => inkwell::IntPredicate::EQ,
                (mir::CmpOp::Ne, _) => inkwell::IntPredicate::NE,
                (mir::CmpOp::Lt, false) => inkwell::IntPredicate::SLT,
                (mir::CmpOp::Lt, true) => inkwell::IntPredicate::ULT,
                (mir::CmpOp::Gt, false) => inkwell::IntPredicate::SGT,
                (mir::CmpOp::Gt, true) => inkwell::IntPredicate::UGT,
                (mir::CmpOp::Le, false) => inkwell::IntPredicate::SLE,
                (mir::CmpOp::Le, true) => inkwell::IntPredicate::ULE,
                (mir::CmpOp::Ge, false) => inkwell::IntPredicate::SGE,
                (mir::CmpOp::Ge, true) => inkwell::IntPredicate::UGE,
            };
            Ok(b!(self
                .bld
                .build_int_compare(pred, l.into_int_value(), r.into_int_value(), "icmp"))
            .into())
        }
    }

    pub(in crate::codegen) fn emit_cast(
        &mut self,
        val: BasicValueEnum<'ctx>,
        _src_ty: &Type,
        target_ty: &Type,
        target_llvm: BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if val.get_type() == target_llvm {
            return Ok(val);
        }
        // Int → Float.
        if val.is_int_value() && target_ty.is_float() {
            return if !_src_ty.is_signed() {
                Ok(b!(self.bld.build_unsigned_int_to_float(
                    val.into_int_value(),
                    target_llvm.into_float_type(),
                    "u2f"
                ))
                .into())
            } else {
                Ok(b!(self.bld.build_signed_int_to_float(
                    val.into_int_value(),
                    target_llvm.into_float_type(),
                    "i2f"
                ))
                .into())
            };
        }
        // Float → Int.
        if val.is_float_value() && target_ty.is_int() {
            return if !target_ty.is_signed() {
                Ok(b!(self.bld.build_float_to_unsigned_int(
                    val.into_float_value(),
                    target_llvm.into_int_type(),
                    "f2u"
                ))
                .into())
            } else {
                Ok(b!(self.bld.build_float_to_signed_int(
                    val.into_float_value(),
                    target_llvm.into_int_type(),
                    "f2i"
                ))
                .into())
            };
        }
        // Int → Int (widen/truncate).
        if val.is_int_value() && target_llvm.is_int_type() {
            let src_bits = val.into_int_value().get_type().get_bit_width();
            let dst_bits = target_llvm.into_int_type().get_bit_width();
            return if dst_bits > src_bits {
                if !_src_ty.is_signed() {
                    Ok(b!(self.bld.build_int_z_extend(
                        val.into_int_value(),
                        target_llvm.into_int_type(),
                        "zext"
                    ))
                    .into())
                } else {
                    Ok(b!(self.bld.build_int_s_extend(
                        val.into_int_value(),
                        target_llvm.into_int_type(),
                        "sext"
                    ))
                    .into())
                }
            } else if dst_bits < src_bits {
                Ok(b!(self.bld.build_int_truncate(
                    val.into_int_value(),
                    target_llvm.into_int_type(),
                    "trunc"
                ))
                .into())
            } else {
                Ok(val)
            };
        }
        // Float → Float.
        if val.is_float_value() && target_llvm.is_float_type() {
            return Ok(b!(self.bld.build_float_cast(
                val.into_float_value(),
                target_llvm.into_float_type(),
                "fcast"
            ))
            .into());
        }
        // Pointer cast.
        if val.is_pointer_value() && target_llvm.is_pointer_type() {
            return Ok(val);
        }
        // Fallback: bitcast via alloca.
        let alloca = self.entry_alloca(val.get_type(), "cast.tmp");
        b!(self.bld.build_store(alloca, val));
        Ok(b!(self.bld.build_load(target_llvm, alloca, "cast")))
    }

    pub(in crate::codegen) fn emit_field_get(
        &mut self,
        obj: mir::ValueId,
        field: &str,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // If the object has a self_allocs entry, use the alloca pointer directly
        // for GEP-based field access (avoids SSA domination issues).
        let obj_val = if let Some(alloca_ptr) = self.self_allocs.get(&obj).copied() {
            alloca_ptr.into()
        } else {
            self.val(obj)
        };

        // Heap nominals (Struct/Enum/Vec/etc.) are reference-typed by
        // construction in MIR — no surface Rc wrapper exists. Field access
        // on a heap nominal therefore proceeds directly via the standard
        // struct/enum/string/vec paths below.
        let obj_ty = self.value_types.get(&obj).cloned();

        // String field access — handle "length" via SSO-aware string_len
        if matches!(&obj_ty, Some(Type::String)) {
            match field {
                "length" => return self.string_len(obj_val),
                "data" => return self.string_data(obj_val),
                _ => {}
            }
        }

        if obj_val.is_struct_value() {
            // Inline struct: extract_value.
            let sv = obj_val.into_struct_value();
            // Look up field index via struct name in types metadata.
            let struct_ty_name = sv
                .get_type()
                .get_name()
                .map(|n| n.to_str().unwrap_or("").to_string());
            if let Some(name) = &struct_ty_name {
                // Check if this is an enum type — enum payloads need special extraction.
                if self.enums.contains_key(name) {
                    if field == "__tag" {
                        // Tag is at struct index 0 (i32). Extend to i64
                        // since the MIR lowering declares __tag as Type::I64.
                        let tag_i32 = b!(self.bld.build_extract_value(sv, 0, "tag"));
                        let i64t = self.ctx.i64_type();
                        let val = b!(self.bld.build_int_z_extend(
                            tag_i32.into_int_value(),
                            i64t,
                            "tag.ext"
                        ));
                        return Ok(val.into());
                    }
                    // Payload fields: _0, _1, ... — extract from the payload byte array.
                    // Offsets must match VariantInit which uses actual type sizes
                    // with 8-byte alignment.
                    if let Some(idx_str) = field.strip_prefix('_') {
                        if let Ok(idx) = idx_str.parse::<usize>() {
                            let st = sv.get_type();
                            let alloca = self.entry_alloca(st.into(), "enum.tmp");
                            b!(self.bld.build_store(alloca, sv));
                            let payload_gep =
                                b!(self.bld.build_struct_gep(st, alloca, 1, "payload"));
                            let res_llvm = self.llvm_ty(result_ty);
                            let byte_offset = self.compute_enum_payload_offset(name, idx);
                            let field_ptr = if byte_offset == 0 {
                                payload_gep
                            } else {
                                let offset_val = self.ctx.i64_type().const_int(byte_offset, false);
                                unsafe {
                                    b!(self.bld.build_gep(
                                        self.ctx.i8_type(),
                                        payload_gep,
                                        &[offset_val],
                                        "payload.field"
                                    ))
                                }
                            };
                            // Check if this field is a recursive reference (boxed as ptr).
                            let is_rec = Compiler::is_recursive_field(result_ty, name);
                            if is_rec {
                                let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                                let heap_ptr =
                                    b!(self.bld.build_load(ptr_ty, field_ptr, "box.ptr"))
                                        .into_pointer_value();
                                let val = b!(self.bld.build_load(res_llvm, heap_ptr, field));
                                return Ok(val);
                            }
                            let val = b!(self.bld.build_load(res_llvm, field_ptr, field));
                            return Ok(val);
                        }
                    }
                }
                let idx = self.field_index(name, field);
                let val = b!(self.bld.build_extract_value(sv, idx, field));
                return Ok(val);
            }
            // Unknown struct type — cannot determine correct field index.
            Err(format!(
                "FieldGet on unknown struct type for field `{field}`"
            ))
        } else if obj_val.is_pointer_value() {
            // Vec .length/.len: read len field from vec header.
            if matches!(field, "length") {
                if matches!(&obj_ty, Some(Type::Vec(_))) {
                    let header_ptr = obj_val.into_pointer_value();
                    let header_ty = self.vec_header_type();
                    let i64t = self.ctx.i64_type();
                    let len_gep = b!(self
                        .bld
                        .build_struct_gep(header_ty, header_ptr, 1, "vl.lenp"));
                    let len = b!(self.bld.build_load(i64t, len_gep, "vl.len"));
                    return Ok(len);
                }
            }
            // Pointer to struct: GEP + load.
            let ptr = obj_val.into_pointer_value();
            let res_llvm = self.llvm_ty(result_ty);
            // Try to find the struct type from var_allocs or compiler vars.
            let struct_name = self
                .var_allocs
                .values()
                .find(|(p, _)| *p == ptr)
                .and_then(|(_, ty)| match ty {
                    Type::Struct(name, _) => Some(name.clone()),
                    _ => None,
                })
                .or_else(|| {
                    // Search compiler's vars for a matching pointer.
                    self.vars.values().find_map(|(p, ty)| {
                        if *p == ptr {
                            match ty {
                                Type::Struct(name, _) => Some(name.clone()),
                                _ => None,
                            }
                        } else {
                            None
                        }
                    })
                })
                .or_else(|| {
                    // Search value_types for the MIR ValueId's type (covers function parameters).
                    // Prefer the locally-rebound `obj_ty` so R3.4.d.1 auto-deref
                    // (which replaced the original Rc<Struct> with the inner
                    // Struct) is honored here too.
                    obj_ty
                        .as_ref()
                        .or_else(|| self.value_types.get(&obj))
                        .and_then(|ty| match ty {
                            Type::Ptr(inner) => match inner.as_ref() {
                                Type::Struct(name, _) | Type::Enum(name) => Some(name.clone()),
                                _ => None,
                            },
                            Type::Struct(name, _) | Type::Enum(name) => Some(name.clone()),
                            _ => None,
                        })
                });
            if let Some(name) = &struct_name {
                if let Some(st) = self.module.get_struct_type(&name.as_str()) {
                    // Enum-typed pointer: handle __tag and payload fields (_0, _1, ...)
                    // with the same layout the struct-value branch uses (tag at index 0
                    // as i32 → i64-zext, payload as byte buffer at index 1 with offsets
                    // from compute_enum_payload_offset).
                    if self.enums.contains_key(name) {
                        if field == "__tag" {
                            let tag_gep = b!(self.bld.build_struct_gep(st, ptr, 0, "tag"));
                            let i32t = self.ctx.i32_type();
                            let i64t = self.ctx.i64_type();
                            let tag_i32 = b!(self.bld.build_load(i32t, tag_gep, "tag"));
                            let val = b!(self.bld.build_int_z_extend(
                                tag_i32.into_int_value(),
                                i64t,
                                "tag.ext"
                            ));
                            return Ok(val.into());
                        }
                        if let Some(idx_str) = field.strip_prefix('_') {
                            if let Ok(idx) = idx_str.parse::<usize>() {
                                let payload_gep =
                                    b!(self.bld.build_struct_gep(st, ptr, 1, "payload"));
                                let byte_offset =
                                    self.compute_enum_payload_offset(&name.as_str(), idx);
                                let field_ptr = if byte_offset == 0 {
                                    payload_gep
                                } else {
                                    let offset_val =
                                        self.ctx.i64_type().const_int(byte_offset, false);
                                    unsafe {
                                        b!(self.bld.build_gep(
                                            self.ctx.i8_type(),
                                            payload_gep,
                                            &[offset_val],
                                            "payload.field"
                                        ))
                                    }
                                };
                                let is_rec =
                                    Compiler::is_recursive_field(result_ty, &name.as_str());
                                if is_rec {
                                    let ptr_ty =
                                        self.ctx.ptr_type(inkwell::AddressSpace::default());
                                    let heap_ptr =
                                        b!(self.bld.build_load(ptr_ty, field_ptr, "box.ptr"))
                                            .into_pointer_value();
                                    let val = b!(self.bld.build_load(res_llvm, heap_ptr, field));
                                    return Ok(val);
                                }
                                let val = b!(self.bld.build_load(res_llvm, field_ptr, field));
                                return Ok(val);
                            }
                        }
                    }
                    let field_idx = self.field_index(&name.as_str(), field);
                    let gep = b!(self.bld.build_struct_gep(st, ptr, field_idx, field));
                    return Ok(b!(self.bld.build_load(res_llvm, gep, field)));
                }
            }
            // No fallback — loading from an unknown struct pointer at offset 0
            // silently produces wrong values for any field other than the first.
            Err(format!(
                "FieldGet on pointer to unknown struct type for field `{field}`"
            ))
        } else if obj_val.is_array_value() {
            // Tuple — represented as an LLVM array [N x T].
            // Fields are named _0, _1, ...
            if let Some(idx_str) = field.strip_prefix('_') {
                if let Ok(idx) = idx_str.parse::<u32>() {
                    let val =
                        b!(self
                            .bld
                            .build_extract_value(obj_val.into_array_value(), idx, field));
                    return Ok(val);
                }
            }
            Ok(obj_val)
        } else {
            Ok(obj_val)
        }
    }

    pub(in crate::codegen) fn emit_vec_new(
        &mut self,
        elems: &[mir::ValueId],
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let elem_ty = match result_ty {
            Type::Vec(e) => e.as_ref(),
            _ => &Type::I64,
        };
        // Inline vec construction matching the sibling `vec.rs` layout: allocate {ptr, i64, i64} header.
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();
        let malloc = self.ensure_malloc();

        let header_size = i64t.const_int(24, false);

        // Perceus Vec reuse: empty `vec()` literal may consume a slot saved
        // by a preceding `Drop(Vec(T))` of the same element type. The slot
        // holds a header pointer whose data buffer is intact and whose
        // capacity is preserved; we only have to reset `len = 0`. On a
        // runtime miss we fall through to a normal malloc.
        if elems.is_empty() {
            if let Some(reused) = self.try_consume_vec_slot() {
                let is_null = b!(self.bld.build_is_null(reused, "vec.reuse.null"));
                let fv = self.current_fn();
                let malloc_bb = self.ctx.append_basic_block(fv, "vec.reuse.malloc");
                let cont_bb = self.ctx.append_basic_block(fv, "vec.reuse.cont");
                let entry_bb = self
                    .bld
                    .get_insert_block()
                    .expect("builder has no insert block");
                b!(self
                    .bld
                    .build_conditional_branch(is_null, malloc_bb, cont_bb));
                self.bld.position_at_end(malloc_bb);
                let m = b!(self
                    .bld
                    .build_call(malloc, &[header_size.into()], "vec.hdr"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_pointer_value();
                // Initialize fresh header to {NULL, 0, 0}.
                let dgep = b!(self.bld.build_struct_gep(header_ty, m, 0, "vec.hdr.d0"));
                b!(self.bld.build_store(dgep, ptr_ty.const_null()));
                let lgep = b!(self.bld.build_struct_gep(header_ty, m, 1, "vec.hdr.l0"));
                b!(self.bld.build_store(lgep, i64t.const_int(0, false)));
                let cgep = b!(self.bld.build_struct_gep(header_ty, m, 2, "vec.hdr.c0"));
                b!(self.bld.build_store(cgep, i64t.const_int(0, false)));
                b!(self.bld.build_unconditional_branch(cont_bb));
                self.bld.position_at_end(cont_bb);
                let phi = b!(self.bld.build_phi(ptr_ty, "vec.hdr.phi"));
                phi.add_incoming(&[(&m, malloc_bb), (&reused, entry_bb)]);
                let header_ptr = phi.as_basic_value().into_pointer_value();
                // Reset `len = 0` (data + cap preserved on hit; on miss they
                // are already zero from the fresh init).
                let len_gep =
                    b!(self
                        .bld
                        .build_struct_gep(header_ty, header_ptr, 1, "vec.len.reset"));
                b!(self.bld.build_store(len_gep, i64t.const_int(0, false)));
                return Ok(header_ptr.into());
            }
        }

        let header_ptr = b!(self
            .bld
            .build_call(malloc, &[header_size.into()], "vec.hdr"))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void")
        .into_pointer_value();

        let n = elems.len();
        let cap = if n == 0 {
            0u64
        } else {
            n.next_power_of_two() as u64
        };

        if n > 0 {
            let lty = self.llvm_ty(elem_ty);
            let elem_size = self.type_store_size(lty);
            let buf_size = i64t.const_int(cap * elem_size, false);
            let buf = b!(self.bld.build_call(malloc, &[buf_size.into()], "vec.buf"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_pointer_value();

            for (i, vid) in elems.iter().enumerate() {
                let val = self.val(*vid);
                let gep = unsafe {
                    b!(self
                        .bld
                        .build_gep(lty, buf, &[i64t.const_int(i as u64, false)], "vec.elem"))
                };
                b!(self.bld.build_store(gep, val));
            }

            let ptr_gep = b!(self
                .bld
                .build_struct_gep(header_ty, header_ptr, 0, "vec.ptr"));
            b!(self.bld.build_store(ptr_gep, buf));
        } else {
            let ptr_gep = b!(self
                .bld
                .build_struct_gep(header_ty, header_ptr, 0, "vec.ptr"));
            b!(self.bld.build_store(ptr_gep, ptr_ty.const_null()));
        }

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "vec.len"));
        b!(self
            .bld
            .build_store(len_gep, i64t.const_int(n as u64, false)));

        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "vec.cap"));
        b!(self.bld.build_store(cap_gep, i64t.const_int(cap, false)));

        Ok(header_ptr.into())
    }
}
