//! Per-statement typing rules.

use crate::intern::Symbol;
use crate::ast;
use crate::hir::{self, DefId, Ownership};
use crate::types::{Scheme, Type};

use super::super::{Typer, VarInfo};

impl Typer {
    pub(crate) fn lower_store_filter(
        &mut self,
        filter: &ast::StoreFilter,
        schema: &[(Symbol, Type)],
        store: &str,
    ) -> Result<hir::StoreFilter, String> {
        let field_ty = schema
            .iter()
            .find(|(n, _)| n == &filter.field)
            .map(|(_, t)| t);
        if field_ty.is_none() {
            return Err(format!("store '{store}' has no field '{}'", filter.field));
        }
        let hvalue = self.lower_expr_expected(&filter.value, field_ty)?;
        let mut hextra = Vec::new();
        for (lop, cond) in &filter.extra {
            let cond_field_ty = schema
                .iter()
                .find(|(n, _)| n == &cond.field)
                .map(|(_, t)| t);
            if cond_field_ty.is_none() {
                return Err(format!("store '{store}' has no field '{}'", cond.field));
            }
            let hv = self.lower_expr_expected(&cond.value, cond_field_ty)?;
            hextra.push((
                *lop,
                hir::StoreFilterCond {
                    field: cond.field.clone(),
                    op: cond.op,
                    value: hv,
                },
            ));
        }
        Ok(hir::StoreFilter {
            field: filter.field.clone(),
            op: filter.op,
            value: hvalue,
            span: filter.span,
            extra: hextra,
        })
    }

    /// Convert a general expression (from a query-block `where` clause) into an
    /// `ast::StoreFilter`.  The expression must be a tree of comparisons joined
    /// by `and` / `or`.
    pub(crate) fn expr_to_store_filter(
        expr: &ast::Expr,
        span: ast::Span,
    ) -> Result<ast::StoreFilter, String> {
        let mut conds: Vec<(Option<ast::LogicalOp>, Symbol, ast::BinOp, ast::Expr)> = Vec::new();
        Self::flatten_filter_expr(expr, None, &mut conds)?;
        if conds.is_empty() {
            return Err("query where clause must be a comparison".into());
        }
        let (_, field, op, value) = conds.remove(0);
        let extra = conds
            .into_iter()
            .map(|(lop, f, o, v)| {
                (
                    lop.unwrap_or(ast::LogicalOp::And),
                    ast::StoreFilterCond {
                        field: f,
                        op: o,
                        value: v,
                    },
                )
            })
            .collect();
        Ok(ast::StoreFilter {
            field,
            op,
            value,
            span,
            extra,
        })
    }

    fn flatten_filter_expr(
        expr: &ast::Expr,
        logical_op: Option<ast::LogicalOp>,
        out: &mut Vec<(Option<ast::LogicalOp>, Symbol, ast::BinOp, ast::Expr)>,
    ) -> Result<(), String> {
        match expr {
            ast::Expr::BinOp(left, ast::BinOp::And, right, _) => {
                Self::flatten_filter_expr(left, logical_op, out)?;
                Self::flatten_filter_expr(right, Some(ast::LogicalOp::And), out)?;
                Ok(())
            }
            ast::Expr::BinOp(left, ast::BinOp::Or, right, _) => {
                Self::flatten_filter_expr(left, logical_op, out)?;
                Self::flatten_filter_expr(right, Some(ast::LogicalOp::Or), out)?;
                Ok(())
            }
            ast::Expr::BinOp(left, op, right, _)
                if matches!(
                    op,
                    ast::BinOp::Eq
                        | ast::BinOp::Ne
                        | ast::BinOp::Lt
                        | ast::BinOp::Gt
                        | ast::BinOp::Le
                        | ast::BinOp::Ge
                ) =>
            {
                let field_name = match left.as_ref() {
                    ast::Expr::Ident(name, _) => *name,
                    _ => return Err("query filter left-hand side must be a field name".into()),
                };
                out.push((logical_op, field_name, *op, *right.clone()));
                Ok(())
            }
            _ => Err("query where clause must be a comparison expression".into()),
        }
    }

    /// Merge multiple where-clause expressions (which AND together) into a
    /// single `ast::StoreFilter`.
    pub(crate) fn merge_where_clauses(
        exprs: &[(ast::Expr, ast::Span)],
    ) -> Result<ast::StoreFilter, String> {
        if exprs.is_empty() {
            return Err("query block requires at least one where clause".into());
        }
        let mut filter = Self::expr_to_store_filter(&exprs[0].0, exprs[0].1)?;
        for (expr, span) in &exprs[1..] {
            let additional = Self::expr_to_store_filter(expr, *span)?;
            // Append as AND conditions
            filter.extra.push((
                ast::LogicalOp::And,
                ast::StoreFilterCond {
                    field: additional.field,
                    op: additional.op,
                    value: additional.value,
                },
            ));
            filter
                .extra
                .extend(additional.extra.into_iter().map(|(lop, c)| (lop, c)));
        }
        Ok(filter)
    }

}
