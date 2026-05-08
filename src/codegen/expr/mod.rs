//! Expression codegen helpers. Reached transitively from `mir_codegen` via the
//! actor/coroutine/closure entry points which still walk HIR. The `compile_str_literal`
//! and `compile_const_expr` helpers are direct MIR utilities.

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
