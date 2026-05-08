//! Target setup, LLVM utility helpers, local variables, allocation, runtime detection, and runtime declarations.

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

    /// Create a zero initializer for the given type (used for global variables).
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

    /// Compile a simple constant HIR expression (literals only, used for global init).
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

    pub(in crate::codegen) fn tag_fn_inner(&self, fv: FunctionValue<'ctx>, will_return: bool) {
        for a in ["nounwind", "nosync", "nofree", "mustprogress"] {
            fv.add_attribute(AttributeLoc::Function, self.attr(a));
        }
        if will_return {
            fv.add_attribute(AttributeLoc::Function, self.attr("willreturn"));
            fv.add_attribute(AttributeLoc::Function, self.attr("norecurse"));
        }
    }

    /// Emit LLVM parameter attributes based on Jinn's ownership model.
    ///
    /// - Owned pointer params  → `noalias` (exclusive, no other ref exists)
    /// - Borrowed pointer params → `noalias readonly` (shared read-only)
    /// - BorrowMut pointer params → `noalias` (exclusive mutable borrow)
    /// - Rc/Weak → shared refcount, no noalias
    /// - Raw → user-managed, no assumptions
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
        // All pointer params are non-nullable in Jinn
        fv.add_attribute(loc, self.attr("nonnull"));
        match ownership {
            hir::Ownership::Owned | hir::Ownership::BorrowMut => {
                fv.add_attribute(loc, self.attr("noalias"));
                // Owned values don't escape the callee
                fv.add_attribute(loc, self.attr("nocapture"));
            }
            hir::Ownership::Borrowed => {
                fv.add_attribute(loc, self.attr("noalias"));
                fv.add_attribute(loc, self.attr("readonly"));
                // Borrowed values don't escape
                fv.add_attribute(loc, self.attr("nocapture"));
            }
            // Rc/Weak are shared-ownership — aliased by design
            // Raw is user-managed — we make no assumptions
            hir::Ownership::Rc | hir::Ownership::Weak | hir::Ownership::Raw => {}
        }
        // Add dereferenceable(N) for known struct sizes
        if let Some(size) = self.dereferenceable_bytes(ty) {
            let deref_attr = self
                .ctx
                .create_enum_attribute(Attribute::get_named_enum_kind_id("dereferenceable"), size);
            fv.add_attribute(loc, deref_attr);
        }
    }

    pub(crate) fn set_var(&mut self, name: &str, ptr: PointerValue<'ctx>, ty: Type) {
        let old = self.vars.insert(name.into(), (ptr, ty));
        self.var_shadows.push((name.to_string(), old));
    }

    pub(crate) fn find_var(&self, name: &str) -> Option<&(PointerValue<'ctx>, Type)> {
        self.vars.get(name)
    }

    pub(crate) fn push_var_scope(&mut self) {
        self.var_scope_markers.push(self.var_shadows.len());
    }

    pub(crate) fn pop_var_scope(&mut self) {
        let marker = self.var_scope_markers.pop().expect("no scope to pop");
        while self.var_shadows.len() > marker {
            let (name, prev) = self.var_shadows.pop().unwrap();
            let sym: Symbol = name.into();
            match prev {
                Some(val) => {
                    self.vars.insert(sym, val);
                }
                None => {
                    self.vars.swap_remove(&sym);
                }
            }
        }
    }

    pub(crate) fn load_var(&mut self, name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some((ptr, ty)) = self.find_var(name).cloned() {
            let load = b!(self.bld.build_load(self.llvm_ty(&ty), ptr, name));
            if self.atomic_vars.contains(&Symbol::intern(name)) {
                ice!(
                    load.as_instruction_value(),
                    "atomic load produced non-instruction"
                )
                .set_atomic_ordering(inkwell::AtomicOrdering::SequentiallyConsistent)
                .map_err(|_| "failed to set atomic ordering")?;
            }
            return Ok(load);
        }
        if let Some(fv) = self.module.get_function(name) {
            let wrapper = self.fn_ref_wrapper(fv);
            let null_env = self
                .ctx
                .ptr_type(inkwell::AddressSpace::default())
                .const_null();
            return self.make_closure(wrapper, null_env);
        }
        Err(format!("undefined: {name}"))
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

    pub(crate) fn entry_alloca_aligned(
        &self,
        ty: BasicTypeEnum<'ctx>,
        name: &str,
        align: u32,
    ) -> PointerValue<'ctx> {
        let ptr = self.entry_alloca(ty, name);
        ice!(
            ptr.as_instruction_value(),
            "alloca produced non-instruction"
        )
        .set_alignment(align)
        .expect("failed to set alignment");
        ptr
    }

    /// Alloca that respects @align layout attribute for struct types.
    pub(crate) fn alloca_for_type(
        &self,
        llvm_ty: BasicTypeEnum<'ctx>,
        name: &str,
        jinn_ty: &Type,
    ) -> PointerValue<'ctx> {
        let align = if let Type::Struct(sname, _) = jinn_ty {
            self.struct_layouts.get(sname).and_then(|l| l.align)
        } else {
            None
        };
        if let Some(a) = align {
            self.entry_alloca_aligned(llvm_ty, name, a)
        } else {
            self.entry_alloca(llvm_ty, name)
        }
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

    /// Return the current LLVM function, panicking with an ICE if none is set.
    #[track_caller]
    pub(crate) fn current_fn(&self) -> FunctionValue<'ctx> {
        ice!(self.cur_fn, "no current function")
    }

    /// Return the current insert basic block, panicking with an ICE if none exists.
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

        // Define the function body inline: call malloc, abort on NULL
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

    /// Try to find and consume a reuse token whose allocation is layout-compatible
    /// with the requested byte size. Returns the saved pointer if found.
    pub(crate) fn try_consume_reuse_token(
        &mut self,
        needed_size: u64,
    ) -> Option<PointerValue<'ctx>> {
        // Find any reuse token whose released type has a compatible layout size
        let matching_id = self
            .hints
            .reuse_candidates
            .iter()
            .chain(self.hints.speculative_reuse.iter())
            .find_map(|(def_id, info)| {
                if self.reuse_tokens.contains_key(def_id) {
                    let released_size =
                        crate::perceus::PerceusPass::type_layout_size_pub(&info.released_ty);
                    if released_size >= needed_size && released_size > 0 {
                        return Some(*def_id);
                    }
                }
                None
            });
        matching_id.and_then(|id| self.reuse_tokens.shift_remove(&id))
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

    pub(crate) fn uses_pool(prog: &hir::Program) -> bool {
        use crate::hir::{BuiltinFn, ExprKind, Stmt};
        fn has_pool(e: &hir::Expr) -> bool {
            matches!(
                &e.kind,
                ExprKind::Builtin(
                    BuiltinFn::PoolNew
                        | BuiltinFn::PoolAlloc
                        | BuiltinFn::PoolFree
                        | BuiltinFn::PoolDestroy,
                    _
                )
            )
        }
        fn scan_block(block: &[hir::Stmt]) -> bool {
            block.iter().any(|s| match s {
                Stmt::Expr(e) => has_pool(e),
                Stmt::Bind(b) => has_pool(&b.value),
                Stmt::If(i) => {
                    scan_block(&i.then)
                        || i.elifs.iter().any(|(_, b)| scan_block(b))
                        || i.els.as_ref().map_or(false, |b| scan_block(b))
                }
                Stmt::While(w) => scan_block(&w.body),
                Stmt::For(f) => scan_block(&f.body),
                Stmt::Loop(l) => scan_block(&l.body),
                Stmt::Match(m) => m.arms.iter().any(|a| scan_block(&a.body)),
                _ => false,
            })
        }
        prog.fns.iter().any(|f| scan_block(&f.body))
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
