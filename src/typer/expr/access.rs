use super::super::Typer;
use crate::ast;
use crate::hir;
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn lower_expr_call(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        match expr {
            ast::Expr::Call(callee, args, span) => {
                if let ast::Expr::Ident(n, _) = callee.as_ref()
                    && n.as_str() == "__err_raise"
                    && args.len() == 1
                {
                    return self.lower_err_raise_expr(&args[0], *span, expected);
                }
                if let ast::Expr::Field(obj, field, fspan) = callee.as_ref()
                    && let ast::Expr::Ident(ref ename, _) = **obj
                    && self.find_var(&ename.as_str()).is_none()
                    && self.is_enum_variant_of(ename, field)
                {
                    let bare = ast::Expr::Call(
                        Box::new(ast::Expr::Ident(*field, *fspan)),
                        args.clone(),
                        *span,
                    );
                    return self.lower_expr_expected(&bare, expected);
                }
                if let ast::Expr::OfCall(inner, type_arg_expr, _) = callee.as_ref()
                    && let ast::Expr::Ident(ctor_name, _) = inner.as_ref()
                {
                    let is_struct_ctor = self.generic_types.contains_key(ctor_name)
                        || self.structs.contains_key(ctor_name);
                    let is_variant_ctor = self.variant_tags.contains_key(ctor_name);
                    if (is_struct_ctor || is_variant_ctor)
                        && let Some(tys) = self.expr_to_type_args(type_arg_expr)
                    {
                        let inits: Vec<ast::FieldInit> = args
                            .iter()
                            .map(|a| ast::FieldInit {
                                name: None,
                                value: a.clone(),
                            })
                            .collect();
                        let result = self.lower_struct_or_variant_with_typeargs(
                            &ctor_name.as_str(),
                            &inits,
                            *span,
                            &tys,
                        )?;
                        if let Some(exp) = expected {
                            self.unify_call_result(exp, &result.ty, *span, "call result");
                        }
                        return Ok(result);
                    }
                }

                if let ast::Expr::Ident(ctor_name, _) = callee.as_ref() {
                    let is_struct = self.generic_types.contains_key(ctor_name)
                        || self.structs.contains_key(ctor_name);
                    let is_variant = self.variant_tags.contains_key(ctor_name);
                    let is_value = self.find_var(&ctor_name.as_str()).is_some()
                        || self.fns.contains_key(ctor_name)
                        || self.generic_fns.contains_key(ctor_name)
                        || self.inferable_fns.contains_key(ctor_name);
                    if (is_struct || is_variant) && !is_value {
                        let inits: Vec<ast::FieldInit> = args
                            .iter()
                            .map(|a| ast::FieldInit {
                                name: None,
                                value: a.clone(),
                            })
                            .collect();
                        if is_variant
                            && let Some(r) = self.try_lower_variant_with_expected(
                                &ctor_name.as_str(),
                                &inits,
                                *span,
                                expected,
                            )?
                        {
                            if let Some(exp) = expected {
                                self.unify_call_result(exp, &r.ty, *span, "call result");
                            }
                            return Ok(r);
                        }
                        let result =
                            self.lower_struct_or_variant(&ctor_name.as_str(), &inits, *span)?;
                        if let Some(exp) = expected {
                            self.unify_call_result(exp, &result.ty, *span, "call result");
                        }
                        return Ok(result);
                    }
                }

                let has_placeholder = self.dollar_stack.is_empty()
                    && args.iter().any(|a| matches!(a, ast::Expr::Placeholder(_)));
                if has_placeholder {
                    let param = ast::Param {
                        name: "__ph".into(),
                        ty: None,
                        default: None,
                        literal: None,
                        access_mod: None,
                        span: *span,
                    };
                    let new_args: Vec<ast::Expr> = args
                        .iter()
                        .map(|a| {
                            if matches!(a, ast::Expr::Placeholder(_)) {
                                ast::Expr::Ident("__ph".into(), a.span())
                            } else {
                                a.clone()
                            }
                        })
                        .collect();
                    let call = ast::Expr::Call(callee.clone(), new_args, *span);
                    let lambda =
                        ast::Expr::Lambda(vec![param], None, vec![ast::Stmt::Expr(call)], *span);
                    return self.lower_expr_expected(&lambda, expected);
                }
                let result = self.lower_call(callee, args, *span)?;
                if let Some(exp) = expected {
                    self.unify_call_result(exp, &result.ty, *span, "call result");
                }
                Ok(result)
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_method(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Method(obj, method, args, span) => {
                if let ast::Expr::Ident(ref name, _) = **obj {
                    /* D4 (task 8-15): imports are explicit. A qualified call
                     * on a name that is not in scope but names an importable
                     * module (std or a sibling file) used to be silently
                     * auto-imported; now it is an error naming the fix. */
                    if !self.modules.contains(name)
                        && self.find_var(&name.as_str()).is_none()
                        && !self.structs.contains_key(name)
                        && !self.enums.contains_key(name)
                        && !self.actors.contains_key(name)
                        && self.importable_module_exists(&name.as_str())
                    {
                        return Err(format!(
                            "{}: module `{}` is used here but not imported; add \
                             `use {}` at the top of the file (imports are explicit \
                             — nothing is pulled in by directory or by name)",
                            span.loc(),
                            name,
                            name,
                        ));
                    }
                    if self.modules.contains(name) && self.find_var(&name.as_str()).is_none() {
                        let qualified_name = Symbol::intern(&format!("{}_{}", name, method));

                        if !self.fns.contains_key(&qualified_name)
                            && !self.inferable_fns.contains_key(&qualified_name)
                            && !self.generic_fns.contains_key(&qualified_name)
                            && let Some((id, ptys, ret)) = self.externs.get(method).cloned()
                        {
                            let mut hargs = Vec::new();
                            for (i, arg) in args.iter().enumerate() {
                                let expected_ty = ptys.get(i);
                                hargs.push(self.lower_expr_expected(arg, expected_ty)?);
                            }
                            for (i, harg) in hargs.iter().enumerate() {
                                if let Some(pty) = ptys.get(i) {
                                    let _ =
                                        self.infer_ctx.unify_at(pty, &harg.ty, *span, "extern arg");
                                }
                            }
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::Call(id, *method, hargs),
                                ty: ret,
                                span: *span,
                            });
                        }
                        let callee = ast::Expr::Ident(qualified_name, *span);
                        let result = self.lower_call(&callee, args, *span)?;
                        if let Some(exp) = expected {
                            self.unify_call_result(exp, &result.ty, *span, "call result");
                        }
                        return Ok(result);
                    }

                    if name == "extern" {
                        if let Some((id, ptys, ret)) = self.externs.get(method).cloned() {
                            let mut hargs = Vec::new();
                            for (i, arg) in args.iter().enumerate() {
                                let expected_ty = ptys.get(i);
                                hargs.push(self.lower_expr_expected(arg, expected_ty)?);
                            }
                            for (i, harg) in hargs.iter().enumerate() {
                                if let Some(pty) = ptys.get(i) {
                                    let _ =
                                        self.infer_ctx.unify_at(pty, &harg.ty, *span, "extern arg");
                                }
                            }
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::Call(id, *method, hargs),
                                ty: ret,
                                span: *span,
                            });
                        }

                        let callee = ast::Expr::Ident(*method, *span);
                        let result = self.lower_call(&callee, args, *span)?;
                        if let Some(exp) = expected {
                            self.unify_call_result(exp, &result.ty, *span, "call result");
                        }
                        return Ok(result);
                    }
                }
                let result = self.lower_method_call(obj, &method.as_str(), args, *span)?;
                if let Some(exp) = expected {
                    self.unify_call_result(exp, &result.ty, *span, "method call result");
                }
                Ok(result)
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_err_raise_expr(
        &mut self,
        variant: &ast::Expr,
        span: ast::Span,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let err_val = self.lower_expr(variant)?;
        let prop = self.propagate_err_value(err_val, span)?;
        let ty = expected.cloned().unwrap_or(Type::Void);
        Ok(hir::Expr {
            kind: hir::ExprKind::Block(vec![
                prop,
                hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::Unreachable,
                    ty: ty.clone(),
                    span,
                }),
            ]),
            ty,
            span,
        })
    }

    pub(in crate::typer) fn is_enum_variant_of(
        &self,
        enum_name: &Symbol,
        variant: &Symbol,
    ) -> bool {
        if !self.enums.contains_key(enum_name) && !self.generic_enums.contains_key(enum_name) {
            return false;
        }
        match self.variant_tags.get(variant) {
            Some((en, _)) => en == enum_name,
            None => self
                .generic_enums
                .get(enum_name)
                .map(|ed| ed.variants.iter().any(|v| &v.name == variant))
                .unwrap_or(false),
        }
    }

    #[allow(clippy::type_complexity, clippy::if_same_then_else)]
    pub(in crate::typer) fn lower_expr_field(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Field(obj, field, span) => {
                if let ast::Expr::Ident(ref name, _) = **obj
                    && self.find_var(&name.as_str()).is_none()
                    && self.is_enum_variant_of(name, field)
                {
                    let callee = ast::Expr::Ident(*field, *span);
                    return self.lower_expr_expected(&callee, expected);
                }
                if let ast::Expr::Ident(ref name, _) = **obj
                    && self.modules.contains(name)
                    && self.find_var(&name.as_str()).is_none()
                {
                    let qualified_name = Symbol::intern(&format!("{}_{}", name, field));
                    let callee = ast::Expr::Ident(qualified_name, *span);
                    return self.lower_expr_expected(&callee, expected);
                }
                let hobj = self.lower_expr(obj)?;

                if let hir::ExprKind::Var(parent_id, parent_name) = &hobj.kind
                    && self.suppress_moved_field_check == 0
                    && let Some(moved) = self.moved_fields.get(parent_id)
                    && moved.contains(field)
                {
                    return Err(format!(
                        "{}: use of moved field `{}.{}`: the field was moved out earlier \
                         (binding an aggregate field moves it, as `take` does); to keep \
                         it, clone at the move site (`copy {}.{}`), or reassign \
                         `{}.{}` before reading it",
                        span.loc(),
                        parent_name,
                        field,
                        parent_name,
                        field,
                        parent_name,
                        field,
                    ));
                }
                let resolved_ty = self.infer_ctx.shallow_resolve(&hobj.ty);

                if let Type::Row(store) = &resolved_ty
                    && let Some(rels) = self.store_relations.get(store)
                    && let Some((_, target, is_has_many)) =
                        rels.iter().find(|(n, _, _)| n == field).copied()
                {
                    if is_has_many {
                        return Err(format!(
                            "{}: `{}.{}` is a has-many relation and is not directly \
                             traversable — query the related store instead, e.g. \
                             `all {} where <foreign-key> eq {}.sid`",
                            span.loc(),
                            store,
                            field,
                            target,
                            store,
                        ));
                    }
                    if !self.store_schemas.contains_key(&target) {
                        return Err(format!(
                            "{}: relation `{}.{}` targets unknown store `{}`",
                            span.loc(),
                            store,
                            field,
                            target,
                        ));
                    }
                    let struct_name = Symbol::intern(&format!("__store_{store}"));
                    let idx = self
                        .structs
                        .get(&struct_name)
                        .and_then(|fs| fs.iter().position(|(n, _)| n == field))
                        .ok_or_else(|| {
                            format!(
                                "{}: relation column `{}.{}` missing from schema",
                                span.loc(),
                                store,
                                field,
                            )
                        })?;
                    let key = hir::Expr {
                        kind: hir::ExprKind::Field(Box::new(hobj.clone()), *field, idx),
                        ty: Type::I64,
                        span: *span,
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::StoreGet(target, Box::new(key)),
                        ty: Type::Row(target),
                        span: *span,
                    });
                }

                if let Type::ActorRef(actor_name) = &resolved_ty
                    && let Some((_, _, handlers)) = self.actors.get(actor_name)
                    && handlers.iter().any(|(n, _, _)| n == field)
                {
                    let call_expr = ast::Expr::Method(obj.clone(), *field, Vec::new(), *span);
                    return self.lower_expr_expected(&call_expr, expected);
                }

                /* D3 (task 8-25): a query is `Result of <row>, StoreError`
                 * — reading a field straight off it is the old fabricated-
                 * zero-row bug wearing a new type. Refuse at compile time
                 * with the two idiomatic forms. */
                if let Type::Enum(ename) = &resolved_ty
                    && ename.as_str().starts_with("Result__G_Row<")
                {
                    return Err(format!(
                        "{}: `.{}` on a query result — a query can miss, so it has \
                         type `Result of <row>, StoreError`; match it:\n    match <query>\n        \
                         Ok(r) ? r.{}\n        Err(e) ? <miss>\nor use the quaternary: \
                         `<store> where <cond> ? $.{} ! <fallback>`",
                        span.loc(),
                        field,
                        field,
                        field,
                    ));
                }

                let peeled_ty = resolved_ty.clone();
                let struct_name = match &peeled_ty {
                    Type::Struct(name, _) => Some(*name),

                    Type::Row(store) => Some(Symbol::intern(&format!("__store_{store}"))),
                    Type::Ptr(inner) => match inner.as_ref() {
                        Type::Struct(name, _) => Some(*name),
                        _ => None,
                    },
                    _ => None,
                };
                let (ty, idx) = if let Some(ref name) = struct_name {
                    if let Some(fields) = self.structs.get(name) {
                        if let Some((i, (_, fty))) =
                            fields.iter().enumerate().find(|(_, (n, _))| n == field)
                        {
                            (self.infer_ctx.shallow_resolve(fty), i)
                        } else {
                            let raw = name.as_str();
                            let display: String =
                                if let Some(stripped) = raw.strip_prefix("__store_") {
                                    format!("{} (query result)", stripped)
                                } else {
                                    raw.to_string()
                                };
                            if raw.starts_with("__store_") && (field == "length" || field == "len")
                            {
                                let store = raw.trim_start_matches("__store_");
                                return Err(format!(
                                    "{}: type '{}' has no field '{}' — \
                                 for the number of matching records use \
                                 `count {} where …`",
                                    span.loc(),
                                    display,
                                    field,
                                    store
                                ));
                            }
                            return Err(format!(
                                "{}: type '{}' has no field '{}'",
                                span.loc(),
                                display,
                                field
                            ));
                        }
                    } else {
                        (Type::I64, 0)
                    }
                } else if matches!(peeled_ty, Type::String)
                    && (field == "length" || field == "byte_count")
                {
                    (Type::I64, 0)
                } else if matches!(&peeled_ty, Type::Vec(_)) && field == "length" {
                    (Type::I64, 0)
                } else if matches!(&peeled_ty, Type::Map(_, _)) && field == "length" {
                    (Type::I64, 0)
                } else if let Type::Tuple(ref tys) = peeled_ty {
                    if let Ok(idx) = field.as_str().parse::<usize>() {
                        if idx < tys.len() {
                            (tys[idx].clone(), idx)
                        } else {
                            return Err(format!(
                                "{}: tuple index {} out of range (tuple has {} elements)",
                                span.loc(),
                                idx,
                                tys.len()
                            ));
                        }
                    } else {
                        (self.infer_ctx.fresh_var(), 0)
                    }
                } else if matches!(resolved_ty, Type::TypeVar(_)) {
                    let var_id = if let Type::TypeVar(v) = resolved_ty {
                        self.infer_ctx.find(v)
                    } else {
                        0
                    };

                    let fty_placeholder = self.infer_ctx.fresh_var();
                    self.field_constraints
                        .entry(var_id)
                        .or_default()
                        .push((*field, fty_placeholder.clone()));

                    let all_required_fields: Vec<(Symbol, Type)> = self
                        .field_constraints
                        .get(&var_id)
                        .cloned()
                        .unwrap_or_default();

                    let candidates: Vec<(Symbol, Vec<(Symbol, Type, usize)>)> = self
                        .structs
                        .iter()
                        .filter_map(|(sname, fields)| {
                            let mut matched = Vec::new();
                            for (req_name, _) in &all_required_fields {
                                if let Some((idx, (_, fty))) = fields
                                    .iter()
                                    .enumerate()
                                    .find(|(_, (fname, _))| fname == req_name)
                                {
                                    matched.push((*req_name, fty.clone(), idx));
                                } else {
                                    return None;
                                }
                            }
                            Some((*sname, matched))
                        })
                        .collect();

                    if candidates.len() == 1 {
                        let (sname, matched_fields) = &candidates[0];
                        let struct_ty = Type::Struct(*sname, vec![]);
                        let _ = self.infer_ctx.unify_at(
                            &resolved_ty,
                            &struct_ty,
                            *span,
                            "field access implies struct type",
                        );
                        for (req_name, req_ty) in &all_required_fields {
                            if let Some((_, actual_ty, _)) =
                                matched_fields.iter().find(|(n, _, _)| n == req_name)
                            {
                                let actual_resolved = self.infer_ctx.shallow_resolve(actual_ty);
                                let _ = self.infer_ctx.unify_at(
                                    req_ty,
                                    &actual_resolved,
                                    *span,
                                    "struct field type",
                                );
                            }
                        }
                        let (fty, idx) = matched_fields
                            .iter()
                            .find(|(n, _, _)| n == field)
                            .map(|(_, t, i)| (self.infer_ctx.shallow_resolve(t), *i))
                            .unwrap_or_else(|| (fty_placeholder, 0));
                        (fty, idx)
                    } else {
                        self.deferred_fields.push(super::DeferredField {
                            receiver_ty: resolved_ty.clone(),
                            field_name: *field,
                            field_ty: fty_placeholder.clone(),
                            span: *span,
                        });
                        (fty_placeholder, 0)
                    }
                } else {
                    (self.infer_ctx.fresh_var(), 0)
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Field(Box::new(hobj), *field, idx),
                    ty,
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_index(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Index(arr, idx, span) => {
                let harr = self.lower_expr(arr)?;
                let hidx = self.lower_expr(idx)?;

                let peeled_ty = harr.ty.clone();
                let const_idx = match &hidx.kind {
                    hir::ExprKind::Int(n) if *n >= 0 => Some(*n as usize),
                    _ => None,
                };
                let elem_ty = match &peeled_ty {
                    Type::Array(et, _) => *et.clone(),
                    Type::Vec(et) => *et.clone(),
                    Type::Ptr(et) => *et.clone(),
                    Type::Map(_, vt) => *vt.clone(),
                    Type::Tuple(tys) => const_idx
                        .and_then(|i| tys.get(i).cloned())
                        .or_else(|| tys.first().cloned())
                        .unwrap_or_else(|| self.infer_ctx.fresh_var()),
                    _ => self.infer_ctx.fresh_var(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Index(Box::new(harr), Box::new(hidx)),
                    ty: elem_ty,
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }
}
