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
                            self.write_var(*bind_name, self.current_block, recv_val);
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

                    let saved_block = self.current_block;
                    self.switch_to(select_block);
                    self.set_terminator(Terminator::Switch(select_val, cases, default_bb));
                    self.switch_to(saved_block);

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
                // A bare anonymous `dispatch` inside a `together` scope is a
                // structured concurrent task: spawn it on the scheduler and
                // register it as a child of the innermost scope. A named
                // `dispatch` remains a lazy generator driven by `.next()`.
                if name.as_str().starts_with("__anon")
                    && let Some(&scope_val) = self.scope_stack.last()
                {
                    // Capture enclosing locals referenced by the task body, by
                    // value, into the task's coroutine struct (same ABI as a
                    // generator's captures). Free vars that are not enclosing
                    // locals (globals, functions) are left to normal lookup.
                    let mut refs = std::collections::HashSet::new();
                    super::closures::collect_var_refs_block(body, &mut refs);
                    let mut captures: Vec<(Symbol, Type)> = refs
                        .into_iter()
                        .filter_map(|n| self.var_types.get(&n).map(|t| (n, t.clone())))
                        .collect();
                    captures.sort_by_key(|(n, _)| *n);

                    let cap_vals: Vec<ValueId> = captures
                        .iter()
                        .map(|(n, t)| self.read_var(*n, self.current_block, t.clone(), span))
                        .collect();

                    self.lower_scope_task(*name, body, &captures, span);

                    let mut args = vec![scope_val];
                    args.extend(cap_vals);
                    return self.emit(
                        InstKind::Call(Symbol::intern(&format!("__scope_spawn_{name}")), args),
                        ty,
                        span,
                    );
                }
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

    pub(super) fn lower_together(
        &mut self,
        name: Option<Symbol>,
        body: &[hir::Stmt],
        errs: &[Symbol],
        handler: Option<&hir::TogetherHandler>,
        span: crate::ast::Span,
    ) -> ValueId {
        let scope = self.emit(
            InstKind::Call(Symbol::intern("__scope_create"), vec![]),
            Type::Ptr(Box::new(Type::Void)),
            span,
        );
        self.scope_stack.push(scope);
        if let Some(n) = name {
            self.scope_named.push((n, scope));
        }
        self.lower_block_stmts(body);
        if name.is_some() {
            self.scope_named.pop();
        }
        self.scope_stack.pop();
        self.emit(
            InstKind::Call(Symbol::intern("__scope_stop_actors"), vec![scope]),
            Type::Void,
            span,
        );

        if errs.is_empty() && handler.is_none() {
            return self.emit(
                InstKind::Call(Symbol::intern("__scope_join"), vec![scope]),
                Type::Void,
                span,
            );
        }

        let result_ty = self.func.ret_ty.clone();
        let err_enum = errs.first().copied().unwrap_or_else(|| Symbol::intern("Void"));
        let got = self.emit(
            InstKind::Call(Symbol::intern("__scope_join_take_error"), vec![scope]),
            Type::I64,
            span,
        );
        let sentinel = self.emit(InstKind::IntConst(i64::MIN), Type::I64, span);
        let has_err = self.emit(
            InstKind::Cmp(crate::mir::CmpOp::Ne, got, sentinel, Type::I64),
            Type::Bool,
            span,
        );
        let prop_bb = self.new_block("scope.err.prop");
        let ok_bb = self.new_block("scope.err.ok");
        let done_bb = self.new_block("scope.err.done");
        self.set_terminator(Terminator::Branch(has_err, prop_bb, ok_bb));

        self.seal_block(prop_bb);
        self.switch_to(prop_bb);
        let err_result = self.emit(
            InstKind::Call(
                Symbol::intern(&format!("__scope_build_err_{err_enum}")),
                vec![got],
            ),
            result_ty.clone(),
            span,
        );

        match handler {
            Some(h) if h.err_arm.is_some() => {
                if let Some(err_arm) = &h.err_arm {
                    let err_inner = self.emit(
                        InstKind::FieldGet(err_result, Symbol::intern("_0")),
                        h.err_ty.clone(),
                        span,
                    );
                    self.write_var(Symbol::intern("err"), self.current_block, err_inner);
                    let _ = h.err_bind;
                    self.lower_block_stmts(err_arm);
                }
                self.set_terminator(Terminator::Goto(done_bb));
            }
            _ => {
                self.lower_deferred_in_reverse();
                self.set_terminator(Terminator::Return(Some(err_result)));
                let dead = self.new_block("scope.err.dead");
                self.switch_to(dead);
                self.mark_dead_block(dead);
            }
        }

        self.seal_block(ok_bb);
        self.switch_to(ok_bb);
        if let Some(h) = handler
            && let Some(ok_arm) = &h.ok_arm
        {
            self.lower_block_stmts(ok_arm);
        }
        self.set_terminator(Terminator::Goto(done_bb));

        self.seal_block(done_bb);
        self.switch_to(done_bb);
        self.emit(InstKind::Void, Type::Void, span)
    }

    pub(super) fn lower_coroutine(
        &mut self,
        name: Symbol,
        body: &[hir::Stmt],
        captures: &[(Symbol, Type)],
        span: crate::ast::Span,
    ) {
        self.lower_coroutine_inner(name, crate::hir::DefId::BUILTIN, body, captures, span, false);
    }

    pub(super) fn lower_scope_task(
        &mut self,
        name: Symbol,
        body: &[hir::Stmt],
        captures: &[(Symbol, Type)],
        span: crate::ast::Span,
    ) {
        self.lower_coroutine_inner(name, crate::hir::DefId::BUILTIN, body, captures, span, true);
    }

    fn lower_coroutine_with_def(
        &mut self,
        name: Symbol,
        def_id: crate::hir::DefId,
        body: &[hir::Stmt],
        captures: &[(Symbol, Type)],
        span: crate::ast::Span,
    ) {
        self.lower_coroutine_inner(name, def_id, body, captures, span, false);
    }

    fn lower_coroutine_inner(
        &mut self,
        name: Symbol,
        def_id: crate::hir::DefId,
        body: &[hir::Stmt],
        captures: &[(Symbol, Type)],
        span: crate::ast::Span,
        scheduler_task: bool,
    ) {
        let coro_fn_name = format!("__coro_{name}");
        let mut sub = Lowerer::new(&coro_fn_name, def_id, span);
        sub.func.ret_ty = Type::Void;
        sub.func.is_coroutine = true;
        sub.func.scheduler_task = scheduler_task;

        let entry = sub.func.entry;
        for (cap_name, cap_ty) in captures {
            let val = sub.new_value();
            sub.func.params.push(Param {
                value: val,
                name: *cap_name,
                ty: cap_ty.clone(),
            });
            sub.var_types.insert(*cap_name, cap_ty.clone());
            sub.current_def
                .entry(entry)
                .or_default()
                .insert(*cap_name, val);
        }

        super::finish_body(&mut sub, body, &Type::Void, span, false);

        if scheduler_task && !sub.function_defers.is_empty() {
            let cleanup = sub.new_block("cancel.cleanup");
            sub.seal_block(cleanup);
            let saved = sub.current_block;
            sub.switch_to(cleanup);
            sub.lower_deferred_in_reverse();
            sub.set_terminator(Terminator::Return(None));
            sub.switch_to(saved);
            sub.func.cancel_cleanup = Some(cleanup);
        }

        self.lambda_fns.push(sub.func);
        self.lambda_fns.append(&mut sub.lambda_fns);
    }
}
