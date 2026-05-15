//! Bottom-up constant folder over the HIR.

use super::eval::try_eval_pure_call;
use crate::ast::{BinOp, Span, UnaryOp};
use crate::hir::{self, Block, Expr, ExprKind, Stmt};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::HashMap;

pub(super) fn fold_block_with_fns(block: &mut Block, pure_fns: &HashMap<Symbol, hir::Fn>) {
    for stmt in block.iter_mut() {
        fold_stmt_with_fns(stmt, pure_fns);
    }
}

pub(super) fn fold_stmt_with_fns(stmt: &mut Stmt, pure_fns: &HashMap<Symbol, hir::Fn>) {
    match stmt {
        Stmt::Bind(bind) => fold_expr_with_fns(&mut bind.value, pure_fns),
        Stmt::TupleBind(_, e, _) => fold_expr_with_fns(e, pure_fns),
        Stmt::Assign(lhs, rhs, _) => {
            fold_expr_with_fns(lhs, pure_fns);
            fold_expr_with_fns(rhs, pure_fns);
        }
        Stmt::Expr(e) => fold_expr_with_fns(e, pure_fns),
        Stmt::If(i) => {
            fold_expr_with_fns(&mut i.cond, pure_fns);
            fold_block_with_fns(&mut i.then, pure_fns);
            for (c, b) in &mut i.elifs {
                fold_expr_with_fns(c, pure_fns);
                fold_block_with_fns(b, pure_fns);
            }
            if let Some(b) = &mut i.els {
                fold_block_with_fns(b, pure_fns);
            }
        }
        Stmt::While(w) => {
            fold_expr_with_fns(&mut w.cond, pure_fns);
            fold_block_with_fns(&mut w.body, pure_fns);
        }
        Stmt::For(f) => {
            fold_expr_with_fns(&mut f.iter, pure_fns);
            if let Some(e) = &mut f.end {
                fold_expr_with_fns(e, pure_fns);
            }
            if let Some(e) = &mut f.step {
                fold_expr_with_fns(e, pure_fns);
            }
            fold_block_with_fns(&mut f.body, pure_fns);
        }
        Stmt::Loop(l) => fold_block_with_fns(&mut l.body, pure_fns),
        Stmt::Ret(Some(e), _, _) => fold_expr_with_fns(e, pure_fns),
        Stmt::Break(Some(e), _) => fold_expr_with_fns(e, pure_fns),
        Stmt::Match(m) => {
            fold_expr_with_fns(&mut m.subject, pure_fns);
            for arm in &mut m.arms {
                fold_block_with_fns(&mut arm.body, pure_fns);
                if let Some(g) = &mut arm.guard {
                    fold_expr_with_fns(g, pure_fns);
                }
            }
        }
        Stmt::ErrReturn(e, _, _) => fold_expr_with_fns(e, pure_fns),
        Stmt::Defer(b, _) => fold_block_with_fns(b, pure_fns),
        Stmt::StoreInsert(_, exprs, _) => {
            for e in exprs {
                fold_expr_with_fns(e, pure_fns);
            }
        }
        Stmt::StoreSet(_, pairs, _, _) => {
            for (_, e) in pairs {
                fold_expr_with_fns(e, pure_fns);
            }
        }
        Stmt::Transaction(b, _) => fold_block_with_fns(b, pure_fns),
        Stmt::ChannelClose(e, _) => fold_expr_with_fns(e, pure_fns),
        Stmt::Stop(e, _) => fold_expr_with_fns(e, pure_fns),
        Stmt::SimFor(f, _) => {
            fold_expr_with_fns(&mut f.iter, pure_fns);
            if let Some(e) = &mut f.end {
                fold_expr_with_fns(e, pure_fns);
            }
            if let Some(e) = &mut f.step {
                fold_expr_with_fns(e, pure_fns);
            }
            fold_block_with_fns(&mut f.body, pure_fns);
        }
        Stmt::SimBlock(b, _) => fold_block_with_fns(b, pure_fns),
        Stmt::Drop(_, _, _, _)
        | Stmt::Continue(_)
        | Stmt::Nop(_)
        | Stmt::Ret(None, _, _)
        | Stmt::Break(None, _)
        | Stmt::Asm(_)
        | Stmt::StoreDelete(_, _, _)
        | Stmt::StoreDestroy(_, _, _)
        | Stmt::StoreRestore(_, _, _)
        | Stmt::StoreSave(_, _)
        | Stmt::UseLocal(_, _, _, _) => {}
        Stmt::GlobalStore(_, e, _) => fold_expr_with_fns(e, pure_fns),
    }
}

pub(super) fn fold_expr_with_fns(expr: &mut Expr, pure_fns: &HashMap<Symbol, hir::Fn>) {
    // First recurse into sub-expressions
    fold_expr(expr);

    // Then try pure function call evaluation
    if let ExprKind::Call(_, name, args) = &expr.kind {
        if args.iter().all(|a| {
            matches!(
                a.kind,
                ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_)
            )
        }) {
            if let Some(result) = try_eval_pure_call(&name.as_str(), args, pure_fns, 0) {
                *expr = result;
            }
        }
    }
}

pub(super) fn fold_block(block: &mut Block) {
    for stmt in block.iter_mut() {
        fold_stmt(stmt);
    }
}

pub(super) fn fold_stmt(stmt: &mut Stmt) {
    match stmt {
        Stmt::Bind(bind) => fold_expr(&mut bind.value),
        Stmt::TupleBind(_, e, _) => fold_expr(e),
        Stmt::Assign(lhs, rhs, _) => {
            fold_expr(lhs);
            fold_expr(rhs);
        }
        Stmt::Expr(e) => fold_expr(e),
        Stmt::If(i) => {
            fold_expr(&mut i.cond);
            fold_block(&mut i.then);
            for (c, b) in &mut i.elifs {
                fold_expr(c);
                fold_block(b);
            }
            if let Some(b) = &mut i.els {
                fold_block(b);
            }
        }
        Stmt::While(w) => {
            fold_expr(&mut w.cond);
            fold_block(&mut w.body);
        }
        Stmt::For(f) => {
            fold_expr(&mut f.iter);
            if let Some(e) = &mut f.end {
                fold_expr(e);
            }
            if let Some(e) = &mut f.step {
                fold_expr(e);
            }
            fold_block(&mut f.body);
        }
        Stmt::Loop(l) => fold_block(&mut l.body),
        Stmt::Ret(Some(e), _, _) => fold_expr(e),
        Stmt::Break(Some(e), _) => fold_expr(e),
        Stmt::Match(m) => {
            fold_expr(&mut m.subject);
            for arm in &mut m.arms {
                fold_block(&mut arm.body);
                if let Some(g) = &mut arm.guard {
                    fold_expr(g);
                }
            }
        }
        Stmt::ErrReturn(e, _, _) => fold_expr(e),
        Stmt::Defer(b, _) => fold_block(b),
        Stmt::Drop(_, _, _, _)
        | Stmt::Continue(_)
        | Stmt::Nop(_)
        | Stmt::Ret(None, _, _)
        | Stmt::Break(None, _)
        | Stmt::Asm(_)
        | Stmt::UseLocal(_, _, _, _) => {}
        Stmt::StoreInsert(_, exprs, _) => {
            for e in exprs {
                fold_expr(e);
            }
        }
        Stmt::StoreDelete(_, _, _) => {}
        Stmt::StoreDestroy(_, _, _) => {}
        Stmt::StoreRestore(_, _, _) => {}
        Stmt::StoreSave(_, _) => {}
        Stmt::StoreSet(_, pairs, _, _) => {
            for (_, e) in pairs {
                fold_expr(e);
            }
        }
        Stmt::Transaction(b, _) => fold_block(b),
        Stmt::ChannelClose(e, _) => fold_expr(e),
        Stmt::Stop(e, _) => fold_expr(e),
        Stmt::SimFor(f, _) => {
            fold_expr(&mut f.iter);
            if let Some(e) = &mut f.end {
                fold_expr(e);
            }
            if let Some(e) = &mut f.step {
                fold_expr(e);
            }
            fold_block(&mut f.body);
        }
        Stmt::SimBlock(b, _) => fold_block(b),
        Stmt::GlobalStore(_, e, _) => fold_expr(e),
    }
}

pub(super) fn fold_expr(expr: &mut Expr) {
    match &mut expr.kind {
        ExprKind::BinOp(l, _, r) => {
            fold_expr(l);
            fold_expr(r);
        }
        ExprKind::UnaryOp(_, e) => fold_expr(e),
        ExprKind::Call(_, _, args) => {
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::IndirectCall(f, args) => {
            fold_expr(f);
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::Builtin(_, args) => {
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::Method(obj, _, _, args) => {
            fold_expr(obj);
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::StringMethod(obj, _, args) | ExprKind::DeferredMethod(obj, _, args) => {
            fold_expr(obj);
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::VecMethod(obj, _, args) => {
            fold_expr(obj);
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::MapMethod(obj, _, args)
        | ExprKind::SetMethod(obj, _, args)
        | ExprKind::PQMethod(obj, _, args) => {
            fold_expr(obj);
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::VecNew(elems) => {
            for e in elems {
                fold_expr(e);
            }
        }
        ExprKind::Field(e, _, _) => fold_expr(e),
        ExprKind::Index(a, b) => {
            fold_expr(a);
            fold_expr(b);
        }
        ExprKind::Ternary(c, t, f) => {
            fold_expr(c);
            fold_expr(t);
            fold_expr(f);
        }
        ExprKind::Coerce(e, _) => fold_expr(e),
        ExprKind::Cast(e, _) => fold_expr(e),
        ExprKind::Array(elems) => {
            for e in elems {
                fold_expr(e);
            }
        }
        ExprKind::Tuple(elems) => {
            for e in elems {
                fold_expr(e);
            }
        }
        ExprKind::Struct(_, fields) | ExprKind::VariantCtor(_, _, _, fields) => {
            for f in fields {
                fold_expr(&mut f.value);
            }
        }
        ExprKind::IfExpr(i) => {
            fold_expr(&mut i.cond);
            fold_block(&mut i.then);
            for (c, b) in &mut i.elifs {
                fold_expr(c);
                fold_block(b);
            }
            if let Some(b) = &mut i.els {
                fold_block(b);
            }
        }
        ExprKind::Pipe(e, _, _, args) => {
            fold_expr(e);
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::Block(b) => fold_block(b),
        ExprKind::Lambda(_, b) => fold_block(b),
        ExprKind::Ref(e) | ExprKind::Deref(e) => fold_expr(e),
        ExprKind::ListComp(body, _, _, iter, end, cond) => {
            fold_expr(body);
            fold_expr(iter);
            if let Some(e) = end {
                fold_expr(e);
            }
            if let Some(c) = cond {
                fold_expr(c);
            }
        }
        ExprKind::Syscall(args) => {
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::Send(obj, _, _, _, args) => {
            fold_expr(obj);
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::ChannelCreate(_, e) => fold_expr(e),
        ExprKind::ChannelSend(c, v) => {
            fold_expr(c);
            fold_expr(v);
        }
        ExprKind::ChannelRecv(c) => fold_expr(c),
        ExprKind::Select(arms, default) => {
            for a in arms {
                fold_expr(&mut a.chan);
                if let Some(v) = &mut a.value {
                    fold_expr(v);
                }
                fold_block(&mut a.body);
            }
            if let Some(b) = default {
                fold_block(b);
            }
        }
        ExprKind::DynDispatch(e, _, _, args) => {
            fold_expr(e);
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::DynCoerce(e, _, _) => fold_expr(e),
        ExprKind::CoroutineCreate(_, stmts) => {
            for s in stmts {
                fold_stmt(s);
            }
        }
        ExprKind::CoroutineNext(e) | ExprKind::Yield(e) => fold_expr(e),
        ExprKind::Unreachable => {}
        ExprKind::StrictCast(e, _) | ExprKind::AsFormat(e, _) | ExprKind::AtomicLoad(e) => {
            fold_expr(e)
        }
        ExprKind::AtomicStore(a, b) | ExprKind::AtomicAdd(a, b) | ExprKind::AtomicSub(a, b) => {
            fold_expr(a);
            fold_expr(b);
        }
        ExprKind::AtomicCas(p, e, n) => {
            fold_expr(p);
            fold_expr(e);
            fold_expr(n);
        }
        ExprKind::Slice(obj, start, end) => {
            fold_expr(obj);
            fold_expr(start);
            fold_expr(end);
        }
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Str(_)
        | ExprKind::Bool(_)
        | ExprKind::None
        | ExprKind::Void
        | ExprKind::Var(_, _)
        | ExprKind::FnRef(_, _)
        | ExprKind::VariantRef(_, _, _)
        | ExprKind::MapNew
        | ExprKind::SetNew
        | ExprKind::PQNew
        | ExprKind::NDArrayNew(_)
        | ExprKind::SIMDNew(_)
        | ExprKind::Spawn(_, _)
        | ExprKind::GlobalLoad(_)
        | ExprKind::StoreQuery(_, _)
        | ExprKind::StoreCount(_)
        | ExprKind::StoreAll(_)
        | ExprKind::ViewCount(_, _)
        | ExprKind::ViewAll(_, _)
        | ExprKind::StoreGet(_, _)
        | ExprKind::StoreFirst(_, _)
        | ExprKind::StoreExists(_, _)
        | ExprKind::StoreDistinct(_, _)
        | ExprKind::StoreSum(_, _)
        | ExprKind::StoreAvg(_, _)
        | ExprKind::StoreMin(_, _)
        | ExprKind::StoreMax(_, _)
        | ExprKind::StoreVersionCount(_, _)
        | ExprKind::StoreHistory(_, _)
        | ExprKind::StoreAtVersion(_, _, _)
        | ExprKind::IterNext(_, _, _)
        | ExprKind::DequeNew => {}
        ExprKind::DequeMethod(obj, _, args) => {
            fold_expr(obj);
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::Grad(e)
        | ExprKind::CowWrap(e)
        | ExprKind::CowClone(e)
        | ExprKind::GeneratorNext(e)
        | ExprKind::EnumUnwrap(e, _, _)
        | ExprKind::EnumIs(e, _) => fold_expr(e),
        ExprKind::Einsum(_, args) => {
            for a in args {
                fold_expr(a);
            }
        }
        ExprKind::Builder(_, fields) => {
            for (_, v) in fields {
                fold_expr(v);
            }
        }
        ExprKind::GeneratorCreate(_, _, stmts) => {
            for s in stmts {
                fold_stmt(s);
            }
        }
        // KV / specialized store ops – fold inner exprs
        ExprKind::KvGet(_, e) | ExprKind::KvHas(_, e) | ExprKind::KvDel(_, e) => fold_expr(e),
        ExprKind::KvSet(_, k, v) | ExprKind::KvIncr(_, k, v) => {
            fold_expr(k);
            fold_expr(v);
        }
        ExprKind::KvCount(_) => {}
        ExprKind::VecNearest(_, v, k) => {
            fold_expr(v);
            fold_expr(k);
        }
        ExprKind::VecInsert(_, v) => fold_expr(v),
        ExprKind::VecCount(_) => {}
        ExprKind::BloomTest(_, _, v) => fold_expr(v),
        ExprKind::FtsSearch(_, _, v) => fold_expr(v),
        ExprKind::FtsCount(_, _) => {}
        ExprKind::GraphFrom(_, e) | ExprKind::GraphTo(_, e) => fold_expr(e),
        ExprKind::TsLatest(_) => {}
    }

    if let Some(folded) = try_fold(expr) {
        *expr = folded;
    }
}

pub(super) fn try_fold(expr: &Expr) -> Option<Expr> {
    let span = expr.span;
    let ty = expr.ty.clone();
    match &expr.kind {
        ExprKind::BinOp(l, op, r) => fold_binop(l, *op, r, ty, span),
        ExprKind::UnaryOp(op, e) => fold_unary(*op, e, ty, span),
        ExprKind::Ternary(c, t, f) => fold_ternary(c, t, f),
        ExprKind::Cast(e, to_ty) => fold_cast(e, to_ty, span),
        ExprKind::Builtin(builtin, args) => fold_builtin(builtin, args, ty, span),
        _ => None,
    }
}

pub(super) fn fold_binop(l: &Expr, op: BinOp, r: &Expr, ty: Type, span: Span) -> Option<Expr> {
    let kind = match (&l.kind, &r.kind) {
        (ExprKind::Int(a), ExprKind::Int(b)) => fold_int_op(*a, op, *b)?,
        (ExprKind::Float(a), ExprKind::Float(b)) => fold_float_op(*a, op, *b)?,
        (ExprKind::Bool(a), ExprKind::Bool(b)) => match op {
            BinOp::And => ExprKind::Bool(*a && *b),
            BinOp::Or => ExprKind::Bool(*a || *b),
            BinOp::Eq => ExprKind::Bool(a == b),
            BinOp::Ne => ExprKind::Bool(a != b),
            _ => return None,
        },
        (ExprKind::Str(a), ExprKind::Str(b)) if op == BinOp::Add => {
            let mut s = a.clone();
            s.push_str(b);
            ExprKind::Str(s)
        }
        _ => return None,
    };
    Some(make(kind, ty, span))
}

pub(super) fn fold_int_op(a: i64, op: BinOp, b: i64) -> Option<ExprKind> {
    match op {
        BinOp::Add => Some(ExprKind::Int(a.wrapping_add(b))),
        BinOp::Sub => Some(ExprKind::Int(a.wrapping_sub(b))),
        BinOp::Mul => Some(ExprKind::Int(a.wrapping_mul(b))),
        BinOp::Div if b != 0 => Some(ExprKind::Int(a / b)),
        BinOp::Mod if b != 0 => Some(ExprKind::Int(a % b)),
        BinOp::Shl if b >= 0 && b < 64 => Some(ExprKind::Int(a.wrapping_shl(b as u32))),
        BinOp::Shr if b >= 0 && b < 64 => Some(ExprKind::Int(a.wrapping_shr(b as u32))),
        BinOp::Ushr if b >= 0 && b < 64 => {
            Some(ExprKind::Int((a as u64).wrapping_shr(b as u32) as i64))
        }
        BinOp::BitAnd => Some(ExprKind::Int(a & b)),
        BinOp::BitOr => Some(ExprKind::Int(a | b)),
        BinOp::BitXor => Some(ExprKind::Int(a ^ b)),
        BinOp::Eq => Some(ExprKind::Bool(a == b)),
        BinOp::Ne => Some(ExprKind::Bool(a != b)),
        BinOp::Lt => Some(ExprKind::Bool(a < b)),
        BinOp::Gt => Some(ExprKind::Bool(a > b)),
        BinOp::Le => Some(ExprKind::Bool(a <= b)),
        BinOp::Ge => Some(ExprKind::Bool(a >= b)),
        _ => None,
    }
}

pub(super) fn fold_float_op(a: f64, op: BinOp, b: f64) -> Option<ExprKind> {
    match op {
        BinOp::Add => Some(ExprKind::Float(a + b)),
        BinOp::Sub => Some(ExprKind::Float(a - b)),
        BinOp::Mul => Some(ExprKind::Float(a * b)),
        BinOp::Div => Some(ExprKind::Float(a / b)),
        BinOp::Eq => Some(ExprKind::Bool(a == b)),
        BinOp::Lt => Some(ExprKind::Bool(a < b)),
        BinOp::Gt => Some(ExprKind::Bool(a > b)),
        BinOp::Le => Some(ExprKind::Bool(a <= b)),
        BinOp::Ge => Some(ExprKind::Bool(a >= b)),
        _ => None,
    }
}

pub(super) fn fold_unary(op: UnaryOp, e: &Expr, ty: Type, span: Span) -> Option<Expr> {
    match (op, &e.kind) {
        (UnaryOp::Neg, ExprKind::Int(n)) => Some(make(ExprKind::Int(-n), ty, span)),
        (UnaryOp::Neg, ExprKind::Float(f)) => Some(make(ExprKind::Float(-f), ty, span)),
        (UnaryOp::Not, ExprKind::Bool(b)) => Some(make(ExprKind::Bool(!b), ty, span)),
        (UnaryOp::BitNot, ExprKind::Int(n)) => Some(make(ExprKind::Int(!n), ty, span)),
        _ => None,
    }
}

pub(super) fn fold_ternary(cond: &Expr, t: &Expr, f: &Expr) -> Option<Expr> {
    if let ExprKind::Bool(b) = &cond.kind {
        Some(if *b { t.clone() } else { f.clone() })
    } else {
        None
    }
}

pub(super) fn fold_cast(e: &Expr, to_ty: &Type, span: Span) -> Option<Expr> {
    match (&e.kind, to_ty) {
        (ExprKind::Int(n), Type::F64) => {
            Some(make(ExprKind::Float(*n as f64), to_ty.clone(), span))
        }
        (ExprKind::Int(n), Type::F32) => {
            Some(make(ExprKind::Float(*n as f64), to_ty.clone(), span))
        }
        (ExprKind::Float(f), Type::I64) => {
            Some(make(ExprKind::Int(*f as i64), to_ty.clone(), span))
        }
        (ExprKind::Float(f), Type::I32) => {
            Some(make(ExprKind::Int(*f as i64), to_ty.clone(), span))
        }
        (ExprKind::Int(n), Type::I8) => {
            Some(make(ExprKind::Int(*n as i8 as i64), to_ty.clone(), span))
        }
        (ExprKind::Int(n), Type::I16) => {
            Some(make(ExprKind::Int(*n as i16 as i64), to_ty.clone(), span))
        }
        (ExprKind::Int(n), Type::I32) => {
            Some(make(ExprKind::Int(*n as i32 as i64), to_ty.clone(), span))
        }
        (ExprKind::Int(n), Type::U8) => {
            Some(make(ExprKind::Int(*n as u8 as i64), to_ty.clone(), span))
        }
        (ExprKind::Int(n), Type::U16) => {
            Some(make(ExprKind::Int(*n as u16 as i64), to_ty.clone(), span))
        }
        (ExprKind::Int(n), Type::U32) => {
            Some(make(ExprKind::Int(*n as u32 as i64), to_ty.clone(), span))
        }
        (ExprKind::Int(n), Type::U64) => Some(make(ExprKind::Int(*n), to_ty.clone(), span)),
        _ => None,
    }
}

pub(super) fn fold_builtin(
    builtin: &hir::BuiltinFn,
    args: &[Expr],
    ty: Type,
    span: Span,
) -> Option<Expr> {
    use hir::BuiltinFn::*;
    let kind = match builtin {
        Ln | Log2 | Log10 | Exp | Exp2 => {
            let ExprKind::Float(x) = &args[0].kind else {
                return None;
            };
            let f: fn(f64) -> f64 = match builtin {
                Ln => f64::ln,
                Log2 => f64::log2,
                Log10 => f64::log10,
                Exp => f64::exp,
                Exp2 => f64::exp2,
                _ => unreachable!(),
            };
            ExprKind::Float(f(*x))
        }
        PowF | Copysign => {
            let (ExprKind::Float(x), ExprKind::Float(y)) = (&args[0].kind, &args[1].kind) else {
                return None;
            };
            match builtin {
                PowF => ExprKind::Float(x.powf(*y)),
                _ => ExprKind::Float(x.copysign(*y)),
            }
        }
        Fma => {
            let (ExprKind::Float(a), ExprKind::Float(b), ExprKind::Float(c)) =
                (&args[0].kind, &args[1].kind, &args[2].kind)
            else {
                return None;
            };
            ExprKind::Float(a.mul_add(*b, *c))
        }
        _ => return None,
    };
    Some(make(kind, ty, span))
}

pub(super) fn make(kind: ExprKind, ty: Type, span: Span) -> Expr {
    Expr { kind, ty, span }
}
