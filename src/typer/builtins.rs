use crate::ast::{self, Span};
use crate::hir;
use crate::types::Type;

use super::Typer;

impl Typer {
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
                // Generate a descriptive message from the assert expression
                let expr_desc = Self::describe_assert_expr(&args[0]);
                let msg_expr = hir::Expr {
                    kind: hir::ExprKind::Str(expr_desc),
                    ty: Type::String,
                    span,
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::Assert, vec![hcond, msg_expr]),
                    ty: Type::Void,
                    span,
                }))
            }
            "log" => Some(self.lower_simple_builtin(args, hir::BuiltinFn::Log, Type::Void, span)),
            "to_string" => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::ToString, Type::String, span))
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
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::RcRetain, Type::Void, span))
            }
            "rc_release" => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::RcRelease, Type::Void, span))
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
                    _ => {
                        return Some(Err(format!(
                            "weak_upgrade() requires a weak value, got {}",
                            harg.ty
                        )));
                    }
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
                    _ => {
                        return Some(Err(format!(
                            "volatile_load() requires a pointer, got {}",
                            harg.ty
                        )));
                    }
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
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::VolatileStore, vec![hptr, hval]),
                    ty: Type::Void,
                    span,
                }))
            }
            "wrapping_add" | "wrapping_sub" | "wrapping_mul" | "saturating_add"
            | "saturating_sub" | "saturating_mul" | "checked_add" | "checked_sub"
            | "checked_mul"
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
                    "saturating_add" => hir::BuiltinFn::SaturatingAdd,
                    "saturating_sub" => hir::BuiltinFn::SaturatingSub,
                    "saturating_mul" => hir::BuiltinFn::SaturatingMul,
                    "checked_add" => hir::BuiltinFn::CheckedAdd,
                    "checked_sub" => hir::BuiltinFn::CheckedSub,
                    "checked_mul" => hir::BuiltinFn::CheckedMul,
                    _ => unreachable!(),
                };
                let ret_ty = if name.starts_with("checked_") {
                    Type::Tuple(vec![ty, Type::Bool])
                } else {
                    ty
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(builtin, vec![lhs, rhs]),
                    ty: ret_ty,
                    span,
                }))
            }
            "signal_handle" if args.len() == 2 && !self.fns.contains_key(name) => Some(
                self.lower_simple_builtin(args, hir::BuiltinFn::SignalHandle, Type::Void, span),
            ),
            "signal_raise" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::SignalRaise, Type::I32, span))
            }
            "signal_ignore" if args.len() == 1 && !self.fns.contains_key(name) => Some(
                self.lower_simple_builtin(args, hir::BuiltinFn::SignalIgnore, Type::Void, span),
            ),
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
                Some(self.lower_simple_builtin(args, builtin, Type::I64, span))
            }
            "__string_from_raw" if args.len() == 3 && !self.fns.contains_key(name) => Some(
                self.lower_simple_builtin(args, hir::BuiltinFn::StringFromRaw, Type::String, span),
            ),
            "__string_from_ptr" if args.len() == 1 && !self.fns.contains_key(name) => Some(
                self.lower_simple_builtin(args, hir::BuiltinFn::StringFromPtr, Type::String, span),
            ),
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
                    .unwrap_or_else(|| self.infer_ctx.fresh_var());
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
            "map" if !self.fns.contains_key(name) => Some(Ok(hir::Expr {
                kind: hir::ExprKind::MapNew,
                ty: Type::Map(
                    Box::new(Type::String),
                    Box::new(self.infer_ctx.fresh_integer_var()),
                ),
                span,
            })),
            "set" if !self.fns.contains_key(name) => Some(Ok(hir::Expr {
                kind: hir::ExprKind::SetNew,
                ty: Type::Set(Box::new(self.infer_ctx.fresh_var())),
                span,
            })),
            "priority_queue" if !self.fns.contains_key(name) => Some(Ok(hir::Expr {
                kind: hir::ExprKind::PQNew,
                ty: Type::PriorityQueue(Box::new(self.infer_ctx.fresh_var())),
                span,
            })),
            "vec_with_alloc" if args.len() == 1 && !self.fns.contains_key(name) => {
                let halloc = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::VecWithAlloc, vec![halloc]),
                    ty: Type::Vec(Box::new(self.infer_ctx.fresh_var())),
                    span,
                }))
            }
            "map_with_alloc" if args.len() == 1 && !self.fns.contains_key(name) => {
                let halloc = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::MapWithAlloc, vec![halloc]),
                    ty: Type::Map(Box::new(self.infer_ctx.fresh_var()), Box::new(self.infer_ctx.fresh_var())),
                    span,
                }))
            }
            "Arena" if args.len() == 1 && !self.fns.contains_key(name) => {
                let harg = match self.lower_expr_expected(&args[0], Some(&Type::I64)) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let _ = self.infer_ctx.unify_at(&harg.ty, &Type::I64, span, "Arena capacity");
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::ArenaNew, vec![harg]),
                    ty: Type::Arena,
                    span,
                }))
            }
            "matmul" if args.len() == 2 && !self.fns.contains_key(name) => {
                let ha = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let hb = match self.lower_expr(&args[1]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let result_ty = ha.ty.clone();
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::Matmul, vec![ha, hb]),
                    ty: result_ty,
                    span,
                }))
            }
            // Constant-time operations (prevents timing side-channel attacks)
            "constant_time_eq" if args.len() == 2 && !self.fns.contains_key(name) => {
                let ha = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let hb = match self.lower_expr(&args[1]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::ConstantTimeEq, vec![ha, hb]),
                    ty: Type::Bool,
                    span,
                }))
            }
            // Comptime reflection builtins
            "fields_of" if args.len() == 1 && !self.fns.contains_key(name) => {
                // fields_of('StructName') → Vec of field name strings at compile time
                let type_name = match &args[0] {
                    ast::Expr::Str(s, _) => s.clone(),
                    ast::Expr::Ident(s, _) => s.clone(),
                    _ => return Some(Err("fields_of expects a type name".into())),
                };
                let fields = self.structs.get(&type_name).cloned().unwrap_or_default();
                let field_exprs: Vec<hir::Expr> = fields
                    .iter()
                    .map(|(fname, _)| hir::Expr {
                        kind: hir::ExprKind::Str(fname.clone()),
                        ty: Type::String,
                        span,
                    })
                    .collect();
                let len = field_exprs.len();
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Array(field_exprs),
                    ty: Type::Array(Box::new(Type::String), len),
                    span,
                }))
            }
            "type_of" if args.len() == 1 && !self.fns.contains_key(name) => {
                // type_of(expr) → string representation of the type at compile time
                let harg = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let ty_str = format!("{}", harg.ty);
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Str(ty_str),
                    ty: Type::String,
                    span,
                }))
            }
            "size_of" if args.len() == 1 && !self.fns.contains_key(name) => {
                // size_of('StructName') or size_of(expr) → byte size as i64
                let size = match &args[0] {
                    ast::Expr::Str(s, _) | ast::Expr::Ident(s, _) => {
                        if let Some(fields) = self.structs.get(s) {
                            fields.len() as i64 * 8 // rough estimate: 8 bytes per field
                        } else {
                            0
                        }
                    }
                    _ => {
                        let harg = match self.lower_expr(&args[0]) {
                            Ok(e) => e,
                            Err(e) => return Some(Err(e)),
                        };
                        match &harg.ty {
                            Type::I8 | Type::U8 | Type::Bool => 1,
                            Type::I16 | Type::U16 => 2,
                            Type::I32 | Type::U32 | Type::F32 => 4,
                            Type::I64 | Type::U64 | Type::F64 => 8,
                            Type::String => 24,
                            _ => 8, // pointer-sized default
                        }
                    }
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Int(size),
                    ty: Type::I64,
                    span,
                }))
            }
            _ => None,
        }
    }

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

    fn lower_exprs(&mut self, exprs: &[ast::Expr]) -> Result<Vec<hir::Expr>, String> {
        exprs.iter().map(|e| self.lower_expr(e)).collect()
    }

    fn describe_assert_expr(expr: &ast::Expr) -> String {
        match expr {
            ast::Expr::BinOp(l, op, r, _) => {
                let ls = Self::describe_assert_expr(l);
                let rs = Self::describe_assert_expr(r);
                let ops = match op {
                    ast::BinOp::Eq => "equals",
                    ast::BinOp::Ne => "not equals",
                    ast::BinOp::Lt => "<",
                    ast::BinOp::Le => "<=",
                    ast::BinOp::Gt => ">",
                    ast::BinOp::Ge => ">=",
                    ast::BinOp::And => "and",
                    ast::BinOp::Or => "or",
                    _ => "op",
                };
                format!("{ls} {ops} {rs}")
            }
            ast::Expr::Ident(name, _) => name.clone(),
            ast::Expr::Int(n, _) => n.to_string(),
            ast::Expr::Float(f, _) => f.to_string(),
            ast::Expr::Str(s, _) => format!("'{s}'"),
            ast::Expr::Bool(b, _) => b.to_string(),
            ast::Expr::Method(obj, method, _, _) => {
                format!("{}.{method}", Self::describe_assert_expr(obj))
            }
            ast::Expr::Field(obj, field, _) => {
                format!("{}.{field}", Self::describe_assert_expr(obj))
            }
            ast::Expr::Call(callee, _, _) => {
                format!("{}(..)", Self::describe_assert_expr(callee))
            }
            _ => "expr".into(),
        }
    }
}
