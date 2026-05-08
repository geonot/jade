//! Builtin and intrinsic codegen: overflow arithmetic, bit operations, string formatting, and sleep.

use super::super::Compiler;
use super::super::b;
use crate::mir;
use crate::types::Type;
use inkwell::AddressSpace;
use inkwell::values::{BasicValue, BasicValueEnum};

mod formatting;
mod overflow;
