use std::collections::HashMap;

use crate::ast::{self, Span};
use crate::hir;
use crate::types::Type;

use super::{Typer, VarInfo};

impl Typer {
    pub(crate) fn lower_call(
        &mut self,
        callee: &ast::Expr,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        if let ast::Expr::Ident(name, _) = callee {
            if let Some(result) = self.try_lower_builtin_call(name, args, span) {
                return result;
            }

            // ── Scheme-based polymorphic call resolution (HM generalization) ──
            // If the function has a generalized scheme in fn_schemes, instantiate
            // it with fresh TypeVars so each call site gets independent type
            // solutions. Then monomorphize for codegen with the resolved types.
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

                    // Lower args with expected types from instantiation
                    let mut hargs: Vec<hir::Expr> = Vec::new();
                    for (i, arg) in args.iter().enumerate() {
                        let expected = inst_params.get(i);
                        hargs.push(self.lower_expr_expected(arg, expected)?);
                    }

                    // Unify each arg with the instantiated param type
                    for (i, ha) in hargs.iter().enumerate() {
                        if let Some(pt) = inst_params.get(i) {
                            let r = self
                                .infer_ctx
                                .unify_at(pt, &ha.ty, span, "function argument");
                            self.collect_unify_error(r);
                        }
                    }

                    // Resolve instantiated types to concrete types for monomorphization.
                    // Temporarily disable strict mode to avoid false positives on
                    // TypeVars that are properly being defaulted from call-site arguments
                    // (e.g., integer literal `40` → Integer constraint → I64 default).
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
                    let type_map = self.build_type_map(name, &normalized, &arg_tys);
                    return self.monomorphize_call(name, &type_map, hargs, span, true);
                }
            }

            if let Some(gf) = self.generic_fns.get(name).cloned() {
                // Explicitly generic functions (with Type::Param) — monomorphize directly
                // Skip inferable fns that have poly schemes (handled above by scheme path)
                // Skip inferable fns that don't have schemes YET (self-recursive lowering)
                // Skip inferable fns with non-poly schemes (monomorphic) — they should
                // fall through to self.fns path for proper arg/param unification
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
                    let type_map = self.build_type_map(name, &gf, &arg_tys);
                    return self.monomorphize_call(name, &type_map, hargs, span, false);
                }
            }

            // Auto-monomorphization fallback for inferable functions:
            // When a function with unannotated params has its TypeVars already solved
            // to concrete types that conflict with the current call's arg types,
            // fall back to monomorphization.
            // Skip during initial body lowering (before scheme is built) to preserve
            // TypeVar polymorphism for generalization.
            if let Some(inf_fn) = self.inferable_fns.get(name).cloned() {
                // Skip if already handled by scheme-based path above
                // Also skip if scheme hasn't been built yet (self-recursive lowering)
                if !self.fn_schemes.get(name).map_or(false, |s| !s.0.is_empty())
                    && self.fn_schemes.contains_key(name)
                {
                    if let Some((_, param_tys, _)) = self.fns.get(name).cloned() {
                        let hargs: Vec<hir::Expr> = args
                            .iter()
                            .map(|e| self.lower_expr(e))
                            .collect::<Result<_, _>>()?;
                        // Use shallow_resolve (not resolve) to avoid triggering
                        // strict errors on TypeVars that will be resolved via
                        // unification in the needs_mono check below
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
                            let type_map = self.build_type_map(name, &normalized, &arg_tys);
                            return self.monomorphize_call(name, &type_map, hargs, span, false);
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

            if let Some(v) = self.find_var(name).cloned() {
                // Poly-lambda monomorphization: when a let-bound lambda has a poly
                // scheme AND we have its AST stored, re-lower it with the concrete
                // types from this call site. This produces a separate specialized
                // function for each type combination (like top-level inferable fns).
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

                            // Lower args with expected types from instantiated scheme
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

                            // Resolve the instantiated types to concrete types
                            let was_strict = self.infer_ctx.is_strict();
                            self.infer_ctx.set_strict(false);
                            let resolved_params: Vec<Type> = inst_params
                                .iter()
                                .map(|t| self.infer_ctx.resolve(t))
                                .collect();
                            let resolved_ret = self.infer_ctx.resolve(&inst_ret);
                            self.infer_ctx.set_strict(was_strict);

                            // Generate mangled name for this specialization
                            let type_suffix: Vec<String> =
                                resolved_params.iter().map(|t| format!("{t}")).collect();
                            let mangled = format!("__poly_{name}_{}", type_suffix.join("_"));

                            // Check if already monomorphized
                            if let Some((id, _, ret)) = self.fns.get(&mangled).cloned() {
                                return Ok(hir::Expr {
                                    kind: hir::ExprKind::Call(id, mangled, hargs),
                                    ty: ret,
                                    span,
                                });
                            }

                            // Re-lower the lambda body within current scope (captures work)
                            let fn_id = self.fresh_id();
                            self.fns.insert(
                                mangled.clone(),
                                (fn_id, resolved_params.clone(), resolved_ret.clone()),
                            );

                            self.push_scope();
                            let mut fn_params = Vec::new();
                            for (i, p) in lparams.iter().enumerate() {
                                let pid = self.fresh_id();
                                let ty = resolved_params.get(i).cloned().unwrap_or(Type::I64);
                                let ownership = Self::ownership_for_type(&ty);
                                self.define_var(
                                    &p.name,
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
                                    span: p.span,
                                });
                            }

                            let hbody = self.lower_block_no_scope(&lbody, &resolved_ret)?;
                            self.pop_scope();

                            // Resolve return type from body tail expression
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

                            // Update fn signature with resolved return
                            if let Some(entry) = self.fns.get_mut(&mangled) {
                                entry.2 = final_ret.clone();
                            }

                            let mono_fn = hir::Fn {
                                def_id: fn_id,
                                name: mangled.clone(),
                                params: fn_params,
                                ret: final_ret.clone(),
                                body: hbody,
                                span: lspan,
                                generic_origin: Some(name.to_string()),
                            };
                            self.mono_fns.push(mono_fn);

                            return Ok(hir::Expr {
                                kind: hir::ExprKind::Call(fn_id, mangled, hargs),
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
                // Phase 6: Higher-order inference — when a variable with TypeVar
                // type is called, unify it with Fn(arg_tys) -> fresh_ret
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

            let _hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            return Err(format!("undefined function: '{name}'"));
        }

        let hcallee = self.lower_expr(callee)?;
        let callee_resolved = self.infer_ctx.shallow_resolve(&hcallee.ty);
        let (ptys, ret) = match &callee_resolved {
            Type::Fn(ptys, ret) => (ptys.clone(), *ret.clone()),
            _ => {
                // Phase 6: Higher-order inference — construct Fn type from args
                // and unify with the callee's TypeVar
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
        // Phase 1.2: Lower object FIRST, then dispatch on its resolved HIR type.
        let hobj = self.lower_expr(obj)?;
        let obj_ty = self.infer_ctx.shallow_resolve(&hobj.ty);

        if matches!(obj_ty, Type::String) {
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let ret_ty = match method {
                "contains" | "starts_with" | "ends_with" => Type::Bool,
                "char_at" | "len" | "find" => Type::I64,
                "slice" | "trim" | "trim_left" | "trim_right" | "replace" | "to_upper"
                | "to_lower" => Type::String,
                "split" => Type::Vec(Box::new(Type::String)),
                _ => Type::I64,
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::StringMethod(Box::new(hobj), method.to_string(), hargs),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Vec(ref elem_ty) = obj_ty {
            let expected_arg_tys: Vec<Option<&Type>> = match method {
                "push" => vec![Some(elem_ty.as_ref())],
                "set" => vec![Some(&Type::I64), Some(elem_ty.as_ref())],
                "get" | "remove" => vec![Some(&Type::I64)],
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
                "pop" | "get" | "remove" => *elem_ty.clone(),
                "len" => Type::I64,
                "set" => Type::Void,
                _ => return Err(format!("no method '{method}' on Vec")),
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::VecMethod(Box::new(hobj), method.to_string(), hargs),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Map(ref key_ty, ref val_ty) = obj_ty {
            let expected_arg_tys: Vec<Option<&Type>> = match method {
                "set" => vec![Some(key_ty.as_ref()), Some(val_ty.as_ref())],
                "get" | "has" | "remove" => vec![Some(key_ty.as_ref())],
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
                "set" | "remove" | "clear" => Type::Void,
                "get" => *val_ty.clone(),
                "has" => Type::Bool,
                "len" => Type::I64,
                "keys" => Type::Vec(key_ty.clone()),
                "values" => Type::Vec(val_ty.clone()),
                _ => return Err(format!("no method '{method}' on Map")),
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::MapMethod(Box::new(hobj), method.to_string(), hargs),
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

        if let Type::DynTrait(ref trait_name) = obj_ty {
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let ret_ty = self.infer_dyn_method_ret(trait_name, method);
            return Ok(hir::Expr {
                kind: hir::ExprKind::DynDispatch(
                    Box::new(hobj),
                    trait_name.clone(),
                    method.to_string(),
                    hargs,
                ),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Struct(ref type_name) = obj_ty {
            let method_name = format!("{type_name}_{method}");
            if let Some((_, param_tys, ret)) = self.fns.get(&method_name).cloned() {
                // param_tys[0] is self, actual args start at [1]
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
                        method_name,
                        method.to_string(),
                        hargs,
                    ),
                    ty: ret,
                    span,
                });
            }
        }

        // Fallback: if receiver is a TypeVar, try row polymorphism.
        // Search for struct methods matching `_method` suffix to infer struct type.
        if matches!(obj_ty, Type::TypeVar(_)) {
            let suffix = format!("_{method}");
            let mut candidates: Vec<(String, Vec<Type>, Type)> = self
                .fns
                .iter()
                .filter(|(name, _)| name.ends_with(&suffix))
                .map(|(name, (_, ptys, ret))| {
                    let type_name = name[..name.len() - suffix.len()].to_string();
                    (type_name, ptys.clone(), ret.clone())
                })
                .filter(|(type_name, _, _)| self.structs.contains_key(type_name))
                .collect();

            // Phase 3A: When multiple candidates, narrow using trait information.
            // Find traits that define this method, then keep only candidates whose
            // type implements at least one such trait.
            if candidates.len() > 1 {
                let defining_traits: Vec<&String> = self
                    .traits
                    .iter()
                    .filter(|(_, sigs)| sigs.iter().any(|s| s.name == method))
                    .map(|(tname, _)| tname)
                    .collect();
                if !defining_traits.is_empty() {
                    let narrowed: Vec<(String, Vec<Type>, Type)> = candidates
                        .iter()
                        .filter(|(type_name, _, _)| {
                            self.trait_impls.get(type_name).map_or(false, |impls| {
                                impls.iter().any(|i| defining_traits.contains(&i))
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
                let struct_ty = Type::Struct(type_name.clone());
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
                        method_name,
                        method.to_string(),
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

            // Trait-guided inference: if a trait defines this method,
            // (1) add a Trait constraint on the receiver TypeVar
            // (2) use the return type from the trait signature as a constraint
            let mut defining_trait_names: Vec<String> = Vec::new();
            for (trait_name, sigs) in &self.traits {
                for sig in sigs {
                    if sig.name == method {
                        defining_trait_names.push(trait_name.clone());
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
            // Add trait constraint on the receiver TypeVar
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
                method: method.to_string(),
                arg_tys,
                ret_ty: ret_ty.clone(),
                span,
            });
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::StringMethod(Box::new(hobj), method.to_string(), hargs),
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
                let mangled = self.monomorphize_fn(name, &type_map)?;
                let (id, _, ret) = self.fns.get(&mangled).cloned().unwrap();
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
                    // R4.3: Unify pipe left type with function's first param
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
                    let mangled = self.monomorphize_fn(name, &type_map)?;
                    let (id, _, ret) = self.fns.get(&mangled).cloned().unwrap();
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Call(id, mangled, all_args),
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
                // R4.3: Unify pipe left type with function's first param
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

    /// Build a type_map from a generic function's params and resolved arg types.
    /// Ensures the generic fn is registered in `self.generic_fns`.
    fn build_type_map(
        &mut self,
        name: &str,
        generic_fn: &ast::Fn,
        arg_tys: &[Type],
    ) -> HashMap<String, Type> {
        if !self.generic_fns.contains_key(name) {
            self.generic_fns
                .insert(name.to_string(), generic_fn.clone());
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

    /// Monomorphize a function call and build the resulting Call expression.
    /// When `coerce` is true, coerces arguments to match the monomorphized parameter types.
    fn monomorphize_call(
        &mut self,
        name: &str,
        type_map: &HashMap<String, Type>,
        mut hargs: Vec<hir::Expr>,
        span: Span,
        coerce: bool,
    ) -> Result<hir::Expr, String> {
        let mangled = self.monomorphize_fn(name, type_map)?;
        let (id, mono_param_tys, ret) = self.fns.get(&mangled).cloned().unwrap();
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
