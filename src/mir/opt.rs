//! MIR optimization passes.
//!
//! Each pass takes `&mut Function` and returns `true` if it changed anything.
//! The driver runs passes in a fixed-point loop until convergence.

use std::collections::{HashMap, HashSet, VecDeque};
use crate::ast::Span;
use crate::types::Type;
use super::*;

// ━━━━━━━━━━━━━━━━━━━━━━ Driver ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Optimization level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel { None, Basic, Full }

/// Run all optimization passes until a fixed point.
pub fn optimize(func: &mut Function, level: OptLevel) {
    if level == OptLevel::None { return; }

    remove_unreachable_blocks(func);

    let max = match level { OptLevel::None => 0, OptLevel::Basic => 4, OptLevel::Full => 16 };
    for _iter in 0..max {
        let mut changed = false;
        changed |= constant_fold(func);
        changed |= copy_propagation(func);
        changed |= store_load_forwarding(func);
        changed |= simplify_phis(func);
        changed |= dead_code_elimination(func);
        changed |= strength_reduction(func);
        if level == OptLevel::Full {
            changed |= global_value_numbering(func);
            changed |= branch_threading(func);
            changed |= loop_invariant_code_motion(func);
        }
        changed |= redundant_store_elimination(func);
        changed |= merge_linear_blocks(func);
        changed |= remove_unreachable_blocks(func);
        if !changed { break; }
    }
}

// ━━━━━━━━━━━━━━━━━━ Value replacement helpers ━━━━━━━━━━━━━━━━━━━━━

/// Apply a value→value substitution map to every use in an instruction.
fn subst_inst(inst: &mut Instruction, map: &HashMap<ValueId, ValueId>) -> bool {
    let mut hit = false;
    macro_rules! sub {
        ($v:expr) => { if let Some(&r) = map.get($v) { *$v = r; hit = true; } };
    }
    match &mut inst.kind {
        InstKind::BinOp(_, a, b) | InstKind::Cmp(_, a, b, _) => { sub!(a); sub!(b); }
        InstKind::UnaryOp(_, v) | InstKind::Cast(v, _) | InstKind::StrictCast(v, _) | InstKind::Ref(v)
        | InstKind::Deref(v) | InstKind::Copy(v) | InstKind::RcInc(v)
        | InstKind::RcDec(v) | InstKind::Alloc(v) => { sub!(v); }
        InstKind::Drop(v, _) => { sub!(v); }
        InstKind::Call(_, args) | InstKind::ArrayInit(args)
        | InstKind::VariantInit(_, _, _, args) => { for a in args { sub!(a); } }
        InstKind::MethodCall(obj, _, args) => { sub!(obj); for a in args { sub!(a); } }
        InstKind::IndirectCall(f, args) => { sub!(f); for a in args { sub!(a); } }
        InstKind::FieldGet(o, _) => { sub!(o); }
        InstKind::FieldSet(o, _, v) => { sub!(o); sub!(v); }
        InstKind::FieldStore(_, _, v) => { sub!(v); }
        InstKind::Index(a, i) => { sub!(a); sub!(i); }
        InstKind::IndexSet(a, i, v) => { sub!(a); sub!(i); sub!(v); }
        InstKind::IndexStore(_, i, v) => { sub!(i); sub!(v); }
        InstKind::StructInit(_, fs) => { for (_, v) in fs { sub!(v); } }
        InstKind::Slice(a, s, e) => { sub!(a); sub!(s); sub!(e); }
        InstKind::Store(_, v) => { sub!(v); }
        // Collections
        InstKind::VecNew(args) => { for a in args { sub!(a); } }
        InstKind::VecPush(vec, val) | InstKind::ChanSend(vec, val) => { sub!(vec); sub!(val); }
        InstKind::VecLen(v) | InstKind::ChanRecv(v) | InstKind::RcClone(v)
        | InstKind::WeakUpgrade(v) | InstKind::Log(v) => { sub!(v); }
        InstKind::RcNew(v, _) => { sub!(v); }
        // Closures
        InstKind::ClosureCreate(_, captures) | InstKind::SpawnActor(_, captures)
        | InstKind::SelectArm(captures, _) => { for a in captures { sub!(a); } }
        InstKind::ClosureCall(f, args) => { sub!(f); for a in args { sub!(a); } }
        InstKind::ChanCreate(_, cap) => { if let Some(c) = cap { sub!(c); } }
        InstKind::MapInit | InstKind::SetInit | InstKind::PQInit | InstKind::DequeInit => {}
        InstKind::Assert(v, _) => { sub!(v); }
        InstKind::DynDispatch(obj, _, _, args) => { sub!(obj); for a in args { sub!(a); } }
        InstKind::DynCoerce(v, _, _) => { sub!(v); }
        InstKind::InlineAsm(_, args) => { for a in args { sub!(a); } }
        InstKind::IntConst(_) | InstKind::FloatConst(_) | InstKind::BoolConst(_)
        | InstKind::StringConst(_) | InstKind::Void | InstKind::Load(_) | InstKind::FnRef(_) => {}
    }
    hit
}

/// Apply a value→value substitution map to a terminator.
fn subst_term(term: &mut Terminator, map: &HashMap<ValueId, ValueId>) -> bool {
    match term {
        Terminator::Branch(c, _, _) => if let Some(&v) = map.get(c) { *c = v; return true; },
        Terminator::Return(Some(v)) => if let Some(&r) = map.get(v) { *v = r; return true; },
        Terminator::Switch(v, _, _) => if let Some(&r) = map.get(v) { *v = r; return true; },
        _ => {}
    }
    false
}

/// Collect all ValueIds *used* (read) anywhere in the function.
fn collect_used(func: &Function) -> HashSet<ValueId> {
    let mut used = HashSet::new();
    for bb in &func.blocks {
        for phi in &bb.phis { for (_, v) in &phi.incoming { used.insert(*v); } }
        for inst in &bb.insts { collect_inst_uses(&inst.kind, &mut used); }
        collect_term_uses(&bb.terminator, &mut used);
    }
    used
}

fn collect_inst_uses(kind: &InstKind, s: &mut HashSet<ValueId>) {
    match kind {
        InstKind::BinOp(_, a, b) | InstKind::Cmp(_, a, b, _) => { s.insert(*a); s.insert(*b); }
        InstKind::UnaryOp(_, v) | InstKind::Cast(v, _) | InstKind::StrictCast(v, _) | InstKind::Ref(v)
        | InstKind::Deref(v) | InstKind::Copy(v) | InstKind::RcInc(v)
        | InstKind::RcDec(v) | InstKind::Alloc(v) => { s.insert(*v); }
        InstKind::Drop(v, _) => { s.insert(*v); }
        InstKind::Call(_, args) | InstKind::ArrayInit(args)
        | InstKind::VariantInit(_, _, _, args) => { for a in args { s.insert(*a); } }
        InstKind::MethodCall(obj, _, args) => { s.insert(*obj); for a in args { s.insert(*a); } }
        InstKind::IndirectCall(f, args) => { s.insert(*f); for a in args { s.insert(*a); } }
        InstKind::FieldGet(o, _) => { s.insert(*o); }
        InstKind::FieldSet(o, _, v) => { s.insert(*o); s.insert(*v); }
        InstKind::FieldStore(_, _, v) => { s.insert(*v); }
        InstKind::Index(a, i) => { s.insert(*a); s.insert(*i); }
        InstKind::IndexSet(a, i, v) => { s.insert(*a); s.insert(*i); s.insert(*v); }
        InstKind::IndexStore(_, i, v) => { s.insert(*i); s.insert(*v); }
        InstKind::StructInit(_, fs) => { for (_, v) in fs { s.insert(*v); } }
        InstKind::Slice(a, lo, hi) => { s.insert(*a); s.insert(*lo); s.insert(*hi); }
        InstKind::Store(_, v) => { s.insert(*v); }
        // Collections
        InstKind::VecNew(args) => { for a in args { s.insert(*a); } }
        InstKind::VecPush(vec, val) | InstKind::ChanSend(vec, val) => { s.insert(*vec); s.insert(*val); }
        InstKind::VecLen(v) | InstKind::ChanRecv(v) | InstKind::RcClone(v)
        | InstKind::WeakUpgrade(v) | InstKind::Log(v) => { s.insert(*v); }
        InstKind::RcNew(v, _) => { s.insert(*v); }
        InstKind::ClosureCreate(_, captures) | InstKind::SpawnActor(_, captures)
        | InstKind::SelectArm(captures, _) => { for a in captures { s.insert(*a); } }
        InstKind::ClosureCall(f, args) => { s.insert(*f); for a in args { s.insert(*a); } }
        InstKind::ChanCreate(_, cap) => { if let Some(c) = cap { s.insert(*c); } }
        InstKind::MapInit | InstKind::SetInit | InstKind::PQInit | InstKind::DequeInit => {}
        InstKind::Assert(v, _) => { s.insert(*v); }
        InstKind::DynDispatch(obj, _, _, args) => { s.insert(*obj); for a in args { s.insert(*a); } }
        InstKind::DynCoerce(v, _, _) => { s.insert(*v); }
        InstKind::InlineAsm(_, args) => { for a in args { s.insert(*a); } }
        InstKind::IntConst(_) | InstKind::FloatConst(_) | InstKind::BoolConst(_)
        | InstKind::StringConst(_) | InstKind::Void | InstKind::Load(_) | InstKind::FnRef(_) => {}
    }
}

fn collect_term_uses(term: &Terminator, s: &mut HashSet<ValueId>) {
    match term {
        Terminator::Branch(c, _, _) | Terminator::Switch(c, _, _) => { s.insert(*c); }
        Terminator::Return(Some(v)) => { s.insert(*v); }
        _ => {}
    }
}

/// Returns `true` if an instruction is side-effect-free and can be eliminated
/// by DCE if its result is unused.
fn is_pure(kind: &InstKind) -> bool {
    matches!(kind,
        InstKind::IntConst(_) | InstKind::FloatConst(_) | InstKind::BoolConst(_)
        | InstKind::StringConst(_) | InstKind::Void | InstKind::FnRef(_)
        | InstKind::BinOp(..) | InstKind::UnaryOp(..) | InstKind::Cmp(..)
        | InstKind::Cast(..) | InstKind::StrictCast(..) | InstKind::Copy(..)
        | InstKind::FieldGet(..) | InstKind::Index(..)
        | InstKind::ArrayInit(_) | InstKind::StructInit(..) | InstKind::VariantInit(..)
        | InstKind::VecLen(..) | InstKind::MapInit | InstKind::SetInit | InstKind::PQInit | InstKind::DequeInit)
    // NOTE: Load is NOT pure — it reads mutable state. However, loads
    // whose results have been forwarded are cleaned up by store_load_forwarding.
    // RcClone is NOT pure — it increments a reference count.
    // WeakUpgrade is NOT pure — it may have runtime side effects.
}

// ━━━━━━━━━━━━━━━━━━━ Constant Folding ━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone)]
enum ConstVal { Int(i64), Float(f64), Bool(bool) }

impl ConstVal {
    fn to_inst(&self) -> InstKind {
        match self {
            ConstVal::Int(n) => InstKind::IntConst(*n),
            ConstVal::Float(f) => InstKind::FloatConst(*f),
            ConstVal::Bool(b) => InstKind::BoolConst(*b),
        }
    }
}

/// Fold instructions with constant operands.
pub fn constant_fold(func: &mut Function) -> bool {
    let mut changed = false;
    let mut consts: HashMap<ValueId, ConstVal> = HashMap::new();

    // Seed from existing constants
    for bb in &func.blocks {
        for inst in &bb.insts {
            if let Some(d) = inst.dest {
                match &inst.kind {
                    InstKind::IntConst(n) => { consts.insert(d, ConstVal::Int(*n)); }
                    InstKind::FloatConst(f) => { consts.insert(d, ConstVal::Float(*f)); }
                    InstKind::BoolConst(b) => { consts.insert(d, ConstVal::Bool(*b)); }
                    _ => {}
                }
            }
        }
    }

    for bb in &mut func.blocks {
        for inst in &mut bb.insts {
            if let Some(d) = inst.dest {
                let folded = match &inst.kind {
                    InstKind::BinOp(op, l, r) => fold_binop(*op, consts.get(l), consts.get(r)),
                    InstKind::Cmp(op, l, r, _)   => fold_cmp(*op, consts.get(l), consts.get(r)),
                    InstKind::UnaryOp(op, v)   => fold_unary(*op, consts.get(v)),
                    InstKind::Cast(v, ty)      => fold_cast(consts.get(v), ty),
                    InstKind::StrictCast(v, ty) => fold_cast(consts.get(v), ty),
                    _ => None,
                };
                if let Some(cv) = folded {
                    inst.kind = cv.to_inst();
                    consts.insert(d, cv);
                    changed = true;
                }
            }
        }

        // Fold branches with known conditions
        if let Terminator::Branch(c, t, f) = &bb.terminator {
            if let Some(ConstVal::Bool(b)) = consts.get(c) {
                bb.terminator = Terminator::Goto(if *b { *t } else { *f });
                changed = true;
            }
        }
    }
    changed
}

fn fold_binop(op: BinOp, l: Option<&ConstVal>, r: Option<&ConstVal>) -> Option<ConstVal> {
    match (l?, r?) {
        (ConstVal::Int(a), ConstVal::Int(b)) => {
            let v = match op {
                BinOp::Add => a.wrapping_add(*b),
                BinOp::Sub => a.wrapping_sub(*b),
                BinOp::Mul => a.wrapping_mul(*b),
                BinOp::Div if *b != 0 => a.wrapping_div(*b),
                BinOp::Mod if *b != 0 => a.wrapping_rem(*b),
                BinOp::BitAnd => a & b,
                BinOp::BitOr  => a | b,
                BinOp::BitXor => a ^ b,
                BinOp::Shl => a.checked_shl(*b as u32)?,
                BinOp::Shr => a.checked_shr(*b as u32)?,
                BinOp::Exp => a.checked_pow(*b as u32)?,
                _ => return None,
            };
            Some(ConstVal::Int(v))
        }
        (ConstVal::Float(a), ConstVal::Float(b)) => {
            let v = match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div if *b != 0.0 => a / b,
                BinOp::Mod if *b != 0.0 => a % b,
                BinOp::Exp => a.powf(*b),
                _ => return None,
            };
            Some(ConstVal::Float(v))
        }
        (ConstVal::Bool(a), ConstVal::Bool(b)) => match op {
            BinOp::And => Some(ConstVal::Bool(*a && *b)),
            BinOp::Or  => Some(ConstVal::Bool(*a || *b)),
            _ => None,
        },
        _ => None,
    }
}

fn fold_cmp(op: CmpOp, l: Option<&ConstVal>, r: Option<&ConstVal>) -> Option<ConstVal> {
    let b = match (l?, r?) {
        (ConstVal::Int(a), ConstVal::Int(b)) => match op {
            CmpOp::Eq => a == b, CmpOp::Ne => a != b,
            CmpOp::Lt => a <  b, CmpOp::Gt => a >  b,
            CmpOp::Le => a <= b, CmpOp::Ge => a >= b,
        },
        (ConstVal::Float(a), ConstVal::Float(b)) => match op {
            CmpOp::Eq => a == b, CmpOp::Ne => a != b,
            CmpOp::Lt => a <  b, CmpOp::Gt => a >  b,
            CmpOp::Le => a <= b, CmpOp::Ge => a >= b,
        },
        _ => return None,
    };
    Some(ConstVal::Bool(b))
}

fn fold_unary(op: UnaryOp, v: Option<&ConstVal>) -> Option<ConstVal> {
    match (op, v?) {
        (UnaryOp::Neg, ConstVal::Int(n))    => Some(ConstVal::Int(-n)),
        (UnaryOp::Neg, ConstVal::Float(f))  => Some(ConstVal::Float(-f)),
        (UnaryOp::Not, ConstVal::Bool(b))   => Some(ConstVal::Bool(!b)),
        (UnaryOp::BitNot, ConstVal::Int(n)) => Some(ConstVal::Int(!n)),
        _ => None,
    }
}

fn fold_cast(v: Option<&ConstVal>, ty: &Type) -> Option<ConstVal> {
    match (v?, ty) {
        (ConstVal::Int(n), Type::F64)  => Some(ConstVal::Float(*n as f64)),
        (ConstVal::Float(f), Type::I64) => Some(ConstVal::Int(*f as i64)),
        (ConstVal::Int(n), Type::Bool)  => Some(ConstVal::Bool(*n != 0)),
        (ConstVal::Bool(b), Type::I64)  => Some(ConstVal::Int(if *b { 1 } else { 0 })),
        _ => None,
    }
}

// ━━━━━━━━━━━━━━━━━━ Copy Propagation ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Replace uses of `Copy` with the original value (transitively).
pub fn copy_propagation(func: &mut Function) -> bool {
    let mut copies: HashMap<ValueId, ValueId> = HashMap::new();
    for bb in &func.blocks {
        for inst in &bb.insts {
            if let (Some(d), InstKind::Copy(src)) = (inst.dest, &inst.kind) {
                copies.insert(d, *src);
            }
        }
    }
    if copies.is_empty() { return false; }

    // Resolve transitive chains
    let resolved: HashMap<ValueId, ValueId> = copies.keys().map(|&k| {
        let mut v = k;
        let mut seen = HashSet::new();
        while let Some(&next) = copies.get(&v) {
            if !seen.insert(v) { break; }
            v = next;
        }
        (k, v)
    }).collect();

    let mut changed = false;
    for bb in &mut func.blocks {
        for phi in &mut bb.phis {
            for (_, v) in &mut phi.incoming {
                if let Some(&r) = resolved.get(v) { *v = r; changed = true; }
            }
        }
        for inst in &mut bb.insts { changed |= subst_inst(inst, &resolved); }
        changed |= subst_term(&mut bb.terminator, &resolved);
        // Remove dead Copy instructions whose dests have been resolved away.
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

// ━━━━━━━━━━━━━━━━━━ Phi Simplification ━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Simplify trivial phi nodes:
///  - All incoming values identical → replace with that value
///  - Single predecessor → replace with the one value
pub fn simplify_phis(func: &mut Function) -> bool {
    let mut replacements: HashMap<ValueId, ValueId> = HashMap::new();

    for bb in &func.blocks {
        for phi in &bb.phis {
            let unique: HashSet<ValueId> = phi.incoming.iter()
                .map(|(_, v)| *v)
                .filter(|v| *v != phi.dest) // ignore self-references
                .collect();
            if unique.len() == 1 {
                replacements.insert(phi.dest, *unique.iter().next().unwrap());
            } else if unique.is_empty() {
                // All incoming values are self-references — unreachable phi, skip.
                continue;
            }
        }
    }

    if replacements.is_empty() { return false; }

    // Resolve transitive chains: if v9 → v_inner and v_inner → v2,
    // replace v9 → v2 directly to avoid dangling references.
    let resolved: HashMap<ValueId, ValueId> = replacements.keys().map(|&k| {
        let mut v = k;
        let mut seen = HashSet::new();
        while let Some(&next) = replacements.get(&v) {
            if !seen.insert(v) { break; }
            v = next;
        }
        (k, v)
    }).collect();

    for bb in &mut func.blocks {
        bb.phis.retain(|phi| !resolved.contains_key(&phi.dest));
        for inst in &mut bb.insts { subst_inst(inst, &resolved); }
        subst_term(&mut bb.terminator, &resolved);
        for phi in &mut bb.phis {
            for (_, v) in &mut phi.incoming {
                if let Some(&r) = resolved.get(v) { *v = r; }
            }
        }
    }
    true
}

// ━━━━━━━━━━━━━━━━ Dead Code Elimination ━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Remove pure instructions whose results are never used.
pub fn dead_code_elimination(func: &mut Function) -> bool {
    let used = collect_used(func);
    let mut changed = false;
    for bb in &mut func.blocks {
        let before = bb.insts.len();
        bb.insts.retain(|inst| {
            inst.dest.map_or(true, |d| used.contains(&d) || !is_pure(&inst.kind))
        });
        if bb.insts.len() != before { changed = true; }
    }
    changed
}

// ━━━━━━━━━━━━━━━━ Strength Reduction ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Replace expensive operations with cheaper equivalents.
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

    // Pre-create shift constants needed for power-of-two multiplication rewrites.
    // Scan for x * 2^k and ensure a constant for k exists.
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
    // Build a map from shift amount → ValueId.
    // Always insert at entry block start to guarantee dominance over all uses.
    let mut shift_vals: HashMap<i64, ValueId> = HashMap::new();
    for shift in needed_shifts {
        if shift_vals.contains_key(&shift) { continue; }
        let d = func.new_value();
        let entry = func.entry;
        func.block_mut(entry).insts.insert(0, Instruction {
            dest: Some(d), kind: InstKind::IntConst(shift),
            ty: Type::I64, span: Span::dummy(), def_id: None,
        });
        shift_vals.insert(shift, d);
    }

    for bb in &mut func.blocks {
        for inst in &mut bb.insts {
            let new_kind = match &inst.kind {
                // x * 2^k → x << k
                InstKind::BinOp(BinOp::Mul, l, r) => match (iconsts.get(l), iconsts.get(r)) {
                    (_, Some(n)) if *n > 0 && (*n as u64).is_power_of_two() => {
                        let shift = (*n as u64).trailing_zeros() as i64;
                        if shift == 0 { Some(InstKind::Copy(*l)) }
                        else { Some(InstKind::BinOp(BinOp::Shl, *l, shift_vals[&shift])) }
                    }
                    (Some(n), _) if *n > 0 && (*n as u64).is_power_of_two() => {
                        let shift = (*n as u64).trailing_zeros() as i64;
                        if shift == 0 { Some(InstKind::Copy(*r)) }
                        else { Some(InstKind::BinOp(BinOp::Shl, *r, shift_vals[&shift])) }
                    }
                    // x * 0 → 0 (IntConst(0) is correct for all int widths
                    // because codegen uses inst.ty to determine the constant width)
                    (_, Some(0)) | (Some(0), _) => Some(InstKind::IntConst(0)),
                    _ => None,
                },
                // x + 0 | x - 0 → copy x
                InstKind::BinOp(BinOp::Add, l, r) if iconsts.get(r) == Some(&0) => Some(InstKind::Copy(*l)),
                InstKind::BinOp(BinOp::Add, l, r) if iconsts.get(l) == Some(&0) => Some(InstKind::Copy(*r)),
                InstKind::BinOp(BinOp::Sub, l, r) if iconsts.get(r) == Some(&0) => Some(InstKind::Copy(*l)),
                // x / 1 → copy x
                InstKind::BinOp(BinOp::Div, l, r) if iconsts.get(r) == Some(&1) => Some(InstKind::Copy(*l)),
                // x % 1 → 0
                InstKind::BinOp(BinOp::Mod, _, r) if iconsts.get(r) == Some(&1) => Some(InstKind::IntConst(0)),
                // x & 0 → 0, x | 0 → copy x, x ^ 0 → copy x
                InstKind::BinOp(BinOp::BitAnd, _, r) if iconsts.get(r) == Some(&0) => Some(InstKind::IntConst(0)),
                InstKind::BinOp(BinOp::BitOr, l, r) if iconsts.get(r) == Some(&0) => Some(InstKind::Copy(*l)),
                InstKind::BinOp(BinOp::BitXor, l, r) if iconsts.get(r) == Some(&0) => Some(InstKind::Copy(*l)),
                // x - x → 0  (same value id)
                InstKind::BinOp(BinOp::Sub, l, r) if l == r => Some(InstKind::IntConst(0)),
                // x ^ x → 0
                InstKind::BinOp(BinOp::BitXor, l, r) if l == r => Some(InstKind::IntConst(0)),
                _ => None,
            };
            if let Some(k) = new_kind { inst.kind = k; changed = true; }
        }
    }
    changed
}

// ━━━━━━━━━━━━━━ Store–Load Forwarding ━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Within each basic block, track the last stored/loaded value per variable.
/// - After `Store(name, val)`, a subsequent `Load(name)` can use `val` directly.
/// - After `Load(name) → v1`, a subsequent `Load(name)` can use `v1` directly.
/// Calls invalidate all forwarding state (conservative).
/// Forwarded loads are removed inline so no dead loads remain.
pub fn store_load_forwarding(func: &mut Function) -> bool {
    let mut replacements: HashMap<ValueId, ValueId> = HashMap::new();
    let mut dead_loads: HashSet<ValueId> = HashSet::new();

    for bb in &func.blocks {
        // Skip blocks that loop back to themselves (self-loops).
        // In such blocks, a Store at the end of the iteration updates
        // the value that a Load at the beginning of the next iteration
        // should see — forwarding would incorrectly use the initial value.
        if bb.terminator.successors().contains(&bb.id) {
            continue;
        }

        let mut known: HashMap<String, ValueId> = HashMap::new();

        for inst in &bb.insts {
            match &inst.kind {
                InstKind::Store(name, val) => {
                    known.insert(name.clone(), *val);
                }
                InstKind::Load(name) => {
                    if let Some(&val) = known.get(name) {
                        if let Some(dest) = inst.dest {
                            replacements.insert(dest, val);
                            dead_loads.insert(dest);
                        }
                    } else if let Some(dest) = inst.dest {
                        known.insert(name.clone(), dest);
                    }
                }
                InstKind::Call(..) | InstKind::MethodCall(..) | InstKind::ChanSend(..) | InstKind::ChanRecv(..)
                | InstKind::SelectArm(..) | InstKind::Log(..) => {
                    known.clear();
                }
                InstKind::FieldStore(var_name, _, _) => {
                    // Mutating a field of a variable invalidates its cached Load.
                    known.remove(var_name);
                }
                InstKind::IndexStore(var_name, _, _) => {
                    known.remove(var_name);
                }
                _ => {}
            }
        }
    }

    if replacements.is_empty() { return false; }

    for bb in &mut func.blocks {
        for phi in &mut bb.phis {
            for (_, v) in &mut phi.incoming {
                if let Some(&r) = replacements.get(v) { *v = r; }
            }
        }
        for inst in &mut bb.insts { subst_inst(inst, &replacements); }
        subst_term(&mut bb.terminator, &replacements);
        // Remove dead loads (whose values were forwarded).
        bb.insts.retain(|inst| {
            !inst.dest.map_or(false, |d| dead_loads.contains(&d))
        });
    }
    true
}

/// Remove stores that are overwritten before being read.
/// Within each basic block, if `Store(name, v1)` is followed by
/// `Store(name, v2)` with no intervening `Load(name)`, the first store
/// is dead.
pub fn redundant_store_elimination(func: &mut Function) -> bool {
    let mut changed = false;

    for bb in &mut func.blocks {
        let mut to_remove: HashSet<usize> = HashSet::new();
        // Maps variable name → index of last Store instruction.
        let mut last_store_idx: HashMap<String, usize> = HashMap::new();

        for (i, inst) in bb.insts.iter().enumerate() {
            match &inst.kind {
                InstKind::Store(name, _) => {
                    if let Some(prev_idx) = last_store_idx.insert(name.clone(), i) {
                        to_remove.insert(prev_idx);
                    }
                }
                InstKind::Load(name) => {
                    // A load reads the stored value — the store is live.
                    last_store_idx.remove(name);
                }
                InstKind::Call(..) | InstKind::ChanSend(..) | InstKind::ChanRecv(..)
                | InstKind::SelectArm(..) | InstKind::Log(..) => {
                    // Conservative: calls/effects might observe memory.
                    last_store_idx.clear();
                }
                _ => {}
            }
        }

        if !to_remove.is_empty() {
            let mut idx = 0;
            bb.insts.retain(|_| {
                let keep = !to_remove.contains(&idx);
                idx += 1;
                keep
            });
            changed = true;
        }
    }
    changed
}

/// Replace branches on constant booleans with unconditional gotos,
/// making the dead successor potentially unreachable.
pub fn constant_branch_elimination(func: &mut Function) -> bool {
    let mut consts: HashMap<ValueId, bool> = HashMap::new();
    for bb in &func.blocks {
        for inst in &bb.insts {
            if let (Some(d), InstKind::BoolConst(b)) = (inst.dest, &inst.kind) {
                consts.insert(d, *b);
            }
        }
    }
    let mut changed = false;
    for bb in &mut func.blocks {
        if let Terminator::Branch(cond, then_bb, else_bb) = &bb.terminator {
            if let Some(&b) = consts.get(cond) {
                let target = if b { *then_bb } else { *else_bb };
                bb.terminator = Terminator::Goto(target);
                changed = true;
            }
        }
    }
    changed
}

// ━━━━━━━━━━━━━━━ Global Value Numbering ━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Eliminate redundant computations by expression hashing.
/// Uses local value numbering (per-block) to avoid dominance issues.
pub fn global_value_numbering(func: &mut Function) -> bool {
    let mut replacements: HashMap<ValueId, ValueId> = HashMap::new();

    for bb in &func.blocks {
        let mut expr_map: HashMap<String, ValueId> = HashMap::new();
        for inst in &bb.insts {
            // Invalidate cached FieldGet/Index entries on mutation.
            match &inst.kind {
                InstKind::FieldSet(_, _, _) | InstKind::FieldStore(_, _, _) => {
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
                if !is_pure(&inst.kind) { continue; }
                if let Some(key) = gvn_key(&inst.kind) {
                    if let Some(&existing) = expr_map.get(&key) {
                        replacements.insert(d, existing);
                    } else {
                        expr_map.insert(key, d);
                    }
                }
            }
        }
    }
    if replacements.is_empty() { return false; }

    for bb in &mut func.blocks {
        for phi in &mut bb.phis {
            for (_, v) in &mut phi.incoming {
                if let Some(&r) = replacements.get(v) { *v = r; }
            }
        }
        for inst in &mut bb.insts { subst_inst(inst, &replacements); }
        subst_term(&mut bb.terminator, &replacements);
    }
    true
}

/// Canonical string key for an expression. Commutative ops have normalized operand order.
fn gvn_key(kind: &InstKind) -> Option<String> {
    match kind {
        InstKind::BinOp(op, l, r) => {
            let (a, b) = if is_commutative(*op) && l.0 > r.0 { (r, l) } else { (l, r) };
            Some(format!("bin:{op:?}:{},{}", a.0, b.0))
        }
        InstKind::Cmp(op, l, r, _) => {
            let (a, b) = if matches!(op, CmpOp::Eq | CmpOp::Ne) && l.0 > r.0 { (r, l) } else { (l, r) };
            Some(format!("cmp:{op:?}:{},{}", a.0, b.0))
        }
        InstKind::UnaryOp(op, v)  => Some(format!("un:{op:?}:{}", v.0)),
        InstKind::FieldGet(o, f)  => Some(format!("fg:{}:{f}", o.0)),
        InstKind::Index(a, i)     => Some(format!("ix:{}:{}", a.0, i.0)),
        InstKind::Cast(v, ty)     => Some(format!("cast:{}:{ty:?}", v.0)),
        InstKind::StrictCast(v, ty) => Some(format!("scast:{}:{ty:?}", v.0)),
        _ => None,
    }
}

fn is_commutative(op: BinOp) -> bool {
    matches!(op, BinOp::Add | BinOp::Mul | BinOp::BitAnd | BinOp::BitOr
              | BinOp::BitXor | BinOp::And | BinOp::Or)
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
    let blocks_snapshot: Vec<(BlockId, Terminator)> = func.blocks.iter()
        .map(|bb| (bb.id, bb.terminator.clone()))
        .collect();

    for (bb_id, term) in &blocks_snapshot {
        if let Terminator::Branch(cond, then_bb, else_bb) = term {
            if let Some(incoming) = phi_vals.get(&(*bb_id, *cond)) {
                for (pred_id, val) in incoming {
                    if let Some(&b) = consts.get(val) {
                        let target = if b { *then_bb } else { *else_bb };
                        // Redirect predecessor's terminator from bb_id to target
                        func.block_mut(*pred_id).terminator.replace_successor(*bb_id, target);
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
    let block_index: HashMap<BlockId, usize> = block_ids.iter().enumerate()
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
                    let header_has_body_succ = func.block(succ)
                        .terminator
                        .successors()
                        .iter()
                        .any(|s| block_index.get(s).map_or(false, |&si| body.contains(&si)));
                    // Additional check: verify all body blocks are reachable from
                    // the header within the body (validates natural loop structure).
                    let header_has_exit = func.block(succ)
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
        let preheader_preds: Vec<BlockId> = header_preds.into_iter()
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
                if !is_pure(&inst.kind) { continue; }
                let Some(dest) = inst.dest else { continue; };

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

        if to_hoist.is_empty() { continue; }

        // Remove hoisted instructions from their original blocks.
        let hoisted_ids: HashSet<ValueId> = to_hoist.iter()
            .filter_map(|i| i.dest)
            .collect();
        for &bi in body {
            func.blocks[bi].insts.retain(|i| {
                i.dest.map_or(true, |d| !hoisted_ids.contains(&d))
            });
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
fn collect_inst_operands(kind: &InstKind) -> Vec<ValueId> {
    let mut s = HashSet::new();
    collect_inst_uses(kind, &mut s);
    s.into_iter().collect()
}

// ━━━━━━━━━━━━━━ Block Merging ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Merge a block B into its sole predecessor A when:
///  - A ends with `Goto(B)` and B has exactly one predecessor (A).
pub fn merge_linear_blocks(func: &mut Function) -> bool {
    let mut changed = false;

    loop {
        let mut merged_any = false;
        let pred_map = func.predecessors();

        for i in 0..func.blocks.len() {
            let bb_id = func.blocks[i].id;
            if bb_id == func.entry { continue; }

            let pred_list = pred_map.get(&bb_id).cloned().unwrap_or_default();
            if pred_list.len() != 1 { continue; }
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
                let val = phi.incoming.iter()
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
                if other_bb.id == bb_id { continue; }
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
        if !merged_any { break; }
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
        if !reachable.insert(id) { continue; }
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

// ━━━━━━━━━━━━━━ Value Range Analysis ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Integer value range (inclusive).
#[derive(Debug, Clone, Copy)]
pub struct IntRange { pub lo: i64, pub hi: i64 }

impl IntRange {
    pub const FULL: Self = Self { lo: i64::MIN, hi: i64::MAX };
    pub fn constant(n: i64) -> Self { Self { lo: n, hi: n } }
    pub fn is_non_negative(&self) -> bool { self.lo >= 0 }

    pub fn intersect(self, other: Self) -> Option<Self> {
        let lo = self.lo.max(other.lo);
        let hi = self.hi.min(other.hi);
        if lo <= hi { Some(Self { lo, hi }) } else { None }
    }
}

/// Compute simple integer ranges for forward dataflow.
pub fn compute_ranges(func: &Function) -> HashMap<ValueId, IntRange> {
    let mut ranges: HashMap<ValueId, IntRange> = HashMap::new();
    for bb in &func.blocks {
        for inst in &bb.insts {
            if let Some(d) = inst.dest {
                let r = match &inst.kind {
                    InstKind::IntConst(n) => Some(IntRange::constant(*n)),
                    InstKind::BinOp(BinOp::Add, l, r) => {
                        match (ranges.get(l), ranges.get(r)) {
                            (Some(a), Some(b)) => Some(IntRange {
                                lo: a.lo.saturating_add(b.lo),
                                hi: a.hi.saturating_add(b.hi),
                            }),
                            _ => None,
                        }
                    }
                    InstKind::BinOp(BinOp::Mul, l, r) => {
                        match (ranges.get(l), ranges.get(r)) {
                            (Some(a), Some(b)) => {
                                let ps = [
                                    a.lo.saturating_mul(b.lo), a.lo.saturating_mul(b.hi),
                                    a.hi.saturating_mul(b.lo), a.hi.saturating_mul(b.hi),
                                ];
                                Some(IntRange {
                                    lo: *ps.iter().min().unwrap(),
                                    hi: *ps.iter().max().unwrap(),
                                })
                            }
                            _ => None,
                        }
                    }
                    InstKind::BinOp(BinOp::Sub, l, r) => {
                        match (ranges.get(l), ranges.get(r)) {
                            (Some(a), Some(b)) => Some(IntRange {
                                lo: a.lo.saturating_sub(b.hi),
                                hi: a.hi.saturating_sub(b.lo),
                            }),
                            _ => None,
                        }
                    }
                    InstKind::BinOp(BinOp::BitAnd, l, r) => {
                        // If both non-negative, result ≤ min(a.hi, b.hi)
                        match (ranges.get(l), ranges.get(r)) {
                            (Some(a), Some(b)) if a.is_non_negative() && b.is_non_negative() => {
                                Some(IntRange { lo: 0, hi: a.hi.min(b.hi) })
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(range) = r { ranges.insert(d, range); }
            }
        }
    }
    ranges
}
