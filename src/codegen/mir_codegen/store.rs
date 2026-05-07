//! Store operation codegen: insert, query, count, all, delete, set, get, first, exists, destroy, restore, save.

use inkwell::values::{BasicValueEnum, PointerValue};
use crate::hir;
use crate::mir;
use crate::types::Type;
use super::super::Compiler;
use super::super::b;

#[path = "store_parts/impl0_part0.rs"]
mod store_store_parts_impl0_part0_rs;
#[path = "store_parts/impl0_part1.rs"]
mod store_store_parts_impl0_part1_rs;
#[path = "store_parts/impl0_part2.rs"]
mod store_store_parts_impl0_part2_rs;
#[path = "store_parts/impl0_part3.rs"]
mod store_store_parts_impl0_part3_rs;
#[path = "store_parts/impl0_part4.rs"]
mod store_store_parts_impl0_part4_rs;
#[path = "store_parts/impl0_part5.rs"]
mod store_store_parts_impl0_part5_rs;
