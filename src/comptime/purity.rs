use std::collections::HashSet;

use crate::hir::{self, Expr, ExprKind, Stmt};
use crate::intern::Symbol;

pub(super) fn is_pure_fn(f: &hir::Fn, pure: &HashSet<Symbol>) -> bool {
    f.body.iter().all(|s| is_pure_stmt(s, pure))
}

pub(super) fn is_pure_stmt(stmt: &Stmt, pure: &HashSet<Symbol>) -> bool {
    match stmt {
        Stmt::Bind(b) => is_pure_expr(&b.value, pure),
        Stmt::Ret(Some(e), _, _) => is_pure_expr(e, pure),
        Stmt::Ret(None, _, _) => true,
        Stmt::If(i) => {
            is_pure_expr(&i.cond, pure)
                && i.then.iter().all(|s| is_pure_stmt(s, pure))
                && i.elifs
                    .iter()
                    .all(|(c, b)| is_pure_expr(c, pure) && b.iter().all(|s| is_pure_stmt(s, pure)))
                && i.els
                    .as_ref()
                    .is_none_or(|b| b.iter().all(|s| is_pure_stmt(s, pure)))
        }
        Stmt::Expr(e) => is_pure_expr(e, pure),
        _ => false,
    }
}

pub(super) fn is_pure_expr(expr: &Expr, pure: &HashSet<Symbol>) -> bool {
    match &expr.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Var(_, _)
        | ExprKind::None
        | ExprKind::Void => true,
        ExprKind::BinOp(l, _, r) => is_pure_expr(l, pure) && is_pure_expr(r, pure),
        ExprKind::UnaryOp(_, e) => is_pure_expr(e, pure),
        ExprKind::Call(_, name, args) => {
            pure.contains(name) && args.iter().all(|a| is_pure_expr(a, pure))
        }
        ExprKind::Ternary(c, t, f) => {
            is_pure_expr(c, pure) && is_pure_expr(t, pure) && is_pure_expr(f, pure)
        }
        ExprKind::Cast(e, _) => is_pure_expr(e, pure),
        ExprKind::IfExpr(i) => {
            is_pure_expr(&i.cond, pure)
                && i.then.iter().all(|s| is_pure_stmt(s, pure))
                && i.elifs
                    .iter()
                    .all(|(c, b)| is_pure_expr(c, pure) && b.iter().all(|s| is_pure_stmt(s, pure)))
                && i.els
                    .as_ref()
                    .is_none_or(|b| b.iter().all(|s| is_pure_stmt(s, pure)))
        }
        _ => false,
    }
}
