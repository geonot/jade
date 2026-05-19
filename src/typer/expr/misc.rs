#![allow(unused_imports, unused_variables)]

use super::super::unify;
use super::super::{Typer, VarInfo};
use crate::ast::{self, BinOp, Span, UnaryOp};
use crate::hir::{self, CoercionKind, DefId, Ownership};
use crate::intern::Symbol;
use crate::types::Type;
use std::path::PathBuf;

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
                        "size" => {
                            let size = match arg.as_ref() {
                                ast::Expr::Ident(s, _) => {
                                    if let Some(fields) = self.structs.get(s) {
                                        fields.len() as i64 * 8
                                    } else {
                                        match &*s.as_str() {
                                            "i8" | "u8" | "bool" => 1,
                                            "i16" | "u16" => 2,
                                            "i32" | "u32" | "f32" => 4,
                                            "i64" | "u64" | "f64" => 8,
                                            "String" | "string" | "str" => 24,
                                            _ => 0,
                                        }
                                    }
                                }
                                ast::Expr::Str(s, _) => {
                                    let sym = Symbol::intern(s);
                                    if let Some(fields) = self.structs.get(&sym) {
                                        fields.len() as i64 * 8
                                    } else {
                                        match s.as_str() {
                                            "i8" | "u8" | "bool" => 1,
                                            "i16" | "u16" => 2,
                                            "i32" | "u32" | "f32" => 4,
                                            "i64" | "u64" | "f64" => 8,
                                            "String" | "string" | "str" => 24,
                                            _ => 0,
                                        }
                                    }
                                }
                                _ => {
                                    let harg = self.lower_expr(arg)?;
                                    match &harg.ty {
                                        Type::I8 | Type::U8 | Type::Bool => 1,
                                        Type::I16 | Type::U16 => 2,
                                        Type::I32 | Type::U32 | Type::F32 => 4,
                                        Type::I64 | Type::U64 | Type::F64 => 8,
                                        Type::String => 24,
                                        Type::Struct(sname, _) => self
                                            .structs
                                            .get(sname)
                                            .map(|f| f.len() as i64 * 8)
                                            .unwrap_or(8),
                                        _ => 8,
                                    }
                                }
                            };
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::Int(size),
                                ty: Type::I64,
                                span: *span,
                            });
                        }
                        _ => {}
                    }
                }

                if let ast::Expr::Ident(name, _) = func.as_ref() {
                    if name == "type" {
                        if let ast::Expr::Ident(vname, _) = arg.as_ref() {
                            if let Some(ty) = self.find_var(&vname.as_str()).map(|i| i.ty.clone()) {
                                let resolved = self.infer_ctx.resolve(&ty);
                                let ty_str = format!("{}", resolved);
                                return Ok(hir::Expr {
                                    kind: hir::ExprKind::Str(ty_str),
                                    ty: Type::String,
                                    span: *span,
                                });
                            }
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
}
