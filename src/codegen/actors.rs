//! Actor system codegen: mailbox structures, spawn, send, actor loop.
//!
//! Runtime model (per actor type):
//!   - State struct: { field0, field1, ... }
//!   - Message struct: tag (i32) + max-sized payload
//!   - Mailbox struct: { ptr channel, i32 alive, state_struct }
//!   - Actor loop fn: recv from channel → switch on tag → dispatch to handler
//!   - Spawn: malloc mailbox + create channel + create coroutine + sched_spawn
//!   - Send: write msg to stack + jade_chan_send
//!
//! Actors are daemon coroutines (don't block jade_sched_run) that receive
//! messages through the existing typed channel infrastructure, giving us
//! scheduler-integrated park/wake, backpressure, and M:N scheduling for free.

use inkwell::module::Linkage;
use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

/// Mailbox capacity (bounded, backpressure on full).
const MAILBOX_CAP: u64 = 256;

impl<'ctx> Compiler<'ctx> {
    /// Ensure basic C functions needed for actor codegen.
    /// The jade_* runtime functions are declared by declare_jade_runtime().
    pub(crate) fn declare_actor_runtime(&mut self) {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        // malloc
        if self.module.get_function("malloc").is_none() {
            let ft = ptr.fn_type(&[i64t.into()], false);
            self.module.add_function("malloc", ft, Some(Linkage::External));
        }
        // memset
        if self.module.get_function("memset").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), i32t.into(), i64t.into()], false);
            self.module.add_function("memset", ft, Some(Linkage::External));
        }
        // free
        if self.module.get_function("free").is_none() {
            let ft = self.ctx.void_type().fn_type(&[ptr.into()], false);
            self.module.add_function("free", ft, Some(Linkage::External));
        }
    }

    /// Create the LLVM struct types for an actor:
    /// - `{ActorName}_state`: actor fields
    /// - `{ActorName}_msg`: { i32 tag, [payload_size x i8] }
    /// - `{ActorName}_mailbox`: { ptr channel, i32 alive, state_struct }
    pub(crate) fn declare_actor(&mut self, ad: &hir::ActorDef) -> Result<(), String> {
        let name = &ad.name;

        // State struct
        let state_name = format!("{name}_state");
        let state_fields: Vec<(String, Type)> = ad
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ty.clone()))
            .collect();
        let state_ltys: Vec<BasicTypeEnum<'ctx>> = state_fields
            .iter()
            .map(|(_, t)| self.llvm_ty(t))
            .collect();
        let state_st = self.ctx.opaque_struct_type(&state_name);
        state_st.set_body(&state_ltys, false);
        self.structs.insert(state_name.clone(), state_fields);

        // Compute max message payload size across all handlers
        let mut max_payload_bytes: u64 = 8; // minimum payload
        for h in &ad.handlers {
            let mut handler_size: u64 = 0;
            for p in &h.params {
                handler_size += self.type_store_size(self.llvm_ty(&p.ty));
            }
            max_payload_bytes = max_payload_bytes.max(handler_size);
        }

        // Message struct: { i32 tag, [max_payload_bytes x i8] payload }
        let msg_name = format!("{name}_msg");
        let msg_st = self.ctx.opaque_struct_type(&msg_name);
        let i32t = self.ctx.i32_type();
        let payload_ty = self.ctx.i8_type().array_type(max_payload_bytes as u32);
        msg_st.set_body(&[i32t.into(), payload_ty.into()], false);

        // Mailbox struct:
        //   ptr          channel     (jade_chan_t*)
        //   i32          alive       (1 = running, 0 = stopped)
        //   state_struct state       (actor fields inline)
        let mb_name = format!("{name}_mailbox");
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let mb_st = self.ctx.opaque_struct_type(&mb_name);
        mb_st.set_body(
            &[
                ptr_ty.into(),    // 0: channel
                i32t.into(),      // 1: alive
                state_st.into(),  // 2: state
            ],
            false,
        );

        Ok(())
    }

    /// Generate the actor loop function: `void {name}_loop(ptr mailbox_arg)`
    /// This runs as a coroutine, receiving messages from the channel.
    pub(crate) fn compile_actor_loop(
        &mut self,
        ad: &hir::ActorDef,
    ) -> Result<(), String> {
        let name = &ad.name;
        let mb_name = format!("{name}_mailbox");
        let msg_name = format!("{name}_msg");
        let loop_name = format!("{name}_loop");

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();

        let mb_st = self.module.get_struct_type(&mb_name).unwrap();
        let msg_st = self.module.get_struct_type(&msg_name).unwrap();

        // Coroutine entry signature: void (ptr)
        let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
        let fv = self.module.add_function(&loop_name, ft, Some(Linkage::Internal));

        let entry = self.ctx.append_basic_block(fv, "entry");
        let loop_bb = self.ctx.append_basic_block(fv, "loop");
        let dispatch_bb = self.ctx.append_basic_block(fv, "dispatch");
        let exit_bb = self.ctx.append_basic_block(fv, "exit");

        // Save and set state
        let old_fn = self.cur_fn;
        self.cur_fn = Some(fv);

        let mb_ptr = fv.get_nth_param(0).unwrap().into_pointer_value();

        // Entry: compute GEPs, alloca for message buffer, jump to loop
        self.bld.position_at_end(entry);
        let ch_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 0, "ch_ptr_ptr"));
        let ch_ptr = b!(self.bld.build_load(ptr_ty, ch_ptr_ptr, "ch_ptr")).into_pointer_value();
        let state_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 2, "state_ptr"));

        // Allocate stack space for one message
        let msg_alloca = self.entry_alloca(msg_st.into(), "msg_buf");

        b!(self.bld.build_unconditional_branch(loop_bb));

        // Loop: receive message from channel
        self.bld.position_at_end(loop_bb);
        let chan_recv = self.module.get_function("jade_chan_recv").unwrap();
        let recv_ok = b!(self.bld.build_call(
            chan_recv,
            &[ch_ptr.into(), msg_alloca.into()],
            "recv_ok"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();

        // If recv returns 0 (channel closed), exit
        let ok = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            recv_ok,
            i32t.const_int(0, false),
            "ok"
        ));
        b!(self.bld.build_conditional_branch(ok, dispatch_bb, exit_bb));

        // Dispatch: read tag, switch to handlers
        self.bld.position_at_end(dispatch_bb);
        let tag_ptr = b!(self.bld.build_struct_gep(msg_st, msg_alloca, 0, "tag_ptr"));
        let tag_val = b!(self.bld.build_load(i32t, tag_ptr, "tag"));
        let payload_ptr = b!(self.bld.build_struct_gep(msg_st, msg_alloca, 1, "payload_ptr"));

        if ad.handlers.is_empty() {
            b!(self.bld.build_unconditional_branch(loop_bb));
        } else {
            let mut handler_bbs = Vec::new();
            for h in &ad.handlers {
                let bb = self.ctx.append_basic_block(fv, &format!("handler_{}", h.name));
                handler_bbs.push((h.tag, bb));
            }

            let default_bb = self.ctx.append_basic_block(fv, "default_handler");
            self.bld.position_at_end(default_bb);
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(dispatch_bb);
            let _switch = b!(self.bld.build_switch(
                tag_val.into_int_value(),
                default_bb,
                &handler_bbs.iter().map(|(tag, bb)| (i32t.const_int(*tag as u64, false), *bb)).collect::<Vec<_>>()
            ));

            // Generate handler bodies
            let state_name = format!("{name}_state");
            let state_st = self.module.get_struct_type(&state_name).unwrap();

            for (i, h) in ad.handlers.iter().enumerate() {
                let bb = handler_bbs[i].1;
                self.bld.position_at_end(bb);

                self.vars.push(std::collections::HashMap::new());

                // Bind state fields as variables
                for (fi, field) in ad.fields.iter().enumerate() {
                    let field_ptr = b!(self.bld.build_struct_gep(
                        state_st,
                        state_ptr,
                        fi as u32,
                        &format!("state_{}", field.name)
                    ));
                    self.set_var(&field.name, field_ptr, field.ty.clone());
                }

                // Extract handler params from payload
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
                    let param_val = b!(self.bld.build_load(pty, param_ptr, &p.name));

                    let alloca = self.entry_alloca(pty, &p.name);
                    b!(self.bld.build_store(alloca, param_val));
                    self.set_var(&p.name, alloca, p.ty.clone());

                    param_offset += psize;
                }

                // Compile handler body
                self.compile_block(&h.body)?;

                // Branch back to loop if no terminator
                if self.no_term() {
                    b!(self.bld.build_unconditional_branch(loop_bb));
                }

                self.vars.pop();
            }
        }

        // Exit: return void (coroutine trampoline marks it DONE)
        self.bld.position_at_end(exit_bb);
        b!(self.bld.build_return(None));

        self.cur_fn = old_fn;
        Ok(())
    }

    /// Compile a `spawn ActorName` expression → returns ActorRef (pointer to mailbox).
    /// Creates a channel, mallocs the mailbox, creates a daemon coroutine, and
    /// spawns it on the scheduler.
    pub(crate) fn compile_spawn(&mut self, actor_name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let mb_name = format!("{actor_name}_mailbox");
        let msg_name = format!("{actor_name}_msg");
        let loop_name = format!("{actor_name}_loop");

        let _ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        let mb_st = self.module.get_struct_type(&mb_name)
            .ok_or_else(|| format!("actor '{actor_name}' not declared"))?;
        let msg_st = self.module.get_struct_type(&msg_name).unwrap();

        let mb_size = self.type_store_size(mb_st.into());
        let msg_size = self.type_store_size(msg_st.into());

        let malloc_fn = self.module.get_function("malloc").unwrap();
        let memset_fn = self.module.get_function("memset").unwrap();

        // Allocate mailbox
        let mb_ptr = b!(self.bld.build_call(
            malloc_fn,
            &[i64t.const_int(mb_size, false).into()],
            "mb_raw"
        )).try_as_basic_value().basic().unwrap();
        // Zero-init
        b!(self.bld.build_call(
            memset_fn,
            &[mb_ptr.into(), i32t.const_int(0, false).into(), i64t.const_int(mb_size, false).into()],
            ""
        ));

        let mb_ptr_v = mb_ptr.into_pointer_value();

        // Create channel: jade_chan_create(msg_size, MAILBOX_CAP)
        let chan_create = self.module.get_function("jade_chan_create").unwrap();
        let ch = b!(self.bld.build_call(
            chan_create,
            &[
                i64t.const_int(msg_size, false).into(),
                i64t.const_int(MAILBOX_CAP, false).into(),
            ],
            "actor_ch"
        )).try_as_basic_value().basic().unwrap();

        // Store channel pointer in mailbox (field 0)
        let ch_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 0, "ch_ptr_ptr"));
        b!(self.bld.build_store(ch_ptr_ptr, ch));

        // Set alive = 1 (field 1)
        let alive_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 1, "alive_ptr"));
        b!(self.bld.build_store(alive_ptr, i32t.const_int(1, false)));

        // Create coroutine: jade_coro_create(loop_fn, mb_ptr)
        let loop_fn = self.module.get_function(&loop_name)
            .ok_or_else(|| format!("actor loop fn '{loop_name}' not found"))?;
        let coro_create = self.module.get_function("jade_coro_create").unwrap();
        let coro = b!(self.bld.build_call(
            coro_create,
            &[
                loop_fn.as_global_value().as_pointer_value().into(),
                mb_ptr_v.into(),
            ],
            "actor_coro"
        )).try_as_basic_value().basic().unwrap();

        // Mark as daemon (actor coroutine doesn't block jade_sched_run)
        let set_daemon = self.module.get_function("jade_coro_set_daemon").unwrap();
        b!(self.bld.build_call(set_daemon, &[coro.into()], ""));

        // Spawn on scheduler
        let sched_spawn = self.module.get_function("jade_sched_spawn").unwrap();
        b!(self.bld.build_call(sched_spawn, &[coro.into()], ""));

        // Return the mailbox pointer as ActorRef
        Ok(mb_ptr_v.into())
    }

    /// Compile a `send target, @handler(args)` expression.
    /// Writes the message to stack and sends it through the actor's channel.
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

        let mb_st = self.module.get_struct_type(&mb_name)
            .ok_or_else(|| format!("mailbox type '{mb_name}' not found"))?;
        let msg_st = self.module.get_struct_type(&msg_name)
            .ok_or_else(|| format!("message type '{msg_name}' not found"))?;

        // Compile target to get mailbox pointer
        let mb_ptr = self.compile_expr(target)?.into_pointer_value();

        // Load channel pointer from mailbox field 0
        let ch_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 0, "ch_ptr_ptr"));
        let ch_ptr = b!(self.bld.build_load(ptr_ty, ch_ptr_ptr, "ch_ptr"));

        // Allocate message on stack
        let msg_alloca = self.entry_alloca(msg_st.into(), "send_msg");

        // Write tag (field 0)
        let tag_ptr = b!(self.bld.build_struct_gep(msg_st, msg_alloca, 0, "tag_ptr"));
        b!(self.bld.build_store(tag_ptr, i32t.const_int(tag as u64, false)));

        // Write payload args (field 1)
        let payload_ptr = b!(self.bld.build_struct_gep(msg_st, msg_alloca, 1, "payload_ptr"));
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

        // Send message through channel
        let chan_send = self.module.get_function("jade_chan_send").unwrap();
        b!(self.bld.build_call(
            chan_send,
            &[ch_ptr.into(), msg_alloca.into()],
            ""
        ));

        Ok(i64t.const_int(0, false).into())
    }

    // ── Coroutine codegen ───────────────────────────────────────────────
    //
    // Layout: { mutex(40B), cond_producer(48B), cond_consumer(48B), value(i64), has_value(i8), done(i8) }
    // Total: allocated as raw bytes via malloc, accessed by byte offsets.
    //
    // Offsets (x86_64 Linux):
    //   0   = pthread_mutex_t  (40 bytes)
    //   40  = pthread_cond_t   (cond_producer, 48 bytes)
    //   88  = pthread_cond_t   (cond_consumer, 48 bytes)
    //   136 = value            (i64, 8 bytes)
    //   144 = has_value        (i8, 1 byte)
    //   145 = done             (i8, 1 byte)
    //   Total = 152 bytes (rounded to 160 for alignment)

    const CORO_MUTEX_OFF: u64 = 0;
    const CORO_COND_PROD_OFF: u64 = 40;
    const CORO_COND_CONS_OFF: u64 = 88;
    const CORO_VALUE_OFF: u64 = 136;
    const CORO_HAS_VALUE_OFF: u64 = 144;
    const CORO_DONE_OFF: u64 = 145;
    const CORO_SIZE: u64 = 160;

    fn coro_field_ptr(
        &self,
        coro_ptr: inkwell::values::PointerValue<'ctx>,
        offset: u64,
        name: &str,
    ) -> Result<inkwell::values::PointerValue<'ctx>, String> {
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        if offset == 0 {
            Ok(coro_ptr)
        } else {
            Ok(unsafe {
                b!(self.bld.build_gep(
                    i8t,
                    coro_ptr,
                    &[i64t.const_int(offset, false)],
                    name
                ))
            })
        }
    }

    pub(crate) fn compile_coroutine_create(
        &mut self,
        name: &str,
        body: &[hir::Stmt],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.declare_actor_runtime(); // ensures malloc/memset/free

        // Declare pthread functions needed by dispatch coroutines
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        macro_rules! ensure {
            ($name:expr, $ft:expr) => {
                if self.module.get_function($name).is_none() {
                    self.module.add_function($name, $ft, Some(Linkage::External));
                }
            };
        }
        ensure!("pthread_create", i32t.fn_type(&[ptr.into(), ptr.into(), ptr.into(), ptr.into()], false));
        ensure!("pthread_join", i32t.fn_type(&[ptr.into(), ptr.into()], false));
        ensure!("pthread_mutex_init", i32t.fn_type(&[ptr.into(), ptr.into()], false));
        ensure!("pthread_mutex_lock", i32t.fn_type(&[ptr.into()], false));
        ensure!("pthread_mutex_unlock", i32t.fn_type(&[ptr.into()], false));
        ensure!("pthread_cond_init", i32t.fn_type(&[ptr.into(), ptr.into()], false));
        ensure!("pthread_cond_wait", i32t.fn_type(&[ptr.into(), ptr.into()], false));
        ensure!("pthread_cond_signal", i32t.fn_type(&[ptr.into()], false));

        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();

        // 1. Build the coroutine body function: void* __coro_<name>(void* coro_ptr)
        let coro_fn_name = format!("__coro_{name}");
        let fn_ty = ptr.fn_type(&[ptr.into()], false);
        let coro_fn = self.module.add_function(&coro_fn_name, fn_ty, Some(Linkage::Internal));

        // Save current state
        let saved_fn = self.cur_fn;
        let saved_bb = self.bld.get_insert_block();
        let saved_vars = std::mem::replace(&mut self.vars, vec![std::collections::HashMap::new()]);
        let saved_loop_stack = std::mem::replace(&mut self.loop_stack, Vec::new());

        // Build coroutine body
        self.cur_fn = Some(coro_fn);
        let entry = self.ctx.append_basic_block(coro_fn, "entry");
        self.bld.position_at_end(entry);

        let coro_ptr_param = coro_fn.get_first_param().unwrap().into_pointer_value();
        let coro_ptr_alloca = self.entry_alloca(ptr.into(), "__coro_ctx");
        b!(self.bld.build_store(coro_ptr_alloca, coro_ptr_param));

        // Store the coro_ptr in a variable the yield code can access
        self.set_var("__coro_ctx", coro_ptr_alloca, Type::Ptr(Box::new(Type::I64)));

        // Compile the body - replace Yield statements with coroutine yield logic
        self.compile_coroutine_body(body)?;

        // After body finishes, set done=1 and signal consumer
        let coro_ptr_val = b!(self.bld.build_load(ptr, coro_ptr_alloca, "coro.ptr")).into_pointer_value();

        // Lock
        let mutex_ptr = self.coro_field_ptr(coro_ptr_val, Self::CORO_MUTEX_OFF, "coro.mutex")?;
        let lock_fn = self.module.get_function("pthread_mutex_lock").unwrap();
        b!(self.bld.build_call(lock_fn, &[mutex_ptr.into()], ""));

        // Set done=1
        let done_ptr = self.coro_field_ptr(coro_ptr_val, Self::CORO_DONE_OFF, "coro.done")?;
        b!(self.bld.build_store(done_ptr, i8t.const_int(1, false)));

        // Unlock
        let unlock_fn = self.module.get_function("pthread_mutex_unlock").unwrap();
        b!(self.bld.build_call(unlock_fn, &[mutex_ptr.into()], ""));

        // Signal consumer
        let cond_cons_ptr = self.coro_field_ptr(coro_ptr_val, Self::CORO_COND_CONS_OFF, "coro.cond_cons")?;
        let cond_signal_fn = self.module.get_function("pthread_cond_signal").unwrap();
        b!(self.bld.build_call(cond_signal_fn, &[cond_cons_ptr.into()], ""));

        // Return null
        let null = ptr.const_null();
        b!(self.bld.build_return(Some(&null)));

        // Restore state
        self.cur_fn = saved_fn;
        self.vars = saved_vars;
        self.loop_stack = saved_loop_stack;

        // Reposition builder to the calling function's block
        let fv = self.cur_fn.unwrap();
        let bb = saved_bb.unwrap_or_else(|| {
            self.ctx.append_basic_block(fv, "coro.after")
        });
        self.bld.position_at_end(bb);

        // malloc
        let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
            let ft = ptr.fn_type(&[i64t.into()], false);
            self.module.add_function("malloc", ft, Some(Linkage::External))
        });
        let coro_mem = b!(self.bld.build_call(malloc_fn, &[i64t.const_int(Self::CORO_SIZE, false).into()], "coro.mem"))
            .try_as_basic_value().basic().unwrap().into_pointer_value();

        // memset to zero
        let memset_fn = self.module.get_function("memset").unwrap_or_else(|| {
            let ft = ptr.fn_type(&[ptr.into(), i32t.into(), i64t.into()], false);
            self.module.add_function("memset", ft, Some(Linkage::External))
        });
        b!(self.bld.build_call(memset_fn, &[coro_mem.into(), i32t.const_int(0, false).into(), i64t.const_int(Self::CORO_SIZE, false).into()], ""));

        // pthread_mutex_init
        let mutex_ptr = self.coro_field_ptr(coro_mem, Self::CORO_MUTEX_OFF, "coro.mutex.init")?;
        let mutex_init_fn = self.module.get_function("pthread_mutex_init").unwrap();
        b!(self.bld.build_call(mutex_init_fn, &[mutex_ptr.into(), ptr.const_null().into()], ""));

        // pthread_cond_init for both conds
        let cond_init_fn = self.module.get_function("pthread_cond_init").unwrap();
        let cond_prod_ptr = self.coro_field_ptr(coro_mem, Self::CORO_COND_PROD_OFF, "coro.cond_prod.init")?;
        b!(self.bld.build_call(cond_init_fn, &[cond_prod_ptr.into(), ptr.const_null().into()], ""));
        let cond_cons_ptr = self.coro_field_ptr(coro_mem, Self::CORO_COND_CONS_OFF, "coro.cond_cons.init")?;
        b!(self.bld.build_call(cond_init_fn, &[cond_cons_ptr.into(), ptr.const_null().into()], ""));

        // pthread_create
        let thread_storage = self.entry_alloca(i64t.into(), "coro.tid");
        let create_fn = self.module.get_function("pthread_create").unwrap();
        b!(self.bld.build_call(create_fn, &[
            thread_storage.into(),
            ptr.const_null().into(),
            coro_fn.as_global_value().as_pointer_value().into(),
            coro_mem.into(),
        ], ""));

        // Register the dispatch name as a variable so bar.next() works
        if name != "__anon" {
            let name_alloca = self.entry_alloca(ptr.into(), name);
            b!(self.bld.build_store(name_alloca, coro_mem));
            self.set_var(name, name_alloca, Type::Coroutine(Box::new(Type::I64)));
        }

        Ok(coro_mem.into())
    }

    fn compile_coroutine_body(
        &mut self,
        body: &[hir::Stmt],
    ) -> Result<(), String> {
        for stmt in body {
            self.compile_coroutine_stmt(stmt)?;
        }
        Ok(())
    }

    fn compile_coroutine_stmt(
        &mut self,
        stmt: &hir::Stmt,
    ) -> Result<(), String> {
        // For-loops with yield inside: compile normally but intercept yields
        match stmt {
            hir::Stmt::For(f) => {
                self.compile_coroutine_for(f)?;
            }
            hir::Stmt::While(w) => {
                // Check if body contains yields - compile as coroutine while
                self.compile_coroutine_while(w)?;
            }
            hir::Stmt::Loop(l) => {
                self.compile_coroutine_loop(l)?;
            }
            hir::Stmt::Expr(e) => {
                // Check if the expression is a yield
                if let hir::ExprKind::Yield(inner) = &e.kind {
                    self.emit_coroutine_yield(inner)?;
                } else {
                    self.compile_expr(e)?;
                }
            }
            hir::Stmt::Ret(val, _, _) => {
                // return in coroutine = final yield + done
                if let Some(e) = val {
                    self.emit_coroutine_yield(e)?;
                }
            }
            hir::Stmt::Bind(bind) => {
                let val = self.compile_expr(&bind.value)?;
                let ty = &bind.ty;
                let a = self.entry_alloca(self.llvm_ty(ty), &bind.name);
                b!(self.bld.build_store(a, val));
                self.set_var(&bind.name, a, ty.clone());
            }
            hir::Stmt::Assign(target, value, _) => {
                self.compile_assign(target, value)?;
            }
            hir::Stmt::If(i) => {
                self.compile_if(i)?;
            }
            _ => {
                self.compile_stmt(stmt)?;
            }
        }
        Ok(())
    }

    fn emit_coroutine_yield(
        &mut self,
        val_expr: &hir::Expr,
    ) -> Result<(), String> {
        let val = self.compile_expr(val_expr)?;
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i8t = self.ctx.i8_type();

        // Load the coroutine context pointer
        let (coro_alloca, _) = self.find_var("__coro_ctx").cloned()
            .ok_or("internal: no __coro_ctx in coroutine body")?;
        let coro_ptr = b!(self.bld.build_load(ptr, coro_alloca, "coro.ctx")).into_pointer_value();

        // Lock mutex
        let mutex_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_MUTEX_OFF, "coro.y.mutex")?;
        let lock_fn = self.module.get_function("pthread_mutex_lock").unwrap();
        b!(self.bld.build_call(lock_fn, &[mutex_ptr.into()], ""));

        // Wait while has_value==1 (consumer hasn't consumed yet)
        let fv = self.cur_fn.unwrap();
        let wait_bb = self.ctx.append_basic_block(fv, "coro.yield.wait");
        let write_bb = self.ctx.append_basic_block(fv, "coro.yield.write");

        b!(self.bld.build_unconditional_branch(wait_bb));
        self.bld.position_at_end(wait_bb);

        let has_val_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_HAS_VALUE_OFF, "coro.y.hv")?;
        let has_val = b!(self.bld.build_load(i8t, has_val_ptr, "hv")).into_int_value();
        let is_full = b!(self.bld.build_int_compare(IntPredicate::EQ, has_val, i8t.const_int(1, false), "full"));
        let wait_body_bb = self.ctx.append_basic_block(fv, "coro.yield.waitbody");
        b!(self.bld.build_conditional_branch(is_full, wait_body_bb, write_bb));

        self.bld.position_at_end(wait_body_bb);
        let cond_prod_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_COND_PROD_OFF, "coro.y.cprod")?;
        let cond_wait_fn = self.module.get_function("pthread_cond_wait").unwrap();
        b!(self.bld.build_call(cond_wait_fn, &[cond_prod_ptr.into(), mutex_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(wait_bb));

        // Write value
        self.bld.position_at_end(write_bb);
        let value_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_VALUE_OFF, "coro.y.val")?;
        // Convert value to i64 for storage
        let i64_val = self.coerce_to_i64(val);
        b!(self.bld.build_store(value_ptr, i64_val));

        // Set has_value=1
        let has_val_ptr2 = self.coro_field_ptr(coro_ptr, Self::CORO_HAS_VALUE_OFF, "coro.y.hv2")?;
        b!(self.bld.build_store(has_val_ptr2, i8t.const_int(1, false)));

        // Unlock
        let unlock_fn = self.module.get_function("pthread_mutex_unlock").unwrap();
        b!(self.bld.build_call(unlock_fn, &[mutex_ptr.into()], ""));

        // Signal consumer
        let cond_cons_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_COND_CONS_OFF, "coro.y.ccons")?;
        let cond_signal_fn = self.module.get_function("pthread_cond_signal").unwrap();
        b!(self.bld.build_call(cond_signal_fn, &[cond_cons_ptr.into()], ""));

        Ok(())
    }

    fn coerce_to_i64(&self, val: BasicValueEnum<'ctx>) -> inkwell::values::IntValue<'ctx> {
        let i64t = self.ctx.i64_type();
        match val {
            BasicValueEnum::IntValue(iv) => {
                if iv.get_type().get_bit_width() == 64 {
                    iv
                } else if iv.get_type().get_bit_width() < 64 {
                    self.bld.build_int_z_extend(iv, i64t, "zext").unwrap()
                } else {
                    self.bld.build_int_truncate(iv, i64t, "trunc").unwrap()
                }
            }
            BasicValueEnum::FloatValue(fv) => {
                self.bld.build_float_to_signed_int(fv, i64t, "f2i").unwrap()
            }
            BasicValueEnum::PointerValue(pv) => {
                self.bld.build_ptr_to_int(pv, i64t, "p2i").unwrap()
            }
            _ => i64t.const_int(0, false),
        }
    }

    pub(crate) fn compile_coroutine_next(
        &mut self,
        coro_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let coro_ptr = self.compile_expr(coro_expr)?.into_pointer_value();
        let _ptr = self.ctx.ptr_type(AddressSpace::default());
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        let fv = self.cur_fn.unwrap();

        // Lock mutex
        let mutex_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_MUTEX_OFF, "coro.n.mutex")?;
        let lock_fn = self.module.get_function("pthread_mutex_lock").unwrap();
        b!(self.bld.build_call(lock_fn, &[mutex_ptr.into()], ""));

        // Wait while has_value==0 AND done==0
        let wait_bb = self.ctx.append_basic_block(fv, "coro.next.wait");
        let read_bb = self.ctx.append_basic_block(fv, "coro.next.read");

        b!(self.bld.build_unconditional_branch(wait_bb));
        self.bld.position_at_end(wait_bb);

        let has_val_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_HAS_VALUE_OFF, "coro.n.hv")?;
        let has_val = b!(self.bld.build_load(i8t, has_val_ptr, "hv")).into_int_value();
        let done_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_DONE_OFF, "coro.n.done")?;
        let done_val = b!(self.bld.build_load(i8t, done_ptr, "done")).into_int_value();

        let has_no_val = b!(self.bld.build_int_compare(IntPredicate::EQ, has_val, i8t.const_int(0, false), "novalue"));
        let not_done = b!(self.bld.build_int_compare(IntPredicate::EQ, done_val, i8t.const_int(0, false), "notdone"));
        let should_wait = b!(self.bld.build_and(has_no_val, not_done, "shouldwait"));

        let wait_body_bb = self.ctx.append_basic_block(fv, "coro.next.waitbody");
        b!(self.bld.build_conditional_branch(should_wait, wait_body_bb, read_bb));

        self.bld.position_at_end(wait_body_bb);
        let cond_cons_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_COND_CONS_OFF, "coro.n.ccons")?;
        let cond_wait_fn = self.module.get_function("pthread_cond_wait").unwrap();
        b!(self.bld.build_call(cond_wait_fn, &[cond_cons_ptr.into(), mutex_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(wait_bb));

        // Read value
        self.bld.position_at_end(read_bb);
        let value_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_VALUE_OFF, "coro.n.val")?;
        let result = b!(self.bld.build_load(i64t, value_ptr, "coro.result"));

        // Set has_value=0
        let has_val_ptr2 = self.coro_field_ptr(coro_ptr, Self::CORO_HAS_VALUE_OFF, "coro.n.hv2")?;
        b!(self.bld.build_store(has_val_ptr2, i8t.const_int(0, false)));

        // Unlock
        let unlock_fn = self.module.get_function("pthread_mutex_unlock").unwrap();
        b!(self.bld.build_call(unlock_fn, &[mutex_ptr.into()], ""));

        // Signal producer (it may be waiting to put more values)
        let cond_prod_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_COND_PROD_OFF, "coro.n.cprod")?;
        let cond_signal_fn = self.module.get_function("pthread_cond_signal").unwrap();
        b!(self.bld.build_call(cond_signal_fn, &[cond_prod_ptr.into()], ""));

        Ok(result)
    }

    fn compile_coroutine_for(&mut self, f: &hir::For) -> Result<(), String> {
        // Compile as normal for but intercept yield in body
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();

        let start_val = self.compile_expr(&f.iter)?;
        let lvar = self.entry_alloca(self.llvm_ty(&f.bind_ty), &f.bind);
        b!(self.bld.build_store(lvar, start_val));
        self.set_var(&f.bind, lvar, f.bind_ty.clone());

        let cond_bb = self.ctx.append_basic_block(fv, "coro.for.cond");
        let body_bb = self.ctx.append_basic_block(fv, "coro.for.body");
        let inc_bb = self.ctx.append_basic_block(fv, "coro.for.inc");
        let end_bb = self.ctx.append_basic_block(fv, "coro.for.end");

        self.loop_stack.push(super::LoopCtx {
            continue_bb: inc_bb,
            break_bb: end_bb,
        });

        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);

        if let Some(ref end) = f.end {
            let cur = b!(self.bld.build_load(self.llvm_ty(&f.bind_ty), lvar, "cur")).into_int_value();
            let end_val = self.compile_expr(end)?.into_int_value();
            let cmp = b!(self.bld.build_int_compare(IntPredicate::SLT, cur, end_val, "cmp"));
            b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));
        } else {
            b!(self.bld.build_unconditional_branch(body_bb));
        }

        self.bld.position_at_end(body_bb);
        for stmt in &f.body {
            self.compile_coroutine_stmt(stmt)?;
        }
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(inc_bb));
        }

        self.bld.position_at_end(inc_bb);
        let cur = b!(self.bld.build_load(self.llvm_ty(&f.bind_ty), lvar, "cur")).into_int_value();
        let step = if let Some(ref s) = f.step {
            self.compile_expr(s)?.into_int_value()
        } else {
            i64t.const_int(1, false)
        };
        let next = b!(self.bld.build_int_add(cur, step, "next"));
        b!(self.bld.build_store(lvar, next));
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(end_bb);
        self.loop_stack.pop();
        Ok(())
    }

    fn compile_coroutine_while(&mut self, w: &hir::While) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let cond_bb = self.ctx.append_basic_block(fv, "coro.while.cond");
        let body_bb = self.ctx.append_basic_block(fv, "coro.while.body");
        let end_bb = self.ctx.append_basic_block(fv, "coro.while.end");

        self.loop_stack.push(super::LoopCtx {
            continue_bb: cond_bb,
            break_bb: end_bb,
        });

        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let cond = self.compile_expr(&w.cond)?.into_int_value();
        b!(self.bld.build_conditional_branch(cond, body_bb, end_bb));

        self.bld.position_at_end(body_bb);
        for stmt in &w.body {
            self.compile_coroutine_stmt(stmt)?;
        }
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(cond_bb));
        }

        self.bld.position_at_end(end_bb);
        self.loop_stack.pop();
        Ok(())
    }

    fn compile_coroutine_loop(&mut self, l: &hir::Loop) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let body_bb = self.ctx.append_basic_block(fv, "coro.loop.body");
        let end_bb = self.ctx.append_basic_block(fv, "coro.loop.end");

        self.loop_stack.push(super::LoopCtx {
            continue_bb: body_bb,
            break_bb: end_bb,
        });

        b!(self.bld.build_unconditional_branch(body_bb));
        self.bld.position_at_end(body_bb);
        for stmt in &l.body {
            self.compile_coroutine_stmt(stmt)?;
        }
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(body_bb));
        }

        self.bld.position_at_end(end_bb);
        self.loop_stack.pop();
        Ok(())
    }

    // ── Channel codegen ─────────────────────────────────────────────────

    /// Compile `channel of T` or `channel of T, cap`
    pub(crate) fn compile_channel_create(
        &mut self,
        elem_ty: &Type,
        cap_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let elem_size = self.type_store_size(self.llvm_ty(elem_ty));
        let cap_val = self.compile_expr(cap_expr)?;
        // Coerce capacity to i64
        let cap_i64 = if cap_val.is_int_value() {
            let iv = cap_val.into_int_value();
            if iv.get_type().get_bit_width() == 64 {
                iv
            } else {
                b!(self.bld.build_int_z_extend(iv, i64t, "cap.zext"))
            }
        } else {
            i64t.const_int(64, false)
        };
        let chan_create = self.module.get_function("jade_chan_create").unwrap();
        let ch = b!(self.bld.build_call(
            chan_create,
            &[
                i64t.const_int(elem_size, false).into(),
                cap_i64.into(),
            ],
            "ch"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap();
        Ok(ch)
    }

    /// Compile `send ch, val` for channels
    pub(crate) fn compile_channel_send(
        &mut self,
        ch_expr: &hir::Expr,
        val_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_ptr = self.compile_expr(ch_expr)?.into_pointer_value();
        let val = self.compile_expr(val_expr)?;
        // Spill value to stack, pass pointer to jade_chan_send
        let val_ty = val.get_type();
        let val_alloca = self.entry_alloca(val_ty, "chan.send.val");
        b!(self.bld.build_store(val_alloca, val));
        let chan_send = self.module.get_function("jade_chan_send").unwrap();
        b!(self.bld.build_call(
            chan_send,
            &[ch_ptr.into(), val_alloca.into()],
            ""
        ));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    /// Compile `receive ch`
    pub(crate) fn compile_channel_recv(
        &mut self,
        ch_expr: &hir::Expr,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_ptr = self.compile_expr(ch_expr)?.into_pointer_value();
        let elem_llvm_ty = self.llvm_ty(result_ty);
        let recv_alloca = self.entry_alloca(elem_llvm_ty, "chan.recv.val");
        let chan_recv = self.module.get_function("jade_chan_recv").unwrap();
        b!(self.bld.build_call(
            chan_recv,
            &[ch_ptr.into(), recv_alloca.into()],
            "chan.recv.ok"
        ));
        let result = b!(self.bld.build_load(elem_llvm_ty, recv_alloca, "chan.recv.result"));
        Ok(result)
    }

    /// Compile `close ch`
    pub(crate) fn compile_channel_close(
        &mut self,
        ch_expr: &hir::Expr,
    ) -> Result<(), String> {
        let ch_ptr = self.compile_expr(ch_expr)?.into_pointer_value();
        let chan_close = self.module.get_function("jade_chan_close").unwrap();
        b!(self.bld.build_call(chan_close, &[ch_ptr.into()], ""));
        Ok(())
    }

    /// Compile `stop actor_ref` — close the actor's channel, causing it to exit.
    pub(crate) fn compile_stop(
        &mut self,
        actor_expr: &hir::Expr,
    ) -> Result<(), String> {
        let actor_ptr = self.compile_expr(actor_expr)?.into_pointer_value();
        // Load channel pointer from mailbox field 0
        // We don't know the exact mailbox type here, so use a raw ptr load
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let ch_ptr = b!(self.bld.build_load(ptr_ty, actor_ptr, "stop_ch_ptr"));
        let chan_close = self.module.get_function("jade_chan_close").unwrap();
        b!(self.bld.build_call(chan_close, &[ch_ptr.into()], ""));
        Ok(())
    }

    // ── Select codegen ──────────────────────────────────────────────────

    /// Compile a select expression.
    ///
    /// Runtime struct layout: jade_select_case_t { ptr chan, ptr data, i32 is_send }
    pub(crate) fn compile_select(
        &mut self,
        arms: &[hir::SelectArm],
        default_body: Option<&hir::Block>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let fv = self.cur_fn.unwrap();
        let n = arms.len();

        // Build the select_case_t struct type: { ptr, ptr, i32 }
        let case_struct_ty = self.ctx.struct_type(
            &[ptr_ty.into(), ptr_ty.into(), i32t.into()],
            false,
        );

        // Allocate array of cases on the stack
        let cases_array_ty = case_struct_ty.array_type(n as u32);
        let cases_alloca = self.entry_alloca(cases_array_ty.into(), "select.cases");

        // For each arm, allocate a data buffer and fill in the case struct
        let mut data_allocas: Vec<inkwell::values::PointerValue<'ctx>> = Vec::new();
        for (i, arm) in arms.iter().enumerate() {
            let ch_val = self.compile_expr(&arm.chan)?.into_pointer_value();

            // Allocate data buffer for this arm
            let elem_llvm_ty = self.llvm_ty(&arm.elem_ty);
            let data_alloca = self.entry_alloca(elem_llvm_ty, &format!("select.data.{i}"));
            data_allocas.push(data_alloca);

            // If send arm, store the value into the data buffer
            if arm.is_send {
                if let Some(ref val_expr) = arm.value {
                    let val = self.compile_expr(val_expr)?;
                    b!(self.bld.build_store(data_alloca, val));
                }
            }

            // GEP into cases_alloca[i]
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

            // Store chan pointer (field 0)
            let chan_field = b!(self.bld.build_struct_gep(case_struct_ty, case_ptr, 0, "case.chan"));
            b!(self.bld.build_store(chan_field, ch_val));

            // Store data pointer (field 1)
            let data_field = b!(self.bld.build_struct_gep(case_struct_ty, case_ptr, 1, "case.data"));
            b!(self.bld.build_store(data_field, data_alloca));

            // Store is_send flag (field 2)
            let is_send_field = b!(self.bld.build_struct_gep(case_struct_ty, case_ptr, 2, "case.is_send"));
            b!(self.bld.build_store(is_send_field, i32t.const_int(if arm.is_send { 1 } else { 0 }, false)));
        }

        // Call jade_select(cases, n, has_default)
        let select_fn = self.module.get_function("jade_select").unwrap();
        let has_default_val = self.ctx.bool_type().const_int(
            if default_body.is_some() { 1 } else { 0 },
            false,
        );
        let result = b!(self.bld.build_call(
            select_fn,
            &[
                cases_alloca.into(),
                i32t.const_int(n as u64, false).into(),
                has_default_val.into(),
            ],
            "select.result"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();

        // Build switch on result
        let default_bb = self.ctx.append_basic_block(fv, "select.default");
        let merge_bb = self.ctx.append_basic_block(fv, "select.merge");
        let arm_bbs: Vec<_> = (0..n)
            .map(|i| self.ctx.append_basic_block(fv, &format!("select.arm.{i}")))
            .collect();

        let cases_vec: Vec<_> = arm_bbs
            .iter()
            .enumerate()
            .map(|(i, bb)| (i32t.const_int(i as u64, false), *bb))
            .collect();
        b!(self.bld.build_switch(result, default_bb, &cases_vec));

        // Compile each arm body
        for (i, (arm, bb)) in arms.iter().zip(arm_bbs.iter()).enumerate() {
            self.bld.position_at_end(*bb);
            self.vars.push(std::collections::HashMap::new());

            // If recv arm with binding, load received value from data buffer
            if !arm.is_send {
                if let Some(ref bind_name) = arm.binding {
                    let elem_llvm_ty = self.llvm_ty(&arm.elem_ty);
                    let val = b!(self.bld.build_load(elem_llvm_ty, data_allocas[i], bind_name));
                    let alloca = self.entry_alloca(elem_llvm_ty, bind_name);
                    b!(self.bld.build_store(alloca, val));
                    self.set_var(bind_name, alloca, arm.elem_ty.clone());
                }
            }

            self.compile_block(&arm.body)?;

            self.vars.pop();
            if self.no_term() {
                b!(self.bld.build_unconditional_branch(merge_bb));
            }
        }

        // Default arm
        self.bld.position_at_end(default_bb);
        if let Some(body) = default_body {
            self.compile_block(body)?;
        }
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(merge_bb));
        }

        self.bld.position_at_end(merge_bb);
        Ok(i64t.const_int(0, false).into())
    }
}
