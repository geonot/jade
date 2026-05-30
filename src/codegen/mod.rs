mod actors;
mod arith;
mod builtins;
mod call;
mod clone;
mod conversions;
mod coroutines;
mod decl;
mod drop;
mod fmt;
mod lambda;
mod map;
pub mod mir_codegen;
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
    DICompileUnit, DIScope, DWARFEmissionKind, DWARFSourceLanguage, DebugInfoBuilder,
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

    pub(crate) var_shadows: Vec<(String, Option<(PointerValue<'ctx>, Type)>)>,

    pub(crate) var_scope_markers: Vec<usize>,
    pub(crate) fns: IndexMap<Symbol, (FunctionValue<'ctx>, Vec<Type>, Type)>,
    pub(crate) structs: IndexMap<Symbol, Vec<(String, Type)>>,
    pub(crate) struct_defaults: IndexMap<Symbol, IndexMap<Symbol, hir::Expr>>,
    pub(crate) struct_layouts: IndexMap<Symbol, crate::ast::LayoutAttrs>,
    pub(crate) enums: IndexMap<Symbol, Vec<(String, Vec<Type>)>>,
    pub(crate) variant_tags: IndexMap<Symbol, (String, u32)>,
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

    pub needs_ssl: bool,

    pub needs_sqlite: bool,
    pub(crate) globals: IndexMap<Symbol, (inkwell::values::GlobalValue<'ctx>, Type)>,
    pub(crate) fast_math_flags: u32,

    pub(crate) alloca_bld: Builder<'ctx>,

    pub target_triple: Option<String>,

    pub target_cpu: Option<String>,

    pub target_features: Option<String>,

    pub standalone: bool,

    tbaa_kind_id: u32,

    tbaa_root: Option<inkwell::values::MetadataValue<'ctx>>,

    pub(crate) empty_vec_growth_floor: u64,

    pub(crate) value_map: std::collections::HashMap<mir::ValueId, BasicValueEnum<'ctx>>,

    pub(crate) block_map: std::collections::HashMap<mir::BlockId, BasicBlock<'ctx>>,

    pub(crate) pending_phis: Vec<PendingPhi<'ctx>>,

    pub(crate) var_allocs: std::collections::HashMap<Symbol, (PointerValue<'ctx>, Type)>,

    pub(crate) value_types: std::collections::HashMap<mir::ValueId, Type>,

    pub(crate) select_data_bufs: std::collections::HashMap<mir::ValueId, Vec<PointerValue<'ctx>>>,

    pub(crate) self_allocs: std::collections::HashMap<mir::ValueId, PointerValue<'ctx>>,

    pub(crate) self_alloc_types: std::collections::HashMap<mir::ValueId, BasicTypeEnum<'ctx>>,

    pub(crate) block_exit_map: std::collections::HashMap<mir::BlockId, BasicBlock<'ctx>>,

    pub(crate) migration_fns: Vec<FunctionValue<'ctx>>,

    pub(crate) global_init_fn: Option<FunctionValue<'ctx>>,

    pub(crate) vec_growth_floor_by_value: std::collections::HashMap<mir::ValueId, u64>,

    pub(crate) current_perceus_meta: mir::PerceusMeta,

    pub(crate) current_reuse_slots: std::collections::HashMap<u32, PointerValue<'ctx>>,

    pub(crate) current_reuse_alloca_slots: std::collections::HashMap<u32, PointerValue<'ctx>>,

    pub(crate) current_alloc_dest: Option<mir::ValueId>,

    /// True while `compile_mir_fn` is emitting a coroutine body (a MIR
    /// `Function` with `is_coroutine == true`). Consulted by `emit_terminator`
    /// so a `Return` marks the generator done and suspends instead of emitting
    /// a normal LLVM `ret`.
    pub(crate) cur_fn_is_coroutine: bool,
}

pub(crate) struct PendingPhi<'ctx> {
    pub phi: inkwell::values::PhiValue<'ctx>,
    pub incoming: Vec<(mir::BlockId, mir::ValueId)>,
}

mod debug;
mod supervisor;
mod support;
