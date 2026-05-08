//! Perceus reference-counting optimization pass root.
//!
//! Perceus operates on **MIR** as a sequence of transformation passes that
//! mutate the program in place; see [`mir_perceus::run`]. The driver invokes
//! the MIR pipeline after lowering and reports stats via `--debug-perceus`.
//!
//! The HIR-level analyzer that previously lived here (`analysis.rs`,
//! `uses/`) has been removed: it produced advisory hints keyed by `DefId`
//! that the MIR-codegen path could not consume, so the work was diagnostic
//! at best. Current callers retain the [`PerceusPass`] type as a shim that
//! returns empty hints; new code should call [`mir_perceus::run`] directly
//! after MIR lowering.

use std::collections::HashMap;

use crate::ast::Span;
use crate::hir::{DefId, Program};
use crate::types::Type;

#[derive(Debug, Clone, Default)]
pub struct PerceusHints {
    /// Drop instructions that the elision pass removed. Kept as an empty set
    /// for backward compatibility; the IR no longer contains those drops.
    pub elide_drops: std::collections::HashSet<DefId>,
    pub reuse_candidates: HashMap<DefId, ReuseInfo>,
    pub borrow_to_move: std::collections::HashSet<DefId>,
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
    pub borrows_promoted: u32,
    pub speculative_reuse_sites: u32,
    pub fbip_sites: u32,
    pub tail_reuse_sites: u32,
    pub drops_fused: u32,
    pub last_use_tracked: u32,
    pub total_bindings_analyzed: u32,
    pub pool_hints_found: u32,
}

pub mod mir_perceus;

/// Backward-compat shim. The old HIR-level analysis is gone; this returns an
/// empty `PerceusHints` so existing call sites compile and their stats lines
/// remain quiet (they are now driven by [`mir_perceus::run`]).
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

    /// No-op; the HIR analyzer has been retired in favour of
    /// [`mir_perceus::run`]. Returns an empty `PerceusHints`.
    pub fn optimize(&mut self, _prog: &Program) -> PerceusHints {
        PerceusHints::default()
    }

    /// Compute the layout size in bytes used by Perceus reuse matching.
    /// Public because codegen consults it for slot-size sanity checks.
    pub fn type_layout_size_pub(ty: &Type) -> u64 {
        match ty {
            Type::I8 | Type::U8 | Type::Bool => 1,
            Type::I16 | Type::U16 => 2,
            Type::I32 | Type::U32 | Type::F32 => 4,
            Type::I64 | Type::U64 | Type::F64 => 8,
            Type::Ptr(_) | Type::Rc(_) | Type::Weak(_) => 8,
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
            Type::DynTrait(_) => 16,
            Type::Vec(_) | Type::Map(_, _) | Type::Set(_) => 24,
            Type::PriorityQueue(_) => 24,
            Type::NDArray(inner, dims) => {
                let elem_size = Self::type_layout_size_pub(inner);
                let total: u64 = dims.iter().map(|&d| d as u64).product();
                elem_size * total
            }
            Type::Channel(_) => 8,
            Type::SIMD(inner, lanes) => Self::type_layout_size_pub(inner) * (*lanes as u64),
            Type::Arena => 24,
            Type::Pool => 8,
            Type::Deque(_) => 24,
            Type::Cow(inner) => Self::type_layout_size_pub(inner),
            Type::Alias(_, inner) | Type::Newtype(_, inner) => Self::type_layout_size_pub(inner),
            Type::Generator(_) => 8,
        }
    }

    /// Two types share a layout slot (used by reuse pairing) iff their
    /// underlying allocation sizes match.
    pub fn layouts_compatible(a: &Type, b: &Type) -> bool {
        let inner_a = match a {
            Type::Rc(inner) => inner.as_ref(),
            _ => a,
        };
        let inner_b = match b {
            Type::Rc(inner) => inner.as_ref(),
            _ => b,
        };
        let sa = Self::type_layout_size_pub(inner_a);
        let sb = Self::type_layout_size_pub(inner_b);
        sa > 0 && sa == sb
    }
}
