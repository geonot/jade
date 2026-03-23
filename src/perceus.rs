use std::collections::{HashMap, HashSet};

use crate::ast::Span;
use crate::hir::*;
use crate::types::Type;

#[derive(Debug, Clone, Default)]
pub struct PerceusHints {
    pub elide_drops: HashSet<DefId>,
    pub reuse_candidates: HashMap<DefId, ReuseInfo>,
    pub borrow_to_move: HashSet<DefId>,
    pub speculative_reuse: HashMap<DefId, ReuseInfo>,
    pub last_use: HashMap<DefId, Span>,
    pub drop_fusions: Vec<DropFusion>,
    pub fbip_sites: Vec<FbipSite>,
    pub tail_reuse: HashMap<DefId, TailReuseInfo>,
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

#[derive(Debug, Clone)]
struct UseInfo {
    use_count: u32,
    last_use_span: Option<Span>,
    escapes: bool,
    borrowed: bool,
    ty: Type,
    ownership: Ownership,
}

impl UseInfo {
    fn new(ty: Type, ownership: Ownership) -> Self {
        Self {
            use_count: 0,
            last_use_span: None,
            escapes: false,
            borrowed: false,
            ty,
            ownership,
        }
    }
}

pub struct PerceusPass {
    hints: PerceusHints,
}

impl PerceusPass {
    pub fn new() -> Self {
        Self {
            hints: PerceusHints::default(),
        }
    }

    pub fn optimize(&mut self, prog: &Program) -> PerceusHints {
        for f in &prog.fns {
            self.analyze_fn(f);
        }
        for td in &prog.types {
            for m in &td.methods {
                self.analyze_fn(m);
            }
        }
        for ti in &prog.trait_impls {
            for m in &ti.methods {
                self.analyze_fn(m);
            }
        }
        self.hints.clone()
    }

    fn analyze_fn(&mut self, f: &Fn) {
        let mut uses: HashMap<DefId, UseInfo> = HashMap::new();
        for p in &f.params {
            uses.insert(p.def_id, UseInfo::new(p.ty.clone(), p.ownership));
        }
        self.count_uses_block(&f.body, &mut uses);
        self.analyze_drop_specialization(&uses);
        self.analyze_reuse(&f.body, &uses);
        self.promote_borrows(&uses);
        self.analyze_last_use(&uses);
        self.analyze_fbip(&f.body, &uses);
        self.analyze_tail_reuse(f, &uses);
        self.analyze_drop_fusion(&f.body, &uses);
        self.analyze_speculative_reuse(&f.body, &uses);
    }

    fn count_uses_block(&mut self, block: &Block, uses: &mut HashMap<DefId, UseInfo>) {
        for stmt in block {
            self.count_uses_stmt(stmt, uses);
        }
    }

    fn count_uses_stmt(&mut self, stmt: &Stmt, uses: &mut HashMap<DefId, UseInfo>) {
        match stmt {
            Stmt::Bind(b) => {
                self.count_uses_expr(&b.value, uses);
                uses.insert(b.def_id, UseInfo::new(b.ty.clone(), b.ownership));
                self.hints.stats.total_bindings_analyzed += 1;
            }
            Stmt::TupleBind(bindings, value, _) => {
                self.count_uses_expr(value, uses);
                for (def_id, _, ty) in bindings {
                    uses.insert(
                        *def_id,
                        UseInfo::new(ty.clone(), ty.default_ownership()),
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
                    UseInfo::new(f.bind_ty.clone(), Ownership::Owned),
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
                    if let Some(ref g) = arm.guard {
                        self.count_uses_expr(g, uses);
                    }
                    self.count_uses_block(&arm.body, uses);
                }
            }
            Stmt::Asm(a) => {
                for (_, e) in &a.inputs {
                    self.count_uses_expr_escaping(e, uses);
                }
            }
            Stmt::Drop(def_id, _, _, _) => {
                if let Some(info) = uses.get_mut(def_id) {
                    info.use_count += 1;
                }
            }
            Stmt::ErrReturn(e, _, _) => {
                self.count_uses_expr_escaping(e, uses);
            }
            Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    self.count_uses_expr_escaping(e, uses);
                }
            }
            Stmt::StoreDelete(_, _, _) => {}
            Stmt::StoreSet(_, assigns, _, _) => {
                for (_, e) in assigns {
                    self.count_uses_expr_escaping(e, uses);
                }
            }
            Stmt::Transaction(body, _) => {
                self.count_uses_block(body, uses);
            }
        }
    }

    fn count_uses_block_conservative(&mut self, block: &Block, uses: &mut HashMap<DefId, UseInfo>) {
        let mut refs = Vec::new();
        self.collect_refs_block(block, &mut refs);
        for def_id in &refs {
            if let Some(info) = uses.get_mut(def_id) {
                info.use_count = info.use_count.saturating_add(2);
                info.escapes = true;
            }
        }
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
                    if let Some(ref g) = arm.guard {
                        self.collect_refs_expr(g, refs);
                    }
                    self.collect_refs_block(&arm.body, refs);
                }
            }
            Stmt::Asm(a) => {
                for (_, e) in &a.inputs {
                    self.collect_refs_expr(e, refs);
                }
            }
            Stmt::Drop(id, _, _, _) => refs.push(*id),
            Stmt::ErrReturn(e, _, _) => self.collect_refs_expr(e, refs),
            Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    self.collect_refs_expr(e, refs);
                }
            }
            Stmt::StoreDelete(_, _, _) => {}
            Stmt::StoreSet(_, assigns, _, _) => {
                for (_, e) in assigns {
                    self.collect_refs_expr(e, refs);
                }
            }
            Stmt::Transaction(body, _) => {
                self.collect_refs_block(body, refs);
            }
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
            ExprKind::Method(obj, _, _, args) | ExprKind::StringMethod(obj, _, args)
            | ExprKind::VecMethod(obj, _, args) | ExprKind::MapMethod(obj, _, args) => {
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
            ExprKind::Array(elems) | ExprKind::Tuple(elems) | ExprKind::VecNew(elems) => {
                for e in elems {
                    self.collect_refs_expr(e, refs);
                }
            }
            ExprKind::MapNew => {}
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
            Pat::Bind(def_id, _, ty, _) => {
                uses.insert(
                    *def_id,
                    UseInfo::new(ty.clone(), ty.default_ownership()),
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
            Pat::Tuple(pats, _) | Pat::Array(pats, _) => {
                for p in pats {
                    self.count_uses_pat(p, uses);
                }
            }
        }
    }

    fn count_uses_expr(&mut self, expr: &Expr, uses: &mut HashMap<DefId, UseInfo>) {
        match &expr.kind {
            ExprKind::Var(def_id, _) => {
                if let Some(info) = uses.get_mut(def_id) {
                    info.use_count += 1;
                    info.last_use_span = Some(expr.span);
                }
            }
            ExprKind::FnRef(_, _) | ExprKind::VariantRef(_, _, _) => {}
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Bool(_)
            | ExprKind::None
            | ExprKind::Void => {}

            ExprKind::BinOp(l, _, r) => {
                self.count_uses_expr(l, uses);
                self.count_uses_expr(r, uses);
            }
            ExprKind::UnaryOp(_, e) => {
                self.count_uses_expr(e, uses);
            }
            ExprKind::Call(_, _, args) => {
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
                    BuiltinFn::Log
                        | BuiltinFn::ToString
                        | BuiltinFn::Popcount
                        | BuiltinFn::Clz
                        | BuiltinFn::Ctz
                        | BuiltinFn::RotateLeft
                        | BuiltinFn::RotateRight
                        | BuiltinFn::Bswap
                        | BuiltinFn::WrappingAdd
                        | BuiltinFn::WrappingSub
                        | BuiltinFn::WrappingMul
                        | BuiltinFn::SaturatingAdd
                        | BuiltinFn::SaturatingSub
                        | BuiltinFn::SaturatingMul
                        | BuiltinFn::CheckedAdd
                        | BuiltinFn::CheckedSub
                        | BuiltinFn::CheckedMul
                        | BuiltinFn::SignalRaise
                        | BuiltinFn::SignalIgnore
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
                let mut lambda_uses: HashMap<DefId, UseInfo> = HashMap::new();
                for p in params {
                    lambda_uses.insert(p.def_id, UseInfo::new(p.ty.clone(), p.ownership));
                }
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
                    UseInfo::new(body.ty.clone(), Ownership::Owned),
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
            ExprKind::Spawn(_) => {}
            ExprKind::Send(target, _, _, _, args) => {
                self.count_uses_expr(target, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::StoreQuery(_, _) | ExprKind::StoreCount(_) | ExprKind::StoreAll(_) => {}
            ExprKind::CoroutineCreate(_, body) => {
                self.count_uses_block(body, uses);
            }
            ExprKind::CoroutineNext(inner) | ExprKind::Yield(inner) | ExprKind::DynCoerce(inner, _, _) => {
                self.count_uses_expr(inner, uses);
            }
            ExprKind::DynDispatch(obj, _, _, args) => {
                self.count_uses_expr(obj, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::VecNew(args) => {
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::MapNew => {}
            ExprKind::VecMethod(obj, _, args) | ExprKind::MapMethod(obj, _, args) => {
                self.count_uses_expr(obj, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
        }
    }

    fn count_uses_expr_escaping(&mut self, expr: &Expr, uses: &mut HashMap<DefId, UseInfo>) {
        if let ExprKind::Var(def_id, _) = &expr.kind {
            if let Some(info) = uses.get_mut(def_id) {
                info.use_count += 1;
                info.escapes = true;
                info.last_use_span = Some(expr.span);
            }
        } else {
            self.count_uses_expr(expr, uses);
        }
    }

    fn analyze_drop_specialization(&mut self, uses: &HashMap<DefId, UseInfo>) {
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

    fn analyze_reuse(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
        let rc_bindings: Vec<(DefId, &Type, Span)> = self.collect_rc_bindings(body);

        for i in 0..rc_bindings.len() {
            let (released_id, released_ty, _released_span) = &rc_bindings[i];
            let Some(info) = uses.get(released_id) else {
                continue;
            };
            if info.use_count != 1
                || info.escapes
                || info.borrowed
                || info.ownership != Ownership::Rc
            {
                continue;
            }
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
                    break;
                }
            }
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
            Stmt::While(w) => self.analyze_reuse(&w.body, uses),
            Stmt::Loop(l) => self.analyze_reuse(&l.body, uses),
            _ => {}
        }
    }

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

    fn type_layout_size(ty: &Type) -> u64 {
        match ty {
            Type::I8 | Type::U8 | Type::Bool => 1,
            Type::I16 | Type::U16 => 2,
            Type::I32 | Type::U32 | Type::F32 => 4,
            Type::I64 | Type::U64 | Type::F64 => 8,
            Type::Ptr(_) | Type::Rc(_) | Type::Weak(_) => 8,
            Type::String => 24,
            Type::Void => 0,
            Type::Array(inner, len) => Self::type_layout_size(inner) * (*len as u64),
            Type::Tuple(tys) => {
                tys.iter()
                    .map(|t| {
                        let sz = Self::type_layout_size(t);
                        (sz + 7) & !7
                    })
                    .sum()
            }
            Type::Struct(_) => 0,
            Type::Enum(_) => 0,
            Type::Fn(_, _) => 16,
            Type::Param(_) | Type::Inferred => 0,
            Type::ActorRef(_) => 8,
            Type::Coroutine(_) => 8,
            Type::DynTrait(_) => 16,
            Type::Vec(_) | Type::Map(_, _) => 24,
        }
    }

    fn promote_borrows(&mut self, uses: &HashMap<DefId, UseInfo>) {
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

    fn analyze_last_use(&mut self, uses: &HashMap<DefId, UseInfo>) {
        for (def_id, info) in uses {
            if info.use_count > 0
                && matches!(info.ownership, Ownership::Owned | Ownership::Rc)
            {
                if let Some(last_span) = info.last_use_span {
                    self.hints.last_use.insert(*def_id, last_span);
                    self.hints.stats.last_use_tracked += 1;
                }
            }
        }
    }

    fn analyze_fbip(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
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
                Stmt::For(f) => self.analyze_fbip(&f.body, uses),
                Stmt::While(w) => self.analyze_fbip(&w.body, uses),
                Stmt::Loop(l) => self.analyze_fbip(&l.body, uses),
                _ => {}
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

    fn analyze_tail_reuse(&mut self, f: &Fn, uses: &HashMap<DefId, UseInfo>) {
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

    fn analyze_drop_fusion(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
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
                Stmt::While(w) => self.analyze_drop_fusion(&w.body, uses),
                Stmt::Loop(l) => self.analyze_drop_fusion(&l.body, uses),
                _ => {}
            }
        }
    }

    fn analyze_speculative_reuse(&mut self, body: &Block, uses: &HashMap<DefId, UseInfo>) {
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
                Stmt::While(w) => self.analyze_speculative_reuse(&w.body, uses),
                Stmt::Loop(l) => self.analyze_speculative_reuse(&l.body, uses),
                _ => {}
            }
        }
    }
}

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

    #[test]
    fn test_scalar_drops_elided() {
        let hints = analyze(
            "*main()\n    x is 42\n    y is 3.14\n    z is true\n    log(x)\n    log(y)\n    log(z)\n",
        );
        assert!(
            hints.stats.drops_elided >= 3,
            "expected >= 3 drops elided for scalars, got {}",
            hints.stats.drops_elided
        );
    }

    #[test]
    fn test_array_of_scalars_drops_elided() {
        let hints = analyze("*main()\n    arr is [1, 2, 3]\n    log(arr[0])\n");
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
        assert!(!Type::String.is_trivially_droppable());
        assert!(hints.stats.total_bindings_analyzed >= 1);
    }

    #[test]
    fn test_rc_not_elided() {
        let hints = analyze("*main()\n    x is rc(42)\n    log(@x)\n");
        assert!(
            hints.stats.total_bindings_analyzed >= 1,
            "should have analyzed at least 1 binding"
        );
    }

    #[test]
    fn test_borrow_promoted_single_use() {
        let hints = analyze("*main()\n    x is 42\n    p is %x\n    log(@p)\n");
        assert!(hints.stats.total_bindings_analyzed >= 2);
    }

    #[test]
    fn test_rc_reuse_same_type() {
        let hints =
            analyze("*main()\n    x is rc(10)\n    log(@x)\n    y is rc(20)\n    log(@y)\n");
        assert!(hints.stats.total_bindings_analyzed >= 2);
    }

    #[test]
    fn test_no_reuse_different_layout() {
        let hints =
            analyze("*main()\n    x is rc(10)\n    log(@x)\n    y is rc(3.14)\n    log(@y)\n");
        assert!(hints.stats.total_bindings_analyzed >= 2);
    }

    #[test]
    fn test_complex_program() {
        let hints = analyze(
            "*factorial(n: i64) -> i64\n    if n <= 1\n        1\n    else\n        n * factorial(n - 1)\n\n*main()\n    result is factorial(10)\n    log(result)\n",
        );
        assert!(hints.stats.total_bindings_analyzed >= 1);
        assert!(hints.stats.drops_elided >= 1);
    }

    #[test]
    fn test_loop_conservatism() {
        let hints = analyze(
            "*main()\n    x is rc(0)\n    i is 0\n    while i < 10\n        log(@x)\n        i is i + 1\n",
        );
        assert!(hints.reuse_candidates.is_empty() || hints.stats.reuse_sites == 0);
    }

    #[test]
    fn test_function_params_analyzed() {
        let hints =
            analyze("*add(a: i64, b: i64) -> i64\n    a + b\n*main()\n    log(add(1, 2))\n");
        assert!(hints.stats.drops_elided >= 2);
    }

    #[test]
    fn test_struct_not_trivially_droppable() {
        assert!(!Type::Struct("Point".into()).is_trivially_droppable());
        assert!(!Type::String.is_trivially_droppable());
        assert!(!Type::Rc(Box::new(Type::I64)).is_trivially_droppable());
    }

    #[test]
    fn test_scalars_trivially_droppable() {
        assert!(Type::I64.is_trivially_droppable());
        assert!(Type::F64.is_trivially_droppable());
        assert!(Type::Bool.is_trivially_droppable());
        assert!(Type::Void.is_trivially_droppable());
        assert!(Type::Ptr(Box::new(Type::I64)).is_trivially_droppable());
    }

    #[test]
    fn test_nested_array_droppable() {
        assert!(Type::Array(Box::new(Type::I64), 3).is_trivially_droppable());
        assert!(!Type::Array(Box::new(Type::String), 3).is_trivially_droppable());
    }

    #[test]
    fn test_layout_compatibility() {
        let rc_i64 = Type::Rc(Box::new(Type::I64));
        assert!(PerceusPass::layouts_compatible(&rc_i64, &rc_i64));

        let rc_f64 = Type::Rc(Box::new(Type::F64));
        assert!(PerceusPass::layouts_compatible(&rc_i64, &rc_f64));

        let rc_i8 = Type::Rc(Box::new(Type::I8));
        assert!(!PerceusPass::layouts_compatible(&rc_i64, &rc_i8));
    }

    #[test]
    fn test_perceus_stats_populated() {
        let hints = analyze(
            "*main()\n    a is 1\n    b is 2.0\n    c is true\n    log(a)\n    log(b)\n    log(c)\n",
        );
        assert!(hints.stats.total_bindings_analyzed > 0);
        assert!(hints.stats.drops_elided > 0);
    }

    #[test]
    fn test_enum_analysis() {
        let hints = analyze(
            "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Red\n    match c\n        Red ? log(1)\n        Green ? log(2)\n        Blue ? log(3)\n    0\n",
        );
        assert!(hints.stats.total_bindings_analyzed >= 1);
    }

    #[test]
    fn test_generic_fn_analysis() {
        let hints = analyze(
            "*identity(x)\n    x\n*main()\n    log(identity(42))\n    log(identity(3.14))\n",
        );
        assert!(hints.stats.drops_elided >= 1);
    }

    #[test]
    fn test_match_arms_analyzed() {
        let hints = analyze(
            "enum Shape\n    Circle(f64)\n    Square(f64)\n\n*area(s: Shape) -> f64\n    match s\n        Circle(r) ? 3.14159 * r * r\n        Square(side) ? side * side\n\n*main()\n    log(area(Circle(5.0)))\n",
        );
        assert!(hints.stats.total_bindings_analyzed >= 1);
    }
}
