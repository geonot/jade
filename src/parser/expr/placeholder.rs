//! Lambda-shorthand placeholder substitution helpers.

use crate::ast::*;

/// Check if an AST expression contains `$` (Placeholder) or `$$` (IndexPlaceholder) anywhere.
pub(in crate::parser) fn contains_placeholder(expr: &Expr) -> bool {
    match expr {
        Expr::Placeholder(_) | Expr::IndexPlaceholder(_) => true,
        Expr::BinOp(l, _, r, _) => contains_placeholder(l) || contains_placeholder(r),
        Expr::UnaryOp(_, e, _) => contains_placeholder(e),
        Expr::Call(f, args, _) => contains_placeholder(f) || args.iter().any(contains_placeholder),
        Expr::Method(obj, _, args, _) => {
            contains_placeholder(obj) || args.iter().any(contains_placeholder)
        }
        Expr::Field(e, _, _) => contains_placeholder(e),
        Expr::Index(a, b, _) => contains_placeholder(a) || contains_placeholder(b),
        Expr::Ternary(a, b, c, _) => {
            contains_placeholder(a) || contains_placeholder(b) || contains_placeholder(c)
        }
        Expr::As(e, _, _) => contains_placeholder(e),
        Expr::Ref(e, _) => contains_placeholder(e),
        Expr::Deref(e, _) => contains_placeholder(e),
        Expr::Array(elems, _) => elems.iter().any(contains_placeholder),
        Expr::Tuple(elems, _) => elems.iter().any(contains_placeholder),
        Expr::Pipe(l, r, _, _) => contains_placeholder(l) || contains_placeholder(r),
        _ => false,
    }
}

/// Replace all `$` (Placeholder) in an expression with `Ident(name)`.
/// Does NOT replace `$$` (IndexPlaceholder).
pub(in crate::parser) fn replace_placeholder(expr: &Expr, name: &str) -> Expr {
    match expr {
        Expr::Placeholder(sp) => Expr::Ident(name.into(), *sp),
        Expr::IndexPlaceholder(_) => expr.clone(),
        Expr::BinOp(l, op, r, sp) => Expr::BinOp(
            Box::new(replace_placeholder(l, name)),
            *op,
            Box::new(replace_placeholder(r, name)),
            *sp,
        ),
        Expr::UnaryOp(op, e, sp) => Expr::UnaryOp(*op, Box::new(replace_placeholder(e, name)), *sp),
        Expr::Call(f, args, sp) => Expr::Call(
            Box::new(replace_placeholder(f, name)),
            args.iter().map(|a| replace_placeholder(a, name)).collect(),
            *sp,
        ),
        Expr::Method(obj, m, args, sp) => Expr::Method(
            Box::new(replace_placeholder(obj, name)),
            m.clone(),
            args.iter().map(|a| replace_placeholder(a, name)).collect(),
            *sp,
        ),
        Expr::Field(e, f, sp) => {
            Expr::Field(Box::new(replace_placeholder(e, name)), f.clone(), *sp)
        }
        Expr::Index(a, b, sp) => Expr::Index(
            Box::new(replace_placeholder(a, name)),
            Box::new(replace_placeholder(b, name)),
            *sp,
        ),
        Expr::Ternary(a, b, c, sp) => Expr::Ternary(
            Box::new(replace_placeholder(a, name)),
            Box::new(replace_placeholder(b, name)),
            Box::new(replace_placeholder(c, name)),
            *sp,
        ),
        Expr::As(e, t, sp) => Expr::As(Box::new(replace_placeholder(e, name)), t.clone(), *sp),
        Expr::Ref(e, sp) => Expr::Ref(Box::new(replace_placeholder(e, name)), *sp),
        Expr::Deref(e, sp) => Expr::Deref(Box::new(replace_placeholder(e, name)), *sp),
        Expr::Array(elems, sp) => Expr::Array(
            elems.iter().map(|e| replace_placeholder(e, name)).collect(),
            *sp,
        ),
        Expr::Tuple(elems, sp) => Expr::Tuple(
            elems.iter().map(|e| replace_placeholder(e, name)).collect(),
            *sp,
        ),
        Expr::Pipe(l, r, extra, sp) => Expr::Pipe(
            Box::new(replace_placeholder(l, name)),
            Box::new(replace_placeholder(r, name)),
            extra.iter().map(|e| replace_placeholder(e, name)).collect(),
            *sp,
        ),
        other => other.clone(),
    }
}

/// Check if any statement in a block contains `$`.
pub(in crate::parser) fn contains_placeholder_in_block(block: &[Stmt]) -> bool {
    block.iter().any(|s| contains_placeholder_in_stmt(s))
}

pub(in crate::parser) fn contains_placeholder_in_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Expr(e) => contains_placeholder(e),
        Stmt::Bind(b) => contains_placeholder(&b.value),
        Stmt::Assign(lhs, rhs, _) => contains_placeholder(lhs) || contains_placeholder(rhs),
        Stmt::If(i) => {
            contains_placeholder(&i.cond)
                || i.then.iter().any(|s| contains_placeholder_in_stmt(s))
                || i.elifs.iter().any(|(c, b)| {
                    contains_placeholder(c) || b.iter().any(|s| contains_placeholder_in_stmt(s))
                })
                || i.els
                    .as_ref()
                    .map_or(false, |b| b.iter().any(|s| contains_placeholder_in_stmt(s)))
        }
        Stmt::While(w) => {
            contains_placeholder(&w.cond) || w.body.iter().any(|s| contains_placeholder_in_stmt(s))
        }
        Stmt::For(f) => {
            contains_placeholder(&f.iter) || f.body.iter().any(|s| contains_placeholder_in_stmt(s))
        }
        Stmt::SimFor(f, _) => {
            contains_placeholder(&f.iter) || f.body.iter().any(|s| contains_placeholder_in_stmt(s))
        }
        Stmt::Loop(l) => l.body.iter().any(|s| contains_placeholder_in_stmt(s)),
        Stmt::Ret(Some(e), _) => contains_placeholder(e),
        Stmt::Break(Some(e), _) => contains_placeholder(e),
        Stmt::Match(m) => {
            contains_placeholder(&m.subject)
                || m.arms
                    .iter()
                    .any(|a| a.body.iter().any(|s| contains_placeholder_in_stmt(s)))
        }
        _ => false,
    }
}

/// Replace all `$` in a block with `Ident(name)`.
pub(in crate::parser) fn replace_placeholder_in_block(block: &[Stmt], name: &str) -> Vec<Stmt> {
    block
        .iter()
        .map(|s| replace_placeholder_in_stmt(s, name))
        .collect()
}

pub(in crate::parser) fn replace_placeholder_in_stmt(stmt: &Stmt, name: &str) -> Stmt {
    match stmt {
        Stmt::Expr(e) => Stmt::Expr(replace_placeholder(e, name)),
        Stmt::Bind(b) => Stmt::Bind(Bind {
            name: b.name.clone(),
            value: replace_placeholder(&b.value, name),
            ty: b.ty.clone(),
            atomic: b.atomic,
            span: b.span,
        }),
        Stmt::Assign(lhs, rhs, sp) => Stmt::Assign(
            replace_placeholder(lhs, name),
            replace_placeholder(rhs, name),
            *sp,
        ),
        Stmt::If(i) => Stmt::If(If {
            cond: replace_placeholder(&i.cond, name),
            then: replace_placeholder_in_block(&i.then, name),
            elifs: i
                .elifs
                .iter()
                .map(|(c, b)| {
                    (
                        replace_placeholder(c, name),
                        replace_placeholder_in_block(b, name),
                    )
                })
                .collect(),
            els: i
                .els
                .as_ref()
                .map(|b| replace_placeholder_in_block(b, name)),
            span: i.span,
        }),
        Stmt::While(w) => Stmt::While(While {
            cond: replace_placeholder(&w.cond, name),
            body: replace_placeholder_in_block(&w.body, name),
            span: w.span,
        }),
        Stmt::For(f) => Stmt::For(For {
            label: f.label.clone(),
            bind: f.bind.clone(),
            bind2: f.bind2.clone(),
            iter: replace_placeholder(&f.iter, name),
            end: f.end.as_ref().map(|e| replace_placeholder(e, name)),
            step: f.step.as_ref().map(|e| replace_placeholder(e, name)),
            body: replace_placeholder_in_block(&f.body, name),
            span: f.span,
        }),
        Stmt::Loop(l) => Stmt::Loop(Loop {
            body: replace_placeholder_in_block(&l.body, name),
            span: l.span,
        }),
        Stmt::Ret(val, sp) => Stmt::Ret(val.as_ref().map(|e| replace_placeholder(e, name)), *sp),
        Stmt::Break(val, sp) => {
            Stmt::Break(val.as_ref().map(|e| replace_placeholder(e, name)), *sp)
        }
        Stmt::Match(m) => Stmt::Match(Match {
            subject: replace_placeholder(&m.subject, name),
            arms: m
                .arms
                .iter()
                .map(|a| Arm {
                    pat: a.pat.clone(),
                    guard: a.guard.as_ref().map(|e| replace_placeholder(e, name)),
                    body: replace_placeholder_in_block(&a.body, name),
                    span: a.span,
                })
                .collect(),
            span: m.span,
        }),
        other => other.clone(),
    }
}

/// Check if an AST expression contains `$$` (IndexPlaceholder) anywhere.
pub(in crate::parser) fn contains_index_placeholder(expr: &Expr) -> bool {
    match expr {
        Expr::IndexPlaceholder(_) => true,
        Expr::BinOp(l, _, r, _) => contains_index_placeholder(l) || contains_index_placeholder(r),
        Expr::UnaryOp(_, e, _) => contains_index_placeholder(e),
        Expr::Call(f, args, _) => {
            contains_index_placeholder(f) || args.iter().any(contains_index_placeholder)
        }
        Expr::Method(obj, _, args, _) => {
            contains_index_placeholder(obj) || args.iter().any(contains_index_placeholder)
        }
        Expr::Field(e, _, _) => contains_index_placeholder(e),
        Expr::Index(a, b, _) => contains_index_placeholder(a) || contains_index_placeholder(b),
        Expr::Ternary(a, b, c, _) => {
            contains_index_placeholder(a)
                || contains_index_placeholder(b)
                || contains_index_placeholder(c)
        }
        Expr::As(e, _, _) => contains_index_placeholder(e),
        Expr::Ref(e, _) => contains_index_placeholder(e),
        Expr::Deref(e, _) => contains_index_placeholder(e),
        Expr::Array(elems, _) => elems.iter().any(contains_index_placeholder),
        Expr::Tuple(elems, _) => elems.iter().any(contains_index_placeholder),
        Expr::Pipe(l, r, _, _) => contains_index_placeholder(l) || contains_index_placeholder(r),
        _ => false,
    }
}

/// Replace all `$$` (IndexPlaceholder) in an expression with `Ident(name)`.
pub(in crate::parser) fn replace_index_placeholder(expr: &Expr, name: &str) -> Expr {
    match expr {
        Expr::IndexPlaceholder(sp) => Expr::Ident(name.into(), *sp),
        Expr::BinOp(l, op, r, sp) => Expr::BinOp(
            Box::new(replace_index_placeholder(l, name)),
            *op,
            Box::new(replace_index_placeholder(r, name)),
            *sp,
        ),
        Expr::UnaryOp(op, e, sp) => {
            Expr::UnaryOp(*op, Box::new(replace_index_placeholder(e, name)), *sp)
        }
        Expr::Call(f, args, sp) => Expr::Call(
            Box::new(replace_index_placeholder(f, name)),
            args.iter()
                .map(|a| replace_index_placeholder(a, name))
                .collect(),
            *sp,
        ),
        Expr::Method(obj, m, args, sp) => Expr::Method(
            Box::new(replace_index_placeholder(obj, name)),
            m.clone(),
            args.iter()
                .map(|a| replace_index_placeholder(a, name))
                .collect(),
            *sp,
        ),
        Expr::Field(e, f, sp) => {
            Expr::Field(Box::new(replace_index_placeholder(e, name)), f.clone(), *sp)
        }
        Expr::Index(a, b, sp) => Expr::Index(
            Box::new(replace_index_placeholder(a, name)),
            Box::new(replace_index_placeholder(b, name)),
            *sp,
        ),
        Expr::Ternary(a, b, c, sp) => Expr::Ternary(
            Box::new(replace_index_placeholder(a, name)),
            Box::new(replace_index_placeholder(b, name)),
            Box::new(replace_index_placeholder(c, name)),
            *sp,
        ),
        Expr::As(e, t, sp) => {
            Expr::As(Box::new(replace_index_placeholder(e, name)), t.clone(), *sp)
        }
        Expr::Ref(e, sp) => Expr::Ref(Box::new(replace_index_placeholder(e, name)), *sp),
        Expr::Deref(e, sp) => Expr::Deref(Box::new(replace_index_placeholder(e, name)), *sp),
        Expr::Array(elems, sp) => Expr::Array(
            elems
                .iter()
                .map(|e| replace_index_placeholder(e, name))
                .collect(),
            *sp,
        ),
        Expr::Tuple(elems, sp) => Expr::Tuple(
            elems
                .iter()
                .map(|e| replace_index_placeholder(e, name))
                .collect(),
            *sp,
        ),
        Expr::Pipe(l, r, extra, sp) => Expr::Pipe(
            Box::new(replace_index_placeholder(l, name)),
            Box::new(replace_index_placeholder(r, name)),
            extra.iter().map(|e| replace_index_placeholder(e, name)).collect(),
            *sp,
        ),
        other => other.clone(),
    }
}

/// Check if any statement in a block contains `$$`.
pub(in crate::parser) fn contains_index_placeholder_in_block(block: &[Stmt]) -> bool {
    block
        .iter()
        .any(|s| contains_index_placeholder_in_stmt(s))
}

pub(in crate::parser) fn contains_index_placeholder_in_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Expr(e) => contains_index_placeholder(e),
        Stmt::Bind(b) => contains_index_placeholder(&b.value),
        Stmt::Assign(lhs, rhs, _) => {
            contains_index_placeholder(lhs) || contains_index_placeholder(rhs)
        }
        Stmt::If(i) => {
            contains_index_placeholder(&i.cond)
                || i.then
                    .iter()
                    .any(|s| contains_index_placeholder_in_stmt(s))
                || i.elifs.iter().any(|(c, b)| {
                    contains_index_placeholder(c)
                        || b.iter().any(|s| contains_index_placeholder_in_stmt(s))
                })
                || i.els.as_ref().map_or(false, |b| {
                    b.iter().any(|s| contains_index_placeholder_in_stmt(s))
                })
        }
        Stmt::While(w) => {
            contains_index_placeholder(&w.cond)
                || w.body
                    .iter()
                    .any(|s| contains_index_placeholder_in_stmt(s))
        }
        Stmt::For(f) => {
            contains_index_placeholder(&f.iter)
                || f.body
                    .iter()
                    .any(|s| contains_index_placeholder_in_stmt(s))
        }
        Stmt::SimFor(f, _) => {
            contains_index_placeholder(&f.iter)
                || f.body
                    .iter()
                    .any(|s| contains_index_placeholder_in_stmt(s))
        }
        Stmt::Loop(l) => l
            .body
            .iter()
            .any(|s| contains_index_placeholder_in_stmt(s)),
        Stmt::Ret(Some(e), _) => contains_index_placeholder(e),
        Stmt::Break(Some(e), _) => contains_index_placeholder(e),
        Stmt::Match(m) => {
            contains_index_placeholder(&m.subject)
                || m.arms.iter().any(|a| {
                    a.body
                        .iter()
                        .any(|s| contains_index_placeholder_in_stmt(s))
                })
        }
        _ => false,
    }
}

/// Replace all `$$` in a block with `Ident(name)`.
pub(in crate::parser) fn replace_index_placeholder_in_block(block: &[Stmt], name: &str) -> Vec<Stmt> {
    block
        .iter()
        .map(|s| replace_index_placeholder_in_stmt(s, name))
        .collect()
}

pub(in crate::parser) fn replace_index_placeholder_in_stmt(stmt: &Stmt, name: &str) -> Stmt {
    match stmt {
        Stmt::Expr(e) => Stmt::Expr(replace_index_placeholder(e, name)),
        Stmt::Bind(b) => Stmt::Bind(Bind {
            name: b.name.clone(),
            value: replace_index_placeholder(&b.value, name),
            ty: b.ty.clone(),
            atomic: b.atomic,
            span: b.span,
        }),
        Stmt::Assign(lhs, rhs, sp) => Stmt::Assign(
            replace_index_placeholder(lhs, name),
            replace_index_placeholder(rhs, name),
            *sp,
        ),
        Stmt::If(i) => Stmt::If(If {
            cond: replace_index_placeholder(&i.cond, name),
            then: replace_index_placeholder_in_block(&i.then, name),
            elifs: i
                .elifs
                .iter()
                .map(|(c, b)| {
                    (
                        replace_index_placeholder(c, name),
                        replace_index_placeholder_in_block(b, name),
                    )
                })
                .collect(),
            els: i
                .els
                .as_ref()
                .map(|b| replace_index_placeholder_in_block(b, name)),
            span: i.span,
        }),
        Stmt::While(w) => Stmt::While(While {
            cond: replace_index_placeholder(&w.cond, name),
            body: replace_index_placeholder_in_block(&w.body, name),
            span: w.span,
        }),
        Stmt::For(f) => Stmt::For(For {
            label: f.label.clone(),
            bind: f.bind.clone(),
            bind2: f.bind2.clone(),
            iter: replace_index_placeholder(&f.iter, name),
            end: f.end.as_ref().map(|e| replace_index_placeholder(e, name)),
            step: f
                .step
                .as_ref()
                .map(|e| replace_index_placeholder(e, name)),
            body: replace_index_placeholder_in_block(&f.body, name),
            span: f.span,
        }),
        Stmt::Loop(l) => Stmt::Loop(Loop {
            body: replace_index_placeholder_in_block(&l.body, name),
            span: l.span,
        }),
        Stmt::Ret(val, sp) => Stmt::Ret(
            val.as_ref().map(|e| replace_index_placeholder(e, name)),
            *sp,
        ),
        Stmt::Break(val, sp) => Stmt::Break(
            val.as_ref().map(|e| replace_index_placeholder(e, name)),
            *sp,
        ),
        Stmt::Match(m) => Stmt::Match(Match {
            subject: replace_index_placeholder(&m.subject, name),
            arms: m
                .arms
                .iter()
                .map(|a| Arm {
                    pat: a.pat.clone(),
                    guard: a
                        .guard
                        .as_ref()
                        .map(|e| replace_index_placeholder(e, name)),
                    body: replace_index_placeholder_in_block(&a.body, name),
                    span: a.span,
                })
                .collect(),
            span: m.span,
        }),
        other => other.clone(),
    }
}
