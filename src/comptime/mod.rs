//! Compile-time evaluation of `comptime` expressions.

use crate::ast::Span;
use crate::hir::{self, Expr, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::HashMap;

mod eval;
mod fold;
mod purity;

use fold::{fold_block_with_fns, fold_expr_with_fns};
use purity::is_pure_fn;

pub fn fold_program(prog: &mut hir::Program) {
    // Build a map of pure functions for comptime evaluation
    let pure_fns: HashMap<Symbol, hir::Fn> = prog
        .fns
        .iter()
        .filter(|f| is_pure_fn(f))
        .map(|f| (f.name.clone(), f.clone()))
        .collect();

    for f in &mut prog.fns {
        fold_block_with_fns(&mut f.body, &pure_fns);
    }
    for td in &mut prog.types {
        for m in &mut td.methods {
            fold_block_with_fns(&mut m.body, &pure_fns);
        }
    }
    for actor in &mut prog.actors {
        for m in &mut actor.handlers {
            fold_block_with_fns(&mut m.body, &pure_fns);
            if let Some(sleep_ms) = &mut m.loop_sleep_ms {
                fold_expr_with_fns(sleep_ms, &pure_fns);
            }
        }
    }
    for imp in &mut prog.trait_impls {
        for m in &mut imp.methods {
            fold_block_with_fns(&mut m.body, &pure_fns);
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum ConstVal {
    Int(i64),
    Float(f64),
    Bool(bool),
    Void,
}

impl ConstVal {
    pub(super) fn to_expr(&self, ty: Type, span: Span) -> Expr {
        let kind = match self {
            ConstVal::Int(v) => ExprKind::Int(*v),
            ConstVal::Float(v) => ExprKind::Float(*v),
            ConstVal::Bool(v) => ExprKind::Bool(*v),
            ConstVal::Void => ExprKind::Void,
        };
        Expr { kind, ty, span }
    }
}
