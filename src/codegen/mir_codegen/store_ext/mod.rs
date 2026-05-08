//! Extended store codegen: KV, graph, time-series, vector, bloom, FTS, distinct, aggregation, and versioning operations.

use super::super::Compiler;
use super::super::b;
use crate::mir;
use inkwell::values::BasicValueEnum;

mod analytics;
mod history;
mod specialized;
