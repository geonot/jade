//! Extracted call-typing rules.

#![allow(unused_imports, unused_variables)]

use std::collections::HashMap;

use crate::ast::{self, Expr, Span};
use crate::hir::{self, ExprKind};
use crate::types::Type;
use crate::intern::Symbol;
use super::super::{Typer, VarInfo, DeferredField};
use super::super::unify;

impl Typer {
    pub(in crate::typer) fn dispatch_view_methods(
        &mut self,
        obj: &ast::Expr,
        method: &str,
        args: &[ast::Expr],
        span: crate::ast::Span,
    ) -> Result<Option<hir::Expr>, String> {
        // View method dispatch: view_name.count(), view_name.all()
        if let ast::Expr::Ident(name, _) = obj {
            if let Some((source, clauses)) = self.view_defs.get(&name.as_str()).cloned() {
                let schema = self
                    .store_schemas
                    .get(&source.as_str())
                    .ok_or_else(|| format!("view '{name}' references unknown store '{source}'"))?
                    .clone();

                // Build filter from view's where clauses
                let where_exprs: Vec<(ast::Expr, ast::Span)> = clauses
                    .iter()
                    .filter_map(|c| {
                        if let ast::QueryClause::Where(expr, cspan) = c {
                            Some((expr.clone(), *cspan))
                        } else {
                            None
                        }
                    })
                    .collect();

                if where_exprs.is_empty() {
                    // View with no filter — delegate to source store
                    match method {
                        "count" => {
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::StoreCount(source),
                                ty: Type::I64,
                                span,
                            }));
                        }
                        "all" => {
                            let struct_name = Symbol::intern(&format!("__store_{source}"));
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::StoreAll(source),
                                ty: Type::Ptr(Box::new(Type::Struct(struct_name, vec![]))),
                                span,
                            }));
                        }
                        "select" | "first" => {
                            // delegate to StoreQuery on source (returns first match)
                            if args.is_empty() {
                                return Err(format!("view .{method}() requires a filter argument"));
                            }
                            let filter_expr = &args[0];
                            let ast_filter = Self::expr_to_store_filter(filter_expr, span)?;
                            let hfilter = self.lower_store_filter(&ast_filter, &schema, &source.as_str())?;
                            let struct_name = Symbol::intern(&format!("__store_{source}"));
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::StoreQuery(source, Box::new(hfilter)),
                                ty: Type::Struct(struct_name, vec![]),
                                span,
                            }));
                        }
                        "exists" => {
                            if args.is_empty() {
                                return Err("view .exists() requires a filter argument".into());
                            }
                            let filter_expr = &args[0];
                            let ast_filter = Self::expr_to_store_filter(filter_expr, span)?;
                            let hfilter = self.lower_store_filter(&ast_filter, &schema, &source.as_str())?;
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::StoreExists(source, Box::new(hfilter)),
                                ty: Type::Bool,
                                span,
                            }));
                        }
                        _ => {
                            return Err(format!(
                                "views support .count(), .all(), .select(), .first(), .exists(); got .{method}()"
                            ));
                        }
                    }
                }

                let ast_filter = Self::merge_where_clauses(&where_exprs)?;
                let hfilter = self.lower_store_filter(&ast_filter, &schema, &source.as_str())?;

                match method {
                    "count" => {
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::ViewCount(source, Box::new(hfilter)),
                            ty: Type::I64,
                            span,
                        }));
                    }
                    "all" => {
                        let struct_name = Symbol::intern(&format!("__store_{source}"));
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::ViewAll(source, Box::new(hfilter)),
                            ty: Type::Ptr(Box::new(Type::Struct(struct_name, vec![]))),
                            span,
                        }));
                    }
                    "select" | "first" => {
                        // For filtered views, the view's filter is already the query filter
                        let struct_name = Symbol::intern(&format!("__store_{source}"));
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::StoreQuery(source.clone(), Box::new(hfilter)),
                            ty: Type::Struct(struct_name, vec![]),
                            span,
                        }));
                    }
                    "exists" => {
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::StoreExists(source.clone(), Box::new(hfilter)),
                            ty: Type::Bool,
                            span,
                        }));
                    }
                    _ => {
                        return Err(format!(
                            "views support .count(), .all(), .select(), .first(), .exists(); got .{method}()"
                        ));
                    }
                }
            }
        }

        Ok(None)
    }

}
