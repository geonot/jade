use std::collections::HashMap;

use crate::hir::*;

use super::{PerceusPass, UseInfo};

impl PerceusPass {
    pub(super) fn count_uses_block(&mut self, block: &Block, uses: &mut HashMap<DefId, UseInfo>) {
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
                    uses.insert(*def_id, UseInfo::new(ty.clone(), ty.default_ownership()));
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
                uses.insert(f.bind_id, UseInfo::new(f.bind_ty.clone(), Ownership::Owned));
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
            Stmt::ChannelClose(e, _) => {
                self.count_uses_expr(e, uses);
            }
            Stmt::Stop(e, _) => {
                self.count_uses_expr(e, uses);
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

    pub(super) fn collect_refs_block(&mut self, block: &Block, refs: &mut Vec<DefId>) {
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
            Stmt::ChannelClose(e, _) => {
                self.collect_refs_expr(e, refs);
            }
            Stmt::Stop(e, _) => {
                self.collect_refs_expr(e, refs);
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
            ExprKind::Method(obj, _, _, args)
            | ExprKind::StringMethod(obj, _, args)
            | ExprKind::VecMethod(obj, _, args)
            | ExprKind::MapMethod(obj, _, args) => {
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
            | ExprKind::IterNext(_, _, _) => {}
        }
    }

    fn count_uses_pat(&mut self, pat: &Pat, uses: &mut HashMap<DefId, UseInfo>) {
        match pat {
            Pat::Wild(_) => {}
            Pat::Bind(def_id, _, ty, _) => {
                uses.insert(*def_id, UseInfo::new(ty.clone(), ty.default_ownership()));
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
                uses.insert(*bind_id, UseInfo::new(body.ty.clone(), Ownership::Owned));
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
            ExprKind::CoroutineNext(inner)
            | ExprKind::Yield(inner)
            | ExprKind::DynCoerce(inner, _, _) => {
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
            ExprKind::IterNext(_, _, _) => {}
            ExprKind::VecMethod(obj, _, args) | ExprKind::MapMethod(obj, _, args) => {
                self.count_uses_expr(obj, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::ChannelCreate(_, cap) => {
                self.count_uses_expr(cap, uses);
            }
            ExprKind::ChannelSend(ch, val) => {
                self.count_uses_expr(ch, uses);
                self.count_uses_expr_escaping(val, uses);
            }
            ExprKind::ChannelRecv(ch) => {
                self.count_uses_expr(ch, uses);
            }
            ExprKind::Select(arms, default_body) => {
                for arm in arms {
                    self.count_uses_expr(&arm.chan, uses);
                    if let Some(ref v) = arm.value {
                        self.count_uses_expr_escaping(v, uses);
                    }
                    if let Some(_bind_name) = &arm.binding {
                        if let Some(bind_id) = arm.bind_id {
                            uses.insert(
                                bind_id,
                                UseInfo::new(arm.elem_ty.clone(), Ownership::Owned),
                            );
                        }
                    }
                    self.count_uses_block(&arm.body, uses);
                }
                if let Some(body) = default_body {
                    self.count_uses_block(body, uses);
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
}
