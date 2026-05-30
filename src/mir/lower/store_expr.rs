use super::super::*;
use super::Lowerer;
use crate::ast;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;

impl Lowerer {
    pub(super) fn lower_expr_store(&mut self, expr: &hir::Expr) -> ValueId {
        let span = expr.span;
        let ty = expr.ty.clone();
        match &expr.kind {
            ExprKind::StoreQuery(store_name, filter) => {
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
                let mut name = format!("__store_query_{store_name}__{}__{op_str}", filter.field);

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
                self.emit(InstKind::RuntimeOp(Symbol::intern(&name), args), ty, span)
            }
            ExprKind::StoreCount(store_name) => self.emit(
                InstKind::RuntimeOp(Symbol::intern(&format!("__store_count_{store_name}")),
                    vec![],
                ),
                ty,
                span,
            ),
            ExprKind::StoreAll(store_name) => self.emit(
                InstKind::RuntimeOp(Symbol::intern(&format!("__store_all_{store_name}")), vec![]),
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
                self.emit(InstKind::RuntimeOp(Symbol::intern(&name), args), ty, span)
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
                self.emit(InstKind::RuntimeOp(Symbol::intern(&name), args), ty, span)
            }
            ExprKind::StoreGet(store_name, key_expr) => {
                let val = self.lower_expr(key_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__store_get_{store_name}")),
                        vec![val],
                    ),
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
                self.emit(InstKind::RuntimeOp(Symbol::intern(&name), args), ty, span)
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
                self.emit(InstKind::RuntimeOp(Symbol::intern(&name), args), ty, span)
            }
            ExprKind::StoreDistinct(store_name, field) => self.emit(
                InstKind::RuntimeOp(Symbol::intern(&format!("__store_distinct_{store_name}__{field}")),
                    vec![],
                ),
                ty,
                span,
            ),

            ExprKind::StoreSum(store_name, field) => self.emit(
                InstKind::RuntimeOp(Symbol::intern(&format!("__store_sum_{store_name}__{field}")),
                    vec![],
                ),
                ty,
                span,
            ),
            ExprKind::StoreAvg(store_name, field) => self.emit(
                InstKind::RuntimeOp(Symbol::intern(&format!("__store_avg_{store_name}__{field}")),
                    vec![],
                ),
                ty,
                span,
            ),
            ExprKind::StoreMin(store_name, field) => self.emit(
                InstKind::RuntimeOp(Symbol::intern(&format!("__store_min_{store_name}__{field}")),
                    vec![],
                ),
                ty,
                span,
            ),
            ExprKind::StoreMax(store_name, field) => self.emit(
                InstKind::RuntimeOp(Symbol::intern(&format!("__store_max_{store_name}__{field}")),
                    vec![],
                ),
                ty,
                span,
            ),

            ExprKind::StoreVersionCount(store_name, sid_expr) => {
                let sid = self.lower_expr(sid_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__store_version_count_{store_name}")),
                        vec![sid],
                    ),
                    ty,
                    span,
                )
            }
            ExprKind::StoreHistory(store_name, sid_expr) => {
                let sid = self.lower_expr(sid_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__store_history_{store_name}")),
                        vec![sid],
                    ),
                    ty,
                    span,
                )
            }
            ExprKind::StoreAtVersion(store_name, sid_expr, ver_expr) => {
                let sid = self.lower_expr(sid_expr);
                let ver = self.lower_expr(ver_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__store_at_version_{store_name}")),
                        vec![sid, ver],
                    ),
                    ty,
                    span,
                )
            }

            ExprKind::KvGet(store_name, key_expr) => {
                let key = self.lower_expr(key_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__kv_get_{store_name}")), vec![key]),
                    ty,
                    span,
                )
            }
            ExprKind::KvHas(store_name, key_expr) => {
                let key = self.lower_expr(key_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__kv_has_{store_name}")), vec![key]),
                    ty,
                    span,
                )
            }
            ExprKind::KvCount(store_name) => self.emit(
                InstKind::RuntimeOp(Symbol::intern(&format!("__kv_count_{store_name}")), vec![]),
                ty,
                span,
            ),
            ExprKind::KvSet(store_name, key_expr, val_expr) => {
                let key = self.lower_expr(key_expr);
                let val = self.lower_expr(val_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__kv_set_{store_name}")),
                        vec![key, val],
                    ),
                    ty,
                    span,
                )
            }
            ExprKind::KvDel(store_name, key_expr) => {
                let key = self.lower_expr(key_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__kv_del_{store_name}")), vec![key]),
                    ty,
                    span,
                )
            }
            ExprKind::KvIncr(store_name, key_expr, delta_expr) => {
                let key = self.lower_expr(key_expr);
                let delta = self.lower_expr(delta_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__kv_incr_{store_name}")),
                        vec![key, delta],
                    ),
                    ty,
                    span,
                )
            }

            ExprKind::VecInsert(store_name, vec_expr) => {
                let v = self.lower_expr(vec_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__vec_insert_{store_name}")),
                        vec![v],
                    ),
                    ty,
                    span,
                )
            }
            ExprKind::VecCount(store_name) => self.emit(
                InstKind::RuntimeOp(Symbol::intern(&format!("__vec_count_{store_name}")), vec![]),
                ty,
                span,
            ),
            ExprKind::BloomTest(store_name, field_name, value_expr) => {
                let v = self.lower_expr(value_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__bloom_test_{store_name}_{field_name}")),
                        vec![v],
                    ),
                    ty,
                    span,
                )
            }
            ExprKind::FtsSearch(store_name, field_name, query_expr) => {
                let q = self.lower_expr(query_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__fts_search_{store_name}_{field_name}")),
                        vec![q],
                    ),
                    ty,
                    span,
                )
            }
            ExprKind::FtsCount(store_name, field_name) => self.emit(
                InstKind::RuntimeOp(Symbol::intern(&format!("__fts_count_{store_name}_{field_name}")),
                    vec![],
                ),
                ty,
                span,
            ),
            ExprKind::VecNearest(store_name, query_expr, k_expr) => {
                let q = self.lower_expr(query_expr);
                let k = self.lower_expr(k_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__vec_nearest_{store_name}")),
                        vec![q, k],
                    ),
                    ty,
                    span,
                )
            }
            ExprKind::GraphFrom(store_name, node_expr) => {
                let n = self.lower_expr(node_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__graph_from_{store_name}")),
                        vec![n],
                    ),
                    ty,
                    span,
                )
            }
            ExprKind::GraphTo(store_name, node_expr) => {
                let n = self.lower_expr(node_expr);
                self.emit(
                    InstKind::RuntimeOp(Symbol::intern(&format!("__graph_to_{store_name}")), vec![n]),
                    ty,
                    span,
                )
            }
            ExprKind::TsLatest(store_name) => self.emit(
                InstKind::RuntimeOp(Symbol::intern(&format!("__ts_latest_{store_name}")), vec![]),
                ty,
                span,
            ),
            _ => unreachable!("expression dispatched to wrong store MIR lowering module"),
        }
    }
}
