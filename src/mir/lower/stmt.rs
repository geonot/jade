use super::super::*;
use super::Lowerer;
use crate::ast::{AccessMod, Span};
use crate::hir::{self, ExprKind};
use crate::types::Type;

impl Lowerer {
    pub(super) fn lower_stmt_core(&mut self, stmt: &hir::Stmt) -> ValueId {
        match stmt {
            hir::Stmt::Bind(b) => {
                // P4 auto-copy: at the binding boundary, when the RHS would
                // otherwise alias storage owned by a parent aggregate
                // (heap-typed field/index read), insert an implicit deep
                // `Clone` so the new binding owns its value independently.
                // `ref`/`mut`/`take` modifiers opt out and keep the raw
                // load (borrow / move semantics).
                let val = match b.access_mod {
                    Some(AccessMod::Ref) | Some(AccessMod::Mut) | Some(AccessMod::Take) => {
                        self.lower_expr(&b.value)
                    }
                    _ => {
                        // R3.3: a binding the typer demoted to `Borrowed`
                        // for a Field/Index read of a clonable type must
                        // NOT be auto-cloned at the MIR boundary — the
                        // typer already suppressed the corresponding
                        // scope-exit Drop, so emitting a Clone here would
                        // leak the fresh allocation.  For all other
                        // owned bindings the auto-clone in
                        // `lower_expr_owned` is what gives the new
                        // binding an independent value.
                        if matches!(b.ownership, hir::Ownership::Borrowed)
                            && matches!(
                                b.value.kind,
                                ExprKind::Field(..) | ExprKind::Index(..)
                            )
                        {
                            self.lower_expr(&b.value)
                        } else if matches!(b.ownership, hir::Ownership::Borrowed)
                            && is_container_read_method(&b.value)
                        {
                            // R3.3 (container reads): same rationale as
                            // Field/Index above. We also flip the just-
                            // emitted MethodCall's `borrow` flag so
                            // codegen can skip the deep clone inside
                            // `vec_get_idx` (and equivalent paths).
                            let v = self.lower_expr(&b.value);
                            self.mark_method_call_borrow(v);
                            v
                        } else {
                            self.lower_expr_owned(&b.value)
                        }
                    }
                };

                // P4 §5.2 Perceus partial-move: `x is take y.field` MUST
                // move the field out and prevent a later double-drop when
                // the parent `y` is dropped (or consumed). Implementation:
                // demote `y` to a mem_var and tombstone the field slot with
                // its LLVM zero-init. All Jinn heap types' drop is
                // null/zero-safe, so the parent's eventual drop becomes a
                // no-op for the moved field.
                if matches!(b.access_mod, Some(AccessMod::Take)) {
                    if let ExprKind::Field(obj, field, _) = &b.value.kind {
                        if let ExprKind::Var(_, parent_name) = &obj.kind {
                            // Only meaningful for non-trivially-droppable fields;
                            // POD fields have no drop, so the tombstone is moot.
                            if !b.value.ty.is_trivially_droppable() {
                                // Demote the parent to memory if it isn't
                                // already, so we can tombstone in place.
                                if !self.mem_vars.contains(parent_name) {
                                    let mut set = std::collections::HashSet::new();
                                    set.insert(parent_name.clone());
                                    self.demote_vars_to_memory(&set, b.span);
                                }
                                self.func.block_mut(self.current_block).insts.push(
                                    Instruction {
                                        dest: None,
                                        kind: InstKind::FieldTombstone(
                                            parent_name.clone(),
                                            field.clone(),
                                        ),
                                        ty: Type::Void,
                                        span: b.span,
                                        def_id: None,
                                    },
                                );
                            }
                        }
                    }
                }
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
                // Same boundary as Bind: RHS is moved into the new home,
                // so auto-clone heap-typed field/index reads.
                let val = self.lower_expr_owned(value);
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
                                        kind: InstKind::FieldStore(*name, *field, val),
                                        ty: obj_ty,
                                        span: target.span,
                                        def_id: None,
                                    });
                                return val;
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
                                return val;
                            }
                            // Non-mem_var array: emit IndexSet and store updated value back.
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

    /// R3.3 (container reads): after emitting a container read at a
    /// `Borrowed`-demoted binding, walk back through the current block to
    /// find the just-emitted instruction whose `dest == v` and, if it is a
    /// `MethodCall`, set its `borrow` flag. Codegen uses this flag to skip
    /// the deep clone that would otherwise be required for the heap-typed
    /// returned element.
    ///
    /// Safe no-op if the producing instruction isn't a `MethodCall` (e.g. it
    /// was constant-folded, substituted, or the value already lives in
    /// `var_map`).
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

/// Returns true when `expr` is a container method-call shape that returns an
/// element aliased to internal storage (Vec.get/first/last, Map.get,
/// Set.peek, PQ.peek*, Deque.front/back). Used by R3.3 demotion at both the
/// typer (escape::apply_demotions) and MIR (lower_stmt(Bind)) boundaries.
pub(crate) fn is_container_read_method(expr: &hir::Expr) -> bool {
    let name = match &expr.kind {
        ExprKind::VecMethod(_, n, _)
        | ExprKind::MapMethod(_, n, _)
        | ExprKind::SetMethod(_, n, _)
        | ExprKind::PQMethod(_, n, _)
        | ExprKind::DequeMethod(_, n, _) => n.as_str(),
        _ => return false,
    };
    matches!(
        &*name,
        "get" | "first" | "last" | "front" | "back" | "peek" | "peek_min" | "peek_max" | "top"
    )
}
