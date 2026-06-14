use super::super::*;
use crate::ast::Span;
use crate::intern::Symbol;
use std::collections::{HashMap, HashSet, VecDeque};

fn reachable_from(func: &Function, start: BlockId) -> HashSet<BlockId> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(start);
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        for succ in func.block(id).terminator.successors() {
            queue.push_back(succ);
        }
    }
    seen
}

fn runs_in_coroutine_context(func: &Function) -> bool {
    func.is_coroutine || func.name.as_str().starts_with("__actor_")
}

pub fn inject_yields(func: &mut Function) -> bool {
    if func.attrs.no_yield || !runs_in_coroutine_context(func) {
        return false;
    }

    let mut reach: HashMap<BlockId, HashSet<BlockId>> = HashMap::new();
    let block_ids: Vec<BlockId> = func.blocks.iter().map(|b| b.id).collect();
    for &id in &block_ids {
        reach.insert(id, reachable_from(func, id));
    }

    let mut yield_srcs: HashSet<BlockId> = HashSet::new();
    for bb in &func.blocks {
        for succ in bb.terminator.successors() {
            if reach.get(&succ).is_some_and(|s| s.contains(&bb.id)) {
                yield_srcs.insert(bb.id);
            }
        }
    }

    if yield_srcs.is_empty() {
        return false;
    }

    for bb in &mut func.blocks {
        if yield_srcs.contains(&bb.id) {
            bb.insts.push(Instruction {
                dest: None,
                kind: InstKind::Call(Symbol::intern("__sched_yield"), Vec::new()),
                ty: Type::Void,
                span: Span::dummy(),
                def_id: None,
            });
        }
    }

    inject_cancel_checks(func, &yield_srcs);
    true
}

/// For a scheduler task with a cancel-cleanup block, make every back-edge a
/// cooperative cancellation point: if the running coroutine has been cancelled
/// (its enclosing scope was `stop`ped or a sibling failed), branch to the
/// cleanup block so the body's defers run before the task exits, instead of
/// looping forever.
fn inject_cancel_checks(func: &mut Function, yield_srcs: &HashSet<BlockId>) {
    let Some(cleanup) = func.cancel_cleanup else {
        return;
    };
    if !func.scheduler_task {
        return;
    }

    let srcs: Vec<BlockId> = func
        .blocks
        .iter()
        .map(|b| b.id)
        .filter(|id| yield_srcs.contains(id) && *id != cleanup)
        .collect();

    for src in srcs {
        let idx = func.blocks.iter().position(|b| b.id == src).unwrap();
        let orig_term = func.blocks[idx].terminator.clone();

        let cont = func.new_block("cancel.cont");
        let chk = func.new_value();
        let flag = func.new_value();

        let cont_bb = func.blocks.iter_mut().find(|b| b.id == cont).unwrap();
        cont_bb.terminator = orig_term;

        let bb = func.blocks.iter_mut().find(|b| b.id == src).unwrap();
        bb.insts.push(Instruction {
            dest: Some(chk),
            kind: InstKind::Call(Symbol::intern("jinn_scope_check_cancelled"), Vec::new()),
            ty: Type::I32,
            span: Span::dummy(),
            def_id: None,
        });
        let zero = func.new_value();
        let bb = func.blocks.iter_mut().find(|b| b.id == src).unwrap();
        bb.insts.push(Instruction {
            dest: Some(zero),
            kind: InstKind::IntConst(0),
            ty: Type::I32,
            span: Span::dummy(),
            def_id: None,
        });
        bb.insts.push(Instruction {
            dest: Some(flag),
            kind: InstKind::Cmp(CmpOp::Ne, chk, zero, Type::I32),
            ty: Type::Bool,
            span: Span::dummy(),
            def_id: None,
        });
        bb.terminator = Terminator::Branch(flag, cleanup, cont);
    }
}
