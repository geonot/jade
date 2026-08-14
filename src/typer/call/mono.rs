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
        ret_ty: Option<&Type>,
    ) -> HashMap<Symbol, Type> {
        if !self.generic_fns.contains_key(name) {
            self.generic_fns.insert(name.into(), generic_fn.clone());
        }
        let mut type_map = HashMap::new();
        for (i, p) in generic_fn.params.iter().enumerate() {
            if let Some(pt) = &p.ty
                && i < arg_tys.len()
            {
                self.collect_type_mapping(pt, &arg_tys[i], &mut type_map);
            }
        }
        if let (Some(declared_ret), Some(rt)) = (&generic_fn.ret, ret_ty) {
            let mut ret_map = HashMap::new();
            self.collect_type_mapping(declared_ret, rt, &mut ret_map);
            for (k, v) in ret_map {
                if Self::is_concrete_type(&v) {
                    type_map.entry(k).or_insert(v);
                }
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
