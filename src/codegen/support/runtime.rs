use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn setup_target(&self) -> Result<(), String> {
        let tm = self.target_machine(OptimizationLevel::None)?;
        let triple = if let Some(ref t) = self.target_triple {
            inkwell::targets::TargetTriple::create(t)
        } else {
            TargetMachine::get_default_triple()
        };
        self.module.set_triple(&triple);
        self.module
            .set_data_layout(&tm.get_target_data().get_data_layout());
        Ok(())
    }

    pub(crate) fn target_machine(&self, opt: OptimizationLevel) -> Result<TargetMachine, String> {
        if let Some(ref triple_str) = self.target_triple {
            Target::initialize_all(&InitializationConfig::default());
            let triple = inkwell::targets::TargetTriple::create(triple_str);
            let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
            let cpu = self.target_cpu.as_deref().unwrap_or("generic");
            let features = self.target_features.as_deref().unwrap_or("");
            target
                .create_target_machine(
                    &triple,
                    cpu,
                    features,
                    opt,
                    RelocMode::PIC,
                    CodeModel::Default,
                )
                .ok_or_else(|| "failed to create target machine".into())
        } else {
            Target::initialize_native(&InitializationConfig::default())
                .map_err(|e| e.to_string())?;
            let triple = TargetMachine::get_default_triple();
            let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
            target
                .create_target_machine(
                    &triple,
                    TargetMachine::get_host_cpu_name().to_str().unwrap(),
                    TargetMachine::get_host_cpu_features().to_str().unwrap(),
                    opt,
                    RelocMode::PIC,
                    CodeModel::Default,
                )
                .ok_or_else(|| "failed to create target machine".into())
        }
    }

    pub(crate) fn attr(&self, name: &str) -> Attribute {
        self.ctx
            .create_enum_attribute(Attribute::get_named_enum_kind_id(name), 0)
    }

    pub(crate) fn zero_init(&self, ty: &Type) -> BasicValueEnum<'ctx> {
        match ty {
            Type::I8 | Type::U8 => self.ctx.i8_type().const_int(0, false).into(),
            Type::I16 | Type::U16 => self.ctx.i16_type().const_int(0, false).into(),
            Type::I32 | Type::U32 => self.ctx.i32_type().const_int(0, false).into(),
            Type::I64 | Type::U64 => self.ctx.i64_type().const_int(0, false).into(),
            Type::F32 => self.ctx.f32_type().const_float(0.0).into(),
            Type::F64 => self.ctx.f64_type().const_float(0.0).into(),
            Type::Bool => self.ctx.bool_type().const_int(0, false).into(),
            _ => self.ctx.i64_type().const_int(0, false).into(),
        }
    }

    pub(crate) fn compile_const_expr(
        &self,
        expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match &expr.kind {
            hir::ExprKind::Int(n) => Ok(self.int_const(*n, &expr.ty).into()),
            hir::ExprKind::Float(f) => Ok(self.ctx.f64_type().const_float(*f).into()),
            hir::ExprKind::Bool(b) => Ok(self.ctx.bool_type().const_int(*b as u64, false).into()),
            hir::ExprKind::Str(s) => {
                let gv = self
                    .bld
                    .build_global_string_ptr(s, "global_str")
                    .map_err(|e| e.to_string())?;
                Ok(gv.as_pointer_value().into())
            }
            _ => Err(format!(
                "global initializer must be a constant expression, got {:?}",
                expr.kind
            )),
        }
    }

    pub(crate) fn tag_fn(&self, fv: FunctionValue<'ctx>) {
        self.tag_fn_inner(fv, true);
    }

    pub(in crate::codegen) fn tag_fn_inner(&self, fv: FunctionValue<'ctx>, _will_return: bool) {
        fv.add_attribute(AttributeLoc::Function, self.attr("nounwind"));
    }

    pub(crate) fn tag_param_ownership(
        &self,
        fv: FunctionValue<'ctx>,
        loc: AttributeLoc,
        ownership: &hir::Ownership,
        ty: &Type,
    ) {
        if !ty.is_ptr_represented() {
            return;
        }

        fv.add_attribute(loc, self.attr("nonnull"));

        match ownership {
            hir::Ownership::Owned | hir::Ownership::BorrowMut | hir::Ownership::Borrowed => {
                fv.add_attribute(loc, self.attr("noalias"));
            }
            hir::Ownership::Raw => {}
        }
    }

    pub(crate) fn set_var(&mut self, name: &str, ptr: PointerValue<'ctx>, ty: Type) {
        let old = self.vars.insert(name.into(), (ptr, ty));
        self.var_shadows.push((name.to_string(), old));
    }

    pub(crate) fn find_var(&self, name: &str) -> Option<&(PointerValue<'ctx>, Type)> {
        self.vars.get(name)
    }

    pub(crate) fn entry_alloca(&self, ty: BasicTypeEnum<'ctx>, name: &str) -> PointerValue<'ctx> {
        let fv = self.current_fn();
        let entry = ice!(fv.get_first_basic_block(), "function has no entry block");
        match entry.get_first_instruction() {
            Some(inst) => self.alloca_bld.position_before(&inst),
            None => self.alloca_bld.position_at_end(entry),
        }
        self.alloca_bld.build_alloca(ty, name).unwrap()
    }

    pub(crate) fn no_term(&self) -> bool {
        self.bld
            .get_insert_block()
            .map_or(true, |bb| bb.get_terminator().is_none())
    }

    pub(crate) fn mk_fn_type(
        &self,
        ret: &Type,
        params: &[BasicMetadataTypeEnum<'ctx>],
        variadic: bool,
    ) -> inkwell::types::FunctionType<'ctx> {
        match ret {
            Type::Void => self.ctx.void_type().fn_type(params, variadic),
            ty => self.llvm_ty(ty).fn_type(params, variadic),
        }
    }

    pub(crate) fn call_result(
        &self,
        csv: inkwell::values::CallSiteValue<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        csv.try_as_basic_value()
            .basic()
            .unwrap_or_else(|| self.ctx.i64_type().const_int(0, false).into())
    }

    #[track_caller]
    pub(crate) fn current_fn(&self) -> FunctionValue<'ctx> {
        ice!(self.cur_fn, "no current function")
    }

    #[track_caller]
    pub(crate) fn current_bb(&self) -> inkwell::basic_block::BasicBlock<'ctx> {
        ice!(self.bld.get_insert_block(), "no current basic block")
    }

    pub(crate) fn ensure_malloc(&mut self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("jinn_xmalloc") {
            return f;
        }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let ft = ptr_ty.fn_type(&[i64t.into()], false);
        let func = self
            .module
            .add_function("jinn_xmalloc", ft, Some(Linkage::WeakAny));

        let entry = self.ctx.append_basic_block(func, "entry");
        let ok_bb = self.ctx.append_basic_block(func, "ok");
        let fail_bb = self.ctx.append_basic_block(func, "fail");

        let saved = self.bld.get_insert_block();
        self.bld.position_at_end(entry);
        let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
            let mft = ptr_ty.fn_type(&[i64t.into()], false);
            self.module
                .add_function("malloc", mft, Some(Linkage::External))
        });
        let size = ice!(func.get_first_param(), "xmalloc missing size param").into_int_value();
        let raw = ice!(
            self.bld
                .build_call(malloc_fn, &[size.into()], "raw")
                .unwrap()
                .try_as_basic_value()
                .basic(),
            "malloc returned void"
        )
        .into_pointer_value();
        let is_null = self.bld.build_is_null(raw, "is_null").unwrap();
        let size_nonzero = self
            .bld
            .build_int_compare(
                inkwell::IntPredicate::UGT,
                size,
                i64t.const_int(0, false),
                "nz",
            )
            .unwrap();
        let should_abort = self.bld.build_and(is_null, size_nonzero, "oom").unwrap();
        self.bld
            .build_conditional_branch(should_abort, fail_bb, ok_bb)
            .unwrap();

        self.bld.position_at_end(fail_bb);
        let abort_fn = self.module.get_function("abort").unwrap_or_else(|| {
            let void_ty = self.ctx.void_type();
            let aft = void_ty.fn_type(&[], false);
            self.module
                .add_function("abort", aft, Some(Linkage::External))
        });
        self.bld.build_call(abort_fn, &[], "").unwrap();
        self.bld.build_unreachable().unwrap();

        self.bld.position_at_end(ok_bb);
        self.bld.build_return(Some(&raw)).unwrap();

        if let Some(bb) = saved {
            self.bld.position_at_end(bb);
        }
        func
    }

    pub(crate) fn get_or_create_reuse_alloca(&mut self, slot: u32) -> PointerValue<'ctx> {
        if let Some(p) = self.current_reuse_alloca_slots.get(&slot) {
            return *p;
        }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let alloca = self.entry_alloca(ptr_ty.into(), &format!("perceus.slot.{slot}"));

        let fv = self.current_fn();
        let entry = fv.get_first_basic_block().expect("entry block");
        let saved = self.bld.get_insert_block();
        match entry.get_first_instruction() {
            Some(first) => self.bld.position_before(&first),
            None => self.bld.position_at_end(entry),
        }

        if let Some(after) = alloca
            .as_instruction_value()
            .and_then(|i| i.get_next_instruction())
        {
            self.bld.position_before(&after);
        } else {
            self.bld.position_at_end(entry);
        }
        let null = ptr_ty.const_null();
        let _ = self.bld.build_store(alloca, null);
        if let Some(b) = saved {
            self.bld.position_at_end(b);
        }
        self.current_reuse_alloca_slots.insert(slot, alloca);
        alloca
    }

    pub(crate) fn drain_reuse_slots(&mut self) {
        if self.current_reuse_alloca_slots.is_empty() {
            return;
        }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let pairs: Vec<(u32, PointerValue<'ctx>)> = self
            .current_reuse_alloca_slots
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        let free_fn = self.ensure_free();
        let header_ty = self.vec_header_type();
        for (slot, alloca) in pairs {
            let cur = match self.bld.build_load(ptr_ty, alloca, "perceus.drain.load") {
                Ok(v) => v.into_pointer_value(),
                Err(_) => continue,
            };
            let is_null = match self.bld.build_is_null(cur, "perceus.drain.null") {
                Ok(b) => b,
                Err(_) => continue,
            };
            let fv = self.current_fn();
            let free_bb = self.ctx.append_basic_block(fv, "perceus.drain.free");
            let cont_bb = self.ctx.append_basic_block(fv, "perceus.drain.cont");
            let _ = self.bld.build_conditional_branch(is_null, cont_bb, free_bb);
            self.bld.position_at_end(free_bb);

            if self.current_perceus_meta.vec_slots.contains(&slot) {
                if let Ok(data_gep) =
                    self.bld
                        .build_struct_gep(header_ty, cur, 0, "perceus.drain.dgep")
                {
                    if let Ok(data_v) = self.bld.build_load(ptr_ty, data_gep, "perceus.drain.d") {
                        let _ = self.bld.build_call(
                            free_fn,
                            &[data_v.into_pointer_value().into()],
                            "perceus.drain.free.buf",
                        );
                    }
                }
            }
            let _ = self
                .bld
                .build_call(free_fn, &[cur.into()], "perceus.drain.free");

            let null = ptr_ty.const_null();
            let _ = self.bld.build_store(alloca, null);
            let _ = self.bld.build_unconditional_branch(cont_bb);
            self.bld.position_at_end(cont_bb);
        }
    }

    pub(crate) fn try_save_vec_slot(
        &mut self,
        dropped: mir::ValueId,
        header_ptr: PointerValue<'ctx>,
    ) -> bool {
        let slot = match self.current_perceus_meta.reuse_save.get(&dropped).copied() {
            Some(s) if self.current_perceus_meta.vec_slots.contains(&s) => s,
            _ => return false,
        };
        let alloca = self.get_or_create_reuse_alloca(slot);
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.vec_header_type();

        if let Ok(prev) = self.bld.build_load(ptr_ty, alloca, "vec.save.prev") {
            let prev_ptr = prev.into_pointer_value();
            if let Ok(is_null) = self.bld.build_is_null(prev_ptr, "vec.save.prev.null") {
                let fv = self.current_fn();
                let free_bb = self.ctx.append_basic_block(fv, "vec.save.free");
                let cont_bb = self.ctx.append_basic_block(fv, "vec.save.cont");
                let _ = self.bld.build_conditional_branch(is_null, cont_bb, free_bb);
                self.bld.position_at_end(free_bb);
                let free_fn = self.ensure_free();
                if let Ok(data_gep) =
                    self.bld
                        .build_struct_gep(header_ty, prev_ptr, 0, "vec.save.dgep")
                {
                    if let Ok(data_v) = self.bld.build_load(ptr_ty, data_gep, "vec.save.d") {
                        let _ = self.bld.build_call(
                            free_fn,
                            &[data_v.into_pointer_value().into()],
                            "vec.save.free.buf",
                        );
                    }
                }
                let _ = self
                    .bld
                    .build_call(free_fn, &[prev_ptr.into()], "vec.save.free.hdr");
                let _ = self.bld.build_unconditional_branch(cont_bb);
                self.bld.position_at_end(cont_bb);
            }
        }
        let _ = self.bld.build_store(alloca, header_ptr);
        true
    }

    pub(crate) fn try_consume_vec_slot(&mut self) -> Option<PointerValue<'ctx>> {
        let dest = self.current_alloc_dest?;
        let slot = *self.current_perceus_meta.reuse_consume.get(&dest)?;
        if !self.current_perceus_meta.vec_slots.contains(&slot) {
            return None;
        }
        let alloca = self.get_or_create_reuse_alloca(slot);
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let cur = self
            .bld
            .build_load(ptr_ty, alloca, "vec.consume.load")
            .ok()?
            .into_pointer_value();

        let null = ptr_ty.const_null();
        let _ = self.bld.build_store(alloca, null);
        Some(cur)
    }

    pub(crate) fn ensure_free(&mut self) -> FunctionValue<'ctx> {
        self.module.get_function("free").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
            self.module
                .add_function("free", ft, Some(Linkage::External))
        })
    }

    pub(crate) fn ensure_snprintf(&mut self) -> FunctionValue<'ctx> {
        self.module.get_function("snprintf").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i64t = self.ctx.i64_type();
            let i32t = self.ctx.i32_type();
            let ft = i32t.fn_type(&[ptr_ty.into(), i64t.into(), ptr_ty.into()], true);
            self.module
                .add_function("snprintf", ft, Some(Linkage::External))
        })
    }

    pub(crate) fn ensure_memcmp(&mut self) -> FunctionValue<'ctx> {
        self.module.get_function("memcmp").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i64t = self.ctx.i64_type();
            let i32t = self.ctx.i32_type();
            let ft = i32t.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64t.into()], false);
            self.module
                .add_function("memcmp", ft, Some(Linkage::External))
        })
    }

    pub(crate) fn ensure_memcpy(&mut self) -> FunctionValue<'ctx> {
        self.module.get_function("memcpy").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i64t = self.ctx.i64_type();
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64t.into()], false);
            self.module
                .add_function("memcpy", ft, Some(Linkage::External))
        })
    }

    pub(crate) fn uses_concurrency(prog: &hir::Program) -> bool {
        use crate::hir::{ExprKind, Stmt};
        fn scan_expr(e: &hir::Expr) -> bool {
            match &e.kind {
                ExprKind::ChannelCreate(_, _)
                | ExprKind::ChannelSend(_, _)
                | ExprKind::ChannelRecv(_)
                | ExprKind::Select(_, _)
                | ExprKind::CoroutineCreate(_, _)
                | ExprKind::Yield(_) => true,
                _ => false,
            }
        }
        fn scan_stmt(s: &hir::Stmt) -> bool {
            match s {
                Stmt::ChannelClose(_, _)
                | Stmt::Stop(_, _)
                | Stmt::SimFor(_, _)
                | Stmt::SimBlock(_, _) => true,
                _ => false,
            }
        }
        fn scan_block(block: &[hir::Stmt]) -> bool {
            block.iter().any(|s| {
                if scan_stmt(s) {
                    return true;
                }
                match s {
                    Stmt::Expr(e) => scan_expr(e),
                    Stmt::Bind(b) => scan_expr(&b.value),
                    Stmt::If(i) => {
                        scan_block(&i.then)
                            || i.elifs.iter().any(|(c, b)| scan_expr(c) || scan_block(b))
                            || i.els.as_ref().map_or(false, |b| scan_block(b))
                    }
                    Stmt::While(w) => scan_expr(&w.cond) || scan_block(&w.body),
                    Stmt::For(f) => scan_expr(&f.iter) || scan_block(&f.body),
                    Stmt::Loop(l) => scan_block(&l.body),
                    Stmt::Match(m) => {
                        scan_expr(&m.subject) || m.arms.iter().any(|a| scan_block(&a.body))
                    }
                    Stmt::Ret(Some(e), _, _) => scan_expr(e),
                    _ => false,
                }
            })
        }
        fn scan_fn(f: &hir::Fn) -> bool {
            scan_block(&f.body)
        }
        prog.fns.iter().any(|f| scan_fn(f))
            || prog
                .types
                .iter()
                .any(|td| td.methods.iter().any(|m| scan_fn(m)))
            || prog
                .trait_impls
                .iter()
                .any(|ti| ti.methods.iter().any(|m| scan_fn(m)))
    }

    pub(crate) fn uses_pool(_prog: &hir::Program) -> bool {
        false
    }

    pub(crate) fn declare_jinn_runtime(&mut self) {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let void = self.ctx.void_type();
        let bool_t = self.ctx.bool_type();

        macro_rules! decl {
            ($name:expr, $ft:expr) => {
                if self.module.get_function($name).is_none() {
                    self.module
                        .add_function($name, $ft, Some(Linkage::External));
                }
            };
        }

        decl!(
            "jinn_coro_create",
            ptr.fn_type(&[ptr.into(), ptr.into()], false)
        );
        decl!("jinn_coro_destroy", void.fn_type(&[ptr.into()], false));
        decl!("jinn_coro_set_daemon", void.fn_type(&[ptr.into()], false));

        decl!("jinn_sched_init", void.fn_type(&[i32t.into()], false));
        decl!("jinn_sched_run", void.fn_type(&[], false));
        decl!("jinn_sched_shutdown", void.fn_type(&[], false));
        decl!("jinn_sched_spawn", void.fn_type(&[ptr.into()], false));
        decl!("jinn_sched_enqueue", void.fn_type(&[ptr.into()], false));
        decl!("jinn_sched_yield", void.fn_type(&[], false));
        decl!("jinn_sched_park", void.fn_type(&[], false));
        decl!("jinn_sched_unpark", void.fn_type(&[ptr.into()], false));
        decl!("jinn_current_coro", ptr.fn_type(&[], false));

        decl!(
            "jinn_chan_create",
            ptr.fn_type(&[i64t.into(), i64t.into()], false)
        );
        decl!("jinn_chan_destroy", void.fn_type(&[ptr.into()], false));
        decl!(
            "jinn_chan_send",
            void.fn_type(&[ptr.into(), ptr.into()], false)
        );
        decl!(
            "jinn_chan_recv",
            i32t.fn_type(&[ptr.into(), ptr.into()], false)
        );
        decl!(
            "jinn_chan_try_recv",
            i32t.fn_type(&[ptr.into(), ptr.into()], false)
        );
        decl!("jinn_chan_close", void.fn_type(&[ptr.into()], false));

        decl!("jinn_actor_destroy", void.fn_type(&[ptr.into()], false));
        decl!("jinn_actor_stop", void.fn_type(&[ptr.into()], false));

        decl!(
            "jinn_select",
            i32t.fn_type(&[ptr.into(), i32t.into(), bool_t.into()], false)
        );

        decl!(
            "jinn_timer_set",
            void.fn_type(&[ptr.into(), i64t.into()], false)
        );
        decl!("jinn_timer_check", void.fn_type(&[], false));

        self.ensure_malloc();
        self.ensure_free();
        self.ensure_memcpy();
    }
}
