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
                        if let ExprKind::Var(_, parent_name) = &obj.kind {
                            if !b.value.ty.is_trivially_droppable() {
                                if !self.mem_vars.contains(parent_name) {
                                    let mut set = std::collections::HashSet::new();
                                    set.insert(parent_name.clone());
                                    self.demote_vars_to_memory(&set, b.span);
                                }
                                self.func
                                    .block_mut(self.current_block)
                                    .insts
                                    .push(Instruction {
                                        dest: None,
                                        kind: InstKind::FieldTombstone(
                                            parent_name.clone(),
                                            field.clone(),
                                        ),
                                        ty: Type::Void,
                                        span: b.span,
                                        def_id: None,
                                    });
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
                if self.mem_vars.contains(&b.name) {
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
                let val = self.lower_expr_owned(value);
                match &target.kind {
                    ExprKind::Var(_, name) => {
                        if self.mem_vars.contains(name) {
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
                        if let ExprKind::Var(_, name) = &obj.kind {
                            if self.mem_vars.contains(name) {
                                let obj_ty = obj.ty.clone();
                                self.func
                                    .block_mut(self.current_block)
                                    .insts
                                    .push(Instruction {
                                        dest: None,
                                        kind: InstKind::FieldStore(*name, *field, val),
                                        ty: obj_ty,
                                        span: target.span,
                                        def_id: None,
                                    });
                                return val;
                            }
                        }

                        self.lower_field_assign(obj, &field.as_str(), val, target.span);
                    }
                    ExprKind::Index(arr, idx) => {
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
                                return val;
                            }

                            let a = self.lower_expr(arr);
                            let i = self.lower_expr(idx);
                            let arr_ty = arr.ty.clone();
                            let updated =
                                self.emit(InstKind::IndexSet(a, i, val), arr_ty, target.span);
                            self.var_map.insert(name.clone(), updated);
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
                if let Some(&val) = self.var_map.get(name) {
                    self.emit_void(InstKind::Drop(val, ty.clone()), *span);
                }
                self.emit(InstKind::Void, Type::Void, *span)
            }
            hir::Stmt::TupleBind(bindings, value, _span) => {
                let val = self.lower_expr(value);
                for (i, (_id, name, bind_ty)) in bindings.iter().enumerate() {
                    let idx = self.emit(InstKind::IntConst(i as i64), Type::I64, Span::dummy());
                    let elem = self.emit(InstKind::Index(val, idx), bind_ty.clone(), Span::dummy());
                    if self.mem_vars.contains(name) {
                        self.emit(
                            InstKind::Store(name.clone(), elem),
                            Type::Void,
                            Span::dummy(),
                        );
                    } else {
                        self.var_map.insert(name.clone(), elem);
                    }
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
