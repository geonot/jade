use super::super::*;
use super::Lowerer;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;

impl Lowerer {
    pub(super) fn lower_expr_concurrency(&mut self, expr: &hir::Expr) -> ValueId {
        let span = expr.span;
        let ty = expr.ty.clone();
        match &expr.kind {
            ExprKind::Spawn(name, inits) => {
                let lowered: Vec<(Symbol, ValueId)> = inits
                    .iter()
                    .map(|(fname, e)| (*fname, self.lower_expr_owned(e)))
                    .collect();
                self.emit(InstKind::SpawnActor(*name, lowered), ty, span)
            }
            ExprKind::Send(target, type_name, handler, _tag, args) => {
                let mut all = vec![self.lower_expr(target)];
                all.extend(args.iter().map(|a| self.lower_expr_owned(a)));
                self.emit(
                    InstKind::Call(
                        Symbol::intern(&format!("__send_{type_name}.{handler}")),
                        all,
                    ),
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
                let v = self.lower_expr_owned(val);
                self.emit(InstKind::ChanSend(ch, v), ty, span)
            }
            ExprKind::ChannelRecv(chan) => {
                let c = self.lower_expr(chan);
                self.emit(InstKind::ChanRecv(c), ty, span)
            }
            ExprKind::Select(arms, default) => {
                // Pure Braun SSA: each arm/default block is lowered then
                // sealed once its predecessor edge from the Switch exists;
                // reads of outer vars build incomplete phis collapsed at
                // seal time, and post-merge reads phi lazily at merge_bb.
                let ch_vals: Vec<ValueId> =
                    arms.iter().map(|arm| self.lower_expr(&arm.chan)).collect();
                let has_default = default.is_some();
                let select_val = self.emit(
                    InstKind::SelectArm(ch_vals.clone(), has_default),
                    ty.clone(),
                    span,
                );

                if !arms.is_empty() {
                    let merge_bb = self.new_block("select.merge");
                    let mut cases: Vec<(i64, BlockId)> = Vec::new();
                    let mut arm_bbs: Vec<BlockId> = Vec::new();
                    for (i, arm) in arms.iter().enumerate() {
                        let arm_bb = self.new_block(&format!("select.arm{i}"));
                        cases.push((i as i64, arm_bb));
                        arm_bbs.push(arm_bb);
                        self.switch_to(arm_bb);
                        if let Some(bind_name) = &arm.binding {
                            let idx_val = self.emit(InstKind::IntConst(i as i64), Type::I64, span);
                            let recv_val = self.emit(
                                InstKind::Call(
                                    Symbol::intern("__select_recv"),
                                    vec![select_val, idx_val],
                                ),
                                arm.elem_ty.clone(),
                                span,
                            );
                            self.write_var(bind_name.clone(), self.current_block, recv_val);
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

                    let select_block = self
                        .func
                        .blocks
                        .iter()
                        .find(|b| b.insts.iter().any(|i| i.dest == Some(select_val)))
                        .map(|b| b.id)
                        .unwrap_or(self.current_block);
                    // Route through set_terminator (on select_block) so
                    // predecessor edges into each arm_bb / default_bb are
                    // tracked for Braun phi construction at merge_bb.
                    let saved_block = self.current_block;
                    self.switch_to(select_block);
                    self.set_terminator(Terminator::Switch(select_val, cases, default_bb));
                    self.switch_to(saved_block);
                    // The Switch terminator on `select_block` now records the
                    // predecessor edges into each arm / default block. Seal them
                    // so the incomplete phis created while lowering their bodies
                    // (reads of outer vars like `total`/`i`/`ch`) are filled and
                    // collapsed. They could not be sealed earlier because their
                    // sole predecessor edge did not yet exist.
                    for &arm_bb in &arm_bbs {
                        self.seal_block(arm_bb);
                    }
                    if has_default {
                        self.seal_block(default_bb);
                    }
                    self.switch_to(merge_bb);
                    self.seal_block(merge_bb);
                }

                self.emit(InstKind::IntConst(0), Type::I64, span)
            }

            ExprKind::CoroutineCreate(name, body) => {
                // Lower the coroutine body as a standalone MIR function
                // `__coro_{name}` (codegen gives it the `void(ptr)` coroutine
                // ABI). Dispatch coroutines never capture, so the param list
                // is empty.
                self.lower_coroutine(*name, body, &[], span);
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

            ExprKind::GeneratorCreate(def_id, name, body, captures) => {
                // Resolve the capture values in the *enclosing* function before
                // lowering the body in a fresh sub-lowerer; these are passed to
                // `__gen_create_{name}` and stored into the generator struct,
                // from which the body's prologue reloads them.
                let mut arg_ids: Vec<ValueId> = Vec::with_capacity(captures.len());
                for (cap_name, cap_ty) in captures {
                    arg_ids.push(self.read_var(
                        *cap_name,
                        self.current_block,
                        cap_ty.clone(),
                        span,
                    ));
                }
                self.lower_coroutine_with_def(*name, *def_id, body, captures, span);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__gen_create_{name}")), arg_ids),
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

    /// Lower a coroutine/generator body into a standalone MIR `Function` named
    /// `__coro_{name}` and append it to `self.lambda_fns`. Captures (empty for
    /// dispatch coroutines) become the function's parameters, in order; codegen
    /// reloads them from the generator struct in the prologue. The body is
    /// lowered like any ordinary `Void`-returning function — `yield` expressions
    /// become `__yield` calls, and fall-through synthesizes a `Return(None)`
    /// terminator that codegen turns into "mark done + suspend".
    pub(super) fn lower_coroutine(
        &mut self,
        name: Symbol,
        body: &[hir::Stmt],
        captures: &[(Symbol, Type)],
        span: crate::ast::Span,
    ) {
        self.lower_coroutine_with_def(name, crate::hir::DefId::BUILTIN, body, captures, span);
    }

    fn lower_coroutine_with_def(
        &mut self,
        name: Symbol,
        def_id: crate::hir::DefId,
        body: &[hir::Stmt],
        captures: &[(Symbol, Type)],
        span: crate::ast::Span,
    ) {
        let coro_fn_name = format!("__coro_{name}");
        let mut sub = Lowerer::new(&coro_fn_name, def_id, span);
        sub.func.ret_ty = Type::Void;
        sub.func.is_coroutine = true;

        let entry = sub.func.entry;
        for (cap_name, cap_ty) in captures {
            let val = sub.new_value();
            sub.func.params.push(Param {
                value: val,
                name: *cap_name,
                ty: cap_ty.clone(),
                ownership: hir::Ownership::Owned,
            });
            sub.var_types.insert(*cap_name, cap_ty.clone());
            sub.current_def
                .entry(entry)
                .or_default()
                .insert(*cap_name, val);
        }

        super::finish_body(&mut sub, body, &Type::Void, span, false);

        self.lambda_fns.push(sub.func);
        self.lambda_fns.append(&mut sub.lambda_fns);
    }
}
