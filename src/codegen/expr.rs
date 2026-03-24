use std::collections::HashMap;
use std::collections::HashSet;

use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate};

use crate::ast::{BinOp, UnaryOp};
use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_expr(
        &mut self,
        expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match &expr.kind {
            hir::ExprKind::Int(n) => Ok(self.int_const(*n, &expr.ty)),
            hir::ExprKind::Float(n) => Ok(self.ctx.f64_type().const_float(*n).into()),
            hir::ExprKind::Str(s) => self.compile_str_literal(s),
            hir::ExprKind::Bool(v) => Ok(self.ctx.bool_type().const_int(*v as u64, false).into()),
            hir::ExprKind::None | hir::ExprKind::Void => {
                Ok(self.ctx.i64_type().const_int(0, false).into())
            }
            hir::ExprKind::Var(_, name) => self.load_var(name),
            hir::ExprKind::FnRef(_, name) => {
                if let Some(fv) = self.module.get_function(name) {
                    Ok(fv.as_global_value().as_pointer_value().into())
                } else {
                    Err(format!("undefined function: {name}"))
                }
            }
            hir::ExprKind::VariantRef(enum_name, variant_name, tag) => {
                self.compile_variant(enum_name, *tag, variant_name, &[])
            }
            hir::ExprKind::BinOp(l, op, r) => self.compile_binop(l, *op, r, &expr.ty),
            hir::ExprKind::UnaryOp(op, e) => self.compile_unary(*op, e),
            hir::ExprKind::Call(_, name, args) => self.compile_direct_call(name, args),
            hir::ExprKind::IndirectCall(callee, args) => self.compile_indirect_call(callee, args),
            hir::ExprKind::Builtin(builtin, args) => self.compile_builtin(builtin, args),
            hir::ExprKind::Method(obj, resolved_name, _method_name, args) => {
                self.compile_method(obj, resolved_name, args)
            }
            hir::ExprKind::StringMethod(obj, method, args) => {
                self.compile_string_method(obj, method, args)
            }
            hir::ExprKind::Field(obj, field, idx) => self.compile_field(obj, field, *idx),
            hir::ExprKind::Index(arr, idx) => self.compile_index(arr, idx),
            hir::ExprKind::Ternary(c, t, e) => self.compile_ternary(c, t, e),
            hir::ExprKind::Coerce(inner, coercion) => {
                let val = self.compile_expr(inner)?;
                self.compile_coercion(val, coercion)
            }
            hir::ExprKind::Cast(inner, target_ty) => self.compile_cast(inner, target_ty),
            hir::ExprKind::Array(elems) => self.compile_array(elems),
            hir::ExprKind::Tuple(elems) => self.compile_tuple(elems),
            hir::ExprKind::Struct(name, inits) => self.compile_struct(name, inits),
            hir::ExprKind::VariantCtor(enum_name, variant_name, tag, inits) => {
                self.compile_variant(enum_name, *tag, variant_name, inits)
            }
            hir::ExprKind::IfExpr(i) => match self.compile_if(i)? {
                Some(v) => Ok(v),
                None => Ok(self.ctx.i64_type().const_int(0, false).into()),
            },
            hir::ExprKind::Pipe(left, _def_id, name, extra_args) => {
                self.compile_pipe(left, name, extra_args)
            }
            hir::ExprKind::Block(block) => match self.compile_block(block)? {
                Some(v) => Ok(v),
                None => Ok(self.ctx.i64_type().const_int(0, false).into()),
            },
            hir::ExprKind::Lambda(params, body) => self.compile_lambda(params, body, &expr.ty),
            hir::ExprKind::Ref(inner) => self.compile_ref(inner),
            hir::ExprKind::Deref(inner) => self.compile_deref(inner),
            hir::ExprKind::ListComp(body, _def_id, bind, iter, end, cond) => {
                self.compile_list_comp(body, bind, iter, end.as_deref(), cond.as_deref())
            }
            hir::ExprKind::Syscall(args) => self.compile_syscall(args),
            hir::ExprKind::Spawn(actor_name) => self.compile_spawn(actor_name),
            hir::ExprKind::Send(target, actor_name, handler_name, tag, args) => {
                self.compile_send(target, actor_name, handler_name, *tag, args)
            }
            hir::ExprKind::StoreQuery(store_name, filter) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_query(store_name, filter, &sd)
            }
            hir::ExprKind::StoreCount(store_name) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_count(store_name, &sd)
            }
            hir::ExprKind::StoreAll(store_name) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_all(store_name, &sd)
            }
            hir::ExprKind::VecNew(elems) => self.compile_vec_new(elems),
            hir::ExprKind::MapNew => self.compile_map_new(),
            hir::ExprKind::VecMethod(obj, method, args) => {
                self.compile_vec_method(obj, method, args)
            }
            hir::ExprKind::MapMethod(obj, method, args) => {
                self.compile_map_method(obj, method, args)
            }
            hir::ExprKind::CoroutineCreate(name, body) => self.compile_coroutine_create(name, body),
            hir::ExprKind::CoroutineNext(coro) => self.compile_coroutine_next(coro),
            hir::ExprKind::Yield(_inner) => {
                panic!("yield expression outside of coroutine body")
            }
            hir::ExprKind::DynDispatch(obj, trait_name, method, args) => {
                self.compile_dyn_dispatch(obj, trait_name, method, args, &expr.ty)
            }
            hir::ExprKind::DynCoerce(inner, type_name, trait_name) => {
                self.compile_dyn_coerce(inner, type_name, trait_name)
            }
            hir::ExprKind::IterNext(iter_var, type_name, method_name) => {
                self.compile_iter_next_by_name(iter_var, type_name, method_name)
            }
            hir::ExprKind::ChannelCreate(elem_ty, cap_expr) => {
                self.compile_channel_create(elem_ty, cap_expr)
            }
            hir::ExprKind::ChannelSend(ch_expr, val_expr) => {
                self.compile_channel_send(ch_expr, val_expr)
            }
            hir::ExprKind::ChannelRecv(ch_expr) => self.compile_channel_recv(ch_expr, &expr.ty),
            hir::ExprKind::Select(arms, default_body) => {
                self.compile_select(arms, default_body.as_ref())
            }
        }
    }

    fn compile_binop(
        &mut self,
        left: &hir::Expr,
        op: BinOp,
        right: &hir::Expr,
        _result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if matches!(op, BinOp::And) {
            return self.compile_short_circuit(left, right, true);
        }
        if matches!(op, BinOp::Or) {
            return self.compile_short_circuit(left, right, false);
        }
        let lty = &left.ty;
        let rty = &right.ty;
        let (lhs, rhs) = if let hir::ExprKind::Int(n) = &left.kind {
            if rty.is_int() {
                (self.int_const(*n, rty), self.compile_expr(right)?)
            } else {
                (self.compile_expr(left)?, self.compile_expr(right)?)
            }
        } else if let hir::ExprKind::Int(n) = &right.kind {
            if lty.is_int() {
                (self.compile_expr(left)?, self.int_const(*n, lty))
            } else {
                (self.compile_expr(left)?, self.compile_expr(right)?)
            }
        } else {
            let (lhs, rhs) = (self.compile_expr(left)?, self.compile_expr(right)?);
            if lty.is_int() && rty.is_int() && lty.bits() != rty.bits() {
                self.coerce_int_width(lhs, rhs, lty, rty)
            } else {
                (lhs, rhs)
            }
        };
        let ety = if matches!(left.kind, hir::ExprKind::Int(..)) && rty.is_int() {
            rty
        } else {
            lty
        };
        if matches!(ety, Type::String) && matches!(op, BinOp::Add) {
            return self.string_concat(lhs, rhs);
        }
        if matches!(ety, Type::String) && matches!(op, BinOp::Eq | BinOp::Ne) {
            return self.string_eq(lhs, rhs, matches!(op, BinOp::Ne));
        }
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            if let Type::Struct(name) = ety {
                let fn_name = format!("{name}_eq");
                if let Some((fv, _, _)) = self.fns.get(&fn_name).cloned() {
                    let result = b!(self
                        .bld
                        .build_call(fv, &[lhs.into(), rhs.into()], "eq.call"))
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                    return if matches!(op, BinOp::Ne) {
                        Ok(b!(self.bld.build_not(result.into_int_value(), "neq")).into())
                    } else {
                        Ok(result)
                    };
                }
            }
        }
        if let Type::Struct(name) = ety {
            let trait_name = match op {
                BinOp::Add => Some("add"),
                BinOp::Sub => Some("sub"),
                BinOp::Mul => Some("mul"),
                BinOp::Div => Some("div"),
                BinOp::Lt => Some("lt"),
                BinOp::Gt => Some("gt"),
                BinOp::Le => Some("le"),
                BinOp::Ge => Some("ge"),
                _ => None,
            };
            if let Some(method) = trait_name {
                let fn_name = format!("{name}_{method}");
                if let Some((fv, _, _)) = self.fns.get(&fn_name).cloned() {
                    let result = b!(self.bld.build_call(
                        fv,
                        &[lhs.into(), rhs.into()],
                        &format!("{method}.call")
                    ))
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                    return Ok(result);
                }
            }
        }
        if ety.is_float() {
            self.compile_float_binop(lhs, rhs, op)
        } else {
            self.compile_int_binop(lhs, rhs, op, ety)
        }
    }

    fn compile_float_binop(
        &mut self,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        op: BinOp,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (l, r) = (lhs.into_float_value(), rhs.into_float_value());
        Ok(match op {
            BinOp::Add => b!(self.bld.build_float_add(l, r, "fadd")).into(),
            BinOp::Sub => b!(self.bld.build_float_sub(l, r, "fsub")).into(),
            BinOp::Mul => b!(self.bld.build_float_mul(l, r, "fmul")).into(),
            BinOp::Div => b!(self.bld.build_float_div(l, r, "fdiv")).into(),
            BinOp::Mod => b!(self.bld.build_float_rem(l, r, "fmod")).into(),
            BinOp::Exp => {
                let f64t = self.ctx.f64_type();
                let pt = f64t.fn_type(&[f64t.into(), f64t.into()], false);
                let pf = self
                    .module
                    .get_function("llvm.pow.f64")
                    .unwrap_or_else(|| self.module.add_function("llvm.pow.f64", pt, None));
                b!(self.bld.build_call(pf, &[l.into(), r.into()], "pow"))
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
            }
            BinOp::Eq => b!(self
                .bld
                .build_float_compare(FloatPredicate::OEQ, l, r, "feq"))
            .into(),
            BinOp::Ne => b!(self
                .bld
                .build_float_compare(FloatPredicate::ONE, l, r, "fne"))
            .into(),
            BinOp::Lt => b!(self
                .bld
                .build_float_compare(FloatPredicate::OLT, l, r, "flt"))
            .into(),
            BinOp::Gt => b!(self
                .bld
                .build_float_compare(FloatPredicate::OGT, l, r, "fgt"))
            .into(),
            BinOp::Le => b!(self
                .bld
                .build_float_compare(FloatPredicate::OLE, l, r, "fle"))
            .into(),
            BinOp::Ge => b!(self
                .bld
                .build_float_compare(FloatPredicate::OGE, l, r, "fge"))
            .into(),
            _ => return Err(format!("unsupported float op: {op:?}")),
        })
    }

    fn compile_int_binop(
        &mut self,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        op: BinOp,
        ety: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (l, r) = (lhs.into_int_value(), rhs.into_int_value());
        let s = ety.is_signed();
        Ok(match op {
            BinOp::Add => b!(self.bld.build_int_add(l, r, "add")).into(),
            BinOp::Sub => b!(self.bld.build_int_sub(l, r, "sub")).into(),
            BinOp::Mul => b!(self.bld.build_int_mul(l, r, "mul")).into(),
            BinOp::Div if s => self.checked_div(l, r, true)?,
            BinOp::Div => self.checked_div(l, r, false)?,
            BinOp::Mod if s => self.checked_rem(l, r, true)?,
            BinOp::Mod => self.checked_rem(l, r, false)?,
            BinOp::Eq => b!(self.bld.build_int_compare(IntPredicate::EQ, l, r, "eq")).into(),
            BinOp::Ne => b!(self.bld.build_int_compare(IntPredicate::NE, l, r, "ne")).into(),
            BinOp::Lt => b!(self.bld.build_int_compare(
                if s {
                    IntPredicate::SLT
                } else {
                    IntPredicate::ULT
                },
                l,
                r,
                "lt"
            ))
            .into(),
            BinOp::Gt => b!(self.bld.build_int_compare(
                if s {
                    IntPredicate::SGT
                } else {
                    IntPredicate::UGT
                },
                l,
                r,
                "gt"
            ))
            .into(),
            BinOp::Le => b!(self.bld.build_int_compare(
                if s {
                    IntPredicate::SLE
                } else {
                    IntPredicate::ULE
                },
                l,
                r,
                "le"
            ))
            .into(),
            BinOp::Ge => b!(self.bld.build_int_compare(
                if s {
                    IntPredicate::SGE
                } else {
                    IntPredicate::UGE
                },
                l,
                r,
                "ge"
            ))
            .into(),
            BinOp::BitAnd => b!(self.bld.build_and(l, r, "and")).into(),
            BinOp::BitOr => b!(self.bld.build_or(l, r, "or")).into(),
            BinOp::BitXor => b!(self.bld.build_xor(l, r, "xor")).into(),
            BinOp::Shl => b!(self.bld.build_left_shift(l, r, "shl")).into(),
            BinOp::Shr => b!(self.bld.build_right_shift(l, r, s, "shr")).into(),
            BinOp::Exp => self.compile_int_pow(l, r)?,
            _ => return Err(format!("unsupported int op: {op:?}")),
        })
    }

    fn checked_div(
        &mut self,
        l: inkwell::values::IntValue<'ctx>,
        r: inkwell::values::IntValue<'ctx>,
        signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let zero = r.get_type().const_int(0, false);
        let is_zero = b!(self
            .bld
            .build_int_compare(IntPredicate::EQ, r, zero, "div.z"));
        let trap_bb = self.ctx.append_basic_block(fv, "div.trap");
        let ok_bb = self.ctx.append_basic_block(fv, "div.ok");
        b!(self.bld.build_conditional_branch(is_zero, trap_bb, ok_bb));
        self.bld.position_at_end(trap_bb);
        self.emit_trap("division by zero");
        self.bld.position_at_end(ok_bb);
        Ok(if signed {
            b!(self.bld.build_int_signed_div(l, r, "sdiv")).into()
        } else {
            b!(self.bld.build_int_unsigned_div(l, r, "udiv")).into()
        })
    }

    fn checked_rem(
        &mut self,
        l: inkwell::values::IntValue<'ctx>,
        r: inkwell::values::IntValue<'ctx>,
        signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let zero = r.get_type().const_int(0, false);
        let is_zero = b!(self
            .bld
            .build_int_compare(IntPredicate::EQ, r, zero, "rem.z"));
        let trap_bb = self.ctx.append_basic_block(fv, "rem.trap");
        let ok_bb = self.ctx.append_basic_block(fv, "rem.ok");
        b!(self.bld.build_conditional_branch(is_zero, trap_bb, ok_bb));
        self.bld.position_at_end(trap_bb);
        self.emit_trap("division by zero");
        self.bld.position_at_end(ok_bb);
        Ok(if signed {
            b!(self.bld.build_int_signed_rem(l, r, "srem")).into()
        } else {
            b!(self.bld.build_int_unsigned_rem(l, r, "urem")).into()
        })
    }

    fn emit_trap(&mut self, msg: &str) {
        let trap_fn = self
            .module
            .get_function("__jade_trap")
            .unwrap_or_else(|| self.build_trap_fn());
        let msg_str = self
            .bld
            .build_global_string_ptr(msg, "trap.msg")
            .expect("build_global_string_ptr");
        self.bld
            .build_call(trap_fn, &[msg_str.as_pointer_value().into()], "")
            .expect("build_call trap");
        self.bld.build_unreachable().expect("build_unreachable");
    }

    fn build_trap_fn(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        let i32t = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
        let f = self
            .module
            .add_function("__jade_trap", ft, Some(Linkage::Internal));
        let entry = self.ctx.append_basic_block(f, "entry");
        let saved_bb = self.bld.get_insert_block();
        self.bld.position_at_end(entry);
        let fprintf_fn = self.module.get_function("fprintf").unwrap_or_else(|| {
            let ft2 = i32t.fn_type(&[ptr_ty.into(), ptr_ty.into()], true);
            self.module
                .add_function("fprintf", ft2, Some(Linkage::External))
        });
        let stderr_g = self.module.get_global("stderr").unwrap_or_else(|| {
            let g = self.module.add_global(ptr_ty, None, "stderr");
            g.set_linkage(Linkage::External);
            g
        });
        let stderr_val = self
            .bld
            .build_load(ptr_ty, stderr_g.as_pointer_value(), "se")
            .expect("load stderr");
        let fmt = self
            .bld
            .build_global_string_ptr("runtime error: %s\n", "trap.fmt")
            .expect("fmt string");
        let msg_param = f.get_nth_param(0).unwrap();
        self.bld
            .build_call(
                fprintf_fn,
                &[
                    stderr_val.into(),
                    fmt.as_pointer_value().into(),
                    msg_param.into(),
                ],
                "",
            )
            .expect("call fprintf");
        let abort_fn = self.module.get_function("abort").unwrap_or_else(|| {
            let ft3 = self.ctx.void_type().fn_type(&[], false);
            self.module
                .add_function("abort", ft3, Some(Linkage::External))
        });
        self.bld.build_call(abort_fn, &[], "").expect("call abort");
        self.bld.build_unreachable().expect("unreachable");
        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }
        f
    }

    fn emit_bounds_check(
        &mut self,
        idx: inkwell::values::IntValue<'ctx>,
        len: u64,
    ) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let len_val = idx.get_type().const_int(len, false);
        let oob = b!(self
            .bld
            .build_int_compare(IntPredicate::UGE, idx, len_val, "oob"));
        let trap_bb = self.ctx.append_basic_block(fv, "oob.trap");
        let ok_bb = self.ctx.append_basic_block(fv, "oob.ok");
        b!(self.bld.build_conditional_branch(oob, trap_bb, ok_bb));
        self.bld.position_at_end(trap_bb);
        self.emit_trap("index out of bounds");
        self.bld.position_at_end(ok_bb);
        Ok(())
    }

    pub(crate) fn compile_int_pow(
        &mut self,
        base: inkwell::values::IntValue<'ctx>,
        exp: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let result_ptr = self.entry_alloca(i64t.into(), "pow.res");
        let base_ptr = self.entry_alloca(i64t.into(), "pow.base");
        let exp_ptr = self.entry_alloca(i64t.into(), "pow.exp");
        b!(self.bld.build_store(result_ptr, i64t.const_int(1, false)));
        b!(self.bld.build_store(base_ptr, base));
        b!(self.bld.build_store(exp_ptr, exp));
        let cond_bb = self.ctx.append_basic_block(fv, "pow.cond");
        let body_bb = self.ctx.append_basic_block(fv, "pow.body");
        let sq_bb = self.ctx.append_basic_block(fv, "pow.sq");
        let end_bb = self.ctx.append_basic_block(fv, "pow.end");
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let e = b!(self.bld.build_load(i64t, exp_ptr, "e")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(
            IntPredicate::SGT,
            e,
            i64t.const_int(0, false),
            "pow.cmp"
        ));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));
        self.bld.position_at_end(body_bb);
        let e = b!(self.bld.build_load(i64t, exp_ptr, "e")).into_int_value();
        let odd = b!(self.bld.build_and(e, i64t.const_int(1, false), "odd"));
        let is_odd = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            odd,
            i64t.const_int(0, false),
            "isodd"
        ));
        let mul_bb = self.ctx.append_basic_block(fv, "pow.mul");
        b!(self.bld.build_conditional_branch(is_odd, mul_bb, sq_bb));
        self.bld.position_at_end(mul_bb);
        let r = b!(self.bld.build_load(i64t, result_ptr, "r")).into_int_value();
        let bv = b!(self.bld.build_load(i64t, base_ptr, "b")).into_int_value();
        let nr = b!(self.bld.build_int_nsw_mul(r, bv, "pow.m"));
        b!(self.bld.build_store(result_ptr, nr));
        b!(self.bld.build_unconditional_branch(sq_bb));
        self.bld.position_at_end(sq_bb);
        let bv = b!(self.bld.build_load(i64t, base_ptr, "b")).into_int_value();
        let nb = b!(self.bld.build_int_nsw_mul(bv, bv, "pow.sq"));
        b!(self.bld.build_store(base_ptr, nb));
        let e = b!(self.bld.build_load(i64t, exp_ptr, "e")).into_int_value();
        let ne = b!(self
            .bld
            .build_right_shift(e, i64t.const_int(1, false), false, "pow.shr"));
        b!(self.bld.build_store(exp_ptr, ne));
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(end_bb);
        Ok(b!(self.bld.build_load(i64t, result_ptr, "pow.result")))
    }

    fn compile_short_circuit(
        &mut self,
        left: &hir::Expr,
        right: &hir::Expr,
        is_and: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let lhs = self.compile_expr(left)?;
        let lbool = self.to_bool(lhs);
        let rhs_bb = self.ctx.append_basic_block(fv, "sc.rhs");
        let merge_bb = self.ctx.append_basic_block(fv, "sc.merge");
        let lhs_bb = self.bld.get_insert_block().unwrap();
        if is_and {
            b!(self.bld.build_conditional_branch(lbool, rhs_bb, merge_bb));
        } else {
            b!(self.bld.build_conditional_branch(lbool, merge_bb, rhs_bb));
        }
        self.bld.position_at_end(rhs_bb);
        let rhs = self.compile_expr(right)?;
        let rbool = self.to_bool(rhs);
        let rhs_end = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));
        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(self.ctx.bool_type(), "sc"));
        let short_val = self
            .ctx
            .bool_type()
            .const_int(if is_and { 0 } else { 1 }, false);
        phi.add_incoming(&[(&short_val, lhs_bb), (&rbool, rhs_end)]);
        Ok(phi.as_basic_value())
    }

    fn compile_unary(
        &mut self,
        op: UnaryOp,
        expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(expr)?;
        Ok(match op {
            UnaryOp::Neg => {
                if expr.ty.is_float() {
                    b!(self.bld.build_float_neg(val.into_float_value(), "fneg")).into()
                } else {
                    b!(self.bld.build_int_nsw_neg(val.into_int_value(), "neg")).into()
                }
            }
            UnaryOp::Not | UnaryOp::BitNot => {
                b!(self.bld.build_not(val.into_int_value(), "not")).into()
            }
        })
    }

    fn compile_ternary(
        &mut self,
        cond: &hir::Expr,
        then_e: &hir::Expr,
        else_e: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let tv = self.compile_expr(cond)?;
        let cv = self.to_bool(tv);
        let tbb = self.ctx.append_basic_block(fv, "t.then");
        let ebb = self.ctx.append_basic_block(fv, "t.else");
        let mbb = self.ctx.append_basic_block(fv, "t.merge");
        b!(self.bld.build_conditional_branch(cv, tbb, ebb));
        self.bld.position_at_end(tbb);
        let tv = self.compile_expr(then_e)?;
        let tbb_end = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(mbb));
        self.bld.position_at_end(ebb);
        let ev = self.compile_expr(else_e)?;
        let ebb_end = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(mbb));
        self.bld.position_at_end(mbb);
        let phi = b!(self.bld.build_phi(self.llvm_ty(&then_e.ty), "tern"));
        phi.add_incoming(&[(&tv, tbb_end), (&ev, ebb_end)]);
        Ok(phi.as_basic_value())
    }

    fn compile_cast(
        &mut self,
        expr: &hir::Expr,
        target: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(expr)?;
        let src = &expr.ty;
        let dst = self.llvm_ty(target);
        if src.is_int() && target.is_float() {
            return Ok(if src.is_signed() {
                b!(self.bld.build_signed_int_to_float(
                    val.into_int_value(),
                    dst.into_float_type(),
                    "sitofp"
                ))
                .into()
            } else {
                b!(self.bld.build_unsigned_int_to_float(
                    val.into_int_value(),
                    dst.into_float_type(),
                    "uitofp"
                ))
                .into()
            });
        }
        if src.is_float() && target.is_int() {
            return Ok(if target.is_signed() {
                b!(self.bld.build_float_to_signed_int(
                    val.into_float_value(),
                    dst.into_int_type(),
                    "fptosi"
                ))
                .into()
            } else {
                b!(self.bld.build_float_to_unsigned_int(
                    val.into_float_value(),
                    dst.into_int_type(),
                    "fptoui"
                ))
                .into()
            });
        }
        if src.is_int() && target.is_int() {
            let (sb, db) = (src.bits(), target.bits());
            return Ok(if sb < db {
                if src.is_signed() {
                    b!(self.bld.build_int_s_extend(
                        val.into_int_value(),
                        dst.into_int_type(),
                        "sext"
                    ))
                    .into()
                } else {
                    b!(self.bld.build_int_z_extend(
                        val.into_int_value(),
                        dst.into_int_type(),
                        "zext"
                    ))
                    .into()
                }
            } else if sb > db {
                b!(self
                    .bld
                    .build_int_truncate(val.into_int_value(), dst.into_int_type(), "trunc"))
                .into()
            } else {
                val
            });
        }
        if src.is_float() && target.is_float() {
            let (sb, db) = (src.bits(), target.bits());
            return Ok(if sb < db {
                b!(self
                    .bld
                    .build_float_ext(val.into_float_value(), dst.into_float_type(), "fpext"))
                .into()
            } else if sb > db {
                b!(self.bld.build_float_trunc(
                    val.into_float_value(),
                    dst.into_float_type(),
                    "fptrunc"
                ))
                .into()
            } else {
                val
            });
        }
        if matches!(src, Type::Bool) && target.is_int() {
            return Ok(b!(self.bld.build_int_z_extend(
                val.into_int_value(),
                dst.into_int_type(),
                "boolext"
            ))
            .into());
        }
        if matches!(src, Type::Ptr(_)) && target.is_int() {
            return Ok(b!(self.bld.build_ptr_to_int(
                val.into_pointer_value(),
                dst.into_int_type(),
                "ptrtoint"
            ))
            .into());
        }
        Err(format!("unsupported cast: {src} as {target}"))
    }

    pub(crate) fn compile_array(
        &mut self,
        elems: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if elems.is_empty() {
            return Err("empty array literal".into());
        }
        let elem_ty = &elems[0].ty;
        let lty = self.llvm_ty(elem_ty);
        let arr_ty = lty.array_type(elems.len() as u32);
        let ptr = self.entry_alloca(arr_ty.into(), "arr");
        for (i, e) in elems.iter().enumerate() {
            let val = self.compile_expr(e)?;
            let gep = unsafe {
                b!(self.bld.build_gep(
                    arr_ty,
                    ptr,
                    &[
                        self.ctx.i64_type().const_int(0, false),
                        self.ctx.i64_type().const_int(i as u64, false)
                    ],
                    "arr.gep"
                ))
            };
            b!(self.bld.build_store(gep, val));
        }
        Ok(ptr.into())
    }

    pub(crate) fn compile_tuple(
        &mut self,
        elems: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ltys: Vec<BasicTypeEnum<'ctx>> = elems.iter().map(|e| self.llvm_ty(&e.ty)).collect();
        let st = self.ctx.struct_type(&ltys, false);
        let ptr = self.entry_alloca(st.into(), "tup");
        for (i, e) in elems.iter().enumerate() {
            let val = self.compile_expr(e)?;
            let gep = b!(self.bld.build_struct_gep(st, ptr, i as u32, "tup.gep"));
            b!(self.bld.build_store(gep, val));
        }
        Ok(b!(self.bld.build_load(st, ptr, "tup")))
    }

    pub(crate) fn compile_struct(
        &mut self,
        name: &str,
        inits: &[hir::FieldInit],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fields = self
            .structs
            .get(name)
            .ok_or_else(|| format!("undefined type: {name}"))?
            .clone();
        let st = self
            .module
            .get_struct_type(name)
            .ok_or_else(|| format!("no LLVM struct: {name}"))?;
        let align = self.struct_layouts.get(name).and_then(|l| l.align);
        let ptr = if let Some(a) = align {
            self.entry_alloca_aligned(st.into(), name, a)
        } else {
            self.entry_alloca(st.into(), name)
        };
        for (i, (fname, fty)) in fields.iter().enumerate() {
            let val = inits
                .iter()
                .find(|fi| fi.name.as_deref() == Some(fname))
                .or_else(|| inits.get(i))
                .map(|fi| self.compile_expr(&fi.value))
                .transpose()?
                .unwrap_or_else(|| self.default_val(fty));
            let gep = b!(self.bld.build_struct_gep(st, ptr, i as u32, fname));
            b!(self.bld.build_store(gep, val));
        }
        Ok(b!(self.bld.build_load(st, ptr, name)))
    }

    pub(crate) fn compile_variant(
        &mut self,
        enum_name: &str,
        tag: u32,
        variant_name: &str,
        inits: &[hir::FieldInit],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let st = self
            .module
            .get_struct_type(enum_name)
            .ok_or_else(|| format!("no LLVM type: {enum_name}"))?;
        let variants = self
            .enums
            .get(enum_name)
            .cloned()
            .ok_or_else(|| format!("undefined enum: {enum_name}"))?;
        let (_, ftys) = variants
            .iter()
            .find(|(n, _)| n == variant_name)
            .ok_or_else(|| format!("no variant {variant_name}"))?;
        let ftys = ftys.clone();
        let ptr = self.entry_alloca(st.into(), variant_name);
        let tag_gep = b!(self.bld.build_struct_gep(st, ptr, 0, "tag"));
        b!(self
            .bld
            .build_store(tag_gep, self.ctx.i32_type().const_int(tag as u64, false)));
        if !ftys.is_empty() {
            let payload_gep = b!(self.bld.build_struct_gep(st, ptr, 1, "payload"));
            let mut offset = 0u64;
            for (i, fty) in ftys.iter().enumerate() {
                let val = inits
                    .get(i)
                    .map(|fi| self.compile_expr(&fi.value))
                    .transpose()?
                    .unwrap_or_else(|| self.default_val(fty));
                let is_rec = Self::is_recursive_field(fty, enum_name);
                let field_ptr = if offset == 0 {
                    payload_gep
                } else {
                    unsafe {
                        b!(self.bld.build_gep(
                            self.ctx.i8_type(),
                            payload_gep,
                            &[self.ctx.i64_type().const_int(offset, false)],
                            "fptr"
                        ))
                    }
                };
                if is_rec {
                    let actual_ty = self.llvm_ty(fty);
                    let size = self.type_store_size(actual_ty);
                    let malloc = self.ensure_malloc();
                    let heap = b!(self.bld.build_call(
                        malloc,
                        &[self.ctx.i64_type().const_int(size, false).into()],
                        "box.alloc"
                    ))
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_pointer_value();
                    b!(self.bld.build_store(heap, val));
                    b!(self.bld.build_store(field_ptr, heap));
                    offset += 8;
                } else {
                    let lty = self.llvm_ty(fty);
                    let coerced = self.coerce_val(val, lty);
                    b!(self.bld.build_store(field_ptr, coerced));
                    offset += self.type_store_size(lty);
                }
            }
        }
        Ok(b!(self.bld.build_load(st, ptr, variant_name)))
    }

    fn compile_field(
        &mut self,
        obj: &hir::Expr,
        field: &str,
        _hir_idx: usize,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_ty = &obj.ty;
        if matches!(obj_ty, Type::String) && field == "length" {
            let sv = self.compile_expr(obj)?;
            return self.string_len(sv);
        }
        if matches!(obj_ty, Type::Vec(_)) && field == "length" {
            let v = self.compile_expr(obj)?;
            return self.vec_len(v.into_pointer_value());
        }
        let (ty_name, is_ptr) = match obj_ty {
            Type::Struct(n) => (n.as_str(), false),
            Type::Ptr(inner) => match inner.as_ref() {
                Type::Struct(n) => (n.as_str(), true),
                other => return Err(format!("field access on non-struct: {other}")),
            },
            other => return Err(format!("field access on non-struct: {other}")),
        };
        let fields = self
            .structs
            .get(ty_name)
            .ok_or_else(|| format!("undefined type: {ty_name}"))?
            .clone();
        let idx = fields
            .iter()
            .position(|(n, _)| n == field)
            .ok_or_else(|| format!("no field '{field}' on {ty_name}"))?;
        let fty = fields[idx].1.clone();
        let st = self
            .module
            .get_struct_type(ty_name)
            .ok_or_else(|| format!("no LLVM struct: {ty_name}"))?;
        if let hir::ExprKind::Var(_, n) = &obj.kind {
            if let Some((ptr, _)) = self.find_var(n).cloned() {
                if is_ptr {
                    let struct_ptr = b!(self.bld.build_load(
                        self.ctx.ptr_type(inkwell::AddressSpace::default()),
                        ptr,
                        "self.ptr"
                    ))
                    .into_pointer_value();
                    let gep = b!(self.bld.build_struct_gep(st, struct_ptr, idx as u32, field));
                    return Ok(b!(self.bld.build_load(self.llvm_ty(&fty), gep, field)));
                }
                let gep = b!(self.bld.build_struct_gep(st, ptr, idx as u32, field));
                return Ok(b!(self.bld.build_load(self.llvm_ty(&fty), gep, field)));
            }
        }
        Err("cannot access field on rvalue".into())
    }

    fn compile_index(
        &mut self,
        arr: &hir::Expr,
        idx: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let arr_ty = &arr.ty;
        let idx_val = self.compile_expr(idx)?.into_int_value();
        match arr_ty {
            Type::Array(elem_ty, n) => {
                let lty = self.llvm_ty(elem_ty);
                let arr_llvm = lty.array_type(*n as u32);
                let arr_ptr = match &arr.kind {
                    hir::ExprKind::Var(_, name) => self
                        .find_var(name)
                        .map(|(ptr, _)| *ptr)
                        .ok_or_else(|| format!("undefined: {name}"))?,
                    _ => self.compile_expr(arr)?.into_pointer_value(),
                };
                let idx_val = self.wrap_negative_index(idx_val, *n as u64)?;
                self.emit_bounds_check(idx_val, *n as u64)?;
                let gep = unsafe {
                    b!(self.bld.build_gep(
                        arr_llvm,
                        arr_ptr,
                        &[self.ctx.i64_type().const_int(0, false), idx_val],
                        "idx"
                    ))
                };
                Ok(b!(self.bld.build_load(lty, gep, "elem")))
            }
            Type::Tuple(tys) => {
                let i = idx_val
                    .get_zero_extended_constant()
                    .ok_or("tuple index must be a constant")?;
                let fty = tys
                    .get(i as usize)
                    .ok_or_else(|| format!("tuple index {i} out of bounds"))?;
                let lty = self.llvm_ty(fty);
                if let hir::ExprKind::Var(_, name) = &arr.kind {
                    if let Some((ptr, _)) = self.find_var(name).cloned() {
                        let tup_ty = self.ctx.struct_type(
                            &tys.iter().map(|t| self.llvm_ty(t)).collect::<Vec<_>>(),
                            false,
                        );
                        let gep = b!(self.bld.build_struct_gep(tup_ty, ptr, i as u32, "tup.idx"));
                        return Ok(b!(self.bld.build_load(lty, gep, "tup.elem")));
                    }
                }
                Err("tuple indexing on rvalue not supported".into())
            }
            _ => {
                let arr_ptr = self.compile_expr(arr)?.into_pointer_value();
                let i64t = self.ctx.i64_type();
                let gep = unsafe { b!(self.bld.build_gep(i64t, arr_ptr, &[idx_val], "idx")) };
                Ok(b!(self.bld.build_load(i64t, gep, "elem")))
            }
        }
    }

    pub(crate) fn compile_str_literal(&mut self, s: &str) -> Result<BasicValueEnum<'ctx>, String> {
        if s.len() <= 23 {
            let st = self.string_type();
            let i8t = self.ctx.i8_type();
            let i64t = self.ctx.i64_type();
            let out = self.entry_alloca(st.into(), "slit");
            b!(self.bld.build_store(out, st.const_zero()));
            for (i, byte) in s.bytes().enumerate() {
                let bp = unsafe {
                    b!(self
                        .bld
                        .build_gep(i8t, out, &[i64t.const_int(i as u64, false)], "sso.b"))
                };
                b!(self.bld.build_store(bp, i8t.const_int(byte as u64, false)));
            }
            let tag_ptr = unsafe {
                b!(self
                    .bld
                    .build_gep(i8t, out, &[i64t.const_int(23, false)], "sso.tag"))
            };
            b!(self
                .bld
                .build_store(tag_ptr, i8t.const_int(0x80 | s.len() as u64, false)));
            Ok(b!(self.bld.build_load(st, out, "slit")))
        } else {
            let gstr = b!(self.bld.build_global_string_ptr(s, "str"));
            let i64t = self.ctx.i64_type();
            self.build_string(
                gstr.as_pointer_value(),
                i64t.const_int(s.len() as u64, false),
                i64t.const_int(0, false),
                "slit",
            )
        }
    }

    fn compile_ref(&mut self, inner: &hir::Expr) -> Result<BasicValueEnum<'ctx>, String> {
        match &inner.kind {
            hir::ExprKind::Var(_, name) => self
                .find_var(name)
                .map(|(ptr, _)| *ptr)
                .ok_or_else(|| format!("cannot take address of '{name}'"))
                .map(|p| p.into()),
            _ => Err("& requires a variable name".into()),
        }
    }

    fn compile_deref(&mut self, inner: &hir::Expr) -> Result<BasicValueEnum<'ctx>, String> {
        if let Type::Rc(ref elem_ty) = inner.ty {
            let rv = self.compile_expr(inner)?;
            return self.rc_deref(rv, elem_ty);
        }
        if let Type::Weak(_) = inner.ty {
            return Err("cannot deref a weak reference directly — use weak_upgrade() first".into());
        }
        let ptr_val = self.compile_expr(inner)?;
        Ok(b!(self.bld.build_load(
            self.ctx.i64_type(),
            ptr_val.into_pointer_value(),
            "deref"
        )))
    }

    fn compile_lambda(
        &mut self,
        params: &[hir::Param],
        body: &hir::Block,
        fn_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (ptys, ret_ty) = match fn_ty {
            Type::Fn(p, r) => (p.clone(), *r.clone()),
            _ => {
                let ptys: Vec<Type> = params.iter().map(|p| p.ty.clone()).collect();
                let ret_ty = Type::Void;
                (ptys, ret_ty)
            }
        };
        let lambda_name = format!("lambda.{}", self.module.get_functions().count());

        let mut body_ids = HashSet::new();
        Self::collect_var_refs_block(body, &mut body_ids);
        let param_names: HashSet<&str> = params.iter().map(|p| p.name.as_str()).collect();
        let mut cap_globals = Vec::new();
        for id in &body_ids {
            if param_names.contains(id.as_str())
                || self.fns.contains_key(id)
                || self.variant_tags.contains_key(id)
            {
                continue;
            }
            if let Some((ptr, ty)) = self.find_var(id).cloned() {
                let val = b!(self.bld.build_load(self.llvm_ty(&ty), ptr, id));
                let gname = format!("{}.cap.{}", lambda_name, id);
                let lt = self.llvm_ty(&ty);
                let g = self.module.add_global(lt, None, &gname);
                g.set_initializer(&self.default_val(&ty));
                g.set_thread_local(true);
                b!(self.bld.build_store(g.as_pointer_value(), val));
                cap_globals.push((id.clone(), g.as_pointer_value(), ty));
            }
        }
        let lp: Vec<BasicMetadataTypeEnum<'ctx>> =
            ptys.iter().map(|t| self.llvm_ty(t).into()).collect();
        let ft = self.mk_fn_type(&ret_ty, &lp, false);
        let lambda_fv = self.module.add_function(&lambda_name, ft, None);
        lambda_fv.add_attribute(
            inkwell::attributes::AttributeLoc::Function,
            self.attr("nounwind"),
        );
        lambda_fv.set_linkage(Linkage::Internal);
        self.fns.insert(
            lambda_name.clone(),
            (lambda_fv, ptys.clone(), ret_ty.clone()),
        );
        let saved_fn = self.cur_fn;
        let saved_bb = self.bld.get_insert_block();
        self.cur_fn = Some(lambda_fv);
        let entry = self.ctx.append_basic_block(lambda_fv, "entry");
        self.bld.position_at_end(entry);
        self.vars.push(HashMap::new());
        for (name, gptr, ty) in &cap_globals {
            let lt = self.llvm_ty(ty);
            let val = b!(self.bld.build_load(lt, *gptr, name));
            let a = self.entry_alloca(lt, name);
            b!(self.bld.build_store(a, val));
            self.set_var(name, a, ty.clone());
        }
        for (i, p) in params.iter().enumerate() {
            let ty = &ptys[i];
            let a = self.entry_alloca(self.llvm_ty(ty), &p.name);
            b!(self
                .bld
                .build_store(a, lambda_fv.get_nth_param(i as u32).unwrap()));
            self.set_var(&p.name, a, ty.clone());
        }
        let last = self.compile_block(body)?;
        if self.no_term() {
            match &ret_ty {
                Type::Void => {
                    b!(self.bld.build_return(None));
                }
                _ => {
                    let rty = self.llvm_ty(&ret_ty);
                    let v = match last {
                        Some(v) if v.get_type() == rty => v,
                        _ => self.default_val(&ret_ty),
                    };
                    b!(self.bld.build_return(Some(&v)));
                }
            }
        }
        self.vars.pop();
        self.cur_fn = saved_fn;
        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }
        Ok(lambda_fv.as_global_value().as_pointer_value().into())
    }

    pub(crate) fn collect_var_refs_block(block: &hir::Block, out: &mut HashSet<String>) {
        for stmt in block {
            match stmt {
                hir::Stmt::Expr(e) | hir::Stmt::Bind(hir::Bind { value: e, .. }) => {
                    Self::collect_var_refs_expr(e, out);
                }
                hir::Stmt::TupleBind(_, v, _) => Self::collect_var_refs_expr(v, out),
                hir::Stmt::Assign(t, v, _) => {
                    Self::collect_var_refs_expr(t, out);
                    Self::collect_var_refs_expr(v, out);
                }
                hir::Stmt::Ret(Some(e), _, _) | hir::Stmt::Break(Some(e), _) => {
                    Self::collect_var_refs_expr(e, out);
                }
                hir::Stmt::If(i) => {
                    Self::collect_var_refs_expr(&i.cond, out);
                    Self::collect_var_refs_block(&i.then, out);
                    for (c, b) in &i.elifs {
                        Self::collect_var_refs_expr(c, out);
                        Self::collect_var_refs_block(b, out);
                    }
                    if let Some(b) = &i.els {
                        Self::collect_var_refs_block(b, out);
                    }
                }
                hir::Stmt::While(w) => {
                    Self::collect_var_refs_expr(&w.cond, out);
                    Self::collect_var_refs_block(&w.body, out);
                }
                hir::Stmt::For(f) => {
                    Self::collect_var_refs_expr(&f.iter, out);
                    Self::collect_var_refs_block(&f.body, out);
                }
                hir::Stmt::Loop(l) => Self::collect_var_refs_block(&l.body, out),
                hir::Stmt::Match(m) => {
                    Self::collect_var_refs_expr(&m.subject, out);
                    for arm in &m.arms {
                        if let Some(ref g) = arm.guard {
                            Self::collect_var_refs_expr(g, out);
                        }
                        Self::collect_var_refs_block(&arm.body, out);
                    }
                }
                hir::Stmt::ErrReturn(e, _, _) => Self::collect_var_refs_expr(e, out),
                _ => {}
            }
        }
    }

    fn collect_var_refs_expr(e: &hir::Expr, out: &mut HashSet<String>) {
        match &e.kind {
            hir::ExprKind::Var(_, n) => {
                out.insert(n.clone());
            }
            hir::ExprKind::BinOp(l, _, r) => {
                Self::collect_var_refs_expr(l, out);
                Self::collect_var_refs_expr(r, out);
            }
            hir::ExprKind::UnaryOp(_, e)
            | hir::ExprKind::Coerce(e, _)
            | hir::ExprKind::Cast(e, _)
            | hir::ExprKind::Ref(e)
            | hir::ExprKind::Deref(e) => Self::collect_var_refs_expr(e, out),
            hir::ExprKind::Call(_, _, args)
            | hir::ExprKind::Builtin(_, args)
            | hir::ExprKind::Syscall(args) => {
                for a in args {
                    Self::collect_var_refs_expr(a, out);
                }
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                Self::collect_var_refs_expr(callee, out);
                for a in args {
                    Self::collect_var_refs_expr(a, out);
                }
            }
            hir::ExprKind::Method(obj, _, _, args)
            | hir::ExprKind::StringMethod(obj, _, args)
            | hir::ExprKind::VecMethod(obj, _, args)
            | hir::ExprKind::MapMethod(obj, _, args) => {
                Self::collect_var_refs_expr(obj, out);
                for a in args {
                    Self::collect_var_refs_expr(a, out);
                }
            }
            hir::ExprKind::Field(e, _, _) => Self::collect_var_refs_expr(e, out),
            hir::ExprKind::Index(a, b) => {
                Self::collect_var_refs_expr(a, out);
                Self::collect_var_refs_expr(b, out);
            }
            hir::ExprKind::Ternary(c, t, f) => {
                Self::collect_var_refs_expr(c, out);
                Self::collect_var_refs_expr(t, out);
                Self::collect_var_refs_expr(f, out);
            }
            hir::ExprKind::Array(es) | hir::ExprKind::Tuple(es) | hir::ExprKind::VecNew(es) => {
                for e in es {
                    Self::collect_var_refs_expr(e, out);
                }
            }
            hir::ExprKind::Struct(_, inits) | hir::ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    Self::collect_var_refs_expr(&fi.value, out);
                }
            }
            hir::ExprKind::IfExpr(i) => {
                Self::collect_var_refs_expr(&i.cond, out);
                Self::collect_var_refs_block(&i.then, out);
                for (c, b) in &i.elifs {
                    Self::collect_var_refs_expr(c, out);
                    Self::collect_var_refs_block(b, out);
                }
                if let Some(b) = &i.els {
                    Self::collect_var_refs_block(b, out);
                }
            }
            hir::ExprKind::Block(b) => Self::collect_var_refs_block(b, out),
            hir::ExprKind::Lambda(_, body) => Self::collect_var_refs_block(body, out),
            hir::ExprKind::Pipe(left, _, _, extra) => {
                Self::collect_var_refs_expr(left, out);
                for a in extra {
                    Self::collect_var_refs_expr(a, out);
                }
            }
            hir::ExprKind::ListComp(body, _, _, iter, end, cond) => {
                Self::collect_var_refs_expr(body, out);
                Self::collect_var_refs_expr(iter, out);
                if let Some(e) = end {
                    Self::collect_var_refs_expr(e, out);
                }
                if let Some(c) = cond {
                    Self::collect_var_refs_expr(c, out);
                }
            }
            _ => {}
        }
    }

    fn compile_syscall(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("syscall requires at least 1 argument (syscall number)".into());
        }
        let i64t = self.ctx.i64_type();
        let mut vals: Vec<BasicValueEnum<'ctx>> = Vec::new();
        for arg in args {
            vals.push(self.compile_expr(arg)?);
        }
        let nargs = vals.len();
        let (template, constraints) = match nargs {
            1 => ("syscall", "={rax},{rax},~{rcx},~{r11},~{memory}"),
            2 => ("syscall", "={rax},{rax},{rdi},~{rcx},~{r11},~{memory}"),
            3 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},~{rcx},~{r11},~{memory}",
            ),
            4 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},~{rcx},~{r11},~{memory}",
            ),
            5 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},~{rcx},~{r11},~{memory}",
            ),
            6 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},{r8},~{rcx},~{r11},~{memory}",
            ),
            7 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},{r8},{r9},~{rcx},~{r11},~{memory}",
            ),
            _ => return Err("syscall supports 0-6 arguments".into()),
        };
        let input_types: Vec<BasicMetadataTypeEnum<'ctx>> =
            vals.iter().map(|_| i64t.into()).collect();
        let ft = i64t.fn_type(&input_types, false);
        let inline_asm = self.ctx.create_inline_asm(
            ft,
            template.to_string(),
            constraints.to_string(),
            true,
            false,
            None,
            false,
        );
        let args_meta: Vec<BasicMetadataValueEnum<'ctx>> =
            vals.iter().map(|v| (*v).into()).collect();
        let result = b!(self
            .bld
            .build_indirect_call(ft, inline_asm, &args_meta, "syscall"));
        Ok(result
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| i64t.const_int(0, false).into()))
    }

    fn compile_list_comp(
        &mut self,
        body: &hir::Expr,
        bind: &str,
        start: &hir::Expr,
        end: Option<&hir::Expr>,
        cond: Option<&hir::Expr>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let end_expr = end.ok_or("list comprehension requires 'to' end bound")?;
        let i64t = self.ctx.i64_type();
        let start_val = self.compile_expr(start)?.into_int_value();
        let end_val = self.compile_expr(end_expr)?.into_int_value();
        let elem_ty = i64t;
        let range = b!(self.bld.build_int_sub(end_val, start_val, "comp.range"));
        let zero = i64t.const_int(0, false);
        let is_pos = b!(self
            .bld
            .build_int_compare(IntPredicate::SGT, range, zero, "comp.pos"));
        let safe_range = b!(self.bld.build_select(is_pos, range, zero, "comp.sz")).into_int_value();
        let elem_size = i64t.const_int(8, false);
        let alloc_size = b!(self.bld.build_int_mul(safe_range, elem_size, "comp.bytes"));
        let malloc_fn = self.ensure_malloc();
        let arr_ptr = b!(self
            .bld
            .build_call(malloc_fn, &[alloc_size.into()], "comp_arr"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();
        let fv = self.cur_fn.unwrap();
        let loop_bb = self.ctx.append_basic_block(fv, "comp_loop");
        let body_bb = self.ctx.append_basic_block(fv, "comp_body");
        let skip_bb = if cond.is_some() {
            Some(self.ctx.append_basic_block(fv, "comp_skip"))
        } else {
            None
        };
        let done_bb = self.ctx.append_basic_block(fv, "comp_done");
        let idx_ptr = self.entry_alloca(i64t.into(), "comp_idx");
        let cnt_ptr = self.entry_alloca(i64t.into(), "comp_cnt");
        b!(self.bld.build_store(idx_ptr, start_val));
        b!(self.bld.build_store(cnt_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(loop_bb);
        let cur_idx = b!(self.bld.build_load(i64t, idx_ptr, "idx")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, cur_idx, end_val, "cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));
        self.bld.position_at_end(body_bb);
        self.vars.push(HashMap::new());
        let bind_alloca = self.entry_alloca(i64t.into(), bind);
        b!(self.bld.build_store(bind_alloca, cur_idx));
        self.set_var(bind, bind_alloca, Type::I64);
        if let Some(cond_expr) = cond {
            let store_bb = self.ctx.append_basic_block(fv, "comp_store");
            let cond_val = self.compile_expr(cond_expr)?;
            let cbool = self.to_bool(cond_val);
            b!(self
                .bld
                .build_conditional_branch(cbool, store_bb, skip_bb.unwrap()));
            self.bld.position_at_end(store_bb);
        }
        let val = self.compile_expr(body)?;
        let cur_cnt = b!(self.bld.build_load(i64t, cnt_ptr, "cnt")).into_int_value();
        let elem_ptr = unsafe { b!(self.bld.build_gep(elem_ty, arr_ptr, &[cur_cnt], "elem")) };
        b!(self.bld.build_store(elem_ptr, val));
        let next_cnt = b!(self
            .bld
            .build_int_add(cur_cnt, i64t.const_int(1, false), "ncnt"));
        b!(self.bld.build_store(cnt_ptr, next_cnt));
        self.vars.pop();
        let next_idx = b!(self
            .bld
            .build_int_add(cur_idx, i64t.const_int(1, false), "nidx"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        if let Some(skip) = skip_bb {
            b!(self.bld.build_unconditional_branch(loop_bb));
            self.bld.position_at_end(skip);
            let cur_idx2 = b!(self.bld.build_load(i64t, idx_ptr, "idx2")).into_int_value();
            let next_idx2 = b!(self
                .bld
                .build_int_add(cur_idx2, i64t.const_int(1, false), "nidx2"));
            b!(self.bld.build_store(idx_ptr, next_idx2));
            b!(self.bld.build_unconditional_branch(loop_bb));
        } else {
            b!(self.bld.build_unconditional_branch(loop_bb));
        }
        self.bld.position_at_end(done_bb);
        Ok(arr_ptr.into())
    }

    fn compile_dyn_coerce(
        &mut self,
        inner: &hir::Expr,
        type_name: &str,
        trait_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(inner)?;
        let ptr = self.ctx.ptr_type(inkwell::AddressSpace::default());

        let data_ptr = if val.is_pointer_value() {
            val.into_pointer_value()
        } else {
            let lty = val.get_type();
            let alloc = self.entry_alloca(lty, "dyn.data");
            b!(self.bld.build_store(alloc, val));
            alloc
        };

        let vtable_ptr = self
            .vtables
            .get(&(type_name.to_string(), trait_name.to_string()))
            .map(|gv| gv.as_pointer_value())
            .unwrap_or_else(|| ptr.const_null());

        let fat_ty = self.ctx.struct_type(&[ptr.into(), ptr.into()], false);
        let fat = fat_ty.const_zero();
        let fat = b!(self
            .bld
            .build_insert_value(fat, data_ptr, 0, "dyn.fat.data"))
        .into_struct_value();
        let fat = b!(self
            .bld
            .build_insert_value(fat, vtable_ptr, 1, "dyn.fat.vtable"))
        .into_struct_value();
        Ok(fat.into())
    }

    fn compile_dyn_dispatch(
        &mut self,
        obj: &hir::Expr,
        trait_name: &str,
        method: &str,
        args: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fat = self.compile_expr(obj)?;
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let fat_ty = self.ctx.struct_type(&[ptr_ty.into(), ptr_ty.into()], false);

        let tmp = self.entry_alloca(fat_ty.into(), "dyn.tmp");
        b!(self.bld.build_store(tmp, fat));
        let data_gep = b!(self.bld.build_struct_gep(fat_ty, tmp, 0, "dyn.data.gep"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dyn.data")).into_pointer_value();
        let vtable_gep = b!(self.bld.build_struct_gep(fat_ty, tmp, 1, "dyn.vtable.gep"));
        let vtable_ptr =
            b!(self.bld.build_load(ptr_ty, vtable_gep, "dyn.vtable")).into_pointer_value();

        let method_idx = self
            .trait_method_order
            .get(trait_name)
            .and_then(|methods| methods.iter().position(|m| m == method))
            .unwrap_or(0) as u64;

        let fn_ptr_gep = unsafe {
            b!(self.bld.build_gep(
                ptr_ty,
                vtable_ptr,
                &[self.ctx.i64_type().const_int(method_idx, false)],
                "dyn.fn.gep"
            ))
        };
        let fn_ptr = b!(self.bld.build_load(ptr_ty, fn_ptr_gep, "dyn.fn")).into_pointer_value();

        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            vec![data_ptr.into()];
        for arg in args {
            let av = self.compile_expr(arg)?;
            call_args.push(av.into());
        }

        let ret_ty = self.llvm_ty(result_ty);
        let mut param_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
        for arg in args {
            param_tys.push(self.llvm_ty(&arg.ty).into());
        }
        let fn_ty = ret_ty.fn_type(&param_tys, false);
        let result = b!(self
            .bld
            .build_indirect_call(fn_ty, fn_ptr, &call_args, "dyn.call"));
        Ok(result
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| self.ctx.i64_type().const_int(0, false).into()))
    }

    fn compile_iter_next_by_name(
        &mut self,
        var_name: &str,
        type_name: &str,
        method_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self
            .find_var(var_name)
            .ok_or_else(|| format!("undefined iter variable: {var_name}"))?
            .0;

        let fn_name = format!("{type_name}_{method_name}");
        let fv = self
            .module
            .get_function(&fn_name)
            .ok_or_else(|| format!("no function {fn_name}"))?;
        let result = b!(self.bld.build_call(fv, &[ptr.into()], "iter.next"));
        Ok(result
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| self.ctx.i64_type().const_int(0, false).into()))
    }
}
