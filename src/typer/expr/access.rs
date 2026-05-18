//! Extracted typing rules.

#![allow(unused_imports, unused_variables)]

use super::super::unify;
use super::super::{Typer, VarInfo};
use crate::ast::{self, BinOp, Span, UnaryOp};
use crate::hir::{self, CoercionKind, DefId, Ownership};
use crate::intern::Symbol;
use crate::types::Type;
use std::path::PathBuf;

impl Typer {
    pub(in crate::typer) fn lower_expr_call(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Call(callee, args, span) => {
                // Generic struct/variant constructor: `T of TypeArg(args)` —
                // The parser shapes this as `Call(OfCall(Ident, type_expr), args)`.
                // Reroute to lower_struct_or_variant with explicit type bindings
                // so monomorphization does not have to infer from arg types.
                if let ast::Expr::OfCall(inner, type_arg_expr, _) = callee.as_ref() {
                    if let ast::Expr::Ident(ctor_name, _) = inner.as_ref() {
                        let is_struct_ctor = self.generic_types.contains_key(ctor_name)
                            || self.structs.contains_key(ctor_name);
                        let is_variant_ctor = self.variant_tags.contains_key(ctor_name);
                        if is_struct_ctor || is_variant_ctor {
                            if let Some(tys) = self.expr_to_type_args(type_arg_expr) {
                                let inits: Vec<ast::FieldInit> = args
                                    .iter()
                                    .map(|a| ast::FieldInit {
                                        name: None,
                                        value: a.clone(),
                                    })
                                    .collect();
                                let result = self.lower_struct_or_variant_with_typeargs(
                                    &ctor_name.as_str(),
                                    &inits,
                                    *span,
                                    &tys,
                                )?;
                                if let Some(exp) = expected {
                                    self.unify_call_result(exp, &result.ty, *span, "call result");
                                }
                                return Ok(result);
                            }
                        }
                    }
                }
                // Positional struct constructor: `Box(7)` for a known
                // (possibly generic) struct, no `is`-named fields. The parser
                // routes Uppercase-Ident + `(` directly to Expr::Struct, but
                // when the caller wrote it after a postfix path (e.g. parens
                // following another postfix) it can also reach here.
                if let ast::Expr::Ident(ctor_name, _) = callee.as_ref() {
                    let is_struct = self.generic_types.contains_key(ctor_name)
                        || self.structs.contains_key(ctor_name);
                    let is_variant = self.variant_tags.contains_key(ctor_name);
                    let is_value = self.find_var(&ctor_name.as_str()).is_some()
                        || self.fns.contains_key(ctor_name)
                        || self.generic_fns.contains_key(ctor_name)
                        || self.inferable_fns.contains_key(ctor_name);
                    if (is_struct || is_variant) && !is_value {
                        let inits: Vec<ast::FieldInit> = args
                            .iter()
                            .map(|a| ast::FieldInit {
                                name: None,
                                value: a.clone(),
                            })
                            .collect();
                        let result =
                            self.lower_struct_or_variant(&ctor_name.as_str(), &inits, *span)?;
                        if let Some(exp) = expected {
                            self.unify_call_result(exp, &result.ty, *span, "call result");
                        }
                        return Ok(result);
                    }
                }
                // Partial application: if any arg is $, wrap in a lambda
                let has_placeholder = args.iter().any(|a| matches!(a, ast::Expr::Placeholder(_)));
                if has_placeholder {
                    let param = ast::Param {
                        name: "__ph".into(),
                        ty: None,
                        default: None,
                        literal: None,
                        access_mod: None,
                        span: *span,
                    };
                    let new_args: Vec<ast::Expr> = args
                        .iter()
                        .map(|a| {
                            if matches!(a, ast::Expr::Placeholder(_)) {
                                ast::Expr::Ident("__ph".into(), a.span())
                            } else {
                                a.clone()
                            }
                        })
                        .collect();
                    let call = ast::Expr::Call(callee.clone(), new_args, *span);
                    let lambda =
                        ast::Expr::Lambda(vec![param], None, vec![ast::Stmt::Expr(call)], *span);
                    return self.lower_expr_expected(&lambda, expected);
                }
                let result = self.lower_call(callee, args, *span)?;
                if let Some(exp) = expected {
                    self.unify_call_result(exp, &result.ty, *span, "call result");
                }
                Ok(result)
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_method(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Method(obj, method, args, span) => {
                // Module-qualified dispatch: module.fn(args) → module_fn(args)
                // Only if the name is NOT a local variable (variables shadow modules)
                if let ast::Expr::Ident(ref name, _) = **obj {
                    if self.modules.contains(name) && self.find_var(&name.as_str()).is_none() {
                        // Rewrite module.fn(args) to a prefixed function call
                        let qualified_name = Symbol::intern(&format!("{}_{}", name, method));
                        // If the qualified name is a known extern called from within
                        // the module, dispatch via the extern path
                        if !self.fns.contains_key(&qualified_name)
                            && !self.inferable_fns.contains_key(&qualified_name)
                            && !self.generic_fns.contains_key(&qualified_name)
                        {
                            if let Some((id, ptys, ret)) = self.externs.get(method).cloned() {
                                let mut hargs = Vec::new();
                                for (i, arg) in args.iter().enumerate() {
                                    let expected_ty = ptys.get(i);
                                    hargs.push(self.lower_expr_expected(arg, expected_ty)?);
                                }
                                for (i, harg) in hargs.iter().enumerate() {
                                    if let Some(pty) = ptys.get(i) {
                                        let _ = self.infer_ctx.unify_at(
                                            pty,
                                            &harg.ty,
                                            *span,
                                            "extern arg",
                                        );
                                    }
                                }
                                return Ok(hir::Expr {
                                    kind: hir::ExprKind::Call(id, method.clone(), hargs),
                                    ty: ret,
                                    span: *span,
                                });
                            }
                        }
                        let callee = ast::Expr::Ident(qualified_name, *span);
                        let result = self.lower_call(&callee, args, *span)?;
                        if let Some(exp) = expected {
                            self.unify_call_result(exp, &result.ty, *span, "call result");
                        }
                        return Ok(result);
                    }
                    // extern.fn(args) → call extern function directly by bare name
                    if name == "extern" {
                        if let Some((id, ptys, ret)) = self.externs.get(method).cloned() {
                            let mut hargs = Vec::new();
                            for (i, arg) in args.iter().enumerate() {
                                let expected_ty = ptys.get(i);
                                hargs.push(self.lower_expr_expected(arg, expected_ty)?);
                            }
                            for (i, harg) in hargs.iter().enumerate() {
                                if let Some(pty) = ptys.get(i) {
                                    let _ =
                                        self.infer_ctx.unify_at(pty, &harg.ty, *span, "extern arg");
                                }
                            }
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::Call(id, method.clone(), hargs),
                                ty: ret,
                                span: *span,
                            });
                        }
                        // Fall through — maybe it's a regular function named "extern_method"
                        let callee = ast::Expr::Ident(method.clone(), *span);
                        let result = self.lower_call(&callee, args, *span)?;
                        if let Some(exp) = expected {
                            self.unify_call_result(exp, &result.ty, *span, "call result");
                        }
                        return Ok(result);
                    }
                }
                let result = self.lower_method_call(obj, &method.as_str(), args, *span)?;
                if let Some(exp) = expected {
                    self.unify_call_result(exp, &result.ty, *span, "method call result");
                }
                Ok(result)
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_field(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Field(obj, field, span) => {
                // Module-qualified constant access: module.CONST → module_CONST
                if let ast::Expr::Ident(ref name, _) = **obj {
                    if self.modules.contains(name) && self.find_var(&name.as_str()).is_none() {
                        let qualified_name = Symbol::intern(&format!("{}_{}", name, field));
                        let callee = ast::Expr::Ident(qualified_name, *span);
                        return self.lower_expr_expected(&callee, expected);
                    }
                }
                let hobj = self.lower_expr(obj)?;
                // P4 §5.2 use-after-partial-move check. If `obj` is a bare
                // var whose field `field` was previously moved out via
                // `take` in this scope (and not reassigned), reading or
                // borrowing the field is a compile error. Without this
                // check the user would silently observe a zero/null value
                // from the tombstone slot instead of an error.
                if let hir::ExprKind::Var(parent_id, parent_name) = &hobj.kind {
                    if self.suppress_moved_field_check == 0 {
                        if let Some(moved) = self.moved_fields.get(parent_id) {
                            if moved.contains(field) {
                                return Err(format!(
                                    "{}: field `{}` of `{}` was moved out by an earlier `take`; \
                                     reassign `{}.{}` before reading it",
                                    span.loc(),
                                    field,
                                    parent_name,
                                    parent_name,
                                    field,
                                ));
                            }
                        }
                    }
                }
                let resolved_ty = self.infer_ctx.shallow_resolve(&hobj.ty);
                // Actor message send-with-no-args sugar: `t.show` where `t :
                // ActorRef(Tally)` is desugared to a method call so the actor
                // dispatch path runs (otherwise we attempt a struct field load
                // on the actor handle, which has no such field).
                if let Type::ActorRef(actor_name) = &resolved_ty {
                    if let Some((_, _, handlers)) = self.actors.get(actor_name) {
                        if handlers.iter().any(|(n, _, _)| n == field) {
                            let call_expr =
                                ast::Expr::Method(obj.clone(), field.clone(), Vec::new(), *span);
                            return self.lower_expr_expected(&call_expr, expected);
                        }
                    }
                }
                // R3.4.d.1 auto-deref: peer through one layer of
                // Rc/RcCell/Arc (and Arc<Mutex<_>>) so field access on a
                // shared-ownership wrapper resolves to the wrapped struct's
                // field. Codegen mirrors this by GEPing into the Rc payload
                // before reading the field — no clone of the wrapped value.
                let peeled_ty = match &resolved_ty {
                    Type::Rc(inner) => self.infer_ctx.shallow_resolve(inner),
                    _ => resolved_ty.clone(),
                };
                let struct_name = match &peeled_ty {
                    Type::Struct(name, _) => Some(name.clone()),
                    // P5 §6: `Row<store>` is a write-through handle whose
                    // inner record layout is the store's auto-generated
                    // `__store_{store}` struct. Field reads on the row
                    // see the underlying struct's fields transparently.
                    Type::Row(store) => Some(Symbol::intern(&format!("__store_{store}"))),
                    Type::Ptr(inner) => match inner.as_ref() {
                        Type::Struct(name, _) => Some(name.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                let (ty, idx) = if let Some(ref name) = struct_name {
                    if let Some(fields) = self.structs.get(name) {
                        if let Some((i, (_, fty))) =
                            fields.iter().enumerate().find(|(_, (n, _))| n == field)
                        {
                            (self.infer_ctx.shallow_resolve(fty), i)
                        } else {
                            // Improve diagnostic for store-query result types:
                            // surface the user-visible store name and suggest
                            // `count <store> where …` for `.length`/`.len`.
                            let raw = name.as_str();
                            let display: String =
                                if let Some(stripped) = raw.strip_prefix("__store_") {
                                    format!("{} (query result)", stripped)
                                } else {
                                    raw.to_string()
                                };
                            if raw.starts_with("__store_") && (field == "length" || field == "len")
                            {
                                let store = raw.trim_start_matches("__store_");
                                return Err(format!(
                                    "{}: type '{}' has no field '{}' — \
                                 for the number of matching records use \
                                 `count {} where …`",
                                    span.loc(),
                                    display,
                                    field,
                                    store
                                ));
                            }
                            return Err(format!(
                                "{}: type '{}' has no field '{}'",
                                span.loc(),
                                display,
                                field
                            ));
                        }
                    } else {
                        (Type::I64, 0)
                    }
                } else if matches!(peeled_ty, Type::String) && field == "length" {
                    (Type::I64, 0)
                } else if matches!(&peeled_ty, Type::Vec(_)) && field == "length" {
                    (Type::I64, 0)
                } else if matches!(&peeled_ty, Type::Map(_, _)) && field == "length" {
                    (Type::I64, 0)
                } else if let Type::Tuple(ref tys) = peeled_ty {
                    if let Ok(idx) = field.as_str().parse::<usize>() {
                        if idx < tys.len() {
                            (tys[idx].clone(), idx)
                        } else {
                            return Err(format!(
                                "{}: tuple index {} out of range (tuple has {} elements)",
                                span.loc(),
                                idx,
                                tys.len()
                            ));
                        }
                    } else {
                        (self.infer_ctx.fresh_var(), 0)
                    }
                } else if matches!(resolved_ty, Type::TypeVar(_)) {
                    let var_id = if let Type::TypeVar(v) = resolved_ty {
                        self.infer_ctx.find(v)
                    } else {
                        0
                    };

                    let fty_placeholder = self.infer_ctx.fresh_var();
                    self.field_constraints
                        .entry(var_id)
                        .or_default()
                        .push((field.clone(), fty_placeholder.clone()));

                    let all_required_fields: Vec<(Symbol, Type)> = self
                        .field_constraints
                        .get(&var_id)
                        .cloned()
                        .unwrap_or_default();

                    let candidates: Vec<(Symbol, Vec<(Symbol, Type, usize)>)> = self
                        .structs
                        .iter()
                        .filter_map(|(sname, fields)| {
                            let mut matched = Vec::new();
                            for (req_name, _) in &all_required_fields {
                                if let Some((idx, (_, fty))) = fields
                                    .iter()
                                    .enumerate()
                                    .find(|(_, (fname, _))| fname == req_name)
                                {
                                    matched.push((*req_name, fty.clone(), idx));
                                } else {
                                    return None;
                                }
                            }
                            Some((*sname, matched))
                        })
                        .collect();

                    if candidates.len() == 1 {
                        let (sname, matched_fields) = &candidates[0];
                        let struct_ty = Type::Struct(*sname, vec![]);
                        let _ = self.infer_ctx.unify_at(
                            &resolved_ty,
                            &struct_ty,
                            *span,
                            "field access implies struct type",
                        );
                        for (req_name, req_ty) in &all_required_fields {
                            if let Some((_, actual_ty, _)) =
                                matched_fields.iter().find(|(n, _, _)| n == req_name)
                            {
                                let actual_resolved = self.infer_ctx.shallow_resolve(actual_ty);
                                let _ = self.infer_ctx.unify_at(
                                    req_ty,
                                    &actual_resolved,
                                    *span,
                                    "struct field type",
                                );
                            }
                        }
                        let (fty, idx) = matched_fields
                            .iter()
                            .find(|(n, _, _)| n == field)
                            .map(|(_, t, i)| (self.infer_ctx.shallow_resolve(t), *i))
                            .unwrap_or_else(|| (fty_placeholder, 0));
                        (fty, idx)
                    } else {
                        self.deferred_fields.push(super::DeferredField {
                            receiver_ty: resolved_ty.clone(),
                            field_name: field.clone(),
                            field_ty: fty_placeholder.clone(),
                            span: *span,
                        });
                        (fty_placeholder, 0)
                    }
                } else {
                    (self.infer_ctx.fresh_var(), 0)
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Field(Box::new(hobj), field.clone(), idx),
                    ty,
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::typer) fn lower_expr_index(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let _ = expected;
        match expr {
            ast::Expr::Index(arr, idx, span) => {
                let harr = self.lower_expr(arr)?;
                let hidx = self.lower_expr(idx)?;
                // R3.4.d.1 auto-deref: peer through one layer of
                // Rc/RcCell/Arc (and Arc<Mutex<_>>) so indexing a
                // shared-ownership wrapper resolves to the wrapped
                // container's element type.
                let peeled_ty = match &harr.ty {
                    Type::Rc(inner) => (**inner).clone(),
                    other => other.clone(),
                };
                let elem_ty = match &peeled_ty {
                    Type::Array(et, _) => *et.clone(),
                    Type::Vec(et) => *et.clone(),
                    Type::Ptr(et) => *et.clone(),
                    Type::Map(_, vt) => *vt.clone(),
                    Type::Tuple(tys) => tys
                        .first()
                        .cloned()
                        .unwrap_or_else(|| self.infer_ctx.fresh_var()),
                    _ => self.infer_ctx.fresh_var(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Index(Box::new(harr), Box::new(hidx)),
                    ty: elem_ty,
                    span: *span,
                })
            }
            _ => unreachable!(),
        }
    }
}
