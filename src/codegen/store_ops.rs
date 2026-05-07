//! High-level store operation lowerings (insert/query/update/delete/count/all/set).
//! `store_load_records` and `store_read_count` are MIR-live; the higher-level wrappers
//! are reached through HIR expression lowering.

use inkwell::IntPredicate;
use inkwell::values::BasicValueEnum;

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

use super::stores::HEADER_SIZE;

#[path = "store_ops_parts/impl0_part0.rs"]
mod store_ops_store_ops_parts_impl0_part0_rs;
#[path = "store_ops_parts/impl0_part1.rs"]
mod store_ops_store_ops_parts_impl0_part1_rs;
