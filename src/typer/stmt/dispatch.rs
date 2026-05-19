use crate::ast;
use crate::hir::{self, DefId, Ownership};
use crate::intern::Symbol;
use crate::types::{Scheme, Type};

use super::super::{Typer, VarInfo};

impl Typer {
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
                        let field_expr =
                            ast::Expr::Field(Box::new(self_expr), b.name.clone(), b.span);
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

                if self.find_var(&b.name.as_str()).is_none() {
                    if let Some((_gexpr, _gspan)) = self.globals.get(&b.name).cloned() {
                        let init_hir = self.lower_expr(&_gexpr)?;
                        let global_ty = init_hir.ty.clone();
                        let hv = self.lower_expr_expected(&b.value, Some(&global_ty))?;
                        return Ok(hir::Stmt::GlobalStore(b.name.clone(), hv, b.span));
                    }
                }
                let value = if let Some(ref ann) = b.ty {
                    let ann_ty = self.resolve_ty(ann.clone());
                    self.lower_expr_expected(&b.value, Some(&ann_ty))?
                } else if let Some(existing) = self.find_var(&b.name.as_str()) {
                    self.lower_expr_expected(&b.value, Some(&existing.ty.clone()))?
                } else {
                    self.lower_expr(&b.value)?
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

                if Self::is_aliased_read_of_heap(&value) && !ty.is_value_clonable() {
                    ownership = Ownership::Borrowed;
                }

                if b.access_mod.is_some() {
                    ownership = self.ownership_with_mod(&ty, b.access_mod)?;
                }

                let partial_move: Option<(DefId, Symbol)> =
                    if matches!(b.access_mod, Some(ast::AccessMod::Take))
                        && let hir::ExprKind::Field(parent, field, _) = &value.kind
                        && let hir::ExprKind::Var(parent_id, _) = &parent.kind
                    {
                        Some((*parent_id, field.clone()))
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
                        name: b.name.clone(),
                        value,
                        ty: existing_ty,
                        ownership,
                        atomic: b.atomic,
                        access_mod: b.access_mod,
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
                                b.name.clone(),
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

                    if matches!(b.access_mod, Some(ast::AccessMod::Const)) {
                        self.const_vars.insert(id);
                    }
                    if let Some((pid, fname)) = partial_move {
                        self.mark_field_moved(pid, fname);
                    }
                    Ok(hir::Stmt::Bind(hir::Bind {
                        def_id: id,
                        name: b.name.clone(),
                        value,
                        ty,
                        ownership,
                        atomic: b.atomic,
                        access_mod: b.access_mod,
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
                if let ast::Expr::Field(obj, field, fspan) = target {
                    if let ast::Expr::Ident(row_name, _) = obj.as_ref() {
                        let probe = self.lower_expr(obj.as_ref())?;
                        let probe_ty = self.infer_ctx.shallow_resolve(&probe.ty);
                        if let Type::Row(store) = &probe_ty {
                            let store = store.clone();
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
                            let r = self.infer_ctx.unify_at(
                                &fty,
                                &hv.ty,
                                *span,
                                "row field assignment",
                            );
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
                }

                self.suppress_moved_field_check += 1;
                let ht = self.lower_expr(target)?;
                self.suppress_moved_field_check -= 1;
                let hv = self.lower_expr_expected(value, Some(&ht.ty))?;
                let r = self.infer_ctx.unify_at(&ht.ty, &hv.ty, *span, "assignment");
                self.collect_unify_error(r);
                let hv = self.maybe_coerce_to(hv, &ht.ty);

                if let hir::ExprKind::Field(parent, field, _) = &ht.kind {
                    if let hir::ExprKind::Var(parent_id, _) = &parent.kind {
                        self.clear_field_moved(*parent_id, field);
                    }
                }
                Ok(hir::Stmt::Assign(ht, hv, *span))
            }

            ast::Stmt::Expr(e) => {
                if let ast::Expr::Query(source, clauses, span) = e {
                    let store_name = match source.as_ref() {
                        ast::Expr::Ident(name, _) => name.clone(),
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
                Ok(hir::Stmt::Expr(he))
            }

            ast::Stmt::If(i) => {
                let hi = self.lower_if(i, ret_ty)?;
                Ok(hir::Stmt::If(hi))
            }

            ast::Stmt::While(w) => {
                let cond = self.lower_expr_expected(&w.cond, Some(&Type::Bool))?;

                let pre = self.snapshot_moved_fields();
                let body = self.lower_block(&w.body, ret_ty)?;
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
                            if let Type::Struct(tn, _) = iter_ty {
                                if self.type_implements_trait(&tn.as_str(), "Iter") {
                                    let elem_ty = self.iter_element_type(&tn.as_str());
                                    return self.desugar_for_iter(
                                        f,
                                        iter,
                                        tn.as_str(),
                                        elem_ty,
                                        ret_ty,
                                    );
                                }
                            }
                            self.infer_ctx.fresh_var()
                        }
                    }
                };
                let bind_id = self.fresh_id();
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
                    (Some(id2), Some(b2.clone()), Some(Type::I64))
                } else {
                    (None, None, None)
                };

                let pre_loop = self.snapshot_moved_fields();
                let body = self.lower_block_no_scope(&f.body, ret_ty)?;
                self.pop_scope();
                self.restore_moved_fields(pre_loop);
                Ok(hir::Stmt::For(hir::For {
                    bind_id,
                    bind: f.bind.clone(),
                    bind_ty,
                    bind2_id,
                    bind2,
                    bind2_ty,
                    iter,
                    end,
                    step,
                    body,
                    label: f.label.clone(),
                    access_mod: f.access_mod,
                    span: f.span,
                }))
            }
            ast::Stmt::Loop(l) => {
                let pre = self.snapshot_moved_fields();
                let body = self.lower_block(&l.body, ret_ty)?;
                self.restore_moved_fields(pre);
                Ok(hir::Stmt::Loop(hir::Loop { body, span: l.span }))
            }

            ast::Stmt::Ret(val, span) => {
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
                let he = self.lower_expr_expected(e, Some(ret_ty))?;

                let resolved = self.infer_ctx.resolve(&he.ty);
                let enum_name: Option<Symbol> = match &resolved {
                    Type::Enum(n) if self.err_enum_names.contains(n) => Some(n.clone()),
                    Type::Struct(n, _) if self.err_enum_names.contains(n) => Some(n.clone()),
                    _ => None,
                };
                if let Some(en) = &enum_name {
                    if !self.current_fn_declared_errors.is_empty()
                        && !self.current_fn_declared_errors.contains(en)
                    {
                        return Err(format!(
                            "`! {0}` returns a variant of err `{0}` at {1:?}, but the function's declared error union (`! ...`) does not list `{0}`. Add `! {0}` to the function signature, or use a different value.",
                            en, span
                        ));
                    }
                    self.current_fn_error_types.insert(en.clone());
                }

                let resolved_ret = self.infer_ctx.resolve(ret_ty);
                let normalized_ret = match &resolved_ret {
                    Type::Struct(n, args) if args.is_empty() && self.enums.contains_key(n) => {
                        Type::Enum(n.clone())
                    }
                    _ => resolved_ret.clone(),
                };
                let resolved_val = self.infer_ctx.resolve(&he.ty);
                let normalized_val = match &resolved_val {
                    Type::Struct(n, args) if args.is_empty() && self.enums.contains_key(n) => {
                        Type::Enum(n.clone())
                    }
                    _ => resolved_val.clone(),
                };
                let unify_res = self.infer_ctx.unify_at(
                    &normalized_val,
                    &normalized_ret,
                    *span,
                    "early-return value (`!`)",
                );
                if let Err(_) = &unify_res {
                    if let Some(en) = &enum_name {
                        return Err(format!(
                            "`! {0}` at {1:?} returns a value of err `{0}`, but this function returns `{2}`. In jinn, errors are values: either declare the function as `returns {0}` and pattern-match at the call site, or encode the error as a value of `{2}` (e.g., a sentinel like `! -1`).",
                            en, span, resolved_ret
                        ));
                    }
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
                            .ok_or_else(|| {
                                format!("store '{store}' insert: missing field '{fname}'")
                            })?;
                        hvalues.push(self.lower_expr_expected(&fi.value, Some(fty))?);
                    }

                    let mut seen = std::collections::HashSet::new();
                    for fi in values {
                        let n = fi.name.as_ref().unwrap();
                        if !user_schema.iter().any(|(sn, _)| sn == n) {
                            return Err(format!("store '{store}' has no field '{n}'"));
                        }
                        if !seen.insert(n.clone()) {
                            return Err(format!(
                                "store '{store}' insert: field '{n}' \
                                 specified twice"
                            ));
                        }
                    }
                    return Ok(hir::Stmt::StoreInsert(store.clone(), hvalues, *span));
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
                Ok(hir::Stmt::StoreInsert(store.clone(), hvalues, *span))
            }

            ast::Stmt::StoreDelete(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, &store.as_str())?;
                Ok(hir::Stmt::StoreDelete(
                    store.clone(),
                    Box::new(hfilter),
                    *span,
                ))
            }

            ast::Stmt::StoreDestroy(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, &store.as_str())?;
                Ok(hir::Stmt::StoreDestroy(
                    store.clone(),
                    Box::new(hfilter),
                    *span,
                ))
            }

            ast::Stmt::StoreRestore(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, &store.as_str())?;
                Ok(hir::Stmt::StoreRestore(
                    store.clone(),
                    Box::new(hfilter),
                    *span,
                ))
            }

            ast::Stmt::StoreSave(store, span) => {
                if !self.store_schemas.contains_key(store) {
                    return Err(format!("unknown store '{store}'"));
                }
                Ok(hir::Stmt::StoreSave(store.clone(), *span))
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
                    store.clone(),
                    hassigns,
                    Box::new(hfilter),
                    *span,
                ))
            }

            ast::Stmt::Transaction(body, span) => {
                let hbody = self.lower_block(body, ret_ty)?;
                Ok(hir::Stmt::Transaction(hbody, *span))
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
                let htarget = self.lower_expr(target)?;
                if !matches!(&htarget.ty, Type::ActorRef(_)) {
                    return Err(format!(
                        "stop: target must be an ActorRef, got {}",
                        htarget.ty
                    ));
                }
                Ok(hir::Stmt::Stop(htarget, *span))
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
                let body = self.lower_block_no_scope(&f.body, ret_ty)?;
                self.pop_scope();
                self.restore_moved_fields(pre_loop);
                Ok(hir::Stmt::SimFor(
                    hir::For {
                        bind_id,
                        bind: f.bind.clone(),
                        bind_ty,
                        bind2_id: None,
                        bind2: None,
                        bind2_ty: None,
                        iter,
                        end,
                        step,
                        body,
                        label: f.label.clone(),
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
            ast::Stmt::UseLocal(u) => Ok(hir::Stmt::UseLocal(
                u.path.clone(),
                u.imports.clone(),
                u.alias.clone(),
                u.span,
            )),
        }
    }
}
