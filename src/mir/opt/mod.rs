//! MIR optimization passes — public API and driver.
//!
//! Each pass takes `&mut Function` and returns `true` if it changed anything.
//! The driver runs passes in a fixed-point loop until convergence.

use super::*;

mod cfg_passes;
mod fold;
mod memory_passes;
mod ranges;
mod scalar_passes;
mod subst;
mod uses;

pub use cfg_passes::{
    branch_threading, global_value_numbering, loop_invariant_code_motion, merge_linear_blocks,
    remove_unreachable_blocks,
};
pub use fold::constant_fold;
pub use memory_passes::{
    constant_branch_elimination, redundant_store_elimination, store_load_forwarding,
};
pub use ranges::{IntRange, compute_ranges};
pub use scalar_passes::{
    copy_propagation, dead_code_elimination, simplify_phis, strength_reduction,
};

/// Optimization level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    None,
    Basic,
    Full,
}

/// Run all optimization passes until a fixed point.
pub fn optimize(func: &mut Function, level: OptLevel) {
    if level == OptLevel::None {
        return;
    }

    remove_unreachable_blocks(func);

    let max = match level {
        OptLevel::None => 0,
        OptLevel::Basic => 4,
        OptLevel::Full => 16,
    };
    for _iter in 0..max {
        let mut changed = false;
        changed |= constant_fold(func);
        changed |= copy_propagation(func);
        changed |= store_load_forwarding(func);
        changed |= simplify_phis(func);
        changed |= dead_code_elimination(func);
        changed |= strength_reduction(func);
        if level == OptLevel::Full {
            changed |= global_value_numbering(func);
            changed |= branch_threading(func);
            changed |= loop_invariant_code_motion(func);
        }
        changed |= redundant_store_elimination(func);
        changed |= merge_linear_blocks(func);
        changed |= remove_unreachable_blocks(func);
        if !changed {
            break;
        }
    }
}
