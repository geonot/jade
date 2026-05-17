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
            | ExprKind::MapMethod(obj, name, args)
            | ExprKind::SetMethod(obj, name, args)
            | ExprKind::PQMethod(obj, name, args)
            | ExprKind::DequeMethod(obj, name, args) => {
                let obj_val = self.lower_expr(obj);
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::MethodCall(obj_val, *name, vals, false), ty, span)
            }
            ExprKind::VecNew(elems) | ExprKind::NDArrayNew(elems) | ExprKind::SIMDNew(elems) => {
                let vals: Vec<ValueId> = elems.iter().map(|e| self.lower_expr(e)).collect();
                self.emit(InstKind::VecNew(vals), ty, span)
            }
            ExprKind::MapNew => self.emit(InstKind::MapInit, ty, span),

            ExprKind::SetNew => self.emit(InstKind::SetInit, ty, span),
            ExprKind::PQNew => self.emit(InstKind::PQInit, ty, span),
            ExprKind::DequeNew => self.emit(InstKind::DequeInit, ty, span),

            ExprKind::ListComp(body_expr, _def_id, bind, iter, end, cond) => {
                // Desugar: vec = VecNew(); for bind in iter..end { if cond { VecPush(vec, body) } }
                let vec_val = self.emit(InstKind::VecNew(vec![]), ty.clone(), span);
                let iter_val = self.lower_expr(iter);

                let cond_bb = self.new_block("listcomp.cond");
                let body_bb = self.new_block("listcomp.body");
                let inc_bb = self.new_block("listcomp.inc");
                let exit_bb = self.new_block("listcomp.exit");

                // Create loop index using Store/Load.
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
                // Bind the loop variable — for collection iteration (no end),
                // bind the element at the current index, not the index itself.
                if end.is_none() {
                    let elem = self.emit(InstKind::Index(iter_val, idx), ty.clone(), span);
                    self.var_map.insert(Symbol::intern(bind), elem);
                } else {
                    self.var_map.insert(Symbol::intern(bind), idx);
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

                // Increment index.
                self.switch_to(inc_bb);
                let cur_idx = self.emit(InstKind::Load(idx_name), Type::I64, span);
                let next_idx =
                    self.emit(InstKind::BinOp(BinOp::Add, cur_idx, one), Type::I64, span);
                self.emit_void_typed(InstKind::Store(idx_name, next_idx), Type::I64, span);
                self.set_terminator(Terminator::Goto(cond_bb));

                self.switch_to(exit_bb);
                vec_val
            }

            // Concurrency primitives — lower as dedicated MIR instructions

            // Iterator
            ExprKind::IterNext(iter_var, type_name, method_name) => {
                if let Some(&v) = self.var_map.get(iter_var) {
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
