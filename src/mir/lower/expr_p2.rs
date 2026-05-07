// Auto-split from lower.rs.
#![allow(unused_imports, unused_variables)]
use crate::intern::Symbol;
use super::super::*;
use crate::ast::{self, Span};
use crate::hir::{self, ExprKind, Pat};
use crate::types::Type;
use std::collections::{HashMap, HashSet};
use super::Lowerer;

impl Lowerer {
    pub(super) fn lower_expr_p2(&mut self, expr: &hir::Expr) -> Option<ValueId> {
        let span = expr.span;
        let ty = expr.ty.clone();
        Some(match &expr.kind {
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
            ExprKind::DynDispatch(obj, trait_name, method, args) => {
                let obj_val = self.lower_expr(obj);
                let arg_vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(
                    InstKind::DynDispatch(obj_val, *trait_name, *method, arg_vals),
                    ty,
                    span,
                )
            }
            ExprKind::DynCoerce(inner, type_name, trait_name) => {
                let inner_val = self.lower_expr(inner);
                self.emit(
                    InstKind::DynCoerce(inner_val, *type_name, *trait_name),
                    ty,
                    span,
                )
            }

            // Store operations — opaque calls
            ExprKind::StoreQuery(store_name, filter) => {
                let filter_val = self.lower_expr(&filter.value);
                let mut args = vec![filter_val];
                // Encode field name and op in the call name for codegen
                let op_str = match filter.op {
                    ast::BinOp::Eq => "eq",
                    ast::BinOp::Ne => "ne",
                    ast::BinOp::Lt => "lt",
                    ast::BinOp::Le => "le",
                    ast::BinOp::Gt => "gt",
                    ast::BinOp::Ge => "ge",
                    _ => "eq",
                };
                let mut name = format!("__store_query_{store_name}__{}__{op_str}", filter.field);
                // Encode extra compound conditions
                for (lop, cond) in &filter.extra {
                    let lop_str = match lop {
                        ast::LogicalOp::And => "and",
                        ast::LogicalOp::Or => "or",
                    };
                    let eop_str = match cond.op {
                        ast::BinOp::Eq => "eq",
                        ast::BinOp::Ne => "ne",
                        ast::BinOp::Lt => "lt",
                        ast::BinOp::Le => "le",
                        ast::BinOp::Gt => "gt",
                        ast::BinOp::Ge => "ge",
                        _ => "eq",
                    };
                    name.push_str(&format!("__{lop_str}__{}__{eop_str}", cond.field));
                    let ev = self.lower_expr(&cond.value);
                    args.push(ev);
                }
                self.emit(InstKind::Call(Symbol::intern(&name), args), ty, span)
            }
            ExprKind::StoreCount(store_name) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__store_count_{store_name}")), vec![]),
                ty,
                span,
            ),
            ExprKind::StoreAll(store_name) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__store_all_{store_name}")), vec![]),
                ty,
                span,
            ),
            ExprKind::ViewCount(store_name, filter) => {
                let filter_val = self.lower_expr(&filter.value);
                let mut args = vec![filter_val];
                let op_str = match filter.op {
                    ast::BinOp::Eq => "eq",
                    ast::BinOp::Ne => "ne",
                    ast::BinOp::Lt => "lt",
                    ast::BinOp::Le => "le",
                    ast::BinOp::Gt => "gt",
                    ast::BinOp::Ge => "ge",
                    _ => "eq",
                };
                let mut name = format!("__view_count_{store_name}__{}__{op_str}", filter.field);
                for (lop, cond) in &filter.extra {
                    let lop_str = match lop {
                        ast::LogicalOp::And => "and",
                        ast::LogicalOp::Or => "or",
                    };
                    let eop_str = match cond.op {
                        ast::BinOp::Eq => "eq",
                        ast::BinOp::Ne => "ne",
                        ast::BinOp::Lt => "lt",
                        ast::BinOp::Le => "le",
                        ast::BinOp::Gt => "gt",
                        ast::BinOp::Ge => "ge",
                        _ => "eq",
                    };
                    name.push_str(&format!("__{lop_str}__{}__{eop_str}", cond.field));
                    let ev = self.lower_expr(&cond.value);
                    args.push(ev);
                }
                self.emit(InstKind::Call(Symbol::intern(&name), args), ty, span)
            }
            ExprKind::ViewAll(store_name, filter) => {
                let filter_val = self.lower_expr(&filter.value);
                let mut args = vec![filter_val];
                let op_str = match filter.op {
                    ast::BinOp::Eq => "eq",
                    ast::BinOp::Ne => "ne",
                    ast::BinOp::Lt => "lt",
                    ast::BinOp::Le => "le",
                    ast::BinOp::Gt => "gt",
                    ast::BinOp::Ge => "ge",
                    _ => "eq",
                };
                let mut name = format!("__view_all_{store_name}__{}__{op_str}", filter.field);
                for (lop, cond) in &filter.extra {
                    let lop_str = match lop {
                        ast::LogicalOp::And => "and",
                        ast::LogicalOp::Or => "or",
                    };
                    let eop_str = match cond.op {
                        ast::BinOp::Eq => "eq",
                        ast::BinOp::Ne => "ne",
                        ast::BinOp::Lt => "lt",
                        ast::BinOp::Le => "le",
                        ast::BinOp::Gt => "gt",
                        ast::BinOp::Ge => "ge",
                        _ => "eq",
                    };
                    name.push_str(&format!("__{lop_str}__{}__{eop_str}", cond.field));
                    let ev = self.lower_expr(&cond.value);
                    args.push(ev);
                }
                self.emit(InstKind::Call(Symbol::intern(&name), args), ty, span)
            }

            ExprKind::StoreGet(store_name, key_expr) => {
                let val = self.lower_expr(key_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__store_get_{store_name}")), vec![val]),
                    ty,
                    span,
                )
            }

            ExprKind::StoreFirst(store_name, filter) => {
                let filter_val = self.lower_expr(&filter.value);
                let op_str = match filter.op {
                    ast::BinOp::Eq => "eq",
                    ast::BinOp::Ne => "ne",
                    ast::BinOp::Lt => "lt",
                    ast::BinOp::Le => "le",
                    ast::BinOp::Gt => "gt",
                    ast::BinOp::Ge => "ge",
                    _ => "eq",
                };
                let mut name = format!("__store_first_{store_name}__{}__{op_str}", filter.field);
                let mut args = vec![filter_val];
                for (logic_op, cond) in &filter.extra {
                    let lop_str = match logic_op {
                        ast::LogicalOp::And => "and",
                        ast::LogicalOp::Or => "or",
                    };
                    let eop_str = match cond.op {
                        ast::BinOp::Eq => "eq",
                        ast::BinOp::Ne => "ne",
                        ast::BinOp::Lt => "lt",
                        ast::BinOp::Le => "le",
                        ast::BinOp::Gt => "gt",
                        ast::BinOp::Ge => "ge",
                        _ => "eq",
                    };
                    name.push_str(&format!("__{lop_str}__{}__{eop_str}", cond.field));
                    args.push(self.lower_expr(&cond.value));
                }
                self.emit(InstKind::Call(Symbol::intern(&name), args), ty, span)
            }

            ExprKind::StoreExists(store_name, filter) => {
                let filter_val = self.lower_expr(&filter.value);
                let op_str = match filter.op {
                    ast::BinOp::Eq => "eq",
                    ast::BinOp::Ne => "ne",
                    ast::BinOp::Lt => "lt",
                    ast::BinOp::Le => "le",
                    ast::BinOp::Gt => "gt",
                    ast::BinOp::Ge => "ge",
                    _ => "eq",
                };
                let mut name = format!("__store_exists_{store_name}__{}__{op_str}", filter.field);
                let mut args = vec![filter_val];
                for (logic_op, cond) in &filter.extra {
                    let lop_str = match logic_op {
                        ast::LogicalOp::And => "and",
                        ast::LogicalOp::Or => "or",
                    };
                    let eop_str = match cond.op {
                        ast::BinOp::Eq => "eq",
                        ast::BinOp::Ne => "ne",
                        ast::BinOp::Lt => "lt",
                        ast::BinOp::Le => "le",
                        ast::BinOp::Gt => "gt",
                        ast::BinOp::Ge => "ge",
                        _ => "eq",
                    };
                    name.push_str(&format!("__{lop_str}__{}__{eop_str}", cond.field));
                    args.push(self.lower_expr(&cond.value));
                }
                self.emit(InstKind::Call(Symbol::intern(&name), args), ty, span)
            }

            ExprKind::StoreDistinct(store_name, field) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__store_distinct_{store_name}__{field}")), vec![]),
                ty,
                span,
            ),

            ExprKind::StoreSum(store_name, field) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__store_sum_{store_name}__{field}")), vec![]),
                ty,
                span,
            ),
            ExprKind::StoreAvg(store_name, field) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__store_avg_{store_name}__{field}")), vec![]),
                ty,
                span,
            ),
            ExprKind::StoreMin(store_name, field) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__store_min_{store_name}__{field}")), vec![]),
                ty,
                span,
            ),
            ExprKind::StoreMax(store_name, field) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__store_max_{store_name}__{field}")), vec![]),
                ty,
                span,
            ),

            ExprKind::StoreVersionCount(store_name, sid_expr) => {
                let sid = self.lower_expr(sid_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__store_version_count_{store_name}")), vec![sid]),
                    ty,
                    span,
                )
            }
            ExprKind::StoreHistory(store_name, sid_expr) => {
                let sid = self.lower_expr(sid_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__store_history_{store_name}")), vec![sid]),
                    ty,
                    span,
                )
            }
            ExprKind::StoreAtVersion(store_name, sid_expr, ver_expr) => {
                let sid = self.lower_expr(sid_expr);
                let ver = self.lower_expr(ver_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__store_at_version_{store_name}")), vec![sid, ver]),
                    ty,
                    span,
                )
            }

            // @kv store operations
            ExprKind::KvGet(store_name, key_expr) => {
                let key = self.lower_expr(key_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__kv_get_{store_name}")), vec![key]),
                    ty,
                    span,
                )
            }
            ExprKind::KvHas(store_name, key_expr) => {
                let key = self.lower_expr(key_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__kv_has_{store_name}")), vec![key]),
                    ty,
                    span,
                )
            }
            ExprKind::KvCount(store_name) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__kv_count_{store_name}")), vec![]),
                ty,
                span,
            ),
            ExprKind::KvSet(store_name, key_expr, val_expr) => {
                let key = self.lower_expr(key_expr);
                let val = self.lower_expr(val_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__kv_set_{store_name}")), vec![key, val]),
                    ty,
                    span,
                )
            }
            _ => return None,
        })
    }
}
