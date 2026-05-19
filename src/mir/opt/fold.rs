use super::super::*;
use crate::types::Type;
use std::collections::HashMap;

pub(super) enum ConstVal {
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl ConstVal {
    fn to_inst(&self) -> InstKind {
        match self {
            ConstVal::Int(n) => InstKind::IntConst(*n),
            ConstVal::Float(f) => InstKind::FloatConst(*f),
            ConstVal::Bool(b) => InstKind::BoolConst(*b),
        }
    }
}

pub fn constant_fold(func: &mut Function) -> bool {
    let mut changed = false;
    let mut consts: HashMap<ValueId, ConstVal> = HashMap::new();

    for bb in &func.blocks {
        for inst in &bb.insts {
            if let Some(d) = inst.dest {
                match &inst.kind {
                    InstKind::IntConst(n) => {
                        consts.insert(d, ConstVal::Int(*n));
                    }
                    InstKind::FloatConst(f) => {
                        consts.insert(d, ConstVal::Float(*f));
                    }
                    InstKind::BoolConst(b) => {
                        consts.insert(d, ConstVal::Bool(*b));
                    }
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
                    InstKind::Cmp(op, l, r, _) => fold_cmp(*op, consts.get(l), consts.get(r)),
                    InstKind::UnaryOp(op, v) => fold_unary(*op, consts.get(v)),
                    InstKind::Cast(v, ty) => fold_cast(consts.get(v), ty),
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
                BinOp::BitOr => a | b,
                BinOp::BitXor => a ^ b,
                BinOp::Shl => a.checked_shl(*b as u32)?,
                BinOp::Shr => a.checked_shr(*b as u32)?,
                BinOp::Ushr => (*a as u64).checked_shr(*b as u32)? as i64,
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
            BinOp::Or => Some(ConstVal::Bool(*a || *b)),
            _ => None,
        },
        _ => None,
    }
}

fn fold_cmp(op: CmpOp, l: Option<&ConstVal>, r: Option<&ConstVal>) -> Option<ConstVal> {
    let b = match (l?, r?) {
        (ConstVal::Int(a), ConstVal::Int(b)) => match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Gt => a > b,
            CmpOp::Le => a <= b,
            CmpOp::Ge => a >= b,
        },
        (ConstVal::Float(a), ConstVal::Float(b)) => match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Gt => a > b,
            CmpOp::Le => a <= b,
            CmpOp::Ge => a >= b,
        },
        _ => return None,
    };
    Some(ConstVal::Bool(b))
}

fn fold_unary(op: UnaryOp, v: Option<&ConstVal>) -> Option<ConstVal> {
    match (op, v?) {
        (UnaryOp::Neg, ConstVal::Int(n)) => Some(ConstVal::Int(-n)),
        (UnaryOp::Neg, ConstVal::Float(f)) => Some(ConstVal::Float(-f)),
        (UnaryOp::Not, ConstVal::Bool(b)) => Some(ConstVal::Bool(!b)),
        (UnaryOp::BitNot, ConstVal::Int(n)) => Some(ConstVal::Int(!n)),
        _ => None,
    }
}

fn fold_cast(v: Option<&ConstVal>, ty: &Type) -> Option<ConstVal> {
    match (v?, ty) {
        (ConstVal::Int(n), Type::F64) => Some(ConstVal::Float(*n as f64)),
        (ConstVal::Float(f), Type::I64) => Some(ConstVal::Int(*f as i64)),
        (ConstVal::Int(n), Type::Bool) => Some(ConstVal::Bool(*n != 0)),
        (ConstVal::Bool(b), Type::I64) => Some(ConstVal::Int(if *b { 1 } else { 0 })),
        _ => None,
    }
}
