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

mod parallel;
mod sequential;
