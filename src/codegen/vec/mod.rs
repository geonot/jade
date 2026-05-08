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

mod core;
mod ordering;
mod transforms;
