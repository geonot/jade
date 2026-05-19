#![allow(unused_imports, unused_variables)]

use std::collections::HashMap;

use super::super::unify;
use super::super::{DeferredField, Typer, VarInfo};
use crate::ast::{self, Expr, Span};
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(crate) fn lower_method_call(
        &mut self,
        obj: &ast::Expr,
        method: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        if let Some(e) = self.dispatch_store_methods(obj, method, args, span)? {
            return Ok(e);
        }

        if let Some(e) = self.dispatch_view_methods(obj, method, args, span)? {
            return Ok(e);
        }

        let hobj = self.lower_expr(obj)?;
        let obj_ty = self.infer_ctx.shallow_resolve(&hobj.ty);

        if let Type::Row(store) = &obj_ty {
            if method == "snapshot" {
                if !args.is_empty() {
                    return Err(format!("{}: `.snapshot()` takes no arguments", span.loc()));
                }
                let struct_ty = Type::Struct(Symbol::intern(&format!("__store_{store}")), vec![]);
                return Ok(hir::Expr {
                    kind: hobj.kind,
                    ty: struct_ty,
                    span,
                });
            }
        }

        if let Type::ActorRef(actor_name) = &obj_ty {
            let (_, _, handlers) = self
                .actors
                .get(actor_name)
                .ok_or_else(|| format!("unknown actor '{actor_name}'"))?
                .clone();
            let (handler_name, handler_ptys, tag) = handlers
                .iter()
                .find(|(n, _, _)| n.as_str() == method)
                .ok_or_else(|| format!("actor '{actor_name}' has no handler '.{method}()'"))?
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

                self.enforce_cross_thread_safe(&harg.ty, span, "actor handler argument")?;
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

        if let Type::Channel(elem_ty) = &obj_ty {
            let elem_ty = (**elem_ty).clone();
            match method {
                "send" => {
                    if args.len() != 1 {
                        return Err(format!(
                            "{}: channel `.send()` takes exactly 1 argument, got {}",
                            span.loc(),
                            args.len()
                        ));
                    }
                    let hval = self.lower_expr_expected(&args[0], Some(&elem_ty))?;
                    let _ = self
                        .infer_ctx
                        .unify_at(&elem_ty, &hval.ty, span, "channel .send()");
                    let hval = self.maybe_coerce_to(hval, &elem_ty);
                    let resolved_elem = self.infer_ctx.shallow_resolve(&elem_ty);
                    self.enforce_cross_thread_safe(&resolved_elem, span, "channel .send()")?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::ChannelSend(Box::new(hobj), Box::new(hval)),
                        ty: Type::Void,
                        span,
                    });
                }
                "recv" => {
                    if !args.is_empty() {
                        return Err(format!(
                            "{}: channel `.recv()` takes no arguments",
                            span.loc()
                        ));
                    }
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::ChannelRecv(Box::new(hobj)),
                        ty: elem_ty,
                        span,
                    });
                }
                "close" => {
                    return Err(format!(
                        "{}: channel close is the statement `close {{ch}}`, not a method; use `close {{ch}}` instead of `.close()`",
                        span.loc()
                    ));
                }
                _ => {
                    return Err(format!(
                        "{}: no method `.{method}()` on channel; available: `.send(v)`, `.recv()`, `.close()`",
                        span.loc()
                    ));
                }
            }
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
            match method {
                "map" => {
                    if args.len() != 1 {
                        return Err("map() requires exactly 1 argument".into());
                    }
                    let ret_elem = self
                        .infer_ctx
                        .fresh_var_at(span, "map() return-element type");
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
                        kind: hir::ExprKind::VecMethod(Box::new(hobj), method.into(), vec![harg]),
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

        if let Type::Enum(ref enum_name) = obj_ty {
            let is_option = enum_name.starts_with("Option_") || enum_name == "Option";
            let is_result = enum_name.starts_with("Result_") || enum_name == "Result";
            if is_option || is_result {
                let variants = self.enums.get(enum_name).cloned().unwrap_or_default();
                match method {
                    "unwrap" => {
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
                    _ => {}
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

        let hobj = hobj;

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

            if method == "log" && args.is_empty() {
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::Log, vec![hobj]),
                    ty: Type::Void,
                    span,
                });
            }
        }

        if matches!(obj_ty, Type::TypeVar(_)) {
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
                            self.trait_impls
                                .get(type_name.as_str())
                                .map_or(false, |impls| {
                                    impls
                                        .iter()
                                        .any(|i| defining_traits.iter().any(|t| **t == i.as_str()))
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
        let ret_ty = self
            .infer_ctx
            .fresh_var_at(span, "unresolved method-call return type");
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
                    super::super::unify::TypeConstraint::Trait(defining_trait_names),
                    span,
                    "method call requires trait",
                );
            }

            self.deferred_methods.push(super::super::DeferredMethod {
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
}
