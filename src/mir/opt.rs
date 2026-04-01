//! MIR optimization passes.
//!
//! Each pass takes `&mut Function` and returns `true` if it changed anything.
//! The driver runs passes in a fixed-point loop until convergence.

use std::collections::{HashMap, HashSet, VecDeque};
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
    for _ in 0..max {
        let mut changed = false;
        changed |= constant_fold(func);
        changed |= copy_propagation(func);
        changed |= simplify_phis(func);
        changed |= dead_code_elimination(func);
        changed |= strength_reduction(func);
        if level == OptLevel::Full {
            changed |= global_value_numbering(func);
            changed |= branch_threading(func);
        }
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
        InstKind::BinOp(_, a, b) | InstKind::Cmp(_, a, b) => { sub!(a); sub!(b); }
        InstKind::UnaryOp(_, v) | InstKind::Cast(v, _) | InstKind::Ref(v)
        | InstKind::Deref(v) | InstKind::Copy(v) | InstKind::RcInc(v)
        | InstKind::RcDec(v) | InstKind::Alloc(v) => { sub!(v); }
        InstKind::Drop(v, _) => { sub!(v); }
        InstKind::Call(_, args) | InstKind::ArrayInit(args)
        | InstKind::VariantInit(_, _, _, args) => { for a in args { sub!(a); } }
        InstKind::MethodCall(obj, _, args) => { sub!(obj); for a in args { sub!(a); } }
        InstKind::IndirectCall(f, args) => { sub!(f); for a in args { sub!(a); } }
        InstKind::FieldGet(o, _) => { sub!(o); }
        InstKind::FieldSet(o, _, v) => { sub!(o); sub!(v); }
        InstKind::Index(a, i) => { sub!(a); sub!(i); }
        InstKind::IndexSet(a, i, v) => { sub!(a); sub!(i); sub!(v); }
        InstKind::StructInit(_, fs) => { for (_, v) in fs { sub!(v); } }
        InstKind::Slice(a, s, e) => { sub!(a); sub!(s); sub!(e); }
        InstKind::Store(_, v) => { sub!(v); }
        InstKind::IntConst(_) | InstKind::FloatConst(_) | InstKind::BoolConst(_)
        | InstKind::StringConst(_) | InstKind::Void | InstKind::Load(_) => {}
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
        InstKind::BinOp(_, a, b) | InstKind::Cmp(_, a, b) => { s.insert(*a); s.insert(*b); }
        InstKind::UnaryOp(_, v) | InstKind::Cast(v, _) | InstKind::Ref(v)
        | InstKind::Deref(v) | InstKind::Copy(v) | InstKind::RcInc(v)
        | InstKind::RcDec(v) | InstKind::Alloc(v) => { s.insert(*v); }
        InstKind::Drop(v, _) => { s.insert(*v); }
        InstKind::Call(_, args) | InstKind::ArrayInit(args)
        | InstKind::VariantInit(_, _, _, args) => { for a in args { s.insert(*a); } }
        InstKind::MethodCall(obj, _, args) => { s.insert(*obj); for a in args { s.insert(*a); } }
        InstKind::IndirectCall(f, args) => { s.insert(*f); for a in args { s.insert(*a); } }
        InstKind::FieldGet(o, _) => { s.insert(*o); }
        InstKind::FieldSet(o, _, v) => { s.insert(*o); s.insert(*v); }
        InstKind::Index(a, i) => { s.insert(*a); s.insert(*i); }
        InstKind::IndexSet(a, i, v) => { s.insert(*a); s.insert(*i); s.insert(*v); }
        InstKind::StructInit(_, fs) => { for (_, v) in fs { s.insert(*v); } }
        InstKind::Slice(a, lo, hi) => { s.insert(*a); s.insert(*lo); s.insert(*hi); }
        InstKind::Store(_, v) => { s.insert(*v); }
        InstKind::IntConst(_) | InstKind::FloatConst(_) | InstKind::BoolConst(_)
        | InstKind::StringConst(_) | InstKind::Void | InstKind::Load(_) => {}
    }
}

fn collect_term_uses(term: &Terminator, s: &mut HashSet<ValueId>) {
    match term {
        Terminator::Branch(c, _, _) | Terminator::Switch(c, _, _) => { s.insert(*c); }
        Terminator::Return(Some(v)) => { s.insert(*v); }
        _ => {}
    }
}

/// Returns `true` if an instruction is side-effect-free.
fn is_pure(kind: &InstKind) -> bool {
    matches!(kind,
        InstKind::IntConst(_) | InstKind::FloatConst(_) | InstKind::BoolConst(_)
        | InstKind::StringConst(_) | InstKind::Void
        | InstKind::BinOp(..) | InstKind::UnaryOp(..) | InstKind::Cmp(..)
        | InstKind::Cast(..) | InstKind::Copy(..) | InstKind::Load(_)
        | InstKind::FieldGet(..) | InstKind::Index(..) | InstKind::Ref(..) | InstKind::Deref(..)
        | InstKind::ArrayInit(_) | InstKind::StructInit(..))
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
                    InstKind::Cmp(op, l, r)   => fold_cmp(*op, consts.get(l), consts.get(r)),
                    InstKind::UnaryOp(op, v)   => fold_unary(*op, consts.get(v)),
                    InstKind::Cast(v, ty)      => fold_cast(consts.get(v), ty),
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
                BinOp::Add => a.checked_add(*b)?,
                BinOp::Sub => a.checked_sub(*b)?,
                BinOp::Mul => a.checked_mul(*b)?,
                BinOp::Div if *b != 0 => a.checked_div(*b)?,
                BinOp::Mod if *b != 0 => a.checked_rem(*b)?,
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
            }
        }
    }

    if replacements.is_empty() { return false; }

    for bb in &mut func.blocks {
        bb.phis.retain(|phi| !replacements.contains_key(&phi.dest));
        for inst in &mut bb.insts { subst_inst(inst, &replacements); }
        subst_term(&mut bb.terminator, &replacements);
        for phi in &mut bb.phis {
            for (_, v) in &mut phi.incoming {
                if let Some(&r) = replacements.get(v) { *v = r; }
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
    // Build a map from shift amount → ValueId
    let mut shift_vals: HashMap<i64, ValueId> = HashMap::new();
    for shift in needed_shifts {
        if shift_vals.contains_key(&shift) { continue; }
        // Look for existing constant first
        let existing = func.blocks.iter().flat_map(|bb| &bb.insts)
            .find_map(|inst| match (&inst.dest, &inst.kind) {
                (Some(d), InstKind::IntConst(n)) if *n == shift => Some(*d),
                _ => None,
            });
        let vid = existing.unwrap_or_else(|| {
            let d = func.new_value();
            let entry = func.entry;
            func.block_mut(entry).insts.insert(0, Instruction {
                dest: Some(d), kind: InstKind::IntConst(shift),
                ty: Type::I64, span: Span::dummy(),
            });
            d
        });
        shift_vals.insert(shift, vid);
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

// ━━━━━━━━━━━━━━━ Global Value Numbering ━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Eliminate redundant computations by expression hashing.
pub fn global_value_numbering(func: &mut Function) -> bool {
    let mut expr_map: HashMap<String, ValueId> = HashMap::new();
    let mut replacements: HashMap<ValueId, ValueId> = HashMap::new();

    for bb in &func.blocks {
        for inst in &bb.insts {
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
        InstKind::Cmp(op, l, r) => Some(format!("cmp:{op:?}:{},{}", l.0, r.0)),
        InstKind::UnaryOp(op, v)  => Some(format!("un:{op:?}:{}", v.0)),
        InstKind::FieldGet(o, f)  => Some(format!("fg:{}:{f}", o.0)),
        InstKind::Index(a, i)     => Some(format!("ix:{}:{}", a.0, i.0)),
        InstKind::Cast(v, ty)     => Some(format!("cast:{}:{ty:?}", v.0)),
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

            // Merge: append B's instructions and terminator to A, remove B
            let b_insts = func.block(bb_id).insts.clone();
            let b_term = func.block(bb_id).terminator.clone();

            let pred_block = func.block_mut(pred_id);
            pred_block.insts.extend(b_insts);
            pred_block.terminator = b_term;

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
    func.blocks.len() != before
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
