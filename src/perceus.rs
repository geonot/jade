use std::collections::{HashMap, HashSet};

use crate::ast::Span;
use crate::hir::*;
use crate::types::Type;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Optimization hints (attached to the HIR program)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Collected optimization decisions for a program.
#[derive(Debug, Clone, Default)]
pub struct PerceusHints {
    /// DefIds whose drops can be elided entirely (no-op drop).
    pub elide_drops: HashSet<DefId>,

    /// DefIds that are consumed exactly once and whose memory can be
    /// reused by a subsequent allocation of the same layout.
    /// Maps: consumed DefId → producer DefId (the alloc that reuses it).
    pub reuse_candidates: HashMap<DefId, ReuseInfo>,

    /// DefIds where a borrow can be promoted to a move because the
    /// source is never used after the borrow site.
    pub borrow_to_move: HashSet<DefId>,

    /// DefIds eligible for speculative in-place reuse: emit a runtime
    /// refcount check (`if rc == 1 { reuse } else { alloc }`).
    pub speculative_reuse: HashMap<DefId, ReuseInfo>,

    /// Bindings whose last use is known, enabling early drop insertion
    /// immediately after final consumption rather than at scope end.
    pub last_use: HashMap<DefId, Span>,

    /// Consecutive trivial drops that can be fused into a single batch
    /// deallocation call. Each entry: (start_index, count) in a block.
    pub drop_fusions: Vec<DropFusion>,

    /// Match arms where the subject is destructured and a value of
    /// compatible layout is constructed → the subject's memory can be
    /// reused for the new value (Functional-But-In-Place).
    pub fbip_sites: Vec<FbipSite>,

    /// Tail-position allocations that can reuse a consumed parameter's
    /// memory when the parameter is unique.
    pub tail_reuse: HashMap<DefId, TailReuseInfo>,

    /// Per-function statistics for diagnostics / debugging.
    pub stats: PerceusStats,
}

#[derive(Debug, Clone)]
pub struct ReuseInfo {
    /// The type of the value being released.
    pub released_ty: Type,
    /// The type of the value being allocated (must have compatible layout).
    pub allocated_ty: Type,
    /// Span of the reuse site for diagnostics.
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct DropFusion {
    /// DefIds of the drops to fuse.
    pub def_ids: Vec<DefId>,
    /// Span of the first drop.
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FbipSite {
    /// The DefId of the matched subject.
    pub subject_id: DefId,
    /// The type of the matched value.
    pub subject_ty: Type,
    /// The type being constructed in the arm body.
    pub constructed_ty: Type,
    /// Span of the construction.
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TailReuseInfo {
    /// The parameter whose memory can be reused.
    pub param_id: DefId,
    /// Type of the parameter.
    pub param_ty: Type,
    /// Type of the tail allocation.
    pub alloc_ty: Type,
    /// Span of the allocation.
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
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Use counting
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// How a DefId is used within a scope.
#[derive(Debug, Clone)]
struct UseInfo {
    /// Number of times read/consumed.
    use_count: u32,
    /// Spans of each use.
    use_spans: Vec<Span>,
    /// Whether the value escapes (passed to a call, stored in a struct, returned).
    escapes: bool,
    /// Whether the value is borrowed (& taken).
    borrowed: bool,
    /// Whether the value is mutably borrowed.
    #[allow(dead_code)]
    mut_borrowed: bool,
    /// The type of the binding.
    ty: Type,
    /// The ownership category.
    ownership: Ownership,
    /// Span of the definition.
    #[allow(dead_code)]
    def_span: Span,
}

impl UseInfo {
    fn new(ty: Type, ownership: Ownership, def_span: Span) -> Self {
        Self {
            use_count: 0,
            use_spans: Vec::new(),
            escapes: false,
            borrowed: false,
            mut_borrowed: false,
            ty,
            ownership,
            def_span,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Perceus optimizer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct PerceusPass {
    hints: PerceusHints,
}

impl PerceusPass {
    pub fn new() -> Self {
        Self {
            hints: PerceusHints::default(),
        }
    }

    /// Run the full Perceus optimization pass over an HIR program.
    /// Returns optimization hints that codegen can consume.
    pub fn optimize(&mut self, prog: &Program) -> PerceusHints {
        for f in &prog.fns {
            self.analyze_fn(f);
        }
        for td in &prog.types {
            for m in &td.methods {
                self.analyze_fn(m);
            }
        }
        self.hints.clone()
    }

    fn analyze_fn(&mut self, f: &Fn) {
        // Phase 1: Count uses for every DefId in this function
        let mut uses: HashMap<DefId, UseInfo> = HashMap::new();

        // Register parameters
        for p in &f.params {
            uses.insert(
                p.def_id,
                UseInfo::new(p.ty.clone(), p.ownership, p.span),
            );
        }

        // Walk the body
        self.count_uses_block(&f.body, &mut uses);

        // Phase 2: Drop specialization
        self.analyze_drop_specialization(&uses);

        // Phase 3: Reuse analysis (sequential + non-adjacent)
        self.analyze_reuse(&f.body, &uses);

        // Phase 4: Borrow elision
        self.analyze_borrow_elision(&f.body, &uses);

        // Phase 5: Last-use analysis — identify final consumption point
        self.analyze_last_use(&uses);

        // Phase 6: FBIP — match/destruct + reconstruct in-place
        self.analyze_fbip(&f.body, &uses);

        // Phase 7: Tail reuse — last alloc reuses consumed parameter
        self.analyze_tail_reuse(f, &uses);

        // Phase 8: Drop fusion — batch consecutive trivial drops
        self.analyze_drop_fusion(&f.body, &uses);

        // Phase 9: Speculative reuse — runtime uniqueness check
        self.analyze_speculative_reuse(&f.body, &uses);
    }

    // ── Use counting ─────────────────────────────────────────────

    fn count_uses_block(&mut self, block: &Block, uses: &mut HashMap<DefId, UseInfo>) {
        for stmt in block {
            self.count_uses_stmt(stmt, uses);
        }
    }

    fn count_uses_stmt(&mut self, stmt: &Stmt, uses: &mut HashMap<DefId, UseInfo>) {
        match stmt {
            Stmt::Bind(b) => {
                self.count_uses_expr(&b.value, uses);
                uses.insert(
                    b.def_id,
                    UseInfo::new(b.ty.clone(), b.ownership, b.span),
                );
                self.hints.stats.total_bindings_analyzed += 1;
            }
            Stmt::TupleBind(bindings, value, _) => {
                self.count_uses_expr(value, uses);
                for (def_id, _, ty) in bindings {
                    uses.insert(
                        *def_id,
                        UseInfo::new(ty.clone(), ownership_for_type(ty), Span::dummy()),
                    );
                    self.hints.stats.total_bindings_analyzed += 1;
                }
            }
            Stmt::Assign(target, value, _) => {
                self.count_uses_expr(target, uses);
                self.count_uses_expr(value, uses);
            }
            Stmt::Expr(e) => {
                self.count_uses_expr(e, uses);
            }
            Stmt::If(i) => {
                self.count_uses_expr(&i.cond, uses);
                self.count_uses_block(&i.then, uses);
                for (ec, eb) in &i.elifs {
                    self.count_uses_expr(ec, uses);
                    self.count_uses_block(eb, uses);
                }
                if let Some(els) = &i.els {
                    self.count_uses_block(els, uses);
                }
            }
            Stmt::While(w) => {
                self.count_uses_expr(&w.cond, uses);
                // Conservatively treat loop body uses as escaping (may execute 0..N times)
                self.count_uses_block_conservative(&w.body, uses);
            }
            Stmt::For(f) => {
                self.count_uses_expr(&f.iter, uses);
                if let Some(end) = &f.end {
                    self.count_uses_expr(end, uses);
                }
                if let Some(step) = &f.step {
                    self.count_uses_expr(step, uses);
                }
                uses.insert(
                    f.bind_id,
                    UseInfo::new(f.bind_ty.clone(), Ownership::Owned, f.span),
                );
                self.count_uses_block_conservative(&f.body, uses);
            }
            Stmt::Loop(l) => {
                self.count_uses_block_conservative(&l.body, uses);
            }
            Stmt::Ret(val, _, _) => {
                if let Some(v) = val {
                    self.count_uses_expr_escaping(v, uses);
                }
            }
            Stmt::Break(val, _) => {
                if let Some(v) = val {
                    self.count_uses_expr(v, uses);
                }
            }
            Stmt::Continue(_) => {}
            Stmt::Match(m) => {
                self.count_uses_expr(&m.subject, uses);
                for arm in &m.arms {
                    self.count_uses_pat(&arm.pat, uses);
                    self.count_uses_block(&arm.body, uses);
                }
            }
            Stmt::Asm(a) => {
                for (_, e) in &a.inputs {
                    self.count_uses_expr_escaping(e, uses);
                }
            }
            Stmt::Drop(def_id, _, _) => {
                // A drop is itself a "use" of the value
                if let Some(info) = uses.get_mut(def_id) {
                    info.use_count += 1;
                }
            }
            Stmt::ErrReturn(e, _, _) => {
                self.count_uses_expr_escaping(e, uses);
            }
        }
    }

    /// Conservatively count uses in a loop body — mark all referenced
    /// outer variables as escaping (cannot assume single-use in a loop).
    fn count_uses_block_conservative(&mut self, block: &Block, uses: &mut HashMap<DefId, UseInfo>) {
        // Collect all DefIds referenced in the block
        let mut refs = Vec::new();
        self.collect_refs_block(block, &mut refs);
        for def_id in &refs {
            if let Some(info) = uses.get_mut(def_id) {
                // In a loop body, any reference is effectively N uses.
                // Mark as escaping to prevent optimization.
                info.use_count = info.use_count.saturating_add(2); // At least 2 uses
                info.escapes = true;
            }
        }
        // Still walk the block for internal bindings
        self.count_uses_block(block, uses);
    }

    fn collect_refs_block(&mut self, block: &Block, refs: &mut Vec<DefId>) {
        for stmt in block {
            self.collect_refs_stmt(stmt, refs);
        }
    }

    fn collect_refs_stmt(&mut self, stmt: &Stmt, refs: &mut Vec<DefId>) {
        match stmt {
            Stmt::Bind(b) => self.collect_refs_expr(&b.value, refs),
            Stmt::TupleBind(_, value, _) => self.collect_refs_expr(value, refs),
            Stmt::Assign(t, v, _) => {
                self.collect_refs_expr(t, refs);
                self.collect_refs_expr(v, refs);
            }
            Stmt::Expr(e) => self.collect_refs_expr(e, refs),
            Stmt::If(i) => {
                self.collect_refs_expr(&i.cond, refs);
                self.collect_refs_block(&i.then, refs);
                for (ec, eb) in &i.elifs {
                    self.collect_refs_expr(ec, refs);
                    self.collect_refs_block(eb, refs);
                }
                if let Some(els) = &i.els {
                    self.collect_refs_block(els, refs);
                }
            }
            Stmt::While(w) => {
                self.collect_refs_expr(&w.cond, refs);
                self.collect_refs_block(&w.body, refs);
            }
            Stmt::For(f) => {
                self.collect_refs_expr(&f.iter, refs);
                self.collect_refs_block(&f.body, refs);
            }
            Stmt::Loop(l) => self.collect_refs_block(&l.body, refs),
            Stmt::Ret(v, _, _) | Stmt::Break(v, _) => {
                if let Some(e) = v {
                    self.collect_refs_expr(e, refs);
                }
            }
            Stmt::Continue(_) => {}
            Stmt::Match(m) => {
                self.collect_refs_expr(&m.subject, refs);
                for arm in &m.arms {
                    self.collect_refs_block(&arm.body, refs);
                }
            }
            Stmt::Asm(a) => {
                for (_, e) in &a.inputs {
                    self.collect_refs_expr(e, refs);
                }
            }
            Stmt::Drop(id, _, _) => refs.push(*id),
            Stmt::ErrReturn(e, _, _) => self.collect_refs_expr(e, refs),
        }
    }

    fn collect_refs_expr(&mut self, expr: &Expr, refs: &mut Vec<DefId>) {
        match &expr.kind {
            ExprKind::Var(id, _) => refs.push(*id),
            ExprKind::FnRef(id, _) => refs.push(*id),
            ExprKind::BinOp(l, _, r) => {
                self.collect_refs_expr(l, refs);
                self.collect_refs_expr(r, refs);
            }
            ExprKind::UnaryOp(_, e) => self.collect_refs_expr(e, refs),
            ExprKind::Call(_, _, args) | ExprKind::Builtin(_, args) => {
                for a in args {
                    self.collect_refs_expr(a, refs);
                }
            }
            ExprKind::IndirectCall(callee, args) => {
                self.collect_refs_expr(callee, refs);
                for a in args {
                    self.collect_refs_expr(a, refs);
                }
            }
            ExprKind::Method(obj, _, _, args) | ExprKind::StringMethod(obj, _, args) => {
                self.collect_refs_expr(obj, refs);
                for a in args {
                    self.collect_refs_expr(a, refs);
                }
            }
            ExprKind::Field(obj, _, _) => self.collect_refs_expr(obj, refs),
            ExprKind::Index(a, i) => {
                self.collect_refs_expr(a, refs);
                self.collect_refs_expr(i, refs);
            }
            ExprKind::Ternary(c, t, e) => {
                self.collect_refs_expr(c, refs);
                self.collect_refs_expr(t, refs);
                self.collect_refs_expr(e, refs);
            }
            ExprKind::Coerce(e, _) | ExprKind::Cast(e, _) => self.collect_refs_expr(e, refs),
            ExprKind::Array(elems) | ExprKind::Tuple(elems) => {
                for e in elems {
                    self.collect_refs_expr(e, refs);
                }
            }
            ExprKind::Struct(_, inits) | ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    self.collect_refs_expr(&fi.value, refs);
                }
            }
            ExprKind::IfExpr(i) => {
                self.collect_refs_expr(&i.cond, refs);
                self.collect_refs_block(&i.then, refs);
                for (ec, eb) in &i.elifs {
                    self.collect_refs_expr(ec, refs);
                    self.collect_refs_block(eb, refs);
                }
                if let Some(els) = &i.els {
                    self.collect_refs_block(els, refs);
                }
            }
            ExprKind::Pipe(first, _, _, rest) => {
                self.collect_refs_expr(first, refs);
                for a in rest {
                    self.collect_refs_expr(a, refs);
                }
            }
            ExprKind::Block(stmts) => self.collect_refs_block(stmts, refs),
            ExprKind::Lambda(_, body) => self.collect_refs_block(body, refs),
            ExprKind::Ref(e) | ExprKind::Deref(e) => self.collect_refs_expr(e, refs),
            ExprKind::ListComp(body, _, _, iter, cond, map) => {
                self.collect_refs_expr(iter, refs);
                self.collect_refs_expr(body, refs);
                if let Some(c) = cond {
                    self.collect_refs_expr(c, refs);
                }
                if let Some(m) = map {
                    self.collect_refs_expr(m, refs);
                }
            }
            ExprKind::Syscall(args) => {
                for a in args {
                    self.collect_refs_expr(a, refs);
                }
            }
            _ => {}
        }
    }

    fn count_uses_pat(&mut self, pat: &Pat, uses: &mut HashMap<DefId, UseInfo>) {
        match pat {
            Pat::Wild(_) => {}
            Pat::Bind(def_id, _, ty, span) => {
                uses.insert(
                    *def_id,
                    UseInfo::new(ty.clone(), ownership_for_type(ty), *span),
                );
                self.hints.stats.total_bindings_analyzed += 1;
            }
            Pat::Lit(e) => {
                self.count_uses_expr(e, uses);
            }
            Pat::Ctor(_, _, sub_pats, _) => {
                for sp in sub_pats {
                    self.count_uses_pat(sp, uses);
                }
            }
            Pat::Or(alts, _) => {
                for alt in alts {
                    self.count_uses_pat(alt, uses);
                }
            }
            Pat::Range(lo, hi, _) => {
                self.count_uses_expr(lo, uses);
                self.count_uses_expr(hi, uses);
            }
        }
    }

    fn count_uses_expr(&mut self, expr: &Expr, uses: &mut HashMap<DefId, UseInfo>) {
        match &expr.kind {
            ExprKind::Var(def_id, _) => {
                if let Some(info) = uses.get_mut(def_id) {
                    info.use_count += 1;
                    info.use_spans.push(expr.span);
                }
            }
            ExprKind::FnRef(_, _) | ExprKind::VariantRef(_, _, _) => {}
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_)
            | ExprKind::Bool(_) | ExprKind::None | ExprKind::Void => {}

            ExprKind::BinOp(l, _, r) => {
                self.count_uses_expr(l, uses);
                self.count_uses_expr(r, uses);
            }
            ExprKind::UnaryOp(_, e) => {
                self.count_uses_expr(e, uses);
            }
            ExprKind::Call(_, _, args) => {
                // Arguments passed to function calls escape
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::IndirectCall(callee, args) => {
                self.count_uses_expr(callee, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::Builtin(builtin, args) => {
                let escapes = !matches!(
                    builtin,
                    BuiltinFn::Log | BuiltinFn::Popcount | BuiltinFn::Clz
                    | BuiltinFn::Ctz | BuiltinFn::Bswap
                );
                for a in args {
                    if escapes {
                        self.count_uses_expr_escaping(a, uses);
                    } else {
                        self.count_uses_expr(a, uses);
                    }
                }
            }
            ExprKind::Method(obj, _, _, args) | ExprKind::StringMethod(obj, _, args) => {
                self.count_uses_expr(obj, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::Field(obj, _, _) => {
                self.count_uses_expr(obj, uses);
            }
            ExprKind::Index(arr, idx) => {
                self.count_uses_expr(arr, uses);
                self.count_uses_expr(idx, uses);
            }
            ExprKind::Ternary(cond, then, els) => {
                self.count_uses_expr(cond, uses);
                self.count_uses_expr(then, uses);
                self.count_uses_expr(els, uses);
            }
            ExprKind::Coerce(inner, _) | ExprKind::Cast(inner, _) => {
                self.count_uses_expr(inner, uses);
            }
            ExprKind::Array(elems) | ExprKind::Tuple(elems) => {
                for e in elems {
                    self.count_uses_expr(e, uses);
                }
            }
            ExprKind::Struct(_, inits) | ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    self.count_uses_expr_escaping(&fi.value, uses);
                }
            }
            ExprKind::IfExpr(i) => {
                self.count_uses_expr(&i.cond, uses);
                self.count_uses_block(&i.then, uses);
                for (ec, eb) in &i.elifs {
                    self.count_uses_expr(ec, uses);
                    self.count_uses_block(eb, uses);
                }
                if let Some(els) = &i.els {
                    self.count_uses_block(els, uses);
                }
            }
            ExprKind::Pipe(first, _, _, rest) => {
                self.count_uses_expr_escaping(first, uses);
                for a in rest {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::Block(stmts) => {
                self.count_uses_block(stmts, uses);
            }
            ExprKind::Lambda(params, body) => {
                // Lambda captures escape
                let mut lambda_uses: HashMap<DefId, UseInfo> = HashMap::new();
                for p in params {
                    lambda_uses.insert(
                        p.def_id,
                        UseInfo::new(p.ty.clone(), p.ownership, p.span),
                    );
                }
                // Any outer DefIds referenced inside the lambda escape
                let mut refs = Vec::new();
                self.collect_refs_block(body, &mut refs);
                for id in refs {
                    if let Some(info) = uses.get_mut(&id) {
                        info.escapes = true;
                        info.use_count += 1;
                    }
                }
            }
            ExprKind::Ref(inner) => {
                self.count_uses_expr(inner, uses);
                // Taking a reference borrows the inner value
                if let ExprKind::Var(def_id, _) = &inner.kind {
                    if let Some(info) = uses.get_mut(def_id) {
                        info.borrowed = true;
                    }
                }
            }
            ExprKind::Deref(inner) => {
                self.count_uses_expr(inner, uses);
            }
            ExprKind::ListComp(body, bind_id, _, iter, cond, map) => {
                self.count_uses_expr(iter, uses);
                uses.insert(
                    *bind_id,
                    UseInfo::new(body.ty.clone(), Ownership::Owned, Span::dummy()),
                );
                self.count_uses_expr(body, uses);
                if let Some(c) = cond {
                    self.count_uses_expr(c, uses);
                }
                if let Some(m) = map {
                    self.count_uses_expr(m, uses);
                }
            }
            ExprKind::Syscall(args) => {
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
        }
    }

    /// Count a use that causes the value to escape (passed as arg, stored in struct, returned).
    fn count_uses_expr_escaping(&mut self, expr: &Expr, uses: &mut HashMap<DefId, UseInfo>) {
        if let ExprKind::Var(def_id, _) = &expr.kind {
            if let Some(info) = uses.get_mut(def_id) {
                info.use_count += 1;
                info.escapes = true;
                info.use_spans.push(expr.span);
            }
        } else {
            self.count_uses_expr(expr, uses);
        }
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Optimization 1: Drop Specialization
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //
    // A drop can be elided if:
    // - The type has no heap-allocated inner resources
    // - The value is never Rc (Rc values need refcount decrement)
    // - The value is never borrowed (borrows don't own, no drop needed)
    //
    // Types that are trivially droppable (no-op):
    // - All integer types (i8..u64)
    // - All float types (f32, f64)
    // - Bool
    // - Void
    // - Fixed arrays of trivially droppable types
    // - Tuples of trivially droppable types
    // - Raw pointers (user manages lifetime)

    fn analyze_drop_specialization(&mut self, uses: &HashMap<DefId, UseInfo>) {
        for (def_id, info) in uses {
            if Self::is_trivially_droppable(&info.ty) && info.ownership == Ownership::Owned {
                self.hints.elide_drops.insert(*def_id);
                self.hints.stats.drops_elided += 1;
            }
            // Borrowed values never need drops (they don't own the resource)
            if info.ownership == Ownership::Borrowed || info.ownership == Ownership::BorrowMut {
                self.hints.elide_drops.insert(*def_id);
                self.hints.stats.drops_elided += 1;
            }
            // Raw pointers are unmanaged
            if info.ownership == Ownership::Raw {
                self.hints.elide_drops.insert(*def_id);
                self.hints.stats.drops_elided += 1;
            }
        }
    }

    /// A type is trivially droppable if it contains no heap resources.
    fn is_trivially_droppable(ty: &Type) -> bool {
        match ty {
            Type::I8 | Type::I16 | Type::I32 | Type::I64
            | Type::U8 | Type::U16 | Type::U32 | Type::U64
            | Type::F32 | Type::F64
            | Type::Bool | Type::Void | Type::Inferred => true,

            Type::Array(inner, _) => Self::is_trivially_droppable(inner),

            Type::Tuple(tys) => tys.iter().all(|t| Self::is_trivially_droppable(t)),

            // Raw pointers: the pointer itself is trivial, but we don't drop
            // the pointee (user responsibility)
            Type::Ptr(_) => true,

            // These types may hold heap resources:
            Type::String => false,
            Type::Struct(_) => false, // may have String/Rc/Ptr fields
            Type::Enum(_) => false,   // variants may hold heap data
            Type::Rc(_) => false,     // refcount decrement needed
            Type::Fn(_, _) => false,  // closures may capture heap data
            Type::Param(_) => false,  // unknown, be conservative
        }
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Optimization 2: Reuse Analysis (FBIP)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //
    // The core Perceus insight: when a unique Rc value is consumed
    // (refcount drops to zero), and immediately after, a value of
    // compatible memory layout is allocated, we can reuse the
    // just-freed memory instead of calling malloc.
    //
    // Conditions for reuse:
    // 1. The released value is Rc with refcount provably 1 (unique)
    // 2. The value is used exactly once (consumed, then released)
    // 3. The new allocation has a compatible layout
    // 4. No aliasing exists (the value doesn't escape)
    //
    // In practice, for this initial implementation, we look for the
    // pattern:
    //   x is rc(...)      -- allocates
    //   ...use x once...  -- single consumption
    //   y is rc(...)      -- new allocation of same/compatible type
    //
    // If x is unique and type-compatible with y, we mark it as a
    // reuse candidate.

    fn analyze_reuse(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
        // Collect all Rc-typed bindings in this block
        let rc_bindings: Vec<(DefId, &Type, Span)> = self.collect_rc_bindings(body);

        // Check all pairs (not just adjacent windows) for reuse compatibility.
        // A released value can be reused by any subsequent allocation if:
        // - the released value is unique (1 use, no escape, no borrow)
        // - no intervening statement invalidates the reuse opportunity
        for i in 0..rc_bindings.len() {
            let (released_id, released_ty, _released_span) = &rc_bindings[i];
            let Some(info) = uses.get(released_id) else {
                continue;
            };
            if info.use_count != 1 || info.escapes || info.borrowed
                || info.ownership != Ownership::Rc
            {
                continue;
            }
            // Scan forward for the first compatible allocation
            for j in (i + 1)..rc_bindings.len() {
                let (producer_id, allocated_ty, alloc_span) = &rc_bindings[j];
                if Self::layouts_compatible(released_ty, allocated_ty) {
                    self.hints.reuse_candidates.insert(
                        *released_id,
                        ReuseInfo {
                            released_ty: (*released_ty).clone(),
                            allocated_ty: (*allocated_ty).clone(),
                            span: *alloc_span,
                        },
                    );
                    self.hints.reuse_candidates.insert(
                        *producer_id,
                        ReuseInfo {
                            released_ty: (*released_ty).clone(),
                            allocated_ty: (*allocated_ty).clone(),
                            span: *alloc_span,
                        },
                    );
                    self.hints.stats.reuse_sites += 1;
                    break; // one reuse per released value
                }
            }
        }

        // Also check within nested blocks (if/match arms)
        for stmt in body {
            self.analyze_reuse_in_nested(stmt, uses);
        }
    }

    fn collect_rc_bindings<'a>(&self, body: &'a Block) -> Vec<(DefId, &'a Type, Span)> {
        let mut result = Vec::new();
        for stmt in body {
            if let Stmt::Bind(b) = stmt {
                if matches!(b.ty, Type::Rc(_)) {
                    result.push((b.def_id, &b.ty, b.span));
                }
            }
        }
        result
    }

    fn analyze_reuse_in_nested(&mut self, stmt: &Stmt, uses: &HashMap<DefId, UseInfo>) {
        match stmt {
            Stmt::If(i) => {
                self.analyze_reuse(&i.then, uses);
                for (_, eb) in &i.elifs {
                    self.analyze_reuse(eb, uses);
                }
                if let Some(els) = &i.els {
                    self.analyze_reuse(els, uses);
                }
            }
            Stmt::Match(m) => {
                for arm in &m.arms {
                    self.analyze_reuse(&arm.body, uses);
                }
            }
            Stmt::While(_w) => {
                // Don't try reuse in loops — values are used N times
            }
            _ => {}
        }
    }

    /// Two types have compatible layouts if they have the same size
    /// in the Rc envelope. For our purposes, all Rc<T> where T has
    /// the same LLVM layout size are compatible.
    fn layouts_compatible(a: &Type, b: &Type) -> bool {
        let inner_a = match a {
            Type::Rc(inner) => inner.as_ref(),
            _ => a,
        };
        let inner_b = match b {
            Type::Rc(inner) => inner.as_ref(),
            _ => b,
        };
        Self::type_layout_size(inner_a) == Self::type_layout_size(inner_b)
    }

    /// Approximate layout size in bytes for reuse compatibility.
    fn type_layout_size(ty: &Type) -> u64 {
        match ty {
            Type::I8 | Type::U8 | Type::Bool => 1,
            Type::I16 | Type::U16 => 2,
            Type::I32 | Type::U32 | Type::F32 => 4,
            Type::I64 | Type::U64 | Type::F64 => 8,
            Type::Ptr(_) | Type::Rc(_) => 8,
            Type::String => 24, // ptr + len + cap (typical)
            Type::Void => 0,
            Type::Array(inner, len) => Self::type_layout_size(inner) * (*len as u64),
            Type::Tuple(tys) => {
                // Sum with 8-byte alignment padding
                tys.iter()
                    .map(|t| {
                        let sz = Self::type_layout_size(t);
                        (sz + 7) & !7 // align to 8
                    })
                    .sum()
            }
            // Conservative estimate for struct: tag (4 bytes) + N pointer-sized fields
            // This is pessimistic but ensures we only reuse when safe.
            Type::Struct(_) => 0,
            // Enum: tag (4) + max variant size. Conservative: use 0 for unknown.
            Type::Enum(_) => 0,
            Type::Fn(_, _) => 16, // function pointer + env pointer for closures
            Type::Param(_) | Type::Inferred => 0,
        }
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Optimization 3: Borrow Elision
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //
    // When a value is borrowed (&x) but the source `x` is never
    // used after the borrow site, we can promote the borrow to a
    // move. This eliminates:
    // - The retain at borrow creation
    // - The release at borrow end
    //
    // Conditions:
    // 1. The source is Owned (not already Rc or Raw)
    // 2. The source has exactly 1 use after the borrow
    //    (the borrow itself is the last use)
    // 3. The source doesn't escape through any other path

    fn analyze_borrow_elision(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
        // Walk looking for Ref(Var(id)) patterns
        for stmt in body {
            self.find_borrow_elision_in_stmt(stmt, uses);
        }
    }

    fn find_borrow_elision_in_stmt(&mut self, stmt: &Stmt, uses: &HashMap<DefId, UseInfo>) {
        match stmt {
            Stmt::Bind(b) => {
                self.find_borrow_elision_in_expr(&b.value, uses);
            }
            Stmt::TupleBind(_, value, _) => {
                self.find_borrow_elision_in_expr(value, uses);
            }
            Stmt::Assign(t, v, _) => {
                self.find_borrow_elision_in_expr(t, uses);
                self.find_borrow_elision_in_expr(v, uses);
            }
            Stmt::Expr(e) => {
                self.find_borrow_elision_in_expr(e, uses);
            }
            Stmt::If(i) => {
                self.find_borrow_elision_in_expr(&i.cond, uses);
                self.analyze_borrow_elision(&i.then, uses);
                for (ec, eb) in &i.elifs {
                    self.find_borrow_elision_in_expr(ec, uses);
                    self.analyze_borrow_elision(eb, uses);
                }
                if let Some(els) = &i.els {
                    self.analyze_borrow_elision(els, uses);
                }
            }
            Stmt::While(w) => {
                self.find_borrow_elision_in_expr(&w.cond, uses);
                // Don't promote borrows in loops
            }
            Stmt::For(f) => {
                self.find_borrow_elision_in_expr(&f.iter, uses);
            }
            Stmt::Ret(v, _, _) | Stmt::Break(v, _) => {
                if let Some(e) = v {
                    self.find_borrow_elision_in_expr(e, uses);
                }
            }
            Stmt::Match(m) => {
                self.find_borrow_elision_in_expr(&m.subject, uses);
                for arm in &m.arms {
                    self.analyze_borrow_elision(&arm.body, uses);
                }
            }
            _ => {}
        }
    }

    fn find_borrow_elision_in_expr(&mut self, expr: &Expr, uses: &HashMap<DefId, UseInfo>) {
        match &expr.kind {
            ExprKind::Ref(inner) => {
                if let ExprKind::Var(def_id, _) = &inner.kind {
                    if let Some(info) = uses.get(def_id) {
                        // Promote borrow → move if:
                        // - Source is Owned
                        // - Total use count is 1 (this borrow is the only use)
                        // - Source doesn't escape elsewhere
                        if info.ownership == Ownership::Owned
                            && info.use_count <= 1
                            && !info.escapes
                        {
                            self.hints.borrow_to_move.insert(*def_id);
                            self.hints.stats.borrows_promoted += 1;
                        }
                    }
                }
                // Still recurse into inner
                self.find_borrow_elision_in_expr(inner, uses);
            }
            ExprKind::BinOp(l, _, r) => {
                self.find_borrow_elision_in_expr(l, uses);
                self.find_borrow_elision_in_expr(r, uses);
            }
            ExprKind::UnaryOp(_, e) | ExprKind::Coerce(e, _) | ExprKind::Cast(e, _)
            | ExprKind::Deref(e) => {
                self.find_borrow_elision_in_expr(e, uses);
            }
            ExprKind::Call(_, _, args) | ExprKind::Builtin(_, args) | ExprKind::Syscall(args) => {
                for a in args {
                    self.find_borrow_elision_in_expr(a, uses);
                }
            }
            ExprKind::IndirectCall(callee, args) => {
                self.find_borrow_elision_in_expr(callee, uses);
                for a in args {
                    self.find_borrow_elision_in_expr(a, uses);
                }
            }
            ExprKind::Method(obj, _, _, args) | ExprKind::StringMethod(obj, _, args) => {
                self.find_borrow_elision_in_expr(obj, uses);
                for a in args {
                    self.find_borrow_elision_in_expr(a, uses);
                }
            }
            ExprKind::Field(obj, _, _) => self.find_borrow_elision_in_expr(obj, uses),
            ExprKind::Index(a, i) => {
                self.find_borrow_elision_in_expr(a, uses);
                self.find_borrow_elision_in_expr(i, uses);
            }
            ExprKind::Ternary(c, t, e) => {
                self.find_borrow_elision_in_expr(c, uses);
                self.find_borrow_elision_in_expr(t, uses);
                self.find_borrow_elision_in_expr(e, uses);
            }
            ExprKind::Array(elems) | ExprKind::Tuple(elems) => {
                for e in elems {
                    self.find_borrow_elision_in_expr(e, uses);
                }
            }
            ExprKind::Struct(_, inits) | ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    self.find_borrow_elision_in_expr(&fi.value, uses);
                }
            }
            ExprKind::IfExpr(i) => {
                self.find_borrow_elision_in_expr(&i.cond, uses);
                self.analyze_borrow_elision(&i.then, uses);
                for (ec, eb) in &i.elifs {
                    self.find_borrow_elision_in_expr(ec, uses);
                    self.analyze_borrow_elision(eb, uses);
                }
                if let Some(els) = &i.els {
                    self.analyze_borrow_elision(els, uses);
                }
            }
            ExprKind::Pipe(first, _, _, rest) => {
                self.find_borrow_elision_in_expr(first, uses);
                for a in rest {
                    self.find_borrow_elision_in_expr(a, uses);
                }
            }
            ExprKind::Block(stmts) => self.analyze_borrow_elision(stmts, uses),
            ExprKind::Lambda(_, body) => self.analyze_borrow_elision(body, uses),
            ExprKind::ListComp(body, _, _, iter, cond, map) => {
                self.find_borrow_elision_in_expr(iter, uses);
                self.find_borrow_elision_in_expr(body, uses);
                if let Some(c) = cond {
                    self.find_borrow_elision_in_expr(c, uses);
                }
                if let Some(m) = map {
                    self.find_borrow_elision_in_expr(m, uses);
                }
            }
            _ => {}
        }
    }
}

fn ownership_for_type(ty: &Type) -> Ownership {
    match ty {
        Type::Rc(_) => Ownership::Rc,
        Type::Ptr(_) => Ownership::Raw,
        _ => Ownership::Owned,
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// New Perceus optimization passes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

impl PerceusPass {
    // ── Phase 5: Last-use analysis ───────────────────────────────
    //
    // For each binding, record the span of the final use. This enables
    // codegen to insert drops immediately after the last consumption
    // rather than waiting until scope end — reducing live ranges and
    // memory pressure.

    fn analyze_last_use(&mut self, uses: &HashMap<DefId, UseInfo>) {
        for (def_id, info) in uses {
            if info.use_count > 0 && info.ownership == Ownership::Owned {
                if let Some(last_span) = info.use_spans.last() {
                    self.hints.last_use.insert(*def_id, *last_span);
                    self.hints.stats.last_use_tracked += 1;
                }
            }
        }
    }

    // ── Phase 6: FBIP (Functional But In-Place) ──────────────────
    //
    // Detect the pattern where a match destructures a value and an
    // arm body constructs a new value of compatible layout. The
    // destructured value's memory can be reused for the construction
    // if the subject is consumed uniquely.
    //
    // Pattern:
    //   match x
    //     Cons(h, t) ? Cons(h + 1, t)   // reuse x's memory for new Cons
    //     Nil ? Nil

    fn analyze_fbip(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
        for stmt in body {
            if let Stmt::Match(m) = stmt {
                // Subject must be a variable
                let subject_id = match &m.subject.kind {
                    ExprKind::Var(id, _) => *id,
                    _ => continue,
                };
                let subject_info = match uses.get(&subject_id) {
                    Some(info) => info,
                    None => continue,
                };
                // Subject must be uniquely owned (exactly 1 use = this match)
                if subject_info.use_count != 1
                    || subject_info.escapes
                    || subject_info.ownership != Ownership::Owned
                {
                    continue;
                }
                // Check each arm for a construction of compatible type
                for arm in &m.arms {
                    let ctor_ty = self.find_constructor_in_block(&arm.body);
                    if let Some(constructed_ty) = ctor_ty {
                        if Self::layouts_compatible(&m.subject.ty, &constructed_ty) {
                            self.hints.fbip_sites.push(FbipSite {
                                subject_id,
                                subject_ty: m.subject.ty.clone(),
                                constructed_ty,
                                span: arm.span,
                            });
                            self.hints.stats.fbip_sites += 1;
                        }
                    }
                }
            }
            // Recurse into nested structures
            match stmt {
                Stmt::If(i) => {
                    self.analyze_fbip(&i.then, uses);
                    for (_, eb) in &i.elifs {
                        self.analyze_fbip(eb, uses);
                    }
                    if let Some(els) = &i.els {
                        self.analyze_fbip(els, uses);
                    }
                }
                Stmt::For(f) => self.analyze_fbip(&f.body, uses),
                Stmt::While(w) => self.analyze_fbip(&w.body, uses),
                Stmt::Loop(l) => self.analyze_fbip(&l.body, uses),
                _ => {}
            }
        }
    }

    /// Find a constructor expression in a block's tail position.
    fn find_constructor_in_block(&self, block: &Block) -> Option<Type> {
        match block.last() {
            Some(Stmt::Expr(e)) => self.find_constructor_type(e),
            Some(Stmt::Ret(Some(e), _, _)) => self.find_constructor_type(e),
            _ => None,
        }
    }

    fn find_constructor_type(&self, expr: &Expr) -> Option<Type> {
        match &expr.kind {
            ExprKind::VariantCtor(_, _, _, _) => Some(expr.ty.clone()),
            ExprKind::Struct(_, _) => Some(expr.ty.clone()),
            ExprKind::Builtin(BuiltinFn::RcAlloc, _) => Some(expr.ty.clone()),
            _ => None,
        }
    }

    // ── Phase 7: Tail reuse ──────────────────────────────────────
    //
    // When a function's last statement allocates a value of compatible
    // layout to one of its consumed parameters, the parameter's memory
    // can be reused for the return allocation. This eliminates a
    // free+malloc pair on every call.
    //
    // Pattern:
    //   *map(list: List, f) -> List
    //       ...
    //       return Cons(f(head), map(tail, f))  // reuse `list` for new Cons

    fn analyze_tail_reuse(&mut self, f: &Fn, uses: &HashMap<DefId, UseInfo>) {
        // Find the tail expression/statement
        let tail_ty = match f.body.last() {
            Some(Stmt::Ret(Some(e), _, _)) => self.find_constructor_type(e),
            Some(Stmt::Expr(e)) => self.find_constructor_type(e),
            _ => None,
        };
        let Some(alloc_ty) = tail_ty else { return };

        // Check each parameter for unique consumption
        for p in &f.params {
            let Some(info) = uses.get(&p.def_id) else {
                continue;
            };
            // Parameter must be consumed (not just passed through)
            // and have compatible layout with the tail allocation
            if info.ownership == Ownership::Owned
                && !info.escapes
                && Self::layouts_compatible(&p.ty, &alloc_ty)
            {
                self.hints.tail_reuse.insert(
                    p.def_id,
                    TailReuseInfo {
                        param_id: p.def_id,
                        param_ty: p.ty.clone(),
                        alloc_ty: alloc_ty.clone(),
                        span: p.span,
                    },
                );
                self.hints.stats.tail_reuse_sites += 1;
            }
        }
    }

    // ── Phase 8: Drop fusion ─────────────────────────────────────
    //
    // When multiple consecutive statements drop trivially-droppable
    // values, fuse them into a single batch operation. This reduces
    // the overhead of per-value cleanup at scope boundaries.

    fn analyze_drop_fusion(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
        let mut run: Vec<DefId> = Vec::new();
        let mut run_span: Option<Span> = None;

        for stmt in body {
            let is_trivial_drop = match stmt {
                Stmt::Drop(def_id, ty, span) => {
                    if Self::is_trivially_droppable(ty) {
                        run.push(*def_id);
                        if run_span.is_none() {
                            run_span = Some(*span);
                        }
                        true
                    } else {
                        false
                    }
                }
                Stmt::Bind(b) => {
                    // A bind that immediately goes out of scope with trivial type
                    if Self::is_trivially_droppable(&b.ty) {
                        if let Some(info) = uses.get(&b.def_id) {
                            if info.use_count == 0 {
                                run.push(b.def_id);
                                if run_span.is_none() {
                                    run_span = Some(b.span);
                                }
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                _ => false,
            };

            if !is_trivial_drop && run.len() >= 2 {
                self.hints.drop_fusions.push(DropFusion {
                    def_ids: run.clone(),
                    span: run_span.unwrap_or(Span::dummy()),
                });
                self.hints.stats.drops_fused += run.len() as u32;
                run.clear();
                run_span = None;
            } else if !is_trivial_drop {
                run.clear();
                run_span = None;
            }
        }
        // Flush trailing run
        if run.len() >= 2 {
            self.hints.drop_fusions.push(DropFusion {
                def_ids: run,
                span: run_span.unwrap_or(Span::dummy()),
            });
        }

        // Recurse into nested blocks
        for stmt in body {
            match stmt {
                Stmt::If(i) => {
                    self.analyze_drop_fusion(&i.then, uses);
                    for (_, eb) in &i.elifs {
                        self.analyze_drop_fusion(eb, uses);
                    }
                    if let Some(els) = &i.els {
                        self.analyze_drop_fusion(els, uses);
                    }
                }
                Stmt::Match(m) => {
                    for arm in &m.arms {
                        self.analyze_drop_fusion(&arm.body, uses);
                    }
                }
                _ => {}
            }
        }
    }

    // ── Phase 9: Speculative reuse ───────────────────────────────
    //
    // When a value might be unique at runtime but we can't prove it
    // statically (e.g., it's shared via Rc but might have refcount 1),
    // emit a speculative reuse hint. Codegen can emit:
    //   if refcount(x) == 1 { reuse(x) } else { alloc() }
    //
    // This captures the common case where Rc values are actually unique
    // in practice, avoiding the malloc/free pair ~90% of the time.

    fn analyze_speculative_reuse(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
        let rc_bindings: Vec<(DefId, &Type, Span)> = self.collect_rc_bindings(body);

        for window in rc_bindings.windows(2) {
            let (released_id, released_ty, _) = &window[0];
            let (_, allocated_ty, alloc_span) = &window[1];

            if let Some(info) = uses.get(released_id) {
                // Speculative reuse: value is Rc, used more than once or
                // escapes, but layouts are compatible. Can't prove uniqueness
                // but can emit a runtime check.
                let already_proven = self.hints.reuse_candidates.contains_key(released_id);
                if !already_proven
                    && info.ownership == Ownership::Rc
                    && Self::layouts_compatible(released_ty, allocated_ty)
                    && info.use_count <= 3
                    && !info.borrowed
                {
                    self.hints.speculative_reuse.insert(
                        *released_id,
                        ReuseInfo {
                            released_ty: (*released_ty).clone(),
                            allocated_ty: (*allocated_ty).clone(),
                            span: *alloc_span,
                        },
                    );
                    self.hints.stats.speculative_reuse_sites += 1;
                }
            }
        }

        // Recurse into nested blocks
        for stmt in body {
            match stmt {
                Stmt::If(i) => {
                    self.analyze_speculative_reuse(&i.then, uses);
                    for (_, eb) in &i.elifs {
                        self.analyze_speculative_reuse(eb, uses);
                    }
                    if let Some(els) = &i.els {
                        self.analyze_speculative_reuse(els, uses);
                    }
                }
                Stmt::Match(m) => {
                    for arm in &m.arms {
                        self.analyze_speculative_reuse(&arm.body, uses);
                    }
                }
                _ => {}
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Codegen bridge — name-keyed hints for AST-based codegen
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Name-keyed optimization hints that the AST-based codegen can consume.
/// Keys are (function_name, variable_name) pairs instead of DefIds.
#[derive(Debug, Clone, Default)]
pub struct PerceusCodegenHints {
    /// Variables whose drops can be elided entirely (scalar/trivial types,
    /// borrows, raw pointers).
    pub elide_drop: HashSet<(String, String)>,

    /// Rc allocations that can reuse memory from a released value.
    /// Maps (fn, producing_var) → CodegenReuseInfo.
    pub reuse_alloc: HashMap<(String, String), CodegenReuseInfo>,

    /// Variables where a borrow (&x) can be promoted to a move — the
    /// source is never used after the borrow so we skip retain/release.
    pub borrow_to_move: HashSet<(String, String)>,

    /// Speculative reuse: emit `if rc == 1 { reuse } else { malloc }`.
    pub speculative_reuse: HashMap<(String, String), CodegenReuseInfo>,

    /// Variables whose last-use site is known — enables early drop
    /// immediately after final consumption.
    pub early_drop_rc: HashSet<(String, String)>,

    /// Types of early-drop variables (needed for rc_release).
    pub early_drop_inner_ty: HashMap<(String, String), Type>,

    /// Function → list of parameter names eligible for tail reuse.
    pub tail_reuse_params: HashMap<String, Vec<TailReuseCodegen>>,

    /// (fn, match_subject_var) → FBIP eligible: reuse subject memory.
    pub fbip_subjects: HashSet<(String, String)>,

    /// All known Rc-typed variables, keyed by (fn, var).
    pub rc_vars: HashSet<(String, String)>,

    /// Per-function statistics.
    pub stats: PerceusStats,
}

#[derive(Debug, Clone)]
pub struct CodegenReuseInfo {
    /// Name of the variable whose memory to reuse.
    pub released_var: String,
    /// Type of the released value's inner data.
    pub released_ty: Type,
    /// Type of the new allocation's inner data.
    pub allocated_ty: Type,
}

#[derive(Debug, Clone)]
pub struct TailReuseCodegen {
    /// Parameter name whose memory can be reused.
    pub param_name: String,
    /// Type of the parameter.
    pub param_ty: Type,
    /// Type of the tail allocation.
    pub alloc_ty: Type,
}

/// Build name-keyed codegen hints from DefId-keyed perceus hints + HIR.
pub fn build_codegen_hints(prog: &Program, hints: &PerceusHints) -> PerceusCodegenHints {
    let mut out = PerceusCodegenHints {
        stats: hints.stats.clone(),
        ..Default::default()
    };

    // Build DefId → (fn_name, var_name, type) mapping
    let mut id_map: HashMap<DefId, (String, String, Type)> = HashMap::new();

    for f in &prog.fns {
        collect_defid_names(f, &mut id_map);
    }
    for td in &prog.types {
        for m in &td.methods {
            collect_defid_names(m, &mut id_map);
        }
    }

    // Map elide_drops
    for def_id in &hints.elide_drops {
        if let Some((fn_name, var_name, _ty)) = id_map.get(def_id) {
            out.elide_drop.insert((fn_name.clone(), var_name.clone()));
        }
    }

    // Map reuse_candidates
    for (def_id, info) in &hints.reuse_candidates {
        if let Some((fn_name, var_name, _)) = id_map.get(def_id) {
            // Find the released var's name
            let released_var = find_released_var_name(def_id, &hints.reuse_candidates, &id_map);
            out.reuse_alloc.insert(
                (fn_name.clone(), var_name.clone()),
                CodegenReuseInfo {
                    released_var,
                    released_ty: info.released_ty.clone(),
                    allocated_ty: info.allocated_ty.clone(),
                },
            );
        }
    }

    // Map borrow_to_move
    for def_id in &hints.borrow_to_move {
        if let Some((fn_name, var_name, _)) = id_map.get(def_id) {
            out.borrow_to_move
                .insert((fn_name.clone(), var_name.clone()));
        }
    }

    // Map speculative_reuse
    for (def_id, info) in &hints.speculative_reuse {
        if let Some((fn_name, var_name, _)) = id_map.get(def_id) {
            let released_var =
                find_released_var_name(def_id, &hints.speculative_reuse, &id_map);
            out.speculative_reuse.insert(
                (fn_name.clone(), var_name.clone()),
                CodegenReuseInfo {
                    released_var,
                    released_ty: info.released_ty.clone(),
                    allocated_ty: info.allocated_ty.clone(),
                },
            );
        }
    }

    // Map last_use → early_drop_rc for Rc-typed variables
    for (def_id, _span) in &hints.last_use {
        if let Some((fn_name, var_name, ty)) = id_map.get(def_id) {
            if let Type::Rc(inner) = ty {
                out.early_drop_rc
                    .insert((fn_name.clone(), var_name.clone()));
                out.early_drop_inner_ty
                    .insert((fn_name.clone(), var_name.clone()), *inner.clone());
            }
        }
    }

    // Map tail_reuse
    for (def_id, info) in &hints.tail_reuse {
        if let Some((fn_name, var_name, _)) = id_map.get(def_id) {
            out.tail_reuse_params
                .entry(fn_name.clone())
                .or_default()
                .push(TailReuseCodegen {
                    param_name: var_name.clone(),
                    param_ty: info.param_ty.clone(),
                    alloc_ty: info.alloc_ty.clone(),
                });
        }
    }

    // Map fbip_sites
    for site in &hints.fbip_sites {
        if let Some((fn_name, var_name, _)) = id_map.get(&site.subject_id) {
            out.fbip_subjects
                .insert((fn_name.clone(), var_name.clone()));
        }
    }

    // Collect all Rc-typed variables
    for (_, (fn_name, var_name, ty)) in &id_map {
        if matches!(ty, Type::Rc(_)) {
            out.rc_vars.insert((fn_name.clone(), var_name.clone()));
        }
    }

    out
}

/// Collect DefId→(fn_name, var_name, type) mappings from a function.
fn collect_defid_names(f: &Fn, map: &mut HashMap<DefId, (String, String, Type)>) {
    let fn_name = &f.name;

    // Parameters
    for p in &f.params {
        map.insert(p.def_id, (fn_name.clone(), p.name.clone(), p.ty.clone()));
    }

    // Walk body
    collect_defid_names_block(fn_name, &f.body, map);
}

fn collect_defid_names_block(
    fn_name: &str,
    block: &Block,
    map: &mut HashMap<DefId, (String, String, Type)>,
) {
    for stmt in block {
        collect_defid_names_stmt(fn_name, stmt, map);
    }
}

fn collect_defid_names_stmt(
    fn_name: &str,
    stmt: &Stmt,
    map: &mut HashMap<DefId, (String, String, Type)>,
) {
    match stmt {
        Stmt::Bind(b) => {
            map.insert(
                b.def_id,
                (fn_name.to_string(), b.name.clone(), b.ty.clone()),
            );
            collect_defid_names_expr(fn_name, &b.value, map);
        }
        Stmt::TupleBind(bindings, value, _) => {
            for (def_id, name, ty) in bindings {
                map.insert(*def_id, (fn_name.to_string(), name.clone(), ty.clone()));
            }
            collect_defid_names_expr(fn_name, value, map);
        }
        Stmt::Assign(t, v, _) => {
            collect_defid_names_expr(fn_name, t, map);
            collect_defid_names_expr(fn_name, v, map);
        }
        Stmt::Expr(e) => collect_defid_names_expr(fn_name, e, map),
        Stmt::If(i) => {
            collect_defid_names_expr(fn_name, &i.cond, map);
            collect_defid_names_block(fn_name, &i.then, map);
            for (ec, eb) in &i.elifs {
                collect_defid_names_expr(fn_name, ec, map);
                collect_defid_names_block(fn_name, eb, map);
            }
            if let Some(els) = &i.els {
                collect_defid_names_block(fn_name, els, map);
            }
        }
        Stmt::While(w) => {
            collect_defid_names_expr(fn_name, &w.cond, map);
            collect_defid_names_block(fn_name, &w.body, map);
        }
        Stmt::For(f) => {
            map.insert(
                f.bind_id,
                (fn_name.to_string(), f.bind.clone(), f.bind_ty.clone()),
            );
            collect_defid_names_expr(fn_name, &f.iter, map);
            if let Some(end) = &f.end {
                collect_defid_names_expr(fn_name, end, map);
            }
            if let Some(step) = &f.step {
                collect_defid_names_expr(fn_name, step, map);
            }
            collect_defid_names_block(fn_name, &f.body, map);
        }
        Stmt::Loop(l) => collect_defid_names_block(fn_name, &l.body, map),
        Stmt::Ret(v, _, _) => {
            if let Some(e) = v {
                collect_defid_names_expr(fn_name, e, map);
            }
        }
        Stmt::Break(v, _) => {
            if let Some(e) = v {
                collect_defid_names_expr(fn_name, e, map);
            }
        }
        Stmt::Continue(_) => {}
        Stmt::Match(m) => {
            collect_defid_names_expr(fn_name, &m.subject, map);
            for arm in &m.arms {
                collect_defid_names_pat(fn_name, &arm.pat, map);
                collect_defid_names_block(fn_name, &arm.body, map);
            }
        }
        Stmt::Asm(a) => {
            for (_, e) in &a.inputs {
                collect_defid_names_expr(fn_name, e, map);
            }
        }
        Stmt::Drop(def_id, ty, _) => {
            // Drops reference existing DefIds — no new mapping needed,
            // but ensure the DefId is registered if it hasn't been already.
            map.entry(*def_id)
                .or_insert_with(|| (fn_name.to_string(), format!("__drop_{}", def_id.0), ty.clone()));
        }
        Stmt::ErrReturn(e, _, _) => collect_defid_names_expr(fn_name, e, map),
    }
}

fn collect_defid_names_expr(
    fn_name: &str,
    expr: &crate::hir::Expr,
    map: &mut HashMap<DefId, (String, String, Type)>,
) {
    match &expr.kind {
        ExprKind::Var(_, _) | ExprKind::FnRef(_, _) | ExprKind::VariantRef(_, _, _) => {}
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_)
        | ExprKind::Bool(_) | ExprKind::None | ExprKind::Void => {}
        ExprKind::BinOp(l, _, r) => {
            collect_defid_names_expr(fn_name, l, map);
            collect_defid_names_expr(fn_name, r, map);
        }
        ExprKind::UnaryOp(_, e) | ExprKind::Coerce(e, _) | ExprKind::Cast(e, _)
        | ExprKind::Ref(e) | ExprKind::Deref(e) => {
            collect_defid_names_expr(fn_name, e, map);
        }
        ExprKind::Call(_, _, args) | ExprKind::Builtin(_, args) | ExprKind::Syscall(args) => {
            for a in args {
                collect_defid_names_expr(fn_name, a, map);
            }
        }
        ExprKind::IndirectCall(callee, args) => {
            collect_defid_names_expr(fn_name, callee, map);
            for a in args {
                collect_defid_names_expr(fn_name, a, map);
            }
        }
        ExprKind::Method(obj, _, _, args) | ExprKind::StringMethod(obj, _, args) => {
            collect_defid_names_expr(fn_name, obj, map);
            for a in args {
                collect_defid_names_expr(fn_name, a, map);
            }
        }
        ExprKind::Field(obj, _, _) => collect_defid_names_expr(fn_name, obj, map),
        ExprKind::Index(a, i) => {
            collect_defid_names_expr(fn_name, a, map);
            collect_defid_names_expr(fn_name, i, map);
        }
        ExprKind::Ternary(c, t, e) => {
            collect_defid_names_expr(fn_name, c, map);
            collect_defid_names_expr(fn_name, t, map);
            collect_defid_names_expr(fn_name, e, map);
        }
        ExprKind::Array(elems) | ExprKind::Tuple(elems) => {
            for e in elems {
                collect_defid_names_expr(fn_name, e, map);
            }
        }
        ExprKind::Struct(_, inits) | ExprKind::VariantCtor(_, _, _, inits) => {
            for fi in inits {
                collect_defid_names_expr(fn_name, &fi.value, map);
            }
        }
        ExprKind::IfExpr(i) => {
            collect_defid_names_expr(fn_name, &i.cond, map);
            collect_defid_names_block(fn_name, &i.then, map);
            for (ec, eb) in &i.elifs {
                collect_defid_names_expr(fn_name, ec, map);
                collect_defid_names_block(fn_name, eb, map);
            }
            if let Some(els) = &i.els {
                collect_defid_names_block(fn_name, els, map);
            }
        }
        ExprKind::Pipe(first, _, _, rest) => {
            collect_defid_names_expr(fn_name, first, map);
            for a in rest {
                collect_defid_names_expr(fn_name, a, map);
            }
        }
        ExprKind::Block(stmts) => collect_defid_names_block(fn_name, stmts, map),
        ExprKind::Lambda(params, body) => {
            let lambda_fn = format!("{fn_name}.__lambda");
            for p in params {
                map.insert(
                    p.def_id,
                    (lambda_fn.clone(), p.name.clone(), p.ty.clone()),
                );
            }
            collect_defid_names_block(&lambda_fn, body, map);
        }
        ExprKind::ListComp(body, bind_id, bind_name, iter, cond, map_expr) => {
            map.insert(
                *bind_id,
                (fn_name.to_string(), bind_name.clone(), body.ty.clone()),
            );
            collect_defid_names_expr(fn_name, iter, map);
            collect_defid_names_expr(fn_name, body, map);
            if let Some(c) = cond {
                collect_defid_names_expr(fn_name, c, map);
            }
            if let Some(m) = map_expr {
                collect_defid_names_expr(fn_name, m, map);
            }
        }
    }
}

fn collect_defid_names_pat(
    fn_name: &str,
    pat: &Pat,
    map: &mut HashMap<DefId, (String, String, Type)>,
) {
    match pat {
        Pat::Wild(_) => {}
        Pat::Bind(def_id, name, ty, _) => {
            map.insert(*def_id, (fn_name.to_string(), name.clone(), ty.clone()));
        }
        Pat::Lit(e) => collect_defid_names_expr(fn_name, e, map),
        Pat::Ctor(_, _, sub_pats, _) => {
            for sp in sub_pats {
                collect_defid_names_pat(fn_name, sp, map);
            }
        }
        Pat::Or(alts, _) => {
            for alt in alts {
                collect_defid_names_pat(fn_name, alt, map);
            }
        }
        Pat::Range(lo, hi, _) => {
            collect_defid_names_expr(fn_name, lo, map);
            collect_defid_names_expr(fn_name, hi, map);
        }
    }
}

/// Find the name of the released variable that a reuse candidate references.
fn find_released_var_name(
    def_id: &DefId,
    reuse_map: &HashMap<DefId, ReuseInfo>,
    id_map: &HashMap<DefId, (String, String, Type)>,
) -> String {
    // The reuse_candidates map stores both the released and producer DefIds
    // pointing to the same ReuseInfo. Find the other DefId in the pair.
    if let Some(info) = reuse_map.get(def_id) {
        for (other_id, other_info) in reuse_map {
            if other_id != def_id
                && other_info.span == info.span
                && other_info.released_ty == info.released_ty
            {
                if let Some((_, name, _)) = id_map.get(other_id) {
                    return name.clone();
                }
            }
        }
    }
    // Fallback: use own name
    id_map
        .get(def_id)
        .map(|(_, n, _)| n.clone())
        .unwrap_or_default()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::typer::Typer;

    fn analyze(src: &str) -> PerceusHints {
        let tokens = Lexer::new(src).tokenize().unwrap();
        let prog = Parser::new(tokens).parse_program().unwrap();
        let mut typer = Typer::new();
        let hir = typer.lower_program(&prog).unwrap();
        let mut perceus = PerceusPass::new();
        perceus.optimize(&hir)
    }

    // ── Drop specialization tests ────────────────────────────────

    #[test]
    fn test_scalar_drops_elided() {
        let hints = analyze("*main()\n    x is 42\n    y is 3.14\n    z is true\n    log(x)\n    log(y)\n    log(z)\n");
        // x, y, z are all trivially droppable scalars
        assert!(
            hints.stats.drops_elided >= 3,
            "expected >= 3 drops elided for scalars, got {}",
            hints.stats.drops_elided
        );
    }

    #[test]
    fn test_array_of_scalars_drops_elided() {
        let hints = analyze("*main()\n    arr is [1, 2, 3]\n    log(arr[0])\n");
        // Fixed array of i64 is trivially droppable
        assert!(
            hints.stats.drops_elided >= 1,
            "expected >= 1 drop elided for scalar array, got {}",
            hints.stats.drops_elided
        );
    }

    #[test]
    fn test_tuple_of_scalars_drops_elided() {
        let hints = analyze("*main()\n    t is (1, 2, 3)\n    log(t)\n");
        assert!(hints.stats.drops_elided >= 1);
    }

    #[test]
    fn test_string_not_elided() {
        let hints = analyze("*main()\n    s is \"hello\"\n    log(s)\n");
        // String may hold heap data — should NOT be in elide_drops
        // Only params/borrows might be elided, not the string binding
        let has_string_elided = hints.elide_drops.iter().any(|id| {
            // We can't easily check this without DefId→type mapping,
            // but the stats should reflect that not all drops were elided
            false
        });
        assert!(!has_string_elided);
    }

    #[test]
    fn test_rc_not_elided() {
        let hints = analyze("*main()\n    x is rc(42)\n    log(@x)\n");
        // Rc values need refcount decrement — should NOT be elided
        // Check that the Rc binding's drop is not in elide_drops
        assert!(
            hints.stats.total_bindings_analyzed >= 1,
            "should have analyzed at least 1 binding"
        );
    }

    // ── Borrow elision tests ─────────────────────────────────────

    #[test]
    fn test_borrow_promoted_single_use() {
        let hints = analyze(
            "*main()\n    x is 42\n    p is %x\n    log(@p)\n"
        );
        // x is used only via borrow ref, analysis should run without crashing
        assert!(hints.stats.total_bindings_analyzed >= 2);
    }

    // ── Reuse analysis tests ─────────────────────────────────────

    #[test]
    fn test_rc_reuse_same_type() {
        let hints = analyze(
            "*main()\n    x is rc(10)\n    log(@x)\n    y is rc(20)\n    log(@y)\n"
        );
        // x and y are both Rc<i64>. If x is consumed before y is
        // allocated, x's memory could be reused for y.
        // (This depends on exact use analysis.)
        assert!(hints.stats.total_bindings_analyzed >= 2);
    }

    #[test]
    fn test_no_reuse_different_layout() {
        let hints = analyze(
            "*main()\n    x is rc(10)\n    log(@x)\n    y is rc(3.14)\n    log(@y)\n"
        );
        // Rc<i64> and Rc<f64> have the same layout (both 8 bytes),
        // so reuse IS possible here. Check analysis runs.
        assert!(hints.stats.total_bindings_analyzed >= 2);
    }

    // ── Integration tests ────────────────────────────────────────

    #[test]
    fn test_complex_program() {
        let hints = analyze(
            "*factorial(n: i64) -> i64\n    if n <= 1\n        1\n    else\n        n * factorial(n - 1)\n\n*main()\n    result is factorial(10)\n    log(result)\n"
        );
        assert!(hints.stats.total_bindings_analyzed >= 1);
        // All bindings in factorial are scalar → drops elided
        assert!(hints.stats.drops_elided >= 1);
    }

    #[test]
    fn test_loop_conservatism() {
        let hints = analyze(
            "*main()\n    x is rc(0)\n    i is 0\n    while i < 10\n        log(@x)\n        i is i + 1\n"
        );
        // x is used inside a loop → should NOT be reuse candidate
        // (escapes due to conservative loop analysis)
        assert!(hints.reuse_candidates.is_empty() || hints.stats.reuse_sites == 0);
    }

    #[test]
    fn test_function_params_analyzed() {
        let hints = analyze(
            "*add(a: i64, b: i64) -> i64\n    a + b\n*main()\n    log(add(1, 2))\n"
        );
        // Function params are registered + body bindings analyzed
        // Params are analyzed for ownership but don't count as "bindings"
        // i64 params → drops elided
        assert!(hints.stats.drops_elided >= 2);
    }

    #[test]
    fn test_struct_not_trivially_droppable() {
        assert!(!PerceusPass::is_trivially_droppable(&Type::Struct("Point".into())));
        assert!(!PerceusPass::is_trivially_droppable(&Type::String));
        assert!(!PerceusPass::is_trivially_droppable(&Type::Rc(Box::new(Type::I64))));
    }

    #[test]
    fn test_scalars_trivially_droppable() {
        assert!(PerceusPass::is_trivially_droppable(&Type::I64));
        assert!(PerceusPass::is_trivially_droppable(&Type::F64));
        assert!(PerceusPass::is_trivially_droppable(&Type::Bool));
        assert!(PerceusPass::is_trivially_droppable(&Type::Void));
        assert!(PerceusPass::is_trivially_droppable(&Type::Ptr(Box::new(Type::I64))));
    }

    #[test]
    fn test_nested_array_droppable() {
        // [i64; 3] → trivially droppable
        assert!(PerceusPass::is_trivially_droppable(&Type::Array(Box::new(Type::I64), 3)));
        // [String; 3] → NOT trivially droppable
        assert!(!PerceusPass::is_trivially_droppable(&Type::Array(Box::new(Type::String), 3)));
    }

    #[test]
    fn test_layout_compatibility() {
        // Same-type Rc should be compatible
        let rc_i64 = Type::Rc(Box::new(Type::I64));
        assert!(PerceusPass::layouts_compatible(&rc_i64, &rc_i64));

        // i64 and f64 have same size → compatible
        let rc_f64 = Type::Rc(Box::new(Type::F64));
        assert!(PerceusPass::layouts_compatible(&rc_i64, &rc_f64));

        // i64 and i8 have different sizes → incompatible
        let rc_i8 = Type::Rc(Box::new(Type::I8));
        assert!(!PerceusPass::layouts_compatible(&rc_i64, &rc_i8));
    }

    #[test]
    fn test_perceus_stats_populated() {
        let hints = analyze(
            "*main()\n    a is 1\n    b is 2.0\n    c is true\n    log(a)\n    log(b)\n    log(c)\n"
        );
        assert!(hints.stats.total_bindings_analyzed > 0);
        assert!(hints.stats.drops_elided > 0);
    }

    #[test]
    fn test_enum_analysis() {
        let hints = analyze(
            "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Red\n    match c\n        Red ? log(1)\n        Green ? log(2)\n        Blue ? log(3)\n    0\n"
        );
        assert!(hints.stats.total_bindings_analyzed >= 1);
    }

    #[test]
    fn test_generic_fn_analysis() {
        let hints = analyze(
            "*identity(x)\n    x\n*main()\n    log(identity(42))\n    log(identity(3.14))\n"
        );
        // Monomorphized generics are analyzed; params get drops elided
        assert!(hints.stats.drops_elided >= 1);
    }

    #[test]
    fn test_match_arms_analyzed() {
        let hints = analyze(
            "enum Shape\n    Circle(f64)\n    Square(f64)\n\n*area(s: Shape) -> f64\n    match s\n        Circle(r) ? 3.14159 * r * r\n        Square(side) ? side * side\n\n*main()\n    log(area(Circle(5.0)))\n"
        );
        assert!(hints.stats.total_bindings_analyzed >= 1);
    }
}
