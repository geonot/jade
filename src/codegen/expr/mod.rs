use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum};
use inkwell::{FloatPredicate, IntPredicate};

use crate::ast::UnaryOp;
use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

mod access;
mod core;
mod runtime;
