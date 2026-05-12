//! Core MIR instruction emission for constants, arithmetic, calls, variables, and globals.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_core_inst(
        &mut self,
        inst: &mir::Instruction,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        Ok(Some(
            (match &inst.kind {
                mir::InstKind::IntConst(n) => {
                    let llvm_ty = self.llvm_ty(&inst.ty);
                    Ok(match &inst.ty {
                        Type::F32 => self.ctx.f32_type().const_float(*n as f64).into(),
                        Type::F64 => self.ctx.f64_type().const_float(*n as f64).into(),
                        _ => llvm_ty.into_int_type().const_int(*n as u64, true).into(),
                    })
                }
                mir::InstKind::FloatConst(f) => Ok(match &inst.ty {
                    Type::F32 => self.ctx.f32_type().const_float(*f).into(),
                    _ => self.ctx.f64_type().const_float(*f).into(),
                }),
                mir::InstKind::BoolConst(b) => {
                    Ok(self.ctx.bool_type().const_int(*b as u64, false).into())
                }
                mir::InstKind::StringConst(s) => self.emit_string_const(s),
                mir::InstKind::Void => Ok(self.ctx.i8_type().const_int(0, false).into()),

                // ── Arithmetic ──
                mir::InstKind::BinOp(op, lhs, rhs) => self.emit_binop(*op, *lhs, *rhs, &inst.ty),
                mir::InstKind::UnaryOp(op, val) => self.emit_unary(*op, *val, &inst.ty),
                mir::InstKind::Cmp(op, lhs, rhs, operand_ty) => {
                    self.emit_cmp(*op, *lhs, *rhs, operand_ty)
                }

                // ── Calls ──
                mir::InstKind::Call(name, args) => {
                    // Check for magic call names first (coroutines, actors, stores)
                    if let Some(result) =
                        self.try_handle_magic_call(&name.as_str(), args, &inst.ty)?
                    {
                        return Ok(Some(result));
                    }
                    // Handle overflow builtins that MIR lowered as __builtin_* calls
                    if let Some(result) = self.try_handle_overflow_builtin(&name.as_str(), args)? {
                        return Ok(Some(result));
                    }
                    let arg_vals: Vec<BasicValueEnum<'ctx>> =
                        args.iter().map(|a| self.val(*a)).collect();
                    if let Some((fv, _, _)) = self.fns.get(name).cloned() {
                        let ptypes = fv.get_type().get_param_types();
                        let st = self.string_type();
                        let md: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = arg_vals
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                if let Some(pt) = ptypes.get(i) {
                                    if v.get_type() == st.into() && pt.is_pointer_type() {
                                        self.string_data(*v).unwrap_or(*v).into()
                                    } else {
                                        (*v).into()
                                    }
                                } else {
                                    (*v).into()
                                }
                            })
                            .collect();
                        let csv = b!(self.bld.build_call(fv, &md, "call"));
                        Ok(self.call_result(csv))
                    } else {
                        // Try looking up as a module-level function.
                        if let Some(fv) = self.module.get_function(&name.as_str()) {
                            let ptypes = fv.get_type().get_param_types();
                            let st = self.string_type();
                            let md: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = arg_vals
                                .iter()
                                .enumerate()
                                .map(|(i, v)| {
                                    if let Some(pt) = ptypes.get(i) {
                                        if v.get_type() == st.into() && pt.is_pointer_type() {
                                            self.string_data(*v).unwrap_or(*v).into()
                                        } else {
                                            (*v).into()
                                        }
                                    } else {
                                        (*v).into()
                                    }
                                })
                                .collect();
                            let csv = b!(self.bld.build_call(fv, &md, "call"));
                            Ok(self.call_result(csv))
                        } else {
                            // libm fallback: declare and call directly.
                            const LIBM_UNARY_F64: &[&str] = &[
                                "fabs", "sqrt", "floor", "ceil", "round", "trunc", "sin",
                                "cos", "tan", "asin", "acos", "atan", "log", "log10", "log2",
                                "exp", "exp2",
                            ];
                            const LIBM_BINARY_F64: &[&str] = &["pow", "atan2", "fmod", "copysign"];
                            let name_str = name.as_str();
                            let f64t = self.ctx.f64_type();
                            if LIBM_UNARY_F64.contains(&&*name_str) && arg_vals.len() == 1 {
                                let sig = f64t.fn_type(&[f64t.into()], false);
                                let fv = self.module.add_function(&name_str, sig, None);
                                let csv = b!(self.bld.build_call(
                                    fv,
                                    &[arg_vals[0].into()],
                                    "libm"
                                ));
                                return Ok(Some(self.call_result(csv)));
                            }
                            if LIBM_BINARY_F64.contains(&&*name_str) && arg_vals.len() == 2 {
                                let sig = f64t.fn_type(&[f64t.into(), f64t.into()], false);
                                let fv = self.module.add_function(&name_str, sig, None);
                                let csv = b!(self.bld.build_call(
                                    fv,
                                    &[arg_vals[0].into(), arg_vals[1].into()],
                                    "libm"
                                ));
                                return Ok(Some(self.call_result(csv)));
                            }
                            Err(format!("unknown function `{name}`"))
                        }
                    }
                }
                mir::InstKind::MethodCall(recv, method, args) => {
                    // Try vec/array methods first (these are inline, not compiled functions)
                    let recv_ty = self.value_types.get(recv).cloned();

                    // String methods
                    if matches!(&recv_ty, Some(Type::String)) {
                        let recv_val = self.val(*recv);
                        match &*method.as_str() {
                            "length" | "len" => return Ok(Some((self.string_len(recv_val))?)),
                            "contains" => {
                                if !args.is_empty() {
                                    let a = self.val(args[0]);
                                    return Ok(Some((self.string_contains(recv_val, a))?));
                                }
                            }
                            "starts_with" => {
                                if !args.is_empty() {
                                    let a = self.val(args[0]);
                                    return Ok(Some((self.string_starts_with(recv_val, a))?));
                                }
                            }
                            "ends_with" => {
                                if !args.is_empty() {
                                    let a = self.val(args[0]);
                                    return Ok(Some((self.string_ends_with(recv_val, a))?));
                                }
                            }
                            "char_at" => {
                                if !args.is_empty() {
                                    let a = self.val(args[0]);
                                    return Ok(Some((self.string_char_at(recv_val, a))?));
                                }
                            }
                            "slice" => {
                                if args.len() >= 2 {
                                    let start = self.val(args[0]);
                                    let end = self.val(args[1]);
                                    return Ok(Some((self.string_slice(recv_val, start, end))?));
                                }
                            }
                            "find" => {
                                if !args.is_empty() {
                                    let a = self.val(args[0]);
                                    return Ok(Some((self.string_find(recv_val, a))?));
                                }
                            }
                            "trim" => return Ok(Some((self.string_trim(recv_val, true, true))?)),
                            "trim_left" => {
                                return Ok(Some((self.string_trim(recv_val, true, false))?));
                            }
                            "trim_right" => {
                                return Ok(Some((self.string_trim(recv_val, false, true))?));
                            }
                            "to_upper" => return Ok(Some((self.string_case(recv_val, true))?)),
                            "to_lower" => return Ok(Some((self.string_case(recv_val, false))?)),
                            "replace" => {
                                if args.len() >= 2 {
                                    let old = self.val(args[0]);
                                    let new = self.val(args[1]);
                                    return Ok(Some((self.string_replace(recv_val, old, new))?));
                                }
                            }
                            "split" => {
                                if !args.is_empty() {
                                    let delim = self.val(args[0]);
                                    return Ok(Some((self.string_split(recv_val, delim))?));
                                }
                            }
                            "lines" => {
                                let newline = self.compile_str_literal("\n")?;
                                return Ok(Some((self.string_split(recv_val, newline))?));
                            }
                            "repeat" => {
                                if !args.is_empty() {
                                    let count = self.val(args[0]);
                                    return Ok(Some((self.string_repeat(recv_val, count))?));
                                }
                            }
                            "is_empty" => {
                                let len = self.string_len(recv_val)?.into_int_value();
                                let i64t = self.ctx.i64_type();
                                let cmp = b!(self.bld.build_int_compare(
                                    inkwell::IntPredicate::EQ,
                                    len,
                                    i64t.const_int(0, false),
                                    "isempty"
                                ));
                                return Ok(Some(cmp.into()));
                            }
                            _ => {} // fall through to function lookup
                        }
                    }

                    let is_vec_or_array =
                        matches!(&recv_ty, Some(Type::Vec(_)) | Some(Type::Array(_, _)));
                    if is_vec_or_array {
                        let recv_val = self.val(*recv);
                        let elem_ty = match &recv_ty {
                            Some(Type::Vec(et)) => *et.clone(),
                            Some(Type::Array(et, _)) => *et.clone(),
                            _ => Type::I64,
                        };
                        // Fixed-size array: len returns constant, contains is inline scan
                        if let Some(Type::Array(_, arr_len)) = recv_ty {
                            match &*method.as_str() {
                                "len" => {
                                    return Ok(Some(
                                        self.ctx.i64_type().const_int(arr_len as u64, false).into(),
                                    ));
                                }
                                _ => {}
                            }
                        }
                        let header_ptr = if recv_val.is_pointer_value() {
                            recv_val.into_pointer_value()
                        } else {
                            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                            b!(self.bld.build_int_to_ptr(
                                recv_val.into_int_value(),
                                ptr_ty,
                                "vec.ptr"
                            ))
                        };
                        let lty = self.llvm_ty(&elem_ty);
                        match &*method.as_str() {
                            "len" | "count" => return Ok(Some((self.vec_len(header_ptr))?)),
                            "push" => {
                                if !args.is_empty() {
                                    let val = self.val(args[0]);
                                    let elem_size = self.type_store_size(lty);
                                    self.vec_push_raw(header_ptr, val, lty, elem_size)?;
                                    return Ok(Some(self.ctx.i8_type().const_int(0, false).into()));
                                }
                                return Err("push() requires an argument".into());
                            }
                            "pop" => return Ok(Some((self.vec_pop(header_ptr, &elem_ty))?)),
                            "get" => {
                                if !args.is_empty() {
                                    let idx = self.val(args[0]).into_int_value();
                                    return Ok(Some(
                                        (self.vec_get_idx(header_ptr, &elem_ty, idx))?,
                                    ));
                                }
                                return Err("get() requires an index".into());
                            }
                            "collect" => return Ok(Some(recv_val)),
                            "set" => {
                                if args.len() >= 2 {
                                    let idx = self.val(args[0]).into_int_value();
                                    let val = self.val(args[1]);
                                    return Ok(Some(
                                        (self.vec_set_val(header_ptr, &elem_ty, idx, val))?,
                                    ));
                                }
                                return Err("set() requires index and value".into());
                            }
                            "remove" => {
                                if !args.is_empty() {
                                    let idx = self.val(args[0]).into_int_value();
                                    return Ok(Some(
                                        (self.vec_remove_val(header_ptr, &elem_ty, idx))?,
                                    ));
                                }
                                return Err("remove() requires an index".into());
                            }
                            "clear" => return Ok(Some((self.vec_clear(header_ptr))?)),
                            "map" | "filter" => {
                                if args.is_empty() {
                                    return Err(format!(
                                        "{}() requires a callback",
                                        method.as_str()
                                    ));
                                }
                                let closure_val = self.val(args[0]);
                                let closure_ty = self
                                    .value_types
                                    .get(&args[0])
                                    .cloned()
                                    .ok_or_else(|| {
                                        format!(
                                            "missing closure type for `{}` callback",
                                            method.as_str()
                                        )
                                    })?;
                                let result = if &*method.as_str() == "map" {
                                    self.vec_map_dynamic(
                                        header_ptr,
                                        &elem_ty,
                                        closure_val,
                                        &closure_ty,
                                    )?
                                } else {
                                    self.vec_filter_dynamic(
                                        header_ptr,
                                        &elem_ty,
                                        closure_val,
                                        &closure_ty,
                                    )?
                                };
                                return Ok(Some(result));
                            }
                            "fold" | "reduce" => {
                                if args.len() < 2 {
                                    return Err(
                                        "fold() requires (initial, callback)".into()
                                    );
                                }
                                let init_val = self.val(args[0]);
                                let closure_val = self.val(args[1]);
                                let closure_ty = self
                                    .value_types
                                    .get(&args[1])
                                    .cloned()
                                    .ok_or_else(|| {
                                        "missing closure type for fold callback"
                                            .to_string()
                                    })?;
                                return Ok(Some(self.vec_fold_dynamic(
                                    header_ptr,
                                    &elem_ty,
                                    init_val,
                                    closure_val,
                                    &closure_ty,
                                )?));
                            }
                            "find" => {
                                if args.is_empty() {
                                    return Err("find() requires a callback".into());
                                }
                                let closure_val = self.val(args[0]);
                                let closure_ty = self
                                    .value_types
                                    .get(&args[0])
                                    .cloned()
                                    .ok_or_else(|| {
                                        "missing closure type for find callback".to_string()
                                    })?;
                                return Ok(Some(self.vec_find_dynamic(
                                    header_ptr,
                                    &elem_ty,
                                    closure_val,
                                    &closure_ty,
                                )?));
                            }
                            "any" | "all" => {
                                if args.is_empty() {
                                    return Err(format!(
                                        "{}() requires a callback",
                                        method.as_str()
                                    ));
                                }
                                let closure_val = self.val(args[0]);
                                let closure_ty = self
                                    .value_types
                                    .get(&args[0])
                                    .cloned()
                                    .ok_or_else(|| {
                                        format!(
                                            "missing closure type for `{}` callback",
                                            method.as_str()
                                        )
                                    })?;
                                let is_any = &*method.as_str() == "any";
                                return Ok(Some(self.vec_any_all_dynamic(
                                    header_ptr,
                                    &elem_ty,
                                    closure_val,
                                    &closure_ty,
                                    is_any,
                                )?));
                            }
                            "sum" => return Ok(Some(self.vec_sum(header_ptr, &elem_ty)?)),
                            "sort" => {
                                return Ok(Some(self.vec_sort(header_ptr, &elem_ty)?));
                            }
                            "reverse" => {
                                return Ok(Some(self.vec_reverse(header_ptr, &elem_ty)?));
                            }
                            "contains" => {
                                if args.is_empty() {
                                    return Err(
                                        "contains() requires a needle".into()
                                    );
                                }
                                let needle = self.val(args[0]);
                                return Ok(Some(self.vec_contains_v(
                                    header_ptr, &elem_ty, needle,
                                )?));
                            }
                            "join" => {
                                if args.is_empty() {
                                    return Err(
                                        "join() requires a separator".into()
                                    );
                                }
                                let sep = self.val(args[0]);
                                return Ok(Some(self.vec_join_v(header_ptr, sep)?));
                            }
                            "take" | "skip" | "drop" => {
                                if args.is_empty() {
                                    return Err(format!(
                                        "{}() requires a count",
                                        method.as_str()
                                    ));
                                }
                                let n = self.val(args[0]).into_int_value();
                                let is_take = &*method.as_str() == "take";
                                return Ok(Some(self.vec_take_skip_v(
                                    header_ptr, &elem_ty, n, is_take,
                                )?));
                            }
                            "slice" => {
                                if args.len() < 2 {
                                    return Err(
                                        "slice() requires (start, end)".into()
                                    );
                                }
                                let s = self.val(args[0]).into_int_value();
                                let e = self.val(args[1]).into_int_value();
                                return Ok(Some(self.vec_slice_v(
                                    header_ptr, &elem_ty, s, e,
                                )?));
                            }
                            "zip" => {
                                if args.is_empty() {
                                    return Err(
                                        "zip() requires another vector".into()
                                    );
                                }
                                let other_val = self.val(args[0]);
                                let other_ptr = if other_val.is_pointer_value() {
                                    other_val.into_pointer_value()
                                } else {
                                    let pt = self.ctx.ptr_type(AddressSpace::default());
                                    b!(self.bld.build_int_to_ptr(
                                        other_val.into_int_value(),
                                        pt,
                                        "zip.optr"
                                    ))
                                };
                                let other_elem_ty = match self
                                    .value_types
                                    .get(&args[0])
                                    .cloned()
                                {
                                    Some(Type::Vec(et)) => *et,
                                    Some(Type::Array(et, _)) => *et,
                                    _ => Type::I64,
                                };
                                return Ok(Some(self.vec_zip_v(
                                    header_ptr,
                                    &elem_ty,
                                    other_ptr,
                                    &other_elem_ty,
                                )?));
                            }
                            _ => {} // fall through to function lookup
                        }
                    }
                    let is_map = matches!(&recv_ty, Some(Type::Map(_, _)))
                        || matches!(&recv_ty, Some(Type::Struct(n, _)) if n.starts_with("Map_"));
                    if is_map {
                        let recv_val = self.val(*recv);
                        let header_ptr = if recv_val.is_pointer_value() {
                            recv_val.into_pointer_value()
                        } else {
                            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                            b!(self.bld.build_int_to_ptr(
                                recv_val.into_int_value(),
                                ptr_ty,
                                "map.ptr"
                            ))
                        };
                        match &*method.as_str() {
                            "len" | "count" => return Ok(Some((self.vec_len(header_ptr))?)),
                            "set" => {
                                if args.len() >= 2 {
                                    let k = self.val(args[0]);
                                    let v = self.val(args[1]);
                                    return Ok(Some((self.map_set_val(header_ptr, k, v))?));
                                }
                                return Err("map.set() requires key and value".into());
                            }
                            "get" => {
                                if !args.is_empty() {
                                    let k = self.val(args[0]);
                                    return Ok(Some((self.map_get_val(header_ptr, k))?));
                                }
                                return Err("map.get() requires a key".into());
                            }
                            "has" | "contains" => {
                                if !args.is_empty() {
                                    let k = self.val(args[0]);
                                    return Ok(Some((self.map_has_val(header_ptr, k))?));
                                }
                                return Err("map.has() requires a key".into());
                            }
                            "remove" => {
                                if !args.is_empty() {
                                    let k = self.val(args[0]);
                                    return Ok(Some((self.map_remove_val(header_ptr, k))?));
                                }
                                return Err("map.remove() requires a key".into());
                            }
                            "clear" => return Ok(Some((self.map_clear(header_ptr))?)),
                            _ => {} // fall through
                        }
                    }

                    let recv_val = self.val(*recv);
                    if let Some((fv, _, _)) = self.fns.get(method).cloned() {
                        // Check if the method expects self by pointer (first param is ptr type)
                        let first_param_is_ptr = fv
                            .get_type()
                            .get_param_types()
                            .first()
                            .map(|t| t.is_pointer_type())
                            .unwrap_or(false);
                        let self_arg: BasicValueEnum<'ctx> =
                            if first_param_is_ptr && !recv_val.is_pointer_value() {
                                // Struct value but method expects pointer: alloca + store.
                                // Cache the alloca so mutations from the method persist across calls
                                // (e.g. iterator .next() mutating self.n in a loop).
                                if let Some(cached) = self.self_allocs.get(recv) {
                                    (*cached).into()
                                } else {
                                    let tmp = self.entry_alloca(recv_val.get_type(), "self.tmp");
                                    // Store the initial value into the alloca.  We must place
                                    // this store in the entry block so it only runs once —
                                    // otherwise a loop would re-init the alloca every iteration,
                                    // clobbering mutations made by the method.
                                    //
                                    // If recv_val was produced in a later block (e.g. via
                                    // insertvalue in a branch), it won't dominate the entry
                                    // block.  In that case, fall back to storing at the
                                    // current position — this is correct for non-loop cases.
                                    let cur_fn = self.cur_fn.expect("ICE: cur_fn not set");
                                    let entry_bb = cur_fn
                                        .get_first_basic_block()
                                        .expect("ICE: function has no entry block");
                                    let _cur_bb = self
                                        .bld
                                        .get_insert_block()
                                        .expect("ICE: builder has no insert block");
                                    let recv_in_entry =
                                        if let Some(inst) = recv_val.as_instruction_value() {
                                            inst.get_parent().map_or(false, |bb| bb == entry_bb)
                                        } else {
                                            true // constants dominate everything
                                        };
                                    if recv_in_entry {
                                        let entry_bld = self.ctx.create_builder();
                                        if let Some(term) = entry_bb.get_terminator() {
                                            entry_bld.position_before(&term);
                                        } else {
                                            entry_bld.position_at_end(entry_bb);
                                        }
                                        entry_bld.build_store(tmp, recv_val).unwrap();
                                    } else {
                                        b!(self.bld.build_store(tmp, recv_val));
                                    }
                                    self.self_allocs.insert(*recv, tmp);
                                    self.self_alloc_types.insert(*recv, recv_val.get_type());
                                    tmp.into()
                                }
                            } else {
                                recv_val
                            };
                        let mut all_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                            vec![self_arg.into()];
                        for a in args {
                            all_args.push(self.val(*a).into());
                        }
                        let csv = b!(self.bld.build_call(fv, &all_args, "mcall"));
                        // After a method call that may have mutated self through the alloca pointer,
                        // update the value map to point to the alloca pointer.
                        // FieldGet/FieldSet already handle pointer values via GEP,
                        // and subsequent method calls will pass the pointer directly.
                        // We avoid reloading the struct here because the reload would be
                        // placed in the current block and may not dominate later uses.
                        if let Some(alloca_ptr) = self.self_allocs.get(recv).copied() {
                            self.value_map.insert(*recv, alloca_ptr.into());
                        }
                        Ok(self.call_result(csv))
                    } else {
                        Err(format!("unknown method `{method}`"))
                    }
                }
                mir::InstKind::IndirectCall(callee, args) => {
                    let callee_val = self.val(*callee);
                    // Closure call: callee is a {fn_ptr, env_ptr} struct.
                    let _closure_ty = self.closure_type();
                    let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                    let fn_ptr = b!(self.bld.build_extract_value(
                        callee_val.into_struct_value(),
                        0,
                        "fn_ptr"
                    ))
                    .into_pointer_value();
                    let env_ptr = b!(self.bld.build_extract_value(
                        callee_val.into_struct_value(),
                        1,
                        "env_ptr"
                    ));
                    let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                        vec![env_ptr.into()];
                    for a in args {
                        call_args.push(self.val(*a).into());
                    }
                    // Build function type for the indirect call.
                    let ret_llvm = self.llvm_ty(&inst.ty);
                    let mut param_tys: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
                    for a in args {
                        param_tys.push(self.val(*a).get_type().into());
                    }
                    let ft = ret_llvm.fn_type(&param_tys, false);
                    let csv = b!(self
                        .bld
                        .build_indirect_call(ft, fn_ptr, &call_args, "icall"));
                    Ok(self.call_result(csv))
                }

                // ── Variables ──
                mir::InstKind::FnRef(name) => {
                    // Create a closure struct {fn_ptr, null_env} wrapping the named function.
                    // A wrapper is needed because closures expect (env_ptr, ...params) calling convention,
                    // but top-level functions only expect (...params).
                    if let Some(fv) = self.module.get_function(&name.as_str()) {
                        let wrapper = self.fn_ref_wrapper(fv);
                        let null_env = self
                            .ctx
                            .ptr_type(inkwell::AddressSpace::default())
                            .const_null();
                        self.make_closure(wrapper, null_env)
                    } else {
                        Err(format!("undefined function `{name}` in FnRef"))
                    }
                }
                mir::InstKind::Load(name) => {
                    if let Some((ptr, ty)) = self.var_allocs.get(name).cloned() {
                        let lt = self.llvm_ty(&ty);
                        let val = b!(self.bld.build_load(lt, ptr, &name.as_str()));
                        if let Some(inst) = val.as_instruction_value() {
                            let tbaa_name = Compiler::tbaa_type_name(&ty);
                            self.set_tbaa(inst, tbaa_name);
                        }
                        Ok(val)
                    } else {
                        // Fall back to Compiler's var lookup.
                        if let Some((ptr, ty)) = self.find_var(&name.as_str()).cloned() {
                            let lt = self.llvm_ty(&ty);
                            let val = b!(self.bld.build_load(lt, ptr, &name.as_str()));
                            if let Some(inst) = val.as_instruction_value() {
                                let tbaa_name = Compiler::tbaa_type_name(&ty);
                                self.set_tbaa(inst, tbaa_name);
                            }
                            Ok(val)
                        } else {
                            Err(format!("Load of undefined variable `{name}`"))
                        }
                    }
                }
                mir::InstKind::Store(name, val) => {
                    let v = self.val(*val);
                    if let Some((ptr, ty)) = self.var_allocs.get(name).cloned() {
                        let store_inst = b!(self.bld.build_store(ptr, v));
                        let tbaa_name = Compiler::tbaa_type_name(&ty);
                        self.set_tbaa(store_inst, tbaa_name);
                    } else {
                        // First store → create alloca.
                        let lt = v.get_type();
                        let ty = inst.ty.clone();
                        let ptr = self.entry_alloca(lt, &name.as_str());
                        let store_inst = b!(self.bld.build_store(ptr, v));
                        let tbaa_name = Compiler::tbaa_type_name(&ty);
                        self.set_tbaa(store_inst, tbaa_name);
                        self.var_allocs.insert(*name, (ptr, ty.clone()));
                        self.set_var(&name.as_str(), ptr, ty);
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }

                // ── Globals ──
                mir::InstKind::GlobalLoad(name) => {
                    if let Some((gv, ty)) = self.globals.get(name).cloned() {
                        let lt = self.llvm_ty(&ty);
                        let val =
                            b!(self
                                .bld
                                .build_load(lt, gv.as_pointer_value(), &name.as_str()));
                        if let Some(inst) = val.as_instruction_value() {
                            self.set_tbaa(inst, Compiler::tbaa_type_name(&ty));
                        }
                        Ok(val)
                    } else {
                        Err(format!(
                            "GlobalLoad of undefined global `{name}`"
                        ))
                    }
                }
                _ => return Ok(None),
            })?,
        ))
    }
}
