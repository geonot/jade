//! Extracted typing rules.

#![allow(unused_imports, unused_variables)]

use crate::intern::Symbol;
use std::path::PathBuf;
use crate::ast::{self, BinOp, Span, UnaryOp};
use crate::hir::{self, CoercionKind, DefId, Ownership};
use crate::types::Type;
use super::super::{Typer, VarInfo};
use super::super::unify;

impl Typer {
    pub(in crate::typer) fn lower_expr_lambda(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Lambda(params, ret, body, span) => {                self.lower_lambda_with_expected(params, ret, body, *span, expected)
        },
            _ => unreachable!(),
        }
    }

}

impl Typer {
    pub(in crate::typer) fn lower_lambda_with_expected(
        &mut self,
        params: &[ast::Param],
        ret: &Option<Type>,
        body: &ast::Block,
        span: Span,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let (expected_ptys, expected_ret) = match expected {
            Some(Type::Fn(ptys, ret)) => (Some(ptys.as_slice()), Some(ret.as_ref())),
            _ => (None, None),
        };

        self.push_scope();
        let mut hparams = Vec::new();
        let mut ptys = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let pid = self.fresh_id();
            let ty = p.ty.clone().unwrap_or_else(|| {
                expected_ptys
                    .and_then(|ep| ep.get(i))
                    .cloned()
                    .unwrap_or_else(|| self.infer_ctx.fresh_var())
            });
            ptys.push(ty.clone());
            let ownership = Self::ownership_for_type(&ty);
            self.define_var(
                &p.name.as_str(),
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
                    scheme: None,
                },
            );
            hparams.push(hir::Param {
                def_id: pid,
                name: p.name.clone(),
                ty,
                ownership,
                default: None,
                span: p.span,
            });
        }

        let ret_ty = ret.clone().unwrap_or_else(|| {
            if let Some(eret) = expected_ret {
                eret.clone()
            } else {
                self.infer_ctx.fresh_var()
            }
        });

        let hbody = self.lower_block_no_scope(body, &ret_ty)?;
        self.pop_scope();

        if let Some(hir::Stmt::Expr(e)) = hbody.last() {
            if e.ty != Type::Void {
                let _ = self.infer_ctx.unify(&ret_ty, &e.ty);
            }
        }

        let final_ret = if ret.is_some() || expected_ret.is_some() {
            ret_ty
        } else {
            match hbody.last() {
                Some(hir::Stmt::Expr(e)) if e.ty != Type::Void => e.ty.clone(),
                _ => {
                    let _ = self.infer_ctx.unify(&ret_ty, &Type::Void);
                    Type::Void
                }
            }
        };

        Ok(hir::Expr {
            kind: hir::ExprKind::Lambda(hparams, hbody),
            ty: Type::Fn(ptys, Box::new(final_ret)),
            span,
        })
    }

}
