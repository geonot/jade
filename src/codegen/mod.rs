mod builtins;
mod call;
mod decl;
mod expr;
mod stmt;
mod strings;
mod types;

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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Compiler state
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

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
}

pub(crate) struct LoopCtx<'ctx> {
    pub continue_bb: BasicBlock<'ctx>,
    pub break_bb: BasicBlock<'ctx>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public API
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

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
            true, // allow_unresolved
            DWARFSourceLanguage::C, // closest match for Jade's ABI
            filename,
            ".",
            "jadec",
            false, // is_optimized (set at emit time)
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

    pub fn emit_object(&self, path: &Path, opt: OptimizationLevel) -> Result<(), String> {
        let passes = match opt {
            OptimizationLevel::None => "default<O0>",
            OptimizationLevel::Less => "default<O1>",
            OptimizationLevel::Default => "default<O2>",
            OptimizationLevel::Aggressive => "default<O3>",
        };
        let tm = self.target_machine(opt)?;
        let pb = PassBuilderOptions::create();
        pb.set_loop_vectorization(matches!(
            opt,
            OptimizationLevel::Default | OptimizationLevel::Aggressive
        ));
        pb.set_loop_slp_vectorization(matches!(
            opt,
            OptimizationLevel::Default | OptimizationLevel::Aggressive
        ));
        pb.set_loop_unrolling(matches!(
            opt,
            OptimizationLevel::Default | OptimizationLevel::Aggressive
        ));
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

        // Phase 1: declare all types, enums, externs, error defs, functions
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
        for f in &prog.fns {
            self.declare_fn(f)?;
        }

        // Phase 2: compile all function and method bodies
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

        self.finalize_debug();
        self.module.verify().map_err(|e| e.to_string())
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DWARF debug info
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

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
            None, // return type
            &[],  // parameter types
            DIFlags::PUBLIC,
        );
        let subprogram = di.create_function(
            file.as_debug_info_scope(),
            name,
            None,
            file,
            line,
            di_type,
            true,   // is_local_to_unit
            true,   // is_definition
            line,   // scope_line
            DIFlags::PUBLIC,
            false,  // is_optimized
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Internal helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

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
