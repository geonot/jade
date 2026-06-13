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
    true
}
