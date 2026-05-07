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
    pub(super) fn lower_expr_p3(&mut self, expr: &hir::Expr) -> Option<ValueId> {
        let span = expr.span;
        let ty = expr.ty.clone();
        Some(match &expr.kind {
            ExprKind::KvDel(store_name, key_expr) => {
                let key = self.lower_expr(key_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__kv_del_{store_name}")), vec![key]),
                    ty,
                    span,
                )
            }
            ExprKind::KvIncr(store_name, key_expr, delta_expr) => {
                let key = self.lower_expr(key_expr);
                let delta = self.lower_expr(delta_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__kv_incr_{store_name}")), vec![key, delta]),
                    ty,
                    span,
                )
            }

            // Specialized store operations
            ExprKind::VecInsert(store_name, vec_expr) => {
                let v = self.lower_expr(vec_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__vec_insert_{store_name}")), vec![v]),
                    ty,
                    span,
                )
            }
            ExprKind::VecCount(store_name) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__vec_count_{store_name}")), vec![]),
                ty,
                span,
            ),
            ExprKind::BloomTest(store_name, field_name, value_expr) => {
                let v = self.lower_expr(value_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__bloom_test_{store_name}_{field_name}")), vec![v]),
                    ty,
                    span,
                )
            }
            ExprKind::FtsSearch(store_name, field_name, query_expr) => {
                let q = self.lower_expr(query_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__fts_search_{store_name}_{field_name}")), vec![q]),
                    ty,
                    span,
                )
            }
            ExprKind::FtsCount(store_name, field_name) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__fts_count_{store_name}_{field_name}")), vec![]),
                ty,
                span,
            ),
            ExprKind::VecNearest(store_name, query_expr, k_expr) => {
                let q = self.lower_expr(query_expr);
                let k = self.lower_expr(k_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__vec_nearest_{store_name}")), vec![q, k]),
                    ty,
                    span,
                )
            }
            ExprKind::GraphFrom(store_name, node_expr) => {
                let n = self.lower_expr(node_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__graph_from_{store_name}")), vec![n]),
                    ty,
                    span,
                )
            }
            ExprKind::GraphTo(store_name, node_expr) => {
                let n = self.lower_expr(node_expr);
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__graph_to_{store_name}")), vec![n]),
                    ty,
                    span,
                )
            }
            ExprKind::TsLatest(store_name) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__ts_latest_{store_name}")), vec![]),
                ty,
                span,
            ),

            // Iterator
            ExprKind::IterNext(iter_var, type_name, method_name) => {
                if let Some(&v) = self.var_map.get(iter_var) {
                    self.emit(
                        InstKind::MethodCall(v, Symbol::intern(&format!("{type_name}_{method_name}")), vec![]),
                        ty,
                        span,
                    )
                } else {
                    self.emit(
                        InstKind::Call(Symbol::intern(&format!("__iter_{type_name}_{method_name}")), vec![]),
                        ty,
                        span,
                    )
                }
            }

            ExprKind::Unreachable => {
                self.set_terminator(Terminator::Unreachable);
                let dead = self.new_block("after.unreachable");
                self.switch_to(dead);
                self.emit(InstKind::Void, ty, span)
            }

            ExprKind::AsFormat(inner, _fmt_str) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__as_format".into(), vec![v]), ty, span)
            }

            ExprKind::Builder(name, fields) => {
                // Desugar builder into StructInit + field sets
                let inits: Vec<(Symbol, ValueId)> = fields
                    .iter()
                    .map(|(n, e)| (*n, self.lower_expr(e)))
                    .collect();
                self.emit(InstKind::StructInit(*name, inits), ty, span)
            }

            ExprKind::CowWrap(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__cow_wrap".into(), vec![v]), ty, span)
            }
            ExprKind::CowClone(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__cow_clone".into(), vec![v]), ty, span)
            }

            ExprKind::GeneratorCreate(_def_id, name, _body) => {
                // Body is compiled separately — don't inline it here.
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__gen_create_{name}")), vec![]),
                    ty,
                    span,
                )
            }
            ExprKind::GeneratorNext(gen_expr) => {
                let g = self.lower_expr(gen_expr);
                self.emit(InstKind::Call("__gen_next".into(), vec![g]), ty, span)
            }

            ExprKind::EnumUnwrap(inner, _enum_name, success_tag) => {
                let subj = self.lower_expr(inner);
                // Get tag (field "__tag" is i64 after extension)
                let tag = self.emit(InstKind::FieldGet(subj, "__tag".into()), Type::I64, span);
                let expected = self.emit(InstKind::IntConst(*success_tag as i64), Type::I64, span);
                let cmp = self.emit(
                    InstKind::Cmp(CmpOp::Eq, tag, expected, Type::I64),
                    Type::Bool,
                    span,
                );
                // Assert: panics with message if tag doesn't match
                self.emit(
                    InstKind::Assert(cmp, "unwrap called on Nothing/Err".into()),
                    Type::Void,
                    span,
                );
                // Extract field _0
                self.emit(InstKind::FieldGet(subj, "_0".into()), ty.clone(), span)
            }

            ExprKind::EnumIs(inner, check_tag) => {
                let subj = self.lower_expr(inner);
                let tag = self.emit(InstKind::FieldGet(subj, "__tag".into()), Type::I64, span);
                let expected = self.emit(InstKind::IntConst(*check_tag as i64), Type::I64, span);
                self.emit(
                    InstKind::Cmp(CmpOp::Eq, tag, expected, Type::I64),
                    Type::Bool,
                    span,
                )
            }

            ExprKind::Grad(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__grad".into(), vec![v]), ty, span)
            }
            ExprKind::Einsum(_pattern, args) => {
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::Call("__einsum".into(), vals), ty, span)
            }

            ExprKind::GlobalLoad(name) => {
                self.emit(InstKind::GlobalLoad(name.clone()), ty, span)
            }
            _ => return None,
        })
    }
}
