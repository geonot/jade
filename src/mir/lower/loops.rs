use super::super::*;
use super::Lowerer;
use crate::hir;
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::HashSet;

impl Lowerer {
    pub(super) fn lower_stmt_loops(&mut self, stmt: &hir::Stmt) -> ValueId {
        match stmt {
            hir::Stmt::While(w) => {
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&w.body, &mut assigned);

                self.demote_vars_to_memory(&assigned, w.span);

                let pre_loop_vars: HashSet<Symbol> = self.var_map.keys().cloned().collect();

                let cond_bb = self.new_block("while.cond");
                let body_bb = self.new_block("while.body");
                let exit_bb = self.new_block("while.exit");

                self.set_terminator(Terminator::Goto(cond_bb));
                self.switch_to(cond_bb);
                let cond = self.lower_expr(&w.cond);
                self.set_terminator(Terminator::Branch(cond, body_bb, exit_bb));

                self.loop_stack.push((cond_bb, exit_bb));
                self.switch_to(body_bb);
                self.lower_block_stmts(&w.body);
                if !self.current_block_has_terminator() {
                    self.set_terminator(Terminator::Goto(cond_bb));
                }
                self.loop_stack.pop();

                self.var_map.retain(|k, _| pre_loop_vars.contains(k));

                self.switch_to(exit_bb);
                self.emit(InstKind::Void, Type::Void, w.span)
            }
            hir::Stmt::For(f) => {
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&f.body, &mut assigned);
                self.demote_vars_to_memory(&assigned, f.span);

                let pre_loop_vars: HashSet<Symbol> = self.var_map.keys().cloned().collect();

                let iter_val = self.lower_expr(&f.iter);
                let cond_bb = self.new_block("for.cond");
                let body_bb = self.new_block("for.body");
                let inc_bb = self.new_block("for.inc");
                let exit_bb = self.new_block("for.exit");

                if let Some(ref lab) = f.label {
                    self.label_stack.push((lab.clone(), inc_bb, exit_bb));
                }

                if let Some(ref end_expr) = f.end {
                    let end_val = self.lower_expr(end_expr);
                    let step_val = if let Some(ref step_expr) = f.step {
                        self.lower_expr(step_expr)
                    } else {
                        self.emit(InstKind::IntConst(1), Type::I64, f.span)
                    };

                    self.emit_void_typed(
                        InstKind::Store(f.bind.clone(), iter_val),
                        f.bind_ty.clone(),
                        f.span,
                    );
                    self.var_map.insert(f.bind.clone(), iter_val);

                    if let Some(ref b2) = f.bind2 {
                        let zero = self.emit(InstKind::IntConst(0), Type::I64, f.span);
                        self.emit_void_typed(InstKind::Store(b2.clone(), zero), Type::I64, f.span);
                    }
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    let counter =
                        self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), f.span);
                    let cmp = self.emit(
                        InstKind::Cmp(CmpOp::Lt, counter, end_val, Type::I64),
                        Type::Bool,
                        f.span,
                    );
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);

                    self.var_map.insert(f.bind.clone(), counter);

                    if let Some(ref b2) = f.bind2 {
                        let idx = self.emit(InstKind::Load(b2.clone()), Type::I64, f.span);
                        self.var_map.insert(b2.clone(), idx);
                    }
                    self.lower_block_stmts(&f.body);
                    if !self.current_block_has_terminator() {
                        self.set_terminator(Terminator::Goto(inc_bb));
                    }
                    self.loop_stack.pop();

                    self.switch_to(inc_bb);
                    let cur = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), f.span);
                    let next = self.emit(
                        InstKind::BinOp(BinOp::Add, cur, step_val),
                        f.bind_ty.clone(),
                        f.span,
                    );
                    self.emit_void_typed(
                        InstKind::Store(f.bind.clone(), next),
                        f.bind_ty.clone(),
                        f.span,
                    );

                    if let Some(ref b2) = f.bind2 {
                        let one = self.emit(InstKind::IntConst(1), Type::I64, f.span);
                        let cur_idx = self.emit(InstKind::Load(b2.clone()), Type::I64, f.span);
                        let next_idx =
                            self.emit(InstKind::BinOp(BinOp::Add, cur_idx, one), Type::I64, f.span);
                        self.emit_void_typed(
                            InstKind::Store(b2.clone(), next_idx),
                            Type::I64,
                            f.span,
                        );
                    }
                    self.set_terminator(Terminator::Goto(cond_bb));
                } else if matches!(f.iter.ty, Type::I64 | Type::I32 | Type::F64) {
                    let zero = self.emit(InstKind::IntConst(0), Type::I64, f.span);
                    let one = self.emit(InstKind::IntConst(1), Type::I64, f.span);
                    let end_val = iter_val;

                    self.emit_void_typed(
                        InstKind::Store(f.bind.clone(), zero),
                        f.bind_ty.clone(),
                        f.span,
                    );
                    self.var_map.insert(f.bind.clone(), zero);
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    let counter =
                        self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), f.span);
                    let cmp = self.emit(
                        InstKind::Cmp(CmpOp::Lt, counter, end_val, Type::I64),
                        Type::Bool,
                        f.span,
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
                    let cur = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), f.span);
                    let next = self.emit(
                        InstKind::BinOp(BinOp::Add, cur, one),
                        f.bind_ty.clone(),
                        f.span,
                    );
                    self.emit_void_typed(
                        InstKind::Store(f.bind.clone(), next),
                        f.bind_ty.clone(),
                        f.span,
                    );
                    self.set_terminator(Terminator::Goto(cond_bb));
                } else if matches!(f.iter.ty, Type::Coroutine(_) | Type::Generator(_)) {
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);

                    let _resume = self.emit(
                        InstKind::Call("__gen_resume".into(), vec![iter_val]),
                        Type::Void,
                        f.span,
                    );

                    let done = self.emit(
                        InstKind::Call("__gen_done".into(), vec![iter_val]),
                        Type::Bool,
                        f.span,
                    );

                    self.set_terminator(Terminator::Branch(done, exit_bb, body_bb));

                    self.loop_stack.push((cond_bb, exit_bb));
                    self.switch_to(body_bb);
                    let val = self.emit(
                        InstKind::Call("__gen_next_val".into(), vec![iter_val]),
                        f.bind_ty.clone(),
                        f.span,
                    );
                    self.var_map.insert(f.bind.clone(), val);
                    self.lower_block_stmts(&f.body);
                    if !self.current_block_has_terminator() {
                        self.set_terminator(Terminator::Goto(cond_bb));
                    }
                    self.loop_stack.pop();
                } else {
                    let zero = self.emit(InstKind::IntConst(0), Type::I64, f.span);
                    let one = self.emit(InstKind::IntConst(1), Type::I64, f.span);
                    let idx_name = Symbol::intern(&format!("__for_idx_{}", f.bind));
                    self.emit_void_typed(InstKind::Store(idx_name, zero), Type::I64, f.span);
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    let len = self.emit(InstKind::VecLen(iter_val), Type::I64, f.span);
                    let idx = self.emit(InstKind::Load(idx_name), Type::I64, f.span);
                    let cmp = self.emit(
                        InstKind::Cmp(CmpOp::Lt, idx, len, Type::I64),
                        Type::Bool,
                        f.span,
                    );
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);
                    let elem = self.emit(
                        InstKind::IndexUnchecked(iter_val, idx),
                        f.bind_ty.clone(),
                        f.span,
                    );
                    self.var_map.insert(f.bind.clone(), elem);

                    if let Some(ref b2) = f.bind2 {
                        self.var_map.insert(b2.clone(), idx);
                    }
                    self.lower_block_stmts(&f.body);
                    if !self.current_block_has_terminator() {
                        self.set_terminator(Terminator::Goto(inc_bb));
                    }
                    self.loop_stack.pop();

                    self.switch_to(inc_bb);
                    let cur_idx = self.emit(InstKind::Load(idx_name), Type::I64, f.span);
                    let next_idx =
                        self.emit(InstKind::BinOp(BinOp::Add, cur_idx, one), Type::I64, f.span);
                    self.emit_void_typed(InstKind::Store(idx_name, next_idx), Type::I64, f.span);
                    self.set_terminator(Terminator::Goto(cond_bb));
                }

                self.var_map.retain(|k, _| pre_loop_vars.contains(k));

                if f.label.is_some() {
                    self.label_stack.pop();
                }

                self.switch_to(exit_bb);

                self.var_map.remove(&f.bind);
                self.mem_vars.insert(f.bind.clone());
                self.emit(InstKind::Void, Type::Void, f.span)
            }
            hir::Stmt::Loop(l) => {
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&l.body, &mut assigned);
                self.demote_vars_to_memory(&assigned, l.span);

                let pre_loop_vars: HashSet<Symbol> = self.var_map.keys().cloned().collect();

                let body_bb = self.new_block("loop.body");
                let exit_bb = self.new_block("loop.exit");

                self.set_terminator(Terminator::Goto(body_bb));
                self.loop_stack.push((body_bb, exit_bb));
                self.switch_to(body_bb);
                self.lower_block_stmts(&l.body);
                if !self.current_block_has_terminator() {
                    self.set_terminator(Terminator::Goto(body_bb));
                }
                self.loop_stack.pop();

                self.var_map.retain(|k, _| pre_loop_vars.contains(k));

                self.switch_to(exit_bb);
                self.emit(InstKind::Void, Type::Void, l.span)
            }
            hir::Stmt::SimFor(f, span) => {
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&f.body, &mut assigned);
                self.demote_vars_to_memory(&assigned, *span);

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
            _ => unreachable!("statement dispatched to wrong MIR lowering module"),
        }
    }
}
