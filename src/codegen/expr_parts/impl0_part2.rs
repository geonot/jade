#![allow(unused_imports, unused_variables)]
use super::*;

impl<'ctx> Compiler<'ctx> {

    pub(in crate::codegen) fn compile_strict_cast(
        &mut self,
        expr: &hir::Expr,
        target: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Strict cast: same as regular cast but with runtime bounds check for narrowing
        let val = self.compile_expr(expr)?;
        let src = &expr.ty;
        if src.is_int() && target.is_int() {
            let dst = self.llvm_ty(target);
            let (sb, db) = (src.bits(), target.bits());
            if sb > db {
                // Narrowing: truncate then sign-extend back and compare
                let truncated = b!(self.bld.build_int_truncate(
                    val.into_int_value(),
                    dst.into_int_type(),
                    "strict.trunc"
                ));
                let extended = if src.is_signed() {
                    b!(self.bld.build_int_s_extend(
                        truncated,
                        val.into_int_value().get_type(),
                        "strict.ext"
                    ))
                } else {
                    b!(self.bld.build_int_z_extend(
                        truncated,
                        val.into_int_value().get_type(),
                        "strict.ext"
                    ))
                };
                let ok = b!(self.bld.build_int_compare(
                    IntPredicate::EQ,
                    val.into_int_value(),
                    extended,
                    "strict.ok"
                ));
                let fv = self.current_fn();
                let pass_bb = self.ctx.append_basic_block(fv, "strict.pass");
                let fail_bb = self.ctx.append_basic_block(fv, "strict.fail");
                b!(self.bld.build_conditional_branch(ok, pass_bb, fail_bb));
                self.bld.position_at_end(fail_bb);
                self.emit_trap("strict cast: value out of range");
                self.bld.position_at_end(pass_bb);
                return Ok(truncated.into());
            } else if sb < db {
                return Ok(if src.is_signed() {
                    b!(self.bld.build_int_s_extend(
                        val.into_int_value(),
                        dst.into_int_type(),
                        "strict.sext"
                    ))
                    .into()
                } else {
                    b!(self.bld.build_int_z_extend(
                        val.into_int_value(),
                        dst.into_int_type(),
                        "strict.zext"
                    ))
                    .into()
                });
            }
            return Ok(val);
        }
        // Float→int strict cast: check for NaN, infinity, and out-of-range
        if src.is_float() && target.is_int() {
            let fv = self.current_fn();
            let float_val = val.into_float_value();
            let dst_int_ty = self.llvm_ty(target).into_int_type();
            let src_float_ty = float_val.get_type();

            // Check NaN: float != float means NaN
            let is_nan = b!(self.bld.build_float_compare(
                FloatPredicate::UNO,
                float_val,
                float_val,
                "strict.isnan"
            ));
            let nan_fail_bb = self.ctx.append_basic_block(fv, "strict.nan_fail");
            let nan_pass_bb = self.ctx.append_basic_block(fv, "strict.nan_pass");
            b!(self
                .bld
                .build_conditional_branch(is_nan, nan_fail_bb, nan_pass_bb));
            self.bld.position_at_end(nan_fail_bb);
            self.emit_trap("strict cast: cannot convert NaN to integer");
            self.bld.position_at_end(nan_pass_bb);

            // Convert float→int
            let int_val = if target.is_signed() {
                b!(self
                    .bld
                    .build_float_to_signed_int(float_val, dst_int_ty, "strict.fptosi"))
            } else {
                b!(self
                    .bld
                    .build_float_to_unsigned_int(float_val, dst_int_ty, "strict.fptoui"))
            };
            // Convert back int→float and compare: if not equal, value was out of range or fractional
            let roundtrip = if target.is_signed() {
                b!(self
                    .bld
                    .build_signed_int_to_float(int_val, src_float_ty, "strict.sitofp"))
            } else {
                b!(self
                    .bld
                    .build_unsigned_int_to_float(int_val, src_float_ty, "strict.uitofp"))
            };
            let ok = b!(self.bld.build_float_compare(
                FloatPredicate::OEQ,
                float_val,
                roundtrip,
                "strict.roundtrip_ok"
            ));
            let pass_bb = self.ctx.append_basic_block(fv, "strict.fti_pass");
            let fail_bb = self.ctx.append_basic_block(fv, "strict.fti_fail");
            b!(self.bld.build_conditional_branch(ok, pass_bb, fail_bb));
            self.bld.position_at_end(fail_bb);
            self.emit_trap("strict cast: float value out of integer range");
            self.bld.position_at_end(pass_bb);
            return Ok(int_val.into());
        }
        // For other casts, fall back to regular cast behavior
        self.compile_cast(expr, target)
    }

    pub(in crate::codegen) fn compile_as_format(
        &mut self,
        expr: &hir::Expr,
        fmt: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match fmt {
            "json" => self.compile_as_json(expr),
            _ => {
                // Fallback: use compile_to_string
                self.compile_to_string(expr)
            }
        }
    }

    pub(in crate::codegen) fn compile_as_json(&mut self, expr: &hir::Expr) -> Result<BasicValueEnum<'ctx>, String> {
        let ty = self.resolve_ty(expr.ty.clone());
        match &ty {
            Type::Struct(name, _) => {
                let fields = self.structs.get(name).cloned().unwrap_or_default();
                let val = self.compile_expr(expr)?;
                // Build JSON: {"field1": val1, "field2": val2, ...}
                let mut result = self.compile_str_literal("{")?;
                let struct_ty = self
                    .module
                    .get_struct_type(&name.as_str())
                    .ok_or_else(|| format!("unknown struct type: {name}"))?;
                for (i, (fname, fty)) in fields.iter().enumerate() {
                    // Add comma separator
                    if i > 0 {
                        let comma = self.compile_str_literal(", ")?;
                        result = self.string_concat(result, comma)?;
                    }
                    // Add "fieldname":
                    let key_str = self.compile_str_literal(&format!("\"{fname}\": "))?;
                    result = self.string_concat(result, key_str)?;
                    // Extract field value
                    let field_val = if val.is_pointer_value() {
                        let fgep = b!(self.bld.build_struct_gep(
                            struct_ty,
                            val.into_pointer_value(),
                            i as u32,
                            "json.fgep"
                        ));
                        b!(self.bld.build_load(self.llvm_ty(fty), fgep, "json.fld"))
                    } else {
                        b!(self.bld.build_extract_value(
                            val.into_struct_value(),
                            i as u32,
                            "json.fld"
                        ))
                    };
                    // Format field value based on type
                    let fval_str = match fty {
                        Type::String => {
                            // Wrap in quotes
                            let q = self.compile_str_literal("\"")?;
                            let s = self.string_concat(q.clone(), field_val)?;
                            self.string_concat(s, q)?
                        }
                        Type::I64 | Type::I32 | Type::I16 | Type::I8 => {
                            self.int_to_string(field_val, false)?
                        }
                        Type::U64 | Type::U32 | Type::U16 | Type::U8 => {
                            self.int_to_string(field_val, true)?
                        }
                        Type::F64 | Type::F32 => self.float_to_string(field_val)?,
                        Type::Bool => self.bool_to_string(field_val)?,
                        _ => self.int_to_string(field_val, false)?,
                    };
                    result = self.string_concat(result, fval_str)?;
                }
                let close = self.compile_str_literal("}")?;
                self.string_concat(result, close)
            }
            Type::String => {
                // String: wrap in quotes
                let q = self.compile_str_literal("\"")?;
                let val = self.compile_expr(expr)?;
                let s = self.string_concat(q.clone(), val)?;
                self.string_concat(s, q)
            }
            _ => {
                // Primitives: just convert to string
                self.compile_to_string(expr)
            }
        }
    }

    pub(in crate::codegen) fn compile_atomic_load(
        &mut self,
        ptr_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let i64t = self.ctx.i64_type();
        let load = b!(self
            .bld
            .build_load(i64t, ptr.into_pointer_value(), "atomic.load"));
        // Set atomic ordering
        load.as_instruction_value().expect("ICE: not an instruction")
            .set_atomic_ordering(inkwell::AtomicOrdering::SequentiallyConsistent)
            .map_err(|_| "failed to set atomic ordering".to_string())?;
        Ok(load)
    }

    pub(in crate::codegen) fn compile_atomic_store(
        &mut self,
        ptr_expr: &hir::Expr,
        val_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let val = self.compile_expr(val_expr)?;
        let store = b!(self.bld.build_store(ptr.into_pointer_value(), val));
        store
            .set_atomic_ordering(inkwell::AtomicOrdering::SequentiallyConsistent)
            .map_err(|_| "failed to set atomic ordering".to_string())?;
        Ok(self.ctx.i64_type().const_zero().into())
    }

    pub(in crate::codegen) fn compile_atomic_add(
        &mut self,
        ptr_expr: &hir::Expr,
        val_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let val = self.compile_expr(val_expr)?;
        let old = b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Add,
            ptr.into_pointer_value(),
            val.into_int_value(),
            inkwell::AtomicOrdering::SequentiallyConsistent,
        ));
        Ok(old.into())
    }

    pub(in crate::codegen) fn compile_atomic_sub(
        &mut self,
        ptr_expr: &hir::Expr,
        val_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let val = self.compile_expr(val_expr)?;
        let old = b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Sub,
            ptr.into_pointer_value(),
            val.into_int_value(),
            inkwell::AtomicOrdering::SequentiallyConsistent,
        ));
        Ok(old.into())
    }

    pub(in crate::codegen) fn compile_atomic_cas(
        &mut self,
        ptr_expr: &hir::Expr,
        expected_expr: &hir::Expr,
        new_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let expected = self.compile_expr(expected_expr)?;
        let new_val = self.compile_expr(new_expr)?;
        let cas = b!(self.bld.build_cmpxchg(
            ptr.into_pointer_value(),
            expected.into_int_value(),
            new_val.into_int_value(),
            inkwell::AtomicOrdering::SequentiallyConsistent,
            inkwell::AtomicOrdering::SequentiallyConsistent,
        ));
        // cmpxchg returns {value, success_bit}; extract the old value
        let old = b!(self.bld.build_extract_value(cas, 0, "cas.old"));
        Ok(old)
    }

    pub(in crate::codegen) fn ensure_deque_fn(
        &self,
        name: &str,
        param_tys: &[BasicMetadataTypeEnum<'ctx>],
        ret_ty: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function(name).unwrap_or_else(|| {
            let ft = ret_ty.fn_type(param_tys, false);
            self.module
                .add_function(name, ft, Some(inkwell::module::Linkage::External))
        })
    }

    pub(in crate::codegen) fn compile_deque_new(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let f = self
            .module
            .get_function("__jade_deque_new")
            .unwrap_or_else(|| {
                let ft = ptr_ty.fn_type(&[], false);
                self.module.add_function(
                    "__jade_deque_new",
                    ft,
                    Some(inkwell::module::Linkage::External),
                )
            });
        let result = b!(self.bld.build_call(f, &[], "deque.new"));
        Ok(self.call_result(result))
    }

    pub(in crate::codegen) fn compile_deque_method(
        &mut self,
        obj: &hir::Expr,
        method: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let void_ty = self.ctx.void_type();
        let handle = self.compile_expr(obj)?;

        match method {
            "push_back" | "push_front" => {
                let rt_name = if method == "push_back" {
                    "__jade_deque_push_back"
                } else {
                    "__jade_deque_push_front"
                };
                let f = self.module.get_function(rt_name).unwrap_or_else(|| {
                    let ft = void_ty.fn_type(&[ptr_ty.into(), i64t.into()], false);
                    self.module
                        .add_function(rt_name, ft, Some(inkwell::module::Linkage::External))
                });
                let val = self.compile_expr(&args[0])?;
                b!(self.bld.build_call(f, &[handle.into(), val.into()], ""));
                Ok(i64t.const_int(0, false).into())
            }
            "pop_front" | "pop_back" => {
                let rt_name = if method == "pop_front" {
                    "__jade_deque_pop_front"
                } else {
                    "__jade_deque_pop_back"
                };
                let f = self.ensure_deque_fn(rt_name, &[ptr_ty.into()], i64t.into());
                let result = b!(self.bld.build_call(f, &[handle.into()], "dq.pop"));
                Ok(self.call_result(result))
            }
            "len" => {
                let f = self.ensure_deque_fn("__jade_deque_len", &[ptr_ty.into()], i64t.into());
                let result = b!(self.bld.build_call(f, &[handle.into()], "dq.len"));
                Ok(self.call_result(result))
            }
            _ => Err(format!("no method '{method}' on Deque")),
        }
    }

    pub(in crate::codegen) fn compile_slice(
        &mut self,
        obj: &hir::Expr,
        start: &hir::Expr,
        end: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // For now, compile as a runtime call to a slice helper
        // Arrays: create a new vec from the slice range
        // Strings: create a substring
        let obj_val = self.compile_expr(obj)?;
        let start_val = self.compile_expr(start)?;
        let end_val = self.compile_expr(end)?;
        match &obj.ty {
            Type::Vec(elem_ty) => {
                // Vec slice: call jade_vec_slice(vec_ptr, start, end, elem_size) → new vec
                let lty = self.llvm_ty(elem_ty);
                let elem_size = self.type_store_size(lty);
                let i64t = self.ctx.i64_type();
                let slice_fn = self
                    .module
                    .get_function("__jade_vec_slice")
                    .unwrap_or_else(|| {
                        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                        let ft = ptr_ty.fn_type(
                            &[ptr_ty.into(), i64t.into(), i64t.into(), i64t.into()],
                            false,
                        );
                        self.module.add_function(
                            "__jade_vec_slice",
                            ft,
                            Some(inkwell::module::Linkage::External),
                        )
                    });
                let esz = i64t.const_int(elem_size, false);
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[obj_val.into(), start_val.into(), end_val.into(), esz.into()],
                    "slice"
                ));
                Ok(result
                    .try_as_basic_value()
                    .basic()
                    .unwrap_or_else(|| self.ctx.i64_type().const_zero().into()))
            }
            Type::String => {
                // String slice: call jade_str_slice(str, start, end) → new str
                let slice_fn = self
                    .module
                    .get_function("__jade_str_slice")
                    .unwrap_or_else(|| {
                        let st = self.string_type();
                        let i64t = self.ctx.i64_type();
                        let ft = st.fn_type(&[st.into(), i64t.into(), i64t.into()], false);
                        self.module.add_function(
                            "__jade_str_slice",
                            ft,
                            Some(inkwell::module::Linkage::External),
                        )
                    });
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[obj_val.into(), start_val.into(), end_val.into()],
                    "str.slice"
                ));
                Ok(result
                    .try_as_basic_value()
                    .basic()
                    .unwrap_or_else(|| self.ctx.i64_type().const_zero().into()))
            }
            _ => Err(format!("slice not supported for type: {}", &obj.ty)),
        }
    }

    /// COW wrap: allocate {rc: i64, data: T}, set rc=1, copy value into data.
    pub(in crate::codegen) fn compile_cow_wrap(&mut self, inner: &hir::Expr) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(inner)?;
        let data_ty = self.llvm_ty(&inner.ty);
        let i64t = self.ctx.i64_type();
        let cow_st = self.ctx.struct_type(&[i64t.into(), data_ty], false);
        let malloc = self.ensure_malloc();
        let size = cow_st.size_of().expect("ICE: type has no size");
        let ptr = b!(self.bld.build_call(malloc, &[size.into()], "cow.alloc"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void")
            .into_pointer_value();
        // rc = 1
        let rc_gep = b!(self.bld.build_struct_gep(cow_st, ptr, 0, "cow.rc"));
        b!(self.bld.build_store(rc_gep, i64t.const_int(1, false)));
        // store data
        let data_gep = b!(self.bld.build_struct_gep(cow_st, ptr, 1, "cow.data"));
        b!(self.bld.build_store(data_gep, val));
        Ok(ptr.into())
    }

    /// COW clone: if RC > 1, duplicate the backing storage and decrement
    /// the original's RC. Otherwise return the same pointer.
    pub(in crate::codegen) fn compile_cow_clone(&mut self, inner: &hir::Expr) -> Result<BasicValueEnum<'ctx>, String> {
        let cow_ptr = self.compile_expr(inner)?.into_pointer_value();
        let cow_inner_ty = match &inner.ty {
            crate::types::Type::Cow(inner) => inner.as_ref().clone(),
            other => other.clone(),
        };
        let data_ty = self.llvm_ty(&cow_inner_ty);
        let i64t = self.ctx.i64_type();
        let cow_st = self.ctx.struct_type(&[i64t.into(), data_ty], false);

        let rc_gep = b!(self.bld.build_struct_gep(cow_st, cow_ptr, 0, "cow.rcp"));
        let rc = b!(self.bld.build_load(i64t, rc_gep, "cow.rc")).into_int_value();
        let needs_clone = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::UGT,
            rc,
            i64t.const_int(1, false),
            "cow.shared"
        ));

        let fn_val = self.current_fn();
        let clone_bb = self.ctx.append_basic_block(fn_val, "cow.clone");
        let done_bb = self.ctx.append_basic_block(fn_val, "cow.done");
        let cur_bb = self.current_bb();
        b!(self
            .bld
            .build_conditional_branch(needs_clone, clone_bb, done_bb));

        // Clone path: allocate new cow, copy data, set rc=1, decrement original rc
        self.bld.position_at_end(clone_bb);
        let malloc = self.ensure_malloc();
        let size = cow_st.size_of().expect("ICE: type has no size");
        let new_ptr = b!(self.bld.build_call(malloc, &[size.into()], "cow.new"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void")
            .into_pointer_value();
        let new_rc = b!(self.bld.build_struct_gep(cow_st, new_ptr, 0, "cow.nrc"));
        b!(self.bld.build_store(new_rc, i64t.const_int(1, false)));
        let new_data = b!(self.bld.build_struct_gep(cow_st, new_ptr, 1, "cow.ndata"));
        let old_data = b!(self.bld.build_struct_gep(cow_st, cow_ptr, 1, "cow.odata"));
        let old_val = b!(self.bld.build_load(data_ty, old_data, "cow.oval"));
        b!(self.bld.build_store(new_data, old_val));
        // Decrement original rc
        let dec = b!(self
            .bld
            .build_int_sub(rc, i64t.const_int(1, false), "cow.dec"));
        b!(self.bld.build_store(rc_gep, dec));
        b!(self.bld.build_unconditional_branch(done_bb));

        // Merge
        self.bld.position_at_end(done_bb);
        let ptr_t = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let phi = b!(self.bld.build_phi(ptr_t, "cow.result"));
        phi.add_incoming(&[(&cow_ptr, cur_bb), (&new_ptr, clone_bb)]);
        Ok(phi.as_basic_value())
    }

    /// Compile `grad(f)` — numerical derivative via central differences.
    /// `f` must be a function `f64 -> f64`. Returns a closure `f64 -> f64`.
    pub(in crate::codegen) fn compile_grad(&mut self, inner: &hir::Expr) -> Result<BasicValueEnum<'ctx>, String> {
        // Compile the inner fn closure
        let f_closure = self.compile_expr(inner)?;

        // Build the derivative wrapper function: (env_ptr, x) -> f64
        // env_ptr points to the original closure stored as {fn_ptr, env_ptr}
        let f64t = self.ctx.f64_type();
        let ptr_t = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let grad_ft = f64t.fn_type(&[ptr_t.into(), f64t.into()], false);
        let grad_fn = self.module.add_function("__grad_wrapper", grad_ft, None);

        let saved_bb = self.bld.get_insert_block();
        let saved_fn = self.cur_fn;
        let entry = self.ctx.append_basic_block(grad_fn, "entry");
        self.bld.position_at_end(entry);
        self.cur_fn = Some(grad_fn);

        let env_arg = grad_fn.get_nth_param(0).expect("ICE: missing param").into_pointer_value();
        let x = grad_fn.get_nth_param(1).expect("ICE: missing param").into_float_value();

        // Load the original closure from the env: {fn_ptr, env_ptr}
        let cl_ty = self.closure_type();
        let orig_cl = b!(self.bld.build_load(cl_ty, env_arg, "orig.cl")).into_struct_value();
        let orig_fn = b!(self.bld.build_extract_value(orig_cl, 0, "orig.fn")).into_pointer_value();
        let orig_env = b!(self.bld.build_extract_value(orig_cl, 1, "orig.env"));

        // Build the inner function type: (env_ptr, f64) -> f64
        let inner_ft = f64t.fn_type(&[ptr_t.into(), f64t.into()], false);

        // h = 1e-8
        let h = f64t.const_float(1e-8);
        let two_h = f64t.const_float(2e-8);

        // x_plus = x + h
        let x_plus = b!(self.bld.build_float_add(x, h, "xp"));
        // x_minus = x - h
        let x_minus = b!(self.bld.build_float_sub(x, h, "xm"));

        // f(x + h)
        let fp = b!(self.bld.build_indirect_call(
            inner_ft,
            orig_fn,
            &[orig_env.into(), x_plus.into()],
            "fp"
        ));
        let fp_val = self.call_result(fp).into_float_value();
        // f(x - h)
        let fm = b!(self.bld.build_indirect_call(
            inner_ft,
            orig_fn,
            &[orig_env.into(), x_minus.into()],
            "fm"
        ));
        let fm_val = self.call_result(fm).into_float_value();
        // (f(x+h) - f(x-h)) / 2h
        let diff = b!(self.bld.build_float_sub(fp_val, fm_val, "diff"));
        let grad_val = b!(self.bld.build_float_div(diff, two_h, "grad"));
        b!(self.bld.build_return(Some(&grad_val)));

        self.cur_fn = saved_fn;
        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }

        // Allocate env holding the original closure, build new closure
        let cl_alloc = self.entry_alloca(cl_ty.into(), "grad.env");
        b!(self.bld.build_store(cl_alloc, f_closure));

        self.make_closure(grad_fn.as_global_value().as_pointer_value(), cl_alloc)
    }
}
