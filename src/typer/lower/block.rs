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
        } else if let Some(hir::Stmt::Expr(_)) = stmts.last() {
            // Implicit return: emit drops AFTER the tail evaluates, excluding
            // any var that was *moved* into the tail (struct ctor / tuple /
            // bare var). Vars merely referenced (via field access, method
            // call, etc.) still need to be dropped, but the drop must happen
            // AFTER the tail has read them. Emitting before-tail would free
            // storage the tail still needs.
            let tail = stmts.pop().unwrap();
            let mut tail_moves = std::collections::HashSet::new();
            if let hir::Stmt::Expr(te) = &tail {
                Self::collect_moved_var_ids(te, &mut tail_moves);
            }
            stmts.push(tail);
            self.emit_scope_drops_excluding(&mut stmts, &tail_moves);
        } else {
            self.emit_scope_drops(&mut stmts);
        }
        self.pop_scope();
        Ok(stmts)
    }

    pub(in crate::typer) fn emit_scope_drops(&mut self, stmts: &mut Vec<hir::Stmt>) {
        self.emit_scope_drops_excluding(stmts, &std::collections::HashSet::new());
    }

    /// Collect variable IDs that have been moved (consumed) by a statement-
    /// level expression earlier in the same block. Currently we recognise
    /// container "consume on push" methods: pushing/inserting a heap-typed
    /// bare variable into a Vec/Set/Map/PQ moves the value into the container,
    /// so it must not be dropped at scope exit (the container owns the
    /// underlying storage now).
    fn collect_block_consumed_ids(
        &mut self,
        stmts: &[hir::Stmt],
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        for s in stmts {
            match s {
                hir::Stmt::Expr(e) => self.collect_consumed_in_expr(e, out),
                // `x is y` (Assign) and `let x = y` (Bind) where `y` is a
                // bare variable of heap-managed type *moves* y into x. The
                // sole owner becomes x; y must not be dropped at scope exit.
                // Without this, both x and y get dropped and the underlying
                // buffer is freed twice (or x is left dangling if y's drop
                // runs first inside a loop body). Resolve any inference
                // variables before checking — the RHS type at this point may
                // still be a TypeVar that resolves to a heap type.
                hir::Stmt::Assign(_target, value, _) => {
                    let resolved = self.infer_ctx.resolve(&value.ty);
                    if Self::expr_type_needs_drop(&resolved) {
                        if let hir::ExprKind::Var(id, _) = &value.kind {
                            out.insert(*id);
                        }
                    }
                }
                hir::Stmt::Bind(b) => {
                    let resolved = self.infer_ctx.resolve(&b.value.ty);
                    if Self::expr_type_needs_drop(&resolved) {
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
        &mut self,
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
                        let resolved = self.infer_ctx.resolve(&a.ty);
                        if Self::expr_type_needs_drop(&resolved)
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
        &mut self,
        stmts: &mut Vec<hir::Stmt>,
        exclude: &std::collections::HashSet<crate::hir::DefId>,
    ) {
        // Snapshot scope entries up-front so we can take &mut self for resolve.
        let scope_entries: Vec<(crate::intern::Symbol, crate::typer::VarInfo)> =
            match self.scopes.last() {
                Some(s) => s.iter().map(|(n, v)| (n.clone(), v.clone())).collect(),
                None => return,
            };
        // In addition to the caller-supplied exclusion set (e.g. tail-expr
        // moves), also exclude any scope variable that was already moved into
        // a container by a prior stmt-level call (e.g. `g.push(row)`).
        let mut consumed: std::collections::HashSet<crate::hir::DefId> = exclude.clone();
        self.collect_block_consumed_ids(stmts, &mut consumed);
        // Resolve any inference variables so that needs_drop sees the concrete
        // type (e.g. Vec(_) rather than TypeVar). Without this, owned heap
        // locals whose Bind site introduced a fresh TyVar would be skipped by
        // needs_drop and silently leak. Pre-resolve all entries up-front to
        // satisfy the borrow checker (resolve needs &mut self.infer_ctx).
        let mut resolved_entries: Vec<(crate::intern::Symbol, crate::typer::VarInfo, Type)> =
            Vec::with_capacity(scope_entries.len());
        for (name, info) in scope_entries {
            let resolved = self.infer_ctx.resolve(&info.ty);
            resolved_entries.push((name, info, resolved));
        }
        let mut drops: Vec<_> = resolved_entries
            .into_iter()
            .filter(|(_, info, resolved)| {
                self.needs_drop(resolved)
                    && !matches!(
                        info.ownership,
                        crate::hir::Ownership::Borrowed | crate::hir::Ownership::BorrowMut
                    )
                    && !consumed.contains(&info.def_id)
            })
            .collect();
        drops.sort_by_key(|(_, info, _)| std::cmp::Reverse(info.def_id.0));
        for (name, info, resolved) in drops {
            stmts.push(hir::Stmt::Drop(
                info.def_id,
                name.clone(),
                resolved,
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

    pub(in crate::typer) fn needs_drop(&self, ty: &Type) -> bool {
        let mut visiting: std::collections::HashSet<crate::intern::Symbol> =
            std::collections::HashSet::new();
        self.needs_drop_inner(ty, &mut visiting)
    }

    /// Recursive implementation of `needs_drop` with cycle protection.
    /// A `visiting` set carries the names of struct/enum types currently on
    /// the inspection stack — a self- or mutually-referential type is treated
    /// as not needing a drop *at the cycle point* (the heap edge is usually
    /// an `Rc`/`Weak`/box which is itself in the base-case heap-owning set).
    fn needs_drop_inner(
        &self,
        ty: &Type,
        visiting: &mut std::collections::HashSet<crate::intern::Symbol>,
    ) -> bool {
        // Base-case heap-owning types: their owned bindings always require
        // scope-exit drop.
        if matches!(
            ty,
            Type::String
                | Type::Vec(_)
                | Type::Map(_, _)
                | Type::Set(_)
                | Type::PriorityQueue(_)
                | Type::Deque(_)
                | Type::Rc(_)
                | Type::Weak(_)
                | Type::Coroutine(_)
                | Type::Generator(_)
                | Type::NDArray(_, _)
                | Type::Channel(_)
                | Type::Cow(_)
        ) {
            return true;
        }
        match ty {
            // A user-defined struct needs drop only when annotated
            // `@resource` (so the auto `*drop` fires at scope exit).
            //
            // NOTE: we deliberately do NOT recurse into struct fields here.
            // Doing so would correctly model "this aggregate owns heap
            // storage", but the matching move-tracking infrastructure is
            // not yet in place: plain function calls (`r is modulo(x, y)`)
            // do not mark bare-Var args as consumed, and function-parameter
            // ownership for compound heap types is not modelled as a borrow
            // either. Until that work lands, recursive needs_drop on
            // aggregates causes double-frees (caller's slot AND callee's
            // parameter both drop the same shared backing buffer).
            // Compound structs may therefore leak their heap fields at
            // scope exit; this matches the pre-P4 behaviour and is a known
            // gap to be closed by a dedicated call-arg / parameter
            // ownership sprint. See `docs/access-semantics-sprint.md` §5.
            Type::Struct(name, _) => self
                .struct_attrs
                .get(name)
                .map(|attrs| attrs.resource)
                .unwrap_or(false),
            // Alias/Newtype are transparent wrappers — preserve the
            // underlying type's drop classification.
            Type::Alias(_, inner) | Type::Newtype(_, inner) => {
                self.needs_drop_inner(inner, visiting)
            }
            _ => false,
        }
    }
}
