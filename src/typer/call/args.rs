#![allow(unused_imports, unused_variables)]

use std::collections::HashMap;

use super::super::unify;
use super::super::{DeferredField, Typer, VarInfo};
use crate::ast::{self, Expr, Span};
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;

pub(super) fn resolve_named_args<'a>(
    param_names: &[String],
    args: &'a [ast::Expr],
    span: Span,
) -> Result<Vec<&'a ast::Expr>, String> {
    let has_named = args.iter().any(|a| matches!(a, Expr::NamedArg(..)));
    if !has_named {
        return Ok(args.iter().collect());
    }

    let mut result: Vec<Option<&ast::Expr>> = vec![None; param_names.len()];
    let mut pos_idx = 0;

    for arg in args {
        match arg {
            Expr::NamedArg(name, inner, _) => {
                if let Some(idx) = param_names.iter().position(|p| p == &name.as_str()) {
                    if result[idx].is_some() {
                        return Err(format!(
                            "duplicate named argument '{name}' at line {}",
                            span.line
                        ));
                    }
                    result[idx] = Some(inner.as_ref());
                } else {
                    return Err(format!(
                        "unknown named argument '{name}' at line {}",
                        span.line
                    ));
                }
            }
            _ => {
                while pos_idx < result.len() && result[pos_idx].is_some() {
                    pos_idx += 1;
                }
                if pos_idx >= param_names.len() {
                    return Err(format!(
                        "too many positional arguments at line {}",
                        span.line
                    ));
                }
                result[pos_idx] = Some(arg);
                pos_idx += 1;
            }
        }
    }

    let mut ordered = Vec::new();
    for (i, slot) in result.into_iter().enumerate() {
        match slot {
            Some(e) => ordered.push(e),
            None => {
                return Err(format!(
                    "missing argument '{}' at line {}",
                    param_names[i], span.line
                ));
            }
        }
    }
    Ok(ordered)
}

impl Typer {
    pub(in crate::typer) fn expand_spread_args(
        &mut self,
        callee: &ast::Expr,
        args: &[ast::Expr],
        _span: Span,
    ) -> Option<Vec<ast::Expr>> {
        let has_spread = args.iter().any(|a| matches!(a, ast::Expr::Spread(..)));
        let expected_param_count = if let ast::Expr::Ident(name, _) = callee {
            self.fns.get(name).map(|(_, ptys, _)| ptys.len())
        } else {
            None
        };

        if has_spread {
            let mut expanded = Vec::new();
            for arg in args {
                if let ast::Expr::Spread(inner, sp) = arg {
                    let inner_lowered = self.lower_expr(inner).ok()?;
                    let resolved_ty = self.infer_ctx.resolve(&inner_lowered.ty);
                    match &resolved_ty {
                        Type::Array(_, len) => {
                            for i in 0..*len {
                                expanded.push(ast::Expr::Index(
                                    Box::new((**inner).clone()),
                                    Box::new(ast::Expr::Int(i as i64, *sp)),
                                    *sp,
                                ));
                            }
                        }
                        Type::Tuple(tys) => {
                            for i in 0..tys.len() {
                                expanded.push(ast::Expr::Index(
                                    Box::new((**inner).clone()),
                                    Box::new(ast::Expr::Int(i as i64, *sp)),
                                    *sp,
                                ));
                            }
                        }
                        _ => {
                            expanded.push((**inner).clone());
                        }
                    }
                } else {
                    expanded.push(arg.clone());
                }
            }
            return Some(expanded);
        }

        if args.len() == 1 && !has_spread {
            if let Some(expected) = expected_param_count {
                if expected > 1 {
                    let inner_lowered = self.lower_expr(&args[0]).ok()?;
                    let resolved_ty = self.infer_ctx.resolve(&inner_lowered.ty);
                    if let Type::Array(_, len) = &resolved_ty {
                        if *len == expected {
                            let sp = args[0].span();
                            let mut expanded = Vec::new();
                            for i in 0..*len {
                                expanded.push(ast::Expr::Index(
                                    Box::new(args[0].clone()),
                                    Box::new(ast::Expr::Int(i as i64, sp)),
                                    sp,
                                ));
                            }
                            return Some(expanded);
                        }
                    }
                }
            }
        }

        None
    }
}
