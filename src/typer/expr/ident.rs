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
                            kind: hir::ExprKind::VariantRef(enum_name, *name, tag),
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
                            .unwrap_or((enum_name, tag));
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(en2, *name, tag2),
                            ty: Type::Enum(en2),
                            span: *span,
                        });
                    }
                } else if let Ok(Some(mangled)) =
                    self.try_monomorphize_generic_variant(&name.as_str(), None)
                    && let Some((_, tag)) = self.variant_tags.get(name).cloned()
                {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::VariantRef(mangled, *name, tag),
                        ty: Type::Enum(mangled),
                        span: *span,
                    });
                }
                if let Some(v) = self.find_var(&name.as_str()) {
                    let def_id = v.def_id;
                    let mono_ty = v.ty.clone();
                    let scheme_clone = v.scheme.clone();
                    if self.suppress_moved_field_check == 0
                        && self.suppress_whole_struct_check == 0
                        && let Some(moved) = self.moved_fields.get(&def_id)
                        && !moved.is_empty()
                    {
                        let mut fields: Vec<String> =
                            moved.iter().map(|f| f.as_str().to_string()).collect();
                        fields.sort();
                        return Err(format!(
                            "{}: `{}` cannot be read as a whole: its field{} {} {} moved out \
                             earlier, so the struct is only partly initialised; read the \
                             remaining fields individually, clone at the move site \
                             (`copy {}.{}`), or reassign `{}.{}` before reading `{}`",
                            span.loc(),
                            name,
                            if fields.len() == 1 { "" } else { "s" },
                            fields
                                .iter()
                                .map(|f| format!("`{f}`"))
                                .collect::<Vec<_>>()
                                .join(", "),
                            if fields.len() == 1 { "was" } else { "were" },
                            name,
                            fields[0],
                            name,
                            fields[0],
                            name,
                        ));
                    }
                    if self.suppress_moved_field_check == 0
                        && let Some(reason) = self.moved_vars.get(&def_id)
                    {
                        return Err(match reason {
                            crate::typer::MoveReason::TakeExplicit => format!(
                                "{}: `{}` was moved out by an earlier `take`; \
                                 reassign `{}` before reading it",
                                span.loc(),
                                name,
                                name,
                            ),
                            crate::typer::MoveReason::ConsumingCall(callee) => format!(
                                "{}: use of moved value `{}`: it was moved into the call \
                                 to `{}`, whose parameter takes ownership (the value \
                                 escapes through `{}`); pass a clone instead \
                                 (`{}2 is copy {}` before the call), or reassign `{}` \
                                 before reading it",
                                span.loc(),
                                name,
                                callee,
                                callee,
                                name,
                                name,
                                name,
                            ),
                            crate::typer::MoveReason::ContainerInsert(meth, at) => format!(
                                "{}: use of moved value `{}`: it was moved into a container \
                                 by the `{}` at {} — the container now owns it, so the \
                                 original name is empty; insert a clone instead \
                                 (`copy {}` at the call), read it back out of the \
                                 container, or reassign `{}` before reading it",
                                span.loc(),
                                name,
                                meth,
                                at.loc(),
                                name,
                                name,
                            ),
                            crate::typer::MoveReason::TaskCapture(at) => format!(
                                "{}: `{}` used after being moved into a concurrent task: \
                                 it was captured by the task started at {} — two tasks \
                                 may not share one aggregate; give each task its own \
                                 value and merge results over a channel, let a single \
                                 actor own it and send it messages, or capture a clone \
                                 (`copy {}`)",
                                span.loc(),
                                name,
                                at.loc(),
                                name,
                            ),
                            crate::typer::MoveReason::Sent(at) => format!(
                                "{}: use of moved value `{}`: it was sent on a channel \
                                 at {} — sends transfer ownership to the receiver; to \
                                 keep a local copy, send a clone (`send ch, copy {}`), \
                                 or reassign `{}` before reading it",
                                span.loc(),
                                name,
                                at.loc(),
                                name,
                                name,
                            ),
                            crate::typer::MoveReason::AssignMove(to, at) => format!(
                                "{}: use of moved value `{}`: it moved at {} (`{} is {}`) \
                                 — aggregates move on assignment; to keep both values, \
                                 clone explicitly (`{} is copy {}`), or reassign `{}` \
                                 before reading it",
                                span.loc(),
                                name,
                                at.loc(),
                                to,
                                name,
                                to,
                                name,
                                name,
                            ),
                        });
                    }
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
                        kind: hir::ExprKind::Var(def_id, *name),
                        ty,
                        span: *span,
                    });
                }
                if let Some(const_expr) = self.consts.get(name).cloned() {
                    if self.const_expansion_stack.contains(name) {
                        return Err(format!(
                            "{}: constant `{}` is defined in terms of itself; constants must be acyclic",
                            span.loc(),
                            name,
                        ));
                    }
                    self.const_expansion_stack.push(*name);
                    let r = self.lower_expr(&const_expr);
                    self.const_expansion_stack.pop();
                    return r;
                }
                if let Some((_expr, _span)) = self.globals.get(name).cloned() {
                    let init_expr = self.lower_expr(&_expr)?;
                    let ty = init_expr.ty.clone();
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::GlobalLoad(*name),
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
                        kind: hir::ExprKind::FnRef(id, *name),
                        ty: fn_ty,
                        span: *span,
                    });
                }
                if self.generic_fns.contains_key(name) {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Var(DefId::BUILTIN, *name),
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
                        let field_expr = ast::Expr::Field(Box::new(self_expr), *name, *span);
                        return self.lower_expr(&field_expr);
                    }
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::Var(DefId::BUILTIN, *name),
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
                            kind: hir::ExprKind::VariantRef(*type_name, *variant_name, tag as u32),
                            ty: Type::Enum(*type_name),
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
