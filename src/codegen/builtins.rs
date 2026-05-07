//! Codegen dispatch for `hir::BuiltinFn` variants. Maps each builtin to LLVM IR or runtime call.

use inkwell::module::Linkage;
use inkwell::values::{BasicValue, BasicValueEnum};
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

#[path = "builtins_parts/impl0_part0.rs"]
mod builtins_builtins_parts_impl0_part0_rs;
#[path = "builtins_parts/impl0_part1.rs"]
mod builtins_builtins_parts_impl0_part1_rs;
#[path = "builtins_parts/impl0_part2.rs"]
mod builtins_builtins_parts_impl0_part2_rs;
#[path = "builtins_parts/impl0_part3.rs"]
mod builtins_builtins_parts_impl0_part3_rs;
