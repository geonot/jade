//! Store schema codegen: declarations, runtime initialization, handle accessors,
//! and WAL helpers. Consumed by `mir_codegen/store.rs` and `mir_codegen/store_ext.rs`.

use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{FunctionValue, PointerValue};

use crate::hir;
use crate::intern::Symbol;
use crate::types::Type;

use super::Compiler;
use super::b;

pub(crate) const STRING_BUF_SIZE: u64 = 256;

pub(crate) const HEADER_SIZE: u64 = 24;

mod handles;
mod indexing;
mod runtime;
