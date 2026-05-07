//! Extended store codegen: KV, graph, time-series, vector, bloom, FTS, distinct, aggregation, and versioning operations.

use inkwell::values::BasicValueEnum;
use crate::mir;
use super::super::b;
use super::super::Compiler;

#[path = "store_ext_parts/impl0_part0.rs"]
mod store_ext_impl0_part0;
#[path = "store_ext_parts/impl0_part1.rs"]
mod store_ext_impl0_part1;
#[path = "store_ext_parts/impl0_part2.rs"]
mod store_ext_impl0_part2;
