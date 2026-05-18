//! Escape analysis — Phase 2 of the access-semantics sprint.
//!
//! Walks the HIR of a single function and assigns each `Bind` a `Tier`:
//!
//! - `T1` — does not escape the source's scope. May be lowered to a raw
//!   pointer alias (no refcount, no drop at scope exit).
//! - `T2` — escapes within a single thread (returned, stored in a struct or
//!   container, captured by a non-spawning closure). Must be lowered to a
//!   single-threaded refcount (`Rc<T>` / `Rc<Cell<T>>`).
//! - `T3` — escapes across threads (sent on a `Channel`, sent to an
//!   `ActorRef`, captured by `spawn`, OR sourced from an `@atomic` type).
//!   Must be lowered to an atomic refcount (`Arc<T>` / `Arc<Mutex<T>>`).
//!
//! `Tier::Auto` is the sentinel for bindings that have not been visited yet.
//!
//! ## Algorithm (forward pass, single fn at a time)
//!
//! 1. Initialise every `Bind` in the function to `T1` (the optimistic
//!    default).  Function parameters are *sinks*: their tier is decided by
//!    their declared `Ownership` and is not analysed here.
//! 2. Walk the body in source order.  For every `ExprKind::Var(def_id, _)`
//!    use, find the enclosing statement and classify the use:
//!    * Return → at least T2.
//!    * Store in a struct field / container / map / set / deque / PQ /
//!      store-insert / global-store → at least T2.
//!    * Sent through a `Channel` (`ChannelSend`) or actor (`Send`) or
//!      captured by `Spawn` → T3 (sticky).
//!    * Captured by a `Lambda` whose body escapes (returned, stored,
//!      spawned, sent) → at least T2.  Conservatively: any capture by a
//!      `Lambda` that is itself bound to a variable or returned upgrades to
//!      at least T2.
//!    * Source type carries `@atomic` (via `ownership == Arc/ArcMut`) →
//!      T3.
//! 3. Tier escalation is monotonic: `T1 → T2 → T3`.  Once a binding hits
//!    T3 it stays T3.
//!
//! ## Status (R3.1)
//!
//! This module is intentionally a *pure analysis*: it does not mutate the
//! HIR.  R3.2 wires `EscapeInfo` into the typer and lets the chosen tier
//! drive `Bind::ownership`.  R3.3/R3.4 add the T1 raw-pointer and T2/T3
//! refcount codegen paths.  Until R3.2 lands, the existing
//! `is_aliased_read_of_heap` safety net in [`crate::typer::stmt::dispatch`]
//! remains the canonical source of truth and this module is not consulted
//! by the compile pipeline.

use std::collections::HashMap;

use crate::hir::{self, BuiltinFn, DefId, Expr, ExprKind, Stmt};

/// Lowering tier assigned to a binding by escape analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Not yet analysed (every binding starts here before the walk).
    Auto,
    /// No escape — lower to a raw pointer alias.
    T1,
    /// Escapes within a single thread — lower to `Rc<T>` or `Rc<Cell<T>>`.
    T2,
    /// Escapes across threads — lower to `Arc<T>` or `Arc<Mutex<T>>`.
    T3,
}

impl Tier {
    /// Monotonic join: `Tier::join(self, other)` returns the strictly
    /// stronger tier.  `T3` absorbs everything; `Auto` is the unit.
    pub fn join(self, other: Tier) -> Tier {
        use Tier::*;
        match (self, other) {
            (Auto, x) | (x, Auto) => x,
            (T3, _) | (_, T3) => T3,
            (T2, _) | (_, T2) => T2,
            (T1, T1) => T1,
        }
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tier::Auto => f.write_str("auto"),
            Tier::T1 => f.write_str("T1"),
            Tier::T2 => f.write_str("T2"),
            Tier::T3 => f.write_str("T3"),
        }
    }
}

/// Per-function map from each `Bind`'s `DefId` to its escape tier.
///
/// Bindings not present in the map are conservatively treated as `T2` by
/// downstream consumers (the safe default — heap-managed values without
/// known short-lived bounds).
#[derive(Debug, Clone, Default)]
pub struct EscapeInfo {
    tiers: HashMap<DefId, Tier>,
}

impl EscapeInfo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a binding's tier.  Returns `Tier::T2` for unknown DefIds.
    pub fn tier(&self, id: DefId) -> Tier {
        self.tiers.get(&id).copied().unwrap_or(Tier::T2)
    }

    /// Record / escalate a binding's tier (monotonic join).
    pub fn escalate(&mut self, id: DefId, tier: Tier) {
        let entry = self.tiers.entry(id).or_insert(Tier::Auto);
        *entry = entry.join(tier);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&DefId, &Tier)> {
        self.tiers.iter()
    }

    pub fn len(&self) -> usize {
        self.tiers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiers.is_empty()
    }
}

/// Run escape analysis on a single function body.
pub fn analyze_fn(f: &hir::Fn) -> EscapeInfo {
    let mut info = EscapeInfo::new();
    // Seed every Bind with T1; the walk below escalates as use-sites demand.
    seed_binds_in_block(&f.body, &mut info);
    // Walk every statement and classify each use of a known binding.
    let mut walker = EscapeWalk {
        info: &mut info,
        in_lambda: 0,
    };
    walker.walk_block(&f.body);
    info
}

/// R3.3 post-pass: mutate a function's HIR body to demote each `Owned`
/// binding the escape analysis classified as `T1` AND whose value is a
/// `Field`/`Index` read of a clonable, heap-managed type to
/// `Ownership::Borrowed`.
///
/// For each demoted `Bind`, the matching `Stmt::Drop(def_id, …)` in the
/// enclosing block is removed (Drops were inserted by the typer at scope
/// exit based on the original `Owned` ownership). The MIR lowerer pairs
/// the demotion by skipping the auto-clone in
/// [`crate::mir::lower::stmt`]'s `Stmt::Bind` handler whenever it sees a
/// `Borrowed` binding with a `Field`/`Index` RHS.
///
/// Together these two changes eliminate the allocate-and-immediately-free
/// pair for short-lived field/index borrows (the canonical "T1" pattern
/// from `docs/access-semantics-sprint.md` §3.3).
///
/// **Soundness**: only bindings the escape analysis proved do NOT escape
/// (the value is never returned, stored in a container or struct,
/// captured by a closure, sent on a channel, etc.) are demoted, so the
/// parent aggregate cannot be dropped before the alias goes out of scope.
///
/// **Scope**: the walk is per-`Stmt`-block local; it does NOT descend
/// into expression-embedded blocks (lambda bodies, comprehensions,
/// coroutines, generators, select arms). Bindings inside those constructs
/// retain their conservative owned-plus-clone behavior — a future
/// extension can handle them once the escape analysis distinguishes
/// captures.
///
/// Returns the number of bindings demoted (for telemetry).
pub fn apply_demotions(f: &mut hir::Fn, info: &EscapeInfo) -> usize {
    let mut count = 0usize;
    demote_block(&mut f.body, info, &mut count);
    count
}

fn demote_block(block: &mut hir::Block, info: &EscapeInfo, count: &mut usize) {
    let mut demoted: std::collections::HashSet<DefId> = std::collections::HashSet::new();
    for stmt in block.iter_mut() {
        if let Stmt::Bind(b) = stmt {
            let value_shape_ok = matches!(b.value.kind, ExprKind::Field(..) | ExprKind::Index(..))
                || is_container_read_method(&b.value);
            let qualifies = b.access_mod.is_none()
                && b.ownership == hir::Ownership::Owned
                && info.tier(b.def_id) == Tier::T1
                && value_shape_ok
                && !b.value.ty.is_trivially_droppable()
                && b.value.ty.is_value_clonable();
            if qualifies {
                b.ownership = hir::Ownership::Borrowed;
                demoted.insert(b.def_id);
                *count += 1;
            }
        }
        recurse_demote_stmt(stmt, info, count);
    }
    if !demoted.is_empty() {
        block.retain(|s| match s {
            Stmt::Drop(id, _, _, _) => !demoted.contains(id),
            _ => true,
        });
    }
}

/// Mirror of `mir::lower::stmt::is_container_read_method` for the HIR-level
/// demotion pass. Recognises the same set of element-borrow-shape container
/// reads (Vec.get/first/last, Map.get, Set.peek*, PQ.peek*, Deque.front/back).
fn is_container_read_method(expr: &hir::Expr) -> bool {
    let name = match &expr.kind {
        ExprKind::VecMethod(_, n, _) | ExprKind::MapMethod(_, n, _) => n.as_str(),
        _ => return false,
    };
    matches!(
        &*name,
        "get" | "first" | "last" | "front" | "back" | "peek" | "peek_min" | "peek_max" | "top"
    )
}

fn recurse_demote_stmt(stmt: &mut Stmt, info: &EscapeInfo, count: &mut usize) {
    match stmt {
        Stmt::If(i) => {
            demote_block(&mut i.then, info, count);
            for (_, b) in &mut i.elifs {
                demote_block(b, info, count);
            }
            if let Some(b) = &mut i.els {
                demote_block(b, info, count);
            }
        }
        Stmt::While(w) => demote_block(&mut w.body, info, count),
        Stmt::For(f) => demote_block(&mut f.body, info, count),
        Stmt::SimFor(f, _) => demote_block(&mut f.body, info, count),
        Stmt::Loop(l) => demote_block(&mut l.body, info, count),
        Stmt::Match(m) => {
            for arm in &mut m.arms {
                demote_block(&mut arm.body, info, count);
            }
        }
        Stmt::Defer(b, _) | Stmt::Transaction(b, _) | Stmt::SimBlock(b, _) => {
            demote_block(b, info, count);
        }
        _ => {}
    }
}

fn seed_binds_in_block(block: &hir::Block, info: &mut EscapeInfo) {
    for s in block {
        seed_binds_in_stmt(s, info);
    }
}

fn seed_binds_in_stmt(stmt: &Stmt, info: &mut EscapeInfo) {
    use Stmt::*;
    match stmt {
        Bind(b) => {
            info.escalate(b.def_id, Tier::T1);
            // Recurse into the RHS for nested lambdas containing their own
            // Bind statements.
            seed_binds_in_expr(&b.value, info);
        }
        TupleBind(bindings, e, _) => {
            for (id, _, _) in bindings {
                info.escalate(*id, Tier::T1);
            }
            seed_binds_in_expr(e, info);
        }
        Assign(lhs, rhs, _) => {
            seed_binds_in_expr(lhs, info);
            seed_binds_in_expr(rhs, info);
        }
        Expr(e) | Ret(Some(e), _, _) | Break(Some(e), _) | ErrReturn(e, _, _) => {
            seed_binds_in_expr(e, info);
        }
        If(i) => {
            seed_binds_in_expr(&i.cond, info);
            seed_binds_in_block(&i.then, info);
            for (c, blk) in &i.elifs {
                seed_binds_in_expr(c, info);
                seed_binds_in_block(blk, info);
            }
            if let Some(blk) = &i.els {
                seed_binds_in_block(blk, info);
            }
        }
        While(w) => {
            seed_binds_in_expr(&w.cond, info);
            seed_binds_in_block(&w.body, info);
        }
        For(fo) => {
            info.escalate(fo.bind_id, Tier::T1);
            seed_binds_in_expr(&fo.iter, info);
            seed_binds_in_block(&fo.body, info);
        }
        SimFor(fo, _) => {
            info.escalate(fo.bind_id, Tier::T1);
            seed_binds_in_expr(&fo.iter, info);
            seed_binds_in_block(&fo.body, info);
        }
        Loop(l) => seed_binds_in_block(&l.body, info),
        SimBlock(b, _) | Defer(b, _) | Transaction(b, _) => seed_binds_in_block(b, info),
        Match(m) => {
            seed_binds_in_expr(&m.subject, info);
            for arm in &m.arms {
                seed_binds_in_block(&arm.body, info);
            }
        }
        Drop(_, _, _, _)
        | Nop(_)
        | Asm(_)
        | Continue(_)
        | Ret(None, _, _)
        | Break(None, _)
        | StoreInsert(_, _, _)
        | StoreDelete(_, _, _)
        | StoreDestroy(_, _, _)
        | StoreSet(_, _, _, _)
        | StoreRestore(_, _, _)
        | StoreSave(_, _)
        | ChannelClose(_, _)
        | Stop(_, _)
        | UseLocal(_, _, _, _)
        | GlobalStore(_, _, _) => {}
    }
}

fn seed_binds_in_expr(expr: &Expr, info: &mut EscapeInfo) {
    use ExprKind::*;
    match &expr.kind {
        Lambda(_, body) | Block(body) => seed_binds_in_block(body, info),
        IfExpr(i) => {
            seed_binds_in_expr(&i.cond, info);
            seed_binds_in_block(&i.then, info);
            for (c, blk) in &i.elifs {
                seed_binds_in_expr(c, info);
                seed_binds_in_block(blk, info);
            }
            if let Some(blk) = &i.els {
                seed_binds_in_block(blk, info);
            }
        }
        Ternary(a, b, c) => {
            seed_binds_in_expr(a, info);
            seed_binds_in_expr(b, info);
            seed_binds_in_expr(c, info);
        }
        BinOp(a, _, b) => {
            seed_binds_in_expr(a, info);
            seed_binds_in_expr(b, info);
        }
        UnaryOp(_, e)
        | Coerce(e, _)
        | Cast(e, _)
        | StrictCast(e, _)
        | Ref(e)
        | Deref(e)
        | Grad(e)
        | AsFormat(e, _)
        | AtomicLoad(e)
        | EnumUnwrap(e, _, _)
        | EnumIs(e, _)
        | CoroutineNext(e)
        | GeneratorNext(e)
        | Yield(e) => seed_binds_in_expr(e, info),
        Call(_, _, args)
        | Builtin(_, args)
        | VecNew(args)
        | Array(args)
        | Tuple(args)
        | Einsum(_, args)
        | Syscall(args) => {
            for a in args {
                seed_binds_in_expr(a, info);
            }
        }
        VariantCtor(_, _, _, inits) => {
            for fi in inits {
                seed_binds_in_expr(&fi.value, info);
            }
        }
        Method(r, _, _, args)
        | StringMethod(r, _, args)
        | DeferredMethod(r, _, args)
        | VecMethod(r, _, args)
        | MapMethod(r, _, args) => {
            seed_binds_in_expr(r, info);
            for a in args {
                seed_binds_in_expr(a, info);
            }
        }
        Pipe(r, _, _, args) => {
            seed_binds_in_expr(r, info);
            for a in args {
                seed_binds_in_expr(a, info);
            }
        }
        IndirectCall(f, args) => {
            seed_binds_in_expr(f, info);
            for a in args {
                seed_binds_in_expr(a, info);
            }
        }
        Send(r, _, _, _, args) => {
            seed_binds_in_expr(r, info);
            for a in args {
                seed_binds_in_expr(a, info);
            }
        }
        ChannelSend(c, v) | AtomicStore(c, v) | AtomicAdd(c, v) | AtomicSub(c, v) | Index(c, v) => {
            seed_binds_in_expr(c, info);
            seed_binds_in_expr(v, info);
        }
        ChannelCreate(_, cap) => seed_binds_in_expr(cap, info),
        AtomicCas(a, b, c) | Slice(a, b, c) => {
            seed_binds_in_expr(a, info);
            seed_binds_in_expr(b, info);
            seed_binds_in_expr(c, info);
        }
        Field(e, _, _) | ChannelRecv(e) => seed_binds_in_expr(e, info),
        Struct(_, inits) => {
            for fi in inits {
                seed_binds_in_expr(&fi.value, info);
            }
        }
        Builder(_, inits) => {
            for (_, e) in inits {
                seed_binds_in_expr(e, info);
            }
        }
        Spawn(_, inits) => {
            for (_, e) in inits {
                seed_binds_in_expr(e, info);
            }
        }
        CoroutineCreate(_, stmts) | GeneratorCreate(_, _, stmts) => {
            for s in stmts {
                seed_binds_in_stmt(s, info);
            }
        }
        ListComp(body, id, _, iter, filt, _) => {
            info.escalate(*id, Tier::T1);
            seed_binds_in_expr(body, info);
            seed_binds_in_expr(iter, info);
            if let Some(f) = filt {
                seed_binds_in_expr(f, info);
            }
        }
        Select(arms, default) => {
            for arm in arms {
                seed_binds_in_expr(&arm.chan, info);
                if let Some(v) = &arm.value {
                    seed_binds_in_expr(v, info);
                }
                if let Some(id) = arm.bind_id {
                    info.escalate(id, Tier::T1);
                }
                seed_binds_in_block(&arm.body, info);
            }
            if let Some(blk) = default {
                seed_binds_in_block(blk, info);
            }
        }
        StoreGet(_, e)
        | KvGet(_, e)
        | KvHas(_, e)
        | KvDel(_, e)
        | VecNearest(_, e, _)
        | VecInsert(_, e)
        | BloomTest(_, _, e)
        | FtsSearch(_, _, e)
        | GraphFrom(_, e)
        | GraphTo(_, e)
        | StoreVersionCount(_, e)
        | StoreHistory(_, e) => seed_binds_in_expr(e, info),
        StoreFirst(_, _) => {}
        KvSet(_, k, v) | KvIncr(_, k, v) | StoreAtVersion(_, k, v) => {
            seed_binds_in_expr(k, info);
            seed_binds_in_expr(v, info);
        }
        Int(_)
        | Float(_)
        | Str(_)
        | Bool(_)
        | None
        | Void
        | Var(_, _)
        | FnRef(_, _)
        | VariantRef(_, _, _)
        | Unreachable
        | GlobalLoad(_)
        | StoreQuery(_, _)
        | StoreCount(_)
        | StoreAll(_)
        | StoreExists(_, _)
        | StoreDistinct(_, _)
        | StoreSum(_, _)
        | StoreAvg(_, _)
        | StoreMin(_, _)
        | StoreMax(_, _)
        | ViewCount(_, _)
        | ViewAll(_, _)
        | KvCount(_)
        | VecCount(_)
        | FtsCount(_, _)
        | TsLatest(_)
        | IterNext(_, _, _)
        | MapNew => {}
    }
}

/// Use-site classifier.  Each call to `note_var_use` records that a known
/// binding is being read; the surrounding context determines what tier the
/// use demands.
struct EscapeWalk<'a> {
    info: &'a mut EscapeInfo,
    /// `> 0` while traversing a `Lambda` body.  Captures inside escaping
    /// lambdas force at least T2.
    in_lambda: u32,
}

impl<'a> EscapeWalk<'a> {
    fn walk_block(&mut self, block: &hir::Block) {
        for s in block {
            self.walk_stmt(s);
        }
    }

    fn walk_stmt(&mut self, stmt: &Stmt) {
        use Stmt::*;
        match stmt {
            Bind(b) => {
                // The RHS is evaluated in the binding's source context.
                // Vars read on the RHS are "consumed by Bind" — that's a
                // local use; tier remains T1 unless something else
                // escalates it.
                self.walk_expr_consumer(&b.value, BindContext::LocalRead);
            }
            TupleBind(_, e, _) => self.walk_expr_consumer(e, BindContext::LocalRead),
            Assign(lhs, rhs, _) => {
                // LHS expression positions like `obj.field` or `vec[i]`
                // mean the RHS is being stored into a longer-lived
                // location.  Any binding flowing into RHS escapes.
                let ctx = lvalue_store_context(lhs);
                self.walk_expr_consumer(lhs, BindContext::LocalRead);
                self.walk_expr_consumer(rhs, ctx);
            }
            Expr(e) => self.walk_expr_consumer(e, BindContext::LocalRead),
            Ret(Some(e), _, _) | ErrReturn(e, _, _) => {
                self.walk_expr_consumer(e, BindContext::Returned);
            }
            Break(Some(e), _) => self.walk_expr_consumer(e, BindContext::LocalRead),
            If(i) => {
                self.walk_expr_consumer(&i.cond, BindContext::LocalRead);
                self.walk_block(&i.then);
                for (c, blk) in &i.elifs {
                    self.walk_expr_consumer(c, BindContext::LocalRead);
                    self.walk_block(blk);
                }
                if let Some(blk) = &i.els {
                    self.walk_block(blk);
                }
            }
            While(w) => {
                self.walk_expr_consumer(&w.cond, BindContext::LocalRead);
                self.walk_block(&w.body);
            }
            For(fo) => {
                self.walk_expr_consumer(&fo.iter, BindContext::LocalRead);
                self.walk_block(&fo.body);
            }
            SimFor(fo, _) => {
                self.walk_expr_consumer(&fo.iter, BindContext::LocalRead);
                self.walk_block(&fo.body);
            }
            Loop(l) => self.walk_block(&l.body),
            SimBlock(b, _) | Defer(b, _) | Transaction(b, _) => self.walk_block(b),
            Match(m) => {
                self.walk_expr_consumer(&m.subject, BindContext::LocalRead);
                for arm in &m.arms {
                    self.walk_block(&arm.body);
                }
            }
            StoreInsert(_, args, _) => {
                for a in args {
                    self.walk_expr_consumer(a, BindContext::StoredInContainer);
                }
            }
            StoreSet(_, fields, _, _) => {
                for (_, e) in fields {
                    self.walk_expr_consumer(e, BindContext::StoredInContainer);
                }
            }
            GlobalStore(_, e, _) => self.walk_expr_consumer(e, BindContext::StoredInContainer),
            ChannelClose(e, _) | Stop(e, _) => self.walk_expr_consumer(e, BindContext::LocalRead),
            Drop(_, _, _, _)
            | Nop(_)
            | Asm(_)
            | Continue(_)
            | Ret(None, _, _)
            | Break(None, _)
            | StoreDelete(_, _, _)
            | StoreDestroy(_, _, _)
            | StoreRestore(_, _, _)
            | StoreSave(_, _)
            | UseLocal(_, _, _, _) => {}
        }
    }

    fn walk_expr_consumer(&mut self, expr: &Expr, ctx: BindContext) {
        use ExprKind::*;
        // First record this expression itself as a use under the given ctx.
        if let Var(id, _) = &expr.kind {
            self.note_var_use(*id, ctx);
        }
        // Then recurse — but the children are typically in `LocalRead`
        // context unless we're at a structural escape site below.
        match &expr.kind {
            Var(_, _)
            | Int(_)
            | Float(_)
            | Str(_)
            | Bool(_)
            | None
            | Void
            | FnRef(_, _)
            | VariantRef(_, _, _)
            | Unreachable
            | GlobalLoad(_)
            | StoreQuery(_, _)
            | StoreCount(_)
            | StoreAll(_)
            | StoreExists(_, _)
            | StoreDistinct(_, _)
            | StoreSum(_, _)
            | StoreAvg(_, _)
            | StoreMin(_, _)
            | StoreMax(_, _)
            | ViewCount(_, _)
            | ViewAll(_, _)
            | KvCount(_)
            | VecCount(_)
            | FtsCount(_, _)
            | TsLatest(_)
            | IterNext(_, _, _)
            | MapNew => {}

            BinOp(a, _, b) => {
                self.walk_expr_consumer(a, BindContext::LocalRead);
                self.walk_expr_consumer(b, BindContext::LocalRead);
            }
            UnaryOp(_, e)
            | Coerce(e, _)
            | Cast(e, _)
            | StrictCast(e, _)
            | Ref(e)
            | Deref(e)
            | Grad(e)
            | AsFormat(e, _)
            | AtomicLoad(e)
            | EnumUnwrap(e, _, _)
            | EnumIs(e, _) => self.walk_expr_consumer(e, BindContext::LocalRead),

            // Stored-into-container sites: arguments flow into the
            // container's storage.  Conservative: every arg escapes.
            VecMethod(r, name, args)
                if matches!(
                    name.as_str().as_ref(),
                    "push" | "push_back" | "push_front" | "insert" | "set"
                ) =>
            {
                self.walk_expr_consumer(r, BindContext::LocalRead);
                for a in args {
                    self.walk_expr_consumer(a, BindContext::StoredInContainer);
                }
            }
            MapMethod(r, name, args)
                if matches!(name.as_str().as_ref(), "insert" | "set" | "put") =>
            {
                self.walk_expr_consumer(r, BindContext::LocalRead);
                for a in args {
                    self.walk_expr_consumer(a, BindContext::StoredInContainer);
                }
            }

            // Cross-thread sites — sticky T3.
            ChannelSend(c, v) => {
                self.walk_expr_consumer(c, BindContext::LocalRead);
                self.walk_expr_consumer(v, BindContext::CrossThread);
            }
            Send(r, _, _, _, args) => {
                self.walk_expr_consumer(r, BindContext::LocalRead);
                for a in args {
                    self.walk_expr_consumer(a, BindContext::CrossThread);
                }
            }
            Spawn(_, inits) => {
                for (_, e) in inits {
                    self.walk_expr_consumer(e, BindContext::CrossThread);
                }
            }

            // Generic recursion: every other node's children are local reads.
            Call(_, _, args)
            | Builtin(_, args)
            | VecNew(args)
            | Array(args)
            | Tuple(args)
            | Einsum(_, args)
            | Syscall(args) => {
                for a in args {
                    self.walk_expr_consumer(a, BindContext::LocalRead);
                }
            }
            VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    self.walk_expr_consumer(&fi.value, BindContext::StoredInContainer);
                }
            }
            VecMethod(r, _, args)
            | MapMethod(r, _, args)
            | Method(r, _, _, args)
            | StringMethod(r, _, args)
            | DeferredMethod(r, _, args) => {
                self.walk_expr_consumer(r, BindContext::LocalRead);
                for a in args {
                    self.walk_expr_consumer(a, BindContext::LocalRead);
                }
            }
            Pipe(r, _, _, args) => {
                self.walk_expr_consumer(r, BindContext::LocalRead);
                for a in args {
                    self.walk_expr_consumer(a, BindContext::LocalRead);
                }
            }
            IndirectCall(f, args) => {
                self.walk_expr_consumer(f, BindContext::LocalRead);
                for a in args {
                    self.walk_expr_consumer(a, BindContext::LocalRead);
                }
            }
            Struct(_, inits) => {
                for fi in inits {
                    self.walk_expr_consumer(&fi.value, BindContext::StoredInContainer);
                }
            }
            Builder(_, inits) => {
                for (_, e) in inits {
                    self.walk_expr_consumer(e, BindContext::StoredInContainer);
                }
            }
            Ternary(a, b, c) | Slice(a, b, c) | AtomicCas(a, b, c) => {
                self.walk_expr_consumer(a, BindContext::LocalRead);
                self.walk_expr_consumer(b, BindContext::LocalRead);
                self.walk_expr_consumer(c, BindContext::LocalRead);
            }
            AtomicStore(p, v) | AtomicAdd(p, v) | AtomicSub(p, v) | Index(p, v) => {
                self.walk_expr_consumer(p, BindContext::LocalRead);
                self.walk_expr_consumer(v, BindContext::LocalRead);
            }
            ChannelCreate(_, cap) => {
                self.walk_expr_consumer(cap, BindContext::LocalRead);
            }
            Field(e, _, _) | ChannelRecv(e) | CoroutineNext(e) | GeneratorNext(e) | Yield(e) => {
                self.walk_expr_consumer(e, BindContext::LocalRead);
            }
            CoroutineCreate(_, stmts) | GeneratorCreate(_, _, stmts) => {
                // Coroutine bodies execute later — anything they capture
                // outlives the current scope.  Treat them like spawn for
                // safety (cross-task even if single-thread).
                for s in stmts {
                    let prev = self.in_lambda;
                    self.in_lambda = prev + 1;
                    self.walk_stmt(s);
                    self.in_lambda = prev;
                }
            }
            Lambda(_, body) => {
                let prev = self.in_lambda;
                self.in_lambda = prev + 1;
                self.walk_block(body);
                self.in_lambda = prev;
            }
            IfExpr(i) => {
                self.walk_expr_consumer(&i.cond, BindContext::LocalRead);
                self.walk_block(&i.then);
                for (c, blk) in &i.elifs {
                    self.walk_expr_consumer(c, BindContext::LocalRead);
                    self.walk_block(blk);
                }
                if let Some(blk) = &i.els {
                    self.walk_block(blk);
                }
            }
            Block(b) => self.walk_block(b),
            ListComp(body, _, _, iter, filt, _) => {
                // The list-comp result is itself a fresh container — its
                // body produces values stored in that container.
                self.walk_expr_consumer(body, BindContext::StoredInContainer);
                self.walk_expr_consumer(iter, BindContext::LocalRead);
                if let Some(f) = filt {
                    self.walk_expr_consumer(f, BindContext::LocalRead);
                }
            }
            Select(arms, default) => {
                for arm in arms {
                    self.walk_expr_consumer(&arm.chan, BindContext::LocalRead);
                    if let Some(v) = &arm.value {
                        self.walk_expr_consumer(v, BindContext::CrossThread);
                    }
                    self.walk_block(&arm.body);
                }
                if let Some(blk) = default {
                    self.walk_block(blk);
                }
            }
            StoreGet(_, e)
            | KvGet(_, e)
            | KvHas(_, e)
            | KvDel(_, e)
            | VecNearest(_, e, _)
            | VecInsert(_, e)
            | BloomTest(_, _, e)
            | FtsSearch(_, _, e)
            | GraphFrom(_, e)
            | GraphTo(_, e)
            | StoreVersionCount(_, e)
            | StoreHistory(_, e) => self.walk_expr_consumer(e, BindContext::LocalRead),
            StoreFirst(_, _) => {}
            KvSet(_, k, v) | KvIncr(_, k, v) | StoreAtVersion(_, k, v) => {
                self.walk_expr_consumer(k, BindContext::LocalRead);
                self.walk_expr_consumer(v, BindContext::StoredInContainer);
            }
        }
    }

    fn note_var_use(&mut self, id: DefId, ctx: BindContext) {
        let tier = match ctx {
            BindContext::LocalRead if self.in_lambda > 0 => Tier::T2,
            BindContext::LocalRead => Tier::T1,
            BindContext::Returned | BindContext::StoredInContainer => Tier::T2,
            BindContext::CrossThread => Tier::T3,
        };
        // Only escalate bindings we've previously seeded — otherwise we'd
        // record tiers for params / globals / unrelated DefIds.
        if self.info.tiers.contains_key(&id) {
            self.info.escalate(id, tier);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum BindContext {
    /// Read for an immediate, scope-local consumer (binding RHS, condition,
    /// arithmetic operand, log/print argument…).
    LocalRead,
    /// Returned from the enclosing function (or `err`-returned).
    Returned,
    /// Stored in a struct field, container slot, store row, or global.
    StoredInContainer,
    /// Sent across a thread boundary (channel/actor/spawn).
    CrossThread,
}

/// Inspect a top-level assignment LHS to decide what kind of *store* it
/// represents.  `obj.field = ...` and `vec[i] = ...` are container stores;
/// a plain `x = ...` is a local rebind.
fn lvalue_store_context(lhs: &Expr) -> BindContext {
    match &lhs.kind {
        ExprKind::Var(_, _) => BindContext::LocalRead,
        ExprKind::Field(_, _, _) | ExprKind::Index(_, _) => BindContext::StoredInContainer,
        _ => BindContext::StoredInContainer,
    }
}

// Silence "unused" warnings for BuiltinFn pulled in for future extension.
#[allow(dead_code)]
fn _builtin_kept_for_future_use(_b: BuiltinFn) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::*;
    use crate::intern::Symbol;
    use crate::types::Type;

    fn sp() -> crate::ast::Span {
        crate::ast::Span::dummy()
    }

    fn var(id: u32, ty: Type) -> Expr {
        Expr {
            kind: ExprKind::Var(DefId(id), Symbol::intern(&format!("v{id}"))),
            ty,
            span: sp(),
        }
    }

    fn make_fn(body: Block, ret: Type) -> hir::Fn {
        hir::Fn {
            def_id: DefId(1),
            name: Symbol::intern("test"),
            params: vec![],
            ret,
            error_types: vec![],
            body,
            span: sp(),
            generic_origin: None,
            is_generator: false,
            attrs: crate::ast::FnAttrs::default(),
        }
    }

    #[test]
    fn tier_join_is_monotonic() {
        assert_eq!(Tier::T1.join(Tier::T2), Tier::T2);
        assert_eq!(Tier::T2.join(Tier::T1), Tier::T2);
        assert_eq!(Tier::T2.join(Tier::T3), Tier::T3);
        assert_eq!(Tier::T3.join(Tier::T1), Tier::T3);
        assert_eq!(Tier::Auto.join(Tier::T1), Tier::T1);
    }

    #[test]
    fn local_read_stays_t1() {
        // x is 42; log(x)
        let body: Block = vec![
            Stmt::Bind(Bind {
                def_id: DefId(10),
                name: Symbol::intern("x"),
                value: Expr {
                    kind: ExprKind::Int(42),
                    ty: Type::I64,
                    span: sp(),
                },
                ty: Type::I64,
                ownership: Ownership::Owned,
                atomic: false,
                access_mod: None,
                span: sp(),
            }),
            Stmt::Expr(Expr {
                kind: ExprKind::Builtin(BuiltinFn::Log, vec![var(10, Type::I64)]),
                ty: Type::Void,
                span: sp(),
            }),
        ];
        let info = analyze_fn(&make_fn(body, Type::Void));
        assert_eq!(info.tier(DefId(10)), Tier::T1);
    }

    #[test]
    fn returned_binding_escalates_to_t2() {
        // x is vec_new(); ret x
        let body: Block = vec![
            Stmt::Bind(Bind {
                def_id: DefId(20),
                name: Symbol::intern("x"),
                value: Expr {
                    kind: ExprKind::VecNew(vec![]),
                    ty: Type::Vec(Box::new(Type::I64)),
                    span: sp(),
                },
                ty: Type::Vec(Box::new(Type::I64)),
                ownership: Ownership::Owned,
                atomic: false,
                access_mod: None,
                span: sp(),
            }),
            Stmt::Ret(
                Some(var(20, Type::Vec(Box::new(Type::I64)))),
                Type::Vec(Box::new(Type::I64)),
                sp(),
            ),
        ];
        let info = analyze_fn(&make_fn(body, Type::Vec(Box::new(Type::I64))));
        assert_eq!(info.tier(DefId(20)), Tier::T2);
    }

    #[test]
    fn channel_send_escalates_to_t3() {
        // x is 5; chan.send(x)
        let chan_ty = Type::Channel(Box::new(Type::I64));
        let body: Block = vec![
            Stmt::Bind(Bind {
                def_id: DefId(30),
                name: Symbol::intern("x"),
                value: Expr {
                    kind: ExprKind::Int(5),
                    ty: Type::I64,
                    span: sp(),
                },
                ty: Type::I64,
                ownership: Ownership::Owned,
                atomic: false,
                access_mod: None,
                span: sp(),
            }),
            Stmt::Expr(Expr {
                kind: ExprKind::ChannelSend(
                    Box::new(Expr {
                        kind: ExprKind::Var(DefId(99), Symbol::intern("chan")),
                        ty: chan_ty.clone(),
                        span: sp(),
                    }),
                    Box::new(var(30, Type::I64)),
                ),
                ty: Type::Void,
                span: sp(),
            }),
        ];
        let info = analyze_fn(&make_fn(body, Type::Void));
        assert_eq!(info.tier(DefId(30)), Tier::T3);
    }

    /// R3.3: a Bind whose RHS is a Field/Index read of a clonable heap
    /// type AND whose use stays local should be demoted Owned → Borrowed
    /// AND its matching `Stmt::Drop` should be removed.
    #[test]
    fn apply_demotions_demotes_t1_field_read_and_removes_drop() {
        // Build:
        //   x is owner.payload    // String field read, T1 (local read only)
        //   <log x>               // local use → stays T1
        //   drop(x)               // typer-emitted scope-exit drop
        //
        // Expect: x.ownership becomes Borrowed, the Drop is removed.
        let owner = Expr {
            kind: ExprKind::Var(DefId(100), Symbol::intern("owner")),
            ty: Type::Struct(Symbol::intern("Box"), vec![]),
            span: sp(),
        };
        let field_read = Expr {
            kind: ExprKind::Field(Box::new(owner), Symbol::intern("payload"), 0),
            ty: Type::String,
            span: sp(),
        };
        let mut hfn = make_fn(
            vec![
                Stmt::Bind(Bind {
                    def_id: DefId(40),
                    name: Symbol::intern("x"),
                    value: field_read,
                    ty: Type::String,
                    ownership: Ownership::Owned,
                    atomic: false,
                    access_mod: None,
                    span: sp(),
                }),
                Stmt::Expr(var(40, Type::String)),
                Stmt::Drop(DefId(40), Symbol::intern("x"), Type::String, sp()),
            ],
            Type::Void,
        );
        let info = analyze_fn(&hfn);
        assert_eq!(info.tier(DefId(40)), Tier::T1);
        let n = apply_demotions(&mut hfn, &info);
        assert_eq!(n, 1, "expected exactly one demotion");
        // The Bind's ownership flipped to Borrowed.
        match &hfn.body[0] {
            Stmt::Bind(b) => assert_eq!(b.ownership, Ownership::Borrowed),
            _ => panic!("expected Bind in slot 0"),
        }
        // The matching Drop was removed.
        assert!(
            !hfn.body
                .iter()
                .any(|s| matches!(s, Stmt::Drop(id, _, _, _) if *id == DefId(40))),
            "Drop(DefId(40)) was not removed: {:?}",
            hfn.body
        );
    }

    /// R3.3 inverse: a Bind whose value escapes (e.g. via `return x`) must
    /// NOT be demoted. Drop must remain in place.
    #[test]
    fn apply_demotions_skips_when_value_escapes() {
        let owner = Expr {
            kind: ExprKind::Var(DefId(100), Symbol::intern("owner")),
            ty: Type::Struct(Symbol::intern("Box"), vec![]),
            span: sp(),
        };
        let field_read = Expr {
            kind: ExprKind::Field(Box::new(owner), Symbol::intern("payload"), 0),
            ty: Type::String,
            span: sp(),
        };
        let mut hfn = make_fn(
            vec![
                Stmt::Bind(Bind {
                    def_id: DefId(50),
                    name: Symbol::intern("x"),
                    value: field_read,
                    ty: Type::String,
                    ownership: Ownership::Owned,
                    atomic: false,
                    access_mod: None,
                    span: sp(),
                }),
                Stmt::Ret(Some(var(50, Type::String)), Type::String, sp()),
            ],
            Type::String,
        );
        let info = analyze_fn(&hfn);
        assert_eq!(info.tier(DefId(50)), Tier::T2);
        let n = apply_demotions(&mut hfn, &info);
        assert_eq!(n, 0, "escaping binding must not be demoted");
        match &hfn.body[0] {
            Stmt::Bind(b) => assert_eq!(b.ownership, Ownership::Owned),
            _ => panic!("expected Bind in slot 0"),
        }
    }

    /// An explicit `access_mod` on the binding (e.g. `x is take y.field`)
    /// must opt out of demotion regardless of escape tier.
    #[test]
    fn apply_demotions_respects_explicit_access_mod() {
        let owner = Expr {
            kind: ExprKind::Var(DefId(100), Symbol::intern("owner")),
            ty: Type::Struct(Symbol::intern("Box"), vec![]),
            span: sp(),
        };
        let field_read = Expr {
            kind: ExprKind::Field(Box::new(owner), Symbol::intern("payload"), 0),
            ty: Type::String,
            span: sp(),
        };
        let mut hfn = make_fn(
            vec![
                Stmt::Bind(Bind {
                    def_id: DefId(60),
                    name: Symbol::intern("x"),
                    value: field_read,
                    ty: Type::String,
                    ownership: Ownership::Owned,
                    atomic: false,
                    access_mod: Some(crate::ast::AccessMod::Take),
                    span: sp(),
                }),
                Stmt::Expr(var(60, Type::String)),
                Stmt::Drop(DefId(60), Symbol::intern("x"), Type::String, sp()),
            ],
            Type::Void,
        );
        let info = analyze_fn(&hfn);
        let n = apply_demotions(&mut hfn, &info);
        assert_eq!(n, 0, "explicit access_mod must opt out of demotion");
        match &hfn.body[0] {
            Stmt::Bind(b) => assert_eq!(b.ownership, Ownership::Owned),
            _ => panic!("expected Bind in slot 0"),
        }
    }

    /// R3.3 (container reads): a Bind whose RHS is `v.get(i)` on a
    /// `Vec<String>` and whose use stays local should be demoted
    /// Owned → Borrowed and its matching scope-exit Drop removed —
    /// mirroring the Field/Index case.
    #[test]
    fn apply_demotions_demotes_t1_vec_get_and_removes_drop() {
        let vec_ty = Type::Vec(Box::new(Type::String));
        let recv = Expr {
            kind: ExprKind::Var(DefId(200), Symbol::intern("v")),
            ty: vec_ty,
            span: sp(),
        };
        let idx = Expr {
            kind: ExprKind::Int(0),
            ty: Type::I64,
            span: sp(),
        };
        let read = Expr {
            kind: ExprKind::VecMethod(Box::new(recv), Symbol::intern("get"), vec![idx]),
            ty: Type::String,
            span: sp(),
        };
        let mut hfn = make_fn(
            vec![
                Stmt::Bind(Bind {
                    def_id: DefId(70),
                    name: Symbol::intern("x"),
                    value: read,
                    ty: Type::String,
                    ownership: Ownership::Owned,
                    atomic: false,
                    access_mod: None,
                    span: sp(),
                }),
                Stmt::Expr(var(70, Type::String)),
                Stmt::Drop(DefId(70), Symbol::intern("x"), Type::String, sp()),
            ],
            Type::Void,
        );
        let info = analyze_fn(&hfn);
        assert_eq!(info.tier(DefId(70)), Tier::T1);
        let n = apply_demotions(&mut hfn, &info);
        assert_eq!(n, 1, "expected exactly one container-read demotion");
        match &hfn.body[0] {
            Stmt::Bind(b) => assert_eq!(b.ownership, Ownership::Borrowed),
            _ => panic!("expected Bind in slot 0"),
        }
        assert!(
            !hfn.body
                .iter()
                .any(|s| matches!(s, Stmt::Drop(id, _, _, _) if *id == DefId(70))),
            "Drop(DefId(70)) was not removed: {:?}",
            hfn.body
        );
    }

    /// R3.3 (container reads, inverse): a Bind whose `v.get(i)` value
    /// escapes via Ret must NOT be demoted, and its Drop must remain.
    #[test]
    fn apply_demotions_skips_when_vec_get_value_escapes() {
        let vec_ty = Type::Vec(Box::new(Type::String));
        let recv = Expr {
            kind: ExprKind::Var(DefId(201), Symbol::intern("v")),
            ty: vec_ty,
            span: sp(),
        };
        let idx = Expr {
            kind: ExprKind::Int(0),
            ty: Type::I64,
            span: sp(),
        };
        let read = Expr {
            kind: ExprKind::VecMethod(Box::new(recv), Symbol::intern("get"), vec![idx]),
            ty: Type::String,
            span: sp(),
        };
        let mut hfn = make_fn(
            vec![
                Stmt::Bind(Bind {
                    def_id: DefId(80),
                    name: Symbol::intern("x"),
                    value: read,
                    ty: Type::String,
                    ownership: Ownership::Owned,
                    atomic: false,
                    access_mod: None,
                    span: sp(),
                }),
                Stmt::Ret(Some(var(80, Type::String)), Type::String, sp()),
            ],
            Type::String,
        );
        let info = analyze_fn(&hfn);
        assert_eq!(info.tier(DefId(80)), Tier::T2);
        let n = apply_demotions(&mut hfn, &info);
        assert_eq!(n, 0, "escaping container read must not be demoted");
        match &hfn.body[0] {
            Stmt::Bind(b) => assert_eq!(b.ownership, Ownership::Owned),
            _ => panic!("expected Bind in slot 0"),
        }
    }
}
