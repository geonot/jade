use super::super::*;
use super::Lowerer;
use crate::ast::{self, Span};
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::HashSet;

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
                    let r = self.lower_expr(rhs);
                    let rhs_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    self.switch_to(merge_bb);
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
                    let r = self.lower_expr(rhs);
                    let rhs_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    self.switch_to(merge_bb);
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
                // Demote variables assigned in branches to memory
                // so the merge point reads current values via Load.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&if_expr.then, &mut assigned);
                for (_, elif_body) in &if_expr.elifs {
                    Self::collect_assigned_vars(elif_body, &mut assigned);
                }
                if let Some(els) = &if_expr.els {
                    Self::collect_assigned_vars(els, &mut assigned);
                }
                let pre_existing: HashSet<Symbol> = assigned
                    .iter()
                    .filter(|n| self.var_map.contains_key(*n))
                    .cloned()
                    .collect();
                self.demote_vars_to_memory(&pre_existing, span);

                // Variables first defined in BOTH then and else → promote to mem_vars
                if if_expr.els.is_some() || !if_expr.elifs.is_empty() {
                    let mut then_binds = HashSet::new();
                    Self::collect_new_binds(&if_expr.then, &mut then_binds);
                    let mut other_binds = HashSet::new();
                    for (_, elif_body) in &if_expr.elifs {
                        Self::collect_new_binds(elif_body, &mut other_binds);
                    }
                    if let Some(els) = &if_expr.els {
                        Self::collect_new_binds(els, &mut other_binds);
                    }
                    for name in then_binds.intersection(&other_binds) {
                        if !self.var_map.contains_key(name) && !self.mem_vars.contains(name) {
                            self.mem_vars.insert(name.clone());
                        }
                    }
                }

                let cond_val = self.lower_expr(&if_expr.cond);
                let then_bb = self.new_block("if.then");
                let merge_bb = self.new_block("if.merge");

                // Determine the false-branch target:
                // elif chain first, then else, then merge.
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

                // Then branch
                self.switch_to(then_bb);
                let then_val = self.lower_block_expr(&if_expr.then);
                let then_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                // Lower elif chains
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
                    let c = self.lower_expr(elif_cond);
                    self.set_terminator(Terminator::Branch(
                        c,
                        elif_body_bb,
                        elif_false_bb.unwrap_or(merge_bb),
                    ));

                    self.switch_to(elif_body_bb);
                    let elif_val = self.lower_block_expr(elif_body);
                    let elif_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    elif_vals.push((elif_end, elif_val));

                    prev_false_bb = elif_false_bb;
                }

                // Else branch
                let else_val_info = if let Some(els) = &if_expr.els {
                    let else_target = prev_false_bb.unwrap_or(else_bb);
                    self.switch_to(else_target);
                    let else_val = self.lower_block_expr(els);
                    let else_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    Some((else_end, else_val))
                } else {
                    None
                };

                // Merge
                self.switch_to(merge_bb);
                if !matches!(ty, Type::Void) && (else_val_info.is_some() || !elif_vals.is_empty()) {
                    let mut incoming = vec![(then_end, then_val)];
                    for &(bb, v) in &elif_vals {
                        incoming.push((bb, v));
                    }
                    if let Some((eb, ev)) = else_val_info {
                        incoming.push((eb, ev));
                    }
                    // If no else branch, add a void from the last false branch
                    if else_val_info.is_none() && elif_vals.is_empty() {
                        // No phi needed — only then branch produces a value
                    }
                    let result = self.new_value();
                    self.func.block_mut(merge_bb).phis.push(Phi {
                        dest: result,
                        ty: ty.clone(),
                        incoming,
                    });
                    result
                } else if !matches!(ty, Type::Void) {
                    // No elif/else — no phi, just pass through then value
                    // via a phi from then or a void from merge
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
                let then_val = self.lower_expr(then_expr);
                let then_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                self.switch_to(else_bb);
                let else_val = self.lower_expr(else_expr);
                let else_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                self.switch_to(merge_bb);
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
