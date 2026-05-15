//! Extracted lowering steps.

#![allow(unused_imports, unused_variables)]

use std::collections::{HashMap, HashSet};

use super::super::unify;
use super::super::{DeferredField, DeferredMethod, Typer, VarInfo};
use crate::ast::{self, Span};
use crate::hir::{self, CoercionKind, DefId, ExprKind, Ownership};
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn hir_tail_type(&self, body: &[hir::Stmt]) -> Option<Type> {
        let last = body
            .iter()
            .rev()
            .find(|s| !matches!(s, hir::Stmt::Drop(..)))?;
        match last {
            hir::Stmt::Expr(e) if e.ty != Type::Void => Some(e.ty.clone()),
            hir::Stmt::If(i) => {
                if i.els.is_some() {
                    self.hir_tail_type(&i.then)
                } else {
                    None
                }
            }
            hir::Stmt::Match(m) => {
                if let Some(arm) = m.arms.first() {
                    self.hir_tail_type(&arm.body)
                } else {
                    None
                }
            }
            hir::Stmt::Ret(Some(e), _, _) => Some(e.ty.clone()),
            _ => None,
        }
    }

    pub(in crate::typer) fn lower_block(
        &mut self,
        block: &ast::Block,
        ret_ty: &Type,
    ) -> Result<hir::Block, String> {
        self.push_scope();
        let mut stmts = self.lower_block_no_scope(block, ret_ty)?;
        let ends_with_jump = stmts.last().map_or(false, |s| {
            matches!(
                s,
                hir::Stmt::Ret(..) | hir::Stmt::Break(..) | hir::Stmt::Continue(..)
            )
        });
        if ends_with_jump {
            let jump = stmts.pop().unwrap();
            // Collect variable IDs referenced in the jump expression so we
            // don't drop them before they're consumed by the return/break.
            let mut jump_refs = std::collections::HashSet::new();
            Self::collect_hir_var_ids_stmt(&jump, &mut jump_refs);
            self.emit_scope_drops_excluding(&mut stmts, &jump_refs);
            stmts.push(jump);
        } else if let Some(hir::Stmt::Expr(tail_expr)) = stmts.last() {
            // Implicit return: exclude variables that are *moved* into the
            // tail expression (struct constructors, tuple literals, bare vars).
            // Method calls, field accesses, etc. borrow — not move — so don't
            // exclude their operands.
            let mut tail_refs = std::collections::HashSet::new();
            Self::collect_moved_var_ids(tail_expr, &mut tail_refs);
            if tail_refs.is_empty() {
                self.emit_scope_drops(&mut stmts);
            } else {
                let tail = stmts.pop().unwrap();
                self.emit_scope_drops_excluding(&mut stmts, &tail_refs);
                stmts.push(tail);
            }
        } else {
            self.emit_scope_drops(&mut stmts);
        }
        self.pop_scope();
        Ok(stmts)
    }

    pub(in crate::typer) fn emit_scope_drops(&self, stmts: &mut Vec<hir::Stmt>) {
        self.emit_scope_drops_excluding(stmts, &std::collections::HashSet::new());
    }

    /// Collect variable IDs that have been moved (consumed) by a statement-
    /// level expression earlier in the same block. Currently we recognise
    /// container "consume on push" methods: pushing/inserting a heap-typed
    /// bare variable into a Vec/Set/Map/PQ moves the value into the container,
    /// so it must not be dropped at scope exit (the container owns the
    /// underlying storage now).
    fn collect_block_consumed_ids(
        stmts: &[hir::Stmt],
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        for s in stmts {
            match s {
                hir::Stmt::Expr(e) => Self::collect_consumed_in_expr(e, out),
                // `x is y` (Assign) and `let x = y` (Bind) where `y` is a
                // bare variable of heap-managed type *moves* y into x. The
                // sole owner becomes x; y must not be dropped at scope exit.
                // Without this, both x and y get dropped and the underlying
                // buffer is freed twice (or x is left dangling if y's drop
                // runs first inside a loop body).
                hir::Stmt::Assign(_target, value, _) => {
                    if Self::expr_type_needs_drop(&value.ty) {
                        if let hir::ExprKind::Var(id, _) = &value.kind {
                            out.insert(*id);
                        }
                    }
                }
                hir::Stmt::Bind(b) => {
                    if Self::expr_type_needs_drop(&b.value.ty) {
                        if let hir::ExprKind::Var(id, _) = &b.value.kind {
                            out.insert(*id);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_consumed_in_expr(
        expr: &hir::Expr,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match &expr.kind {
            hir::ExprKind::VecMethod(_, meth, args)
            | hir::ExprKind::SetMethod(_, meth, args)
            | hir::ExprKind::MapMethod(_, meth, args)
            | hir::ExprKind::PQMethod(_, meth, args) => {
                let m_owned = meth.as_str();
                let m: &str = m_owned.as_ref();
                if matches!(
                    m,
                    "push"
                        | "push_back"
                        | "push_front"
                        | "insert"
                        | "append"
                        | "add"
                        | "put"
                        | "set"
                        | "enqueue"
                        | "send"
                ) {
                    for a in args {
                        if Self::expr_type_needs_drop(&a.ty)
                            && matches!(a.kind, hir::ExprKind::Var(_, _))
                        {
                            if let hir::ExprKind::Var(id, _) = &a.kind {
                                out.insert(*id);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn expr_type_needs_drop(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Vec(_)
                | Type::Map(_, _)
                | Type::Set(_)
                | Type::PriorityQueue(_)
                | Type::String
                | Type::Rc(_)
                | Type::NDArray(_, _)
                | Type::Struct(_, _)
                | Type::Enum(_)
        )
    }

    pub(in crate::typer) fn emit_scope_drops_excluding(
        &self,
        stmts: &mut Vec<hir::Stmt>,
        exclude: &std::collections::HashSet<crate::hir::DefId>,
    ) {
        let scope = match self.scopes.last() {
            Some(s) => s,
            None => return,
        };
        // In addition to the caller-supplied exclusion set (e.g. tail-expr
        // moves), also exclude any scope variable that was already moved into
        // a container by a prior stmt-level call (e.g. `g.push(row)`).
        let mut consumed: std::collections::HashSet<crate::hir::DefId> = exclude.clone();
        Self::collect_block_consumed_ids(stmts, &mut consumed);
        let mut drops: Vec<_> = scope
            .iter()
            .filter(|(_, info)| {
                Self::needs_drop(&info.ty)
                    && !matches!(
                        info.ownership,
                        crate::hir::Ownership::Borrowed | crate::hir::Ownership::BorrowMut
                    )
                    && !consumed.contains(&info.def_id)
            })
            .collect();
        drops.sort_by_key(|(_, info)| std::cmp::Reverse(info.def_id.0));
        for (name, info) in drops {
            stmts.push(hir::Stmt::Drop(
                info.def_id,
                name.clone(),
                info.ty.clone(),
                crate::ast::Span::dummy(),
            ));
        }
    }

    /// Collect variable IDs that are *moved* (consumed) by an expression.
    /// Only struct constructors, tuple literals, and bare variable references
    /// count as moves. Method calls, field accesses, etc. borrow their receiver.
    pub(in crate::typer) fn collect_moved_var_ids(
        expr: &hir::Expr,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match &expr.kind {
            hir::ExprKind::Var(id, _) => {
                out.insert(*id);
            }
            hir::ExprKind::Struct(_, inits) | hir::ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    Self::collect_moved_var_ids(&fi.value, out);
                }
            }
            hir::ExprKind::Tuple(es) | hir::ExprKind::Array(es) => {
                for e in es {
                    Self::collect_moved_var_ids(e, out);
                }
            }
            _ => {}
        }
    }

    pub(in crate::typer) fn collect_hir_var_ids_expr(
        expr: &hir::Expr,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match &expr.kind {
            hir::ExprKind::Var(id, _) => {
                out.insert(*id);
            }
            hir::ExprKind::BinOp(l, _, r) => {
                Self::collect_hir_var_ids_expr(l, out);
                Self::collect_hir_var_ids_expr(r, out);
            }
            hir::ExprKind::UnaryOp(_, e) => Self::collect_hir_var_ids_expr(e, out),
            hir::ExprKind::Call(_, _, args) => {
                for a in args {
                    Self::collect_hir_var_ids_expr(a, out);
                }
            }
            hir::ExprKind::Struct(_, inits) | hir::ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    Self::collect_hir_var_ids_expr(&fi.value, out);
                }
            }
            hir::ExprKind::IfExpr(i) => {
                Self::collect_hir_var_ids_expr(&i.cond, out);
                for s in &i.then {
                    Self::collect_hir_var_ids_stmt(s, out);
                }
                for (c, b) in &i.elifs {
                    Self::collect_hir_var_ids_expr(c, out);
                    for s in b {
                        Self::collect_hir_var_ids_stmt(s, out);
                    }
                }
                if let Some(b) = &i.els {
                    for s in b {
                        Self::collect_hir_var_ids_stmt(s, out);
                    }
                }
            }
            hir::ExprKind::Index(e, i) => {
                Self::collect_hir_var_ids_expr(e, out);
                Self::collect_hir_var_ids_expr(i, out);
            }
            hir::ExprKind::Field(e, _, _) => Self::collect_hir_var_ids_expr(e, out),
            hir::ExprKind::Method(e, _, _, args)
            | hir::ExprKind::StringMethod(e, _, args)
            | hir::ExprKind::DeferredMethod(e, _, args)
            | hir::ExprKind::VecMethod(e, _, args)
            | hir::ExprKind::MapMethod(e, _, args)
            | hir::ExprKind::SetMethod(e, _, args)
            | hir::ExprKind::PQMethod(e, _, args)
            | hir::ExprKind::DequeMethod(e, _, args) => {
                Self::collect_hir_var_ids_expr(e, out);
                for a in args {
                    Self::collect_hir_var_ids_expr(a, out);
                }
            }
            hir::ExprKind::Tuple(es) | hir::ExprKind::Array(es) => {
                for e in es {
                    Self::collect_hir_var_ids_expr(e, out);
                }
            }
            hir::ExprKind::Block(stmts) => {
                for s in stmts {
                    Self::collect_hir_var_ids_stmt(s, out);
                }
            }
            hir::ExprKind::Lambda(_, body) => {
                for s in body {
                    Self::collect_hir_var_ids_stmt(s, out);
                }
            }
            hir::ExprKind::Ref(e) | hir::ExprKind::Deref(e) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::ExprKind::Pipe(e, _, _, rest) => {
                Self::collect_hir_var_ids_expr(e, out);
                for a in rest {
                    Self::collect_hir_var_ids_expr(a, out);
                }
            }
            hir::ExprKind::Cast(e, _) => Self::collect_hir_var_ids_expr(e, out),
            _ => {}
        }
    }

    pub(in crate::typer) fn collect_hir_var_ids_stmt(
        stmt: &hir::Stmt,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match stmt {
            hir::Stmt::Expr(e) | hir::Stmt::Bind(hir::Bind { value: e, .. }) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::Stmt::Ret(Some(e), _, _)
            | hir::Stmt::Break(Some(e), _)
            | hir::Stmt::ErrReturn(e, _, _) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::Stmt::Assign(t, v, _) => {
                Self::collect_hir_var_ids_expr(t, out);
                Self::collect_hir_var_ids_expr(v, out);
            }
            hir::Stmt::If(i) => {
                Self::collect_hir_var_ids_expr(&i.cond, out);
                for s in &i.then {
                    Self::collect_hir_var_ids_stmt(s, out);
                }
                for (c, b) in &i.elifs {
                    Self::collect_hir_var_ids_expr(c, out);
                    for s in b {
                        Self::collect_hir_var_ids_stmt(s, out);
                    }
                }
                if let Some(b) = &i.els {
                    for s in b {
                        Self::collect_hir_var_ids_stmt(s, out);
                    }
                }
            }
            _ => {}
        }
    }

    pub(in crate::typer) fn needs_drop(ty: &Type) -> bool {
        matches!(
            ty,
            Type::String
                | Type::Vec(_)
                | Type::Map(_, _)
                | Type::Rc(_)
                | Type::Weak(_)
                | Type::Coroutine(_)
        )
    }
}
