//! Store operation codegen: insert, query, count, all, delete, set, get, first, exists, destroy, restore, save.

use super::super::Compiler;
use super::super::b;
use crate::hir;
use crate::mir;
use crate::types::Type;
use inkwell::values::{BasicValueEnum, PointerValue};

mod delete;
mod insert;
mod lifecycle;
mod mutation;
mod query;
mod read;
