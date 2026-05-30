use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use super::Compiler;
use super::b;

mod runtime_alloc;
