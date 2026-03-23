mod builtins;
mod call;
mod collections;
mod decl;
mod expr;
mod stmt;
mod strings;
mod types;
mod actors;
mod stores;

use std::collections::HashMap;
use std::path::Path;

use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::{AddressSpace, OptimizationLevel};

use inkwell::attributes::{Attribute, AttributeLoc};

use inkwell::debug_info::{AsDIScope, DICompileUnit, DIFlags, DIFlagsConstants, DIScope, DWARFEmissionKind, DWARFSourceLanguage, DebugInfoBuilder};

use crate::hir;
use crate::perceus::PerceusHints;
use crate::types::Type;

macro_rules! b {
    ($e:expr) => {
        $e.map_err(|e| e.to_string())?
    };
}
pub(crate) use b;

pub struct Compiler<'ctx> {
    pub(crate) ctx: &'ctx Context,
    pub(crate) module: Module<'ctx>,
    pub(crate) bld: Builder<'ctx>,
    pub(crate) cur_fn: Option<FunctionValue<'ctx>>,
    pub(crate) vars: Vec<HashMap<String, (PointerValue<'ctx>, Type)>>,
    pub(crate) fns: HashMap<String, (FunctionValue<'ctx>, Vec<Type>, Type)>,
    pub(crate) structs: HashMap<String, Vec<(String, Type)>>,
    pub(crate) struct_layouts: HashMap<String, crate::ast::LayoutAttrs>,
    pub(crate) enums: HashMap<String, Vec<(String, Vec<Type>)>>,
    pub(crate) variant_tags: HashMap<String, (String, u32)>,
    pub(crate) loop_stack: Vec<LoopCtx<'ctx>>,
    pub(crate) source: String,
    pub(crate) hints: PerceusHints,
    pub(crate) lib_mode: bool,
    pub(crate) debug: bool,
    pub(crate) di_builder: Option<DebugInfoBuilder<'ctx>>,
    pub(crate) di_compile_unit: Option<DICompileUnit<'ctx>>,
    pub(crate) di_scope_stack: Vec<DIScope<'ctx>>,
    pub(crate) filename: String,
    pub(crate) store_defs: HashMap<String, hir::StoreDef>,
    /// VTable globals: (type_name, trait_name) → global pointer to vtable array
    pub(crate) vtables: HashMap<(String, String), inkwell::values::GlobalValue<'ctx>>,
    /// Trait method order: trait_name → [method_name, ...]
    pub(crate) trait_method_order: HashMap<String, Vec<String>>,
}

pub(crate) struct LoopCtx<'ctx> {
    pub continue_bb: BasicBlock<'ctx>,
    pub break_bb: BasicBlock<'ctx>,
}

impl<'ctx> Compiler<'ctx> {
    pub fn new(ctx: &'ctx Context, name: &str) -> Self {
        Self {
            module: ctx.create_module(name),
            bld: ctx.create_builder(),
            ctx,
            cur_fn: None,
            vars: vec![HashMap::new()],
            fns: HashMap::new(),
            structs: HashMap::new(),
            struct_layouts: HashMap::new(),
            enums: HashMap::new(),
            variant_tags: HashMap::new(),
            loop_stack: Vec::new(),
            source: String::new(),
            hints: PerceusHints::default(),
            lib_mode: false,
            debug: false,
            di_builder: None,
            di_compile_unit: None,
            di_scope_stack: Vec::new(),
            filename: name.to_string(),
            store_defs: HashMap::new(),
            vtables: HashMap::new(),
            trait_method_order: HashMap::new(),
        }
    }

    pub fn set_source(&mut self, src: &str) {
        self.source = src.to_string();
    }

    pub fn set_lib_mode(&mut self) {
        self.lib_mode = true;
    }

    pub fn enable_debug(&mut self, filename: &str) {
        self.debug = true;
        self.filename = filename.to_string();
        let (di_builder, di_cu) = self.module.create_debug_info_builder(
            true,
            // DWARF: use user-defined language range for Jade (0x8000 + 1)
            // C is used as fallback since inkwell doesn't expose DW_LANG_lo_user directly
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
        let passes = match opt {
            OptimizationLevel::None => "default<O0>",
            OptimizationLevel::Less => "default<O1>",
            OptimizationLevel::Default => "default<O2>",
            OptimizationLevel::Aggressive => "default<O3>",
        };
        let tm = self.target_machine(opt)?;
        let pb = PassBuilderOptions::create();
        let o2plus = matches!(opt, OptimizationLevel::Default | OptimizationLevel::Aggressive);
        pb.set_loop_vectorization(o2plus);
        pb.set_loop_slp_vectorization(o2plus);
        pb.set_loop_unrolling(o2plus);
        pb.set_loop_interleaving(o2plus);
        pb.set_call_graph_profile(o2plus);
        pb.set_merge_functions(matches!(opt, OptimizationLevel::Aggressive));
        self.module.run_passes(passes, &tm, pb).map_err(|e| e.to_string())?;
        Ok(self.module.print_to_string().to_string())
    }

    pub fn emit_object(&self, path: &Path, opt: OptimizationLevel) -> Result<(), String> {
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
        tm.write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| e.to_string())
    }

    pub fn compile_program(
        &mut self,
        prog: &hir::Program,
        hints: PerceusHints,
    ) -> Result<(), String> {
        self.hints = hints;
        self.setup_target()?;
        self.declare_builtins();

        // SSO correctness: string struct must be exactly 24 bytes ({ptr, i64, i64}).
        debug_assert_eq!(
            self.type_store_size(self.string_type().into()),
            24,
            "String SSO layout changed — expected 24 bytes"
        );

        for td in &prog.types {
            self.declare_type(td)?;
            for m in &td.methods {
                self.declare_method(&td.name, m)?;
            }
        }
        for ed in &prog.enums {
            self.declare_enum(ed)?;
        }
        for ef in &prog.externs {
            self.declare_extern(ef)?;
        }
        for ed in &prog.err_defs {
            self.declare_err_def(ed)?;
        }

        // Declare actor runtime if actors are present
        if !prog.actors.is_empty() {
            self.declare_actor_runtime();
            for ad in &prog.actors {
                self.declare_actor(ad)?;
            }
        }

        // Declare store runtime if stores are present
        if !prog.stores.is_empty() {
            self.declare_store_runtime();
            for sd in &prog.stores {
                self.declare_store(sd)?;
                self.store_defs.insert(sd.name.clone(), sd.clone());
            }
        }

        for f in &prog.fns {
            self.declare_fn(f)?;
        }

        // Declare trait impl methods (before compilation, so calls resolve)
        for ti in &prog.trait_impls {
            for m in &ti.methods {
                if !self.fns.contains_key(&m.name) {
                    self.declare_method(&ti.type_name, m)?;
                }
            }
        }

        // Generate vtables for dyn Trait dispatch (after declarations, before body compilation)
        self.generate_vtables(&prog.trait_impls)?;

        // Compile actor loop functions before user functions
        // (so spawn can reference the loop fn)
        for ad in &prog.actors {
            self.compile_actor_loop(ad)?;
        }

        for f in &prog.fns {
            self.compile_fn(f)?;
        }
        for td in &prog.types {
            for m in &td.methods {
                if self.fns.contains_key(&m.name) {
                    self.compile_method_body(&td.name, &m.name, m)?;
                }
            }
        }

        // Compile trait impl method bodies
        for ti in &prog.trait_impls {
            for m in &ti.methods {
                if self.fns.contains_key(&m.name) {
                    self.compile_method_body(&ti.type_name, &m.name, m)?;
                }
            }
        }

        // Generate vtables already done above (before body compilation)

        self.finalize_debug();
        self.module.verify().map_err(|e| e.to_string())
    }

    fn generate_vtables(&mut self, trait_impls: &[hir::TraitImpl]) -> Result<(), String> {
        let ptr = self.ctx.ptr_type(inkwell::AddressSpace::default());

        // Build trait method order from the impls
        for ti in trait_impls {
            if let Some(ref trait_name) = ti.trait_name {
                let order = self.trait_method_order
                    .entry(trait_name.clone())
                    .or_default();
                for m in &ti.methods {
                    // Method names are mangled as TypeName_methodname
                    let base_name = m.name.strip_prefix(&format!("{}_", ti.type_name))
                        .unwrap_or(&m.name);
                    if !order.contains(&base_name.to_string()) {
                        order.push(base_name.to_string());
                    }
                }
            }
        }

        // For each (type, trait) pair, build thunks and a vtable global
        for ti in trait_impls {
            if let Some(ref trait_name) = ti.trait_name {
                let method_order = self.trait_method_order.get(trait_name).cloned().unwrap_or_default();
                let mut fn_ptrs: Vec<inkwell::values::PointerValue<'ctx>> = Vec::new();
                for method_name in &method_order {
                    let mangled = format!("{}_{method_name}", ti.type_name);
                    if let Some((fv, param_tys, ret_ty)) = self.fns.get(&mangled).cloned() {
                        // Generate a thunk: takes ptr as self, loads concrete type, calls original
                        let thunk_name = format!("__thunk_{mangled}");
                        let mut thunk_param_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = vec![ptr.into()];
                        // Skip the first param (self) — remaining params keep their types
                        for pt in param_tys.iter().skip(1) {
                            thunk_param_tys.push(self.llvm_ty(pt).into());
                        }
                        let thunk_ret = self.llvm_ty(&ret_ty);
                        let thunk_fn_ty = thunk_ret.fn_type(&thunk_param_tys, false);
                        let thunk_fn = self.module.add_function(&thunk_name, thunk_fn_ty, None);
                        let entry = self.ctx.append_basic_block(thunk_fn, "entry");
                        self.bld.position_at_end(entry);
                        // Load concrete type from ptr
                        let self_ptr = thunk_fn.get_first_param().unwrap().into_pointer_value();
                        let concrete_ty: inkwell::types::BasicTypeEnum<'ctx> = self.module.get_struct_type(&ti.type_name)
                            .map(|st| st.into())
                            .unwrap_or_else(|| self.ctx.i64_type().into());
                        let loaded_self = b!(self.bld.build_load(concrete_ty, self_ptr, "self.loaded"));
                        // Build call args: loaded self + forwarded params
                        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = vec![loaded_self.into()];
                        for i in 1..thunk_fn.count_params() {
                            call_args.push(thunk_fn.get_nth_param(i).unwrap().into());
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
                self.vtables.insert(
                    (ti.type_name.clone(), trait_name.clone()),
                    gv,
                );
            }
        }
        Ok(())
    }
}

impl<'ctx> Compiler<'ctx> {
    fn finalize_debug(&self) {
        if let Some(ref di) = self.di_builder {
            di.finalize();
        }
    }

    /// Create a debug info subprogram for a function and set it.
    pub(crate) fn create_debug_function(
        &mut self,
        fv: FunctionValue<'ctx>,
        name: &str,
        line: u32,
    ) {
        if !self.debug {
            return;
        }
        let di = self.di_builder.as_ref().unwrap();
        let cu = self.di_compile_unit.as_ref().unwrap();
        let file = cu.get_file();
        let di_type = di.create_subroutine_type(
            file,
            None,
            &[],
            DIFlags::PUBLIC,
        );
        let subprogram = di.create_function(
            file.as_debug_info_scope(),
            name,
            None,
            file,
            line,
            di_type,
            true,
            true,
            line,
            DIFlags::PUBLIC,
            false,
        );
        fv.set_subprogram(subprogram);
        self.di_scope_stack.push(subprogram.as_debug_info_scope());
    }

    /// Pop the debug scope (call at end of function compilation).
    pub(crate) fn pop_debug_scope(&mut self) {
        if self.debug {
            self.di_scope_stack.pop();
        }
    }

    /// Emit a debug location for the current instruction position.
    pub(crate) fn set_debug_location(&self, line: u32, col: u32) {
        if !self.debug {
            return;
        }
        if let Some(scope) = self.di_scope_stack.last() {
            let di = self.di_builder.as_ref().unwrap();
            let loc = di.create_debug_location(self.ctx, line, col, *scope, None);
            self.bld.set_current_debug_location(loc);
        }
    }
}

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn setup_target(&self) -> Result<(), String> {
        let tm = self.target_machine(OptimizationLevel::None)?;
        self.module.set_triple(&TargetMachine::get_default_triple());
        self.module
            .set_data_layout(&tm.get_target_data().get_data_layout());
        Ok(())
    }

    pub(crate) fn target_machine(&self, opt: OptimizationLevel) -> Result<TargetMachine, String> {
        Target::initialize_native(&InitializationConfig::default()).map_err(|e| e.to_string())?;
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

    pub(crate) fn attr(&self, name: &str) -> Attribute {
        self.ctx
            .create_enum_attribute(Attribute::get_named_enum_kind_id(name), 0)
    }

    pub(crate) fn tag_fn(&self, fv: FunctionValue<'ctx>) {
        self.tag_fn_inner(fv, true);
    }

    pub(crate) fn tag_fn_noreturn_ok(&self, fv: FunctionValue<'ctx>) {
        self.tag_fn_inner(fv, false);
    }

    fn tag_fn_inner(&self, fv: FunctionValue<'ctx>, will_return: bool) {
        for a in ["nounwind", "nosync", "nofree", "mustprogress"] {
            fv.add_attribute(AttributeLoc::Function, self.attr(a));
        }
        if will_return {
            fv.add_attribute(AttributeLoc::Function, self.attr("willreturn"));
            fv.add_attribute(AttributeLoc::Function, self.attr("norecurse"));
        }
    }

    pub(crate) fn set_var(&mut self, name: &str, ptr: PointerValue<'ctx>, ty: Type) {
        self.vars
            .last_mut()
            .unwrap()
            .insert(name.to_string(), (ptr, ty));
    }

    pub(crate) fn find_var(&self, name: &str) -> Option<&(PointerValue<'ctx>, Type)> {
        self.vars.iter().rev().find_map(|s| s.get(name))
    }

    pub(crate) fn load_var(&mut self, name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some((ptr, ty)) = self.find_var(name).cloned() {
            return Ok(b!(self.bld.build_load(self.llvm_ty(&ty), ptr, name)));
        }
        if let Some(fv) = self.module.get_function(name) {
            return Ok(fv.as_global_value().as_pointer_value().into());
        }
        Err(format!("undefined: {name}"))
    }

    pub(crate) fn entry_alloca(&self, ty: BasicTypeEnum<'ctx>, name: &str) -> PointerValue<'ctx> {
        let entry = self.cur_fn.unwrap().get_first_basic_block().unwrap();
        let tmp = self.ctx.create_builder();
        match entry.get_first_instruction() {
            Some(inst) => tmp.position_before(&inst),
            None => tmp.position_at_end(entry),
        }
        tmp.build_alloca(ty, name).unwrap()
    }

    pub(crate) fn entry_alloca_aligned(
        &self,
        ty: BasicTypeEnum<'ctx>,
        name: &str,
        align: u32,
    ) -> PointerValue<'ctx> {
        let ptr = self.entry_alloca(ty, name);
        ptr.as_instruction_value()
            .unwrap()
            .set_alignment(align)
            .unwrap();
        ptr
    }

    pub(crate) fn no_term(&self) -> bool {
        self.bld
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none()
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

    pub(crate) fn ensure_malloc(&mut self) -> FunctionValue<'ctx> {
        self.module.get_function("malloc").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i64t = self.ctx.i64_type();
            let ft = ptr_ty.fn_type(&[i64t.into()], false);
            self.module
                .add_function("malloc", ft, Some(Linkage::External))
        })
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
}
