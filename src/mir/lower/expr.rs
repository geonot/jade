use super::super::*;
use super::Lowerer;
use crate::ast;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;

impl Lowerer {
    pub(super) fn lower_expr_value(&mut self, expr: &hir::Expr) -> ValueId {
        let span = expr.span;
        let ty = expr.ty.clone();
        match &expr.kind {
            ExprKind::Int(n) => self.emit(InstKind::IntConst(*n), ty, span),
            ExprKind::Float(f) => self.emit(InstKind::FloatConst(*f), ty, span),
            ExprKind::Bool(b) => self.emit(InstKind::BoolConst(*b), ty, span),
            ExprKind::Str(s) => self.emit(InstKind::StringConst(s.clone()), ty, span),
            ExprKind::Void => self.emit(InstKind::Void, Type::Void, span),
            ExprKind::None => self.emit(InstKind::IntConst(0), ty, span),

            ExprKind::Var(def_id, name) => {
                if let Some((field_sym, field_ty)) = self.field_lookup(*def_id) {
                    let self_state = self.field_self();
                    return self.emit(InstKind::FieldGet(self_state, field_sym), field_ty, span);
                }
                let key = self.var_key(*def_id, *name);
                self.read_var(key, self.current_block, ty, span)
            }

            ExprKind::BinOp(lhs, op, rhs) => {
                let l = self.lower_expr(lhs);
                let r = self.lower_expr(rhs);
                let operand_ty = lhs.ty.clone();
                let cmp = match op {
                    ast::BinOp::Eq => Some(CmpOp::Eq),
                    ast::BinOp::Ne => Some(CmpOp::Ne),
                    ast::BinOp::Lt => Some(CmpOp::Lt),
                    ast::BinOp::Gt => Some(CmpOp::Gt),
                    ast::BinOp::Le => Some(CmpOp::Le),
                    ast::BinOp::Ge => Some(CmpOp::Ge),
                    ast::BinOp::And | ast::BinOp::Or => {
                        unreachable!("short-circuit binop lowered by expr_control")
                    }
                    _ => None,
                };
                let kind = match cmp {
                    Some(c) => InstKind::Cmp(c, l, r, operand_ty),
                    None => InstKind::BinOp(super::lower_binop(op), l, r),
                };
                let reads_operands = cmp.is_some() || matches!(ty, Type::String);
                let out = self.emit(kind, ty, span);
                if reads_operands {
                    self.drop_if_fresh(l, lhs, span);
                    self.drop_if_fresh(r, rhs, span);
                }
                out
            }
            ExprKind::UnaryOp(op, inner) => {
                let v = self.lower_expr(inner);
                let mir_op = super::lower_unaryop(op);
                self.emit(InstKind::UnaryOp(mir_op, v), ty, span)
            }
            ExprKind::Call(def_id, name, args) => {
                let arg_vals: Vec<ValueId> =
                    args.iter().map(|a| self.lower_expr_owned(a)).collect();
                let key = self.call_key(*def_id, *name);
                self.emit(InstKind::Call(key, arg_vals), ty, span)
            }
            ExprKind::IndirectCall(callee, args) => {
                let f = self.lower_expr(callee);
                let arg_vals: Vec<ValueId> =
                    args.iter().map(|a| self.lower_expr_owned(a)).collect();
                self.emit(InstKind::IndirectCall(f, arg_vals), ty, span)
            }

            ExprKind::Method(obj, mangled_name, _method_name, args) => {
                let obj_val = self.lower_expr(obj);
                let arg_vals: Vec<ValueId> =
                    args.iter().map(|a| self.lower_expr_owned(a)).collect();
                self.emit(
                    InstKind::MethodCall(obj_val, *mangled_name, arg_vals, false),
                    ty,
                    span,
                )
            }
            ExprKind::Field(obj, field, _idx) => {
                let obj_val = self.lower_expr(obj);
                self.emit(InstKind::FieldGet(obj_val, *field), ty, span)
            }
            ExprKind::Index(arr, idx) => {
                let a = self.lower_expr(arr);
                let i = self.lower_expr(idx);
                self.emit(InstKind::Index(a, i), ty, span)
            }
            ExprKind::Struct(name, inits) => {
                let fields: Vec<(Symbol, ValueId)> = inits
                    .iter()
                    .map(|fi| {
                        let v = self.lower_expr_owned(&fi.value);
                        (fi.name.unwrap_or(Symbol::intern("")), v)
                    })
                    .collect();
                self.emit(InstKind::StructInit(*name, fields), ty, span)
            }
            ExprKind::VariantCtor(enum_name, variant_name, tag, inits) => {
                let arg_vals: Vec<ValueId> = inits
                    .iter()
                    .map(|fi| self.lower_expr_owned(&fi.value))
                    .collect();
                self.emit(
                    InstKind::VariantInit(*enum_name, *variant_name, *tag, arg_vals),
                    ty,
                    span,
                )
            }
            ExprKind::VariantRef(enum_name, variant_name, tag) => self.emit(
                InstKind::VariantInit(*enum_name, *variant_name, *tag, vec![]),
                ty,
                span,
            ),
            ExprKind::Coerce(inner, kind) => match kind {
                hir::CoercionKind::ArrayToVec { elem_ty, len } => {
                    let arr = self.lower_expr(inner);
                    let elem_ty = elem_ty.clone();
                    let n = *len;
                    let elems: Vec<ValueId> = (0..n)
                        .map(|i| {
                            let idx = self.emit(InstKind::IntConst(i as i64), Type::I64, span);
                            self.emit(InstKind::IndexUnchecked(arr, idx), elem_ty.clone(), span)
                        })
                        .collect();
                    self.emit(InstKind::VecNew(elems), ty, span)
                }

                _ => {
                    let v = self.lower_expr(inner);
                    self.emit(InstKind::Cast(v, ty.clone()), ty, span)
                }
            },

            ExprKind::Pipe(inner, def_id, name, extra_args) => {
                let mut args = vec![self.lower_expr_owned(inner)];
                args.extend(extra_args.iter().map(|a| self.lower_expr_owned(a)));
                let key = self.call_key(*def_id, *name);
                self.emit(InstKind::Call(key, args), ty, span)
            }

            ExprKind::Cast(inner, target_ty) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Cast(v, target_ty.clone()), ty, span)
            }
            ExprKind::StrictCast(inner, target_ty) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::StrictCast(v, target_ty.clone()), ty, span)
            }
            ExprKind::Ref(inner) => {
                if let ExprKind::Var(def_id, name) = &inner.kind
                    && Self::addressable_scalar(&inner.ty)
                    && self.field_lookup(*def_id).is_none()
                {
                    let key = self.var_key(*def_id, *name);
                    if !self.borrowed_params.contains(&key) {
                        return self.take_address(key, inner.ty.clone(), ty, span);
                    }
                }
                let v = self.lower_expr(inner);
                self.emit(InstKind::Ref(v), ty, span)
            }
            ExprKind::Deref(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Deref(v), ty, span)
            }
            ExprKind::Array(elems) | ExprKind::Tuple(elems) => {
                let vals: Vec<ValueId> = elems.iter().map(|e| self.lower_expr_owned(e)).collect();
                self.emit(InstKind::ArrayInit(vals), ty, span)
            }
            ExprKind::Slice(arr, start, end) => {
                let a = self.lower_expr(arr);
                let s = self.lower_expr(start);
                let e = self.lower_expr(end);
                self.emit(InstKind::Slice(a, s, e), ty, span)
            }
            ExprKind::FnRef(def_id, name) => {
                let key = self.call_key(*def_id, *name);
                self.emit(InstKind::FnRef(key), ty, span)
            }
            ExprKind::Builder(name, fields) => {
                let inits: Vec<(Symbol, ValueId)> = fields
                    .iter()
                    .map(|(n, e)| (*n, self.lower_expr_owned(e)))
                    .collect();
                self.emit(InstKind::StructInit(*name, inits), ty, span)
            }
            ExprKind::AsFormat(inner, _fmt_str) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__as_format".into(), vec![v]), ty, span)
            }
            ExprKind::EnumUnwrap(inner, _enum_name, success_tag) => {
                let subj = self.lower_expr(inner);

                let tag = self.emit(InstKind::FieldGet(subj, "__tag".into()), Type::I64, span);
                let expected = self.emit(InstKind::IntConst(*success_tag as i64), Type::I64, span);
                let cmp = self.emit(
                    InstKind::Cmp(CmpOp::Eq, tag, expected, Type::I64),
                    Type::Bool,
                    span,
                );

                self.emit(
                    InstKind::Assert(cmp, "unwrap called on Nothing/Err".into()),
                    Type::Void,
                    span,
                );

                self.emit(
                    InstKind::FieldGet(
                        subj,
                        crate::intern::Symbol::intern(&format!("__v{success_tag}_0")),
                    ),
                    ty.clone(),
                    span,
                )
            }
            ExprKind::EnumIs(inner, check_tag) => {
                let subj = self.lower_expr(inner);
                let tag = self.emit(InstKind::FieldGet(subj, "__tag".into()), Type::I64, span);
                let expected = self.emit(InstKind::IntConst(*check_tag as i64), Type::I64, span);
                self.emit(
                    InstKind::Cmp(CmpOp::Eq, tag, expected, Type::I64),
                    Type::Bool,
                    span,
                )
            }
            ExprKind::GlobalLoad(name) => self.emit(InstKind::GlobalLoad(*name), ty, span),
            ExprKind::SizeOf(t) => self.emit(InstKind::SizeOf(t.clone()), ty, span),
            ExprKind::Unreachable => {
                self.set_terminator(Terminator::Unreachable);
                let dead = self.new_block("after.unreachable");
                self.switch_to(dead);
                self.emit(InstKind::Void, ty, span)
            }
            _ => unreachable!("expression dispatched to wrong MIR lowering module"),
        }
    }
}
