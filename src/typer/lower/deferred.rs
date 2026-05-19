#![allow(unused_imports, unused_variables)]

use std::collections::{HashMap, HashSet};

use super::super::unify;
use super::super::{DeferredField, DeferredMethod, Typer, VarInfo};
use crate::ast::{self, Span};
use crate::hir::{self, CoercionKind, DefId, ExprKind, Ownership};
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn resolve_deferred_methods(&mut self) {
        let deferred = std::mem::take(&mut self.deferred_methods);
        for dm in &deferred {
            let recv_ty = self.infer_ctx.shallow_resolve(&dm.receiver_ty);
            match &recv_ty {
                Type::Vec(elem_ty) => {
                    let elem = elem_ty.as_ref().clone();
                    if dm.method == "push" {
                        if let Some(arg_ty) = dm.arg_tys.first() {
                            let _ = self
                                .infer_ctx
                                .unify_at(&elem, arg_ty, dm.span, "vec.push arg");
                        }
                    } else if dm.method == "set" {
                        if let Some(idx_ty) = dm.arg_tys.first() {
                            let _ = self.infer_ctx.unify_at(
                                &Type::I64,
                                idx_ty,
                                dm.span,
                                "vec.set index",
                            );
                        }
                        if let Some(val_ty) = dm.arg_tys.get(1) {
                            let _ =
                                self.infer_ctx
                                    .unify_at(&elem, val_ty, dm.span, "vec.set value");
                        }
                    }
                    let actual_ret = match Self::vec_method_ret_ty(&dm.method.as_str(), &elem) {
                        Some(ty) => ty,
                        None => continue,
                    };
                    let _ = self.infer_ctx.unify_at(
                        &dm.ret_ty,
                        &actual_ret,
                        dm.span,
                        "deferred vec method return",
                    );
                }
                Type::Map(key_ty, val_ty) => {
                    let key = key_ty.as_ref().clone();
                    let val = val_ty.as_ref().clone();
                    match &*dm.method.as_str() {
                        "set" => {
                            if let Some(k) = dm.arg_tys.first() {
                                let _ = self.infer_ctx.unify_at(&key, k, dm.span, "map.set key");
                            }
                            if let Some(v) = dm.arg_tys.get(1) {
                                let _ = self.infer_ctx.unify_at(&val, v, dm.span, "map.set value");
                            }
                        }
                        "get" | "has" => {
                            if let Some(k) = dm.arg_tys.first() {
                                let reason = if dm.method == "get" {
                                    "map.get key"
                                } else {
                                    "map.has key"
                                };
                                let _ = self.infer_ctx.unify_at(&key, k, dm.span, reason);
                            }
                        }
                        _ => {}
                    }
                    let actual_ret = match Self::map_method_ret_ty(&dm.method.as_str(), &key, &val)
                    {
                        Some(ty) => ty,
                        None => continue,
                    };
                    let _ = self.infer_ctx.unify_at(
                        &dm.ret_ty,
                        &actual_ret,
                        dm.span,
                        "deferred map method return",
                    );
                }
                Type::String => {
                    let actual_ret = match Self::string_method_ret_ty(&dm.method.as_str()) {
                        Some(ty) => ty,
                        None => continue,
                    };
                    let _ = self.infer_ctx.unify_at(
                        &dm.ret_ty,
                        &actual_ret,
                        dm.span,
                        "deferred string method return",
                    );
                }
                Type::Struct(type_name, _) => {
                    let method_name = format!("{}_{}", type_name, dm.method);
                    if let Some((_, param_tys, ret)) = self.fns.get(&method_name).cloned() {
                        for (i, arg_ty) in dm.arg_tys.iter().enumerate() {
                            if let Some(expected) = param_tys.get(i + 1) {
                                let _ = self.infer_ctx.unify_at(
                                    expected,
                                    arg_ty,
                                    dm.span,
                                    "deferred struct method arg",
                                );
                            }
                        }
                        let _ = self.infer_ctx.unify_at(
                            &dm.ret_ty,
                            &ret,
                            dm.span,
                            "deferred struct method return",
                        );
                    }
                }
                Type::Coroutine(yield_ty) => {
                    if dm.method == "next" {
                        let _ = self.infer_ctx.unify_at(
                            &dm.ret_ty,
                            yield_ty,
                            dm.span,
                            "deferred coroutine.next return",
                        );
                    }
                }
                _ => {
                    if matches!(recv_ty, Type::TypeVar(_)) {
                        if Self::is_string_exclusive_method(&dm.method.as_str()) {
                            let _ = self.infer_ctx.unify_at(
                                &recv_ty,
                                &Type::String,
                                dm.span,
                                "deferred string-exclusive method implies String",
                            );
                            if let Some(actual_ret) =
                                Self::string_method_ret_ty(&dm.method.as_str())
                            {
                                let _ = self.infer_ctx.unify_at(
                                    &dm.ret_ty,
                                    &actual_ret,
                                    dm.span,
                                    "deferred string method return",
                                );
                            }
                            continue;
                        }

                        let suffix = format!("_{}", dm.method);
                        let mut candidates: Vec<(Symbol, Vec<Type>, Type)> = self
                            .fns
                            .iter()
                            .filter(|(name, _)| name.ends_with(&suffix))
                            .map(|(name, (_, ptys, ret))| {
                                let type_name = {
                                    let __n = name.as_str();
                                    Symbol::intern(&__n[..__n.len() - suffix.len()])
                                };
                                (type_name, ptys.clone(), ret.clone())
                            })
                            .filter(|(type_name, _, _)| self.structs.contains_key(type_name))
                            .collect();

                        if let Type::TypeVar(v) = recv_ty {
                            let constraint = self.infer_ctx.constraint(v);
                            if let super::super::unify::TypeConstraint::Trait(ref required_traits) =
                                constraint
                            {
                                let narrowed: Vec<(Symbol, Vec<Type>, Type)> = candidates
                                    .iter()
                                    .filter(|(type_name, _, _)| {
                                        self.trait_impls.get(type_name).map_or(false, |impls| {
                                            required_traits.iter().all(|rt| impls.contains(rt))
                                        })
                                    })
                                    .cloned()
                                    .collect();
                                if !narrowed.is_empty() {
                                    candidates = narrowed;
                                }
                            }
                        }

                        if candidates.len() > 1 {
                            let defining_traits: Vec<&Symbol> = self
                                .traits
                                .iter()
                                .filter(|(_, sigs)| sigs.iter().any(|s| s.name == dm.method))
                                .map(|(tname, _)| tname)
                                .collect();
                            if !defining_traits.is_empty() {
                                let narrowed: Vec<(Symbol, Vec<Type>, Type)> = candidates
                                    .iter()
                                    .filter(|(type_name, _, _)| {
                                        self.trait_impls.get(type_name).map_or(false, |impls| {
                                            impls.iter().any(|i| {
                                                defining_traits.iter().any(|dt| **dt == i.as_str())
                                            })
                                        })
                                    })
                                    .cloned()
                                    .collect();
                                if !narrowed.is_empty() {
                                    candidates = narrowed;
                                }
                            }
                        }

                        candidates.sort_by(|(a, _, _), (b, _, _)| a.cmp(b));

                        if candidates.len() > 1 {
                            let names: Vec<String> =
                                candidates.iter().map(|(n, _, _)| n.as_str()).collect();
                            self.type_errors.push(format!(
                                "{}: ambiguous method `{}`: multiple types have this method: {}",
                                dm.span.loc(),
                                dm.method,
                                names.join(", ")
                            ));
                        } else if candidates.len() == 1 {
                            let (type_name, param_tys, ret) = &candidates[0];
                            let struct_ty = Type::Struct(*type_name, vec![]);
                            let _ = self.infer_ctx.unify_at(
                                &recv_ty,
                                &struct_ty,
                                dm.span,
                                "deferred method candidate",
                            );
                            for (i, arg_ty) in dm.arg_tys.iter().enumerate() {
                                if let Some(expected) = param_tys.get(i + 1) {
                                    let _ = self.infer_ctx.unify_at(
                                        expected,
                                        arg_ty,
                                        dm.span,
                                        "deferred method arg",
                                    );
                                }
                            }
                            let _ = self.infer_ctx.unify_at(
                                &dm.ret_ty,
                                &ret,
                                dm.span,
                                "deferred method return",
                            );
                        }
                    }
                }
            }
        }
    }

    pub(in crate::typer) fn resolve_deferred_fields(&mut self) {
        let deferred = std::mem::take(&mut self.deferred_fields);

        let mut by_receiver: HashMap<u32, Vec<&super::super::DeferredField>> = HashMap::new();
        let mut resolved_concrete: Vec<&super::super::DeferredField> = Vec::new();
        for df in &deferred {
            let recv_ty = self.infer_ctx.shallow_resolve(&df.receiver_ty);
            match recv_ty {
                Type::TypeVar(v) => {
                    let root = self.infer_ctx.find(v);
                    by_receiver.entry(root).or_default().push(df);
                }
                _ => resolved_concrete.push(df),
            }
        }

        for df in resolved_concrete {
            let recv_ty = self.infer_ctx.shallow_resolve(&df.receiver_ty);
            if let Type::Struct(ref name, _) = recv_ty {
                if let Some(fields) = self.structs.get(name) {
                    if let Some((_, fty)) = fields.iter().find(|(n, _)| n == &df.field_name) {
                        let fty = fty.clone();
                        let _ = self.infer_ctx.unify_at(
                            &df.field_ty,
                            &fty,
                            df.span,
                            "deferred field access",
                        );
                    }
                }
            } else if matches!(recv_ty, Type::String) && df.field_name == "length" {
                let _ = self.infer_ctx.unify_at(
                    &df.field_ty,
                    &Type::I64,
                    df.span,
                    "deferred string.length",
                );
            }
        }

        for (_root, fields) in by_receiver {
            let required_fields: Vec<(String, &Type)> = fields
                .iter()
                .map(|df| (df.field_name.as_str(), &df.field_ty))
                .collect();

            let extra_constraints: Vec<(Symbol, Type)> = self
                .field_constraints
                .get(&_root)
                .cloned()
                .unwrap_or_default();

            let mut candidates: Vec<Symbol> = self
                .structs
                .iter()
                .filter(|(_, struct_fields)| {
                    required_fields
                        .iter()
                        .all(|(req, _)| struct_fields.iter().any(|(n, _)| n == req))
                        && extra_constraints
                            .iter()
                            .all(|(req, _)| struct_fields.iter().any(|(n, _)| n == req))
                })
                .map(|(name, _)| name.clone())
                .collect();
            candidates.sort();

            if candidates.len() > 1 {
                let field_names: Vec<String> =
                    fields.iter().map(|f| f.field_name.as_str()).collect();
                self.type_errors.push(format!(
                    "{}: ambiguous field access ({}): multiple types have these fields: {}",
                    fields[0].span.loc(),
                    field_names.join(", "),
                    Symbol::join_vec(&candidates, ", ")
                ));
            } else if candidates.len() == 1 {
                let sname = &candidates[0];
                let struct_ty = Type::Struct(*sname, vec![]);
                let span = fields[0].span;
                let _ = self.infer_ctx.unify_at(
                    &fields[0].receiver_ty,
                    &struct_ty,
                    span,
                    "deferred field constraints imply struct type",
                );
                if let Some(struct_fields) = self.structs.get(sname).cloned() {
                    for df in &fields {
                        if let Some((_, fty)) =
                            struct_fields.iter().find(|(n, _)| n == &df.field_name)
                        {
                            let _ = self.infer_ctx.unify_at(
                                &df.field_ty,
                                fty,
                                df.span,
                                "deferred field access",
                            );
                        }
                    }
                }
            }
        }
    }

    pub(in crate::typer) fn resolve_trait_constrained_vars(&mut self) {
        let n = self.infer_ctx.num_vars();
        for v in 0..n {
            let root = self.infer_ctx.find(v);
            if root != v {
                continue;
            }
            let resolved = self.infer_ctx.shallow_resolve(&Type::TypeVar(root));
            if !matches!(resolved, Type::TypeVar(_)) {
                continue;
            }
            let constraint = self.infer_ctx.constraint(root);
            if let super::super::unify::TypeConstraint::Trait(ref required_traits) = constraint {
                if required_traits.is_empty() {
                    continue;
                }
                let mut candidates: Vec<String> = Vec::new();
                for (type_name, impl_traits) in &self.trait_impls {
                    if required_traits.iter().all(|rt| impl_traits.contains(rt)) {
                        candidates.push(type_name.as_str());
                    }
                }
                candidates.sort();

                if candidates.len() > 1 {
                    self.type_errors.push(format!(
                        "ambiguous type: multiple types implement traits {}: {}",
                        required_traits.join(" + "),
                        candidates.join(", ")
                    ));
                } else if candidates.len() == 1 {
                    let ty = match &*candidates[0].as_str() {
                        "i8" => Type::I8,
                        "i16" => Type::I16,
                        "i32" => Type::I32,
                        "i64" => Type::I64,
                        "u8" => Type::U8,
                        "u16" => Type::U16,
                        "u32" => Type::U32,
                        "u64" => Type::U64,
                        "f32" => Type::F32,
                        "f64" => Type::F64,
                        "bool" => Type::Bool,
                        "String" => Type::String,
                        name => Type::Struct(name.into(), vec![]),
                    };
                    let _ = self.infer_ctx.unify(&Type::TypeVar(root), &ty);
                }
            }
        }
    }
}
