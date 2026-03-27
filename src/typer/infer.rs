use crate::ast;
use crate::hir::{self, CoercionKind};
use crate::types::Type;

use super::Typer;

impl Typer {
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
            hir::Stmt::If(i) => i
                .then
                .iter()
                .chain(i.elifs.iter().flat_map(|(_, blk)| blk))
                .chain(i.els.iter().flatten())
                .find_map(|s| self.find_yield_type_stmt(s)),
            hir::Stmt::While(w) => w.body.iter().find_map(|s| self.find_yield_type_stmt(s)),
            hir::Stmt::For(f) => f.body.iter().find_map(|s| self.find_yield_type_stmt(s)),
            hir::Stmt::Loop(l) => l.body.iter().find_map(|s| self.find_yield_type_stmt(s)),
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

    pub(crate) fn infer_field_ty(&mut self, f: &ast::Field) -> Type {
        let var = self.infer_ctx.fresh_var();
        if let Some(ref default) = f.default {
            if let Some(ty) = Self::literal_type(default) {
                let _ = self.infer_ctx.unify(&var, &ty);
            }
        }
        var
    }

    fn literal_type(expr: &crate::ast::Expr) -> Option<Type> {
        match expr {
            crate::ast::Expr::Int(_, _) => Some(Type::I64),
            crate::ast::Expr::Float(_, _) => Some(Type::F64),
            crate::ast::Expr::Str(_, _) => Some(Type::String),
            crate::ast::Expr::Bool(_, _) => Some(Type::Bool),
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
            return (
                Self::make_coerce(
                    lhs,
                    CoercionKind::IntToFloat {
                        signed: lt.is_signed(),
                    },
                    rt,
                ),
                rhs,
            );
        }
        if lt.is_float() && rt.is_int() {
            return (
                lhs,
                Self::make_coerce(
                    rhs,
                    CoercionKind::IntToFloat {
                        signed: rt.is_signed(),
                    },
                    lt,
                ),
            );
        }
        if lt.is_float() && rt.is_float() && lt.bits() != rt.bits() {
            if lt.bits() < rt.bits() {
                return (Self::make_coerce(lhs, CoercionKind::FloatWiden, rt), rhs);
            } else {
                return (lhs, Self::make_coerce(rhs, CoercionKind::FloatWiden, lt));
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
            (lhs, Self::make_coerce(rhs, coercion, lt))
        } else {
            let coercion = CoercionKind::IntWiden {
                from_bits: lt.bits(),
                to_bits: rt.bits(),
                signed: lt.is_signed(),
            };
            (Self::make_coerce(lhs, coercion, rt), rhs)
        }
    }
}
