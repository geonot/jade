use super::super::Typer;
use crate::ast::{self, BinOp, UnaryOp};
use crate::hir;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn lower_expr_bin_op(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::BinOp(lhs, op, rhs, span) => {
                let hl = self.lower_expr(lhs)?;
                let hr = self.lower_expr_expected(rhs, Some(&hl.ty))?;
                let r = self
                    .infer_ctx
                    .unify_at(&hl.ty, &hr.ty, *span, "binary operands");
                self.collect_unify_error(r);

                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Exp => {
                        let resolved_l = self.infer_ctx.shallow_resolve(&hl.ty);
                        let resolved_r = self.infer_ctx.shallow_resolve(&hr.ty);

                        let is_string_concat = matches!(op, BinOp::Add)
                            && (matches!(resolved_l, Type::String)
                                || matches!(resolved_r, Type::String));
                        if is_string_concat {
                            if matches!(resolved_l, Type::TypeVar(_)) {
                                let _ = self.infer_ctx.unify_at(
                                    &hl.ty,
                                    &Type::String,
                                    *span,
                                    "string concatenation",
                                );
                            }
                            if matches!(resolved_r, Type::TypeVar(_)) {
                                let _ = self.infer_ctx.unify_at(
                                    &hr.ty,
                                    &Type::String,
                                    *span,
                                    "string concatenation",
                                );
                            }
                        } else {
                            let arith_constraint = if matches!(op, BinOp::Add) {
                                super::unify::TypeConstraint::Addable
                            } else {
                                super::unify::TypeConstraint::Numeric
                            };
                            if matches!(resolved_l, Type::TypeVar(_)) {
                                let _ = self.infer_ctx.constrain(
                                    &hl.ty,
                                    arith_constraint.clone(),
                                    *span,
                                    "arithmetic operator requires numeric type",
                                );
                            }
                            if matches!(resolved_r, Type::TypeVar(_)) {
                                let _ = self.infer_ctx.constrain(
                                    &hr.ty,
                                    arith_constraint,
                                    *span,
                                    "arithmetic operator requires numeric type",
                                );
                            }
                        }
                    }
                    BinOp::Shl
                    | BinOp::Shr
                    | BinOp::Ushr
                    | BinOp::BitAnd
                    | BinOp::BitOr
                    | BinOp::BitXor => {
                        let _ = self.infer_ctx.constrain(
                            &hl.ty,
                            super::unify::TypeConstraint::Integer,
                            *span,
                            "bitwise operator requires integer type",
                        );
                        let _ = self.infer_ctx.constrain(
                            &hr.ty,
                            super::unify::TypeConstraint::Integer,
                            *span,
                            "bitwise operator requires integer type",
                        );
                    }
                    BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                        let resolved_l = self.infer_ctx.shallow_resolve(&hl.ty);
                        if matches!(resolved_l, Type::TypeVar(_)) {
                            let _ = self.infer_ctx.constrain(
                                &hl.ty,
                                super::unify::TypeConstraint::Numeric,
                                *span,
                                "comparison operator requires numeric type",
                            );
                        }
                    }
                    _ => {}
                }

                let (hl, hr) = self.coerce_binop_operands(hl, hr);

                if matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Exp
                ) {
                    let rl = self.infer_ctx.shallow_resolve(&hl.ty);
                    let rr = self.infer_ctx.shallow_resolve(&hr.ty);
                    let valid = |t: &Type, allow_string: bool| -> bool {
                        match t {
                            Type::TypeVar(_) | Type::Ptr(_) => true,
                            t if t.is_num() => true,
                            Type::String if allow_string => true,

                            Type::Struct(_, _) => true,
                            _ => false,
                        }
                    };
                    let allow_string = matches!(op, BinOp::Add);
                    if !valid(&rl, allow_string) || !valid(&rr, allow_string) {
                        let opname = match op {
                            BinOp::Add => "+",
                            BinOp::Sub => "-",
                            BinOp::Mul => "*",
                            BinOp::Div => "/",
                            BinOp::Mod => "%",
                            BinOp::Exp => "^",
                            _ => unreachable!(),
                        };
                        return Err(format!(
                            "operator `{opname}` not defined for `{rl}` and `{rr}` (line {}); requires numeric{} types or a struct with an `{}` method",
                            span.line,
                            if allow_string { ", String," } else { "" },
                            match op {
                                BinOp::Add => "add",
                                BinOp::Sub => "sub",
                                BinOp::Mul => "mul",
                                BinOp::Div => "div",
                                BinOp::Mod => "mod",
                                BinOp::Exp => "pow",
                                _ => unreachable!(),
                            }
                        ));
                    }
                }

                let result_ty = match op {
                    BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::Le
                    | BinOp::Ge
                    | BinOp::And
                    | BinOp::Or => Type::Bool,
                    _ => hl.ty.clone(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::BinOp(Box::new(hl), *op, Box::new(hr)),
                    ty: result_ty,
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_unary_op(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::UnaryOp(op, inner, span) => {
                let hi = self.lower_expr(inner)?;
                let ty = match op {
                    UnaryOp::Not => {
                        let resolved = self.infer_ctx.resolve(&hi.ty);
                        if resolved.is_int() {
                            hi.ty.clone()
                        } else {
                            Type::Bool
                        }
                    }
                    UnaryOp::Neg => {
                        let _ = self.infer_ctx.constrain(
                            &hi.ty,
                            super::unify::TypeConstraint::Numeric,
                            *span,
                            "negation requires numeric type",
                        );
                        hi.ty.clone()
                    }
                    UnaryOp::BitNot => {
                        let _ = self.infer_ctx.constrain(
                            &hi.ty,
                            super::unify::TypeConstraint::Integer,
                            *span,
                            "bitwise not requires integer type",
                        );
                        hi.ty.clone()
                    }
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::UnaryOp(*op, Box::new(hi)),
                    ty,
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }
}
