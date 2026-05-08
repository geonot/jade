use super::super::*;
use super::Lowerer;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::HashSet;

impl Lowerer {
    pub(super) fn lower_expr_concurrency(&mut self, expr: &hir::Expr) -> ValueId {
        let span = expr.span;
        let ty = expr.ty.clone();
        match &expr.kind {
            ExprKind::Spawn(name) => self.emit(InstKind::SpawnActor(*name, vec![]), ty, span),
            ExprKind::Send(target, _type_name, handler, _tag, args) => {
                let mut all = vec![self.lower_expr(target)];
                all.extend(args.iter().map(|a| self.lower_expr(a)));
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__send_{handler}")), all),
                    ty,
                    span,
                )
            }
            ExprKind::ChannelCreate(elem_ty, cap) => {
                let cap_val = self.lower_expr(cap);
                self.emit(
                    InstKind::ChanCreate(elem_ty.clone(), Some(cap_val)),
                    ty,
                    span,
                )
            }
            ExprKind::ChannelSend(chan, val) => {
                let ch = self.lower_expr(chan);
                let v = self.lower_expr(val);
                self.emit(InstKind::ChanSend(ch, v), ty, span)
            }
            ExprKind::ChannelRecv(chan) => {
                let c = self.lower_expr(chan);
                self.emit(InstKind::ChanRecv(c), ty, span)
            }
            ExprKind::Select(arms, default) => {
                // Demote variables assigned in any arm to memory (Store/Load)
                // so the merge point sees correct values.
                let mut assigned = HashSet::new();
                for arm in arms.iter() {
                    Self::collect_assigned_vars(&arm.body, &mut assigned);
                }
                if let Some(def_body) = default {
                    Self::collect_assigned_vars(def_body, &mut assigned);
                }
                let pre_existing: HashSet<Symbol> = assigned
                    .iter()
                    .filter(|n| self.var_map.contains_key(*n))
                    .cloned()
                    .collect();
                self.demote_vars_to_memory(&pre_existing, span);

                // Lower select as a SelectArm with all channel values
                let ch_vals: Vec<ValueId> =
                    arms.iter().map(|arm| self.lower_expr(&arm.chan)).collect();
                let has_default = default.is_some();
                let select_val = self.emit(
                    InstKind::SelectArm(ch_vals.clone(), has_default),
                    ty.clone(),
                    span,
                );
                // Lower bodies as a switch on the selected arm index
                if !arms.is_empty() {
                    let merge_bb = self.new_block("select.merge");
                    let mut cases: Vec<(i64, BlockId)> = Vec::new();
                    for (i, arm) in arms.iter().enumerate() {
                        let arm_bb = self.new_block(&format!("select.arm{i}"));
                        cases.push((i as i64, arm_bb));
                        self.switch_to(arm_bb);
                        if let Some(bind_name) = &arm.binding {
                            // Use __select_recv instead of ChanRecv — jade_select
                            // already received the data into the case data buffer.
                            let idx_val = self.emit(InstKind::IntConst(i as i64), Type::I64, span);
                            let recv_val = self.emit(
                                InstKind::Call(
                                    Symbol::intern("__select_recv"),
                                    vec![select_val, idx_val],
                                ),
                                arm.elem_ty.clone(),
                                span,
                            );
                            self.var_map.insert(bind_name.clone(), recv_val);
                        }
                        self.lower_block_stmts(&arm.body);
                        self.set_terminator(Terminator::Goto(merge_bb));
                    }
                    let default_bb = if let Some(def_body) = default {
                        let db = self.new_block("select.default");
                        self.switch_to(db);
                        self.lower_block_stmts(def_body);
                        self.set_terminator(Terminator::Goto(merge_bb));
                        db
                    } else {
                        merge_bb
                    };
                    // We need to go back and set the switch terminator
                    // The select_val block ended where we emitted SelectArm
                    // Find the block that contains the select inst
                    let select_block = self
                        .func
                        .blocks
                        .iter()
                        .find(|b| b.insts.iter().any(|i| i.dest == Some(select_val)))
                        .map(|b| b.id)
                        .unwrap_or(self.current_block);
                    self.func.block_mut(select_block).terminator =
                        Terminator::Switch(select_val, cases, default_bb);
                    self.switch_to(merge_bb);
                }
                // Return 0 from the merge block, not the jade_select result
                self.emit(InstKind::IntConst(0), Type::I64, span)
            }

            // Atomics — all lowered as intrinsic calls
            ExprKind::CoroutineCreate(name, _body) => {
                // Body is compiled separately — don't inline it here.
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__coro_create_{name}")), vec![]),
                    ty,
                    span,
                )
            }
            ExprKind::CoroutineNext(coro) => {
                let c = self.lower_expr(coro);
                self.emit(InstKind::Call("__coro_next".into(), vec![c]), ty, span)
            }
            ExprKind::Yield(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__yield".into(), vec![v]), ty, span)
            }

            // Dynamic dispatch
            ExprKind::GeneratorCreate(_def_id, name, _body) => {
                // Body is compiled separately — don't inline it here.
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__gen_create_{name}")), vec![]),
                    ty,
                    span,
                )
            }
            ExprKind::GeneratorNext(gen_expr) => {
                let g = self.lower_expr(gen_expr);
                self.emit(InstKind::Call("__gen_next".into(), vec![g]), ty, span)
            }
            _ => unreachable!("expression dispatched to wrong MIR lowering module"),
        }
    }
}
