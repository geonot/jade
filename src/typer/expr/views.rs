use crate::ast;
use crate::hir;
use crate::types::Type;

use super::super::Typer;

pub(crate) fn type_contains_view(ty: &Type) -> bool {
    match ty {
        Type::View(_) => true,
        Type::Vec(i)
        | Type::Array(i, _)
        | Type::Ptr(i)
        | Type::Channel(i)
        | Type::Coroutine(i)
        | Type::Generator(i)
        | Type::Alias(_, i)
        | Type::Newtype(_, i)
        | Type::Frozen(i) => type_contains_view(i),
        Type::Map(k, v) => type_contains_view(k) || type_contains_view(v),
        Type::Tuple(ts) => ts.iter().any(type_contains_view),
        Type::Struct(_, args) => args.iter().any(type_contains_view),
        Type::Fn(ps, r) => ps.iter().any(type_contains_view) || type_contains_view(r),
        _ => false,
    }
}

impl Typer {
    pub(in crate::typer) fn coerce_arg_to_view(
        &mut self,
        param_ty: Option<&Type>,
        ha: &mut hir::Expr,
        span: ast::Span,
    ) {
        let Some(pt) = param_ty else { return };
        let Type::View(want_elem) = self.infer_ctx.shallow_resolve(pt) else {
            return;
        };
        let is_string = matches!(self.infer_ctx.shallow_resolve(&ha.ty), Type::String);
        if is_string {
            if self
                .infer_ctx
                .unify_at(&want_elem, &Type::U8, span, "view parameter element")
                .is_err()
            {
                return;
            }
            let old = std::mem::replace(
                ha,
                hir::Expr {
                    kind: hir::ExprKind::Void,
                    ty: Type::Void,
                    span,
                },
            );
            *ha = hir::Expr {
                kind: hir::ExprKind::StringMethod(Box::new(old), "view_full".into(), vec![]),
                ty: Type::View(want_elem),
                span,
            };
            return;
        }
        let have_elem = match self.infer_ctx.shallow_resolve(&ha.ty) {
            Type::Vec(e) | Type::Array(e, _) => e,
            _ => return,
        };
        let _ = self
            .infer_ctx
            .unify_at(&want_elem, &have_elem, span, "view parameter element");
        let old = std::mem::replace(
            ha,
            hir::Expr {
                kind: hir::ExprKind::Void,
                ty: Type::Void,
                span,
            },
        );
        *ha = hir::Expr {
            kind: hir::ExprKind::VecMethod(Box::new(old), "view_full".into(), vec![]),
            ty: Type::View(want_elem),
            span,
        };
    }

    pub(in crate::typer) fn clone_string_capture(&mut self, e: &mut hir::Expr) {
        let peeled = crate::typer::Typer::peel_move_wrappers(e);
        if !matches!(peeled.kind, hir::ExprKind::Var(..)) {
            return;
        }
        let was_strict = self.infer_ctx.is_strict();
        self.infer_ctx.set_strict(false);
        let resolved = self.infer_ctx.resolve(&e.ty);
        self.infer_ctx.set_strict(was_strict);
        if !matches!(resolved, Type::String | Type::Channel(_)) {
            return;
        }
        let span = e.span;
        let out_ty = if matches!(resolved, Type::String) {
            Type::String
        } else {
            resolved
        };
        let old = std::mem::replace(
            e,
            hir::Expr {
                kind: hir::ExprKind::Void,
                ty: Type::Void,
                span,
            },
        );
        *e = hir::Expr {
            kind: hir::ExprKind::StringMethod(Box::new(old), "__clone".into(), vec![]),
            ty: out_ty,
            span,
        };
    }

    pub(in crate::typer) fn clone_string_captures_in_inits(
        &mut self,
        inits: &mut [hir::FieldInit],
    ) {
        for fi in inits.iter_mut() {
            self.clone_string_capture(&mut fi.value);
        }
    }

    pub(in crate::typer) fn clone_string_captures_in_elems(&mut self, elems: &mut [hir::Expr]) {
        for e in elems.iter_mut() {
            self.clone_string_capture(e);
        }
    }

    pub(in crate::typer) fn reject_view_annotation(
        &mut self,
        ty: &Type,
        what: &str,
        span: ast::Span,
        allow_top_level: bool,
    ) -> Result<(), String> {
        let resolved = self.infer_ctx.shallow_resolve(ty);
        let nested = match &resolved {
            Type::View(inner) if allow_top_level => type_contains_view(inner),
            other => type_contains_view(other),
        };
        if nested {
            return Err(format!(
                "{}: a view cannot appear in {}: views are second-class borrows that \
                 may flow down into calls but never escape — pass the owning container, \
                 or copy the data (`slice` copies)",
                span.loc(),
                what,
            ));
        }
        Ok(())
    }
}
