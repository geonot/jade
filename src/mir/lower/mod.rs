//! HIR → MIR lowering.
//!
//! Converts HIR functions into MIR basic blocks with explicit control flow.

use crate::intern::Symbol;
use super::*;
use crate::ast::{self, Span};
use crate::hir::{self, ExprKind, Pat};
use crate::types::Type;
use std::collections::{HashMap, HashSet};

mod ctx;
mod expr_p0;
mod expr_p1;
mod expr_p2;
mod expr_p3;
mod stmt_p0;
mod stmt_p1;
mod stmt_p2;
mod stmt_p3;
mod stmt_p4;
mod util;

use ctx::Lowerer;

pub fn lower_program(prog: &hir::Program) -> Program {
    let mut functions = Vec::new();
    for f in &prog.fns {
        functions.extend(lower_function(f));
    }
    // Also lower type methods
    for td in &prog.types {
        for m in &td.methods {
            functions.extend(lower_function(m));
        }
    }
    // Also lower trait impl methods
    for ti in &prog.trait_impls {
        for m in &ti.methods {
            functions.extend(lower_function(m));
        }
    }
    let types = prog
        .types
        .iter()
        .map(|td| TypeDef {
            name: td.name.clone(),
            fields: td
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.ty.clone()))
                .collect(),
        })
        .collect();
    let externs = prog
        .externs
        .iter()
        .map(|ef| ExternDecl {
            name: ef.name.clone(),
            params: ef.params.iter().map(|p| p.1.clone()).collect(),
            ret: ef.ret.clone(),
        })
        .collect();
    let globals = prog
        .globals
        .iter()
        .map(|g| GlobalDef {
            name: g.name.clone(),
            ty: g.ty.clone(),
        })
        .collect();
    Program {
        functions,
        types,
        externs,
        globals,
    }
}


fn lower_function(f: &hir::Fn) -> Vec<Function> {
    let mut lowerer = Lowerer::new(&f.name.as_str(), f.def_id, f.span);
    lowerer.func.ret_ty = f.ret.clone();
    lowerer.func.attrs = f.attrs.clone();

    // Create value IDs for parameters
    for p in &f.params {
        let val = lowerer.new_value();
        lowerer.func.params.push(Param {
            value: val,
            name: p.name.clone(),
            ty: p.ty.clone(),
        });
        lowerer.var_map.insert(p.name.clone(), val);
    }

    // Lower body
    let mut last = lowerer.emit(InstKind::Void, Type::Void, f.span);
    for stmt in &f.body {
        let v = lowerer.lower_stmt(stmt);
        // Don't let Drop/void statements clobber the result value
        // for non-void functions (drops are inserted by perceus after
        // the last-expression that should be returned).
        if !matches!(stmt, hir::Stmt::Drop(..)) {
            last = v;
        }
    }

    // Add implicit return if not already terminated
    if matches!(
        lowerer.func.block(lowerer.current_block).terminator,
        Terminator::Unreachable
    ) {
        lowerer.lower_deferred_in_reverse();
        if matches!(f.ret, Type::Void) {
            lowerer.set_terminator(Terminator::Return(None));
        } else {
            lowerer.set_terminator(Terminator::Return(Some(last)));
        }
    }

    let mut result = vec![lowerer.func];
    result.append(&mut lowerer.lambda_fns);
    result
}

fn lower_binop(op: &ast::BinOp) -> BinOp {
    match op {
        ast::BinOp::Add => BinOp::Add,
        ast::BinOp::Sub => BinOp::Sub,
        ast::BinOp::Mul => BinOp::Mul,
        ast::BinOp::Div => BinOp::Div,
        ast::BinOp::Mod => BinOp::Mod,
        ast::BinOp::Exp => BinOp::Exp,
        ast::BinOp::BitAnd => BinOp::BitAnd,
        ast::BinOp::BitOr => BinOp::BitOr,
        ast::BinOp::BitXor => BinOp::BitXor,
        ast::BinOp::Shl => BinOp::Shl,
        ast::BinOp::Shr => BinOp::Shr,
        ast::BinOp::Ushr => BinOp::Ushr,
        ast::BinOp::And => BinOp::And,
        ast::BinOp::Or => BinOp::Or,
        // Comparisons handled separately in lower_expr; this path is unreachable.
        ast::BinOp::Eq
        | ast::BinOp::Ne
        | ast::BinOp::Lt
        | ast::BinOp::Gt
        | ast::BinOp::Le
        | ast::BinOp::Ge => {
            unreachable!("comparison ops should be handled by lower_expr, not lower_binop")
        }
    }
}

fn lower_unaryop(op: &ast::UnaryOp) -> UnaryOp {
    match op {
        ast::UnaryOp::Neg => UnaryOp::Neg,
        ast::UnaryOp::Not => UnaryOp::Not,
        ast::UnaryOp::BitNot => UnaryOp::BitNot,
    }
}

impl Lowerer {
    pub(super) fn lower_expr(&mut self, expr: &hir::Expr) -> ValueId {
        if let Some(v) = self.lower_expr_p0(expr) { return v; }
        if let Some(v) = self.lower_expr_p1(expr) { return v; }
        if let Some(v) = self.lower_expr_p2(expr) { return v; }
        if let Some(v) = self.lower_expr_p3(expr) { return v; }
        panic!("unhandled ExprKind in lower_expr: {:?}", expr.kind)
    }
    pub(super) fn lower_stmt(&mut self, stmt: &hir::Stmt) -> ValueId {
        if let Some(v) = self.lower_stmt_p0(stmt) { return v; }
        if let Some(v) = self.lower_stmt_p1(stmt) { return v; }
        if let Some(v) = self.lower_stmt_p2(stmt) { return v; }
        if let Some(v) = self.lower_stmt_p3(stmt) { return v; }
        if let Some(v) = self.lower_stmt_p4(stmt) { return v; }
        panic!("unhandled Stmt in lower_stmt")
    }
}
