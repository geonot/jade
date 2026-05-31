use super::*;

mod cfg_passes;
mod memory_passes;
mod scalar_passes;
pub(crate) mod subst;
mod uses;

pub use cfg_passes::{merge_linear_blocks, remove_unreachable_blocks};
pub use memory_passes::store_load_forwarding;
pub use scalar_passes::{dead_code_elimination, simplify_phis};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    None,
    Basic,
    Full,
}

pub fn optimize(func: &mut Function, level: OptLevel) {
    if level == OptLevel::None {
        return;
    }

    remove_unreachable_blocks(func);

    let max = match level {
        OptLevel::None => 0,
        OptLevel::Basic => 4,
        OptLevel::Full => 8,
    };
    for _iter in 0..max {
        let mut changed = false;
        changed |= store_load_forwarding(func);
        changed |= simplify_phis(func);
        changed |= dead_code_elimination(func);
        changed |= merge_linear_blocks(func);
        changed |= remove_unreachable_blocks(func);
        if !changed {
            break;
        }
    }
}
