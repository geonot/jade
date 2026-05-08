use super::super::*;
use super::Lowerer;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::HashSet;

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
}
