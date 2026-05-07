//! Codegen root: the `Compiler` struct and shared LLVM utilities. The `mir_codegen/`
//! submodule provides additional `impl` blocks that consume MIR; sibling files in this
//! directory provide helpers that operate on HIR-shaped data (actor/coroutine/closure
//! definitions, struct/enum schemas, etc.) — both groups extend the same `Compiler<'ctx>`.

mod actors;
mod arith;
mod builtins;
mod call;
mod channels;
mod conversions;
mod coroutines;
mod decl;
mod drop;
mod expr;
mod fmt;
mod lambda;
mod loops;
mod map;
pub mod mir_codegen;
mod pattern_match;
mod rc;
mod set;
mod stmt;
mod store_filter;
mod store_ops;
mod stores;
mod string_ops;
mod string_transform;
mod strings;
mod types;
mod vec;

use crate::intern::Symbol;
use std::collections::HashSet;
use indexmap::IndexMap;
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

use inkwell::debug_info::{
    DICompileUnit, DIFlags, DIFlagsConstants, DIScope, DWARFEmissionKind,
    DWARFSourceLanguage, DebugInfoBuilder,
};

use crate::hir;
use crate::mir;
use crate::perceus::PerceusHints;
use crate::types::Type;

macro_rules! b {
    ($e:expr) => {
        $e.map_err(|e| e.to_string())?
    };
}
pub(crate) use b;

/// Internal Compiler Error: replaces `unwrap()` with a diagnostic message
/// that includes source location for debuggability.
macro_rules! ice {
    ($opt:expr, $msg:expr) => {
        $opt.unwrap_or_else(|| {
            panic!(
                "internal compiler error: {} (at {}:{}:{})",
                $msg,
                file!(),
                line!(),
                column!()
            )
        })
    };
}
pub(crate) use ice;

/// Look up a runtime/builtin function by name; panic with an ICE diagnostic
/// if it has not been declared yet. Use only for symbols that codegen
/// guarantees to be present (declared in `runtime/jade_rt.h` or via
/// `declare_runtime_*`).
#[inline]
pub(crate) fn fn_or_die<'ctx>(
    module: &inkwell::module::Module<'ctx>,
    name: &str,
) -> inkwell::values::FunctionValue<'ctx> {
    module.get_function(name).unwrap_or_else(|| {
        panic!("internal compiler error: runtime function `{name}` not declared")
    })
}

pub struct Compiler<'ctx> {
    pub(crate) ctx: &'ctx Context,
    pub(crate) module: Module<'ctx>,
    pub(crate) bld: Builder<'ctx>,
    pub(crate) cur_fn: Option<FunctionValue<'ctx>>,
    pub(crate) vars: IndexMap<Symbol, (PointerValue<'ctx>, Type)>,
    /// Shadow stack for scope-based variable shadowing.
    /// Each entry records (name, previous_value) to restore on scope pop.
    pub(crate) var_shadows: Vec<(String, Option<(PointerValue<'ctx>, Type)>)>,
    /// Scope markers: indices into var_shadows where each scope starts.
    pub(crate) var_scope_markers: Vec<usize>,
    pub(crate) fns: IndexMap<Symbol, (FunctionValue<'ctx>, Vec<Type>, Type)>,
    pub(crate) structs: IndexMap<Symbol, Vec<(String, Type)>>,
    pub(crate) struct_defaults: IndexMap<Symbol, IndexMap<Symbol, hir::Expr>>,
    pub(crate) struct_layouts: IndexMap<Symbol, crate::ast::LayoutAttrs>,
    pub(crate) enums: IndexMap<Symbol, Vec<(String, Vec<Type>)>>,
    pub(crate) variant_tags: IndexMap<Symbol, (String, u32)>,
    pub(crate) loop_stack: Vec<LoopCtx<'ctx>>,
    pub(crate) source: String,
    pub(crate) hints: PerceusHints,
    pub(crate) lib_mode: bool,
    pub(crate) debug: bool,
    pub(crate) di_builder: Option<DebugInfoBuilder<'ctx>>,
    pub(crate) di_compile_unit: Option<DICompileUnit<'ctx>>,
    pub(crate) di_scope_stack: Vec<DIScope<'ctx>>,
    pub(crate) filename: String,
    pub(crate) store_defs: IndexMap<Symbol, hir::StoreDef>,
    pub(crate) actor_defs: IndexMap<Symbol, hir::ActorDef>,
    pub(crate) vtables: IndexMap<(String, String), inkwell::values::GlobalValue<'ctx>>,
    pub(crate) trait_method_order: IndexMap<Symbol, Vec<String>>,
    pub needs_runtime: bool,
    /// Program uses TLS or crypto functions requiring OpenSSL.
    pub needs_ssl: bool,
    /// Program uses SQLite functions requiring libsqlite3.
    pub needs_sqlite: bool,
    pub(crate) globals: IndexMap<Symbol, (inkwell::values::GlobalValue<'ctx>, Type)>,
    pub(crate) fast_math_flags: u32,
    /// Reuse tokens: DefId → saved heap pointer for Perceus reuse.
    /// When a drop is skipped for reuse, the pointer is stashed here
    /// so the next compatible allocation can reuse it instead of malloc.
    pub(crate) reuse_tokens: IndexMap<hir::DefId, PointerValue<'ctx>>,
    /// Cached builder used exclusively for entry-block alloca insertion.
    pub(crate) alloca_bld: Builder<'ctx>,
    /// Set of variable names declared with `atomic` keyword.
    pub(crate) atomic_vars: HashSet<Symbol>,
    /// Override target triple for cross-compilation (None = host).
    pub target_triple: Option<String>,
    /// Override CPU name for cross-compilation.
    pub target_cpu: Option<String>,
    /// Override CPU features for cross-compilation.
    pub target_features: Option<String>,
    /// Standalone mode: no runtime, no libc dependency.
    pub standalone: bool,
    /// TBAA metadata kind ID (cached).
    tbaa_kind_id: u32,
    /// TBAA root node for type-based alias analysis.
    tbaa_root: Option<inkwell::values::MetadataValue<'ctx>>,
    /// Per-compilation floor for first growth of empty vectors.
    /// Tuned from MIR VecNew/VecPush patterns in the current program.
    pub(crate) empty_vec_growth_floor: u64,

    // ── MIR-codegen working state (formerly on the separate MirCodegen struct) ──
    /// MIR ValueId → LLVM value.
    pub(crate) value_map: std::collections::HashMap<mir::ValueId, BasicValueEnum<'ctx>>,
    /// MIR BlockId → LLVM basic block (per-function, rebuilt each time).
    pub(crate) block_map: std::collections::HashMap<mir::BlockId, BasicBlock<'ctx>>,
    /// Phi nodes that need back-patching after all blocks are emitted.
    pub(crate) pending_phis: Vec<PendingPhi<'ctx>>,
    /// MIR ValueId → variable alloca (for Store/Load variable pairs).
    pub(crate) var_allocs: std::collections::HashMap<Symbol, (PointerValue<'ctx>, Type)>,
    /// MIR ValueId → Jade type (for FieldGet struct type resolution on parameters).
    pub(crate) value_types: std::collections::HashMap<mir::ValueId, Type>,
    /// Coroutine/generator bodies extracted from HIR, keyed by name.
    pub(crate) coro_bodies: std::collections::HashMap<Symbol, Vec<hir::Stmt>>,
    /// Select data buffers: select_val ValueId → Vec<PointerValue> (one per arm).
    pub(crate) select_data_bufs: std::collections::HashMap<mir::ValueId, Vec<PointerValue<'ctx>>>,
    /// MIR ValueId → alloca for structs that were passed by pointer to methods
    /// (cached so mutations persist).
    pub(crate) self_allocs: std::collections::HashMap<mir::ValueId, PointerValue<'ctx>>,
    /// MIR ValueId → original LLVM BasicTypeEnum for `self_allocs` entries (for lazy reload).
    pub(crate) self_alloc_types: std::collections::HashMap<mir::ValueId, BasicTypeEnum<'ctx>>,
    /// MIR BlockId → actual LLVM exit block (may differ from `block_map` when
    /// helpers like `string_concat` create intermediate LLVM blocks).
    pub(crate) block_exit_map: std::collections::HashMap<mir::BlockId, BasicBlock<'ctx>>,
    /// Generated migration function values to call from main wrapper.
    pub(crate) migration_fns: Vec<FunctionValue<'ctx>>,
    /// Generated global initializer function to call from main wrapper.
    pub(crate) global_init_fn: Option<FunctionValue<'ctx>>,
    /// Per-function `VecNew(ValueId)` → growth floor used for `VecPush` on that vec.
    pub(crate) vec_growth_floor_by_value: std::collections::HashMap<mir::ValueId, u64>,
}

pub(crate) struct PendingPhi<'ctx> {
    pub phi: inkwell::values::PhiValue<'ctx>,
    pub incoming: Vec<(mir::BlockId, mir::ValueId)>,
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
    pub(crate) fn tbaa_type_node(&self, name: &str) -> Option<inkwell::values::MetadataValue<'ctx>> {
        let root = self.tbaa_root.as_ref()?;
        let name_md = self.ctx.metadata_string(name);
        // TBAA type descriptor: {name, parent, constant_flag=0}
        let zero = self.ctx.i64_type().const_int(0, false);
        Some(self.ctx.metadata_node(&[name_md.into(), (*root).into(), zero.into()]))
    }

    /// Create a TBAA access tag for a load/store and attach it to the instruction.
    pub(crate) fn set_tbaa(&self, inst: inkwell::values::InstructionValue<'ctx>, type_name: &str) {
        if let Some(type_node) = self.tbaa_type_node(type_name) {
            let zero = self.ctx.i64_type().const_int(0, false);
            // TBAA access tag: {base_type, access_type, offset}
            let access_tag = self.ctx.metadata_node(&[type_node.into(), type_node.into(), zero.into()]);
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

    fn run_optimization_passes(&self, opt: OptimizationLevel) -> Result<TargetMachine, String> {
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
                        let self_ptr = ice!(thunk_fn.get_first_param(), "vtable thunk missing self param").into_pointer_value();
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
                            call_args.push(ice!(thunk_fn.get_nth_param(i), "vtable thunk missing param").into());
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

#[allow(dead_code)]
impl<'ctx> Compiler<'ctx> {
    fn finalize_debug(&self) {
        if let Some(ref di) = self.di_builder {
            di.finalize();
        }
    }

    pub(crate) fn pop_debug_scope(&mut self) {
        if self.debug {
            self.di_scope_stack.pop();
        }
    }

    pub(crate) fn set_debug_location(&self, line: u32, col: u32) {
        if !self.debug {
            return;
        }
        if let Some(scope) = self.di_scope_stack.last() {
            let di = ice!(self.di_builder.as_ref(), "debug info builder not initialized");
            let loc = di.create_debug_location(self.ctx, line, col, *scope, None);
            self.bld.set_current_debug_location(loc);
        }
    }

    /// R15: emit `llvm.dbg.declare` for an alloca'd local so debuggers
    /// (lldb, gdb) can resolve `frame variable <name>`. Uses an opaque
    /// 64-bit basic type as a stand-in DIType — accurate enough for
    /// integers/pointers and gives the variable a name binding.
    /// No-op when debug info is disabled.
    pub(crate) fn attach_dbg_declare(
        &self,
        ptr: PointerValue<'ctx>,
        name: &str,
        line: u32,
    ) {
        if !self.debug {
            return;
        }
        let Some(ref di) = self.di_builder else { return };
        let Some(scope) = self.di_scope_stack.last().copied() else { return };
        let Some(ref cu) = self.di_compile_unit else { return };
        let file = cu.get_file();
        // Use a generic 64-bit unsigned DI type. This is a stand-in:
        // proper per-Type DI metadata is a follow-up. lldb still prints
        // the address and bytes, which is the main thing the user gets.
        let di_ty = di.create_basic_type("__jade_local", 64, 0x07 /* DW_ATE_unsigned */, DIFlags::PUBLIC);
        let Ok(di_ty) = di_ty else { return };
        let var_info = di.create_auto_variable(
            scope,
            name,
            file,
            line,
            di_ty.as_type(),
            true,
            DIFlags::PUBLIC,
            0,
        );
        let loc = di.create_debug_location(self.ctx, line, 1, scope, None);
        let Some(bb) = self.bld.get_insert_block() else { return };
        di.insert_declare_at_end(ptr, Some(var_info), None, loc, bb);
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
    pub(crate) fn compile_const_expr(&self, expr: &hir::Expr) -> Result<BasicValueEnum<'ctx>, String> {
        match &expr.kind {
            hir::ExprKind::Int(n) => Ok(self.int_const(*n, &expr.ty).into()),
            hir::ExprKind::Float(f) => Ok(self.ctx.f64_type().const_float(*f).into()),
            hir::ExprKind::Bool(b) => Ok(self.ctx.bool_type().const_int(*b as u64, false).into()),
            hir::ExprKind::Str(s) => {
                let gv = self.bld.build_global_string_ptr(s, "global_str").map_err(|e| e.to_string())?;
                Ok(gv.as_pointer_value().into())
            }
            _ => Err(format!("global initializer must be a constant expression, got {:?}", expr.kind)),
        }
    }

    pub(crate) fn tag_fn(&self, fv: FunctionValue<'ctx>) {
        self.tag_fn_inner(fv, true);
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
            let deref_attr = self.ctx.create_enum_attribute(
                Attribute::get_named_enum_kind_id("dereferenceable"),
                size,
            );
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
                Some(val) => { self.vars.insert(sym, val); }
                None => { self.vars.swap_remove(&sym); }
            }
        }
    }

    pub(crate) fn load_var(&mut self, name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some((ptr, ty)) = self.find_var(name).cloned() {
            let load = b!(self.bld.build_load(self.llvm_ty(&ty), ptr, name));
            if self.atomic_vars.contains(&Symbol::intern(name)) {
                ice!(load.as_instruction_value(), "atomic load produced non-instruction")
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
        ice!(ptr.as_instruction_value(), "alloca produced non-instruction")
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
        let func = self.module.add_function("jade_xmalloc", ft, Some(Linkage::WeakAny));

        // Define the function body inline: call malloc, abort on NULL
        let entry = self.ctx.append_basic_block(func, "entry");
        let ok_bb = self.ctx.append_basic_block(func, "ok");
        let fail_bb = self.ctx.append_basic_block(func, "fail");

        let saved = self.bld.get_insert_block();
        self.bld.position_at_end(entry);
        let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
            let mft = ptr_ty.fn_type(&[i64t.into()], false);
            self.module.add_function("malloc", mft, Some(Linkage::External))
        });
        let size = ice!(func.get_first_param(), "xmalloc missing size param").into_int_value();
        let raw = ice!(self.bld.build_call(malloc_fn, &[size.into()], "raw")
            .unwrap().try_as_basic_value().basic(), "malloc returned void").into_pointer_value();
        let is_null = self.bld.build_is_null(raw, "is_null").unwrap();
        let size_nonzero = self.bld.build_int_compare(
            inkwell::IntPredicate::UGT, size, i64t.const_int(0, false), "nz"
        ).unwrap();
        let should_abort = self.bld.build_and(is_null, size_nonzero, "oom").unwrap();
        self.bld.build_conditional_branch(should_abort, fail_bb, ok_bb).unwrap();

        self.bld.position_at_end(fail_bb);
        let abort_fn = self.module.get_function("abort").unwrap_or_else(|| {
            let void_ty = self.ctx.void_type();
            let aft = void_ty.fn_type(&[], false);
            self.module.add_function("abort", aft, Some(Linkage::External))
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
        let sup_create = self.module.get_function("jade_sup_create").unwrap_or_else(|| {
            let ft = ptr.fn_type(&[i32t.into()], false);
            self.module.add_function("jade_sup_create", ft, Some(Linkage::External))
        });
        let sup_register = self.module.get_function("jade_sup_register").unwrap_or_else(|| {
            let ft = i64t.fn_type(&[ptr.into(), ptr.into(), ptr.into(), ptr.into()], false);
            self.module.add_function("jade_sup_register", ft, Some(Linkage::External))
        });
        let sup_start_fn = self.module.get_function("jade_sup_start").unwrap_or_else(|| {
            let ft = void.fn_type(&[ptr.into()], false);
            self.module.add_function("jade_sup_start", ft, Some(Linkage::External))
        });
        let sup_rcount = self.module.get_function("jade_sup_restart_count").unwrap_or_else(|| {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module.add_function("jade_sup_restart_count", ft, Some(Linkage::External))
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
        let mut child_info: Vec<(inkwell::values::FunctionValue<'ctx>, inkwell::values::FunctionValue<'ctx>, String)> = Vec::new();
        for child in &sup.children {
            let factory_fv = self.ensure_actor_factory(&child.as_str())?;
            let loop_name = format!("{}_loop", child.as_str());
            let loop_fv = self
                .module
                .get_function(&loop_name)
                .ok_or_else(|| format!("supervisor '{}': child loop '{loop_name}' not found", sup.name))?;
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
            let name_global = self.bld
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
        b!(self.bld.build_conditional_branch(cur2_null, zero_bb, load_bb));
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

        self.fns.insert(start_name.clone().into(), (start_fv, vec![], Type::I64));
        self.fns.insert(rc_name.clone().into(), (rc_fv, vec![], Type::I64));
        Ok(())
    }
}
