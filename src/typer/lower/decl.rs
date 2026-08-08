use super::super::{Typer, VarInfo};
use crate::ast::{self, Span};
use crate::hir::{self, DefId, Ownership};
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn lower_actor_def(
        &mut self,
        ad: &ast::ActorDef,
    ) -> Result<hir::ActorDef, String> {
        let (id, ref declared_fields, ref handler_info) = self
            .actors
            .get(&ad.name)
            .ok_or_else(|| format!("undeclared actor: {}", ad.name))?
            .clone();

        let fields: Vec<hir::Field> = ad
            .fields
            .iter()
            .map(|f| {
                let ty = declared_fields
                    .iter()
                    .find(|(n, _)| n == &f.name)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)));
                let default = f.default.as_ref().map(|e| {
                    self.lower_expr_expected(e, Some(&ty))
                        .unwrap_or_else(|_| hir::Expr {
                            kind: hir::ExprKind::Int(0),
                            ty: Type::I64,
                            span: e.span(),
                        })
                });
                hir::Field {
                    name: f.name,
                    ty,
                    default,
                    access_mod: f.access_mod,
                    span: f.span,
                }
            })
            .collect();

        let mut hir_handlers = Vec::new();

        let field_def_ids: Vec<crate::hir::DefId> =
            fields.iter().map(|_| self.fresh_id()).collect();
        for (i, h) in ad.handlers.iter().enumerate() {
            self.push_scope();
            for (f, &fid) in fields.iter().zip(field_def_ids.iter()) {
                self.define_var(
                    &f.name.as_str(),
                    VarInfo {
                        def_id: fid,
                        ty: f.ty.clone(),
                        ownership: Ownership::Owned,
                        scheme: None,
                    },
                );
            }
            let mut params = Vec::new();
            let declared_ptys = &handler_info[i].1;
            if h.is_loop && !h.params.is_empty() {
                return Err(format!(
                    "{}: *loop handler cannot declare parameters",
                    h.span.loc()
                ));
            }
            for (pi, p) in h.params.iter().enumerate() {
                let pid = self.fresh_id();
                let actor_names: std::collections::HashSet<Symbol> =
                    self.actors.keys().cloned().collect();
                let ty =
                    p.ty.clone()
                        .map(|t| Self::normalize_actor_refs(t, &actor_names))
                        .unwrap_or_else(|| {
                            declared_ptys
                                .get(pi)
                                .map(|t| self.infer_ctx.resolve(t))
                                .unwrap_or(Type::I64)
                        });
                let ownership = self
                    .param_ownership_with_mod(&ty, p.access_mod)
                    .unwrap_or_else(|_| Self::ownership_for_type(&ty));
                self.define_var(
                    &p.name.as_str(),
                    VarInfo {
                        def_id: pid,
                        ty: ty.clone(),
                        ownership,
                        scheme: None,
                    },
                );
                params.push(hir::Param {
                    def_id: pid,
                    name: p.name,
                    ty,
                    ownership,
                    default: None,
                    access_mod: p.access_mod,
                    span: p.span,
                });
            }
            let loop_sleep_ms = if h.is_loop {
                h.loop_sleep_ms
                    .as_ref()
                    .map(|e| self.lower_expr_expected(e, Some(&Type::I64)))
                    .transpose()?
            } else {
                None
            };
            let body = self.lower_block(&h.body, &Type::Void)?;
            self.pop_scope();
            hir_handlers.push(hir::HandlerDef {
                name: h.name,
                params,
                is_loop: h.is_loop,
                loop_sleep_ms,
                body,
                tag: handler_info[i].2,
                span: h.span,
            });
        }

        Ok(hir::ActorDef {
            def_id: id,
            name: ad.name,
            fields,
            field_def_ids,
            handlers: hir_handlers,
            span: ad.span,
        })
    }

    pub(in crate::typer) fn lower_store_def(
        &mut self,
        sd: &ast::StoreDef,
    ) -> Result<hir::StoreDef, String> {
        let id = self.fresh_id();
        let is_simple = sd.decorators.contains(&ast::StoreDecorator::Simple);
        let dummy_span = ast::Span::dummy();

        let mut fields: Vec<hir::StoreField> = Vec::new();

        if !is_simple {
            let builtin = |name: &str, ty: Type| hir::StoreField {
                name: name.into(),
                ty,
                default: None,
                decorators: vec![],
                is_relation: false,
                is_has_many: false,
                span: dummy_span,
            };
            fields.push(builtin("sid", Type::I64));
            fields.push(builtin("uuid", Type::String));
            fields.push(builtin("hash", Type::String));
            fields.push(builtin("created", Type::I64));
            fields.push(builtin("updated", Type::I64));
            fields.push(builtin("deleted", Type::I64));
        }

        let is_versioned = sd.decorators.contains(&ast::StoreDecorator::Versioned);
        if is_versioned {
            fields.push(hir::StoreField {
                name: "__version".into(),
                ty: Type::I64,
                default: None,
                decorators: vec![],
                is_relation: false,
                is_has_many: false,
                span: dummy_span,
            });
        }
        for f in &sd.fields {
            if f.is_relation && f.is_has_many {
                continue;
            }
            let ty = if f.is_relation {
                Type::I64
            } else {
                f.ty.clone().unwrap_or(Type::I64)
            };
            crate::store_decorators::validate_field_decorators(
                &sd.name.as_str(),
                &f.name.as_str(),
                &ty,
                &f.decorators,
            )?;
            fields.push(hir::StoreField {
                name: f.name,
                ty,
                default: None,
                decorators: f.decorators.clone(),
                is_relation: f.is_relation,
                is_has_many: f.is_has_many,
                span: f.span,
            });
        }
        let mut hir_methods = Vec::new();
        for m in &sd.methods {
            let hm = self.lower_method_by_ptr(&sd.name.as_str(), m)?;
            hir_methods.push(hm);
        }
        Ok(hir::StoreDef {
            def_id: id,
            name: sd.name,
            decorators: sd.decorators.clone(),
            fields,
            methods: hir_methods,
            span: sd.span,
        })
    }

    pub(in crate::typer) fn lower_impl_block(
        &mut self,
        ib: &ast::ImplBlock,
    ) -> Result<hir::TraitImpl, String> {
        let is_static_trait = ib.trait_name.map(|t| t.as_str() == "From").unwrap_or(false);
        let mut hir_methods = Vec::new();
        for m in &ib.methods {
            let takes_self = m.params.first().map(|p| p.name == "self").unwrap_or(false);
            let hm = if is_static_trait && !takes_self {
                self.lower_static_method(&ib.type_name.as_str(), m)?
            } else {
                self.lower_method_by_ptr(&ib.type_name.as_str(), m)?
            };
            hir_methods.push(hm);
        }
        if let Some(trait_name) = ib.trait_name
            && let Some(synthesized) = self
                .trait_default_methods
                .get(&(ib.type_name, trait_name))
                .cloned()
        {
            for m in &synthesized {
                let hm = self.lower_method_by_ptr(&ib.type_name.as_str(), m)?;
                hir_methods.push(hm);
            }
        }
        Ok(hir::TraitImpl {
            trait_name: ib.trait_name,
            trait_type_args: ib.trait_type_args.clone(),
            type_name: ib.type_name,
            methods: hir_methods,
            span: ib.span,
        })
    }

    pub(in crate::typer) fn build_fn_scheme(&mut self, name: Symbol, hfn: &hir::Fn) {
        let param_tys: Vec<crate::types::Type> = hfn
            .params
            .iter()
            .map(|p| self.infer_ctx.canonicalize_type(&p.ty))
            .collect();
        let ret_ty = self.infer_ctx.canonicalize_type(&hfn.ret);
        let fn_ty = crate::types::Type::Fn(param_tys.clone(), Box::new(ret_ty.clone()));
        let scheme = self.generalize(&fn_ty);
        if scheme.is_poly() {
            self.infer_ctx.mark_quantified(&scheme.quantified);
        }
        if self.debug_types && scheme.is_poly() {
            tracing::debug!(
                target: "jinnc::type",
                "scheme {} :: ∀{:?}. ({}) -> {}",
                name,
                scheme.quantified,
                param_tys
                    .iter()
                    .map(|t| format!("{t}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                ret_ty
            );
        }
        self.fn_schemes
            .insert(name, (scheme.quantified, param_tys, ret_ty));
    }

    pub(in crate::typer) fn lower_fn(&mut self, f: &ast::Fn) -> Result<hir::Fn, String> {
        let suppress =
            self.inferable_fns.contains_key(&f.name) && !self.infer_ctx.suppress_unsolved_reports;
        if suppress {
            self.infer_ctx.suppress_unsolved_reports = true;
        }
        let out = self.lower_fn_inner(f);
        if suppress {
            self.infer_ctx.suppress_unsolved_reports = false;
        }
        out
    }

    fn lower_fn_inner(&mut self, f: &ast::Fn) -> Result<hir::Fn, String> {
        let mut hfn = self.lower_fn_deferred(f)?;

        let einfo = crate::escape::analyze_fn(&hfn);
        for (id, t) in einfo.iter() {
            self.escape_tiers.insert(*id, *t);
        }

        let _demoted = crate::escape::apply_demotions(&mut hfn, &einfo);
        Ok(hfn)
    }

    pub(in crate::typer) fn lower_fn_deferred(&mut self, f: &ast::Fn) -> Result<hir::Fn, String> {
        let (id, ptys, ret) = self
            .fns
            .get(&f.name)
            .ok_or_else(|| format!("undeclared function: {}", f.name))?
            .clone();

        self.push_scope();
        let mut params = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            let pid = self.fresh_id();
            let ty = ptys[i].clone();

            let eff_mod = self
                .fn_param_access
                .get(&f.name)
                .and_then(|a| a.get(i).copied())
                .flatten()
                .or(p.access_mod);
            let ownership = self
                .param_ownership_with_mod(&ty, eff_mod)
                .map_err(|e| format!("{}: {e}", p.span.loc()))?;
            self.define_var(
                &p.name.as_str(),
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
                    scheme: None,
                },
            );
            let hir_default = if let Some(ref def_expr) = p.default {
                self.lower_expr(def_expr).ok()
            } else {
                None
            };
            params.push(hir::Param {
                def_id: pid,
                name: p.name,
                ty,
                ownership,
                default: hir_default,
                access_mod: eff_mod,
                span: p.span,
            });
        }
        let prev_param_ids = std::mem::replace(
            &mut self.current_fn_param_ids,
            params.iter().map(|p| p.def_id).collect(),
        );
        let prev_fn_ret = self.current_fn_ret_ty.replace(ret.clone());
        let prev_is_main = self.current_fn_is_main;
        self.current_fn_is_main = f.name.as_str() == "main";

        let prev_inferred = std::mem::take(&mut self.current_fn_error_types);
        let prev_declared = std::mem::take(&mut self.current_fn_declared_errors);

        let mut declared_err_names: Vec<Symbol> = Vec::new();
        for et in &f.error_types {
            let name = match et {
                Type::Enum(n) | Type::Struct(n, _) | Type::Param(n) => Some(*n),
                _ => None,
            };
            match name {
                Some(n) if self.err_enum_names.contains(&n) => {
                    declared_err_names.push(n);
                }
                Some(n) => {
                    return Err(format!(
                        "function '{}' declares error type '{}' which is not an `err` definition",
                        f.name, n
                    ));
                }
                None => {
                    return Err(format!(
                        "function '{}' declares non-enum error type '{:?}'",
                        f.name, et
                    ));
                }
            }
        }
        self.current_fn_declared_errors = declared_err_names.clone();

        let mut body = self.lower_block_no_scope_with_tail(&f.body, &ret, Some(&ret))?;
        self.finalize_block_drops(&mut body);

        for (i, p) in params.iter_mut().enumerate() {
            if !matches!(p.ownership, Ownership::Owned) || p.access_mod.is_some() {
                continue;
            }
            let resolved = self.infer_ctx.resolve(&p.ty);
            if resolved == p.ty {
                continue;
            }
            let eff_mod = self
                .fn_param_access
                .get(&f.name)
                .and_then(|a| a.get(i).copied())
                .flatten()
                .or(p.access_mod);
            if eff_mod.is_some() {
                continue;
            }
            if matches!(
                self.param_ownership_with_mod(&resolved, None),
                Ok(Ownership::Borrowed) | Ok(Ownership::BorrowMut)
            ) {
                p.ownership = Ownership::Borrowed;
            }
        }

        let borrowed_param_ids: std::collections::HashSet<crate::hir::DefId> = params
            .iter()
            .filter(|p| {
                matches!(
                    p.ownership,
                    Ownership::Borrowed | Ownership::BorrowMut | Ownership::Raw
                )
            })
            .map(|p| p.def_id)
            .collect();
        if !borrowed_param_ids.is_empty() {
            Self::strip_drops_for(&mut body, &borrowed_param_ids);
        }

        {
            let mut locals: std::collections::HashMap<crate::hir::DefId, (Symbol, Type)> =
                std::collections::HashMap::new();
            Self::collect_local_binds(&body, &mut locals);
            self.check_escaping_lambda_captures(&body, &locals)?;
        }

        let inferred_err: Vec<Symbol> = self.current_fn_error_types.iter().cloned().collect();
        self.last_inferred_errors = self.current_fn_error_types.clone();
        if !declared_err_names.is_empty() {
            self.check_error_soundness(f.name, &inferred_err, &declared_err_names, f.span)?;
        }

        let mut error_types: Vec<Type> = Vec::new();
        let mut seen: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
        let visible: Vec<Symbol> = if declared_err_names.is_empty() {
            inferred_err.clone()
        } else {
            declared_err_names.clone()
        };
        for n in visible {
            if seen.insert(n) {
                error_types.push(Type::Enum(n));
            }
        }

        self.current_fn_param_ids = prev_param_ids;
        self.current_fn_ret_ty = prev_fn_ret;
        self.current_fn_is_main = prev_is_main;
        self.current_fn_error_types = prev_inferred;
        self.current_fn_declared_errors = prev_declared;
        self.pop_scope();

        if f.ret.is_none() && f.name != "main" {
            if let Some(tail_ty) = self.hir_tail_type(&body) {
                let r = self
                    .infer_ctx
                    .unify_at(&ret, &tail_ty, f.span, "function tail expression");
                self.collect_unify_error(r);
            } else {
                let _ = self.infer_ctx.unify(&ret, &Type::Void);
            }
        } else if f.ret.is_some()
            && f.name != "main"
            && let Some(tail_ty) = self.hir_tail_type(&body)
        {
            let rt = self.infer_ctx.shallow_resolve(&ret);
            let tt = self.infer_ctx.shallow_resolve(&tail_ty);
            let r = self
                .infer_ctx
                .unify_at(&ret, &tail_ty, f.span, "function tail expression");
            let rt_lax = matches!(rt, Type::Ptr(_)) || self.infer_ctx.type_has_unresolved(&rt);
            let tt_lax = matches!(tt, Type::Ptr(_)) || self.infer_ctx.type_has_unresolved(&tt);
            if let Err(e) = r
                && !rt_lax
                && !tt_lax
                && !(rt.is_num() && tt.is_num())
            {
                return Err(format!(
                    "{}: function `{}` declares `returns {}` but its body produces a \
                         different type: {e}",
                    f.span.loc(),
                    f.name,
                    ret,
                ));
            }
        }

        let final_body = if f.is_generator && f.name != "main" {
            let body_span = f.span;
            let captures: Vec<(Symbol, Type)> =
                params.iter().map(|p| (p.name, p.ty.clone())).collect();
            let gen_expr = hir::Expr {
                kind: hir::ExprKind::GeneratorCreate(
                    id,
                    f.name,
                    std::mem::take(&mut body),
                    captures,
                ),
                ty: ret.clone(),
                span: body_span,
            };
            vec![hir::Stmt::Ret(Some(gen_expr), ret.clone(), body_span)]
        } else {
            body
        };

        Ok(hir::Fn {
            def_id: id,
            name: f.name,
            params,
            ret: ret.clone(),
            error_types,
            body: final_body,
            span: f.span,
            generic_origin: None,
            is_generator: f.is_generator,
            attrs: f.attrs.clone(),
        })
    }

    pub(in crate::typer) fn lower_test_block(
        &mut self,
        tb: &ast::TestBlock,
        fn_name: &str,
    ) -> Result<hir::Fn, String> {
        let id = self.fresh_id();
        self.push_scope();
        let body = self.lower_block(&tb.body, &Type::Void)?;
        self.pop_scope();
        Ok(hir::Fn {
            def_id: id,
            name: fn_name.into(),
            params: vec![],
            ret: Type::Void,
            error_types: Vec::new(),
            body,
            span: tb.span,
            generic_origin: None,
            is_generator: false,
            attrs: crate::ast::FnAttrs::default(),
        })
    }

    pub(in crate::typer) fn build_test_runner(&mut self, tests: &[(String, String)]) -> hir::Fn {
        let id = self.fresh_id();
        let s = Span::dummy();
        let mut body: hir::Block = Vec::new();
        for (display_name, fn_name) in tests {
            body.push(hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::Builtin(
                    hir::BuiltinFn::Log,
                    vec![hir::Expr {
                        kind: hir::ExprKind::Str(format!("test {display_name} ...")),
                        ty: Type::String,
                        span: s,
                    }],
                ),
                ty: Type::Void,
                span: s,
            }));
            let test_id = self.fns.get(fn_name).unwrap().0;
            body.push(hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::Call(test_id, Symbol::intern(fn_name), vec![]),
                ty: Type::Void,
                span: s,
            }));
            body.push(hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::Builtin(
                    hir::BuiltinFn::Log,
                    vec![hir::Expr {
                        kind: hir::ExprKind::Str("  ok".into()),
                        ty: Type::String,
                        span: s,
                    }],
                ),
                ty: Type::Void,
                span: s,
            }));
        }
        hir::Fn {
            def_id: id,
            name: "main".into(),
            params: vec![],
            ret: Type::I32,
            error_types: Vec::new(),
            body,
            span: s,
            generic_origin: None,
            is_generator: false,
            attrs: crate::ast::FnAttrs::default(),
        }
    }

    pub(in crate::typer) fn lower_type_def(
        &mut self,
        td: &ast::TypeDef,
    ) -> Result<hir::TypeDef, String> {
        let id = self.fresh_id();
        let declared_fields = self.structs.get(&td.name).cloned().unwrap_or_default();
        let fields: Vec<hir::Field> = td
            .fields
            .iter()
            .map(|f| {
                let raw_ty = declared_fields
                    .iter()
                    .find(|(n, _)| n == &f.name)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)));
                let ty = self.infer_ctx.resolve(&raw_ty);
                let default = f.default.as_ref().map(|e| {
                    let lowered =
                        self.lower_expr_expected(e, Some(&ty))
                            .unwrap_or_else(|_| hir::Expr {
                                kind: hir::ExprKind::Int(0),
                                ty: Type::I64,
                                span: e.span(),
                            });
                    let _ =
                        self.infer_ctx
                            .unify_at(&ty, &lowered.ty, f.span, "field default value");
                    lowered
                });
                hir::Field {
                    name: f.name,
                    ty,
                    default,
                    access_mod: f.access_mod,
                    span: f.span,
                }
            })
            .collect();

        let mut hir_methods = Vec::new();
        for m in &td.methods {
            let method_name = format!("{}_{}", td.name, m.name);
            if self.fns.contains_key(&method_name) {
                let hm = self.lower_method_by_ptr(&td.name.as_str(), m)?;
                hir_methods.push(hm);
            }
        }

        Ok(hir::TypeDef {
            def_id: id,
            name: td.name,
            fields,
            methods: hir_methods,
            layout: td.layout.clone(),
            span: td.span,
        })
    }

    pub(in crate::typer) fn lower_method_by_ptr(
        &mut self,
        type_name: &str,
        m: &ast::Fn,
    ) -> Result<hir::Fn, String> {
        self.lower_method_impl(type_name, m, true)
    }

    pub(in crate::typer) fn lower_static_method(
        &mut self,
        type_name: &str,
        m: &ast::Fn,
    ) -> Result<hir::Fn, String> {
        let method_name = Self::from_method_name(type_name, m);
        let (id, ptys, ret) = self
            .fns
            .get(&method_name)
            .ok_or_else(|| format!("undeclared method: {method_name}"))?
            .clone();

        let prev_method_type = self.current_method_type.take();
        self.current_method_type = Some(type_name.to_string());
        self.push_scope();
        let mut params = Vec::new();
        for (i, p) in m.params.iter().enumerate() {
            let pid = self.fresh_id();
            let ty = ptys[i].clone();
            let ownership = self
                .param_ownership_with_mod(&ty, p.access_mod)
                .map_err(|e| format!("{}: {e}", p.span.loc()))?;
            self.define_var(
                &p.name.as_str(),
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
                    scheme: None,
                },
            );
            params.push(hir::Param {
                def_id: pid,
                name: p.name,
                ty,
                ownership,
                default: None,
                access_mod: p.access_mod,
                span: p.span,
            });
        }

        let body = self.lower_block_with_tail(&m.body, &ret, Some(&ret))?;
        self.pop_scope();
        self.current_method_type = prev_method_type;

        if m.ret.is_none() {
            if let Some(tail_ty) = self.hir_tail_type(&body) {
                let r = self
                    .infer_ctx
                    .unify_at(&ret, &tail_ty, m.span, "static method tail");
                self.collect_unify_error(r);
            } else {
                let _ = self.infer_ctx.unify(&ret, &Type::Void);
            }
        }

        Ok(hir::Fn {
            def_id: id,
            name: method_name.into(),
            params,
            ret,
            error_types: Vec::new(),
            body,
            span: m.span,
            generic_origin: None,
            is_generator: false,
            attrs: m.attrs.clone(),
        })
    }

    pub(in crate::typer) fn lower_method_impl(
        &mut self,
        type_name: &str,
        m: &ast::Fn,
        by_ptr: bool,
    ) -> Result<hir::Fn, String> {
        let method_name = format!("{type_name}_{}", m.name);
        let (id, ptys, ret) = self
            .fns
            .get(&method_name)
            .ok_or_else(|| format!("undeclared method: {method_name}"))?
            .clone();

        let prev_method_type = self.current_method_type.take();
        self.current_method_type = Some(type_name.to_string());
        self.push_scope();
        let mut params = Vec::new();

        let self_id = self.fresh_id();
        let self_ty = ptys[0].clone();
        self.define_var(
            "self",
            VarInfo {
                def_id: self_id,
                ty: self_ty.clone(),
                ownership: Ownership::BorrowMut,
                scheme: None,
            },
        );
        params.push(hir::Param {
            def_id: self_id,
            name: "self".into(),
            ty: self_ty,
            ownership: Ownership::BorrowMut,
            default: None,
            access_mod: None,
            span: m.span,
        });

        let param_iter: Box<dyn Iterator<Item = &ast::Param>> = if by_ptr {
            Box::new(m.params.iter().filter(|p| p.name != "self"))
        } else {
            Box::new(m.params.iter())
        };
        for (i, p) in param_iter.enumerate() {
            let pid = self.fresh_id();
            let ty = ptys[i + 1].clone();
            let ownership = self
                .param_ownership_with_mod(&ty, p.access_mod)
                .map_err(|e| format!("{}: {e}", p.span.loc()))?;
            self.define_var(
                &p.name.as_str(),
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
                    scheme: None,
                },
            );
            params.push(hir::Param {
                def_id: pid,
                name: p.name,
                ty,
                ownership,
                default: None,
                access_mod: p.access_mod,
                span: p.span,
            });
        }

        let body = self.lower_block_with_tail(&m.body, &ret, Some(&ret))?;
        self.pop_scope();
        self.current_method_type = prev_method_type;

        let reason = if by_ptr {
            "ptr method tail expression"
        } else {
            "method tail expression"
        };
        if m.ret.is_none() {
            if let Some(tail_ty) = self.hir_tail_type(&body) {
                let r = self.infer_ctx.unify_at(&ret, &tail_ty, m.span, reason);
                self.collect_unify_error(r);
            } else {
                let _ = self.infer_ctx.unify(&ret, &Type::Void);
            }
        }

        Ok(hir::Fn {
            def_id: id,
            name: method_name.into(),
            params,
            ret,
            error_types: Vec::new(),
            body,
            span: m.span,
            generic_origin: None,
            is_generator: false,
            attrs: m.attrs.clone(),
        })
    }

    pub(in crate::typer) fn lower_enum_def(&mut self, ed: &ast::EnumDef) -> hir::EnumDef {
        let id = self.fresh_id();
        let variants: Vec<hir::Variant> = ed
            .variants
            .iter()
            .enumerate()
            .map(|(tag, v)| hir::Variant {
                name: v.name,
                fields: v
                    .fields
                    .iter()
                    .map(|f| hir::VField {
                        name: f.name,
                        ty: f.ty.clone(),
                    })
                    .collect(),
                tag: v.discriminant.map(|d| d as u32).unwrap_or(tag as u32),
                discriminant: v.discriminant,
                span: v.span,
            })
            .collect();
        hir::EnumDef {
            def_id: id,
            name: ed.name,
            variants,
            span: ed.span,
        }
    }

    pub(in crate::typer) fn lower_extern(&self, ef: &ast::ExternFn) -> hir::ExternFn {
        let (id, _, _) = self
            .externs
            .get(&ef.name)
            .cloned()
            .unwrap_or_else(|| (DefId::BUILTIN, vec![], Type::Void));
        hir::ExternFn {
            def_id: id,
            name: ef.name,
            params: ef.params.clone(),
            ret: ef.ret.clone(),
            variadic: ef.variadic,
            span: ef.span,
        }
    }

    pub(in crate::typer) fn lower_err_def(&mut self, ed: &ast::ErrDef) -> hir::ErrDef {
        let id = self.fresh_id();
        let variants: Vec<hir::ErrVariant> = ed
            .variants
            .iter()
            .enumerate()
            .map(|(tag, v)| hir::ErrVariant {
                name: v.name,
                fields: v.fields.clone(),
                tag: tag as u32,
                span: v.span,
            })
            .collect();
        hir::ErrDef {
            def_id: id,
            name: ed.name,
            variants,
            span: ed.span,
        }
    }

    pub(in crate::typer) fn type_implements_trait(
        &self,
        type_name: &str,
        trait_name: &str,
    ) -> bool {
        self.trait_impls
            .get(type_name)
            .map(|impls| impls.contains(&trait_name.to_string()))
            .unwrap_or(false)
    }
}
