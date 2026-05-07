//! Call-site typing: argument unification, generic instantiation, method resolution.

use crate::intern::Symbol;
use std::collections::HashMap;

use crate::ast::{self, Expr, Span};
use crate::hir;
use crate::types::Type;

use super::{Typer, VarInfo};

/// Resolve named arguments to positional order.
/// Given param names and call-site args (which may include NamedArg nodes),
/// reorder them to match param order. Positional args fill left-to-right,
/// named args fill by name. Returns the reordered arg list.
fn resolve_named_args<'a>(
    param_names: &[String],
    args: &'a [ast::Expr],
    span: Span,
) -> Result<Vec<&'a ast::Expr>, String> {
    let has_named = args.iter().any(|a| matches!(a, Expr::NamedArg(..)));
    if !has_named {
        return Ok(args.iter().collect());
    }

    let mut result: Vec<Option<&ast::Expr>> = vec![None; param_names.len()];
    let mut pos_idx = 0;

    for arg in args {
        match arg {
            Expr::NamedArg(name, inner, _) => {
                if let Some(idx) = param_names.iter().position(|p| p == &name.as_str()) {
                    if result[idx].is_some() {
                        return Err(format!(
                            "duplicate named argument '{name}' at line {}",
                            span.line
                        ));
                    }
                    result[idx] = Some(inner.as_ref());
                } else {
                    return Err(format!(
                        "unknown named argument '{name}' at line {}",
                        span.line
                    ));
                }
            }
            _ => {
                while pos_idx < result.len() && result[pos_idx].is_some() {
                    pos_idx += 1;
                }
                if pos_idx >= param_names.len() {
                    return Err(format!(
                        "too many positional arguments at line {}",
                        span.line
                    ));
                }
                result[pos_idx] = Some(arg);
                pos_idx += 1;
            }
        }
    }

    // Check all required slots filled
    let mut ordered = Vec::new();
    for (i, slot) in result.into_iter().enumerate() {
        match slot {
            Some(e) => ordered.push(e),
            None => {
                return Err(format!(
                    "missing argument '{}' at line {}",
                    param_names[i], span.line
                ));
            }
        }
    }
    Ok(ordered)
}

impl Typer {
    /// Expand spread arguments in a call's argument list.
    /// `...arr` where `arr` is a fixed-size array becomes individual index expressions.
    /// Also handles implicit spreading: if a single array arg is passed where N params expected.
    fn expand_spread_args(
        &mut self,
        callee: &ast::Expr,
        args: &[ast::Expr],
        _span: Span,
    ) -> Option<Vec<ast::Expr>> {
        let has_spread = args.iter().any(|a| matches!(a, ast::Expr::Spread(..)));
        let expected_param_count = if let ast::Expr::Ident(name, _) = callee {
            self.fns.get(name).map(|(_, ptys, _)| ptys.len())
        } else {
            None
        };

        if has_spread {
            // Explicit spread: expand ...arr into individual elements
            let mut expanded = Vec::new();
            for arg in args {
                if let ast::Expr::Spread(inner, sp) = arg {
                    // Lower the inner expression to determine its type
                    let inner_lowered = self.lower_expr(inner).ok()?;
                    let resolved_ty = self.infer_ctx.resolve(&inner_lowered.ty);
                    match &resolved_ty {
                        Type::Array(_, len) => {
                            for i in 0..*len {
                                expanded.push(ast::Expr::Index(
                                    Box::new((**inner).clone()),
                                    Box::new(ast::Expr::Int(i as i64, *sp)),
                                    *sp,
                                ));
                            }
                        }
                        Type::Tuple(tys) => {
                            for i in 0..tys.len() {
                                expanded.push(ast::Expr::Index(
                                    Box::new((**inner).clone()),
                                    Box::new(ast::Expr::Int(i as i64, *sp)),
                                    *sp,
                                ));
                            }
                        }
                        _ => {
                            // For Vec/other types, we can't expand at compile time
                            // Just pass the inner expression as-is
                            expanded.push((**inner).clone());
                        }
                    }
                } else {
                    expanded.push(arg.clone());
                }
            }
            return Some(expanded);
        }

        // Implicit spreading: single array arg for multi-param function
        if args.len() == 1 && !has_spread {
            if let Some(expected) = expected_param_count {
                if expected > 1 {
                    let inner_lowered = self.lower_expr(&args[0]).ok()?;
                    let resolved_ty = self.infer_ctx.resolve(&inner_lowered.ty);
                    if let Type::Array(_, len) = &resolved_ty {
                        if *len == expected {
                            let sp = args[0].span();
                            let mut expanded = Vec::new();
                            for i in 0..*len {
                                expanded.push(ast::Expr::Index(
                                    Box::new(args[0].clone()),
                                    Box::new(ast::Expr::Int(i as i64, sp)),
                                    sp,
                                ));
                            }
                            return Some(expanded);
                        }
                    }
                }
            }
        }

        None
    }

    pub(crate) fn lower_call(
        &mut self,
        callee: &ast::Expr,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        // Expand spread arguments (...arr) into individual element args
        let spread_expanded;
        let args = if let Some(expanded) = self.expand_spread_args(callee, args, span) {
            spread_expanded = expanded;
            &spread_expanded[..]
        } else {
            args
        };
        // Strip NamedArg wrappers if the callee is a known function
        let resolved;
        let args = if args.iter().any(|a| matches!(a, Expr::NamedArg(..))) {
            if let ast::Expr::Ident(name, _) = callee {
                if let Some(param_names) = self.fn_param_names.get(name).cloned() {
                    let ordered = resolve_named_args(&param_names, args, span)?;
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
                            let type_map = self.build_type_map(&name.as_str(), &normalized, &arg_tys);
                            return self.monomorphize_call(&name.as_str(), &type_map, hargs, span, false);
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
                // Fill in defaults for missing arguments
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

    pub(crate) fn lower_method_call(
        &mut self,
        obj: &ast::Expr,
        method: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        // Store aggregation methods: store.avg(field), store.sum(field), etc.
        if let ast::Expr::Ident(name, _) = obj {
            if self.store_schemas.contains_key(&name.as_str()) {
                // @kv store methods: .set(), .get(), .del(), .has(), .incr(), .count()
                let is_kv = self
                    .store_decorators
                    .get(&name.as_str())
                    .map(|decs| decs.iter().any(|d| *d == crate::ast::StoreDecorator::Kv))
                    .unwrap_or(false);
                if is_kv {
                    match method {
                        "set" => {
                            if args.len() != 2 {
                                return Err("kv.set() requires 2 arguments (key, value)".into());
                            }
                            let key_expr =
                                self.lower_expr_expected(&args[0], Some(&Type::String))?;
                            let val_expr = self.lower_expr_expected(&args[1], Some(&Type::I64))?;
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::KvSet(
                                    name.clone(),
                                    Box::new(key_expr),
                                    Box::new(val_expr),
                                ),
                                ty: Type::Void,
                                span,
                            });
                        }
                        "get" => {
                            if args.len() != 1 {
                                return Err("kv.get() requires 1 argument (key)".into());
                            }
                            let key_expr =
                                self.lower_expr_expected(&args[0], Some(&Type::String))?;
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::KvGet(name.clone(), Box::new(key_expr)),
                                ty: Type::I64,
                                span,
                            });
                        }
                        "has" => {
                            if args.len() != 1 {
                                return Err("kv.has() requires 1 argument (key)".into());
                            }
                            let key_expr =
                                self.lower_expr_expected(&args[0], Some(&Type::String))?;
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::KvHas(name.clone(), Box::new(key_expr)),
                                ty: Type::Bool,
                                span,
                            });
                        }
                        "count" => {
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::KvCount(name.clone()),
                                ty: Type::I64,
                                span,
                            });
                        }
                        "del" => {
                            if args.len() != 1 {
                                return Err("kv.del() requires 1 argument (key)".into());
                            }
                            let key_expr =
                                self.lower_expr_expected(&args[0], Some(&Type::String))?;
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::KvDel(name.clone(), Box::new(key_expr)),
                                ty: Type::Void,
                                span,
                            });
                        }
                        "incr" => {
                            let (key_expr, delta_expr) = if args.len() == 1 {
                                let k = self.lower_expr_expected(&args[0], Some(&Type::String))?;
                                let d = hir::Expr {
                                    kind: hir::ExprKind::Int(1),
                                    ty: Type::I64,
                                    span,
                                };
                                (k, d)
                            } else if args.len() == 2 {
                                let k = self.lower_expr_expected(&args[0], Some(&Type::String))?;
                                let d = self.lower_expr_expected(&args[1], Some(&Type::I64))?;
                                (k, d)
                            } else {
                                return Err(
                                    "kv.incr() requires 1-2 arguments (key [, delta])".into()
                                );
                            };
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::KvIncr(
                                    name.clone(),
                                    Box::new(key_expr),
                                    Box::new(delta_expr),
                                ),
                                ty: Type::Void,
                                span,
                            });
                        }
                        "decr" => {
                            let key_expr = if args.len() >= 1 {
                                self.lower_expr_expected(&args[0], Some(&Type::String))?
                            } else {
                                return Err(
                                    "kv.decr() requires 1-2 arguments (key [, delta])".into()
                                );
                            };
                            let delta_expr = if args.len() == 2 {
                                // Negate the delta
                                let d = self.lower_expr_expected(&args[1], Some(&Type::I64))?;
                                hir::Expr {
                                    kind: hir::ExprKind::BinOp(
                                        Box::new(hir::Expr {
                                            kind: hir::ExprKind::Int(0),
                                            ty: Type::I64,
                                            span,
                                        }),
                                        crate::ast::BinOp::Sub,
                                        Box::new(d),
                                    ),
                                    ty: Type::I64,
                                    span,
                                }
                            } else {
                                hir::Expr {
                                    kind: hir::ExprKind::Int(-1),
                                    ty: Type::I64,
                                    span,
                                }
                            };
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::KvIncr(
                                    name.clone(),
                                    Box::new(key_expr),
                                    Box::new(delta_expr),
                                ),
                                ty: Type::Void,
                                span,
                            });
                        }
                        _ => {
                            return Err(format!(
                                "@kv store supports .set(), .get(), .del(), .has(), .incr(), .decr(), .count(); got .{method}()"
                            ));
                        }
                    }
                }

                // @graph store methods
                let is_graph = self
                    .store_decorators
                    .get(&name.as_str())
                    .map(|decs| decs.iter().any(|d| *d == crate::ast::StoreDecorator::Graph))
                    .unwrap_or(false);
                if is_graph {
                    match method {
                        "from" => {
                            if args.len() != 1 {
                                return Err("graph.from() requires 1 argument (node)".into());
                            }
                            // .from(node) queries the first user field of the graph store
                            let schema = self.store_schemas.get(&name.as_str()).unwrap();
                            let builtin = [
                                "sid",
                                "uuid",
                                "hash",
                                "created",
                                "updated",
                                "deleted",
                                "__version",
                            ];
                            let user_fields: Vec<_> = schema
                                .iter()
                                .filter(|(n, _)| !builtin.iter().any(|b| *n == *b))
                                .collect();
                            let first_ty = user_fields
                                .first()
                                .map(|(_, t)| t.clone())
                                .unwrap_or(Type::I64);
                            let node_expr = self.lower_expr_expected(&args[0], Some(&first_ty))?;
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::GraphFrom(name.clone(), Box::new(node_expr)),
                                ty: Type::I64,
                                span,
                            });
                        }
                        "to" => {
                            if args.len() != 1 {
                                return Err("graph.to() requires 1 argument (node)".into());
                            }
                            // .to(node) queries the second user field of the graph store
                            let schema = self.store_schemas.get(&name.as_str()).unwrap();
                            let builtin = [
                                "sid",
                                "uuid",
                                "hash",
                                "created",
                                "updated",
                                "deleted",
                                "__version",
                            ];
                            let user_fields: Vec<_> = schema
                                .iter()
                                .filter(|(n, _)| !builtin.iter().any(|b| *n == *b))
                                .collect();
                            let second_ty = user_fields
                                .get(1)
                                .map(|(_, t)| t.clone())
                                .unwrap_or(Type::I64);
                            let node_expr = self.lower_expr_expected(&args[0], Some(&second_ty))?;
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::GraphTo(name.clone(), Box::new(node_expr)),
                                ty: Type::I64,
                                span,
                            });
                        }
                        _ => {} // fall through to aggregation methods
                    }
                }

                // @timeseries store methods
                let is_ts = self
                    .store_decorators
                    .get(&name.as_str())
                    .map(|decs| {
                        decs.iter()
                            .any(|d| matches!(d, crate::ast::StoreDecorator::TimeSeries(_)))
                    })
                    .unwrap_or(false);
                if is_ts {
                    match method {
                        "latest" => {
                            if !args.is_empty() {
                                return Err("timeseries.latest() takes no arguments".into());
                            }
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::TsLatest(name.clone()),
                                ty: Type::I64,
                                span,
                            });
                        }
                        _ => {} // fall through
                    }
                }

                // @vector store methods
                let vec_dims = self.store_decorators.get(&name.as_str()).and_then(|decs| {
                    decs.iter().find_map(|d| match d {
                        crate::ast::StoreDecorator::Vector(dims) => Some(*dims),
                        _ => None,
                    })
                });
                if let Some(_dims) = vec_dims {
                    match method {
                        "nearest" => {
                            if args.len() != 2 {
                                return Err(
                                    "vector.nearest() requires 2 arguments (query_array, k)".into(),
                                );
                            }
                            let query_expr = self.lower_expr(&args[0])?;
                            let k_expr = self.lower_expr(&args[1])?;
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::VecNearest(
                                    name.clone(),
                                    Box::new(query_expr),
                                    Box::new(k_expr),
                                ),
                                ty: Type::I64,
                                span,
                            });
                        }
                        "insert" => {
                            if args.len() != 1 {
                                return Err(
                                    "vector.insert() requires 1 argument (vector array)".into()
                                );
                            }
                            let vec_expr = self.lower_expr(&args[0])?;
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::VecInsert(name.clone(), Box::new(vec_expr)),
                                ty: Type::I64,
                                span,
                            });
                        }
                        "count" => {
                            if !args.is_empty() {
                                return Err("vector.count() takes no arguments".into());
                            }
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::VecCount(name.clone()),
                                ty: Type::I64,
                                span,
                            });
                        }
                        _ => {} // fall through
                    }
                }

                // @bloom field methods: store.maybe(field, value)
                if method == "maybe" {
                    if args.len() != 2 {
                        return Err("maybe() requires 2 arguments (field_name, value)".into());
                    }
                    let field = match &args[0] {
                        ast::Expr::Ident(f, _) => f.clone(),
                        _ => return Err("maybe() first argument must be a field name".into()),
                    };
                    let val_expr = self.lower_expr(&args[1])?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::BloomTest(name.clone(), field, Box::new(val_expr)),
                        ty: Type::Bool,
                        span,
                    });
                }

                // @search field methods: store.search(field, term)
                if method == "search" {
                    if args.len() != 2 {
                        return Err("search() requires 2 arguments (field_name, query)".into());
                    }
                    let field = match &args[0] {
                        ast::Expr::Ident(f, _) => f.clone(),
                        _ => return Err("search() first argument must be a field name".into()),
                    };
                    let query_expr = self.lower_expr(&args[1])?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::FtsSearch(name.clone(), field, Box::new(query_expr)),
                        ty: Type::I64,
                        span,
                    });
                }
                if method == "search_count" {
                    if args.len() != 1 {
                        return Err("search_count() requires 1 argument (field_name)".into());
                    }
                    let field = match &args[0] {
                        ast::Expr::Ident(f, _) => f.clone(),
                        _ => return Err("search_count() argument must be a field name".into()),
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::FtsCount(name.clone(), field),
                        ty: Type::I64,
                        span,
                    });
                }

                match method {
                    "sum" | "avg" | "min" | "max" => {
                        if args.len() != 1 {
                            return Err(format!("{method}() requires exactly 1 field argument"));
                        }
                        let field = match &args[0] {
                            ast::Expr::Ident(f, _) => f.clone(),
                            _ => return Err(format!("{method}() argument must be a field name")),
                        };
                        let kind = match method {
                            "sum" => hir::ExprKind::StoreSum(name.clone(), field.clone()),
                            "avg" => hir::ExprKind::StoreAvg(name.clone(), field.clone()),
                            "min" => hir::ExprKind::StoreMin(name.clone(), field.clone()),
                            "max" => hir::ExprKind::StoreMax(name.clone(), field.clone()),
                            _ => unreachable!(),
                        };
                        let field_ty = self.store_schemas.get(&name.as_str()).and_then(|schema| {
                            schema
                                .iter()
                                .find(|(n, _)| n == &field)
                                .map(|(_, t)| t.clone())
                        });
                        let ret_ty = if method == "avg" {
                            Type::F64
                        } else {
                            match field_ty {
                                Some(Type::F64) | Some(Type::F32) => Type::F64,
                                _ => Type::I64,
                            }
                        };
                        return Ok(hir::Expr {
                            kind,
                            ty: ret_ty,
                            span,
                        });
                    }
                    "distinct" => {
                        if args.len() != 1 {
                            return Err("distinct() requires exactly 1 field argument".into());
                        }
                        let field = match &args[0] {
                            ast::Expr::Ident(f, _) => f.clone(),
                            _ => return Err("distinct() argument must be a field name".into()),
                        };
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::StoreDistinct(name.clone(), field),
                            ty: Type::I64,
                            span,
                        });
                    }
                    "count" => {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::StoreCount(name.clone()),
                            ty: Type::I64,
                            span,
                        });
                    }
                    "version_count" => {
                        if args.len() != 1 {
                            return Err("version_count() requires exactly 1 argument (sid)".into());
                        }
                        let sid_expr = self.lower_expr_expected(&args[0], Some(&Type::I64))?;
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::StoreVersionCount(
                                name.clone(),
                                Box::new(sid_expr),
                            ),
                            ty: Type::I64,
                            span,
                        });
                    }
                    "history" => {
                        if args.len() != 1 {
                            return Err("history() requires exactly 1 argument (sid)".into());
                        }
                        let sid_expr = self.lower_expr_expected(&args[0], Some(&Type::I64))?;
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::StoreHistory(name.clone(), Box::new(sid_expr)),
                            ty: Type::I64, // returns count of versions written
                            span,
                        });
                    }
                    "at_version" => {
                        if args.len() != 2 {
                            return Err("at_version() requires 2 arguments (sid, version)".into());
                        }
                        let sid_expr = self.lower_expr_expected(&args[0], Some(&Type::I64))?;
                        let ver_expr = self.lower_expr_expected(&args[1], Some(&Type::I64))?;
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::StoreAtVersion(
                                name.clone(),
                                Box::new(sid_expr),
                                Box::new(ver_expr),
                            ),
                            ty: Type::I64, // returns 1 if found, 0 if not
                            span,
                        });
                    }
                    _ => {}
                }
            }
        }

        // View method dispatch: view_name.count(), view_name.all()
        if let ast::Expr::Ident(name, _) = obj {
            if let Some((source, clauses)) = self.view_defs.get(&name.as_str()).cloned() {
                let schema = self
                    .store_schemas
                    .get(&source.as_str())
                    .ok_or_else(|| format!("view '{name}' references unknown store '{source}'"))?
                    .clone();

                // Build filter from view's where clauses
                let where_exprs: Vec<(ast::Expr, ast::Span)> = clauses
                    .iter()
                    .filter_map(|c| {
                        if let ast::QueryClause::Where(expr, cspan) = c {
                            Some((expr.clone(), *cspan))
                        } else {
                            None
                        }
                    })
                    .collect();

                if where_exprs.is_empty() {
                    // View with no filter — delegate to source store
                    match method {
                        "count" => {
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::StoreCount(source),
                                ty: Type::I64,
                                span,
                            });
                        }
                        "all" => {
                            let struct_name = Symbol::intern(&format!("__store_{source}"));
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::StoreAll(source),
                                ty: Type::Ptr(Box::new(Type::Struct(struct_name, vec![]))),
                                span,
                            });
                        }
                        "select" | "first" => {
                            // delegate to StoreQuery on source (returns first match)
                            if args.is_empty() {
                                return Err(format!("view .{method}() requires a filter argument"));
                            }
                            let filter_expr = &args[0];
                            let ast_filter = Self::expr_to_store_filter(filter_expr, span)?;
                            let hfilter = self.lower_store_filter(&ast_filter, &schema, &source.as_str())?;
                            let struct_name = Symbol::intern(&format!("__store_{source}"));
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::StoreQuery(source, Box::new(hfilter)),
                                ty: Type::Struct(struct_name, vec![]),
                                span,
                            });
                        }
                        "exists" => {
                            if args.is_empty() {
                                return Err("view .exists() requires a filter argument".into());
                            }
                            let filter_expr = &args[0];
                            let ast_filter = Self::expr_to_store_filter(filter_expr, span)?;
                            let hfilter = self.lower_store_filter(&ast_filter, &schema, &source.as_str())?;
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::StoreExists(source, Box::new(hfilter)),
                                ty: Type::Bool,
                                span,
                            });
                        }
                        _ => {
                            return Err(format!(
                                "views support .count(), .all(), .select(), .first(), .exists(); got .{method}()"
                            ));
                        }
                    }
                }

                let ast_filter = Self::merge_where_clauses(&where_exprs)?;
                let hfilter = self.lower_store_filter(&ast_filter, &schema, &source.as_str())?;

                match method {
                    "count" => {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::ViewCount(source, Box::new(hfilter)),
                            ty: Type::I64,
                            span,
                        });
                    }
                    "all" => {
                        let struct_name = Symbol::intern(&format!("__store_{source}"));
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::ViewAll(source, Box::new(hfilter)),
                            ty: Type::Ptr(Box::new(Type::Struct(struct_name, vec![]))),
                            span,
                        });
                    }
                    "select" | "first" => {
                        // For filtered views, the view's filter is already the query filter
                        let struct_name = Symbol::intern(&format!("__store_{source}"));
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::StoreQuery(source.clone(), Box::new(hfilter)),
                            ty: Type::Struct(struct_name, vec![]),
                            span,
                        });
                    }
                    "exists" => {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::StoreExists(source.clone(), Box::new(hfilter)),
                            ty: Type::Bool,
                            span,
                        });
                    }
                    _ => {
                        return Err(format!(
                            "views support .count(), .all(), .select(), .first(), .exists(); got .{method}()"
                        ));
                    }
                }
            }
        }

        let hobj = self.lower_expr(obj)?;
        let obj_ty = self.infer_ctx.shallow_resolve(&hobj.ty);

        if let Type::ActorRef(actor_name) = &obj_ty {
            let (_, _, handlers) = self
                .actors
                .get(actor_name)
                .ok_or_else(|| format!("unknown actor '{actor_name}'"))?
                .clone();
            let (handler_name, handler_ptys, tag) = handlers
                .iter()
                .find(|(n, _, _)| n.as_str() == method)
                .ok_or_else(|| {
                    format!(
                        "actor '{actor_name}' has no handler '.{method}()'"
                    )
                })?
                .clone();

            if tag == u32::MAX {
                return Err(format!(
                    "actor '{actor_name}' handler '.{method}()' is reserved for *loop and cannot be sent"
                ));
            }

            if args.len() != handler_ptys.len() {
                return Err(format!(
                    "actor handler '.{method}()' on '{actor_name}' expects {} argument(s), got {}",
                    handler_ptys.len(),
                    args.len()
                ));
            }

            let mut hargs: Vec<hir::Expr> = Vec::with_capacity(args.len());
            for (i, arg) in args.iter().enumerate() {
                let harg = self.lower_expr_expected(arg, Some(&handler_ptys[i]))?;
                let _ = self.infer_ctx.unify_at(
                    &handler_ptys[i],
                    &harg.ty,
                    span,
                    "actor method argument",
                );
                hargs.push(harg);
            }

            return Ok(hir::Expr {
                kind: hir::ExprKind::Send(
                    Box::new(hobj),
                    actor_name.clone(),
                    handler_name,
                    tag,
                    hargs,
                ),
                ty: Type::Void,
                span,
            });
        }

        if matches!(obj_ty, Type::String) {
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let ret_ty = Self::string_method_ret_ty(method).unwrap_or(Type::I64);
            return Ok(hir::Expr {
                kind: hir::ExprKind::StringMethod(Box::new(hobj), method.into(), hargs),
                ty: ret_ty,
                span,
            });
        }

        let vec_elem_ty = match &obj_ty {
            Type::Vec(et) => Some(et.clone()),
            Type::Array(et, _) => Some(et.clone()),
            _ => None,
        };

        if let Some(ref elem_ty) = vec_elem_ty {
            // Iterator combinator methods that need special type handling
            match method {
                "map" => {
                    if args.len() != 1 {
                        return Err("map() requires exactly 1 argument".into());
                    }
                    let ret_elem = self.infer_ctx.fresh_var();
                    let fn_ty =
                        Type::Fn(vec![elem_ty.as_ref().clone()], Box::new(ret_elem.clone()));
                    let harg = self.lower_expr_expected(&args[0], Some(&fn_ty))?;
                    let _ = self
                        .infer_ctx
                        .unify_at(&fn_ty, &harg.ty, span, "map callback");
                    let ret_ty = Type::Vec(Box::new(ret_elem));
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::VecMethod(Box::new(hobj), "map".into(), vec![harg]),
                        ty: ret_ty,
                        span,
                    });
                }
                "filter" => {
                    if args.len() != 1 {
                        return Err("filter() requires exactly 1 argument".into());
                    }
                    let fn_ty = Type::Fn(vec![elem_ty.as_ref().clone()], Box::new(Type::Bool));
                    let harg = self.lower_expr_expected(&args[0], Some(&fn_ty))?;
                    let _ = self
                        .infer_ctx
                        .unify_at(&fn_ty, &harg.ty, span, "filter callback");
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::VecMethod(Box::new(hobj), "filter".into(), vec![harg]),
                        ty: Type::Vec(elem_ty.clone()),
                        span,
                    });
                }
                "fold" => {
                    if args.len() != 2 {
                        return Err("fold() requires exactly 2 arguments (init, fn)".into());
                    }
                    let hinit = self.lower_expr(&args[0])?;
                    let acc_ty = hinit.ty.clone();
                    let fn_ty = Type::Fn(
                        vec![acc_ty.clone(), elem_ty.as_ref().clone()],
                        Box::new(acc_ty.clone()),
                    );
                    let hfn = self.lower_expr_expected(&args[1], Some(&fn_ty))?;
                    let _ = self
                        .infer_ctx
                        .unify_at(&fn_ty, &hfn.ty, span, "fold callback");
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::VecMethod(
                            Box::new(hobj),
                            "fold".into(),
                            vec![hinit, hfn],
                        ),
                        ty: acc_ty,
                        span,
                    });
                }
                "any" | "all" => {
                    if args.len() != 1 {
                        return Err(format!("{method}() requires exactly 1 argument"));
                    }
                    let fn_ty = Type::Fn(vec![elem_ty.as_ref().clone()], Box::new(Type::Bool));
                    let harg = self.lower_expr_expected(&args[0], Some(&fn_ty))?;
                    let _ = self
                        .infer_ctx
                        .unify_at(&fn_ty, &harg.ty, span, "predicate callback");
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::VecMethod(
                            Box::new(hobj),
                            method.into(),
                            vec![harg],
                        ),
                        ty: Type::Bool,
                        span,
                    });
                }
                "find" => {
                    if args.len() != 1 {
                        return Err("find() requires exactly 1 argument".into());
                    }
                    let fn_ty = Type::Fn(vec![elem_ty.as_ref().clone()], Box::new(Type::Bool));
                    let harg = self.lower_expr_expected(&args[0], Some(&fn_ty))?;
                    let _ = self
                        .infer_ctx
                        .unify_at(&fn_ty, &harg.ty, span, "find callback");
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::VecMethod(Box::new(hobj), "find".into(), vec![harg]),
                        ty: elem_ty.as_ref().clone(),
                        span,
                    });
                }
                "zip" | "chain" => {
                    if args.len() != 1 {
                        return Err(format!("{method}() requires exactly 1 argument"));
                    }
                    let harg = self.lower_expr(&args[0])?;
                    if method == "chain" {
                        let _ = self
                            .infer_ctx
                            .unify_at(&obj_ty, &harg.ty, span, "chain argument");
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VecMethod(
                                Box::new(hobj),
                                "chain".into(),
                                vec![harg],
                            ),
                            ty: obj_ty.clone(),
                            span,
                        });
                    }
                    // zip: Vec<A>.zip(Vec<B>) -> Vec<(A, B)>
                    let other_elem = match &harg.ty {
                        Type::Vec(et) => et.as_ref().clone(),
                        _ => return Err("zip() argument must be a Vec".into()),
                    };
                    let tuple_ty = Type::Tuple(vec![elem_ty.as_ref().clone(), other_elem]);
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::VecMethod(Box::new(hobj), "zip".into(), vec![harg]),
                        ty: Type::Vec(Box::new(tuple_ty)),
                        span,
                    });
                }
                _ => {}
            }
            let expected_arg_tys: Vec<Option<&Type>> = match method {
                "push" => vec![Some(elem_ty.as_ref())],
                "set" => vec![Some(&Type::I64), Some(elem_ty.as_ref())],
                "get" | "remove" | "take" | "skip" => vec![Some(&Type::I64)],
                "contains" => vec![Some(elem_ty.as_ref())],
                "join" => vec![Some(&Type::String)],
                _ => vec![],
            };
            let hargs: Vec<hir::Expr> = args
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    self.lower_expr_expected(e, expected_arg_tys.get(i).copied().flatten())
                })
                .collect::<Result<_, _>>()?;
            // Explicitly unify argument types with expected types
            for (i, ha) in hargs.iter().enumerate() {
                if let Some(Some(expected)) = expected_arg_tys.get(i) {
                    let _ = self
                        .infer_ctx
                        .unify_at(expected, &ha.ty, span, "vec method argument");
                }
            }
            let ret_ty = Self::vec_method_ret_ty(method, elem_ty)
                .ok_or_else(|| format!("no method '{method}' on Vec"))?;
            return Ok(hir::Expr {
                kind: hir::ExprKind::VecMethod(Box::new(hobj), method.into(), hargs),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Map(ref key_ty, ref val_ty) = obj_ty {
            let expected_arg_tys: Vec<Option<&Type>> = match method {
                "set" => vec![Some(key_ty.as_ref()), Some(val_ty.as_ref())],
                "get" | "has" | "remove" | "contains" => vec![Some(key_ty.as_ref())],
                _ => vec![],
            };
            let hargs: Vec<hir::Expr> = args
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    self.lower_expr_expected(e, expected_arg_tys.get(i).copied().flatten())
                })
                .collect::<Result<_, _>>()?;
            // Explicitly unify argument types with expected types
            for (i, ha) in hargs.iter().enumerate() {
                if let Some(Some(expected)) = expected_arg_tys.get(i) {
                    let _ = self
                        .infer_ctx
                        .unify_at(expected, &ha.ty, span, "map method argument");
                }
            }
            let ret_ty = Self::map_method_ret_ty(method, key_ty, val_ty)
                .ok_or_else(|| format!("no method '{method}' on Map"))?;
            return Ok(hir::Expr {
                kind: hir::ExprKind::MapMethod(Box::new(hobj), method.into(), hargs),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Set(ref elem_ty) = obj_ty {
            let expected_arg_tys: Vec<Option<&Type>> = match method {
                "add" => vec![Some(elem_ty.as_ref())],
                "contains" => vec![Some(elem_ty.as_ref())],
                "remove" => vec![Some(elem_ty.as_ref())],
                "union" | "difference" | "intersection" => vec![Some(&obj_ty)],
                _ => vec![],
            };
            let hargs: Vec<hir::Expr> = args
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    self.lower_expr_expected(e, expected_arg_tys.get(i).copied().flatten())
                })
                .collect::<Result<_, _>>()?;
            for (i, ha) in hargs.iter().enumerate() {
                if let Some(Some(expected)) = expected_arg_tys.get(i) {
                    let _ = self
                        .infer_ctx
                        .unify_at(expected, &ha.ty, span, "set method argument");
                }
            }
            let ret_ty = Self::set_method_ret_ty(method, elem_ty)
                .ok_or_else(|| format!("no method '{method}' on Set"))?;
            return Ok(hir::Expr {
                kind: hir::ExprKind::SetMethod(Box::new(hobj), method.into(), hargs),
                ty: ret_ty,
                span,
            });
        }

        if let Type::PriorityQueue(ref elem_ty) = obj_ty {
            let expected_arg_tys: Vec<Option<&Type>> = match method {
                "push" => vec![Some(elem_ty.as_ref()), Some(&Type::I64)], // value, priority
                "pop" | "peek" => vec![],
                "len" => vec![],
                "is_empty" => vec![],
                "clear" => vec![],
                _ => vec![],
            };
            let hargs: Vec<hir::Expr> = args
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    self.lower_expr_expected(e, expected_arg_tys.get(i).copied().flatten())
                })
                .collect::<Result<_, _>>()?;
            let ret_ty = match method {
                "push" | "clear" => Type::Void,
                "pop" | "peek" => *elem_ty.clone(),
                "len" => Type::I64,
                "is_empty" => Type::Bool,
                _ => return Err(format!("no method '{method}' on PriorityQueue")),
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::PQMethod(Box::new(hobj), method.into(), hargs),
                ty: ret_ty,
                span,
            });
        }

        // Char/Unicode methods on integer types (char codepoints)
        if matches!(
            obj_ty,
            Type::I8
                | Type::I16
                | Type::I32
                | Type::I64
                | Type::U8
                | Type::U16
                | Type::U32
                | Type::U64
        ) {
            let char_ret = match method {
                "is_digit" | "is_alpha" | "is_alphanumeric" | "is_upper" | "is_lower"
                | "is_whitespace" => Some(Type::Bool),
                "to_upper" | "to_lower" | "to_code" => Some(Type::I64),
                _ => None,
            };
            if let Some(ret_ty) = char_ret {
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(
                        hir::BuiltinFn::CharMethod(method.into()),
                        vec![hobj],
                    ),
                    ty: ret_ty,
                    span,
                });
            }
        }

        // Float math methods on f64/f32 types
        if matches!(obj_ty, Type::F64 | Type::F32) {
            let float_ret = match method {
                "sqrt" | "abs" | "floor" | "ceil" | "round" | "trunc" | "sin" | "cos" | "tan"
                | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "exp" | "exp2" | "ln"
                | "log2" | "log10" | "cbrt" | "recip" | "signum" => Some(obj_ty.clone()),
                "pow" | "atan2" | "copysign" | "min" | "max" => Some(obj_ty.clone()),
                "is_nan" | "is_infinite" | "is_finite" => Some(Type::Bool),
                "to_int" => Some(Type::I64),
                _ => None,
            };
            if let Some(ret_ty) = float_ret {
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                let mut all_args = vec![hobj];
                all_args.extend(hargs);
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(
                        hir::BuiltinFn::FloatMethod(method.into()),
                        all_args,
                    ),
                    ty: ret_ty,
                    span,
                });
            }
        }

        if matches!(obj_ty, Type::Arena) {
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr_expected(e, Some(&Type::I64)))
                .collect::<Result<_, _>>()?;
            for ha in &hargs {
                let _ = self
                    .infer_ctx
                    .unify_at(&ha.ty, &Type::I64, span, "arena method argument");
            }
            let (builtin, ret_ty) = match method {
                "alloc" => (hir::BuiltinFn::ArenaAlloc, Type::Ptr(Box::new(Type::I8))),
                "reset" => (hir::BuiltinFn::ArenaReset, Type::Void),
                _ => return Err(format!("no method '{method}' on Arena")),
            };
            let mut all_args = vec![hobj];
            all_args.extend(hargs);
            return Ok(hir::Expr {
                kind: hir::ExprKind::Builtin(builtin, all_args),
                ty: ret_ty,
                span,
            });
        }

        if matches!(obj_ty, Type::Pool) {
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let (builtin, ret_ty) = match method {
                "alloc" => (hir::BuiltinFn::PoolAlloc, Type::Ptr(Box::new(Type::I8))),
                "free" => (hir::BuiltinFn::PoolFree, Type::Void),
                "destroy" => (hir::BuiltinFn::PoolDestroy, Type::Void),
                _ => return Err(format!("no method '{method}' on Pool")),
            };
            let mut all_args = vec![hobj];
            all_args.extend(hargs);
            return Ok(hir::Expr {
                kind: hir::ExprKind::Builtin(builtin, all_args),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Coroutine(ref yield_ty) = obj_ty {
            if method == "next" {
                return Ok(hir::Expr {
                    kind: hir::ExprKind::CoroutineNext(Box::new(hobj)),
                    ty: *yield_ty.clone(),
                    span,
                });
            }
            return Err(format!("no method '{method}' on Coroutine"));
        }

        if let Type::Generator(ref yield_ty) = obj_ty {
            if method == "next" {
                return Ok(hir::Expr {
                    kind: hir::ExprKind::GeneratorNext(Box::new(hobj)),
                    ty: *yield_ty.clone(),
                    span,
                });
            }
            return Err(format!("no method '{method}' on Generator"));
        }

        if let Type::DynTrait(ref trait_name) = obj_ty {
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let ret_ty = self.infer_dyn_method_ret(&trait_name.as_str(), method);
            return Ok(hir::Expr {
                kind: hir::ExprKind::DynDispatch(
                    Box::new(hobj),
                    trait_name.clone(),
                    method.into(),
                    hargs,
                ),
                ty: ret_ty,
                span,
            });
        }

        // ── Option / Result enum methods ──
        if let Type::Enum(ref enum_name) = obj_ty {
            let is_option = enum_name.starts_with("Option_") || enum_name == "Option";
            let is_result = enum_name.starts_with("Result_") || enum_name == "Result";
            if is_option || is_result {
                let variants = self.enums.get(enum_name).cloned().unwrap_or_default();
                match method {
                    "unwrap" => {
                        // Some/Ok is tag 0, inner type is field 0
                        let inner_ty = variants
                            .first()
                            .and_then(|(_, ftys)| ftys.first().cloned())
                            .unwrap_or(Type::I64);
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::EnumUnwrap(Box::new(hobj), enum_name.clone(), 0),
                            ty: inner_ty,
                            span,
                        });
                    }
                    "is_some" if is_option => {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::EnumIs(Box::new(hobj), 0),
                            ty: Type::Bool,
                            span,
                        });
                    }
                    "is_nothing" if is_option => {
                        let nothing_tag = variants
                            .iter()
                            .position(|(n, _)| n == "Nothing")
                            .unwrap_or(1) as u32;
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::EnumIs(Box::new(hobj), nothing_tag),
                            ty: Type::Bool,
                            span,
                        });
                    }
                    "is_ok" if is_result => {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::EnumIs(Box::new(hobj), 0),
                            ty: Type::Bool,
                            span,
                        });
                    }
                    "is_err" if is_result => {
                        let err_tag =
                            variants.iter().position(|(n, _)| n == "Err").unwrap_or(1) as u32;
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::EnumIs(Box::new(hobj), err_tag),
                            ty: Type::Bool,
                            span,
                        });
                    }
                    "unwrap_or" if args.len() == 1 => {
                        let inner_ty = variants
                            .first()
                            .and_then(|(_, ftys)| ftys.first().cloned())
                            .unwrap_or(Type::I64);
                        // Lower the default argument, then use a ternary: is_some ? unwrap : default
                        let default_arg = self.lower_expr_expected(&args[0], Some(&inner_ty))?;
                        let is_check = hir::Expr {
                            kind: hir::ExprKind::EnumIs(Box::new(hobj.clone()), 0),
                            ty: Type::Bool,
                            span,
                        };
                        let unwrap_expr = hir::Expr {
                            kind: hir::ExprKind::EnumUnwrap(Box::new(hobj), enum_name.clone(), 0),
                            ty: inner_ty.clone(),
                            span,
                        };
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::Ternary(
                                Box::new(is_check),
                                Box::new(unwrap_expr),
                                Box::new(default_arg),
                            ),
                            ty: inner_ty,
                            span,
                        });
                    }
                    _ => {} // Fall through for other methods
                }
            }
        }

        let struct_type_name = match &obj_ty {
            Type::Struct(name, _) => Some(name.clone()),
            Type::Ptr(inner) => {
                if let Type::Struct(name, _) = inner.as_ref() {
                    Some(name.clone())
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(ref type_name) = struct_type_name {
            let method_name = format!("{type_name}_{method}");
            if let Some((_, param_tys, ret)) = self.fns.get(&method_name).cloned() {
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let expected = param_tys.get(i + 1);
                        self.lower_expr_expected(e, expected)
                    })
                    .collect::<Result<_, _>>()?;
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Method(
                        Box::new(hobj),
                        Symbol::intern(&method_name),
                        Symbol::intern(method),
                        hargs,
                    ),
                    ty: ret,
                    span,
                });
            }
        }

        if matches!(obj_ty, Type::TypeVar(_)) {
            // String-exclusive methods: if receiver is TypeVar and method is unique to String,
            // immediately constrain receiver to String and dispatch.
            if Self::is_string_exclusive_method(method) {
                let _ = self.infer_ctx.unify_at(
                    &obj_ty,
                    &Type::String,
                    span,
                    "method call implies String type",
                );
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                let ret_ty = Self::string_method_ret_ty(method).unwrap_or(Type::I64);
                return Ok(hir::Expr {
                    kind: hir::ExprKind::StringMethod(Box::new(hobj), method.into(), hargs),
                    ty: ret_ty,
                    span,
                });
            }

            let suffix = format!("_{method}");
            let mut candidates: Vec<(String, Vec<Type>, Type)> = self
                .fns
                .iter()
                .filter(|(name, _)| name.ends_with(&suffix))
                .map(|(name, (_, ptys, ret))| {
                    let name_s = name.as_str();
                    let type_name = name_s[..name_s.len() - suffix.len()].to_string();
                    (type_name, ptys.clone(), ret.clone())
                })
                .filter(|(type_name, _, _)| self.structs.contains_key(type_name.as_str()))
                .collect();

            if candidates.len() > 1 {
                let defining_traits: Vec<&Symbol> = self
                    .traits
                    .iter()
                    .filter(|(_, sigs)| sigs.iter().any(|s| s.name == method))
                    .map(|(tname, _)| tname)
                    .collect();
                if !defining_traits.is_empty() {
                    let narrowed: Vec<(String, Vec<Type>, Type)> = candidates
                        .iter()
                        .filter(|(type_name, _, _)| {
                            self.trait_impls.get(type_name.as_str()).map_or(false, |impls| {
                                impls.iter().any(|i| defining_traits.iter().any(|t| **t == i.as_str()))
                            })
                        })
                        .cloned()
                        .collect();
                    if !narrowed.is_empty() {
                        candidates = narrowed;
                    }
                }
            }

            if candidates.len() == 1 {
                let (type_name, param_tys, ret) = &candidates[0];
                let struct_ty = Type::Struct(Symbol::intern(type_name), vec![]);
                let _ = self.infer_ctx.unify_at(
                    &obj_ty,
                    &struct_ty,
                    span,
                    "method call implies struct type",
                );
                let method_name = format!("{}_{}", type_name, method);
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let expected = param_tys.get(i + 1);
                        self.lower_expr_expected(e, expected)
                    })
                    .collect::<Result<_, _>>()?;
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Method(
                        Box::new(hobj),
                        Symbol::intern(&method_name),
                        Symbol::intern(method),
                        hargs,
                    ),
                    ty: ret.clone(),
                    span,
                });
            }
        }

        let hargs: Vec<hir::Expr> = args
            .iter()
            .map(|e| self.lower_expr(e))
            .collect::<Result<_, _>>()?;
        let ret_ty = self.infer_ctx.fresh_var();
        if matches!(obj_ty, Type::TypeVar(_)) {
            let arg_tys: Vec<Type> = hargs.iter().map(|a| a.ty.clone()).collect();

            let mut defining_trait_names: Vec<String> = Vec::new();
            for (trait_name, sigs) in &self.traits {
                for sig in sigs {
                    if sig.name == method {
                        defining_trait_names.push(trait_name.as_str());
                        if let Some(ref trait_ret) = sig._ret {
                            let _ = self.infer_ctx.unify_at(
                                &ret_ty,
                                trait_ret,
                                span,
                                "trait method return type",
                            );
                        }
                    }
                }
            }
            if !defining_trait_names.is_empty() {
                let _ = self.infer_ctx.constrain(
                    &obj_ty,
                    super::unify::TypeConstraint::Trait(defining_trait_names),
                    span,
                    "method call requires trait",
                );
            }

            self.deferred_methods.push(super::DeferredMethod {
                receiver_ty: obj_ty.clone(),
                method: method.into(),
                arg_tys,
                ret_ty: ret_ty.clone(),
                span,
            });
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::DeferredMethod(Box::new(hobj), method.into(), hargs),
            ty: ret_ty,
            span,
        })
    }

    pub(crate) fn lower_pipe(
        &mut self,
        left: &ast::Expr,
        right: &ast::Expr,
        extra_args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        let hleft = self.lower_expr(left)?;
        if let ast::Expr::Ident(name, _) = right {
            if let Some(gf) = self.generic_fns.get(name).cloned() {
                let left_ty = hleft.ty.clone();
                let mut type_map = HashMap::new();
                if let Some(p) = gf.params.first() {
                    if let Some(Type::Param(tp)) = &p.ty {
                        type_map.insert(tp.clone(), left_ty);
                    }
                }
                for tp in &gf.type_params {
                    type_map.entry(tp.clone()).or_insert(Type::I64);
                }
                let mangled = self.monomorphize_fn(&name.as_str(), &type_map)?;
                let (id, _, ret) = self
                    .fns
                    .get(&mangled)
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "internal compiler error: monomorphized fn '{mangled}' not found after instantiation"
                        )
                    })?;
                let mut all_args = vec![hleft];
                for a in extra_args {
                    all_args.push(self.lower_expr(a)?);
                }
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Call(id, mangled, all_args),
                    ty: ret,
                    span,
                });
            }
            if let Some((id, _, ret)) = self.fns.get(name).cloned() {
                let mut all_args = vec![hleft];
                for a in extra_args {
                    all_args.push(self.lower_expr(a)?);
                }
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Pipe(
                        Box::new(all_args.remove(0)),
                        id,
                        name.clone(),
                        all_args,
                    ),
                    ty: ret,
                    span,
                });
            }
            let hright = self.lower_expr(right)?;
            let ret = match &hright.ty {
                Type::Fn(params, r) => {
                    if let Some(first_param) = params.first() {
                        let r =
                            self.infer_ctx
                                .unify_at(&hleft.ty, first_param, span, "pipe argument");
                        self.collect_unify_error(r);
                    }
                    *r.clone()
                }
                _ => self.infer_ctx.fresh_var(),
            };
            let mut all_args = vec![hleft];
            for a in extra_args {
                all_args.push(self.lower_expr(a)?);
            }
            return Ok(hir::Expr {
                kind: hir::ExprKind::IndirectCall(Box::new(hright), all_args),
                ty: ret,
                span,
            });
        }

        if let ast::Expr::Call(callee, call_args, _) = right {
            if let ast::Expr::Ident(name, _) = callee.as_ref() {
                let has_placeholder = call_args
                    .iter()
                    .any(|a| matches!(a, ast::Expr::Placeholder(_)));
                let mut all_args = Vec::new();
                if has_placeholder {
                    for a in call_args {
                        if matches!(a, ast::Expr::Placeholder(_)) {
                            all_args.push(hleft.clone());
                        } else {
                            all_args.push(self.lower_expr(a)?);
                        }
                    }
                } else {
                    all_args.push(hleft.clone());
                    for a in call_args {
                        all_args.push(self.lower_expr(a)?);
                    }
                }
                if let Some(gf) = self.generic_fns.get(name).cloned() {
                    let left_ty = all_args[0].ty.clone();
                    let mut type_map = HashMap::new();
                    if let Some(p) = gf.params.first() {
                        if let Some(Type::Param(tp)) = &p.ty {
                            type_map.insert(tp.clone(), left_ty);
                        }
                    }
                    for tp in &gf.type_params {
                        type_map.entry(tp.clone()).or_insert(Type::I64);
                    }
                    let mangled = self.monomorphize_fn(&name.as_str(), &type_map)?;
                    let mangled_sym = mangled;
                    let (id, _, ret) = self
                        .fns
                        .get(&mangled)
                        .cloned()
                        .ok_or_else(|| {
                            format!(
                                "internal compiler error: monomorphized fn '{mangled}' not found after instantiation"
                            )
                        })?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Call(id, mangled_sym, all_args),
                        ty: ret,
                        span,
                    });
                }
                if let Some((id, _, ret)) = self.fns.get(name).cloned() {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Pipe(
                            Box::new(all_args.remove(0)),
                            id,
                            name.clone(),
                            all_args,
                        ),
                        ty: ret,
                        span,
                    });
                }
            }
        }

        let hright = self.lower_expr(right)?;
        let ret = match &hright.ty {
            Type::Fn(params, r) => {
                if let Some(first_param) = params.first() {
                    let _ = self
                        .infer_ctx
                        .unify_at(&hleft.ty, first_param, span, "pipe argument");
                }
                *r.clone()
            }
            _ => self.infer_ctx.fresh_var(),
        };
        let mut all_args = vec![hleft];
        for a in extra_args {
            all_args.push(self.lower_expr(a)?);
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::IndirectCall(Box::new(hright), all_args),
            ty: ret,
            span,
        })
    }

    pub(crate) fn build_type_map(
        &mut self,
        name: &str,
        generic_fn: &ast::Fn,
        arg_tys: &[Type],
    ) -> HashMap<Symbol, Type> {
        if !self.generic_fns.contains_key(name) {
            self.generic_fns
                .insert(name.into(), generic_fn.clone());
        }
        let mut type_map = HashMap::new();
        for (i, p) in generic_fn.params.iter().enumerate() {
            if let Some(Type::Param(tp)) = &p.ty {
                if i < arg_tys.len() {
                    type_map.insert(tp.clone(), arg_tys[i].clone());
                }
            }
        }
        for tp in &generic_fn.type_params {
            type_map.entry(tp.clone()).or_insert(Type::I64);
        }
        type_map
    }

    fn monomorphize_call(
        &mut self,
        name: &str,
        type_map: &HashMap<Symbol, Type>,
        mut hargs: Vec<hir::Expr>,
        span: Span,
        coerce: bool,
    ) -> Result<hir::Expr, String> {
        let mangled = self.monomorphize_fn(name, type_map)?;
        let (id, mono_param_tys, ret) = self
            .fns
            .get(&mangled)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "internal compiler error: monomorphized fn '{mangled}' not found after instantiation"
                )
            })?;
        if coerce {
            for (i, ha) in hargs.iter_mut().enumerate() {
                if let Some(pt) = mono_param_tys.get(i) {
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
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::Call(id, mangled, hargs),
            ty: ret,
            span,
        })
    }
}
