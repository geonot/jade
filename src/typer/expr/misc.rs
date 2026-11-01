use super::super::Typer;
use crate::ast;
use crate::hir;
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn lower_expr_grad(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Grad(inner, span) => {
                let hinner = self.lower_expr(inner)?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Grad(Box::new(hinner)),
                    ty: Type::Void,
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_einsum(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Einsum(notation, operands, span) => {
                let hops: Vec<hir::Expr> = operands
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Einsum(Symbol::intern(notation), hops),
                    ty: Type::Void,
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_of_call(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::OfCall(func, arg, span) => {
                if let ast::Expr::Ident(name, _) = func.as_ref() {
                    match &*name.as_str() {
                        "fields" => {
                            let type_name = match arg.as_ref() {
                                ast::Expr::Ident(s, _) => *s,
                                ast::Expr::Str(s, _) => Symbol::intern(s),
                                _ => return Err("fields of expects a type name".into()),
                            };
                            let fields = self.structs.get(&type_name).cloned().unwrap_or_default();
                            let field_exprs: Vec<hir::Expr> = fields
                                .iter()
                                .map(|(fname, _)| hir::Expr {
                                    kind: hir::ExprKind::Str(fname.as_str()),
                                    ty: Type::String,
                                    span: *span,
                                })
                                .collect();
                            let len = field_exprs.len();
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::Array(field_exprs),
                                ty: Type::Array(Box::new(Type::String), len),
                                span: *span,
                            });
                        }
                        "size" => return self.lower_size_of(arg, *span),
                        _ => {}
                    }
                }

                if let ast::Expr::Ident(name, _) = func.as_ref()
                    && name == "type"
                {
                    if let ast::Expr::Ident(vname, _) = arg.as_ref()
                        && let Some(ty) = self.find_var(&vname.as_str()).map(|i| i.ty.clone())
                    {
                        let resolved = self.infer_ctx.resolve(&ty);
                        let ty_str = format!("{}", resolved);
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::Str(ty_str),
                            ty: Type::String,
                            span: *span,
                        });
                    }
                    let harg = self.lower_expr(arg)?;
                    let resolved = self.infer_ctx.resolve(&harg.ty);
                    let ty_str = format!("{}", resolved);
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Str(ty_str),
                        ty: Type::String,
                        span: *span,
                    });
                }

                let hfunc = self.lower_expr(func)?;
                let harg = self.lower_expr(arg)?;
                let ret_ty = self.infer_ctx.fresh_var();
                Ok(hir::Expr {
                    kind: hir::ExprKind::IndirectCall(Box::new(hfunc), vec![harg]),
                    ty: ret_ty,
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_size_of(
        &mut self,
        arg: &ast::Expr,
        span: ast::Span,
    ) -> Result<hir::Expr, String> {
        let named = match arg {
            ast::Expr::Ident(name, _) if self.find_var(&name.as_str()).is_none() => {
                Some(Self::ident_to_type(&name.as_str()))
            }
            ast::Expr::Str(name, _) => Some(Self::ident_to_type(name)),
            ast::Expr::Tuple(..) | ast::Expr::OfCall(..) => self.expr_to_single_type(arg),
            _ => None,
        };
        let ty = match named {
            Some(t) => self.resolve_ty(t),
            None => {
                let harg = self.lower_expr(arg)?;
                self.infer_ctx.resolve(&harg.ty)
            }
        };
        let known = match &ty {
            Type::Struct(name, args) => args.is_empty() && self.structs.contains_key(name),
            Type::Void | Type::Param(_) | Type::TypeVar(_) => false,
            other => Self::is_concrete_type(other),
        };
        if !known {
            return Err(format!(
                "{}: `size of` needs a concrete, declared type; `{ty}` has no native layout",
                span.loc()
            ));
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::SizeOf(ty),
            ty: Type::I64,
            span,
        })
    }
}
