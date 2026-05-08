//! Helper methods for MIR codegen: binary/unary/comparison ops, casts, field access, closures, channels, slicing, and coroutine body extraction.

use super::super::Compiler;
use super::super::b;
use crate::hir;
use crate::intern::Symbol;
use crate::mir;
use crate::types::Type;
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::BasicValueEnum;
use std::collections::HashMap;

mod runtime;
mod values;
