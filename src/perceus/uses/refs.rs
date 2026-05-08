//! Sub-pass of perceus/uses split.

use crate::hir::*;

use super::super::PerceusPass;

impl PerceusPass {
    pub(in crate::perceus) fn collect_refs_block(&mut self, block: &Block, refs: &mut Vec<DefId>) {
        for stmt in block {
            self.collect_refs_stmt(stmt, refs);
        }
    }

    pub(in crate::perceus) fn collect_refs_stmt(&mut self, stmt: &Stmt, refs: &mut Vec<DefId>) {
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
            Stmt::Defer(body, _) => self.collect_refs_block(body, refs),
            Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    self.collect_refs_expr(e, refs);
                }
            }
            Stmt::StoreDelete(_, _, _) => {}
            Stmt::StoreDestroy(_, _, _) => {}
            Stmt::StoreRestore(_, _, _) => {}
            Stmt::StoreSave(_, _) => {}
            Stmt::StoreSet(_, assigns, _, _) => {
                for (_, e) in assigns {
                    self.collect_refs_expr(e, refs);
                }
            }
            Stmt::Transaction(body, _) => {
                self.collect_refs_block(body, refs);
            }
            Stmt::ChannelClose(e, _) => {
                self.collect_refs_expr(e, refs);
            }
            Stmt::Stop(e, _) => {
                self.collect_refs_expr(e, refs);
            }
            Stmt::SimFor(f, _) => {
                self.collect_refs_expr(&f.iter, refs);
                self.collect_refs_block(&f.body, refs);
            }
            Stmt::SimBlock(b, _) => {
                self.collect_refs_block(b, refs);
            }
            Stmt::UseLocal(_, _, _, _) => {}
            Stmt::GlobalStore(_, e, _) => {
                self.collect_refs_expr(e, refs);
            }
        }
    }

    pub(in crate::perceus) fn collect_refs_expr(&mut self, expr: &Expr, refs: &mut Vec<DefId>) {
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
            ExprKind::Method(obj, _, _, args)
            | ExprKind::StringMethod(obj, _, args)
            | ExprKind::DeferredMethod(obj, _, args)
            | ExprKind::VecMethod(obj, _, args)
            | ExprKind::MapMethod(obj, _, args)
            | ExprKind::SetMethod(obj, _, args)
            | ExprKind::PQMethod(obj, _, args) => {
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
            ExprKind::MapNew | ExprKind::SetNew | ExprKind::PQNew | ExprKind::NDArrayNew(_) => {}
            ExprKind::SIMDNew(elems) => {
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
            ExprKind::ChannelCreate(_, cap) => {
                self.collect_refs_expr(cap, refs);
            }
            ExprKind::ChannelSend(ch, val) => {
                self.collect_refs_expr(ch, refs);
                self.collect_refs_expr(val, refs);
            }
            ExprKind::ChannelRecv(ch) => {
                self.collect_refs_expr(ch, refs);
            }
            ExprKind::Select(arms, default_body) => {
                for arm in arms {
                    self.collect_refs_expr(&arm.chan, refs);
                    if let Some(ref v) = arm.value {
                        self.collect_refs_expr(v, refs);
                    }
                    self.collect_refs_block(&arm.body, refs);
                }
                if let Some(body) = default_body {
                    self.collect_refs_block(body, refs);
                }
            }
            ExprKind::Send(obj, _, _, _, args) => {
                self.collect_refs_expr(obj, refs);
                for a in args {
                    self.collect_refs_expr(a, refs);
                }
            }
            ExprKind::CoroutineCreate(_, body) => {
                for s in body {
                    self.collect_refs_stmt(s, refs);
                }
            }
            ExprKind::CoroutineNext(e) | ExprKind::Yield(e) | ExprKind::DynCoerce(e, _, _) => {
                self.collect_refs_expr(e, refs);
            }
            ExprKind::DynDispatch(obj, _, _, args) => {
                self.collect_refs_expr(obj, refs);
                for a in args {
                    self.collect_refs_expr(a, refs);
                }
            }
            ExprKind::StoreQuery(_, filter) => {
                self.collect_refs_expr(&filter.value, refs);
                for (_, cond) in &filter.extra {
                    self.collect_refs_expr(&cond.value, refs);
                }
            }
            ExprKind::ViewCount(_, filter) | ExprKind::ViewAll(_, filter) => {
                self.collect_refs_expr(&filter.value, refs);
                for (_, cond) in &filter.extra {
                    self.collect_refs_expr(&cond.value, refs);
                }
            }
            ExprKind::StoreFirst(_, filter) | ExprKind::StoreExists(_, filter) => {
                self.collect_refs_expr(&filter.value, refs);
                for (_, cond) in &filter.extra {
                    self.collect_refs_expr(&cond.value, refs);
                }
            }
            ExprKind::StoreGet(_, key) => {
                self.collect_refs_expr(key, refs);
            }
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Bool(_)
            | ExprKind::None
            | ExprKind::Void
            | ExprKind::VariantRef(_, _, _)
            | ExprKind::Spawn(_)
            | ExprKind::StoreCount(_)
            | ExprKind::StoreAll(_)
            | ExprKind::StoreDistinct(_, _)
            | ExprKind::StoreSum(_, _)
            | ExprKind::StoreAvg(_, _)
            | ExprKind::StoreMin(_, _)
            | ExprKind::StoreMax(_, _)
            | ExprKind::StoreVersionCount(_, _)
            | ExprKind::StoreHistory(_, _)
            | ExprKind::StoreAtVersion(_, _, _)
            | ExprKind::IterNext(_, _, _) => {}
            ExprKind::GlobalLoad(_) => {}
            ExprKind::Unreachable => {}
            ExprKind::StrictCast(e, _) | ExprKind::AsFormat(e, _) | ExprKind::AtomicLoad(e) => {
                self.collect_refs_expr(e, refs);
            }
            ExprKind::AtomicStore(a, b) | ExprKind::AtomicAdd(a, b) | ExprKind::AtomicSub(a, b) => {
                self.collect_refs_expr(a, refs);
                self.collect_refs_expr(b, refs);
            }
            ExprKind::AtomicCas(p, e, n) => {
                self.collect_refs_expr(p, refs);
                self.collect_refs_expr(e, refs);
                self.collect_refs_expr(n, refs);
            }
            ExprKind::Slice(obj, start, end) => {
                self.collect_refs_expr(obj, refs);
                self.collect_refs_expr(start, refs);
                self.collect_refs_expr(end, refs);
            }
            ExprKind::DequeNew => {}
            ExprKind::DequeMethod(obj, _, args) => {
                self.collect_refs_expr(obj, refs);
                for a in args {
                    self.collect_refs_expr(a, refs);
                }
            }
            ExprKind::Grad(e)
            | ExprKind::CowWrap(e)
            | ExprKind::CowClone(e)
            | ExprKind::GeneratorNext(e)
            | ExprKind::EnumUnwrap(e, _, _)
            | ExprKind::EnumIs(e, _) => {
                self.collect_refs_expr(e, refs);
            }
            ExprKind::Einsum(_, args) => {
                for a in args {
                    self.collect_refs_expr(a, refs);
                }
            }
            ExprKind::Builder(_, fields) => {
                for (_, v) in fields {
                    self.collect_refs_expr(v, refs);
                }
            }
            ExprKind::GeneratorCreate(_, _, stmts) => {
                self.collect_refs_block(stmts, refs);
            }
            ExprKind::KvGet(_, e) | ExprKind::KvHas(_, e) | ExprKind::KvDel(_, e) => {
                self.collect_refs_expr(e, refs)
            }
            ExprKind::KvSet(_, k, v) | ExprKind::KvIncr(_, k, v) => {
                self.collect_refs_expr(k, refs);
                self.collect_refs_expr(v, refs);
            }
            ExprKind::KvCount(_) | ExprKind::TsLatest(_) => {}
            ExprKind::VecNearest(_, v, k) => {
                self.collect_refs_expr(v, refs);
                self.collect_refs_expr(k, refs);
            }
            ExprKind::VecInsert(_, v) => self.collect_refs_expr(v, refs),
            ExprKind::VecCount(_) | ExprKind::FtsCount(_, _) => {}
            ExprKind::BloomTest(_, _, v) => self.collect_refs_expr(v, refs),
            ExprKind::FtsSearch(_, _, v) => self.collect_refs_expr(v, refs),
            ExprKind::GraphFrom(_, e) | ExprKind::GraphTo(_, e) => self.collect_refs_expr(e, refs),
        }
    }
}
