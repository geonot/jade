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
                    if !is_struct_ctor
                        && !is_variant_ctor
                        && let Some(gf) = self.generic_fns.get(ctor_name).cloned()
                        && let Some(tys) = self.expr_to_type_args(type_arg_expr)
                    {
                        let tparams = if gf.type_params.is_empty() {
                            self.effective_type_params(&gf)
                        } else {
                            gf.type_params.clone()
                        };
                        if tys.len() != tparams.len() {
                            return Err(format!(
                                "{}: `{}` declares {} type parameter(s) but this call \
                                 supplies {}",
                                span.loc(),
                                ctor_name,
                                tparams.len(),
                                tys.len()
                            ));
                        }
                        let mut type_map = std::collections::HashMap::new();
                        for (tp, ta) in tparams.iter().zip(tys.iter()) {
                            type_map.insert(*tp, self.resolve_ty(ta.clone()));
                        }
                        let mut hargs: Vec<hir::Expr> = Vec::new();
                        for (i, arg) in args.iter().enumerate() {
                            let exp_t = gf
                                .params
                                .get(i)
                                .and_then(|p| p.ty.as_ref())
                                .map(|t| Self::substitute_type_params(t, &type_map));
                            hargs.push(self.lower_expr_expected(arg, exp_t.as_ref())?);
                        }
                        self.instantiated_generics.insert(*ctor_name);
                        let result = self.monomorphize_call(
                            &ctor_name.as_str(),
                            &type_map,
                            hargs,
                            *span,
                            true,
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
                let result = self.lower_call_expected(callee, args, *span, expected)?;
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
                            let arg_tys: Vec<_> = hargs.iter().map(|ha| ha.ty.clone()).collect();
                            for (i, aty) in arg_tys.iter().enumerate() {
                                if let Some(pty) = ptys.get(i).cloned() {
                                    self.check_extern_arg(&pty, aty, *span, "extern arg");
                                }
                            }
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::Call(id, *method, hargs),
                                ty: ret,
                                span: *span,
                            });
                        }
                        let callee = ast::Expr::Ident(qualified_name, *span);
                        let result = self.lower_call_expected(&callee, args, *span, expected)?;
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
                            let arg_tys: Vec<_> = hargs.iter().map(|ha| ha.ty.clone()).collect();
                            for (i, aty) in arg_tys.iter().enumerate() {
                                if let Some(pty) = ptys.get(i).cloned() {
                                    self.check_extern_arg(&pty, aty, *span, "extern arg");
                                }
                            }
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::Call(id, *method, hargs),
                                ty: ret,
                                span: *span,
                            });
                        }

                        let callee = ast::Expr::Ident(*method, *span);
                        let result = self.lower_call_expected(&callee, args, *span, expected)?;
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
                self.suppress_whole_struct_check += 1;
                let hobj_res = self.lower_expr(obj);
                self.suppress_whole_struct_check -= 1;
                let hobj = hobj_res?;

                if self.suppress_moved_field_check == 0
                    && let Some(mut read_place) = crate::typer::place::place_of_expr(&hobj)
                {
                    read_place
                        .proj
                        .push(crate::typer::place::Proj::Field(*field));
                    let hit = self
                        .moves
                        .covering(&read_place)
                        .filter(|e| !e.place.is_root())
                        .map(|e| e.place.render())
                        .or_else(|| self.moves.within(&read_place).map(|e| e.place.render()));
                    if let Some(moved_place) = hit {
                        let read = read_place.render();
                        return Err(format!(
                            "{}: use of moved field `{}`: `{}` was moved out earlier \
                             (binding an aggregate field moves it, as `take` does); to keep \
                             it, clone at the move site (`copy {}`), or reassign \
                             `{}` before reading it",
                            span.loc(),
                            read,
                            moved_place,
                            moved_place,
                            moved_place,
                        ));
                    }
                }
                let resolved_ty = match self.infer_ctx.shallow_resolve(&hobj.ty) {
                    Type::Frozen(inner) => self.infer_ctx.shallow_resolve(&inner),
                    other => other,
                };
                let resolved_ty = self.normalize_named_ty(resolved_ty);

                if let Type::Row(store) = &resolved_ty
                    && let Some(rels) = self.store_relations.get(store)
                    && let Some((_, target, is_has_many, _)) =
                        rels.iter().find(|(n, _, _, _)| n == field).copied()
                {
                    if !self.store_schemas.contains_key(&target) {
                        return Err(format!(
                            "{}: relation `{}.{}` targets unknown store `{}`",
                            span.loc(),
                            store,
                            field,
                            target,
                        ));
                    }
                    if is_has_many {
                        let fk_field = self.store_relations.get(&target).and_then(|rs| {
                            rs.iter()
                                .find(|(_, t, hm, _)| !hm && t == store)
                                .map(|(f, _, _, _)| *f)
                        });
                        let Some(fk_field) = fk_field else {
                            return Err(format!(
                                "{}: `{}.{}` cannot be traversed — store `{}` has no \
                                 belongs-to relation back to `{}`; declare \
                                 `&<name> as {}` in `store {}`",
                                span.loc(),
                                store,
                                field,
                                target,
                                store,
                                store,
                                target,
                            ));
                        };
                        let struct_name = Symbol::intern(&format!("__store_{store}"));
                        let sid_sym: Symbol = "sid".into();
                        let sid_idx = self
                            .structs
                            .get(&struct_name)
                            .and_then(|fs| fs.iter().position(|(n, _)| *n == sid_sym));
                        let Some(sid_idx) = sid_idx else {
                            return Err(format!(
                                "{}: `{}.{}` cannot be traversed — store `{}` is @simple \
                                 and has no sid",
                                span.loc(),
                                store,
                                field,
                                store,
                            ));
                        };
                        let sid = hir::Expr {
                            kind: hir::ExprKind::Field(Box::new(hobj.clone()), sid_sym, sid_idx),
                            ty: Type::I64,
                            span: *span,
                        };
                        let filter = hir::StoreFilter {
                            field: fk_field,
                            op: crate::ast::BinOp::Eq,
                            value: sid,
                            span: *span,
                            extra: vec![],
                            pred: crate::ast::FilterPred::Cmp,
                        };
                        let elem =
                            Type::Struct(Symbol::intern(&format!("__store_{target}")), vec![]);
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::StoreAllWhere(target, Box::new(filter)),
                            ty: Type::Vec(Box::new(elem)),
                            span: *span,
                        });
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
                if matches!(&peeled_ty, Type::View(_)) && (field == "length" || field == "count") {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Field(Box::new(hobj), *field, 0),
                        ty: Type::I64,
                        span: *span,
                    });
                }
                let struct_name = match &peeled_ty {
                    Type::Struct(name, _) => Some(*name),

                    Type::Row(store) => Some(Symbol::intern(&format!("__store_{store}"))),
                    Type::Ptr(inner) => match inner.as_ref() {
                        Type::Struct(name, _) => Some(*name),
                        _ => None,
                    },
                    Type::View(inner) => match self.infer_ctx.shallow_resolve(inner) {
                        Type::Struct(name, _) => Some(name),
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
                            let display: String = if let Some(stripped) =
                                raw.strip_prefix("__store_")
                            {
                                format!("{} (query result)", stripped)
                            } else if let Some((base, args)) = self.infer_ctx.mono_origin(name) {
                                let rendered: Vec<String> =
                                    args.iter().map(|a| format!("{a}")).collect();
                                format!("{}<{}>", base, rendered.join(", "))
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
                    } else if let Some(gtd) = self.generic_types.get(name).cloned() {
                        let args = match &peeled_ty {
                            Type::Struct(_, args) => args.clone(),
                            Type::Ptr(inner) | Type::View(inner) => {
                                match self.infer_ctx.shallow_resolve(inner) {
                                    Type::Struct(_, args) => args,
                                    _ => vec![],
                                }
                            }
                            _ => vec![],
                        };
                        let mut type_map = std::collections::HashMap::new();
                        for (tp, ta) in gtd.type_params.iter().zip(args.iter()) {
                            type_map.insert(*tp, ta.clone());
                        }
                        if let Some((i, f)) = gtd
                            .fields
                            .iter()
                            .enumerate()
                            .find(|(_, f)| f.name == *field)
                        {
                            let declared = f.ty.clone().unwrap_or(Type::I64);
                            let substituted = Self::substitute_type_params(&declared, &type_map);
                            (self.infer_ctx.shallow_resolve(&substituted), i)
                        } else {
                            return Err(format!(
                                "{}: type '{}' has no field '{}'",
                                span.loc(),
                                name,
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
                } else if matches!(&peeled_ty, Type::View(_)) && field == "length" {
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

                if let Type::Map(key_ty, val_ty) = self.infer_ctx.shallow_resolve(&harr.ty) {
                    let hidx = self.lower_expr_expected(idx, Some(&key_ty))?;
                    let _ = self
                        .infer_ctx
                        .unify_at(&key_ty, &hidx.ty, *span, "map index key");
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::MapMethod(Box::new(harr), "get".into(), vec![hidx]),
                        ty: *val_ty,
                        span: *span,
                    });
                }
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
                    Type::Tuple(tys) => match const_idx {
                        None => {
                            return Err(format!(
                                "{}: tuple indices must be integer literals — a tuple's \
                                 element types differ per position, so the index must be \
                                 known at compile time; use a Vec for runtime indexing",
                                span.loc()
                            ));
                        }
                        Some(i) if i >= tys.len() => {
                            return Err(format!(
                                "{}: tuple index {} is out of range for a {}-element tuple",
                                span.loc(),
                                i,
                                tys.len()
                            ));
                        }
                        Some(i) => tys[i].clone(),
                    },
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
