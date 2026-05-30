use super::super::Compiler;
use super::super::b;
use crate::mir;
use crate::types::Type;
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::values::{BasicValue, BasicValueEnum};

impl<'ctx> Compiler<'ctx> {
    pub(super) fn try_handle_magic_call(
        &mut self,
        name: &str,
        args: &[mir::ValueId],
        _result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        if let Some(coro_name) = name.strip_prefix("__coro_create_") {
            return self.emit_coro_create(coro_name, args).map(Some);
        }

        if let Some(gen_name) = name.strip_prefix("__gen_create_") {
            return self.emit_coro_create(gen_name, args).map(Some);
        }

        if name == "__coro_next" || name == "__gen_next" {
            if let Some(&gen_val) = args.first() {
                return self.emit_coro_next(gen_val).map(Some);
            }
        }

        if name == "__gen_resume" {
            if let Some(&gen_val) = args.first() {
                let gen_ptr = self.val(gen_val).into_pointer_value();
                let gen_resume = self
                    .module
                    .get_function("jinn_gen_resume")
                    .ok_or("jinn_gen_resume not declared")?;
                b!(self.bld.build_call(gen_resume, &[gen_ptr.into()], ""));
                return Ok(Some(self.ctx.i64_type().const_int(0, false).into()));
            }
        }

        if name == "__gen_done" {
            if let Some(&gen_val) = args.first() {
                let gen_ptr = self.val(gen_val).into_pointer_value();
                let i8t = self.ctx.i8_type();
                let done_ptr =
                    self.gen_field_ptr(gen_ptr, Compiler::GEN_DONE_OFF, "gen.done.ptr")?;
                let done = b!(self.bld.build_load(i8t, done_ptr, "gen.done"));
                let done_bool = b!(self.bld.build_int_compare(
                    inkwell::IntPredicate::NE,
                    done.into_int_value(),
                    i8t.const_int(0, false),
                    "gen.done.bool"
                ));
                return Ok(Some(done_bool.into()));
            }
        }

        if name == "__gen_next_val" {
            if let Some(&gen_val) = args.first() {
                let gen_ptr = self.val(gen_val).into_pointer_value();
                let i8t = self.ctx.i8_type();
                let i64t = self.ctx.i64_type();
                let value_ptr =
                    self.gen_field_ptr(gen_ptr, Compiler::GEN_VALUE_OFF, "gen.val.ptr")?;
                let result = b!(self.bld.build_load(i64t, value_ptr, "gen.val"));

                let has_val_ptr =
                    self.gen_field_ptr(gen_ptr, Compiler::GEN_HAS_VALUE_OFF, "gen.hv.ptr")?;
                b!(self.bld.build_store(has_val_ptr, i8t.const_int(0, false)));
                return Ok(Some(result));
            }
        }

        if name == "__yield" {
            if let Some(&val) = args.first() {
                return self.emit_coro_yield(val).map(Some);
            }
        }

        if name == "__select_recv" {
            if args.len() >= 2 {
                let select_vid = args[0];
                let idx_val = self.val(args[1]).into_int_value();
                let idx = idx_val.get_zero_extended_constant().unwrap_or(0) as usize;
                if let Some(bufs) = self.select_data_bufs.get(&select_vid) {
                    if let Some(&buf_ptr) = bufs.get(idx) {
                        let i64t = self.ctx.i64_type();
                        let val = b!(self.bld.build_load(i64t, buf_ptr, "recv.val"));
                        return Ok(Some(val));
                    }
                }

                return Ok(Some(self.ctx.i64_type().const_int(0, false).into()));
            }
        }

        if let Some(rest) = name.strip_prefix("__send_") {
            return self.emit_actor_send(rest, args).map(Some);
        }

        if let Some(store_name) = name.strip_prefix("__store_insert_") {
            return self.emit_store_insert(store_name, args).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_query_") {
            return self.emit_store_query(rest, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_count_") {
            return self.emit_store_count(store_name).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_all_") {
            return self.emit_store_all(store_name).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__view_count_") {
            return self.emit_view_count(rest, args).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__view_all_") {
            return self.emit_view_all(rest, args).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_delete_") {
            return self.emit_store_delete(rest, args).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_set_") {
            return self.emit_store_set(rest, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_get_") {
            return self.emit_store_get(store_name, args).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_first_") {
            return self.emit_store_first(rest, args).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_exists_") {
            return self.emit_store_exists(rest, args).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_destroy_") {
            return self.emit_store_destroy(rest, args).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_restore_") {
            return self.emit_store_restore(rest, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_save_") {
            return self.emit_store_save(store_name).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_distinct_") {
            return self.emit_store_distinct(rest).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_sum_") {
            return self.emit_store_agg(rest, "sum").map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_avg_") {
            return self.emit_store_agg(rest, "avg").map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_min_") {
            return self.emit_store_agg(rest, "min").map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_max_") {
            return self.emit_store_agg(rest, "max").map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_version_count_") {
            return self.emit_store_version_count(store_name, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_history_") {
            return self.emit_store_history(store_name, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_at_version_") {
            return self.emit_store_at_version(store_name, args).map(Some);
        }

        if let Some(store_name) = name.strip_prefix("__kv_set_") {
            return self.emit_kv_set(store_name, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__kv_get_") {
            return self.emit_kv_get(store_name, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__kv_has_") {
            return self.emit_kv_has(store_name, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__kv_del_") {
            return self.emit_kv_del(store_name, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__kv_incr_") {
            return self.emit_kv_incr(store_name, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__kv_count_") {
            return self.emit_kv_count(store_name).map(Some);
        }

        if let Some(store_name) = name.strip_prefix("__graph_from_") {
            return self.emit_graph_query(store_name, "from", args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__graph_to_") {
            return self.emit_graph_query(store_name, "to", args).map(Some);
        }

        if let Some(store_name) = name.strip_prefix("__ts_latest_") {
            return self.emit_ts_latest(store_name).map(Some);
        }

        if let Some(store_name) = name.strip_prefix("__vec_nearest_") {
            return self.emit_vec_nearest(store_name, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__vec_insert_") {
            return self.emit_vec_insert(store_name, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__vec_count_") {
            return self.emit_vec_count(store_name).map(Some);
        }

        if let Some(rest) = name.strip_prefix("__bloom_test_") {
            return self.emit_bloom_test(rest, args).map(Some);
        }

        if let Some(rest) = name.strip_prefix("__fts_search_") {
            return self.emit_fts_search(rest, args).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__fts_count_") {
            return self.emit_fts_count(rest).map(Some);
        }

        if name == "__txn_begin" || name == "__txn_commit" {
            return Ok(Some(self.ctx.i8_type().const_int(0, false).into()));
        }

        if name == "__chan_close" {
            if let Some(&ch_val) = args.first() {
                let ch_ptr = self.val(ch_val).into_pointer_value();
                let chan_close = self
                    .module
                    .get_function("jinn_chan_close")
                    .ok_or("jinn_chan_close not declared")?;
                b!(self.bld.build_call(chan_close, &[ch_ptr.into()], ""));
                return Ok(Some(self.ctx.i8_type().const_int(0, false).into()));
            }
        }

        if name == "__stop" {
            if let Some(&actor_val) = args.first() {
                let actor_ptr = self.val(actor_val).into_pointer_value();
                let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                let ch_ptr =
                    b!(self.bld.build_load(ptr_ty, actor_ptr, "stop.ch")).into_pointer_value();
                let chan_close = self
                    .module
                    .get_function("jinn_chan_close")
                    .ok_or("jinn_chan_close not declared")?;
                b!(self.bld.build_call(chan_close, &[ch_ptr.into()], ""));
                return Ok(Some(self.ctx.i8_type().const_int(0, false).into()));
            }
        }

        if name == "__atomic_load" {
            if let Some(&ptr_val) = args.first() {
                let ptr = self.val(ptr_val).into_pointer_value();
                let i64t = self.ctx.i64_type();
                let load = b!(self.bld.build_load(i64t, ptr, "atomic.load"));
                load.as_instruction_value()
                    .expect("ICE: not an instruction")
                    .set_atomic_ordering(inkwell::AtomicOrdering::SequentiallyConsistent)
                    .map_err(|_| "failed to set atomic ordering")?;
                return Ok(Some(load));
            }
        }
        if name == "__atomic_store" {
            if args.len() >= 2 {
                let ptr = self.val(args[0]).into_pointer_value();
                let val = self.val(args[1]);
                let store = b!(self.bld.build_store(ptr, val));
                store
                    .set_atomic_ordering(inkwell::AtomicOrdering::SequentiallyConsistent)
                    .map_err(|_| "failed to set atomic ordering")?;
                return Ok(Some(self.ctx.i64_type().const_zero().into()));
            }
        }
        if name == "__atomic_add" {
            if args.len() >= 2 {
                let ptr = self.val(args[0]).into_pointer_value();
                let val = self.val(args[1]).into_int_value();
                let old = b!(self.bld.build_atomicrmw(
                    inkwell::AtomicRMWBinOp::Add,
                    ptr,
                    val,
                    inkwell::AtomicOrdering::SequentiallyConsistent,
                ));
                return Ok(Some(old.into()));
            }
        }
        if name == "__atomic_sub" {
            if args.len() >= 2 {
                let ptr = self.val(args[0]).into_pointer_value();
                let val = self.val(args[1]).into_int_value();
                let old = b!(self.bld.build_atomicrmw(
                    inkwell::AtomicRMWBinOp::Sub,
                    ptr,
                    val,
                    inkwell::AtomicOrdering::SequentiallyConsistent,
                ));
                return Ok(Some(old.into()));
            }
        }
        if name == "__atomic_cas" {
            if args.len() >= 3 {
                let ptr = self.val(args[0]).into_pointer_value();
                let expected = self.val(args[1]).into_int_value();
                let new_val = self.val(args[2]).into_int_value();
                let cas = b!(self.bld.build_cmpxchg(
                    ptr,
                    expected,
                    new_val,
                    inkwell::AtomicOrdering::SequentiallyConsistent,
                    inkwell::AtomicOrdering::SequentiallyConsistent,
                ));
                let old = b!(self.bld.build_extract_value(cas, 0, "cas.old"));
                return Ok(Some(old));
            }
        }

        Ok(None)
    }

    pub(super) fn emit_coro_create(
        &mut self,
        name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.declare_actor_runtime();
        self.declare_gen_runtime();

        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        // The coroutine/generator body was lowered to a standalone MIR
        // function `__coro_{name}` (declared by `declare_mir_fn` with the
        // `void(ptr)` coroutine ABI). Here we only allocate and initialize the
        // generator struct, store the captures (= call args) into it, and wire
        // up the real coroutine via `jinn_coro_create`. The body reloads the
        // captures from the struct in its own prologue.
        let coro_fn_name = format!("__coro_{name}");
        let coro_fn = self
            .module
            .get_function(&coro_fn_name)
            .ok_or_else(|| format!("coroutine body fn `{coro_fn_name}` not declared"))?;

        let arg_vals: Vec<BasicValueEnum<'ctx>> = args.iter().map(|vid| self.val(*vid)).collect();

        let total_size = Compiler::GEN_SIZE + (arg_vals.len() as u64) * 8;
        let malloc_fn = self.ensure_malloc();
        let gen_mem = b!(self.bld.build_call(
            malloc_fn,
            &[i64t.const_int(total_size, false).into()],
            "gen.mem"
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: malloc returned void")
        .into_pointer_value();

        let memset_fn = self.module.get_function("memset").unwrap_or_else(|| {
            let ft = ptr.fn_type(&[ptr.into(), i32t.into(), i64t.into()], false);
            self.module
                .add_function("memset", ft, Some(Linkage::External))
        });
        b!(self.bld.build_call(
            memset_fn,
            &[
                gen_mem.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(total_size, false).into()
            ],
            ""
        ));

        for (i, val) in arg_vals.iter().enumerate() {
            let off = Compiler::GEN_SIZE + (i as u64) * 8;
            let slot_ptr = self.gen_field_ptr(gen_mem, off, "cap.slot")?;
            b!(self.bld.build_store(slot_ptr, *val));
        }

        let coro_create = crate::codegen::fn_or_die(&self.module, "jinn_coro_create");
        let coro = b!(self.bld.build_call(
            coro_create,
            &[
                coro_fn.as_global_value().as_pointer_value().into(),
                gen_mem.into(),
            ],
            "gen.coro"
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: jinn_coro_create returned void");

        let coro_ptr_field =
            self.gen_field_ptr(gen_mem, Compiler::GEN_CORO_PTR_OFF, "gen.coro_ptr")?;
        b!(self.bld.build_store(coro_ptr_field, coro));

        // Bind the coroutine's source name (e.g. `producer`) so a later
        // `producer.next()` (lowered to `load producer` + `__coro_next`)
        // resolves to this generator struct. Anonymous coroutines skip this.
        if name != "__anon" {
            let name_alloca = self.entry_alloca(ptr.into(), name);
            b!(self.bld.build_store(name_alloca, gen_mem));
            self.set_var(name, name_alloca, Type::Coroutine(Box::new(Type::I64)));
        }

        Ok(gen_mem.into())
    }

    pub(super) fn emit_coro_next(
        &mut self,
        gen_val_id: mir::ValueId,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let gen_ptr = self.val(gen_val_id).into_pointer_value();
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();

        let gen_resume = self
            .module
            .get_function("jinn_gen_resume")
            .ok_or("jinn_gen_resume not declared")?;
        b!(self.bld.build_call(gen_resume, &[gen_ptr.into()], ""));

        let value_ptr = self.gen_field_ptr(gen_ptr, Compiler::GEN_VALUE_OFF, "gen.n.val")?;
        let result = b!(self.bld.build_load(i64t, value_ptr, "gen.result"));

        let has_val_ptr = self.gen_field_ptr(gen_ptr, Compiler::GEN_HAS_VALUE_OFF, "gen.n.hv")?;
        b!(self.bld.build_store(has_val_ptr, i8t.const_int(0, false)));

        Ok(result)
    }

    pub(super) fn emit_coro_yield(
        &mut self,
        val_id: mir::ValueId,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if self.find_var("__coro_ctx").is_none() {
            return Ok(self.ctx.i64_type().const_int(0, false).into());
        }

        let val = self.val(val_id);
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i8t = self.ctx.i8_type();

        let (gen_alloca, _) = self.find_var("__coro_ctx").cloned().unwrap();
        let gen_ptr = b!(self.bld.build_load(ptr, gen_alloca, "gen.ctx")).into_pointer_value();

        let value_ptr = self.gen_field_ptr(gen_ptr, Compiler::GEN_VALUE_OFF, "gen.y.val")?;
        let i64_val = self.coerce_to_i64(val);
        b!(self.bld.build_store(value_ptr, i64_val));

        let has_val_ptr = self.gen_field_ptr(gen_ptr, Compiler::GEN_HAS_VALUE_OFF, "gen.y.hv")?;
        b!(self.bld.build_store(has_val_ptr, i8t.const_int(1, false)));

        let gen_suspend = self
            .module
            .get_function("jinn_gen_suspend")
            .ok_or("jinn_gen_suspend not declared")?;
        b!(self.bld.build_call(gen_suspend, &[gen_ptr.into()], ""));

        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(super) fn emit_spawn_actor_with_inits(
        &mut self,
        actor_name: &str,
        inits: &[(crate::intern::Symbol, BasicValueEnum<'ctx>)],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_spawn_with_init_vals(actor_name, inits)
    }

    pub(super) fn emit_actor_send(
        &mut self,
        encoded: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        let (actor_hint, handler_name) = match encoded.split_once('.') {
            Some((a, h)) => (Some(a), h),
            None => (None, encoded),
        };

        let (actor_name, tag, handler_params) = {
            let mut found = None;
            for (aname, ad) in &self.actor_defs {
                if let Some(hint) = actor_hint {
                    if aname.as_str() != hint {
                        continue;
                    }
                }
                for h in &ad.handlers {
                    if h.is_loop {
                        continue;
                    }
                    if h.name == handler_name {
                        let param_tys: Vec<Type> = h.params.iter().map(|p| p.ty.clone()).collect();
                        found = Some((aname.clone(), h.tag, param_tys));
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            found.ok_or_else(|| format!("unknown actor handler '{encoded}'"))?
        };

        let mb_name = format!("{actor_name}_mailbox");
        let msg_name = format!("{actor_name}_msg");

        let mb_st = self
            .module
            .get_struct_type(&mb_name)
            .ok_or_else(|| format!("mailbox type '{mb_name}' not found"))?;
        let msg_st = self
            .module
            .get_struct_type(&msg_name)
            .ok_or_else(|| format!("message type '{msg_name}' not found"))?;

        let mb_ptr = self.val(args[0]).into_pointer_value();

        let ch_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 0, "ch_ptr_ptr"));
        let ch_ptr = b!(self.bld.build_load(ptr_ty, ch_ptr_ptr, "ch_ptr"));

        let msg_alloca = self.entry_alloca(msg_st.into(), "send_msg");

        let tag_ptr = b!(self.bld.build_struct_gep(msg_st, msg_alloca, 0, "tag_ptr"));
        b!(self
            .bld
            .build_store(tag_ptr, i32t.const_int(tag as u64, false)));

        let payload_ptr = b!(self
            .bld
            .build_struct_gep(msg_st, msg_alloca, 1, "payload_ptr"));

        let mut arg_offset: u64 = 0;
        for (i, param_ty) in handler_params.iter().enumerate() {
            if i + 1 >= args.len() {
                break;
            }
            let val = self.val(args[i + 1]);
            let pty = self.llvm_ty(param_ty);
            let psize = self.type_store_size(pty);
            let offset_val = i64t.const_int(arg_offset, false);
            let dest = unsafe {
                b!(self.bld.build_gep(
                    self.ctx.i8_type(),
                    payload_ptr,
                    &[offset_val.into()],
                    "arg_ptr"
                ))
            };
            b!(self.bld.build_store(dest, val));
            arg_offset += psize;
        }

        let chan_send = self
            .module
            .get_function("jinn_chan_send")
            .ok_or("jinn_chan_send not declared")?;
        b!(self
            .bld
            .build_call(chan_send, &[ch_ptr.into(), msg_alloca.into()], ""));

        Ok(i64t.const_int(0, false).into())
    }

    pub(super) fn emit_select(
        &mut self,
        channels: &[mir::ValueId],
        dest: mir::ValueId,
        has_default: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let n = channels.len();

        let case_struct_ty = self
            .ctx
            .struct_type(&[ptr_ty.into(), ptr_ty.into(), i32t.into()], false);
        let cases_array_ty = case_struct_ty.array_type(n as u32);
        let cases_alloca = self.entry_alloca(cases_array_ty.into(), "select.cases");

        let mut data_bufs = Vec::new();
        for (i, ch_vid) in channels.iter().enumerate() {
            let ch_val = self.val(*ch_vid).into_pointer_value();

            let data_alloca = self.entry_alloca(i64t.into(), &format!("select.data.{i}"));
            data_bufs.push(data_alloca);

            let idx0 = i32t.const_int(0, false);
            let idx_i = i32t.const_int(i as u64, false);
            let case_ptr = unsafe {
                b!(self.bld.build_gep(
                    cases_array_ty,
                    cases_alloca,
                    &[idx0, idx_i],
                    &format!("select.case.{i}")
                ))
            };

            let chan_field =
                b!(self
                    .bld
                    .build_struct_gep(case_struct_ty, case_ptr, 0, "case.chan"));
            b!(self.bld.build_store(chan_field, ch_val));

            let data_field =
                b!(self
                    .bld
                    .build_struct_gep(case_struct_ty, case_ptr, 1, "case.data"));
            b!(self.bld.build_store(data_field, data_alloca));

            let is_send_field =
                b!(self
                    .bld
                    .build_struct_gep(case_struct_ty, case_ptr, 2, "case.is_send"));
            b!(self
                .bld
                .build_store(is_send_field, i32t.const_int(0, false)));
        }

        self.select_data_bufs.insert(dest, data_bufs);

        let select_fn = self
            .module
            .get_function("jinn_select")
            .ok_or("jinn_select not declared")?;
        let has_default = self.ctx.bool_type().const_int(has_default as u64, false);
        let result = b!(self.bld.build_call(
            select_fn,
            &[
                cases_alloca.into(),
                i32t.const_int(n as u64, false).into(),
                has_default.into(),
            ],
            "select.result"
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void");

        Ok(result)
    }
}
