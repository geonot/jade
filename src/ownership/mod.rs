//! Ownership and borrow checking pass operating on HIR.

use crate::intern::Symbol;
use std::collections::HashMap;

use crate::hir::*;
use crate::types::Type;

#[derive(Debug, Clone)]
pub struct OwnershipDiag {
    pub kind: DiagKind,
    pub span: crate::ast::Span,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiagKind {
    UseAfterMove,
    DoubleMutableBorrow,
    MoveOfBorrowed,
    InvalidRcDeref,
    ReturnOfBorrowed,
    WeakUpgradeWithoutCheck,
    Warning,
}

#[derive(Debug, Clone)]
struct VarState {
    ownership: Ownership,
    ty: Type,
    moved: bool,
    borrow_count: u32,
    mut_borrowed: bool,
    move_span: Option<crate::ast::Span>,
}

pub struct OwnershipVerifier {
    scopes: Vec<HashMap<DefId, VarState>>,
    pub diagnostics: Vec<OwnershipDiag>,
    fn_ret_types: HashMap<Symbol, Type>,
}

impl OwnershipVerifier {
    pub fn new() -> Self {
        Self {
            scopes: Vec::new(),
            diagnostics: Vec::new(),
            fn_ret_types: HashMap::new(),
        }
    }

    pub fn verify(&mut self, prog: &Program) -> Vec<OwnershipDiag> {
        for f in &prog.fns {
            self.fn_ret_types.insert(f.name.clone(), f.ret.clone());
        }

        for f in &prog.fns {
            self.verify_fn(f);
        }

        for td in &prog.types {
            for m in &td.methods {
                self.verify_fn(m);
            }
        }

        for ti in &prog.trait_impls {
            for m in &ti.methods {
                self.verify_fn(m);
            }
        }

        self.diagnostics.clone()
    }

    fn verify_fn(&mut self, f: &Fn) {
        self.push_scope();

        for p in &f.params {
            self.define(
                p.def_id,
                VarState {
                    ownership: p.ownership,
                    ty: p.ty.clone(),
                    moved: false,
                    borrow_count: 0,
                    mut_borrowed: false,
                    move_span: None,
                },
            );
        }

        self.verify_block(&f.body);
        self.pop_scope();
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define(&mut self, id: DefId, state: VarState) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(id, state);
        }
    }

    fn lookup(&self, id: DefId) -> Option<&VarState> {
        for scope in self.scopes.iter().rev() {
            if let Some(s) = scope.get(&id) {
                return Some(s);
            }
        }
        None
    }

    fn lookup_mut(&mut self, id: DefId) -> Option<&mut VarState> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(s) = scope.get_mut(&id) {
                return Some(s);
            }
        }
        None
    }
    fn check_use(&mut self, id: DefId, name: &str, span: crate::ast::Span) {
        if id == DefId::BUILTIN {
            return;
        }
        if let Some(state) = self.lookup(id).cloned() {
            if state.moved && state.ownership == Ownership::Owned {
                self.diagnostics.push(OwnershipDiag {
                    kind: DiagKind::UseAfterMove,
                    span,
                    message: format!(
                        "use of moved value `{name}` (moved at line {})",
                        state.move_span.map(|s| s.line).unwrap_or(0)
                    ),
                });
            }
            if state.ownership == Ownership::Weak {
                self.diagnostics.push(OwnershipDiag {
                    kind: DiagKind::WeakUpgradeWithoutCheck,
                    span,
                    message: format!(
                        "weak reference `{name}` used directly — \
                         call weak_upgrade() and check for none before use"
                    ),
                });
            }
        }
    }

    fn record_borrow(&mut self, id: DefId, mutable: bool, span: crate::ast::Span) {
        let state_snapshot = self.lookup(id).cloned();
        let Some(state) = state_snapshot else { return };

        if state.moved {
            self.diagnostics.push(OwnershipDiag {
                kind: DiagKind::MoveOfBorrowed,
                span,
                message: "cannot borrow moved value".into(),
            });
            return;
        }
        if mutable {
            if state.borrow_count > 0 || state.mut_borrowed {
                self.diagnostics.push(OwnershipDiag {
                    kind: DiagKind::DoubleMutableBorrow,
                    span,
                    message: "cannot mutably borrow: already borrowed".into(),
                });
            }
            if let Some(s) = self.lookup_mut(id) {
                s.mut_borrowed = true;
            }
        } else {
            if state.mut_borrowed {
                self.diagnostics.push(OwnershipDiag {
                    kind: DiagKind::DoubleMutableBorrow,
                    span,
                    message: "cannot immutably borrow: already mutably borrowed".into(),
                });
            }
            if let Some(s) = self.lookup_mut(id) {
                s.borrow_count += 1;
            }
        }
    }

    fn record_move(&mut self, id: DefId, span: crate::ast::Span) {
        if id == DefId::BUILTIN {
            return;
        }
        if let Some(state) = self.lookup(id).cloned() {
            if (state.ownership == Ownership::Owned || state.ownership == Ownership::BorrowMut)
                && !state.ty.is_trivially_droppable()
            {
                if let Some(s) = self.lookup_mut(id) {
                    s.moved = true;
                    s.move_span = Some(span);
                }
            }
        }
    }

    fn check_return_borrows(&mut self, expr: &Expr, span: crate::ast::Span) {
        if let ExprKind::Ref(inner) = &expr.kind {
            if let Some((def_id, name)) = Self::extract_root_var(inner) {
                if let Some(state) = self.lookup(def_id) {
                    if state.ownership == Ownership::Owned {
                        self.diagnostics.push(OwnershipDiag {
                            kind: DiagKind::ReturnOfBorrowed,
                            span,
                            message: format!(
                                "returning reference to local variable `{name}` — \
                                 value will be dropped when function returns"
                            ),
                        });
                    }
                }
            }
        }
    }

    /// Extract the root variable from an expression (follows field access and index chains).
    fn extract_root_var(expr: &Expr) -> Option<(DefId, String)> {
        match &expr.kind {
            ExprKind::Var(def_id, name) => Some((*def_id, name.as_str())),
            ExprKind::Field(obj, _, _) => Self::extract_root_var(obj),
            ExprKind::Index(obj, _) => Self::extract_root_var(obj),
            _ => None,
        }
    }

}

#[cfg(test)]
mod tests;
mod verify;
mod walks;
