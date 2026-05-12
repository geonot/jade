//! Perceus reference-counting optimization on MIR.
//!
//! This module implements Perceus as a sequence of **transformation passes**
//! over MIR (not advisory hints). Each pass mutates the program directly and
//! updates `PerceusStats` for `--debug-perceus` reporting. Codegen consumes
//! the post-transform IR plus the per-function `mir::PerceusMeta` side-table
//! for slot-keyed reuse.
//!
//! Pass pipeline (per function, in order):
//!   1. **count_uses**     — exact SSA use-counting (`Drop` / `RcInc` /
//!                            `RcDec` are *not* counted as semantic uses).
//!   2. **drop_sinking**   — move each `Drop` to immediately after the last
//!                            semantic use of its operand within the same
//!                            basic block.
//!   3. **drop_elision**   — physically delete `Drop(v, ty)` when
//!                            `ty.is_trivially_droppable()`.
//!   4. **borrow_promote** — delete `RcInc`/`RcDec` for `Type::Rc(_)`
//!                            values with at most one semantic use, no
//!                            escape, no closure capture.
//!   5. **reuse_pairing**  — pair `Drop(v_old)` with the next layout-
//!                            compatible `RcNew` in the same block via a
//!                            slot id stored in `func.perceus`.
//!   6. **drop_fusion**    — coalesce runs of >=2 consecutive `Drop`s into
//!                            a single `InstKind::DropMany`.
//!
//! All passes preserve SSA: drop sinking only moves drops backward inside a
//! single block; drop elision and fusion only delete or coalesce Drop
//! instructions, which have no SSA dest.

use std::collections::{HashMap, HashSet};

use crate::ast::Span;
use crate::mir::{self, InstKind, Instruction, Terminator, ValueId};
use crate::types::Type;

use super::{PerceusHints, PerceusPass};

// ── Public entry points ──────────────────────────────────────────

/// Backward-compatible entry. Now takes `&mut` because Perceus rewrites the
/// IR in place; the old `&` shape leaked an immutable borrow that callers
/// have all been migrated away from.
pub fn analyze_mir_program(prog: &mut mir::Program) -> PerceusHints {
    run(prog)
}

/// Run the Perceus transform pipeline. Returns aggregate stats; per-function
/// reuse metadata is stored on `Function::perceus`.
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

    let uses = count_uses(func);
    borrow_promote(func, &uses, hints);
    reuse_pairing(func, &uses, hints, next_slot);
    vec_reuse_pairing(func, &uses, hints, next_slot);
    drop_fusion(func, hints);
}

// ── Use info ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct UseInfo {
    use_count: u32,
    escapes: bool,
    captured: bool,
    ty: Type,
    /// (block_index, instruction_index) of the last *semantic* use.
    /// `instruction_index == bb.insts.len()` means "the terminator".
    last_use: Option<(usize, usize)>,
}

fn count_uses(func: &mir::Function) -> HashMap<ValueId, UseInfo> {
    let mut uses: HashMap<ValueId, UseInfo> = HashMap::new();

    // Pass 1: register definitions (params, phi dests, instruction dests).
    for p in &func.params {
        uses.insert(
            p.value,
            UseInfo {
                use_count: 0,
                escapes: false,
                captured: false,
                ty: p.ty.clone(),
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
                ty: phi.ty.clone(),
                last_use: None,
            });
        }
        for inst in &bb.insts {
            if let Some(d) = inst.dest {
                uses.entry(d).or_insert_with(|| UseInfo {
                    use_count: 0,
                    escapes: false,
                    captured: false,
                    ty: inst.ty.clone(),
                    last_use: None,
                });
            }
        }
    }

    // Pass 2: count semantic uses.
    for (bi, bb) in func.blocks.iter().enumerate() {
        for phi in &bb.phis {
            for (_, vid) in &phi.incoming {
                if let Some(info) = uses.get_mut(vid) {
                    info.use_count += 1;
                }
            }
        }
        for (ii, inst) in bb.insts.iter().enumerate() {
            // Refcount/drop ops are NOT semantic uses. Their operands are
            // tracked by the dedicated reuse / fusion passes.
            match &inst.kind {
                InstKind::Drop(_, _)
                | InstKind::DropMany(_)
                | InstKind::RcInc(_)
                | InstKind::RcDec(_) => {}
                _ => {
                    for op in inst_operands(&inst.kind) {
                        if let Some(info) = uses.get_mut(&op) {
                            info.use_count += 1;
                            info.last_use = Some((bi, ii));
                        }
                    }
                }
            }

            // Escape tracking — a value escapes when we cannot prove the
            // callee/peer/storage cannot stash it.
            match &inst.kind {
                InstKind::Call(_, args)
                | InstKind::IndirectCall(_, args)
                | InstKind::ClosureCall(_, args)
                | InstKind::SpawnActor(_, args) => {
                    for a in args {
                        if let Some(info) = uses.get_mut(a) {
                            info.escapes = true;
                        }
                    }
                }
                InstKind::MethodCall(recv, _, args) => {
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
                InstKind::RcNew(val, _) => {
                    if let Some(info) = uses.get_mut(val) {
                        info.escapes = true;
                    }
                }
                _ => {}
            }
        }
        // Terminator operand contributions; they are the last "instruction"
        // of the block, indexed at bb.insts.len().
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
        | InstKind::SetInit
        | InstKind::PQInit
        | InstKind::DequeInit
        | InstKind::Load(_)
        | InstKind::FnRef(_)
        | InstKind::GlobalLoad(_) => vec![],

        InstKind::BinOp(_, a, b) | InstKind::Cmp(_, a, b, _) => vec![*a, *b],
        InstKind::UnaryOp(_, a) => vec![*a],

        InstKind::Call(_, args) => args.clone(),
        InstKind::MethodCall(recv, _, args) => {
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
        InstKind::RcInc(v) | InstKind::RcDec(v) => vec![*v],
        InstKind::Copy(v) => vec![*v],
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

        InstKind::RcNew(val, _) => vec![*val],
        InstKind::RcClone(v) | InstKind::WeakUpgrade(v) => vec![*v],

        InstKind::SpawnActor(_, args) => args.clone(),
        InstKind::ChanCreate(..) => vec![],
        InstKind::ChanSend(ch, val) => vec![*ch, *val],
        InstKind::ChanRecv(ch) => vec![*ch],
        InstKind::SelectArm(channels, _) => channels.clone(),

        InstKind::Log(v) => vec![*v],
        InstKind::Assert(v, _) => vec![*v],

        InstKind::DynDispatch(obj, _, _, args) => {
            let mut v = vec![*obj];
            v.extend(args);
            v
        }
        InstKind::DynCoerce(v, _, _) => vec![*v],

        InstKind::InlineAsm(_, args) => args.clone(),
    }
}

// ── Pass: drop sinking ───────────────────────────────────────────

/// Move each `Drop(v)` instruction backward to immediately after the last
/// semantic use of `v` within the same basic block. If the last use is in a
/// different block (or the value has no semantic use), the drop is left in
/// place. Returns the number of drops actually relocated.
///
/// Soundness: we never move a drop *across* a use of the same value (the
/// destination is `last_use_idx + 1`, after every use). We never move a drop
/// out of its block. We never reorder relative to other drops of the same
/// value (each drop is processed independently against its own last-use).
fn drop_sinking(func: &mut mir::Function, uses: &HashMap<ValueId, UseInfo>) -> u32 {
    let mut sunk = 0u32;
    for bi in 0..func.blocks.len() {
        let orig: Vec<Instruction> = std::mem::take(&mut func.blocks[bi].insts);
        let mut new_insts: Vec<Instruction> = Vec::with_capacity(orig.len());
        // (anchor_orig_idx, drop_inst). When we have just appended the
        // instruction that lived at `anchor_orig_idx` in `orig`, we flush all
        // deferred drops with that anchor.
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
            // Flush any deferred drops anchored at i.
            let mut k = 0;
            while k < deferred.len() {
                if deferred[k].0 == i {
                    new_insts.push(deferred.swap_remove(k).1);
                } else {
                    k += 1;
                }
            }
        }
        // Drops anchored beyond the last instruction (e.g. last use was the
        // terminator) get appended at the very end of the block, just before
        // the terminator runs.
        for (_, inst) in deferred.drain(..) {
            new_insts.push(inst);
        }
        func.blocks[bi].insts = new_insts;
    }
    sunk
}

// ── Pass: drop elision ───────────────────────────────────────────

/// Physically delete `Drop(v, ty)` whenever `ty.is_trivially_droppable()`.
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

// ── Pass: borrow promotion ───────────────────────────────────────

/// Delete `RcInc(v)` / `RcDec(v)` for `Type::Rc(_)` values with at most one
/// semantic use, no escape, and no closure capture.
fn borrow_promote(
    func: &mut mir::Function,
    uses: &HashMap<ValueId, UseInfo>,
    hints: &mut PerceusHints,
) {
    let mut promotable: HashSet<ValueId> = HashSet::new();
    for (vid, info) in uses {
        if matches!(info.ty, Type::Rc(_))
            && info.use_count <= 1
            && !info.escapes
            && !info.captured
        {
            promotable.insert(*vid);
        }
    }
    if promotable.is_empty() {
        return;
    }
    let mut promoted = 0u32;
    for bb in &mut func.blocks {
        bb.insts.retain(|inst| match &inst.kind {
            InstKind::RcInc(v) | InstKind::RcDec(v) if promotable.contains(v) => {
                promoted += 1;
                false
            }
            _ => true,
        });
    }
    hints.stats.borrows_promoted += promoted;
}

// ── Pass: reuse pairing ──────────────────────────────────────────

/// Pair `Drop(v_old)` with the next layout-compatible `RcNew` in the same
/// block, recording the pairing as a slot id in `func.perceus`. Codegen turns
/// the pair into a literal in-place reuse: the Drop stashes the heap pointer
/// instead of freeing it; the matching alloc consumes the slot instead of
/// calling malloc.
///
/// Soundness restrictions:
///   * `v_old` must be `Type::Rc(_)`, `use_count == 1`, `!escapes`,
///     `!captured`.
///   * The matching alloc must be `RcNew(_, inner)` whose inner type has the
///     same layout size as `v_old`'s inner type.
///   * Any intervening Call / IndirectCall / MethodCall / ClosureCall /
///     ChanSend / SpawnActor kills the candidate (the pointer might escape
///     and be freed underneath us).
fn reuse_pairing(
    func: &mut mir::Function,
    uses: &HashMap<ValueId, UseInfo>,
    hints: &mut PerceusHints,
    next_slot: &mut u32,
) {
    let mut pairs = 0u32;
    for bi in 0..func.blocks.len() {
        // Pass A: collect Drop and RcNew positions in this block, plus
        // positions of escape-killing operations.
        #[derive(Debug)]
        struct DropSite {
            inst_idx: usize,
            value: ValueId,
            size: u64,
        }
        #[derive(Debug)]
        struct AllocSite {
            inst_idx: usize,
            dest: ValueId,
            size: u64,
        }
        let mut drops: Vec<DropSite> = Vec::new();
        let mut allocs: Vec<AllocSite> = Vec::new();
        let mut kill_idxs: Vec<usize> = Vec::new();
        for (ii, inst) in func.blocks[bi].insts.iter().enumerate() {
            match &inst.kind {
                InstKind::Drop(v, Type::Rc(inner)) => {
                    if let Some(info) = uses.get(v) {
                        if info.use_count == 1 && !info.escapes && !info.captured {
                            let size = PerceusPass::type_layout_size_pub(inner);
                            if size > 0 {
                                drops.push(DropSite { inst_idx: ii, value: *v, size });
                            }
                        }
                    }
                }
                InstKind::RcNew(_, alloc_inner) => {
                    if let Some(dest) = inst.dest {
                        let size = PerceusPass::type_layout_size_pub(alloc_inner);
                        if size > 0 {
                            allocs.push(AllocSite { inst_idx: ii, dest, size });
                        }
                    }
                }
                InstKind::Call(_, _)
                | InstKind::IndirectCall(_, _)
                | InstKind::MethodCall(_, _, _)
                | InstKind::ClosureCall(_, _)
                | InstKind::ChanSend(_, _)
                | InstKind::SpawnActor(_, _) => {
                    kill_idxs.push(ii);
                }
                _ => {}
            }
        }

        // Detect whether this block is on a back-edge (it is itself the
        // target of a branch/goto from a block that comes later in the
        // function CFG, OR it branches to itself directly). When the block
        // is a loop body, end-of-block Drop may pair with start-of-block
        // RcNew because the `next iteration's` RcNew comes after the Drop
        // along the back-edge.
        let is_loop_body = block_is_loop_body(func, bi);

        let mut used_drops: Vec<bool> = vec![false; drops.len()];
        let mut used_allocs: Vec<bool> = vec![false; allocs.len()];
        let mut decisions: Vec<(ValueId, ValueId, u32)> = Vec::new();

        // Strategy 1: forward pair Drop@i → next compatible RcNew@j>i with
        // no kill in [i,j].
        for (di, d) in drops.iter().enumerate() {
            if used_drops[di] { continue; }
            for (ai, a) in allocs.iter().enumerate() {
                if used_allocs[ai] { continue; }
                if a.inst_idx <= d.inst_idx { continue; }
                if a.size != d.size { continue; }
                let killed = kill_idxs.iter().any(|&k| k > d.inst_idx && k < a.inst_idx);
                if killed { continue; }
                let slot = *next_slot;
                *next_slot += 1;
                decisions.push((d.value, a.dest, slot));
                used_drops[di] = true;
                used_allocs[ai] = true;
                break;
            }
        }

        // Strategy 2 (loop reuse): for any remaining Drop AFTER a remaining
        // RcNew in a loop body, pair them — the next iteration's RcNew
        // consumes the slot saved by this iteration's Drop.
        if is_loop_body {
            for (di, d) in drops.iter().enumerate() {
                if used_drops[di] { continue; }
                for (ai, a) in allocs.iter().enumerate() {
                    if used_allocs[ai] { continue; }
                    if a.inst_idx >= d.inst_idx { continue; }
                    if a.size != d.size { continue; }
                    // Kill check on the back-edge slice (Drop..end) ∪
                    // (start..Alloc) — any escape op invalidates the slot.
                    let killed = kill_idxs
                        .iter()
                        .any(|&k| (k > d.inst_idx) || (k < a.inst_idx));
                    if killed { continue; }
                    let slot = *next_slot;
                    *next_slot += 1;
                    decisions.push((d.value, a.dest, slot));
                    used_drops[di] = true;
                    used_allocs[ai] = true;
                    break;
                }
            }
        }

        for (drop_v, alloc_dest, slot) in decisions {
            func.perceus.reuse_save.insert(drop_v, slot);
            func.perceus.reuse_consume.insert(alloc_dest, slot);
            pairs += 1;
        }
    }

    // Strategy 3: cross-block reuse within a loop body. The runtime alloca
    // slot survives back-edges, so any unpaired `Drop(Rc)` in a loop body can
    // pair with any unpaired `RcNew` of the same payload size in the same
    // loop body, provided the loop body contains no escape ops. The save and
    // consume sites need not be in the same block: codegen guarantees the
    // slot starts NULL, save stores into it (freeing prior tenant), consume
    // loads it (mallocs on NULL), and a function-exit drain frees survivors.
    pairs += cross_block_loop_reuse(func, uses, next_slot);
    hints.stats.reuse_sites += pairs;
}

/// Cross-block reuse pairing pass. Walks every back-edge target, materializes
/// the loop body it heads (set of blocks dominated by the header that can
/// reach the header), and pairs unpaired Drops with unpaired RcNews of the
/// same payload size — but only if the loop body is escape-free (no Call /
/// IndirectCall / MethodCall / ClosureCall / ChanSend / SpawnActor anywhere
/// inside it).
fn cross_block_loop_reuse(
    func: &mut mir::Function,
    uses: &HashMap<ValueId, UseInfo>,
    next_slot: &mut u32,
) -> u32 {
    let mut paired = 0u32;
    let loops = collect_loop_bodies(func);
    for body in loops {
        // Reject if any block in the body contains an escape op.
        let has_escape = body.iter().any(|&bi| {
            func.blocks[bi].insts.iter().any(|inst| {
                matches!(
                    inst.kind,
                    InstKind::Call(_, _)
                        | InstKind::IndirectCall(_, _)
                        | InstKind::MethodCall(_, _, _)
                        | InstKind::ClosureCall(_, _)
                        | InstKind::ChanSend(_, _)
                        | InstKind::SpawnActor(_, _)
                )
            })
        });
        if has_escape {
            continue;
        }
        // Collect unpaired Drop(Rc) and RcNew sites across the body.
        let mut drops: Vec<(ValueId, u64)> = Vec::new();
        let mut allocs: Vec<(ValueId, u64)> = Vec::new();
        for &bi in &body {
            for inst in &func.blocks[bi].insts {
                match &inst.kind {
                    InstKind::Drop(v, Type::Rc(inner))
                        if !func.perceus.reuse_save.contains_key(v) =>
                    {
                        if let Some(info) = uses.get(v) {
                            if info.use_count == 1 && !info.escapes && !info.captured {
                                let size = PerceusPass::type_layout_size_pub(inner);
                                if size > 0 {
                                    drops.push((*v, size));
                                }
                            }
                        }
                    }
                    InstKind::RcNew(_, alloc_inner) => {
                        if let Some(dest) = inst.dest {
                            if !func.perceus.reuse_consume.contains_key(&dest) {
                                let size = PerceusPass::type_layout_size_pub(alloc_inner);
                                if size > 0 {
                                    allocs.push((dest, size));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // Greedy pair on size match.
        let mut used_a = vec![false; allocs.len()];
        for (dv, ds) in &drops {
            for (ai, (av, asz)) in allocs.iter().enumerate() {
                if used_a[ai] || asz != ds {
                    continue;
                }
                let slot = *next_slot;
                *next_slot += 1;
                func.perceus.reuse_save.insert(*dv, slot);
                func.perceus.reuse_consume.insert(*av, slot);
                used_a[ai] = true;
                paired += 1;
                break;
            }
        }
    }
    paired
}

/// Collect each loop body in the function as a Vec of block indices. A loop
/// body is a maximal SCC reachable from a back-edge target. We use a simple
/// rule: for each block H that is a back-edge target (i.e., some block B has
/// H as a successor and H ≤ B in RPO or there is a back-path from H's
/// successors to H), the body is { blocks reachable from H that can reach H }.
/// Bodies may overlap; we deduplicate identical bodies.
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
    // Reverse adjacency.
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
        // Header iff some pred is reachable from h's successors (i.e., back-edge into h).
        if !block_is_loop_body(func, h) {
            continue;
        }
        // Body = { b : b reachable from h AND h reachable from b }.
        // Forward reachable from h.
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
        // Backward reachable from h (uses preds).
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

/// Block `bi` is a loop body iff there is a path from any successor of `bi`
/// back to `bi`. Implemented with a forward DFS from each successor.
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

// ── Pass: vec reuse pairing ──────────────────────────────────────

/// Pair `Drop(Vec(T))` with the next `VecNew([])` of `Vec(T)` (same element
/// type) so that the dropped vector's *header + data buffer* is recycled
/// instead of freed and re-malloc'd. The save path deep-drops elements
/// (necessary to release element-owned heap), then stashes the header
/// pointer; the consume path resets `len = 0`, leaving the data buffer's
/// capacity intact for the next push run.
///
/// Wins relative to baseline:
///   * 1 fewer `malloc(24)` per loop iteration (header)
///   * 1 fewer `malloc(cap*sizeof(T))` per loop iteration (data buffer)
///   * Eliminates capacity ramp-up on every iteration
///
/// Soundness restrictions:
///   * `v_old` must be `Type::Vec(T)`, `use_count == 1`, `!escapes`,
///     `!captured`.
///   * The matching alloc must be `VecNew(empty)` of `Vec(T)` with the
///     SAME inner type T (so the buffer's element layout is reusable).
///   * Same-block, same-block-loop-back-edge, and cross-block-loop-body
///     patterns are all supported via the unified strategy below.
///   * Loop-body cross-block pairing requires the loop body to be
///     escape-free (any Call / IndirectCall / MethodCall / ClosureCall /
///     ChanSend / SpawnActor invalidates the slot).
fn vec_reuse_pairing(
    func: &mut mir::Function,
    uses: &HashMap<ValueId, UseInfo>,
    hints: &mut PerceusHints,
    next_slot: &mut u32,
) {
    let mut pairs = 0u32;

    // Per-block strategies (1 forward, 2 back-edge).
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
                InstKind::Drop(v, Type::Vec(elem))
                    if !func.perceus.reuse_save.contains_key(v) =>
                {
                    // The Drop instruction itself proves uniqueness at
                    // this program point — even if `push`/`get` are seen as
                    // escapes by the conservative use analysis, ownership
                    // semantics guarantee no live alias remains.
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
                | InstKind::MethodCall(_, _, _)
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

        // Strategy 1: forward pair (Drop@i → VecNew@j>i with no kill in [i,j]).
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
                let killed = kill_idxs
                    .iter()
                    .any(|&k| k > d.inst_idx && k < a.inst_idx);
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

        // Strategy 2: back-edge same-block (Drop@end → VecNew@start of next iter).
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

    // Strategy 3: cross-block loop-body pairing.
    let loops = collect_loop_bodies(func);
    for body in loops {
        let has_escape = body.iter().any(|&bi| {
            func.blocks[bi].insts.iter().any(|inst| {
                matches!(
                    inst.kind,
                    InstKind::Call(_, _)
                        | InstKind::IndirectCall(_, _)
                        | InstKind::MethodCall(_, _, _)
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

// ── Pass: drop fusion ────────────────────────────────────────────

/// Replace runs of >=2 consecutive `Drop` instructions with a single
/// `InstKind::DropMany`. Drops tagged for reuse-save are excluded so codegen
/// can still stash their pointers individually.
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
