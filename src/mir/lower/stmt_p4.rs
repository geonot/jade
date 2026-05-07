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
    pub(super) fn lower_stmt_p4(&mut self, stmt: &hir::Stmt) -> Option<ValueId> {
        Some(match stmt {
            hir::Stmt::Transaction(body, span) => {
                self.emit(
                    InstKind::Call("__txn_begin".into(), vec![]),
                    Type::Void,
                    *span,
                );
                self.lower_block_stmts(body);
                self.emit(
                    InstKind::Call("__txn_commit".into(), vec![]),
                    Type::Void,
                    *span,
                )
            }

            hir::Stmt::SimFor(f, span) => {
                // NOTE: sim for is lowered as sequential for — parallelism
                // semantics are not yet implemented in MIR codegen. This is
                // an accepted limitation; future work to emit parallel
                // execution primitives or mark loops for LLVM vectorization.

                // Demote variables assigned inside sim-for body to memory.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&f.body, &mut assigned);
                self.demote_vars_to_memory(&assigned, *span);

                // Parallel for — lower same as sequential for in MIR.
                let iter_val = self.lower_expr(&f.iter);
                let cond_bb = self.new_block("simfor.cond");
                let body_bb = self.new_block("simfor.body");
                let inc_bb = self.new_block("simfor.inc");
                let exit_bb = self.new_block("simfor.exit");

                if let Some(ref end_expr) = f.end {
                    let end_val = self.lower_expr(end_expr);
                    let step_val = if let Some(ref step_expr) = f.step {
                        self.lower_expr(step_expr)
                    } else {
                        self.emit(InstKind::IntConst(1), Type::I64, *span)
                    };
                    self.emit_void_typed(
                        InstKind::Store(f.bind.clone(), iter_val),
                        f.bind_ty.clone(),
                        *span,
                    );
                    self.var_map.insert(f.bind.clone(), iter_val);
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    let counter =
                        self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), *span);
                    let cmp = self.emit(
                        InstKind::Cmp(CmpOp::Lt, counter, end_val, Type::I64),
                        Type::Bool,
                        *span,
                    );
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);
                    self.var_map.insert(f.bind.clone(), counter);
                    self.lower_block_stmts(&f.body);
                    if !self.current_block_has_terminator() {
                        self.set_terminator(Terminator::Goto(inc_bb));
                    }
                    self.loop_stack.pop();

                    self.switch_to(inc_bb);
                    let cur = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), *span);
                    let next = self.emit(
                        InstKind::BinOp(BinOp::Add, cur, step_val),
                        f.bind_ty.clone(),
                        *span,
                    );
                    self.emit_void_typed(
                        InstKind::Store(f.bind.clone(), next),
                        f.bind_ty.clone(),
                        *span,
                    );
                    self.set_terminator(Terminator::Goto(cond_bb));
                } else if matches!(f.iter.ty, Type::I64 | Type::I32 | Type::F64) {
                    // Implicit range: `sim for i in N` means 0..N
                    let zero = self.emit(InstKind::IntConst(0), Type::I64, *span);
                    let one = self.emit(InstKind::IntConst(1), Type::I64, *span);
                    let end_val = iter_val;

                    self.emit_void_typed(
                        InstKind::Store(f.bind.clone(), zero),
                        f.bind_ty.clone(),
                        *span,
                    );
                    self.var_map.insert(f.bind.clone(), zero);
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    let counter =
                        self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), *span);
                    let cmp = self.emit(
                        InstKind::Cmp(CmpOp::Lt, counter, end_val, Type::I64),
                        Type::Bool,
                        *span,
                    );
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);
                    self.var_map.insert(f.bind.clone(), counter);
                    self.lower_block_stmts(&f.body);
                    if !self.current_block_has_terminator() {
                        self.set_terminator(Terminator::Goto(inc_bb));
                    }
                    self.loop_stack.pop();

                    self.switch_to(inc_bb);
                    let cur = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), *span);
                    let next = self.emit(
                        InstKind::BinOp(BinOp::Add, cur, one),
                        f.bind_ty.clone(),
                        *span,
                    );
                    self.emit_void_typed(
                        InstKind::Store(f.bind.clone(), next),
                        f.bind_ty.clone(),
                        *span,
                    );
                    self.set_terminator(Terminator::Goto(cond_bb));
                } else {
                    let zero = self.emit(InstKind::IntConst(0), Type::I64, *span);
                    let one = self.emit(InstKind::IntConst(1), Type::I64, *span);
                    let idx_name = Symbol::intern(&format!("__simfor_idx_{}", f.bind));
                    self.emit_void_typed(InstKind::Store(idx_name, zero), Type::I64, *span);
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    let len = self.emit(InstKind::VecLen(iter_val), Type::I64, *span);
                    let idx = self.emit(InstKind::Load(idx_name), Type::I64, *span);
                    let cmp = self.emit(
                        InstKind::Cmp(CmpOp::Lt, idx, len, Type::I64),
                        Type::Bool,
                        *span,
                    );
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);
                    let elem = self.emit(
                        InstKind::IndexUnchecked(iter_val, idx),
                        f.bind_ty.clone(),
                        *span,
                    );
                    self.var_map.insert(f.bind.clone(), elem);
                    // If bind2 is present, expose the index as a user variable.
                    if let Some(ref b2) = f.bind2 {
                        self.var_map.insert(b2.clone(), idx);
                    }
                    self.lower_block_stmts(&f.body);
                    if !self.current_block_has_terminator() {
                        self.set_terminator(Terminator::Goto(inc_bb));
                    }
                    self.loop_stack.pop();

                    self.switch_to(inc_bb);
                    let cur_idx = self.emit(InstKind::Load(idx_name), Type::I64, *span);
                    let next_idx =
                        self.emit(InstKind::BinOp(BinOp::Add, cur_idx, one), Type::I64, *span);
                    self.emit_void_typed(InstKind::Store(idx_name, next_idx), Type::I64, *span);
                    self.set_terminator(Terminator::Goto(cond_bb));
                }

                self.switch_to(exit_bb);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::SimBlock(body, span) => {
                self.lower_block_stmts(body);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::UseLocal(_, _, _, _) => {
                // No-op in MIR — use declarations are resolved at HIR level
                self.emit(InstKind::Void, Type::Void, Span::dummy())
            }

            hir::Stmt::GlobalStore(name, value, _span) => {
                let val = self.lower_expr(value);
                self.emit(InstKind::GlobalStore(name.clone(), val), Type::Void, Span::dummy())
            }
            _ => return None,
        })
    }
}
