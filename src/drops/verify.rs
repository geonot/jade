use std::collections::{HashMap, HashSet};

use crate::mir::{self, BlockId, InstKind, Terminator, ValueId};
use crate::types::Type;

use super::mir_drops::inst_operands;

pub(super) fn enabled() -> bool {
    std::env::var("JINN_MIR_VERIFY").as_deref() != Ok("0")
}

pub(super) type DropMultiset = Vec<Vec<(ValueId, Type)>>;

pub(super) fn snapshot(func: &mir::Function) -> DropMultiset {
    func.blocks
        .iter()
        .map(|b| {
            let mut drops: Vec<(ValueId, Type)> = Vec::new();
            for inst in &b.insts {
                match &inst.kind {
                    InstKind::Drop(v, ty) => drops.push((*v, ty.clone())),
                    InstKind::DropMany(items) => {
                        drops.extend(items.iter().cloned());
                    }
                    _ => {}
                }
            }
            drops.sort_by_key(|(v, _)| v.0);
            drops
        })
        .collect()
}

pub(super) fn check_preserved(
    fname: &str,
    phase: &str,
    before: &DropMultiset,
    func: &mir::Function,
    allow_trivial_removal: bool,
    errors: &mut Vec<String>,
) {
    let after = snapshot(func);
    if before.len() != after.len() {
        errors.push(format!(
            "[fn {fname}] {phase} changed the block count ({} -> {})",
            before.len(),
            after.len()
        ));
        return;
    }
    for (bi, (b, a)) in before.iter().zip(after.iter()).enumerate() {
        let mut b_iter = b.iter().peekable();
        let mut a_iter = a.iter().peekable();
        loop {
            match (b_iter.peek(), a_iter.peek()) {
                (None, None) => break,
                (Some((bv, _)), Some((av, _))) if bv == av => {
                    b_iter.next();
                    a_iter.next();
                }
                (Some((bv, bty)), rest) if rest.is_none_or(|(av, _)| bv.0 < av.0) => {
                    if !(allow_trivial_removal && bty.is_trivially_droppable()) {
                        errors.push(format!(
                            "[fn {fname}] {phase} lost the drop of v{} in block {bi}",
                            bv.0
                        ));
                    }
                    b_iter.next();
                }
                (_, Some((av, _))) => {
                    errors.push(format!(
                        "[fn {fname}] {phase} introduced a drop of v{} in block {bi}",
                        av.0
                    ));
                    a_iter.next();
                }
                (Some(_), None) => unreachable!(),
            }
        }
    }
}

fn terminator_operands(t: &Terminator) -> Vec<ValueId> {
    match t {
        Terminator::Branch(c, _, _) => vec![*c],
        Terminator::Return(Some(v)) => vec![*v],
        Terminator::Switch(s, _, _) => vec![*s],
        Terminator::Goto(_) | Terminator::Return(None) | Terminator::Unreachable => vec![],
    }
}

fn process(
    func: &mir::Function,
    bi: usize,
    entry: &HashSet<ValueId>,
    mut errors: Option<&mut Vec<String>>,
) -> HashSet<ValueId> {
    let block = &func.blocks[bi];
    let mut dropped = entry.clone();
    for phi in &block.phis {
        dropped.remove(&phi.dest);
    }
    for inst in &block.insts {
        match &inst.kind {
            InstKind::Drop(v, _) => {
                if dropped.contains(v)
                    && let Some(errs) = errors.as_deref_mut()
                {
                    errs.push(format!(
                        "[fn {}] v{} is dropped twice on a path through block {bi}",
                        func.name, v.0
                    ));
                }
                dropped.insert(*v);
            }
            InstKind::DropMany(items) => {
                for (v, _) in items {
                    if dropped.contains(v)
                        && let Some(errs) = errors.as_deref_mut()
                    {
                        errs.push(format!(
                            "[fn {}] v{} is dropped twice on a path through block {bi} (fused)",
                            func.name, v.0
                        ));
                    }
                    dropped.insert(*v);
                }
            }
            kind => {
                if let Some(errs) = errors.as_deref_mut() {
                    for u in inst_operands(kind) {
                        if dropped.contains(&u) {
                            errs.push(format!(
                                "[fn {}] v{} is used in block {bi} after being dropped",
                                func.name, u.0
                            ));
                        }
                    }
                }
            }
        }
        if let Some(d) = inst.dest {
            dropped.remove(&d);
        }
    }
    if let Some(errs) = errors {
        for u in terminator_operands(&block.terminator) {
            if dropped.contains(&u) {
                errs.push(format!(
                    "[fn {}] v{} is used by the terminator of block {bi} after being dropped",
                    func.name, u.0
                ));
            }
        }
    }
    dropped
}

pub(super) fn verify_function(func: &mir::Function) -> Vec<String> {
    let mut errors: Vec<String> = Vec::new();

    let index_of: HashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    let n = func.blocks.len();

    let mut entry_state: Vec<HashSet<ValueId>> = vec![HashSet::new(); n];
    let mut exit_state: Vec<HashSet<ValueId>> = vec![HashSet::new(); n];

    loop {
        let mut changed = false;
        for bi in 0..n {
            let exit = process(func, bi, &entry_state[bi], None);
            if exit != exit_state[bi] {
                exit_state[bi] = exit;
                changed = true;
            }
            for succ in func.blocks[bi].terminator.successors() {
                if let Some(&si) = index_of.get(&succ) {
                    let add: Vec<ValueId> = exit_state[bi]
                        .iter()
                        .filter(|v| !entry_state[si].contains(v))
                        .copied()
                        .collect();
                    if !add.is_empty() {
                        entry_state[si].extend(add);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    for (bi, entry) in entry_state.iter().enumerate() {
        let _ = process(func, bi, entry, Some(&mut errors));
    }

    for (bi, block) in func.blocks.iter().enumerate() {
        for phi in &block.phis {
            for (pred, v) in &phi.incoming {
                if let Some(&pi) = index_of.get(pred)
                    && exit_state[pi].contains(v)
                {
                    errors.push(format!(
                        "[fn {}] phi v{} in block {bi} reads v{} along the edge from block {pi}, \
                         but that value is dropped before the edge",
                        func.name, phi.dest.0, v.0
                    ));
                }
            }
        }
    }

    let mut drop_counts: HashMap<ValueId, (u32, u32)> = HashMap::new();
    let mut vecnew_dests: HashSet<ValueId> = HashSet::new();
    for block in &func.blocks {
        for inst in &block.insts {
            match &inst.kind {
                InstKind::Drop(v, _) => drop_counts.entry(*v).or_default().0 += 1,
                InstKind::DropMany(items) => {
                    for (v, _) in items {
                        drop_counts.entry(*v).or_default().1 += 1;
                    }
                }
                InstKind::VecNew(_) => {
                    if let Some(d) = inst.dest {
                        vecnew_dests.insert(d);
                    }
                }
                _ => {}
            }
        }
    }
    for (v, slot) in &func.drops.reuse_save {
        let (plain, fused) = drop_counts.get(v).copied().unwrap_or((0, 0));
        if plain == 0 {
            errors.push(format!(
                "[fn {}] reuse slot {slot} saves v{} but no Drop of it remains",
                func.name, v.0
            ));
        }
        if fused > 0 {
            errors.push(format!(
                "[fn {}] reuse slot {slot} saves v{} but its drop was fused into DropMany",
                func.name, v.0
            ));
        }
    }
    for (v, slot) in &func.drops.reuse_consume {
        if !vecnew_dests.contains(v) {
            errors.push(format!(
                "[fn {}] reuse slot {slot} is consumed by v{} which is not a VecNew",
                func.name, v.0
            ));
        }
    }
    let known_slots: HashSet<u32> = func
        .drops
        .reuse_save
        .values()
        .chain(func.drops.reuse_consume.values())
        .copied()
        .collect();
    for slot in &func.drops.vec_slots {
        if !known_slots.contains(slot) {
            errors.push(format!(
                "[fn {}] vec slot {slot} has no save or consume site",
                func.name
            ));
        }
    }

    let mut seen: HashSet<String> = HashSet::new();
    errors.retain(|e| seen.insert(e.clone()));
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::DefId;
    use crate::intern::Symbol;
    use crate::mir::{BasicBlock, DropMeta, Function, Instruction, Phi};

    fn inst(dest: Option<u32>, kind: InstKind, ty: Type) -> Instruction {
        Instruction {
            dest: dest.map(ValueId),
            kind,
            ty,
            span: crate::ast::Span::dummy(),
            def_id: None,
        }
    }

    fn func(blocks: Vec<BasicBlock>) -> Function {
        Function {
            name: Symbol::intern("t"),
            def_id: DefId(0),
            pkg_id: None,
            params: Vec::new(),
            ret_ty: Type::Void,
            blocks,
            entry: BlockId(0),
            span: crate::ast::Span::dummy(),
            next_value: 100,
            next_block: 100,
            attrs: Default::default(),
            is_coroutine: false,
            scheduler_task: false,
            cancel_cleanup: None,
            drops: DropMeta::default(),
        }
    }

    fn block(id: u32, insts: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            id: BlockId(id),
            label: Symbol::intern(&format!("b{id}")),
            phis: Vec::new(),
            insts,
            terminator,
        }
    }

    #[test]
    fn clean_drop_passes() {
        let f = func(vec![block(
            0,
            vec![
                inst(
                    Some(1),
                    InstKind::VecNew(vec![]),
                    Type::Vec(Box::new(Type::I64)),
                ),
                inst(
                    None,
                    InstKind::Drop(ValueId(1), Type::Vec(Box::new(Type::I64))),
                    Type::Void,
                ),
            ],
            Terminator::Return(None),
        )]);
        assert!(verify_function(&f).is_empty());
    }

    #[test]
    fn use_after_drop_is_reported() {
        let f = func(vec![block(
            0,
            vec![
                inst(
                    Some(1),
                    InstKind::VecNew(vec![]),
                    Type::Vec(Box::new(Type::I64)),
                ),
                inst(
                    None,
                    InstKind::Drop(ValueId(1), Type::Vec(Box::new(Type::I64))),
                    Type::Void,
                ),
                inst(Some(2), InstKind::VecLen(ValueId(1)), Type::I64),
            ],
            Terminator::Return(None),
        )]);
        let errs = verify_function(&f);
        assert!(
            errs.iter().any(|e| e.contains("after being dropped")),
            "{errs:?}"
        );
    }

    #[test]
    fn double_drop_across_join_is_reported() {
        let vty = Type::Vec(Box::new(Type::I64));
        let f = func(vec![
            block(
                0,
                vec![
                    inst(Some(1), InstKind::VecNew(vec![]), vty.clone()),
                    inst(Some(2), InstKind::BoolConst(true), Type::Bool),
                ],
                Terminator::Branch(ValueId(2), BlockId(1), BlockId(2)),
            ),
            block(
                1,
                vec![inst(
                    None,
                    InstKind::Drop(ValueId(1), vty.clone()),
                    Type::Void,
                )],
                Terminator::Goto(BlockId(2)),
            ),
            block(
                2,
                vec![inst(
                    None,
                    InstKind::Drop(ValueId(1), vty.clone()),
                    Type::Void,
                )],
                Terminator::Return(None),
            ),
        ]);
        let errs = verify_function(&f);
        assert!(errs.iter().any(|e| e.contains("dropped twice")), "{errs:?}");
    }

    #[test]
    fn loop_local_drop_does_not_false_positive() {
        let vty = Type::Vec(Box::new(Type::I64));
        let mut header = block(
            1,
            vec![
                inst(Some(10), InstKind::VecNew(vec![]), vty.clone()),
                inst(Some(11), InstKind::VecLen(ValueId(10)), Type::I64),
                inst(None, InstKind::Drop(ValueId(10), vty.clone()), Type::Void),
                inst(Some(12), InstKind::BoolConst(true), Type::Bool),
            ],
            Terminator::Branch(ValueId(12), BlockId(1), BlockId(2)),
        );
        header.phis.push(Phi {
            dest: ValueId(20),
            ty: Type::I64,
            incoming: vec![(BlockId(0), ValueId(2)), (BlockId(1), ValueId(11))],
        });
        let f = func(vec![
            block(
                0,
                vec![inst(Some(2), InstKind::IntConst(0), Type::I64)],
                Terminator::Goto(BlockId(1)),
            ),
            header,
            block(2, vec![], Terminator::Return(None)),
        ]);
        let errs = verify_function(&f);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn preservation_flags_a_lost_drop() {
        let vty = Type::Vec(Box::new(Type::I64));
        let mut f = func(vec![block(
            0,
            vec![
                inst(Some(1), InstKind::VecNew(vec![]), vty.clone()),
                inst(None, InstKind::Drop(ValueId(1), vty.clone()), Type::Void),
            ],
            Terminator::Return(None),
        )]);
        let before = snapshot(&f);
        f.blocks[0].insts.pop();
        let mut errs = Vec::new();
        check_preserved("t", "test-phase", &before, &f, false, &mut errs);
        assert!(errs.iter().any(|e| e.contains("lost the drop")), "{errs:?}");
        let mut ok_errs = Vec::new();
        check_preserved("t", "test-phase", &before, &f, true, &mut ok_errs);
        assert!(!ok_errs.is_empty(), "vec drops are not trivially droppable");
    }
}
