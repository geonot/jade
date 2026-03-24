use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn llvm_ty(&self, ty: &Type) -> BasicTypeEnum<'ctx> {
        match ty {
            Type::I8 | Type::U8 => self.ctx.i8_type().into(),
            Type::I16 | Type::U16 => self.ctx.i16_type().into(),
            Type::I32 | Type::U32 => self.ctx.i32_type().into(),
            Type::I64 | Type::U64 => self.ctx.i64_type().into(),
            Type::F32 => self.ctx.f32_type().into(),
            Type::F64 => self.ctx.f64_type().into(),
            Type::Bool => self.ctx.bool_type().into(),
            Type::Void => self.ctx.i8_type().into(),
            Type::String => self.string_type().into(),
            Type::Inferred | Type::TypeVar(_) => self.ctx.i64_type().into(),
            Type::Struct(name) | Type::Enum(name) => self
                .module
                .get_struct_type(name)
                .map(|s| s.into())
                .unwrap_or_else(|| self.ctx.i64_type().into()),
            Type::Array(et, n) => self.llvm_ty(et).array_type(*n as u32).into(),
            Type::Vec(_) | Type::Map(_, _) => {
                // Vec and Map are heap-allocated: {ptr, len, cap} or {ptr, len, cap}
                self.ctx.ptr_type(AddressSpace::default()).into()
            }
            Type::Tuple(tys) => self
                .ctx
                .struct_type(
                    &tys.iter().map(|t| self.llvm_ty(t)).collect::<Vec<_>>(),
                    false,
                )
                .into(),
            Type::Fn(_, _) | Type::Ptr(_) | Type::Rc(_) | Type::Weak(_) | Type::ActorRef(_) | Type::Coroutine(_) | Type::Channel(_) => {
                self.ctx.ptr_type(AddressSpace::default()).into()
            }
            Type::DynTrait(_) => {
                // Fat pointer: {data: ptr, vtable: ptr}
                let ptr = self.ctx.ptr_type(AddressSpace::default());
                self.ctx.struct_type(&[ptr.into(), ptr.into()], false).into()
            }
            Type::Param(_) => self.ctx.i64_type().into(),
        }
    }

    pub(crate) fn string_type(&self) -> inkwell::types::StructType<'ctx> {
        self.module.get_struct_type("String").unwrap_or_else(|| {
            let st = self.ctx.opaque_struct_type("String");
            st.set_body(
                &[
                    self.ctx.ptr_type(AddressSpace::default()).into(),
                    self.ctx.i64_type().into(),
                    self.ctx.i64_type().into(),
                ],
                false,
            );
            st
        })
    }

    pub(crate) fn type_store_size(&self, ty: BasicTypeEnum<'ctx>) -> u64 {
        match ty {
            BasicTypeEnum::IntType(it) => ((it.get_bit_width() + 7) / 8) as u64,
            BasicTypeEnum::FloatType(ft) => {
                if ft == self.ctx.f32_type() {
                    4
                } else {
                    8
                }
            }
            BasicTypeEnum::PointerType(_) => 8,
            BasicTypeEnum::StructType(st) => {
                let fields = st.get_field_types();
                let mut offset = 0u64;
                let mut max_align = 1u64;
                for f in &fields {
                    let fs = self.type_store_size(*f);
                    let fa = self.type_abi_align(*f);
                    offset = (offset + fa - 1) & !(fa - 1);
                    offset += fs;
                    max_align = max_align.max(fa);
                }
                (offset + max_align - 1) & !(max_align - 1)
            }
            BasicTypeEnum::ArrayType(at) => {
                if at.len() == 0 {
                    return 0;
                }
                let elem: BasicTypeEnum = at
                    .get_element_type()
                    .try_into()
                    .unwrap_or(self.ctx.i8_type().into());
                self.type_store_size(elem) * at.len() as u64
            }
            _ => 8,
        }
    }

    pub(crate) fn type_abi_align(&self, ty: BasicTypeEnum<'ctx>) -> u64 {
        match ty {
            BasicTypeEnum::IntType(it) => {
                let bytes = ((it.get_bit_width() + 7) / 8) as u64;
                bytes.next_power_of_two().min(8)
            }
            BasicTypeEnum::FloatType(_) => self.type_store_size(ty).min(8),
            BasicTypeEnum::PointerType(_) => 8,
            BasicTypeEnum::StructType(st) => st
                .get_field_types()
                .iter()
                .map(|f| self.type_abi_align(*f))
                .max()
                .unwrap_or(1),
            BasicTypeEnum::ArrayType(at) => {
                let elem: BasicTypeEnum = at
                    .get_element_type()
                    .try_into()
                    .unwrap_or(self.ctx.i8_type().into());
                self.type_abi_align(elem)
            }
            _ => 8,
        }
    }

    pub(crate) fn default_val(&self, ty: &Type) -> BasicValueEnum<'ctx> {
        match ty {
            Type::I8 | Type::U8 => self.ctx.i8_type().const_int(0, false).into(),
            Type::I16 | Type::U16 => self.ctx.i16_type().const_int(0, false).into(),
            Type::I32 | Type::U32 => self.ctx.i32_type().const_int(0, false).into(),
            Type::I64 | Type::U64 => self.ctx.i64_type().const_int(0, false).into(),
            Type::F32 => self.ctx.f32_type().const_float(0.0).into(),
            Type::F64 => self.ctx.f64_type().const_float(0.0).into(),
            Type::Bool => self.ctx.bool_type().const_int(0, false).into(),
            Type::String => self.string_type().const_zero().into(),
            _ => self.ctx.i64_type().const_int(0, false).into(),
        }
    }

    pub(crate) fn int_const(&self, n: i64, ty: &Type) -> BasicValueEnum<'ctx> {
        match ty.bits() {
            8 => self.ctx.i8_type().const_int(n as u64, true).into(),
            16 => self.ctx.i16_type().const_int(n as u64, true).into(),
            32 => self.ctx.i32_type().const_int(n as u64, true).into(),
            _ => self.ctx.i64_type().const_int(n as u64, true).into(),
        }
    }

    pub(crate) fn coerce_val(
        &self,
        val: BasicValueEnum<'ctx>,
        target: BasicTypeEnum<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        if val.get_type() == target {
            return val;
        }
        if val.is_int_value() && target.is_int_type() {
            let (fb, tb) = (
                val.into_int_value().get_type().get_bit_width(),
                target.into_int_type().get_bit_width(),
            );
            if fb < tb {
                return self
                    .bld
                    .build_int_z_extend(val.into_int_value(), target.into_int_type(), "ext")
                    .unwrap()
                    .into();
            } else if fb > tb {
                return self
                    .bld
                    .build_int_truncate(val.into_int_value(), target.into_int_type(), "trunc")
                    .unwrap()
                    .into();
            }
        }
        val
    }

    pub(crate) fn coerce_int_width(
        &self,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        lty: &Type,
        rty: &Type,
    ) -> (BasicValueEnum<'ctx>, BasicValueEnum<'ctx>) {
        if !lty.is_int() || !rty.is_int() || lty.bits() == rty.bits() {
            return (lhs, rhs);
        }
        if lty.bits() > rty.bits() {
            let ext = if rty.is_signed() {
                self.bld
                    .build_int_s_extend(
                        rhs.into_int_value(),
                        lhs.into_int_value().get_type(),
                        "widen",
                    )
                    .unwrap()
            } else {
                self.bld
                    .build_int_z_extend(
                        rhs.into_int_value(),
                        lhs.into_int_value().get_type(),
                        "widen",
                    )
                    .unwrap()
            };
            (lhs, ext.into())
        } else {
            let ext = if lty.is_signed() {
                self.bld
                    .build_int_s_extend(
                        lhs.into_int_value(),
                        rhs.into_int_value().get_type(),
                        "widen",
                    )
                    .unwrap()
            } else {
                self.bld
                    .build_int_z_extend(
                        lhs.into_int_value(),
                        rhs.into_int_value().get_type(),
                        "widen",
                    )
                    .unwrap()
            };
            (ext.into(), rhs)
        }
    }

    pub(crate) fn to_bool(&self, val: BasicValueEnum<'ctx>) -> IntValue<'ctx> {
        let iv = val.into_int_value();
        if iv.get_type().get_bit_width() == 1 {
            iv
        } else {
            self.bld
                .build_int_compare(
                    IntPredicate::NE,
                    iv,
                    iv.get_type().const_int(0, false),
                    "tobool",
                )
                .unwrap()
        }
    }

    pub(crate) fn wrap_negative_index(
        &mut self,
        idx: IntValue<'ctx>,
        len: u64,
    ) -> Result<IntValue<'ctx>, String> {
        if let Some(c) = idx.get_sign_extended_constant() {
            if c < 0 {
                return Ok(self
                    .ctx
                    .i64_type()
                    .const_int((len as i64 + c) as u64, false));
            }
            return Ok(idx);
        }
        let i64t = self.ctx.i64_type();
        let zero = i64t.const_int(0, false);
        let is_neg = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, idx, zero, "neg"));
        let wrapped = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(len, false), "wrap"));
        Ok(b!(self.bld.build_select(is_neg, wrapped, idx, "idx.wrap")).into_int_value())
    }

    pub(crate) fn resolve_ty(&self, ty: Type) -> Type {
        match &ty {
            Type::Struct(n) if self.enums.contains_key(n) => Type::Enum(n.clone()),
            _ => ty,
        }
    }

    /// Compile an explicit HIR coercion.
    pub(crate) fn compile_coercion(
        &mut self,
        val: BasicValueEnum<'ctx>,
        coercion: &hir::CoercionKind,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match coercion {
            hir::CoercionKind::IntWiden {
                to_bits, signed, ..
            } => {
                let target = self.ctx.custom_width_int_type(*to_bits).into();
                Ok(if *signed {
                    b!(self
                        .bld
                        .build_int_s_extend(val.into_int_value(), target, "sext"))
                    .into()
                } else {
                    b!(self
                        .bld
                        .build_int_z_extend(val.into_int_value(), target, "zext"))
                    .into()
                })
            }
            hir::CoercionKind::IntTrunc { to_bits, .. } => {
                let target = self.ctx.custom_width_int_type(*to_bits);
                Ok(b!(self
                    .bld
                    .build_int_truncate(val.into_int_value(), target, "trunc"))
                .into())
            }
            hir::CoercionKind::FloatWiden => Ok(b!(self.bld.build_float_ext(
                val.into_float_value(),
                self.ctx.f64_type(),
                "fpext"
            ))
            .into()),
            hir::CoercionKind::FloatNarrow => Ok(b!(self.bld.build_float_trunc(
                val.into_float_value(),
                self.ctx.f32_type(),
                "fptrunc"
            ))
            .into()),
            hir::CoercionKind::IntToFloat { signed } => {
                let f64t = self.ctx.f64_type();
                Ok(if *signed {
                    b!(self
                        .bld
                        .build_signed_int_to_float(val.into_int_value(), f64t, "sitofp"))
                    .into()
                } else {
                    b!(self
                        .bld
                        .build_unsigned_int_to_float(val.into_int_value(), f64t, "uitofp"))
                    .into()
                })
            }
            hir::CoercionKind::FloatToInt { signed } => {
                let i64t = self.ctx.i64_type();
                Ok(if *signed {
                    b!(self
                        .bld
                        .build_float_to_signed_int(val.into_float_value(), i64t, "fptosi"))
                    .into()
                } else {
                    b!(self
                        .bld
                        .build_float_to_unsigned_int(val.into_float_value(), i64t, "fptoui"))
                    .into()
                })
            }
            hir::CoercionKind::BoolToInt => Ok(b!(self.bld.build_int_z_extend(
                val.into_int_value(),
                self.ctx.i64_type(),
                "boolext"
            ))
            .into()),
        }
    }

    /// Check if a field type is a recursive reference to the enclosing enum.
    pub(crate) fn is_recursive_field(fty: &Type, enum_name: &str) -> bool {
        match fty {
            Type::Enum(n) | Type::Struct(n) => n == enum_name,
            _ => false,
        }
    }
}
