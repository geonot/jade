//! Vector helpers on `Compiler<'ctx>`: header layout, growth, push/pop, slicing,
//! and HOF lowerings. Consumed by both `mir_codegen` and sibling helper files.

use inkwell::module::Linkage;
use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

#[path = "vec_parts/impl0_part0.rs"]
mod vec_vec_parts_impl0_part0_rs;
#[path = "vec_parts/impl0_part1.rs"]
mod vec_vec_parts_impl0_part1_rs;
#[path = "vec_parts/impl0_part2.rs"]
mod vec_vec_parts_impl0_part2_rs;
