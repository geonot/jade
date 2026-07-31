use super::super::Typer;
use crate::ast;
use crate::hir;
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn lower_store_insert_values(
        &mut self,
        store: &Symbol,
        values: &[ast::FieldInit],
    ) -> Result<Vec<hir::Expr>, String> {
        let schema = self
            .store_schemas
            .get(store)
            .ok_or_else(|| format!("unknown store '{store}'"))?
            .clone();

        let builtin_names = [
            "sid",
            "uuid",
            "hash",
            "created",
            "updated",
            "deleted",
            "__version",
        ];
        let user_schema: Vec<_> = schema
            .iter()
            .filter(|(n, _)| !builtin_names.iter().any(|b| *n == *b))
            .cloned()
            .collect();

        let any_named = values.iter().any(|fi| fi.name.is_some());
        let all_named = values.iter().all(|fi| fi.name.is_some());

        if any_named && !all_named {
            return Err(format!(
                "store '{store}': cannot mix named and positional \
                 fields in a single insert"
            ));
        }

        if all_named && !values.is_empty() {
            let mut hvalues = Vec::with_capacity(user_schema.len());
            for (fname, fty) in &user_schema {
                let fi = values
                    .iter()
                    .find(|fi| fi.name.as_ref() == Some(fname))
                    .ok_or_else(|| format!("store '{store}' insert: missing field '{fname}'"))?;
                hvalues.push(self.lower_expr_expected(&fi.value, Some(fty))?);
            }

            let mut seen = std::collections::HashSet::new();
            for fi in values {
                let n = fi.name.as_ref().unwrap();
                if !user_schema.iter().any(|(sn, _)| sn == n) {
                    return Err(format!("store '{store}' has no field '{n}'"));
                }
                if !seen.insert(*n) {
                    return Err(format!(
                        "store '{store}' insert: field '{n}' \
                         specified twice"
                    ));
                }
            }
            return Ok(hvalues);
        }

        if values.len() != user_schema.len() {
            return Err(format!(
                "store '{store}' has {} fields but {} values given",
                user_schema.len(),
                values.len()
            ));
        }
        let mut hvalues = Vec::new();
        for (fi, (_fname, fty)) in values.iter().zip(user_schema.iter()) {
            hvalues.push(self.lower_expr_expected(&fi.value, Some(fty))?);
        }
        Ok(hvalues)
    }

    fn store_result_subject(
        &mut self,
        call_name: String,
        args: Vec<hir::Expr>,
        err_variants: &[(&str, i64)],
        span: ast::Span,
    ) -> Result<hir::Expr, String> {
        let store_err = Symbol::intern("StoreError");
        let result_enum = {
            let mut m = std::collections::HashMap::new();
            m.insert(Symbol::intern("T"), Type::I64);
            m.insert(Symbol::intern("E"), Type::Enum(store_err));
            self.monomorphize_enum("Result", &m)?
        };
        let result_ty = Type::Enum(result_enum);
        self.current_fn_error_types.insert(store_err);

        let status_id = self.fresh_id();
        let status_var = hir::Expr {
            kind: hir::ExprKind::Var(status_id, "__st".into()),
            ty: Type::I64,
            span,
        };
        let status_bind = hir::Stmt::Bind(hir::Bind {
            def_id: status_id,
            name: "__st".into(),
            value: hir::Expr {
                kind: hir::ExprKind::Call(hir::DefId(0), Symbol::intern(&call_name), args),
                ty: Type::I64,
                span,
            },
            ty: Type::I64,
            ownership: crate::typer::Ownership::Owned,
            atomic: false,
            access_mod: None,
            span,
        });

        let ok_tag = self
            .enums
            .get(&result_enum)
            .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == "Ok"))
            .unwrap_or(0) as u32;
        let err_tag = self
            .enums
            .get(&result_enum)
            .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == "Err"))
            .unwrap_or(1) as u32;

        let mk_int = |v: i64| hir::Expr {
            kind: hir::ExprKind::Int(v),
            ty: Type::I64,
            span,
        };
        let mk_err_value = |this: &Typer, vname: &str| -> hir::Expr {
            let tag = this
                .enums
                .get(&store_err)
                .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == vname))
                .unwrap_or(0) as u32;
            let ctor = hir::Expr {
                kind: hir::ExprKind::VariantCtor(store_err, vname.into(), tag, vec![]),
                ty: Type::Enum(store_err),
                span,
            };
            hir::Expr {
                kind: hir::ExprKind::VariantCtor(
                    result_enum,
                    "Err".into(),
                    err_tag,
                    vec![hir::FieldInit {
                        name: None,
                        value: ctor,
                    }],
                ),
                ty: result_ty.clone(),
                span,
            }
        };

        let ok_value = hir::Expr {
            kind: hir::ExprKind::VariantCtor(
                result_enum,
                "Ok".into(),
                ok_tag,
                vec![hir::FieldInit {
                    name: None,
                    value: status_var.clone(),
                }],
            ),
            ty: result_ty.clone(),
            span,
        };

        let (last_variant, rest) = err_variants.split_last().unwrap();
        let mut else_expr = mk_err_value(self, last_variant.0);
        for (vname, code) in rest.iter().rev() {
            let cond = hir::Expr {
                kind: hir::ExprKind::BinOp(
                    Box::new(status_var.clone()),
                    ast::BinOp::Eq,
                    Box::new(mk_int(*code)),
                ),
                ty: Type::Bool,
                span,
            };
            else_expr = hir::Expr {
                kind: hir::ExprKind::Ternary(
                    Box::new(cond),
                    Box::new(mk_err_value(self, vname)),
                    Box::new(else_expr),
                ),
                ty: result_ty.clone(),
                span,
            };
        }

        let is_ok = hir::Expr {
            kind: hir::ExprKind::BinOp(
                Box::new(status_var.clone()),
                ast::BinOp::Ge,
                Box::new(mk_int(0)),
            ),
            ty: Type::Bool,
            span,
        };

        let ternary = hir::Expr {
            kind: hir::ExprKind::Ternary(Box::new(is_ok), Box::new(ok_value), Box::new(else_expr)),
            ty: result_ty.clone(),
            span,
        };

        Ok(hir::Expr {
            kind: hir::ExprKind::Block(vec![status_bind, hir::Stmt::Expr(ternary)]),
            ty: result_ty,
            span,
        })
    }

    pub(in crate::typer) fn lower_expr_store_insert(
        &mut self,
        store: &Symbol,
        values: &[ast::FieldInit],
        span: ast::Span,
    ) -> Result<hir::Expr, String> {
        let hvalues = self.lower_store_insert_values(store, values)?;
        self.store_result_subject(
            format!("__store_insert_status_{store}"),
            hvalues,
            &[("Duplicate", -1), ("Constraint", -2)],
            span,
        )
    }

    pub(in crate::typer) fn lower_expr_store_update(
        &mut self,
        store: &Symbol,
        assignments: &[(Symbol, ast::Expr)],
        filter: &ast::StoreFilter,
        span: ast::Span,
    ) -> Result<hir::Expr, String> {
        let schema = self
            .store_schemas
            .get(store)
            .ok_or_else(|| format!("unknown store '{store}'"))?
            .clone();
        let hfilter = self.lower_store_filter(filter, &schema, &store.as_str())?;
        let mut hassigns = Vec::new();
        for (fname, fval) in assignments {
            if let Some((_, fty)) = schema.iter().find(|(n, _)| n == fname) {
                hassigns.push((*fname, self.lower_expr_expected(fval, Some(fty))?));
            } else {
                return Err(format!("store '{store}' has no field '{fname}'"));
            }
        }

        let mut args = vec![hfilter.value.clone()];
        for (_, cond) in &hfilter.extra {
            args.push(cond.value.clone());
        }
        let field_names: Vec<Symbol> = hassigns.iter().map(|(n, _)| *n).collect();
        args.extend(hassigns.into_iter().map(|(_, e)| e));

        let encoded = crate::hir::encode_store_set_call(*store, &hfilter, &field_names).replacen(
            "__store_set_",
            "__store_set_status_",
            1,
        );
        self.store_result_subject(encoded, args, &[("Missing", -1)], span)
    }

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
                let hfilter_exists =
                    self.lower_store_filter(filter, &schema, &store.as_str())?;

                /* D3 (task 8-25): a query that can match nothing has type
                 * `Result of <row>, StoreError` — reading a field of a miss
                 * is no longer expressible. It used to fabricate a zero row
                 * (`name=[] age=0`) indistinguishable from real data. The
                 * quaternary handles it with no ceremony
                 * (`users where … ? $.field ! fallback`), and `!! err`
                 * propagates inside a fallible function. Desugar:
                 * `exists ? Ok(row-read) ! Err(Missing)`. */
                let span = *span;
                let row_ty = Type::Row(*store);
                let store_err = Symbol::intern("StoreError");
                let result_enum = {
                    let mut m = std::collections::HashMap::new();
                    m.insert(Symbol::intern("T"), row_ty.clone());
                    m.insert(Symbol::intern("E"), Type::Enum(store_err));
                    self.monomorphize_enum("Result", &m)?
                };
                let result_ty = Type::Enum(result_enum);
                self.current_fn_error_types.insert(store_err);

                let ok_tag = self
                    .enums
                    .get(&result_enum)
                    .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == "Ok"))
                    .unwrap_or(0) as u32;
                let err_tag = self
                    .enums
                    .get(&result_enum)
                    .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == "Err"))
                    .unwrap_or(1) as u32;
                let missing_tag = self
                    .enums
                    .get(&store_err)
                    .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == "Missing"))
                    .unwrap_or(0) as u32;

                let exists = hir::Expr {
                    kind: hir::ExprKind::StoreExists(*store, Box::new(hfilter_exists)),
                    ty: Type::Bool,
                    span,
                };
                let row_read = hir::Expr {
                    kind: hir::ExprKind::StoreQuery(*store, Box::new(hfilter)),
                    ty: row_ty,
                    span,
                };
                let ok_val = hir::Expr {
                    kind: hir::ExprKind::VariantCtor(
                        result_enum,
                        "Ok".into(),
                        ok_tag,
                        vec![hir::FieldInit {
                            name: None,
                            value: row_read,
                        }],
                    ),
                    ty: result_ty.clone(),
                    span,
                };
                let missing = hir::Expr {
                    kind: hir::ExprKind::VariantCtor(
                        store_err,
                        "Missing".into(),
                        missing_tag,
                        vec![],
                    ),
                    ty: Type::Enum(store_err),
                    span,
                };
                let err_val = hir::Expr {
                    kind: hir::ExprKind::VariantCtor(
                        result_enum,
                        "Err".into(),
                        err_tag,
                        vec![hir::FieldInit {
                            name: None,
                            value: missing,
                        }],
                    ),
                    ty: result_ty.clone(),
                    span,
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Ternary(
                        Box::new(exists),
                        Box::new(ok_val),
                        Box::new(err_val),
                    ),
                    ty: result_ty,
                    span,
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
                        kind: hir::ExprKind::ViewCount(*store, Box::new(hfilter)),
                        ty: Type::I64,
                        span: *span,
                    })
                } else {
                    Ok(hir::Expr {
                        kind: hir::ExprKind::StoreCount(*store),
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
                /* D3 (task 8-25): `all <store>` is a first-class row set —
                 * a Vec of the store's record struct. The old Ptr type had
                 * no length, so iteration walked garbage. */
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreAll(*store),
                    ty: Type::Vec(Box::new(Type::Struct(struct_name, vec![]))),
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

                let _struct_name = Symbol::intern(&format!("__store_{store}"));
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreGet(*store, Box::new(hkey)),
                    ty: Type::Row(*store),
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

                let _struct_name = Symbol::intern(&format!("__store_{store}"));
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreFirst(*store, Box::new(hfilter)),
                    ty: Type::Row(*store),
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
                    kind: hir::ExprKind::StoreExists(*store, Box::new(hfilter)),
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
                    kind: hir::ExprKind::StoreDistinct(*store, *field),
                    ty: Type::Vec(Box::new(Type::String)),
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }
}
