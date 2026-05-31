use super::subst::{subst_inst, subst_term};
use super::uses::{collect_used, is_pure};

use super::super::*;
use std::collections::{HashMap, HashSet};

pub fn simplify_phis(func: &mut Function) -> bool {
    let mut replacements: HashMap<ValueId, ValueId> = HashMap::new();

    for bb in &func.blocks {
        for phi in &bb.phis {
            let unique: HashSet<ValueId> = phi
                .incoming
                .iter()
                .map(|(_, v)| *v)
                .filter(|v| *v != phi.dest)
                .collect();
            if unique.len() == 1 {
                replacements.insert(phi.dest, *unique.iter().next().unwrap());
            } else if unique.is_empty() {
                continue;
            }
        }
    }

    if replacements.is_empty() {
        return false;
    }

    let resolved: HashMap<ValueId, ValueId> = replacements
        .keys()
        .map(|&k| {
            let mut v = k;
            let mut seen = HashSet::new();
            while let Some(&next) = replacements.get(&v) {
                if !seen.insert(v) {
                    break;
                }
                v = next;
            }
            (k, v)
        })
        .collect();

    for bb in &mut func.blocks {
        bb.phis.retain(|phi| !resolved.contains_key(&phi.dest));
        for inst in &mut bb.insts {
            subst_inst(inst, &resolved);
        }
        subst_term(&mut bb.terminator, &resolved);
        for phi in &mut bb.phis {
            for (_, v) in &mut phi.incoming {
                if let Some(&r) = resolved.get(v) {
                    *v = r;
                }
            }
        }
    }
    true
}

pub fn dead_code_elimination(func: &mut Function) -> bool {
    let used = collect_used(func);
    let mut changed = false;
    for bb in &mut func.blocks {
        let before = bb.insts.len();
        bb.insts.retain(|inst| {
            inst.dest
                .map_or(true, |d| used.contains(&d) || !is_pure(&inst.kind))
        });
        if bb.insts.len() != before {
            changed = true;
        }
    }
    changed
}
