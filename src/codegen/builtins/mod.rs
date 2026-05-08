//! Codegen dispatch for `hir::BuiltinFn` variants. Maps each builtin to LLVM IR or runtime call.

use inkwell::module::Linkage;
use inkwell::values::{BasicValue, BasicValueEnum};
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

mod dispatch_math;
mod float_string;
mod intrinsics;
mod runtime_alloc;
