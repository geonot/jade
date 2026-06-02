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
            let loop_fn_name = crate::mir::actor_handler_fn_name(ad.name.clone(), loop_h);
            let loop_fv = self.module.get_function(&loop_fn_name).unwrap_or_else(|| {
                panic!("ICE: actor loop handler fn not lowered: {loop_fn_name}")
            });
            b!(self.bld.build_call(loop_fv, &[state_ptr.into()], ""));

            let sched_yield = crate::codegen::fn_or_die(&self.module, "jinn_sched_yield");
            if loop_h.loop_sleep_ms.is_some() {
                let i64t = self.ctx.i64_type();

                let sleep_fn_name = crate::mir::actor_sleep_fn_name(ad.name.clone());
                let sleep_fv = self
                    .module
                    .get_function(&sleep_fn_name)
                    .unwrap_or_else(|| panic!("ICE: actor sleep fn not lowered: {sleep_fn_name}"));
                let sleep_ms = b!(self
                    .bld
                    .build_call(sleep_fv, &[state_ptr.into()], "sleep_ms"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: sleep fn returned void")
                .into_int_value();
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
                self.emit_sleep_ms_val(sleep_ms)?;
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

            for (i, h) in message_handlers.iter().enumerate() {
                let bb = handler_bbs[i].1;
                self.bld.position_at_end(bb);

                let has_self = h.params.first().is_some_and(|p| p.name.as_str() == "self");
                let msg_params: &[hir::Param] = if has_self {
                    &h.params[1..]
                } else {
                    &h.params[..]
                };

                let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                    vec![state_ptr.into()];

                let i64t = self.ctx.i64_type();
                let mut param_offset: u64 = 0;
                for p in msg_params {
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

                    if matches!(p.ty, Type::Struct(..) | Type::Tuple(..) | Type::Enum(..)) {
                        call_args.push(param_ptr.into());
                    } else {
                        let param_val = b!(self.bld.build_load(pty, param_ptr, &p.name.as_str()));
                        call_args.push(param_val.into());
                    }
                    param_offset += psize;
                }

                let handler_fn_name = crate::mir::actor_handler_fn_name(ad.name.clone(), h);
                let handler_fv = self
                    .module
                    .get_function(&handler_fn_name)
                    .unwrap_or_else(|| {
                        panic!("ICE: actor handler fn not lowered: {handler_fn_name}")
                    });
                b!(self.bld.build_call(handler_fv, &call_args, ""));

                if self.no_term() {
                    b!(self.bld.build_unconditional_branch(loop_bb));
                }
            }
        }

        self.bld.position_at_end(exit_bb);

        if let Some(destroy_fn) = self.module.get_function("jinn_actor_destroy") {
            b!(self.bld.build_call(destroy_fn, &[mb_ptr.into()], ""));
        }
        b!(self.bld.build_return(None));

        self.cur_fn = old_fn;
        Ok(())
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

        if let Some(ad) = self.actor_defs.get(actor_name).cloned() {
            let state_name = format!("{actor_name}_state");
            if let Some(state_st) = self.module.get_struct_type(&state_name) {
                let state_ptr = b!(self
                    .bld
                    .build_struct_gep(mb_st, mb_ptr_v, 2, "state_init_ptr"));
                let init_fn = self
                    .module
                    .get_function(&crate::mir::actor_init_fn_name(ad.name.clone()))
                    .unwrap_or_else(|| panic!("ICE: actor init fn not lowered: {actor_name}"));
                b!(self.bld.build_call(init_fn, &[state_ptr.into()], ""));

                for (uname, uval) in inits {
                    let fi = ad
                        .fields
                        .iter()
                        .position(|f| f.name == *uname)
                        .ok_or_else(|| format!("spawn '{actor_name}': unknown field '{uname}'"))?;
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
            let state_ptr = b!(self
                .bld
                .build_struct_gep(mb_st, mb_ptr_v, 2, "state_init_ptr"));
            let init_fn = self
                .module
                .get_function(&crate::mir::actor_init_fn_name(ad.name.clone()))
                .unwrap_or_else(|| panic!("ICE: actor init fn not lowered: {actor_name}"));
            b!(self.bld.build_call(init_fn, &[state_ptr.into()], ""));
        }

        b!(self.bld.build_return(Some(&mb_ptr_v)));

        self.cur_fn = old_fn;
        if let Some(bb) = old_bb {
            self.bld.position_at_end(bb);
        }
        Ok(fv)
    }
}
