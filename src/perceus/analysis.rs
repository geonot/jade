use std::collections::HashMap;

use crate::ast::Span;
use crate::hir::*;
use crate::types::Type;

use super::{DropFusion, FbipSite, PerceusPass, PoolHint, ReuseInfo, TailReuseInfo, UseInfo};

impl PerceusPass {
    pub(super) fn analyze_drop_specialization(&mut self, uses: &HashMap<DefId, UseInfo>) {
        for (def_id, info) in uses {
            let elide = info.ty.is_trivially_droppable() && info.ownership == Ownership::Owned
                || matches!(
                    info.ownership,
                    Ownership::Borrowed | Ownership::BorrowMut | Ownership::Raw
                );
            if elide {
                self.hints.elide_drops.insert(*def_id);
                self.hints.stats.drops_elided += 1;
            }
        }
    }

    pub(super) fn analyze_reuse(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
        let rc_bindings: Vec<(DefId, &Type, Span)> = self.collect_rc_bindings(body);

        // Group Rc bindings by layout size for O(n) matching instead of O(n²)
        let mut released_by_size: HashMap<u64, Vec<(DefId, &Type, Span)>> = HashMap::new();

        for (id, ty, span) in &rc_bindings {
            let Some(info) = uses.get(id) else {
                continue;
            };
            if info.use_count != 1
                || info.escapes
                || info.borrowed
                || info.ownership != Ownership::Rc
            {
                // Not a release candidate; check if it can be an allocation target
                let size = Self::type_layout_size(match ty {
                    Type::Rc(inner) => inner.as_ref(),
                    _ => ty,
                });
                if let Some(candidates) = released_by_size.get_mut(&size) {
                    if let Some((released_id, released_ty, _)) = candidates.pop() {
                        self.hints.reuse_candidates.insert(
                            released_id,
                            ReuseInfo {
                                released_ty: released_ty.clone(),
                                allocated_ty: (*ty).clone(),
                                span: *span,
                            },
                        );
                        self.hints.reuse_candidates.insert(
                            *id,
                            ReuseInfo {
                                released_ty: released_ty.clone(),
                                allocated_ty: (*ty).clone(),
                                span: *span,
                            },
                        );
                        self.hints.stats.reuse_sites += 1;
                    }
                }
                continue;
            }
            // This is a release candidate — add to pool by layout size
            let size = Self::type_layout_size(match ty {
                Type::Rc(inner) => inner.as_ref(),
                _ => ty,
            });
            released_by_size.entry(size).or_default().push((*id, *ty, *span));
        }

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
            Stmt::For(f) => self.analyze_reuse(&f.body, uses),
            Stmt::SimFor(f, _) => self.analyze_reuse(&f.body, uses),
            Stmt::SimBlock(b, _) => self.analyze_reuse(b, uses),
            Stmt::While(w) => self.analyze_reuse(&w.body, uses),
            Stmt::Loop(l) => self.analyze_reuse(&l.body, uses),
            Stmt::Transaction(body, _) => self.analyze_reuse(body, uses),
            Stmt::Bind(_)
            | Stmt::TupleBind(_, _, _)
            | Stmt::Assign(_, _, _)
            | Stmt::Expr(_)
            | Stmt::Ret(_, _, _)
            | Stmt::Break(_, _)
            | Stmt::Continue(_)
            | Stmt::Asm(_)
            | Stmt::Drop(_, _, _, _)
            | Stmt::ErrReturn(_, _, _)
            | Stmt::StoreInsert(_, _, _)
            | Stmt::StoreDelete(_, _, _)
            | Stmt::StoreSet(_, _, _, _)
            | Stmt::ChannelClose(_, _)
            | Stmt::Stop(_, _)
            | Stmt::UseLocal(_, _, _, _) => {}
        }
    }

    pub(super) fn layouts_compatible(a: &Type, b: &Type) -> bool {
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

    fn type_layout_size(ty: &Type) -> u64 {
        Self::type_layout_size_pub(ty)
    }

    /// Public API for layout size computation, used by codegen for reuse matching.
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
            Type::Arena => 24, // {ptr, cap, offset}
            Type::Pool => 8, // opaque pointer
            Type::Deque(_) => 24,
            Type::Cow(inner) => Self::type_layout_size_pub(inner),
            Type::Alias(_, inner) | Type::Newtype(_, inner) => Self::type_layout_size_pub(inner),
            Type::Generator(_) => 8,
        }
    }

    pub(super) fn promote_borrows(&mut self, uses: &HashMap<DefId, UseInfo>) {
        for (&def_id, info) in uses {
            if info.borrowed
                && info.ownership == Ownership::Owned
                && info.use_count <= 1
                && !info.escapes
            {
                self.hints.borrow_to_move.insert(def_id);
                self.hints.stats.borrows_promoted += 1;
            }
        }
    }

    pub(super) fn analyze_last_use(&mut self, uses: &HashMap<DefId, UseInfo>) {
        for (def_id, info) in uses {
            if info.use_count > 0 && matches!(info.ownership, Ownership::Owned | Ownership::Rc) {
                if let Some(last_span) = info.last_use_span {
                    self.hints.last_use.insert(*def_id, last_span);
                    self.hints.stats.last_use_tracked += 1;
                }
            }
        }
    }

    pub(super) fn analyze_fbip(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
        for stmt in body {
            if let Stmt::Match(m) = stmt {
                let subject_id = match &m.subject.kind {
                    ExprKind::Var(id, _) => *id,
                    _ => continue,
                };
                let subject_info = match uses.get(&subject_id) {
                    Some(info) => info,
                    None => continue,
                };
                if subject_info.use_count != 1
                    || subject_info.escapes
                    || subject_info.ownership != Ownership::Owned
                {
                    continue;
                }
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
                Stmt::Match(m) => {
                    for arm in &m.arms {
                        self.analyze_fbip(&arm.body, uses);
                    }
                }
                Stmt::For(f) => self.analyze_fbip(&f.body, uses),
                Stmt::SimFor(f, _) => self.analyze_fbip(&f.body, uses),
                Stmt::SimBlock(b, _) => self.analyze_fbip(b, uses),
                Stmt::While(w) => self.analyze_fbip(&w.body, uses),
                Stmt::Loop(l) => self.analyze_fbip(&l.body, uses),
                Stmt::Transaction(body, _) => self.analyze_fbip(body, uses),
                Stmt::Bind(_)
                | Stmt::TupleBind(_, _, _)
                | Stmt::Assign(_, _, _)
                | Stmt::Expr(_)
                | Stmt::Ret(_, _, _)
                | Stmt::Break(_, _)
                | Stmt::Continue(_)
                | Stmt::Asm(_)
                | Stmt::Drop(_, _, _, _)
                | Stmt::ErrReturn(_, _, _)
                | Stmt::StoreInsert(_, _, _)
                | Stmt::StoreDelete(_, _, _)
                | Stmt::StoreSet(_, _, _, _)
                | Stmt::ChannelClose(_, _)
                | Stmt::Stop(_, _)
                | Stmt::UseLocal(_, _, _, _) => {}
            }
        }
    }

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

    pub(super) fn analyze_tail_reuse(&mut self, f: &Fn, uses: &HashMap<DefId, UseInfo>) {
        let tail_ty = match f.body.last() {
            Some(Stmt::Ret(Some(e), _, _)) => self.find_constructor_type(e),
            Some(Stmt::Expr(e)) => self.find_constructor_type(e),
            _ => None,
        };
        let Some(alloc_ty) = tail_ty else { return };

        for p in &f.params {
            let Some(info) = uses.get(&p.def_id) else {
                continue;
            };
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

    pub(super) fn analyze_drop_fusion(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
        let mut run: Vec<DefId> = Vec::new();
        let mut run_span: Option<Span> = None;

        for stmt in body {
            let is_trivial_drop = match stmt {
                Stmt::Drop(def_id, _, ty, span) => {
                    if ty.is_trivially_droppable() {
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
                    if b.ty.is_trivially_droppable() {
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
        if run.len() >= 2 {
            self.hints.drop_fusions.push(DropFusion {
                def_ids: run,
                span: run_span.unwrap_or(Span::dummy()),
            });
        }

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
                Stmt::For(f) => self.analyze_drop_fusion(&f.body, uses),
                Stmt::SimFor(f, _) => self.analyze_drop_fusion(&f.body, uses),
                Stmt::SimBlock(b, _) => self.analyze_drop_fusion(b, uses),
                Stmt::While(w) => self.analyze_drop_fusion(&w.body, uses),
                Stmt::Loop(l) => self.analyze_drop_fusion(&l.body, uses),
                Stmt::Transaction(body, _) => self.analyze_drop_fusion(body, uses),
                Stmt::Bind(_)
                | Stmt::TupleBind(_, _, _)
                | Stmt::Assign(_, _, _)
                | Stmt::Expr(_)
                | Stmt::Ret(_, _, _)
                | Stmt::Break(_, _)
                | Stmt::Continue(_)
                | Stmt::Asm(_)
                | Stmt::Drop(_, _, _, _)
                | Stmt::ErrReturn(_, _, _)
                | Stmt::StoreInsert(_, _, _)
                | Stmt::StoreDelete(_, _, _)
                | Stmt::StoreSet(_, _, _, _)
                | Stmt::ChannelClose(_, _)
                | Stmt::Stop(_, _)
                | Stmt::UseLocal(_, _, _, _) => {}
            }
        }
    }

    pub(super) fn analyze_speculative_reuse(
        &mut self,
        body: &Block,
        uses: &HashMap<DefId, UseInfo>,
    ) {
        let rc_bindings: Vec<(DefId, &Type, Span)> = self.collect_rc_bindings(body);

        for window in rc_bindings.windows(2) {
            let (released_id, released_ty, _) = &window[0];
            let (_, allocated_ty, alloc_span) = &window[1];

            if let Some(info) = uses.get(released_id) {
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
                Stmt::For(f) => self.analyze_speculative_reuse(&f.body, uses),
                Stmt::SimFor(f, _) => self.analyze_speculative_reuse(&f.body, uses),
                Stmt::SimBlock(b, _) => self.analyze_speculative_reuse(b, uses),
                Stmt::While(w) => self.analyze_speculative_reuse(&w.body, uses),
                Stmt::Loop(l) => self.analyze_speculative_reuse(&l.body, uses),
                Stmt::Transaction(body, _) => self.analyze_speculative_reuse(body, uses),
                Stmt::Bind(_)
                | Stmt::TupleBind(_, _, _)
                | Stmt::Assign(_, _, _)
                | Stmt::Expr(_)
                | Stmt::Ret(_, _, _)
                | Stmt::Break(_, _)
                | Stmt::Continue(_)
                | Stmt::Asm(_)
                | Stmt::Drop(_, _, _, _)
                | Stmt::ErrReturn(_, _, _)
                | Stmt::StoreInsert(_, _, _)
                | Stmt::StoreDelete(_, _, _)
                | Stmt::StoreSet(_, _, _, _)
                | Stmt::ChannelClose(_, _)
                | Stmt::Stop(_, _)
                | Stmt::UseLocal(_, _, _, _) => {}
            }
        }
    }

    /// Detect Rc/struct allocations inside loops that could benefit from pool allocation.
    /// When a loop body allocates same-typed Rc values repeatedly, a pool pre-allocation
    /// would eliminate per-iteration malloc overhead.
    pub(super) fn analyze_pool_hints(&mut self, body: &Block) {
        for stmt in body {
            let loop_body = match stmt {
                Stmt::While(w) => Some(&w.body),
                Stmt::For(f) => Some(&f.body),
                Stmt::SimFor(f, _) => Some(&f.body),
                Stmt::Loop(l) => Some(&l.body),
                _ => None,
            };
            if let Some(lb) = loop_body {
                let mut alloc_types: HashMap<u64, (Type, Span)> = HashMap::new();
                self.collect_alloc_types(lb, &mut alloc_types);
                for (size, (ty, span)) in alloc_types {
                    self.hints.pool_hints.push(PoolHint {
                        alloc_ty: ty,
                        size,
                        span,
                    });
                    self.hints.stats.pool_hints_found += 1;
                }
                // Recurse into the loop body for nested loops
                self.analyze_pool_hints(lb);
            } else {
                // Recurse into nested blocks
                match stmt {
                    Stmt::If(i) => {
                        self.analyze_pool_hints(&i.then);
                        for (_, eb) in &i.elifs {
                            self.analyze_pool_hints(eb);
                        }
                        if let Some(els) = &i.els {
                            self.analyze_pool_hints(els);
                        }
                    }
                    Stmt::Match(m) => {
                        for arm in &m.arms {
                            self.analyze_pool_hints(&arm.body);
                        }
                    }
                    Stmt::Transaction(body, _) => self.analyze_pool_hints(body),
                    _ => {}
                }
            }
        }
    }

    /// Collect allocation types found in a block (Rc allocs and struct constructions).
    fn collect_alloc_types(&self, body: &Block, found: &mut HashMap<u64, (Type, Span)>) {
        for stmt in body {
            match stmt {
                Stmt::Bind(b) => {
                    if let Some((ty, span)) = self.expr_alloc_type(&b.value, b.span) {
                        let size = Self::type_layout_size_pub(&ty);
                        if size > 0 {
                            found.entry(size).or_insert((ty, span));
                        }
                    }
                }
                Stmt::Expr(e) => {
                    if let Some((ty, span)) = self.expr_alloc_type(e, e.span) {
                        let size = Self::type_layout_size_pub(&ty);
                        if size > 0 {
                            found.entry(size).or_insert((ty, span));
                        }
                    }
                }
                Stmt::If(i) => {
                    self.collect_alloc_types(&i.then, found);
                    for (_, eb) in &i.elifs {
                        self.collect_alloc_types(eb, found);
                    }
                    if let Some(els) = &i.els {
                        self.collect_alloc_types(els, found);
                    }
                }
                Stmt::Match(m) => {
                    for arm in &m.arms {
                        self.collect_alloc_types(&arm.body, found);
                    }
                }
                _ => {}
            }
        }
    }

    /// Check if an expression represents a heap allocation, returning its inner type.
    fn expr_alloc_type(&self, expr: &Expr, _fallback_span: Span) -> Option<(Type, Span)> {
        match &expr.kind {
            ExprKind::Builtin(BuiltinFn::RcAlloc, _) => {
                let inner = match &expr.ty {
                    Type::Rc(inner) => inner.as_ref().clone(),
                    _ => expr.ty.clone(),
                };
                Some((inner, expr.span))
            }
            ExprKind::VariantCtor(_, _, _, _) => Some((expr.ty.clone(), expr.span)),
            ExprKind::Struct(_, _) => Some((expr.ty.clone(), expr.span)),
            _ => None,
        }
    }
}
