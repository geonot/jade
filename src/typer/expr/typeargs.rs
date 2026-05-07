//! Extracted typing rules.

#![allow(unused_imports, unused_variables)]

use crate::intern::Symbol;
use std::path::PathBuf;
use crate::ast::{self, BinOp, Span, UnaryOp};
use crate::hir::{self, CoercionKind, DefId, Ownership};
use crate::types::Type;
use super::super::{Typer, VarInfo};
use super::super::unify;

impl Typer {
    pub(in crate::typer) fn collect_type_mapping(
        declared: &Type,
        concrete: &Type,
        map: &mut std::collections::HashMap<Symbol, Type>,
    ) {
        match declared {
            Type::Param(name) => {
                map.entry(name.clone()).or_insert_with(|| concrete.clone());
            }
            Type::Vec(inner) => {
                if let Type::Vec(ci) = concrete {
                    Self::collect_type_mapping(inner, ci, map);
                }
            }
            Type::Ptr(inner) => {
                if let Type::Ptr(ci) = concrete {
                    Self::collect_type_mapping(inner, ci, map);
                }
            }
            Type::Rc(inner) => {
                if let Type::Rc(ci) = concrete {
                    Self::collect_type_mapping(inner, ci, map);
                }
            }
            Type::Fn(params, ret) => {
                if let Type::Fn(cp, cr) = concrete {
                    for (dp, cp) in params.iter().zip(cp.iter()) {
                        Self::collect_type_mapping(dp, cp, map);
                    }
                    Self::collect_type_mapping(ret, cr, map);
                }
            }
            _ => {}
        }
    }

    /// Substitute type parameter names in a type with their concrete types.
    pub(in crate::typer) fn substitute_type_params(
        ty: &Type,
        map: &std::collections::HashMap<Symbol, Type>,
    ) -> Type {
        match ty {
            Type::Param(name) => map.get(name).cloned().unwrap_or_else(|| ty.clone()),
            Type::Vec(inner) => Type::Vec(Box::new(Self::substitute_type_params(inner, map))),
            Type::Ptr(inner) => Type::Ptr(Box::new(Self::substitute_type_params(inner, map))),
            Type::Rc(inner) => Type::Rc(Box::new(Self::substitute_type_params(inner, map))),
            Type::Fn(params, ret) => Type::Fn(
                params
                    .iter()
                    .map(|p| Self::substitute_type_params(p, map))
                    .collect(),
                Box::new(Self::substitute_type_params(ret, map)),
            ),
            Type::Struct(name, args) => Type::Struct(
                name.clone(),
                args.iter()
                    .map(|a| Self::substitute_type_params(a, map))
                    .collect(),
            ),
            _ => ty.clone(),
        }
    }

    /// Convert a type-argument expression as it appears after `of` in a
    /// generic constructor call (e.g. `Box of i64(7)` or
    /// `Pair of (i64, String)(1, "a")`) into the corresponding ordered
    /// list of `Type`s. Returns `None` if any sub-expression cannot be
    /// resolved to a type.
    pub(crate) fn expr_to_type_args(&self, e: &ast::Expr) -> Option<Vec<Type>> {
        match e {
            ast::Expr::Tuple(elems, _) => {
                let mut tys = Vec::with_capacity(elems.len());
                for el in elems {
                    tys.push(self.expr_to_single_type(el)?);
                }
                Some(tys)
            }
            _ => Some(vec![self.expr_to_single_type(e)?]),
        }
    }

    fn expr_to_single_type(&self, e: &ast::Expr) -> Option<Type> {
        match e {
            ast::Expr::Ident(name, _) => Some(Self::ident_to_type(&name.as_str())),
            ast::Expr::OfCall(outer, inner, _) => {
                // E.g. `Vec of i64`, `Box of i64`.
                let outer_name = match outer.as_ref() {
                    ast::Expr::Ident(n, _) => n.as_str(),
                    _ => return None,
                };
                let inner_ty = self.expr_to_single_type(inner)?;
                match &*outer_name {
                    "Vec" => Some(Type::Vec(Box::new(inner_ty))),
                    "Ptr" => Some(Type::Ptr(Box::new(inner_ty))),
                    "Rc" => Some(Type::Rc(Box::new(inner_ty))),
                    other => Some(Type::Struct(Symbol::intern(other), vec![inner_ty])),
                }
            }
            ast::Expr::Tuple(elems, _) => {
                let mut tys = Vec::with_capacity(elems.len());
                for el in elems {
                    tys.push(self.expr_to_single_type(el)?);
                }
                Some(Type::Tuple(tys))
            }
            _ => None,
        }
    }

    fn ident_to_type(n: &str) -> Type {
        match n {
            "i8" => Type::I8,
            "i16" => Type::I16,
            "i32" => Type::I32,
            "int" | "i64" => Type::I64,
            "u8" => Type::U8,
            "u16" => Type::U16,
            "u32" => Type::U32,
            "u64" => Type::U64,
            "f32" => Type::F32,
            "float" | "f64" => Type::F64,
            "bool" => Type::Bool,
            "void" => Type::Void,
            "str" | "String" => Type::String,
            s if s.len() == 1 && s.chars().next().is_some_and(char::is_uppercase) => {
                Type::Param(Symbol::intern(s))
            }
            _ => Type::Struct(Symbol::intern(n), vec![]),
        }
    }
}
