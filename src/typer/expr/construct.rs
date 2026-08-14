use super::super::Typer;
use crate::ast::{self, Span};
use crate::hir;
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn lower_expr_struct(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        match expr {
            ast::Expr::Struct(name, inits, span) => {
                let known_struct = self.structs.contains_key(name)
                    || self.generic_types.contains_key(name)
                    || self.variant_tags.contains_key(name)
                    || name.as_str() == "Arena"
                    || name.as_str() == "Pool";
                if !known_struct
                    && inits.iter().all(|fi| fi.name.is_none())
                    && (self.fns.contains_key(name)
                        || self.inferable_fns.contains_key(name)
                        || self.generic_fns.contains_key(name)
                        || self.externs.contains_key(name))
                {
                    let args: Vec<ast::Expr> = inits.iter().map(|fi| fi.value.clone()).collect();
                    let callee = ast::Expr::Ident(*name, *span);
                    return self.lower_call_expected(&callee, &args, *span, expected);
                }
                if self.variant_tags.contains_key(name)
                    && let Some(r) = self.try_lower_variant_with_expected(
                        &name.as_str(),
                        inits,
                        *span,
                        expected,
                    )?
                {
                    return Ok(r);
                }
                self.lower_struct_or_variant(&name.as_str(), inits, *span)
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_builder(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Builder(name, fields, span) => {
                let hfields: Vec<(Symbol, hir::Expr)> = fields
                    .iter()
                    .map(|f| Ok((f.name, self.lower_expr(&f.value)?)))
                    .collect::<Result<_, String>>()?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Builder(*name, hfields),
                    ty: Type::Void,
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }
}

impl Typer {
    pub(crate) fn try_lower_variant_with_expected(
        &mut self,
        name: &str,
        inits: &[ast::FieldInit],
        span: Span,
        expected: Option<&Type>,
    ) -> Result<Option<hir::Expr>, String> {
        let resolved = expected.map(|t| self.infer_ctx.resolve(t));
        let enum_name = match resolved {
            Some(Type::Enum(n)) => n,
            Some(Type::Struct(n, args)) if self.generic_enums.contains_key(&n) => {
                let ge = self.generic_enums.get(&n).cloned().unwrap();
                if args.len() != ge.type_params.len() {
                    return Ok(None);
                }
                let mut type_map = std::collections::HashMap::new();
                for (tp, ta) in ge.type_params.iter().zip(args.iter()) {
                    type_map.insert(*tp, ta.clone());
                }
                self.monomorphize_enum(&n.as_str(), &type_map)?
            }
            _ => return Ok(None),
        };
        let variants = match self.enums.get(&enum_name) {
            Some(v) => v.clone(),
            None => return Ok(None),
        };
        let Some(tag) = variants.iter().position(|(n, _)| n.as_str() == name) else {
            return Ok(None);
        };
        let payload_tys = variants[tag].1.clone();
        let tag = tag as u32;
        let hinits: Vec<hir::FieldInit> = inits
            .iter()
            .enumerate()
            .map(|(i, fi)| {
                let exp = payload_tys.get(i);
                Ok(hir::FieldInit {
                    name: fi.name,
                    value: self.lower_expr_expected(&fi.value, exp)?,
                })
            })
            .collect::<Result<_, String>>()?;
        let mut hinits = hinits;
        self.clone_string_captures_in_inits(&mut hinits);
        Ok(Some(hir::Expr {
            kind: hir::ExprKind::VariantCtor(enum_name, name.into(), tag, hinits),
            ty: Type::Enum(enum_name),
            span,
        }))
    }

    pub(crate) fn lower_struct_or_variant_with_typeargs(
        &mut self,
        name: &str,
        inits: &[ast::FieldInit],
        span: Span,
        type_args: &[Type],
    ) -> Result<hir::Expr, String> {
        if let Some(gtd) = self.generic_types.get(name).cloned() {
            if type_args.len() != gtd.type_params.len() {
                return Err(format!(
                    "type '{}' expects {} type argument(s), got {}",
                    name,
                    gtd.type_params.len(),
                    type_args.len()
                ));
            }
            let mut type_map = std::collections::HashMap::new();
            for (tp, ta) in gtd.type_params.iter().zip(type_args.iter()) {
                type_map.insert(*tp, ta.clone());
            }

            let concrete_fields: Vec<(Symbol, Type)> = gtd
                .fields
                .iter()
                .map(|f| {
                    let ty =
                        f.ty.as_ref()
                            .map(|t| Self::substitute_type_params(t, &type_map))
                            .unwrap_or(Type::I64);
                    (f.name, ty)
                })
                .collect();

            let ty_suffix = gtd
                .type_params
                .iter()
                .map(|tp| format!("{}", type_map.get(tp).unwrap()))
                .collect::<Vec<_>>()
                .join("_");
            let mangled = Symbol::intern(&format!("{name}_{ty_suffix}"));
            self.infer_ctx
                .record_mono_origin(mangled, Symbol::intern(name), type_args.to_vec());

            if !self.structs.contains_key(&mangled) {
                self.structs.insert(mangled, concrete_fields.clone());
                let hir_fields: Vec<hir::Field> = concrete_fields
                    .iter()
                    .map(|(fname, fty)| hir::Field {
                        name: *fname,
                        ty: fty.clone(),
                        default: None,
                        access_mod: None,
                        span,
                    })
                    .collect();
                let htd = hir::TypeDef {
                    def_id: self.fresh_id(),
                    name: mangled,
                    fields: hir_fields,
                    methods: Vec::new(),
                    layout: gtd.layout.clone(),
                    span,
                };
                self.mono_types.push(htd);
                self.instantiate_generic_methods(&gtd, mangled, &type_map);
            }

            let mut hinits: Vec<hir::FieldInit> = Vec::with_capacity(inits.len());
            for (i, fi) in inits.iter().enumerate() {
                let declared_ty = if let Some(fname) = &fi.name {
                    concrete_fields
                        .iter()
                        .find(|(n, _)| n == fname)
                        .map(|(_, ty)| ty.clone())
                } else {
                    concrete_fields.get(i).map(|(_, ty)| ty.clone())
                };
                let val = self.lower_expr_expected(&fi.value, declared_ty.as_ref())?;
                if let Some(declared_ty) = declared_ty.as_ref() {
                    let r =
                        self.infer_ctx
                            .unify_at(declared_ty, &val.ty, span, "generic struct field");
                    self.collect_unify_error(r);
                }
                hinits.push(hir::FieldInit {
                    name: fi.name,
                    value: val,
                });
            }

            self.clone_string_captures_in_inits(&mut hinits);
            return Ok(hir::Expr {
                kind: hir::ExprKind::Struct(mangled, hinits),
                ty: Type::Struct(mangled, vec![]),
                span,
            });
        }

        if self.structs.contains_key(name) {
            return self.lower_struct_or_variant(name, inits, span);
        }

        self.lower_struct_or_variant(name, inits, span)
    }

    fn edit_distance(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.to_ascii_lowercase().chars().collect();
        let b: Vec<char> = b.to_ascii_lowercase().chars().collect();
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        let mut cur = vec![0usize; b.len() + 1];
        for i in 1..=a.len() {
            cur[0] = i;
            for j in 1..=b.len() {
                let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
                cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
            }
            std::mem::swap(&mut prev, &mut cur);
        }
        prev[b.len()]
    }

    fn unknown_constructor_error(&self, name: &str, span: Span) -> String {
        let mut candidates: Vec<String> = self
            .structs
            .keys()
            .map(|s| s.as_str())
            .chain(self.variant_tags.keys().map(|s| s.as_str()))
            .filter(|c| !c.contains("__G_"))
            .collect();
        candidates.sort();
        candidates.dedup();

        const FOREIGN: &[(&str, &str)] = &[
            ("None", "Nothing"),
            ("Null", "Nothing"),
            ("Nil", "Nothing"),
            ("Just", "Some"),
            ("Error", "Err"),
        ];
        if let Some((_, jinn)) = FOREIGN.iter().find(|(foreign, _)| *foreign == name)
            && candidates.iter().any(|c| c == jinn)
        {
            return format!(
                "{}: unknown variant `{}` — Jinn spells this `{}`",
                span.loc(),
                name,
                jinn,
            );
        }
        let budget = (name.len() / 3).clamp(1, 3);
        let near = candidates
            .iter()
            .map(|c| (Self::edit_distance(name, c), c))
            .filter(|(d, _)| *d <= budget)
            .min_by_key(|(d, c)| (*d, c.len()))
            .map(|(_, c)| c);
        match near {
            Some(c) => format!(
                "{}: unknown type or variant `{}` — did you mean `{}`? \
                 (a constructor call must name a declared `type`, `actor`, or enum variant; \
                 check for a missing `use`)",
                span.loc(),
                name,
                c,
            ),
            None => format!(
                "{}: unknown type or variant `{}`: a constructor call must name a declared \
                 `type`, `actor`, or enum variant; declare it, or add the `use` that brings \
                 it into scope",
                span.loc(),
                name,
            ),
        }
    }

    pub(in crate::typer) fn lower_struct_or_variant(
        &mut self,
        name: &str,
        inits: &[ast::FieldInit],
        span: Span,
    ) -> Result<hir::Expr, String> {
        if name == "Arena" && inits.len() == 1 {
            return Err("Arena type removed".into());
        }

        if name == "Pool" && inits.len() == 2 {
            return Err("Pool type removed".into());
        }

        if let Some((enum_name, tag)) = self.variant_tags.get(name).cloned() {
            let variant_fields: Vec<Type> = self
                .enums
                .get(&enum_name)
                .and_then(|vs| vs.iter().find(|(vn, _)| vn == name))
                .map(|(_, ftys)| ftys.clone())
                .unwrap_or_default();
            let hinits: Vec<hir::FieldInit> = inits
                .iter()
                .enumerate()
                .map(|(i, fi)| {
                    let expected = variant_fields.get(i);
                    Ok(hir::FieldInit {
                        name: fi.name,
                        value: self.lower_expr_expected(&fi.value, expected)?,
                    })
                })
                .collect::<Result<_, String>>()?;
            let mut hinits = hinits;
            self.clone_string_captures_in_inits(&mut hinits);
            return Ok(hir::Expr {
                kind: hir::ExprKind::VariantCtor(enum_name, name.into(), tag, hinits),
                ty: Type::Enum(enum_name),
                span,
            });
        }

        let struct_fields = self.structs.get(name).cloned();

        if struct_fields.is_none()
            && let Some(gtd) = self.generic_types.get(name).cloned()
        {
            let mut hinits_g: Vec<hir::FieldInit> = inits
                .iter()
                .map(|fi| {
                    Ok(hir::FieldInit {
                        name: fi.name,
                        value: self.lower_expr(&fi.value)?,
                    })
                })
                .collect::<Result<_, String>>()?;

            let mut type_map = std::collections::HashMap::new();
            for (i, fi) in hinits_g.iter().enumerate() {
                let field_def = if let Some(fname) = &fi.name {
                    gtd.fields.iter().find(|f| &f.name == fname)
                } else {
                    gtd.fields.get(i)
                };
                if let Some(field_def) = field_def
                    && let Some(ref declared_ty) = field_def.ty
                {
                    let cty = fi.value.ty.clone();
                    self.collect_type_mapping(declared_ty, &cty, &mut type_map);
                }
            }

            for v in type_map.values_mut() {
                *v = self.infer_ctx.resolve(v);
            }
            for tp in &gtd.type_params {
                if !type_map.contains_key(tp) {
                    let v = self.infer_ctx.fresh_var_at(
                        span,
                        "type parameter of this generic type is not fixed by any constructor \
                         argument; bind it with an annotation like `as Name<...>` or the \
                         explicit `Name of type(...)` constructor form",
                    );
                    type_map.insert(*tp, v);
                }
            }

            let concrete_fields: Vec<(Symbol, Type)> = gtd
                .fields
                .iter()
                .map(|f| {
                    let ty =
                        f.ty.as_ref()
                            .map(|t| Self::substitute_type_params(t, &type_map))
                            .unwrap_or(Type::I64);
                    (f.name, ty)
                })
                .collect();

            let all_concrete = gtd.type_params.iter().all(|tp| {
                type_map
                    .get(tp)
                    .map(Self::is_concrete_type)
                    .unwrap_or(false)
            });

            let (ctor_name, expr_ty) = if all_concrete {
                let ty_suffix = gtd
                    .type_params
                    .iter()
                    .map(|tp| format!("{}", type_map.get(tp).unwrap_or(&Type::I64)))
                    .collect::<Vec<_>>()
                    .join("_");
                let mangled = Symbol::intern(&format!("{name}_{ty_suffix}"));
                let ordered_args: Vec<Type> = gtd
                    .type_params
                    .iter()
                    .map(|tp| type_map.get(tp).cloned().unwrap_or(Type::I64))
                    .collect();
                self.infer_ctx
                    .record_mono_origin(mangled, Symbol::intern(name), ordered_args);

                if !self.structs.contains_key(&mangled) {
                    self.structs.insert(mangled, concrete_fields.clone());

                    let hir_fields: Vec<hir::Field> = concrete_fields
                        .iter()
                        .map(|(fname, fty)| hir::Field {
                            name: *fname,
                            ty: fty.clone(),
                            default: None,
                            access_mod: None,
                            span,
                        })
                        .collect();
                    let htd = hir::TypeDef {
                        def_id: self.fresh_id(),
                        name: mangled,
                        fields: hir_fields,
                        methods: Vec::new(),
                        layout: gtd.layout.clone(),
                        span,
                    };
                    self.mono_types.push(htd);
                    self.instantiate_generic_methods(&gtd, mangled, &type_map);
                }
                (mangled, Type::Struct(mangled, vec![]))
            } else {
                let base = Symbol::intern(name);
                let args: Vec<Type> = gtd
                    .type_params
                    .iter()
                    .map(|tp| type_map.get(tp).cloned().unwrap_or(Type::I64))
                    .collect();
                (base, Type::Struct(base, args))
            };

            for (i, fi) in hinits_g.iter_mut().enumerate() {
                let declared_ty = if let Some(fname) = &fi.name {
                    concrete_fields
                        .iter()
                        .find(|(n, _)| n == fname)
                        .map(|(_, ty)| ty)
                } else {
                    concrete_fields.get(i).map(|(_, ty)| ty)
                };
                if let Some(declared_ty) = declared_ty {
                    let _ = self.infer_ctx.unify_at(
                        declared_ty,
                        &fi.value.ty,
                        span,
                        "generic struct field",
                    );
                }
            }

            self.clone_string_captures_in_inits(&mut hinits_g);
            return Ok(hir::Expr {
                kind: hir::ExprKind::Struct(ctor_name, hinits_g),
                ty: expr_ty,
                span,
            });
        }

        let mut hinits: Vec<hir::FieldInit> = inits
            .iter()
            .enumerate()
            .map(|(i, fi)| {
                let expected = struct_fields.as_ref().and_then(|fields| {
                    if let Some(fname) = fi.name.as_ref() {
                        fields
                            .iter()
                            .find(|(n, _)| n == fname)
                            .map(|(_, ty)| ty.clone())
                    } else {
                        fields.get(i).map(|(_, ty)| ty.clone())
                    }
                });
                Ok(hir::FieldInit {
                    name: fi.name,
                    value: self.lower_expr_expected(&fi.value, expected.as_ref())?,
                })
            })
            .collect::<Result<_, String>>()?;

        let arg_tys: Vec<Type> = hinits.iter().map(|fi| fi.value.ty.clone()).collect();
        if let Ok(Some(mangled)) = self.try_monomorphize_generic_variant(name, Some(&arg_tys)) {
            let (_, tag) = self.variant_tags.get(name).cloned().unwrap_or((mangled, 0));
            self.clone_string_captures_in_inits(&mut hinits);
            return Ok(hir::Expr {
                kind: hir::ExprKind::VariantCtor(mangled, name.into(), tag, hinits),
                ty: Type::Enum(mangled),
                span,
            });
        }

        if let Some(fields) = self.structs.get(name).cloned() {
            if self.inferred_field_structs.contains(&Symbol::intern(name)) {
                let any_named = hinits.iter().any(|fi| fi.name.is_some());
                let needs_mono = fields.iter().enumerate().any(|(i, (fname, declared_ty))| {
                    let resolved = self.infer_ctx.shallow_resolve(declared_ty);
                    let arg_ty = if let Some(fi) = hinits.iter().find(|fi| fi.name == Some(*fname))
                    {
                        Some(&fi.value.ty)
                    } else if any_named {
                        None
                    } else {
                        hinits.get(i).map(|fi| &fi.value.ty)
                    };
                    let arg_ty = match arg_ty {
                        Some(t) => t,
                        None => return false,
                    };
                    let arg_resolved = self.infer_ctx.shallow_resolve(arg_ty);

                    match &resolved {
                        Type::TypeVar(v) => {
                            let root = self.infer_ctx.find(*v);
                            let constraint = self.infer_ctx.constraint(root);
                            match &arg_resolved {
                                Type::TypeVar(av) => {
                                    let arg_root = self.infer_ctx.find(*av);
                                    let arg_constraint = self.infer_ctx.constraint(arg_root);
                                    super::unify::InferCtx::constraints_conflict(
                                        &constraint,
                                        &arg_constraint,
                                    )
                                }
                                _ => match constraint {
                                    super::unify::TypeConstraint::Integer
                                        if !arg_resolved.is_int() =>
                                    {
                                        true
                                    }
                                    super::unify::TypeConstraint::Float
                                        if !arg_resolved.is_float() =>
                                    {
                                        true
                                    }
                                    super::unify::TypeConstraint::Numeric
                                        if !arg_resolved.is_num() =>
                                    {
                                        true
                                    }
                                    super::unify::TypeConstraint::Addable
                                        if !arg_resolved.is_num()
                                            && !matches!(arg_resolved, Type::String) =>
                                    {
                                        true
                                    }
                                    _ => false,
                                },
                            }
                        }
                        _ => {
                            if matches!(arg_resolved, Type::TypeVar(_)) {
                                return false;
                            }
                            resolved != arg_resolved
                        }
                    }
                });

                if needs_mono {
                    let arg_tys: Vec<Type> = fields
                        .iter()
                        .enumerate()
                        .map(|(i, (fname, declared_ty))| {
                            let provided = if let Some(fi) =
                                hinits.iter().find(|fi| fi.name == Some(*fname))
                            {
                                Some(&fi.value.ty)
                            } else if any_named {
                                None
                            } else {
                                hinits.get(i).map(|fi| &fi.value.ty)
                            };
                            match provided {
                                Some(t) => self.infer_ctx.shallow_resolve(t),
                                None => self.infer_ctx.shallow_resolve(declared_ty),
                            }
                        })
                        .collect();
                    let mangled_name = self.monomorphize_struct(name, &fields, &arg_tys, span)?;
                    self.clone_string_captures_in_inits(&mut hinits);
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Struct(mangled_name, hinits),
                        ty: Type::Struct(mangled_name, vec![]),
                        span,
                    });
                }
            }

            for (i, fi) in hinits.iter_mut().enumerate() {
                let declared_ty = if let Some(fname) = &fi.name {
                    fields.iter().find(|(n, _)| n == fname).map(|(_, ty)| ty)
                } else {
                    fields.get(i).map(|(_, ty)| ty)
                };
                if let Some(declared_ty) = declared_ty {
                    let _ = self.infer_ctx.unify_at(
                        declared_ty,
                        &fi.value.ty,
                        span,
                        "struct literal field",
                    );
                    let taken = std::mem::replace(
                        &mut fi.value,
                        hir::Expr {
                            kind: hir::ExprKind::Void,
                            ty: Type::Void,
                            span,
                        },
                    );
                    fi.value = self.maybe_coerce_to(taken, declared_ty);
                }
            }
        }

        if !self.structs.contains_key(&Symbol::intern(name)) {
            return Err(self.unknown_constructor_error(name, span));
        }

        self.clone_string_captures_in_inits(&mut hinits);
        Ok(hir::Expr {
            kind: hir::ExprKind::Struct(name.into(), hinits),
            ty: Type::Struct(name.into(), vec![]),
            span,
        })
    }
}
