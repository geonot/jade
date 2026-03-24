use crate::ast;
use crate::hir::{self, CoercionKind};
use crate::types::Type;

use super::Typer;

impl Typer {
    // ── AST-level type inference (expr_ty_ast, infer_ret_ast) REMOVED ──
    // The dual type system has been eliminated. All type inference now goes
    // through the unification-based system (InferCtx + lower_expr_expected).
    // Monomorphization uses fresh TypeVars and resolves them via unification
    // during lowering.

    pub(crate) fn infer_coroutine_yield_type(&self, body: &[hir::Stmt]) -> Type {
        for stmt in body {
            if let Some(ty) = self.find_yield_type_stmt(stmt) {
                return ty;
            }
        }
        if let Some(hir::Stmt::Ret(Some(e), ty, _)) = body.last() {
            let _ = e;
            return ty.clone();
        }
        Type::I64
    }

    fn find_yield_type_stmt(&self, stmt: &hir::Stmt) -> Option<Type> {
        match stmt {
            hir::Stmt::Expr(e) => self.find_yield_type_expr(e),
            hir::Stmt::If(i) => {
                for s in &i.then {
                    if let Some(ty) = self.find_yield_type_stmt(s) {
                        return Some(ty);
                    }
                }
                for (_, blk) in &i.elifs {
                    for s in blk {
                        if let Some(ty) = self.find_yield_type_stmt(s) {
                            return Some(ty);
                        }
                    }
                }
                if let Some(els) = &i.els {
                    for s in els {
                        if let Some(ty) = self.find_yield_type_stmt(s) {
                            return Some(ty);
                        }
                    }
                }
                None
            }
            hir::Stmt::While(w) => {
                for s in &w.body {
                    if let Some(ty) = self.find_yield_type_stmt(s) {
                        return Some(ty);
                    }
                }
                None
            }
            hir::Stmt::For(f) => {
                for s in &f.body {
                    if let Some(ty) = self.find_yield_type_stmt(s) {
                        return Some(ty);
                    }
                }
                None
            }
            hir::Stmt::Loop(l) => {
                for s in &l.body {
                    if let Some(ty) = self.find_yield_type_stmt(s) {
                        return Some(ty);
                    }
                }
                None
            }
            hir::Stmt::Ret(Some(e), _, _) => Some(e.ty.clone()),
            _ => None,
        }
    }

    fn find_yield_type_expr(&self, e: &hir::Expr) -> Option<Type> {
        if let hir::ExprKind::Yield(inner) = &e.kind {
            return Some(inner.ty.clone());
        }
        None
    }

    pub(crate) fn infer_dyn_method_ret(&self, trait_name: &str, method: &str) -> Type {
        for (type_name, impls) in &self.trait_impls {
            if impls.contains(&trait_name.to_string()) {
                let fn_name = format!("{type_name}_{method}");
                if let Some((_, _, ret)) = self.fns.get(&fn_name) {
                    return ret.clone();
                }
            }
        }
        Type::I64
    }

    pub(crate) fn infer_field_ty(&mut self, _f: &ast::Field) -> Type {
        self.infer_ctx.fresh_var()
    }

    pub(crate) fn builtin_ret_ty(name: &str) -> Option<Type> {
        match name {
            "__ln" | "__log2" | "__log10" | "__exp" | "__exp2" | "__powf" | "__copysign"
            | "__fma" | "__time_monotonic" => Some(Type::F64),
            "__fmt_float" | "__fmt_hex" | "__fmt_oct" | "__fmt_bin" | "__string_from_raw"
            | "__string_from_ptr" => Some(Type::String),
            "__get_args" => Some(Type::Vec(Box::new(Type::String))),
            "__file_exists" => Some(Type::Bool),
            "__sleep_ms" => Some(Type::Void),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn builtin_param_tys(name: &str) -> Option<Vec<Type>> {
        match name {
            "__ln" | "__log2" | "__log10" | "__exp" | "__exp2" => Some(vec![Type::F64]),
            "__powf" | "__copysign" => Some(vec![Type::F64, Type::F64]),
            "__fma" => Some(vec![Type::F64, Type::F64, Type::F64]),
            "__fmt_float" => Some(vec![Type::F64, Type::I64]),
            "__fmt_hex" | "__fmt_oct" | "__fmt_bin" | "__sleep_ms" => Some(vec![Type::I64]),
            "__string_from_ptr" => Some(vec![Type::Ptr(Box::new(Type::I8))]),
            "__string_from_raw" => Some(vec![Type::Ptr(Box::new(Type::I8)), Type::I64, Type::I64]),
            "__file_exists" => Some(vec![Type::String]),
            _ => None,
        }
    }

    pub(crate) fn needs_int_coercion(from: &Type, to: &Type) -> Option<CoercionKind> {
        if !from.is_int() || !to.is_int() {
            return None;
        }
        let fb = from.bits();
        let tb = to.bits();
        if fb == tb {
            return None;
        }
        if fb < tb {
            Some(CoercionKind::IntWiden {
                from_bits: fb,
                to_bits: tb,
                signed: from.is_signed(),
            })
        } else {
            Some(CoercionKind::IntTrunc {
                from_bits: fb,
                to_bits: tb,
            })
        }
    }

    pub(crate) fn coerce_binop_operands(
        &self,
        lhs: hir::Expr,
        rhs: hir::Expr,
    ) -> (hir::Expr, hir::Expr) {
        let lt = lhs.ty.clone();
        let rt = rhs.ty.clone();
        if lt.is_int() && rt.is_float() {
            let span = lhs.span;
            return (
                hir::Expr {
                    kind: hir::ExprKind::Coerce(
                        Box::new(lhs),
                        CoercionKind::IntToFloat {
                            signed: lt.is_signed(),
                        },
                    ),
                    ty: rt,
                    span,
                },
                rhs,
            );
        }
        if lt.is_float() && rt.is_int() {
            let span = rhs.span;
            return (
                lhs,
                hir::Expr {
                    kind: hir::ExprKind::Coerce(
                        Box::new(rhs),
                        CoercionKind::IntToFloat {
                            signed: rt.is_signed(),
                        },
                    ),
                    ty: lt,
                    span,
                },
            );
        }
        if lt.is_float() && rt.is_float() && lt.bits() != rt.bits() {
            if lt.bits() < rt.bits() {
                let span = lhs.span;
                return (
                    hir::Expr {
                        kind: hir::ExprKind::Coerce(Box::new(lhs), CoercionKind::FloatWiden),
                        ty: rt,
                        span,
                    },
                    rhs,
                );
            } else {
                let span = rhs.span;
                return (
                    lhs,
                    hir::Expr {
                        kind: hir::ExprKind::Coerce(Box::new(rhs), CoercionKind::FloatWiden),
                        ty: lt,
                        span,
                    },
                );
            }
        }
        if !lt.is_int() || !rt.is_int() || lt.bits() == rt.bits() {
            return (lhs, rhs);
        }
        if lt.bits() > rt.bits() {
            let coercion = CoercionKind::IntWiden {
                from_bits: rt.bits(),
                to_bits: lt.bits(),
                signed: rt.is_signed(),
            };
            let span = rhs.span;
            (
                lhs,
                hir::Expr {
                    kind: hir::ExprKind::Coerce(Box::new(rhs), coercion),
                    ty: lt,
                    span,
                },
            )
        } else {
            let coercion = CoercionKind::IntWiden {
                from_bits: lt.bits(),
                to_bits: rt.bits(),
                signed: lt.is_signed(),
            };
            let span = lhs.span;
            (
                hir::Expr {
                    kind: hir::ExprKind::Coerce(Box::new(lhs), coercion),
                    ty: rt,
                    span,
                },
                rhs,
            )
        }
    }
}
