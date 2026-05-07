//! Per-expression typing rules.

mod ident;
mod typeargs;
mod op;
mod access;
mod control;
mod store;
mod concur;
mod construct;
mod lambda;
mod misc;

use crate::intern::Symbol;
use std::path::PathBuf;

use crate::ast::{self, BinOp, Span, UnaryOp};
use crate::hir::{self, CoercionKind, DefId, Ownership};
use crate::types::Type;

use super::{Typer, VarInfo};
pub(super) use super::{DeferredField, unify};

impl Typer {
    pub(crate) fn lower_expr(&mut self, expr: &ast::Expr) -> Result<hir::Expr, String> {
        self.lower_expr_expected(expr, None)
    }

    pub(crate) fn lower_expr_expected(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        if let ast::Expr::Lambda(params, ret, body, span) = expr {
            return self.lower_lambda_with_expected(params, ret, body, *span, expected);
        }

        if let ast::Expr::Array(elems, span) = expr {
            let expected_elem = match expected {
                Some(Type::Array(et, _)) => Some(et.as_ref()),
                _ => None,
            };
            let helems: Vec<hir::Expr> = elems
                .iter()
                .map(|e| self.lower_expr_expected(e, expected_elem))
                .collect::<Result<_, _>>()?;
            let et = helems
                .first()
                .map(|e| e.ty.clone())
                .or_else(|| expected_elem.cloned())
                .unwrap_or_else(|| self.infer_ctx.fresh_var());
            for elem in helems.iter().skip(1) {
                let _ = self
                    .infer_ctx
                    .unify_at(&et, &elem.ty, *span, "array element");
            }
            let len = helems.len();
            return Ok(hir::Expr {
                kind: hir::ExprKind::Array(helems),
                ty: Type::Array(Box::new(et), len),
                span: *span,
            });
        }

        match expr {
            ast::Expr::Int(n, span) => {
                let ty = match expected {
                    Some(t) if t.is_int() => t.clone(),
                    Some(t) => {
                        let fresh = self.infer_ctx.fresh_integer_var();
                        let _ = self.infer_ctx.unify(&fresh, t);
                        fresh
                    }
                    None => self.infer_ctx.fresh_integer_var(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Int(*n),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Float(n, span) => {
                let ty = match expected {
                    Some(t) if t.is_float() => t.clone(),
                    Some(t) => {
                        let fresh = self.infer_ctx.fresh_float_var();
                        let _ = self.infer_ctx.unify(&fresh, t);
                        fresh
                    }
                    None => self.infer_ctx.fresh_float_var(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Float(*n),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Str(s, span) => Ok(hir::Expr {
                kind: hir::ExprKind::Str(s.clone()),
                ty: Type::String,
                span: *span,
            }),

            ast::Expr::Bool(v, span) => Ok(hir::Expr {
                kind: hir::ExprKind::Bool(*v),
                ty: Type::Bool,
                span: *span,
            }),

            ast::Expr::None(span) => {
                let ty = expected
                    .cloned()
                    .unwrap_or_else(|| self.infer_ctx.fresh_var());
                Ok(hir::Expr {
                    kind: hir::ExprKind::None,
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Void(span) => Ok(hir::Expr {
                kind: hir::ExprKind::Void,
                ty: Type::Void,
                span: *span,
            }),

            ast::Expr::Ident(..) => self.lower_expr_ident(expr, expected),
            ast::Expr::QualifiedIdent(..) => self.lower_expr_qualified_ident(expr, expected),
            ast::Expr::BinOp(..) => self.lower_expr_bin_op(expr, expected),
            ast::Expr::UnaryOp(..) => self.lower_expr_unary_op(expr, expected),
            ast::Expr::Call(..) => self.lower_expr_call(expr, expected),
            ast::Expr::Method(..) => self.lower_expr_method(expr, expected),
            ast::Expr::Field(..) => self.lower_expr_field(expr, expected),
            ast::Expr::Index(..) => self.lower_expr_index(expr, expected),
            ast::Expr::Ternary(cond, then, els, span) => {
                let hc = self.lower_expr(cond)?;
                let ht = self.lower_expr_expected(then, expected)?;
                let he = self.lower_expr_expected(els, expected)?;
                // For partial ternaries (if-only or else-only), skip unification
                // when one branch is Void — the result type comes from the non-Void branch.
                let ty = match (&ht.ty, &he.ty) {
                    (Type::Void, _) => {
                        he.ty.clone()
                    }
                    (_, Type::Void) => {
                        ht.ty.clone()
                    }
                    _ => {
                        let _ = self
                            .infer_ctx
                            .unify_at(&ht.ty, &he.ty, *span, "ternary branches");
                        ht.ty.clone()
                    }
                };
                let he = self.maybe_coerce_to(he, &ty);
                Ok(hir::Expr {
                    kind: hir::ExprKind::Ternary(Box::new(hc), Box::new(ht), Box::new(he)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::As(inner, target_ty, span) => {
                let hi = self.lower_expr(inner)?;
                let ty = target_ty.clone();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Cast(Box::new(hi), ty.clone()),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Array(..) => unreachable!("handled above"),

            ast::Expr::Tuple(elems, span) => {
                let helems: Vec<hir::Expr> = elems
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                let tys: Vec<Type> = helems.iter().map(|e| e.ty.clone()).collect();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Tuple(helems),
                    ty: Type::Tuple(tys),
                    span: *span,
                })
            }

            ast::Expr::Struct(..) => self.lower_expr_struct(expr, expected),
            ast::Expr::IfExpr(..) => self.lower_expr_if_expr(expr, expected),
            ast::Expr::Pipe(..) => self.lower_expr_pipe(expr, expected),
            ast::Expr::Block(..) => self.lower_expr_block(expr, expected),
            ast::Expr::Lambda(..) => self.lower_expr_lambda(expr, expected),
            ast::Expr::Placeholder(span) => Ok(hir::Expr {
                kind: hir::ExprKind::Void,
                ty: expected
                    .cloned()
                    .unwrap_or_else(|| self.infer_ctx.fresh_var()),
                span: *span,
            }),

            ast::Expr::IndexPlaceholder(span) => Ok(hir::Expr {
                kind: hir::ExprKind::Void,
                ty: expected
                    .cloned()
                    .unwrap_or_else(|| self.infer_ctx.fresh_var()),
                span: *span,
            }),

            ast::Expr::Ref(inner, span) => {
                let hi = self.lower_expr(inner)?;
                // % always produces a raw i8 pointer (e.g. String → C buffer ptr)
                let ty = Type::Ptr(Box::new(Type::I8));
                Ok(hir::Expr {
                    kind: hir::ExprKind::Ref(Box::new(hi)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Deref(inner, span) => {
                let hi = self.lower_expr(inner)?;
                let resolved = self.infer_ctx.shallow_resolve(&hi.ty);
                let ty = match &resolved {
                    Type::Ptr(inner_ty) | Type::Rc(inner_ty) => *inner_ty.clone(),
                    Type::TypeVar(_) => {
                        let inner_var = self.infer_ctx.fresh_var();
                        let ptr_ty = Type::Ptr(Box::new(inner_var.clone()));
                        let _ = self
                            .infer_ctx
                            .unify_at(&resolved, &ptr_ty, *span, "dereference");
                        inner_var
                    }
                    other => {
                        return Err(format!(
                            "line {}:{}: cannot dereference type `{}`",
                            span.line, span.col, other
                        ));
                    }
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Deref(Box::new(hi)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::ListComp(..) => self.lower_expr_list_comp(expr, expected),
            ast::Expr::Syscall(args, span) => {
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Syscall(hargs),
                    ty: Type::I64,
                    span: *span,
                })
            }

            ast::Expr::Embed(path, span) => {
                let base = self
                    .source_dir
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("."));
                let file_path = base.join(path);
                let contents = std::fs::read_to_string(&file_path)
                    .map_err(|e| format!("embed '{}': {}", file_path.display(), e))?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Str(contents),
                    ty: Type::String,
                    span: *span,
                })
            }
            ast::Expr::Query(..) => self.lower_expr_query(expr, expected),
            ast::Expr::StoreQuery(..) => self.lower_expr_store_query(expr, expected),
            ast::Expr::StoreCount(..) => self.lower_expr_store_count(expr, expected),
            ast::Expr::StoreAll(..) => self.lower_expr_store_all(expr, expected),
            ast::Expr::StoreGet(..) => self.lower_expr_store_get(expr, expected),
            ast::Expr::StoreFirst(..) => self.lower_expr_store_first(expr, expected),
            ast::Expr::StoreExists(..) => self.lower_expr_store_exists(expr, expected),
            ast::Expr::StoreDistinct(..) => self.lower_expr_store_distinct(expr, expected),
            ast::Expr::Spawn(..) => self.lower_expr_spawn(expr, expected),
            ast::Expr::Send(..) => self.lower_expr_send(expr, expected),
            ast::Expr::Receive(_, span) => Err(format!(
                "line {}:{}: 'receive' is not supported outside actor handlers; \
                 use channels directly with 'receive ch'",
                span.line, span.col,
            )),

            ast::Expr::Yield(..) => self.lower_expr_yield(expr, expected),
            ast::Expr::DispatchBlock(..) => self.lower_expr_dispatch_block(expr, expected),
            ast::Expr::ChannelCreate(..) => self.lower_expr_channel_create(expr, expected),
            ast::Expr::ChannelSend(..) => self.lower_expr_channel_send(expr, expected),
            ast::Expr::ChannelRecv(..) => self.lower_expr_channel_recv(expr, expected),
            ast::Expr::Select(..) => self.lower_expr_select(expr, expected),
            ast::Expr::Unreachable(span) => Ok(hir::Expr {
                kind: hir::ExprKind::Unreachable,
                ty: Type::Void,
                span: *span,
            }),

            ast::Expr::AsFormat(inner, fmt, span) => {
                let hinner = self.lower_expr(inner)?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::AsFormat(Box::new(hinner), fmt.clone()),
                    ty: Type::String,
                    span: *span,
                })
            }

            ast::Expr::StrictCast(inner, target_ty, span) => {
                let hinner = self.lower_expr(inner)?;
                let resolved = self.resolve_ty(target_ty.clone());
                Ok(hir::Expr {
                    kind: hir::ExprKind::StrictCast(Box::new(hinner), resolved.clone()),
                    ty: resolved,
                    span: *span,
                })
            }

            ast::Expr::Slice(obj, start, end, span) => {
                let hobj = self.lower_expr(obj)?;
                let hstart = self.lower_expr_expected(start, Some(&Type::I64))?;
                let hend = self.lower_expr_expected(end, Some(&Type::I64))?;
                let result_ty = hobj.ty.clone();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Slice(Box::new(hobj), Box::new(hstart), Box::new(hend)),
                    ty: result_ty,
                    span: *span,
                })
            }
            ast::Expr::NamedArg(_, inner, _) => {
                // NamedArg should be resolved by lower_call before reaching here;
                // if it somehow reaches lower_expr, just lower the inner expression
                self.lower_expr_expected(inner, expected)
            }
            ast::Expr::Spread(inner, _span) => {
                // Spread lowered to the inner expression — actual spreading
                // is handled by lower_call
                self.lower_expr(inner)
            }
            ast::Expr::NDArray(..) => self.lower_expr_n_d_array(expr, expected),
            ast::Expr::SIMDLit(..) => self.lower_expr_s_i_m_d_lit(expr, expected),
            ast::Expr::Grad(..) => self.lower_expr_grad(expr, expected),
            ast::Expr::Einsum(..) => self.lower_expr_einsum(expr, expected),
            ast::Expr::Builder(..) => self.lower_expr_builder(expr, expected),
            ast::Expr::Deque(..) => self.lower_expr_deque(expr, expected),
            ast::Expr::OfCall(..) => self.lower_expr_of_call(expr, expected),
        }
    }

    /// Collect a mapping from type parameter names to concrete types
    /// by walking a declared (possibly-generic) type alongside a concrete type.
    pub(crate) fn maybe_coerce_to(&mut self, expr: hir::Expr, target: &Type) -> hir::Expr {
        if &expr.ty == target {
            return expr;
        }
        if let Some(ref coercion) = Self::needs_int_coercion(&expr.ty, target) {
            if matches!(coercion, CoercionKind::IntTrunc { .. }) {
                self.warnings.push(format!(
                    "implicit truncation from {} to {} may lose data (line {})",
                    expr.ty, target, expr.span.line
                ));
            }
            return Self::make_coerce(expr, coercion.clone(), target.clone());
        }
        if expr.ty.is_int() && target.is_float() {
            return Self::make_coerce(
                expr,
                CoercionKind::IntToFloat { signed: true },
                target.clone(),
            );
        }
        if expr.ty.is_float() && target.is_int() {
            self.warnings.push(format!(
                "implicit float-to-int conversion may lose precision (line {})",
                expr.span.line
            ));
            return Self::make_coerce(
                expr,
                CoercionKind::FloatToInt {
                    signed: target.is_signed(),
                },
                target.clone(),
            );
        }
        if expr.ty.is_float() && target.is_float() && expr.ty.bits() != target.bits() {
            let coercion = if expr.ty.bits() < target.bits() {
                CoercionKind::FloatWiden
            } else {
                CoercionKind::FloatNarrow
            };
            return Self::make_coerce(expr, coercion, target.clone());
        }
        if expr.ty == Type::Bool && target.is_int() {
            return Self::make_coerce(expr, CoercionKind::BoolToInt, target.clone());
        }
        if let Type::DynTrait(trait_name) = &target {
            if let Type::Struct(type_name, _) = &expr.ty {
                let tn = type_name.clone();
                let trn = trait_name.clone();
                let span = expr.span;
                return hir::Expr {
                    kind: hir::ExprKind::DynCoerce(Box::new(expr), tn, trn),
                    ty: target.clone(),
                    span,
                };
            }
        }
        expr
    }
}
