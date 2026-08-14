use super::super::{Typer, VarInfo};
use crate::ast;
use crate::hir::{self, Ownership};
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn lower_expr_if_expr(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::IfExpr(i) => {
                let result_ty = expected
                    .cloned()
                    .unwrap_or_else(|| self.infer_ctx.fresh_var());
                let hi = self.lower_if_with_tail(i, &result_ty, Some(&result_ty))?;
                let mut ty = self.join_branch_type(&hi.then).unwrap_or(Type::Void);
                if ty != Type::Void {
                    let r = self.infer_ctx.unify_at(
                        &result_ty,
                        &ty,
                        i.span,
                        "if-expression then branch",
                    );
                    self.collect_unify_error(r);
                }
                let mut rest: Vec<&hir::Block> = hi.elifs.iter().map(|(_, b)| b).collect();
                if let Some(ref els) = hi.els {
                    rest.push(els);
                }
                let arms: Vec<Option<Type>> =
                    rest.iter().map(|b| self.join_branch_type(b)).collect();
                for arm_ty in arms {
                    match (&ty, arm_ty) {
                        (Type::Void, Some(a)) => ty = a,
                        (j, Some(a)) => {
                            let j = j.clone();
                            self.unify_join_arm(&j, &a, i.span, "if");
                        }
                        _ => {}
                    }
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::IfExpr(Box::new(hi)),
                    ty,
                    span: i.span,
                })
            }
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
            ast::Expr::Pipe(left, right, extra_args, span) => {
                self.lower_pipe(left, right, extra_args, *span)
            }
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
            ast::Expr::Block(stmts, span) => {
                self.push_scope();
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
            }
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
            ast::Expr::ListComp(body_expr, var, iter_expr, iter_end, cond, span) => {
                let hiter = self.lower_expr(iter_expr)?;

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

                let ty = Type::Vec(Box::new(hbody.ty.clone()));
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
            }
            _ => unreachable!(),
        }
    }

    #[allow(clippy::if_same_then_else)]
    pub(in crate::typer) fn lower_expr_query(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Query(source, clauses, span) => {
                let store_name = match source.as_ref() {
                    ast::Expr::Ident(name, _) => *name,
                    _ => return Err("query block source must be a store name".into()),
                };
                let schema = self
                    .store_schemas
                    .get(&store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();

                let parts = Self::partition_query_clauses(clauses)?;
                let where_exprs = parts.where_exprs;
                let has_delete = parts.has_delete;
                let sets = parts.sets;

                if let Some(key) = parts.group {
                    if has_delete || !sets.is_empty() {
                        return Err("a `group` query cannot combine with `delete` or `set`".into());
                    }
                    let hfilter = if where_exprs.is_empty() {
                        None
                    } else {
                        let ast_filter = Self::merge_where_clauses(&where_exprs)?;
                        Some(self.lower_store_filter(&ast_filter, &schema, &store_name.as_str())?)
                    };
                    return self.lower_query_group(
                        store_name,
                        &schema,
                        key,
                        parts.select,
                        hfilter,
                        *span,
                    );
                }
                if parts.select.is_some() {
                    return Err("`select` requires a `group` clause; bind fields from the \
                                query result instead"
                        .into());
                }

                if where_exprs.is_empty() {
                    return Err("query block requires at least one where clause".into());
                }

                let ast_filter = Self::merge_where_clauses(&where_exprs)?;
                let hfilter =
                    self.lower_store_filter(&ast_filter, &schema, &store_name.as_str())?;

                if has_delete {
                    Ok(hir::Expr {
                        kind: hir::ExprKind::Void,
                        ty: Type::Void,
                        span: *span,
                    })
                } else if !sets.is_empty() {
                    Ok(hir::Expr {
                        kind: hir::ExprKind::Void,
                        ty: Type::Void,
                        span: *span,
                    })
                } else {
                    let struct_name = Symbol::intern(&format!("__store_{store_name}"));
                    Ok(hir::Expr {
                        kind: hir::ExprKind::StoreQuery(store_name, Box::new(hfilter)),
                        ty: Type::Struct(struct_name, vec![]),
                        span: *span,
                    })
                }
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn partition_query_clauses(
        clauses: &[ast::QueryClause],
    ) -> Result<QueryParts, String> {
        let mut parts = QueryParts {
            where_exprs: Vec::new(),
            has_delete: false,
            sets: Vec::new(),
            group: None,
            select: None,
        };
        for clause in clauses {
            match clause {
                ast::QueryClause::Where(expr, cspan) => {
                    parts.where_exprs.push((expr.clone(), *cspan));
                }
                ast::QueryClause::Delete(_) => {
                    parts.has_delete = true;
                }
                ast::QueryClause::Set(field, val, _) => {
                    parts.sets.push((*field, val.clone()));
                }
                ast::QueryClause::Group(field, _) => {
                    if parts.group.is_some() {
                        return Err("a query block may have at most one `group` clause".into());
                    }
                    parts.group = Some(*field);
                }
                ast::QueryClause::Select(items, _) => {
                    if parts.select.is_some() {
                        return Err("a query block may have at most one `select` clause".into());
                    }
                    parts.select = Some(items.clone());
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
        Ok(parts)
    }

    fn lower_query_group(
        &mut self,
        store_name: Symbol,
        schema: &[(Symbol, Type)],
        key: Symbol,
        select: Option<Vec<ast::SelectItem>>,
        hfilter: Option<hir::StoreFilter>,
        span: ast::Span,
    ) -> Result<hir::Expr, String> {
        let field_ty = |fld: &Symbol| {
            schema
                .iter()
                .find(|(n, _)| n == fld)
                .map(|(_, t)| t.clone())
        };
        let key_ty = field_ty(&key)
            .ok_or_else(|| format!("group: unknown field '{key}' in store '{store_name}'"))?;

        let items = select.unwrap_or_else(|| {
            vec![
                ast::SelectItem::Field(key, span),
                ast::SelectItem::Agg(Symbol::intern("count"), None, span),
            ]
        });
        match items.first() {
            Some(ast::SelectItem::Field(f, _)) if *f == key => {}
            _ => {
                return Err(format!(
                    "the first `select` item must be the group key `{key}`"
                ));
            }
        }
        if items.len() < 2 {
            return Err(
                "`select` must include at least one aggregate: `count`, `sum(f)`, \
                        `avg(f)`, `min(f)`, or `max(f)`"
                    .into(),
            );
        }

        let mut aggs: Vec<(hir::GroupAgg, Option<Symbol>)> = Vec::new();
        let mut elem_tys: Vec<Type> = vec![key_ty];
        for item in &items[1..] {
            match item {
                ast::SelectItem::Field(f, _) => {
                    return Err(format!(
                        "`{f}` is not the group key; aggregate it (e.g. `sum({f})`) or \
                         group by it"
                    ));
                }
                ast::SelectItem::Agg(name, arg, _) => {
                    let agg = match &*name.as_str() {
                        "count" => hir::GroupAgg::Count,
                        "sum" => hir::GroupAgg::Sum,
                        "avg" => hir::GroupAgg::Avg,
                        "min" => hir::GroupAgg::Min,
                        "max" => hir::GroupAgg::Max,
                        other => {
                            return Err(format!(
                                "unknown aggregate `{other}`; expected `count`, `sum`, \
                                 `avg`, `min`, or `max`"
                            ));
                        }
                    };
                    if agg == hir::GroupAgg::Count {
                        if arg.is_some() {
                            return Err("`count` in a select takes no field argument".into());
                        }
                        aggs.push((agg, None));
                        elem_tys.push(Type::I64);
                        continue;
                    }
                    let vf = arg.ok_or_else(|| {
                        format!("`{name}` requires a field argument, e.g. `{name}(amount)`")
                    })?;
                    let vty = field_ty(&vf).ok_or_else(|| {
                        format!("{name}: unknown field '{vf}' in store '{store_name}'")
                    })?;
                    if !matches!(vty, Type::I64 | Type::F64 | Type::F32) {
                        return Err(format!(
                            "`{name}({vf})` requires a numeric field; `{vf}` is {vty}"
                        ));
                    }
                    let out_ty = if agg == hir::GroupAgg::Avg {
                        Type::F64
                    } else {
                        match vty {
                            Type::F64 | Type::F32 => Type::F64,
                            _ => Type::I64,
                        }
                    };
                    aggs.push((agg, Some(vf)));
                    elem_tys.push(out_ty);
                }
            }
        }

        Ok(hir::Expr {
            kind: hir::ExprKind::StoreQueryGroup(store_name, key, aggs, hfilter.map(Box::new)),
            ty: Type::Vec(Box::new(Type::Tuple(elem_tys))),
            span,
        })
    }
}

pub(in crate::typer) struct QueryParts {
    pub(in crate::typer) where_exprs: Vec<(ast::Expr, ast::Span)>,
    pub(in crate::typer) has_delete: bool,
    pub(in crate::typer) sets: Vec<(Symbol, ast::Expr)>,
    pub(in crate::typer) group: Option<Symbol>,
    pub(in crate::typer) select: Option<Vec<ast::SelectItem>>,
}
