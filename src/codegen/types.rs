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
                .unwrap_or_else(|| panic!("unresolved nominal layout: {name}")),
            Type::Array(et, n) => self
                .llvm_ty(et)
                .array_type(u32::try_from(*n).expect("array exceeds native layout limit"))
                .into(),
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
                    .unwrap_or_else(|| panic!("unresolved store row layout: {name}"))
            }
            Type::View(_) => self.view_type().into(),
            Type::Frozen(inner) => self.llvm_ty(inner),
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

    pub(crate) fn view_type(&self) -> inkwell::types::StructType<'ctx> {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        self.ctx
            .struct_type(&[ptr.into(), self.ctx.i64_type().into()], false)
    }

    pub(crate) fn layout(&self) -> &inkwell::targets::TargetData {
        self.target_data
            .as_ref()
            .expect("target layout not initialized")
    }

    pub(crate) fn type_store_size(&self, ty: BasicTypeEnum<'ctx>) -> u64 {
        assert!(ty.is_sized(), "unsized native layout: {ty}");
        self.layout().get_abi_size(&ty)
    }

    pub(crate) fn type_size_of(&self, ty: &Type) -> u64 {
        self.type_store_size(self.llvm_ty(ty))
    }

    pub(crate) fn type_abi_align(&self, ty: BasicTypeEnum<'ctx>) -> u64 {
        assert!(ty.is_sized(), "unsized native layout: {ty}");
        u64::from(self.layout().get_abi_alignment(&ty))
    }

    pub(crate) fn default_val(&self, ty: &Type) -> BasicValueEnum<'ctx> {
        self.llvm_ty(ty).const_zero()
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
