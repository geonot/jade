use crate::types::Type;
use std::collections::HashMap;

use super::{InferCtx, TypeConstraint};

impl InferCtx {
    pub(in crate::typer) fn type_to_impl_name(ty: &Type) -> Option<String> {
        match ty {
            Type::I8 => Some("i8".into()),
            Type::I16 => Some("i16".into()),
            Type::I32 => Some("i32".into()),
            Type::I64 => Some("i64".into()),
            Type::U8 => Some("u8".into()),
            Type::U16 => Some("u16".into()),
            Type::U32 => Some("u32".into()),
            Type::U64 => Some("u64".into()),
            Type::F32 => Some("f32".into()),
            Type::F64 => Some("f64".into()),
            Type::Bool => Some("bool".into()),
            Type::String => Some("String".into()),
            Type::Struct(name, _) => Some(name.as_str()),
            _ => None,
        }
    }

    pub(in crate::typer) fn occurs_in(&mut self, v: u32, ty: &Type) -> bool {
        match ty {
            Type::TypeVar(u) => {
                let root = self.find(*u);
                if root == v {
                    return true;
                }
                if let Some(resolved) = self.types[root as usize].clone() {
                    return self.occurs_in(v, &resolved);
                }
                false
            }
            Type::Array(inner, _)
            | Type::Vec(inner)
            | Type::Ptr(inner)
            | Type::Coroutine(inner)
            | Type::Generator(inner)
            | Type::Channel(inner) => self.occurs_in(v, inner),
            Type::Map(k, val) => self.occurs_in(v, k) || self.occurs_in(v, val),
            Type::Tuple(tys) => tys.iter().any(|t| self.occurs_in(v, t)),
            Type::Fn(params, ret) => {
                params.iter().any(|t| self.occurs_in(v, t)) || self.occurs_in(v, ret)
            }
            _ => false,
        }
    }

    pub(crate) fn canonicalize_type(&mut self, ty: &Type) -> Type {
        match ty {
            Type::TypeVar(v) => {
                let root = self.find(*v);
                if let Some(resolved) = self.types[root as usize].clone() {
                    self.canonicalize_type(&resolved)
                } else {
                    Type::TypeVar(root)
                }
            }
            Type::Array(inner, len) => Type::Array(Box::new(self.canonicalize_type(inner)), *len),
            Type::Vec(inner) => Type::Vec(Box::new(self.canonicalize_type(inner))),
            Type::Map(k, v) => Type::Map(
                Box::new(self.canonicalize_type(k)),
                Box::new(self.canonicalize_type(v)),
            ),
            Type::Tuple(tys) => {
                Type::Tuple(tys.iter().map(|t| self.canonicalize_type(t)).collect())
            }
            Type::Fn(params, ret) => Type::Fn(
                params.iter().map(|t| self.canonicalize_type(t)).collect(),
                Box::new(self.canonicalize_type(ret)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(self.canonicalize_type(inner))),
            Type::Coroutine(inner) => Type::Coroutine(Box::new(self.canonicalize_type(inner))),
            Type::Generator(inner) => Type::Generator(Box::new(self.canonicalize_type(inner))),
            Type::Channel(inner) => Type::Channel(Box::new(self.canonicalize_type(inner))),
            _ => ty.clone(),
        }
    }

    pub(crate) fn shallow_resolve(&mut self, ty: &Type) -> Type {
        match ty {
            Type::TypeVar(v) => {
                let root = self.find(*v);
                if let Some(resolved) = self.types[root as usize].clone() {
                    self.shallow_resolve(&resolved)
                } else {
                    Type::TypeVar(root)
                }
            }
            _ => ty.clone(),
        }
    }

    pub(crate) fn resolve(&mut self, ty: &Type) -> Type {
        self.resolve_core(ty, self.collect_default_warnings)
    }

    pub(in crate::typer) fn resolve_container_elem(&mut self, ty: &Type) -> Type {
        if let Type::TypeVar(v) = ty {
            let root = self.find(*v);
            if let Some(resolved) = self.types[root as usize].clone() {
                return self.resolve_core(&resolved, self.collect_default_warnings);
            }
            let constraint = &self.constraints[root as usize];
            match constraint {
                TypeConstraint::Float => Type::F64,
                TypeConstraint::None | TypeConstraint::Numeric | TypeConstraint::Addable => {
                    Type::I64
                }
                TypeConstraint::Integer => Type::I64,
                TypeConstraint::Trait(_) => self.resolve_core(ty, self.collect_default_warnings),
            }
        } else {
            self.resolve_core(ty, self.collect_default_warnings)
        }
    }

    pub(in crate::typer) fn resolve_core(&mut self, ty: &Type, warn_only: bool) -> Type {
        match ty {
            Type::TypeVar(v) => {
                let root = self.find(*v);
                if let Some(resolved) = self.types[root as usize].clone() {
                    return self.resolve_core(&resolved, warn_only);
                }
                let constraint = &self.constraints[root as usize];
                let default_ty = match constraint {
                    TypeConstraint::Float => Type::F64,
                    _ => Type::I64,
                };
                if warn_only && !self.pedantic {
                    if let Some(origin) = &self.origins[root as usize] {
                        match constraint {
                            TypeConstraint::None => {
                                self.default_warnings.push(format!(
                                    "{}: unsolved type variable defaulted to i64 ({}). Consider adding `: i64` or the appropriate type annotation.",
                                    origin.span.loc(), origin.reason
                                ));
                            }
                            TypeConstraint::Numeric => {
                                self.default_warnings.push(format!(
                                    "{}: numeric type defaults to i64 ({}). Add `: i64` for integer or `: f64` for float.",
                                    origin.span.loc(), origin.reason
                                ));
                            }
                            _ => {}
                        }
                    }
                } else if self.strict_types && !self.quantified_vars.contains(&root) {
                    let usage_notes = {
                        let sites = &self.usage_sites[root as usize];
                        if !sites.is_empty() {
                            let mut notes = String::new();
                            for (site_span, site_reason) in sites.iter().take(5) {
                                notes.push_str(&format!(
                                    "\n  note: used at {} ({})",
                                    site_span.loc(),
                                    site_reason
                                ));
                            }
                            if sites.len() > 5 {
                                notes.push_str(&format!(
                                    "\n  note: ... and {} more usage(s)",
                                    sites.len() - 5
                                ));
                            }
                            notes
                        } else {
                            String::new()
                        }
                    };
                    match constraint {
                        TypeConstraint::None => {
                            let msg = if let Some(origin) = &self.origins[root as usize] {
                                format!(
                                    "{}: ambiguous type: cannot infer type for this expression ({})\n  help: consider adding a type annotation, e.g. `: i64` or `: String`{}",
                                    origin.span.loc(),
                                    origin.reason,
                                    usage_notes
                                )
                            } else {
                                format!(
                                    "ambiguous type: unsolved type variable ?{root}\n  help: add a type annotation to resolve the ambiguity{usage_notes}"
                                )
                            };
                            self.strict_errors.push(msg);
                        }
                        TypeConstraint::Numeric => {
                            let msg = if let Some(origin) = &self.origins[root as usize] {
                                format!(
                                    "{}: numeric type defaults to i64 ({})\n  help: add `: i64` for integer or `: f64` for float",
                                    origin.span.loc(),
                                    origin.reason
                                )
                            } else {
                                format!(
                                    "numeric type defaults to i64 for ?{root}\n  help: add `: i64` for integer or `: f64` for float"
                                )
                            };
                            self.default_warnings.push(msg);
                        }
                        TypeConstraint::Trait(traits) => {
                            let traits_str = traits.join(", ");
                            let msg = if let Some(origin) = &self.origins[root as usize] {
                                format!(
                                    "{}: ambiguous type: cannot infer concrete type for trait-constrained variable (requires: {}) ({})\n  help: add a type annotation for a type that implements {}{}",
                                    origin.span.loc(),
                                    traits_str,
                                    origin.reason,
                                    traits_str,
                                    usage_notes
                                )
                            } else {
                                format!(
                                    "ambiguous type: unsolved type variable ?{root} with trait bound(s) [{traits_str}]\n  help: add a type annotation for a type that implements {traits_str}{usage_notes}"
                                )
                            };
                            self.strict_errors.push(msg);
                        }
                        TypeConstraint::Integer if self.pedantic => {
                            let msg = if let Some(origin) = &self.origins[root as usize] {
                                format!(
                                    "{}: pedantic: integer type defaults to i64 ({})\n  help: add an explicit annotation, e.g. `: i64` or `: i32`",
                                    origin.span.loc(),
                                    origin.reason
                                )
                            } else {
                                format!(
                                    "pedantic: unsolved integer type variable ?{root} defaults to i64\n  help: add an explicit annotation, e.g. `: i64` or `: i32`"
                                )
                            };
                            self.strict_errors.push(msg);
                        }
                        TypeConstraint::Float if self.pedantic => {
                            let msg = if let Some(origin) = &self.origins[root as usize] {
                                format!(
                                    "{}: pedantic: float type defaults to f64 ({})\n  help: add an explicit annotation, e.g. `: f64` or `: f32`",
                                    origin.span.loc(),
                                    origin.reason
                                )
                            } else {
                                format!(
                                    "pedantic: unsolved float type variable ?{root} defaults to f64\n  help: add an explicit annotation, e.g. `: f64` or `: f32`"
                                )
                            };
                            self.strict_errors.push(msg);
                        }
                        _ => {}
                    }
                }
                default_ty
            }
            Type::Array(inner, len) => {
                Type::Array(Box::new(self.resolve_container_elem(inner)), *len)
            }
            Type::Vec(inner) => Type::Vec(Box::new(self.resolve_container_elem(inner))),
            Type::Map(k, v) => Type::Map(
                Box::new(self.resolve_core(k, warn_only)),
                Box::new(self.resolve_core(v, warn_only)),
            ),
            Type::Tuple(tys) => Type::Tuple(
                tys.iter()
                    .map(|t| self.resolve_core(t, warn_only))
                    .collect(),
            ),
            Type::Fn(params, ret) => Type::Fn(
                params
                    .iter()
                    .map(|t| self.resolve_core(t, warn_only))
                    .collect(),
                Box::new(self.resolve_core(ret, warn_only)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(self.resolve_core(inner, warn_only))),
            Type::Coroutine(inner) => {
                Type::Coroutine(Box::new(self.resolve_core(inner, warn_only)))
            }
            Type::Generator(inner) => {
                Type::Generator(Box::new(self.resolve_core(inner, warn_only)))
            }
            Type::Channel(inner) => Type::Channel(Box::new(self.resolve_core(inner, warn_only))),
            _ => ty.clone(),
        }
    }

    pub(crate) fn instantiate(&mut self, scheme: &crate::types::Scheme) -> Type {
        if scheme.quantified.is_empty() {
            return scheme.ty.clone();
        }
        let subst: HashMap<u32, Type> = scheme
            .quantified
            .iter()
            .map(|&v| {
                let root = self.find(v);
                let constraint = self.constraints[root as usize].clone();
                let fresh = match constraint {
                    TypeConstraint::Integer => self.fresh_integer_var(),
                    TypeConstraint::Float => self.fresh_float_var(),
                    TypeConstraint::Numeric => self.fresh_numeric_var(),
                    TypeConstraint::Addable => {
                        let var = self.fresh_var();
                        if let Type::TypeVar(id) = var {
                            let root = self.find(id);
                            self.constraints[root as usize] = TypeConstraint::Addable;
                        }
                        var
                    }
                    TypeConstraint::Trait(ref traits) => {
                        let var = self.fresh_var();
                        if let Type::TypeVar(id) = var {
                            let root = self.find(id);
                            self.constraints[root as usize] = TypeConstraint::Trait(traits.clone());
                        }
                        var
                    }
                    TypeConstraint::None => self.fresh_var(),
                };
                (v, fresh)
            })
            .collect();
        self.substitute(&scheme.ty, &subst)
    }

    pub(in crate::typer) fn substitute(&self, ty: &Type, subst: &HashMap<u32, Type>) -> Type {
        match ty {
            Type::TypeVar(v) => {
                if let Some(replacement) = subst.get(v) {
                    replacement.clone()
                } else {
                    ty.clone()
                }
            }
            Type::Array(inner, len) => Type::Array(Box::new(self.substitute(inner, subst)), *len),
            Type::Vec(inner) => Type::Vec(Box::new(self.substitute(inner, subst))),
            Type::Map(k, v) => Type::Map(
                Box::new(self.substitute(k, subst)),
                Box::new(self.substitute(v, subst)),
            ),
            Type::Tuple(tys) => {
                Type::Tuple(tys.iter().map(|t| self.substitute(t, subst)).collect())
            }
            Type::Fn(params, ret) => Type::Fn(
                params.iter().map(|t| self.substitute(t, subst)).collect(),
                Box::new(self.substitute(ret, subst)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(self.substitute(inner, subst))),
            Type::Coroutine(inner) => Type::Coroutine(Box::new(self.substitute(inner, subst))),
            Type::Generator(inner) => Type::Generator(Box::new(self.substitute(inner, subst))),
            Type::Channel(inner) => Type::Channel(Box::new(self.substitute(inner, subst))),
            _ => ty.clone(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn try_resolve(&mut self, ty: &Type) -> Option<Type> {
        match ty {
            Type::TypeVar(v) => {
                let root = self.find(*v);
                if let Some(resolved) = self.types[root as usize].clone() {
                    self.try_resolve(&resolved)
                } else {
                    None
                }
            }

            _ => Some(ty.clone()),
        }
    }
}
