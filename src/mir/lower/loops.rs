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
                        self.emit_void_typed(InstKind::Store(b2.clone(), zero), Type::I64, f.span);
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
                    self.emit_void_typed(InstKind::Store(idx_name, zero), Type::I64, f.span);
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
            _ => unreachable!("statement dispatched to wrong MIR lowering module"),
        }
    }
}
