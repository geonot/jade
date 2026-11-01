use super::super::*;
use super::Lowerer;
use crate::ast::Span;
use crate::hir;
use crate::intern::Symbol;
use crate::types::Type;

impl Lowerer {
    pub(super) fn lower_stmt_loops(&mut self, stmt: &hir::Stmt) -> ValueId {
        match stmt {
            hir::Stmt::While(w) => {
                let cond_bb = self.new_block("while.cond");
                let body_bb = self.new_block("while.body");
                let exit_bb = self.new_block("while.exit");

                self.set_terminator(Terminator::Goto(cond_bb));
                self.switch_to(cond_bb);
                self.lower_cancel_checkpoint(w.span, true);

                let cond = self.lower_expr(&w.cond);
                self.set_terminator(Terminator::Branch(cond, body_bb, exit_bb));

                self.switch_to(body_bb);
                self.seal_block(body_bb);

                self.loop_stack.push((cond_bb, exit_bb));
                self.lower_block_stmts(&w.body);
                if !self.current_block_has_terminator() {
                    self.set_terminator(Terminator::Goto(cond_bb));
                }
                self.loop_stack.pop();

                self.seal_block(cond_bb);

                self.switch_to(exit_bb);
                self.seal_block(exit_bb);

                self.emit(InstKind::Void, Type::Void, w.span)
            }
            hir::Stmt::For(f) => {
                let mut f = f.clone();
                f.bind = self.alias_binder(f.bind_id, f.bind);
                if let (Some(id2), Some(b2)) = (f.bind2_id, f.bind2) {
                    f.bind2 = Some(self.alias_binder(id2, b2));
                }
                let f = &f;
                let iter_val = self.lower_expr(&f.iter);
                let cond_bb = self.new_block("for.cond");
                let body_bb = self.new_block("for.body");
                let inc_bb = self.new_block("for.inc");
                let exit_bb = self.new_block("for.exit");

                if let Some(ref lab) = f.label {
                    self.label_stack.push((*lab, inc_bb, exit_bb));
                }

                if matches!(f.iter.ty, Type::Channel(_)) && f.end.is_none() {
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    self.lower_cancel_checkpoint(f.span, true);
                    let got = self.emit(
                        InstKind::Call("__chan_iter_next".into(), vec![iter_val]),
                        Type::Bool,
                        f.span,
                    );
                    self.lower_cancel_checkpoint(f.span, false);
                    self.set_terminator(Terminator::Branch(got, body_bb, exit_bb));

                    self.loop_stack.push((cond_bb, exit_bb));
                    self.switch_to(body_bb);
                    self.seal_block(body_bb);
                    let val = self.emit(
                        InstKind::Call("__chan_iter_value".into(), vec![iter_val]),
                        f.bind_ty.clone(),
                        f.span,
                    );
                    if !self.bind_cleanup_cell(
                        f.bind_id,
                        f.bind,
                        val,
                        f.bind_ty.clone(),
                        true,
                        f.span,
                    ) {
                        self.write_var(f.bind, self.current_block, val);
                    }
                    self.lower_block_stmts(&f.body);
                    if !self.current_block_has_terminator() {
                        self.set_terminator(Terminator::Goto(cond_bb));
                    }
                    self.loop_stack.pop();
                } else if matches!(f.iter.ty, Type::Coroutine(_) | Type::Generator(_))
                    && f.end.is_none()
                {
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    self.lower_cancel_checkpoint(f.span, true);
                    let _resume = self.emit(
                        InstKind::Call("__gen_resume".into(), vec![iter_val]),
                        Type::Void,
                        f.span,
                    );

                    self.lower_cancel_checkpoint(f.span, false);
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
                    self.bind_cleanup_borrow(f.bind_id, f.bind, val, f.bind_ty.clone(), f.span);
                    self.lower_block_stmts(&f.body);
                    if !self.current_block_has_terminator() {
                        self.set_terminator(Terminator::Goto(cond_bb));
                    }
                    self.loop_stack.pop();
                } else {
                    self.lower_counted_for(
                        f,
                        iter_val,
                        f.span,
                        [cond_bb, body_bb, inc_bb, exit_bb],
                    );
                }

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
                let body_bb = self.new_block("loop.body");
                let exit_bb = self.new_block("loop.exit");

                self.set_terminator(Terminator::Goto(body_bb));

                self.switch_to(body_bb);
                self.lower_cancel_checkpoint(l.span, true);

                self.loop_stack.push((body_bb, exit_bb));
                self.lower_block_stmts(&l.body);
                if !self.current_block_has_terminator() {
                    self.set_terminator(Terminator::Goto(body_bb));
                }
                self.loop_stack.pop();

                self.seal_block(body_bb);
                self.switch_to(exit_bb);
                self.seal_block(exit_bb);

                self.emit(InstKind::Void, Type::Void, l.span)
            }
            hir::Stmt::SimFor(f, span) => {
                let mut f = f.clone();
                f.bind = self.alias_binder(f.bind_id, f.bind);
                if let (Some(id2), Some(b2)) = (f.bind2_id, f.bind2) {
                    f.bind2 = Some(self.alias_binder(id2, b2));
                }
                let f = &f;
                let iter_val = self.lower_expr(&f.iter);
                let cond_bb = self.new_block("simfor.cond");
                let body_bb = self.new_block("simfor.body");
                let inc_bb = self.new_block("simfor.inc");
                let exit_bb = self.new_block("simfor.exit");

                self.lower_counted_for(f, iter_val, *span, [cond_bb, body_bb, inc_bb, exit_bb]);

                self.seal_block(inc_bb);
                self.seal_block(cond_bb);

                self.switch_to(exit_bb);
                self.seal_block(exit_bb);

                self.emit(InstKind::Void, Type::Void, *span)
            }
            _ => unreachable!("statement dispatched to wrong MIR lowering module"),
        }
    }

    fn lower_counted_for(
        &mut self,
        f: &hir::For,
        iter_val: ValueId,
        span: Span,
        [cond_bb, body_bb, inc_bb, exit_bb]: [BlockId; 4],
    ) {
        let each = f.end.is_none() && !matches!(f.iter.ty, Type::I64 | Type::I32 | Type::F64);
        let (idx_name, idx_ty, start, end, step) = if let Some(ref end_expr) = f.end {
            let end_val = self.lower_expr(end_expr);
            let step_val = match f.step {
                Some(ref step_expr) => self.lower_expr(step_expr),
                None => self.emit(InstKind::IntConst(1), Type::I64, span),
            };
            (f.bind, f.bind_ty.clone(), iter_val, Some(end_val), step_val)
        } else {
            let zero = self.emit(InstKind::IntConst(0), Type::I64, span);
            let one = self.emit(InstKind::IntConst(1), Type::I64, span);
            if each {
                let name = Symbol::intern(&format!("__for_idx_{}", f.bind));
                (name, Type::I64, zero, None, one)
            } else {
                (f.bind, f.bind_ty.clone(), zero, Some(iter_val), one)
            }
        };
        self.write_var(idx_name, self.current_block, start);
        let pos = match f.bind2 {
            Some(b2) if !each => {
                let zero = self.emit(InstKind::IntConst(0), Type::I64, span);
                self.write_var(b2, self.current_block, zero);
                Some(b2)
            }
            _ => None,
        };
        self.set_terminator(Terminator::Goto(cond_bb));

        self.switch_to(cond_bb);
        self.lower_cancel_checkpoint(span, true);
        let idx = self.read_var(idx_name, self.current_block, idx_ty.clone(), span);
        let limit = match end {
            Some(e) => e,
            None => self.emit(InstKind::VecLen(iter_val), Type::I64, span),
        };
        let cmp = self.emit(
            InstKind::Cmp(CmpOp::Lt, idx, limit, Type::I64),
            Type::Bool,
            span,
        );
        self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

        self.loop_stack.push((inc_bb, exit_bb));
        self.switch_to(body_bb);
        self.seal_block(body_bb);
        if each {
            let elem = self.emit(
                InstKind::IndexUnchecked(iter_val, idx),
                f.bind_ty.clone(),
                span,
            );
            self.bind_cleanup_borrow(f.bind_id, f.bind, elem, f.bind_ty.clone(), span);
            if let Some(b2) = f.bind2 {
                self.write_var(b2, body_bb, idx);
            }
        }
        let pos_val = pos.map(|b2| (b2, self.read_var(b2, body_bb, Type::I64, span)));
        self.lower_block_stmts(&f.body);
        if !self.current_block_has_terminator() {
            self.set_terminator(Terminator::Goto(inc_bb));
        }
        self.loop_stack.pop();

        self.switch_to(inc_bb);
        let next = self.emit(InstKind::BinOp(BinOp::Add, idx, step), idx_ty, span);
        self.write_var(idx_name, inc_bb, next);
        if let Some((b2, pv)) = pos_val {
            let one = self.emit(InstKind::IntConst(1), Type::I64, span);
            let next_pos = self.emit(InstKind::BinOp(BinOp::Add, pv, one), Type::I64, span);
            self.write_var(b2, inc_bb, next_pos);
        }
        self.set_terminator(Terminator::Goto(cond_bb));
    }
}
