#![allow(unused_imports, unused_variables)]

use std::collections::HashMap;

use super::super::unify;
use super::super::{DeferredField, Typer, VarInfo};
use crate::ast::{self, Expr, Span};
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(crate) fn build_type_map(
        &mut self,
        name: &str,
        generic_fn: &ast::Fn,
        arg_tys: &[Type],
    ) -> HashMap<Symbol, Type> {
        if !self.generic_fns.contains_key(name) {
            self.generic_fns.insert(name.into(), generic_fn.clone());
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

    pub(in crate::typer) fn monomorphize_call(
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
