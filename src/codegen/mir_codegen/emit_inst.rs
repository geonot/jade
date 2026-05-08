//! MIR instruction dispatch and value emission.

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
                            Err(format!("mir_codegen: unknown function `{name}`"))
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
                        Err(format!("mir_codegen: unknown method `{method}`"))
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
                        Err(format!("mir_codegen: undefined function `{name}` in FnRef"))
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
                            Err(format!("mir_codegen: Load of undefined variable `{name}`"))
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
                            "mir_codegen: GlobalLoad of undefined global `{name}`"
                        ))
                    }
                }
                _ => return Ok(None),
            })?,
        ))
    }
}

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_aggregate_memory_inst(
        &mut self,
        inst: &mir::Instruction,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        Ok(Some(
            (match &inst.kind {
                mir::InstKind::GlobalStore(name, val_id) => {
                    let v = self.val(*val_id);
                    if let Some((gv, ty)) = self.globals.get(name).cloned() {
                        let si = b!(self.bld.build_store(gv.as_pointer_value(), v));
                        self.set_tbaa(si, Compiler::tbaa_type_name(&ty));
                        Ok(self.ctx.i8_type().const_int(0, false).into())
                    } else {
                        Err(format!(
                            "mir_codegen: GlobalStore to undefined global `{name}`"
                        ))
                    }
                }

                // ── Struct/Aggregate ──
                mir::InstKind::StructInit(name, fields) => {
                    let st = self
                        .module
                        .get_struct_type(&name.as_str())
                        .ok_or_else(|| format!("mir_codegen: unknown struct `{name}`"))?;
                    let field_defs: Vec<(String, Type)> =
                        self.structs.get(name).cloned().unwrap_or_default();
                    let defaults = self.struct_defaults.get(name).cloned();
                    let mut agg: BasicValueEnum<'ctx> = st.const_zero().into();
                    // Track which field indices were explicitly provided.
                    let mut provided = std::collections::HashSet::new();
                    for (i, (fname, vid)) in fields.iter().enumerate() {
                        let v = self.val(*vid);
                        let idx = if fname.is_empty() {
                            provided.insert(i as u32);
                            i as u32
                        } else {
                            let pos = field_defs
                                .iter()
                                .position(|(n, _)| fname.with_str(|s| n == s))
                                .ok_or_else(|| {
                                    format!("mir_codegen: struct `{name}` has no field `{fname}`")
                                })? as u32;
                            provided.insert(pos);
                            pos
                        };
                        let label = if fname.is_empty() {
                            field_defs.get(i).map(|s| s.0.as_str()).unwrap_or("field")
                        } else {
                            &fname.as_str()
                        };
                        agg =
                            b!(self
                                .bld
                                .build_insert_value(agg.into_struct_value(), v, idx, label))
                            .into_struct_value()
                            .into();
                    }
                    // Fill in defaults for missing fields.
                    for (i, (fname, fty)) in field_defs.iter().enumerate() {
                        let idx = i as u32;
                        if provided.contains(&idx) {
                            continue;
                        }
                        let val =
                            if let Some(def_expr) = defaults.as_ref().and_then(|d| d.get(fname)) {
                                self.compile_const_expr(def_expr)?
                            } else {
                                self.default_val(fty)
                            };
                        agg = b!(self.bld.build_insert_value(
                            agg.into_struct_value(),
                            val,
                            idx,
                            fname
                        ))
                        .into_struct_value()
                        .into();
                    }
                    Ok(agg)
                }
                mir::InstKind::VariantInit(enum_name, variant, tag, payload) => {
                    let enum_ty = self.llvm_ty(&Type::Enum(*enum_name));
                    let st = enum_ty.into_struct_type();
                    let i32t = self.ctx.i32_type();
                    let mut agg: BasicValueEnum<'ctx> = st.const_zero().into();
                    // Field 0 = tag.
                    agg = b!(self.bld.build_insert_value(
                        agg.into_struct_value(),
                        i32t.const_int(*tag as u64, false),
                        0,
                        "tag"
                    ))
                    .into_struct_value()
                    .into();
                    // Payload into field 1 (stored as a byte array, need to bitcast via alloca).
                    if !payload.is_empty() {
                        let alloca = self.entry_alloca(enum_ty, "variant.tmp");
                        b!(self.bld.build_store(alloca, agg));
                        let payload_gep = b!(self.bld.build_struct_gep(st, alloca, 1, "payload"));
                        // Look up variant field types for recursive-field detection.
                        let variant_field_types: Vec<Type> = self
                            .enums
                            .get(enum_name)
                            .and_then(|vs| vs.iter().find(|(vn, _)| variant.with_str(|s| vn == s)))
                            .map(|(_, ftys)| ftys.clone())
                            .unwrap_or_default();
                        // Store payload fields at proper byte offsets based on actual type sizes.
                        let mut byte_offset: u64 = 0;
                        for (i, vid) in payload.iter().enumerate() {
                            let v = self.val(*vid);
                            let is_rec = variant_field_types
                                .get(i)
                                .map(|fty| Compiler::is_recursive_field(fty, &enum_name.as_str()))
                                .unwrap_or(false);
                            let field_ptr = if byte_offset == 0 {
                                payload_gep
                            } else {
                                let offset_val = self.ctx.i64_type().const_int(byte_offset, false);
                                unsafe {
                                    b!(self.bld.build_gep(
                                        self.ctx.i8_type(),
                                        payload_gep,
                                        &[offset_val],
                                        "payload.elem"
                                    ))
                                }
                            };
                            if is_rec {
                                // Box the recursive field: malloc, store value, store pointer.
                                let actual_ty =
                                    self.llvm_ty(variant_field_types.get(i).unwrap_or(&Type::I64));
                                let size = self.type_store_size(actual_ty);
                                let malloc_fn = self.ensure_malloc();
                                let heap = b!(self.bld.build_call(
                                    malloc_fn,
                                    &[self.ctx.i64_type().const_int(size, false).into()],
                                    "box.alloc"
                                ))
                                .try_as_basic_value()
                                .basic()
                                .expect("ICE: call returned void")
                                .into_pointer_value();
                                b!(self.bld.build_store(heap, v));
                                b!(self.bld.build_store(field_ptr, heap));
                                byte_offset += 8;
                            } else {
                                b!(self.bld.build_store(field_ptr, v));
                                let type_size = v
                                    .get_type()
                                    .size_of()
                                    .map(|s| s.get_zero_extended_constant().unwrap_or(8))
                                    .unwrap_or(8);
                                byte_offset += (type_size + 7) & !7;
                            }
                        }
                        agg = b!(self.bld.build_load(enum_ty, alloca, "variant.loaded"));
                    }
                    Ok(agg)
                }
                mir::InstKind::ArrayInit(elems) => {
                    if elems.is_empty() {
                        let arr_ty = self.llvm_ty(&inst.ty);
                        return Ok(Some(arr_ty.const_zero()));
                    }
                    let elem_vals: Vec<BasicValueEnum<'ctx>> =
                        elems.iter().map(|v| self.val(*v)).collect();
                    let elem_ty = elem_vals[0].get_type();
                    let arr_ty = elem_ty.array_type(elems.len() as u32);
                    let alloca = self.entry_alloca(arr_ty.into(), "arr");
                    for (i, v) in elem_vals.iter().enumerate() {
                        let idx = self.ctx.i64_type().const_int(i as u64, false);
                        let zero = self.ctx.i64_type().const_int(0, false);
                        let ptr = unsafe {
                            b!(self.bld.build_gep(arr_ty, alloca, &[zero, idx], "arr.elem"))
                        };
                        b!(self.bld.build_store(ptr, *v));
                    }
                    Ok(b!(self.bld.build_load(arr_ty, alloca, "arr.val")).into())
                }

                // ── Field access ──
                mir::InstKind::FieldGet(obj, field) => {
                    self.emit_field_get(*obj, &field.as_str(), &inst.ty)
                }
                mir::InstKind::FieldSet(obj, field, val) => {
                    // If the object has a self_allocs entry, use the alloca pointer directly
                    // to avoid SSA domination issues with insertvalue across branches.
                    let obj_val = if let Some(alloca_ptr) = self.self_allocs.get(obj).copied() {
                        alloca_ptr.into()
                    } else {
                        self.val(*obj)
                    };
                    let v = self.val(*val);
                    if obj_val.is_pointer_value() {
                        // obj is a pointer to a struct (alloca).
                        // inst.ty carries the struct type from lowering.
                        let struct_name = self.struct_name_from_type(&inst.ty).or_else(|| {
                            // Also try var_allocs for the struct name.
                            self.var_allocs
                                .values()
                                .find(|(ptr, _)| *ptr == obj_val.into_pointer_value())
                                .and_then(|(_, ty)| match ty {
                                    Type::Struct(name, _) => Some(name.as_str()),
                                    _ => None,
                                })
                        });
                        if let Some(name) = &struct_name {
                            if let Some(st) = self.module.get_struct_type(name) {
                                let field_idx = self.field_index(name, &field.as_str());
                                let gep = b!(self.bld.build_struct_gep(
                                    st,
                                    obj_val.into_pointer_value(),
                                    field_idx,
                                    &field.as_str()
                                ));
                                b!(self.bld.build_store(gep, v));
                            }
                        }
                        // Return the pointer so MIR SSA chaining of field assignments
                        // continues to target the same struct (e.g. self.a is X; self.b is Y).
                        return Ok(Some(obj_val));
                    } else if obj_val.is_struct_value() {
                        // SSA struct value — use insert_value for immutable update.
                        let sv = obj_val.into_struct_value();
                        let struct_ty_name = sv
                            .get_type()
                            .get_name()
                            .map(|n| n.to_str().unwrap_or("").to_string());
                        if let Some(name) = &struct_ty_name {
                            let field_idx = self.field_index(name, &field.as_str());
                            let updated =
                                b!(self
                                    .bld
                                    .build_insert_value(sv, v, field_idx, &field.as_str()));
                            return Ok(Some(updated.into_struct_value().into()));
                        }
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::FieldStore(var_name, field, val) => {
                    // Direct field store into a named variable's alloca.
                    let v = self.val(*val);
                    if let Some((alloca, ty)) = self.var_allocs.get(var_name).cloned() {
                        let struct_name = self.struct_name_from_type(&ty);
                        if let Some(name) = &struct_name {
                            if let Some(st) = self.module.get_struct_type(name) {
                                let field_idx = self.field_index(name, &field.as_str());
                                let gep = b!(self.bld.build_struct_gep(
                                    st,
                                    alloca,
                                    field_idx,
                                    &field.as_str()
                                ));
                                b!(self.bld.build_store(gep, v));
                            }
                        }
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }

                // ── Indexing ──
                mir::InstKind::Index(base, idx) | mir::InstKind::IndexUnchecked(base, idx) => {
                    let base_val = self.val(*base);
                    let idx_val = self.val(*idx);
                    let base_ty = self.value_types.get(base);
                    let unchecked = matches!(&inst.kind, mir::InstKind::IndexUnchecked(_, _));

                    // String indexing: get char at index (returns byte as i64)
                    if matches!(base_ty, Some(Type::String)) {
                        return Ok(Some((self.string_char_at(base_val, idx_val))?));
                    }

                    // For arrays: GEP into the array.
                    if base_val.get_type().is_array_type() {
                        let arr_ty = base_val.get_type().into_array_type();
                        let arr_len = arr_ty.len() as u64;
                        let i64t = self.ctx.i64_type();
                        let final_idx = if unchecked {
                            idx_val.into_int_value()
                        } else {
                            // Wrap negative indices: if idx < 0, idx = len + idx
                            let idx_int = idx_val.into_int_value();
                            let is_neg = b!(self.bld.build_int_compare(
                                inkwell::IntPredicate::SLT,
                                idx_int,
                                i64t.const_int(0, false),
                                "neg"
                            ));
                            let wrapped = b!(self.bld.build_int_nsw_add(
                                idx_int,
                                i64t.const_int(arr_len, false),
                                "wrap"
                            ));
                            b!(self.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                                .into_int_value()
                        };
                        let alloca = self.entry_alloca(arr_ty.into(), "idx.tmp");
                        b!(self.bld.build_store(alloca, base_val));
                        let zero = i64t.const_int(0, false);
                        let ptr = unsafe {
                            b!(self
                                .bld
                                .build_gep(arr_ty, alloca, &[zero, final_idx], "idx.ptr"))
                        };
                        let elem_ty = self.llvm_ty(&inst.ty);
                        Ok(b!(self.bld.build_load(elem_ty, ptr, "idx.val")))
                    } else if base_val.get_type().is_pointer_type() {
                        // Vec indexing: header is { ptr, len, cap }.
                        let header_ptr = base_val.into_pointer_value();
                        let header_ty = self.vec_header_type();
                        let elem_ty = self.llvm_ty(&inst.ty);
                        let i64t = self.ctx.i64_type();
                        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                        let ptr_gep = b!(self
                            .bld
                            .build_struct_gep(header_ty, header_ptr, 0, "vi.ptrp"));
                        let data_ptr = b!(self.bld.build_load(ptr_ty, ptr_gep, "vi.data"))
                            .into_pointer_value();
                        let final_idx = if unchecked {
                            idx_val.into_int_value()
                        } else {
                            let len_gep = b!(self
                                .bld
                                .build_struct_gep(header_ty, header_ptr, 1, "vi.lenp"));
                            let len =
                                b!(self.bld.build_load(i64t, len_gep, "vi.len")).into_int_value();
                            // Wrap negative indices: if idx < 0, idx = len + idx
                            let idx_int = idx_val.into_int_value();
                            let is_neg = b!(self.bld.build_int_compare(
                                inkwell::IntPredicate::SLT,
                                idx_int,
                                i64t.const_int(0, false),
                                "neg"
                            ));
                            let wrapped = b!(self.bld.build_int_nsw_add(idx_int, len, "wrap"));
                            let final_idx =
                                b!(self.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                                    .into_int_value();
                            self.emit_vec_bounds_check(final_idx, len)?;
                            final_idx
                        };
                        let elem_gep = unsafe {
                            b!(self
                                .bld
                                .build_gep(elem_ty, data_ptr, &[final_idx], "vi.egep"))
                        };
                        Ok(b!(self.bld.build_load(elem_ty, elem_gep, "vi.elem")))
                    } else if base_val.is_struct_value() {
                        // Tuple indexing: extract element from struct value.
                        if let Some(idx_const) =
                            idx_val.into_int_value().get_zero_extended_constant()
                        {
                            let elem = b!(self.bld.build_extract_value(
                                base_val.into_struct_value(),
                                idx_const as u32,
                                "tup.elem"
                            ));
                            Ok(elem)
                        } else {
                            // Dynamic index: store to alloca and GEP.
                            let st = base_val.get_type();
                            let alloca = self.entry_alloca(st, "tup.idx");
                            b!(self.bld.build_store(alloca, base_val));
                            let elem_ty = self.llvm_ty(&inst.ty);
                            let zero = self.ctx.i64_type().const_int(0, false);
                            let ptr = unsafe {
                                b!(self.bld.build_gep(
                                    st,
                                    alloca,
                                    &[zero, idx_val.into_int_value()],
                                    "tup.ptr"
                                ))
                            };
                            Ok(b!(self.bld.build_load(elem_ty, ptr, "tup.val")))
                        }
                    } else {
                        Ok(self.ctx.i8_type().const_int(0, false).into())
                    }
                }
                mir::InstKind::IndexSet(base, idx, val) => {
                    let base_val = self.val(*base);
                    let idx_val = self.val(*idx);
                    let v = self.val(*val);
                    if base_val.get_type().is_array_type() {
                        let arr_ty = base_val.get_type().into_array_type();
                        let arr_len = arr_ty.len() as u64;
                        let alloca = self.entry_alloca(arr_ty.into(), "idxset.tmp");
                        b!(self.bld.build_store(alloca, base_val));
                        let i64t = self.ctx.i64_type();
                        let zero = i64t.const_int(0, false);
                        // Wrap negative indices
                        let idx_int = idx_val.into_int_value();
                        let is_neg = b!(self.bld.build_int_compare(
                            inkwell::IntPredicate::SLT,
                            idx_int,
                            zero,
                            "neg"
                        ));
                        let wrapped = b!(self.bld.build_int_nsw_add(
                            idx_int,
                            i64t.const_int(arr_len, false),
                            "wrap"
                        ));
                        let final_idx = b!(self.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                            .into_int_value();
                        let ptr = unsafe {
                            b!(self
                                .bld
                                .build_gep(arr_ty, alloca, &[zero, final_idx], "idxset.ptr"))
                        };
                        b!(self.bld.build_store(ptr, v));
                        // Load the modified array back so the mutation is visible.
                        let updated = b!(self.bld.build_load(arr_ty, alloca, "idxset.updated"));
                        return Ok(Some(updated));
                    } else if base_val.get_type().is_pointer_type() {
                        // Vec: header is { ptr, len, cap }.
                        let header_ptr = base_val.into_pointer_value();
                        let header_ty = self.vec_header_type();
                        let elem_ty = v.get_type();
                        let i64t = self.ctx.i64_type();
                        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                        let ptr_gep = b!(self
                            .bld
                            .build_struct_gep(header_ty, header_ptr, 0, "vis.ptrp"));
                        let data_ptr = b!(self.bld.build_load(ptr_ty, ptr_gep, "vis.data"))
                            .into_pointer_value();
                        let len_gep = b!(self
                            .bld
                            .build_struct_gep(header_ty, header_ptr, 1, "vis.lenp"));
                        let len =
                            b!(self.bld.build_load(i64t, len_gep, "vis.len")).into_int_value();
                        self.emit_vec_bounds_check(idx_val.into_int_value(), len)?;
                        let elem_gep = unsafe {
                            b!(self.bld.build_gep(
                                elem_ty,
                                data_ptr,
                                &[idx_val.into_int_value()],
                                "vis.egep"
                            ))
                        };
                        b!(self.bld.build_store(elem_gep, v));
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::IndexStore(var_name, idx, val) => {
                    // Direct index store into a named variable's alloca.
                    let idx_val = self.val(*idx);
                    let v = self.val(*val);
                    if let Some((alloca, ty)) = self.var_allocs.get(var_name).cloned() {
                        let llvm_ty = self.llvm_ty(&ty);
                        if llvm_ty.is_array_type() {
                            let arr_ty = llvm_ty.into_array_type();
                            let arr_len = arr_ty.len() as u64;
                            let i64t = self.ctx.i64_type();
                            let zero = i64t.const_int(0, false);
                            // Wrap negative indices
                            let idx_int = idx_val.into_int_value();
                            let is_neg = b!(self.bld.build_int_compare(
                                inkwell::IntPredicate::SLT,
                                idx_int,
                                zero,
                                "neg"
                            ));
                            let wrapped = b!(self.bld.build_int_nsw_add(
                                idx_int,
                                i64t.const_int(arr_len, false),
                                "wrap"
                            ));
                            let final_idx =
                                b!(self.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                                    .into_int_value();
                            let ptr = unsafe {
                                b!(self.bld.build_gep(
                                    arr_ty,
                                    alloca,
                                    &[zero, final_idx],
                                    "idxstore.ptr"
                                ))
                            };
                            b!(self.bld.build_store(ptr, v));
                        } else {
                            // Vec or other pointer-based type: load the header and index into data.
                            let header_ty = self.vec_header_type();
                            let i64t = self.ctx.i64_type();
                            let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                            let header_ptr = b!(self.bld.build_load(ptr_ty, alloca, "vis.hdr"))
                                .into_pointer_value();
                            let ptr_gep = b!(self
                                .bld
                                .build_struct_gep(header_ty, header_ptr, 0, "vis.ptrp"));
                            let data_ptr = b!(self.bld.build_load(ptr_ty, ptr_gep, "vis.data"))
                                .into_pointer_value();
                            let len_gep = b!(self
                                .bld
                                .build_struct_gep(header_ty, header_ptr, 1, "vis.lenp"));
                            let len =
                                b!(self.bld.build_load(i64t, len_gep, "vis.len")).into_int_value();
                            let elem_ty = v.get_type();
                            self.emit_vec_bounds_check(idx_val.into_int_value(), len)?;
                            let elem_gep = unsafe {
                                b!(self.bld.build_gep(
                                    elem_ty,
                                    data_ptr,
                                    &[idx_val.into_int_value()],
                                    "vis.egep"
                                ))
                            };
                            b!(self.bld.build_store(elem_gep, v));
                        }
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }

                // ── Cast / Ref / Deref ──
                mir::InstKind::Cast(val, target_ty) => {
                    let v = self.val(*val);
                    let target_llvm = self.llvm_ty(target_ty);
                    self.emit_cast(v, &inst.ty, target_ty, target_llvm)
                }
                mir::InstKind::StrictCast(val, target_ty) => {
                    let v = self.val(*val);
                    let target_llvm = self.llvm_ty(target_ty);
                    let casted = self.emit_cast(v, &inst.ty, target_ty, target_llvm)?;
                    // Validate: cast back and compare to original to detect overflow.
                    let source_llvm = v.get_type();
                    if v.is_int_value() && casted.is_int_value() {
                        let back = self.emit_cast(casted, target_ty, &inst.ty, source_llvm)?;
                        let eq = b!(self.bld.build_int_compare(
                            inkwell::IntPredicate::EQ,
                            v.into_int_value(),
                            back.into_int_value(),
                            "strict.eq"
                        ));
                        // If not equal, trap
                        let cur_fn = self.bld.get_insert_block().unwrap().get_parent().unwrap();
                        let ok_bb = self.ctx.append_basic_block(cur_fn, "strict.ok");
                        let trap_bb = self.ctx.append_basic_block(cur_fn, "strict.trap");
                        b!(self.bld.build_conditional_branch(eq, ok_bb, trap_bb));
                        self.bld.position_at_end(trap_bb);
                        if let Some(trap) = self.module.get_function("llvm.trap") {
                            b!(self.bld.build_call(trap, &[], ""));
                        }
                        b!(self.bld.build_unreachable());
                        self.bld.position_at_end(ok_bb);
                    }
                    Ok(casted)
                }
                mir::InstKind::Ref(val) => {
                    let v = self.val(*val);
                    let alloca = self.entry_alloca(v.get_type(), "ref");
                    b!(self.bld.build_store(alloca, v));
                    Ok(alloca.into())
                }
                mir::InstKind::Deref(val) => {
                    let v = self.val(*val);
                    if !v.is_pointer_value() {
                        return Err(format!("mir_codegen: Deref on non-pointer value {:?}", val));
                    }
                    // RC deref: skip refcount field, load from field 1
                    let val_ty = self.value_types.get(val).cloned();
                    if let Some(Type::Rc(ref inner)) = val_ty {
                        return Ok(Some((self.rc_deref(v, inner))?));
                    }
                    let inner_ty = self.llvm_ty(&inst.ty);
                    Ok(b!(self.bld.build_load(
                        inner_ty,
                        v.into_pointer_value(),
                        "deref"
                    )))
                }

                // ── Memory / RC ──
                mir::InstKind::Alloc(val) => {
                    let v = self.val(*val);
                    let malloc = self.ensure_malloc();
                    let size = v
                        .get_type()
                        .size_of()
                        .unwrap_or(self.ctx.i64_type().const_int(8, false));
                    let ptr = b!(self.bld.build_call(malloc, &[size.into()], "alloc"))
                        .try_as_basic_value()
                        .basic()
                        .expect("ICE: call returned void");
                    b!(self.bld.build_store(ptr.into_pointer_value(), v));
                    Ok(ptr)
                }
                _ => return Ok(None),
            })?,
        ))
    }
}

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_ownership_collection_inst(
        &mut self,
        inst: &mir::Instruction,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        Ok(Some(
            (match &inst.kind {
                mir::InstKind::Drop(val, ty) => {
                    let v = self.val(*val);
                    self.drop_value(v, ty)?;
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::RcInc(val) => {
                    let v = self.val(*val);
                    if let Type::Rc(inner) = &inst.ty {
                        self.rc_retain(v, inner)?;
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::RcDec(val) => {
                    let v = self.val(*val);
                    if let Type::Rc(inner) = &inst.ty {
                        self.rc_release(v, inner)?;
                    }
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::RcNew(val, inner_ty) => {
                    let v = self.val(*val);
                    self.rc_alloc(inner_ty, v)
                }
                mir::InstKind::RcClone(val) => {
                    let v = self.val(*val);
                    if let Type::Rc(inner) = &inst.ty {
                        self.rc_retain(v, inner)?;
                    }
                    Ok(v)
                }
                mir::InstKind::WeakUpgrade(val) => {
                    let v = self.val(*val);
                    if let Type::Weak(inner) | Type::Rc(inner) = &inst.ty {
                        self.weak_upgrade(v, inner)
                    } else {
                        Ok(v)
                    }
                }

                // ── Copy ──
                mir::InstKind::Copy(val) => Ok(self.val(*val)),

                // ── Slice ──
                mir::InstKind::Slice(base, lo, hi) => self.emit_slice(*base, *lo, *hi, &inst.ty),

                // ── Collections ──
                mir::InstKind::VecNew(elems) => self.emit_vec_new(elems, &inst.ty),
                mir::InstKind::VecPush(vec, elem) => {
                    let vec_val = self.val(*vec).into_pointer_value();
                    let elem_val = self.val(*elem);
                    let lty = elem_val.get_type();
                    let elem_size = self.type_store_size(lty);
                    let growth_floor = self
                        .vec_growth_floor_by_value
                        .get(vec)
                        .copied()
                        .unwrap_or(self.empty_vec_growth_floor);
                    self.vec_push_raw_with_floor(vec_val, elem_val, lty, elem_size, growth_floor)?;
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::VecLen(vec) => {
                    let vec_val = self.val(*vec);
                    let i64t = self.ctx.i64_type();
                    let vec_ty = self.value_types.get(vec);
                    if matches!(vec_ty, Some(Type::String)) || vec_val.is_struct_value() {
                        // String: struct { ptr, len, cap }; len is field 1.
                        self.string_len(vec_val)
                    } else if vec_val.is_pointer_value() {
                        // Vec (heap-allocated): ptr to {ptr, i64, i64}; len is field 1.
                        let header_ty = self.vec_header_type();
                        let header_ptr = vec_val.into_pointer_value();
                        let len_gep = b!(self
                            .bld
                            .build_struct_gep(header_ty, header_ptr, 1, "vl.len"));
                        Ok(b!(self.bld.build_load(i64t, len_gep, "vl.v")))
                    } else if vec_val.is_array_value() {
                        // Fixed-size array: length is known at compile time.
                        let arr_len = vec_val.into_array_value().get_type().len();
                        Ok(i64t.const_int(arr_len as u64, false).into())
                    } else {
                        Err("mir_codegen: VecLen on non-vec/array value".into())
                    }
                }
                mir::InstKind::MapInit => self.compile_map_new(),
                mir::InstKind::SetInit => {
                    if let Some(fv) = self.module.get_function("jade_set_new") {
                        let csv = b!(self.bld.build_call(fv, &[], "set"));
                        Ok(self.call_result(csv))
                    } else {
                        Err(
                        "mir_codegen: SetInit used but jade_set_new runtime function not declared"
                            .into(),
                    )
                    }
                }
                mir::InstKind::PQInit => {
                    if let Some(fv) = self.module.get_function("jade_pq_new") {
                        let csv = b!(self.bld.build_call(fv, &[], "pq"));
                        Ok(self.call_result(csv))
                    } else {
                        Err(
                        "mir_codegen: PQInit used but jade_pq_new runtime function not declared"
                            .into(),
                    )
                    }
                }
                mir::InstKind::DequeInit => {
                    if let Some(fv) = self.module.get_function("jade_deque_new") {
                        let csv = b!(self.bld.build_call(fv, &[], "deque"));
                        Ok(self.call_result(csv))
                    } else {
                        Err("mir_codegen: DequeInit used but jade_deque_new runtime function not declared".into())
                    }
                }

                // ── Closures ──,
                _ => return Ok(None),
            })?,
        ))
    }
}

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_runtime_inst(
        &mut self,
        inst: &mir::Instruction,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        Ok(Some(
            (match &inst.kind {
                mir::InstKind::ClosureCreate(fn_name, captures) => {
                    self.emit_closure_create(&fn_name.as_str(), captures, &inst.ty)
                }
                mir::InstKind::ClosureCall(callee, args) => {
                    // Same as IndirectCall for closures.
                    let callee_val = self.val(*callee);
                    let closure_st = callee_val.into_struct_value();
                    let fn_ptr = b!(self.bld.build_extract_value(closure_st, 0, "fn_ptr"))
                        .into_pointer_value();
                    let env_ptr = b!(self.bld.build_extract_value(closure_st, 1, "env_ptr"));
                    let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                    let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                        vec![env_ptr.into()];
                    let mut param_tys: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
                    for a in args {
                        let v = self.val(*a);
                        call_args.push(v.into());
                        param_tys.push(v.get_type().into());
                    }
                    let ret_llvm = self.llvm_ty(&inst.ty);
                    let ft = ret_llvm.fn_type(&param_tys, false);
                    let csv =
                        b!(self
                            .bld
                            .build_indirect_call(ft, fn_ptr, &call_args, "closure.call"));
                    Ok(self.call_result(csv))
                }

                // ── Actors / Channels ──
                mir::InstKind::SpawnActor(name, args) => {
                    if !args.is_empty() {
                        return Err(format!(
                            "mir_codegen: SpawnActor '{name}' has {} constructor args but actor spawn does not yet support arguments",
                            args.len()
                        ));
                    }
                    self.emit_spawn_actor(&name.as_str())
                }
                mir::InstKind::ChanCreate(elem_ty, cap) => {
                    self.emit_chan_create(elem_ty, cap.as_ref())
                }
                mir::InstKind::ChanSend(ch, val) => self.emit_chan_send(*ch, *val),
                mir::InstKind::ChanRecv(ch) => self.emit_chan_recv(*ch, &inst.ty),
                mir::InstKind::SelectArm(channels, has_default) => {
                    // Select: build case array, call jade_select, return index.
                    let ch_vids: Vec<mir::ValueId> = channels.clone();
                    let dest = inst.dest.unwrap();
                    self.emit_select(&ch_vids, dest, *has_default)
                }

                // ── Builtins ──
                mir::InstKind::Log(val) => {
                    let v = self.val(*val);
                    self.emit_log(v, &inst.ty)?;
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::Assert(val, msg) => {
                    let v = self.val(*val);
                    let fv = self.cur_fn.expect("ICE: cur_fn not set");
                    let cond = v.into_int_value();
                    let pass_bb = self.ctx.append_basic_block(fv, "assert.pass");
                    let fail_bb = self.ctx.append_basic_block(fv, "assert.fail");
                    b!(self.bld.build_conditional_branch(cond, pass_bb, fail_bb));
                    self.bld.position_at_end(fail_bb);
                    // Print assertion message and abort.
                    if let Some(printf) = self.module.get_function("printf") {
                        let fmt_str = format!("assertion failed: {msg}\n\0");
                        let gv = self
                            .bld
                            .build_global_string_ptr(&fmt_str, "assert.msg")
                            .map_err(|e| e.to_string())?;
                        b!(self
                            .bld
                            .build_call(printf, &[gv.as_pointer_value().into()], ""));
                    }
                    // Call abort.
                    let abort = self.module.get_function("abort").unwrap_or_else(|| {
                        let ft = self.ctx.void_type().fn_type(&[], false);
                        self.module
                            .add_function("abort", ft, Some(Linkage::External))
                    });
                    b!(self.bld.build_call(abort, &[], ""));
                    b!(self.bld.build_unreachable());
                    self.bld.position_at_end(pass_bb);
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }

                // ── Dynamic dispatch ──
                mir::InstKind::DynDispatch(obj, trait_name, method, args) => self
                    .emit_dyn_dispatch(
                        *obj,
                        &trait_name.as_str(),
                        &method.as_str(),
                        args,
                        &inst.ty,
                    ),

                mir::InstKind::DynCoerce(inner, type_name, trait_name) => {
                    let val = self.val(*inner);
                    let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

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
                        .unwrap_or_else(|| ptr_ty.const_null());

                    let fat_ty = self.ctx.struct_type(&[ptr_ty.into(), ptr_ty.into()], false);
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

                mir::InstKind::InlineAsm(template, args) => {
                    let arg_vals: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                        args.iter().map(|a| self.val(*a).into()).collect();
                    let arg_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = arg_vals
                        .iter()
                        .map(|v| match v {
                            inkwell::values::BasicMetadataValueEnum::IntValue(iv) => {
                                iv.get_type().into()
                            }
                            inkwell::values::BasicMetadataValueEnum::FloatValue(fv) => {
                                fv.get_type().into()
                            }
                            inkwell::values::BasicMetadataValueEnum::PointerValue(pv) => {
                                pv.get_type().into()
                            }
                            _ => self.ctx.i64_type().into(),
                        })
                        .collect();
                    let ft = self.ctx.void_type().fn_type(&arg_tys, false);
                    let asm = self.ctx.create_inline_asm(
                        ft,
                        template.clone(),
                        String::new(), // constraints
                        true,          // has side effects
                        false,         // needs aligned stack
                        None,          // dialect
                        false,         // can throw
                    );
                    b!(self.bld.build_indirect_call(ft, asm, &arg_vals, ""));
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                _ => return Ok(None),
            })?,
        ))
    }
}
