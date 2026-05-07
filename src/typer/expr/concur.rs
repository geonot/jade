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
    pub(in crate::typer) fn lower_expr_spawn(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Spawn(name, span) => {                if !self.actors.contains_key(name) {
                return Err(format!("spawn: unknown actor '{name}'"));
            }
            Ok(hir::Expr {
                kind: hir::ExprKind::Spawn(name.clone()),
                ty: Type::ActorRef(name.clone()),
                span: *span,
            })
        },
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_send(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Send(target, handler, args, span) => {                let htarget = self.lower_expr(target)?;
            if !matches!(&htarget.ty, Type::ActorRef(_)) {
                return Err(format!(
                    "send: target must be an ActorRef, got {}",
                    htarget.ty
                ));
            }
            let arg_placeholders = std::iter::repeat_n("_", args.len())
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "line {}:{}: actor send syntax 'send target, @{}(...)' is not supported; use method-call syntax instead: target.{}({})",
                span.line, span.col, handler, handler, arg_placeholders
            ))
        },
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_yield(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Yield(inner, span) => {                let hi = self.lower_expr(inner)?;
            let ty = hi.ty.clone();
            Ok(hir::Expr {
                kind: hir::ExprKind::Yield(Box::new(hi)),
                ty,
                span: *span,
            })
        },
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_dispatch_block(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::DispatchBlock(name, body, span) => {                let hbody = self.lower_block_no_scope(body, &Type::Void)?;
            let yield_ty = self.infer_coroutine_yield_type(&hbody);
            let coro_ty = Type::Coroutine(Box::new(yield_ty));
            if name != "__anon" {
                let id = self.fresh_id();
                // Use Borrowed ownership so emit_scope_drops skips
                // this variable — the Bind target owns the allocation.
                self.define_var(
                    &name.as_str(),
                    VarInfo {
                        def_id: id,
                        ty: coro_ty.clone(),
                        ownership: crate::hir::Ownership::Borrowed,
                        scheme: None,
                    },
                );
            }
            Ok(hir::Expr {
                kind: hir::ExprKind::CoroutineCreate(name.clone(), hbody),
                ty: coro_ty,
                span: *span,
            })
        },
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_channel_create(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::ChannelCreate(elem_ty, cap, span) => {                let hcap = self.lower_expr(cap)?;
            let resolved_elem_ty = match elem_ty {
                Some(ty) => ty.clone(),
                None => self.infer_ctx.fresh_var(),
            };
            Ok(hir::Expr {
                kind: hir::ExprKind::ChannelCreate(resolved_elem_ty.clone(), Box::new(hcap)),
                ty: Type::Channel(Box::new(resolved_elem_ty)),
                span: *span,
            })
        },
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_channel_send(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::ChannelSend(ch, val, span) => {                let hch = self.lower_expr(ch)?;
            let resolved_ch_ty = self.infer_ctx.shallow_resolve(&hch.ty);
            let elem_ty = match &resolved_ch_ty {
                Type::Channel(t) => (**t).clone(),
                Type::TypeVar(_) => {
                    let elem_var = self.infer_ctx.fresh_var();
                    let chan_ty = Type::Channel(Box::new(elem_var.clone()));
                    let _ = self.infer_ctx.unify_at(
                        &resolved_ch_ty,
                        &chan_ty,
                        *span,
                        "channel send infers channel type",
                    );
                    elem_var
                }
                _ => return Err(format!("send: target must be a Channel, got {}", hch.ty)),
            };
            let hval = self.lower_expr(val)?;
            let _ = self
                .infer_ctx
                .unify_at(&elem_ty, &hval.ty, *span, "channel send");
            let hval = self.maybe_coerce_to(hval, &elem_ty);
            Ok(hir::Expr {
                kind: hir::ExprKind::ChannelSend(Box::new(hch), Box::new(hval)),
                ty: Type::Void,
                span: *span,
            })
        },
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_channel_recv(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::ChannelRecv(ch, span) => {                let hch = self.lower_expr(ch)?;
            let resolved_ch_ty = self.infer_ctx.shallow_resolve(&hch.ty);
            let elem_ty = match &resolved_ch_ty {
                Type::Channel(t) => (**t).clone(),
                Type::TypeVar(_) => {
                    let elem_var = self.infer_ctx.fresh_var();
                    let chan_ty = Type::Channel(Box::new(elem_var.clone()));
                    let _ = self.infer_ctx.unify_at(
                        &resolved_ch_ty,
                        &chan_ty,
                        *span,
                        "channel recv infers channel type",
                    );
                    elem_var
                }
                _ => return Err(format!("receive: target must be a Channel, got {}", hch.ty)),
            };
            Ok(hir::Expr {
                kind: hir::ExprKind::ChannelRecv(Box::new(hch)),
                ty: elem_ty,
                span: *span,
            })
        },
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_select(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Select(arms, default_body, span) => {                let mut harms = Vec::new();
            for arm in arms {
                let hch = self.lower_expr(&arm.chan)?;
                let resolved_sel_ch = self.infer_ctx.shallow_resolve(&hch.ty);
                let elem_ty = match &resolved_sel_ch {
                    Type::Channel(t) => (**t).clone(),
                    Type::TypeVar(_) => {
                        let elem_var = self.infer_ctx.fresh_var();
                        let chan_ty = Type::Channel(Box::new(elem_var.clone()));
                        let _ = self.infer_ctx.unify_at(
                            &resolved_sel_ch,
                            &chan_ty,
                            arm.span,
                            "select infers channel type",
                        );
                        elem_var
                    }
                    _ => {
                        return Err(format!(
                            "select: channel must be a Channel type, got {}",
                            hch.ty
                        ));
                    }
                };
                let hval = if let Some(ref v) = arm.value {
                    let hv = self.lower_expr(v)?;
                    if arm.is_send {
                        let _ =
                            self.infer_ctx
                                .unify_at(&elem_ty, &hv.ty, arm.span, "select send");
                    }
                    Some(hv)
                } else {
                    None
                };
                let bind_id = arm.binding.as_ref().map(|_| self.fresh_id());
                if let (Some(name), Some(id)) = (&arm.binding, bind_id) {
                    self.define_var(
                        &name.as_str(),
                        VarInfo {
                            def_id: id,
                            ty: elem_ty.clone(),
                            ownership: hir::Ownership::Owned,
                            scheme: None,
                        },
                    );
                }
                let hbody = self.lower_block_no_scope(&arm.body, &Type::Void)?;
                harms.push(hir::SelectArm {
                    is_send: arm.is_send,
                    chan: hch,
                    value: hval,
                    binding: arm.binding.clone(),
                    bind_id,
                    elem_ty,
                    body: hbody,
                    span: arm.span,
                });
            }
            let hdefault = if let Some(body) = default_body {
                Some(self.lower_block_no_scope(body, &Type::Void)?)
            } else {
                None
            };
            Ok(hir::Expr {
                kind: hir::ExprKind::Select(harms, hdefault),
                ty: Type::Void,
                span: *span,
            })
        },
            _ => unreachable!(),
        }
    }

}
