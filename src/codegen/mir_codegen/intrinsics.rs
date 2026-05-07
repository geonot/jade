//! Builtin and intrinsic codegen: overflow arithmetic, bit operations, string formatting, and sleep.

use inkwell::AddressSpace;
use inkwell::values::{BasicValue, BasicValueEnum};
use crate::mir;
use crate::types::Type;
use super::super::b;
use super::super::Compiler;

#[path = "intrinsics_parts/impl0_part0.rs"]
mod intrinsics_intrinsics_parts_impl0_part0_rs;
#[path = "intrinsics_parts/impl0_part1.rs"]
mod intrinsics_intrinsics_parts_impl0_part1_rs;
