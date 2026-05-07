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
    pub(super) fn lower_stmt_p0(&mut self, stmt: &hir::Stmt) -> Option<ValueId> {
        Some(match stmt {
            hir::Stmt::Bind(b) => {
                let val = self.lower_expr(&b.value);
                // Store the DefId on the instruction that produced this value,
                // so MIR Perceus can track binding → value relationships.
                if let Some(inst) = self
                    .func
                    .block_mut(self.current_block)
                    .insts
                    .iter_mut()
                    .rev()
                    .find(|i| i.dest == Some(val))
                {
                    inst.def_id = Some(b.def_id);
                }
                if self.mem_vars.contains(&b.name) {
                    // Variable is memory-backed (reassigned in a loop/branch).
                    // Emit Store with the variable's type so codegen allocas are correct.
                    self.func
                        .block_mut(self.current_block)
                        .insts
                        .push(Instruction {
                            dest: None,
                            kind: InstKind::Store(b.name.clone(), val),
                            ty: b.ty.clone(),
                            span: b.span,
                            def_id: None,
                        });
                } else {
                    self.var_map.insert(b.name.clone(), val);
                }
                val
            }

            hir::Stmt::Assign(target, value, _span) => {
                let val = self.lower_expr(value);
                match &target.kind {
                    ExprKind::Var(_, name) => {
                        if self.mem_vars.contains(name) {
                            // Use the value's type from the expression.
                            self.func
                                .block_mut(self.current_block)
                                .insts
                                .push(Instruction {
                                    dest: None,
                                    kind: InstKind::Store(name.clone(), val),
                                    ty: value.ty.clone(),
                                    span: target.span,
                                    def_id: None,
                                });
                        } else {
                            self.var_map.insert(name.clone(), val);
                        }
                    }
                    ExprKind::Field(obj, field, _) => {
                        // If the object is a mem_var, emit a direct field store
                        // on the variable name so codegen can GEP into the alloca.
                        if let ExprKind::Var(_, name) = &obj.kind {
                            if self.mem_vars.contains(name) {
                                let obj_ty = obj.ty.clone();
                                self.func
                                    .block_mut(self.current_block)
                                    .insts
                                    .push(Instruction {
                                        dest: None,
                                        kind: InstKind::FieldStore(
                                            *name,
                                            *field,
                                            val,
                                        ),
                                        ty: obj_ty,
                                        span: target.span,
                                        def_id: None,
                                    });
                                return Some(val);
                            }
                        }
                        // SSA field set: produce updated struct and propagate
                        // back up through nested field chains to the root variable.
                        self.lower_field_assign(obj, &field.as_str(), val, target.span);
                    }
                    ExprKind::Index(arr, idx) => {
                        // If the array is a mem_var, emit a direct index store
                        // on the variable name so codegen can GEP into the alloca.
                        if let ExprKind::Var(_, name) = &arr.kind {
                            if self.mem_vars.contains(name) {
                                let i = self.lower_expr(idx);
                                let arr_ty = arr.ty.clone();
                                self.func
                                    .block_mut(self.current_block)
                                    .insts
                                    .push(Instruction {
                                        dest: None,
                                        kind: InstKind::IndexStore(name.clone(), i, val),
                                        ty: arr_ty,
                                        span: target.span,
                                        def_id: None,
                                    });
                                return Some(val);
                            }
                            // Non-mem_var array: emit IndexSet and store updated value back.
                            let a = self.lower_expr(arr);
                            let i = self.lower_expr(idx);
                            let arr_ty = arr.ty.clone();
                            let updated =
                                self.emit(InstKind::IndexSet(a, i, val), arr_ty, target.span);
                            self.var_map.insert(name.clone(), updated);
                            return Some(val);
                        }
                        let a = self.lower_expr(arr);
                        let i = self.lower_expr(idx);
                        self.emit_void(InstKind::IndexSet(a, i, val), target.span);
                    }
                    _ => {}
                }
                val
            }

            hir::Stmt::Expr(e) => self.lower_expr(e),

            hir::Stmt::If(if_stmt) => {
                // Demote variables assigned in any branch to memory
                // so the merge point gets the correct value via Load.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&if_stmt.then, &mut assigned);
                for (_, elif_body) in &if_stmt.elifs {
                    Self::collect_assigned_vars(elif_body, &mut assigned);
                }
                if let Some(els) = &if_stmt.els {
                    Self::collect_assigned_vars(els, &mut assigned);
                }
                // Only demote vars that already exist in var_map (were defined before if).
                let pre_existing: HashSet<Symbol> = assigned
                    .iter()
                    .filter(|n| self.var_map.contains_key(*n))
                    .cloned()
                    .collect();
                self.demote_vars_to_memory(&pre_existing, if_stmt.span);

                // Variables first defined (via Bind) in BOTH then and else branches
                // must also be promoted to mem_vars so they're accessible after merge.
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
                    // Variables defined in then AND (else or elif) → promote to mem_vars
                    for name in then_binds.intersection(&other_binds) {
                        if !self.var_map.contains_key(name) && !self.mem_vars.contains(name) {
                            self.mem_vars.insert(name.clone());
                        }
                    }
                }

                let cond = self.lower_expr(&if_stmt.cond);
                let then_bb = self.new_block("if.then");
                let merge_bb = self.new_block("if.merge");

                // Save var_map keys before entering branches so we can clean up
                // variables only defined in the then-branch (not reachable at merge
                // when there's no else).
                let vars_before_if: HashSet<Symbol> = self.var_map.keys().cloned().collect();

                // Determine the false-branch target:
                // elif chain first, then else, then merge.
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

                // Lower elif chains.
                let mut elif_vals: Vec<(BlockId, ValueId)> = Vec::new();
                let mut prev_false_bb = first_elif_bb;
                for (i, (elif_cond, elif_body)) in if_stmt.elifs.iter().enumerate() {
                    let elif_test = prev_false_bb.unwrap();
                    let elif_body_bb = self.new_block("elif.body");

                    // Determine where a false elif branches to.
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
                    // The else block target was the last false branch.
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

                // Remove variables from var_map that were newly defined only
                // inside the then-branch (or elif bodies) but not on all paths.
                // At the merge point these values don't dominate, so they must
                // not remain in var_map; the next definition will use Store/Load.
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

                // If all branches produce non-void values, insert a phi at merge.
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

            hir::Stmt::While(w) => {
                // Demote variables assigned inside loop body to memory
                // so each iteration re-reads the current value via Load.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&w.body, &mut assigned);
                // Also check condition for assigned vars (unlikely but safe)
                self.demote_vars_to_memory(&assigned, w.span);

                // Snapshot var_map keys before the loop body so we can remove
                // variables first defined inside the loop — their SSA values
                // don't dominate the exit block.
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

                // Remove var_map entries for variables first defined inside the
                // loop body.  Their values were produced in the body block(s)
                // and do NOT dominate the exit block (which is reachable from
                // the condition block when the loop is never entered).
                self.var_map.retain(|k, _| pre_loop_vars.contains(k));

                self.switch_to(exit_bb);
                self.emit(InstKind::Void, Type::Void, w.span)
            }

            hir::Stmt::For(f) => {
                // Demote variables assigned inside for loop body to memory
                // so each iteration re-reads the current value via Load.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&f.body, &mut assigned);
                self.demote_vars_to_memory(&assigned, f.span);

                // Snapshot var_map keys before the loop body so we can remove
                // variables first defined inside the loop — their SSA values
                // don't dominate the exit block.
                let pre_loop_vars: HashSet<Symbol> = self.var_map.keys().cloned().collect();

                // Range-based for: `for i in start..end`
                // If `end` is present, this is a range for; otherwise iterate
                // the collection via index.
                let iter_val = self.lower_expr(&f.iter);
                let cond_bb = self.new_block("for.cond");
                let body_bb = self.new_block("for.body");
                let inc_bb = self.new_block("for.inc");
                let exit_bb = self.new_block("for.exit");

                if let Some(ref end_expr) = f.end {
                    // Range for: iter_val = start, end = end_expr
                    let end_val = self.lower_expr(end_expr);
                    let step_val = if let Some(ref step_expr) = f.step {
                        self.lower_expr(step_expr)
                    } else {
                        self.emit(InstKind::IntConst(1), Type::I64, f.span)
                    };

                    // Store counter as a variable.
                    self.emit_void_typed(
                        InstKind::Store(f.bind.clone(), iter_val),
                        f.bind_ty.clone(),
                        f.span,
                    );
                    self.var_map.insert(f.bind.clone(), iter_val);
                    // Initialize bind2 index to 0 for range-for.
                    if let Some(ref b2) = f.bind2 {
                        let zero = self.emit(InstKind::IntConst(0), Type::I64, f.span);
                        self.emit_void_typed(
                            InstKind::Store(b2.clone(), zero),
                            Type::I64,
                            f.span,
                        );
                    }
                    self.set_terminator(Terminator::Goto(cond_bb));

                    // Condition: load counter, compare < end.
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
                    // Re-bind the loop variable to the loaded counter so body can use it.
                    self.var_map.insert(f.bind.clone(), counter);
                    // If bind2 is present, expose a 0-based index for range-for.
                    if let Some(ref b2) = f.bind2 {
                        let idx = self.emit(InstKind::Load(b2.clone()), Type::I64, f.span);
                        self.var_map.insert(b2.clone(), idx);
                    }
                    self.lower_block_stmts(&f.body);
                    if !self.current_block_has_terminator() {
                        self.set_terminator(Terminator::Goto(inc_bb));
                    }
                    self.loop_stack.pop();

                    // Increment
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
                    // Increment bind2 index.
                    if let Some(ref b2) = f.bind2 {
                        let one = self.emit(InstKind::IntConst(1), Type::I64, f.span);
                        let cur_idx = self.emit(InstKind::Load(b2.clone()), Type::I64, f.span);
                        let next_idx = self.emit(
                            InstKind::BinOp(BinOp::Add, cur_idx, one),
                            Type::I64,
                            f.span,
                        );
                        self.emit_void_typed(
                            InstKind::Store(b2.clone(), next_idx),
                            Type::I64,
                            f.span,
                        );
                    }
                    self.set_terminator(Terminator::Goto(cond_bb));
                } else if matches!(f.iter.ty, Type::I64 | Type::I32 | Type::F64) {
                    // Range for with implicit start=0: `for i in N` means 0..N
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
                    // Generator/coroutine for: resume loop.
                    // cond: resume gen; done = __gen_done(gen); branch !done ? body : exit
                    // body: val = __gen_next_val(gen); bind = val; ... body ...; goto cond
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    // Resume the generator
                    let _resume = self.emit(
                        InstKind::Call("__gen_resume".into(), vec![iter_val]),
                        Type::Void,
                        f.span,
                    );
                    // Check done flag
                    let done = self.emit(
                        InstKind::Call("__gen_done".into(), vec![iter_val]),
                        Type::Bool,
                        f.span,
                    );
                    // Branch: if done, exit; else body
                    self.set_terminator(Terminator::Branch(done, exit_bb, body_bb));

                    // Body: read yielded value and bind it
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
                    // Collection for: iterate with index.
                    let zero = self.emit(InstKind::IntConst(0), Type::I64, f.span);
                    let one = self.emit(InstKind::IntConst(1), Type::I64, f.span);
                    let idx_name = Symbol::intern(&format!("__for_idx_{}", f.bind));
                    self.emit_void_typed(
                        InstKind::Store(idx_name, zero),
                        Type::I64,
                        f.span,
                    );
                    self.set_terminator(Terminator::Goto(cond_bb));

                    // Condition: idx < len (re-read length each iteration so
                    // mutations to the collection between outer iterations are
                    // visible).
                    self.switch_to(cond_bb);
                    let len = self.emit(InstKind::VecLen(iter_val), Type::I64, f.span);
                    let idx = self.emit(InstKind::Load(idx_name), Type::I64, f.span);
                    let cmp = self.emit(
                        InstKind::Cmp(CmpOp::Lt, idx, len, Type::I64),
                        Type::Bool,
                        f.span,
                    );
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    // Body: bind element.
                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);
                    let elem = self.emit(
                        InstKind::IndexUnchecked(iter_val, idx),
                        f.bind_ty.clone(),
                        f.span,
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

                    // Increment index.
                    self.switch_to(inc_bb);
                    let cur_idx = self.emit(InstKind::Load(idx_name), Type::I64, f.span);
                    let next_idx =
                        self.emit(InstKind::BinOp(BinOp::Add, cur_idx, one), Type::I64, f.span);
                    self.emit_void_typed(InstKind::Store(idx_name, next_idx), Type::I64, f.span);
                    self.set_terminator(Terminator::Goto(cond_bb));
                }

                // Remove var_map entries for variables first defined inside the
                // loop body — they don't dominate the exit block.
                self.var_map.retain(|k, _| pre_loop_vars.contains(k));

                self.switch_to(exit_bb);
                // The loop bound variable's SSA value is from the condition
                // block and is not valid here.  Mark it as memory-resident so
                // subsequent demote_vars_to_memory won't try to store a stale
                // ValueId from a non-dominating block.
                self.var_map.remove(&f.bind);
                self.mem_vars.insert(f.bind.clone());
                self.emit(InstKind::Void, Type::Void, f.span)
            }

            _ => return None,
        })
    }
}

impl Lowerer {
    pub(super) fn lower_block_stmts(&mut self, stmts: &[hir::Stmt]) {
        for stmt in stmts {
            self.lower_stmt(stmt);
        }
    }

}
