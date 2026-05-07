//! Helper methods for MIR codegen: binary/unary/comparison ops, casts, field access, closures, channels, slicing, and coroutine body extraction.

use crate::intern::Symbol;
use std::collections::HashMap;
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::BasicValueEnum;
use crate::hir;
use crate::mir;
use crate::types::Type;
use super::super::b;
use super::super::Compiler;

impl<'ctx> Compiler<'ctx> {
    pub(super) fn emit_string_const(&mut self, s: &str) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_str_literal(s)
    }

    pub(super) fn emit_binop(
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
                    let first_param_is_ptr = fv
                        .get_type()
                        .get_param_types()
                        .first()
                        .map(|t| t.is_pointer_type())
                        .unwrap_or(false);
                    let self_arg: BasicValueEnum<'ctx> =
                        if first_param_is_ptr && !l.is_pointer_value() {
                            let tmp = self.entry_alloca(l.get_type(), "op.self");
                            b!(self.bld.build_store(tmp, l));
                            tmp.into()
                        } else {
                            l
                        };
                    let csv = b!(self.bld.build_call(
                        fv,
                        &[self_arg.into(), r.into()],
                        &format!("{method_name}.call")
                    ));
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
                        self.module
                            .add_function("pow", ft, Some(Linkage::External))
                    });
                    let result = b!(self
                        .bld
                        .build_call(pow, &[lf.into(), rf.into()], "pow"));
                    return Ok(self.call_result(result));
                }
                _ => return Err(format!("mir_codegen: unsupported float binop {op:?}")),
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
            let li = l.into_int_value();
            let ri = r.into_int_value();
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
                    b!(self
                        .bld
                        .build_right_shift(li, ri, false, "ushr"))
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
                        self.module
                            .add_function("pow", ft, Some(Linkage::External))
                    });
                    let result = b!(self
                        .bld
                        .build_call(pow, &[lf.into(), rf.into()], "pow"));
                    let fv = self.call_result(result).into_float_value();
                    let iv =
                        b!(self
                            .bld
                            .build_float_to_signed_int(fv, li.get_type(), "exp.i"));
                    return Ok(iv.into());
                }
            };
            Ok(res.into())
        }
    }

    pub(super) fn emit_unary(
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
            mir::UnaryOp::BitNot => {
                Ok(b!(self.bld.build_not(v.into_int_value(), "bitnot")).into())
            }
        }
    }

    pub(super) fn emit_cmp(
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
                    let first_param_is_ptr = fv
                        .get_type()
                        .get_param_types()
                        .first()
                        .map(|t| t.is_pointer_type())
                        .unwrap_or(false);
                    let self_arg: BasicValueEnum<'ctx> =
                        if first_param_is_ptr && !l.is_pointer_value() {
                            let tmp = self.entry_alloca(l.get_type(), "cmp.self");
                            b!(self.bld.build_store(tmp, l));
                            tmp.into()
                        } else {
                            l
                        };
                    let csv =
                        b!(self
                            .bld
                            .build_call(fv, &[self_arg.into(), r.into()], "cmp.call"));
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
            Ok(b!(self.bld.build_int_compare(
                pred,
                l.into_int_value(),
                r.into_int_value(),
                "icmp"
            ))
            .into())
        }
    }

    pub(super) fn emit_cast(
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

    pub(super) fn emit_field_get(
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

        // String field access — handle "length"/"len" via SSO-aware string_len
        let obj_ty = self.value_types.get(&obj).cloned();
        if matches!(&obj_ty, Some(Type::String)) {
            match field {
                "length" | "len" => return self.string_len(obj_val),
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
                            // Check if this field is a recursive reference (boxed as ptr).
                            let is_rec = Compiler::is_recursive_field(result_ty, name);
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
                let idx = self.field_index(name, field);
                let val = b!(self.bld.build_extract_value(sv, idx, field));
                return Ok(val);
            }
            // Unknown struct type — cannot determine correct field index.
            Err(format!(
                "mir_codegen: FieldGet on unknown struct type for field `{field}`"
            ))
        } else if obj_val.is_pointer_value() {
            // Vec .length/.len: read len field from vec header.
            if matches!(field, "length" | "len") {
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
                    self.value_types.get(&obj).and_then(|ty| match ty {
                        Type::Ptr(inner) => match inner.as_ref() {
                            Type::Struct(name, _) => Some(name.clone()),
                            _ => None,
                        },
                        Type::Struct(name, _) => Some(name.clone()),
                        _ => None,
                    })
                });
            if let Some(name) = &struct_name {
                if let Some(st) = self.module.get_struct_type(&name.as_str()) {
                    let field_idx = self.field_index(&name.as_str(), field);
                    let gep = b!(self.bld.build_struct_gep(st, ptr, field_idx, field));
                    return Ok(b!(self.bld.build_load(res_llvm, gep, field)));
                }
            }
            // No fallback — loading from an unknown struct pointer at offset 0
            // silently produces wrong values for any field other than the first.
            Err(format!(
                "mir_codegen: FieldGet on pointer to unknown struct type for field `{field}`"
            ))
        } else if obj_val.is_array_value() {
            // Tuple — represented as an LLVM array [N x T].
            // Fields are named _0, _1, ...
            if let Some(idx_str) = field.strip_prefix('_') {
                if let Ok(idx) = idx_str.parse::<u32>() {
                    let val = b!(self.bld.build_extract_value(
                        obj_val.into_array_value(),
                        idx,
                        field
                    ));
                    return Ok(val);
                }
            }
            Ok(obj_val)
        } else {
            Ok(obj_val)
        }
    }

    pub(super) fn emit_vec_new(
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
            let buf = b!(self
                .bld
                .build_call(malloc, &[buf_size.into()], "vec.buf"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void")
            .into_pointer_value();

            for (i, vid) in elems.iter().enumerate() {
                let val = self.val(*vid);
                let gep = unsafe {
                    b!(self.bld.build_gep(
                        lty,
                        buf,
                        &[i64t.const_int(i as u64, false)],
                        "vec.elem"
                    ))
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
        b!(self
            .bld
            .build_store(cap_gep, i64t.const_int(cap, false)));

        Ok(header_ptr.into())
    }

    pub(super) fn emit_closure_create(
        &mut self,
        fn_name: &str,
        captures: &[mir::ValueId],
        _result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let closure_ty = self.closure_type();

        // Look up the inner lambda function (has captures prepended as params).
        let inner_fv = if let Some((fv, _, _)) = self.fns.get(fn_name).cloned() {
            Some(fv)
        } else {
            self.module.get_function(fn_name)
        };

        // Build env struct from capture values.
        let cap_vals: Vec<BasicValueEnum<'ctx>> = captures.iter().map(|v| self.val(*v)).collect();
        let cap_tys: Vec<BasicTypeEnum<'ctx>> = cap_vals.iter().map(|v| v.get_type()).collect();

        let env_ptr = if !captures.is_empty() {
            let env_struct_ty = self.ctx.struct_type(&cap_tys, false);
            let env_size = env_struct_ty.size_of().expect("ICE: type has no size");
            let malloc = self.ensure_malloc();
            let ep = b!(self
                .bld
                .build_call(malloc, &[env_size.into()], "env.alloc"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void")
            .into_pointer_value();
            for (i, v) in cap_vals.iter().enumerate() {
                let gep =
                    b!(self
                        .bld
                        .build_struct_gep(env_struct_ty, ep, i as u32, "env.field"));
                b!(self.bld.build_store(gep, *v));
            }
            ep
        } else {
            ptr_ty.const_null()
        };

        // Build a wrapper function that takes (env_ptr, ...declared_params)
        // and calls the inner function with (captures..., declared_params...).
        let wrapper_ptr = if let Some(ifv) = inner_fv {
            let wrapper_name = format!("{fn_name}.env_wrap");
            if let Some(w) = self.module.get_function(&wrapper_name) {
                w.as_global_value().as_pointer_value()
            } else {
                let inner_type = ifv.get_type();
                let inner_params = inner_type.get_param_types();
                let n_captures = captures.len();
                // Declared params are everything after the captures.
                let declared_param_tys = &inner_params[n_captures..];
                let mut wrapper_params: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
                wrapper_params.extend(
                    declared_param_tys
                        .iter()
                        .map(|t| BasicMetadataTypeEnum::from(*t)),
                );
                let wrapper_ft = match inner_type.get_return_type() {
                    Some(ret) => ret.fn_type(&wrapper_params, false),
                    None => self.ctx.void_type().fn_type(&wrapper_params, false),
                };
                let wrapper_fv = self.module.add_function(
                    &wrapper_name,
                    wrapper_ft,
                    Some(inkwell::module::Linkage::Internal),
                );
                self.tag_fn(wrapper_fv);

                let saved_bb = self.bld.get_insert_block();
                let entry = self.ctx.append_basic_block(wrapper_fv, "entry");
                self.bld.position_at_end(entry);

                // Build call args: unpack captures from env, then forward declared params.
                let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
                if n_captures > 0 {
                    let env_struct_ty = self.ctx.struct_type(&cap_tys, false);
                    let env_param = wrapper_fv.get_nth_param(0).expect("ICE: missing param").into_pointer_value();
                    for i in 0..n_captures {
                        let gep = b!(self.bld.build_struct_gep(
                            env_struct_ty,
                            env_param,
                            i as u32,
                            "cap.gep"
                        ));
                        let load_ty: BasicTypeEnum<'ctx> = inner_params[i].try_into().unwrap();
                        let cap = b!(self.bld.build_load(load_ty, gep, "cap.load"));
                        call_args.push(cap.into());
                    }
                }
                // Forward declared params (skip env_ptr at index 0).
                for i in 0..declared_param_tys.len() {
                    let p = wrapper_fv.get_nth_param((i + 1) as u32).unwrap();
                    call_args.push(p.into());
                }

                let result = self
                    .bld
                    .build_call(ifv, &call_args, "lam.call")
                    .unwrap();
                match inner_type.get_return_type() {
                    Some(_) => {
                        let rv = self.call_result(result);
                        self.bld.build_return(Some(&rv)).unwrap();
                    }
                    None => {
                        self.bld.build_return(None).unwrap();
                    }
                }

                if let Some(bb) = saved_bb {
                    self.bld.position_at_end(bb);
                }
                wrapper_fv.as_global_value().as_pointer_value()
            }
        } else {
            // Fallback: no function found, use null.
            ptr_ty.const_null()
        };

        // Build {wrapper_ptr, env_ptr} closure struct.
        let mut agg: BasicValueEnum<'ctx> = closure_ty.const_zero().into();
        agg = b!(self.bld.build_insert_value(
            agg.into_struct_value(),
            wrapper_ptr,
            0,
            "closure.fn"
        ))
        .into_struct_value()
        .into();
        agg = b!(self.bld.build_insert_value(
            agg.into_struct_value(),
            env_ptr,
            1,
            "closure.env"
        ))
        .into_struct_value()
        .into();
        Ok(agg)
    }

    pub(super) fn emit_chan_create(
        &mut self,
        elem_ty: &Type,
        cap: Option<&mir::ValueId>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        if let Some(fv) = self.module.get_function("jade_chan_create") {
            let elem_size = self
                .llvm_ty(elem_ty)
                .size_of()
                .unwrap_or(i64t.const_int(8, false));
            let capacity = if let Some(cap_id) = cap {
                self.val(*cap_id).into_int_value()
            } else {
                i64t.const_int(64, false) // default capacity
            };
            let csv =
                b!(self
                    .bld
                    .build_call(fv, &[elem_size.into(), capacity.into()], "chan"));
            Ok(self.call_result(csv))
        } else {
            Ok(ptr_ty.const_null().into())
        }
    }

    pub(super) fn emit_chan_send(
        &mut self,
        ch: mir::ValueId,
        val: mir::ValueId,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_val = self.val(ch);
        let v = self.val(val);
        if let Some(fv) = self.module.get_function("jade_chan_send") {
            let alloca = self.entry_alloca(v.get_type(), "send.tmp");
            b!(self.bld.build_store(alloca, v));
            b!(self
                .bld
                .build_call(fv, &[ch_val.into(), alloca.into()], ""));
        }
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(super) fn emit_chan_recv(
        &mut self,
        ch: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_val = self.val(ch);
        if let Some(fv) = self.module.get_function("jade_chan_recv") {
            let elem_llvm = self.llvm_ty(result_ty);
            let alloca = self.entry_alloca(elem_llvm, "recv.tmp");
            b!(self
                .bld
                .build_call(fv, &[ch_val.into(), alloca.into()], ""));
            Ok(b!(self.bld.build_load(elem_llvm, alloca, "recv.val")))
        } else {
            Ok(self.default_val(result_ty))
        }
    }

    pub(super) fn field_index(&self, struct_name: &str, field: &str) -> u32 {
        self.structs
            .get(struct_name)
            .and_then(|fields| fields.iter().position(|(n, _)| n == field))
            .unwrap_or(0) as u32
    }

    pub(super) fn struct_name_from_type(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Struct(name, _) => Some(name.as_str()),
            Type::Ptr(inner) => match inner.as_ref() {
                Type::Struct(name, _) => Some(name.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Compute the byte offset for enum payload field at `target_idx`,
    /// matching the VariantInit layout (8-byte aligned actual type sizes).
    /// When we don't have type info, defaults to `target_idx * 8`.
    pub(super) fn compute_enum_payload_offset(&self, enum_name: &str, target_idx: usize) -> u64 {
        if let Some(variants) = self.enums.get(enum_name) {
            for (_, field_types) in variants {
                if field_types.len() > target_idx {
                    let mut offset: u64 = 0;
                    for (i, fty) in field_types.iter().enumerate() {
                        if i == target_idx {
                            return offset;
                        }
                        let type_size = if Compiler::is_recursive_field(fty, enum_name) {
                            8 // pointer
                        } else {
                            self.llvm_ty(fty)
                                .size_of()
                                .map(|s| s.get_zero_extended_constant().unwrap_or(8))
                                .unwrap_or(8)
                        };
                        offset += (type_size + 7) & !7;
                    }
                }
            }
        }
        (target_idx * 8) as u64
    }

    /// Emit dynamic dispatch: fat pointer vtable lookup and indirect call.
    pub(super) fn emit_dyn_dispatch(
        &mut self,
        obj: mir::ValueId,
        trait_name: &str,
        method: &str,
        args: &[mir::ValueId],
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fat = self.val(obj);
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let fat_ty = self
            .ctx
            .struct_type(&[ptr_ty.into(), ptr_ty.into()], false);

        let tmp = self.entry_alloca(fat_ty.into(), "dyn.tmp");
        b!(self.bld.build_store(tmp, fat));
        let data_gep = b!(self
            .bld
            .build_struct_gep(fat_ty, tmp, 0, "dyn.data.gep"));
        let data_ptr =
            b!(self.bld.build_load(ptr_ty, data_gep, "dyn.data")).into_pointer_value();
        let vtable_gep = b!(self
            .bld
            .build_struct_gep(fat_ty, tmp, 1, "dyn.vtable.gep"));
        let vtable_ptr =
            b!(self.bld.build_load(ptr_ty, vtable_gep, "dyn.vtable")).into_pointer_value();

        let method_idx = self
            .trait_method_order
            .get(trait_name)
            .and_then(|methods| methods.iter().position(|m| m == method))
            .unwrap_or(0) as u64;

        let fn_ptr_gep = unsafe {
            b!(self.bld.build_gep(
                ptr_ty,
                vtable_ptr,
                &[self.ctx.i64_type().const_int(method_idx, false)],
                "dyn.fn.gep"
            ))
        };
        let fn_ptr =
            b!(self.bld.build_load(ptr_ty, fn_ptr_gep, "dyn.fn")).into_pointer_value();

        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            vec![data_ptr.into()];
        for a in args {
            call_args.push(self.val(*a).into());
        }

        let ret_ty = self.llvm_ty(result_ty);
        let mut param_tys: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
        for a in args {
            param_tys.push(self.val(*a).get_type().into());
        }
        let fn_ty = ret_ty.fn_type(&param_tys, false);
        let result = b!(self
            .bld
            .build_indirect_call(fn_ty, fn_ptr, &call_args, "dyn.call"));
        Ok(result
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| self.ctx.i64_type().const_int(0, false).into()))
    }

    /// Emit slice operation for Vec or String types.
    pub(super) fn emit_slice(
        &mut self,
        base: mir::ValueId,
        lo: mir::ValueId,
        hi: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let base_val = self.val(base);
        let lo_val = self.val(lo);
        let hi_val = self.val(hi);

        match result_ty {
            Type::Vec(_) => {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i64t = self.ctx.i64_type();
                let slice_fn = self
                    .module
                    .get_function("__jade_vec_slice")
                    .unwrap_or_else(|| {
                        let ft = ptr_ty.fn_type(&[ptr_ty.into(), i64t.into(), i64t.into()], false);
                        self.module.add_function(
                            "__jade_vec_slice",
                            ft,
                            Some(Linkage::External),
                        )
                    });
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[base_val.into(), lo_val.into(), hi_val.into()],
                    "slice"
                ));
                Ok(self.call_result(result))
            }
            Type::String => {
                let st = self.llvm_ty(&Type::String);
                let i64t = self.ctx.i64_type();
                let slice_fn = self
                    .module
                    .get_function("__jade_str_slice")
                    .unwrap_or_else(|| {
                        let ft = st.fn_type(&[st.into(), i64t.into(), i64t.into()], false);
                        self.module.add_function(
                            "__jade_str_slice",
                            ft,
                            Some(Linkage::External),
                        )
                    });
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[base_val.into(), lo_val.into(), hi_val.into()],
                    "str.slice"
                ));
                Ok(self.call_result(result))
            }
            _ => Ok(self.ctx.i8_type().const_int(0, false).into()),
        }
    }

    // ── HIR coroutine/generator body extraction ───────────────────

    /// Walk the entire HIR program to extract CoroutineCreate and GeneratorCreate
    /// bodies, keyed by their name for later use in MIR codegen.
    pub(super) fn extract_coro_bodies_from_program(
        prog: &hir::Program,
        out: &mut HashMap<Symbol, Vec<hir::Stmt>>,
    ) {
        for f in &prog.fns {
            for stmt in &f.body {
                Self::extract_coro_bodies_from_stmt(stmt, out);
            }
        }
        for td in &prog.types {
            for m in &td.methods {
                for stmt in &m.body {
                    Self::extract_coro_bodies_from_stmt(stmt, out);
                }
            }
        }
        for ti in &prog.trait_impls {
            for m in &ti.methods {
                for stmt in &m.body {
                    Self::extract_coro_bodies_from_stmt(stmt, out);
                }
            }
        }
    }

    pub(super) fn extract_coro_bodies_from_stmt(stmt: &hir::Stmt, out: &mut HashMap<Symbol, Vec<hir::Stmt>>) {
        match stmt {
            hir::Stmt::Bind(b) => Self::extract_coro_bodies_from_expr(&b.value, out),
            hir::Stmt::Expr(e) => Self::extract_coro_bodies_from_expr(e, out),
            hir::Stmt::If(i) => {
                Self::extract_coro_bodies_from_expr(&i.cond, out);
                for s in &i.then {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
                if let Some(ref eb) = i.els {
                    for s in eb {
                        Self::extract_coro_bodies_from_stmt(s, out);
                    }
                }
                for elif in &i.elifs {
                    Self::extract_coro_bodies_from_expr(&elif.0, out);
                    for s in &elif.1 {
                        Self::extract_coro_bodies_from_stmt(s, out);
                    }
                }
            }
            hir::Stmt::While(w) => {
                Self::extract_coro_bodies_from_expr(&w.cond, out);
                for s in &w.body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::Stmt::For(f) => {
                Self::extract_coro_bodies_from_expr(&f.iter, out);
                for s in &f.body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::Stmt::Loop(l) => {
                for s in &l.body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::Stmt::Ret(Some(e), _, _) => Self::extract_coro_bodies_from_expr(e, out),
            hir::Stmt::Assign(a, b, _) => {
                Self::extract_coro_bodies_from_expr(a, out);
                Self::extract_coro_bodies_from_expr(b, out);
            }
            hir::Stmt::Match(m) => {
                Self::extract_coro_bodies_from_expr(&m.subject, out);
                for arm in &m.arms {
                    for s in &arm.body {
                        Self::extract_coro_bodies_from_stmt(s, out);
                    }
                }
            }
            hir::Stmt::SimFor(f, _) => {
                Self::extract_coro_bodies_from_expr(&f.iter, out);
                for s in &f.body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::Stmt::SimBlock(b, _) => {
                for s in b {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            _ => {}
        }
    }

    pub(super) fn extract_coro_bodies_from_expr(expr: &hir::Expr, out: &mut HashMap<Symbol, Vec<hir::Stmt>>) {
        match &expr.kind {
            hir::ExprKind::CoroutineCreate(name, body) => {
                out.insert(name.clone(), body.clone());
                // Also recurse into the body for nested coroutines
                for s in body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::ExprKind::GeneratorCreate(_, name, body) => {
                out.insert(name.clone(), body.clone());
                for s in body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::ExprKind::BinOp(a, _, b) => {
                Self::extract_coro_bodies_from_expr(a, out);
                Self::extract_coro_bodies_from_expr(b, out);
            }
            hir::ExprKind::UnaryOp(_, a) => Self::extract_coro_bodies_from_expr(a, out),
            hir::ExprKind::Call(_, _, args) => {
                for a in args {
                    Self::extract_coro_bodies_from_expr(a, out);
                }
            }
            hir::ExprKind::IndirectCall(f, args) => {
                Self::extract_coro_bodies_from_expr(f, out);
                for a in args {
                    Self::extract_coro_bodies_from_expr(a, out);
                }
            }
            hir::ExprKind::IfExpr(i) => {
                Self::extract_coro_bodies_from_expr(&i.cond, out);
                for s in &i.then {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
                if let Some(ref eb) = i.els {
                    for s in eb {
                        Self::extract_coro_bodies_from_stmt(s, out);
                    }
                }
            }
            hir::ExprKind::Block(b) => {
                for s in b {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::ExprKind::Lambda(_, b) => {
                for s in b {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            _ => {}
        }
    }
}
