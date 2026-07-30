use crate::hir::{self, Expr, ExprKind, Stmt};

pub(super) fn is_pure_fn(f: &hir::Fn) -> bool {
    f.body.iter().all(is_pure_stmt)
}

pub(super) fn is_pure_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Bind(b) => is_pure_expr(&b.value),
        Stmt::Ret(Some(e), _, _) => is_pure_expr(e),
        Stmt::Ret(None, _, _) => true,
        Stmt::If(i) => {
            is_pure_expr(&i.cond)
                && i.then.iter().all(is_pure_stmt)
                && i.elifs
                    .iter()
                    .all(|(c, b)| is_pure_expr(c) && b.iter().all(is_pure_stmt))
                && i.els.as_ref().is_none_or(|b| b.iter().all(is_pure_stmt))
        }
        Stmt::Expr(e) => is_pure_expr(e),
        _ => false,
    }
}

pub(super) fn is_pure_expr(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Var(_, _)
        | ExprKind::None
        | ExprKind::Void => true,
        ExprKind::BinOp(l, _, r) => is_pure_expr(l) && is_pure_expr(r),
        ExprKind::UnaryOp(_, e) => is_pure_expr(e),
        ExprKind::Call(_, _, args) => args.iter().all(is_pure_expr),
        ExprKind::Ternary(c, t, f) => is_pure_expr(c) && is_pure_expr(t) && is_pure_expr(f),
        ExprKind::Cast(e, _) => is_pure_expr(e),
        ExprKind::IfExpr(i) => {
            is_pure_expr(&i.cond)
                && i.then.iter().all(is_pure_stmt)
                && i.elifs
                    .iter()
                    .all(|(c, b)| is_pure_expr(c) && b.iter().all(is_pure_stmt))
                && i.els.as_ref().is_none_or(|b| b.iter().all(is_pure_stmt))
        }
        _ => false,
    }
}
