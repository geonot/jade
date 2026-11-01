use super::super::*;
use super::Lowerer;
use crate::ast::Span;
use crate::hir::{self, DefId};
use crate::intern::Symbol;
use crate::types::Type;

impl Lowerer {
    pub(super) fn lower_cancel_checkpoint(&mut self, span: Span, yield_now: bool) {
        if !self.cancellation_enabled || self.current_block_has_terminator() {
            return;
        }
        if yield_now && !self.func.attrs.no_yield {
            self.emit(
                InstKind::Call("__sched_poll".into(), vec![]),
                Type::Void,
                span,
            );
        }
        let flag = self.emit(
            InstKind::Call("jinn_scope_check_cancelled".into(), vec![]),
            Type::I32,
            span,
        );
        let zero = self.emit(InstKind::IntConst(0), Type::I32, span);
        let cancelled = self.emit(
            InstKind::Cmp(CmpOp::Ne, flag, zero, Type::I32),
            Type::Bool,
            span,
        );
        let unwind = self.new_block("cancel.unwind");
        self.func.cancel_cleanup.get_or_insert(unwind);
        let next = self.new_block("cancel.cont");
        self.set_terminator(Terminator::Branch(cancelled, unwind, next));
        self.switch_to(unwind);
        self.seal_block(unwind);
        self.emit(
            InstKind::Call("__txn_rollback".into(), vec![]),
            Type::Void,
            span,
        );
        for scope in self.owned_scopes.clone().into_iter().rev() {
            self.emit(
                InstKind::Call("__scope_cancel".into(), vec![scope]),
                Type::Void,
                span,
            );
            self.emit(
                InstKind::Call("__scope_join".into(), vec![scope]),
                Type::Void,
                span,
            );
        }
        self.lower_deferred_in_reverse();
        if matches!(self.func.ret_ty, Type::Void) {
            self.set_terminator(Terminator::Return(None));
        } else {
            let value = self.emit(
                InstKind::Call("__cancel_default".into(), vec![]),
                self.func.ret_ty.clone(),
                span,
            );
            self.set_terminator(Terminator::Return(Some(value)));
        }
        self.switch_to(next);
        self.seal_block(next);
    }

    pub(super) fn prepare_cleanup(&mut self, body: &[hir::Stmt]) {
        let mut refs = super::closures::VarRefs::default();
        super::closures::collect_var_refs_block(body, &mut refs);
        self.cleanup_bindings.extend(refs.deferred_vars);
        loop {
            let count = self.cleanup_bindings.len();
            for (id, sources) in &refs.borrow_sources {
                if self.cleanup_bindings.contains(id) {
                    self.cleanup_bindings.extend(sources);
                }
            }
            if self.cleanup_bindings.len() == count {
                break;
            }
        }
        if refs.has_defer {
            self.cleanup_frame(self.func.span);
        }
    }

    pub(super) fn cleanup_frame(&mut self, span: Span) -> ValueId {
        if let Some(frame) = self.cleanup_frame {
            return frame;
        }
        let saved = self.current_block;
        self.current_block = self.func.entry;
        let frame = self.emit(
            InstKind::Call("__cleanup_new".into(), vec![]),
            Type::Ptr(Box::new(Type::Void)),
            span,
        );
        let inst = self.func.block_mut(self.func.entry).insts.pop().unwrap();
        self.func.block_mut(self.func.entry).insts.insert(0, inst);
        self.current_block = saved;
        self.cleanup_frame = Some(frame);
        frame
    }

    pub(super) fn bind_cleanup_cell(
        &mut self,
        id: DefId,
        name: Symbol,
        value: ValueId,
        ty: Type,
        owns: bool,
        span: Span,
    ) -> bool {
        if !self.cleanup_bindings.contains(&id) {
            return false;
        }
        let key = self.alias_binder(id, name);
        if self.cleanup_cells.contains_key(&key) {
            self.write_var(key, self.current_block, value);
            return true;
        }
        let frame = self.cleanup_frame(span);
        let cell_name = Symbol::intern(&format!("__cleanup_cell_{}", id.0));
        let ptr_ty = Type::Ptr(Box::new(ty.clone()));
        let cell = self.emit(
            InstKind::Call(
                if owns {
                    "__cleanup_cell_owned"
                } else {
                    "__cleanup_cell_borrowed"
                }
                .into(),
                vec![frame, value],
            ),
            ptr_ty.clone(),
            span,
        );
        self.var_types.insert(cell_name, ptr_ty);
        self.current_def
            .entry(self.current_block)
            .or_default()
            .insert(cell_name, cell);
        self.var_types.insert(key, ty);
        self.cleanup_cells.insert(key, cell_name);
        true
    }

    pub(super) fn bind_cleanup_borrow(
        &mut self,
        id: DefId,
        name: Symbol,
        value: ValueId,
        ty: Type,
        span: Span,
    ) {
        let key = self.alias_binder(id, name);
        let retained = self.cleanup_bindings.contains(&id);
        let owns = retained && ty.is_value_clonable();
        let value = if owns && !ty.is_trivially_droppable() {
            self.emit(InstKind::Clone(value, ty.clone()), ty.clone(), span)
        } else {
            value
        };
        if !self.bind_cleanup_cell(id, name, value, ty, owns, span) {
            self.write_var(key, self.current_block, value);
        }
    }

    pub(super) fn lower_defer(&mut self, body: &[hir::Stmt], span: Span) -> ValueId {
        let frame = self.cleanup_frame(span);
        let name = Symbol::intern(&format!(
            "__defer_{}_{}",
            self.func.name, self.func.next_value
        ));
        let mut refs = super::closures::VarRefs::default();
        super::closures::collect_var_refs_block(body, &mut refs);
        let mut refs: Vec<_> = refs.vars.into_iter().collect();
        refs.sort_by_key(|(id, _)| id.0);
        let mut sub = Lowerer::new(&name.as_str(), DefId::BUILTIN, span);
        let mut captures = Vec::new();
        if let Some(field_ctx) = &self.field_ctx {
            let state = field_ctx.self_state;
            let state_ty = field_ctx.state_ty.clone();
            let map = field_ctx.map.clone();
            let value = sub.new_value();
            let name = Symbol::intern("__self_state");
            let ty = Type::Ptr(Box::new(state_ty.clone()));
            sub.func.params.push(Param {
                value,
                name,
                ty: ty.clone(),
                by_ref: false,
                owned: false,
            });
            sub.var_types.insert(name, ty);
            sub.current_def
                .entry(sub.func.entry)
                .or_default()
                .insert(name, value);
            sub.field_ctx = Some(super::ctx::FieldCtx {
                self_state: value,
                state_ty,
                map,
            });
            captures.push(state);
        }
        for (id, source_name) in refs {
            let key = self.var_key(id, source_name);
            let Some(ty) = self.var_types.get(&key).cloned() else {
                continue;
            };
            let (value, param_ty) = if let Some(&cell_name) = self.cleanup_cells.get(&key) {
                let ptr_ty = Type::Ptr(Box::new(ty.clone()));
                let cell = self.read_var(cell_name, self.current_block, ptr_ty.clone(), span);
                sub.cleanup_cells.insert(key, cell_name);
                (cell, ptr_ty)
            } else {
                (
                    self.read_var(key, self.current_block, ty.clone(), span),
                    ty.clone(),
                )
            };
            let param_name = sub.cleanup_cells.get(&key).copied().unwrap_or(key);
            let value_id = sub.new_value();
            sub.func.params.push(Param {
                value: value_id,
                name: param_name,
                ty: param_ty.clone(),
                by_ref: false,
                owned: false,
            });
            sub.var_types.insert(param_name, param_ty);
            sub.var_types.insert(key, ty);
            sub.binder_alias.insert(id, key);
            sub.current_def
                .entry(sub.func.entry)
                .or_default()
                .insert(param_name, value_id);
            captures.push(value);
        }
        super::finish_body(&mut sub, body, &Type::Void, span, false);
        self.lambda_fns.push(sub.func);
        self.lambda_fns.append(&mut sub.lambda_fns);
        let closure_ty = Type::Fn(vec![], Box::new(Type::Void));
        let closure = self.emit(InstKind::ClosureCreate(name, captures), closure_ty, span);
        self.emit(
            InstKind::Call("__cleanup_push".into(), vec![frame, closure]),
            Type::Void,
            span,
        )
    }

    pub(super) fn lower_deferred_in_reverse(&mut self) {
        if let Some(frame) = self.cleanup_frame {
            self.emit(
                InstKind::Call("__cleanup_run".into(), vec![frame]),
                Type::Void,
                self.func.span,
            );
        }
    }
}
