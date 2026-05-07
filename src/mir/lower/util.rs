// Auto-split from lower.rs.
#![allow(unused_imports, unused_variables)]
use crate::intern::Symbol;
use super::super::*;
use crate::ast::{self, Span};
use crate::hir::{self, ExprKind, Pat};
use crate::types::Type;
use std::collections::{HashMap, HashSet};
use super::Lowerer;

impl Lowerer {
    pub(super) fn collect_assigned_vars(body: &[hir::Stmt], assigned: &mut HashSet<Symbol>) {
        for stmt in body {
            match stmt {
                hir::Stmt::Bind(b) => {
                    assigned.insert(b.name.clone());
                }
                hir::Stmt::Assign(target, _, _) => {
                    if let ExprKind::Var(_, name) = &target.kind {
                        assigned.insert(name.clone());
                    }
                }
                hir::Stmt::If(i) => {
                    Self::collect_assigned_vars(&i.then, assigned);
                    for (_, elif_body) in &i.elifs {
                        Self::collect_assigned_vars(elif_body, assigned);
                    }
                    if let Some(els) = &i.els {
                        Self::collect_assigned_vars(els, assigned);
                    }
                }
                hir::Stmt::While(w) => {
                    Self::collect_assigned_vars(&w.body, assigned);
                }
                hir::Stmt::For(f) => {
                    Self::collect_assigned_vars(&f.body, assigned);
                }
                hir::Stmt::Loop(l) => {
                    Self::collect_assigned_vars(&l.body, assigned);
                }
                hir::Stmt::Match(m) => {
                    for arm in &m.arms {
                        Self::collect_assigned_vars(&arm.body, assigned);
                    }
                }
                hir::Stmt::TupleBind(bindings, _, _) => {
                    for (_, name, _) in bindings {
                        assigned.insert(name.clone());
                    }
                }
                hir::Stmt::Expr(e) => {
                    Self::collect_assigned_vars_in_expr(e, assigned);
                }
                _ => {}
            }
        }
    }

    /// Walk an expression tree to find block-containing expressions
    /// and collect assigned vars from their bodies.
    pub(super) fn collect_assigned_vars_in_expr(expr: &hir::Expr, assigned: &mut HashSet<Symbol>) {
        match &expr.kind {
            ExprKind::Select(arms, default) => {
                for arm in arms {
                    Self::collect_assigned_vars(&arm.body, assigned);
                }
                if let Some(def_body) = default {
                    Self::collect_assigned_vars(def_body, assigned);
                }
            }
            ExprKind::IfExpr(i) => {
                Self::collect_assigned_vars(&i.then, assigned);
                if let Some(els) = &i.els {
                    Self::collect_assigned_vars(els, assigned);
                }
            }
            ExprKind::Block(stmts) => {
                Self::collect_assigned_vars(stmts, assigned);
            }
            ExprKind::Lambda(_, body) => {
                Self::collect_assigned_vars(body, assigned);
            }
            ExprKind::ListComp(body, _, _, iter, end, cond) => {
                Self::collect_assigned_vars_in_expr(body, assigned);
                Self::collect_assigned_vars_in_expr(iter, assigned);
                if let Some(e) = end {
                    Self::collect_assigned_vars_in_expr(e, assigned);
                }
                if let Some(c) = cond {
                    Self::collect_assigned_vars_in_expr(c, assigned);
                }
            }
            // Recurse into sub-expressions that may contain blocks.
            ExprKind::BinOp(l, _, r) => {
                Self::collect_assigned_vars_in_expr(l, assigned);
                Self::collect_assigned_vars_in_expr(r, assigned);
            }
            ExprKind::Ternary(c, t, f) => {
                Self::collect_assigned_vars_in_expr(c, assigned);
                Self::collect_assigned_vars_in_expr(t, assigned);
                Self::collect_assigned_vars_in_expr(f, assigned);
            }
            ExprKind::Call(_, _, args) => {
                for a in args {
                    Self::collect_assigned_vars_in_expr(a, assigned);
                }
            }
            ExprKind::IndirectCall(f, args) => {
                Self::collect_assigned_vars_in_expr(f, assigned);
                for a in args {
                    Self::collect_assigned_vars_in_expr(a, assigned);
                }
            }
            ExprKind::Method(obj, _, _, args)
            | ExprKind::StringMethod(obj, _, args)
            | ExprKind::VecMethod(obj, _, args)
            | ExprKind::MapMethod(obj, _, args)
            | ExprKind::SetMethod(obj, _, args)
            | ExprKind::DeferredMethod(obj, _, args) => {
                Self::collect_assigned_vars_in_expr(obj, assigned);
                for a in args {
                    Self::collect_assigned_vars_in_expr(a, assigned);
                }
            }
            _ => {}
        }
    }

    /// Collect variable names first defined via Bind in a block (non-recursive into sub-blocks).
    pub(super) fn collect_new_binds(body: &[hir::Stmt], binds: &mut HashSet<Symbol>) {
        for stmt in body {
            match stmt {
                hir::Stmt::Bind(b) => {
                    binds.insert(b.name.clone());
                }
                hir::Stmt::TupleBind(bindings, _, _) => {
                    for (_, name, _) in bindings {
                        binds.insert(name.clone());
                    }
                }
                _ => {}
            }
        }
    }

    /// Demote variables to memory (Store/Load) — emit Store for their current
    /// var_map value and remove them from var_map so reads use Load.
    pub(super) fn demote_vars_to_memory(&mut self, vars: &HashSet<Symbol>, span: Span) {
        for name in vars {
            if let Some(&val) = self.var_map.get(name) {
                // Find the type of this variable from the value.
                let ty = self
                    .func
                    .blocks
                    .iter()
                    .flat_map(|bb| bb.insts.iter())
                    .find(|i| i.dest == Some(val))
                    .map(|i| i.ty.clone())
                    .or_else(|| {
                        self.func
                            .params
                            .iter()
                            .find(|p| p.value == val)
                            .map(|p| p.ty.clone())
                    })
                    .unwrap_or(Type::I64);
                // Emit Store with the variable's type (not Void) so codegen
                // creates the alloca with the correct LLVM type.
                self.func
                    .block_mut(self.current_block)
                    .insts
                    .push(Instruction {
                        dest: None,
                        kind: InstKind::Store(name.clone(), val),
                        ty,
                        span,
                        def_id: None,
                    });
                self.var_map.remove(name);
                self.mem_vars.insert(name.clone());
            }
        }
    }

    /// Collect variable names referenced in a block of HIR statements.
    pub(super) fn collect_expr_var_refs_block(
        body: &[hir::Stmt],
        refs: &mut std::collections::HashSet<Symbol>,
    ) {
        for stmt in body {
            Self::collect_expr_var_refs_stmt(stmt, refs);
        }
    }

    pub(super) fn collect_expr_var_refs_stmt(stmt: &hir::Stmt, refs: &mut std::collections::HashSet<Symbol>) {
        match stmt {
            hir::Stmt::Bind(b) => Self::collect_expr_var_refs_expr(&b.value, refs),
            hir::Stmt::Assign(t, v, _) => {
                Self::collect_expr_var_refs_expr(t, refs);
                Self::collect_expr_var_refs_expr(v, refs);
            }
            hir::Stmt::Expr(e) => Self::collect_expr_var_refs_expr(e, refs),
            hir::Stmt::If(i) => {
                Self::collect_expr_var_refs_expr(&i.cond, refs);
                Self::collect_expr_var_refs_block(&i.then, refs);
                for (c, b) in &i.elifs {
                    Self::collect_expr_var_refs_expr(c, refs);
                    Self::collect_expr_var_refs_block(b, refs);
                }
                if let Some(els) = &i.els {
                    Self::collect_expr_var_refs_block(els, refs);
                }
            }
            hir::Stmt::While(w) => {
                Self::collect_expr_var_refs_expr(&w.cond, refs);
                Self::collect_expr_var_refs_block(&w.body, refs);
            }
            hir::Stmt::For(f) => {
                Self::collect_expr_var_refs_expr(&f.iter, refs);
                Self::collect_expr_var_refs_block(&f.body, refs);
            }
            hir::Stmt::Loop(l) => Self::collect_expr_var_refs_block(&l.body, refs),
            hir::Stmt::Ret(Some(e), _, _) => Self::collect_expr_var_refs_expr(e, refs),
            hir::Stmt::Match(m) => {
                Self::collect_expr_var_refs_expr(&m.subject, refs);
                for arm in &m.arms {
                    Self::collect_expr_var_refs_block(&arm.body, refs);
                }
            }
            hir::Stmt::Break(Some(e), _)
            | hir::Stmt::ErrReturn(e, _, _)
            | hir::Stmt::ChannelClose(e, _)
            | hir::Stmt::Stop(e, _) => {
                Self::collect_expr_var_refs_expr(e, refs);
            }
            hir::Stmt::TupleBind(_, e, _) => Self::collect_expr_var_refs_expr(e, refs),
            hir::Stmt::SimFor(f, _) => {
                Self::collect_expr_var_refs_expr(&f.iter, refs);
                Self::collect_expr_var_refs_block(&f.body, refs);
            }
            hir::Stmt::SimBlock(body, _) | hir::Stmt::Transaction(body, _) => {
                Self::collect_expr_var_refs_block(body, refs);
            }
            hir::Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    Self::collect_expr_var_refs_expr(e, refs);
                }
            }
            hir::Stmt::StoreSet(_, updates, _, _) => {
                for (_, e) in updates {
                    Self::collect_expr_var_refs_expr(e, refs);
                }
            }
            _ => {}
        }
    }

    pub(super) fn collect_expr_var_refs_expr(expr: &hir::Expr, refs: &mut std::collections::HashSet<Symbol>) {
        match &expr.kind {
            ExprKind::Var(_, name) => {
                refs.insert(name.clone());
            }
            ExprKind::BinOp(l, _, r) => {
                Self::collect_expr_var_refs_expr(l, refs);
                Self::collect_expr_var_refs_expr(r, refs);
            }
            ExprKind::UnaryOp(_, e)
            | ExprKind::Ref(e)
            | ExprKind::Deref(e)
            | ExprKind::Cast(e, _)
            | ExprKind::StrictCast(e, _)
            | ExprKind::Coerce(e, _) => {
                Self::collect_expr_var_refs_expr(e, refs);
            }
            ExprKind::Call(_, _, args)
            | ExprKind::Array(args)
            | ExprKind::Tuple(args)
            | ExprKind::VecNew(args)
            | ExprKind::NDArrayNew(args)
            | ExprKind::SIMDNew(args)
            | ExprKind::Syscall(args) => {
                for a in args {
                    Self::collect_expr_var_refs_expr(a, refs);
                }
            }
            ExprKind::IndirectCall(f, args) => {
                Self::collect_expr_var_refs_expr(f, refs);
                for a in args {
                    Self::collect_expr_var_refs_expr(a, refs);
                }
            }
            ExprKind::Method(obj, _, _, args)
            | ExprKind::StringMethod(obj, _, args)
            | ExprKind::VecMethod(obj, _, args)
            | ExprKind::MapMethod(obj, _, args)
            | ExprKind::SetMethod(obj, _, args)
            | ExprKind::PQMethod(obj, _, args)
            | ExprKind::DequeMethod(obj, _, args)
            | ExprKind::DeferredMethod(obj, _, args) => {
                Self::collect_expr_var_refs_expr(obj, refs);
                for a in args {
                    Self::collect_expr_var_refs_expr(a, refs);
                }
            }
            ExprKind::Field(obj, _, _) => Self::collect_expr_var_refs_expr(obj, refs),
            ExprKind::Index(a, i) => {
                Self::collect_expr_var_refs_expr(a, refs);
                Self::collect_expr_var_refs_expr(i, refs);
            }
            ExprKind::IfExpr(i) => {
                Self::collect_expr_var_refs_expr(&i.cond, refs);
                Self::collect_expr_var_refs_block(&i.then, refs);
                if let Some(els) = &i.els {
                    Self::collect_expr_var_refs_block(els, refs);
                }
            }
            ExprKind::Ternary(c, t, f) => {
                Self::collect_expr_var_refs_expr(c, refs);
                Self::collect_expr_var_refs_expr(t, refs);
                Self::collect_expr_var_refs_expr(f, refs);
            }
            ExprKind::Struct(_, fields) | ExprKind::VariantCtor(_, _, _, fields) => {
                for fi in fields {
                    Self::collect_expr_var_refs_expr(&fi.value, refs);
                }
            }
            ExprKind::Select(arms, default) => {
                for arm in arms {
                    Self::collect_expr_var_refs_expr(&arm.chan, refs);
                    if let Some(v) = &arm.value {
                        Self::collect_expr_var_refs_expr(v, refs);
                    }
                    Self::collect_expr_var_refs_block(&arm.body, refs);
                }
                if let Some(def) = default {
                    Self::collect_expr_var_refs_block(def, refs);
                }
            }
            ExprKind::DynDispatch(obj, _, _, args)
            | ExprKind::Send(obj, _, _, _, args)
            | ExprKind::Pipe(obj, _, _, args) => {
                Self::collect_expr_var_refs_expr(obj, refs);
                for a in args {
                    Self::collect_expr_var_refs_expr(a, refs);
                }
            }
            ExprKind::Builtin(_, args) => {
                for a in args {
                    Self::collect_expr_var_refs_expr(a, refs);
                }
            }
            ExprKind::ChannelSend(a, b)
            | ExprKind::AtomicStore(a, b)
            | ExprKind::AtomicAdd(a, b)
            | ExprKind::AtomicSub(a, b) => {
                Self::collect_expr_var_refs_expr(a, refs);
                Self::collect_expr_var_refs_expr(b, refs);
            }
            ExprKind::ChannelRecv(e)
            | ExprKind::CoroutineNext(e)
            | ExprKind::Yield(e)
            | ExprKind::DynCoerce(e, _, _)
            | ExprKind::AsFormat(e, _)
            | ExprKind::AtomicLoad(e)
            | ExprKind::Slice(e, _, _)
            | ExprKind::Grad(e) => {
                Self::collect_expr_var_refs_expr(e, refs);
                if let ExprKind::Slice(_, lo, hi) = &expr.kind {
                    Self::collect_expr_var_refs_expr(lo, refs);
                    Self::collect_expr_var_refs_expr(hi, refs);
                }
            }
            ExprKind::ChannelCreate(_, cap) => Self::collect_expr_var_refs_expr(cap, refs),
            ExprKind::ListComp(body, _, _, iter, end, cond) => {
                Self::collect_expr_var_refs_expr(body, refs);
                Self::collect_expr_var_refs_expr(iter, refs);
                if let Some(e) = end {
                    Self::collect_expr_var_refs_expr(e, refs);
                }
                if let Some(c) = cond {
                    Self::collect_expr_var_refs_expr(c, refs);
                }
            }
            ExprKind::CoroutineCreate(_, stmts) => Self::collect_expr_var_refs_block(stmts, refs),
            ExprKind::AtomicCas(a, b, c) => {
                Self::collect_expr_var_refs_expr(a, refs);
                Self::collect_expr_var_refs_expr(b, refs);
                Self::collect_expr_var_refs_expr(c, refs);
            }
            ExprKind::Einsum(_, args) => {
                for a in args {
                    Self::collect_expr_var_refs_expr(a, refs);
                }
            }
            ExprKind::Builder(_, fields) => {
                for (_, e) in fields {
                    Self::collect_expr_var_refs_expr(e, refs);
                }
            }
            ExprKind::Block(stmts) => Self::collect_expr_var_refs_block(stmts, refs),
            ExprKind::Lambda(_, body) => Self::collect_expr_var_refs_block(body, refs),
            _ => {}
        }
    }
}
