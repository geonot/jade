use super::super::Typer;
use crate::ast;
use crate::hir::{self, DefId};
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn lower_expr_ident(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Ident(name, span) => {
                if let Some((enum_name, tag)) = self.variant_tags.get(name).cloned() {
                    let is_unit = self
                        .enums
                        .get(&enum_name)
                        .and_then(|vs| vs.iter().find(|(vn, _)| vn == name))
                        .map(|(_, fs)| fs.is_empty())
                        .unwrap_or(false);
                    if is_unit {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(enum_name.clone(), name.clone(), tag),
                            ty: Type::Enum(enum_name),
                            span: *span,
                        });
                    }
                    if let Ok(Some(_mangled)) =
                        self.try_monomorphize_generic_variant(&name.as_str(), None)
                    {
                        let (en2, tag2) = self
                            .variant_tags
                            .get(name)
                            .cloned()
                            .unwrap_or((enum_name.clone(), tag));
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(en2.clone(), name.clone(), tag2),
                            ty: Type::Enum(en2),
                            span: *span,
                        });
                    }
                } else if let Ok(Some(mangled)) =
                    self.try_monomorphize_generic_variant(&name.as_str(), None)
                {
                    if let Some((_, tag)) = self.variant_tags.get(name).cloned() {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(mangled, *name, tag),
                            ty: Type::Enum(mangled),
                            span: *span,
                        });
                    }
                }
                if let Some(v) = self.find_var(&name.as_str()) {
                    let def_id = v.def_id;
                    let mono_ty = v.ty.clone();
                    let scheme_clone = v.scheme.clone();
                    let ty = match (&scheme_clone, expected) {
                        (Some(scheme), Some(exp)) if scheme.is_poly() => {
                            let inst = self.infer_ctx.instantiate(scheme);
                            let _ = self.infer_ctx.unify(&inst, exp);
                            inst
                        }
                        (Some(scheme), _) if scheme.is_poly() => self.infer_ctx.instantiate(scheme),
                        _ => mono_ty,
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Var(def_id, name.clone()),
                        ty,
                        span: *span,
                    });
                }
                if let Some(const_expr) = self.consts.get(name).cloned() {
                    return self.lower_expr(&const_expr);
                }
                if let Some((_expr, _span)) = self.globals.get(name).cloned() {
                    let init_expr = self.lower_expr(&_expr)?;
                    let ty = init_expr.ty.clone();
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::GlobalLoad(name.clone()),
                        ty,
                        span: *span,
                    });
                }
                if let Some((id, ptys, ret)) = self.fns.get(name).cloned() {
                    let fn_ty =
                        if let Some((ref q, ref sp, ref sr)) = self.fn_schemes.get(name).cloned() {
                            if !q.is_empty() {
                                let scheme = crate::types::Scheme {
                                    quantified: q.clone(),
                                    ty: Type::Fn(sp.clone(), Box::new(sr.clone())),
                                };
                                self.infer_ctx.instantiate(&scheme)
                            } else {
                                Type::Fn(ptys, Box::new(ret))
                            }
                        } else {
                            Type::Fn(ptys, Box::new(ret))
                        };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::FnRef(id, name.clone()),
                        ty: fn_ty,
                        span: *span,
                    });
                }
                if self.generic_fns.contains_key(name) {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Var(DefId::BUILTIN, name.clone()),
                        ty: self.infer_ctx.fresh_var(),
                        span: *span,
                    });
                }

                if let Some(ref type_name) = self.current_method_type.clone() {
                    let is_field = self
                        .structs
                        .get(type_name)
                        .map(|fields| fields.iter().any(|(n, _)| n == name))
                        .unwrap_or(false);
                    if is_field {
                        let self_expr = ast::Expr::Ident("self".into(), *span);
                        let field_expr = ast::Expr::Field(Box::new(self_expr), name.clone(), *span);
                        return self.lower_expr(&field_expr);
                    }
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::Var(DefId::BUILTIN, name.clone()),
                    ty: self.infer_ctx.fresh_var(),
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_qualified_ident(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::QualifiedIdent(type_name, variant_name, span) => {
                if let Some(variants) = self.enums.get(type_name) {
                    if let Some((tag, (_, _))) = variants
                        .iter()
                        .enumerate()
                        .find(|(_, (vn, _))| vn == variant_name)
                    {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(
                                type_name.clone(),
                                variant_name.clone(),
                                tag as u32,
                            ),
                            ty: Type::Enum(type_name.clone()),
                            span: *span,
                        });
                    }
                    return Err(format!(
                        "{}: '{}' has no variant '{}'",
                        span.loc(),
                        type_name,
                        variant_name
                    ));
                }
                Err(format!(
                    "{}: '{}' is not an error or enum type",
                    span.loc(),
                    type_name
                ))
            }
            _ => unreachable!(),
        }
    }
}
