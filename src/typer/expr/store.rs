//! Extracted typing rules.

#![allow(unused_imports, unused_variables)]

use super::super::unify;
use super::super::{Typer, VarInfo};
use crate::ast::{self, BinOp, Span, UnaryOp};
use crate::hir::{self, CoercionKind, DefId, Ownership};
use crate::intern::Symbol;
use crate::types::Type;
use std::path::PathBuf;

impl Typer {
    pub(in crate::typer) fn lower_expr_store_query(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::StoreQuery(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, &store.as_str())?;
                let struct_name = Symbol::intern(&format!("__store_{store}"));
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreQuery(store.clone(), Box::new(hfilter)),
                    ty: Type::Struct(struct_name, vec![]),
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_store_count(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::StoreCount(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                if let Some(filter) = filter {
                    let hfilter = self.lower_store_filter(filter, &schema, &store.as_str())?;
                    Ok(hir::Expr {
                        kind: hir::ExprKind::ViewCount(store.clone(), Box::new(hfilter)),
                        ty: Type::I64,
                        span: *span,
                    })
                } else {
                    Ok(hir::Expr {
                        kind: hir::ExprKind::StoreCount(store.clone()),
                        ty: Type::I64,
                        span: *span,
                    })
                }
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_store_all(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::StoreAll(store, span) => {
                if !self.store_schemas.contains_key(store) {
                    return Err(format!("unknown store '{store}'"));
                }
                let struct_name = Symbol::intern(&format!("__store_{store}"));
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreAll(store.clone()),
                    ty: Type::Ptr(Box::new(Type::Struct(struct_name, vec![]))),
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_store_get(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::StoreGet(store, key_expr, span) => {
                if !self.store_schemas.contains_key(store) {
                    return Err(format!("unknown store '{store}'"));
                }
                let hkey = self.lower_expr(key_expr)?;
                let struct_name = Symbol::intern(&format!("__store_{store}"));
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreGet(store.clone(), Box::new(hkey)),
                    ty: Type::Struct(struct_name, vec![]),
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_store_first(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::StoreFirst(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, &store.as_str())?;
                let struct_name = Symbol::intern(&format!("__store_{store}"));
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreFirst(store.clone(), Box::new(hfilter)),
                    ty: Type::Struct(struct_name, vec![]),
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_store_exists(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::StoreExists(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, &store.as_str())?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreExists(store.clone(), Box::new(hfilter)),
                    ty: Type::Bool,
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_store_distinct(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::StoreDistinct(store, field, span) => {
                if !self.store_schemas.contains_key(store) {
                    return Err(format!("unknown store '{store}'"));
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreDistinct(store.clone(), field.clone()),
                    ty: Type::Vec(Box::new(Type::String)),
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }
}
