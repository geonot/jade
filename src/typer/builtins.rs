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
        if let Some(op) = crate::integer_arithmetic::IntegerArithmetic::from_source(name)
            && !self.fns.contains_key(name)
        {
            return Some((|| {
                if args.len() != 2 {
                    return Err(format!("{name} requires exactly two integer operands"));
                }
                let lhs = self.lower_expr(&args[0])?;
                let rhs = self.lower_expr_expected(&args[1], Some(&lhs.ty))?;
                self.infer_ctx.constrain(
                    &lhs.ty,
                    super::unify::TypeConstraint::Integer,
                    span,
                    "integer arithmetic operand",
                )?;
                self.infer_ctx
                    .unify_at(&lhs.ty, &rhs.ty, span, "integer arithmetic operands")?;
                let ty = op.result_type(&lhs.ty);
                Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(op.builtin.clone(), vec![lhs, rhs]),
                    ty,
                    span,
                })
            })());
        }
        match name {
            "assert" => {
                if args.is_empty() {
                    return Some(Err("assert requires a condition".into()));
                }
                let hcond = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };

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
            "log" | "print" => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::Log, Type::Void, span))
            }
            "eprint" => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::Eprint, Type::Void, span))
            }
            "to_string" => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::ToString, Type::String, span))
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
            "__signal_handle" | "signal_handle"
                if args.len() == 2 && !self.fns.contains_key(name) =>
            {
                Some(self.lower_simple_builtin(
                    args,
                    hir::BuiltinFn::SignalHandle,
                    Type::Void,
                    span,
                ))
            }
            "__signal_raise" | "signal_raise"
                if args.len() == 1 && !self.fns.contains_key(name) =>
            {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::SignalRaise, Type::I32, span))
            }
            "__signal_ignore" | "signal_ignore"
                if args.len() == 1 && !self.fns.contains_key(name) =>
            {
                Some(self.lower_simple_builtin(
                    args,
                    hir::BuiltinFn::SignalIgnore,
                    Type::Void,
                    span,
                ))
            }
            "__signal_default" | "signal_default"
                if args.len() == 1 && !self.fns.contains_key(name) =>
            {
                Some(self.lower_simple_builtin(
                    args,
                    hir::BuiltinFn::SignalDefault,
                    Type::Void,
                    span,
                ))
            }
            "__signal_kill" | "signal_kill" if args.len() == 2 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::SignalKill, Type::I32, span))
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
                Some(self.lower_simple_builtin(args, builtin, Type::I64, span))
            }
            "__string_from_raw" | "__string_from_ptr_len"
                if (2..=3).contains(&args.len()) && !self.fns.contains_key(name) =>
            {
                Some(self.lower_simple_builtin(
                    args,
                    hir::BuiltinFn::StringFromRaw,
                    Type::String,
                    span,
                ))
            }
            "__string_from_ptr" if args.len() == 1 && !self.fns.contains_key(name) => Some(
                self.lower_simple_builtin(args, hir::BuiltinFn::StringFromPtr, Type::String, span),
            ),

            "chr" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_int_to_string_builtin(args, hir::BuiltinFn::Chr, "chr", span))
            }
            "byte" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_int_to_string_builtin(args, hir::BuiltinFn::Byte, "byte", span))
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
                    ty: Type::I64,
                    span,
                }))
            }
            "__sleep_ms" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::SleepMs, Type::Void, span))
            }
            "__file_exists" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::FileExists, Type::Bool, span))
            }
            "vec" | "vector" => {
                let hargs = match self.lower_exprs(args) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                let elem_ty = hargs.first().map(|a| a.ty.clone()).unwrap_or_else(|| {
                    self.infer_ctx
                        .fresh_var_at(span, "empty `vec()` literal — element type is unknown")
                });
                for a in hargs.iter().skip(1) {
                    let _ = self
                        .infer_ctx
                        .unify_at(&elem_ty, &a.ty, span, "vec element");
                }
                let mut hargs = hargs;
                self.clone_string_captures_in_elems(&mut hargs);
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::VecNew(hargs),
                    ty: Type::Vec(Box::new(elem_ty)),
                    span,
                }))
            }
            "map" if !self.fns.contains_key(name) => {
                if !args.is_empty() {
                    let probe = match self.lower_expr(&args[0]) {
                        Ok(e) => e,
                        Err(e) => return Some(Err(e)),
                    };
                    let probe_ty = self.infer_ctx.shallow_resolve(&probe.ty);
                    if matches!(probe_ty, Type::Vec(_) | Type::Array(_, _)) {
                        return Some(self.lower_method_call(&args[0], "map", &args[1..], span));
                    }
                }
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::MapNew,
                    ty: Type::Map(Box::new(Type::String), Box::new(self.infer_ctx.fresh_var())),
                    span,
                }))
            }
            "vec_with_alloc" if args.len() == 1 && !self.fns.contains_key(name) => {
                let halloc = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::Builtin(hir::BuiltinFn::VecWithAlloc, vec![halloc]),
                    ty: Type::Vec(Box::new(self.infer_ctx.fresh_var_at(
                        span,
                        "empty `vec_with_alloc()` literal — element type is unknown",
                    ))),
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
                    ty: Type::Map(
                        Box::new(self.infer_ctx.fresh_var()),
                        Box::new(self.infer_ctx.fresh_var()),
                    ),
                    span,
                }))
            }
            "Arena" if args.len() == 1 && !self.fns.contains_key(name) => {
                Some(Err("Arena type removed".into()))
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

            "fields_of" if args.len() == 1 && !self.fns.contains_key(name) => {
                let type_name = match &args[0] {
                    ast::Expr::Str(s, _) => s.clone(),
                    ast::Expr::Ident(s, _) => s.as_str(),
                    _ => return Some(Err("fields_of expects a type name".into())),
                };
                let fields = self.structs.get(&type_name).cloned().unwrap_or_default();
                let field_exprs: Vec<hir::Expr> = fields
                    .iter()
                    .map(|(fname, _)| hir::Expr {
                        kind: hir::ExprKind::Str(fname.as_str()),
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
                Some(self.lower_size_of(&args[0], span))
            }
            "likely" => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::Likely, Type::Bool, span))
            }
            "unlikely" => {
                Some(self.lower_simple_builtin(args, hir::BuiltinFn::Unlikely, Type::Bool, span))
            }
            "atomic_load" if args.len() == 1 && !self.fns.contains_key(name) => {
                let harg = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                if !matches!(harg.ty, Type::Ptr(_)) {
                    return Some(Err(format!(
                        "atomic_load() requires a pointer, got {}",
                        harg.ty
                    )));
                }
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::AtomicLoad(Box::new(harg)),
                    ty: Type::I64,
                    span,
                }))
            }
            "atomic_store" if args.len() == 2 && !self.fns.contains_key(name) => {
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
                        "atomic_store() first arg must be a pointer, got {}",
                        hptr.ty
                    )));
                }
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::AtomicStore(Box::new(hptr), Box::new(hval)),
                    ty: Type::Void,
                    span,
                }))
            }
            "atomic_add" if args.len() == 2 && !self.fns.contains_key(name) => {
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
                        "atomic_add() first arg must be a pointer, got {}",
                        hptr.ty
                    )));
                }
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::AtomicAdd(Box::new(hptr), Box::new(hval)),
                    ty: Type::I64,
                    span,
                }))
            }
            "atomic_cas" if args.len() == 3 && !self.fns.contains_key(name) => {
                let hptr = match self.lower_expr(&args[0]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let hexpected = match self.lower_expr(&args[1]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let hnew = match self.lower_expr(&args[2]) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                if !matches!(hptr.ty, Type::Ptr(_)) {
                    return Some(Err(format!(
                        "atomic_cas() first arg must be a pointer, got {}",
                        hptr.ty
                    )));
                }
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::AtomicCas(
                        Box::new(hptr),
                        Box::new(hexpected),
                        Box::new(hnew),
                    ),
                    ty: Type::I64,
                    span,
                }))
            }
            "atomic_sub" if args.len() == 2 && !self.fns.contains_key(name) => {
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
                        "atomic_sub() first arg must be a pointer, got {}",
                        hptr.ty
                    )));
                }
                Some(Ok(hir::Expr {
                    kind: hir::ExprKind::AtomicSub(Box::new(hptr), Box::new(hval)),
                    ty: Type::I64,
                    span,
                }))
            }
            "Pool" if args.len() == 2 && !self.fns.contains_key(name) => {
                Some(Err("Pool type removed".into()))
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
        if matches!(builtin, hir::BuiltinFn::Log | hir::BuiltinFn::Eprint)
            && let Some(a) = hargs.first()
            && matches!(a.kind, hir::ExprKind::Send(..))
        {
            return Err(format!(
                "{}: an actor handler call produces no value — messages are \
                 asynchronous sends, so there is nothing to print here; have the \
                 handler log its own state, or pass a reply channel to read a \
                 value back",
                span.loc(),
            ));
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::Builtin(builtin, hargs),
            ty,
            span,
        })
    }

    fn lower_int_to_string_builtin(
        &mut self,
        args: &[ast::Expr],
        builtin: hir::BuiltinFn,
        bname: &str,
        span: Span,
    ) -> Result<hir::Expr, String> {
        let hargs = self.lower_exprs(args)?;
        if let Some(a) = hargs.first() {
            let resolved = self.infer_ctx.resolve(&a.ty);
            match &resolved {
                t if t.is_int() => {}
                Type::TypeVar(_) => {
                    let _ = self.infer_ctx.unify(&a.ty, &Type::I64);
                }
                other => {
                    return Err(format!(
                        "{}: `{bname}` takes an integer code, found {other}",
                        span.loc()
                    ));
                }
            }
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::Builtin(builtin, hargs),
            ty: Type::String,
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
            ast::Expr::Ident(name, _) => name.as_str(),
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
