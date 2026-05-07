use crate::ast::*;

pub(super) fn body_contains_yield(body: &[Stmt]) -> bool {
    body.iter().any(|s| stmt_has_yield(s))
}

pub(super) fn stmt_has_yield(s: &Stmt) -> bool {
    match s {
        Stmt::Expr(e) | Stmt::Ret(Some(e), _) | Stmt::Break(Some(e), _) => expr_has_yield(e),
        Stmt::If(i) => {
            expr_has_yield(&i.cond)
                || body_contains_yield(&i.then)
                || i.elifs
                    .iter()
                    .any(|(c, b)| expr_has_yield(c) || body_contains_yield(b))
                || i.els.as_ref().is_some_and(|b| body_contains_yield(b))
        }
        Stmt::While(w) => expr_has_yield(&w.cond) || body_contains_yield(&w.body),
        Stmt::For(f) => body_contains_yield(&f.body),
        Stmt::Loop(l) => body_contains_yield(&l.body),
        Stmt::Match(m) => m.arms.iter().any(|a| body_contains_yield(&a.body)),
        _ => false,
    }
}

pub(super) fn expr_has_yield(e: &Expr) -> bool {
    matches!(e, Expr::Yield(_, _))
}

