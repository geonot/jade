use super::super::Typer;
use crate::ast::{self, Span};
use crate::hir;
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

        if let Type::Row(store) = &obj_ty
            && method == "snapshot" {
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
                    *actor_name,
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
            if (is_option || is_result)
                && let Some(e) = self
                    .lower_option_result_method(&hobj, *enum_name, is_option, method, args, span)?
            {
                return Ok(e);
            }
        }

        let struct_type_name = match &obj_ty {
            Type::Struct(name, _) => Some(*name),
            Type::Ptr(inner) => {
                if let Type::Struct(name, _) = inner.as_ref() {
                    Some(*name)
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
                                .is_some_and(|impls| {
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

    fn ok_inner_ty(&self, enum_name: Symbol) -> Type {
        self.enums
            .get(&enum_name)
            .and_then(|vs| vs.first())
            .and_then(|(_, ftys)| ftys.first().cloned())
            .unwrap_or(Type::I64)
    }

    fn err_inner_ty(&self, enum_name: Symbol) -> Type {
        self.enums
            .get(&enum_name)
            .and_then(|vs| vs.get(1))
            .and_then(|(_, ftys)| ftys.first().cloned())
            .unwrap_or(Type::I64)
    }

    fn variant_tag_in(&self, enum_name: Symbol, variant: &str) -> u32 {
        self.enums
            .get(&enum_name)
            .and_then(|vs| vs.iter().position(|(n, _)| n == variant))
            .unwrap_or(0) as u32
    }

    fn enum_is(&self, recv: &hir::Expr, tag: u32, span: Span) -> hir::Expr {
        hir::Expr {
            kind: hir::ExprKind::EnumIs(Box::new(recv.clone()), tag),
            ty: Type::Bool,
            span,
        }
    }

    fn enum_unwrap(
        &self,
        recv: &hir::Expr,
        enum_name: Symbol,
        tag: u32,
        ty: Type,
        span: Span,
    ) -> hir::Expr {
        hir::Expr {
            kind: hir::ExprKind::EnumUnwrap(Box::new(recv.clone()), enum_name, tag),
            ty,
            span,
        }
    }

    fn variant_ctor(
        &self,
        enum_name: Symbol,
        variant: &str,
        payload: Option<hir::Expr>,
        span: Span,
    ) -> hir::Expr {
        let tag = self.variant_tag_in(enum_name, variant);
        let inits = match payload {
            Some(v) => vec![hir::FieldInit { name: None, value: v }],
            None => vec![],
        };
        hir::Expr {
            kind: hir::ExprKind::VariantCtor(enum_name, variant.into(), tag, inits),
            ty: Type::Enum(enum_name),
            span,
        }
    }

    fn lower_option_result_method(
        &mut self,
        hobj: &hir::Expr,
        enum_name: Symbol,
        is_option: bool,
        method: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<Option<hir::Expr>, String> {
        let ok_tag = self.variant_tag_in(enum_name, if is_option { "Some" } else { "Ok" });
        let inner_ty = self.ok_inner_ty(enum_name);

        match method {
            "unwrap" => Ok(Some(self.enum_unwrap(hobj, enum_name, ok_tag, inner_ty, span))),
            "is_some" if is_option => Ok(Some(self.enum_is(hobj, ok_tag, span))),
            "is_none" | "is_nothing" if is_option => {
                let t = self.variant_tag_in(enum_name, "Nothing");
                Ok(Some(self.enum_is(hobj, t, span)))
            }
            "is_ok" if !is_option => Ok(Some(self.enum_is(hobj, ok_tag, span))),
            "is_err" if !is_option => {
                let t = self.variant_tag_in(enum_name, "Err");
                Ok(Some(self.enum_is(hobj, t, span)))
            }
            "unwrap_or" if args.len() == 1 => {
                let default_arg = self.lower_expr_expected(&args[0], Some(&inner_ty))?;
                let is_check = self.enum_is(hobj, ok_tag, span);
                let unwrap_expr =
                    self.enum_unwrap(hobj, enum_name, ok_tag, inner_ty.clone(), span);
                Ok(Some(hir::Expr {
                    kind: hir::ExprKind::Ternary(
                        Box::new(is_check),
                        Box::new(unwrap_expr),
                        Box::new(default_arg),
                    ),
                    ty: inner_ty,
                    span,
                }))
            }
            "map" if args.len() == 1 => {
                let fn_ret = self.infer_ctx.fresh_var_at(span, "map() callback result");
                let fn_ty = Type::Fn(vec![inner_ty.clone()], Box::new(fn_ret.clone()));
                let hf = self.lower_expr_expected(&args[0], Some(&fn_ty))?;
                let _ = self.infer_ctx.unify_at(&fn_ty, &hf.ty, span, "map callback");
                let u_ty = self.infer_ctx.shallow_resolve(&fn_ret);
                let target = if is_option {
                    self.mono_option(&u_ty)?
                } else {
                    let e_ty = self.err_inner_ty(enum_name);
                    self.mono_result(&u_ty, &e_ty)?
                };
                let unwrap_expr =
                    self.enum_unwrap(hobj, enum_name, ok_tag, inner_ty.clone(), span);
                let mapped = hir::Expr {
                    kind: hir::ExprKind::IndirectCall(Box::new(hf), vec![unwrap_expr]),
                    ty: u_ty.clone(),
                    span,
                };
                let ok_name = if is_option { "Some" } else { "Ok" };
                let some_branch = self.variant_ctor(target, ok_name, Some(mapped), span);
                let else_branch = if is_option {
                    self.variant_ctor(target, "Nothing", None, span)
                } else {
                    let err_ty = self.err_inner_ty(enum_name);
                    let err_tag = self.variant_tag_in(enum_name, "Err");
                    let err_val = self.enum_unwrap(hobj, enum_name, err_tag, err_ty, span);
                    self.variant_ctor(target, "Err", Some(err_val), span)
                };
                let is_check = self.enum_is(hobj, ok_tag, span);
                Ok(Some(hir::Expr {
                    kind: hir::ExprKind::Ternary(
                        Box::new(is_check),
                        Box::new(some_branch),
                        Box::new(else_branch),
                    ),
                    ty: Type::Enum(target),
                    span,
                }))
            }
            "and_then" if args.len() == 1 => {
                let fn_ret = self.infer_ctx.fresh_var_at(span, "and_then() callback result");
                let fn_ty = Type::Fn(vec![inner_ty.clone()], Box::new(fn_ret.clone()));
                let hf = self.lower_expr_expected(&args[0], Some(&fn_ty))?;
                let _ = self
                    .infer_ctx
                    .unify_at(&fn_ty, &hf.ty, span, "and_then callback");
                let target = self.infer_ctx.shallow_resolve(&fn_ret);
                let target_name = match &target {
                    Type::Enum(n) => *n,
                    _ => return Err(format!("{}: and_then callback must return an Option/Result", span.loc())),
                };
                let unwrap_expr =
                    self.enum_unwrap(hobj, enum_name, ok_tag, inner_ty.clone(), span);
                let applied = hir::Expr {
                    kind: hir::ExprKind::IndirectCall(Box::new(hf), vec![unwrap_expr]),
                    ty: target.clone(),
                    span,
                };
                let else_branch = if is_option {
                    self.variant_ctor(target_name, "Nothing", None, span)
                } else {
                    let err_ty = self.err_inner_ty(enum_name);
                    let err_tag = self.variant_tag_in(enum_name, "Err");
                    let err_val = self.enum_unwrap(hobj, enum_name, err_tag, err_ty, span);
                    self.variant_ctor(target_name, "Err", Some(err_val), span)
                };
                let is_check = self.enum_is(hobj, ok_tag, span);
                Ok(Some(hir::Expr {
                    kind: hir::ExprKind::Ternary(
                        Box::new(is_check),
                        Box::new(applied),
                        Box::new(else_branch),
                    ),
                    ty: target,
                    span,
                }))
            }
            "ok_or" if is_option && args.len() == 1 => {
                let herr = self.lower_expr(&args[0])?;
                let err_ty = herr.ty.clone();
                let target = self.mono_result(&inner_ty, &err_ty)?;
                let unwrap_expr =
                    self.enum_unwrap(hobj, enum_name, ok_tag, inner_ty.clone(), span);
                let ok_branch = self.variant_ctor(target, "Ok", Some(unwrap_expr), span);
                let err_branch = self.variant_ctor(target, "Err", Some(herr), span);
                let is_check = self.enum_is(hobj, ok_tag, span);
                Ok(Some(hir::Expr {
                    kind: hir::ExprKind::Ternary(
                        Box::new(is_check),
                        Box::new(ok_branch),
                        Box::new(err_branch),
                    ),
                    ty: Type::Enum(target),
                    span,
                }))
            }
            "map_err" if !is_option && args.len() == 1 => {
                let err_ty = self.err_inner_ty(enum_name);
                let fn_ret = self.infer_ctx.fresh_var_at(span, "map_err() callback result");
                let fn_ty = Type::Fn(vec![err_ty.clone()], Box::new(fn_ret.clone()));
                let hf = self.lower_expr_expected(&args[0], Some(&fn_ty))?;
                let _ = self
                    .infer_ctx
                    .unify_at(&fn_ty, &hf.ty, span, "map_err callback");
                let f_ty = self.infer_ctx.shallow_resolve(&fn_ret);
                let target = self.mono_result(&inner_ty, &f_ty)?;
                let ok_val = self.enum_unwrap(hobj, enum_name, ok_tag, inner_ty.clone(), span);
                let ok_branch = self.variant_ctor(target, "Ok", Some(ok_val), span);
                let err_tag = self.variant_tag_in(enum_name, "Err");
                let err_val = self.enum_unwrap(hobj, enum_name, err_tag, err_ty, span);
                let mapped = hir::Expr {
                    kind: hir::ExprKind::IndirectCall(Box::new(hf), vec![err_val]),
                    ty: f_ty.clone(),
                    span,
                };
                let err_branch = self.variant_ctor(target, "Err", Some(mapped), span);
                let is_check = self.enum_is(hobj, ok_tag, span);
                Ok(Some(hir::Expr {
                    kind: hir::ExprKind::Ternary(
                        Box::new(is_check),
                        Box::new(ok_branch),
                        Box::new(err_branch),
                    ),
                    ty: Type::Enum(target),
                    span,
                }))
            }
            "ok" if !is_option => {
                let target = self.mono_option(&inner_ty)?;
                let ok_val = self.enum_unwrap(hobj, enum_name, ok_tag, inner_ty.clone(), span);
                let some_branch = self.variant_ctor(target, "Some", Some(ok_val), span);
                let none_branch = self.variant_ctor(target, "Nothing", None, span);
                let is_check = self.enum_is(hobj, ok_tag, span);
                Ok(Some(hir::Expr {
                    kind: hir::ExprKind::Ternary(
                        Box::new(is_check),
                        Box::new(some_branch),
                        Box::new(none_branch),
                    ),
                    ty: Type::Enum(target),
                    span,
                }))
            }
            "err" if !is_option => {
                let err_ty = self.err_inner_ty(enum_name);
                let target = self.mono_option(&err_ty)?;
                let err_tag = self.variant_tag_in(enum_name, "Err");
                let err_val = self.enum_unwrap(hobj, enum_name, err_tag, err_ty, span);
                let some_branch = self.variant_ctor(target, "Some", Some(err_val), span);
                let none_branch = self.variant_ctor(target, "Nothing", None, span);
                let is_err = self.enum_is(hobj, err_tag, span);
                Ok(Some(hir::Expr {
                    kind: hir::ExprKind::Ternary(
                        Box::new(is_err),
                        Box::new(some_branch),
                        Box::new(none_branch),
                    ),
                    ty: Type::Enum(target),
                    span,
                }))
            }
            _ => Ok(None),
        }
    }

    fn mono_option(&mut self, t: &Type) -> Result<Symbol, String> {
        let mut m = std::collections::HashMap::new();
        m.insert(Symbol::intern("T"), t.clone());
        self.monomorphize_enum("Option", &m)
    }

    fn mono_result(&mut self, t: &Type, e: &Type) -> Result<Symbol, String> {
        let mut m = std::collections::HashMap::new();
        m.insert(Symbol::intern("T"), t.clone());
        m.insert(Symbol::intern("E"), e.clone());
        self.monomorphize_enum("Result", &m)
    }
}
