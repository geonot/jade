//! CFG passes: GVN, branch threading, LICM, block merging, dead-block removal.

use super::subst::{subst_inst, subst_term};
use super::uses::{collect_inst_operands, is_pure};

use super::super::*;
use crate::ast::Span;
use crate::intern::Symbol;
use std::collections::{HashMap, HashSet, VecDeque};

pub fn global_value_numbering(func: &mut Function) -> bool {
    let mut replacements: HashMap<ValueId, ValueId> = HashMap::new();

    for bb in &func.blocks {
        let mut expr_map: HashMap<Symbol, ValueId> = HashMap::new();
        for inst in &bb.insts {
            // Invalidate cached FieldGet/Index entries on mutation.
            match &inst.kind {
                InstKind::FieldSet(_, _, _)
                | InstKind::FieldStore(_, _, _)
                | InstKind::FieldTombstone(_, _) => {
                    expr_map.retain(|k, _| !k.starts_with("fg:"));
                }
                InstKind::IndexSet(_, _, _) | InstKind::IndexStore(_, _, _) => {
                    expr_map.retain(|k, _| !k.starts_with("ix:"));
                }
                InstKind::Call(..) | InstKind::MethodCall(..) => {
                    // Calls may mutate anything; invalidate all field/index entries.
                    expr_map.retain(|k, _| !k.starts_with("fg:") && !k.starts_with("ix:"));
                }
                _ => {}
            }
            if let Some(d) = inst.dest {
                if !is_pure(&inst.kind) {
                    continue;
                }
                if let Some(key) = gvn_key(&inst.kind) {
                    if let Some(&existing) = expr_map.get(&Symbol::intern(&key)) {
                        replacements.insert(d, existing);
                    } else {
                        expr_map.insert(key.into(), d);
                    }
                }
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
    }
    true
}

/// Canonical string key for an expression. Commutative ops have normalized operand order.
fn gvn_key(kind: &InstKind) -> Option<String> {
    match kind {
        InstKind::BinOp(op, l, r) => {
            let (a, b) = if is_commutative(*op) && l.0 > r.0 {
                (r, l)
            } else {
                (l, r)
            };
            Some(format!("bin:{op:?}:{},{}", a.0, b.0))
        }
        InstKind::Cmp(op, l, r, _) => {
            let (a, b) = if matches!(op, CmpOp::Eq | CmpOp::Ne) && l.0 > r.0 {
                (r, l)
            } else {
                (l, r)
            };
            Some(format!("cmp:{op:?}:{},{}", a.0, b.0))
        }
        InstKind::UnaryOp(op, v) => Some(format!("un:{op:?}:{}", v.0)),
        InstKind::FieldGet(o, f) => Some(format!("fg:{}:{f}", o.0)),
        InstKind::Index(a, i) | InstKind::IndexUnchecked(a, i) => {
            Some(format!("ix:{}:{}", a.0, i.0))
        }
        InstKind::Cast(v, ty) => Some(format!("cast:{}:{ty:?}", v.0)),
        InstKind::StrictCast(v, ty) => Some(format!("scast:{}:{ty:?}", v.0)),
        _ => None,
    }
}

fn is_commutative(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Add
            | BinOp::Mul
            | BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::And
            | BinOp::Or
    )
}

// ━━━━━━━━━━━━━━━━ Branch Threading ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// If a block ends with a branch whose condition is determined by a phi,
/// and a predecessor supplies a constant for that phi, thread the edge
/// directly to the known successor.
pub fn branch_threading(func: &mut Function) -> bool {
    let mut changed = false;

    // Collect phi → condition mappings for branches
    let mut phi_vals: HashMap<(BlockId, ValueId), Vec<(BlockId, ValueId)>> = HashMap::new();
    for bb in &func.blocks {
        for phi in &bb.phis {
            phi_vals.insert((bb.id, phi.dest), phi.incoming.clone());
        }
    }

    // Find constants across the function
    let mut consts: HashMap<ValueId, bool> = HashMap::new();
    for bb in &func.blocks {
        for inst in &bb.insts {
            if let (Some(d), InstKind::BoolConst(b)) = (inst.dest, &inst.kind) {
                consts.insert(d, *b);
            }
        }
    }

    // For each branch on a phi value, check if any predecessor provides a constant
    let blocks_snapshot: Vec<(BlockId, Terminator)> = func
        .blocks
        .iter()
        .map(|bb| (bb.id, bb.terminator.clone()))
        .collect();

    for (bb_id, term) in &blocks_snapshot {
        if let Terminator::Branch(cond, then_bb, else_bb) = term {
            if let Some(incoming) = phi_vals.get(&(*bb_id, *cond)) {
                for (pred_id, val) in incoming {
                    if let Some(&b) = consts.get(val) {
                        let target = if b { *then_bb } else { *else_bb };
                        if target == *bb_id {
                            // Redirecting to self would be a no-op or worse.
                            continue;
                        }
                        // Capture, for every phi at bb_id, the value that
                        // pred_id was providing — so we can forward those
                        // bindings to `target`'s phis below.
                        let forwarded: Vec<(ValueId, ValueId)> = func
                            .block(*bb_id)
                            .phis
                            .iter()
                            .filter_map(|phi| {
                                phi.incoming
                                    .iter()
                                    .find(|(p, _)| p == pred_id)
                                    .map(|(_, v)| (phi.dest, *v))
                            })
                            .collect();

                        // Redirect predecessor's terminator from bb_id to target
                        func.block_mut(*pred_id)
                            .terminator
                            .replace_successor(*bb_id, target);

                        // Phi maintenance:
                        //  (a) Remove pred_id's incoming entry from each phi at
                        //      bb_id — pred_id no longer flows in here.
                        //  (b) For each phi at `target`, add an incoming entry
                        //      for pred_id. The value is whatever bb_id's phi
                        //      with the same dest was carrying (when target's
                        //      phi references a phi defined at bb_id), otherwise
                        //      reuse the value bb_id was already supplying for
                        //      its incoming edge to target.
                        {
                            let bb = func.block_mut(*bb_id);
                            let pred_still_referenced = match &bb.terminator {
                                Terminator::Branch(_, t, e) => *t == *bb_id || *e == *bb_id,
                                _ => false,
                            };
                            // pred_still_referenced is unused here; left for clarity.
                            let _ = pred_still_referenced;
                            for phi in &mut bb.phis {
                                phi.incoming.retain(|(p, _)| p != pred_id);
                            }
                        }

                        // Determine what each phi at `target` should receive
                        // from pred_id. The natural choice is the value that
                        // `bb_id` itself contributes to `target` for that phi,
                        // unless that value is one of bb_id's own phis — in
                        // which case forward the value pred_id was supplying
                        // to that phi.
                        let target_phi_updates: Vec<(ValueId, ValueId)> = func
                            .block(target)
                            .phis
                            .iter()
                            .filter_map(|phi| {
                                let v = phi
                                    .incoming
                                    .iter()
                                    .find(|(p, _)| *p == *bb_id)
                                    .map(|(_, v)| *v)?;
                                let chosen = if let Some((_, fwd)) =
                                    forwarded.iter().find(|(d, _)| *d == v)
                                {
                                    *fwd
                                } else {
                                    v
                                };
                                Some((phi.dest, chosen))
                            })
                            .collect();

                        let target_block = func.block_mut(target);
                        for phi in &mut target_block.phis {
                            // Avoid duplicate entries.
                            if phi.incoming.iter().any(|(p, _)| p == pred_id) {
                                continue;
                            }
                            if let Some((_, v)) =
                                target_phi_updates.iter().find(|(d, _)| *d == phi.dest)
                            {
                                phi.incoming.push((*pred_id, *v));
                            }
                        }

                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

// ━━━━━━━━━━━━━━ Loop-Invariant Code Motion ━━━━━━━━━━━━━━━━━━━━━━━

/// Hoist loop-invariant pure instructions to the loop preheader.
/// A loop is detected by finding back-edges (edges to a block with a
/// lower/equal index in the block list).
pub fn loop_invariant_code_motion(func: &mut Function) -> bool {
    let mut changed = false;

    // Build block index map.
    let block_ids: Vec<BlockId> = func.blocks.iter().map(|b| b.id).collect();
    let block_index: HashMap<BlockId, usize> = block_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();

    // Find back-edges: (from, to) where to <= from in block order.
    // NOTE: This assumes blocks are roughly in topological order. After
    // DCE/merging passes, verify the back-edge really forms a natural loop
    // by checking the header dominates all body blocks (approximated by
    // ensuring the header has a successor inside the body range).
    let mut loops: Vec<(BlockId, HashSet<usize>)> = Vec::new(); // (header, body block indices)

    for (i, bb) in func.blocks.iter().enumerate() {
        for succ in bb.terminator.successors() {
            if let Some(&succ_idx) = block_index.get(&succ) {
                if succ_idx <= i {
                    // Back-edge from block i to block succ_idx.
                    // Loop body = blocks [succ_idx..=i].
                    let body: HashSet<usize> = (succ_idx..=i).collect();
                    // Verify this is a real loop: the header must have at least
                    // one successor inside the loop body. A merge block (e.g.
                    // match.merge with return) has no body successors and is
                    // not a loop header.
                    let header_has_body_succ = func
                        .block(succ)
                        .terminator
                        .successors()
                        .iter()
                        .any(|s| block_index.get(s).map_or(false, |&si| body.contains(&si)));
                    // Additional check: verify all body blocks are reachable from
                    // the header within the body (validates natural loop structure).
                    let header_has_exit = func
                        .block(succ)
                        .terminator
                        .successors()
                        .iter()
                        .any(|s| block_index.get(s).map_or(true, |&si| !body.contains(&si)));
                    if header_has_body_succ && (body.len() > 1 || header_has_exit) {
                        loops.push((succ, body));
                    }
                }
            }
        }
    }

    if loops.is_empty() {
        return false;
    }

    // Collect all definitions (which block index defines each value).
    let mut def_block: HashMap<ValueId, usize> = HashMap::new();
    for (i, bb) in func.blocks.iter().enumerate() {
        for p in &func.params {
            def_block.entry(p.value).or_insert(0); // params are "defined" in entry
        }
        for phi in &bb.phis {
            def_block.insert(phi.dest, i);
        }
        for inst in &bb.insts {
            if let Some(d) = inst.dest {
                def_block.insert(d, i);
            }
        }
    }

    for (header, body) in &loops {
        let _header_idx = match block_index.get(header) {
            Some(&idx) => idx,
            None => continue,
        };

        // Find or identify the single predecessor outside the loop as preheader.
        let pred_map = func.predecessors();
        let header_preds = pred_map.get(header).cloned().unwrap_or_default();
        let preheader_preds: Vec<BlockId> = header_preds
            .into_iter()
            .filter(|p| {
                let pi = block_index.get(p).copied().unwrap_or(usize::MAX);
                !body.contains(&pi)
            })
            .collect();

        if preheader_preds.len() != 1 {
            continue; // Multiple entries or no clear preheader — skip.
        }
        let preheader_id = preheader_preds[0];

        // Collect instructions to hoist: pure, all operands defined outside the loop.
        let mut to_hoist: Vec<Instruction> = Vec::new();
        let mut hoisted_defs: HashSet<ValueId> = HashSet::new();

        // Iterate blocks in the loop body.
        for &bi in body {
            let bb = &func.blocks[bi];
            for inst in &bb.insts {
                if !is_pure(&inst.kind) {
                    continue;
                }
                let Some(dest) = inst.dest else {
                    continue;
                };

                // Check all operands are defined outside the loop (or already hoisted).
                let operands = collect_inst_operands(&inst.kind);
                let all_outside = operands.iter().all(|op| {
                    hoisted_defs.contains(op) || {
                        let d = def_block.get(op).copied().unwrap_or(0);
                        !body.contains(&d)
                    }
                });

                if all_outside {
                    to_hoist.push(inst.clone());
                    hoisted_defs.insert(dest);
                }
            }
        }

        if to_hoist.is_empty() {
            continue;
        }

        // Remove hoisted instructions from their original blocks.
        let hoisted_ids: HashSet<ValueId> = to_hoist.iter().filter_map(|i| i.dest).collect();
        for &bi in body {
            func.blocks[bi]
                .insts
                .retain(|i| i.dest.map_or(true, |d| !hoisted_ids.contains(&d)));
        }

        // Insert hoisted instructions at the end of preheader (before terminator).
        let ph_block = func.block_mut(preheader_id);
        for inst in to_hoist {
            ph_block.insts.push(inst);
        }
        changed = true;
    }

    changed
}

/// Collect all operand ValueIds from an instruction kind.
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

            // Check pred ends with Goto to this block
            if !matches!(func.block(pred_id).terminator, Terminator::Goto(t) if t == bb_id) {
                continue;
            }

            // Merge: append B's instructions and terminator to A, remove B.
            // Convert any phi nodes in B to Copy instructions (safe because
            // B has exactly one predecessor, so each phi has one incoming value).
            let b_phis = func.block(bb_id).phis.clone();
            let b_insts = func.block(bb_id).insts.clone();
            let b_term = func.block(bb_id).terminator.clone();

            let pred_block = func.block_mut(pred_id);
            for phi in b_phis {
                // Pick the incoming value from the actual predecessor, not just .first(),
                // because branch_threading may leave stale phi entries from dead predecessors.
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

            // Update phi incoming edges in other blocks that reference the
            // removed block — remap to the predecessor that absorbed it.
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
            break; // restart since indices changed
        }
        if !merged_any {
            break;
        }
    }
    changed
}

// ━━━━━━━━━━━━━━ Unreachable Block Removal ━━━━━━━━━━━━━━━━━━━━━━━━

/// Remove blocks not reachable from the entry via BFS.
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
    // Clean up phi incoming edges that reference removed blocks.
    if changed {
        for bb in &mut func.blocks {
            for phi in &mut bb.phis {
                phi.incoming.retain(|(bid, _)| reachable.contains(bid));
            }
        }
    }
    changed
}
