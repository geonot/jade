use inkwell::IntPredicate;
use inkwell::values::BasicValueEnum;

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

use super::stores::HEADER_SIZE;

mod mutation;
mod query;
