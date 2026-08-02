use crate::ast;
use crate::hir::{self, Ownership};
use crate::types::Type;

use super::super::{Typer, VarInfo};

impl Typer {
    pub(in crate::typer) fn try_ternary_as_quaternary(
        &mut self,
        cond: &ast::Expr,
        then: &ast::Expr,
        els: &ast::Expr,
        span: ast::Span,
        expected: Option<&Type>,
    ) -> Result<Option<hir::Expr>, String> {
        let snapshot = self.lower_expr(cond)?;
        let ty = self.infer_ctx.shallow_resolve(&snapshot.ty);
        let (is_option, is_result) = match &ty {
            Type::Enum(n) => {
                let s = n.as_str();
                (
                    s.starts_with("Option_") || s == "Option",
                    s.starts_with("Result_") || s == "Result",
                )
            }
            Type::Struct(n, _) => (n.as_str() == "Option", n.as_str() == "Result"),
            _ => (false, false),
        };
        if !is_option && !is_result {
            return Ok(None);
        }

        let ok_arm = match then {
            ast::Expr::Void(_) => None,
            other => Some(other),
        };
        if is_option {
            let nothing_arm = match els {
                ast::Expr::Void(_) => None,
                other => Some(other),
            };
            let r = self.lower_quaternary(cond, ok_arm, nothing_arm, None, span, expected)?;
            Ok(Some(r))
        } else {
            let err_arm = match els {
                ast::Expr::Void(_) => None,
                other => Some(other),
            };
            let r = self.lower_quaternary(cond, ok_arm, None, err_arm, span, expected)?;
            Ok(Some(r))
        }
    }

    pub(in crate::typer) fn reconcile_ternary_arms(
        &mut self,
        ht: hir::Expr,
        he: hir::Expr,
    ) -> Result<(hir::Expr, hir::Expr), String> {
        let tt = self.infer_ctx.shallow_resolve(&ht.ty);
        let et = self.infer_ctx.shallow_resolve(&he.ty);
        let t_fallible = self.is_fallible_ty(&tt);
        let e_fallible = self.is_fallible_ty(&et);
        if t_fallible == e_fallible {
            return Ok((ht, he));
        }
        if !self.enclosing_fn_is_fallible() {
            return Ok((ht, he));
        }
        let ht = if t_fallible {
            self.implicit_propagate(ht.clone())?.unwrap_or(ht)
        } else {
            ht
        };
        let he = if e_fallible {
            self.implicit_propagate(he.clone())?.unwrap_or(he)
        } else {
            he
        };
        Ok((ht, he))
    }

    fn is_fallible_ty(&self, ty: &Type) -> bool {
        match ty {
            Type::Enum(n) => {
                let s = n.as_str();
                s.starts_with("Result_")
                    || s == "Result"
                    || s.starts_with("Option_")
                    || s == "Option"
            }
            Type::Struct(n, _) => n.as_str() == "Result" || n.as_str() == "Option",
            _ => false,
        }
    }

    pub(in crate::typer) fn implicit_propagate(
        &mut self,
        value: hir::Expr,
    ) -> Result<Option<hir::Expr>, String> {
        let ty = self.infer_ctx.shallow_resolve(&value.ty);
        let is_fallible = match &ty {
            Type::Enum(n) => {
                let s = n.as_str();
                s.starts_with("Result_")
                    || s == "Result"
                    || s.starts_with("Option_")
                    || s == "Option"
            }
            Type::Struct(n, _) => n.as_str() == "Result" || n.as_str() == "Option",
            _ => false,
        };
        if !is_fallible {
            return Ok(None);
        }
        if matches!(
            &value.kind,
            hir::ExprKind::VariantCtor(..) | hir::ExprKind::VariantRef(..)
        ) {
            return Ok(None);
        }
        if !self.enclosing_fn_is_fallible() {
            if self.current_fn_is_main {
                return Ok(None);
            }
            return Err(format!(
                "{}: error propagation is only valid inside a function whose result type \
                 is a `Result`/`Option`: the value here is fallible (`{}`) and bare use \
                 propagates its error, but the enclosing function is not fallible; declare \
                 its error union with `! E` (e.g. `returns T ! E`), or handle the value \
                 with `? $ !! ...`",
                value.span.loc(),
                ty
            ));
        }
        let span = value.span;
        let r = self.build_quaternary(value, None, None, None, span, None)?;
        Ok(Some(r))
    }

    pub(in crate::typer) fn enclosing_fn_is_fallible(&mut self) -> bool {
        let Some(ret) = self.current_fn_ret_ty.clone() else {
            return false;
        };
        let resolved = self.infer_ctx.shallow_resolve(&ret);
        match &resolved {
            Type::Enum(n) => {
                let s = n.as_str();
                s.starts_with("Result_")
                    || s == "Result"
                    || s.starts_with("Option_")
                    || s == "Option"
            }
            Type::Struct(n, _) => n.as_str() == "Result" || n.as_str() == "Option",
            _ => false,
        }
    }

    pub(in crate::typer) fn lower_quaternary(
        &mut self,
        subject: &ast::Expr,
        ok_arm: Option<&ast::Expr>,
        nothing_arm: Option<&ast::Expr>,
        err_arm: Option<&ast::Expr>,
        span: ast::Span,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let hsubj = self.lower_expr(subject)?;
        self.build_quaternary(hsubj, ok_arm, nothing_arm, err_arm, span, expected)
    }

    fn build_quaternary(
        &mut self,
        hsubj: hir::Expr,
        ok_arm: Option<&ast::Expr>,
        nothing_arm: Option<&ast::Expr>,
        err_arm: Option<&ast::Expr>,
        span: ast::Span,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let subj_ty = self.infer_ctx.shallow_resolve(&hsubj.ty);

        let enum_name = match &subj_ty {
            Type::Enum(n) => Some(*n),
            Type::Struct(n, _) if n.as_str() == "Result" || n.as_str() == "Option" => Some(*n),
            _ => None,
        };
        let Some(enum_name) = enum_name else {
            return Err(format!(
                "{}: the subject of `?`/`!!` must be a `Result` or `Option`, found `{}`",
                span.loc(),
                subj_ty
            ));
        };

        let ename = enum_name.as_str();
        let is_option = ename.starts_with("Option_") || ename == "Option";
        let is_result = ename.starts_with("Result_") || ename == "Result";
        if !is_option && !is_result {
            return Err(format!(
                "{}: the subject of `?`/`!!` must be a `Result` or `Option`, found `{}`",
                span.loc(),
                subj_ty
            ));
        }

        let ok_variant = if is_option { "Some" } else { "Ok" };
        let bad_variant = if is_option { "Nothing" } else { "Err" };
        let ok_tag = self
            .enums
            .get(&enum_name)
            .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == ok_variant))
            .unwrap_or(0) as u32;
        let bad_tag = self
            .enums
            .get(&enum_name)
            .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == bad_variant))
            .unwrap_or(1) as u32;
        let ok_inner = self.ok_inner_ty_pub(enum_name);
        let err_inner = self
            .enums
            .get(&enum_name)
            .and_then(|vs| vs.get(bad_tag as usize))
            .and_then(|(_, fs)| fs.first().cloned())
            .unwrap_or(Type::Void);

        let subj_id = self.fresh_id();
        let subj_bind = hir::Stmt::Bind(hir::Bind {
            def_id: subj_id,
            name: "__q_subj".into(),
            value: hsubj,
            ty: subj_ty.clone(),
            ownership: Ownership::Owned,
            atomic: false,
            access_mod: None,
            span,
        });
        let subj_ref = hir::Expr {
            kind: hir::ExprKind::Var(subj_id, "__q_subj".into()),
            ty: subj_ty.clone(),
            span,
        };

        let is_ok = hir::Expr {
            kind: hir::ExprKind::EnumIs(Box::new(subj_ref.clone()), ok_tag),
            ty: Type::Bool,
            span,
        };

        let ok_unwrap = hir::Expr {
            kind: hir::ExprKind::EnumUnwrap(Box::new(subj_ref.clone()), enum_name, ok_tag),
            ty: ok_inner.clone(),
            span,
        };
        let dollar_id = self.fresh_id();
        let ok_branch = self.lower_arm_with_dollar(
            ok_arm,
            dollar_id,
            ok_unwrap,
            ok_inner.clone(),
            expected,
            span,
        )?;

        let bad_branch = if is_option {
            self.lower_nothing_arm(nothing_arm, &ok_branch.ty, span)?
        } else {
            let err_unwrap = hir::Expr {
                kind: hir::ExprKind::EnumUnwrap(Box::new(subj_ref.clone()), enum_name, bad_tag),
                ty: err_inner.clone(),
                span,
            };
            self.lower_err_arm(err_arm, err_unwrap, err_inner.clone(), &ok_branch.ty, span)?
        };

        let result_ty = match (&ok_branch.ty, &bad_branch.ty) {
            (Type::Void, _) => bad_branch.ty.clone(),
            (_, Type::Void) => ok_branch.ty.clone(),
            _ => {
                let _ =
                    self.infer_ctx
                        .unify_at(&ok_branch.ty, &bad_branch.ty, span, "quaternary arms");
                ok_branch.ty.clone()
            }
        };
        let ok_branch = self.maybe_coerce_to(ok_branch, &result_ty);
        let bad_branch = self.maybe_coerce_to(bad_branch, &result_ty);

        let ternary = hir::Expr {
            kind: hir::ExprKind::Ternary(
                Box::new(is_ok),
                Box::new(ok_branch),
                Box::new(bad_branch),
            ),
            ty: result_ty.clone(),
            span,
        };

        Ok(hir::Expr {
            kind: hir::ExprKind::Block(vec![subj_bind, hir::Stmt::Expr(ternary)]),
            ty: result_ty,
            span,
        })
    }

    fn lower_arm_with_dollar(
        &mut self,
        arm: Option<&ast::Expr>,
        dollar_id: hir::DefId,
        unwrap_val: hir::Expr,
        unwrap_ty: Type,
        expected: Option<&Type>,
        span: ast::Span,
    ) -> Result<hir::Expr, String> {
        let dollar_bind = hir::Stmt::Bind(hir::Bind {
            def_id: dollar_id,
            name: "$".into(),
            value: unwrap_val,
            ty: unwrap_ty.clone(),
            ownership: Ownership::Owned,
            atomic: false,
            access_mod: None,
            span,
        });

        self.dollar_stack.push((dollar_id, unwrap_ty.clone()));
        self.push_scope();
        self.define_var(
            "$",
            VarInfo {
                def_id: dollar_id,
                ty: unwrap_ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );
        let body = match arm {
            Some(e) => self.lower_expr_expected(e, expected),
            None => Ok(hir::Expr {
                kind: hir::ExprKind::Var(dollar_id, "$".into()),
                ty: unwrap_ty.clone(),
                span,
            }),
        };
        self.pop_scope();
        self.dollar_stack.pop();
        let body = body?;

        let ty = body.ty.clone();
        Ok(hir::Expr {
            kind: hir::ExprKind::Block(vec![dollar_bind, hir::Stmt::Expr(body)]),
            ty,
            span,
        })
    }

    fn lower_err_arm(
        &mut self,
        arm: Option<&ast::Expr>,
        err_unwrap: hir::Expr,
        err_ty: Type,
        ok_ty: &Type,
        span: ast::Span,
    ) -> Result<hir::Expr, String> {
        let err_id = self.fresh_id();
        let err_bind = hir::Stmt::Bind(hir::Bind {
            def_id: err_id,
            name: "err".into(),
            value: err_unwrap,
            ty: err_ty.clone(),
            ownership: Ownership::Owned,
            atomic: false,
            access_mod: None,
            span,
        });

        match arm {
            None => {
                let err_ref = hir::Expr {
                    kind: hir::ExprKind::Var(err_id, "err".into()),
                    ty: err_ty.clone(),
                    span,
                };
                let prop = self.propagate_err_value(err_ref, span)?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Block(vec![
                        err_bind,
                        prop,
                        hir::Stmt::Expr(hir::Expr {
                            kind: hir::ExprKind::Unreachable,
                            ty: ok_ty.clone(),
                            span,
                        }),
                    ]),
                    ty: ok_ty.clone(),
                    span,
                })
            }
            Some(e) => {
                self.push_scope();
                self.define_var(
                    "err",
                    VarInfo {
                        def_id: err_id,
                        ty: err_ty.clone(),
                        ownership: Ownership::Owned,
                        scheme: None,
                    },
                );
                let is_bare_err = matches!(e, ast::Expr::Ident(n, _) if n.as_str() == "err");
                let body = self.lower_expr(e);
                self.pop_scope();
                let body = body?;

                if is_bare_err {
                    let prop = self.propagate_err_value(body, span)?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Block(vec![
                            err_bind,
                            prop,
                            hir::Stmt::Expr(hir::Expr {
                                kind: hir::ExprKind::Unreachable,
                                ty: ok_ty.clone(),
                                span,
                            }),
                        ]),
                        ty: ok_ty.clone(),
                        span,
                    });
                }

                let ty = body.ty.clone();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Block(vec![err_bind, hir::Stmt::Expr(body)]),
                    ty,
                    span,
                })
            }
        }
    }

    fn lower_nothing_arm(
        &mut self,
        arm: Option<&ast::Expr>,
        ok_ty: &Type,
        span: ast::Span,
    ) -> Result<hir::Expr, String> {
        match arm {
            Some(e) => self.lower_expr_expected(e, Some(ok_ty)),
            None => {
                let none_val = self.propagate_none_value(ok_ty, span)?;
                Ok(none_val)
            }
        }
    }

    fn propagate_none_value(&mut self, ok_ty: &Type, span: ast::Span) -> Result<hir::Expr, String> {
        let ret_ty = self.current_fn_ret_ty.clone().unwrap_or(Type::Void);
        let resolved = self.infer_ctx.shallow_resolve(&ret_ty);
        let opt_enum = match &resolved {
            Type::Enum(n) if n.as_str().starts_with("Option_") || n.as_str() == "Option" => {
                Some(*n)
            }
            _ => None,
        };
        if let Some(opt_enum) = opt_enum {
            let nothing_tag = self
                .enums
                .get(&opt_enum)
                .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == "Nothing"))
                .unwrap_or(1) as u32;
            let nothing = hir::Expr {
                kind: hir::ExprKind::VariantCtor(opt_enum, "Nothing".into(), nothing_tag, vec![]),
                ty: Type::Enum(opt_enum),
                span,
            };
            let prop = hir::Stmt::ErrReturn(nothing, Type::Enum(opt_enum), span);
            return Ok(hir::Expr {
                kind: hir::ExprKind::Block(vec![
                    prop,
                    hir::Stmt::Expr(hir::Expr {
                        kind: hir::ExprKind::Unreachable,
                        ty: ok_ty.clone(),
                        span,
                    }),
                ]),
                ty: ok_ty.clone(),
                span,
            });
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::Unreachable,
            ty: ok_ty.clone(),
            span,
        })
    }
}
