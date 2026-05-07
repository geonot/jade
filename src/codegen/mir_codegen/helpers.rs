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

#[path = "helpers_parts/impl0_part0.rs"]
mod helpers_helpers_parts_impl0_part0_rs;
#[path = "helpers_parts/impl0_part1.rs"]
mod helpers_helpers_parts_impl0_part1_rs;
