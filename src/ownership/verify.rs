//! Tree walks for `verify_*` and `verify_pat`.

use super::{DiagKind, OwnershipDiag, OwnershipVerifier, VarState};
use crate::hir::*;

impl OwnershipVerifier {
    pub(super) fn verify_block(&mut self, block: &Block) {
        self.push_scope();
        for stmt in block {
            self.verify_stmt(stmt);
        }
        self.pop_scope();
    }

    fn verify_block_no_scope(&mut self, block: &Block) {
        for stmt in block {
            self.verify_stmt(stmt);
        }
    }

    fn verify_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Bind(b) => {
                self.verify_expr(&b.value);
                self.define(
                    b.def_id,
                    VarState {
                        ownership: b.ownership,
                        ty: b.ty.clone(),
                        moved: false,
                        borrow_count: 0,
                        mut_borrowed: false,
                        move_span: None,
                    },
                );
            }
            Stmt::TupleBind(bindings, value, _) => {
                self.verify_expr(value);
                for (def_id, _, ty) in bindings {
                    self.define(
                        *def_id,
                        VarState {
                            ownership: ty.default_ownership(),
                            ty: ty.clone(),
                            moved: false,
                            borrow_count: 0,
                            mut_borrowed: false,
                            move_span: None,
                        },
                    );
                }
            }
            Stmt::Assign(target, value, _) => {
                self.verify_expr(target);
                self.verify_expr(value);
            }
            Stmt::Expr(e) => {
                self.verify_expr(e);
            }
            Stmt::If(i) => {
                self.verify_expr(&i.cond);
                self.verify_block(&i.then);
                for (ec, eb) in &i.elifs {
                    self.verify_expr(ec);
                    self.verify_block(eb);
                }
                if let Some(els) = &i.els {
                    self.verify_block(els);
                }
            }
            Stmt::While(w) => {
                self.verify_expr(&w.cond);
                self.verify_block(&w.body);
            }
            Stmt::For(f) => {
                self.verify_expr(&f.iter);
                if let Some(end) = &f.end {
                    self.verify_expr(end);
                }
                if let Some(step) = &f.step {
                    self.verify_expr(step);
                }
                self.push_scope();
                self.define(
                    f.bind_id,
                    VarState {
                        ownership: Ownership::Owned,
                        ty: f.bind_ty.clone(),
                        moved: false,
                        borrow_count: 0,
                        mut_borrowed: false,
                        move_span: None,
                    },
                );
                self.verify_block_no_scope(&f.body);
                self.pop_scope();
            }
            Stmt::Loop(l) => {
                self.verify_block(&l.body);
            }
            Stmt::Ret(val, _, span) => {
                if let Some(v) = val {
                    self.check_return_borrows(v, *span);
                    self.verify_expr(v);
                }
            }
            Stmt::Break(val, _) => {
                if let Some(v) = val {
                    self.verify_expr(v);
                }
            }
            Stmt::Continue(_) => {}
            Stmt::Nop(_) => {}
            Stmt::Match(m) => {
                self.verify_expr(&m.subject);
                for arm in &m.arms {
                    self.push_scope();
                    self.verify_pat(&arm.pat);
                    if let Some(ref g) = arm.guard {
                        self.verify_expr(g);
                    }
                    self.verify_block_no_scope(&arm.body);
                    self.pop_scope();
                }
            }
            Stmt::Asm(a) => {
                for (_, e) in &a.inputs {
                    self.verify_expr(e);
                }
            }
            Stmt::Drop(def_id, _, _, span) => {
                if let Some(state) = self.lookup(*def_id) {
                    if !state.moved {
                        self.record_move(*def_id, *span);
                    }
                }
            }
            Stmt::ErrReturn(e, _, _) => {
                self.verify_expr(e);
            }
            Stmt::Defer(body, _) => {
                self.verify_block(body);
            }
            Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    self.verify_expr(e);
                }
            }
            Stmt::StoreDelete(_, _, _) => {}
            Stmt::StoreDestroy(_, _, _) => {}
            Stmt::StoreRestore(_, _, _) => {}
            Stmt::StoreSave(_, _) => {}
            Stmt::StoreSet(_, assigns, _, _) => {
                for (_, e) in assigns {
                    self.verify_expr(e);
                }
            }
            Stmt::Transaction(body, _) => {
                self.verify_block(body);
            }
            Stmt::ChannelClose(e, _) => {
                self.verify_expr(e);
            }
            Stmt::Stop(e, _) => {
                self.verify_expr(e);
            }
            Stmt::SimFor(f, _) => {
                self.verify_expr(&f.iter);
                if let Some(end) = &f.end {
                    self.verify_expr(end);
                }
                if let Some(step) = &f.step {
                    self.verify_expr(step);
                }
                self.push_scope();
                self.define(
                    f.bind_id,
                    VarState {
                        ownership: Ownership::Owned,
                        ty: f.bind_ty.clone(),
                        moved: false,
                        borrow_count: 0,
                        mut_borrowed: false,
                        move_span: None,
                    },
                );
                self.verify_block_no_scope(&f.body);
                self.pop_scope();
            }
            Stmt::SimBlock(b, _) => {
                self.verify_block_no_scope(b);
            }
            Stmt::UseLocal(_, _, _, _) => {}
            Stmt::GlobalStore(_, e, _) => {
                self.verify_expr(e);
            }
        }
    }

    fn verify_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Bool(_)
            | ExprKind::None
            | ExprKind::Void => {}

            ExprKind::Var(def_id, name) => {
                self.check_use(*def_id, &name.as_str(), expr.span);
            }

            ExprKind::FnRef(_, _) => {}

            ExprKind::GlobalLoad(_) => {}

            ExprKind::VariantRef(_, _, _) => {}

            ExprKind::BinOp(lhs, _, rhs) => {
                self.verify_expr(lhs);
                self.verify_expr(rhs);
            }

            ExprKind::UnaryOp(_, inner) => {
                self.verify_expr(inner);
            }

            ExprKind::Call(_, _, args) => {
                for a in args {
                    if let ExprKind::Var(def_id, name) = &a.kind {
                        self.check_use(*def_id, &name.as_str(), a.span);
                        // Implicit borrow: Perceus RC handles copies at runtime.
                        // Only record a borrow, not a move, so the variable
                        // can be reused in subsequent expressions.
                    } else {
                        self.verify_expr(a);
                    }
                }
            }

            ExprKind::IndirectCall(callee, args) => {
                self.verify_expr(callee);
                for a in args {
                    if let ExprKind::Var(def_id, name) = &a.kind {
                        self.check_use(*def_id, &name.as_str(), a.span);
                    } else {
                        self.verify_expr(a);
                    }
                }
            }

            ExprKind::Builtin(_, args) => {
                for a in args {
                    self.verify_expr(a);
                }
            }

            ExprKind::Method(obj, _, _, args)
            | ExprKind::StringMethod(obj, _, args)
            | ExprKind::DeferredMethod(obj, _, args) => {
                self.verify_expr(obj);
                for a in args {
                    self.verify_expr(a);
                }
            }

            ExprKind::Field(obj, _, _) => {
                self.verify_expr(obj);
            }

            ExprKind::Index(arr, idx) => {
                self.verify_expr(arr);
                self.verify_expr(idx);
            }

            ExprKind::Ternary(cond, then, els) => {
                self.verify_expr(cond);
                self.verify_expr(then);
                self.verify_expr(els);
            }

            ExprKind::Coerce(inner, _) => {
                self.verify_expr(inner);
            }

            ExprKind::Cast(inner, _) => {
                self.verify_expr(inner);
            }

            ExprKind::Array(elems) => {
                for e in elems {
                    self.verify_expr(e);
                }
            }

            ExprKind::Tuple(elems) => {
                for e in elems {
                    self.verify_expr(e);
                }
            }

            ExprKind::Struct(_, inits) | ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    self.verify_expr(&fi.value);
                }
            }

            ExprKind::IfExpr(i) => {
                self.verify_expr(&i.cond);
                self.verify_block(&i.then);
                for (ec, eb) in &i.elifs {
                    self.verify_expr(ec);
                    self.verify_block(eb);
                }
                if let Some(els) = &i.els {
                    self.verify_block(els);
                }
            }

            ExprKind::Pipe(first, _, _, rest) => {
                self.verify_expr(first);
                for a in rest {
                    self.verify_expr(a);
                }
            }

            ExprKind::Block(stmts) => {
                self.verify_block(stmts);
            }

            ExprKind::Lambda(params, body) => {
                // Track captures from outer scope as moves
                let mut captured_ids = std::collections::HashSet::new();
                Self::collect_var_ids_block(body, &mut captured_ids);
                let param_ids: std::collections::HashSet<DefId> =
                    params.iter().map(|p| p.def_id).collect();
                for cap_id in &captured_ids {
                    if param_ids.contains(cap_id) || *cap_id == DefId::BUILTIN {
                        continue;
                    }
                    if let Some(state) = self.lookup(*cap_id) {
                        if state.moved && state.ownership == Ownership::Owned {
                            self.diagnostics.push(OwnershipDiag {
                                kind: DiagKind::UseAfterMove,
                                span: expr.span,
                                message: "lambda captures already-moved value".into(),
                            });
                        } else if state.ownership == Ownership::Owned {
                            // Capturing an owned variable constitutes a move
                            self.record_move(*cap_id, expr.span);
                        }
                    }
                }
                self.push_scope();
                for p in params {
                    self.define(
                        p.def_id,
                        VarState {
                            ownership: p.ty.default_ownership(),
                            ty: p.ty.clone(),
                            moved: false,
                            borrow_count: 0,
                            mut_borrowed: false,
                            move_span: None,
                        },
                    );
                }
                // Define captured variables as available in the lambda scope
                for cap_id in &captured_ids {
                    if param_ids.contains(cap_id) || *cap_id == DefId::BUILTIN {
                        continue;
                    }
                    if let Some(state) = self.lookup(*cap_id) {
                        self.define(
                            *cap_id,
                            VarState {
                                ownership: state.ownership,
                                ty: state.ty.clone(),
                                moved: false,
                                borrow_count: 0,
                                mut_borrowed: false,
                                move_span: None,
                            },
                        );
                    }
                }
                self.verify_block_no_scope(body);
                self.pop_scope();
            }

            ExprKind::Ref(inner) => {
                self.verify_expr(inner);
                if let ExprKind::Var(def_id, _) = &inner.kind {
                    self.record_borrow(*def_id, false, expr.span);
                }
            }

            ExprKind::Deref(inner) => {
                self.verify_expr(inner);
            }

            ExprKind::ListComp(body, _, _, iter, cond, map) => {
                self.verify_expr(iter);
                self.push_scope();
                self.verify_expr(body);
                if let Some(c) = cond {
                    self.verify_expr(c);
                }
                if let Some(m) = map {
                    self.verify_expr(m);
                }
                self.pop_scope();
            }

            ExprKind::Syscall(args) => {
                for a in args {
                    self.verify_expr(a);
                }
            }

            ExprKind::Spawn(_) => {}

            ExprKind::Send(target, _, _, _, args) => {
                self.verify_expr(target);
                for a in args {
                    self.verify_expr(a);
                }
            }

            ExprKind::StoreQuery(_, _)
            | ExprKind::StoreCount(_)
            | ExprKind::StoreAll(_)
            | ExprKind::ViewCount(_, _)
            | ExprKind::ViewAll(_, _)
            | ExprKind::StoreDistinct(_, _)
            | ExprKind::StoreSum(_, _)
            | ExprKind::StoreAvg(_, _)
            | ExprKind::StoreMin(_, _)
            | ExprKind::StoreMax(_, _)
            | ExprKind::StoreVersionCount(_, _)
            | ExprKind::StoreHistory(_, _)
            | ExprKind::StoreAtVersion(_, _, _) => {}
            ExprKind::StoreGet(_, key) => {
                self.verify_expr(key);
            }
            ExprKind::StoreFirst(_, _) | ExprKind::StoreExists(_, _) => {}
            ExprKind::CoroutineCreate(_, body) => {
                self.verify_block(body);
            }
            ExprKind::CoroutineNext(inner) | ExprKind::Yield(inner) => {
                self.verify_expr(inner);
            }
            ExprKind::DynDispatch(obj, _, _, args) => {
                self.verify_expr(obj);
                for a in args {
                    self.verify_expr(a);
                }
            }
            ExprKind::DynCoerce(inner, _, _) => {
                self.verify_expr(inner);
            }
            ExprKind::VecNew(args) => {
                for a in args {
                    self.verify_expr(a);
                }
            }
            ExprKind::MapNew
            | ExprKind::SetNew
            | ExprKind::PQNew
            | ExprKind::NDArrayNew(_)
            | ExprKind::SIMDNew(_) => {}
            ExprKind::VecMethod(obj, _, args)
            | ExprKind::MapMethod(obj, _, args)
            | ExprKind::SetMethod(obj, _, args)
            | ExprKind::PQMethod(obj, _, args) => {
                self.verify_expr(obj);
                for a in args {
                    self.verify_expr(a);
                }
            }
            ExprKind::IterNext(_, _, _) => {}
            ExprKind::ChannelCreate(_, cap) => {
                self.verify_expr(cap);
            }
            ExprKind::ChannelSend(ch, val) => {
                self.verify_expr(ch);
                self.verify_expr(val);
                // Sending through a channel constitutes a move
                if let ExprKind::Var(def_id, _name) = &val.kind {
                    self.record_move(*def_id, val.span);
                } else if let Some((root_id, _)) = Self::extract_root_var(val) {
                    self.record_move(root_id, val.span);
                }
            }
            ExprKind::ChannelRecv(ch) => {
                self.verify_expr(ch);
            }
            ExprKind::Select(arms, default_body) => {
                for arm in arms {
                    self.verify_expr(&arm.chan);
                    if let Some(ref v) = arm.value {
                        self.verify_expr(v);
                    }
                    self.verify_block(&arm.body);
                }
                if let Some(body) = default_body {
                    self.verify_block(body);
                }
            }
            ExprKind::Unreachable => {}
            ExprKind::StrictCast(inner, _)
            | ExprKind::AsFormat(inner, _)
            | ExprKind::AtomicLoad(inner) => {
                self.verify_expr(inner);
            }
            ExprKind::AtomicStore(a, b) | ExprKind::AtomicAdd(a, b) | ExprKind::AtomicSub(a, b) => {
                self.verify_expr(a);
                self.verify_expr(b);
            }
            ExprKind::AtomicCas(ptr, expected, new) => {
                self.verify_expr(ptr);
                self.verify_expr(expected);
                self.verify_expr(new);
            }
            ExprKind::Slice(obj, start, end) => {
                self.verify_expr(obj);
                self.verify_expr(start);
                self.verify_expr(end);
            }
            ExprKind::DequeNew => {}
            ExprKind::DequeMethod(obj, _, args) => {
                self.verify_expr(obj);
                for a in args {
                    self.verify_expr(a);
                }
            }
            ExprKind::Grad(e)
            | ExprKind::CowWrap(e)
            | ExprKind::CowClone(e)
            | ExprKind::GeneratorNext(e)
            | ExprKind::EnumUnwrap(e, _, _)
            | ExprKind::EnumIs(e, _) => {
                self.verify_expr(e);
            }
            ExprKind::Einsum(_, args) => {
                for a in args {
                    self.verify_expr(a);
                }
            }
            ExprKind::Builder(_, fields) => {
                for (_, v) in fields {
                    self.verify_expr(v);
                }
            }
            ExprKind::GeneratorCreate(_, _, stmts) => {
                self.verify_block(stmts);
            }
            ExprKind::KvGet(_, e) | ExprKind::KvHas(_, e) | ExprKind::KvDel(_, e) => {
                self.verify_expr(e)
            }
            ExprKind::KvSet(_, k, v) | ExprKind::KvIncr(_, k, v) => {
                self.verify_expr(k);
                self.verify_expr(v);
            }
            ExprKind::KvCount(_) | ExprKind::TsLatest(_) => {}
            ExprKind::VecNearest(_, v, k) => {
                self.verify_expr(v);
                self.verify_expr(k);
            }
            ExprKind::VecInsert(_, v) => self.verify_expr(v),
            ExprKind::VecCount(_) | ExprKind::FtsCount(_, _) => {}
            ExprKind::BloomTest(_, _, v) => self.verify_expr(v),
            ExprKind::FtsSearch(_, _, v) => self.verify_expr(v),
            ExprKind::GraphFrom(_, e) | ExprKind::GraphTo(_, e) => self.verify_expr(e),
        }
    }

    fn verify_pat(&mut self, pat: &Pat) {
        match pat {
            Pat::Wild(_) => {}
            Pat::Bind(def_id, _, ty, _) => {
                self.define(
                    *def_id,
                    VarState {
                        ownership: ty.default_ownership(),
                        ty: ty.clone(),
                        moved: false,
                        borrow_count: 0,
                        mut_borrowed: false,
                        move_span: None,
                    },
                );
            }
            Pat::Lit(e) => {
                self.verify_expr(e);
            }
            Pat::Ctor(_, _, sub_pats, _) => {
                for sp in sub_pats {
                    self.verify_pat(sp);
                }
            }
            Pat::Or(alts, _) => {
                for alt in alts {
                    self.verify_pat(alt);
                }
            }
            Pat::Range(lo, hi, _) => {
                self.verify_expr(lo);
                self.verify_expr(hi);
            }
            Pat::Tuple(pats, _) | Pat::Array(pats, _) => {
                for p in pats {
                    self.verify_pat(p);
                }
            }
        }
    }
}
