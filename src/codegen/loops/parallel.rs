//! Sim-for and sim-block HIR codegen.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_sim_for(
        &mut self,
        f: &hir::For,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        // sim for: spawn each iteration as a coroutine, wait for all to finish.
        //
        // Layout of per-iteration arg struct passed to each coroutine:
        //   offset 0: iter_val  (i64)   — the loop variable value
        //   offset 8: counter   (*i64)  — pointer to shared atomic counter
        // Total: 16 bytes
        //
        // Counter is decremented (atomically) by each coroutine on completion.
        // Main code spins/yields until counter reaches 0.

        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let _i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let void = self.ctx.void_type();
        let fv = self.current_fn();

        // Compute range bounds
        let start_val = if f.end.is_some() {
            self.compile_expr(&f.iter)?.into_int_value()
        } else {
            i64t.const_int(0, false)
        };
        let end_val = if let Some(end) = &f.end {
            self.compile_expr(end)?.into_int_value()
        } else {
            self.compile_expr(&f.iter)?.into_int_value()
        };
        let step_val = if let Some(step) = &f.step {
            self.compile_expr(step)?.into_int_value()
        } else {
            i64t.const_int(1, false)
        };

        // Build iteration body function: void __sim_iter_N(void *arg)
        static SIM_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let sim_id = SIM_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let iter_fn_name = format!("__sim_iter_{sim_id}");
        let iter_fn_ty = void.fn_type(&[ptr.into()], false);
        let iter_fn = self
            .module
            .add_function(&iter_fn_name, iter_fn_ty, Some(Linkage::Internal));

        let saved_fn = self.cur_fn;
        let saved_bb = self.bld.get_insert_block();
        let saved_vars = std::mem::replace(&mut self.vars, IndexMap::new());
        let saved_shadows = std::mem::replace(&mut self.var_shadows, Vec::new());
        let saved_markers = std::mem::replace(&mut self.var_scope_markers, Vec::new());
        let saved_loop_stack = std::mem::replace(&mut self.loop_stack, Vec::new());

        self.cur_fn = Some(iter_fn);
        let entry = self.ctx.append_basic_block(iter_fn, "entry");
        self.bld.position_at_end(entry);

        let arg_ptr = iter_fn
            .get_first_param()
            .expect("ICE: function has no first param")
            .into_pointer_value();

        // Load iter_val from arg[0]
        let iter_val_ptr = arg_ptr; // offset 0
        let iter_val = b!(self.bld.build_load(i64t, iter_val_ptr, "iter_val"));

        // Load counter ptr from arg[8]
        let counter_ptr_ptr = unsafe {
            b!(self.bld.build_gep(
                self.ctx.i8_type(),
                arg_ptr,
                &[i64t.const_int(8, false)],
                "counter_pp"
            ))
        };
        let counter_ptr =
            b!(self.bld.build_load(ptr, counter_ptr_ptr, "counter_ptr")).into_pointer_value();

        // Set up the loop variable
        let lvar = self.entry_alloca(i64t.into(), &f.bind.as_str());
        b!(self.bld.build_store(lvar, iter_val));
        self.set_var(&f.bind.as_str(), lvar, Type::I64);

        // Compile the loop body
        self.compile_block(&f.body)?;

        // Atomically decrement counter
        if self.no_term() {
            b!(self.bld.build_atomicrmw(
                inkwell::AtomicRMWBinOp::Sub,
                counter_ptr,
                i64t.const_int(1, false),
                inkwell::AtomicOrdering::AcquireRelease,
            ));
            // Free the arg struct
            let free_fn = crate::codegen::fn_or_die(&self.module, "free");
            b!(self.bld.build_call(free_fn, &[arg_ptr.into()], ""));
            b!(self.bld.build_return(None));
        }

        // Restore caller context
        self.cur_fn = saved_fn;
        self.vars = saved_vars;
        self.var_shadows = saved_shadows;
        self.var_scope_markers = saved_markers;
        self.loop_stack = saved_loop_stack;

        let bb = saved_bb.unwrap_or_else(|| self.ctx.append_basic_block(fv, "sim.after"));
        self.bld.position_at_end(bb);

        // Allocate atomic counter
        let counter_alloca = self.entry_alloca(i64t.into(), "sim.counter");
        b!(self
            .bld
            .build_store(counter_alloca, i64t.const_int(0, false)));

        let malloc_fn = self.ensure_malloc();
        let coro_create = crate::codegen::fn_or_die(&self.module, "jinn_coro_create");
        let sched_spawn = crate::codegen::fn_or_die(&self.module, "jinn_sched_spawn");

        // Spawn loop: for i in start..end step step
        let spawn_var = self.entry_alloca(i64t.into(), "sim.i");
        b!(self.bld.build_store(spawn_var, start_val));

        let spawn_cond = self.ctx.append_basic_block(fv, "sim.spawn.cond");
        let spawn_body = self.ctx.append_basic_block(fv, "sim.spawn.body");
        let spawn_inc = self.ctx.append_basic_block(fv, "sim.spawn.inc");
        let spawn_done = self.ctx.append_basic_block(fv, "sim.spawn.done");

        b!(self.bld.build_unconditional_branch(spawn_cond));
        self.bld.position_at_end(spawn_cond);
        let cur_i = b!(self.bld.build_load(i64t, spawn_var, "si")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, cur_i, end_val, "sim.cmp"));
        b!(self
            .bld
            .build_conditional_branch(cmp, spawn_body, spawn_done));

        self.bld.position_at_end(spawn_body);

        // Increment counter atomically
        b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Add,
            counter_alloca,
            i64t.const_int(1, false),
            inkwell::AtomicOrdering::AcquireRelease,
        ));

        // Allocate arg struct (16 bytes: i64 iter_val, ptr counter)
        let arg_mem =
            b!(self
                .bld
                .build_call(malloc_fn, &[i64t.const_int(16, false).into()], "sim.arg"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void")
            .into_pointer_value();

        // Store iter_val at offset 0
        b!(self.bld.build_store(arg_mem, cur_i));
        // Store counter_ptr at offset 8
        let counter_field = unsafe {
            b!(self.bld.build_gep(
                self.ctx.i8_type(),
                arg_mem,
                &[i64t.const_int(8, false)],
                "sim.arg.cp"
            ))
        };
        b!(self.bld.build_store(counter_field, counter_alloca));

        // Create and spawn coroutine
        let coro = b!(self.bld.build_call(
            coro_create,
            &[
                iter_fn.as_global_value().as_pointer_value().into(),
                arg_mem.into(),
            ],
            "sim.coro"
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void");
        b!(self.bld.build_call(sched_spawn, &[coro.into()], ""));

        b!(self.bld.build_unconditional_branch(spawn_inc));
        self.bld.position_at_end(spawn_inc);
        let cur_i = b!(self.bld.build_load(i64t, spawn_var, "si")).into_int_value();
        let next_i = b!(self.bld.build_int_nsw_add(cur_i, step_val, "sim.next"));
        b!(self.bld.build_store(spawn_var, next_i));
        b!(self.bld.build_unconditional_branch(spawn_cond));

        // Wait for all iterations to complete
        self.bld.position_at_end(spawn_done);
        let wait_cond = self.ctx.append_basic_block(fv, "sim.wait");
        let wait_done = self.ctx.append_basic_block(fv, "sim.done");
        b!(self.bld.build_unconditional_branch(wait_cond));

        self.bld.position_at_end(wait_cond);
        let remaining = b!(self.bld.build_load(i64t, counter_alloca, "sim.rem")).into_int_value();
        let all_done = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            remaining,
            i64t.const_int(0, false),
            "sim.alldone"
        ));
        let wait_yield = self.ctx.append_basic_block(fv, "sim.wait.yield");
        b!(self
            .bld
            .build_conditional_branch(all_done, wait_done, wait_yield));

        self.bld.position_at_end(wait_yield);
        let sched_yield = crate::codegen::fn_or_die(&self.module, "jinn_sched_yield");
        b!(self.bld.build_call(sched_yield, &[], ""));
        b!(self.bld.build_unconditional_branch(wait_cond));

        self.bld.position_at_end(wait_done);
        Ok(None)
    }

    pub(crate) fn compile_sim_block(
        &mut self,
        stmts: &[hir::Stmt],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        // sim block: spawn each statement as a coroutine, wait for all to finish.
        //
        // Each statement gets its own void(void*) wrapper function.
        // A shared atomic counter tracks how many are still running.

        if stmts.is_empty() {
            return Ok(None);
        }

        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let void = self.ctx.void_type();
        let fv = self.current_fn();

        static SIM_BLK_COUNTER: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let blk_id = SIM_BLK_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Build one wrapper function per statement
        let mut stmt_fns = Vec::new();
        for (i, stmt) in stmts.iter().enumerate() {
            let fn_name = format!("__sim_blk_{blk_id}_s{i}");
            let fn_ty = void.fn_type(&[ptr.into()], false);
            let wrapper = self
                .module
                .add_function(&fn_name, fn_ty, Some(Linkage::Internal));

            let saved_fn = self.cur_fn;
            let saved_bb = self.bld.get_insert_block();
            let saved_vars = std::mem::replace(&mut self.vars, IndexMap::new());
            let saved_shadows = std::mem::replace(&mut self.var_shadows, Vec::new());
            let saved_markers = std::mem::replace(&mut self.var_scope_markers, Vec::new());
            let saved_loop_stack = std::mem::replace(&mut self.loop_stack, Vec::new());

            self.cur_fn = Some(wrapper);
            let entry = self.ctx.append_basic_block(wrapper, "entry");
            self.bld.position_at_end(entry);

            let arg_ptr = wrapper
                .get_first_param()
                .expect("ICE: function has no first param")
                .into_pointer_value();

            // arg_ptr points to a single ptr: the counter
            let counter_ptr =
                b!(self.bld.build_load(ptr, arg_ptr, "counter_ptr")).into_pointer_value();

            // Compile the statement
            self.compile_stmt(stmt)?;

            // Atomically decrement counter
            if self.no_term() {
                b!(self.bld.build_atomicrmw(
                    inkwell::AtomicRMWBinOp::Sub,
                    counter_ptr,
                    i64t.const_int(1, false),
                    inkwell::AtomicOrdering::AcquireRelease,
                ));
                let free_fn = crate::codegen::fn_or_die(&self.module, "free");
                b!(self.bld.build_call(free_fn, &[arg_ptr.into()], ""));
                b!(self.bld.build_return(None));
            }

            self.cur_fn = saved_fn;
            self.vars = saved_vars;
            self.var_shadows = saved_shadows;
            self.var_scope_markers = saved_markers;
            self.loop_stack = saved_loop_stack;
            if let Some(bb) = saved_bb {
                self.bld.position_at_end(bb);
            }

            stmt_fns.push(wrapper);
        }

        // Back in the caller: allocate atomic counter, spawn all, wait
        let counter_alloca = self.entry_alloca(i64t.into(), "simb.counter");
        let n = stmts.len() as u64;
        b!(self
            .bld
            .build_store(counter_alloca, i64t.const_int(n, false)));

        let malloc_fn = self.ensure_malloc();
        let coro_create = crate::codegen::fn_or_die(&self.module, "jinn_coro_create");
        let sched_spawn = crate::codegen::fn_or_die(&self.module, "jinn_sched_spawn");

        for wrapper in &stmt_fns {
            // Allocate arg struct (8 bytes: just a pointer to counter)
            let arg_mem =
                b!(self
                    .bld
                    .build_call(malloc_fn, &[i64t.const_int(8, false).into()], "simb.arg"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_pointer_value();

            // Store counter_ptr
            b!(self.bld.build_store(arg_mem, counter_alloca));

            // Create and spawn
            let coro = b!(self.bld.build_call(
                coro_create,
                &[
                    wrapper.as_global_value().as_pointer_value().into(),
                    arg_mem.into(),
                ],
                "simb.coro"
            ))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void");
            b!(self.bld.build_call(sched_spawn, &[coro.into()], ""));
        }

        // Wait for all statements to complete
        let wait_cond = self.ctx.append_basic_block(fv, "simb.wait");
        let wait_done = self.ctx.append_basic_block(fv, "simb.done");
        b!(self.bld.build_unconditional_branch(wait_cond));

        self.bld.position_at_end(wait_cond);
        let remaining = b!(self.bld.build_load(i64t, counter_alloca, "simb.rem")).into_int_value();
        let all_done = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            remaining,
            i64t.const_int(0, false),
            "simb.alldone"
        ));
        let wait_yield = self.ctx.append_basic_block(fv, "simb.wait.yield");
        b!(self
            .bld
            .build_conditional_branch(all_done, wait_done, wait_yield));

        self.bld.position_at_end(wait_yield);
        let sched_yield = crate::codegen::fn_or_die(&self.module, "jinn_sched_yield");
        b!(self.bld.build_call(sched_yield, &[], ""));
        b!(self.bld.build_unconditional_branch(wait_cond));

        self.bld.position_at_end(wait_done);
        Ok(None)
    }
}
