//! Actor system codegen: mailbox structures, spawn, send, actor loop.
//!
//! Runtime model (per actor type):
//!   - State struct: { field0, field1, ... }
//!   - Message union: tag (i32) + max-sized payload
//!   - Mailbox struct: { mutex, cond_notempty, cond_notfull, buf_ptr, cap, head, tail, count, alive, state }
//!   - Actor loop fn: dequeue → switch on tag → dispatch to handler
//!   - Spawn: malloc mailbox + init mutex/conds + pthread_create
//!   - Send: lock → enqueue → signal → unlock

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
    /// Declare pthreads/mutex/condvar externs if not already present.
    pub(crate) fn declare_actor_runtime(&mut self) {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        // pthread_create(thread*, attr*, start_routine, arg) -> i32
        if self.module.get_function("pthread_create").is_none() {
            let ft = i32t.fn_type(&[ptr.into(), ptr.into(), ptr.into(), ptr.into()], false);
            self.module.add_function("pthread_create", ft, Some(Linkage::External));
        }
        // pthread_mutex_init(mutex*, attr*) -> i32
        if self.module.get_function("pthread_mutex_init").is_none() {
            let ft = i32t.fn_type(&[ptr.into(), ptr.into()], false);
            self.module.add_function("pthread_mutex_init", ft, Some(Linkage::External));
        }
        // pthread_mutex_lock(mutex*) -> i32
        if self.module.get_function("pthread_mutex_lock").is_none() {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module.add_function("pthread_mutex_lock", ft, Some(Linkage::External));
        }
        // pthread_mutex_unlock(mutex*) -> i32
        if self.module.get_function("pthread_mutex_unlock").is_none() {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module.add_function("pthread_mutex_unlock", ft, Some(Linkage::External));
        }
        // pthread_cond_init(cond*, attr*) -> i32
        if self.module.get_function("pthread_cond_init").is_none() {
            let ft = i32t.fn_type(&[ptr.into(), ptr.into()], false);
            self.module.add_function("pthread_cond_init", ft, Some(Linkage::External));
        }
        // pthread_cond_wait(cond*, mutex*) -> i32
        if self.module.get_function("pthread_cond_wait").is_none() {
            let ft = i32t.fn_type(&[ptr.into(), ptr.into()], false);
            self.module.add_function("pthread_cond_wait", ft, Some(Linkage::External));
        }
        // pthread_cond_signal(cond*) -> i32
        if self.module.get_function("pthread_cond_signal").is_none() {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module.add_function("pthread_cond_signal", ft, Some(Linkage::External));
        }
        // pthread_detach(thread) -> i32
        if self.module.get_function("pthread_detach").is_none() {
            let ft = i32t.fn_type(&[i64t.into()], false);
            self.module.add_function("pthread_detach", ft, Some(Linkage::External));
        }
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
    /// - `{ActorName}_mailbox`: { [40 x i8] mutex, [48 x i8] cond_ne, [48 x i8] cond_nf,
    ///      ptr buf, i64 cap, i64 head, i64 tail, i64 count, i32 alive, state_struct }
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
        //   [40 x i8]  mutex        (pthread_mutex_t — opaque, 40 bytes on Linux x86_64)
        //   [48 x i8]  cond_notempty
        //   [48 x i8]  cond_notfull
        //   ptr         buf          (pointer to msg array)
        //   i64         cap
        //   i64         head
        //   i64         tail
        //   i64         count
        //   i32         alive        (1 = running, 0 = stopped)
        //   state_struct state       (actor fields inline)
        let mb_name = format!("{name}_mailbox");
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let mutex_ty = self.ctx.i8_type().array_type(40);
        let cond_ty = self.ctx.i8_type().array_type(48);
        let mb_st = self.ctx.opaque_struct_type(&mb_name);
        mb_st.set_body(
            &[
                mutex_ty.into(),  // 0: mutex
                cond_ty.into(),   // 1: cond_notempty
                cond_ty.into(),   // 2: cond_notfull
                ptr_ty.into(),    // 3: buf
                i64t.into(),      // 4: cap
                i64t.into(),      // 5: head
                i64t.into(),      // 6: tail
                i64t.into(),      // 7: count
                i32t.into(),      // 8: alive
                state_st.into(),  // 9: state
            ],
            false,
        );

        Ok(())
    }

    /// Generate the actor loop function: `void {name}_loop(ptr mailbox_arg)`
    /// This runs on the spawned thread and loops: dequeue → dispatch → repeat.
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
        let i64t = self.ctx.i64_type();

        let mb_st = self.module.get_struct_type(&mb_name).unwrap();
        let msg_st = self.module.get_struct_type(&msg_name).unwrap();

        // Declare the loop function: ptr -> ptr (pthread start_routine signature)
        let ft = ptr_ty.fn_type(&[ptr_ty.into()], false);
        let fv = self.module.add_function(&loop_name, ft, Some(Linkage::Internal));

        let entry = self.ctx.append_basic_block(fv, "entry");
        let loop_bb = self.ctx.append_basic_block(fv, "loop");
        let exit_bb = self.ctx.append_basic_block(fv, "exit");

        // Save and set state
        let old_fn = self.cur_fn;
        self.cur_fn = Some(fv);

        let mb_ptr = fv.get_nth_param(0).unwrap().into_pointer_value();

        // Entry: compute all struct GEPs once (invariant), then jump to loop
        self.bld.position_at_end(entry);
        let mutex_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 0, "mutex_ptr"));
        let cond_ne_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 1, "cond_ne_ptr"));
        let cond_nf_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 2, "cond_nf_ptr"));
        let buf_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 3, "buf_ptr_ptr"));
        let head_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 5, "head_ptr"));
        let count_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 7, "count_ptr"));
        let alive_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 8, "alive_ptr"));
        let state_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 9, "state_ptr"));
        b!(self.bld.build_unconditional_branch(loop_bb));

        // Loop header: lock, wait if empty, dequeue
        self.bld.position_at_end(loop_bb);
        // Lock
        let lock_fn = self.module.get_function("pthread_mutex_lock").unwrap();
        b!(self.bld.build_call(lock_fn, &[mutex_ptr.into()], ""));

        // While count == 0 && alive, wait on cond_notempty
        let wait_bb = self.ctx.append_basic_block(fv, "wait");
        let dequeue_bb = self.ctx.append_basic_block(fv, "dequeue");
        b!(self.bld.build_unconditional_branch(wait_bb));

        self.bld.position_at_end(wait_bb);
        let count_val = b!(self.bld.build_load(i64t, count_ptr, "count"));
        let has_msg = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            count_val.into_int_value(),
            i64t.const_int(0, false),
            "has_msg"
        ));
        // If messages available, dequeue immediately
        let check_alive_bb = self.ctx.append_basic_block(fv, "check_alive");
        b!(self.bld.build_conditional_branch(has_msg, dequeue_bb, check_alive_bb));

        // Check alive — if dead and empty, exit; if dead but has msgs, drain first
        self.bld.position_at_end(check_alive_bb);
        let alive_val = b!(self.bld.build_load(i32t, alive_ptr, "alive"));
        let is_dead = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            alive_val.into_int_value(),
            i32t.const_int(0, false),
            "is_dead"
        ));
        let do_wait = self.ctx.append_basic_block(fv, "do_wait");
        let exit_unlock_bb = self.ctx.append_basic_block(fv, "exit_unlock");
        b!(self.bld.build_conditional_branch(is_dead, exit_unlock_bb, do_wait));

        // do_wait: call pthread_cond_wait, then re-check
        self.bld.position_at_end(do_wait);
        let cond_wait_fn = self.module.get_function("pthread_cond_wait").unwrap();
        b!(self.bld.build_call(cond_wait_fn, &[cond_ne_ptr.into(), mutex_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(wait_bb));

        // exit_unlock: unlock and exit
        self.bld.position_at_end(exit_unlock_bb);
        let unlock_fn = self.module.get_function("pthread_mutex_unlock").unwrap();
        b!(self.bld.build_call(unlock_fn, &[mutex_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(exit_bb));

        // Dequeue: read msg at head, advance head, decrement count
        self.bld.position_at_end(dequeue_bb);
        let buf_ptr = b!(self.bld.build_load(ptr_ty, buf_ptr_ptr, "buf_ptr"));
        let head_val = b!(self.bld.build_load(i64t, head_ptr, "head"));

        // msg_ptr = buf + head * sizeof(msg)
        let msg_size = self.type_store_size(msg_st.into());
        let msg_size_val = i64t.const_int(msg_size, false);
        let offset = b!(self.bld.build_int_mul(head_val.into_int_value(), msg_size_val, "offset"));

        let msg_ptr = unsafe {
            b!(self.bld.build_gep(
                self.ctx.i8_type(),
                buf_ptr.into_pointer_value(),
                &[offset.into()],
                "msg_ptr"
            ))
        };

        // Copy tag
        let tag_ptr = b!(self.bld.build_struct_gep(msg_st, msg_ptr, 0, "tag_ptr"));
        let tag_val = b!(self.bld.build_load(i32t, tag_ptr, "tag"));

        // Advance head = (head + 1) & (CAP - 1)  [power-of-2 ring buffer]
        let one = i64t.const_int(1, false);
        let cap_mask = i64t.const_int(MAILBOX_CAP - 1, false);
        let new_head = b!(self.bld.build_int_add(head_val.into_int_value(), one, "new_head_raw"));
        let new_head = b!(self.bld.build_and(new_head, cap_mask, "new_head"));
        b!(self.bld.build_store(head_ptr, new_head));

        // Decrement count
        let count_val2 = b!(self.bld.build_load(i64t, count_ptr, "count2"));
        let new_count = b!(self.bld.build_int_sub(count_val2.into_int_value(), one, "new_count"));
        b!(self.bld.build_store(count_ptr, new_count));

        // Unlock BEFORE signal — avoids "hurry up and wait" (woken thread
        // can grab the mutex immediately instead of blocking on it)
        let unlock_fn = self.module.get_function("pthread_mutex_unlock").unwrap();
        b!(self.bld.build_call(unlock_fn, &[mutex_ptr.into()], ""));

        // Signal notfull (after unlock — reduces contention)
        let cond_signal_fn = self.module.get_function("pthread_cond_signal").unwrap();
        b!(self.bld.build_call(cond_signal_fn, &[cond_nf_ptr.into()], ""));

        // Switch on tag → handler blocks
        let payload_ptr = b!(self.bld.build_struct_gep(msg_st, msg_ptr, 1, "payload_ptr"));

        if ad.handlers.is_empty() {
            // No handlers, just loop
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

            self.bld.position_at_end(dequeue_bb);
            // Reposition — we were at dequeue_bb, but we need to add switch at end
            // Actually we just continue from where we are
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

                // Save current vars and set up handler scope
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
                let mut param_offset: u64 = 0;
                for p in &h.params {
                    let pty = self.llvm_ty(&p.ty);
                    let psize = self.type_store_size(pty);
                    // GEP into payload at offset
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

                    // Store in a local alloca
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

        // Exit block
        self.bld.position_at_end(exit_bb);
        let null = ptr_ty.const_null();
        b!(self.bld.build_return(Some(&null)));

        self.cur_fn = old_fn;
        Ok(())
    }

    /// Compile a `spawn ActorName` expression → returns ActorRef (pointer to mailbox)
    pub(crate) fn compile_spawn(&mut self, actor_name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let mb_name = format!("{actor_name}_mailbox");
        let msg_name = format!("{actor_name}_msg");
        let loop_name = format!("{actor_name}_loop");

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
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

        // Init mutex
        let mutex_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 0, "mutex_ptr"));
        let mutex_init_fn = self.module.get_function("pthread_mutex_init").unwrap();
        b!(self.bld.build_call(
            mutex_init_fn,
            &[mutex_ptr.into(), ptr_ty.const_null().into()],
            ""
        ));

        // Init cond_notempty
        let cond_ne_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 1, "cond_ne_ptr"));
        let cond_init_fn = self.module.get_function("pthread_cond_init").unwrap();
        b!(self.bld.build_call(
            cond_init_fn,
            &[cond_ne_ptr.into(), ptr_ty.const_null().into()],
            ""
        ));

        // Init cond_notfull
        let cond_nf_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 2, "cond_nf_ptr"));
        b!(self.bld.build_call(
            cond_init_fn,
            &[cond_nf_ptr.into(), ptr_ty.const_null().into()],
            ""
        ));

        // Allocate message buffer: malloc(cap * sizeof(msg))
        let cap = MAILBOX_CAP;
        let buf_bytes = cap * msg_size;
        let buf_ptr = b!(self.bld.build_call(
            malloc_fn,
            &[i64t.const_int(buf_bytes, false).into()],
            "buf_raw"
        )).try_as_basic_value().basic().unwrap();

        // Store buf pointer
        let buf_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 3, "buf_ptr_ptr"));
        b!(self.bld.build_store(buf_ptr_ptr, buf_ptr));

        // Store cap
        let cap_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 4, "cap_ptr"));
        b!(self.bld.build_store(cap_ptr, i64t.const_int(cap, false)));

        // head, tail, count already 0 from memset

        // Set alive = 1
        let alive_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 8, "alive_ptr"));
        b!(self.bld.build_store(alive_ptr, i32t.const_int(1, false)));

        // Initialize state fields to defaults
        let state_name = format!("{actor_name}_state");
        let _state_st = self.module.get_struct_type(&state_name).unwrap();
        // State fields are already zero-init from memset — defaults would need
        // to be compiled into the init section in a production impl

        // Create thread
        let thread_alloca = self.entry_alloca(i64t.into(), "thread_id");
        let loop_fn = self.module.get_function(&loop_name)
            .ok_or_else(|| format!("actor loop fn '{loop_name}' not found"))?;
        let pthread_create_fn = self.module.get_function("pthread_create").unwrap();
        b!(self.bld.build_call(
            pthread_create_fn,
            &[
                thread_alloca.into(),
                ptr_ty.const_null().into(),
                loop_fn.as_global_value().as_pointer_value().into(),
                mb_ptr_v.into(),
            ],
            ""
        ));

        // Detach thread (fire and forget)
        let thread_id = b!(self.bld.build_load(i64t, thread_alloca, "tid"));
        let pthread_detach_fn = self.module.get_function("pthread_detach").unwrap();
        b!(self.bld.build_call(
            pthread_detach_fn,
            &[thread_id.into()],
            ""
        ));

        // Return the mailbox pointer as ActorRef
        Ok(mb_ptr_v.into())
    }

    /// Compile a `send target, @handler(args)` expression.
    /// Locks the mailbox, waits if full, writes the message, signals, unlocks.
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
            .ok_or_else(|| format!("actor mailbox type '{mb_name}' not found"))?;
        let msg_st = self.module.get_struct_type(&msg_name).unwrap();

        let mb_ptr = self.compile_expr(target)?.into_pointer_value();

        // Lock
        let mutex_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 0, "mutex_ptr"));
        let lock_fn = self.module.get_function("pthread_mutex_lock").unwrap();
        b!(self.bld.build_call(lock_fn, &[mutex_ptr.into()], ""));

        // Wait while full
        let fv = self.cur_fn.unwrap();
        let wait_bb = self.ctx.append_basic_block(fv, "send_wait");
        let do_wait_bb = self.ctx.append_basic_block(fv, "send_do_wait");
        let enqueue_bb = self.ctx.append_basic_block(fv, "send_enqueue");

        b!(self.bld.build_unconditional_branch(wait_bb));
        self.bld.position_at_end(wait_bb);

        let count_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 7, "count_ptr"));
        let count_val = b!(self.bld.build_load(i64t, count_ptr, "count"));
        let cap_const = i64t.const_int(MAILBOX_CAP, false);
        let is_full = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            count_val.into_int_value(),
            cap_const,
            "is_full"
        ));
        b!(self.bld.build_conditional_branch(is_full, do_wait_bb, enqueue_bb));

        // Do wait on cond_notfull
        self.bld.position_at_end(do_wait_bb);
        let cond_nf_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 2, "cond_nf_ptr"));
        let cond_wait_fn = self.module.get_function("pthread_cond_wait").unwrap();
        b!(self.bld.build_call(cond_wait_fn, &[cond_nf_ptr.into(), mutex_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(wait_bb));

        // Enqueue: write msg at tail
        self.bld.position_at_end(enqueue_bb);
        let buf_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 3, "buf_ptr_ptr"));
        let buf_ptr = b!(self.bld.build_load(ptr_ty, buf_ptr_ptr, "buf_ptr"));
        let tail_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 6, "tail_ptr"));
        let tail_val = b!(self.bld.build_load(i64t, tail_ptr, "tail"));

        let msg_size = self.type_store_size(msg_st.into());
        let msg_size_val = i64t.const_int(msg_size, false);
        let offset = b!(self.bld.build_int_mul(tail_val.into_int_value(), msg_size_val, "offset"));

        let msg_ptr = unsafe {
            b!(self.bld.build_gep(
                self.ctx.i8_type(),
                buf_ptr.into_pointer_value(),
                &[offset.into()],
                "msg_ptr"
            ))
        };

        // Write tag
        let tag_ptr = b!(self.bld.build_struct_gep(msg_st, msg_ptr, 0, "tag_ptr"));
        b!(self.bld.build_store(tag_ptr, i32t.const_int(tag as u64, false)));

        // Write payload (args packed sequentially)
        let payload_ptr = b!(self.bld.build_struct_gep(msg_st, msg_ptr, 1, "payload_ptr"));
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

        // Advance tail = (tail + 1) & (CAP - 1)  [power-of-2 ring buffer]
        let one = i64t.const_int(1, false);
        let cap_mask = i64t.const_int(MAILBOX_CAP - 1, false);
        let new_tail = b!(self.bld.build_int_add(tail_val.into_int_value(), one, "new_tail_raw"));
        let new_tail = b!(self.bld.build_and(new_tail, cap_mask, "new_tail"));
        b!(self.bld.build_store(tail_ptr, new_tail));

        // Increment count
        let count_val2 = b!(self.bld.build_load(i64t, count_ptr, "count2"));
        let new_count = b!(self.bld.build_int_add(count_val2.into_int_value(), one, "new_count"));
        b!(self.bld.build_store(count_ptr, new_count));

        // Unlock BEFORE signal — avoids "hurry up and wait"
        let unlock_fn = self.module.get_function("pthread_mutex_unlock").unwrap();
        b!(self.bld.build_call(unlock_fn, &[mutex_ptr.into()], ""));

        // Signal notempty (after unlock — reduces contention)
        let cond_ne_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 1, "cond_ne_ptr"));
        let cond_signal_fn = self.module.get_function("pthread_cond_signal").unwrap();
        b!(self.bld.build_call(cond_signal_fn, &[cond_ne_ptr.into()], ""));

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
        self.declare_actor_runtime(); // ensures pthread externs exist

        // Also declare pthread_join if needed
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        if self.module.get_function("pthread_join").is_none() {
            let ft = i32t.fn_type(&[ptr.into(), ptr.into()], false);
            self.module.add_function("pthread_join", ft, Some(Linkage::External));
        }

        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();

        // 1. Build the coroutine body function: void* __coro_<name>(void* coro_ptr)
        let coro_fn_name = format!("__coro_{name}");
        let fn_ty = ptr.fn_type(&[ptr.into()], false);
        let coro_fn = self.module.add_function(&coro_fn_name, fn_ty, Some(Linkage::Internal));

        // Save current state
        let saved_fn = self.cur_fn;
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

        // Now in the caller: malloc coro struct, init, pthread_create
        let fv = self.cur_fn.unwrap();
        let bb = self.bld.get_insert_block().unwrap_or_else(|| {
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

    /// Compile `stop actor_ref`
    pub(crate) fn compile_stop(
        &mut self,
        actor_expr: &hir::Expr,
    ) -> Result<(), String> {
        let actor_ptr = self.compile_expr(actor_expr)?.into_pointer_value();
        let actor_stop = self.module.get_function("jade_actor_stop")
            .unwrap_or_else(|| {
                // Fallback: set alive=0 directly (field 8 in mailbox struct)
                // But we should have tile runtime stop function
                panic!("jade_actor_stop not declared")
            });
        b!(self.bld.build_call(actor_stop, &[actor_ptr.into()], ""));
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
