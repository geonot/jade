#![allow(unused_imports, unused_variables)]
use super::*;

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
            hir::BuiltinFn::Print => self.compile_print(args),
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
            hir::BuiltinFn::SignalDefault => {
                if args.len() != 1 {
                    return Err("signal_default() takes 1 argument".into());
                }
                self.compile_signal_default(args)
            }
            hir::BuiltinFn::SignalKill => {
                if args.len() != 2 {
                    return Err("signal_kill() takes 2 arguments".into());
                }
                self.compile_signal_kill(args)
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
            | hir::BuiltinFn::CompTimeSizeOf => Err(format!(
                "builtin {:?} should not appear in codegen",
                builtin
            )),
            hir::BuiltinFn::CharMethod(method) => self.compile_char_method(&method.as_str(), args),
            hir::BuiltinFn::Matmul => self.compile_matmul(args),
            hir::BuiltinFn::RegexMatch | hir::BuiltinFn::RegexFindAll => Err(format!(
                "builtin {:?} should be lowered to string methods",
                builtin
            )),
            hir::BuiltinFn::ConstantTimeEq => self.compile_constant_time_eq(args),
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
                    .expect("ICE: call returned void")
                    .into_pointer_value();
                let memset = self
                    .module
                    .get_function("llvm.memset.p0.i64")
                    .unwrap_or_else(|| {
                        self.module.add_function(
                            "llvm.memset.p0.i64",
                            self.ctx.void_type().fn_type(
                                &[
                                    ptr_t.into(),
                                    self.ctx.i8_type().into(),
                                    i64t.into(),
                                    self.ctx.bool_type().into(),
                                ],
                                false,
                            ),
                            None,
                        )
                    });
                b!(self.bld.build_call(
                    memset,
                    &[
                        ptr.into(),
                        self.ctx.i8_type().const_zero().into(),
                        size.into(),
                        self.ctx.bool_type().const_zero().into()
                    ],
                    "",
                ));
                Ok(ptr.into())
            }
            hir::BuiltinFn::DequeNew => Err("DequeNew should be handled via ExprKind".into()),
            hir::BuiltinFn::GradFn => Err("GradFn not yet implemented in codegen".into()),
            hir::BuiltinFn::Einsum => Err("Einsum should be handled via ExprKind".into()),
            hir::BuiltinFn::CowWrap => Err("CowWrap should be handled via ExprKind".into()),
            hir::BuiltinFn::Likely | hir::BuiltinFn::Unlikely => {
                if args.len() != 1 {
                    return Err("likely/unlikely takes exactly 1 boolean argument".into());
                }
                let cond = self.compile_expr(&args[0])?;
                let i1ty = self.ctx.bool_type();
                let ft = i1ty.fn_type(&[i1ty.into(), i1ty.into()], false);
                let expect_fn = self
                    .module
                    .get_function("llvm.expect.i1")
                    .unwrap_or_else(|| self.module.add_function("llvm.expect.i1", ft, None));
                let expected = match builtin {
                    hir::BuiltinFn::Likely => i1ty.const_int(1, false),
                    _ => i1ty.const_int(0, false),
                };
                let result =
                    b!(self
                        .bld
                        .build_call(expect_fn, &[cond.into(), expected.into()], "expect"));
                Ok(self.call_result(result))
            }
            hir::BuiltinFn::PoolNew => self.compile_pool_new(args),
            hir::BuiltinFn::PoolAlloc => self.compile_pool_alloc(args),
            hir::BuiltinFn::PoolFree => self.compile_pool_free(args),
            hir::BuiltinFn::PoolDestroy => self.compile_pool_destroy(args),
            hir::BuiltinFn::FloatMethod(method) => self.compile_float_method(&method.as_str(), args),
        }
    }

    pub(in crate::codegen) fn compile_constant_time_eq(
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
                .unwrap_or_else(|| {
                    self.module
                        .add_function("__jade_constant_time_eq", fn_type, None)
                });
            let result = b!(self.bld.build_call(func, &[a.into(), b.into()], "ct.eq"));
            Ok(self.call_result(result))
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

    pub(in crate::codegen) fn compile_matmul(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
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
            let rt_fn = self
                .module
                .get_function("__jade_matmul")
                .unwrap_or_else(|| {
                    let i64t = self.ctx.i64_type();
                    let ptr_t = self.ctx.ptr_type(inkwell::AddressSpace::default());
                    let ft = ptr_t.fn_type(
                        &[
                            ptr_t.into(),
                            ptr_t.into(),
                            i64t.into(),
                            i64t.into(),
                            i64t.into(),
                        ],
                        false,
                    );
                    self.module.add_function("__jade_matmul", ft, None)
                });
            let i64t = self.ctx.i64_type();
            let result = b!(self.bld.build_call(
                rt_fn,
                &[
                    a_ptr.into(),
                    b_ptr.into(),
                    i64t.const_int(0, false).into(),
                    i64t.const_int(0, false).into(),
                    i64t.const_int(0, false).into()
                ],
                "matmul.rt"
            ))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void");
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
        .expect("ICE: call returned void")
        .into_pointer_value();

        // Zero-initialize result
        let memset = self
            .module
            .get_function("llvm.memset.p0.i64")
            .unwrap_or_else(|| {
                let ptr_t = self.ctx.ptr_type(inkwell::AddressSpace::default());
                self.module.add_function(
                    "llvm.memset.p0.i64",
                    self.ctx.void_type().fn_type(
                        &[
                            ptr_t.into(),
                            self.ctx.i8_type().into(),
                            i64t.into(),
                            self.ctx.bool_type().into(),
                        ],
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

        let fn_val = self.current_fn();
        let outer_bb = self.ctx.append_basic_block(fn_val, "mm.i");
        let mid_bb = self.ctx.append_basic_block(fn_val, "mm.j");
        let inner_bb = self.ctx.append_basic_block(fn_val, "mm.k");
        let inner_body = self.ctx.append_basic_block(fn_val, "mm.body");
        let _inner_end = self.ctx.append_basic_block(fn_val, "mm.k.end");
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
        let i_cmp = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::ULT,
            i,
            i64t.const_int(m as u64, false),
            "i.cmp"
        ));
        b!(self.bld.build_conditional_branch(i_cmp, mid_bb, outer_end));

        // Mid loop: j
        self.bld.position_at_end(mid_bb);
        b!(self.bld.build_store(j_ptr, i64t.const_zero()));
        b!(self.bld.build_unconditional_branch(inner_bb));

        self.bld.position_at_end(inner_bb);
        let j = b!(self.bld.build_load(i64t, j_ptr, "j")).into_int_value();
        let j_cmp = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::ULT,
            j,
            i64t.const_int(n_val, false),
            "j.cmp"
        ));
        b!(self
            .bld
            .build_conditional_branch(j_cmp, inner_body, mid_end));

        // Inner loop body: k
        self.bld.position_at_end(inner_body);
        b!(self.bld.build_store(k_ptr, i64t.const_zero()));

        let k_loop = self.ctx.append_basic_block(fn_val, "mm.kloop");
        let k_body = self.ctx.append_basic_block(fn_val, "mm.kbody");
        let k_end = self.ctx.append_basic_block(fn_val, "mm.kend");
        b!(self.bld.build_unconditional_branch(k_loop));

        self.bld.position_at_end(k_loop);
        let k = b!(self.bld.build_load(i64t, k_ptr, "k")).into_int_value();
        let k_cmp = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::ULT,
            k,
            i64t.const_int(k_val, false),
            "k.cmp"
        ));
        b!(self.bld.build_conditional_branch(k_cmp, k_body, k_end));

        self.bld.position_at_end(k_body);
        let i2 = b!(self.bld.build_load(i64t, i_ptr, "i2")).into_int_value();
        let j2 = b!(self.bld.build_load(i64t, j_ptr, "j2")).into_int_value();
        // A[i*k1+k]
        let a_idx = b!(self
            .bld
            .build_int_mul(i2, i64t.const_int(k_val, false), "a.row"));
        let a_idx = b!(self.bld.build_int_add(a_idx, k, "a.idx"));
        let a_ep = unsafe { b!(self.bld.build_gep(f64t, a_ptr, &[a_idx.into()], "a.ep")) };
        let a_val = b!(self.bld.build_load(f64t, a_ep, "a.val")).into_float_value();
        // B[k*n+j]
        let b_idx = b!(self
            .bld
            .build_int_mul(k, i64t.const_int(n_val, false), "b.row"));
        let b_idx = b!(self.bld.build_int_add(b_idx, j2, "b.idx"));
        let b_ep = unsafe { b!(self.bld.build_gep(f64t, b_ptr, &[b_idx.into()], "b.ep")) };
        let b_val = b!(self.bld.build_load(f64t, b_ep, "b.val")).into_float_value();
        // result[i*n+j] += a * b
        let r_idx = b!(self
            .bld
            .build_int_mul(i2, i64t.const_int(n_val, false), "r.row"));
        let r_idx = b!(self.bld.build_int_add(r_idx, j2, "r.idx"));
        let r_ep = unsafe {
            b!(self
                .bld
                .build_gep(f64t, result_ptr, &[r_idx.into()], "r.ep"))
        };
        let r_val = b!(self.bld.build_load(f64t, r_ep, "r.cur")).into_float_value();
        let prod = b!(self.bld.build_float_mul(a_val, b_val, "mm.prod"));
        let sum = b!(self.bld.build_float_add(r_val, prod, "mm.sum"));
        b!(self.bld.build_store(r_ep, sum));

        let k_next = b!(self
            .bld
            .build_int_add(k, i64t.const_int(1, false), "k.next"));
        b!(self.bld.build_store(k_ptr, k_next));
        b!(self.bld.build_unconditional_branch(k_loop));

        self.bld.position_at_end(k_end);
        let j_next = b!(self
            .bld
            .build_int_add(j, i64t.const_int(1, false), "j.next"));
        b!(self.bld.build_store(j_ptr, j_next));
        b!(self.bld.build_unconditional_branch(inner_bb));

        self.bld.position_at_end(mid_end);
        let i_next = b!(self
            .bld
            .build_int_add(i, i64t.const_int(1, false), "i.next"));
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

    pub(in crate::codegen) fn compile_einsum_dot(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let a_ptr = self.compile_expr(&args[0])?.into_pointer_value();
        let b_ptr = self.compile_expr(&args[1])?.into_pointer_value();
        let n = match &args[0].ty {
            Type::NDArray(_, dims) if dims.len() == 1 => dims[0] as u64,
            _ => return Err("einsum dot requires 1D NDArrays".into()),
        };
        let i64t = self.ctx.i64_type();
        let f64t = self.ctx.f64_type();
        let fn_val = self.current_fn();

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
        let cmp = b!(self.bld.build_int_compare(
            IntPredicate::ULT,
            i,
            i64t.const_int(n, false),
            "dot.cmp"
        ));
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
        let i_next = b!(self
            .bld
            .build_int_add(i, i64t.const_int(1, false), "i.next"));
        b!(self.bld.build_store(iv, i_next));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(end_bb);
        Ok(b!(self.bld.build_load(f64t, acc, "dot.result")))
    }
}
