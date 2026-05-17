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

            // Builtin functions — dedicated MIR instructions for optimizable ones
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
                        // NOTE: inst.ty carries the argument type (not Void) so
                        // codegen can determine the format specifier for printing.
                        self.emit(InstKind::Log(v), arg_ty, span)
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
                    BuiltinFn::RcAlloc => {
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        // `ty` is the wrapper (Rc<T> / RcCell<T> / Arc<T>);
                        // RcNew's second field is the INNER payload type so
                        // codegen can build the correct `{strong, weak, T}`
                        // layout. Passing the wrapper here builds a layout
                        // sized for `Rc<Wrapper>` (one-ptr payload) and then
                        // stores a full T into it — heap corruption for any
                        // T larger than a pointer (Rc<String>, Rc<Vec<_>>,
                        // Rc<Struct>, ...).
                        let inner = match &ty {
                            Type::Rc(inner) | Type::RcCell(inner) | Type::Arc(inner) => {
                                (**inner).clone()
                            }
                            other => other.clone(),
                        };
                        self.emit(InstKind::RcNew(v, inner), ty, span)
                    }
                    BuiltinFn::RcRetain => {
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::RcClone(v), ty, span)
                    }
                    BuiltinFn::RcRelease => {
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::RcDec(v), ty, span)
                    }
                    BuiltinFn::WeakUpgrade => {
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::WeakUpgrade(v), ty, span)
                    }
                    BuiltinFn::WeakDowngrade => {
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::WeakDowngrade(v), ty, span)
                    }
                    BuiltinFn::FloatMethod(method) => {
                        // Map x.abs()/floor()/ceil()/sqrt()/etc. to libm calls.
                        let m = method.as_str();
                        let libm_name: &str = match &*m {
                            "abs" => "fabs",
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
                            other => other, // assume libm name matches
                        };
                        self.emit(
                            InstKind::Call(Symbol::intern(libm_name), vals),
                            ty,
                            span,
                        )
                    }
                    _ => {
                        let name = Symbol::intern(&format!("__builtin_{builtin:?}"));
                        self.emit(InstKind::Call(name, vals), ty, span)
                    }
                }
            }

            // Syscall — opaque call
            ExprKind::Syscall(args) => {
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::Call("__syscall".into(), vals), ty, span)
            }

            // Coroutines — opaque calls
            ExprKind::DynDispatch(obj, trait_name, method, args) => {
                let obj_val = self.lower_expr(obj);
                let arg_vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(
                    InstKind::DynDispatch(obj_val, *trait_name, *method, arg_vals),
                    ty,
                    span,
                )
            }
            ExprKind::DynCoerce(inner, type_name, trait_name) => {
                let inner_val = self.lower_expr(inner);
                self.emit(
                    InstKind::DynCoerce(inner_val, *type_name, *trait_name),
                    ty,
                    span,
                )
            }

            // Store operations — opaque calls
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
