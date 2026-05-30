use super::super::*;
use super::Lowerer;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;

impl Lowerer {
    pub(super) fn lower_expr_collections(&mut self, expr: &hir::Expr) -> ValueId {
        let span = expr.span;
        let ty = expr.ty.clone();
        match &expr.kind {
            ExprKind::StringMethod(obj, name, args)
            | ExprKind::DeferredMethod(obj, name, args)
            | ExprKind::VecMethod(obj, name, args)
            | ExprKind::MapMethod(obj, name, args) => {
                let obj_val = self.lower_expr(obj);
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::MethodCall(obj_val, *name, vals, false), ty, span)
            }
            ExprKind::VecNew(elems) => {
                let vals: Vec<ValueId> = elems.iter().map(|e| self.lower_expr(e)).collect();
                self.emit(InstKind::VecNew(vals), ty, span)
            }
            ExprKind::MapNew => self.emit(InstKind::MapInit, ty, span),

            ExprKind::ListComp(body_expr, _def_id, bind, iter, end, cond) => {
                let vec_val = self.emit(InstKind::VecNew(vec![]), ty.clone(), span);
                let iter_val = self.lower_expr(iter);

                let cond_bb = self.new_block("listcomp.cond");
                let body_bb = self.new_block("listcomp.body");
                let inc_bb = self.new_block("listcomp.inc");
                let exit_bb = self.new_block("listcomp.exit");

                let init_val = if end.is_some() {
                    iter_val
                } else {
                    self.emit(InstKind::IntConst(0), Type::I64, span)
                };
                let one = self.emit(InstKind::IntConst(1), Type::I64, span);
                let end_val = if let Some(e) = end {
                    self.lower_expr(e)
                } else {
                    self.emit(InstKind::VecLen(iter_val), Type::I64, span)
                };
                let idx_name = Symbol::intern(&format!("__listcomp_idx_{bind}"));
                self.emit_void_typed(InstKind::Store(idx_name, init_val), Type::I64, span);

                self.set_terminator(Terminator::Goto(cond_bb));
                self.switch_to(cond_bb);
                let idx = self.emit(InstKind::Load(idx_name), Type::I64, span);
                let cmp = self.emit(
                    InstKind::Cmp(CmpOp::Lt, idx, end_val, Type::I64),
                    Type::Bool,
                    span,
                );
                self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                self.switch_to(body_bb);

                if end.is_none() {
                    let elem = self.emit(InstKind::Index(iter_val, idx), ty.clone(), span);
                    self.write_var(Symbol::intern(bind), self.current_block, elem);
                } else {
                    self.write_var(Symbol::intern(bind), self.current_block, idx);
                }
                let elem_val = self.lower_expr(body_expr);
                if let Some(c) = cond {
                    let filter_bb = self.new_block("listcomp.filter");
                    let push_bb = self.new_block("listcomp.push");
                    let cond_val = self.lower_expr(c);
                    self.set_terminator(Terminator::Branch(cond_val, push_bb, filter_bb));

                    self.switch_to(push_bb);
                    self.emit_void(InstKind::VecPush(vec_val, elem_val), span);
                    self.set_terminator(Terminator::Goto(inc_bb));

                    self.switch_to(filter_bb);
                    self.set_terminator(Terminator::Goto(inc_bb));
                } else {
                    self.emit_void(InstKind::VecPush(vec_val, elem_val), span);
                    self.set_terminator(Terminator::Goto(inc_bb));
                }

                self.switch_to(inc_bb);
                let cur_idx = self.emit(InstKind::Load(idx_name), Type::I64, span);
                let next_idx =
                    self.emit(InstKind::BinOp(BinOp::Add, cur_idx, one), Type::I64, span);
                self.emit_void_typed(InstKind::Store(idx_name, next_idx), Type::I64, span);
                self.set_terminator(Terminator::Goto(cond_bb));

                self.switch_to(exit_bb);
                vec_val
            }

            ExprKind::IterNext(iter_var, type_name, method_name) => {
                if let Some(vty) = self.var_types.get(iter_var).cloned() {
                    let v = self.read_var(iter_var.clone(), self.current_block, vty, span);
                    self.emit(
                        InstKind::MethodCall(
                            v,
                            Symbol::intern(&format!("{type_name}_{method_name}")),
                            vec![],
                            false,
                        ),
                        ty,
                        span,
                    )
                } else {
                    self.emit(
                        InstKind::Call(
                            Symbol::intern(&format!("__iter_{type_name}_{method_name}")),
                            vec![],
                        ),
                        ty,
                        span,
                    )
                }
            }
            _ => unreachable!("expression dispatched to wrong MIR lowering module"),
        }
    }
}
