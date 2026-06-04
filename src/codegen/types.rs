use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::{AddressSpace, IntPredicate};

use crate::types::Type;

use super::Compiler;

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
            Type::TypeVar(v) => {
                panic!(
                    "ICE: unresolved TypeVar({v}) reached codegen — this indicates a monomorphization bug in the typer"
                );
            }
            Type::Struct(name, _) | Type::Enum(name) => self
                .module
                .get_struct_type(&name.as_str())
                .map(|s| s.into())
                .unwrap_or_else(|| self.ctx.i64_type().into()),
            Type::Array(et, n) => self.llvm_ty(et).array_type(*n as u32).into(),
            Type::Vec(_) | Type::Map(_, _) => self.ctx.ptr_type(AddressSpace::default()).into(),
            Type::Tuple(tys) => self
                .ctx
                .struct_type(
                    &tys.iter().map(|t| self.llvm_ty(t)).collect::<Vec<_>>(),
                    false,
                )
                .into(),
            Type::Fn(_, _) => self.closure_type().into(),
            Type::Ptr(_) | Type::ActorRef(_) | Type::Coroutine(_) | Type::Channel(_) => {
                self.ctx.ptr_type(AddressSpace::default()).into()
            }
            Type::Param(name) => {
                panic!(
                    "ICE: unresolved type parameter '{name}' reached codegen — this indicates a monomorphization bug in the typer"
                );
            }
            Type::Generator(_) => self.ctx.ptr_type(AddressSpace::default()).into(),
            Type::Alias(_, inner) | Type::Newtype(_, inner) => self.llvm_ty(inner),

            Type::Row(name) => {
                let sname = format!("__store_{}", name.as_str());
                self.module
                    .get_struct_type(&sname)
                    .map(|s| s.into())
                    .unwrap_or_else(|| self.ctx.i64_type().into())
            }
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

    pub(crate) fn closure_type(&self) -> inkwell::types::StructType<'ctx> {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        self.ctx.struct_type(&[ptr.into(), ptr.into()], false)
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
            Type::Fn(_, _) => self.closure_type().const_zero().into(),
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

    pub(crate) fn is_recursive_field(fty: &Type, enum_name: &str) -> bool {
        match fty {
            Type::Enum(n) | Type::Struct(n, _) => n == enum_name,
            _ => false,
        }
    }
}
