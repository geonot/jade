use super::super::*;
use super::Lowerer;
use crate::hir::{self, Pat};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::HashSet;

impl Lowerer {
    pub(super) fn lower_stmt_control(&mut self, stmt: &hir::Stmt) -> ValueId {
        match stmt {
            hir::Stmt::If(if_stmt) => {
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&if_stmt.then, &mut assigned);
                for (_, elif_body) in &if_stmt.elifs {
                    Self::collect_assigned_vars(elif_body, &mut assigned);
                }
                if let Some(els) = &if_stmt.els {
                    Self::collect_assigned_vars(els, &mut assigned);
                }

                let pre_existing: HashSet<Symbol> = assigned
                    .iter()
                    .filter(|n| self.var_map.contains_key(*n))
                    .cloned()
                    .collect();
                self.demote_vars_to_memory(&pre_existing, if_stmt.span);

                if if_stmt.els.is_some() || !if_stmt.elifs.is_empty() {
                    let mut then_binds = HashSet::new();
                    Self::collect_new_binds(&if_stmt.then, &mut then_binds);
                    let mut other_binds = HashSet::new();
                    for (_, elif_body) in &if_stmt.elifs {
                        Self::collect_new_binds(elif_body, &mut other_binds);
                    }
                    if let Some(els) = &if_stmt.els {
                        Self::collect_new_binds(els, &mut other_binds);
                    }

                    for name in then_binds.intersection(&other_binds) {
                        if !self.var_map.contains_key(name) && !self.mem_vars.contains(name) {
                            self.mem_vars.insert(name.clone());
                        }
                    }
                }

                let cond = self.lower_expr(&if_stmt.cond);
                let then_bb = self.new_block("if.then");
                let merge_bb = self.new_block("if.merge");

                let vars_before_if: HashSet<Symbol> = self.var_map.keys().cloned().collect();

                let first_elif_bb = if !if_stmt.elifs.is_empty() {
                    Some(self.new_block("elif.test"))
                } else {
                    None
                };
                let else_bb = if if_stmt.els.is_some() && first_elif_bb.is_none() {
                    self.new_block("if.else")
                } else {
                    first_elif_bb.unwrap_or(merge_bb)
                };

                self.set_terminator(Terminator::Branch(cond, then_bb, else_bb));

                self.switch_to(then_bb);
                let then_val = self.lower_block_expr(&if_stmt.then);
                let then_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                let mut elif_vals: Vec<(BlockId, ValueId)> = Vec::new();
                let mut prev_false_bb = first_elif_bb;
                for (i, (elif_cond, elif_body)) in if_stmt.elifs.iter().enumerate() {
                    let elif_test = prev_false_bb.unwrap();
                    let elif_body_bb = self.new_block("elif.body");

                    let is_last_elif = i + 1 == if_stmt.elifs.len();
                    let elif_false_bb = if is_last_elif {
                        if if_stmt.els.is_some() {
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

                let else_val_info = if let Some(els) = &if_stmt.els {
                    let else_target = prev_false_bb.unwrap_or(else_bb);
                    self.switch_to(else_target);
                    let else_val = self.lower_block_expr(els);
                    let else_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    Some((else_end, else_val))
                } else {
                    None
                };

                self.switch_to(merge_bb);

                if else_val_info.is_none() {
                    let new_vars: Vec<Symbol> = self
                        .var_map
                        .keys()
                        .filter(|k| !vars_before_if.contains(*k))
                        .cloned()
                        .collect();
                    for v in new_vars {
                        self.var_map.remove(&v);
                    }
                }

                let then_ty = self.value_type(then_val);
                if !matches!(then_ty, Type::Void) && else_val_info.is_some() {
                    let mut incoming = vec![(then_end, then_val)];
                    for &(bb, v) in &elif_vals {
                        incoming.push((bb, v));
                    }
                    if let Some((eb, ev)) = else_val_info {
                        incoming.push((eb, ev));
                    }
                    let result = self.new_value();
                    self.func.block_mut(merge_bb).phis.push(Phi {
                        dest: result,
                        ty: then_ty,
                        incoming,
                    });
                    result
                } else {
                    self.emit(InstKind::Void, Type::Void, if_stmt.span)
                }
            }
            hir::Stmt::Match(m) => {
                let mut assigned = HashSet::new();
                for arm in &m.arms {
                    Self::collect_assigned_vars(&arm.body, &mut assigned);
                }

                let pre_existing: HashSet<Symbol> = assigned
                    .into_iter()
                    .filter(|v| self.var_map.contains_key(v))
                    .collect();
                self.demote_vars_to_memory(&pre_existing, m.span);

                let subj = self.lower_expr(&m.subject);
                let merge_bb = self.new_block("match.merge");

                if m.arms.is_empty() {
                    self.switch_to(merge_bb);
                    return self.emit(InstKind::Void, Type::Void, m.span);
                }

                let is_enum = matches!(m.subject.ty, Type::Enum(_));
                let has_ctor = m.arms.iter().any(|a| matches!(a.pat, Pat::Ctor(..)));
                let all_lit = m
                    .arms
                    .iter()
                    .all(|a| matches!(a.pat, Pat::Lit(_) | Pat::Wild(_)));
                let result_ty = m.ty.clone();
                let has_result = !matches!(result_ty, Type::Void);

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

                let has_guard = m.arms.iter().any(|a| a.guard.is_some());

                let mut phi_entries: Vec<(ValueId, BlockId)> = Vec::new();

                if !has_dup_tags && !has_guard && (is_enum || has_ctor || all_lit) {
                    let disc = if is_enum || has_ctor {
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
                                let lit_val = self.lower_expr(lit_expr);

                                if let Some(ival) = self.try_extract_int_const(lit_val) {
                                    cases.push((ival, arm_bb));
                                } else {
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

                    if !has_explicit_default {
                        self.switch_to(unreach_bb);
                    }

                    for (arm_bb, arm) in arm_blocks {
                        self.switch_to(arm_bb);

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

                                let needs_sub_check =
                                    sub_pats.iter().any(|sp| matches!(sp, Pat::Ctor(..)));
                                if needs_sub_check {
                                    let sub_check_bb = self.new_block("match.subcheck");
                                    self.set_terminator(Terminator::Branch(
                                        tag_cmp,
                                        sub_check_bb,
                                        next_bb,
                                    ));
                                    self.switch_to(sub_check_bb);

                                    let mut all_match = tag_cmp;
                                    for (idx, sp) in sub_pats.iter().enumerate() {
                                        if let Pat::Ctor(_, inner_tag, _, _) = sp {
                                            let field_val = self.emit(
                                                InstKind::FieldGet(
                                                    subj,
                                                    Symbol::intern(&format!("_{idx}")),
                                                ),
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
                                self.set_terminator(Terminator::Goto(arm_bb));
                            }
                        }

                        self.switch_to(arm_bb);

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
            hir::Stmt::Ret(val, _ret_ty, span) => {
                if let Some(v) = val {
                    let rv = self.lower_expr_owned(v);
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
            hir::Stmt::ErrReturn(expr, _ty, span) => {
                let v = self.lower_expr_owned(expr);
                self.lower_deferred_in_reverse();
                self.set_terminator(Terminator::Return(Some(v)));
                let dead = self.new_block("after.err_return");
                self.switch_to(dead);
                self.emit(InstKind::Void, Type::Void, *span)
            }
            hir::Stmt::Break(val, span) => {
                let mut handled_label = false;
                if let Some(v) = val {
                    if let hir::ExprKind::Str(s) = &v.kind {
                        if let Some(name) = s.strip_prefix("__break_label__") {
                            let want = crate::intern::Symbol::intern(name);
                            if let Some((_, _, exit)) = self
                                .label_stack
                                .iter()
                                .rev()
                                .find(|(l, _, _)| *l == want)
                                .copied()
                            {
                                self.set_terminator(Terminator::Goto(exit));
                                handled_label = true;
                            }
                        } else if let Some(name) = s.strip_prefix("__continue_label__") {
                            let want = crate::intern::Symbol::intern(name);
                            if let Some((_, cont, _)) = self
                                .label_stack
                                .iter()
                                .rev()
                                .find(|(l, _, _)| *l == want)
                                .copied()
                            {
                                self.set_terminator(Terminator::Goto(cont));
                                handled_label = true;
                            }
                        }
                    }
                }
                if !handled_label {
                    if let Some((_, exit)) = self.loop_stack.last().copied() {
                        if let Some(v) = val {
                            let _ = self.lower_expr(v);
                        }
                        self.set_terminator(Terminator::Goto(exit));
                    }
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
            _ => unreachable!("statement dispatched to wrong MIR lowering module"),
        }
    }
}
