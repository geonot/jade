use inkwell::module::Linkage;
use inkwell::values::{BasicValue, BasicValueEnum};
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_builtin(
        &mut self,
        builtin: &hir::BuiltinFn,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match builtin {
            hir::BuiltinFn::ActorSpawn | hir::BuiltinFn::ActorSend => {
                Err("actor builtins are dispatched via ExprKind, not BuiltinFn".into())
            }
            hir::BuiltinFn::Log => self.compile_log(args),
            hir::BuiltinFn::ToString => {
                if args.len() != 1 {
                    return Err("to_string() takes 1 argument".into());
                }
                self.compile_to_string(&args[0])
            }
            hir::BuiltinFn::RcAlloc => {
                if args.len() != 1 {
                    return Err("rc() takes 1 argument".into());
                }
                let val = self.compile_expr(&args[0])?;
                self.rc_alloc(&args[0].ty, val)
            }
            hir::BuiltinFn::RcRetain => {
                if args.len() != 1 {
                    return Err("rc_retain() takes 1 argument".into());
                }
                let val = self.compile_expr(&args[0])?;
                if let Type::Rc(inner) = &args[0].ty {
                    self.rc_retain(val, inner)?;
                    Ok(val)
                } else {
                    Err("rc_retain: argument must be Rc type".into())
                }
            }
            hir::BuiltinFn::RcRelease => {
                if args.len() != 1 {
                    return Err("rc_release() takes 1 argument".into());
                }
                let val = self.compile_expr(&args[0])?;
                if let Type::Rc(inner) = &args[0].ty {
                    self.rc_release(val, inner)?;
                    Ok(self.ctx.i64_type().const_int(0, false).into())
                } else {
                    Err("rc_release: argument must be Rc type".into())
                }
            }
            hir::BuiltinFn::WeakDowngrade => {
                if args.len() != 1 {
                    return Err("weak() takes 1 argument (an rc value)".into());
                }
                let val = self.compile_expr(&args[0])?;
                if let Type::Rc(inner) = &args[0].ty {
                    self.weak_downgrade(val, inner)
                } else {
                    Err("weak(): argument must be rc type".into())
                }
            }
            hir::BuiltinFn::WeakUpgrade => {
                if args.len() != 1 {
                    return Err("weak_upgrade() takes 1 argument".into());
                }
                let val = self.compile_expr(&args[0])?;
                if let Type::Weak(inner) = &args[0].ty {
                    self.weak_upgrade(val, inner)
                } else {
                    Err("weak_upgrade(): argument must be weak type".into())
                }
            }
            hir::BuiltinFn::WeakAlloc => {
                Err("weak_alloc is internal — use weak() on an rc value".into())
            }
            hir::BuiltinFn::VolatileLoad => {
                if args.len() != 1 {
                    return Err("volatile_load() takes 1 argument".into());
                }
                let ptr_val = self.compile_expr(&args[0])?;
                if let Type::Ptr(inner) = &args[0].ty {
                    self.compile_volatile_load(ptr_val, inner)
                } else {
                    Err("volatile_load(): argument must be a pointer".into())
                }
            }
            hir::BuiltinFn::VolatileStore => {
                if args.len() != 2 {
                    return Err("volatile_store() takes 2 arguments (ptr, value)".into());
                }
                let ptr_val = self.compile_expr(&args[0])?;
                let val = self.compile_expr(&args[1])?;
                if let Type::Ptr(inner) = &args[0].ty {
                    self.compile_volatile_store(ptr_val, val, inner)
                } else {
                    Err("volatile_store(): first argument must be a pointer".into())
                }
            }
            hir::BuiltinFn::WrappingAdd
            | hir::BuiltinFn::WrappingSub
            | hir::BuiltinFn::WrappingMul => {
                if args.len() != 2 {
                    return Err("wrapping op takes 2 arguments".into());
                }
                self.compile_wrapping_op(builtin, args)
            }
            hir::BuiltinFn::SaturatingAdd
            | hir::BuiltinFn::SaturatingSub
            | hir::BuiltinFn::SaturatingMul => {
                if args.len() != 2 {
                    return Err("saturating op takes 2 arguments".into());
                }
                self.compile_saturating_op(builtin, args)
            }
            hir::BuiltinFn::CheckedAdd
            | hir::BuiltinFn::CheckedSub
            | hir::BuiltinFn::CheckedMul => {
                if args.len() != 2 {
                    return Err("checked op takes 2 arguments".into());
                }
                self.compile_checked_op(builtin, args)
            }
            hir::BuiltinFn::SignalHandle => {
                if args.len() != 2 {
                    return Err("signal_handle() takes 2 arguments (signum, handler)".into());
                }
                self.compile_signal_handle(args)
            }
            hir::BuiltinFn::SignalRaise => {
                if args.len() != 1 {
                    return Err("signal_raise() takes 1 argument".into());
                }
                self.compile_signal_raise(args)
            }
            hir::BuiltinFn::SignalIgnore => {
                if args.len() != 1 {
                    return Err("signal_ignore() takes 1 argument".into());
                }
                self.compile_signal_ignore(args)
            }
            hir::BuiltinFn::Popcount
            | hir::BuiltinFn::Clz
            | hir::BuiltinFn::Ctz
            | hir::BuiltinFn::RotateLeft
            | hir::BuiltinFn::RotateRight
            | hir::BuiltinFn::Bswap => self.compile_bit_intrinsic(builtin, args),
            hir::BuiltinFn::Assert => {
                if args.is_empty() {
                    return Err("assert requires a condition".into());
                }
                self.compile_assert(args)
            }
            hir::BuiltinFn::Ln => self.compile_f64_intrinsic("llvm.log.f64", args),
            hir::BuiltinFn::Log2 => self.compile_f64_intrinsic("llvm.log2.f64", args),
            hir::BuiltinFn::Log10 => self.compile_f64_intrinsic("llvm.log10.f64", args),
            hir::BuiltinFn::Exp => self.compile_f64_intrinsic("llvm.exp.f64", args),
            hir::BuiltinFn::Exp2 => self.compile_f64_intrinsic("llvm.exp2.f64", args),
            hir::BuiltinFn::PowF => self.compile_f64_intrinsic("llvm.pow.f64", args),
            hir::BuiltinFn::Copysign => self.compile_f64_intrinsic("llvm.copysign.f64", args),
            hir::BuiltinFn::Fma => self.compile_f64_intrinsic("llvm.fma.f64", args),
            hir::BuiltinFn::StringFromRaw => self.compile_string_from_raw(args),
            hir::BuiltinFn::StringFromPtr => self.compile_string_from_ptr(args),
            hir::BuiltinFn::GetArgs => self.compile_get_args(),
            hir::BuiltinFn::FmtFloat => self.compile_fmt_float(args),
            hir::BuiltinFn::FmtHex => self.compile_fmt_snprintf("%lx", args),
            hir::BuiltinFn::FmtOct => self.compile_fmt_snprintf("%lo", args),
            hir::BuiltinFn::FmtBin => self.compile_fmt_bin(args),
            hir::BuiltinFn::TimeMonotonic => self.compile_time_monotonic(),
            hir::BuiltinFn::SleepMs => self.compile_sleep_ms(args),
            hir::BuiltinFn::FileExists => self.compile_file_exists(args),
            hir::BuiltinFn::ArenaNew => self.compile_arena_new(args),
            hir::BuiltinFn::ArenaAlloc => self.compile_arena_alloc(args),
            hir::BuiltinFn::ArenaReset => self.compile_arena_reset(args),
            hir::BuiltinFn::AtomicLoad
            | hir::BuiltinFn::AtomicStore
            | hir::BuiltinFn::AtomicAdd
            | hir::BuiltinFn::AtomicSub
            | hir::BuiltinFn::AtomicCas
            | hir::BuiltinFn::CompTimeTypeOf
            | hir::BuiltinFn::CompTimeFieldsOf
            | hir::BuiltinFn::CompTimeSizeOf => {
                Err(format!("builtin {:?} should not appear in codegen", builtin))
            }
            hir::BuiltinFn::CharMethod(method) => {
                self.compile_char_method(method, args)
            }
            hir::BuiltinFn::Matmul => {
                self.compile_matmul(args)
            }
            hir::BuiltinFn::RegexMatch | hir::BuiltinFn::RegexFindAll => {
                Err(format!("builtin {:?} should be lowered to string methods", builtin))
            }
            hir::BuiltinFn::ConstantTimeEq => {
                self.compile_constant_time_eq(args)
            }
            hir::BuiltinFn::VecWithAlloc | hir::BuiltinFn::MapWithAlloc => {
                // Allocator-aware collection: allocate {ptr, len, cap, alloc_ptr}
                // For now, create the collection normally and store the allocator ref
                let _alloc = self.compile_expr(&args[0])?;
                let ptr_t = self.ctx.ptr_type(AddressSpace::default());
                let i64t = self.ctx.i64_type();
                let malloc = self.ensure_malloc();
                let size = i64t.const_int(32, false); // 32 bytes: {ptr, len, cap, alloc}
                let ptr = b!(self.bld.build_call(malloc, &[size.into()], "alloc_col"))
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_pointer_value();
                let memset = self.module.get_function("llvm.memset.p0.i64").unwrap_or_else(|| {
                    self.module.add_function(
                        "llvm.memset.p0.i64",
                        self.ctx.void_type().fn_type(
                            &[ptr_t.into(), self.ctx.i8_type().into(), i64t.into(), self.ctx.bool_type().into()],
                            false,
                        ),
                        None,
                    )
                });
                b!(self.bld.build_call(
                    memset,
                    &[ptr.into(), self.ctx.i8_type().const_zero().into(), size.into(), self.ctx.bool_type().const_zero().into()],
                    "",
                ));
                Ok(ptr.into())
            }
            hir::BuiltinFn::DequeNew => {
                Err("DequeNew should be handled via ExprKind".into())
            }
            hir::BuiltinFn::GradFn => {
                Err("GradFn not yet implemented in codegen".into())
            }
            hir::BuiltinFn::Einsum => {
                Err("Einsum should be handled via ExprKind".into())
            }
            hir::BuiltinFn::CowWrap => {
                Err("CowWrap should be handled via ExprKind".into())
            }
            hir::BuiltinFn::Likely | hir::BuiltinFn::Unlikely => {
                if args.len() != 1 {
                    return Err("likely/unlikely takes exactly 1 boolean argument".into());
                }
                let cond = self.compile_expr(&args[0])?;
                let i1ty = self.ctx.bool_type();
                let ft = i1ty.fn_type(&[i1ty.into(), i1ty.into()], false);
                let expect_fn = self.module.get_function("llvm.expect.i1")
                    .unwrap_or_else(|| self.module.add_function("llvm.expect.i1", ft, None));
                let expected = match builtin {
                    hir::BuiltinFn::Likely => i1ty.const_int(1, false),
                    _ => i1ty.const_int(0, false),
                };
                let result = b!(self.bld.build_call(expect_fn, &[cond.into(), expected.into()], "expect"));
                Ok(result.try_as_basic_value().basic().unwrap())
            }
            hir::BuiltinFn::PoolNew => self.compile_pool_new(args),
            hir::BuiltinFn::PoolAlloc => self.compile_pool_alloc(args),
            hir::BuiltinFn::PoolFree => self.compile_pool_free(args),
            hir::BuiltinFn::PoolDestroy => self.compile_pool_destroy(args),
            hir::BuiltinFn::FloatMethod(method) => self.compile_float_method(method, args),
        }
    }

    fn compile_constant_time_eq(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Constant-time equality comparison using XOR accumulation
        // For strings: XOR each byte, accumulate result, compare to zero
        // For integers: single XOR + compare to zero
        let a = self.compile_expr(&args[0])?;
        let b = self.compile_expr(&args[1])?;

        if args[0].ty == Type::String {
            // Call runtime stub for string constant-time comparison
            let ptr_t = self.ctx.ptr_type(AddressSpace::default());
            let bool_t = self.ctx.bool_type();
            let fn_type = bool_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
            let func = self
                .module
                .get_function("__jade_constant_time_eq")
                .unwrap_or_else(|| self.module.add_function("__jade_constant_time_eq", fn_type, None));
            let result = b!(self.bld.build_call(func, &[a.into(), b.into()], "ct.eq"));
            Ok(result.try_as_basic_value().basic().unwrap())
        } else {
            // Integer constant-time: XOR then compare to zero
            let i64t = self.ctx.i64_type();
            let av = a.into_int_value();
            let bv = b.into_int_value();
            let xor = b!(self.bld.build_xor(av, bv, "ct.xor"));
            let is_zero = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                xor,
                i64t.const_int(0, false),
                "ct.iszero"
            ));
            Ok(is_zero.into())
        }
    }

    fn compile_matmul(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // matmul(A, B) — both are NDArray pointers of f64
        // For 2D: result[i][j] = sum_k A[i][k] * B[k][j]
        // Gets dims from the type. Falls back to runtime stub.
        let a_ptr = self.compile_expr(&args[0])?.into_pointer_value();
        let b_ptr = self.compile_expr(&args[1])?.into_pointer_value();

        // Extract dimensions from types
        let (m, k1) = match &args[0].ty {
            Type::NDArray(_, dims) if dims.len() == 2 => (dims[0], dims[1]),
            _ => (0, 0),
        };
        let (k2, n) = match &args[1].ty {
            Type::NDArray(_, dims) if dims.len() == 2 => (dims[0], dims[1]),
            _ => (0, 0),
        };

        if m == 0 || k1 == 0 || k1 != k2 || n == 0 {
            // Fallback: call runtime stub
            let rt_fn = self.module.get_function("__jade_matmul").unwrap_or_else(|| {
                let i64t = self.ctx.i64_type();
                let ptr_t = self.ctx.ptr_type(inkwell::AddressSpace::default());
                let ft = ptr_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64t.into(), i64t.into(), i64t.into()], false);
                self.module.add_function("__jade_matmul", ft, None)
            });
            let i64t = self.ctx.i64_type();
            let result = b!(self.bld.build_call(
                rt_fn,
                &[a_ptr.into(), b_ptr.into(), i64t.const_int(0, false).into(), i64t.const_int(0, false).into(), i64t.const_int(0, false).into()],
                "matmul.rt"
            )).try_as_basic_value().basic().unwrap();
            return Ok(result);
        }

        let i64t = self.ctx.i64_type();
        let f64t = self.ctx.f64_type();
        let malloc = self.ensure_malloc();

        // Allocate result: m * n * 8
        let total = (m * n) as u64;
        let byte_size = i64t.const_int(total * 8, false);
        let result_ptr = b!(self
            .bld
            .build_call(malloc, &[byte_size.into()], "mm.result"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        // Zero-initialize result
        let memset = self.module.get_function("llvm.memset.p0.i64").unwrap_or_else(|| {
            let ptr_t = self.ctx.ptr_type(inkwell::AddressSpace::default());
            self.module.add_function(
                "llvm.memset.p0.i64",
                self.ctx.void_type().fn_type(
                    &[ptr_t.into(), self.ctx.i8_type().into(), i64t.into(), self.ctx.bool_type().into()],
                    false,
                ),
                None,
            )
        });
        b!(self.bld.build_call(
            memset,
            &[
                result_ptr.into(),
                self.ctx.i8_type().const_zero().into(),
                byte_size.into(),
                self.ctx.bool_type().const_zero().into(),
            ],
            "",
        ));

        // Triple nested loop: i=0..m, j=0..n, k=0..k1
        // result[i*n+j] += A[i*k1+k] * B[k*n+j]
        let k_val = k1 as u64;
        let n_val = n as u64;

        let fn_val = self.cur_fn.unwrap();
        let outer_bb = self.ctx.append_basic_block(fn_val, "mm.i");
        let mid_bb = self.ctx.append_basic_block(fn_val, "mm.j");
        let inner_bb = self.ctx.append_basic_block(fn_val, "mm.k");
        let inner_body = self.ctx.append_basic_block(fn_val, "mm.body");
        let inner_end = self.ctx.append_basic_block(fn_val, "mm.k.end");
        let mid_end = self.ctx.append_basic_block(fn_val, "mm.j.end");
        let outer_end = self.ctx.append_basic_block(fn_val, "mm.end");

        let i_ptr = self.entry_alloca(i64t.into(), "mm.i");
        let j_ptr = self.entry_alloca(i64t.into(), "mm.j");
        let k_ptr = self.entry_alloca(i64t.into(), "mm.k");

        b!(self.bld.build_store(i_ptr, i64t.const_zero()));
        b!(self.bld.build_unconditional_branch(outer_bb));

        // Outer loop: i
        self.bld.position_at_end(outer_bb);
        let i = b!(self.bld.build_load(i64t, i_ptr, "i")).into_int_value();
        let i_cmp = b!(self.bld.build_int_compare(inkwell::IntPredicate::ULT, i, i64t.const_int(m as u64, false), "i.cmp"));
        b!(self.bld.build_conditional_branch(i_cmp, mid_bb, outer_end));

        // Mid loop: j
        self.bld.position_at_end(mid_bb);
        b!(self.bld.build_store(j_ptr, i64t.const_zero()));
        b!(self.bld.build_unconditional_branch(inner_bb));

        self.bld.position_at_end(inner_bb);
        let j = b!(self.bld.build_load(i64t, j_ptr, "j")).into_int_value();
        let j_cmp = b!(self.bld.build_int_compare(inkwell::IntPredicate::ULT, j, i64t.const_int(n_val, false), "j.cmp"));
        b!(self.bld.build_conditional_branch(j_cmp, inner_body, mid_end));

        // Inner loop body: k
        self.bld.position_at_end(inner_body);
        b!(self.bld.build_store(k_ptr, i64t.const_zero()));

        let k_loop = self.ctx.append_basic_block(fn_val, "mm.kloop");
        let k_body = self.ctx.append_basic_block(fn_val, "mm.kbody");
        let k_end = self.ctx.append_basic_block(fn_val, "mm.kend");
        b!(self.bld.build_unconditional_branch(k_loop));

        self.bld.position_at_end(k_loop);
        let k = b!(self.bld.build_load(i64t, k_ptr, "k")).into_int_value();
        let k_cmp = b!(self.bld.build_int_compare(inkwell::IntPredicate::ULT, k, i64t.const_int(k_val, false), "k.cmp"));
        b!(self.bld.build_conditional_branch(k_cmp, k_body, k_end));

        self.bld.position_at_end(k_body);
        let i2 = b!(self.bld.build_load(i64t, i_ptr, "i2")).into_int_value();
        let j2 = b!(self.bld.build_load(i64t, j_ptr, "j2")).into_int_value();
        // A[i*k1+k]
        let a_idx = b!(self.bld.build_int_mul(i2, i64t.const_int(k_val, false), "a.row"));
        let a_idx = b!(self.bld.build_int_add(a_idx, k, "a.idx"));
        let a_ep = unsafe { b!(self.bld.build_gep(f64t, a_ptr, &[a_idx.into()], "a.ep")) };
        let a_val = b!(self.bld.build_load(f64t, a_ep, "a.val")).into_float_value();
        // B[k*n+j]
        let b_idx = b!(self.bld.build_int_mul(k, i64t.const_int(n_val, false), "b.row"));
        let b_idx = b!(self.bld.build_int_add(b_idx, j2, "b.idx"));
        let b_ep = unsafe { b!(self.bld.build_gep(f64t, b_ptr, &[b_idx.into()], "b.ep")) };
        let b_val = b!(self.bld.build_load(f64t, b_ep, "b.val")).into_float_value();
        // result[i*n+j] += a * b
        let r_idx = b!(self.bld.build_int_mul(i2, i64t.const_int(n_val, false), "r.row"));
        let r_idx = b!(self.bld.build_int_add(r_idx, j2, "r.idx"));
        let r_ep = unsafe { b!(self.bld.build_gep(f64t, result_ptr, &[r_idx.into()], "r.ep")) };
        let r_val = b!(self.bld.build_load(f64t, r_ep, "r.cur")).into_float_value();
        let prod = b!(self.bld.build_float_mul(a_val, b_val, "mm.prod"));
        let sum = b!(self.bld.build_float_add(r_val, prod, "mm.sum"));
        b!(self.bld.build_store(r_ep, sum));

        let k_next = b!(self.bld.build_int_add(k, i64t.const_int(1, false), "k.next"));
        b!(self.bld.build_store(k_ptr, k_next));
        b!(self.bld.build_unconditional_branch(k_loop));

        self.bld.position_at_end(k_end);
        let j_next = b!(self.bld.build_int_add(j, i64t.const_int(1, false), "j.next"));
        b!(self.bld.build_store(j_ptr, j_next));
        b!(self.bld.build_unconditional_branch(inner_bb));

        self.bld.position_at_end(mid_end);
        let i_next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "i.next"));
        b!(self.bld.build_store(i_ptr, i_next));
        b!(self.bld.build_unconditional_branch(outer_bb));

        self.bld.position_at_end(outer_end);
        Ok(result_ptr.into())
    }

    /// Einsum codegen: parse notation, dispatch to known patterns or generic loop nest
    pub(crate) fn compile_einsum(
        &mut self,
        notation: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Parse notation: "ij,jk->ik" → inputs=["ij","jk"], output="ik"
        let parts: Vec<&str> = notation.split("->").collect();
        let (inp_str, out_str) = match parts.as_slice() {
            [inp, out] => (*inp, *out),
            _ => return Err(format!("invalid einsum notation: {notation}")),
        };
        let inputs: Vec<&str> = inp_str.split(',').collect();

        // Matmul: ij,jk->ik
        if inputs.len() == 2 && args.len() == 2 {
            if inputs[0] == "ij" && inputs[1] == "jk" && out_str == "ik" {
                return self.compile_matmul(args);
            }
            // Dot product: i,i->
            if inputs[0] == "i" && inputs[1] == "i" && out_str.is_empty() {
                return self.compile_einsum_dot(args);
            }
        }
        // Trace: ii->
        if inputs.len() == 1 && args.len() == 1 && inputs[0] == "ii" && out_str.is_empty() {
            return self.compile_einsum_trace(args);
        }
        // Transpose: ij->ji
        if inputs.len() == 1 && args.len() == 1 && inputs[0] == "ij" && out_str == "ji" {
            return self.compile_einsum_transpose(args);
        }
        Err(format!("unsupported einsum pattern: {notation}"))
    }

    fn compile_einsum_dot(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let a_ptr = self.compile_expr(&args[0])?.into_pointer_value();
        let b_ptr = self.compile_expr(&args[1])?.into_pointer_value();
        let n = match &args[0].ty {
            Type::NDArray(_, dims) if dims.len() == 1 => dims[0] as u64,
            _ => return Err("einsum dot requires 1D NDArrays".into()),
        };
        let i64t = self.ctx.i64_type();
        let f64t = self.ctx.f64_type();
        let fn_val = self.cur_fn.unwrap();

        let acc = self.entry_alloca(f64t.into(), "dot.acc");
        b!(self.bld.build_store(acc, f64t.const_float(0.0)));
        let iv = self.entry_alloca(i64t.into(), "dot.i");
        b!(self.bld.build_store(iv, i64t.const_zero()));

        let loop_bb = self.ctx.append_basic_block(fn_val, "dot.loop");
        let body_bb = self.ctx.append_basic_block(fn_val, "dot.body");
        let end_bb = self.ctx.append_basic_block(fn_val, "dot.end");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let i = b!(self.bld.build_load(i64t, iv, "i")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(IntPredicate::ULT, i, i64t.const_int(n, false), "dot.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));

        self.bld.position_at_end(body_bb);
        let a_ep = unsafe { b!(self.bld.build_gep(f64t, a_ptr, &[i], "a.ep")) };
        let a_val = b!(self.bld.build_load(f64t, a_ep, "a.v")).into_float_value();
        let b_ep = unsafe { b!(self.bld.build_gep(f64t, b_ptr, &[i], "b.ep")) };
        let b_val = b!(self.bld.build_load(f64t, b_ep, "b.v")).into_float_value();
        let prod = b!(self.bld.build_float_mul(a_val, b_val, "dot.prod"));
        let cur = b!(self.bld.build_load(f64t, acc, "dot.cur")).into_float_value();
        let sum = b!(self.bld.build_float_add(cur, prod, "dot.sum"));
        b!(self.bld.build_store(acc, sum));
        let i_next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "i.next"));
        b!(self.bld.build_store(iv, i_next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(end_bb);
        Ok(b!(self.bld.build_load(f64t, acc, "dot.result")))
    }

    fn compile_einsum_trace(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let a_ptr = self.compile_expr(&args[0])?.into_pointer_value();
        let n = match &args[0].ty {
            Type::NDArray(_, dims) if dims.len() == 2 && dims[0] == dims[1] => dims[0] as u64,
            _ => return Err("einsum trace requires square 2D NDArray".into()),
        };
        let i64t = self.ctx.i64_type();
        let f64t = self.ctx.f64_type();
        let fn_val = self.cur_fn.unwrap();

        let acc = self.entry_alloca(f64t.into(), "tr.acc");
        b!(self.bld.build_store(acc, f64t.const_float(0.0)));
        let iv = self.entry_alloca(i64t.into(), "tr.i");
        b!(self.bld.build_store(iv, i64t.const_zero()));

        let loop_bb = self.ctx.append_basic_block(fn_val, "tr.loop");
        let body_bb = self.ctx.append_basic_block(fn_val, "tr.body");
        let end_bb = self.ctx.append_basic_block(fn_val, "tr.end");
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let i = b!(self.bld.build_load(i64t, iv, "i")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(IntPredicate::ULT, i, i64t.const_int(n, false), "tr.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));

        self.bld.position_at_end(body_bb);
        // A[i*n + i]
        let idx = b!(self.bld.build_int_mul(i, i64t.const_int(n, false), "tr.row"));
        let idx = b!(self.bld.build_int_add(idx, i, "tr.diag"));
        let ep = unsafe { b!(self.bld.build_gep(f64t, a_ptr, &[idx], "tr.ep")) };
        let val = b!(self.bld.build_load(f64t, ep, "tr.v")).into_float_value();
        let cur = b!(self.bld.build_load(f64t, acc, "tr.cur")).into_float_value();
        let sum = b!(self.bld.build_float_add(cur, val, "tr.sum"));
        b!(self.bld.build_store(acc, sum));
        let i_next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "i.next"));
        b!(self.bld.build_store(iv, i_next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(end_bb);
        Ok(b!(self.bld.build_load(f64t, acc, "tr.result")))
    }

    fn compile_einsum_transpose(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let a_ptr = self.compile_expr(&args[0])?.into_pointer_value();
        let (m, n) = match &args[0].ty {
            Type::NDArray(_, dims) if dims.len() == 2 => (dims[0] as u64, dims[1] as u64),
            _ => return Err("einsum transpose requires 2D NDArray".into()),
        };
        let i64t = self.ctx.i64_type();
        let f64t = self.ctx.f64_type();
        let malloc = self.ensure_malloc();

        let total = m * n;
        let byte_size = i64t.const_int(total * 8, false);
        let result_ptr = b!(self.bld.build_call(malloc, &[byte_size.into()], "tp.ptr"))
            .try_as_basic_value().basic().unwrap().into_pointer_value();

        let fn_val = self.cur_fn.unwrap();
        let iv = self.entry_alloca(i64t.into(), "tp.i");
        let jv = self.entry_alloca(i64t.into(), "tp.j");
        b!(self.bld.build_store(iv, i64t.const_zero()));

        let i_loop = self.ctx.append_basic_block(fn_val, "tp.i");
        let j_loop = self.ctx.append_basic_block(fn_val, "tp.j");
        let body = self.ctx.append_basic_block(fn_val, "tp.body");
        let j_end = self.ctx.append_basic_block(fn_val, "tp.j.end");
        let i_end = self.ctx.append_basic_block(fn_val, "tp.end");
        b!(self.bld.build_unconditional_branch(i_loop));

        self.bld.position_at_end(i_loop);
        let i = b!(self.bld.build_load(i64t, iv, "i")).into_int_value();
        let cmp_i = b!(self.bld.build_int_compare(IntPredicate::ULT, i, i64t.const_int(m, false), "tp.icmp"));
        b!(self.bld.build_conditional_branch(cmp_i, j_loop, i_end));

        self.bld.position_at_end(j_loop);
        b!(self.bld.build_store(jv, i64t.const_zero()));
        let j_loop2 = self.ctx.append_basic_block(fn_val, "tp.j2");
        b!(self.bld.build_unconditional_branch(j_loop2));

        self.bld.position_at_end(j_loop2);
        let j = b!(self.bld.build_load(i64t, jv, "j")).into_int_value();
        let cmp_j = b!(self.bld.build_int_compare(IntPredicate::ULT, j, i64t.const_int(n, false), "tp.jcmp"));
        b!(self.bld.build_conditional_branch(cmp_j, body, j_end));

        self.bld.position_at_end(body);
        // src[i*n + j] -> dst[j*m + i]
        let src_idx = b!(self.bld.build_int_mul(i, i64t.const_int(n, false), "s.row"));
        let src_idx = b!(self.bld.build_int_add(src_idx, j, "s.idx"));
        let src_ep = unsafe { b!(self.bld.build_gep(f64t, a_ptr, &[src_idx], "s.ep")) };
        let val = b!(self.bld.build_load(f64t, src_ep, "s.v")).into_float_value();
        let dst_idx = b!(self.bld.build_int_mul(j, i64t.const_int(m, false), "d.row"));
        let dst_idx = b!(self.bld.build_int_add(dst_idx, i, "d.idx"));
        let dst_ep = unsafe { b!(self.bld.build_gep(f64t, result_ptr, &[dst_idx], "d.ep")) };
        b!(self.bld.build_store(dst_ep, val));

        let j_next = b!(self.bld.build_int_add(j, i64t.const_int(1, false), "j.next"));
        b!(self.bld.build_store(jv, j_next));
        b!(self.bld.build_unconditional_branch(j_loop2));

        self.bld.position_at_end(j_end);
        let i_next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "i.next"));
        b!(self.bld.build_store(iv, i_next));
        b!(self.bld.build_unconditional_branch(i_loop));

        self.bld.position_at_end(i_end);
        Ok(result_ptr.into())
    }

    fn compile_bit_intrinsic(
        &mut self,
        builtin: &hir::BuiltinFn,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let intrinsic = match builtin {
            hir::BuiltinFn::Popcount => "popcount",
            hir::BuiltinFn::Clz => "clz",
            hir::BuiltinFn::Ctz => "ctz",
            hir::BuiltinFn::RotateLeft => "rotate_left",
            hir::BuiltinFn::RotateRight => "rotate_right",
            hir::BuiltinFn::Bswap => "bswap",
            _ => return Err("not a bit intrinsic".into()),
        };
        if args.is_empty() {
            return Err(format!("{intrinsic}() requires at least one argument"));
        }
        let val = self.compile_expr(&args[0])?;
        let int_val = val.into_int_value();
        let bw = int_val.get_type().get_bit_width();
        let llvm_name = match intrinsic {
            "popcount" => format!("llvm.ctpop.i{bw}"),
            "clz" => format!("llvm.ctlz.i{bw}"),
            "ctz" => format!("llvm.cttz.i{bw}"),
            "rotate_left" => format!("llvm.fshl.i{bw}"),
            "rotate_right" => format!("llvm.fshr.i{bw}"),
            "bswap" => format!("llvm.bswap.i{bw}"),
            _ => unreachable!(),
        };
        let it = int_val.get_type();
        match intrinsic {
            "popcount" | "bswap" => {
                let ft = it.fn_type(&[it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                Ok(b!(self.bld.build_call(f, &[int_val.into()], intrinsic))
                    .try_as_basic_value()
                    .basic()
                    .unwrap())
            }
            "clz" | "ctz" => {
                let false_val = self.ctx.bool_type().const_int(0, false);
                let ft = it.fn_type(&[it.into(), self.ctx.bool_type().into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                Ok(b!(self
                    .bld
                    .build_call(f, &[int_val.into(), false_val.into()], intrinsic))
                .try_as_basic_value()
                .basic()
                .unwrap())
            }
            "rotate_left" | "rotate_right" => {
                if args.len() < 2 {
                    return Err(format!("{intrinsic}() requires two arguments"));
                }
                let amt = self.compile_expr(&args[1])?.into_int_value();
                let ft = it.fn_type(&[it.into(), it.into(), it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                Ok(b!(self.bld.build_call(
                    f,
                    &[int_val.into(), int_val.into(), amt.into()],
                    intrinsic
                ))
                .try_as_basic_value()
                .basic()
                .unwrap())
            }
            _ => Err(format!("unknown intrinsic: {intrinsic}")),
        }
    }

    fn compile_volatile_load(
        &mut self,
        ptr_val: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lty = self.llvm_ty(inner);
        let ptr = ptr_val.into_pointer_value();
        let load = b!(self.bld.build_load(lty, ptr, "vol.load"));
        load.as_instruction_value()
            .unwrap()
            .set_volatile(true)
            .unwrap();
        Ok(load)
    }

    fn compile_volatile_store(
        &mut self,
        ptr_val: BasicValueEnum<'ctx>,
        val: BasicValueEnum<'ctx>,
        _inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = ptr_val.into_pointer_value();
        let store = b!(self.bld.build_store(ptr, val));
        store.set_volatile(true).unwrap();
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    fn compile_wrapping_op(
        &mut self,
        builtin: &hir::BuiltinFn,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lhs = self.compile_expr(&args[0])?.into_int_value();
        let rhs = self.compile_expr(&args[1])?.into_int_value();
        Ok(match builtin {
            hir::BuiltinFn::WrappingAdd => b!(self.bld.build_int_add(lhs, rhs, "wrap.add")).into(),
            hir::BuiltinFn::WrappingSub => b!(self.bld.build_int_sub(lhs, rhs, "wrap.sub")).into(),
            hir::BuiltinFn::WrappingMul => b!(self.bld.build_int_mul(lhs, rhs, "wrap.mul")).into(),
            _ => unreachable!(),
        })
    }

    fn compile_saturating_op(
        &mut self,
        builtin: &hir::BuiltinFn,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lhs = self.compile_expr(&args[0])?.into_int_value();
        let rhs = self.compile_expr(&args[1])?.into_int_value();
        let bw = lhs.get_type().get_bit_width();
        let signed = args[0].ty.is_signed();
        let (intrinsic_name, it) = match builtin {
            hir::BuiltinFn::SaturatingAdd if signed => {
                (format!("llvm.sadd.sat.i{bw}"), lhs.get_type())
            }
            hir::BuiltinFn::SaturatingAdd => (format!("llvm.uadd.sat.i{bw}"), lhs.get_type()),
            hir::BuiltinFn::SaturatingSub if signed => {
                (format!("llvm.ssub.sat.i{bw}"), lhs.get_type())
            }
            hir::BuiltinFn::SaturatingSub => (format!("llvm.usub.sat.i{bw}"), lhs.get_type()),
            hir::BuiltinFn::SaturatingMul => {
                return self.compile_saturating_mul(lhs, rhs, signed);
            }
            _ => unreachable!(),
        };
        let ft = it.fn_type(&[it.into(), it.into()], false);
        let f = self
            .module
            .get_function(&intrinsic_name)
            .unwrap_or_else(|| self.module.add_function(&intrinsic_name, ft, None));
        Ok(b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "sat"))
            .try_as_basic_value()
            .basic()
            .unwrap())
    }

    fn compile_saturating_mul(
        &mut self,
        lhs: inkwell::values::IntValue<'ctx>,
        rhs: inkwell::values::IntValue<'ctx>,
        signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let bw = lhs.get_type().get_bit_width();
        let it = lhs.get_type();
        let intrinsic = if signed {
            format!("llvm.smul.with.overflow.i{bw}")
        } else {
            format!("llvm.umul.with.overflow.i{bw}")
        };
        let overflow_ty = self
            .ctx
            .struct_type(&[it.into(), self.ctx.bool_type().into()], false);
        let ft = overflow_ty.fn_type(&[it.into(), it.into()], false);
        let f = self
            .module
            .get_function(&intrinsic)
            .unwrap_or_else(|| self.module.add_function(&intrinsic, ft, None));
        let result = b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "smul"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        let val = b!(self
            .bld
            .build_extract_value(result.into_struct_value(), 0, "mul.val"))
        .into_int_value();
        let overflowed = b!(self
            .bld
            .build_extract_value(result.into_struct_value(), 1, "mul.of"))
        .into_int_value();

        let max_val = if signed {
            it.const_int((1u64 << (bw - 1)) - 1, false)
        } else {
            it.const_all_ones()
        };
        let clamped: BasicValueEnum = b!(self.bld.build_select::<BasicValueEnum, _>(
            overflowed,
            max_val.into(),
            val.into(),
            "sat.mul"
        ));

        let _ = fv;
        Ok(clamped)
    }

    fn compile_checked_op(
        &mut self,
        builtin: &hir::BuiltinFn,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lhs = self.compile_expr(&args[0])?.into_int_value();
        let rhs = self.compile_expr(&args[1])?.into_int_value();
        let bw = lhs.get_type().get_bit_width();
        let it = lhs.get_type();
        let signed = args[0].ty.is_signed();
        let intrinsic = match builtin {
            hir::BuiltinFn::CheckedAdd if signed => format!("llvm.sadd.with.overflow.i{bw}"),
            hir::BuiltinFn::CheckedAdd => format!("llvm.uadd.with.overflow.i{bw}"),
            hir::BuiltinFn::CheckedSub if signed => format!("llvm.ssub.with.overflow.i{bw}"),
            hir::BuiltinFn::CheckedSub => format!("llvm.usub.with.overflow.i{bw}"),
            hir::BuiltinFn::CheckedMul if signed => format!("llvm.smul.with.overflow.i{bw}"),
            hir::BuiltinFn::CheckedMul => format!("llvm.umul.with.overflow.i{bw}"),
            _ => unreachable!(),
        };
        let overflow_ty = self
            .ctx
            .struct_type(&[it.into(), self.ctx.bool_type().into()], false);
        let ft = overflow_ty.fn_type(&[it.into(), it.into()], false);
        let f = self
            .module
            .get_function(&intrinsic)
            .unwrap_or_else(|| self.module.add_function(&intrinsic, ft, None));
        let result = b!(self.bld.build_call(f, &[lhs.into(), rhs.into()], "chk"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        Ok(result)
    }

    fn ensure_signal(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("signal").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i32t = self.ctx.i32_type();
            let ft = ptr_ty.fn_type(&[i32t.into(), ptr_ty.into()], false);
            self.module
                .add_function("signal", ft, Some(inkwell::module::Linkage::External))
        })
    }

    fn ensure_raise(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("raise").unwrap_or_else(|| {
            let i32t = self.ctx.i32_type();
            let ft = i32t.fn_type(&[i32t.into()], false);
            self.module
                .add_function("raise", ft, Some(inkwell::module::Linkage::External))
        })
    }

    fn compile_signal_handle(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let signum = self.compile_expr(&args[0])?;
        let handler = self.compile_expr(&args[1])?;
        let signal_fn = self.ensure_signal();
        let sig32 = b!(self.bld.build_int_truncate(signum.into_int_value(), self.ctx.i32_type(), "sig.trunc"));
        b!(self
            .bld
            .build_call(signal_fn, &[sig32.into(), handler.into()], "sig"));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    fn compile_signal_raise(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let signum = self.compile_expr(&args[0])?;
        let raise_fn = self.ensure_raise();
        let sig32 = b!(self.bld.build_int_truncate(signum.into_int_value(), self.ctx.i32_type(), "sig.trunc"));
        Ok(b!(self.bld.build_call(raise_fn, &[sig32.into()], "raise"))
            .try_as_basic_value()
            .basic()
            .unwrap())
    }

    fn compile_signal_ignore(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let signum = self.compile_expr(&args[0])?;
        let signal_fn = self.ensure_signal();
        let sig32 = b!(self.bld.build_int_truncate(signum.into_int_value(), self.ctx.i32_type(), "sig.trunc"));
        let sig_ign = self
            .bld
            .build_int_to_ptr(
                self.ctx.i64_type().const_int(1, false),
                self.ctx.ptr_type(AddressSpace::default()),
                "sig.ign",
            )
            .unwrap();
        b!(self
            .bld
            .build_call(signal_fn, &[sig32.into(), sig_ign.into()], "sig.ign"));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    fn compile_assert(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let cond_expr = &args[0];
        let fv = self.cur_fn.unwrap();
        let cond_val = self.compile_expr(cond_expr)?;
        let cond = self.to_bool(cond_val);

        let pass_bb = self.ctx.append_basic_block(fv, "assert.pass");
        let fail_bb = self.ctx.append_basic_block(fv, "assert.fail");
        b!(self.bld.build_conditional_branch(cond, pass_bb, fail_bb));

        self.bld.position_at_end(fail_bb);
        let printf = self.module.get_function("printf").unwrap();
        let line = cond_expr.span.line;
        // Use the descriptive message if provided
        let desc = if args.len() > 1 {
            if let hir::ExprKind::Str(ref s) = args[1].kind {
                s.clone()
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        let msg = if desc.is_empty() {
            format!("assertion failed at line {line}\n\0")
        } else {
            format!("assertion failed: {desc} (line {line})\n\0")
        };
        let gs = b!(self.bld.build_global_string_ptr(&msg, "assert.msg"));
        b!(self
            .bld
            .build_call(printf, &[gs.as_pointer_value().into()], ""));
        let exit_fn = self.module.get_function("exit").unwrap_or_else(|| {
            let i32t = self.ctx.i32_type();
            self.module.add_function(
                "exit",
                self.ctx.void_type().fn_type(&[i32t.into()], false),
                Some(Linkage::External),
            )
        });
        b!(self.bld.build_call(
            exit_fn,
            &[self.ctx.i32_type().const_int(1, false).into()],
            ""
        ));
        b!(self.bld.build_unreachable());

        self.bld.position_at_end(pass_bb);
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    fn compile_f64_intrinsic(
        &mut self,
        name: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let f64t = self.ctx.f64_type();
        let param_types: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> =
            (0..args.len()).map(|_| f64t.into()).collect();
        let f = self.module.get_function(name).unwrap_or_else(|| {
            self.module
                .add_function(name, f64t.fn_type(&param_types, false), None)
        });
        let compiled: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = args
            .iter()
            .map(|a| Ok(self.compile_expr(a)?.into_float_value().into()))
            .collect::<Result<_, String>>()?;
        Ok(b!(self.bld.build_call(f, &compiled, ""))
            .try_as_basic_value()
            .basic()
            .unwrap())
    }

    fn compile_float_method(
        &mut self,
        method: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // args[0] is the receiver (the f64 value), rest are method arguments
        let receiver = self.compile_expr(&args[0])?.into_float_value();
        let f64t = self.ctx.f64_type();
        let i64t = self.ctx.i64_type();

        match method {
            // Single-argument LLVM intrinsics
            "sqrt" => {
                let f = self.module.get_function("llvm.sqrt.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.sqrt.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "sqrt")).try_as_basic_value().basic().unwrap())
            }
            "abs" => {
                let f = self.module.get_function("llvm.fabs.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.fabs.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "abs")).try_as_basic_value().basic().unwrap())
            }
            "floor" => {
                let f = self.module.get_function("llvm.floor.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.floor.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "floor")).try_as_basic_value().basic().unwrap())
            }
            "ceil" => {
                let f = self.module.get_function("llvm.ceil.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.ceil.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "ceil")).try_as_basic_value().basic().unwrap())
            }
            "round" => {
                let f = self.module.get_function("llvm.round.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.round.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "round")).try_as_basic_value().basic().unwrap())
            }
            "trunc" => {
                let f = self.module.get_function("llvm.trunc.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.trunc.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "trunc")).try_as_basic_value().basic().unwrap())
            }
            // Trig via libm
            "sin" => {
                let f = self.module.get_function("llvm.sin.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.sin.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "sin")).try_as_basic_value().basic().unwrap())
            }
            "cos" => {
                let f = self.module.get_function("llvm.cos.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.cos.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "cos")).try_as_basic_value().basic().unwrap())
            }
            "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "cbrt" => {
                let f = self.module.get_function(method).unwrap_or_else(|| {
                    self.module.add_function(method, f64t.fn_type(&[f64t.into()], false), Some(Linkage::External))
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], method)).try_as_basic_value().basic().unwrap())
            }
            "exp" => {
                let f = self.module.get_function("llvm.exp.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.exp.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "exp")).try_as_basic_value().basic().unwrap())
            }
            "exp2" => {
                let f = self.module.get_function("llvm.exp2.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.exp2.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "exp2")).try_as_basic_value().basic().unwrap())
            }
            "ln" => {
                let f = self.module.get_function("llvm.log.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.log.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "ln")).try_as_basic_value().basic().unwrap())
            }
            "log2" => {
                let f = self.module.get_function("llvm.log2.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.log2.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "log2")).try_as_basic_value().basic().unwrap())
            }
            "log10" => {
                let f = self.module.get_function("llvm.log10.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.log10.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into()], "log10")).try_as_basic_value().basic().unwrap())
            }
            "recip" => {
                let one = f64t.const_float(1.0);
                Ok(b!(self.bld.build_float_div(one, receiver, "recip")).into())
            }
            "signum" => {
                // signum: returns -1.0, 0.0, or 1.0
                let zero = f64t.const_float(0.0);
                let neg_one = f64t.const_float(-1.0);
                let pos_one = f64t.const_float(1.0);
                let is_pos = b!(self.bld.build_float_compare(inkwell::FloatPredicate::OGT, receiver, zero, "pos"));
                let is_neg = b!(self.bld.build_float_compare(inkwell::FloatPredicate::OLT, receiver, zero, "neg"));
                let sel1 = b!(self.bld.build_select(
                    is_neg,
                    BasicValueEnum::FloatValue(neg_one),
                    BasicValueEnum::FloatValue(zero),
                    "s1",
                ))
                    .into_float_value();
                Ok(b!(self.bld.build_select(is_pos, BasicValueEnum::FloatValue(pos_one), BasicValueEnum::FloatValue(sel1), "signum")).into())
            }
            // Two-argument methods
            "pow" => {
                if args.len() < 2 { return Err("pow() requires 1 argument".into()); }
                let exp = self.compile_expr(&args[1])?.into_float_value();
                let f = self.module.get_function("llvm.pow.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.pow.f64", f64t.fn_type(&[f64t.into(), f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into(), exp.into()], "pow")).try_as_basic_value().basic().unwrap())
            }
            "atan2" => {
                if args.len() < 2 { return Err("atan2() requires 1 argument".into()); }
                let other = self.compile_expr(&args[1])?.into_float_value();
                let f = self.module.get_function("atan2").unwrap_or_else(|| {
                    self.module.add_function("atan2", f64t.fn_type(&[f64t.into(), f64t.into()], false), Some(Linkage::External))
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into(), other.into()], "atan2")).try_as_basic_value().basic().unwrap())
            }
            "copysign" => {
                if args.len() < 2 { return Err("copysign() requires 1 argument".into()); }
                let sign = self.compile_expr(&args[1])?.into_float_value();
                let f = self.module.get_function("llvm.copysign.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.copysign.f64", f64t.fn_type(&[f64t.into(), f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into(), sign.into()], "copysign")).try_as_basic_value().basic().unwrap())
            }
            "min" => {
                if args.len() < 2 { return Err("min() requires 1 argument".into()); }
                let other = self.compile_expr(&args[1])?.into_float_value();
                let f = self.module.get_function("llvm.minnum.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.minnum.f64", f64t.fn_type(&[f64t.into(), f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into(), other.into()], "fmin")).try_as_basic_value().basic().unwrap())
            }
            "max" => {
                if args.len() < 2 { return Err("max() requires 1 argument".into()); }
                let other = self.compile_expr(&args[1])?.into_float_value();
                let f = self.module.get_function("llvm.maxnum.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.maxnum.f64", f64t.fn_type(&[f64t.into(), f64t.into()], false), None)
                });
                Ok(b!(self.bld.build_call(f, &[receiver.into(), other.into()], "fmax")).try_as_basic_value().basic().unwrap())
            }
            // Boolean predicates
            "is_nan" => {
                let result = b!(self.bld.build_float_compare(inkwell::FloatPredicate::UNO, receiver, receiver, "isnan"));
                Ok(result.into())
            }
            "is_infinite" => {
                let abs_f = self.module.get_function("llvm.fabs.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.fabs.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                let abs_val = b!(self.bld.build_call(abs_f, &[receiver.into()], "abs")).try_as_basic_value().basic().unwrap().into_float_value();
                let inf = f64t.const_float(f64::INFINITY);
                let result = b!(self.bld.build_float_compare(inkwell::FloatPredicate::OEQ, abs_val, inf, "isinf"));
                Ok(result.into())
            }
            "is_finite" => {
                let abs_f = self.module.get_function("llvm.fabs.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.fabs.f64", f64t.fn_type(&[f64t.into()], false), None)
                });
                let abs_val = b!(self.bld.build_call(abs_f, &[receiver.into()], "abs")).try_as_basic_value().basic().unwrap().into_float_value();
                let inf = f64t.const_float(f64::INFINITY);
                let not_inf = b!(self.bld.build_float_compare(inkwell::FloatPredicate::ONE, abs_val, inf, "notinf"));
                let not_nan = b!(self.bld.build_float_compare(inkwell::FloatPredicate::ORD, receiver, receiver, "notnan"));
                Ok(b!(self.bld.build_and(not_inf, not_nan, "isfinite")).into())
            }
            "clamp" => {
                if args.len() < 3 { return Err("clamp() takes 2 arguments (lo, hi)".into()); }
                let lo = self.compile_expr(&args[1])?.into_float_value();
                let hi = self.compile_expr(&args[2])?.into_float_value();
                let min_f = self.module.get_function("llvm.minnum.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.minnum.f64", f64t.fn_type(&[f64t.into(), f64t.into()], false), None)
                });
                let max_f = self.module.get_function("llvm.maxnum.f64").unwrap_or_else(|| {
                    self.module.add_function("llvm.maxnum.f64", f64t.fn_type(&[f64t.into(), f64t.into()], false), None)
                });
                let min_val = b!(self.bld.build_call(min_f, &[receiver.into(), hi.into()], "clamp.min")).try_as_basic_value().basic().unwrap().into_float_value();
                Ok(b!(self.bld.build_call(max_f, &[min_val.into(), lo.into()], "clamp.max")).try_as_basic_value().basic().unwrap())
            }
            "to_int" => {
                Ok(b!(self.bld.build_float_to_signed_int(receiver, i64t, "ftoi")).into())
            }
            _ => Err(format!("unknown float method '{method}'")),
        }
    }

    fn compile_string_from_raw(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(&args[0])?;
        let len = self.compile_expr(&args[1])?;
        let cap = if args.len() > 2 {
            self.compile_expr(&args[2])?
        } else {
            len
        };
        self.build_string(ptr, len, cap, "sfr")
    }

    fn compile_string_from_ptr(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(&args[0])?;
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let strlen = self.module.get_function("strlen").unwrap_or_else(|| {
            self.module.add_function(
                "strlen",
                i64t.fn_type(&[ptr_ty.into()], false),
                Some(Linkage::External),
            )
        });
        let len = b!(self.bld.build_call(strlen, &[ptr.into()], "sfp.len"))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let size = b!(self
            .bld
            .build_int_nsw_add(len, i64t.const_int(1, false), "sfp.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "sfp.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[buf.into(), ptr.into(), size.into()], ""));
        self.build_string(buf, len, size, "sfp")
    }

    pub(crate) fn compile_get_args(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let argc_g = self
            .module
            .get_global("__jade_argc")
            .ok_or("__jade_argc global not found")?;
        let argv_g = self
            .module
            .get_global("__jade_argv")
            .ok_or("__jade_argv global not found")?;
        let argc =
            b!(self.bld.build_load(i32t, argc_g.as_pointer_value(), "argc")).into_int_value();
        let argc64 = b!(self.bld.build_int_s_extend(argc, i64t, "argc64"));
        let argv = b!(self
            .bld
            .build_load(ptr_ty, argv_g.as_pointer_value(), "argv"))
        .into_pointer_value();

        let header_ptr = self.compile_vec_new(&[])?.into_pointer_value();
        let header_ty = self.vec_header_type();
        let st = self.string_type();
        let str_size: u64 = 24;

        let fv = self.cur_fn.unwrap();
        let loop_bb = self.ctx.append_basic_block(fv, "args.loop");
        let body_bb = self.ctx.append_basic_block(fv, "args.body");
        let done_bb = self.ctx.append_basic_block(fv, "args.done");
        let i_ptr = self.entry_alloca(i64t.into(), "args.i");
        b!(self.bld.build_store(i_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let i = b!(self.bld.build_load(i64t, i_ptr, "i")).into_int_value();
        let cond = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, i, argc64, "args.cond"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let arg_pp = unsafe { b!(self.bld.build_gep(ptr_ty, argv, &[i], "arg.pp")) };
        let arg_p = b!(self.bld.build_load(ptr_ty, arg_pp, "arg.p")).into_pointer_value();
        let strlen = self.module.get_function("strlen").unwrap_or_else(|| {
            self.module.add_function(
                "strlen",
                i64t.fn_type(&[ptr_ty.into()], false),
                Some(Linkage::External),
            )
        });
        let slen = b!(self.bld.build_call(strlen, &[arg_p.into()], "arg.len"))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let size = b!(self
            .bld
            .build_int_nsw_add(slen, i64t.const_int(1, false), "arg.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "arg.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[buf.into(), arg_p.into(), size.into()], ""));
        let s = self.build_string(buf, slen, size, "arg.s")?;

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "ga.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "ga.len")).into_int_value();
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "ga.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "ga.cap")).into_int_value();
        let needs_grow = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, len, cap, "ga.full"));
        let grow_bb = self.ctx.append_basic_block(fv, "ga.grow");
        let store_bb = self.ctx.append_basic_block(fv, "ga.store");
        b!(self
            .bld
            .build_conditional_branch(needs_grow, grow_bb, store_bb));

        self.bld.position_at_end(grow_bb);
        let doubled = b!(self
            .bld
            .build_int_nsw_mul(cap, i64t.const_int(2, false), "ga.dbl"));
        let new_cap_cmp = b!(self.bld.build_int_compare(
            IntPredicate::SGT,
            doubled,
            i64t.const_int(4, false),
            "ga.cmp"
        ));
        let new_cap =
            b!(self
                .bld
                .build_select(new_cap_cmp, doubled, i64t.const_int(4, false), "ga.nc"))
            .into_int_value();
        let new_size =
            b!(self
                .bld
                .build_int_nsw_mul(new_cap, i64t.const_int(str_size, false), "ga.ns"));
        let realloc = self.ensure_realloc();
        let data_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "ga.datap"));
        let old_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "ga.optr"));
        let new_ptr =
            b!(self
                .bld
                .build_call(realloc, &[old_ptr.into(), new_size.into()], "ga.nptr"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        b!(self.bld.build_store(data_gep, new_ptr));
        b!(self.bld.build_store(cap_gep, new_cap));
        b!(self.bld.build_unconditional_branch(store_bb));

        self.bld.position_at_end(store_bb);
        let data_gep2 = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "ga.dp2"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep2, "ga.dp")).into_pointer_value();
        let elem_gep = unsafe { b!(self.bld.build_gep(st, data_ptr, &[len], "ga.ep")) };
        b!(self.bld.build_store(elem_gep, s));
        let new_len = b!(self
            .bld
            .build_int_nsw_add(len, i64t.const_int(1, false), "ga.nl"));
        b!(self.bld.build_store(len_gep, new_len));

        let next = b!(self
            .bld
            .build_int_nsw_add(i, i64t.const_int(1, false), "args.next"));
        b!(self.bld.build_store(i_ptr, next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        Ok(header_ptr.into())
    }

    /// Arena(cap) — allocate arena struct with malloc'd buffer
    pub(crate) fn compile_arena_new(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.len() != 1 {
            return Err("Arena() takes 1 argument (capacity)".into());
        }
        let cap = self.compile_expr(&args[0])?.into_int_value();
        let malloc = self.ensure_malloc();
        let base = b!(self
            .bld
            .build_call(malloc, &[cap.into()], "arena.buf"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        let arena_ty = self.arena_type();
        let ptr = self.entry_alloca(arena_ty.into(), "arena");
        let base_gep = b!(self.bld.build_struct_gep(arena_ty, ptr, 0, "arena.base"));
        b!(self.bld.build_store(base_gep, base));
        let cap_gep = b!(self.bld.build_struct_gep(arena_ty, ptr, 1, "arena.cap"));
        b!(self.bld.build_store(cap_gep, cap));
        let off_gep = b!(self.bld.build_struct_gep(arena_ty, ptr, 2, "arena.off"));
        b!(self
            .bld
            .build_store(off_gep, self.ctx.i64_type().const_int(0, false)));

        Ok(b!(self.bld.build_load(arena_ty, ptr, "arena.val")))
    }

    /// arena.alloc(nbytes) — bump-allocate from the arena
    pub(crate) fn compile_arena_alloc(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // args[0] = arena, args[1] = nbytes
        if args.len() != 2 {
            return Err("arena.alloc() takes 1 argument (size)".into());
        }
        let arena_val = self.compile_expr(&args[0])?;
        let nbytes = self.compile_expr(&args[1])?.into_int_value();

        let arena_ty = self.arena_type();
        let spill = self.entry_alloca(arena_ty.into(), "arena.spill");
        b!(self.bld.build_store(spill, arena_val));

        // Load base and offset
        let base_gep = b!(self.bld.build_struct_gep(arena_ty, spill, 0, "a.base.p"));
        let base = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            base_gep,
            "a.base"
        ))
        .into_pointer_value();
        let off_gep = b!(self.bld.build_struct_gep(arena_ty, spill, 2, "a.off.p"));
        let offset = b!(self
            .bld
            .build_load(self.ctx.i64_type(), off_gep, "a.off"))
        .into_int_value();

        // result = base + offset
        let result = unsafe {
            b!(self.bld.build_gep(
                self.ctx.i8_type(),
                base,
                &[offset],
                "arena.ptr"
            ))
        };

        // new_offset = offset + nbytes
        let new_off = b!(self
            .bld
            .build_int_add(offset, nbytes, "arena.new_off"));
        b!(self.bld.build_store(off_gep, new_off));

        // Write back to original variable if possible
        if let hir::ExprKind::Var(_, name) = &args[0].kind {
            if let Some((var_ptr, _)) = self.find_var(name).cloned() {
                let updated = b!(self.bld.build_load(arena_ty, spill, "arena.updated"));
                b!(self.bld.build_store(var_ptr, updated));
            }
        }

        Ok(result.into())
    }

    /// arena.reset() — reset offset to 0
    pub(crate) fn compile_arena_reset(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("arena.reset() requires arena receiver".into());
        }
        let arena_val = self.compile_expr(&args[0])?;
        let arena_ty = self.arena_type();
        let spill = self.entry_alloca(arena_ty.into(), "arena.spill");
        b!(self.bld.build_store(spill, arena_val));

        let off_gep = b!(self.bld.build_struct_gep(arena_ty, spill, 2, "a.off.p"));
        b!(self
            .bld
            .build_store(off_gep, self.ctx.i64_type().const_int(0, false)));

        // Write back to original variable
        if let hir::ExprKind::Var(_, name) = &args[0].kind {
            if let Some((var_ptr, _)) = self.find_var(name).cloned() {
                let updated = b!(self.bld.build_load(arena_ty, spill, "arena.reset"));
                b!(self.bld.build_store(var_ptr, updated));
            }
        }

        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    // ── Pool allocator builtins ─────────────────────────────────────

    fn ensure_pool_fn(&self, name: &str, fn_type: inkwell::types::FunctionType<'ctx>) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function(name).unwrap_or_else(|| {
            self.module.add_function(name, fn_type, Some(Linkage::External))
        })
    }

    pub(crate) fn compile_pool_new(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.len() != 2 {
            return Err("Pool() takes 2 arguments (obj_size, count)".into());
        }
        let obj_size = self.compile_expr(&args[0])?.into_int_value();
        let count = self.compile_expr(&args[1])?.into_int_value();
        let ptr_t = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let ft = ptr_t.fn_type(&[i64t.into(), i64t.into()], false);
        let func = self.ensure_pool_fn("jade_pool_create", ft);
        let result = b!(self.bld.build_call(func, &[obj_size.into(), count.into()], "pool.new"));
        Ok(result.try_as_basic_value().basic().unwrap())
    }

    pub(crate) fn compile_pool_alloc(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("pool.alloc() requires pool receiver".into());
        }
        let pool_ptr = self.compile_expr(&args[0])?.into_pointer_value();
        let ptr_t = self.ctx.ptr_type(AddressSpace::default());
        let ft = ptr_t.fn_type(&[ptr_t.into()], false);
        let func = self.ensure_pool_fn("jade_pool_alloc", ft);
        let result = b!(self.bld.build_call(func, &[pool_ptr.into()], "pool.alloc"));
        Ok(result.try_as_basic_value().basic().unwrap())
    }

    pub(crate) fn compile_pool_free(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.len() != 2 {
            return Err("pool.free() takes 1 argument (ptr)".into());
        }
        let pool_ptr = self.compile_expr(&args[0])?.into_pointer_value();
        let obj_ptr = self.compile_expr(&args[1])?.into_pointer_value();
        let ptr_t = self.ctx.ptr_type(AddressSpace::default());
        let void_t = self.ctx.void_type();
        let ft = void_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        let func = self.ensure_pool_fn("jade_pool_free", ft);
        b!(self.bld.build_call(func, &[pool_ptr.into(), obj_ptr.into()], ""));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    pub(crate) fn compile_pool_destroy(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("pool.destroy() requires pool receiver".into());
        }
        let pool_ptr = self.compile_expr(&args[0])?.into_pointer_value();
        let ptr_t = self.ctx.ptr_type(AddressSpace::default());
        let void_t = self.ctx.void_type();
        let ft = void_t.fn_type(&[ptr_t.into()], false);
        let func = self.ensure_pool_fn("jade_pool_destroy", ft);
        b!(self.bld.build_call(func, &[pool_ptr.into()], ""));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    fn compile_char_method(
        &mut self,
        method: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let char_val = self.compile_expr(&args[0])?.into_int_value();
        let i64t = self.ctx.i64_type();
        let bool_t = self.ctx.bool_type();

        match method {
            "to_code" => Ok(char_val.into()),
            "is_digit" => {
                // 0x30..=0x39 ('0'..='9')
                let ge = b!(self.bld.build_int_compare(
                    IntPredicate::SGE, char_val, i64t.const_int(0x30, false), "ch.ge0"
                ));
                let le = b!(self.bld.build_int_compare(
                    IntPredicate::SLE, char_val, i64t.const_int(0x39, false), "ch.le9"
                ));
                let result = b!(self.bld.build_and(ge, le, "ch.isdigit"));
                Ok(result.into())
            }
            "is_alpha" => {
                // A-Z (0x41..=0x5A) or a-z (0x61..=0x7A)
                let ge_a = b!(self.bld.build_int_compare(
                    IntPredicate::SGE, char_val, i64t.const_int(0x41, false), "ch.geA"
                ));
                let le_z = b!(self.bld.build_int_compare(
                    IntPredicate::SLE, char_val, i64t.const_int(0x5A, false), "ch.leZ"
                ));
                let upper = b!(self.bld.build_and(ge_a, le_z, "ch.isupper"));
                let ge_la = b!(self.bld.build_int_compare(
                    IntPredicate::SGE, char_val, i64t.const_int(0x61, false), "ch.gea"
                ));
                let le_lz = b!(self.bld.build_int_compare(
                    IntPredicate::SLE, char_val, i64t.const_int(0x7A, false), "ch.lez"
                ));
                let lower = b!(self.bld.build_and(ge_la, le_lz, "ch.islower"));
                let result = b!(self.bld.build_or(upper, lower, "ch.isalpha"));
                Ok(result.into())
            }
            "is_alphanumeric" => {
                // Combination of is_alpha and is_digit
                let ge_0 = b!(self.bld.build_int_compare(IntPredicate::SGE, char_val, i64t.const_int(0x30, false), "ch.ge0"));
                let le_9 = b!(self.bld.build_int_compare(IntPredicate::SLE, char_val, i64t.const_int(0x39, false), "ch.le9"));
                let digit = b!(self.bld.build_and(ge_0, le_9, "ch.dig"));
                let ge_a = b!(self.bld.build_int_compare(IntPredicate::SGE, char_val, i64t.const_int(0x41, false), "ch.geA"));
                let le_z = b!(self.bld.build_int_compare(IntPredicate::SLE, char_val, i64t.const_int(0x5A, false), "ch.leZ"));
                let upper = b!(self.bld.build_and(ge_a, le_z, "ch.up"));
                let ge_la = b!(self.bld.build_int_compare(IntPredicate::SGE, char_val, i64t.const_int(0x61, false), "ch.gea"));
                let le_lz = b!(self.bld.build_int_compare(IntPredicate::SLE, char_val, i64t.const_int(0x7A, false), "ch.lez"));
                let lower = b!(self.bld.build_and(ge_la, le_lz, "ch.lo"));
                let alpha = b!(self.bld.build_or(upper, lower, "ch.al"));
                let result = b!(self.bld.build_or(digit, alpha, "ch.alnum"));
                Ok(result.into())
            }
            "is_upper" => {
                let ge = b!(self.bld.build_int_compare(IntPredicate::SGE, char_val, i64t.const_int(0x41, false), "ch.geA"));
                let le = b!(self.bld.build_int_compare(IntPredicate::SLE, char_val, i64t.const_int(0x5A, false), "ch.leZ"));
                Ok(b!(self.bld.build_and(ge, le, "ch.isupper")).into())
            }
            "is_lower" => {
                let ge = b!(self.bld.build_int_compare(IntPredicate::SGE, char_val, i64t.const_int(0x61, false), "ch.gea"));
                let le = b!(self.bld.build_int_compare(IntPredicate::SLE, char_val, i64t.const_int(0x7A, false), "ch.lez"));
                Ok(b!(self.bld.build_and(ge, le, "ch.islower")).into())
            }
            "is_whitespace" => {
                // space(0x20), tab(0x09), newline(0x0A), carriage return(0x0D)
                let is_sp = b!(self.bld.build_int_compare(IntPredicate::EQ, char_val, i64t.const_int(0x20, false), "ch.sp"));
                let is_tab = b!(self.bld.build_int_compare(IntPredicate::EQ, char_val, i64t.const_int(0x09, false), "ch.tab"));
                let is_nl = b!(self.bld.build_int_compare(IntPredicate::EQ, char_val, i64t.const_int(0x0A, false), "ch.nl"));
                let is_cr = b!(self.bld.build_int_compare(IntPredicate::EQ, char_val, i64t.const_int(0x0D, false), "ch.cr"));
                let t1 = b!(self.bld.build_or(is_sp, is_tab, "ch.ws1"));
                let t2 = b!(self.bld.build_or(is_nl, is_cr, "ch.ws2"));
                Ok(b!(self.bld.build_or(t1, t2, "ch.isws")).into())
            }
            "to_upper" => {
                // If lowercase (0x61..=0x7A), subtract 0x20
                let ge = b!(self.bld.build_int_compare(IntPredicate::SGE, char_val, i64t.const_int(0x61, false), "ch.gea"));
                let le = b!(self.bld.build_int_compare(IntPredicate::SLE, char_val, i64t.const_int(0x7A, false), "ch.lez"));
                let is_lower = b!(self.bld.build_and(ge, le, "ch.islo"));
                let upper = b!(self.bld.build_int_nsw_sub(char_val, i64t.const_int(0x20, false), "ch.toU"));
                Ok(b!(self.bld.build_select(is_lower, upper, char_val, "ch.toupper")).into())
            }
            "to_lower" => {
                // If uppercase (0x41..=0x5A), add 0x20
                let ge = b!(self.bld.build_int_compare(IntPredicate::SGE, char_val, i64t.const_int(0x41, false), "ch.geA"));
                let le = b!(self.bld.build_int_compare(IntPredicate::SLE, char_val, i64t.const_int(0x5A, false), "ch.leZ"));
                let is_upper = b!(self.bld.build_and(ge, le, "ch.isup"));
                let lower = b!(self.bld.build_int_add(char_val, i64t.const_int(0x20, false), "ch.toL"));
                Ok(b!(self.bld.build_select(is_upper, lower, char_val, "ch.tolower")).into())
            }
            "to_float" => {
                let f64t = self.ctx.f64_type();
                let result = b!(self.bld.build_signed_int_to_float(char_val, f64t, "i2f"));
                Ok(result.into())
            }
            "abs" => {
                // x < 0 ? -x : x
                let neg = b!(self.bld.build_int_neg(char_val, "int.neg"));
                let is_neg = b!(self.bld.build_int_compare(IntPredicate::SLT, char_val, i64t.const_zero(), "int.isneg"));
                Ok(b!(self.bld.build_select(is_neg, neg, char_val, "int.abs")).into())
            }
            "min" => {
                if args.len() < 2 {
                    return Err("min() takes 1 argument".into());
                }
                let other = self.compile_expr(&args[1])?.into_int_value();
                let cmp = b!(self.bld.build_int_compare(IntPredicate::SLT, char_val, other, "int.lt"));
                Ok(b!(self.bld.build_select(cmp, char_val, other, "int.min")).into())
            }
            "max" => {
                if args.len() < 2 {
                    return Err("max() takes 1 argument".into());
                }
                let other = self.compile_expr(&args[1])?.into_int_value();
                let cmp = b!(self.bld.build_int_compare(IntPredicate::SGT, char_val, other, "int.gt"));
                Ok(b!(self.bld.build_select(cmp, char_val, other, "int.max")).into())
            }
            "clamp" => {
                if args.len() < 3 {
                    return Err("clamp() takes 2 arguments (lo, hi)".into());
                }
                let lo = self.compile_expr(&args[1])?.into_int_value();
                let hi = self.compile_expr(&args[2])?.into_int_value();
                // max(lo, min(x, hi))
                let cmp_hi = b!(self.bld.build_int_compare(IntPredicate::SLT, char_val, hi, "clamp.lthi"));
                let min_val = b!(self.bld.build_select(cmp_hi, char_val, hi, "clamp.min")).into_int_value();
                let cmp_lo = b!(self.bld.build_int_compare(IntPredicate::SGT, min_val, lo, "clamp.gtlo"));
                Ok(b!(self.bld.build_select(cmp_lo, min_val, lo, "clamp.max")).into())
            }
            "to_str" => {
                self.compile_to_string(&args[0])
            }
            _ => Err(format!("unknown char method '{method}'")),
        }
    }
}
