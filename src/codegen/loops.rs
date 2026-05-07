//! Codegen for loop constructs: `while`, `for`, `loop`, and parallel/sim variants.

use indexmap::IndexMap;
use inkwell::module::Linkage;
use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

#[path = "loops_parts/impl0_part0.rs"]
mod loops_loops_parts_impl0_part0_rs;
#[path = "loops_parts/impl0_part1.rs"]
mod loops_loops_parts_impl0_part1_rs;
