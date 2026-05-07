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
    pub(super) fn lower_stmt_p2(&mut self, stmt: &hir::Stmt) -> Option<ValueId> {
        Some(match stmt {
            hir::Stmt::Drop(_, name, ty, span) => {
                if let Some(&val) = self.var_map.get(name) {
                    self.emit_void(InstKind::Drop(val, ty.clone()), *span);
                }
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::TupleBind(bindings, value, _span) => {
                let val = self.lower_expr(value);
                for (i, (_id, name, bind_ty)) in bindings.iter().enumerate() {
                    let idx = self.emit(InstKind::IntConst(i as i64), Type::I64, Span::dummy());
                    let elem = self.emit(InstKind::Index(val, idx), bind_ty.clone(), Span::dummy());
                    if self.mem_vars.contains(name) {
                        self.emit(InstKind::Store(name.clone(), elem), Type::Void, Span::dummy());
                    } else {
                        self.var_map.insert(name.clone(), elem);
                    }
                }
                val
            }

            hir::Stmt::ErrReturn(expr, _ty, span) => {
                let v = self.lower_expr(expr);
                self.lower_deferred_in_reverse();
                self.set_terminator(Terminator::Return(Some(v)));
                let dead = self.new_block("after.err_return");
                self.switch_to(dead);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::ChannelClose(ch, span) => {
                let c = self.lower_expr(ch);
                self.emit(
                    InstKind::Call("__chan_close".into(), vec![c]),
                    Type::Void,
                    *span,
                )
            }

            hir::Stmt::Stop(expr, span) => {
                let v = self.lower_expr(expr);
                self.emit(InstKind::Call("__stop".into(), vec![v]), Type::Void, *span)
            }

            hir::Stmt::Asm(asm) => {
                let input_vals: Vec<_> =
                    asm.inputs.iter().map(|(_, e)| self.lower_expr(e)).collect();
                self.emit(
                    InstKind::InlineAsm(asm.template.clone(), input_vals),
                    Type::Void,
                    asm.span,
                )
            }

            _ => return None,
        })
    }
}
