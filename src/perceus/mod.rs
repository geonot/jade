use std::collections::HashMap;

use crate::ast::Span;
use crate::hir::DefId;
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
