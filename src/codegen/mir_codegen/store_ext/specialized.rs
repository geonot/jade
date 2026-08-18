use super::*;

impl<'ctx> Compiler<'ctx> {
    fn kv_value_is_f64(&self, store_name: &str) -> bool {
        const BUILTIN: &[&str] = &[
            "sid",
            "uuid",
            "hash",
            "created",
            "updated",
            "deleted",
            "__version",
        ];
        self.store_defs
            .get(store_name)
            .map(|sd| {
                sd.fields.iter().any(|f| {
                    !BUILTIN.contains(&&*f.name.as_str())
                        && matches!(&*f.name.as_str(), "val" | "value")
                        && matches!(f.ty, crate::types::Type::F64)
                })
            })
            .unwrap_or(false)
    }

    pub(in crate::codegen) fn emit_kv_set(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.len() < 2 {
            return Err("kv_set requires key and value args".into());
        }
        let kv = self.load_kv_handle(store_name)?;
        let key_val = self.value_map[&args[0]];
        let val_val = self.value_map[&args[1]];

        let key_data = self.string_data(key_val)?;
        let key_len = self.string_len(key_val)?;

        let stored = if self.kv_value_is_f64(store_name) {
            b!(self
                .bld
                .build_bit_cast(val_val.into_float_value(), self.ctx.i64_type(), "kv.f2i"))
        } else {
            val_val
        };

        let set_fn = crate::codegen::fn_or_die(&self.module, "jinn_kv_set");
        b!(self.bld.build_call(
            set_fn,
            &[kv.into(), key_data.into(), key_len.into(), stored.into()],
            ""
        ));
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn emit_kv_get(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("kv_get requires key arg".into());
        }
        let kv = self.load_kv_handle(store_name)?;
        let key_val = self.value_map[&args[0]];

        let key_data = self.string_data(key_val)?;
        let key_len = self.string_len(key_val)?;

        let get_fn = crate::codegen::fn_or_die(&self.module, "jinn_kv_get");
        let result = self
            .call_result(b!(self.bld.build_call(
                get_fn,
                &[kv.into(), key_data.into(), key_len.into()],
                "kv.val"
            )))
            .into_int_value();
        if self.kv_value_is_f64(store_name) {
            let f = b!(self
                .bld
                .build_bit_cast(result, self.ctx.f64_type(), "kv.i2f"));
            return Ok(f);
        }
        Ok(result.into())
    }

    pub(in crate::codegen) fn emit_kv_has(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("kv_has requires key arg".into());
        }
        let kv = self.load_kv_handle(store_name)?;
        let key_val = self.value_map[&args[0]];

        let key_data = self.string_data(key_val)?;
        let key_len = self.string_len(key_val)?;

        let has_fn = crate::codegen::fn_or_die(&self.module, "jinn_kv_has");
        let result = self
            .call_result(b!(self.bld.build_call(
                has_fn,
                &[kv.into(), key_data.into(), key_len.into()],
                "kv.has"
            )))
            .into_int_value();

        let bool_val = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            result,
            self.ctx.i32_type().const_int(0, false),
            "kv.has.bool"
        ));
        Ok(bool_val.into())
    }

    pub(in crate::codegen) fn emit_kv_del(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("kv_del requires key arg".into());
        }
        let kv = self.load_kv_handle(store_name)?;
        let key_val = self.value_map[&args[0]];

        let key_data = self.string_data(key_val)?;
        let key_len = self.string_len(key_val)?;

        let del_fn = crate::codegen::fn_or_die(&self.module, "jinn_kv_del");
        b!(self
            .bld
            .build_call(del_fn, &[kv.into(), key_data.into(), key_len.into()], ""));
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn emit_kv_incr(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.len() < 2 {
            return Err("kv_incr requires key and delta args".into());
        }
        let kv = self.load_kv_handle(store_name)?;
        let key_val = self.value_map[&args[0]];
        let delta_val = self.value_map[&args[1]];

        let key_data = self.string_data(key_val)?;
        let key_len = self.string_len(key_val)?;

        let incr_fn = crate::codegen::fn_or_die(&self.module, "jinn_kv_incr");
        b!(self.bld.build_call(
            incr_fn,
            &[kv.into(), key_data.into(), key_len.into(), delta_val.into()],
            ""
        ));
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn emit_kv_count(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let kv = self.load_kv_handle(store_name)?;
        let count_fn = crate::codegen::fn_or_die(&self.module, "jinn_kv_count");
        let result = self
            .call_result(b!(self.bld.build_call(count_fn, &[kv.into()], "kv.cnt")))
            .into_int_value();
        Ok(result.into())
    }

    pub(in crate::codegen) fn emit_graph_query(
        &mut self,
        store_name: &str,
        direction: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err(format!("graph.{direction}() requires a node argument"));
        }
        let (sd, _st, rec_size, _fp) = self.setup_store_access(store_name)?;
        let fp = self.store_lock(store_name)?;
        let node_val = self.value_map[&args[0]];

        let builtin_names = [
            "sid",
            "uuid",
            "hash",
            "created",
            "updated",
            "deleted",
            "__version",
        ];
        let user_fields: Vec<(usize, &crate::hir::StoreField)> = sd
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| !builtin_names.contains(&&*f.name.as_str()))
            .collect();

        let target_idx = if direction == "from" { 0usize } else { 1 };
        if target_idx >= user_fields.len() {
            return Err(format!(
                "@graph store '{store_name}' needs at least 2 user fields (src, dst)"
            ));
        }
        let (field_idx, field_def) = user_fields[target_idx];
        let field_ty = field_def.ty.clone();

        let neighbor_target = if direction == "from" { 1usize } else { 0 };
        let (neighbor_idx, neighbor_def) = user_fields
            .get(neighbor_target)
            .copied()
            .unwrap_or(user_fields[target_idx]);
        let neighbor_ty = neighbor_def.ty.clone();

        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let neighbor_lty = self.llvm_ty(&neighbor_ty);
        let neighbor_size = self.type_store_size(neighbor_lty);

        let header_ty = self.vec_header_type();
        let malloc_fn = self.ensure_malloc();
        let result_vec = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[i64t.const_int(24, false).into()],
                "g.vec"
            )))
            .into_pointer_value();
        let gv_d = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 0, "g.vec.d"));
        b!(self.bld.build_store(gv_d, ptr_ty.const_null()));
        let gv_l = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 1, "g.vec.l"));
        b!(self.bld.build_store(gv_l, i64t.const_int(0, false)));
        let gv_c = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 2, "g.vec.c"));
        b!(self.bld.build_store(gv_c, i64t.const_int(0, false)));

        let total = self.store_read_count(fp, rec_size, store_name)?;
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(crate::codegen::stores::HEADER_SIZE, false)
                    .into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));

        let rec_name = format!("__store_{store_name}_rec");
        let rec_st = self
            .module
            .get_struct_type(&rec_name)
            .ok_or_else(|| format!("no record type for '{store_name}'"))?;
        let rec_buf = self.entry_alloca(rec_st.into(), "g.rec");

        let rec_size_val = i64t.const_int(rec_size, false);
        let match_count = self.entry_alloca(i64t.into(), "g.matches");
        b!(self.bld.build_store(match_count, i64t.const_int(0, false)));

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let loop_bb = self.ctx.append_basic_block(fv, "g.loop");
        let body_bb = self.ctx.append_basic_block(fv, "g.body");
        let inc_bb = self.ctx.append_basic_block(fv, "g.inc");
        let done_bb = self.ctx.append_basic_block(fv, "g.done");

        let idx = self.entry_alloca(i64t.into(), "g.idx");
        b!(self.bld.build_store(idx, i64t.const_int(0, false)));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let cur_idx = b!(self.bld.build_load(i64t, idx, "g.i")).into_int_value();
        let cond =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, cur_idx, total, "g.cmp"));
        b!(self.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        b!(self.bld.build_call(
            fread_fn,
            &[
                rec_buf.into(),
                rec_size_val.into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));

        let field_ptr = b!(self
            .bld
            .build_struct_gep(rec_st, rec_buf, field_idx as u32, "g.fp"));
        let cmp_result = match &field_ty {
            crate::types::Type::I64 => {
                let fval = b!(self.bld.build_load(i64t, field_ptr, "g.fv")).into_int_value();
                b!(self.bld.build_int_compare(
                    inkwell::IntPredicate::EQ,
                    fval,
                    node_val.into_int_value(),
                    "g.eq"
                ))
            }
            crate::types::Type::String => {
                let memcmp_fn = crate::codegen::fn_or_die(&self.module, "memcmp");
                let node_data = self.string_data(node_val)?;
                let node_len = self.string_len(node_val)?;

                let stored_len =
                    b!(self.bld.build_load(i64t, field_ptr, "g.slen")).into_int_value();

                let len_eq = b!(self.bld.build_int_compare(
                    inkwell::IntPredicate::EQ,
                    stored_len,
                    node_len.into_int_value(),
                    "g.leq"
                ));

                let _ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                let data_ptr = unsafe {
                    b!(self.bld.build_gep(
                        self.ctx.i8_type(),
                        field_ptr,
                        &[i64t.const_int(8, false)],
                        "g.sdp"
                    ))
                };
                let cmp_val = self
                    .call_result(b!(self.bld.build_call(
                        memcmp_fn,
                        &[data_ptr.into(), node_data.into(), node_len.into()],
                        "g.cmp"
                    )))
                    .into_int_value();
                let data_eq = b!(self.bld.build_int_compare(
                    inkwell::IntPredicate::EQ,
                    cmp_val,
                    i32t.const_int(0, false),
                    "g.deq"
                ));
                b!(self.bld.build_and(len_eq, data_eq, "g.match"))
            }
            _ => {
                return Err(format!(
                    "graph field type {:?} not supported for comparison",
                    field_ty
                ));
            }
        };

        let match_bb = self.ctx.append_basic_block(fv, "g.matched");
        b!(self
            .bld
            .build_conditional_branch(cmp_result, match_bb, inc_bb));

        self.bld.position_at_end(match_bb);
        let cur_count = b!(self.bld.build_load(i64t, match_count, "g.mc")).into_int_value();
        let new_count = b!(self
            .bld
            .build_int_add(cur_count, i64t.const_int(1, false), "g.mc1"));
        b!(self.bld.build_store(match_count, new_count));

        let neighbor_gep =
            b!(self
                .bld
                .build_struct_gep(rec_st, rec_buf, neighbor_idx as u32, "g.np"));
        let neighbor_val =
            match crate::codegen::store_filter::normalize_store_field_type(&neighbor_ty) {
                crate::types::Type::String => self.read_string_from_fixed_buf(neighbor_gep)?,
                ref nty => {
                    let lty = self.llvm_ty(nty);
                    b!(self.bld.build_load(lty, neighbor_gep, "g.nval"))
                }
            };
        self.vec_push_raw(result_vec, neighbor_val, neighbor_lty, neighbor_size)?;
        b!(self.bld.build_unconditional_branch(inc_bb));

        self.bld.position_at_end(inc_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(cur_idx, i64t.const_int(1, false), "g.ni"));
        b!(self.bld.build_store(idx, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let _ = match_count;
        self.store_unlock(store_name, fp)?;
        Ok(result_vec.into())
    }

    pub(in crate::codegen) fn emit_ts_latest(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (_sd, _st, rec_size, _fp) = self.setup_store_access(store_name)?;
        let fp = self.store_lock(store_name)?;
        let count = self.store_read_count(fp, rec_size, store_name)?;
        self.store_unlock(store_name, fp)?;
        Ok(count.into())
    }

    pub(in crate::codegen) fn emit_vec_nearest(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();
        let dims = sd
            .decorators
            .iter()
            .find_map(|d| match d {
                crate::ast::StoreDecorator::Vector(n) => Some(*n),
                _ => None,
            })
            .ok_or_else(|| format!("store '{store_name}' is not @vector"))?;

        let i64t = self.ctx.i64_type();
        let _f64t = self.ctx.f64_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

        let vec_handle = self.load_vec_handle(store_name, dims)?;

        let arg_val = self.val(args[0]);
        let query_ptr = if arg_val.is_pointer_value() {
            let header_ty = self.vec_header_type();
            let gep =
                b!(self
                    .bld
                    .build_struct_gep(header_ty, arg_val.into_pointer_value(), 0, "vn.dp"));
            b!(self.bld.build_load(ptr_ty, gep, "vn.data")).into_pointer_value()
        } else {
            let alloca = self.entry_alloca(arg_val.get_type(), "vn.arr");
            b!(self.bld.build_store(alloca, arg_val));
            alloca
        };

        let k_val = self.val(args[1]).into_int_value();
        let f64t = self.ctx.f64_type();

        let malloc_fn = self.ensure_malloc();
        let k_bytes = b!(self
            .bld
            .build_int_mul(k_val, i64t.const_int(8, false), "vn.kbytes"));
        let out_indices = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[k_bytes.into()],
                "vec.out"
            )))
            .into_pointer_value();
        let out_dists = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[k_bytes.into()],
                "vec.dists"
            )))
            .into_pointer_value();

        let nearest_fn = crate::codegen::fn_or_die(&self.module, "jinn_vec_nearest_scored");
        let found = self
            .call_result(b!(self.bld.build_call(
                nearest_fn,
                &[
                    vec_handle.into(),
                    query_ptr.into(),
                    k_val.into(),
                    out_indices.into(),
                    out_dists.into()
                ],
                "vec.found"
            )))
            .into_int_value();

        let tuple_ty = self.ctx.struct_type(&[i64t.into(), f64t.into()], false);
        let tuple_size = self.type_store_size(tuple_ty.into());

        let header_ty = self.vec_header_type();
        let result_vec = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[i64t.const_int(24, false).into()],
                "vn.vec"
            )))
            .into_pointer_value();
        let v_dgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 0, "vn.vec.d"));
        b!(self.bld.build_store(v_dgep, ptr_ty.const_null()));
        let v_lgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 1, "vn.vec.l"));
        b!(self.bld.build_store(v_lgep, i64t.const_int(0, false)));
        let v_cgep = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 2, "vn.vec.c"));
        b!(self.bld.build_store(v_cgep, i64t.const_int(0, false)));

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let idx_ptr = self.entry_alloca(i64t.into(), "vn.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "vn.loop");
        let body_bb = self.ctx.append_basic_block(fv, "vn.body");
        let done_bb = self.ctx.append_basic_block(fv, "vn.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "vn.i")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, idx, found, "vn.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let i_gep = unsafe { b!(self.bld.build_gep(i64t, out_indices, &[idx], "vn.igep")) };
        let i_val = b!(self.bld.build_load(i64t, i_gep, "vn.ival"));
        let d_gep = unsafe { b!(self.bld.build_gep(f64t, out_dists, &[idx], "vn.dgep")) };
        let d_val = b!(self.bld.build_load(f64t, d_gep, "vn.dval"));

        let undef = tuple_ty.get_undef();
        let with0 = b!(self.bld.build_insert_value(undef, i_val, 0, "vn.ins0")).into_struct_value();
        let tuple_val =
            b!(self.bld.build_insert_value(with0, d_val, 1, "vn.ins1")).into_struct_value();
        self.vec_push_raw(result_vec, tuple_val.into(), tuple_ty.into(), tuple_size)?;

        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "vn.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[out_indices.into()], ""));
        b!(self.bld.build_call(free_fn, &[out_dists.into()], ""));
        Ok(result_vec.into())
    }

    pub(in crate::codegen) fn emit_vec_insert(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();
        let dims = sd
            .decorators
            .iter()
            .find_map(|d| match d {
                crate::ast::StoreDecorator::Vector(n) => Some(*n),
                _ => None,
            })
            .ok_or_else(|| format!("store '{store_name}' is not @vector"))?;

        let vec_handle = self.load_vec_handle(store_name, dims)?;
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

        let arg_val = self.val(args[0]);
        let data_ptr = if arg_val.is_pointer_value() {
            let header_ty = self.vec_header_type();
            let gep =
                b!(self
                    .bld
                    .build_struct_gep(header_ty, arg_val.into_pointer_value(), 0, "vi.dp"));
            b!(self.bld.build_load(ptr_ty, gep, "vi.data")).into_pointer_value()
        } else {
            let alloca = self.entry_alloca(arg_val.get_type(), "vi.arr");
            b!(self.bld.build_store(alloca, arg_val));
            alloca
        };

        let insert_fn = crate::codegen::fn_or_die(&self.module, "jinn_vec_insert");
        b!(self
            .bld
            .build_call(insert_fn, &[vec_handle.into(), data_ptr.into()], ""));

        let count_fn = crate::codegen::fn_or_die(&self.module, "jinn_vec_count");
        let count = self.call_result(b!(self.bld.build_call(
            count_fn,
            &[vec_handle.into()],
            "vi.cnt"
        )));
        Ok(count)
    }

    pub(in crate::codegen) fn emit_vec_count(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();
        let dims = sd
            .decorators
            .iter()
            .find_map(|d| match d {
                crate::ast::StoreDecorator::Vector(n) => Some(*n),
                _ => None,
            })
            .ok_or_else(|| format!("store '{store_name}' is not @vector"))?;

        let vec_handle = self.load_vec_handle(store_name, dims)?;
        let count_fn = crate::codegen::fn_or_die(&self.module, "jinn_vec_count");
        let count = self.call_result(b!(self.bld.build_call(
            count_fn,
            &[vec_handle.into()],
            "vc.cnt"
        )));
        Ok(count)
    }

    pub(in crate::codegen) fn emit_bloom_test(
        &mut self,
        rest: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = rest.splitn(2, '_').collect();
        if parts.len() < 2 || args.is_empty() {
            return Err(format!("malformed bloom_test name: {rest}"));
        }
        let store_name = parts[0];
        let field_name = parts[1];

        let bloom = self.load_bloom_handle(store_name, field_name, 10000)?;
        let val = self.val(args[0]);

        let test_fn = self.module.get_function("jinn_bloom_test_i64").unwrap();
        let result = self
            .call_result(b!(self.bld.build_call(
                test_fn,
                &[bloom.into(), val.into()],
                "bloom.res"
            )))
            .into_int_value();

        let bool_val = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            result,
            self.ctx.i64_type().const_int(0, false),
            "bloom.bool"
        ));
        Ok(bool_val.into())
    }

    pub(in crate::codegen) fn emit_fts_search(
        &mut self,
        rest: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = rest.splitn(2, '_').collect();
        if parts.len() < 2 || args.is_empty() {
            return Err(format!("malformed fts_search name: {rest}"));
        }
        let store_name = parts[0];
        let field_name = parts[1];

        let fts = self.load_fts_handle(store_name, field_name)?;
        let query_val = self.val(args[0]);

        let query_data = self.string_data(query_val)?;
        let query_len = self.string_len(query_val)?;

        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

        let search_fn = crate::codegen::fn_or_die(&self.module, "jinn_fts_search_n");
        let count = self
            .call_result(b!(self.bld.build_call(
                search_fn,
                &[fts.into(), query_data.into(), query_len.into()],
                "fts.cnt"
            )))
            .into_int_value();

        let malloc_fn = self.ensure_malloc();
        let one = i64t.const_int(1, false);
        let buf_bytes = b!(self
            .bld
            .build_int_mul(count, i64t.const_int(8, false), "fts.bb"));
        let buf_alloc = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                buf_bytes,
                i64t.const_int(0, false),
                "fts.isz"
            )),
            one,
            buf_bytes,
            "fts.alloc"
        ))
        .into_int_value();
        let out_ids = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[buf_alloc.into()],
                "fts.ids"
            )))
            .into_pointer_value();

        let fill_fn = crate::codegen::fn_or_die(&self.module, "jinn_fts_search_ids_n");
        let found = self
            .call_result(b!(self.bld.build_call(
                fill_fn,
                &[
                    fts.into(),
                    query_data.into(),
                    query_len.into(),
                    out_ids.into(),
                    count.into()
                ],
                "fts.found"
            )))
            .into_int_value();

        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();
        let ensure_fn_name = format!("__store_ensure_{store_name}");
        if let Some(ensure_fn) = self.module.get_function(&ensure_fn_name) {
            b!(self.bld.build_call(ensure_fn, &[], ""));
        } else {
            let ensure_fn = self.gen_store_ensure_open(&sd)?;
            b!(self.bld.build_call(ensure_fn, &[], ""));
        }
        let fp = self.store_lock(store_name)?;
        let rec_size = self.store_record_size(&sd);
        let rec_st = self
            .module
            .get_struct_type(&format!("__store_{store_name}_rec"))
            .expect("ICE: struct type not declared");
        let jinn_st = self
            .module
            .get_struct_type(&format!("__store_{store_name}"))
            .expect("ICE: struct type not declared");
        let jinn_size = self.type_store_size(jinn_st.into());
        let total = self.store_read_count(fp, rec_size, store_name)?;
        let raw_buf = self.store_load_records(fp, total, rec_size)?;
        self.store_unlock(store_name, fp)?;

        let jinn_total =
            b!(self
                .bld
                .build_int_mul(found, i64t.const_int(jinn_size, false), "fts.jt"));
        let jinn_alloc = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                jinn_total,
                i64t.const_int(0, false),
                "fts.jisz"
            )),
            one,
            jinn_total,
            "fts.jalloc"
        ))
        .into_int_value();
        let jinn_buf = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[jinn_alloc.into()],
                "fts.jbuf"
            )))
            .into_pointer_value();

        let sid_field = sd.fields.iter().position(|f| f.name == "sid");
        let del_idx = sd.fields.iter().position(|f| f.name == "deleted");

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let out_ptr = self.entry_alloca(i64t.into(), "fts.out");
        b!(self.bld.build_store(out_ptr, i64t.const_int(0, false)));
        let i_ptr = self.entry_alloca(i64t.into(), "fts.i");
        b!(self.bld.build_store(i_ptr, i64t.const_int(0, false)));
        let j_ptr = self.entry_alloca(i64t.into(), "fts.j");

        let oloop_bb = self.ctx.append_basic_block(fv, "fts.oloop");
        let obody_bb = self.ctx.append_basic_block(fv, "fts.obody");
        let iloop_bb = self.ctx.append_basic_block(fv, "fts.iloop");
        let ibody_bb = self.ctx.append_basic_block(fv, "fts.ibody");
        let imatch_bb = self.ctx.append_basic_block(fv, "fts.imatch");
        let icopy_bb = self.ctx.append_basic_block(fv, "fts.icopy");
        let inext_bb = self.ctx.append_basic_block(fv, "fts.inext");
        let onext_bb = self.ctx.append_basic_block(fv, "fts.onext");
        let odone_bb = self.ctx.append_basic_block(fv, "fts.odone");

        b!(self.bld.build_unconditional_branch(oloop_bb));

        self.bld.position_at_end(oloop_bb);
        let i = b!(self.bld.build_load(i64t, i_ptr, "fts.iv")).into_int_value();
        let ocmp = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, i, found, "fts.ocmp"));
        b!(self.bld.build_conditional_branch(ocmp, obody_bb, odone_bb));

        self.bld.position_at_end(obody_bb);
        let id_gep = unsafe { b!(self.bld.build_gep(i64t, out_ids, &[i], "fts.idp")) };
        let want_id = b!(self.bld.build_load(i64t, id_gep, "fts.id")).into_int_value();
        if sid_field.is_some() {
            b!(self.bld.build_store(j_ptr, i64t.const_int(0, false)));
        } else {
            let j0 = b!(self
                .bld
                .build_int_sub(want_id, i64t.const_int(1, false), "fts.j0"));
            b!(self.bld.build_store(j_ptr, j0));
        }
        b!(self.bld.build_unconditional_branch(iloop_bb));

        self.bld.position_at_end(iloop_bb);
        let j = b!(self.bld.build_load(i64t, j_ptr, "fts.jv")).into_int_value();
        let icmp = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, j, total, "fts.icmp"));
        b!(self.bld.build_conditional_branch(icmp, ibody_bb, onext_bb));

        self.bld.position_at_end(ibody_bb);
        let raw_off = b!(self
            .bld
            .build_int_mul(j, i64t.const_int(rec_size, false), "fts.roff"));
        let raw_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), raw_buf, &[raw_off], "fts.rptr"))
        };
        let sid = if let Some(si) = sid_field {
            let sid_gep = b!(self
                .bld
                .build_struct_gep(rec_st, raw_ptr, si as u32, "fts.sidp"));
            b!(self.bld.build_load(i64t, sid_gep, "fts.sid")).into_int_value()
        } else {
            b!(self
                .bld
                .build_int_add(j, i64t.const_int(1, false), "fts.sid"))
        };
        let is_match =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::EQ, sid, want_id, "fts.m"));
        b!(self
            .bld
            .build_conditional_branch(is_match, imatch_bb, inext_bb));

        self.bld.position_at_end(imatch_bb);
        if let Some(di) = del_idx {
            let del_gep = b!(self
                .bld
                .build_struct_gep(rec_st, raw_ptr, di as u32, "fts.delp"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "fts.del")).into_int_value();
            let is_del = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "fts.isdel"
            ));
            b!(self
                .bld
                .build_conditional_branch(is_del, onext_bb, icopy_bb));
        } else {
            b!(self.bld.build_unconditional_branch(icopy_bb));
        }

        self.bld.position_at_end(icopy_bb);
        let jinn_val = self.load_store_record_as_jinn(rec_st, raw_ptr, &sd)?;
        let out = b!(self.bld.build_load(i64t, out_ptr, "fts.ov")).into_int_value();
        let jinn_off =
            b!(self
                .bld
                .build_int_mul(out, i64t.const_int(jinn_size, false), "fts.joff"));
        let jinn_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), jinn_buf, &[jinn_off], "fts.jptr"))
        };
        b!(self.bld.build_store(jinn_ptr, jinn_val));
        let out_next = b!(self
            .bld
            .build_int_add(out, i64t.const_int(1, false), "fts.oinc"));
        b!(self.bld.build_store(out_ptr, out_next));
        b!(self.bld.build_unconditional_branch(onext_bb));

        self.bld.position_at_end(inext_bb);
        let j_next = b!(self
            .bld
            .build_int_add(j, i64t.const_int(1, false), "fts.jinc"));
        b!(self.bld.build_store(j_ptr, j_next));
        b!(self.bld.build_unconditional_branch(iloop_bb));

        self.bld.position_at_end(onext_bb);
        let i_next = b!(self
            .bld
            .build_int_add(i, i64t.const_int(1, false), "fts.iinc"));
        b!(self.bld.build_store(i_ptr, i_next));
        b!(self.bld.build_unconditional_branch(oloop_bb));

        self.bld.position_at_end(odone_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[raw_buf.into()], ""));
        b!(self.bld.build_call(free_fn, &[out_ids.into()], ""));
        let final_out = b!(self.bld.build_load(i64t, out_ptr, "fts.n")).into_int_value();

        let header_ty = self.vec_header_type();
        let result_vec = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[i64t.const_int(24, false).into()],
                "fts.vec"
            )))
            .into_pointer_value();
        let fv_d = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 0, "fts.vec.d"));
        b!(self.bld.build_store(fv_d, jinn_buf));
        let fv_l = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 1, "fts.vec.l"));
        b!(self.bld.build_store(fv_l, final_out));
        let fv_c = b!(self
            .bld
            .build_struct_gep(header_ty, result_vec, 2, "fts.vec.c"));
        b!(self.bld.build_store(fv_c, found));
        let _ = ptr_ty;

        Ok(result_vec.into())
    }
}
