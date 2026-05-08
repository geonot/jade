//! Supervisor codegen helpers.

use super::*;

impl<'ctx> Compiler<'ctx> {
    /// Compile a supervisor definition.
    ///
    /// Emits two LLVM functions per supervisor:
    ///   `<name>_start()`         — first call creates the runtime supervisor,
    ///                              registers each child (factory + loop), and
    ///                              starts it. Subsequent calls are no-ops.
    ///   `<name>_restart_count()` — returns the runtime restart counter.
    ///
    /// A module-private global `<name>_g` (jade_sup_t*) holds the supervisor
    /// handle. The strategy enum (one_for_one / one_for_all / rest_for_one)
    /// is passed through to `jade_sup_create`. Each child's loop function
    /// (`<actor>_loop`) is paired with a generated factory
    /// (`<actor>_create_mb`) so the runtime can re-allocate its mailbox on
    /// restart without compile-time knowledge of the layout.
    pub(crate) fn compile_supervisor(&mut self, sup: &hir::SupervisorDef) -> Result<(), String> {
        use inkwell::AddressSpace;
        use inkwell::module::Linkage;

        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let void = self.ctx.void_type();

        // Declare runtime functions (idempotent).
        let sup_create = self
            .module
            .get_function("jade_sup_create")
            .unwrap_or_else(|| {
                let ft = ptr.fn_type(&[i32t.into()], false);
                self.module
                    .add_function("jade_sup_create", ft, Some(Linkage::External))
            });
        let sup_register = self
            .module
            .get_function("jade_sup_register")
            .unwrap_or_else(|| {
                let ft = i64t.fn_type(&[ptr.into(), ptr.into(), ptr.into(), ptr.into()], false);
                self.module
                    .add_function("jade_sup_register", ft, Some(Linkage::External))
            });
        let sup_start_fn = self
            .module
            .get_function("jade_sup_start")
            .unwrap_or_else(|| {
                let ft = void.fn_type(&[ptr.into()], false);
                self.module
                    .add_function("jade_sup_start", ft, Some(Linkage::External))
            });
        let sup_rcount = self
            .module
            .get_function("jade_sup_restart_count")
            .unwrap_or_else(|| {
                let ft = i32t.fn_type(&[ptr.into()], false);
                self.module
                    .add_function("jade_sup_restart_count", ft, Some(Linkage::External))
            });

        // Module-private global holding the supervisor handle.
        let g_name = format!("{}_g", sup.name);
        let g = self.module.add_global(ptr, None, &g_name);
        g.set_initializer(&ptr.const_null());
        g.set_linkage(Linkage::Internal);
        let g_ptr = g.as_pointer_value();

        // Strategy code.
        let strat_code: u64 = match sup.strategy {
            hir::SupervisorStrategy::OneForOne => 0,
            hir::SupervisorStrategy::OneForAll => 1,
            hir::SupervisorStrategy::RestForOne => 2,
        };

        // Ensure factory + loop exist for each child up-front so we can take
        // their function pointers.
        let mut child_info: Vec<(
            inkwell::values::FunctionValue<'ctx>,
            inkwell::values::FunctionValue<'ctx>,
            String,
        )> = Vec::new();
        for child in &sup.children {
            let factory_fv = self.ensure_actor_factory(&child.as_str())?;
            let loop_name = format!("{}_loop", child.as_str());
            let loop_fv = self.module.get_function(&loop_name).ok_or_else(|| {
                format!(
                    "supervisor '{}': child loop '{loop_name}' not found",
                    sup.name
                )
            })?;
            child_info.push((factory_fv, loop_fv, child.as_str().to_string()));
        }

        // ── <sup>_start() -> i64 ──
        let start_name = format!("{}_start", sup.name);
        let start_ft = i64t.fn_type(&[], false);
        let start_fv = self.module.add_function(&start_name, start_ft, None);
        let entry = self.ctx.append_basic_block(start_fv, "entry");
        let init_bb = self.ctx.append_basic_block(start_fv, "init");
        let ret_bb = self.ctx.append_basic_block(start_fv, "ret");

        let old_fn = self.cur_fn;
        let old_bb = self.bld.get_insert_block();
        self.cur_fn = Some(start_fv);

        self.bld.position_at_end(entry);
        let cur = b!(self.bld.build_load(ptr, g_ptr, "sup_cur")).into_pointer_value();
        let is_null = b!(self.bld.build_is_null(cur, "is_null"));
        b!(self.bld.build_conditional_branch(is_null, init_bb, ret_bb));

        self.bld.position_at_end(init_bb);
        let new_sup = b!(self.bld.build_call(
            sup_create,
            &[i32t.const_int(strat_code, false).into()],
            "sup_new",
        ))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void")
        .into_pointer_value();
        b!(self.bld.build_store(g_ptr, new_sup));

        for (factory_fv, loop_fv, name) in &child_info {
            let name_global = self
                .bld
                .build_global_string_ptr(name, &format!("__sup_child_name_{name}"))
                .map_err(|e| e.to_string())?
                .as_pointer_value();
            b!(self.bld.build_call(
                sup_register,
                &[
                    new_sup.into(),
                    factory_fv.as_global_value().as_pointer_value().into(),
                    loop_fv.as_global_value().as_pointer_value().into(),
                    name_global.into(),
                ],
                "",
            ));
        }

        b!(self.bld.build_call(sup_start_fn, &[new_sup.into()], ""));
        b!(self.bld.build_unconditional_branch(ret_bb));

        self.bld.position_at_end(ret_bb);
        b!(self.bld.build_return(Some(&i64t.const_int(0, false))));

        // ── <sup>_restart_count() -> i64 ──
        let rc_name = format!("{}_restart_count", sup.name);
        let rc_ft = i64t.fn_type(&[], false);
        let rc_fv = self.module.add_function(&rc_name, rc_ft, None);
        let rc_entry = self.ctx.append_basic_block(rc_fv, "entry");
        self.cur_fn = Some(rc_fv);
        self.bld.position_at_end(rc_entry);
        let cur2 = b!(self.bld.build_load(ptr, g_ptr, "sup_cur")).into_pointer_value();
        let cur2_null = b!(self.bld.build_is_null(cur2, "is_null"));
        let zero_bb = self.ctx.append_basic_block(rc_fv, "zero");
        let load_bb = self.ctx.append_basic_block(rc_fv, "load");
        b!(self
            .bld
            .build_conditional_branch(cur2_null, zero_bb, load_bb));
        self.bld.position_at_end(zero_bb);
        b!(self.bld.build_return(Some(&i64t.const_int(0, false))));
        self.bld.position_at_end(load_bb);
        let r = b!(self.bld.build_call(sup_rcount, &[cur2.into()], "rc"))
            .try_as_basic_value()
            .basic()
            .expect("ICE: call returned void")
            .into_int_value();
        let r64 = b!(self.bld.build_int_s_extend(r, i64t, "rc64"));
        b!(self.bld.build_return(Some(&r64)));

        self.cur_fn = old_fn;
        if let Some(bb) = old_bb {
            self.bld.position_at_end(bb);
        }

        self.fns
            .insert(start_name.clone().into(), (start_fv, vec![], Type::I64));
        self.fns
            .insert(rc_name.clone().into(), (rc_fv, vec![], Type::I64));
        Ok(())
    }
}
