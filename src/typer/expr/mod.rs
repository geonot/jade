mod access;
mod concur;
mod construct;
mod control;
mod ident;
mod lambda;
mod misc;
mod op;
mod quaternary;
mod store;
mod typeargs;

use std::path::PathBuf;

use crate::ast::{self};
use crate::hir::{self, CoercionKind};
use crate::types::Type;

use super::Typer;
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
                if let Some(reinterpreted) =
                    self.try_ternary_as_quaternary(cond, then, els, *span, expected)?
                {
                    return Ok(reinterpreted);
                }
                let hc = self.lower_expr(cond)?;
                let ht = self.lower_expr_expected(then, expected)?;
                let he = self.lower_expr_expected(els, expected)?;

                let ty = match (&ht.ty, &he.ty) {
                    (Type::Void, _) => he.ty.clone(),
                    (_, Type::Void) => ht.ty.clone(),
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

            ast::Expr::Quaternary(subject, ok, nothing, err, span) => self.lower_quaternary(
                subject,
                ok.as_deref(),
                nothing.as_deref(),
                err.as_deref(),
                *span,
                expected,
            ),

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
            ast::Expr::Placeholder(span) => {
                if let Some((def_id, ty)) = self.dollar_stack.last().cloned() {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Var(def_id, "$".into()),
                        ty,
                        span: *span,
                    });
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::Void,
                    ty: expected
                        .cloned()
                        .unwrap_or_else(|| self.infer_ctx.fresh_var()),
                    span: *span,
                })
            }

            ast::Expr::IndexPlaceholder(span) => Ok(hir::Expr {
                kind: hir::ExprKind::Void,
                ty: expected
                    .cloned()
                    .unwrap_or_else(|| self.infer_ctx.fresh_var()),
                span: *span,
            }),

            ast::Expr::Ref(inner, span) => {
                let hi = self.lower_expr(inner)?;

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
                    Type::Ptr(inner_ty) => *inner_ty.clone(),
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
                            "{}: cannot dereference type `{}`",
                            span.loc(),
                            other
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
            ast::Expr::StoreInsert(store, values, span) => {
                self.lower_expr_store_insert(store, values, *span)
            }
            ast::Expr::StoreUpdate(store, assignments, filter, span) => {
                self.lower_expr_store_update(store, assignments, filter, *span)
            }
            ast::Expr::StoreFirst(..) => self.lower_expr_store_first(expr, expected),
            ast::Expr::StoreExists(..) => self.lower_expr_store_exists(expr, expected),
            ast::Expr::StoreDistinct(..) => self.lower_expr_store_distinct(expr, expected),
            ast::Expr::Spawn(..) => self.lower_expr_spawn(expr, expected),
            ast::Expr::Send(..) => self.lower_expr_send(expr, expected),
            ast::Expr::Receive(_, span) => Err(format!(
                "{}: 'receive' is not supported outside actor handlers; \
                 use channels directly with 'receive ch'",
                span.loc(),
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
                    kind: hir::ExprKind::AsFormat(Box::new(hinner), *fmt),
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
            ast::Expr::NamedArg(_, inner, _) => self.lower_expr_expected(inner, expected),
            ast::Expr::Spread(inner, _span) => self.lower_expr(inner),
            ast::Expr::Grad(..) => self.lower_expr_grad(expr, expected),
            ast::Expr::Einsum(..) => self.lower_expr_einsum(expr, expected),
            ast::Expr::Builder(..) => self.lower_expr_builder(expr, expected),
            ast::Expr::OfCall(..) => self.lower_expr_of_call(expr, expected),
        }
    }

    pub(crate) fn maybe_coerce_to(&mut self, mut expr: hir::Expr, target: &Type) -> hir::Expr {
        // Resolve inference variables for the coercion *decision* only. A value
        // derived from an integer literal (e.g. `n is 7`) may still carry an
        // unbound integer TypeVar whose `is_int()`/`is_float()` queries return
        // false, which would otherwise silently skip a required numeric
        // coercion and emit a type-mismatched call in codegen.
        //
        // Critically, we must NOT mutate `expr.ty` on the no-coercion path:
        // the node's original TypeVar may still need to be unified/solved by
        // later inference (HOFs, lambdas, generic calls). We only concretize
        // `expr.ty` when actually wrapping the node in a `Coerce`, where the
        // source type is genuinely fixed and MIR lowering needs it concrete.
        let et = self.infer_ctx.resolve(&expr.ty);
        let tt = self.infer_ctx.resolve(target);
        if et == tt {
            return expr;
        }
        if let Some(coercion) = Self::needs_int_coercion(&et, &tt) {
            if matches!(coercion, CoercionKind::IntTrunc { .. }) {
                self.warnings.push(format!(
                    "implicit truncation from {} to {} may lose data (line {})",
                    et, tt, expr.span.line
                ));
            }
            expr.ty = et;
            return Self::make_coerce(expr, coercion, tt);
        }
        if et.is_int() && tt.is_float() {
            expr.ty = et;
            return Self::make_coerce(expr, CoercionKind::IntToFloat { signed: true }, tt);
        }
        if et.is_float() && tt.is_int() {
            self.warnings.push(format!(
                "implicit float-to-int conversion may lose precision (line {})",
                expr.span.line
            ));
            let signed = tt.is_signed();
            expr.ty = et;
            return Self::make_coerce(expr, CoercionKind::FloatToInt { signed }, tt);
        }
        if et.is_float() && tt.is_float() && et.bits() != tt.bits() {
            let coercion = if et.bits() < tt.bits() {
                CoercionKind::FloatWiden
            } else {
                CoercionKind::FloatNarrow
            };
            expr.ty = et;
            return Self::make_coerce(expr, coercion, tt);
        }
        if et == Type::Bool && tt.is_int() {
            expr.ty = et;
            return Self::make_coerce(expr, CoercionKind::BoolToInt, tt);
        }

        if let (Type::Array(arr_elem, len), Type::Vec(vec_elem)) = (&et, &tt)
            && **arr_elem == **vec_elem {
                let elem_ty = (**arr_elem).clone();
                let len = *len as u64;
                expr.ty = et.clone();
                return Self::make_coerce(expr, CoercionKind::ArrayToVec { elem_ty, len }, tt);
            }
        expr
    }
}
