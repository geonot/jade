use super::super::*;
use super::Lowerer;
use crate::hir;
use crate::intern::Symbol;
use crate::types::Type;

impl Lowerer {
    pub(super) fn lower_stmt_loops(&mut self, stmt: &hir::Stmt) -> ValueId {
        match stmt {
            hir::Stmt::While(w) => {
                // Pure Braun SSA construction for `while`:
                //   entry --> cond_bb (UNSEALED; body's back-edge pending)
                //   cond_bb --branch(cond)--> body_bb | exit_bb
                //   body_bb (sealed early; pred = cond_bb) --> ... --> cond_bb
                //
                // Loop-carried vars need no pre-seeding: a read inside the
                // (unsealed) loop header inserts an incomplete phi whose
                // operands are filled when cond_bb is sealed after the
                // back-edge from the body is installed.
                let cond_bb = self.new_block("while.cond");
                let body_bb = self.new_block("while.body");
                let exit_bb = self.new_block("while.exit");

                self.set_terminator(Terminator::Goto(cond_bb));
                self.switch_to(cond_bb);

                let cond = self.lower_expr(&w.cond);
                self.set_terminator(Terminator::Branch(cond, body_bb, exit_bb));

                // Body: single pred (cond_bb) so safe to seal immediately.
                self.switch_to(body_bb);
                self.seal_block(body_bb);

                self.loop_stack.push((cond_bb, exit_bb));
                self.lower_block_stmts(&w.body);
                if !self.current_block_has_terminator() {
                    self.set_terminator(Terminator::Goto(cond_bb));
                }
                self.loop_stack.pop();

                // Back-edge installed; seal cond_bb (fills its incomplete phis).
                self.seal_block(cond_bb);

                self.switch_to(exit_bb);
                self.seal_block(exit_bb);

                self.emit(InstKind::Void, Type::Void, w.span)
            }
            hir::Stmt::For(f) => {
                // Pure Braun SSA for `for`:
                //   - cond_bb: UNSEALED (preds = entry + inc_bb back-edge);
                //     seal AFTER inc_bb terminates Goto(cond_bb).
                //   - body_bb: sealed immediately (single pred cond_bb).
                //   - inc_bb: UNSEALED until body completes (continues land
                //     here as additional preds); seal AFTER body lowering.
                //   - exit_bb: UNSEALED until body completes (breaks land
                //     here as additional preds); seal AFTER body lowering.
                //
                // The bind / bind2 / __for_idx counters remain on Load/Store
                // (they are loop-internal, not user variables). User-assigned
                // vars in the body are Braun-managed via incomplete phis.
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
                    self.write_var(f.bind.clone(), self.current_block, iter_val);

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
                    self.seal_block(body_bb);

                    self.write_var(f.bind.clone(), self.current_block, counter);

                    if let Some(ref b2) = f.bind2 {
                        let idx = self.emit(InstKind::Load(b2.clone()), Type::I64, f.span);
                        self.write_var(b2.clone(), self.current_block, idx);
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
                    self.write_var(f.bind.clone(), self.current_block, zero);
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
                    self.seal_block(body_bb);
                    self.write_var(f.bind.clone(), self.current_block, counter);
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
                    self.seal_block(body_bb);
                    let val = self.emit(
                        InstKind::Call("__gen_next_val".into(), vec![iter_val]),
                        f.bind_ty.clone(),
                        f.span,
                    );
                    self.write_var(f.bind.clone(), self.current_block, val);
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
                    self.seal_block(body_bb);
                    let elem = self.emit(
                        InstKind::IndexUnchecked(iter_val, idx),
                        f.bind_ty.clone(),
                        f.span,
                    );
                    self.write_var(f.bind.clone(), self.current_block, elem);

                    if let Some(ref b2) = f.bind2 {
                        self.write_var(b2.clone(), self.current_block, idx);
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

                // All loop edges installed. Seal in order: inc_bb (preds:
                // body + continues), cond_bb (preds: entry + inc back-edge),
                // exit_bb (preds: cond false + breaks). seal_block on a
                // block with no incomplete phis is a no-op.
                self.seal_block(inc_bb);
                self.seal_block(cond_bb);

                if f.label.is_some() {
                    self.label_stack.pop();
                }

                self.switch_to(exit_bb);
                self.seal_block(exit_bb);

                self.emit(InstKind::Void, Type::Void, f.span)
            }
            hir::Stmt::Loop(l) => {
                // Pure Braun SSA for `loop { ... }`:
                //   entry --> body_bb (UNSEALED; back-edge from body pending)
                //   body_bb --> body_bb (back-edge)
                //   body_bb --break--> exit_bb (via Terminator::Goto from break)
                let body_bb = self.new_block("loop.body");
                let exit_bb = self.new_block("loop.exit");

                self.set_terminator(Terminator::Goto(body_bb));

                self.switch_to(body_bb);

                self.loop_stack.push((body_bb, exit_bb));
                self.lower_block_stmts(&l.body);
                if !self.current_block_has_terminator() {
                    self.set_terminator(Terminator::Goto(body_bb));
                }
                self.loop_stack.pop();

                // All back-edges + break edges installed; seal both blocks.
                self.seal_block(body_bb);
                self.switch_to(exit_bb);
                self.seal_block(exit_bb);

                self.emit(InstKind::Void, Type::Void, l.span)
            }
            hir::Stmt::SimFor(f, span) => {
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
                    self.write_var(f.bind.clone(), self.current_block, iter_val);
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
                    self.seal_block(body_bb);
                    self.write_var(f.bind.clone(), self.current_block, counter);
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
                    self.write_var(f.bind.clone(), self.current_block, zero);
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
                    self.seal_block(body_bb);
                    self.write_var(f.bind.clone(), self.current_block, counter);
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
                    self.seal_block(body_bb);
                    let elem = self.emit(
                        InstKind::IndexUnchecked(iter_val, idx),
                        f.bind_ty.clone(),
                        *span,
                    );
                    self.write_var(f.bind.clone(), self.current_block, elem);

                    if let Some(ref b2) = f.bind2 {
                        self.write_var(b2.clone(), self.current_block, idx);
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

                self.seal_block(inc_bb);
                self.seal_block(cond_bb);

                self.switch_to(exit_bb);
                self.seal_block(exit_bb);

                self.emit(InstKind::Void, Type::Void, *span)
            }
            _ => unreachable!("statement dispatched to wrong MIR lowering module"),
        }
    }
}
