//! Codegen for actor spawn/send/become/supervisor operations.

use inkwell::module::Linkage;
use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

const MAILBOX_CAP: u64 = 256;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn declare_actor_runtime(&mut self) {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        if self.module.get_function("malloc").is_none() {
            let ft = ptr.fn_type(&[i64t.into()], false);
            self.module
                .add_function("malloc", ft, Some(Linkage::External));
        }
        if self.module.get_function("memset").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), i32t.into(), i64t.into()], false);
            self.module
                .add_function("memset", ft, Some(Linkage::External));
        }
        if self.module.get_function("free").is_none() {
            let ft = self.ctx.void_type().fn_type(&[ptr.into()], false);
            self.module
                .add_function("free", ft, Some(Linkage::External));
        }
    }

    pub(crate) fn declare_actor(&mut self, ad: &hir::ActorDef) -> Result<(), String> {
        let name = &ad.name;

        let state_name = format!("{name}_state");
        let state_fields: Vec<(String, Type)> = ad
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.ty.clone()))
            .collect();
        let state_ltys: Vec<BasicTypeEnum<'ctx>> =
            state_fields.iter().map(|(_, t)| self.llvm_ty(t)).collect();
        let state_st = self.ctx.opaque_struct_type(&state_name);
        state_st.set_body(&state_ltys, false);
        self.structs.insert(state_name.into(), state_fields);

        let mut max_payload_bytes: u64 = 8;
        for h in &ad.handlers {
            if h.is_loop {
                continue;
            }
            let mut handler_size: u64 = 0;
            for p in &h.params {
                handler_size += self.type_store_size(self.llvm_ty(&p.ty));
            }
            max_payload_bytes = max_payload_bytes.max(handler_size);
        }

        let msg_name = format!("{name}_msg");
        let msg_st = self.ctx.opaque_struct_type(&msg_name);
        let i32t = self.ctx.i32_type();
        let payload_ty = self.ctx.i8_type().array_type(max_payload_bytes as u32);
        msg_st.set_body(&[i32t.into(), payload_ty.into()], false);

        let mb_name = format!("{name}_mailbox");
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let mb_st = self.ctx.opaque_struct_type(&mb_name);
        mb_st.set_body(&[ptr_ty.into(), i32t.into(), state_st.into()], false);

        Ok(())
    }

    pub(crate) fn compile_actor_loop(&mut self, ad: &hir::ActorDef) -> Result<(), String> {
        let name = &ad.name;
        let mb_name = format!("{name}_mailbox");
        let msg_name = format!("{name}_msg");
        let loop_name = format!("{name}_loop");

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();

        let mb_st = self
            .module
            .get_struct_type(&mb_name)
            .expect("ICE: struct type not declared");
        let msg_st = self
            .module
            .get_struct_type(&msg_name)
            .expect("ICE: struct type not declared");

        let loop_handler = ad.handlers.iter().find(|h| h.is_loop);
        let message_handlers: Vec<&hir::HandlerDef> =
            ad.handlers.iter().filter(|h| !h.is_loop).collect();

        let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
        let fv = self
            .module
            .add_function(&loop_name, ft, Some(Linkage::Internal));

        let entry = self.ctx.append_basic_block(fv, "entry");
        let loop_bb = self.ctx.append_basic_block(fv, "loop");
        let dispatch_bb = self.ctx.append_basic_block(fv, "dispatch");
        let exit_bb = self.ctx.append_basic_block(fv, "exit");

        let old_fn = self.cur_fn;
        self.cur_fn = Some(fv);

        let mb_ptr = fv
            .get_nth_param(0)
            .expect("ICE: missing param")
            .into_pointer_value();

        self.bld.position_at_end(entry);
        let ch_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 0, "ch_ptr_ptr"));
        let ch_ptr = b!(self.bld.build_load(ptr_ty, ch_ptr_ptr, "ch_ptr")).into_pointer_value();
        let state_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 2, "state_ptr"));

        let msg_alloca = self.entry_alloca(msg_st.into(), "msg_buf");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        if let Some(loop_h) = loop_handler {
            self.push_var_scope();

            let state_name = format!("{name}_state");
            let state_st = self
                .module
                .get_struct_type(&state_name)
                .expect("ICE: struct type not declared");
            for (fi, field) in ad.fields.iter().enumerate() {
                let field_ptr = b!(self.bld.build_struct_gep(
                    state_st,
                    state_ptr,
                    fi as u32,
                    &format!("state_{}", field.name)
                ));
                self.set_var(&field.name.as_str(), field_ptr, field.ty.clone());
            }

            self.compile_block(&loop_h.body)?;
            self.pop_var_scope();

            if !self.no_term() {
                return Err(format!(
                    "actor '{name}': *loop handler cannot terminate control flow"
                ));
            }

            let sched_yield = crate::codegen::fn_or_die(&self.module, "jinn_sched_yield");
            if let Some(sleep_expr) = &loop_h.loop_sleep_ms {
                let i64t = self.ctx.i64_type();
                let sleep_ms = self.compile_expr(sleep_expr)?.into_int_value();
                let should_sleep = b!(self.bld.build_int_compare(
                    IntPredicate::SGT,
                    sleep_ms,
                    i64t.const_int(0, false),
                    "loop_should_sleep"
                ));

                let sleep_bb = self.ctx.append_basic_block(fv, "loop_sleep");
                let yield_bb = self.ctx.append_basic_block(fv, "loop_yield");
                let pause_done_bb = self.ctx.append_basic_block(fv, "loop_pause_done");
                b!(self
                    .bld
                    .build_conditional_branch(should_sleep, sleep_bb, yield_bb));

                self.bld.position_at_end(sleep_bb);
                let _ = self.compile_sleep_ms(std::slice::from_ref(sleep_expr))?;
                b!(self.bld.build_unconditional_branch(pause_done_bb));

                self.bld.position_at_end(yield_bb);
                b!(self.bld.build_call(sched_yield, &[], ""));
                b!(self.bld.build_unconditional_branch(pause_done_bb));

                self.bld.position_at_end(pause_done_bb);
            } else {
                b!(self.bld.build_call(sched_yield, &[], ""));
            }

            let chan_try_recv = crate::codegen::fn_or_die(&self.module, "jinn_chan_try_recv");
            let recv_state = b!(self.bld.build_call(
                chan_try_recv,
                &[ch_ptr.into(), msg_alloca.into()],
                "recv_state"
            ))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void")
            .into_int_value();

            let got_msg = b!(self.bld.build_int_compare(
                IntPredicate::EQ,
                recv_state,
                i32t.const_int(1, false),
                "got_msg"
            ));
            let check_closed_bb = self.ctx.append_basic_block(fv, "check_closed");
            b!(self
                .bld
                .build_conditional_branch(got_msg, dispatch_bb, check_closed_bb));

            self.bld.position_at_end(check_closed_bb);
            let is_closed = b!(self.bld.build_int_compare(
                IntPredicate::EQ,
                recv_state,
                i32t.const_int(u32::MAX as u64, false),
                "is_closed"
            ));
            b!(self
                .bld
                .build_conditional_branch(is_closed, exit_bb, loop_bb));
        } else {
            let chan_recv = crate::codegen::fn_or_die(&self.module, "jinn_chan_recv");
            let recv_ok =
                b!(self
                    .bld
                    .build_call(chan_recv, &[ch_ptr.into(), msg_alloca.into()], "recv_ok"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_int_value();

            let ok = b!(self.bld.build_int_compare(
                IntPredicate::NE,
                recv_ok,
                i32t.const_int(0, false),
                "ok"
            ));
            b!(self.bld.build_conditional_branch(ok, dispatch_bb, exit_bb));
        }

        self.bld.position_at_end(dispatch_bb);
        let tag_ptr = b!(self.bld.build_struct_gep(msg_st, msg_alloca, 0, "tag_ptr"));
        let tag_val = b!(self.bld.build_load(i32t, tag_ptr, "tag"));
        let payload_ptr = b!(self
            .bld
            .build_struct_gep(msg_st, msg_alloca, 1, "payload_ptr"));

        if message_handlers.is_empty() {
            b!(self.bld.build_unconditional_branch(loop_bb));
        } else {
            let mut handler_bbs = Vec::new();
            for h in &message_handlers {
                let bb = self
                    .ctx
                    .append_basic_block(fv, &format!("handler_{}", h.name));
                handler_bbs.push((h.tag, bb));
            }

            let default_bb = self.ctx.append_basic_block(fv, "default_handler");
            self.bld.position_at_end(default_bb);
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(dispatch_bb);
            let _switch = b!(self.bld.build_switch(
                tag_val.into_int_value(),
                default_bb,
                &handler_bbs
                    .iter()
                    .map(|(tag, bb)| (i32t.const_int(*tag as u64, false), *bb))
                    .collect::<Vec<_>>()
            ));

            let state_name = format!("{name}_state");
            let state_st = self
                .module
                .get_struct_type(&state_name)
                .expect("ICE: struct type not declared");

            for (i, h) in message_handlers.iter().enumerate() {
                let bb = handler_bbs[i].1;
                self.bld.position_at_end(bb);

                self.push_var_scope();

                for (fi, field) in ad.fields.iter().enumerate() {
                    let field_ptr = b!(self.bld.build_struct_gep(
                        state_st,
                        state_ptr,
                        fi as u32,
                        &format!("state_{}", field.name)
                    ));
                    self.set_var(&field.name.as_str(), field_ptr, field.ty.clone());
                }

                let i64t = self.ctx.i64_type();
                let mut param_offset: u64 = 0;
                for p in &h.params {
                    let pty = self.llvm_ty(&p.ty);
                    let psize = self.type_store_size(pty);
                    let offset_val = i64t.const_int(param_offset, false);
                    let param_ptr = unsafe {
                        b!(self.bld.build_gep(
                            self.ctx.i8_type(),
                            payload_ptr,
                            &[offset_val.into()],
                            &format!("param_{}_ptr", p.name)
                        ))
                    };
                    let param_val = b!(self.bld.build_load(pty, param_ptr, &p.name.as_str()));

                    let alloca = self.entry_alloca(pty, &p.name.as_str());
                    b!(self.bld.build_store(alloca, param_val));
                    self.set_var(&p.name.as_str(), alloca, p.ty.clone());

                    param_offset += psize;
                }

                self.compile_block(&h.body)?;

                if self.no_term() {
                    b!(self.bld.build_unconditional_branch(loop_bb));
                }

                self.pop_var_scope();
            }
        }

        self.bld.position_at_end(exit_bb);
        // Clean up: destroy the mailbox (frees channel + mailbox memory)
        if let Some(destroy_fn) = self.module.get_function("jinn_actor_destroy") {
            b!(self.bld.build_call(destroy_fn, &[mb_ptr.into()], ""));
        }
        b!(self.bld.build_return(None));

        self.cur_fn = old_fn;
        Ok(())
    }

    pub(crate) fn compile_spawn(
        &mut self,
        actor_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_spawn_with_inits(actor_name, &[])
    }

    /// Synthesize the initial value for an actor state field. Honors the
    /// declared default if any; otherwise returns an empty container for
    /// collection types (Vec/Map/Set/PriorityQueue/Deque) so they aren't
    /// left as null pointers from the mailbox memset. Scalars and other
    /// types return None — they remain zero-initialized.
    pub(crate) fn synthesize_field_init(
        &mut self,
        field: &crate::hir::Field,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        if let Some(ref default_expr) = field.default {
            return Ok(Some(self.compile_expr(default_expr)?));
        }
        match &field.ty {
            Type::Vec(_) => Ok(Some(self.compile_vec_new(&[])?)),
            Type::Map(_, _) => Ok(Some(self.compile_map_new()?)),
            _ => Ok(None),
        }
    }

    /// Spawn an actor and apply user-provided field-init overrides
    /// (`spawn Foo(field is val, ...)`). Defaults from the actor declaration
    /// are written first; any user inits then override them.
    pub(crate) fn compile_spawn_with_inits(
        &mut self,
        actor_name: &str,
        inits: &[(crate::intern::Symbol, crate::hir::Expr)],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lowered: Vec<(crate::intern::Symbol, BasicValueEnum<'ctx>)> = inits
            .iter()
            .map(|(name, e)| Ok::<_, String>((*name, self.compile_expr(e)?)))
            .collect::<Result<_, _>>()?;
        self.compile_spawn_with_init_vals(actor_name, &lowered)
    }

    pub(crate) fn compile_spawn_with_init_vals(
        &mut self,
        actor_name: &str,
        inits: &[(crate::intern::Symbol, BasicValueEnum<'ctx>)],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let mb_name = format!("{actor_name}_mailbox");
        let msg_name = format!("{actor_name}_msg");
        let loop_name = format!("{actor_name}_loop");

        let _ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        let mb_st = self
            .module
            .get_struct_type(&mb_name)
            .ok_or_else(|| format!("actor '{actor_name}' not declared"))?;
        let msg_st = self
            .module
            .get_struct_type(&msg_name)
            .expect("ICE: struct type not declared");

        let mb_size = self.type_store_size(mb_st.into());
        let msg_size = self.type_store_size(msg_st.into());

        let malloc_fn = crate::codegen::fn_or_die(&self.module, "malloc");
        let memset_fn = crate::codegen::fn_or_die(&self.module, "memset");

        let mb_ptr = b!(self.bld.build_call(
            malloc_fn,
            &[i64t.const_int(mb_size, false).into()],
            "mb_raw"
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void");
        b!(self.bld.build_call(
            memset_fn,
            &[
                mb_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(mb_size, false).into()
            ],
            ""
        ));

        let mb_ptr_v = mb_ptr.into_pointer_value();

        let chan_create = crate::codegen::fn_or_die(&self.module, "jinn_chan_create");
        let ch = b!(self.bld.build_call(
            chan_create,
            &[
                i64t.const_int(msg_size, false).into(),
                i64t.const_int(MAILBOX_CAP, false).into(),
            ],
            "actor_ch"
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void");

        let ch_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 0, "ch_ptr_ptr"));
        b!(self.bld.build_store(ch_ptr_ptr, ch));

        let alive_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 1, "alive_ptr"));
        b!(self.bld.build_store(alive_ptr, i32t.const_int(1, false)));

        // Initialize actor state fields with default values (if any)
        if let Some(ad) = self.actor_defs.get(actor_name).cloned() {
            let state_name = format!("{actor_name}_state");
            if let Some(state_st) = self.module.get_struct_type(&state_name) {
                let state_ptr = b!(self
                    .bld
                    .build_struct_gep(mb_st, mb_ptr_v, 2, "state_init_ptr"));
                for (fi, field) in ad.fields.iter().enumerate() {
                    let field_ptr = b!(self.bld.build_struct_gep(
                        state_st,
                        state_ptr,
                        fi as u32,
                        &format!("state_init_{}", field.name)
                    ));
                    let val = self.synthesize_field_init(field)?;
                    if let Some(v) = val {
                        b!(self.bld.build_store(field_ptr, v));
                    }
                }
                // Apply user-provided spawn-init overrides
                for (uname, uval) in inits {
                    let fi = ad
                        .fields
                        .iter()
                        .position(|f| f.name == *uname)
                        .ok_or_else(|| {
                            format!("spawn '{actor_name}': unknown field '{uname}'")
                        })?;
                    let field_ptr = b!(self.bld.build_struct_gep(
                        state_st,
                        state_ptr,
                        fi as u32,
                        &format!("state_user_{}", uname)
                    ));
                    b!(self.bld.build_store(field_ptr, *uval));
                }
            }
        }

        let loop_fn = self
            .module
            .get_function(&loop_name)
            .ok_or_else(|| format!("actor loop fn '{loop_name}' not found"))?;
        let coro_create = crate::codegen::fn_or_die(&self.module, "jinn_coro_create");
        let coro = b!(self.bld.build_call(
            coro_create,
            &[
                loop_fn.as_global_value().as_pointer_value().into(),
                mb_ptr_v.into(),
            ],
            "actor_coro"
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void");

        let set_daemon = crate::codegen::fn_or_die(&self.module, "jinn_coro_set_daemon");
        b!(self.bld.build_call(set_daemon, &[coro.into()], ""));

        let sched_spawn = crate::codegen::fn_or_die(&self.module, "jinn_sched_spawn");
        b!(self.bld.build_call(sched_spawn, &[coro.into()], ""));

        Ok(mb_ptr_v.into())
    }

    /// Emit (lazily) a per-actor factory function:
    ///   void *<actor>_create_mb(void)
    /// Allocates and initialises a fresh mailbox (channel + alive flag +
    /// state defaults). Used by the supervisor runtime to (re)spawn this
    /// actor without compile-time knowledge of its layout.
    pub(crate) fn ensure_actor_factory(
        &mut self,
        actor_name: &str,
    ) -> Result<inkwell::values::FunctionValue<'ctx>, String> {
        let fn_name = format!("{actor_name}_create_mb");
        if let Some(fv) = self.module.get_function(&fn_name) {
            return Ok(fv);
        }

        let mb_name = format!("{actor_name}_mailbox");
        let msg_name = format!("{actor_name}_msg");

        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        let mb_st = self
            .module
            .get_struct_type(&mb_name)
            .ok_or_else(|| format!("actor '{actor_name}' not declared"))?;
        let msg_st = self
            .module
            .get_struct_type(&msg_name)
            .expect("ICE: struct type not declared");

        let mb_size = self.type_store_size(mb_st.into());
        let msg_size = self.type_store_size(msg_st.into());

        let ft = ptr.fn_type(&[], false);
        let fv = self
            .module
            .add_function(&fn_name, ft, Some(Linkage::Internal));
        let entry = self.ctx.append_basic_block(fv, "entry");

        let old_fn = self.cur_fn;
        let old_bb = self.bld.get_insert_block();
        self.cur_fn = Some(fv);
        self.bld.position_at_end(entry);

        let malloc_fn = crate::codegen::fn_or_die(&self.module, "malloc");
        let memset_fn = crate::codegen::fn_or_die(&self.module, "memset");
        let chan_create = crate::codegen::fn_or_die(&self.module, "jinn_chan_create");

        let mb_ptr = b!(self.bld.build_call(
            malloc_fn,
            &[i64t.const_int(mb_size, false).into()],
            "mb_raw"
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void");
        b!(self.bld.build_call(
            memset_fn,
            &[
                mb_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(mb_size, false).into(),
            ],
            ""
        ));
        let mb_ptr_v = mb_ptr.into_pointer_value();

        let ch = b!(self.bld.build_call(
            chan_create,
            &[
                i64t.const_int(msg_size, false).into(),
                i64t.const_int(MAILBOX_CAP, false).into(),
            ],
            "actor_ch"
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void");

        let ch_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 0, "ch_ptr_ptr"));
        b!(self.bld.build_store(ch_ptr_ptr, ch));

        let alive_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 1, "alive_ptr"));
        b!(self.bld.build_store(alive_ptr, i32t.const_int(1, false)));

        if let Some(ad) = self.actor_defs.get(actor_name).cloned() {
            let state_name = format!("{actor_name}_state");
            if let Some(state_st) = self.module.get_struct_type(&state_name) {
                let state_ptr = b!(self
                    .bld
                    .build_struct_gep(mb_st, mb_ptr_v, 2, "state_init_ptr"));
                for (fi, field) in ad.fields.iter().enumerate() {
                    let field_ptr = b!(self.bld.build_struct_gep(
                        state_st,
                        state_ptr,
                        fi as u32,
                        &format!("state_init_{}", field.name)
                    ));
                    let val = self.synthesize_field_init(field)?;
                    if let Some(v) = val {
                        b!(self.bld.build_store(field_ptr, v));
                    }
                }
            }
        }

        b!(self.bld.build_return(Some(&mb_ptr_v)));

        self.cur_fn = old_fn;
        if let Some(bb) = old_bb {
            self.bld.position_at_end(bb);
        }
        Ok(fv)
    }

    pub(crate) fn compile_send(
        &mut self,
        target: &hir::Expr,
        actor_name: &str,
        _handler_name: &str,
        tag: u32,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let mb_name = format!("{actor_name}_mailbox");
        let msg_name = format!("{actor_name}_msg");

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        let mb_st = self
            .module
            .get_struct_type(&mb_name)
            .ok_or_else(|| format!("mailbox type '{mb_name}' not found"))?;
        let msg_st = self
            .module
            .get_struct_type(&msg_name)
            .ok_or_else(|| format!("message type '{msg_name}' not found"))?;

        let mb_ptr = self.compile_expr(target)?.into_pointer_value();

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
        for arg in args {
            let val = self.compile_expr(arg)?;
            let pty = self.llvm_ty(&arg.ty);
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

        let chan_send = crate::codegen::fn_or_die(&self.module, "jinn_chan_send");
        b!(self
            .bld
            .build_call(chan_send, &[ch_ptr.into(), msg_alloca.into()], ""));

        Ok(i64t.const_int(0, false).into())
    }
}
