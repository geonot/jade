use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, FloatPredicate, IntPredicate};

use crate::ast::BinOp;
use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_binop(
        &mut self,
        left: &hir::Expr,
        op: BinOp,
        right: &hir::Expr,
        _result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // NDArray element-wise operations (broadcasting)
        if let Type::NDArray(elem_ty, dims) = &left.ty {
            return self.compile_ndarray_elementwise(left, op, right, elem_ty, dims);
        }
        // SIMD vector operations — LLVM vector ops work directly
        if let Type::SIMD(_, _) = &left.ty {
            let lhs = self.compile_expr(left)?;
            let rhs = self.compile_expr(right)?;
            let lv = lhs.into_vector_value();
            let rv = rhs.into_vector_value();
            let result = match op {
                BinOp::Add => b!(self.bld.build_float_add(lv, rv, "simd.add")).into(),
                BinOp::Sub => b!(self.bld.build_float_sub(lv, rv, "simd.sub")).into(),
                BinOp::Mul => b!(self.bld.build_float_mul(lv, rv, "simd.mul")).into(),
                BinOp::Div => b!(self.bld.build_float_div(lv, rv, "simd.div")).into(),
                _ => return Err(format!("unsupported SIMD binop: {op:?}")),
            };
            self.tag_fast_math(result);
            return Ok(result);
        }
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
            if let Type::Struct(name, _) = ety {
                let fn_name = format!("{name}_equal");
                if let Some((fv, _, _)) = self.fns.get(&fn_name).cloned() {
                    let first_param_is_ptr = fv.get_type().get_param_types().first().map(|t| t.is_pointer_type()).unwrap_or(false);
                    let self_arg: BasicValueEnum = if first_param_is_ptr {
                        let tmp = self.entry_alloca(self.llvm_ty(ety), "eq.self");
                        b!(self.bld.build_store(tmp, lhs));
                        tmp.into()
                    } else {
                        lhs
                    };
                    let result = b!(self
                        .bld
                        .build_call(fv, &[self_arg.into(), rhs.into()], "eq.call"))
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
        if let Type::Struct(name, _) = ety {
            let trait_name = match op {
                BinOp::Add => Some("add"),
                BinOp::Sub => Some("sub"),
                BinOp::Mul => Some("mul"),
                BinOp::Div => Some("div"),
                BinOp::Lt => Some("less"),
                BinOp::Gt => Some("greater"),
                BinOp::Le => Some("less_eq"),
                BinOp::Ge => Some("greater_eq"),
                _ => None,
            };
            if let Some(method) = trait_name {
                let fn_name = format!("{name}_{method}");
                if let Some((fv, _, _)) = self.fns.get(&fn_name).cloned() {
                    let first_param_is_ptr = fv.get_type().get_param_types().first().map(|t| t.is_pointer_type()).unwrap_or(false);
                    let self_arg: BasicValueEnum = if first_param_is_ptr {
                        let tmp = self.entry_alloca(self.llvm_ty(ety), "op.self");
                        b!(self.bld.build_store(tmp, lhs));
                        tmp.into()
                    } else {
                        lhs
                    };
                    let result = b!(self.bld.build_call(
                        fv,
                        &[self_arg.into(), rhs.into()],
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
        let result: BasicValueEnum = match op {
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
        };
        self.tag_fast_math(result);
        Ok(result)
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
            BinOp::Div if s => self.checked_divmod(l, r, true, true)?,
            BinOp::Div => self.checked_divmod(l, r, false, true)?,
            BinOp::Mod if s => self.checked_divmod(l, r, true, false)?,
            BinOp::Mod => self.checked_divmod(l, r, false, false)?,
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

    fn checked_divmod(
        &mut self,
        l: inkwell::values::IntValue<'ctx>,
        r: inkwell::values::IntValue<'ctx>,
        signed: bool,
        is_div: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let prefix = if is_div { "div" } else { "rem" };
        let zero = r.get_type().const_int(0, false);
        let is_zero =
            b!(self
                .bld
                .build_int_compare(IntPredicate::EQ, r, zero, &format!("{prefix}.z")));
        let trap_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.trap"));
        let ok_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.ok"));
        b!(self.bld.build_conditional_branch(is_zero, trap_bb, ok_bb));
        self.bld.position_at_end(trap_bb);
        self.emit_trap("division by zero");
        self.bld.position_at_end(ok_bb);
        Ok(match (is_div, signed) {
            (true, true) => b!(self.bld.build_int_signed_div(l, r, "sdiv")).into(),
            (true, false) => b!(self.bld.build_int_unsigned_div(l, r, "udiv")).into(),
            (false, true) => b!(self.bld.build_int_signed_rem(l, r, "srem")).into(),
            (false, false) => b!(self.bld.build_int_unsigned_rem(l, r, "urem")).into(),
        })
    }

    pub(crate) fn emit_trap(&mut self, msg: &str) {
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

    pub(crate) fn emit_bounds_check(
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

    pub(crate) fn compile_ndarray_elementwise(
        &mut self,
        left: &hir::Expr,
        op: BinOp,
        right: &hir::Expr,
        _elem_ty: &Type,
        dims: &[usize],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let f64t = self.ctx.f64_type();
        let malloc = self.ensure_malloc();

        let lptr = self.compile_expr(left)?.into_pointer_value();
        let rptr = self.compile_expr(right)?.into_pointer_value();

        // Total elements = product of dims
        let total: u64 = dims.iter().map(|&d| d as u64).product();
        let total_v = i64t.const_int(total, false);

        // Allocate result array
        let elem_size = i64t.const_int(8, false);
        let byte_size = b!(self.bld.build_int_mul(total_v, elem_size, "bcast.bytes"));
        let result_ptr = b!(self
            .bld
            .build_call(malloc, &[byte_size.into()], "bcast.result"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        // Loop over elements
        let fn_val = self.cur_fn.unwrap();
        let loop_bb = self.ctx.append_basic_block(fn_val, "bcast.loop");
        let body_bb = self.ctx.append_basic_block(fn_val, "bcast.body");
        let end_bb = self.ctx.append_basic_block(fn_val, "bcast.end");

        let idx_ptr = self.entry_alloca(i64t.into(), "bcast.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_zero()));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "bcast.i")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::ULT,
            idx,
            total_v,
            "bcast.cmp"
        ));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));

        self.bld.position_at_end(body_bb);
        let lep = unsafe { b!(self.bld.build_gep(f64t, lptr, &[idx.into()], "bcast.lep")) };
        let rep = unsafe { b!(self.bld.build_gep(f64t, rptr, &[idx.into()], "bcast.rep")) };
        let lv = b!(self.bld.build_load(f64t, lep, "bcast.lv")).into_float_value();
        let rv = b!(self.bld.build_load(f64t, rep, "bcast.rv")).into_float_value();

        let result_elem = match op {
            BinOp::Add => b!(self.bld.build_float_add(lv, rv, "bcast.add")),
            BinOp::Sub => b!(self.bld.build_float_sub(lv, rv, "bcast.sub")),
            BinOp::Mul => b!(self.bld.build_float_mul(lv, rv, "bcast.mul")),
            BinOp::Div => b!(self.bld.build_float_div(lv, rv, "bcast.div")),
            _ => b!(self.bld.build_float_add(lv, rv, "bcast.fallback")),
        };

        let oep = unsafe { b!(self.bld.build_gep(f64t, result_ptr, &[idx.into()], "bcast.oep")) };
        b!(self.bld.build_store(oep, result_elem));

        let next = b!(self.bld.build_int_add(idx, i64t.const_int(1, false), "bcast.next"));
        b!(self.bld.build_store(idx_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(end_bb);
        Ok(result_ptr.into())
    }
}
