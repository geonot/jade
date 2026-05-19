//! Codegen root: the `Compiler` struct and shared LLVM utilities. The `mir_codegen/`
//! submodule provides additional `impl` blocks that consume MIR; sibling files in this
//! directory provide helpers that operate on HIR-shaped data (actor/coroutine/closure
//! definitions, struct/enum schemas, etc.) — both groups extend the same `Compiler<'ctx>`.

mod actors;
mod arith;
mod builtins;
mod call;
mod channels;
mod clone;
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
use indexmap::IndexMap;
use std::collections::HashSet;
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
    DICompileUnit, DIFlags, DIFlagsConstants, DIScope, DWARFEmissionKind, DWARFSourceLanguage,
    DebugInfoBuilder,
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
/// guarantees to be present (declared in `runtime/jinn_rt.h` or via
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
    /// MIR ValueId → Jinn type (for FieldGet struct type resolution on parameters).
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
    /// Perceus side-table of the MIR function currently being compiled.
    /// Populated at the top of `compile_mir_fn`; consulted by Drop / VecNew /
    /// StructInit / VariantInit handlers to decide whether to save a
    /// reuse slot (Drop) or consume one (alloc).
    pub(crate) current_perceus_meta: mir::PerceusMeta,
    /// Maps the SSA `ValueId` of a `Drop`ped pointer (in the *current* MIR
    /// function) to the heap pointer that the codegen has stashed into a
    /// reuse slot, so a later `VecNew` in the same function can consume it.
    /// Used for forward-pairing within a single basic block (SSA scope).
    pub(crate) current_reuse_slots: std::collections::HashMap<u32, PointerValue<'ctx>>,
    /// Per-slot **stack alloca** holding the current saved heap pointer at
    /// runtime. Populated lazily on first save/consume of each slot id.
    /// This is the storage that lets loop-body reuse work across iterations
    /// of the dynamic loop (the SSA-scoped HashMap above does not survive
    /// runtime back-edges).
    pub(crate) current_reuse_alloca_slots: std::collections::HashMap<u32, PointerValue<'ctx>>,
    /// MIR ValueId of the current allocation site, so the runtime
    /// alloc helper can consult `current_perceus_meta.reuse_consume`.
    pub(crate) current_alloc_dest: Option<mir::ValueId>,
}

pub(crate) struct PendingPhi<'ctx> {
    pub phi: inkwell::values::PhiValue<'ctx>,
    pub incoming: Vec<(mir::BlockId, mir::ValueId)>,
}

pub(crate) struct LoopCtx<'ctx> {
    pub continue_bb: BasicBlock<'ctx>,
    pub break_bb: BasicBlock<'ctx>,
}

mod debug;
mod supervisor;
mod support;
