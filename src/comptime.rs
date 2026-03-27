use crate::ast::{BinOp, Span, UnaryOp};
use crate::hir::{self, Block, Expr, ExprKind, Stmt};
use crate::types::Type;

pub fn fold_program(prog: &mut hir::Program) {
    for f in &mut prog.fns {
        fold_block(&mut f.body);
    }
    for td in &mut prog.types {
        for m in &mut td.methods {
            fold_block(&mut m.body);
        }
    }
    for actor in &mut prog.actors {
        for m in &mut actor.handlers {
            fold_block(&mut m.body);
        }
    }
    for imp in &mut prog.trait_impls {
        for m in &mut imp.methods {
            fold_block(&mut m.body);
        }
    }
}

fn fold_block(block: &mut Block) {
    for stmt in block.iter_mut() {
        fold_stmt(stmt);
    }
}

fn fold_stmt(stmt: &mut Stmt) {
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
        Stmt::Drop(_, _, _, _)
        | Stmt::Continue(_)
        | Stmt::Ret(None, _, _)
        | Stmt::Break(None, _)
        | Stmt::Asm(_) => {}
        Stmt::StoreInsert(_, exprs, _) => {
            for e in exprs {
                fold_expr(e);
            }
        }
        Stmt::StoreDelete(_, _, _) => {}
        Stmt::StoreSet(_, pairs, _, _) => {
            for (_, e) in pairs {
                fold_expr(e);
            }
        }
        Stmt::Transaction(b, _) => fold_block(b),
        Stmt::ChannelClose(e, _) => fold_expr(e),
        Stmt::Stop(e, _) => fold_expr(e),
    }
}

fn fold_expr(expr: &mut Expr) {
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
        ExprKind::StringMethod(obj, _, args) => {
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
        ExprKind::MapMethod(obj, _, args) => {
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
        | ExprKind::Spawn(_)
        | ExprKind::StoreQuery(_, _)
        | ExprKind::StoreCount(_)
        | ExprKind::StoreAll(_)
        | ExprKind::IterNext(_, _, _) => {}
    }

    if let Some(folded) = try_fold(expr) {
        *expr = folded;
    }
}

fn try_fold(expr: &Expr) -> Option<Expr> {
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

fn fold_binop(l: &Expr, op: BinOp, r: &Expr, ty: Type, span: Span) -> Option<Expr> {
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

fn fold_int_op(a: i64, op: BinOp, b: i64) -> Option<ExprKind> {
    match op {
        BinOp::Add => Some(ExprKind::Int(a.wrapping_add(b))),
        BinOp::Sub => Some(ExprKind::Int(a.wrapping_sub(b))),
        BinOp::Mul => Some(ExprKind::Int(a.wrapping_mul(b))),
        BinOp::Div if b != 0 => Some(ExprKind::Int(a / b)),
        BinOp::Mod if b != 0 => Some(ExprKind::Int(a % b)),
        BinOp::Shl if b >= 0 && b < 64 => Some(ExprKind::Int(a.wrapping_shl(b as u32))),
        BinOp::Shr if b >= 0 && b < 64 => Some(ExprKind::Int(a.wrapping_shr(b as u32))),
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

fn fold_float_op(a: f64, op: BinOp, b: f64) -> Option<ExprKind> {
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

fn fold_unary(op: UnaryOp, e: &Expr, ty: Type, span: Span) -> Option<Expr> {
    match (op, &e.kind) {
        (UnaryOp::Neg, ExprKind::Int(n)) => Some(make(ExprKind::Int(-n), ty, span)),
        (UnaryOp::Neg, ExprKind::Float(f)) => Some(make(ExprKind::Float(-f), ty, span)),
        (UnaryOp::Not, ExprKind::Bool(b)) => Some(make(ExprKind::Bool(!b), ty, span)),
        (UnaryOp::BitNot, ExprKind::Int(n)) => Some(make(ExprKind::Int(!n), ty, span)),
        _ => None,
    }
}

fn fold_ternary(cond: &Expr, t: &Expr, f: &Expr) -> Option<Expr> {
    if let ExprKind::Bool(b) = &cond.kind {
        Some(if *b { t.clone() } else { f.clone() })
    } else {
        None
    }
}

fn fold_cast(e: &Expr, to_ty: &Type, span: Span) -> Option<Expr> {
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

fn fold_builtin(builtin: &hir::BuiltinFn, args: &[Expr], ty: Type, span: Span) -> Option<Expr> {
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

fn make(kind: ExprKind, ty: Type, span: Span) -> Expr {
    Expr { kind, ty, span }
}
