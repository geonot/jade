use super::super::*;
use super::subst::{subst_inst, subst_term};
use crate::intern::Symbol;
use std::collections::{HashMap, HashSet};

pub fn store_load_forwarding(func: &mut Function) -> bool {
    let mut replacements: HashMap<ValueId, ValueId> = HashMap::new();
    let mut dead_loads: HashSet<ValueId> = HashSet::new();

    for bb in &func.blocks {
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

        bb.insts
            .retain(|inst| !inst.dest.map_or(false, |d| dead_loads.contains(&d)));
    }
    true
}
