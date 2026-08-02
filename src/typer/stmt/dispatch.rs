use crate::ast;
use crate::hir::{self, DefId, Ownership};
use crate::intern::Symbol;
use crate::types::{Scheme, Type};

use super::super::{Typer, VarInfo};

impl Typer {
    pub(in crate::typer) fn try_wrap_err_return(
        &mut self,
        e: &ast::Expr,
        result_enum: Symbol,
        result_ty: &Type,
        span: ast::Span,
    ) -> Result<Option<hir::Stmt>, String> {
        let err_payload = self
            .enums
            .get(&result_enum)
            .and_then(|vs| vs.iter().find(|(n, _)| n.as_str() == "Err"))
            .and_then(|(_, f)| f.first().cloned());
        let err_tag = self
            .enums
            .get(&result_enum)
            .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == "Err"))
            .map(|i| i as u32)
            .unwrap_or(1);
        let Some(err_payload) = err_payload else {
            return Ok(None);
        };

        let hv = self.lower_expr(e)?;
        let val_ty = self.infer_ctx.resolve(&hv.ty);

        if val_ty == *result_ty {
            return Ok(Some(hir::Stmt::ErrReturn(hv, result_ty.clone(), span)));
        }

        let norm = |t: &Type| -> Option<Symbol> {
            match t {
                Type::Enum(n) => Some(*n),
                Type::Struct(n, _) => Some(*n),
                _ => None,
            }
        };
        let target_err_enum = norm(&err_payload);
        let src_enum = norm(&val_ty);

        let same_err = matches!((src_enum, target_err_enum), (Some(a), Some(b)) if a == b);
        let wrapped_val = if val_ty == err_payload || same_err {
            hv
        } else if let (Some(tgt), Some(src)) = (target_err_enum, src_enum) {
            let from_name: Symbol = format!("{tgt}_from_{src}").into();
            if let Some((id, _ptys, fret)) = self.fns.get(&from_name).cloned() {
                let ur =
                    self.infer_ctx
                        .unify_at(&fret, &err_payload, span, "From conversion result");
                self.collect_unify_error(ur);
                hir::Expr {
                    kind: hir::ExprKind::Call(id, from_name, vec![hv]),
                    ty: err_payload.clone(),
                    span,
                }
            } else if self.err_enum_names.contains(&src) && self.err_enum_names.contains(&tgt) {
                return Err(format!(
                    "{}: no conversion `{src} -> {tgt}`: propagation here yields an err \
                     `{src}`, but this function's error type is `{tgt}` and there is no \
                     `impl From of {src} for {tgt}`; add that impl with \
                     `*from(e as {src}) returns {tgt}`, or add `| {src}` to the function's \
                     error union",
                    span.loc()
                ));
            } else {
                return Ok(None);
            }
        } else {
            return Ok(None);
        };

        if let Some(en) = &target_err_enum
            && self.err_enum_names.contains(en)
        {
            self.current_fn_error_types.insert(*en);
        }
        if let Some(en) = &src_enum
            && self.err_enum_names.contains(en)
        {
            self.current_fn_error_types.insert(*en);
        }

        let err_ctor = hir::Expr {
            kind: hir::ExprKind::VariantCtor(
                result_enum,
                "Err".into(),
                err_tag,
                vec![hir::FieldInit {
                    name: None,
                    value: wrapped_val,
                }],
            ),
            ty: result_ty.clone(),
            span,
        };
        Ok(Some(hir::Stmt::ErrReturn(
            err_ctor,
            result_ty.clone(),
            span,
        )))
    }

    pub(in crate::typer) fn propagate_err_value(
        &mut self,
        err_val: hir::Expr,
        span: ast::Span,
    ) -> Result<hir::Stmt, String> {
        let ret_ty = self.current_fn_ret_ty.clone().unwrap_or(Type::Void);
        let resolved_ret = self.infer_ctx.resolve(&ret_ty);
        let result_enum: Option<Symbol> = match &resolved_ret {
            Type::Enum(rn) if self.is_result_enum(*rn) => Some(*rn),
            Type::Struct(rn, args)
                if rn.as_str() == "Result" && self.generic_enums.contains_key(rn) =>
            {
                let ge = self.generic_enums.get(rn).cloned().unwrap();
                if args.len() == ge.type_params.len() {
                    let mut tm = std::collections::HashMap::new();
                    for (tp, ta) in ge.type_params.iter().zip(args.iter()) {
                        tm.insert(*tp, ta.clone());
                    }
                    self.monomorphize_enum(&rn.as_str(), &tm).ok()
                } else {
                    None
                }
            }
            _ => None,
        };

        let Some(result_enum) = result_enum else {
            return Ok(hir::Stmt::ErrReturn(err_val, resolved_ret, span));
        };

        let mono_ret = Type::Enum(result_enum);
        let err_payload = self
            .enums
            .get(&result_enum)
            .and_then(|vs| vs.iter().find(|(n, _)| n.as_str() == "Err"))
            .and_then(|(_, f)| f.first().cloned());
        let err_tag = self
            .enums
            .get(&result_enum)
            .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == "Err"))
            .map(|i| i as u32)
            .unwrap_or(1);
        let Some(err_payload) = err_payload else {
            return Ok(hir::Stmt::ErrReturn(err_val, mono_ret, span));
        };

        let val_ty = self.infer_ctx.resolve(&err_val.ty);
        let norm = |t: &Type| -> Option<Symbol> {
            match t {
                Type::Enum(n) => Some(*n),
                Type::Struct(n, _) => Some(*n),
                _ => None,
            }
        };
        let target_err_enum = norm(&err_payload);
        let src_enum = norm(&val_ty);
        let same_err = matches!((src_enum, target_err_enum), (Some(a), Some(b)) if a == b);

        let wrapped_val = if val_ty == err_payload || same_err {
            err_val
        } else if let (Some(tgt), Some(src)) = (target_err_enum, src_enum) {
            let from_name: Symbol = format!("{tgt}_from_{src}").into();
            if let Some((id, _ptys, fret)) = self.fns.get(&from_name).cloned() {
                let ur =
                    self.infer_ctx
                        .unify_at(&fret, &err_payload, span, "From conversion result");
                self.collect_unify_error(ur);
                hir::Expr {
                    kind: hir::ExprKind::Call(id, from_name, vec![err_val]),
                    ty: err_payload.clone(),
                    span,
                }
            } else if self.err_enum_names.contains(&src) && self.err_enum_names.contains(&tgt) {
                return Err(format!(
                    "{}: no conversion `{src} -> {tgt}`: propagation here yields an err \
                     `{src}`, but this function's error type is `{tgt}` and there is no \
                     `impl From of {src} for {tgt}`; add that impl with \
                     `*from(e as {src}) returns {tgt}`, or add `| {src}` to the function's \
                     error union",
                    span.loc()
                ));
            } else {
                err_val
            }
        } else {
            err_val
        };

        if let Some(en) = &target_err_enum
            && self.err_enum_names.contains(en)
        {
            self.current_fn_error_types.insert(*en);
        }
        if let Some(en) = &src_enum
            && self.err_enum_names.contains(en)
        {
            self.current_fn_error_types.insert(*en);
        }

        let err_ctor = hir::Expr {
            kind: hir::ExprKind::VariantCtor(
                result_enum,
                "Err".into(),
                err_tag,
                vec![hir::FieldInit {
                    name: None,
                    value: wrapped_val,
                }],
            ),
            ty: mono_ret.clone(),
            span,
        };
        Ok(hir::Stmt::ErrReturn(err_ctor, mono_ret, span))
    }

    pub(in crate::typer) fn is_aliased_read_of_heap(expr: &hir::Expr) -> bool {
        let needs_drop = matches!(
            expr.ty,
            Type::Vec(_) | Type::Map(_, _) | Type::String | Type::Struct(_, _) | Type::Enum(_)
        );
        if !needs_drop {
            return false;
        }
        match &expr.kind {
            hir::ExprKind::VecMethod(_, name, _) | hir::ExprKind::MapMethod(_, name, _) => {
                matches!(
                    name.as_str().as_ref(),
                    "get" | "peek" | "front" | "back" | "first" | "last"
                )
            }
            _ => false,
        }
    }

    pub(crate) fn lower_stmt(
        &mut self,
        stmt: &ast::Stmt,
        ret_ty: &Type,
    ) -> Result<hir::Stmt, String> {
        match stmt {
            ast::Stmt::Bind(b) => {
                if self.current_method_type.is_some() && self.find_var(&b.name.as_str()).is_none() {
                    let type_name = self.current_method_type.clone().unwrap();
                    let is_field = self
                        .structs
                        .get(&type_name)
                        .map(|fields| fields.iter().any(|(n, _)| n == &b.name))
                        .unwrap_or(false);
                    if is_field {
                        let self_expr = ast::Expr::Ident("self".into(), b.span);
                        let field_expr = ast::Expr::Field(Box::new(self_expr), b.name, b.span);
                        let ht = self.lower_expr(&field_expr)?;
                        let hv = self.lower_expr_expected(&b.value, Some(&ht.ty))?;
                        let r = self
                            .infer_ctx
                            .unify_at(&ht.ty, &hv.ty, b.span, "field assignment");
                        self.collect_unify_error(r);
                        let hv = self.maybe_coerce_to(hv, &ht.ty);
                        return Ok(hir::Stmt::Assign(ht, hv, b.span));
                    }
                }

                if self.find_var(&b.name.as_str()).is_none()
                    && let Some((_gexpr, _gspan)) = self.globals.get(&b.name).cloned()
                {
                    let init_hir = self.lower_expr(&_gexpr)?;
                    let global_ty = init_hir.ty.clone();
                    let hv = self.lower_expr_expected(&b.value, Some(&global_ty))?;
                    return Ok(hir::Stmt::GlobalStore(b.name, hv, b.span));
                }
                let value = if let Some(ref ann) = b.ty {
                    let ann_ty = self.resolve_ty(ann.clone());
                    self.lower_expr_expected(&b.value, Some(&ann_ty))?
                } else if let Some(existing) = self.find_var(&b.name.as_str()) {
                    self.lower_expr_expected(&b.value, Some(&existing.ty.clone()))?
                } else {
                    self.lower_expr(&b.value)?
                };
                let value = if b.ty.is_none()
                    && !matches!(&b.value, ast::Expr::Quaternary(..) | ast::Expr::Ternary(..))
                {
                    match self.implicit_propagate(value.clone())? {
                        Some(v) => v,
                        None => value,
                    }
                } else {
                    value
                };
                let ty = if let Some(ref ann) = b.ty {
                    let ann_ty = self.resolve_ty(ann.clone());
                    let _ = self
                        .infer_ctx
                        .unify_at(&ann_ty, &value.ty, b.span, "bind annotation");
                    ann_ty
                } else {
                    value.ty.clone()
                };
                let mut ownership = Self::ownership_for_type(&ty);

                let is_resource = self.type_has_resource_annotation(&ty);
                if is_resource && b.access_mod.is_none() && Self::is_aliased_read_of_heap(&value) {
                    return Err(format!(
                        "{}: cannot bind `@resource` value `{}` from a container read without an access modifier; use `take`, `ref`, or `mut`",
                        b.span.loc(),
                        ty
                    ));
                }

                // Non-strict: `ty` may still hold unsolved vars here (e.g. a
                // polymorphic lambda bind before generalization); an
                // unsolved type is simply not an aggregate.
                let resolved_bind_ty = {
                    let was_strict = self.infer_ctx.is_strict();
                    self.infer_ctx.set_strict(false);
                    let r = self.infer_ctx.resolve(&ty);
                    self.infer_ctx.set_strict(was_strict);
                    r
                };

                // M4 (memory-model.md): a container slot cannot be
                // tombstoned, so binding an aggregate element would alias
                // the container's memory. Scalar and String elements bind
                // freely (they copy); expression-position reads stay
                // borrows. Checked on the expression KIND plus the
                // resolved type — `is_aliased_read_of_heap` tests the
                // unresolved `expr.ty`, which is still a TypeVar for an
                // inferred-element container.
                let is_element_read = match &value.kind {
                    hir::ExprKind::VecMethod(_, mname, _)
                    | hir::ExprKind::MapMethod(_, mname, _) => matches!(
                        mname.as_str().as_ref(),
                        "get" | "peek" | "front" | "back" | "first" | "last"
                    ),
                    hir::ExprKind::Index(..) => true,
                    _ => false,
                };
                if b.access_mod.is_none()
                    && is_element_read
                    && self.type_is_aggregate(&resolved_bind_ty)
                {
                    return Err(format!(
                        "{}: cannot bind aggregate element to `{}` — binding would alias \
                         the container's memory; clone it (`{} is copy ...`) or remove \
                         it (`{} is take ...`)",
                        b.span.loc(),
                        b.name,
                        b.name,
                        b.name,
                    ));
                }

                // M3 (memory-model.md): a plain bind of an aggregate
                // struct field is a partial move, exactly as `take b.f`
                // does today — canonicalize the access modifier so the
                // MIR FieldClear tombstone and the moved-field
                // diagnostics both apply.
                let access_mod = {
                    let field_of_var = matches!(
                        &value.kind,
                        hir::ExprKind::Field(parent, _, _)
                            if matches!(parent.kind, hir::ExprKind::Var(..))
                    );
                    if b.access_mod.is_none()
                        && field_of_var
                        && self.type_is_aggregate(&resolved_bind_ty)
                    {
                        Some(ast::AccessMod::Take)
                    } else {
                        b.access_mod
                    }
                };

                if Self::is_aliased_read_of_heap(&value) && !ty.is_value_clonable() {
                    ownership = Ownership::Borrowed;
                }

                if access_mod.is_some() {
                    ownership = self.ownership_with_mod(&ty, access_mod)?;
                }

                let partial_move: Option<(DefId, Symbol)> =
                    if matches!(access_mod, Some(ast::AccessMod::Take))
                        && let hir::ExprKind::Field(parent, field, _) = &value.kind
                        && let hir::ExprKind::Var(parent_id, _) = &parent.kind
                    {
                        Some((*parent_id, *field))
                    } else {
                        None
                    };
                let id = self.fresh_id();
                if let Some(existing) = self.find_var(&b.name.as_str()) {
                    let id = existing.def_id;

                    if self.const_vars.contains(&id) {
                        return Err(format!(
                            "cannot rebind `{}`: it was declared with `is const`",
                            b.name.as_str()
                        ));
                    }
                    let existing_ty = existing.ty.clone();
                    let value = self.maybe_coerce_to(value, &existing_ty);

                    self.clear_all_moved_for(id);
                    self.update_var(
                        &b.name.as_str(),
                        VarInfo {
                            def_id: id,
                            ty: existing_ty.clone(),
                            ownership,
                            scheme: None,
                        },
                    );
                    if let Some((pid, fname)) = partial_move {
                        self.mark_field_moved(pid, fname);
                    }
                    Ok(hir::Stmt::Bind(hir::Bind {
                        def_id: id,
                        name: b.name,
                        value,
                        ty: existing_ty,
                        ownership,
                        atomic: b.atomic,
                        access_mod,
                        span: b.span,
                    }))
                } else {
                    let scheme = if Self::is_syntactic_value(&b.value) {
                        self.generalize(&ty)
                    } else {
                        Scheme::mono(ty.clone())
                    };
                    if scheme.is_poly() {
                        self.infer_ctx.mark_quantified(&scheme.quantified);
                        self.deferred_quantified_vars
                            .extend(scheme.quantified.iter().copied());
                        if let ast::Expr::Lambda(params, ret, body, lspan) = &b.value {
                            self.poly_lambda_asts.insert(
                                b.name,
                                (params.clone(), ret.clone(), body.clone(), *lspan),
                            );
                        }
                    }
                    self.define_var(
                        &b.name.as_str(),
                        VarInfo {
                            def_id: id,
                            ty: ty.clone(),
                            ownership,
                            scheme: Some(scheme),
                        },
                    );

                    if matches!(access_mod, Some(ast::AccessMod::Const)) {
                        self.const_vars.insert(id);
                    }
                    if let Some((pid, fname)) = partial_move {
                        self.mark_field_moved(pid, fname);
                    }
                    Ok(hir::Stmt::Bind(hir::Bind {
                        def_id: id,
                        name: b.name,
                        value,
                        ty,
                        ownership,
                        atomic: b.atomic,
                        access_mod,
                        span: b.span,
                    }))
                }
            }

            ast::Stmt::TupleBind(names, value, span) => {
                let hval = self.lower_expr(value)?;
                let resolved_ty = self.infer_ctx.shallow_resolve(&hval.ty);
                let tys = match &resolved_ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => (0..names.len())
                        .map(|_| self.infer_ctx.fresh_var())
                        .collect(),
                };
                let bindings: Vec<(DefId, Symbol, Type)> = names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| {
                        let ty = tys
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| self.infer_ctx.fresh_var());
                        let id = self.fresh_id();
                        self.define_var(
                            &n.as_str(),
                            VarInfo {
                                def_id: id,
                                ty: ty.clone(),
                                ownership: Self::ownership_for_type(&ty),
                                scheme: None,
                            },
                        );
                        (id, *n, ty)
                    })
                    .collect();
                Ok(hir::Stmt::TupleBind(bindings, hval, *span))
            }

            ast::Stmt::Assign(target, value, span) => {
                if let ast::Expr::Field(obj, field, fspan) = target
                    && let ast::Expr::Ident(row_name, _) = obj.as_ref()
                {
                    let probe = self.lower_expr(obj.as_ref())?;
                    let probe_ty = self.infer_ctx.shallow_resolve(&probe.ty);
                    if let Type::Row(store) = &probe_ty {
                        let store = *store;
                        let schema = self
                            .store_schemas
                            .get(&store)
                            .ok_or_else(|| format!("unknown store '{store}'"))?
                            .clone();
                        let (_, fty) = schema
                            .iter()
                            .find(|(n, _)| n == field)
                            .ok_or_else(|| {
                                format!(
                                    "{}: store '{}' has no field '{}'",
                                    fspan.loc(),
                                    store,
                                    field,
                                )
                            })?
                            .clone();
                        let hv = self.lower_expr_expected(value, Some(&fty))?;
                        let r =
                            self.infer_ctx
                                .unify_at(&fty, &hv.ty, *span, "row field assignment");
                        self.collect_unify_error(r);
                        let hv = self.maybe_coerce_to(hv, &fty);

                        let sid_sym = Symbol::intern("sid");
                        let sid_expr = hir::Expr {
                            kind: hir::ExprKind::Field(Box::new(probe), sid_sym, 0),
                            ty: Type::I64,
                            span: *fspan,
                        };
                        let hfilter = hir::StoreFilter {
                            field: sid_sym,
                            op: ast::BinOp::Eq,
                            value: sid_expr,
                            span: *span,
                            extra: Vec::new(),
                            pred: ast::FilterPred::Cmp,
                        };
                        let _ = row_name;
                        return Ok(hir::Stmt::StoreSet(
                            store,
                            vec![(*field, hv)],
                            Box::new(hfilter),
                            *span,
                        ));
                    }
                }

                self.suppress_moved_field_check += 1;
                let ht = self.lower_expr(target)?;
                self.suppress_moved_field_check -= 1;
                let hv = self.lower_expr_expected(value, Some(&ht.ty))?;
                let r = self.infer_ctx.unify_at(&ht.ty, &hv.ty, *span, "assignment");
                self.collect_unify_error(r);
                let hv = self.maybe_coerce_to(hv, &ht.ty);

                if let hir::ExprKind::Field(parent, field, _) = &ht.kind
                    && let hir::ExprKind::Var(parent_id, _) = &parent.kind
                {
                    self.clear_field_moved(*parent_id, field);
                }
                Ok(hir::Stmt::Assign(ht, hv, *span))
            }

            ast::Stmt::Expr(e) => {
                if let ast::Expr::Query(source, clauses, span) = e {
                    let store_name = match source.as_ref() {
                        ast::Expr::Ident(name, _) => *name,
                        _ => return Err("query block source must be a store name".into()),
                    };
                    let schema = self
                        .store_schemas
                        .get(&store_name)
                        .ok_or_else(|| format!("unknown store '{store_name}'"))?
                        .clone();

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
                                sets.push((*field, val.clone()));
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

                    if !where_exprs.is_empty() && has_delete {
                        let ast_filter = Self::merge_where_clauses(&where_exprs)?;
                        let hfilter =
                            self.lower_store_filter(&ast_filter, &schema, &store_name.as_str())?;
                        return Ok(hir::Stmt::StoreDelete(store_name, Box::new(hfilter), *span));
                    }

                    if !where_exprs.is_empty() && !sets.is_empty() {
                        let ast_filter = Self::merge_where_clauses(&where_exprs)?;
                        let hfilter =
                            self.lower_store_filter(&ast_filter, &schema, &store_name.as_str())?;
                        let mut hassigns = Vec::new();
                        for (fname, fval) in &sets {
                            if let Some((_, fty)) = schema.iter().find(|(n, _)| n == fname) {
                                hassigns.push((*fname, self.lower_expr_expected(fval, Some(fty))?));
                            } else {
                                return Err(format!("store '{store_name}' has no field '{fname}'"));
                            }
                        }
                        return Ok(hir::Stmt::StoreSet(
                            store_name,
                            hassigns,
                            Box::new(hfilter),
                            *span,
                        ));
                    }
                }

                let he = self.lower_expr(e)?;

                if let ast::Expr::Method(recv, _, _, mspan) = e
                    && matches!(recv.as_ref(), ast::Expr::Ident(_, _))
                {
                    let target_opt = match &he.kind {
                        hir::ExprKind::VecMethod(obj, _, _)
                        | hir::ExprKind::MapMethod(obj, _, _) => {
                            if matches!(obj.kind, hir::ExprKind::Var(_, _)) {
                                let obj_resolved = self.infer_ctx.resolve(&obj.ty);
                                let he_resolved = self.infer_ctx.resolve(&he.ty);
                                if obj_resolved == he_resolved
                                    && matches!(obj_resolved, Type::Vec(_) | Type::Map(_, _))
                                {
                                    Some((**obj).clone())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    if let Some(target) = target_opt {
                        return Ok(hir::Stmt::Assign(target, he, *mspan));
                    }
                }
                let he = if matches!(e, ast::Expr::Quaternary(..) | ast::Expr::Ternary(..)) {
                    he
                } else {
                    match self.implicit_propagate(he.clone())? {
                        Some(v) => v,
                        None => he,
                    }
                };
                Ok(hir::Stmt::Expr(he))
            }

            ast::Stmt::If(i) => {
                let hi = self.lower_if(i, ret_ty)?;
                Ok(hir::Stmt::If(hi))
            }

            ast::Stmt::While(w) => {
                let cond = self.lower_expr_expected(&w.cond, Some(&Type::Bool))?;

                let outer_ids = self.in_scope_def_ids();
                let pre = self.snapshot_moved_fields();
                let body = self.lower_block(&w.body, ret_ty)?;
                self.check_loop_body_moves(&pre, &outer_ids, w.span)?;
                self.restore_moved_fields(pre);
                Ok(hir::Stmt::While(hir::While {
                    cond,
                    body,
                    span: w.span,
                }))
            }

            ast::Stmt::For(f) => {
                let iter = self.lower_expr(&f.iter)?;
                let end = f.end.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                let step = f.step.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                let resolved_iter_ty = self.infer_ctx.shallow_resolve(&iter.ty);

                if let (Some(val_bind), Type::Map(key_ty, val_ty)) = (&f.bind2, &resolved_iter_ty) {
                    return self.desugar_for_map(
                        f,
                        &val_bind.as_str(),
                        iter,
                        key_ty,
                        val_ty,
                        ret_ty,
                    );
                }

                let iter_is_int = resolved_iter_ty.is_int()
                    || if let Type::TypeVar(id) = &resolved_iter_ty {
                        let c = self.infer_ctx.constraint(*id);
                        matches!(
                            c,
                            super::super::unify::TypeConstraint::Integer
                                | super::super::unify::TypeConstraint::Numeric
                        )
                    } else {
                        false
                    };
                let bind_ty = if end.is_some() || iter_is_int {
                    Type::I64
                } else {
                    match &iter.ty {
                        Type::Array(et, _) => *et.clone(),
                        Type::Ptr(et) => *et.clone(),
                        Type::Vec(et) => *et.clone(),
                        Type::String => Type::I64,
                        _ => {
                            let iter_ty = iter.ty.clone();
                            if let Type::Struct(tn, _) = iter_ty
                                && self.type_implements_trait(&tn.as_str(), "Iter")
                            {
                                let elem_ty = self.iter_element_type(&tn.as_str());
                                return self.desugar_for_iter(
                                    f,
                                    iter,
                                    tn.as_str(),
                                    elem_ty,
                                    ret_ty,
                                );
                            }
                            self.infer_ctx.fresh_var()
                        }
                    }
                };
                let bind_id = self.fresh_id();
                let outer_ids = self.in_scope_def_ids();
                self.push_scope();

                let is_collection_for = !(end.is_some() || iter_is_int);
                let binder_ownership = if is_collection_for {
                    Ownership::Borrowed
                } else {
                    Ownership::Owned
                };
                self.define_var(
                    &f.bind.as_str(),
                    VarInfo {
                        def_id: bind_id,
                        ty: bind_ty.clone(),
                        ownership: binder_ownership,
                        scheme: None,
                    },
                );

                let (bind2_id, bind2, bind2_ty) = if let Some(ref b2) = f.bind2 {
                    let id2 = self.fresh_id();
                    self.define_var(
                        &b2.as_str(),
                        VarInfo {
                            def_id: id2,
                            ty: Type::I64,
                            ownership: Ownership::Owned,
                            scheme: None,
                        },
                    );
                    (Some(id2), Some(*b2), Some(Type::I64))
                } else {
                    (None, None, None)
                };

                let pre_loop = self.snapshot_moved_fields();
                let mut body = self.lower_block_no_scope(&f.body, ret_ty)?;
                self.check_loop_body_moves(&pre_loop, &outer_ids, f.span)?;
                self.finalize_loop_body_drops(&mut body);
                self.pop_scope();
                self.restore_moved_fields(pre_loop);
                Ok(hir::Stmt::For(hir::For {
                    bind_id,
                    bind: f.bind,
                    bind_ty,
                    bind2_id,
                    bind2,
                    bind2_ty,
                    iter,
                    end,
                    step,
                    body,
                    label: f.label,
                    access_mod: f.access_mod,
                    span: f.span,
                }))
            }
            ast::Stmt::Loop(l) => {
                let outer_ids = self.in_scope_def_ids();
                let pre = self.snapshot_moved_fields();
                let body = self.lower_block(&l.body, ret_ty)?;
                self.check_loop_body_moves(&pre, &outer_ids, l.span)?;
                self.restore_moved_fields(pre);
                Ok(hir::Stmt::Loop(hir::Loop { body, span: l.span }))
            }

            ast::Stmt::Ret(val, span) => {
                let resolved_ret = self.infer_ctx.shallow_resolve(ret_ty);
                if let (Some(e), Some(result_enum)) =
                    (val.as_ref(), self.result_enum_of(&resolved_ret))
                {
                    let result_ty = Type::Enum(result_enum);
                    let he = if Self::is_result_variant_expr(e) {
                        self.lower_expr_expected(e, Some(&result_ty))?
                    } else {
                        let ok_inner = self.ok_inner_ty_pub(result_enum);
                        self.lower_expr_expected(e, Some(&ok_inner))?
                    };
                    let val_ty = self.infer_ctx.resolve(&he.ty);
                    let he = if self.result_enum_of(&val_ty).is_some() {
                        he
                    } else {
                        self.auto_wrap_ok(he, result_enum)
                    };
                    return Ok(hir::Stmt::Ret(Some(he), result_ty, *span));
                }
                let hval = val
                    .as_ref()
                    .map(|e| self.lower_expr_expected(e, Some(ret_ty)))
                    .transpose()?;
                if let Some(ref v) = hval {
                    let _ = self
                        .infer_ctx
                        .unify_at(&v.ty, ret_ty, *span, "return value");
                }
                let hval = hval.map(|v| self.maybe_coerce_to(v, ret_ty));
                Ok(hir::Stmt::Ret(hval, ret_ty.clone(), *span))
            }

            ast::Stmt::Break(val, span) => {
                let hval = val.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                Ok(hir::Stmt::Break(hval, *span))
            }

            ast::Stmt::Continue(span) => Ok(hir::Stmt::Continue(*span)),
            ast::Stmt::Nop(span) => Ok(hir::Stmt::Nop(*span)),

            ast::Stmt::Match(m) => {
                let hm = self.lower_match(m, ret_ty)?;
                Ok(hir::Stmt::Match(hm))
            }

            ast::Stmt::Asm(a) => {
                let inputs: Vec<(String, hir::Expr)> = a
                    .inputs
                    .iter()
                    .map(|(c, e)| Ok((c.clone(), self.lower_expr(e)?)))
                    .collect::<Result<_, String>>()?;
                Ok(hir::Stmt::Asm(hir::AsmBlock {
                    template: a.template.clone(),
                    outputs: a.outputs.clone(),
                    inputs,
                    clobbers: a.clobbers.clone(),
                    span: a.span,
                }))
            }

            ast::Stmt::ErrReturn(e, span) => {
                let resolved_ret_ty = self.infer_ctx.resolve(ret_ty);
                if let ast::Expr::Ident(n, _) = e
                    && n.as_str().contains("__prop_e_")
                {
                    let is_result_ret = match &resolved_ret_ty {
                        Type::Enum(rn) => {
                            let s = rn.as_str();
                            s.starts_with("Result_")
                                || s == "Result"
                                || s.starts_with("Option_")
                                || s == "Option"
                        }
                        Type::Struct(rn, _) => rn.as_str() == "Result" || rn.as_str() == "Option",
                        _ => false,
                    };
                    if !is_result_ret {
                        return Err(format!(
                            "{}: error propagation is only valid inside a function whose \
                             result type is a `Result`/`Option`; declare the enclosing \
                             function's error union with `! E` (e.g. `returns T ! E`)",
                            span.loc()
                        ));
                    }
                }
                let result_enum_name: Option<Symbol> = match &resolved_ret_ty {
                    Type::Enum(rn)
                        if rn.as_str().starts_with("Result_") || rn.as_str() == "Result" =>
                    {
                        Some(*rn)
                    }
                    Type::Struct(rn, args)
                        if rn.as_str() == "Result" && self.generic_enums.contains_key(rn) =>
                    {
                        let ge = self.generic_enums.get(rn).cloned().unwrap();
                        if args.len() == ge.type_params.len() {
                            let mut tm = std::collections::HashMap::new();
                            for (tp, ta) in ge.type_params.iter().zip(args.iter()) {
                                tm.insert(*tp, ta.clone());
                            }
                            self.monomorphize_enum(&rn.as_str(), &tm).ok()
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(rn) = result_enum_name {
                    let mono_ret = Type::Enum(rn);
                    if let Some(wrapped) = self.try_wrap_err_return(e, rn, &mono_ret, *span)? {
                        return Ok(wrapped);
                    }
                }

                let he = self.lower_expr_expected(e, Some(ret_ty))?;

                let resolved = self.infer_ctx.resolve(&he.ty);
                let enum_name: Option<Symbol> = match &resolved {
                    Type::Enum(n) if self.err_enum_names.contains(n) => Some(*n),
                    Type::Struct(n, _) if self.err_enum_names.contains(n) => Some(*n),
                    _ => None,
                };
                if let Some(en) = &enum_name {
                    if !self.current_fn_declared_errors.is_empty()
                        && !self.current_fn_declared_errors.contains(en)
                    {
                        return Err(format!(
                            "{1}: `! {0}` raises a variant of err `{0}`, but this function's \
                             declared error union does not list `{0}`; add `! {0}` to the \
                             signature, or raise an error the signature declares",
                            en,
                            span.loc()
                        ));
                    }
                    self.current_fn_error_types.insert(*en);
                }

                let resolved_ret = self.infer_ctx.resolve(ret_ty);
                let normalized_ret = match &resolved_ret {
                    Type::Struct(n, args) if args.is_empty() && self.enums.contains_key(n) => {
                        Type::Enum(*n)
                    }
                    _ => resolved_ret.clone(),
                };
                let resolved_val = self.infer_ctx.resolve(&he.ty);
                let normalized_val = match &resolved_val {
                    Type::Struct(n, args) if args.is_empty() && self.enums.contains_key(n) => {
                        Type::Enum(*n)
                    }
                    _ => resolved_val.clone(),
                };
                let unify_res = self.infer_ctx.unify_at_tolerant(
                    &normalized_val,
                    &normalized_ret,
                    *span,
                    "early-return value (`!`)",
                );
                if unify_res.is_err()
                    && let Some(en) = &enum_name
                {
                    return Err(format!(
                        "{1}: `err {0}` raises an error, but this function returns `{2}` with no \
                         declared error union; add `! {0}` to the signature (making the result \
                         `Result of {2}, {0}`), or return a value of `{2}`",
                        en,
                        span.loc(),
                        resolved_ret
                    ));
                }
                self.collect_unify_error(unify_res);

                let he = self.maybe_coerce_to(he, ret_ty);
                Ok(hir::Stmt::ErrReturn(he, ret_ty.clone(), *span))
            }

            ast::Stmt::Defer(body, span) => {
                let hbody = self.lower_block(body, ret_ty)?;
                Ok(hir::Stmt::Defer(hbody, *span))
            }

            ast::Stmt::StoreInsert(store, values, span) => {
                if self.enclosing_fn_is_fallible() {
                    let insert = self.lower_expr_store_insert(store, values, *span)?;
                    if let Some(prop) = self.implicit_propagate(insert.clone())? {
                        return Ok(hir::Stmt::Expr(prop));
                    }
                    return Ok(hir::Stmt::Expr(insert));
                }
                let hvalues = self.lower_store_insert_values(store, values)?;
                Ok(hir::Stmt::StoreInsert(*store, hvalues, *span))
            }

            ast::Stmt::StoreDelete(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, &store.as_str())?;
                Ok(hir::Stmt::StoreDelete(*store, Box::new(hfilter), *span))
            }

            ast::Stmt::StoreDestroy(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, &store.as_str())?;
                Ok(hir::Stmt::StoreDestroy(*store, Box::new(hfilter), *span))
            }

            ast::Stmt::StoreRestore(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, &store.as_str())?;
                Ok(hir::Stmt::StoreRestore(*store, Box::new(hfilter), *span))
            }

            ast::Stmt::StoreSave(store, span) => {
                if !self.store_schemas.contains_key(store) {
                    return Err(format!("unknown store '{store}'"));
                }
                Ok(hir::Stmt::StoreSave(*store, *span))
            }

            ast::Stmt::StoreCompact(store, span) => {
                if !self.store_schemas.contains_key(store) {
                    return Err(format!("unknown store '{store}'"));
                }
                Ok(hir::Stmt::StoreCompact(*store, *span))
            }

            ast::Stmt::StoreSet(store, assignments, filter, span) => {
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
                Ok(hir::Stmt::StoreSet(
                    *store,
                    hassigns,
                    Box::new(hfilter),
                    *span,
                ))
            }

            ast::Stmt::Transaction(body, span) => {
                let hbody = self.lower_block(body, ret_ty)?;
                Ok(hir::Stmt::Transaction(hbody, *span))
            }

            ast::Stmt::Together(name, body, handler, span) => {
                if let Some(n) = name {
                    self.scope_names.push(*n);
                }
                let before: std::collections::BTreeSet<Symbol> =
                    self.current_fn_error_types.clone();
                let hbody = self.lower_block(body, ret_ty);
                if name.is_some() {
                    self.scope_names.pop();
                }
                let hbody = hbody?;
                let errs: Vec<Symbol> = self
                    .current_fn_error_types
                    .difference(&before)
                    .cloned()
                    .collect();

                let hhandler = if handler.ok_arm.is_some() || handler.err_arm.is_some() {
                    let err_ty = errs.first().map(|e| Type::Enum(*e)).unwrap_or(Type::Void);
                    let err_bind = self.fresh_id();

                    let h_ok = if let Some(ok) = &handler.ok_arm {
                        let dollar_id = self.fresh_id();
                        self.dollar_stack.push((dollar_id, Type::Void));
                        self.push_scope();
                        self.define_var(
                            "$",
                            VarInfo {
                                def_id: dollar_id,
                                ty: Type::Void,
                                ownership: Ownership::Owned,
                                scheme: None,
                            },
                        );
                        let he = self.lower_expr(ok);
                        self.pop_scope();
                        self.dollar_stack.pop();
                        Some(vec![hir::Stmt::Expr(he?)])
                    } else {
                        None
                    };

                    let h_err = if let Some(err) = &handler.err_arm {
                        self.push_scope();
                        self.define_var(
                            "err",
                            VarInfo {
                                def_id: err_bind,
                                ty: err_ty.clone(),
                                ownership: Ownership::Owned,
                                scheme: None,
                            },
                        );
                        let he = self.lower_expr(err);
                        self.pop_scope();
                        Some(vec![hir::Stmt::Expr(he?)])
                    } else {
                        None
                    };

                    Some(hir::TogetherHandler {
                        err_bind,
                        err_ty,
                        err_arm: h_err,
                        ok_arm: h_ok,
                    })
                } else {
                    None
                };

                Ok(hir::Stmt::Together(*name, hbody, errs, hhandler, *span))
            }

            ast::Stmt::ChannelClose(ch, span) => {
                let hch = self.lower_expr(ch)?;
                let resolved = self.infer_ctx.shallow_resolve(&hch.ty);
                if !matches!(&resolved, Type::Channel(_) | Type::TypeVar(_)) {
                    return Err(format!("close: target must be a Channel, got {}", hch.ty));
                }
                Ok(hir::Stmt::ChannelClose(hch, *span))
            }

            ast::Stmt::Stop(target, span) => {
                if let ast::Expr::Ident(n, _) = target
                    && self.scope_names.contains(n)
                {
                    return Ok(hir::Stmt::ScopeCancel(*n, *span));
                }
                let htarget = self.lower_expr(target)?;
                if !matches!(&htarget.ty, Type::ActorRef(_)) {
                    return Err(format!(
                        "stop: target must be an ActorRef, got {}",
                        htarget.ty
                    ));
                }
                Ok(hir::Stmt::Stop(htarget, *span))
            }

            ast::Stmt::Join(target, span) => {
                let htarget = self.lower_expr(target)?;
                if !matches!(&htarget.ty, Type::ActorRef(_)) {
                    return Err(format!(
                        "join: target must be an ActorRef, got {}",
                        htarget.ty
                    ));
                }
                Ok(hir::Stmt::Join(htarget, *span))
            }

            ast::Stmt::SimFor(f, span) => {
                let iter = self.lower_expr(&f.iter)?;
                let end = f.end.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                let step = f.step.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                let bind_ty = match &iter.ty {
                    Type::Array(et, _) => *et.clone(),
                    Type::Vec(et) => *et.clone(),
                    _ => {
                        if end.is_some() {
                            Type::I64
                        } else {
                            self.infer_ctx.fresh_var()
                        }
                    }
                };
                let bind_id = self.fresh_id();
                let outer_ids = self.in_scope_def_ids();
                self.push_scope();
                self.define_var(
                    &f.bind.as_str(),
                    VarInfo {
                        def_id: bind_id,
                        ty: bind_ty.clone(),
                        ownership: Ownership::Owned,
                        scheme: None,
                    },
                );

                let pre_loop = self.snapshot_moved_fields();
                let mut body = self.lower_block_no_scope(&f.body, ret_ty)?;
                self.check_loop_body_moves(&pre_loop, &outer_ids, *span)?;
                /* M8 (task 8-8): every `sim for` iteration is its own
                 * concurrent task, and an aggregate moves into at most
                 * one task — so capturing one here is a hard error, not
                 * a mark (there is no single task to own it). */
                if let Some((_, name)) = self
                    .collect_aggregate_captures(&body, &outer_ids)
                    .into_iter()
                    .next()
                {
                    return Err(format!(
                        "{}: `{}` cannot be captured by `sim for` — every iteration \
                         is a concurrent task and an aggregate moves into at most \
                         one task; give each iteration its own value and merge \
                         results over a channel, let a single actor own it, or \
                         capture a clone (`copy {}`)",
                        span.loc(),
                        name,
                        name,
                    ));
                }
                self.finalize_loop_body_drops(&mut body);
                self.pop_scope();
                self.restore_moved_fields(pre_loop);
                Ok(hir::Stmt::SimFor(
                    hir::For {
                        bind_id,
                        bind: f.bind,
                        bind_ty,
                        bind2_id: None,
                        bind2: None,
                        bind2_ty: None,
                        iter,
                        end,
                        step,
                        body,
                        label: f.label,
                        access_mod: None,
                        span: f.span,
                    },
                    *span,
                ))
            }
            ast::Stmt::SimBlock(body, span) => {
                let hbody = self.lower_block_no_scope(body, ret_ty)?;
                Ok(hir::Stmt::SimBlock(hbody, *span))
            }
            ast::Stmt::UseLocal(u) => {
                if u.path.len() == 1 {
                    self.resolve_scoped_use(u.path[0])?;
                } else if u.path.len() > 1 {
                    self.resolve_scoped_path_use(&u.path)?;
                }
                Ok(hir::Stmt::UseLocal(
                    u.path.clone(),
                    u.imports.clone(),
                    u.alias,
                    u.span,
                ))
            }
        }
    }
}
