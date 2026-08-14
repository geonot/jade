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
