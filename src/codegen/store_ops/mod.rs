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

mod mutation;
mod query;
