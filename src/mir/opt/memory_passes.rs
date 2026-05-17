//! Memory passes: store-load forwarding, redundant store elim, branch elim.

use super::super::*;
use super::subst::{subst_inst, subst_term};
use std::collections::{HashMap, HashSet};

pub fn store_load_forwarding(func: &mut Function) -> bool {
    let mut replacements: HashMap<ValueId, ValueId> = HashMap::new();
    let mut dead_loads: HashSet<ValueId> = HashSet::new();

    for bb in &func.blocks {
        // Skip blocks that loop back to themselves (self-loops).
        // In such blocks, a Store at the end of the iteration updates
        // the value that a Load at the beginning of the next iteration
        // should see — forwarding would incorrectly use the initial value.
        if bb.terminator.successors().contains(&bb.id) {
            continue;
        }

        let mut known: HashMap<Symbol, ValueId> = HashMap::new();

        for inst in &bb.insts {
            match &inst.kind {
                InstKind::Store(name, val) => {
                    known.insert(*name, *val);
                }
                InstKind::Load(name) => {
                    if let Some(&val) = known.get(name) {
                        if let Some(dest) = inst.dest {
                            replacements.insert(dest, val);
                            dead_loads.insert(dest);
                        }
                    } else if let Some(dest) = inst.dest {
                        known.insert(*name, dest);
                    }
                }
                InstKind::Call(..)
                | InstKind::MethodCall(..)
                | InstKind::ChanSend(..)
                | InstKind::ChanRecv(..)
                | InstKind::SelectArm(..)
                | InstKind::Log(..) => {
                    known.clear();
                }
                InstKind::FieldStore(var_name, _, _) => {
                    // Mutating a field of a variable invalidates its cached Load.
                    known.remove(var_name);
                }
                InstKind::FieldTombstone(var_name, _) => {
                    known.remove(var_name);
                }
                InstKind::IndexStore(var_name, _, _) => {
                    known.remove(var_name);
                }
                _ => {}
            }
        }
    }

    if replacements.is_empty() {
        return false;
    }

    for bb in &mut func.blocks {
        for phi in &mut bb.phis {
            for (_, v) in &mut phi.incoming {
                if let Some(&r) = replacements.get(v) {
                    *v = r;
                }
            }
        }
        for inst in &mut bb.insts {
            subst_inst(inst, &replacements);
        }
        subst_term(&mut bb.terminator, &replacements);
        // Remove dead loads (whose values were forwarded).
        bb.insts
            .retain(|inst| !inst.dest.map_or(false, |d| dead_loads.contains(&d)));
    }
    true
}

/// Remove stores that are overwritten before being read.
/// Within each basic block, if `Store(name, v1)` is followed by
/// `Store(name, v2)` with no intervening `Load(name)`, the first store
/// is dead.
pub fn redundant_store_elimination(func: &mut Function) -> bool {
    let mut changed = false;

    for bb in &mut func.blocks {
        let mut to_remove: HashSet<usize> = HashSet::new();
        // Maps variable name → index of last Store instruction.
        let mut last_store_idx: HashMap<Symbol, usize> = HashMap::new();

        for (i, inst) in bb.insts.iter().enumerate() {
            match &inst.kind {
                InstKind::Store(name, _) => {
                    if let Some(prev_idx) = last_store_idx.insert(*name, i) {
                        to_remove.insert(prev_idx);
                    }
                }
                InstKind::Load(name) => {
                    // A load reads the stored value — the store is live.
                    last_store_idx.remove(name);
                }
                InstKind::Call(..)
                | InstKind::ChanSend(..)
                | InstKind::ChanRecv(..)
                | InstKind::SelectArm(..)
                | InstKind::Log(..) => {
                    // Conservative: calls/effects might observe memory.
                    last_store_idx.clear();
                }
                _ => {}
            }
        }

        if !to_remove.is_empty() {
            let mut idx = 0;
            bb.insts.retain(|_| {
                let keep = !to_remove.contains(&idx);
                idx += 1;
                keep
            });
            changed = true;
        }
    }
    changed
}

/// Replace branches on constant booleans with unconditional gotos,
/// making the dead successor potentially unreachable.
pub fn constant_branch_elimination(func: &mut Function) -> bool {
    let mut consts: HashMap<ValueId, bool> = HashMap::new();
    for bb in &func.blocks {
        for inst in &bb.insts {
            if let (Some(d), InstKind::BoolConst(b)) = (inst.dest, &inst.kind) {
                consts.insert(d, *b);
            }
        }
    }
    let mut changed = false;
    for bb in &mut func.blocks {
        if let Terminator::Branch(cond, then_bb, else_bb) = &bb.terminator {
            if let Some(&b) = consts.get(cond) {
                let target = if b { *then_bb } else { *else_bb };
                bb.terminator = Terminator::Goto(target);
                changed = true;
            }
        }
    }
    changed
}
