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
    pub(super) fn lower_stmt_p3(&mut self, stmt: &hir::Stmt) -> Option<ValueId> {
        Some(match stmt {
            hir::Stmt::StoreInsert(store_name, exprs, span) => {
                let vals: Vec<_> = exprs.iter().map(|e| self.lower_expr(e)).collect();
                self.emit(
                    InstKind::Call(Symbol::intern(&format!("__store_insert_{store_name}")), vals),
                    Type::Void,
                    *span,
                )
            }

            hir::Stmt::StoreDelete(store_name, filter, span) => {
                // Encode filter field+op in the call name so MIR codegen can reconstruct.
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
                let mut extra_vals = Vec::new();
                for (_logic_op, cond) in &filter.extra {
                    extra_vals.push(self.lower_expr(&cond.value));
                }
                let mut all_vals = vec![filter_val];
                all_vals.extend(extra_vals);
                // Encode extra filter conditions in the name: __store_delete_{name}__{field}__{op}[__and__{field2}__{op2}]*
                let mut encoded =
                    format!("__store_delete_{store_name}__{}__{op_str}", filter.field);
                for (logic_op, cond) in &filter.extra {
                    let lop = match logic_op {
                        ast::LogicalOp::And => "and",
                        ast::LogicalOp::Or => "or",
                    };
                    let cop_str = match cond.op {
                        ast::BinOp::Eq => "eq",
                        ast::BinOp::Ne => "ne",
                        ast::BinOp::Lt => "lt",
                        ast::BinOp::Le => "le",
                        ast::BinOp::Gt => "gt",
                        ast::BinOp::Ge => "ge",
                        _ => "eq",
                    };
                    encoded.push_str(&format!("__{lop}__{}__{cop_str}", cond.field));
                }
                self.emit(InstKind::Call(Symbol::intern(&encoded), all_vals), Type::Void, *span)
            }

            hir::Stmt::StoreDestroy(store_name, filter, span) => {
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
                let mut all_vals = vec![filter_val];
                for (_logic_op, cond) in &filter.extra {
                    all_vals.push(self.lower_expr(&cond.value));
                }
                let mut encoded =
                    format!("__store_destroy_{store_name}__{}__{op_str}", filter.field);
                for (logic_op, cond) in &filter.extra {
                    let lop = match logic_op {
                        ast::LogicalOp::And => "and",
                        ast::LogicalOp::Or => "or",
                    };
                    let cop_str = match cond.op {
                        ast::BinOp::Eq => "eq",
                        ast::BinOp::Ne => "ne",
                        ast::BinOp::Lt => "lt",
                        ast::BinOp::Le => "le",
                        ast::BinOp::Gt => "gt",
                        ast::BinOp::Ge => "ge",
                        _ => "eq",
                    };
                    encoded.push_str(&format!("__{lop}__{}__{cop_str}", cond.field));
                }
                self.emit(InstKind::Call(Symbol::intern(&encoded), all_vals), Type::Void, *span)
            }

            hir::Stmt::StoreRestore(store_name, filter, span) => {
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
                let mut all_vals = vec![filter_val];
                for (_logic_op, cond) in &filter.extra {
                    all_vals.push(self.lower_expr(&cond.value));
                }
                let mut encoded =
                    format!("__store_restore_{store_name}__{}__{op_str}", filter.field);
                for (logic_op, cond) in &filter.extra {
                    let lop = match logic_op {
                        ast::LogicalOp::And => "and",
                        ast::LogicalOp::Or => "or",
                    };
                    let cop_str = match cond.op {
                        ast::BinOp::Eq => "eq",
                        ast::BinOp::Ne => "ne",
                        ast::BinOp::Lt => "lt",
                        ast::BinOp::Le => "le",
                        ast::BinOp::Gt => "gt",
                        ast::BinOp::Ge => "ge",
                        _ => "eq",
                    };
                    encoded.push_str(&format!("__{lop}__{}__{cop_str}", cond.field));
                }
                self.emit(InstKind::Call(Symbol::intern(&encoded), all_vals), Type::Void, *span)
            }

            hir::Stmt::StoreSave(store_name, span) => self.emit(
                InstKind::Call(Symbol::intern(&format!("__store_save_{store_name}")), vec![]),
                Type::Void,
                *span,
            ),

            hir::Stmt::StoreSet(store_name, fields, filter, span) => {
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
                let mut extra_vals = Vec::new();
                for (_logic_op, cond) in &filter.extra {
                    extra_vals.push(self.lower_expr(&cond.value));
                }
                let mut all_vals = vec![filter_val];
                all_vals.extend(extra_vals);
                // Append field assignment values after filter values.
                let field_names: Vec<Symbol> = fields.iter().map(|(n, _)| n.clone()).collect();
                all_vals.extend(fields.iter().map(|(_, e)| self.lower_expr(e)));
                // Encode: __store_set_{name}__{field}__{op}[__and/or_{field2}__{op2}]*__fields_{f1}_{f2}_...
                let mut encoded = format!("__store_set_{store_name}__{}__{op_str}", filter.field);
                for (logic_op, cond) in &filter.extra {
                    let lop = match logic_op {
                        ast::LogicalOp::And => "and",
                        ast::LogicalOp::Or => "or",
                    };
                    let cop_str = match cond.op {
                        ast::BinOp::Eq => "eq",
                        ast::BinOp::Ne => "ne",
                        ast::BinOp::Lt => "lt",
                        ast::BinOp::Le => "le",
                        ast::BinOp::Gt => "gt",
                        ast::BinOp::Ge => "ge",
                        _ => "eq",
                    };
                    encoded.push_str(&format!("__{lop}__{}__{cop_str}", cond.field));
                }
                encoded.push_str("__fields");
                for fname in &field_names {
                    encoded.push_str(&format!("_{fname}"));
                }
                self.emit(InstKind::Call(Symbol::intern(&encoded), all_vals), Type::Void, *span)
            }

            _ => return None,
        })
    }
}
