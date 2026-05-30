use super::super::*;
use super::Lowerer;
use crate::ast::{self, Span};
use crate::hir::{self, ExprKind};
use crate::types::Type;

impl Lowerer {
    pub(super) fn lower_expr_control(&mut self, expr: &hir::Expr) -> ValueId {
        let span = expr.span;
        let ty = expr.ty.clone();
        match &expr.kind {
            ExprKind::BinOp(lhs, op, rhs) => {
                if *op == ast::BinOp::And {
                    let l = self.lower_expr(lhs);
                    let false_val = self.emit(InstKind::BoolConst(false), Type::Bool, span);
                    let rhs_bb = self.new_block("and.rhs");
                    let merge_bb = self.new_block("and.merge");
                    let cur_bb = self.current_block;
                    self.set_terminator(Terminator::Branch(l, rhs_bb, merge_bb));
                    self.switch_to(rhs_bb);
                    self.seal_block(rhs_bb);
                    let r = self.lower_expr(rhs);
                    let rhs_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    self.switch_to(merge_bb);
                    self.seal_block(merge_bb);
                    let phi = self.func.new_value();
                    self.func.block_mut(merge_bb).phis.push(crate::mir::Phi {
                        dest: phi,
                        ty: Type::Bool,
                        incoming: vec![(cur_bb, false_val), (rhs_end, r)],
                    });
                    phi
                } else if *op == ast::BinOp::Or {
                    let l = self.lower_expr(lhs);
                    let true_val = self.emit(InstKind::BoolConst(true), Type::Bool, span);
                    let rhs_bb = self.new_block("or.rhs");
                    let merge_bb = self.new_block("or.merge");
                    let cur_bb = self.current_block;
                    self.set_terminator(Terminator::Branch(l, merge_bb, rhs_bb));
                    self.switch_to(rhs_bb);
                    self.seal_block(rhs_bb);
                    let r = self.lower_expr(rhs);
                    let rhs_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    self.switch_to(merge_bb);
                    self.seal_block(merge_bb);
                    let phi = self.func.new_value();
                    self.func.block_mut(merge_bb).phis.push(crate::mir::Phi {
                        dest: phi,
                        ty: Type::Bool,
                        incoming: vec![(cur_bb, true_val), (rhs_end, r)],
                    });
                    phi
                } else {
                    unreachable!("non-short-circuit binop lowered by expr")
                }
            }

            ExprKind::Block(stmts) => self.lower_block_expr(stmts),
            ExprKind::IfExpr(if_expr) => {
                // Pure Braun SSA: branches write into their per-block
                // `current_def`; reads after the merge build phis on demand.
                let cond_val = self.lower_expr(&if_expr.cond);
                let then_bb = self.new_block("if.then");
                let merge_bb = self.new_block("if.merge");

                let first_elif_bb = if !if_expr.elifs.is_empty() {
                    Some(self.new_block("elif.test"))
                } else {
                    None
                };
                let else_bb = if if_expr.els.is_some() && first_elif_bb.is_none() {
                    self.new_block("if.else")
                } else {
                    first_elif_bb.unwrap_or(merge_bb)
                };

                self.set_terminator(Terminator::Branch(cond_val, then_bb, else_bb));

                // Each branch block has a single predecessor (recorded by the
                // Branch terminator above / the per-elif Branch below) so it is
                // safe to seal immediately. Sealing BEFORE lowering the body is
                // required: reads in an unsealed block insert incomplete phis
                // that are only filled at seal time — if the block is never
                // sealed those phis stay empty and LLVM rejects them.
                self.switch_to(then_bb);
                self.seal_block(then_bb);
                let then_val = self.lower_block_expr(&if_expr.then);
                let then_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                let mut elif_vals: Vec<(BlockId, ValueId)> = Vec::new();
                let mut prev_false_bb = first_elif_bb;
                for (i, (elif_cond, elif_body)) in if_expr.elifs.iter().enumerate() {
                    let elif_test = prev_false_bb.unwrap();
                    let elif_body_bb = self.new_block("elif.body");

                    let is_last_elif = i + 1 == if_expr.elifs.len();
                    let elif_false_bb = if is_last_elif {
                        if if_expr.els.is_some() {
                            Some(self.new_block("if.else"))
                        } else {
                            None
                        }
                    } else {
                        Some(self.new_block("elif.test"))
                    };

                    self.switch_to(elif_test);
                    self.seal_block(elif_test);
                    let c = self.lower_expr(elif_cond);
                    self.set_terminator(Terminator::Branch(
                        c,
                        elif_body_bb,
                        elif_false_bb.unwrap_or(merge_bb),
                    ));

                    self.switch_to(elif_body_bb);
                    self.seal_block(elif_body_bb);
                    let elif_val = self.lower_block_expr(elif_body);
                    let elif_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    elif_vals.push((elif_end, elif_val));

                    prev_false_bb = elif_false_bb;
                }

                let else_val_info = if let Some(els) = &if_expr.els {
                    let else_target = prev_false_bb.unwrap_or(else_bb);
                    self.switch_to(else_target);
                    self.seal_block(else_target);
                    let else_val = self.lower_block_expr(els);
                    let else_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    Some((else_end, else_val))
                } else {
                    None
                };

                self.switch_to(merge_bb);
                self.seal_block(merge_bb);
                // Reads after the merge build phis lazily via `read_var`.
                if !matches!(ty, Type::Void) && (else_val_info.is_some() || !elif_vals.is_empty()) {
                    let mut incoming = vec![(then_end, then_val)];
                    for &(bb, v) in &elif_vals {
                        incoming.push((bb, v));
                    }
                    if let Some((eb, ev)) = else_val_info {
                        incoming.push((eb, ev));
                    }

                    // Branches that diverged (return/break/continue) end in a
                    // dead `after.*` block not wired into merge_bb; drop them so
                    // the result phi only references real predecessors.
                    incoming.retain(|(bb, _)| !self.unreachable_blocks.contains(bb));
                    if incoming.is_empty() {
                        self.emit(InstKind::Void, Type::Void, span)
                    } else {
                        let result = self.new_value();
                        self.func.block_mut(merge_bb).phis.push(Phi {
                            dest: result,
                            ty: ty.clone(),
                            incoming,
                        });
                        result
                    }
                } else if !matches!(ty, Type::Void) {
                    let void_val = self.emit(InstKind::Void, Type::Void, span);
                    let result = self.new_value();
                    self.func.block_mut(merge_bb).phis.push(Phi {
                        dest: result,
                        ty: ty.clone(),
                        incoming: vec![(then_end, then_val), (self.current_block, void_val)],
                    });
                    result
                } else {
                    self.emit(InstKind::Void, Type::Void, span)
                }
            }
            ExprKind::Ternary(cond, then_expr, else_expr) => {
                let cond_val = self.lower_expr(cond);
                let then_bb = self.new_block("ternary.then");
                let else_bb = self.new_block("ternary.else");
                let merge_bb = self.new_block("ternary.merge");

                self.set_terminator(Terminator::Branch(cond_val, then_bb, else_bb));

                self.switch_to(then_bb);
                self.seal_block(then_bb);
                let then_val = self.lower_expr(then_expr);
                let then_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                self.switch_to(else_bb);
                self.seal_block(else_bb);
                let else_val = self.lower_expr(else_expr);
                let else_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                self.switch_to(merge_bb);
                self.seal_block(merge_bb);
                let result = self.new_value();
                self.func.block_mut(merge_bb).phis.push(Phi {
                    dest: result,
                    ty: ty.clone(),
                    incoming: vec![(then_end, then_val), (else_end, else_val)],
                });
                result
            }
            _ => unreachable!("expression dispatched to wrong MIR lowering module"),
        }
    }
}

impl Lowerer {
    pub(super) fn lower_block_expr(&mut self, stmts: &[hir::Stmt]) -> ValueId {
        let mut last = self.emit(InstKind::Void, Type::Void, Span::dummy());
        for stmt in stmts {
            last = self.lower_stmt(stmt);
        }
        last
    }
}
