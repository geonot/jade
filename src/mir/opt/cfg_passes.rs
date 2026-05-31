use super::super::*;
use crate::ast::Span;
use std::collections::{HashSet, VecDeque};

pub fn merge_linear_blocks(func: &mut Function) -> bool {
    let mut changed = false;

    loop {
        let mut merged_any = false;
        let pred_map = func.predecessors();

        for i in 0..func.blocks.len() {
            let bb_id = func.blocks[i].id;
            if bb_id == func.entry {
                continue;
            }

            let pred_list = pred_map.get(&bb_id).cloned().unwrap_or_default();
            if pred_list.len() != 1 {
                continue;
            }
            let pred_id = pred_list[0];

            if !matches!(func.block(pred_id).terminator, Terminator::Goto(t) if t == bb_id) {
                continue;
            }

            let b_phis = func.block(bb_id).phis.clone();
            let b_insts = func.block(bb_id).insts.clone();
            let b_term = func.block(bb_id).terminator.clone();

            let pred_block = func.block_mut(pred_id);
            for phi in b_phis {
                let val = phi
                    .incoming
                    .iter()
                    .find(|(bid, _)| *bid == pred_id)
                    .map(|(_, v)| *v)
                    .or_else(|| phi.incoming.first().map(|(_, v)| *v));
                if let Some(val) = val {
                    pred_block.insts.push(Instruction {
                        dest: Some(phi.dest),
                        kind: InstKind::Copy(val),
                        ty: phi.ty,
                        span: Span::dummy(),
                        def_id: None,
                    });
                }
            }
            pred_block.insts.extend(b_insts);
            pred_block.terminator = b_term;

            for other_bb in &mut func.blocks {
                if other_bb.id == bb_id {
                    continue;
                }
                for phi in &mut other_bb.phis {
                    for (bid, _) in &mut phi.incoming {
                        if *bid == bb_id {
                            *bid = pred_id;
                        }
                    }
                }
            }

            func.blocks.retain(|b| b.id != bb_id);
            merged_any = true;
            changed = true;
            break;
        }
        if !merged_any {
            break;
        }
    }
    changed
}

pub fn remove_unreachable_blocks(func: &mut Function) -> bool {
    let mut reachable = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(func.entry);
    while let Some(id) = queue.pop_front() {
        if !reachable.insert(id) {
            continue;
        }
        for succ in func.block(id).terminator.successors() {
            queue.push_back(succ);
        }
    }
    let before = func.blocks.len();
    func.blocks.retain(|b| reachable.contains(&b.id));
    let changed = func.blocks.len() != before;

    if changed {
        for bb in &mut func.blocks {
            for phi in &mut bb.phis {
                phi.incoming.retain(|(bid, _)| reachable.contains(bid));
            }
        }
    }
    changed
}
