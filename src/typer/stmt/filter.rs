use crate::ast;
use crate::hir::{self};
use crate::intern::Symbol;
use crate::types::Type;

use super::super::Typer;

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
        if !matches!(filter.pred, ast::FilterPred::Cmp) && !matches!(field_ty, Some(Type::String)) {
            return Err(format!(
                "store '{store}' field '{}' must be a String for text predicates",
                filter.field
            ));
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
            if !matches!(cond.pred, ast::FilterPred::Cmp)
                && !matches!(cond_field_ty, Some(Type::String))
            {
                return Err(format!(
                    "store '{store}' field '{}' must be a String for text predicates",
                    cond.field
                ));
            }
            let hv = self.lower_expr_expected(&cond.value, cond_field_ty)?;
            hextra.push((
                *lop,
                hir::StoreFilterCond {
                    field: cond.field,
                    op: cond.op,
                    value: hv,
                    pred: cond.pred,
                },
            ));
        }
        Ok(hir::StoreFilter {
            field: filter.field,
            op: filter.op,
            value: hvalue,
            span: filter.span,
            extra: hextra,
            pred: filter.pred,
        })
    }

    pub(crate) fn expr_to_store_filter(
        expr: &ast::Expr,
        span: ast::Span,
    ) -> Result<ast::StoreFilter, String> {
        let mut conds: Vec<FlatCond> = Vec::new();
        Self::flatten_filter_expr(expr, None, &mut conds, &mut 0)?;
        Self::build_store_filter(conds, span)
    }

    fn build_store_filter(
        mut conds: Vec<FlatCond>,
        span: ast::Span,
    ) -> Result<ast::StoreFilter, String> {
        if conds.is_empty() {
            return Err("query where clause must be a comparison".into());
        }
        let groups: std::collections::BTreeSet<u32> =
            conds.iter().filter_map(|c| c.in_group).collect();
        if !groups.is_empty() {
            if groups.len() > 1 {
                return Err(
                    "a filter may contain at most one `in [..]` clause; a second one \
                     cannot be expressed in a flat and/or chain"
                        .into(),
                );
            }
            if conds
                .iter()
                .any(|c| c.in_group.is_none() && c.lop == Some(ast::LogicalOp::Or))
            {
                return Err(
                    "mixing `or` with an `in [..]` clause is ambiguous; split the query \
                     or rewrite the `in` as explicit `or` comparisons"
                        .into(),
                );
            }
            conds.sort_by_key(|c| c.in_group.is_none());
        }
        let head = conds.remove(0);
        let head_group = head.in_group;
        let extra = conds
            .into_iter()
            .map(|c| {
                let lop = if c.in_group.is_some() && c.in_group == head_group {
                    ast::LogicalOp::Or
                } else if c.in_group.is_some() {
                    c.lop.unwrap_or(ast::LogicalOp::Or)
                } else if head_group.is_some() {
                    ast::LogicalOp::And
                } else {
                    c.lop.unwrap_or(ast::LogicalOp::And)
                };
                (
                    lop,
                    ast::StoreFilterCond {
                        field: c.field,
                        op: c.op,
                        value: c.value,
                        pred: c.pred,
                    },
                )
            })
            .collect();
        Ok(ast::StoreFilter {
            field: head.field,
            op: head.op,
            value: head.value,
            span,
            extra,
            pred: head.pred,
        })
    }

    fn flatten_filter_expr(
        expr: &ast::Expr,
        logical_op: Option<ast::LogicalOp>,
        out: &mut Vec<FlatCond>,
        next_group: &mut u32,
    ) -> Result<(), String> {
        match expr {
            ast::Expr::BinOp(left, ast::BinOp::And, right, _) => {
                Self::flatten_filter_expr(left, logical_op, out, next_group)?;
                Self::flatten_filter_expr(right, Some(ast::LogicalOp::And), out, next_group)?;
                Ok(())
            }
            ast::Expr::BinOp(left, ast::BinOp::Or, right, _) => {
                Self::flatten_filter_expr(left, logical_op, out, next_group)?;
                Self::flatten_filter_expr(right, Some(ast::LogicalOp::Or), out, next_group)?;
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
                out.push(FlatCond {
                    lop: logical_op,
                    field: field_name,
                    op: *op,
                    value: *right.clone(),
                    pred: ast::FilterPred::Cmp,
                    in_group: None,
                });
                Ok(())
            }
            ast::Expr::Method(recv, mname, margs, _)
                if mname.as_str() == "contains"
                    && in_list_values(recv).is_some()
                    && margs.len() == 1 =>
            {
                let field_name = match &margs[0] {
                    ast::Expr::Ident(name, _) => *name,
                    _ => {
                        return Err(
                            "`in [..]` in a query filter requires a field name on the left".into(),
                        );
                    }
                };
                let values = in_list_values(recv).unwrap_or_default();
                if values.is_empty() {
                    return Err("`in [..]` in a query filter requires at least one value".into());
                }
                if values.len() > 1 && logical_op == Some(ast::LogicalOp::Or) {
                    return Err(
                        "mixing `or` with an `in [..]` clause is ambiguous; split the query \
                         or rewrite the `in` as explicit `or` comparisons"
                            .into(),
                    );
                }
                let gid = *next_group;
                *next_group += 1;
                for (i, v) in values.iter().enumerate() {
                    out.push(FlatCond {
                        lop: if i == 0 {
                            logical_op
                        } else {
                            Some(ast::LogicalOp::Or)
                        },
                        field: field_name,
                        op: ast::BinOp::Eq,
                        value: v.clone(),
                        pred: ast::FilterPred::Cmp,
                        in_group: if values.len() > 1 { Some(gid) } else { None },
                    });
                }
                Ok(())
            }
            ast::Expr::Method(recv, mname, margs, _) if margs.len() == 1 => {
                let pred = match mname.as_str().as_str() {
                    "contains" => ast::FilterPred::Contains,
                    "starts_with" => ast::FilterPred::StartsWith,
                    "ends_with" => ast::FilterPred::EndsWith,
                    "iequals" => ast::FilterPred::IEq,
                    "icontains" => ast::FilterPred::IContains,
                    "istarts_with" => ast::FilterPred::IStartsWith,
                    "iends_with" => ast::FilterPred::IEndsWith,
                    _ => return Err("query where clause must be a comparison expression".into()),
                };
                let field_name = match recv.as_ref() {
                    ast::Expr::Ident(name, _) => *name,
                    _ => return Err("query filter left-hand side must be a field name".into()),
                };
                out.push(FlatCond {
                    lop: logical_op,
                    field: field_name,
                    op: ast::BinOp::Eq,
                    value: margs[0].clone(),
                    pred,
                    in_group: None,
                });
                Ok(())
            }
            _ => Err("query where clause must be a comparison expression".into()),
        }
    }

    pub(crate) fn merge_where_clauses(
        exprs: &[(ast::Expr, ast::Span)],
    ) -> Result<ast::StoreFilter, String> {
        if exprs.is_empty() {
            return Err("query block requires at least one where clause".into());
        }
        let mut conds: Vec<FlatCond> = Vec::new();
        let mut next_group = 0u32;
        for (i, (expr, _)) in exprs.iter().enumerate() {
            let start = conds.len();
            Self::flatten_filter_expr(expr, None, &mut conds, &mut next_group)?;
            if i > 0
                && let Some(first) = conds.get_mut(start)
                && first.lop.is_none()
            {
                first.lop = Some(ast::LogicalOp::And);
            }
        }
        Self::build_store_filter(conds, exprs[0].1)
    }
}

struct FlatCond {
    lop: Option<ast::LogicalOp>,
    field: Symbol,
    op: ast::BinOp,
    value: ast::Expr,
    pred: ast::FilterPred,
    in_group: Option<u32>,
}

fn in_list_values(e: &ast::Expr) -> Option<&[ast::Expr]> {
    match e {
        ast::Expr::Array(values, _) => Some(values),
        ast::Expr::Call(callee, values, _) if matches!(callee.as_ref(), ast::Expr::Ident(n, _) if n.as_str() == "vector") => {
            Some(values)
        }
        _ => None,
    }
}
