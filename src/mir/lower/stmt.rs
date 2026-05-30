use super::super::*;
use super::Lowerer;
use crate::ast::{AccessMod, Span};
use crate::hir::{self, ExprKind};
use crate::types::Type;

impl Lowerer {
    pub(super) fn lower_stmt_core(&mut self, stmt: &hir::Stmt) -> ValueId {
        match stmt {
            hir::Stmt::Bind(b) => {
                let val = match b.access_mod {
                    Some(AccessMod::Take) => self.lower_expr(&b.value),
                    _ => {
                        if matches!(b.ownership, hir::Ownership::Borrowed)
                            && matches!(b.value.kind, ExprKind::Field(..) | ExprKind::Index(..))
                        {
                            self.lower_expr(&b.value)
                        } else if matches!(b.ownership, hir::Ownership::Borrowed)
                            && is_container_read_method(&b.value)
                        {
                            let v = self.lower_expr(&b.value);
                            self.mark_method_call_borrow(v);
                            v
                        } else {
                            self.lower_expr_owned(&b.value)
                        }
                    }
                };

                if matches!(b.access_mod, Some(AccessMod::Take)) {
                    if let ExprKind::Field(obj, field, _) = &b.value.kind {
                        if let ExprKind::Var(parent_did, parent_name) = &obj.kind {
                            if !b.value.ty.is_trivially_droppable() {
                                let parent_ty = obj.ty.clone();
                                // If the parent struct is itself an actor field,
                                // read it from / write it back to the state
                                // struct; otherwise use the SSA local.
                                if let Some((parent_field_sym, parent_field_ty)) =
                                    self.field_lookup(*parent_did)
                                {
                                    let self_state = self.field_self();
                                    let state_ty = self.field_state_ty();
                                    let parent_val = self.emit(
                                        InstKind::FieldGet(self_state, parent_field_sym),
                                        parent_field_ty,
                                        b.span,
                                    );
                                    let cleared = self.emit(
                                        InstKind::FieldClear(parent_val, field.clone()),
                                        parent_ty,
                                        b.span,
                                    );
                                    self.emit_void_typed(
                                        InstKind::FieldSet(self_state, parent_field_sym, cleared),
                                        state_ty,
                                        b.span,
                                    );
                                } else {
                                    // SSA-form field tombstone: read the parent's
                                    // current SSA value, emit `FieldClear` to
                                    // produce a new struct value with the field
                                    // zeroed, and write the new value back as the
                                    // parent's definition. No memory demotion
                                    // needed — Perceus + drop see the cleared
                                    // field on the new SSA value.
                                    let parent_val = self.read_var(
                                        parent_name.clone(),
                                        self.current_block,
                                        parent_ty.clone(),
                                        b.span,
                                    );
                                    let cleared = self.emit(
                                        InstKind::FieldClear(parent_val, field.clone()),
                                        parent_ty,
                                        b.span,
                                    );
                                    self.write_var(
                                        parent_name.clone(),
                                        self.current_block,
                                        cleared,
                                    );
                                }
                            }
                        }
                    }
                }

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
                // In an actor handler, `field is expr` desugars to a `Bind`
                // that re-uses the field's canonical DefId. Such a rebinding
                // is a write to the persistent state struct, not a fresh SSA
                // local — store through `self_state`. Subsequent bare reads of
                // the field re-load from the struct (see `lower_expr_value`),
                // so no SSA rebinding is needed.
                if let Some((field_sym, _)) = self.field_lookup(b.def_id) {
                    let self_state = self.field_self();
                    let state_ty = self.field_state_ty();
                    self.emit_void_typed(
                        InstKind::FieldSet(self_state, field_sym, val),
                        state_ty,
                        b.span,
                    );
                } else {
                    self.write_var(b.name.clone(), self.current_block, val);
                }
                val
            }
            hir::Stmt::Assign(target, value, _span) => {
                let val = self.lower_expr_owned(value);
                match &target.kind {
                    ExprKind::Var(def_id, name) => {
                        // Bare assignment to an actor field stores through the
                        // persistent state struct rather than rebinding an SSA
                        // local.
                        if let Some((field_sym, _)) = self.field_lookup(*def_id) {
                            let self_state = self.field_self();
                            let state_ty = self.field_state_ty();
                            self.emit_void_typed(
                                InstKind::FieldSet(self_state, field_sym, val),
                                state_ty,
                                target.span,
                            );
                        } else {
                            self.write_var(name.clone(), self.current_block, val);
                        }
                    }
                    ExprKind::Field(obj, field, _) => {
                        self.lower_field_assign(obj, &field.as_str(), val, target.span);
                    }
                    ExprKind::Index(arr, idx) => {
                        if let ExprKind::Var(def_id, name) = &arr.kind {
                            let a = self.lower_expr(arr);
                            let i = self.lower_expr(idx);
                            let arr_ty = arr.ty.clone();
                            let updated =
                                self.emit(InstKind::IndexSet(a, i, val), arr_ty, target.span);
                            // If the indexed array is itself an actor field,
                            // write the updated array back into the state
                            // struct; otherwise rebind the SSA local.
                            if let Some((field_sym, _)) = self.field_lookup(*def_id) {
                                let self_state = self.field_self();
                                let state_ty = self.field_state_ty();
                                self.emit_void_typed(
                                    InstKind::FieldSet(self_state, field_sym, updated),
                                    state_ty,
                                    target.span,
                                );
                            } else {
                                self.write_var(name.clone(), self.current_block, updated);
                            }
                            return val;
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
            hir::Stmt::Drop(_, name, ty, span) => {
                if self.var_types.contains_key(name) {
                    let val = self.read_var(name.clone(), self.current_block, ty.clone(), *span);
                    self.emit_void(InstKind::Drop(val, ty.clone()), *span);
                }
                self.emit(InstKind::Void, Type::Void, *span)
            }
            hir::Stmt::TupleBind(bindings, value, _span) => {
                let val = self.lower_expr(value);
                for (i, (_id, name, bind_ty)) in bindings.iter().enumerate() {
                    let idx = self.emit(InstKind::IntConst(i as i64), Type::I64, Span::dummy());
                    let elem = self.emit(InstKind::Index(val, idx), bind_ty.clone(), Span::dummy());
                    self.write_var(name.clone(), self.current_block, elem);
                }
                val
            }
            hir::Stmt::Defer(body, span) => {
                self.function_defers.push(body.clone());
                self.emit(InstKind::Void, Type::Void, *span)
            }
            _ => unreachable!("statement dispatched to wrong MIR lowering module"),
        }
    }
}

impl Lowerer {
    pub(super) fn lower_block_stmts(&mut self, stmts: &[hir::Stmt]) {
        for stmt in stmts {
            self.lower_stmt(stmt);
        }
    }

    pub(super) fn mark_method_call_borrow(&mut self, v: ValueId) {
        let blk = self.func.block_mut(self.current_block);
        for inst in blk.insts.iter_mut().rev() {
            if inst.dest == Some(v) {
                if let InstKind::MethodCall(_, _, _, borrow) = &mut inst.kind {
                    *borrow = true;
                }
                break;
            }
        }
    }
}

pub(crate) fn is_container_read_method(expr: &hir::Expr) -> bool {
    let name = match &expr.kind {
        ExprKind::VecMethod(_, n, _) | ExprKind::MapMethod(_, n, _) => n.as_str(),
        _ => return false,
    };
    matches!(
        &*name,
        "get" | "first" | "last" | "front" | "back" | "peek" | "peek_min" | "peek_max" | "top"
    )
}
