use std::collections::HashMap;

use super::super::Typer;
use crate::ast::{self, Span};
use crate::hir;
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
            if let Some(Type::Param(tp)) = &p.ty
                && i < arg_tys.len()
            {
                type_map.insert(*tp, arg_tys[i].clone());
            }
        }
        for tp in &generic_fn.type_params {
            type_map.entry(*tp).or_insert(Type::I64);
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
        // Numeric argument coercion to the concrete monomorphized parameter
        // types. Only applied on the fully-inferred (fn_schemes) path where the
        // argument types are already concrete; on the generic_fns path the
        // argument TypeVars must be *unified* with the parameter types by the
        // caller, not coerced (a Coerce wrapper would leave the var unsolved).
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
