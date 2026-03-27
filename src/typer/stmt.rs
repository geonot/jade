use crate::ast;
use crate::hir::{self, DefId, Ownership};
use crate::types::{Scheme, Type};

use super::{Typer, VarInfo};

impl Typer {
    pub(crate) fn lower_stmt(
        &mut self,
        stmt: &ast::Stmt,
        ret_ty: &Type,
    ) -> Result<hir::Stmt, String> {
        match stmt {
            ast::Stmt::Bind(b) => {
                let value = if let Some(ref ann) = b.ty {
                    let ann_ty = self.resolve_ty(ann.clone());
                    self.lower_expr_expected(&b.value, Some(&ann_ty))?
                } else if let Some(existing) = self.find_var(&b.name) {
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
                let ownership = Self::ownership_for_type(&ty);
                let id = self.fresh_id();
                if let Some(existing) = self.find_var(&b.name) {
                    let id = existing.def_id;
                    let existing_ty = existing.ty.clone();
                    let value = self.maybe_coerce_to(value, &existing_ty);
                    self.update_var(
                        &b.name,
                        VarInfo {
                            def_id: id,
                            ty: existing_ty.clone(),
                            ownership,
                            scheme: None,
                        },
                    );
                    Ok(hir::Stmt::Bind(hir::Bind {
                        def_id: id,
                        name: b.name.clone(),
                        value,
                        ty: existing_ty,
                        ownership,
                        span: b.span,
                    }))
                } else {
                    // Let-generalization with value restriction:
                    // generalize if the RHS is a syntactic value (no side effects)
                    let scheme = if Self::is_syntactic_value(&b.value) {
                        self.generalize(&ty)
                    } else {
                        Scheme::mono(ty.clone())
                    };
                    // After let-generalization, defer defaulting of quantified TypeVars
                    // so that later statements in the same block can solve them via
                    // unification (e.g., `let f = *fn(x) x; greet(f, 'hello')` where
                    // greet expects (String) -> String — the call solves f's TypeVar
                    // to String). We default remaining unsolved vars at block end.
                    if scheme.is_poly() {
                        // R3.1: Mark quantified vars so they don't trigger strict-mode
                        // errors at the definition site (polymorphic — solved at call site).
                        self.infer_ctx.mark_quantified(&scheme.quantified);
                        self.deferred_quantified_vars
                            .extend(scheme.quantified.iter().copied());
                        // Store the lambda AST for poly-scheme let-bound lambdas so
                        // we can re-lower (monomorphize) at each call site with the
                        // resolved concrete types. This fixes polymorphic multi-use
                        // at codegen: id(42) and id("hello") each get separate copies.
                        if let ast::Expr::Lambda(params, ret, body, lspan) = &b.value {
                            self.poly_lambda_asts.insert(
                                b.name.clone(),
                                (params.clone(), ret.clone(), body.clone(), *lspan),
                            );
                        }
                    }
                    self.define_var(
                        &b.name,
                        VarInfo {
                            def_id: id,
                            ty: ty.clone(),
                            ownership,
                            scheme: Some(scheme),
                        },
                    );
                    Ok(hir::Stmt::Bind(hir::Bind {
                        def_id: id,
                        name: b.name.clone(),
                        value,
                        ty,
                        ownership,
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
                let bindings: Vec<(DefId, String, Type)> = names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| {
                        let ty = tys
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| self.infer_ctx.fresh_var());
                        let id = self.fresh_id();
                        self.define_var(
                            n,
                            VarInfo {
                                def_id: id,
                                ty: ty.clone(),
                                ownership: Self::ownership_for_type(&ty),
                                scheme: None,
                            },
                        );
                        (id, n.clone(), ty)
                    })
                    .collect();
                Ok(hir::Stmt::TupleBind(bindings, hval, *span))
            }

            ast::Stmt::Assign(target, value, span) => {
                let ht = self.lower_expr(target)?;
                let hv = self.lower_expr_expected(value, Some(&ht.ty))?;
                let r = self.infer_ctx.unify_at(&ht.ty, &hv.ty, *span, "assignment");
                self.collect_unify_error(r);
                let hv = self.maybe_coerce_to(hv, &ht.ty);
                Ok(hir::Stmt::Assign(ht, hv, *span))
            }

            ast::Stmt::Expr(e) => {
                let he = self.lower_expr(e)?;
                Ok(hir::Stmt::Expr(he))
            }

            ast::Stmt::If(i) => {
                let hi = self.lower_if(i, ret_ty)?;
                Ok(hir::Stmt::If(hi))
            }

            ast::Stmt::While(w) => {
                let cond = self.lower_expr_expected(&w.cond, Some(&Type::Bool))?;
                let body = self.lower_block(&w.body, ret_ty)?;
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
                let iter_is_int = resolved_iter_ty.is_int()
                    || if let Type::TypeVar(id) = &resolved_iter_ty {
                        let c = self.infer_ctx.constraint(*id);
                        matches!(
                            c,
                            super::unify::TypeConstraint::Integer
                                | super::unify::TypeConstraint::Numeric
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
                            if let Type::Struct(tn) = iter_ty {
                                if self.type_implements_trait(&tn, "Iter") {
                                    let elem_ty = self.iter_element_type(&tn);
                                    return self.desugar_for_iter(f, iter, tn, elem_ty, ret_ty);
                                }
                            }
                            self.infer_ctx.fresh_var()
                        }
                    }
                };
                let bind_id = self.fresh_id();
                self.push_scope();
                self.define_var(
                    &f.bind,
                    VarInfo {
                        def_id: bind_id,
                        ty: bind_ty.clone(),
                        ownership: Ownership::Owned,
                        scheme: None,
                    },
                );
                let body = self.lower_block_no_scope(&f.body, ret_ty)?;
                self.pop_scope();
                Ok(hir::Stmt::For(hir::For {
                    bind_id,
                    bind: f.bind.clone(),
                    bind_ty,
                    iter,
                    end,
                    step,
                    body,
                    span: f.span,
                }))
            }

            ast::Stmt::Loop(l) => {
                let body = self.lower_block(&l.body, ret_ty)?;
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
                let he = self.lower_expr(e)?;
                Ok(hir::Stmt::ErrReturn(he, ret_ty.clone(), *span))
            }

            ast::Stmt::StoreInsert(store, values, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                if values.len() != schema.len() {
                    return Err(format!(
                        "store '{store}' has {} fields but {} values given",
                        schema.len(),
                        values.len()
                    ));
                }
                let mut hvalues = Vec::new();
                for (v, (_fname, fty)) in values.iter().zip(schema.iter()) {
                    hvalues.push(self.lower_expr_expected(v, Some(fty))?);
                }
                Ok(hir::Stmt::StoreInsert(store.clone(), hvalues, *span))
            }

            ast::Stmt::StoreDelete(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, store)?;
                Ok(hir::Stmt::StoreDelete(
                    store.clone(),
                    Box::new(hfilter),
                    *span,
                ))
            }

            ast::Stmt::StoreSet(store, assignments, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, store)?;
                let mut hassigns = Vec::new();
                for (fname, fval) in assignments {
                    if let Some((_, fty)) = schema.iter().find(|(n, _)| n == fname) {
                        hassigns.push((fname.clone(), self.lower_expr_expected(fval, Some(fty))?));
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
        }
    }

    pub(crate) fn lower_store_filter(
        &mut self,
        filter: &ast::StoreFilter,
        schema: &[(String, Type)],
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

    pub(crate) fn lower_block_no_scope(
        &mut self,
        block: &ast::Block,
        ret_ty: &Type,
    ) -> Result<hir::Block, String> {
        let deferred_snapshot = self.deferred_quantified_vars.len();
        let mut stmts = Vec::new();
        let block_len = block.len();
        for (idx, s) in block.iter().enumerate() {
            // Propagate expected type into tail expressions
            if idx == block_len - 1 {
                if let crate::ast::Stmt::Expr(e) = s {
                    let he = self.lower_expr_expected(e, Some(ret_ty))?;
                    stmts.push(hir::Stmt::Expr(he));
                    continue;
                }
            }
            stmts.push(self.lower_stmt(s, ret_ty)?);
        }
        // Default deferred quantified vars from this block scope
        if self.deferred_quantified_vars.len() > deferred_snapshot {
            let vars_to_default: Vec<u32> = self
                .deferred_quantified_vars
                .drain(deferred_snapshot..)
                .collect();
            self.infer_ctx.default_quantified_vars(&vars_to_default);
        }
        Ok(stmts)
    }

    pub(crate) fn lower_if(&mut self, i: &ast::If, ret_ty: &Type) -> Result<hir::If, String> {
        let cond = self.lower_expr_expected(&i.cond, Some(&Type::Bool))?;
        let then = self.lower_block(&i.then, ret_ty)?;
        let mut elifs = Vec::new();
        for (ec, eb) in &i.elifs {
            let hc = self.lower_expr_expected(ec, Some(&Type::Bool))?;
            let hb = self.lower_block(eb, ret_ty)?;
            elifs.push((hc, hb));
        }
        let els = i
            .els
            .as_ref()
            .map(|b| self.lower_block(b, ret_ty))
            .transpose()?;
        Ok(hir::If {
            cond,
            then,
            elifs,
            els,
            span: i.span,
        })
    }

    pub(crate) fn lower_match(
        &mut self,
        m: &ast::Match,
        ret_ty: &Type,
    ) -> Result<hir::Match, String> {
        let subject = self.lower_expr(&m.subject)?;
        let subj_ty = subject.ty.clone();
        let mut arms = Vec::new();
        // R2.3: Unify all arm tail expression types so match arms produce
        // consistent types. Only track if arms actually have tail expressions.
        let mut first_arm_ty: Option<Type> = None;
        for a in &m.arms {
            self.push_scope();
            let pat = self.lower_pat(&a.pat, &subj_ty)?;
            let guard = a
                .guard
                .as_ref()
                .map(|g| self.lower_expr_expected(g, Some(&Type::Bool)))
                .transpose()?;
            let body = self.lower_block_no_scope(&a.body, ret_ty)?;
            // Unify each arm's tail expression type with other arms
            if let Some(hir::Stmt::Expr(tail_expr)) = body.last() {
                if let Some(ref first_ty) = first_arm_ty {
                    let _ = self.infer_ctx.unify_at(
                        first_ty,
                        &tail_expr.ty,
                        a.span,
                        "match arm result type",
                    );
                } else {
                    first_arm_ty = Some(tail_expr.ty.clone());
                }
            }
            self.pop_scope();
            arms.push(hir::Arm {
                pat,
                guard,
                body,
                span: a.span,
            });
        }
        // Resolve TypeVars in subject type before exhaustiveness check
        let resolved_subj_ty = self.infer_ctx.resolve(&subj_ty);
        // Use the unified arm type if available, otherwise the subject type
        let result_ty = first_arm_ty
            .map(|t| self.infer_ctx.shallow_resolve(&t))
            .unwrap_or_else(|| subj_ty.clone());
        let result = hir::Match {
            subject,
            arms,
            ty: result_ty,
            span: m.span,
        };

        self.check_exhaustiveness(&resolved_subj_ty, &result.arms, m.span)?;

        Ok(result)
    }

    pub(crate) fn lower_pat(
        &mut self,
        pat: &ast::Pat,
        expected_ty: &Type,
    ) -> Result<hir::Pat, String> {
        match pat {
            ast::Pat::Wild(span) => Ok(hir::Pat::Wild(*span)),
            ast::Pat::Ident(name, span) => {
                if let Some((en, tag)) = self.variant_tags.get(name).cloned() {
                    // S6: Unify expected type with the enum type for zero-arg variants
                    let enum_ty = Type::Enum(en.clone());
                    let _ = self.infer_ctx.unify_at(
                        expected_ty,
                        &enum_ty,
                        *span,
                        "match pattern implies enum type",
                    );
                    return Ok(hir::Pat::Ctor(name.clone(), tag, vec![], *span));
                }
                let id = self.fresh_id();
                let ty = expected_ty.clone();
                self.define_var(
                    name,
                    VarInfo {
                        def_id: id,
                        ty: ty.clone(),
                        ownership: Self::ownership_for_type(&ty),
                        scheme: None,
                    },
                );
                Ok(hir::Pat::Bind(id, name.clone(), ty, *span))
            }
            ast::Pat::Lit(e) => {
                let he = self.lower_expr(e)?;
                Ok(hir::Pat::Lit(he))
            }
            ast::Pat::Ctor(name, sub_pats, span) => {
                let tag = self.variant_tags.get(name).map(|(_, t)| *t).unwrap_or(0);

                let enum_name = self.variant_tags.get(name).map(|(en, _)| en.clone());

                // S6: Unify the expected type (match subject) with the enum type
                // so that unannotated function params get their type from match patterns.
                if let Some(ref en) = enum_name {
                    let enum_ty = Type::Enum(en.clone());
                    let _ = self.infer_ctx.unify_at(
                        expected_ty,
                        &enum_ty,
                        *span,
                        "match pattern implies enum type",
                    );
                }

                let field_tys: Vec<Type> = if let Some(ref en) = enum_name {
                    if let Some(variants) = self.enums.get(en) {
                        variants
                            .iter()
                            .find(|(vn, _)| vn == name)
                            .map(|(_, ftys)| ftys.clone())
                            .unwrap_or_default()
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                };

                let mut hpats = Vec::new();
                for (i, sp) in sub_pats.iter().enumerate() {
                    let ft = field_tys
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| self.infer_ctx.fresh_var());
                    hpats.push(self.lower_pat(sp, &ft)?);
                }
                Ok(hir::Pat::Ctor(name.clone(), tag, hpats, *span))
            }
            ast::Pat::Or(pats, span) => {
                let mut hpats = Vec::new();
                for p in pats {
                    hpats.push(self.lower_pat(p, expected_ty)?);
                }
                Ok(hir::Pat::Or(hpats, *span))
            }
            ast::Pat::Range(lo, hi, span) => {
                let hlo = self.lower_expr(lo)?;
                let hhi = self.lower_expr(hi)?;
                Ok(hir::Pat::Range(Box::new(hlo), Box::new(hhi), *span))
            }
            ast::Pat::Tuple(pats, span) => {
                let tys = match expected_ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => (0..pats.len())
                        .map(|_| self.infer_ctx.fresh_var())
                        .collect(),
                };
                let mut hpats = Vec::new();
                for (i, p) in pats.iter().enumerate() {
                    let ety = tys
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| self.infer_ctx.fresh_var());
                    hpats.push(self.lower_pat(p, &ety)?);
                }
                Ok(hir::Pat::Tuple(hpats, *span))
            }
            ast::Pat::Array(pats, span) => {
                let elem_ty = match expected_ty {
                    Type::Array(et, _) => et.as_ref().clone(),
                    _ => self.infer_ctx.fresh_var(),
                };
                let mut hpats = Vec::new();
                for p in pats {
                    hpats.push(self.lower_pat(p, &elem_ty)?);
                }
                Ok(hir::Pat::Array(hpats, *span))
            }
        }
    }
}
