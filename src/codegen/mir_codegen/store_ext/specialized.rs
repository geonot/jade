use super::*;

impl<'ctx> Compiler<'ctx> {
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

        let set_fn = crate::codegen::fn_or_die(&self.module, "jinn_kv_set");
        b!(self.bld.build_call(
            set_fn,
            &[kv.into(), key_data.into(), key_len.into(), val_val.into()],
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
        let (sd, _st, rec_size, fp) = self.setup_store_access(store_name)?;
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

        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(24, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));

        let _ftell_fn = crate::codegen::fn_or_die(&self.module, "ftell");
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));
        let count_alloca = self.entry_alloca(i64t.into(), "g.cnt");
        b!(self.bld.build_call(
            fread_fn,
            &[
                count_alloca.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));
        let total = b!(self.bld.build_load(i64t, count_alloca, "g.total")).into_int_value();

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(24, false).into(),
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
        b!(self.bld.build_unconditional_branch(inc_bb));

        self.bld.position_at_end(inc_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(cur_idx, i64t.const_int(1, false), "g.ni"));
        b!(self.bld.build_store(idx, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);
        let result = b!(self.bld.build_load(i64t, match_count, "g.result")).into_int_value();
        Ok(result.into())
    }

    pub(in crate::codegen) fn emit_ts_latest(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (_sd, _st, _rec_size, fp) = self.setup_store_access(store_name)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));
        let count_alloca = self.entry_alloca(i64t.into(), "ts.cnt_buf");
        b!(self.bld.build_call(
            fread_fn,
            &[
                count_alloca.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));
        let count = b!(self.bld.build_load(i64t, count_alloca, "ts.count")).into_int_value();
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

        let out_indices = b!(self.bld.build_array_alloca(i64t, k_val, "vec.out"));

        let nearest_fn = crate::codegen::fn_or_die(&self.module, "jinn_vec_nearest");
        let result = self.call_result(b!(self.bld.build_call(
            nearest_fn,
            &[
                vec_handle.into(),
                query_ptr.into(),
                k_val.into(),
                out_indices.into()
            ],
            "vec.found"
        )));
        Ok(result)
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

        let search_fn = crate::codegen::fn_or_die(&self.module, "jinn_fts_search_n");
        let count = self
            .call_result(b!(self.bld.build_call(
                search_fn,
                &[fts.into(), query_data.into(), query_len.into()],
                "fts.res"
            )))
            .into_int_value();
        Ok(count.into())
    }
}
