use super::super::*;
use super::Lowerer;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;

impl Lowerer {
    pub(super) fn lower_expr_intrinsics(&mut self, expr: &hir::Expr) -> ValueId {
        let span = expr.span;
        let ty = expr.ty.clone();
        match &expr.kind {
            ExprKind::AtomicLoad(p) => {
                let v = self.lower_expr(p);
                self.emit(InstKind::Call("__atomic_load".into(), vec![v]), ty, span)
            }
            ExprKind::AtomicStore(p, val) => {
                let args = vec![self.lower_expr(p), self.lower_expr(val)];
                self.emit(InstKind::Call("__atomic_store".into(), args), ty, span)
            }
            ExprKind::AtomicAdd(p, val) => {
                let args = vec![self.lower_expr(p), self.lower_expr(val)];
                self.emit(InstKind::Call("__atomic_add".into(), args), ty, span)
            }
            ExprKind::AtomicSub(p, val) => {
                let args = vec![self.lower_expr(p), self.lower_expr(val)];
                self.emit(InstKind::Call("__atomic_sub".into(), args), ty, span)
            }
            ExprKind::AtomicCas(p, expected, desired) => {
                let args = vec![
                    self.lower_expr(p),
                    self.lower_expr(expected),
                    self.lower_expr(desired),
                ];
                self.emit(InstKind::Call("__atomic_cas".into(), args), ty, span)
            }

            ExprKind::Builtin(builtin, args) => {
                use crate::hir::BuiltinFn;
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                match builtin {
                    BuiltinFn::Log => {
                        let arg_ty = args.first().map(|a| a.ty.clone()).unwrap_or(Type::I64);
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));

                        self.emit(InstKind::Log(v), arg_ty, span)
                    }
                    BuiltinFn::Eprint => {
                        let arg_ty = args.first().map(|a| a.ty.clone()).unwrap_or(Type::I64);
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));

                        self.emit(InstKind::Eprint(v), arg_ty, span)
                    }
                    BuiltinFn::Print => {
                        let name = Symbol::intern("__builtin_Print");
                        self.emit(InstKind::Call(name, vals), ty, span)
                    }
                    BuiltinFn::Assert => {
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::Assert(v, "assertion failed".into()), ty, span)
                    }
                    BuiltinFn::FloatMethod(method) => {
                        let m = method.as_str();
                        let recv_ty = args.first().map(|a| a.ty.clone()).unwrap_or(Type::F64);
                        match &*m {
                            "is_nan" => {
                                let x = vals[0];
                                self.emit(InstKind::Cmp(CmpOp::Ne, x, x, recv_ty), Type::Bool, span)
                            }
                            "is_infinite" => {
                                let x = vals[0];
                                let mag = self.emit(
                                    InstKind::Call(Symbol::intern("fabs"), vec![x]),
                                    recv_ty.clone(),
                                    span,
                                );
                                let inf = self.emit(
                                    InstKind::FloatConst(f64::INFINITY),
                                    recv_ty.clone(),
                                    span,
                                );
                                self.emit(
                                    InstKind::Cmp(CmpOp::Eq, mag, inf, recv_ty),
                                    Type::Bool,
                                    span,
                                )
                            }
                            "is_finite" => {
                                let x = vals[0];
                                let mag = self.emit(
                                    InstKind::Call(Symbol::intern("fabs"), vec![x]),
                                    recv_ty.clone(),
                                    span,
                                );
                                let inf = self.emit(
                                    InstKind::FloatConst(f64::INFINITY),
                                    recv_ty.clone(),
                                    span,
                                );
                                self.emit(
                                    InstKind::Cmp(CmpOp::Lt, mag, inf, recv_ty),
                                    Type::Bool,
                                    span,
                                )
                            }
                            "to_int" => {
                                let x = vals[0];
                                self.emit(InstKind::Cast(x, Type::I64), Type::I64, span)
                            }
                            "recip" => {
                                let x = vals[0];
                                let one =
                                    self.emit(InstKind::FloatConst(1.0), recv_ty.clone(), span);
                                self.emit(InstKind::BinOp(BinOp::Div, one, x), ty, span)
                            }
                            "signum" => {
                                let x = vals[0];
                                let one =
                                    self.emit(InstKind::FloatConst(1.0), recv_ty.clone(), span);
                                self.emit(
                                    InstKind::Call(Symbol::intern("copysign"), vec![one, x]),
                                    ty,
                                    span,
                                )
                            }
                            _ => {
                                let libm_name: &str = match &*m {
                                    "abs" => "fabs",
                                    "min" => "fmin",
                                    "max" => "fmax",
                                    "sqrt" => "sqrt",
                                    "floor" => "floor",
                                    "ceil" => "ceil",
                                    "round" => "round",
                                    "trunc" => "trunc",
                                    "sin" => "sin",
                                    "cos" => "cos",
                                    "tan" => "tan",
                                    "asin" => "asin",
                                    "acos" => "acos",
                                    "atan" => "atan",
                                    "log" | "ln" => "log",
                                    "log10" => "log10",
                                    "log2" => "log2",
                                    "exp" => "exp",
                                    "exp2" => "exp2",
                                    other => other,
                                };
                                self.emit(InstKind::Call(Symbol::intern(libm_name), vals), ty, span)
                            }
                        }
                    }
                    _ => {
                        let name = Symbol::intern(&format!("__builtin_{builtin:?}"));
                        self.emit(InstKind::Call(name, vals), ty, span)
                    }
                }
            }

            ExprKind::Syscall(args) => {
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::Call("__syscall".into(), vals), ty, span)
            }

            ExprKind::Grad(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__grad".into(), vec![v]), ty, span)
            }
            ExprKind::Einsum(_pattern, args) => {
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::Call("__einsum".into(), vals), ty, span)
            }
            _ => unreachable!("expression dispatched to wrong MIR lowering module"),
        }
    }
}
