use std::collections::HashMap;

use crate::ast::Span;
use crate::hir::{DefId, Program};
use crate::types::Type;

#[derive(Debug, Clone, Default)]
pub struct PerceusHints {
    pub elide_drops: std::collections::HashSet<DefId>,
    pub reuse_candidates: HashMap<DefId, ReuseInfo>,
    pub speculative_reuse: HashMap<DefId, ReuseInfo>,
    pub last_use: HashMap<DefId, Span>,
    pub drop_fusions: Vec<DropFusion>,
    pub fbip_sites: Vec<FbipSite>,
    pub tail_reuse: HashMap<DefId, TailReuseInfo>,
    pub pool_hints: Vec<PoolHint>,
    pub stats: PerceusStats,
}

#[derive(Debug, Clone)]
pub struct ReuseInfo {
    pub released_ty: Type,
    pub allocated_ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct DropFusion {
    pub def_ids: Vec<DefId>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FbipSite {
    pub subject_id: DefId,
    pub subject_ty: Type,
    pub constructed_ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TailReuseInfo {
    pub param_id: DefId,
    pub param_ty: Type,
    pub alloc_ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct PoolHint {
    pub alloc_ty: Type,
    pub size: u64,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct PerceusStats {
    pub drops_elided: u32,
    pub reuse_sites: u32,
    pub speculative_reuse_sites: u32,
    pub fbip_sites: u32,
    pub tail_reuse_sites: u32,
    pub drops_fused: u32,
    pub last_use_tracked: u32,
    pub total_bindings_analyzed: u32,
    pub pool_hints_found: u32,
}

pub mod mir_perceus;

pub struct PerceusPass {
    pub(crate) hints: PerceusHints,
}

impl Default for PerceusPass {
    fn default() -> Self {
        Self::new()
    }
}

impl PerceusPass {
    pub fn new() -> Self {
        Self {
            hints: PerceusHints::default(),
        }
    }

    pub fn optimize(&mut self, _prog: &Program) -> PerceusHints {
        PerceusHints::default()
    }

    pub fn type_layout_size_pub(ty: &Type) -> u64 {
        match ty {
            Type::I8 | Type::U8 | Type::Bool => 1,
            Type::I16 | Type::U16 => 2,
            Type::I32 | Type::U32 | Type::F32 => 4,
            Type::I64 | Type::U64 | Type::F64 => 8,
            Type::Ptr(_) => 8,
            Type::String => 24,
            Type::Void => 0,
            Type::Array(inner, len) => Self::type_layout_size_pub(inner) * (*len as u64),
            Type::Tuple(tys) => tys
                .iter()
                .map(|t| {
                    let sz = Self::type_layout_size_pub(t);
                    (sz + 7) & !7
                })
                .sum(),
            Type::Struct(_, _) => 0,
            Type::Enum(_) => 0,
            Type::Fn(_, _) => 16,
            Type::Param(_) | Type::TypeVar(_) => 0,
            Type::ActorRef(_) => 8,
            Type::Coroutine(_) => 8,
            Type::Vec(_) | Type::Map(_, _) => 24,
            Type::Channel(_) => 8,
            Type::Alias(_, inner) | Type::Newtype(_, inner) => Self::type_layout_size_pub(inner),
            Type::Generator(_) => 8,

            Type::Row(_) => 8,
        }
    }

    pub fn layouts_compatible(a: &Type, b: &Type) -> bool {
        let sa = Self::type_layout_size_pub(a);
        let sb = Self::type_layout_size_pub(b);
        sa > 0 && sa == sb
    }
}
