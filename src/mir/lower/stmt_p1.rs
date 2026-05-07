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
    pub(super) fn lower_stmt_p1(&mut self, stmt: &hir::Stmt) -> Option<ValueId> {
        Some(match stmt {
            hir::Stmt::Loop(l) => {
                // Demote variables assigned inside loop body to memory.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&l.body, &mut assigned);
                self.demote_vars_to_memory(&assigned, l.span);

                // Snapshot var_map keys before the loop body.
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

                // Remove var_map entries for variables first defined inside the
                // loop body — they don't dominate the exit block.
                self.var_map.retain(|k, _| pre_loop_vars.contains(k));

                self.switch_to(exit_bb);
                self.emit(InstKind::Void, Type::Void, l.span)
            }

            hir::Stmt::Ret(val, _ret_ty, span) => {
                if let Some(v) = val {
                    let rv = self.lower_expr(v);
                    self.lower_deferred_in_reverse();
                    self.set_terminator(Terminator::Return(Some(rv)));
                } else {
                    self.lower_deferred_in_reverse();
                    self.set_terminator(Terminator::Return(None));
                }
                let dead = self.new_block("after.ret");
                self.switch_to(dead);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::Defer(body, span) => {
                self.function_defers.push(body.clone());
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::Break(val, span) => {
                if let Some((_, exit)) = self.loop_stack.last().copied() {
                    if let Some(v) = val {
                        let _ = self.lower_expr(v);
                    }
                    self.set_terminator(Terminator::Goto(exit));
                }
                let dead = self.new_block("after.break");
                self.switch_to(dead);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::Continue(span) => {
                if let Some((cont, _)) = self.loop_stack.last().copied() {
                    self.set_terminator(Terminator::Goto(cont));
                }
                let dead = self.new_block("after.continue");
                self.switch_to(dead);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::Match(m) => {
                // Demote variables assigned in any match arm to memory.
                let mut assigned = HashSet::new();
                for arm in &m.arms {
                    Self::collect_assigned_vars(&arm.body, &mut assigned);
                }
                // Only demote pre-existing variables (new bindings in arms are fine).
                let pre_existing: HashSet<Symbol> = assigned
                    .into_iter()
                    .filter(|v| self.var_map.contains_key(v))
                    .collect();
                self.demote_vars_to_memory(&pre_existing, m.span);

                let subj = self.lower_expr(&m.subject);
                let merge_bb = self.new_block("match.merge");

                if m.arms.is_empty() {
                    self.switch_to(merge_bb);
                    return Some(self.emit(InstKind::Void, Type::Void, m.span));
                }

                // Check if this is an integer/enum tag match (Switch) or
                // needs sequential comparison (if-else chain).
                let is_enum = matches!(m.subject.ty, Type::Enum(_));
                let has_ctor = m.arms.iter().any(|a| matches!(a.pat, Pat::Ctor(..)));
                let all_lit = m
                    .arms
                    .iter()
                    .all(|a| matches!(a.pat, Pat::Lit(_) | Pat::Wild(_)));
                let result_ty = m.ty.clone();
                let has_result = !matches!(result_ty, Type::Void);

                // Check for duplicate outer tags (e.g. Wrap(X) / Wrap(Y) both match tag 0).
                // If so, fall back to sequential if-else chain.
                let has_dup_tags = {
                    let mut seen = HashSet::new();
                    m.arms.iter().any(|a| {
                        if let Pat::Ctor(_, tag, _, _) = &a.pat {
                            !seen.insert(*tag)
                        } else {
                            false
                        }
                    })
                };

                // If any arm has a guard, fall back to sequential if-else chain.
                let has_guard = m.arms.iter().any(|a| a.guard.is_some());

                // Track (value, block) pairs from each arm for Phi creation.
                let mut phi_entries: Vec<(ValueId, BlockId)> = Vec::new();

                if !has_dup_tags && !has_guard && (is_enum || has_ctor || all_lit) {
                    // Switch-based match on integer/enum discriminant.
                    let disc = if is_enum || has_ctor {
                        // Extract tag from variant.
                        self.emit(InstKind::FieldGet(subj, "__tag".into()), Type::I64, m.span)
                    } else {
                        subj
                    };

                    let mut cases: Vec<(i64, BlockId)> = Vec::new();
                    let mut has_explicit_default = false;
                    let unreach_bb = self.new_block("match.unreach");
                    let mut default_bb = unreach_bb;
                    let mut arm_blocks = Vec::new();

                    for arm in &m.arms {
                        let arm_bb = self.new_block("match.arm");
                        arm_blocks.push((arm_bb, arm));

                        match &arm.pat {
                            Pat::Lit(lit_expr) => {
                                // Lower the literal to get its constant value.
                                let lit_val = self.lower_expr(lit_expr);
                                // Find the integer constant if possible.
                                if let Some(ival) = self.try_extract_int_const(lit_val) {
                                    cases.push((ival, arm_bb));
                                } else {
                                    // Non-integer literal — fallback, use as default.
                                    default_bb = arm_bb;
                                }
                            }
                            Pat::Ctor(_, tag, _, _) => {
                                cases.push((*tag as i64, arm_bb));
                            }
                            Pat::Wild(_) => {
                                default_bb = arm_bb;
                                has_explicit_default = true;
                            }
                            _ => {
                                default_bb = arm_bb;
                                has_explicit_default = true;
                            }
                        }
                    }

                    self.set_terminator(Terminator::Switch(disc, cases, default_bb));

                    // If no explicit default arm, make the unreachable block dead.
                    if !has_explicit_default {
                        self.switch_to(unreach_bb);
                        // This block should never be reached; leave as Unreachable.
                    }

                    for (arm_bb, arm) in arm_blocks {
                        self.switch_to(arm_bb);
                        // Bind pattern variables for Ctor patterns.
                        if let Pat::Ctor(_, _, sub_pats, _) = &arm.pat {
                            for (i, sp) in sub_pats.iter().enumerate() {
                                if let Pat::Bind(_, name, ty, _) = sp {
                                    let field = self.emit(
                                        InstKind::FieldGet(subj, Symbol::intern(&format!("_{i}"))),
                                        ty.clone(),
                                        arm.span,
                                    );
                                    self.var_map.insert(name.clone(), field);
                                }
                            }
                        }
                        if let Pat::Bind(_, name, _ty, _) = &arm.pat {
                            self.var_map.insert(name.clone(), subj);
                        }
                        // Bind pattern variables for Tuple patterns.
                        if let Pat::Tuple(sub_pats, _) = &arm.pat {
                            for (i, sp) in sub_pats.iter().enumerate() {
                                if let Pat::Bind(_, name, ty, _) = sp {
                                    let field = self.emit(
                                        InstKind::FieldGet(subj, Symbol::intern(&format!("_{i}"))),
                                        ty.clone(),
                                        arm.span,
                                    );
                                    self.var_map.insert(name.clone(), field);
                                }
                            }
                        }
                        // Bind pattern variables for Array patterns.
                        if let Pat::Array(sub_pats, _) = &arm.pat {
                            for (i, sp) in sub_pats.iter().enumerate() {
                                if let Pat::Bind(_, name, ty, _) = sp {
                                    let idx = self.emit(
                                        InstKind::IntConst(i as i64),
                                        Type::I64,
                                        arm.span,
                                    );
                                    let elem =
                                        self.emit(InstKind::Index(subj, idx), ty.clone(), arm.span);
                                    self.var_map.insert(name.clone(), elem);
                                }
                            }
                        }
                        let mut arm_last = self.emit(InstKind::Void, Type::Void, arm.span);
                        for s in &arm.body {
                            arm_last = self.lower_stmt(s);
                        }
                        if !self.current_block_has_terminator() {
                            if has_result {
                                phi_entries.push((arm_last, self.current_block));
                            }
                            self.set_terminator(Terminator::Goto(merge_bb));
                        }
                    }
                } else {
                    // Sequential if-else chain for complex patterns.
                    let mut next_test = self.current_block;
                    for (i, arm) in m.arms.iter().enumerate() {
                        let arm_bb = self.new_block("match.arm");
                        let is_last = i + 1 == m.arms.len();
                        let next_bb = if is_last {
                            merge_bb
                        } else {
                            self.new_block("match.next")
                        };

                        self.switch_to(next_test);

                        match &arm.pat {
                            Pat::Wild(_) => {
                                self.set_terminator(Terminator::Goto(arm_bb));
                            }
                            Pat::Bind(_, name, _ty, _) => {
                                // Bind always matches.
                                self.var_map.insert(name.clone(), subj);
                                self.set_terminator(Terminator::Goto(arm_bb));
                            }
                            Pat::Lit(lit_expr) => {
                                let lit_val = self.lower_expr(lit_expr);
                                let subj_ty = self.value_type(subj);
                                let cmp = self.emit(
                                    InstKind::Cmp(CmpOp::Eq, subj, lit_val, subj_ty),
                                    Type::Bool,
                                    arm.span,
                                );
                                self.set_terminator(Terminator::Branch(cmp, arm_bb, next_bb));
                            }
                            Pat::Ctor(_, tag, sub_pats, _) => {
                                // Compare tag, then optionally check sub-patterns.
                                let tag_val = self.emit(
                                    InstKind::FieldGet(subj, "__tag".into()),
                                    Type::I64,
                                    arm.span,
                                );
                                let tag_const =
                                    self.emit(InstKind::IntConst(*tag as i64), Type::I64, arm.span);
                                let tag_cmp = self.emit(
                                    InstKind::Cmp(CmpOp::Eq, tag_val, tag_const, Type::I64),
                                    Type::Bool,
                                    arm.span,
                                );
                                // Check if any sub-pattern needs matching (nested Ctors).
                                let needs_sub_check =
                                    sub_pats.iter().any(|sp| matches!(sp, Pat::Ctor(..)));
                                if needs_sub_check {
                                    // Tag matches → check sub-patterns.
                                    let sub_check_bb = self.new_block("match.subcheck");
                                    self.set_terminator(Terminator::Branch(
                                        tag_cmp,
                                        sub_check_bb,
                                        next_bb,
                                    ));
                                    self.switch_to(sub_check_bb);
                                    // Compare inner tags.
                                    let mut all_match = tag_cmp; // will be overwritten
                                    for (idx, sp) in sub_pats.iter().enumerate() {
                                        if let Pat::Ctor(_, inner_tag, _, _) = sp {
                                            let field_val = self.emit(
                                                InstKind::FieldGet(subj, Symbol::intern(&format!("_{idx}"))),
                                                Type::I64,
                                                arm.span,
                                            );
                                            let inner_tag_val = self.emit(
                                                InstKind::FieldGet(field_val, "__tag".into()),
                                                Type::I64,
                                                arm.span,
                                            );
                                            let inner_const = self.emit(
                                                InstKind::IntConst(*inner_tag as i64),
                                                Type::I64,
                                                arm.span,
                                            );
                                            all_match = self.emit(
                                                InstKind::Cmp(
                                                    CmpOp::Eq,
                                                    inner_tag_val,
                                                    inner_const,
                                                    Type::I64,
                                                ),
                                                Type::Bool,
                                                arm.span,
                                            );
                                        }
                                    }
                                    self.set_terminator(Terminator::Branch(
                                        all_match, arm_bb, next_bb,
                                    ));
                                } else {
                                    self.set_terminator(Terminator::Branch(
                                        tag_cmp, arm_bb, next_bb,
                                    ));
                                }
                            }
                            Pat::Or(alternatives, _) => {
                                // Or pattern: match if ANY alternative matches.
                                // Build a chain: check alt1 → arm_bb, else check alt2 → arm_bb, else ... → next_bb
                                let mut cur_test = self.current_block;
                                for (ai, alt) in alternatives.iter().enumerate() {
                                    let is_last_alt = ai + 1 == alternatives.len();
                                    let fail_bb = if is_last_alt {
                                        next_bb
                                    } else {
                                        self.new_block("or.next")
                                    };
                                    self.switch_to(cur_test);
                                    match alt {
                                        Pat::Lit(lit_expr) => {
                                            let lit_val = self.lower_expr(lit_expr);
                                            let subj_ty = self.value_type(subj);
                                            let cmp = self.emit(
                                                InstKind::Cmp(CmpOp::Eq, subj, lit_val, subj_ty),
                                                Type::Bool,
                                                arm.span,
                                            );
                                            self.set_terminator(Terminator::Branch(
                                                cmp, arm_bb, fail_bb,
                                            ));
                                        }
                                        Pat::Wild(_) => {
                                            self.set_terminator(Terminator::Goto(arm_bb));
                                        }
                                        Pat::Bind(_, name, _ty, _) => {
                                            self.var_map.insert(name.clone(), subj);
                                            self.set_terminator(Terminator::Goto(arm_bb));
                                        }
                                        Pat::Range(lo, hi, _) => {
                                            let lo_val = self.lower_expr(lo);
                                            let hi_val = self.lower_expr(hi);
                                            let subj_ty = self.value_type(subj);
                                            let ge = self.emit(
                                                InstKind::Cmp(
                                                    CmpOp::Ge,
                                                    subj,
                                                    lo_val,
                                                    subj_ty.clone(),
                                                ),
                                                Type::Bool,
                                                arm.span,
                                            );
                                            let le = self.emit(
                                                InstKind::Cmp(CmpOp::Le, subj, hi_val, subj_ty),
                                                Type::Bool,
                                                arm.span,
                                            );
                                            let in_range = self.emit(
                                                InstKind::BinOp(BinOp::And, ge, le),
                                                Type::Bool,
                                                arm.span,
                                            );
                                            self.set_terminator(Terminator::Branch(
                                                in_range, arm_bb, fail_bb,
                                            ));
                                        }
                                        _ => {
                                            // Sub-pattern types we don't handle in or: fallback to match
                                            self.set_terminator(Terminator::Goto(arm_bb));
                                        }
                                    }
                                    cur_test = fail_bb;
                                }
                            }
                            Pat::Range(lo, hi, _) => {
                                let lo_val = self.lower_expr(lo);
                                let hi_val = self.lower_expr(hi);
                                let subj_ty = self.value_type(subj);
                                let ge = self.emit(
                                    InstKind::Cmp(CmpOp::Ge, subj, lo_val, subj_ty.clone()),
                                    Type::Bool,
                                    arm.span,
                                );
                                let le = self.emit(
                                    InstKind::Cmp(CmpOp::Le, subj, hi_val, subj_ty),
                                    Type::Bool,
                                    arm.span,
                                );
                                let in_range = self.emit(
                                    InstKind::BinOp(BinOp::And, ge, le),
                                    Type::Bool,
                                    arm.span,
                                );
                                self.set_terminator(Terminator::Branch(in_range, arm_bb, next_bb));
                            }
                            _ => {
                                // Fallback: unconditional (catches Tuple, Array, etc.)
                                self.set_terminator(Terminator::Goto(arm_bb));
                            }
                        }

                        self.switch_to(arm_bb);
                        // Bind pattern variables for Ctor patterns (sequential match).
                        if let Pat::Ctor(_, _, sub_pats, _) = &arm.pat {
                            for (i, sp) in sub_pats.iter().enumerate() {
                                if let Pat::Bind(_, name, ty, _) = sp {
                                    let field = self.emit(
                                        InstKind::FieldGet(subj, Symbol::intern(&format!("_{i}"))),
                                        ty.clone(),
                                        arm.span,
                                    );
                                    self.var_map.insert(name.clone(), field);
                                }
                            }
                        }
                        // Bind pattern variables for Tuple patterns (sequential match).
                        if let Pat::Tuple(sub_pats, _) = &arm.pat {
                            for (i, sp) in sub_pats.iter().enumerate() {
                                if let Pat::Bind(_, name, ty, _) = sp {
                                    let field = self.emit(
                                        InstKind::FieldGet(subj, Symbol::intern(&format!("_{i}"))),
                                        ty.clone(),
                                        arm.span,
                                    );
                                    self.var_map.insert(name.clone(), field);
                                }
                            }
                        }
                        // Bind pattern variables for Array patterns (sequential match).
                        if let Pat::Array(sub_pats, _) = &arm.pat {
                            for (i, sp) in sub_pats.iter().enumerate() {
                                if let Pat::Bind(_, name, ty, _) = sp {
                                    let idx = self.emit(
                                        InstKind::IntConst(i as i64),
                                        Type::I64,
                                        arm.span,
                                    );
                                    let elem =
                                        self.emit(InstKind::Index(subj, idx), ty.clone(), arm.span);
                                    self.var_map.insert(name.clone(), elem);
                                }
                            }
                        }
                        // Evaluate guard (when clause) — if false, skip to next arm.
                        if let Some(guard_expr) = &arm.guard {
                            let guard_val = self.lower_expr(guard_expr);
                            let body_bb = self.new_block("match.guard_pass");
                            self.set_terminator(Terminator::Branch(guard_val, body_bb, next_bb));
                            self.switch_to(body_bb);
                        }
                        let mut arm_last = self.emit(InstKind::Void, Type::Void, arm.span);
                        for s in &arm.body {
                            arm_last = self.lower_stmt(s);
                        }
                        if !self.current_block_has_terminator() {
                            if has_result {
                                phi_entries.push((arm_last, self.current_block));
                            }
                            self.set_terminator(Terminator::Goto(merge_bb));
                        }

                        next_test = next_bb;
                    }
                    // If the last arm didn't have a wild/bind, ensure we go to merge.
                    if next_test != merge_bb {
                        self.switch_to(next_test);
                        self.set_terminator(Terminator::Goto(merge_bb));
                    }
                }

                self.switch_to(merge_bb);
                if has_result && !phi_entries.is_empty() {
                    let dest = self.new_value();
                    let incoming: Vec<(BlockId, ValueId)> =
                        phi_entries.iter().map(|(val, blk)| (*blk, *val)).collect();
                    self.func.block_mut(merge_bb).phis.push(Phi {
                        dest,
                        ty: result_ty,
                        incoming,
                    });
                    dest
                } else {
                    self.emit(InstKind::Void, Type::Void, m.span)
                }
            }

            _ => return None,
        })
    }
}
