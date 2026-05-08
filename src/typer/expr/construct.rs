//! Extracted typing rules.

#![allow(unused_imports, unused_variables)]

use super::super::unify;
use super::super::{Typer, VarInfo};
use crate::ast::{self, BinOp, Span, UnaryOp};
use crate::hir::{self, CoercionKind, DefId, Ownership};
use crate::intern::Symbol;
use crate::types::Type;
use std::path::PathBuf;

impl Typer {
    pub(in crate::typer) fn lower_expr_struct(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Struct(name, inits, span) => {
                // The parser routes `Uppercase(args)` directly to Expr::Struct
                // (per N-2 generic-ctor support). If no struct/generic/variant
                // exists with this name but a function does, fall back to a
                // regular function call. This permits identifiers like
                // `MySupervisor_start()` or any uppercase-prefixed function.
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
                    let callee = ast::Expr::Ident(name.clone(), *span);
                    return self.lower_call(&callee, &args, *span);
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
    pub(crate) fn lower_struct_or_variant_with_typeargs(
        &mut self,
        name: &str,
        inits: &[ast::FieldInit],
        span: Span,
        type_args: &[Type],
    ) -> Result<hir::Expr, String> {
        // Generic struct path with explicit type bindings.
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
                type_map.insert(tp.clone(), ta.clone());
            }

            // Build concrete fields and mangled name up front.
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

            if !self.structs.contains_key(&mangled) {
                self.structs.insert(mangled, concrete_fields.clone());
                let hir_fields: Vec<hir::Field> = concrete_fields
                    .iter()
                    .map(|(fname, fty)| hir::Field {
                        name: *fname,
                        ty: fty.clone(),
                        default: None,
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

                for m in &gtd.methods {
                    let mut mono_method = m.clone();
                    for p in &mut mono_method.params {
                        if let Some(ref ty) = p.ty {
                            p.ty = Some(Self::substitute_type_params(ty, &type_map));
                        }
                    }
                    if let Some(ref ret) = mono_method.ret {
                        mono_method.ret = Some(Self::substitute_type_params(ret, &type_map));
                    }
                    self.methods
                        .entry(mangled)
                        .or_default()
                        .push(mono_method.clone());
                    self.declare_method_sig_by_ptr(&mangled.as_str(), &mono_method);
                }
            }

            // Lower the inits with the concrete field types as expected
            // hints, so literal arguments (e.g. `7`) get the right type.
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

            return Ok(hir::Expr {
                kind: hir::ExprKind::Struct(mangled, hinits),
                ty: Type::Struct(mangled, vec![]),
                span,
            });
        }

        // Non-generic struct: explicit `of` arguments are ignored but
        // accepted gracefully so the constructor still works.
        if self.structs.contains_key(name) {
            return self.lower_struct_or_variant(name, inits, span);
        }

        // Variant constructor — just delegate; type args inform inference
        // through the field expected types.
        self.lower_struct_or_variant(name, inits, span)
    }

    pub(in crate::typer) fn lower_struct_or_variant(
        &mut self,
        name: &str,
        inits: &[ast::FieldInit],
        span: Span,
    ) -> Result<hir::Expr, String> {
        // Handle Arena(cap) as builtin
        if name == "Arena" && inits.len() == 1 {
            let harg = self.lower_expr_expected(&inits[0].value, Some(&Type::I64))?;
            let _ = self
                .infer_ctx
                .unify_at(&harg.ty, &Type::I64, span, "Arena capacity");
            return Ok(hir::Expr {
                kind: hir::ExprKind::Builtin(crate::hir::BuiltinFn::ArenaNew, vec![harg]),
                ty: Type::Arena,
                span,
            });
        }

        // Handle Pool(obj_size, count) as builtin
        if name == "Pool" && inits.len() == 2 {
            let hsize = self.lower_expr_expected(&inits[0].value, Some(&Type::I64))?;
            let hcount = self.lower_expr_expected(&inits[1].value, Some(&Type::I64))?;
            let _ = self
                .infer_ctx
                .unify_at(&hsize.ty, &Type::I64, span, "Pool obj_size");
            let _ = self
                .infer_ctx
                .unify_at(&hcount.ty, &Type::I64, span, "Pool count");
            return Ok(hir::Expr {
                kind: hir::ExprKind::Builtin(crate::hir::BuiltinFn::PoolNew, vec![hsize, hcount]),
                ty: Type::Pool,
                span,
            });
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
                        name: fi.name.clone(),
                        value: self.lower_expr_expected(&fi.value, expected)?,
                    })
                })
                .collect::<Result<_, String>>()?;
            return Ok(hir::Expr {
                kind: hir::ExprKind::VariantCtor(enum_name, name.into(), tag, hinits),
                ty: Type::Enum(enum_name),
                span,
            });
        }

        let struct_fields = self.structs.get(name).cloned();

        // ── Generic type monomorphization ──
        // If the struct isn't known but is a generic type, monomorphize it.
        if struct_fields.is_none() {
            if let Some(gtd) = self.generic_types.get(name).cloned() {
                // Lower all field init values first (without expected type — generic).
                let mut hinits_g: Vec<hir::FieldInit> = inits
                    .iter()
                    .map(|fi| {
                        Ok(hir::FieldInit {
                            name: fi.name.clone(),
                            value: self.lower_expr(&fi.value)?,
                        })
                    })
                    .collect::<Result<_, String>>()?;

                // Build type_map: map type params to concrete types from args.
                let mut type_map = std::collections::HashMap::new();
                for (i, fi) in hinits_g.iter().enumerate() {
                    let field_def = if let Some(fname) = &fi.name {
                        gtd.fields.iter().find(|f| &f.name == fname)
                    } else {
                        gtd.fields.get(i)
                    };
                    if let Some(field_def) = field_def {
                        if let Some(ref declared_ty) = field_def.ty {
                            Self::collect_type_mapping(declared_ty, &fi.value.ty, &mut type_map);
                        }
                    }
                }

                // Fill in any unmapped type params with i64 default
                for tp in &gtd.type_params {
                    type_map.entry(tp.clone()).or_insert(Type::I64);
                }

                // Build concrete field list
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

                // Build mangled name
                let ty_suffix = gtd
                    .type_params
                    .iter()
                    .map(|tp| format!("{}", type_map.get(tp).unwrap_or(&Type::I64)))
                    .collect::<Vec<_>>()
                    .join("_");
                let mangled = Symbol::intern(&format!("{name}_{ty_suffix}"));

                if !self.structs.contains_key(&mangled) {
                    // Register the monomorphized struct
                    self.structs.insert(mangled, concrete_fields.clone());

                    // Build HIR TypeDef for codegen
                    let hir_fields: Vec<hir::Field> = concrete_fields
                        .iter()
                        .map(|(fname, fty)| hir::Field {
                            name: *fname,
                            ty: fty.clone(),
                            default: None,
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

                    // Register methods for the monomorphized type
                    for m in &gtd.methods {
                        let mut mono_method = m.clone();
                        // Substitute type params in method param types and return type
                        for p in &mut mono_method.params {
                            if let Some(ref ty) = p.ty {
                                p.ty = Some(Self::substitute_type_params(ty, &type_map));
                            }
                        }
                        if let Some(ref ret) = mono_method.ret {
                            mono_method.ret = Some(Self::substitute_type_params(ret, &type_map));
                        }
                        self.methods
                            .entry(mangled)
                            .or_default()
                            .push(mono_method.clone());
                        self.declare_method_sig_by_ptr(&mangled.as_str(), &mono_method);
                    }
                }

                // Unify field inits with concrete types
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

                return Ok(hir::Expr {
                    kind: hir::ExprKind::Struct(mangled, hinits_g),
                    ty: Type::Struct(mangled, vec![]),
                    span,
                });
            }
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
                    name: fi.name.clone(),
                    value: self.lower_expr_expected(&fi.value, expected.as_ref())?,
                })
            })
            .collect::<Result<_, String>>()?;

        let arg_tys: Vec<Type> = hinits.iter().map(|fi| fi.value.ty.clone()).collect();
        if let Ok(Some(mangled)) = self.try_monomorphize_generic_variant(name, Some(&arg_tys)) {
            let (_, tag) = self.variant_tags.get(name).cloned().unwrap_or((mangled, 0));
            return Ok(hir::Expr {
                kind: hir::ExprKind::VariantCtor(mangled, name.into(), tag, hinits),
                ty: Type::Enum(mangled),
                span,
            });
        }

        if let Some(fields) = self.structs.get(name).cloned() {
            // For structs with inferred fields, check if the resolved field types
            // conflict with the argument types. If so, create a monomorphized variant.
            if self.inferred_field_structs.contains(&Symbol::intern(name)) {
                let needs_mono = fields.iter().enumerate().any(|(i, (fname, declared_ty))| {
                    let resolved = self.infer_ctx.shallow_resolve(declared_ty);
                    let arg_ty = if let Some(fi) = hinits.iter().find(|fi| fi.name == Some(*fname))
                    {
                        Some(&fi.value.ty)
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
                                    // Both TypeVars — check constraint compatibility
                                    let arg_root = self.infer_ctx.find(*av);
                                    let arg_constraint = self.infer_ctx.constraint(arg_root);
                                    super::unify::InferCtx::constraints_conflict(
                                        &constraint,
                                        &arg_constraint,
                                    )
                                }
                                _ => {
                                    // Concrete arg type — check constraint compatibility
                                    match constraint {
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
                                    }
                                }
                            }
                        }
                        _ => {
                            // Already resolved to concrete type — direct comparison
                            if matches!(arg_resolved, Type::TypeVar(_)) {
                                return false;
                            }
                            resolved != arg_resolved
                        }
                    }
                });

                if needs_mono {
                    let arg_tys: Vec<Type> = hinits
                        .iter()
                        .map(|fi| self.infer_ctx.shallow_resolve(&fi.value.ty))
                        .collect();
                    let mangled_name = self.monomorphize_struct(name, &fields, &arg_tys, span)?;
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

        Ok(hir::Expr {
            kind: hir::ExprKind::Struct(name.into(), hinits),
            ty: Type::Struct(name.into(), vec![]),
            span,
        })
    }
}
