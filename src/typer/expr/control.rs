//! Extracted typing rules.

#![allow(unused_imports, unused_variables)]

use crate::intern::Symbol;
use std::path::PathBuf;
use crate::ast::{self, BinOp, Span, UnaryOp};
use crate::hir::{self, CoercionKind, DefId, Ownership};
use crate::types::Type;
use super::super::{Typer, VarInfo};
use super::super::unify;

impl Typer {
    pub(in crate::typer) fn lower_expr_if_expr(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::IfExpr(i) => {                let result_ty = expected
                .cloned()
                .unwrap_or_else(|| self.infer_ctx.fresh_var());
            let hi = self.lower_if(i, &result_ty)?;
            let ty = match hi.then.last() {
                Some(hir::Stmt::Expr(e)) => e.ty.clone(),
                _ => Type::Void,
            };
            if let Some(ref els) = hi.els {
                if let Some(hir::Stmt::Expr(e)) = els.last() {
                    let r =
                        self.infer_ctx
                            .unify_at(&ty, &e.ty, i.span, "if-expression branches");
                    self.collect_unify_error(r);
                }
            }
            for (_, branch) in &hi.elifs {
                if let Some(hir::Stmt::Expr(e)) = branch.last() {
                    let r = self.infer_ctx.unify_at(&ty, &e.ty, i.span, "elif branch");
                    self.collect_unify_error(r);
                }
            }
            Ok(hir::Expr {
                kind: hir::ExprKind::IfExpr(Box::new(hi)),
                ty,
                span: i.span,
            })
        },
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_pipe(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Pipe(left, right, extra_args, span) => {                self.lower_pipe(left, right, extra_args, *span)
        },
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_block(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Block(stmts, span) => {                self.push_scope();
            let mut hstmts = Vec::new();
            let len = stmts.len();
            for (i, s) in stmts.iter().enumerate() {
                if i == len - 1 {
                    if let ast::Stmt::Expr(e) = s {
                        let he = self.lower_expr_expected(e, expected)?;
                        hstmts.push(hir::Stmt::Expr(he));
                    } else {
                        hstmts.push(self.lower_stmt(s, &Type::Void)?);
                    }
                } else {
                    hstmts.push(self.lower_stmt(s, &Type::Void)?);
                }
            }
            self.pop_scope();
            let ty = match hstmts.last() {
                Some(hir::Stmt::Expr(e)) => e.ty.clone(),
                _ => Type::Void,
            };
            Ok(hir::Expr {
                kind: hir::ExprKind::Block(hstmts),
                ty,
                span: *span,
            })
        },
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_list_comp(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::ListComp(body_expr, var, iter_expr, iter_end, cond, span) => {                let hiter = self.lower_expr(iter_expr)?;

            let is_range = iter_end.is_some();
            let bind_ty = if is_range {
                Type::I64
            } else {
                match &hiter.ty {
                    Type::Array(et, _) | Type::Ptr(et) => *et.clone(),
                    Type::Vec(et) => *et.clone(),
                    _ => self.infer_ctx.fresh_var(),
                }
            };
            let bind_id = self.fresh_id();
            self.push_scope();
            self.define_var(
                var,
                VarInfo {
                    def_id: bind_id,
                    ty: bind_ty,
                    ownership: Ownership::Owned,
                    scheme: None,
                },
            );
            let hbody = self.lower_expr(body_expr)?;
            let hend = iter_end.as_ref().map(|c| self.lower_expr(c)).transpose()?;
            let hcond = cond
                .as_ref()
                .map(|m| self.lower_expr_expected(m, Some(&Type::Bool)))
                .transpose()?;
            self.pop_scope();

            let ty = Type::Ptr(Box::new(hbody.ty.clone()));
            Ok(hir::Expr {
                kind: hir::ExprKind::ListComp(
                    Box::new(hbody),
                    bind_id,
                    var.clone(),
                    Box::new(hiter),
                    hend.map(Box::new),
                    hcond.map(Box::new),
                ),
                ty,
                span: *span,
            })
        },
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_query(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Query(source, clauses, span) => {                // Extract store name from source expression
            let store_name = match source.as_ref() {
                ast::Expr::Ident(name, _) => name.clone(),
                _ => return Err("query block source must be a store name".into()),
            };
            let schema = self
                .store_schemas
                .get(&store_name)
                .ok_or_else(|| format!("unknown store '{store_name}'"))?
                .clone();

            // Collect clauses
            let mut where_exprs: Vec<(ast::Expr, ast::Span)> = Vec::new();
            let mut has_delete = false;
            let mut sets: Vec<(Symbol, ast::Expr)> = Vec::new();
            for clause in clauses {
                match clause {
                    ast::QueryClause::Where(expr, cspan) => {
                        where_exprs.push((expr.clone(), *cspan));
                    }
                    ast::QueryClause::Delete(_) => {
                        has_delete = true;
                    }
                    ast::QueryClause::Set(field, val, _) => {
                        sets.push((field.clone(), val.clone()));
                    }
                    ast::QueryClause::Sort(_, _, _) => {
                        return Err("query 'sort' clause is not yet implemented".into());
                    }
                    ast::QueryClause::Limit(_, _) => {
                        return Err("query 'limit' clause is not yet implemented".into());
                    }
                    ast::QueryClause::Take(_, _) => {
                        return Err("query 'take' clause is not yet implemented".into());
                    }
                    ast::QueryClause::Skip(_, _) => {
                        return Err("query 'skip' clause is not yet implemented".into());
                    }
                }
            }

            if where_exprs.is_empty() {
                return Err("query block requires at least one where clause".into());
            }

            let ast_filter = Self::merge_where_clauses(&where_exprs)?;
            let hfilter = self.lower_store_filter(&ast_filter, &schema, &store_name.as_str())?;

            if has_delete {
                // Delete query block — void expression, side-effect handled
                // via the stmt-level interceptor
                Ok(hir::Expr {
                    kind: hir::ExprKind::Void,
                    ty: Type::Void,
                    span: *span,
                })
            } else if !sets.is_empty() {
                // Set query block — void expression, side-effect handled
                // via the stmt-level interceptor
                Ok(hir::Expr {
                    kind: hir::ExprKind::Void,
                    ty: Type::Void,
                    span: *span,
                })
            } else {
                // Read query → StoreQuery
                let struct_name = Symbol::intern(&format!("__store_{store_name}"));
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreQuery(store_name, Box::new(hfilter)),
                    ty: Type::Struct(struct_name, vec![]),
                    span: *span,
                })
            }
        },
            _ => unreachable!(),
        }
    }

}
