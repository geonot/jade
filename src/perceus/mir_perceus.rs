use std::collections::{HashMap, HashSet};

use crate::ast::Span;
use crate::mir::{self, InstKind, Instruction, Terminator, ValueId};
use crate::types::Type;

use super::PerceusHints;

pub fn analyze_mir_program(prog: &mut mir::Program) -> PerceusHints {
    run(prog)
}

pub fn run(prog: &mut mir::Program) -> PerceusHints {
    let mut hints = PerceusHints::default();
    let mut next_slot: u32 = 0;
    for func in &mut prog.functions {
        run_on_function(func, &mut hints, &mut next_slot);
    }
    hints
}

fn run_on_function(func: &mut mir::Function, hints: &mut PerceusHints, next_slot: &mut u32) {
    let uses = count_uses(func);
    hints.stats.total_bindings_analyzed += uses.len() as u32;

    let sunk = drop_sinking(func, &uses);
    hints.stats.last_use_tracked += sunk;

    drop_elision(func, hints);

    vec_reuse_pairing(func, hints, next_slot);
    drop_fusion(func, hints);
}

#[derive(Debug, Clone)]
struct UseInfo {
    use_count: u32,
    escapes: bool,
    captured: bool,

    last_use: Option<(usize, usize)>,
}

fn count_uses(func: &mir::Function) -> HashMap<ValueId, UseInfo> {
    let mut uses: HashMap<ValueId, UseInfo> = HashMap::new();

    for p in &func.params {
        uses.insert(
            p.value,
            UseInfo {
                use_count: 0,
                escapes: false,
                captured: false,
                last_use: None,
            },
        );
    }
    for bb in &func.blocks {
        for phi in &bb.phis {
            uses.entry(phi.dest).or_insert_with(|| UseInfo {
                use_count: 0,
                escapes: false,
                captured: false,
                last_use: None,
            });
        }
        for inst in &bb.insts {
            if let Some(d) = inst.dest {
                uses.entry(d).or_insert_with(|| UseInfo {
                    use_count: 0,
                    escapes: false,
                    captured: false,
                    last_use: None,
                });
            }
        }
    }

    for (bi, bb) in func.blocks.iter().enumerate() {
        for phi in &bb.phis {
            for (_, vid) in &phi.incoming {
                if let Some(info) = uses.get_mut(vid) {
                    info.use_count += 1;
                }
            }
        }
        for (ii, inst) in bb.insts.iter().enumerate() {
            match &inst.kind {
                InstKind::Drop(_, _) | InstKind::DropMany(_) => {}
                _ => {
                    for op in inst_operands(&inst.kind) {
                        if let Some(info) = uses.get_mut(&op) {
                            info.use_count += 1;
                            info.last_use = Some((bi, ii));
                        }
                    }
                }
            }

            match &inst.kind {
                InstKind::Call(_, args)
                | InstKind::IndirectCall(_, args)
                | InstKind::ClosureCall(_, args) => {
                    for a in args {
                        if let Some(info) = uses.get_mut(a) {
                            info.escapes = true;
                        }
                    }
                }
                InstKind::SpawnActor(_, inits) => {
                    for (_, a) in inits {
                        if let Some(info) = uses.get_mut(a) {
                            info.escapes = true;
                        }
                    }
                }
                InstKind::MethodCall(recv, _, args, _) => {
                    if let Some(info) = uses.get_mut(recv) {
                        info.escapes = true;
                    }
                    for a in args {
                        if let Some(info) = uses.get_mut(a) {
                            info.escapes = true;
                        }
                    }
                }
                InstKind::ClosureCreate(_, captures) => {
                    for c in captures {
                        if let Some(info) = uses.get_mut(c) {
                            info.escapes = true;
                            info.captured = true;
                        }
                    }
                }
                InstKind::ChanSend(_, val) => {
                    if let Some(info) = uses.get_mut(val) {
                        info.escapes = true;
                    }
                }
                InstKind::Store(_, val) | InstKind::GlobalStore(_, val) => {
                    if let Some(info) = uses.get_mut(val) {
                        info.escapes = true;
                    }
                }
                InstKind::FieldStore(_, _, val) | InstKind::IndexStore(_, _, val) => {
                    if let Some(info) = uses.get_mut(val) {
                        info.escapes = true;
                    }
                }
                InstKind::FieldSet(_, _, val) | InstKind::IndexSet(_, _, val) => {
                    if let Some(info) = uses.get_mut(val) {
                        info.escapes = true;
                    }
                }
                InstKind::StructInit(_, fields) => {
                    for (_, v) in fields {
                        if let Some(info) = uses.get_mut(v) {
                            info.escapes = true;
                        }
                    }
                }
                InstKind::VariantInit(_, _, _, payload) => {
                    for v in payload {
                        if let Some(info) = uses.get_mut(v) {
                            info.escapes = true;
                        }
                    }
                }
                InstKind::ArrayInit(elems) | InstKind::VecNew(elems) => {
                    for v in elems {
                        if let Some(info) = uses.get_mut(v) {
                            info.escapes = true;
                        }
                    }
                }
                InstKind::VecPush(_, elem) => {
                    if let Some(info) = uses.get_mut(elem) {
                        info.escapes = true;
                    }
                }
                _ => {}
            }
        }

        let term_idx = bb.insts.len();
        match &bb.terminator {
            Terminator::Branch(c, _, _) => {
                if let Some(info) = uses.get_mut(c) {
                    info.use_count += 1;
                    info.last_use = Some((bi, term_idx));
                }
            }
            Terminator::Switch(d, _, _) => {
                if let Some(info) = uses.get_mut(d) {
                    info.use_count += 1;
                    info.last_use = Some((bi, term_idx));
                }
            }
            Terminator::Return(Some(v)) => {
                if let Some(info) = uses.get_mut(v) {
                    info.use_count += 1;
                    info.last_use = Some((bi, term_idx));
                    info.escapes = true;
                }
            }
            _ => {}
        }
    }

    uses
}

fn inst_operands(kind: &InstKind) -> Vec<ValueId> {
    match kind {
        InstKind::IntConst(_)
        | InstKind::FloatConst(_)
        | InstKind::BoolConst(_)
        | InstKind::StringConst(_)
        | InstKind::Void
        | InstKind::MapInit
        | InstKind::Load(_)
        | InstKind::FnRef(_)
        | InstKind::GlobalLoad(_) => vec![],

        InstKind::BinOp(_, a, b) | InstKind::Cmp(_, a, b, _) => vec![*a, *b],
        InstKind::UnaryOp(_, a) => vec![*a],

        InstKind::Call(_, args) => args.clone(),
        InstKind::MethodCall(recv, _, args, _) => {
            let mut v = vec![*recv];
            v.extend(args);
            v
        }
        InstKind::IndirectCall(callee, args) => {
            let mut v = vec![*callee];
            v.extend(args);
            v
        }

        InstKind::Store(_, val) | InstKind::GlobalStore(_, val) => vec![*val],
        InstKind::FieldGet(obj, _) => vec![*obj],
        InstKind::FieldSet(obj, _, val) => vec![*obj, *val],
        InstKind::FieldStore(_, _, val) => vec![*val],
        InstKind::FieldTombstone(_, _) => vec![],
        InstKind::Index(base, idx) | InstKind::IndexUnchecked(base, idx) => vec![*base, *idx],
        InstKind::IndexSet(base, idx, val) => vec![*base, *idx, *val],
        InstKind::IndexStore(_, idx, val) => vec![*idx, *val],

        InstKind::StructInit(_, fields) => fields.iter().map(|(_, v)| *v).collect(),
        InstKind::VariantInit(_, _, _, payload) => payload.clone(),
        InstKind::ArrayInit(elems) => elems.clone(),

        InstKind::Cast(v, _) | InstKind::StrictCast(v, _) => vec![*v],
        InstKind::Ref(v) | InstKind::Deref(v) => vec![*v],

        InstKind::Alloc(v) => vec![*v],
        InstKind::Drop(v, _) => vec![*v],
        InstKind::DropMany(items) => items.iter().map(|(v, _)| *v).collect(),
        InstKind::Copy(v) => vec![*v],
        InstKind::Clone(v, _) => vec![*v],
        InstKind::Slice(a, b, c) => vec![*a, *b, *c],

        InstKind::VecNew(elems) => elems.clone(),
        InstKind::VecPush(vec, elem) => vec![*vec, *elem],
        InstKind::VecLen(vec) => vec![*vec],

        InstKind::ClosureCreate(_, captures) => captures.clone(),
        InstKind::ClosureCall(callee, args) => {
            let mut v = vec![*callee];
            v.extend(args);
            v
        }

        InstKind::SpawnActor(_, inits) => inits.iter().map(|(_, v)| *v).collect(),
        InstKind::ChanCreate(..) => vec![],
        InstKind::ChanSend(ch, val) => vec![*ch, *val],
        InstKind::ChanRecv(ch) => vec![*ch],
        InstKind::SelectArm(channels, _) => channels.clone(),

        InstKind::Log(v) => vec![*v],
        InstKind::Assert(v, _) => vec![*v],

        InstKind::InlineAsm(_, args) => args.clone(),
    }
}

fn drop_sinking(func: &mut mir::Function, uses: &HashMap<ValueId, UseInfo>) -> u32 {
    let mut sunk = 0u32;
    for bi in 0..func.blocks.len() {
        let orig: Vec<Instruction> = std::mem::take(&mut func.blocks[bi].insts);
        let mut new_insts: Vec<Instruction> = Vec::with_capacity(orig.len());

        let mut deferred: Vec<(usize, Instruction)> = Vec::new();

        for (i, inst) in orig.into_iter().enumerate() {
            let mut deferred_this_drop = false;
            if let InstKind::Drop(v, _) = &inst.kind {
                if let Some(info) = uses.get(v) {
                    if let Some((lu_bi, lu_ii)) = info.last_use {
                        if lu_bi == bi && lu_ii > i && lu_ii < usize::MAX {
                            deferred.push((lu_ii, inst.clone()));
                            deferred_this_drop = true;
                            sunk += 1;
                        }
                    }
                }
            }
            if !deferred_this_drop {
                new_insts.push(inst);
            }

            let mut k = 0;
            while k < deferred.len() {
                if deferred[k].0 == i {
                    new_insts.push(deferred.swap_remove(k).1);
                } else {
                    k += 1;
                }
            }
        }

        for (_, inst) in deferred.drain(..) {
            new_insts.push(inst);
        }
        func.blocks[bi].insts = new_insts;
    }
    sunk
}

fn drop_elision(func: &mut mir::Function, hints: &mut PerceusHints) {
    let mut elided = 0u32;
    for bb in &mut func.blocks {
        bb.insts.retain(|inst| match &inst.kind {
            InstKind::Drop(_, ty) if ty.is_trivially_droppable() => {
                elided += 1;
                false
            }
            _ => true,
        });
    }
    hints.stats.drops_elided += elided;
}

fn collect_loop_bodies(func: &mir::Function) -> Vec<Vec<usize>> {
    let n = func.blocks.len();
    let succs = |b: usize| -> Vec<usize> {
        match &func.blocks[b].terminator {
            Terminator::Goto(t) => vec![t.0 as usize],
            Terminator::Branch(_, t, e) => vec![t.0 as usize, e.0 as usize],
            Terminator::Switch(_, arms, default) => {
                let mut v: Vec<usize> = arms.iter().map(|(_, t)| t.0 as usize).collect();
                v.push(default.0 as usize);
                v
            }
            _ => vec![],
        }
    };

    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for b in 0..n {
        for s in succs(b) {
            if s < n {
                preds[s].push(b);
            }
        }
    }
    let mut bodies: Vec<Vec<usize>> = Vec::new();
    let mut seen: HashSet<Vec<usize>> = HashSet::new();
    for h in 0..n {
        if !block_is_loop_body(func, h) {
            continue;
        }

        let mut fwd = vec![false; n];
        let mut stack = vec![h];
        while let Some(b) = stack.pop() {
            if b >= n || fwd[b] {
                continue;
            }
            fwd[b] = true;
            for s in succs(b) {
                stack.push(s);
            }
        }

        let mut bwd = vec![false; n];
        let mut stack = vec![h];
        while let Some(b) = stack.pop() {
            if bwd[b] {
                continue;
            }
            bwd[b] = true;
            for &p in &preds[b] {
                stack.push(p);
            }
        }
        let mut body: Vec<usize> = (0..n).filter(|&b| fwd[b] && bwd[b]).collect();
        body.sort_unstable();
        if body.len() >= 1 && seen.insert(body.clone()) {
            bodies.push(body);
        }
    }
    bodies
}

fn block_is_loop_body(func: &mir::Function, bi: usize) -> bool {
    let n = func.blocks.len();
    let succs = |b: usize| -> Vec<usize> {
        match &func.blocks[b].terminator {
            Terminator::Goto(t) => vec![t.0 as usize],
            Terminator::Branch(_, t, e) => vec![t.0 as usize, e.0 as usize],
            Terminator::Switch(_, arms, default) => {
                let mut v: Vec<usize> = arms.iter().map(|(_, t)| t.0 as usize).collect();
                v.push(default.0 as usize);
                v
            }
            _ => vec![],
        }
    };
    let mut visited = vec![false; n];
    let mut stack: Vec<usize> = succs(bi);
    while let Some(b) = stack.pop() {
        if b >= n {
            continue;
        }
        if b == bi {
            return true;
        }
        if visited[b] {
            continue;
        }
        visited[b] = true;
        for s in succs(b) {
            stack.push(s);
        }
    }
    false
}

fn vec_reuse_pairing(
    func: &mut mir::Function,
    hints: &mut PerceusHints,
    next_slot: &mut u32,
) {
    let mut pairs = 0u32;

    for bi in 0..func.blocks.len() {
        struct DropSite {
            inst_idx: usize,
            value: ValueId,
            elem_ty: Type,
        }
        struct AllocSite {
            inst_idx: usize,
            dest: ValueId,
            elem_ty: Type,
        }
        let mut drops: Vec<DropSite> = Vec::new();
        let mut allocs: Vec<AllocSite> = Vec::new();
        let mut kill_idxs: Vec<usize> = Vec::new();
        for (ii, inst) in func.blocks[bi].insts.iter().enumerate() {
            match &inst.kind {
                InstKind::Drop(v, Type::Vec(elem)) if !func.perceus.reuse_save.contains_key(v) => {
                    drops.push(DropSite {
                        inst_idx: ii,
                        value: *v,
                        elem_ty: (**elem).clone(),
                    });
                }
                InstKind::VecNew(elems) if elems.is_empty() => {
                    if let (Some(dest), Type::Vec(elem)) = (inst.dest, &inst.ty) {
                        if !func.perceus.reuse_consume.contains_key(&dest) {
                            allocs.push(AllocSite {
                                inst_idx: ii,
                                dest,
                                elem_ty: (**elem).clone(),
                            });
                        }
                    }
                }
                InstKind::Call(_, _)
                | InstKind::IndirectCall(_, _)
                | InstKind::MethodCall(_, _, _, _)
                | InstKind::ClosureCall(_, _)
                | InstKind::ChanSend(_, _)
                | InstKind::SpawnActor(_, _) => {
                    kill_idxs.push(ii);
                }
                _ => {}
            }
        }
        let is_loop = block_is_loop_body(func, bi);
        let mut used_d = vec![false; drops.len()];
        let mut used_a = vec![false; allocs.len()];
        let mut decisions: Vec<(ValueId, ValueId, u32)> = Vec::new();

        for (di, d) in drops.iter().enumerate() {
            if used_d[di] {
                continue;
            }
            for (ai, a) in allocs.iter().enumerate() {
                if used_a[ai] || a.inst_idx <= d.inst_idx {
                    continue;
                }
                if a.elem_ty != d.elem_ty {
                    continue;
                }
                let killed = kill_idxs.iter().any(|&k| k > d.inst_idx && k < a.inst_idx);
                if killed {
                    continue;
                }
                let slot = *next_slot;
                *next_slot += 1;
                decisions.push((d.value, a.dest, slot));
                used_d[di] = true;
                used_a[ai] = true;
                break;
            }
        }

        if is_loop {
            for (di, d) in drops.iter().enumerate() {
                if used_d[di] {
                    continue;
                }
                for (ai, a) in allocs.iter().enumerate() {
                    if used_a[ai] || a.inst_idx >= d.inst_idx {
                        continue;
                    }
                    if a.elem_ty != d.elem_ty {
                        continue;
                    }
                    let killed = kill_idxs
                        .iter()
                        .any(|&k| (k > d.inst_idx) || (k < a.inst_idx));
                    if killed {
                        continue;
                    }
                    let slot = *next_slot;
                    *next_slot += 1;
                    decisions.push((d.value, a.dest, slot));
                    used_d[di] = true;
                    used_a[ai] = true;
                    break;
                }
            }
        }

        for (drop_v, alloc_dest, slot) in decisions {
            func.perceus.reuse_save.insert(drop_v, slot);
            func.perceus.reuse_consume.insert(alloc_dest, slot);
            func.perceus.vec_slots.insert(slot);
            pairs += 1;
        }
    }

    let loops = collect_loop_bodies(func);
    for body in loops {
        let has_escape = body.iter().any(|&bi| {
            func.blocks[bi].insts.iter().any(|inst| {
                matches!(
                    inst.kind,
                    InstKind::Call(_, _)
                        | InstKind::IndirectCall(_, _)
                        | InstKind::MethodCall(_, _, _, _)
                        | InstKind::ClosureCall(_, _)
                        | InstKind::ChanSend(_, _)
                        | InstKind::SpawnActor(_, _)
                )
            })
        });
        if has_escape {
            continue;
        }
        let mut drops: Vec<(ValueId, Type)> = Vec::new();
        let mut allocs: Vec<(ValueId, Type)> = Vec::new();
        for &bi in &body {
            for inst in &func.blocks[bi].insts {
                match &inst.kind {
                    InstKind::Drop(v, Type::Vec(elem))
                        if !func.perceus.reuse_save.contains_key(v) =>
                    {
                        drops.push((*v, (**elem).clone()));
                    }
                    InstKind::VecNew(elems) if elems.is_empty() => {
                        if let (Some(dest), Type::Vec(elem)) = (inst.dest, &inst.ty) {
                            if !func.perceus.reuse_consume.contains_key(&dest) {
                                allocs.push((dest, (**elem).clone()));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut used_a = vec![false; allocs.len()];
        for (dv, dt) in &drops {
            for (ai, (av, at)) in allocs.iter().enumerate() {
                if used_a[ai] || at != dt {
                    continue;
                }
                let slot = *next_slot;
                *next_slot += 1;
                func.perceus.reuse_save.insert(*dv, slot);
                func.perceus.reuse_consume.insert(*av, slot);
                func.perceus.vec_slots.insert(slot);
                used_a[ai] = true;
                pairs += 1;
                break;
            }
        }
    }

    hints.stats.reuse_sites += pairs;
}

fn drop_fusion(func: &mut mir::Function, hints: &mut PerceusHints) {
    let mut fused = 0u32;
    for bb in &mut func.blocks {
        let mut new_insts: Vec<Instruction> = Vec::with_capacity(bb.insts.len());
        let mut run: Vec<(ValueId, Type)> = Vec::new();
        let mut run_span: Option<Span> = None;
        let drained: Vec<Instruction> = std::mem::take(&mut bb.insts);
        for inst in drained {
            let is_fusible = matches!(&inst.kind, InstKind::Drop(v, _)
                if !func.perceus.reuse_save.contains_key(v));
            if is_fusible {
                if let InstKind::Drop(v, ty) = inst.kind {
                    if run.is_empty() {
                        run_span = Some(inst.span);
                    }
                    run.push((v, ty));
                }
            } else {
                flush_run(&mut new_insts, &mut run, &mut run_span, &mut fused);
                new_insts.push(inst);
            }
        }
        flush_run(&mut new_insts, &mut run, &mut run_span, &mut fused);
        bb.insts = new_insts;
    }
    hints.stats.drops_fused += fused;
}

fn flush_run(
    out: &mut Vec<Instruction>,
    run: &mut Vec<(ValueId, Type)>,
    run_span: &mut Option<Span>,
    fused: &mut u32,
) {
    if run.len() >= 2 {
        let count = run.len() as u32;
        let items = std::mem::take(run);
        out.push(Instruction {
            dest: None,
            kind: InstKind::DropMany(items),
            ty: Type::Void,
            span: run_span.take().unwrap_or(Span::dummy()),
            def_id: None,
        });
        *fused += count;
    } else if let Some((v, ty)) = run.pop() {
        out.push(Instruction {
            dest: None,
            kind: InstKind::Drop(v, ty),
            ty: Type::Void,
            span: run_span.take().unwrap_or(Span::dummy()),
            def_id: None,
        });
    }
}
