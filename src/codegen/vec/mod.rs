use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::types::Type;

use super::Compiler;
use super::b;

mod core;
mod ordering;
mod transforms;
