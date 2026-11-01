use std::collections::{HashMap, HashSet};

use crate::intern::Symbol;
use crate::mir::{self, BlockId, InstKind, Instruction, Terminator, ValueId};
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

pub(super) use super::ConsumingMap;

pub(super) fn owning_allocs(
    func: &mir::Function,
    consuming: &ConsumingMap,
    resources: &HashSet<Symbol>,
    heap_structs: &HashSet<Symbol>,
) -> HashMap<ValueId, Type> {
    let mut owning: HashMap<ValueId, Type> = func
        .params
        .iter()
        .filter(|p| p.owned && !p.ty.is_trivially_droppable())
        .map(|p| (p.value, p.ty.clone()))
        .collect();
    for block in &func.blocks {
        for inst in &block.insts {
            let Some(dest) = inst.dest else { continue };
            match &inst.kind {
                InstKind::VecNew(_) | InstKind::MapInit | InstKind::ChanCreate(..) => {
                    owning.insert(dest, inst.ty.clone());
                }
                InstKind::StructInit(name, _)
                    if resources.contains(name) || heap_structs.contains(name) =>
                {
                    owning.insert(dest, inst.ty.clone());
                }
                InstKind::ClosureCreate(_, captures) if !captures.is_empty() => {
                    owning.insert(dest, inst.ty.clone());
                }
                InstKind::Clone(_, _)
                | InstKind::VariantInit(..)
                | InstKind::ArrayInit(_)
                | InstKind::ChanRecv(_)
                | InstKind::IndirectCall(_, _)
                | InstKind::ClosureCall(_, _)
                    if !inst.ty.is_trivially_droppable() =>
                {
                    owning.insert(dest, inst.ty.clone());
                }
                InstKind::BinOp(_, _, _) if matches!(inst.ty, Type::String) => {
                    owning.insert(dest, inst.ty.clone());
                }
                InstKind::MethodCall(_, name, _, borrow)
                    if !*borrow
                        && matches!(inst.ty, Type::String | Type::Vec(_) | Type::Map(_, _))
                        && crate::builtin_methods::FRESH_RESULT_METHODS
                            .contains(&&*name.as_str()) =>
                {
                    owning.insert(dest, inst.ty.clone());
                }
                InstKind::Call(name, _)
                    if !inst.ty.is_trivially_droppable()
                        && (consuming.contains_key(name)
                            || is_store_vec_alloc(&name.as_str())
                            || name.as_str().starts_with("__gen_create_")
                            || name.as_str().starts_with("__coro_create_")
                            || name.as_str() == "__select_recv"
                            || name.as_str() == PAYLOAD_MOVE) =>
                {
                    owning.insert(dest, inst.ty.clone());
                }
                _ => {}
            }
        }
    }
    loop {
        let before = owning.len();
        for block in &func.blocks {
            for phi in &block.phis {
                if phi.incoming.iter().any(|(_, v)| owning.contains_key(v)) {
                    owning.insert(phi.dest, phi.ty.clone());
                }
            }
            for inst in &block.insts {
                let source = match inst.kind {
                    InstKind::Copy(v)
                    | InstKind::Cast(v, _)
                    | InstKind::FieldSet(v, _, _)
                    | InstKind::FieldClear(v, _)
                    | InstKind::IndexSet(v, _, _) => Some(v),
                    _ => None,
                };
                if let (Some(dest), Some(source)) = (inst.dest, source)
                    && owning.contains_key(&source)
                    && !inst.ty.is_trivially_droppable()
                {
                    owning.insert(dest, inst.ty.clone());
                }
            }
        }
        if before == owning.len() {
            break;
        }
    }
    owning
}

fn is_store_vec_alloc(name: &str) -> bool {
    name.starts_with("__store_all_")
        || name.starts_with("__store_allq_")
        || name.starts_with("__store_distinct_")
        || name.starts_with("__store_group_")
        || name.starts_with("__store_qgroup_")
        || name.starts_with("__store_history_")
}

fn reads_without_taking(kind: &InstKind) -> bool {
    matches!(
        kind,
        InstKind::VecLen(_)
            | InstKind::FieldGet(_, _)
            | InstKind::Index(_, _)
            | InstKind::IndexUnchecked(_, _)
            | InstKind::Cmp(_, _, _, _)
            | InstKind::BinOp(_, _, _)
            | InstKind::UnaryOp(_, _)
            | InstKind::Log(_)
            | InstKind::Eprint(_)
            | InstKind::Assert(_, _)
            | InstKind::Slice(_, _, _)
            | InstKind::Clone(_, _)
            | InstKind::ChanRecv(_)
            | InstKind::Deref(_)
            | InstKind::SelectArm(_, _)
            | InstKind::Ref(_)
            | InstKind::Alloc(_)
    )
}

pub(super) const PAYLOAD_MOVE: &str = "__builtin_PayloadMove";

pub(super) fn strict_transfers(kind: &InstKind, consuming: &ConsumingMap) -> Vec<ValueId> {
    match kind {
        InstKind::Drop(_, _) | InstKind::DropMany(_) => vec![],
        InstKind::Call(name, args) if name.as_str() == PAYLOAD_MOVE => vec![args[0]],
        InstKind::VecPush(_, value) | InstKind::ChanSend(_, value) => vec![*value],
        InstKind::Call(name, args)
            if name.as_str().starts_with("__gen_create_")
                || name.as_str().starts_with("__coro_create_") =>
        {
            args.clone()
        }
        InstKind::Call(name, args)
            if name.as_str().starts_with("__scope_spawn_")
                || name.as_str().starts_with("__send_") =>
        {
            args.iter().skip(1).copied().collect()
        }
        InstKind::Call(name, args)
            if matches!(
                &*name.as_str(),
                "__cleanup_cell_owned" | "__cleanup_cell_set" | "__cleanup_push"
            ) =>
        {
            args.get(1).copied().into_iter().collect()
        }
        InstKind::IndirectCall(_, _) | InstKind::ClosureCall(_, _) => vec![],
        InstKind::Call(name, args) => match consuming.get(name) {
            Some(slots) => args
                .iter()
                .enumerate()
                .filter(|(i, _)| slots.get(*i).copied().unwrap_or(false))
                .map(|(_, v)| *v)
                .collect(),
            None => vec![],
        },
        InstKind::MethodCall(recv, name, args, _) => match consuming.get(name) {
            Some(slots) => {
                let mut out = Vec::new();
                if slots.first().copied().unwrap_or(false) {
                    out.push(*recv);
                }
                for (i, a) in args.iter().enumerate() {
                    if slots.get(i + 1).copied().unwrap_or(false) {
                        out.push(*a);
                    }
                }
                out
            }
            None => {
                let m = name.as_str();
                if crate::builtin_methods::INSERTING_METHODS.contains(&m.as_str()) {
                    args.clone()
                } else {
                    vec![]
                }
            }
        },
        k if reads_without_taking(k) => vec![],
        k => inst_operands(k),
    }
}

fn process_must_held(
    func: &mir::Function,
    bi: usize,
    owning: &HashMap<ValueId, Type>,
    consuming: &ConsumingMap,
    entry: &HashSet<ValueId>,
) -> HashSet<ValueId> {
    let block = &func.blocks[bi];
    let mut held = entry.clone();
    for inst in &block.insts {
        match &inst.kind {
            InstKind::Drop(v, _) => {
                held.remove(v);
            }
            InstKind::DropMany(items) => {
                for (v, _) in items {
                    held.remove(v);
                }
            }
            kind => {
                for op in strict_transfers(kind, consuming) {
                    held.remove(&op);
                }
            }
        }
        if let Some(d) = inst.dest
            && owning.contains_key(&d)
        {
            held.insert(d);
        }
    }
    held
}

fn must_held_entry_states(
    func: &mir::Function,
    owning: &HashMap<ValueId, Type>,
    consuming: &ConsumingMap,
) -> Vec<Option<HashSet<ValueId>>> {
    let index_of: HashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    let n = func.blocks.len();
    let entry_bi = index_of.get(&func.entry).copied().unwrap_or(0);
    let mut reachable = HashSet::new();
    let mut stack = vec![entry_bi];
    while let Some(bi) = stack.pop() {
        if reachable.insert(bi) {
            stack.extend(
                func.blocks[bi]
                    .terminator
                    .successors()
                    .iter()
                    .filter_map(|s| index_of.get(s).copied()),
            );
        }
    }
    let all: HashSet<ValueId> = owning.keys().copied().collect();
    let initial: HashSet<ValueId> = func
        .params
        .iter()
        .filter(|p| p.owned && owning.contains_key(&p.value))
        .map(|p| p.value)
        .collect();
    let mut entry_state: Vec<Option<HashSet<ValueId>>> = vec![None; n];
    for &bi in &reachable {
        entry_state[bi] = Some(if bi == entry_bi {
            initial.clone()
        } else {
            all.clone()
        });
    }
    let predecessors = func.predecessors();
    loop {
        let mut changed = false;
        let exit_state: Vec<Option<HashSet<ValueId>>> = entry_state
            .iter()
            .enumerate()
            .map(|(bi, entry)| {
                entry
                    .as_ref()
                    .map(|entry| process_must_held(func, bi, owning, consuming, entry))
            })
            .collect();
        for bi in 0..n {
            if bi == entry_bi || !reachable.contains(&bi) {
                continue;
            }
            let mut merged: Option<HashSet<ValueId>> = None;
            for pred in &predecessors[&func.blocks[bi].id] {
                let pi = index_of[pred];
                let Some(exit) = &exit_state[pi] else {
                    continue;
                };
                let mut flow = exit.clone();
                let mut destinations = Vec::new();
                for phi in &func.blocks[bi].phis {
                    if phi
                        .incoming
                        .iter()
                        .any(|(p, v)| p == pred && exit.contains(v))
                    {
                        destinations.push(phi.dest);
                    }
                    for (_, v) in phi.incoming.iter().filter(|(p, _)| p == pred) {
                        flow.remove(v);
                    }
                }
                flow.extend(destinations);
                merged = Some(match merged {
                    None => flow,
                    Some(cur) => cur.intersection(&flow).copied().collect(),
                });
            }
            if entry_state[bi] != merged {
                entry_state[bi] = merged;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    entry_state
}

pub(super) fn must_held_at_returns(
    func: &mir::Function,
    consuming: &ConsumingMap,
    resources: &HashSet<Symbol>,
    heap_structs: &HashSet<Symbol>,
) -> Vec<(usize, Vec<(ValueId, Type)>)> {
    let owning = owning_allocs(func, consuming, resources, heap_structs);
    if owning.is_empty() {
        return Vec::new();
    }
    let entry_state = must_held_entry_states(func, &owning, consuming);

    let mut out = Vec::new();
    for (bi, entry) in entry_state.iter().enumerate() {
        let Some(entry) = entry else { continue };
        let Terminator::Return(ret) = &func.blocks[bi].terminator else {
            continue;
        };
        let mut held = process_must_held(func, bi, &owning, consuming, entry);
        if let Some(v) = ret {
            held.remove(v);
        }
        if held.is_empty() {
            continue;
        }
        let mut items: Vec<(ValueId, Type)> = held
            .into_iter()
            .map(|v| (v, owning.get(&v).cloned().unwrap_or(Type::Void)))
            .collect();
        items.sort_by_key(|(v, _)| v.0);
        out.push((bi, items));
    }
    out
}

pub(super) fn insert_edge_escape_drops(
    func: &mut mir::Function,
    consuming: &ConsumingMap,
    resources: &HashSet<Symbol>,
    heap_structs: &HashSet<Symbol>,
) -> u32 {
    let owning = owning_allocs(func, consuming, resources, heap_structs);
    if owning.is_empty() {
        return 0;
    }
    let mut inserted = 0;
    loop {
        let n = func.blocks.len();
        let index_of: HashMap<BlockId, usize> = func
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id, i))
            .collect();
        let mut uses = vec![HashSet::new(); n];
        let mut defs = vec![HashSet::new(); n];
        for (bi, block) in func.blocks.iter().enumerate() {
            defs[bi].extend(block.phis.iter().map(|p| p.dest));
            for inst in &block.insts {
                for v in inst_operands(&inst.kind) {
                    if !defs[bi].contains(&v) {
                        uses[bi].insert(v);
                    }
                }
                defs[bi].extend(inst.dest);
            }
            uses[bi].extend(
                terminator_operands(&block.terminator)
                    .into_iter()
                    .filter(|v| !defs[bi].contains(v)),
            );
        }
        let mut live_in = uses.clone();
        loop {
            let mut changed = false;
            for bi in (0..n).rev() {
                let mut live = uses[bi].clone();
                for succ in func.blocks[bi].terminator.successors() {
                    let si = index_of[&succ];
                    live.extend(
                        live_in[si]
                            .iter()
                            .filter(|v| !defs[bi].contains(v))
                            .copied(),
                    );
                    for phi in &func.blocks[si].phis {
                        live.extend(
                            phi.incoming
                                .iter()
                                .filter(|(p, v)| *p == func.blocks[bi].id && !defs[bi].contains(v))
                                .map(|(_, v)| *v),
                        );
                    }
                }
                if live != live_in[bi] {
                    live_in[bi] = live;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let entry_states = must_held_entry_states(func, &owning, consuming);
        let mut candidates = Vec::new();
        for (bi, entry) in entry_states.iter().enumerate() {
            let Some(entry) = entry else { continue };
            let held = process_must_held(func, bi, &owning, consuming, entry);
            let mut successors = func.blocks[bi].terminator.successors();
            successors.sort_by_key(|b| b.0);
            successors.dedup();
            for succ in successors {
                let si = index_of[&succ];
                let mut used = live_in[si].clone();
                used.extend(entry_states[si].iter().flatten().copied());
                for phi in &func.blocks[si].phis {
                    used.extend(
                        phi.incoming
                            .iter()
                            .filter(|(p, _)| *p == func.blocks[bi].id)
                            .map(|(_, v)| *v),
                    );
                }
                let mut dead: Vec<ValueId> = held.difference(&used).copied().collect();
                dead.sort_by_key(|v| v.0);
                if !dead.is_empty() {
                    candidates.push((bi, succ, dead));
                }
            }
            if !candidates.is_empty() {
                break;
            }
        }
        if candidates.is_empty() {
            break;
        }
        for (bi, succ, dead) in candidates {
            let pred = func.blocks[bi].id;
            let span = func.blocks[bi]
                .insts
                .last()
                .map(|i| i.span)
                .unwrap_or(func.span);
            let target = if matches!(func.blocks[bi].terminator, Terminator::Goto(_)) {
                pred
            } else {
                let edge = func.new_block("drop.edge");
                func.blocks[bi].terminator.replace_successor(succ, edge);
                for phi in &mut func.block_mut(succ).phis {
                    for (p, _) in &mut phi.incoming {
                        if *p == pred {
                            *p = edge;
                        }
                    }
                }
                func.block_mut(edge).terminator = Terminator::Goto(succ);
                edge
            };
            for v in dead {
                func.block_mut(target).insts.push(Instruction {
                    dest: None,
                    kind: InstKind::Drop(v, owning[&v].clone()),
                    ty: Type::Void,
                    span,
                    def_id: None,
                });
                inserted += 1;
            }
        }
    }
    inserted
}

pub(super) fn verify_function_leaks(
    func: &mir::Function,
    consuming: &ConsumingMap,
    resources: &HashSet<Symbol>,
    heap_structs: &HashSet<Symbol>,
) -> Vec<String> {
    must_held_at_returns(func, consuming, resources, heap_structs)
        .into_iter()
        .flat_map(|(bi, items)| {
            let name = func.name;
            items.into_iter().map(move |(v, _)| {
                format!(
                    "[fn {name}] v{} is allocated on every path reaching the return in \
                     block {bi} but is neither dropped nor moved before it — it leaks",
                    v.0
                )
            })
        })
        .collect()
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
    fn phi_normalization_removes_dead_cycles_and_coalesces_live_owners() {
        let mut f = func(vec![
            block(
                0,
                vec![inst(Some(0), InstKind::BoolConst(true), Type::Bool)],
                Terminator::Branch(ValueId(0), BlockId(1), BlockId(2)),
            ),
            block(
                1,
                vec![inst(
                    Some(1),
                    InstKind::VecNew(vec![]),
                    Type::Vec(Box::new(Type::I64)),
                )],
                Terminator::Goto(BlockId(3)),
            ),
            block(
                2,
                vec![inst(
                    Some(2),
                    InstKind::VecNew(vec![]),
                    Type::Vec(Box::new(Type::I64)),
                )],
                Terminator::Goto(BlockId(3)),
            ),
            block(
                3,
                vec![inst(
                    None,
                    InstKind::Call(Symbol::intern("borrow"), vec![ValueId(5), ValueId(6)]),
                    Type::Void,
                )],
                Terminator::Return(None),
            ),
        ]);
        for dest in [5, 6] {
            f.blocks[3].phis.push(Phi {
                dest: ValueId(dest),
                ty: Type::Vec(Box::new(Type::I64)),
                incoming: vec![(BlockId(1), ValueId(1)), (BlockId(2), ValueId(2))],
            });
        }
        for (dest, input) in [(7, 8), (8, 7)] {
            f.blocks[3].phis.push(Phi {
                dest: ValueId(dest),
                ty: Type::I64,
                incoming: vec![(BlockId(1), ValueId(input)), (BlockId(2), ValueId(input))],
            });
        }
        while crate::mir::opt::simplify_phis(&mut f) {}
        assert_eq!(f.blocks[3].phis.len(), 1);
        assert_eq!(f.blocks[3].phis[0].dest, ValueId(5));
        assert!(
            matches!(&f.blocks[3].insts[0].kind, InstKind::Call(_, args) if args == &[ValueId(5), ValueId(5)])
        );
    }

    #[test]
    fn mixed_borrowed_owned_phi_gets_an_owned_input_on_only_the_borrowed_edge() {
        let mut f = func(vec![
            block(
                0,
                vec![inst(Some(1), InstKind::BoolConst(true), Type::Bool)],
                Terminator::Branch(ValueId(1), BlockId(1), BlockId(3)),
            ),
            block(
                1,
                vec![inst(
                    Some(2),
                    InstKind::Clone(ValueId(0), Type::String),
                    Type::String,
                )],
                Terminator::Goto(BlockId(3)),
            ),
            block(
                3,
                vec![inst(
                    Some(4),
                    InstKind::FieldGet(ValueId(3), Symbol::intern("length")),
                    Type::I64,
                )],
                Terminator::Return(None),
            ),
        ]);
        f.params.push(crate::mir::Param {
            value: ValueId(0),
            name: Symbol::intern("borrowed"),
            ty: Type::String,
            by_ref: false,
            owned: false,
        });
        f.blocks[2].phis.push(Phi {
            dest: ValueId(3),
            ty: Type::String,
            incoming: vec![(BlockId(0), ValueId(0)), (BlockId(1), ValueId(2))],
        });
        let mut program = mir::Program {
            functions: vec![f],
            types: vec![],
            externs: vec![],
            globals: vec![],
            pkg_id: None,
            item_pkgs: HashMap::new(),
        };
        super::super::mir_drops::run(&mut program, &ConsumingMap::new()).unwrap();
        let f = &program.functions[0];
        let edge = f
            .blocks
            .iter()
            .find(|b| b.label.as_str().starts_with("own.phi.edge"))
            .unwrap();
        assert!(
            edge.insts
                .iter()
                .any(|i| matches!(i.kind, InstKind::Clone(ValueId(0), Type::String)))
        );
        assert!(
            f.blocks[2]
                .insts
                .iter()
                .any(|i| matches!(i.kind, InstKind::Drop(ValueId(3), Type::String)))
        );
        assert!(
            !f.blocks
                .iter()
                .flat_map(|b| &b.insts)
                .any(|i| matches!(i.kind, InstKind::Drop(ValueId(0), _)))
        );
        assert!(verify_function(f).is_empty());
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
    fn early_return_leak_is_reported() {
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
            block(1, vec![], Terminator::Return(None)),
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
        let errs = verify_function_leaks(
            &f,
            &ConsumingMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        assert!(errs.iter().any(|e| e.contains("it leaks")), "{errs:?}");
    }

    #[test]
    fn unknown_call_borrows_unless_ownership_is_declared() {
        let vty = Type::Vec(Box::new(Type::I64));
        let f = func(vec![block(
            0,
            vec![
                inst(Some(1), InstKind::VecNew(vec![]), vty.clone()),
                inst(
                    Some(2),
                    InstKind::Call(Symbol::intern("sink"), vec![ValueId(1)]),
                    Type::Void,
                ),
            ],
            Terminator::Return(None),
        )]);
        let resources = HashSet::new();
        let heap_structs = HashSet::new();
        let mut consuming = ConsumingMap::new();
        let errors = verify_function_leaks(&f, &consuming, &resources, &heap_structs);
        assert!(errors.iter().any(|e| e.contains("it leaks")), "{errors:?}");
        consuming.insert(Symbol::intern("sink"), vec![true]);
        assert!(verify_function_leaks(&f, &consuming, &resources, &heap_structs).is_empty());
    }

    #[test]
    fn borrowing_known_call_does_not_transfer() {
        let vty = Type::Vec(Box::new(Type::I64));
        let f = func(vec![block(
            0,
            vec![
                inst(Some(1), InstKind::VecNew(vec![]), vty.clone()),
                inst(
                    Some(2),
                    InstKind::Call(Symbol::intern("peek"), vec![ValueId(1)]),
                    Type::Void,
                ),
            ],
            Terminator::Return(None),
        )]);
        let mut consuming = ConsumingMap::new();
        consuming.insert(Symbol::intern("peek"), vec![false]);
        let errs = verify_function_leaks(
            &f,
            &consuming,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        assert!(errs.iter().any(|e| e.contains("it leaks")), "{errs:?}");
        let fixed = {
            let mut f2 = f;
            let n = super::super::mir_drops::insert_missing_return_drops(
                &mut f2,
                &consuming,
                &std::collections::HashSet::new(),
                &std::collections::HashSet::new(),
            );
            assert_eq!(n, 1);
            verify_function_leaks(
                &f2,
                &consuming,
                &std::collections::HashSet::new(),
                &std::collections::HashSet::new(),
            )
        };
        assert!(fixed.is_empty(), "{fixed:?}");
    }

    #[test]
    fn conditional_transfer_drops_only_the_nonconsuming_edge() {
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
                    Some(3),
                    InstKind::Call(Symbol::intern("eat"), vec![ValueId(1)]),
                    Type::Void,
                )],
                Terminator::Goto(BlockId(2)),
            ),
            block(2, vec![], Terminator::Return(None)),
        ]);
        let mut f2 = f;
        let mut consuming = ConsumingMap::new();
        consuming.insert(Symbol::intern("eat"), vec![true]);
        let edges = insert_edge_escape_drops(&mut f2, &consuming, &HashSet::new(), &HashSet::new());
        assert_eq!(edges, 1);
        let edge = f2.blocks.iter().find(|b| b.id.0 >= 100).unwrap();
        assert!(matches!(edge.insts[0].kind, InstKind::Drop(ValueId(1), _)));
        assert!(matches!(edge.terminator, Terminator::Goto(BlockId(2))));
        assert_eq!(f2.blocks[1].insts.len(), 1);
        assert!(f2.blocks[2].insts.is_empty());
        assert!(verify_function(&f2).is_empty());
        let n = super::super::mir_drops::insert_missing_return_drops(
            &mut f2,
            &consuming,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        assert_eq!(
            n, 0,
            "conditionally-moved value must not get an unconditional drop"
        );
        assert!(
            verify_function_leaks(
                &f2,
                &consuming,
                &std::collections::HashSet::new(),
                &std::collections::HashSet::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn dropped_on_every_path_is_clean() {
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
                Terminator::Return(None),
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
        assert!(
            verify_function_leaks(
                &f,
                &ConsumingMap::new(),
                &std::collections::HashSet::new(),
                &std::collections::HashSet::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn read_only_uses_do_not_discharge() {
        let vty = Type::Vec(Box::new(Type::I64));
        let f = func(vec![block(
            0,
            vec![
                inst(Some(1), InstKind::VecNew(vec![]), vty.clone()),
                inst(Some(2), InstKind::VecLen(ValueId(1)), Type::I64),
            ],
            Terminator::Return(None),
        )]);
        let errs = verify_function_leaks(
            &f,
            &ConsumingMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        assert!(errs.iter().any(|e| e.contains("it leaks")), "{errs:?}");
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
