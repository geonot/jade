//! Compiler construction, tuning, metadata, debug setup, IR emission, optimization, and vtables.

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
            current_perceus_meta: mir::PerceusMeta::default(),
            current_reuse_slots: std::collections::HashMap::new(),
            current_reuse_alloca_slots: std::collections::HashMap::new(),
            current_alloc_dest: None,
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
        let root_name = self.ctx.metadata_string("Jinn TBAA");
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

    /// Map a Jinn type to a TBAA type name for alias analysis.
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
            "jinnc",
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
