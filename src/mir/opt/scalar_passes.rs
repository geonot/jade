use super::subst::{subst_inst, subst_term};
use super::uses::{collect_used, is_pure, may_trap};

use super::super::*;
use std::collections::{HashMap, HashSet};

pub fn simplify_phis(func: &mut Function) -> bool {
    let mut replacements: HashMap<ValueId, ValueId> = HashMap::new();

    let mut live = HashSet::new();
    for bb in &func.blocks {
        for inst in &bb.insts {
            super::uses::collect_inst_uses(&inst.kind, &mut live);
        }
        match &bb.terminator {
            Terminator::Branch(v, _, _)
            | Terminator::Switch(v, _, _)
            | Terminator::Return(Some(v)) => {
                live.insert(*v);
            }
            _ => {}
        }
    }
    loop {
        let before = live.len();
        for bb in &func.blocks {
            for phi in &bb.phis {
                if live.contains(&phi.dest) {
                    live.extend(phi.incoming.iter().map(|(_, v)| *v));
                }
            }
        }
        if before == live.len() {
            break;
        }
    }
    let mut removed = false;
    for bb in &mut func.blocks {
        let before = bb.phis.len();
        bb.phis.retain(|p| live.contains(&p.dest));
        removed |= before != bb.phis.len();
    }

    for bb in &func.blocks {
        for (index, phi) in bb.phis.iter().enumerate() {
            if let Some(previous) = bb.phis[..index]
                .iter()
                .find(|other| other.ty == phi.ty && other.incoming == phi.incoming)
            {
                replacements.insert(phi.dest, previous.dest);
                continue;
            }
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
        return removed;
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
                .is_none_or(|d| used.contains(&d) || !is_pure(&inst.kind) || may_trap(&inst.kind))
        });
        if bb.insts.len() != before {
            changed = true;
        }
    }
    changed
}
