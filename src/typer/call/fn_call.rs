#![allow(unused_imports, unused_variables)]

use std::collections::HashMap;

use super::super::unify;
use super::super::{DeferredField, Typer, VarInfo};
use crate::ast::{self, Expr, Span};
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(crate) fn lower_call(
        &mut self,
        callee: &ast::Expr,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        let spread_expanded;
        let args = if let Some(expanded) = self.expand_spread_args(callee, args, span) {
            spread_expanded = expanded;
            &spread_expanded[..]
        } else {
            args
        };

        let resolved;
        let args = if args.iter().any(|a| matches!(a, Expr::NamedArg(..))) {
            if let ast::Expr::Ident(name, _) = callee {
                if let Some(param_names) = self.fn_param_names.get(name).cloned() {
                    let ordered = super::args::resolve_named_args(&param_names, args, span)?;
                    resolved = ordered.into_iter().cloned().collect::<Vec<_>>();
                    &resolved[..]
                } else {
                    args
                }
            } else {
                args
            }
        } else {
            args
        };
        if let ast::Expr::Ident(name, _) = callee {
            if let Some(result) = self.try_lower_builtin_call(&name.as_str(), args, span) {
                return result;
            }

            if let Some((ref quantified, ref scheme_params, ref scheme_ret)) =
                self.fn_schemes.get(name).cloned()
            {
                if !quantified.is_empty() {
                    let scheme = crate::types::Scheme {
                        quantified: quantified.clone(),
                        ty: Type::Fn(scheme_params.clone(), Box::new(scheme_ret.clone())),
                    };
                    let instantiated = self.infer_ctx.instantiate(&scheme);
                    let (inst_params, _inst_ret) = match instantiated {
                        Type::Fn(ps, r) => (ps, *r),
                        _ => unreachable!("scheme instantiation should produce Fn type"),
                    };

                    let mut hargs: Vec<hir::Expr> = Vec::new();
                    for (i, arg) in args.iter().enumerate() {
                        let expected = inst_params.get(i);
                        hargs.push(self.lower_expr_expected(arg, expected)?);
                    }

                    for (i, ha) in hargs.iter().enumerate() {
                        if let Some(pt) = inst_params.get(i) {
                            let r = self
                                .infer_ctx
                                .unify_at(pt, &ha.ty, span, "function argument");
                            self.collect_unify_error(r);
                        }
                    }

                    let was_strict = self.infer_ctx.is_strict();
                    self.infer_ctx.set_strict(false);
                    let arg_tys: Vec<Type> = inst_params
                        .iter()
                        .map(|t| self.infer_ctx.resolve(t))
                        .collect();
                    self.infer_ctx.set_strict(was_strict);

                    let inf_fn = self
                        .inferable_fns
                        .get(name)
                        .cloned()
                        .expect("fn_schemes should have corresponding inferable_fn");
                    let normalized = Self::normalize_inferable_fn(&inf_fn);
                    let type_map = self.build_type_map(&name.as_str(), &normalized, &arg_tys);
                    return self.monomorphize_call(&name.as_str(), &type_map, hargs, span, true);
                }
            }

            if let Some(gf) = self.generic_fns.get(name).cloned() {
                let has_poly_scheme = self.fn_schemes.get(name).map_or(false, |s| !s.0.is_empty());
                let is_inferable = self.inferable_fns.contains_key(name);
                let is_inferable_without_scheme =
                    is_inferable && !self.fn_schemes.contains_key(name);
                if !has_poly_scheme && !is_inferable_without_scheme && !is_inferable {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    let arg_tys: Vec<Type> = hargs
                        .iter()
                        .map(|e| self.infer_ctx.resolve(&e.ty))
                        .collect();
                    let type_map = self.build_type_map(&name.as_str(), &gf, &arg_tys);
                    return self.monomorphize_call(&name.as_str(), &type_map, hargs, span, false);
                }
            }

            if let Some(inf_fn) = self.inferable_fns.get(name).cloned() {
                if !self.fn_schemes.get(name).map_or(false, |s| !s.0.is_empty())
                    && self.fn_schemes.contains_key(name)
                {
                    if let Some((_, param_tys, _)) = self.fns.get(name).cloned() {
                        let hargs: Vec<hir::Expr> = args
                            .iter()
                            .map(|e| self.lower_expr(e))
                            .collect::<Result<_, _>>()?;
                        let arg_tys: Vec<Type> = hargs
                            .iter()
                            .map(|e| self.infer_ctx.shallow_resolve(&e.ty))
                            .collect();
                        let resolved_params: Vec<Type> = param_tys
                            .iter()
                            .map(|t| self.infer_ctx.shallow_resolve(t))
                            .collect();
                        let needs_mono =
                            resolved_params.iter().zip(arg_tys.iter()).any(|(pt, at)| {
                                !matches!(pt, Type::TypeVar(_))
                                    && pt != at
                                    && self.infer_ctx.unify(pt, at).is_err()
                            });
                        if needs_mono {
                            let normalized = Self::normalize_inferable_fn(&inf_fn);
                            let type_map =
                                self.build_type_map(&name.as_str(), &normalized, &arg_tys);
                            return self.monomorphize_call(
                                &name.as_str(),
                                &type_map,
                                hargs,
                                span,
                                false,
                            );
                        }
                    }
                }
            }

            if let Some((id, param_tys, ret)) = self.fns.get(name).cloned() {
                let mut hargs: Vec<hir::Expr> = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    let expected = param_tys.get(i);
                    hargs.push(self.lower_expr_expected(arg, expected)?);
                }

                if hargs.len() < param_tys.len() {
                    if let Some(defaults) = self.fn_defaults.get(name).cloned() {
                        for i in hargs.len()..param_tys.len() {
                            if let Some(Some(def_expr)) = defaults.get(i) {
                                let expected = param_tys.get(i);
                                hargs.push(self.lower_expr_expected(def_expr, expected)?);
                            }
                        }
                    }
                }
                for (i, ha) in hargs.iter().enumerate() {
                    if let Some(pt) = param_tys.get(i) {
                        let _ = self
                            .infer_ctx
                            .unify_at(pt, &ha.ty, span, "function argument");
                    }
                }
                for (i, ha) in hargs.iter_mut().enumerate() {
                    if let Some(pt) = param_tys.get(i) {
                        let taken = std::mem::replace(
                            ha,
                            hir::Expr {
                                kind: hir::ExprKind::Int(0),
                                ty: Type::I64,
                                span,
                            },
                        );
                        *ha = self.maybe_coerce_to(taken, pt);
                    }
                }
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Call(id, name.clone(), hargs),
                    ty: ret,
                    span,
                });
            }

            if let Some(v) = self.find_var(&name.as_str()).cloned() {
                if let Some(scheme) = &v.scheme {
                    if scheme.is_poly() {
                        if let Some((lparams, lret, lbody, lspan)) =
                            self.poly_lambda_asts.get(name).cloned()
                        {
                            let inst = self.infer_ctx.instantiate(scheme);
                            let (inst_params, inst_ret) = match &inst {
                                Type::Fn(p, r) => (p.clone(), *r.clone()),
                                _ => (vec![], inst.clone()),
                            };

                            let mut hargs = Vec::new();
                            for (i, arg) in args.iter().enumerate() {
                                let expected = inst_params.get(i);
                                hargs.push(self.lower_expr_expected(arg, expected)?);
                            }
                            for (i, ha) in hargs.iter().enumerate() {
                                if let Some(pt) = inst_params.get(i) {
                                    let _ = self.infer_ctx.unify_at(
                                        pt,
                                        &ha.ty,
                                        span,
                                        "poly lambda argument",
                                    );
                                }
                            }

                            let was_strict = self.infer_ctx.is_strict();
                            self.infer_ctx.set_strict(false);
                            let resolved_params: Vec<Type> = inst_params
                                .iter()
                                .map(|t| self.infer_ctx.resolve(t))
                                .collect();
                            let resolved_ret = self.infer_ctx.resolve(&inst_ret);
                            self.infer_ctx.set_strict(was_strict);

                            let type_suffix: Vec<String> =
                                resolved_params.iter().map(|t| format!("{t}")).collect();
                            let mangled = format!("__poly_{name}_{}", type_suffix.join("_"));

                            let mangled_sym = Symbol::intern(&mangled);
                            if let Some((id, _, ret)) = self.fns.get(&mangled).cloned() {
                                return Ok(hir::Expr {
                                    kind: hir::ExprKind::Call(id, mangled_sym, hargs),
                                    ty: ret,
                                    span,
                                });
                            }

                            let fn_id = self.fresh_id();
                            self.fns.insert(
                                mangled_sym,
                                (fn_id, resolved_params.clone(), resolved_ret.clone()),
                            );

                            self.push_scope();
                            let mut fn_params = Vec::new();
                            for (i, p) in lparams.iter().enumerate() {
                                let pid = self.fresh_id();
                                let ty = resolved_params.get(i).cloned().unwrap_or(Type::I64);
                                let ownership = Self::ownership_for_type(&ty);
                                self.define_var(
                                    &p.name.as_str(),
                                    VarInfo {
                                        def_id: pid,
                                        ty: ty.clone(),
                                        ownership,
                                        scheme: None,
                                    },
                                );
                                fn_params.push(hir::Param {
                                    def_id: pid,
                                    name: p.name.clone(),
                                    ty,
                                    ownership,
                                    default: None,
                                    access_mod: None,
                                    span: p.span,
                                });
                            }

                            let hbody = self.lower_block_no_scope(&lbody, &resolved_ret)?;
                            self.pop_scope();

                            let final_ret = if lret.is_some() {
                                resolved_ret.clone()
                            } else if let Some(crate::hir::Stmt::Expr(e)) = hbody.last() {
                                if e.ty != Type::Void {
                                    let _ = self.infer_ctx.unify(&resolved_ret, &e.ty);
                                    self.infer_ctx.resolve(&resolved_ret)
                                } else {
                                    resolved_ret.clone()
                                }
                            } else {
                                resolved_ret.clone()
                            };

                            if let Some(entry) = self.fns.get_mut(&mangled_sym) {
                                entry.2 = final_ret.clone();
                            }

                            let mono_fn = hir::Fn {
                                def_id: fn_id,
                                name: mangled_sym,
                                params: fn_params,
                                ret: final_ret.clone(),
                                error_types: Vec::new(),
                                body: hbody,
                                span: lspan,
                                generic_origin: Some(*name),
                                is_generator: false,
                                attrs: crate::ast::FnAttrs::default(),
                            };
                            self.mono_fns.push(mono_fn);

                            return Ok(hir::Expr {
                                kind: hir::ExprKind::Call(fn_id, mangled_sym, hargs),
                                ty: final_ret,
                                span,
                            });
                        }
                    }
                }

                let resolved_ty = self.infer_ctx.shallow_resolve(&v.ty);
                if let Type::Fn(ptys, ret) = &resolved_ty {
                    let ret = *ret.clone();
                    let ptys = ptys.clone();
                    let fn_expr = hir::Expr {
                        kind: hir::ExprKind::Var(v.def_id, name.clone()),
                        ty: resolved_ty.clone(),
                        span,
                    };
                    let mut hargs = Vec::new();
                    for (i, arg) in args.iter().enumerate() {
                        let expected = ptys.get(i);
                        hargs.push(self.lower_expr_expected(arg, expected)?);
                    }
                    for (i, ha) in hargs.iter().enumerate() {
                        if let Some(pt) = ptys.get(i) {
                            let _ =
                                self.infer_ctx
                                    .unify_at(pt, &ha.ty, span, "indirect call argument");
                        }
                    }
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::IndirectCall(Box::new(fn_expr), hargs),
                        ty: ret,
                        span,
                    });
                }
                if matches!(resolved_ty, Type::TypeVar(_) | Type::Param(_)) {
                    let mut hargs = Vec::new();
                    for arg in args.iter() {
                        hargs.push(self.lower_expr(arg)?);
                    }
                    let arg_tys: Vec<Type> = hargs.iter().map(|a| a.ty.clone()).collect();
                    let ret = self.infer_ctx.fresh_var();
                    let fn_ty = Type::Fn(arg_tys, Box::new(ret.clone()));
                    let _ = self
                        .infer_ctx
                        .unify_at(&v.ty, &fn_ty, span, "higher-order call");
                    let fn_expr = hir::Expr {
                        kind: hir::ExprKind::Var(v.def_id, name.clone()),
                        ty: fn_ty,
                        span,
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::IndirectCall(Box::new(fn_expr), hargs),
                        ty: ret,
                        span,
                    });
                }
            }

            if let Some((id, ptys, ret)) = self.externs.get(name).cloned() {
                let mut hargs = Vec::new();
                for arg in args.iter() {
                    hargs.push(self.lower_expr(arg)?);
                }
                for (i, ha) in hargs.iter().enumerate() {
                    if let Some(pt) = ptys.get(i) {
                        let _ = self
                            .infer_ctx
                            .unify_at(pt, &ha.ty, span, "extern call argument");
                    }
                }
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Call(id, name.clone(), hargs),
                    ty: ret,
                    span,
                });
            }

            return Err(format!("undefined function: '{name}'"));
        }

        let hcallee = self.lower_expr(callee)?;
        let callee_resolved = self.infer_ctx.shallow_resolve(&hcallee.ty);
        let (ptys, ret) = match &callee_resolved {
            Type::Fn(ptys, ret) => (ptys.clone(), *ret.clone()),
            _ => {
                let mut hargs: Vec<hir::Expr> = Vec::new();
                for arg in args.iter() {
                    hargs.push(self.lower_expr(arg)?);
                }
                let arg_tys: Vec<Type> = hargs.iter().map(|a| a.ty.clone()).collect();
                let ret = self.infer_ctx.fresh_var();
                let fn_ty = Type::Fn(arg_tys, Box::new(ret.clone()));
                let _ = self
                    .infer_ctx
                    .unify_at(&hcallee.ty, &fn_ty, span, "higher-order call");
                return Ok(hir::Expr {
                    kind: hir::ExprKind::IndirectCall(
                        Box::new(hir::Expr {
                            ty: fn_ty,
                            ..hcallee
                        }),
                        hargs,
                    ),
                    ty: ret,
                    span,
                });
            }
        };
        let mut hargs: Vec<hir::Expr> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let expected = ptys.get(i);
            hargs.push(self.lower_expr_expected(arg, expected)?);
        }
        for (i, ha) in hargs.iter().enumerate() {
            if let Some(pt) = ptys.get(i) {
                let _ = self
                    .infer_ctx
                    .unify_at(pt, &ha.ty, span, "indirect call argument");
            }
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::IndirectCall(Box::new(hcallee), hargs),
            ty: ret,
            span,
        })
    }
}
