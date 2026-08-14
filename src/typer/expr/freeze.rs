use crate::ast;
use crate::hir;
use crate::types::Type;

use super::super::{MoveReason, Typer};

impl Typer {
    pub(in crate::typer) fn lower_expr_freeze(
        &mut self,
        inner: &ast::Expr,
        span: ast::Span,
    ) -> Result<hir::Expr, String> {
        let hi = self.lower_expr(inner)?;
        let was_strict = self.infer_ctx.is_strict();
        self.infer_ctx.set_strict(false);
        let resolved = self.infer_ctx.resolve(&hi.ty);
        self.infer_ctx.set_strict(was_strict);
        if matches!(resolved, Type::Frozen(_)) {
            return Err(format!(
                "{}: this value is already frozen: `freeze` is one-way, so freezing \
                 twice does nothing",
                span.loc()
            ));
        }
        if let Some(offense) = self.unfreezable_path(&resolved) {
            return Err(format!(
                "{}: cannot freeze a value of type `{}`: {}",
                span.loc(),
                resolved,
                offense
            ));
        }
        if let Some(pl) = crate::typer::place::place_of_expr(&hi)
            && self.type_is_aggregate(&resolved)
        {
            if self
                .find_var_by_id(pl.root)
                .is_some_and(|info| matches!(info.ownership, hir::Ownership::Borrowed))
            {
                return Err(format!(
                    "{}: cannot freeze `{}`: it is a borrow, not an owner (a parameter \
                     borrows unless the function consumes it); bind a clone first \
                     (`c is copy {}`) and freeze that, or declare the parameter `take`",
                    span.loc(),
                    pl.render(),
                    pl.render(),
                ));
            }
            self.mark_place_moved_checked(pl, MoveReason::Freeze(span), span)?;
        }
        Ok(hir::Expr {
            kind: hi.kind,
            ty: Type::Frozen(Box::new(resolved)),
            span,
        })
    }

    pub(in crate::typer) fn reject_frozen_receiver_write(
        &self,
        peeled_ty: &Type,
        method: &str,
        hobj: &hir::Expr,
        span: ast::Span,
    ) -> Result<(), String> {
        let recv_name = crate::typer::place::place_of_expr(hobj)
            .map(|p| p.render())
            .unwrap_or_else(|| "this value".to_string());
        if crate::typer::mutate_infer::is_builtin_mutating_method(method) {
            return Err(format!(
                "{}: `{}` is frozen: `{}` mutates its receiver, and a frozen value can \
                 never be written; mutate before the `freeze`, or rebuild a new value \
                 and freeze that",
                span.loc(),
                recv_name,
                method,
            ));
        }
        if let Type::Struct(sname, _) = peeled_ty {
            let mangled = crate::intern::Symbol::intern(&format!("{sname}_{method}"));
            let mutates_recv = self
                .fn_param_mutates
                .get(&mangled)
                .and_then(|v| v.first())
                .copied()
                .unwrap_or(false);
            let consumes_recv = matches!(
                self.fn_param_access
                    .get(&mangled)
                    .and_then(|v| v.first())
                    .copied()
                    .flatten(),
                Some(ast::AccessMod::Take)
            );
            if mutates_recv || consumes_recv {
                let what = if mutates_recv { "mutates" } else { "consumes" };
                return Err(format!(
                    "{}: `{}` is frozen: `{}` {} its receiver, and a frozen value can \
                     never be written; mutate before the `freeze`, or rebuild a new \
                     value and freeze that",
                    span.loc(),
                    recv_name,
                    method,
                    what,
                ));
            }
        }
        Ok(())
    }

    pub(in crate::typer) fn peel_frozen_arg(
        &mut self,
        callee: crate::intern::Symbol,
        slot: usize,
        param_ty: Option<&Type>,
        ha: &mut hir::Expr,
        span: ast::Span,
    ) -> Result<(), String> {
        let Type::Frozen(inner) = self.infer_ctx.shallow_resolve(&ha.ty) else {
            return Ok(());
        };
        let Some(pt) = param_ty else { return Ok(()) };
        let pt_res = self.infer_ctx.shallow_resolve(pt);
        if matches!(pt_res, Type::Frozen(_) | Type::TypeVar(_) | Type::Param(_)) {
            return Ok(());
        }
        let mutates = self
            .fn_param_mutates
            .get(&callee)
            .and_then(|v| v.get(slot))
            .copied()
            .unwrap_or(false);
        let consumes = matches!(
            self.fn_param_access
                .get(&callee)
                .and_then(|v| v.get(slot))
                .copied()
                .flatten(),
            Some(ast::AccessMod::Take)
        );
        if mutates || consumes {
            let arg_name = crate::typer::place::place_of_expr(ha)
                .map(|p| p.render())
                .unwrap_or_else(|| "this value".to_string());
            let callee_str = callee.as_str();
            let shown = Self::display_fn_name(&callee_str);
            let what = if mutates {
                "mutates it, and a frozen value can never be written"
            } else {
                "takes ownership, which would thaw it (the new owner would treat it \
                 as mutable), and `freeze` is one-way"
            };
            return Err(format!(
                "{}: `{}` is frozen: parameter {} of `{}` {}; pass a copy instead \
                 (bind `copy {}` to a fresh name), or mutate before the `freeze`",
                span.loc(),
                arg_name,
                slot + 1,
                shown,
                what,
                arg_name,
            ));
        }
        ha.ty = (*inner).clone();
        Ok(())
    }

    pub(in crate::typer) fn frozen_assign_offender(
        &mut self,
        target: &hir::Expr,
    ) -> Option<String> {
        let mut cur = target;
        loop {
            let obj = match &cur.kind {
                hir::ExprKind::Field(obj, _, _) => obj,
                hir::ExprKind::Index(obj, _) => obj,
                hir::ExprKind::Deref(obj) => obj,
                _ => return None,
            };
            if matches!(self.infer_ctx.shallow_resolve(&obj.ty), Type::Frozen(_)) {
                return Some(
                    crate::typer::place::place_of_expr(obj)
                        .map(|p| p.render())
                        .unwrap_or_else(|| "a frozen value".to_string()),
                );
            }
            cur = obj;
        }
    }

    fn unfreezable_path(&self, ty: &Type) -> Option<String> {
        let mut visiting = std::collections::HashSet::new();
        self.unfreezable_path_inner(ty, &mut visiting)
    }

    fn unfreezable_path_inner(
        &self,
        ty: &Type,
        visiting: &mut std::collections::HashSet<crate::intern::Symbol>,
    ) -> Option<String> {
        match ty {
            Type::I8
            | Type::I16
            | Type::I32
            | Type::I64
            | Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::F32
            | Type::F64
            | Type::Bool
            | Type::Void
            | Type::String => None,
            Type::Vec(inner) | Type::Array(inner, _) | Type::Frozen(inner) => {
                self.unfreezable_path_inner(inner, visiting)
            }
            Type::Map(k, v) => self
                .unfreezable_path_inner(k, visiting)
                .or_else(|| self.unfreezable_path_inner(v, visiting)),
            Type::Tuple(elems) => elems
                .iter()
                .find_map(|t| self.unfreezable_path_inner(t, visiting)),
            Type::Alias(_, inner) | Type::Newtype(_, inner) => {
                self.unfreezable_path_inner(inner, visiting)
            }
            Type::Struct(name, args) => {
                if self
                    .struct_attrs
                    .get(name)
                    .map(|a| a.resource)
                    .unwrap_or(false)
                {
                    return Some(format!(
                        "`{name}` is a @resource type, and a resource is a live handle, \
                         not shareable data"
                    ));
                }
                if !visiting.insert(*name) {
                    return None;
                }
                let field_types = self.struct_field_types(name, args);
                let field_names: Vec<crate::intern::Symbol> = self
                    .structs
                    .get(name)
                    .map(|fs| fs.iter().map(|(n, _)| *n).collect())
                    .unwrap_or_default();
                let result = field_types.iter().enumerate().find_map(|(i, fty)| {
                    self.unfreezable_path_inner(fty, visiting).map(|why| {
                        let fname = field_names
                            .get(i)
                            .map(|n: &crate::intern::Symbol| n.to_string())
                            .unwrap_or_else(|| i.to_string());
                        format!("field `{name}.{fname}` blocks it: {why}")
                    })
                });
                visiting.remove(name);
                result
            }
            Type::Enum(name) => {
                if !visiting.insert(*name) {
                    return None;
                }
                let result = self.enums.get(name).and_then(|variants| {
                    variants.iter().find_map(|(vname, ftys)| {
                        ftys.iter()
                            .find_map(|t| self.unfreezable_path_inner(t, visiting))
                            .map(|why| format!("variant `{name}:{vname}` blocks it: {why}"))
                    })
                });
                visiting.remove(name);
                result
            }
            Type::Param(_) | Type::TypeVar(_) => Some(
                "its type is not concrete here; freeze where the concrete type is known".into(),
            ),
            other => Some(format!(
                "`{other}` is not freezable data (raw pointers, channels, actors, \
                 coroutines, generators, functions, views, and open stores stay live)"
            )),
        }
    }
}
