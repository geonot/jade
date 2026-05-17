//! Extracted lowering steps.

#![allow(unused_imports, unused_variables)]

use std::collections::{HashMap, HashSet};

use super::super::unify;
use super::super::{DeferredField, DeferredMethod, Typer, VarInfo};
use crate::ast::{self, Span};
use crate::hir::{self, CoercionKind, DefId, ExprKind, Ownership};
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
                    name: f.name.clone(),
                    ty,
                    default,
                    access_mod: f.access_mod,
                    span: f.span,
                }
            })
            .collect();

        let mut hir_handlers = Vec::new();
        for (i, h) in ad.handlers.iter().enumerate() {
            self.push_scope();
            for f in &fields {
                let fid = self.fresh_id();
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
                let ty = p
                    .ty
                    .clone()
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
                    name: p.name.clone(),
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
                name: h.name.clone(),
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
            name: ad.name.clone(),
            fields,
            handlers: hir_handlers,
            span: ad.span,
        })
    }

    pub(in crate::typer) fn lower_store_def(
        &mut self,
        sd: &ast::StoreDef,
    ) -> Result<hir::StoreDef, String> {
        let id = self.fresh_id();
        let is_simple = sd
            .decorators
            .iter()
            .any(|d| *d == ast::StoreDecorator::Simple);
        let dummy_span = ast::Span::dummy();

        let mut fields: Vec<hir::StoreField> = Vec::new();
        // Inject built-in fields unless @simple
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
        // Add __version field for @versioned stores
        let is_versioned = sd
            .decorators
            .iter()
            .any(|d| *d == ast::StoreDecorator::Versioned);
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
            fields.push(hir::StoreField {
                name: f.name.clone(),
                ty: f.ty.clone().unwrap_or(Type::I64),
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
            name: sd.name.clone(),
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
        let mut hir_methods = Vec::new();
        for m in &ib.methods {
            let hm = self.lower_method_by_ptr(&ib.type_name.as_str(), m)?;
            hir_methods.push(hm);
        }
        Ok(hir::TraitImpl {
            trait_name: ib.trait_name.clone(),
            trait_type_args: ib.trait_type_args.clone(),
            type_name: ib.type_name.clone(),
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
        if self.debug_types {
            if scheme.is_poly() {
                eprintln!(
                    "[type:scheme] {} :: ∀{:?}. ({}) -> {}",
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
        }
        self.fn_schemes
            .insert(name, (scheme.quantified, param_tys, ret_ty));
    }

    pub(in crate::typer) fn lower_fn(&mut self, f: &ast::Fn) -> Result<hir::Fn, String> {
        let mut hfn = self.lower_fn_deferred(f)?;
        if f.ret.is_none() && f.name != "main" {
            if !self.inferable_fns.contains_key(&f.name) {
                hfn.ret = self.infer_ctx.resolve(&hfn.ret);
            }
        }
        // R3.2: escape-analysis post-pass.  Records the inferred tier for
        // every binding `DefId` defined in this function so later passes
        // (R3.3 codegen) can decide between T1 raw-borrow and T2+ owned
        // codegen without re-deriving the heuristic per binding.
        let einfo = crate::escape::analyze_fn(&hfn);
        for (id, t) in einfo.iter() {
            self.escape_tiers.insert(*id, *t);
        }
        // R3.3: mutate the freshly-lowered HIR in place to demote `Owned`
        // bindings of `Field`/`Index` reads whose escape tier is `T1`
        // (short-lived borrows of a clonable heap value).  The matching
        // `Stmt::Drop` is removed in the same pass; the MIR lowerer pairs
        // this by skipping the auto-clone for `Borrowed` Field/Index
        // bindings (see `src/mir/lower/stmt.rs`).  Net effect: no alloc,
        // no copy, no free on the hot field-access path.
        let _demoted = crate::escape::apply_demotions(&mut hfn, &einfo);
        Ok(hfn)
    }

    pub(in crate::typer) fn lower_fn_deferred(&mut self, f: &ast::Fn) -> Result<hir::Fn, String> {
        let (id, ptys, ret) = self
            .fns
            .get(&f.name)
            .ok_or_else(|| format!("undeclared function: {}", f.name))?
            .clone();

        // Push the function body scope and define parameters *inside* it
        // (rather than in a separate outer scope). This is the Stage-A
        // refactor that lets `emit_scope_drops_excluding` see parameters
        // and emit per-param drops at function body end for params whose
        // ownership is `Owned` (notably explicit-`take` params). Borrowed
        // params (the new default for heap-managed types) are skipped by
        // the drop filter, preserving the long-standing "caller still owns
        // the value across the call" semantics for unannotated parameters.
        self.push_scope();
        let mut params = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
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
            let hir_default = if let Some(ref def_expr) = p.default {
                self.lower_expr(def_expr).ok()
            } else {
                None
            };
            params.push(hir::Param {
                def_id: pid,
                name: p.name.clone(),
                ty,
                ownership,
                default: hir_default,
                access_mod: p.access_mod,
                span: p.span,
            });
        }
        let prev_fn_ret = self.current_fn_ret_ty.replace(ret.clone());
        // Snapshot & reset per-function error-tracking state.
        let prev_inferred = std::mem::take(&mut self.current_fn_error_types);
        let prev_declared = std::mem::take(&mut self.current_fn_declared_errors);
        // Seed declared error-union from the AST signature.
        let mut declared_err_names: Vec<Symbol> = Vec::new();
        for et in &f.error_types {
            // The parser may emit Param/Enum/Struct/UserDefined for an
            // identifier in type position; resolve to the err-enum name.
            let name = match et {
                Type::Enum(n) | Type::Struct(n, _) | Type::Param(n) => Some(n.clone()),
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
        // Inline the body of `lower_block` so parameters (defined in the
        // current scope above) are seen by `finalize_block_drops` and get
        // per-param drop emission alongside body locals.
        let mut body = self.lower_block_no_scope(&f.body, &ret)?;
        self.finalize_block_drops(&mut body);
        // Final error union: union of declared + inferred.
        let mut error_types: Vec<Type> = Vec::new();
        let mut seen: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
        for n in declared_err_names
            .into_iter()
            .chain(self.current_fn_error_types.iter().cloned())
        {
            if seen.insert(n.clone()) {
                error_types.push(Type::Enum(n));
            }
        }
        // Restore.
        self.current_fn_ret_ty = prev_fn_ret;
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
        }

        Ok(hir::Fn {
            def_id: id,
            name: f.name.clone(),
            params,
            ret,
            error_types,
            body,
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
                    name: f.name.clone(),
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
            name: td.name.clone(),
            fields,
            methods: hir_methods,
            layout: td.layout.clone(),
            span: td.span,
        })
    }

    #[allow(dead_code)]
    pub(in crate::typer) fn lower_method(
        &mut self,
        type_name: &str,
        m: &ast::Fn,
    ) -> Result<hir::Fn, String> {
        self.lower_method_impl(type_name, m, false)
    }

    pub(in crate::typer) fn lower_method_by_ptr(
        &mut self,
        type_name: &str,
        m: &ast::Fn,
    ) -> Result<hir::Fn, String> {
        self.lower_method_impl(type_name, m, true)
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
                name: p.name.clone(),
                ty,
                ownership,
                default: None,
                access_mod: p.access_mod,
                span: p.span,
            });
        }

        let body = self.lower_block(&m.body, &ret)?;
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
            // Do NOT resolve here. Methods on different types are lowered in
            // source order, but their return-type variables may be unified
            // through cross-method calls (e.g. type A's method tail-calls type
            // B's method whose body has not yet been lowered). Resolving now
            // would default A's ret-var to i64 before B's body unifies B's
            // ret-var with its true type. The global `resolve_fn` pass at the
            // end of `lower_program` runs after all bodies, so it sees the
            // fully-constrained type.
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
                name: v.name.clone(),
                fields: v
                    .fields
                    .iter()
                    .map(|f| hir::VField {
                        name: f.name.clone(),
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
            name: ed.name.clone(),
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
            name: ef.name.clone(),
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
                name: v.name.clone(),
                fields: v.fields.clone(),
                tag: tag as u32,
                span: v.span,
            })
            .collect();
        hir::ErrDef {
            def_id: id,
            name: ed.name.clone(),
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
