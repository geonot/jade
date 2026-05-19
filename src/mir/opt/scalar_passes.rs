use super::subst::{subst_inst, subst_term};
use super::uses::{collect_used, is_pure};

use super::super::*;
use crate::ast::Span;
use crate::types::Type;
use std::collections::{HashMap, HashSet};

pub fn copy_propagation(func: &mut Function) -> bool {
    let mut copies: HashMap<ValueId, ValueId> = HashMap::new();
    for bb in &func.blocks {
        for inst in &bb.insts {
            if let (Some(d), InstKind::Copy(src)) = (inst.dest, &inst.kind) {
                copies.insert(d, *src);
            }
        }
    }
    if copies.is_empty() {
        return false;
    }

    let resolved: HashMap<ValueId, ValueId> = copies
        .keys()
        .map(|&k| {
            let mut v = k;
            let mut seen = HashSet::new();
            while let Some(&next) = copies.get(&v) {
                if !seen.insert(v) {
                    break;
                }
                v = next;
            }
            (k, v)
        })
        .collect();

    let mut changed = false;
    for bb in &mut func.blocks {
        for phi in &mut bb.phis {
            for (_, v) in &mut phi.incoming {
                if let Some(&r) = resolved.get(v) {
                    *v = r;
                    changed = true;
                }
            }
        }
        for inst in &mut bb.insts {
            changed |= subst_inst(inst, &resolved);
        }
        changed |= subst_term(&mut bb.terminator, &resolved);

        bb.insts.retain(|inst| {
            if let (Some(d), InstKind::Copy(_)) = (inst.dest, &inst.kind) {
                !resolved.contains_key(&d)
            } else {
                true
            }
        });
    }
    changed
}

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

pub fn strength_reduction(func: &mut Function) -> bool {
    let mut changed = false;
    let mut iconsts: HashMap<ValueId, i64> = HashMap::new();

    for bb in &func.blocks {
        for inst in &bb.insts {
            if let (Some(d), InstKind::IntConst(n)) = (inst.dest, &inst.kind) {
                iconsts.insert(d, *n);
            }
        }
    }

    let mut needed_shifts: Vec<i64> = Vec::new();
    for bb in &func.blocks {
        for inst in &bb.insts {
            if let InstKind::BinOp(BinOp::Mul, l, r) = &inst.kind {
                for v in [l, r] {
                    if let Some(&n) = iconsts.get(v) {
                        if n > 1 && (n as u64).is_power_of_two() {
                            needed_shifts.push((n as u64).trailing_zeros() as i64);
                        }
                    }
                }
            }
        }
    }

    let mut shift_vals: HashMap<i64, ValueId> = HashMap::new();
    for shift in needed_shifts {
        if shift_vals.contains_key(&shift) {
            continue;
        }
        let d = func.new_value();
        let entry = func.entry;
        func.block_mut(entry).insts.insert(
            0,
            Instruction {
                dest: Some(d),
                kind: InstKind::IntConst(shift),
                ty: Type::I64,
                span: Span::dummy(),
                def_id: None,
            },
        );
        shift_vals.insert(shift, d);
    }

    for bb in &mut func.blocks {
        for inst in &mut bb.insts {
            let new_kind = match &inst.kind {
                InstKind::BinOp(BinOp::Mul, l, r) => match (iconsts.get(l), iconsts.get(r)) {
                    (_, Some(n)) if *n > 0 && (*n as u64).is_power_of_two() => {
                        let shift = (*n as u64).trailing_zeros() as i64;
                        if shift == 0 {
                            Some(InstKind::Copy(*l))
                        } else {
                            Some(InstKind::BinOp(BinOp::Shl, *l, shift_vals[&shift]))
                        }
                    }
                    (Some(n), _) if *n > 0 && (*n as u64).is_power_of_two() => {
                        let shift = (*n as u64).trailing_zeros() as i64;
                        if shift == 0 {
                            Some(InstKind::Copy(*r))
                        } else {
                            Some(InstKind::BinOp(BinOp::Shl, *r, shift_vals[&shift]))
                        }
                    }

                    (_, Some(0)) | (Some(0), _) => Some(InstKind::IntConst(0)),
                    _ => None,
                },

                InstKind::BinOp(BinOp::Add, l, r) if iconsts.get(r) == Some(&0) => {
                    Some(InstKind::Copy(*l))
                }
                InstKind::BinOp(BinOp::Add, l, r) if iconsts.get(l) == Some(&0) => {
                    Some(InstKind::Copy(*r))
                }
                InstKind::BinOp(BinOp::Sub, l, r) if iconsts.get(r) == Some(&0) => {
                    Some(InstKind::Copy(*l))
                }

                InstKind::BinOp(BinOp::Div, l, r) if iconsts.get(r) == Some(&1) => {
                    Some(InstKind::Copy(*l))
                }

                InstKind::BinOp(BinOp::Mod, _, r) if iconsts.get(r) == Some(&1) => {
                    Some(InstKind::IntConst(0))
                }

                InstKind::BinOp(BinOp::BitAnd, _, r) if iconsts.get(r) == Some(&0) => {
                    Some(InstKind::IntConst(0))
                }
                InstKind::BinOp(BinOp::BitOr, l, r) if iconsts.get(r) == Some(&0) => {
                    Some(InstKind::Copy(*l))
                }
                InstKind::BinOp(BinOp::BitXor, l, r) if iconsts.get(r) == Some(&0) => {
                    Some(InstKind::Copy(*l))
                }

                InstKind::BinOp(BinOp::Sub, l, r) if l == r => Some(InstKind::IntConst(0)),

                InstKind::BinOp(BinOp::BitXor, l, r) if l == r => Some(InstKind::IntConst(0)),
                _ => None,
            };
            if let Some(k) = new_kind {
                inst.kind = k;
                changed = true;
            }
        }
    }
    changed
}
