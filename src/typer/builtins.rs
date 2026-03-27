use crate::ast::{self, Span};
use crate::hir;
use crate::types::Type;

use super::Typer;

impl Typer {
    /// Try to lower a named call as a builtin function.
    /// Returns `Some(Ok(expr))` if the name matches a builtin,
    /// `Some(Err(..))` if it matches but has an error, or `None` to fall through.
    pub(crate) fn try_lower_builtin_call(
        &mut self,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<Result<hir::Expr, String>> {
        match name {
            "assert" => {
                if args.is_empty() {
                    return Some(Err("assert requires a condition".into()));
                }
                let hcond = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::Assert, vec![hcond]),
                    ty: Type::Void,
                    span,
                }))
            }
            "log" => {
                let hargs = match self.lower_exprs(args) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::Log, hargs),
                    ty: Type::Void,
                    span,
                }))
            }
            "to_string" => {
                let hargs = match self.lower_exprs(args) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::ToString, hargs),
                    ty: Type::String,
                    span,
                }))
            }
            "rc" if args.len() == 1 => {
                let harg = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let inner_ty = harg.ty.clone();
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::RcAlloc, vec![harg]),
                    ty: Type::Rc(Box::new(inner_ty)),
                    span,
                }))
            }
            "rc_retain" => {
                let hargs = match self.lower_exprs(args) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::RcRetain, hargs),
                    ty: Type::Void,
                    span,
                }))
            }
            "rc_release" => {
                let hargs = match self.lower_exprs(args) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::RcRelease, hargs),
                    ty: Type::Void,
                    span,
                }))
            }
            "weak" if args.len() == 1 && !self.fns.contains_key(name) => {
                let harg = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let inner_ty = match &harg.ty {
                    Type::Rc(inner) => inner.as_ref().clone(),
                    _ => return Some(Err(format!("weak() requires an rc value, got {}", harg.ty))),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::WeakDowngrade, vec![harg]),
                    ty: Type::Weak(Box::new(inner_ty)),
                    span,
                }))
            }
            "weak_upgrade" if args.len() == 1 && !self.fns.contains_key(name) => {
                let harg = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let inner_ty = match &harg.ty {
                    Type::Weak(inner) => inner.as_ref().clone(),
                    _ => return Some(Err(format!(
                        "weak_upgrade() requires a weak value, got {}",
                        harg.ty
                    ))),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::WeakUpgrade, vec![harg]),
                    ty: Type::Rc(Box::new(inner_ty)),
                    span,
                }))
            }
            "volatile_load" if args.len() == 1 && !self.fns.contains_key(name) => {
                let harg = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let inner_ty = match &harg.ty {
                    Type::Ptr(inner) => inner.as_ref().clone(),
                    _ => return Some(Err(format!(
                        "volatile_load() requires a pointer, got {}",
                        harg.ty
                    ))),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::VolatileLoad, vec![harg]),
                    ty: inner_ty,
                    span,
                }))
            }
            "volatile_store" if args.len() == 2 && !self.fns.contains_key(name) => {
                let hptr = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let hval = match self.lower_expr(&args[1]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                if !matches!(hptr.ty, Type::Ptr(_)) {
                    return Some(Err(format!(
                        "volatile_store() first arg must be a pointer, got {}",
                        hptr.ty
                    )));
                }
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(
                        hir::BuiltinFn::VolatileStore,
                        vec![hptr, hval],
                    ),
                    ty: Type::Void,
                    span,
                }))
            }
            "wrapping_add" | "wrapping_sub" | "wrapping_mul"
                if args.len() == 2 && !self.fns.contains_key(name) =>
            {
                let lhs = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let rhs = match self.lower_expr(&args[1]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let ty = lhs.ty.clone();
                let builtin = match name {
                    "wrapping_add" => hir::BuiltinFn::WrappingAdd,
                    "wrapping_sub" => hir::BuiltinFn::WrappingSub,
                    "wrapping_mul" => hir::BuiltinFn::WrappingMul,
                    _ => unreachable!(),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(builtin, vec![lhs, rhs]),
                    ty,
                    span,
                }))
            }
            "saturating_add" | "saturating_sub" | "saturating_mul"
                if args.len() == 2 && !self.fns.contains_key(name) =>
            {
                let lhs = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let rhs = match self.lower_expr(&args[1]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let ty = lhs.ty.clone();
                let builtin = match name {
                    "saturating_add" => hir::BuiltinFn::SaturatingAdd,
                    "saturating_sub" => hir::BuiltinFn::SaturatingSub,
                    "saturating_mul" => hir::BuiltinFn::SaturatingMul,
                    _ => unreachable!(),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(builtin, vec![lhs, rhs]),
                    ty,
                    span,
                }))
            }
            "checked_add" | "checked_sub" | "checked_mul"
                if args.len() == 2 && !self.fns.contains_key(name) =>
            {
                let lhs = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let rhs = match self.lower_expr(&args[1]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let ty = lhs.ty.clone();
                let builtin = match name {
                    "checked_add" => hir::BuiltinFn::CheckedAdd,
                    "checked_sub" => hir::BuiltinFn::CheckedSub,
                    "checked_mul" => hir::BuiltinFn::CheckedMul,
                    _ => unreachable!(),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(builtin, vec![lhs, rhs]),
                    ty: Type::Tuple(vec![ty, Type::Bool]),
                    span,
                }))
            }
            "signal_handle" if args.len() == 2 && !self.fns.contains_key(name) => {
                let hsig = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let hhandler = match self.lower_expr(&args[1]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(
                        hir::BuiltinFn::SignalHandle,
                        vec![hsig, hhandler],
                    ),
                    ty: Type::Void,
                    span,
                }))
            }
            "signal_raise" if args.len() == 1 && !self.fns.contains_key(name) => {
                let hsig = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::SignalRaise, vec![hsig]),
                    ty: Type::I32,
                    span,
                }))
            }
            "signal_ignore" if args.len() == 1 && !self.fns.contains_key(name) => {
                let hsig = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::SignalIgnore, vec![hsig]),
                    ty: Type::Void,
                    span,
                }))
            }
            "popcount" | "clz" | "ctz" | "rotate_left" | "rotate_right" | "bswap" => {
                let builtin = match name {
                    "popcount" => hir::BuiltinFn::Popcount,
                    "clz" => hir::BuiltinFn::Clz,
                    "ctz" => hir::BuiltinFn::Ctz,
                    "rotate_left" => hir::BuiltinFn::RotateLeft,
                    "rotate_right" => hir::BuiltinFn::RotateRight,
                    "bswap" => hir::BuiltinFn::Bswap,
                    _ => unreachable!(),
                };
                let hargs = match self.lower_exprs(args) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(builtin, hargs),
                    ty: Type::I64,
                    span,
                }))
            }
            "__string_from_raw" if args.len() == 3 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::StringFromRaw, Type::String, span))
            }
            "__string_from_ptr" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::StringFromPtr, Type::String, span))
            }
            "__get_args" if args.is_empty() && !self.fns.contains_key(name) => {
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::GetArgs, vec![]),
                    ty: Type::Vec(Box::new(Type::String)),
                    span,
                }))
            }
            "__ln" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::Ln, Type::F64, span))
            }
            "__log2" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::Log2, Type::F64, span))
            }
            "__log10" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::Log10, Type::F64, span))
            }
            "__exp" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::Exp, Type::F64, span))
            }
            "__exp2" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::Exp2, Type::F64, span))
            }
            "__powf" if args.len() == 2 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::PowF, Type::F64, span))
            }
            "__copysign" if args.len() == 2 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::Copysign, Type::F64, span))
            }
            "__fma" if args.len() == 3 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::Fma, Type::F64, span))
            }
            "__fmt_float" if args.len() == 2 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::FmtFloat, Type::String, span))
            }
            "__fmt_hex" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::FmtHex, Type::String, span))
            }
            "__fmt_oct" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::FmtOct, Type::String, span))
            }
            "__fmt_bin" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::FmtBin, Type::String, span))
            }
            "__time_monotonic" if args.is_empty() && !self.fns.contains_key(name) => {
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::TimeMonotonic, vec![]),
                    ty: Type::F64,
                    span,
                }))
            }
            "__sleep_ms" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::SleepMs, Type::Void, span))
            }
            "__file_exists" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::FileExists, Type::Bool, span))
            }
            "vec" if !self.fns.contains_key(name) => {
                let hargs = match self.lower_exprs(args) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                let elem_ty = hargs
                    .first()
                    .map(|a| a.ty.clone())
                    .unwrap_or_else(|| self.infer_ctx.fresh_integer_var());
                for a in hargs.iter().skip(1) {
                    let _ = self
                        .infer_ctx
                        .unify_at(&elem_ty, &a.ty, span, "vec element");
                }
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::VecNew(hargs),
                    ty: Type::Vec(Box::new(elem_ty)),
                    span,
                }))
            }
            "map" if !self.fns.contains_key(name) => {
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::MapNew,
                    ty: Type::Map(Box::new(Type::String), Box::new(self.infer_ctx.fresh_integer_var())),
                    span,
                }))
            }
            _ => None,
        }
    }

    /// Lower a simple builtin call: lower all args, produce Builtin expr with given type.
    fn lower_simple_builtin(
        &mut self,
        args: &[ast::Expr],
        builtin: hir::BuiltinFn,
        ty: Type,
        span: Span,
    ) -> Result<hir::Expr, String> {
        let hargs = self.lower_exprs(args)?;
        Ok(hir::Expr {
            kind: hir::ExprKind::Builtin(builtin, hargs),
            ty,
            span,
        })
    }

    /// Lower a slice of expressions.
    fn lower_exprs(&mut self, exprs: &[ast::Expr]) -> Result<Vec<hir::Expr>, String> {
        exprs.iter().map(|e| self.lower_expr(e)).collect()
    }
}
