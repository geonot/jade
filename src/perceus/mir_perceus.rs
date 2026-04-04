//! Perceus reference-counting optimization on MIR.
//!
//! This module re-implements the Perceus analysis passes operating directly on
//! MIR's SSA form rather than HIR.  SSA makes several analyses exact:
//!
//!   - **Use counting** is trivial: each ValueId has exactly one definition,
//!     so we just count operand occurrences.
//!   - **Last use** is determined by linear scan through blocks.
//!   - **Escape analysis** tracks ClosureCreate captures and Call arguments.
//!   - **Reuse matching** finds RcNew/RcDec pairs with layout-compatible types.
//!
//! The output is a `PerceusHints` struct identical to the HIR Perceus output,
//! so the MIR codegen backend can consume it without changes.

use std::collections::HashMap;

use crate::ast::Span;
use crate::hir::DefId;
use crate::mir::{self, InstKind, ValueId};
use crate::types::Type;

use super::{DropFusion, FbipSite, PerceusHints, PerceusPass, PoolHint, ReuseInfo, TailReuseInfo};

/// Per-value use information gathered from MIR SSA form.
#[derive(Debug, Clone)]
struct MirUseInfo {
    /// Number of times this ValueId is referenced as an operand.
    use_count: u32,
    /// Span of the last instruction that uses this value.
    last_use_span: Option<Span>,
    /// Whether this value escapes (passed to a call, stored in a closure env, etc.).
    escapes: bool,
    /// Whether this value is captured by a closure.
    captured: bool,
    /// The type of this value.
    ty: Type,
    /// The HIR DefId, if the producing instruction was annotated with one.
    def_id: Option<DefId>,
}

/// Analyze a full MIR program and produce Perceus hints.
pub fn analyze_mir_program(prog: &mir::Program) -> PerceusHints {
    let mut hints = PerceusHints::default();

    for func in &prog.functions {
        analyze_mir_fn(func, &mut hints);
    }

    hints
}

/// Analyze a single MIR function.
fn analyze_mir_fn(func: &mir::Function, hints: &mut PerceusHints) {
    // 1. Build use-def information.
    let uses = count_uses(func);

    // 2. Borrow promotion: promote single-use non-escaping values to moves.
    analyze_borrow_promotion(&uses, hints);

    // 3. Drop specialization: elide drops for trivially-droppable values.
    analyze_drop_specialization(&uses, hints);

    // 4. Drop fusion: coalesce consecutive trivially-droppable drops.
    analyze_drop_fusion(func, &uses, hints);

    // 5. Reuse analysis: find RcNew/RcDec pairs with compatible layouts.
    analyze_reuse(func, &uses, hints);

    // 6. Last-use analysis.
    analyze_last_use(&uses, hints);

    // 7. FBIP analysis: find match+construct patterns where allocation can reuse.
    analyze_fbip(func, &uses, hints);

    // 8. Tail reuse: parameter allocation reuse for tail-position constructors.
    analyze_tail_reuse(func, &uses, hints);

    // 9. Speculative reuse: nearby Rc alloc/dealloc pairs.
    analyze_speculative_reuse(func, &uses, hints);

    // 10. Pool hints: detect loop-body allocations for pre-allocation.
    analyze_pool_hints(func, hints);
}

// ── Use counting ─────────────────────────────────────────────────

fn count_uses(func: &mir::Function) -> HashMap<ValueId, MirUseInfo> {
    let mut uses: HashMap<ValueId, MirUseInfo> = HashMap::new();

    // ── Pass 1: Register all definitions (params, phi dests, instruction dests) ──
    for p in &func.params {
        uses.insert(
            p.value,
            MirUseInfo {
                use_count: 0,
                last_use_span: None,
                escapes: false,
                captured: false,
                ty: p.ty.clone(),
                def_id: None,
            },
        );
    }

    for bb in &func.blocks {
        for phi in &bb.phis {
            uses.entry(phi.dest).or_insert_with(|| MirUseInfo {
                use_count: 0,
                last_use_span: None,
                escapes: false,
                captured: false,
                ty: phi.ty.clone(),
                def_id: None,
            });
        }
        for inst in &bb.insts {
            if let Some(dest) = inst.dest {
                uses.entry(dest).or_insert_with(|| MirUseInfo {
                    use_count: 0,
                    last_use_span: None,
                    escapes: false,
                    captured: false,
                    ty: inst.ty.clone(),
                    def_id: inst.def_id,
                });
            }
        }
    }

    // ── Pass 2: Count all uses now that every definition is registered ──
    for bb in &func.blocks {
        // Phi incoming values count as uses.
        for phi in &bb.phis {
            for (_, vid) in &phi.incoming {
                if let Some(info) = uses.get_mut(vid) {
                    info.use_count += 1;
                }
            }
        }

        // Count each operand as a use.
        for inst in &bb.insts {
            for operand in inst_operands(&inst.kind) {
                if let Some(info) = uses.get_mut(&operand) {
                    info.use_count += 1;
                    info.last_use_span = Some(inst.span);
                }
            }

            // Track escapes: values passed to calls or stored in closures.
            match &inst.kind {
                InstKind::Call(_, args) | InstKind::IndirectCall(_, args) => {
                    for a in args {
                        if let Some(info) = uses.get_mut(a) {
                            info.escapes = true;
                        }
                    }
                }
                InstKind::MethodCall(_, _, args) => {
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
                InstKind::SpawnActor(_, args) => {
                    for a in args {
                        if let Some(info) = uses.get_mut(a) {
                            info.escapes = true;
                        }
                    }
                }
                InstKind::ChanSend(_, val) => {
                    if let Some(info) = uses.get_mut(val) {
                        info.escapes = true;
                    }
                }
                _ => {}
            }
        }

        // Count terminator operands.
        match &bb.terminator {
            mir::Terminator::Branch(cond, _, _) => {
                if let Some(info) = uses.get_mut(cond) {
                    info.use_count += 1;
                }
            }
            mir::Terminator::Return(Some(val)) => {
                if let Some(info) = uses.get_mut(val) {
                    info.use_count += 1;
                    // Returned values escape to the caller.
                    info.escapes = true;
                }
            }
            mir::Terminator::Switch(disc, _, _) => {
                if let Some(info) = uses.get_mut(disc) {
                    info.use_count += 1;
                }
            }
            _ => {}
        }
    }

    uses
}

/// Extract all ValueId operands from an instruction kind.
fn inst_operands(kind: &InstKind) -> Vec<ValueId> {
    match kind {
        InstKind::IntConst(_)
        | InstKind::FloatConst(_)
        | InstKind::BoolConst(_)
        | InstKind::StringConst(_)
        | InstKind::Void
        | InstKind::MapInit
        | InstKind::SetInit => vec![],

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

        InstKind::Load(_) | InstKind::FnRef(_) => vec![],
        InstKind::Store(_, val) => vec![*val],

        InstKind::FieldGet(obj, _) => vec![*obj],
        InstKind::FieldSet(obj, _, val) => vec![*obj, *val],
        InstKind::FieldStore(_, _, val) => vec![*val],
        InstKind::Index(base, idx) => vec![*base, *idx],
        InstKind::IndexSet(base, idx, val) => vec![*base, *idx, *val],

        InstKind::StructInit(_, fields) => fields.iter().map(|(_, v)| *v).collect(),
        InstKind::VariantInit(_, _, _, payload) => payload.clone(),
        InstKind::ArrayInit(elems) => elems.clone(),

        InstKind::Cast(v, _) => vec![*v],
        InstKind::Ref(v) => vec![*v],
        InstKind::Deref(v) => vec![*v],

        InstKind::Alloc(v) => vec![*v],
        InstKind::Drop(v, _) => vec![*v],
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
        InstKind::RcClone(v) => vec![*v],
        InstKind::WeakUpgrade(v) => vec![*v],

        InstKind::SpawnActor(_, args) => args.clone(),
        InstKind::ChanCreate(_) => vec![],
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
    }
}

// ── Drop specialization ─────────────────────────────────────────

fn analyze_drop_specialization(uses: &HashMap<ValueId, MirUseInfo>, hints: &mut PerceusHints) {
    for (_, info) in uses {
        if let Some(def_id) = info.def_id {
            if info.ty.is_trivially_droppable() {
                hints.elide_drops.insert(def_id);
                hints.stats.drops_elided += 1;
            }
        }
        hints.stats.total_bindings_analyzed += 1;
    }
}

// ── Borrow promotion ────────────────────────────────────────────

/// Promote single-use, non-escaping, non-captured Rc values to moves.
/// In SSA, a value used exactly once that doesn't escape can be moved instead of cloned.
fn analyze_borrow_promotion(uses: &HashMap<ValueId, MirUseInfo>, hints: &mut PerceusHints) {
    for (_, info) in uses {
        if let Some(def_id) = info.def_id {
            if matches!(info.ty, Type::Rc(_))
                && info.use_count <= 1
                && !info.escapes
                && !info.captured
            {
                hints.borrow_to_move.insert(def_id);
                hints.stats.borrows_promoted += 1;
            }
        }
    }
}

// ── Drop fusion ─────────────────────────────────────────────────

/// Coalesce consecutive trivially-droppable Drop instructions within a basic block.
fn analyze_drop_fusion(
    func: &mir::Function,
    uses: &HashMap<ValueId, MirUseInfo>,
    hints: &mut PerceusHints,
) {
    for bb in &func.blocks {
        let mut run: Vec<DefId> = Vec::new();
        let mut run_span: Option<Span> = None;

        for inst in &bb.insts {
            let is_trivial_drop = if let InstKind::Drop(val, ty) = &inst.kind {
                if ty.is_trivially_droppable() {
                    if let Some(info) = uses.get(val) {
                        if let Some(def_id) = info.def_id {
                            run.push(def_id);
                            if run_span.is_none() {
                                run_span = Some(inst.span);
                            }
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if !is_trivial_drop {
                if run.len() >= 2 {
                    hints.drop_fusions.push(DropFusion {
                        def_ids: run.clone(),
                        span: run_span.unwrap_or(Span::dummy()),
                    });
                    hints.stats.drops_fused += run.len() as u32;
                }
                run.clear();
                run_span = None;
            }
        }
        // Flush any remaining run at block end.
        if run.len() >= 2 {
            hints.drop_fusions.push(DropFusion {
                def_ids: run,
                span: run_span.unwrap_or(Span::dummy()),
            });
        }
    }
}

// ── Pool hints ──────────────────────────────────────────────────

/// Detect Rc/struct allocations inside loop bodies that could benefit from
/// pool pre-allocation. In MIR SSA, loops are back-edges in the CFG.
/// We detect loops by looking for back-edges (blocks that branch to earlier blocks).
fn analyze_pool_hints(func: &mir::Function, hints: &mut PerceusHints) {
    use std::collections::HashSet;

    // Simple loop detection: a back-edge target's successor blocks form the loop body.
    // For simplicity, we check each block's successors — if any target has a lower index,
    // the blocks between the target and current form a loop.
    let block_ids: Vec<mir::BlockId> = func.blocks.iter().map(|b| b.id).collect();
    let block_index: HashMap<mir::BlockId, usize> = block_ids.iter().enumerate()
        .map(|(i, id)| (*id, i))
        .collect();

    let mut loop_body_blocks: HashSet<usize> = HashSet::new();

    for (i, bb) in func.blocks.iter().enumerate() {
        let successors = terminator_successors(&bb.terminator);
        for succ in &successors {
            if let Some(&succ_idx) = block_index.get(succ) {
                if succ_idx <= i {
                    // Back-edge detected: blocks [succ_idx..=i] form a loop body.
                    for j in succ_idx..=i {
                        loop_body_blocks.insert(j);
                    }
                }
            }
        }
    }

    // Scan loop body blocks for allocation instructions.
    let mut alloc_types: HashMap<u64, (Type, Span)> = HashMap::new();
    for &idx in &loop_body_blocks {
        let bb = &func.blocks[idx];
        for inst in &bb.insts {
            let alloc_ty = match &inst.kind {
                InstKind::RcNew(_, inner_ty) => Some(inner_ty.clone()),
                InstKind::StructInit(_, _) => Some(inst.ty.clone()),
                InstKind::VariantInit(_, _, _, _) => Some(inst.ty.clone()),
                _ => None,
            };
            if let Some(ty) = alloc_ty {
                let size = PerceusPass::type_layout_size_pub(&ty);
                if size > 0 {
                    alloc_types.entry(size).or_insert((ty, inst.span));
                }
            }
        }
    }

    for (size, (ty, span)) in alloc_types {
        hints.pool_hints.push(PoolHint {
            alloc_ty: ty,
            size,
            span,
        });
        hints.stats.pool_hints_found += 1;
    }
}

/// Get successor block IDs from a terminator.
fn terminator_successors(term: &mir::Terminator) -> Vec<mir::BlockId> {
    match term {
        mir::Terminator::Goto(b) => vec![*b],
        mir::Terminator::Branch(_, t, e) => vec![*t, *e],
        mir::Terminator::Switch(_, cases, default) => {
            let mut v: Vec<mir::BlockId> = cases.iter().map(|(_, b)| *b).collect();
            v.push(*default);
            v
        }
        mir::Terminator::Return(_) | mir::Terminator::Unreachable => vec![],
    }
}

// ── Reuse analysis ──────────────────────────────────────────────

fn analyze_reuse(
    func: &mir::Function,
    uses: &HashMap<ValueId, MirUseInfo>,
    hints: &mut PerceusHints,
) {
    // Collect Rc-typed values: (ValueId, Type, Span, Option<DefId>).
    let mut rc_values: Vec<(ValueId, Type, Span, Option<DefId>)> = Vec::new();

    for bb in &func.blocks {
        for inst in &bb.insts {
            if let Some(dest) = inst.dest {
                if matches!(inst.ty, Type::Rc(_)) {
                    rc_values.push((dest, inst.ty.clone(), inst.span, inst.def_id));
                }
                // Also catch RcNew instructions directly.
                if matches!(inst.kind, InstKind::RcNew(_, _)) {
                    // Already caught by type check above, but ensure it's in the list.
                }
            }
        }
    }

    // Group by layout size for O(n) matching.
    let mut released_by_size: HashMap<u64, Vec<(ValueId, Type, Span, Option<DefId>)>> =
        HashMap::new();

    for (vid, ty, span, def_id) in &rc_values {
        let info = match uses.get(vid) {
            Some(i) => i,
            None => continue,
        };

        let inner_ty = match &ty {
            Type::Rc(inner) => inner.as_ref(),
            _ => &ty,
        };
        let size = PerceusPass::type_layout_size_pub(inner_ty);

        // Single-use, non-escaping Rc values are release candidates.
        if info.use_count <= 1 && !info.escapes && !info.captured {
            released_by_size
                .entry(size)
                .or_default()
                .push((*vid, ty.clone(), *span, *def_id));
        } else {
            // Check if we can match this allocation against a released value.
            if let Some(candidates) = released_by_size.get_mut(&size) {
                if let Some((_, released_ty, _, released_def_id)) = candidates.pop() {
                    if let (Some(r_id), Some(a_id)) = (released_def_id, def_id) {
                        hints.reuse_candidates.insert(
                            r_id,
                            ReuseInfo {
                                released_ty: released_ty.clone(),
                                allocated_ty: ty.clone(),
                                span: *span,
                            },
                        );
                        hints.reuse_candidates.insert(
                            *a_id,
                            ReuseInfo {
                                released_ty,
                                allocated_ty: ty.clone(),
                                span: *span,
                            },
                        );
                        hints.stats.reuse_sites += 1;
                    }
                }
            }
        }
    }
}

// ── Last-use analysis ───────────────────────────────────────────

fn analyze_last_use(uses: &HashMap<ValueId, MirUseInfo>, hints: &mut PerceusHints) {
    for (_, info) in uses {
        if let Some(def_id) = info.def_id {
            if info.use_count > 0 {
                if let Some(span) = info.last_use_span {
                    hints.last_use.insert(def_id, span);
                    hints.stats.last_use_tracked += 1;
                }
            }
        }
    }
}

// ── FBIP analysis ───────────────────────────────────────────────

/// Functional But In-Place: detect match patterns where a destructured value's
/// memory can be reused for the newly constructed return value.
fn analyze_fbip(
    func: &mir::Function,
    uses: &HashMap<ValueId, MirUseInfo>,
    hints: &mut PerceusHints,
) {
    // Look for patterns in the MIR:
    //   1. A Switch terminator on some value `disc`
    //   2. In the case arms, a VariantInit or StructInit that constructs a value
    //      with a layout compatible with the Switch subject's type
    //   3. The subject has use_count == 1 and doesn't escape

    for bb in &func.blocks {
        if let mir::Terminator::Switch(disc, cases, _) = &bb.terminator {
            let disc_info = match uses.get(disc) {
                Some(i) => i,
                None => continue,
            };
            if disc_info.use_count > 2 || disc_info.escapes {
                continue;
            }

            // Check each case arm for a constructor.
            for (_, target_id) in cases {
                let target_bb = match func.blocks.iter().find(|b| b.id == *target_id) {
                    Some(b) => b,
                    None => continue,
                };
                for inst in &target_bb.insts {
                    let constructed_ty = match &inst.kind {
                        InstKind::VariantInit(_, _, _, _) => Some(&inst.ty),
                        InstKind::StructInit(_, _) => Some(&inst.ty),
                        InstKind::RcNew(_, _) => Some(&inst.ty),
                        _ => None,
                    };
                    if let Some(ctor_ty) = constructed_ty {
                        if PerceusPass::layouts_compatible(&disc_info.ty, ctor_ty) {
                            if let Some(def_id) = disc_info.def_id {
                                hints.fbip_sites.push(FbipSite {
                                    subject_id: def_id,
                                    subject_ty: disc_info.ty.clone(),
                                    constructed_ty: ctor_ty.clone(),
                                    span: inst.span,
                                });
                                hints.stats.fbip_sites += 1;
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── Tail reuse analysis ─────────────────────────────────────────

fn analyze_tail_reuse(
    func: &mir::Function,
    uses: &HashMap<ValueId, MirUseInfo>,
    hints: &mut PerceusHints,
) {
    // Find the return value's type by looking at Return terminators.
    let mut return_constructor_ty: Option<Type> = None;

    for bb in &func.blocks {
        if let mir::Terminator::Return(Some(ret_val)) = &bb.terminator {
            // Check if the return value was produced by a constructor.
            for inst in bb.insts.iter().rev() {
                if inst.dest == Some(*ret_val) {
                    match &inst.kind {
                        InstKind::VariantInit(_, _, _, _)
                        | InstKind::StructInit(_, _)
                        | InstKind::RcNew(_, _) => {
                            return_constructor_ty = Some(inst.ty.clone());
                        }
                        _ => {}
                    }
                    break;
                }
            }
        }
    }

    let alloc_ty = match return_constructor_ty {
        Some(ty) => ty,
        None => return,
    };

    // Check each parameter: if owned, non-escaping, and layout-compatible, tag for reuse.
    for p in &func.params {
        let info = match uses.get(&p.value) {
            Some(i) => i,
            None => continue,
        };
        if !info.escapes && PerceusPass::layouts_compatible(&p.ty, &alloc_ty) {
            hints.tail_reuse.insert(
                func.def_id,
                TailReuseInfo {
                    param_id: func.def_id,
                    param_ty: p.ty.clone(),
                    alloc_ty: alloc_ty.clone(),
                    span: func.span,
                },
            );
            hints.stats.tail_reuse_sites += 1;
            break; // One tail reuse per function is sufficient.
        }
    }
}

// ── Speculative reuse ───────────────────────────────────────────

fn analyze_speculative_reuse(
    func: &mir::Function,
    uses: &HashMap<ValueId, MirUseInfo>,
    hints: &mut PerceusHints,
) {
    // Find adjacent RcDec → RcNew pairs within the same block where the
    // released and allocated types have compatible layouts.
    for bb in &func.blocks {
        let mut last_rc_dec: Option<(ValueId, &Type, Span, Option<DefId>)> = None;

        for inst in &bb.insts {
            match &inst.kind {
                InstKind::RcDec(val) => {
                    if let Some(info) = uses.get(val) {
                        last_rc_dec = Some((*val, &info.ty, inst.span, info.def_id));
                    }
                }
                InstKind::RcNew(_, alloc_ty) => {
                    if let Some((_, released_ty, _, released_def_id)) = &last_rc_dec {
                        if PerceusPass::layouts_compatible(released_ty, alloc_ty) {
                            if let (Some(r_id), Some(_a_id)) = (released_def_id, inst.def_id) {
                                if !hints.reuse_candidates.contains_key(r_id) {
                                    hints.speculative_reuse.insert(
                                        *r_id,
                                        ReuseInfo {
                                            released_ty: (*released_ty).clone(),
                                            allocated_ty: alloc_ty.clone(),
                                            span: inst.span,
                                        },
                                    );
                                    hints.stats.speculative_reuse_sites += 1;
                                }
                            }
                        }
                    }
                    last_rc_dec = None;
                }
                _ => {
                    // Any intervening instruction invalidates the RcDec candidate.
                    if !matches!(inst.kind, InstKind::Copy(_) | InstKind::Void) {
                        last_rc_dec = None;
                    }
                }
            }
        }
    }
}
