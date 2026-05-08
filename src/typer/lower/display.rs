//! Extracted lowering steps.

#![allow(unused_imports, unused_variables)]

use std::collections::{HashMap, HashSet};

use super::super::unify;
use super::super::{DeferredField, DeferredMethod, Typer, VarInfo};
use crate::ast::{self, Span};
use crate::hir::{self, CoercionKind, DefId, ExprKind, Ownership};
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn auto_derive_display(&mut self, prog: &mut hir::Program) {
        // Collect struct names that need Display
        let mut needs_display: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
        for f in &prog.fns {
            Self::collect_display_usage(&f.body, &mut needs_display);
        }
        for ti in &prog.trait_impls {
            for m in &ti.methods {
                Self::collect_display_usage(&m.body, &mut needs_display);
            }
        }
        // Remove structs that already have a display method
        needs_display.retain(|name| !self.fns.contains_key(&format!("{name}_display")));

        // Generate display methods for structs
        for type_name in &needs_display {
            if let Some(fields) = self.structs.get(type_name).cloned() {
                let method_name: Symbol = format!("{type_name}_display").into();
                let self_id = self.fresh_id();
                let self_ty = Type::Struct(type_name.clone(), vec![]);
                let span = crate::ast::Span::dummy();

                // Build a single nested concat expression:
                // "TypeName(" + field1_label + to_string(field1) + ... + ")"
                let mk_str = |s: String| hir::Expr {
                    kind: hir::ExprKind::Str(s),
                    ty: Type::String,
                    span,
                };
                let concat = |a: hir::Expr, b: hir::Expr| hir::Expr {
                    kind: hir::ExprKind::BinOp(Box::new(a), crate::ast::BinOp::Add, Box::new(b)),
                    ty: Type::String,
                    span,
                };

                let mut result = mk_str(format!("{type_name}("));

                for (i, (fname, fty)) in fields.iter().enumerate() {
                    let label = if i == 0 {
                        format!("{fname}: ")
                    } else {
                        format!(", {fname}: ")
                    };
                    result = concat(result, mk_str(label));

                    let field_val = hir::Expr {
                        kind: hir::ExprKind::Field(
                            Box::new(hir::Expr {
                                kind: hir::ExprKind::Var(self_id, "__self".into()),
                                ty: self_ty.clone(),
                                span,
                            }),
                            fname.clone(),
                            i,
                        ),
                        ty: fty.clone(),
                        span,
                    };
                    let to_string = hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::ToString, vec![field_val]),
                        ty: Type::String,
                        span,
                    };
                    result = concat(result, to_string);
                }

                result = concat(result, mk_str(")".into()));
                let body = vec![hir::Stmt::Expr(result)];

                let hir_fn = hir::Fn {
                    def_id: self.fresh_id(),
                    name: method_name,
                    params: vec![hir::Param {
                        def_id: self_id,
                        name: "__self".into(),
                        ty: self_ty.clone(),
                        ownership: hir::Ownership::Owned,
                        default: None,
                        span,
                    }],
                    ret: Type::String,
                    error_types: Vec::new(),
                    body,
                    span,
                    generic_origin: None,
                    is_generator: false,
                    attrs: crate::ast::FnAttrs::default(),
                };
                self.fns
                    .insert(method_name, (hir_fn.def_id, vec![self_ty], Type::String));
                prog.fns.push(hir_fn);
            }
        }
    }

    pub(in crate::typer) fn collect_display_usage(
        block: &[hir::Stmt],
        needs: &mut std::collections::HashSet<Symbol>,
    ) {
        for stmt in block {
            Self::collect_display_usage_stmt(stmt, needs);
        }
    }

    pub(in crate::typer) fn collect_display_usage_stmt(
        stmt: &hir::Stmt,
        needs: &mut std::collections::HashSet<Symbol>,
    ) {
        match stmt {
            hir::Stmt::Bind(b) => Self::collect_display_usage_expr(&b.value, needs),
            hir::Stmt::TupleBind(_, e, _) => Self::collect_display_usage_expr(e, needs),
            hir::Stmt::Assign(l, r, _) => {
                Self::collect_display_usage_expr(l, needs);
                Self::collect_display_usage_expr(r, needs);
            }
            hir::Stmt::Expr(e) => Self::collect_display_usage_expr(e, needs),
            hir::Stmt::If(i) => {
                Self::collect_display_usage_expr(&i.cond, needs);
                Self::collect_display_usage(&i.then, needs);
                for (c, b) in &i.elifs {
                    Self::collect_display_usage_expr(c, needs);
                    Self::collect_display_usage(b, needs);
                }
                if let Some(b) = &i.els {
                    Self::collect_display_usage(b, needs);
                }
            }
            hir::Stmt::While(w) => {
                Self::collect_display_usage_expr(&w.cond, needs);
                Self::collect_display_usage(&w.body, needs);
            }
            hir::Stmt::For(f) => {
                Self::collect_display_usage_expr(&f.iter, needs);
                Self::collect_display_usage(&f.body, needs);
            }
            hir::Stmt::Loop(l) => Self::collect_display_usage(&l.body, needs),
            hir::Stmt::Match(m) => {
                Self::collect_display_usage_expr(&m.subject, needs);
                for a in &m.arms {
                    Self::collect_display_usage(&a.body, needs);
                }
            }
            hir::Stmt::Ret(Some(e), _, _) => Self::collect_display_usage_expr(e, needs),
            hir::Stmt::Break(Some(e), _) => Self::collect_display_usage_expr(e, needs),
            hir::Stmt::ErrReturn(e, _, _) => Self::collect_display_usage_expr(e, needs),
            _ => {}
        }
    }

    pub(in crate::typer) fn collect_display_usage_expr(
        expr: &hir::Expr,
        needs: &mut std::collections::HashSet<Symbol>,
    ) {
        match &expr.kind {
            hir::ExprKind::Builtin(hir::BuiltinFn::Log, args) => {
                for a in args {
                    if let Type::Struct(name, _) = &a.ty {
                        needs.insert(name.clone());
                    }
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::Builtin(hir::BuiltinFn::ToString, args) => {
                for a in args {
                    if let Type::Struct(name, _) = &a.ty {
                        needs.insert(name.clone());
                    }
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::BinOp(l, _, r) => {
                Self::collect_display_usage_expr(l, needs);
                Self::collect_display_usage_expr(r, needs);
            }
            hir::ExprKind::Call(_, _, args)
            | hir::ExprKind::Builtin(_, args)
            | hir::ExprKind::VecNew(args)
            | hir::ExprKind::Array(args)
            | hir::ExprKind::Tuple(args) => {
                for a in args {
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                Self::collect_display_usage_expr(callee, needs);
                for a in args {
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::Method(recv, _, _, args)
            | hir::ExprKind::StringMethod(recv, _, args)
            | hir::ExprKind::DeferredMethod(recv, _, args)
            | hir::ExprKind::VecMethod(recv, _, args)
            | hir::ExprKind::MapMethod(recv, _, args)
            | hir::ExprKind::SetMethod(recv, _, args)
            | hir::ExprKind::PQMethod(recv, _, args)
            | hir::ExprKind::DynDispatch(recv, _, _, args) => {
                Self::collect_display_usage_expr(recv, needs);
                for a in args {
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::Pipe(l, _, _, args) => {
                Self::collect_display_usage_expr(l, needs);
                for a in args {
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::Lambda(_, body) | hir::ExprKind::Block(body) => {
                Self::collect_display_usage(body, needs);
            }
            hir::ExprKind::IfExpr(i) => {
                Self::collect_display_usage_expr(&i.cond, needs);
                Self::collect_display_usage(&i.then, needs);
                for (c, b) in &i.elifs {
                    Self::collect_display_usage_expr(c, needs);
                    Self::collect_display_usage(b, needs);
                }
                if let Some(b) = &i.els {
                    Self::collect_display_usage(b, needs);
                }
            }
            _ => {}
        }
    }
}
