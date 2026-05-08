//! Compiler construction, target setup, LLVM helpers, runtime declarations, and module-level utilities.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub fn new(ctx: &'ctx Context, name: &str) -> Self {
        Self {
            module: ctx.create_module(name),
            bld: ctx.create_builder(),
            alloca_bld: ctx.create_builder(),
            ctx,
            cur_fn: None,
            vars: IndexMap::new(),
            var_shadows: Vec::new(),
            var_scope_markers: Vec::new(),
            fns: IndexMap::new(),
            structs: IndexMap::new(),
            struct_defaults: IndexMap::new(),
            struct_layouts: IndexMap::new(),
            enums: IndexMap::new(),
            variant_tags: IndexMap::new(),
            loop_stack: Vec::new(),
            source: String::new(),
            hints: PerceusHints::default(),
            lib_mode: false,
            debug: false,
            di_builder: None,
            di_compile_unit: None,
            di_scope_stack: Vec::new(),
            filename: name.to_string(),
            store_defs: IndexMap::new(),
            actor_defs: IndexMap::new(),
            vtables: IndexMap::new(),
            trait_method_order: IndexMap::new(),
            needs_runtime: false,
            needs_ssl: false,
            needs_sqlite: false,
            globals: IndexMap::new(),
            fast_math_flags: 0,
            reuse_tokens: IndexMap::new(),
            atomic_vars: HashSet::new(),
            target_triple: None,
            target_cpu: None,
            target_features: None,
            standalone: false,
            tbaa_kind_id: ctx.get_kind_id("tbaa"),
            tbaa_root: None,
            empty_vec_growth_floor: 16,
            value_map: std::collections::HashMap::new(),
            block_map: std::collections::HashMap::new(),
            pending_phis: Vec::new(),
            var_allocs: std::collections::HashMap::new(),
            value_types: std::collections::HashMap::new(),
            coro_bodies: std::collections::HashMap::new(),
            select_data_bufs: std::collections::HashMap::new(),
            self_allocs: std::collections::HashMap::new(),
            self_alloc_types: std::collections::HashMap::new(),
            block_exit_map: std::collections::HashMap::new(),
            migration_fns: Vec::new(),
            global_init_fn: None,
            vec_growth_floor_by_value: std::collections::HashMap::new(),
        }
    }

    pub fn set_empty_vec_growth_floor(&mut self, cap: u64) {
        // Keep this small and power-of-two for allocator friendliness.
        let clamped = cap.clamp(16, 128);
        self.empty_vec_growth_floor = clamped.next_power_of_two();
    }

    /// Infer a per-program empty-vec initial growth floor from MIR.
    ///
    /// We analyze empty `VecNew([])` sites and count static `VecPush` uses on
    /// those vectors. Then we choose a conservative floor:
    /// - max/p90 >= 32 -> 64
    /// - max/p90 >= 16 -> 32
    /// - otherwise     -> 16
    ///
    /// This remains robust for outlier-heavy programs (e.g. alloc churn loops)
    /// while avoiding over-allocation in tiny push-only scripts.
    pub fn tune_empty_vec_growth_floor_from_mir(&mut self, prog: &mir::Program) {
        use std::collections::HashMap;

        let mut empty_vec_push_counts: Vec<u32> = Vec::new();

        for func in &prog.functions {
            let mut pushes_by_vec: HashMap<mir::ValueId, u32> = HashMap::new();

            for bb in &func.blocks {
                for inst in &bb.insts {
                    if let mir::InstKind::VecPush(vec_id, _) = inst.kind {
                        *pushes_by_vec.entry(vec_id).or_insert(0) += 1;
                    }
                }
            }

            for bb in &func.blocks {
                for inst in &bb.insts {
                    if let (Some(dest), mir::InstKind::VecNew(elems)) = (&inst.dest, &inst.kind) {
                        if elems.is_empty() {
                            empty_vec_push_counts.push(*pushes_by_vec.get(dest).unwrap_or(&0));
                        }
                    }
                }
            }
        }

        if empty_vec_push_counts.is_empty() {
            return;
        }

        empty_vec_push_counts.sort_unstable();
        let max_push = *empty_vec_push_counts.last().unwrap_or(&0);
        let p90_idx = ((empty_vec_push_counts.len() as f64) * 0.90).floor() as usize;
        let p90_idx = p90_idx.min(empty_vec_push_counts.len().saturating_sub(1));
        let p90 = empty_vec_push_counts[p90_idx];

        let floor = if max_push >= 32 || p90 >= 32 {
            64
        } else if max_push >= 16 || p90 >= 16 {
            32
        } else {
            16
        };
        self.set_empty_vec_growth_floor(floor);
    }

    /// Initialize the TBAA metadata tree.  Must be called after `new()`.
    pub fn init_tbaa(&mut self) {
        let root_name = self.ctx.metadata_string("Jade TBAA");
        let root = self.ctx.metadata_node(&[root_name.into()]);
        self.tbaa_root = Some(root);
    }

    /// Create a TBAA type descriptor node for a named type (scalar/pointer).
    pub(crate) fn tbaa_type_node(
        &self,
        name: &str,
    ) -> Option<inkwell::values::MetadataValue<'ctx>> {
        let root = self.tbaa_root.as_ref()?;
        let name_md = self.ctx.metadata_string(name);
        // TBAA type descriptor: {name, parent, constant_flag=0}
        let zero = self.ctx.i64_type().const_int(0, false);
        Some(
            self.ctx
                .metadata_node(&[name_md.into(), (*root).into(), zero.into()]),
        )
    }

    /// Create a TBAA access tag for a load/store and attach it to the instruction.
    pub(crate) fn set_tbaa(&self, inst: inkwell::values::InstructionValue<'ctx>, type_name: &str) {
        if let Some(type_node) = self.tbaa_type_node(type_name) {
            let zero = self.ctx.i64_type().const_int(0, false);
            // TBAA access tag: {base_type, access_type, offset}
            let access_tag =
                self.ctx
                    .metadata_node(&[type_node.into(), type_node.into(), zero.into()]);
            let _ = inst.set_metadata(access_tag, self.tbaa_kind_id);
        }
    }

    /// Map a Jade type to a TBAA type name for alias analysis.
    pub(crate) fn tbaa_type_name(ty: &Type) -> &'static str {
        match ty {
            Type::I8 | Type::U8 => "i8",
            Type::I16 | Type::U16 => "i16",
            Type::I32 | Type::U32 => "i32",
            Type::I64 | Type::U64 => "i64",
            Type::F32 => "f32",
            Type::F64 => "f64",
            Type::Bool => "bool",
            Type::String => "string",
            Type::Vec(_) => "vec",
            Type::Map(_, _) => "map",
            Type::Set(_) => "set",
            Type::Channel(_) => "channel",
            Type::ActorRef(_) => "actor_ref",
            Type::Struct(_, _) => "struct",
            Type::Enum(_) => "enum",
            Type::Fn(_, _) => "closure",
            Type::Ptr(_) | Type::Rc(_) | Type::Weak(_) => "pointer",
            _ => "any",
        }
    }

    /// Return the known dereferenceable byte count for a pointer-represented type,
    /// or None if the size is unknown at compile time.
    pub(crate) fn dereferenceable_bytes(&self, ty: &Type) -> Option<u64> {
        match ty {
            Type::Struct(name, _) => {
                // Use LLVM's struct type to compute size if registered
                let st = self.module.get_struct_type(&name.as_str())?;
                let dl_str = self.module.get_data_layout();
                let td = inkwell::targets::TargetData::create(dl_str.as_str().to_str().ok()?);
                Some(td.get_abi_size(&st))
            }
            _ => None,
        }
    }

    pub fn set_source(&mut self, src: &str) {
        self.source = src.to_string();
    }

    pub fn set_lib_mode(&mut self) {
        self.lib_mode = true;
    }

    /// Enable fast-math flags on all floating-point instructions.
    /// Flags: nnan | ninf | nsz | arcp | contract | afn | reassoc
    pub fn set_fast_math(&mut self, enable: bool) {
        if enable {
            // LLVMFastMathAll = 0x7F (all 7 flags)
            self.fast_math_flags = 0x7F;
        } else {
            self.fast_math_flags = 0;
        }
    }

    /// Enable deterministic FP mode — disables all fast-math reordering.
    /// This is the default (flags = 0), but calling this explicitly after
    /// set_fast_math will override back to strict IEEE 754 compliance.
    pub fn set_deterministic_fp(&mut self) {
        self.fast_math_flags = 0;
    }

    /// Tag a float instruction with the current fast-math flags.
    pub(crate) fn tag_fast_math(&self, val: BasicValueEnum<'ctx>) {
        if self.fast_math_flags != 0 {
            if let Some(inst) = val.as_instruction_value() {
                inst.set_fast_math_flags(self.fast_math_flags);
            }
        }
    }

    pub fn enable_debug(&mut self, filename: &str) {
        self.debug = true;
        self.filename = filename.to_string();
        let (di_builder, di_cu) = self.module.create_debug_info_builder(
            true,
            DWARFSourceLanguage::C,
            filename,
            ".",
            "jadec",
            false,
            "",
            0,
            "",
            DWARFEmissionKind::Full,
            0,
            false,
            false,
            "",
            "",
        );
        self.di_builder = Some(di_builder);
        self.di_compile_unit = Some(di_cu);
    }

    pub fn emit_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn emit_ir_optimized(&self, opt: OptimizationLevel) -> Result<String, String> {
        self.run_optimization_passes(opt)?;
        Ok(self.module.print_to_string().to_string())
    }

    pub fn emit_object(&self, path: &Path, opt: OptimizationLevel) -> Result<(), String> {
        let tm = self.run_optimization_passes(opt)?;
        tm.write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| e.to_string())
    }

    pub(in crate::codegen) fn run_optimization_passes(
        &self,
        opt: OptimizationLevel,
    ) -> Result<TargetMachine, String> {
        let passes = match opt {
            OptimizationLevel::None => "default<O0>",
            OptimizationLevel::Less => "default<O1>",
            OptimizationLevel::Default => "default<O2>",
            OptimizationLevel::Aggressive => "default<O3>",
        };
        let tm = self.target_machine(opt)?;
        let pb = PassBuilderOptions::create();
        let o2plus = matches!(
            opt,
            OptimizationLevel::Default | OptimizationLevel::Aggressive
        );
        pb.set_loop_vectorization(o2plus);
        pb.set_loop_slp_vectorization(o2plus);
        pb.set_loop_unrolling(o2plus);
        pb.set_loop_interleaving(o2plus);
        pb.set_call_graph_profile(o2plus);
        pb.set_merge_functions(matches!(opt, OptimizationLevel::Aggressive));
        self.module
            .run_passes(passes, &tm, pb)
            .map_err(|e| e.to_string())?;
        Ok(tm)
    }

    pub(crate) fn generate_vtables(
        &mut self,
        trait_impls: &[hir::TraitImpl],
    ) -> Result<(), String> {
        let ptr = self.ctx.ptr_type(inkwell::AddressSpace::default());

        for ti in trait_impls {
            if let Some(ref trait_name) = ti.trait_name {
                let order = self
                    .trait_method_order
                    .entry(trait_name.clone())
                    .or_default();
                for m in &ti.methods {
                    let base_name = m
                        .name
                        .strip_prefix(&format!("{}_", ti.type_name))
                        .unwrap_or(m.name);
                    if !order.contains(&base_name.to_string()) {
                        order.push(base_name.to_string());
                    }
                }
            }
        }

        for ti in trait_impls {
            if let Some(ref trait_name) = ti.trait_name {
                let method_order = self
                    .trait_method_order
                    .get(trait_name)
                    .cloned()
                    .unwrap_or_default();
                let mut fn_ptrs: Vec<inkwell::values::PointerValue<'ctx>> = Vec::new();
                for method_name in &method_order {
                    let mangled = format!("{}_{method_name}", ti.type_name);
                    if let Some((fv, param_tys, ret_ty)) = self.fns.get(&mangled).cloned() {
                        let thunk_name = format!("__thunk_{mangled}");
                        let mut thunk_param_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> =
                            vec![ptr.into()];
                        for pt in param_tys.iter().skip(1) {
                            thunk_param_tys.push(self.llvm_ty(pt).into());
                        }
                        let thunk_ret = self.llvm_ty(&ret_ty);
                        let thunk_fn_ty = thunk_ret.fn_type(&thunk_param_tys, false);
                        let thunk_fn = self.module.add_function(&thunk_name, thunk_fn_ty, None);
                        let entry = self.ctx.append_basic_block(thunk_fn, "entry");
                        self.bld.position_at_end(entry);
                        let self_ptr = ice!(
                            thunk_fn.get_first_param(),
                            "vtable thunk missing self param"
                        )
                        .into_pointer_value();
                        let first_arg: inkwell::values::BasicValueEnum<'ctx> =
                            if matches!(param_tys.first(), Some(Type::Ptr(_))) {
                                self_ptr.into()
                            } else {
                                let concrete_ty: inkwell::types::BasicTypeEnum<'ctx> = self
                                    .module
                                    .get_struct_type(&ti.type_name.as_str())
                                    .map(|st| st.into())
                                    .unwrap_or_else(|| self.ctx.i64_type().into());
                                b!(self.bld.build_load(concrete_ty, self_ptr, "self.loaded"))
                            };
                        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                            vec![first_arg.into()];
                        for i in 1..thunk_fn.count_params() {
                            call_args.push(
                                ice!(thunk_fn.get_nth_param(i), "vtable thunk missing param")
                                    .into(),
                            );
                        }
                        let result = b!(self.bld.build_call(fv, &call_args, "thunk.call"));
                        if let Some(rv) = result.try_as_basic_value().basic() {
                            b!(self.bld.build_return(Some(&rv)));
                        } else {
                            b!(self.bld.build_return(None));
                        }
                        fn_ptrs.push(thunk_fn.as_global_value().as_pointer_value());
                    } else {
                        fn_ptrs.push(ptr.const_null());
                    }
                }
                if fn_ptrs.is_empty() {
                    continue;
                }
                let arr_ty = ptr.array_type(fn_ptrs.len() as u32);
                let vtable_const = ptr.const_array(&fn_ptrs);
                let vtable_name = format!("__vtable_{}_{}", ti.type_name, trait_name);
                let gv = self.module.add_global(arr_ty, None, &vtable_name);
                gv.set_initializer(&vtable_const);
                gv.set_constant(true);
                gv.set_linkage(inkwell::module::Linkage::Internal);
                self.vtables
                    .insert((ti.type_name.as_str(), trait_name.as_str()), gv);
            }
        }
        Ok(())
    }
}

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

    /// Emit LLVM parameter attributes based on Jade's ownership model.
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
        // All pointer params are non-nullable in Jade
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
        jade_ty: &Type,
    ) -> PointerValue<'ctx> {
        let align = if let Type::Struct(sname, _) = jade_ty {
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
        if let Some(f) = self.module.get_function("jade_xmalloc") {
            return f;
        }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let ft = ptr_ty.fn_type(&[i64t.into()], false);
        let func = self
            .module
            .add_function("jade_xmalloc", ft, Some(Linkage::WeakAny));

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

    pub(crate) fn declare_jade_runtime(&mut self) {
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
            "jade_coro_create",
            ptr.fn_type(&[ptr.into(), ptr.into()], false)
        );
        decl!("jade_coro_destroy", void.fn_type(&[ptr.into()], false));
        decl!("jade_coro_set_daemon", void.fn_type(&[ptr.into()], false));

        decl!("jade_sched_init", void.fn_type(&[i32t.into()], false));
        decl!("jade_sched_run", void.fn_type(&[], false));
        decl!("jade_sched_shutdown", void.fn_type(&[], false));
        decl!("jade_sched_spawn", void.fn_type(&[ptr.into()], false));
        decl!("jade_sched_enqueue", void.fn_type(&[ptr.into()], false));
        decl!("jade_sched_yield", void.fn_type(&[], false));
        decl!("jade_sched_park", void.fn_type(&[], false));
        decl!("jade_sched_unpark", void.fn_type(&[ptr.into()], false));
        decl!("jade_current_coro", ptr.fn_type(&[], false));

        decl!(
            "jade_chan_create",
            ptr.fn_type(&[i64t.into(), i64t.into()], false)
        );
        decl!("jade_chan_destroy", void.fn_type(&[ptr.into()], false));
        decl!(
            "jade_chan_send",
            void.fn_type(&[ptr.into(), ptr.into()], false)
        );
        decl!(
            "jade_chan_recv",
            i32t.fn_type(&[ptr.into(), ptr.into()], false)
        );
        decl!(
            "jade_chan_try_recv",
            i32t.fn_type(&[ptr.into(), ptr.into()], false)
        );
        decl!("jade_chan_close", void.fn_type(&[ptr.into()], false));

        decl!("jade_actor_destroy", void.fn_type(&[ptr.into()], false));
        decl!("jade_actor_stop", void.fn_type(&[ptr.into()], false));

        decl!(
            "jade_select",
            i32t.fn_type(&[ptr.into(), i32t.into(), bool_t.into()], false)
        );

        decl!(
            "jade_timer_set",
            void.fn_type(&[ptr.into(), i64t.into()], false)
        );
        decl!("jade_timer_check", void.fn_type(&[], false));

        self.ensure_malloc();
        self.ensure_free();
        self.ensure_memcpy();
    }
}
