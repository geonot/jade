use std::collections::HashMap;

use crate::hir::*;
use crate::types::Type;

#[derive(Debug, Clone)]
pub struct OwnershipDiag {
    pub kind: DiagKind,
    pub span: crate::ast::Span,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiagKind {
    UseAfterMove,
    DoubleMutableBorrow,
    MoveOfBorrowed,
    InvalidRcDeref,
    ReturnOfBorrowed,
    WeakUpgradeWithoutCheck,
    Warning,
}

#[derive(Debug, Clone)]
struct VarState {
    ownership: Ownership,
    ty: Type,
    moved: bool,
    borrow_count: u32,
    mut_borrowed: bool,
    move_span: Option<crate::ast::Span>,
}

pub struct OwnershipVerifier {
    scopes: Vec<HashMap<DefId, VarState>>,
    pub diagnostics: Vec<OwnershipDiag>,
    fn_ret_types: HashMap<String, Type>,
}

impl OwnershipVerifier {
    pub fn new() -> Self {
        Self {
            scopes: Vec::new(),
            diagnostics: Vec::new(),
            fn_ret_types: HashMap::new(),
        }
    }

    pub fn verify(&mut self, prog: &Program) -> Vec<OwnershipDiag> {
        for f in &prog.fns {
            self.fn_ret_types.insert(f.name.clone(), f.ret.clone());
        }

        for f in &prog.fns {
            self.verify_fn(f);
        }

        for td in &prog.types {
            for m in &td.methods {
                self.verify_fn(m);
            }
        }

        for ti in &prog.trait_impls {
            for m in &ti.methods {
                self.verify_fn(m);
            }
        }

        self.diagnostics.clone()
    }

    fn verify_fn(&mut self, f: &Fn) {
        self.push_scope();

        for p in &f.params {
            self.define(
                p.def_id,
                VarState {
                    ownership: p.ownership,
                    ty: p.ty.clone(),
                    moved: false,
                    borrow_count: 0,
                    mut_borrowed: false,
                    move_span: None,
                },
            );
        }

        self.verify_block(&f.body);
        self.pop_scope();
    }

    fn verify_block(&mut self, block: &Block) {
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
            Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    self.verify_expr(e);
                }
            }
            Stmt::StoreDelete(_, _, _) => {}
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
                if let Some(end) = &f.end { self.verify_expr(end); }
                if let Some(step) = &f.step { self.verify_expr(step); }
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
            Stmt::UseLocal(_, _, _, _) => {}
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
                self.check_use(*def_id, name, expr.span);
            }

            ExprKind::FnRef(_, _) => {}

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
                        self.check_use(*def_id, name, a.span);
                        self.record_move(*def_id, a.span);
                    } else {
                        self.verify_expr(a);
                    }
                }
            }

            ExprKind::IndirectCall(callee, args) => {
                self.verify_expr(callee);
                for a in args {
                    if let ExprKind::Var(def_id, name) = &a.kind {
                        self.check_use(*def_id, name, a.span);
                        self.record_move(*def_id, a.span);
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

            ExprKind::Method(obj, _, _, args) | ExprKind::StringMethod(obj, _, args) => {
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

            ExprKind::StoreQuery(_, _) | ExprKind::StoreCount(_) | ExprKind::StoreAll(_) => {}
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
            ExprKind::MapNew | ExprKind::SetNew | ExprKind::PQNew | ExprKind::NDArrayNew(_) | ExprKind::SIMDNew(_) => {}
            ExprKind::VecMethod(obj, _, args) | ExprKind::MapMethod(obj, _, args) | ExprKind::SetMethod(obj, _, args) | ExprKind::PQMethod(obj, _, args) => {
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
            ExprKind::StrictCast(inner, _) | ExprKind::AsFormat(inner, _) | ExprKind::AtomicLoad(inner) => {
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
                for a in args { self.verify_expr(a); }
            }
            ExprKind::Grad(e) | ExprKind::CowWrap(e) | ExprKind::CowClone(e) | ExprKind::GeneratorNext(e) => {
                self.verify_expr(e);
            }
            ExprKind::Einsum(_, args) => {
                for a in args { self.verify_expr(a); }
            }
            ExprKind::Builder(_, fields) => {
                for (_, v) in fields { self.verify_expr(v); }
            }
            ExprKind::GeneratorCreate(_, _, stmts) => {
                self.verify_block(stmts);
            }
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
    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define(&mut self, id: DefId, state: VarState) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(id, state);
        }
    }

    fn lookup(&self, id: DefId) -> Option<&VarState> {
        for scope in self.scopes.iter().rev() {
            if let Some(s) = scope.get(&id) {
                return Some(s);
            }
        }
        None
    }

    fn lookup_mut(&mut self, id: DefId) -> Option<&mut VarState> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(s) = scope.get_mut(&id) {
                return Some(s);
            }
        }
        None
    }
    fn check_use(&mut self, id: DefId, name: &str, span: crate::ast::Span) {
        if id == DefId::BUILTIN {
            return;
        }
        if let Some(state) = self.lookup(id).cloned() {
            if state.moved && state.ownership == Ownership::Owned {
                self.diagnostics.push(OwnershipDiag {
                    kind: DiagKind::UseAfterMove,
                    span,
                    message: format!(
                        "use of moved value `{name}` (moved at line {})",
                        state.move_span.map(|s| s.line).unwrap_or(0)
                    ),
                });
            }
            if state.ownership == Ownership::Weak {
                self.diagnostics.push(OwnershipDiag {
                    kind: DiagKind::WeakUpgradeWithoutCheck,
                    span,
                    message: format!(
                        "weak reference `{name}` used directly — \
                         call weak_upgrade() and check for none before use"
                    ),
                });
            }
        }
    }

    fn record_borrow(&mut self, id: DefId, mutable: bool, span: crate::ast::Span) {
        let state_snapshot = self.lookup(id).cloned();
        let Some(state) = state_snapshot else { return };

        if state.moved {
            self.diagnostics.push(OwnershipDiag {
                kind: DiagKind::MoveOfBorrowed,
                span,
                message: "cannot borrow moved value".into(),
            });
            return;
        }
        if mutable {
            if state.borrow_count > 0 || state.mut_borrowed {
                self.diagnostics.push(OwnershipDiag {
                    kind: DiagKind::DoubleMutableBorrow,
                    span,
                    message: "cannot mutably borrow: already borrowed".into(),
                });
            }
            if let Some(s) = self.lookup_mut(id) {
                s.mut_borrowed = true;
            }
        } else {
            if state.mut_borrowed {
                self.diagnostics.push(OwnershipDiag {
                    kind: DiagKind::DoubleMutableBorrow,
                    span,
                    message: "cannot immutably borrow: already mutably borrowed".into(),
                });
            }
            if let Some(s) = self.lookup_mut(id) {
                s.borrow_count += 1;
            }
        }
    }

    fn record_move(&mut self, id: DefId, span: crate::ast::Span) {
        if id == DefId::BUILTIN {
            return;
        }
        if let Some(state) = self.lookup(id).cloned() {
            if (state.ownership == Ownership::Owned || state.ownership == Ownership::BorrowMut)
                && !state.ty.is_trivially_droppable()
            {
                if let Some(s) = self.lookup_mut(id) {
                    s.moved = true;
                    s.move_span = Some(span);
                }
            }
        }
    }

    fn check_return_borrows(&mut self, expr: &Expr, span: crate::ast::Span) {
        if let ExprKind::Ref(inner) = &expr.kind {
            if let ExprKind::Var(def_id, name) = &inner.kind {
                if let Some(state) = self.lookup(*def_id) {
                    if state.ownership == Ownership::Owned {
                        self.diagnostics.push(OwnershipDiag {
                            kind: DiagKind::ReturnOfBorrowed,
                            span,
                            message: format!(
                                "returning reference to local variable `{name}` — \
                                 value will be dropped when function returns"
                            ),
                        });
                    }
                }
            }
        }
    }

    /// Collect all DefIds referenced as variables in a block (for capture analysis).
    fn collect_var_ids_block(block: &Block, out: &mut std::collections::HashSet<DefId>) {
        for stmt in block {
            Self::collect_var_ids_stmt(stmt, out);
        }
    }

    fn collect_var_ids_stmt(stmt: &Stmt, out: &mut std::collections::HashSet<DefId>) {
        match stmt {
            Stmt::Expr(e) | Stmt::Bind(Bind { value: e, .. }) => Self::collect_var_ids_expr(e, out),
            Stmt::TupleBind(_, v, _) => Self::collect_var_ids_expr(v, out),
            Stmt::Assign(t, v, _) => {
                Self::collect_var_ids_expr(t, out);
                Self::collect_var_ids_expr(v, out);
            }
            Stmt::If(i) => {
                Self::collect_var_ids_expr(&i.cond, out);
                Self::collect_var_ids_block(&i.then, out);
                for (c, b) in &i.elifs {
                    Self::collect_var_ids_expr(c, out);
                    Self::collect_var_ids_block(b, out);
                }
                if let Some(b) = &i.els {
                    Self::collect_var_ids_block(b, out);
                }
            }
            Stmt::While(w) => {
                Self::collect_var_ids_expr(&w.cond, out);
                Self::collect_var_ids_block(&w.body, out);
            }
            Stmt::For(f) => {
                Self::collect_var_ids_expr(&f.iter, out);
                Self::collect_var_ids_block(&f.body, out);
            }
            Stmt::Loop(l) => Self::collect_var_ids_block(&l.body, out),
            Stmt::Match(m) => {
                Self::collect_var_ids_expr(&m.subject, out);
                for arm in &m.arms {
                    if let Some(ref g) = arm.guard {
                        Self::collect_var_ids_expr(g, out);
                    }
                    Self::collect_var_ids_block(&arm.body, out);
                }
            }
            Stmt::Ret(Some(e), _, _) | Stmt::Break(Some(e), _) | Stmt::ErrReturn(e, _, _) => {
                Self::collect_var_ids_expr(e, out);
            }
            _ => {}
        }
    }

    fn collect_var_ids_expr(e: &Expr, out: &mut std::collections::HashSet<DefId>) {
        match &e.kind {
            ExprKind::Var(def_id, _) => { out.insert(*def_id); }
            ExprKind::BinOp(l, _, r) | ExprKind::Index(l, r) => {
                Self::collect_var_ids_expr(l, out);
                Self::collect_var_ids_expr(r, out);
            }
            ExprKind::UnaryOp(_, inner) | ExprKind::Coerce(inner, _)
            | ExprKind::Cast(inner, _) | ExprKind::Ref(inner) | ExprKind::Deref(inner) => {
                Self::collect_var_ids_expr(inner, out);
            }
            ExprKind::Call(_, _, args) | ExprKind::Builtin(_, args) | ExprKind::Syscall(args) => {
                for a in args { Self::collect_var_ids_expr(a, out); }
            }
            ExprKind::IndirectCall(callee, args) => {
                Self::collect_var_ids_expr(callee, out);
                for a in args { Self::collect_var_ids_expr(a, out); }
            }
            ExprKind::Method(obj, _, _, args) | ExprKind::StringMethod(obj, _, args)
            | ExprKind::VecMethod(obj, _, args) | ExprKind::MapMethod(obj, _, args) => {
                Self::collect_var_ids_expr(obj, out);
                for a in args { Self::collect_var_ids_expr(a, out); }
            }
            ExprKind::Field(e, _, _) => Self::collect_var_ids_expr(e, out),
            ExprKind::Ternary(c, t, f) => {
                Self::collect_var_ids_expr(c, out);
                Self::collect_var_ids_expr(t, out);
                Self::collect_var_ids_expr(f, out);
            }
            ExprKind::Array(es) | ExprKind::Tuple(es) | ExprKind::VecNew(es) => {
                for e in es { Self::collect_var_ids_expr(e, out); }
            }
            ExprKind::Struct(_, inits) | ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits { Self::collect_var_ids_expr(&fi.value, out); }
            }
            ExprKind::Lambda(_, body) | ExprKind::CoroutineCreate(_, body)
            | ExprKind::GeneratorCreate(_, _, body) => {
                Self::collect_var_ids_block(body, out);
            }
            ExprKind::Block(b) => Self::collect_var_ids_block(b, out),
            ExprKind::IfExpr(i) => {
                Self::collect_var_ids_expr(&i.cond, out);
                Self::collect_var_ids_block(&i.then, out);
                for (c, b) in &i.elifs {
                    Self::collect_var_ids_expr(c, out);
                    Self::collect_var_ids_block(b, out);
                }
                if let Some(b) = &i.els {
                    Self::collect_var_ids_block(b, out);
                }
            }
            ExprKind::Pipe(left, _, _, extra) => {
                Self::collect_var_ids_expr(left, out);
                for a in extra { Self::collect_var_ids_expr(a, out); }
            }
            ExprKind::ChannelSend(ch, val) => {
                Self::collect_var_ids_expr(ch, out);
                Self::collect_var_ids_expr(val, out);
            }
            ExprKind::ChannelRecv(ch) => Self::collect_var_ids_expr(ch, out),
            ExprKind::Send(t, _, _, _, args) => {
                Self::collect_var_ids_expr(t, out);
                for a in args { Self::collect_var_ids_expr(a, out); }
            }
            ExprKind::Slice(o, s, e) => {
                Self::collect_var_ids_expr(o, out);
                Self::collect_var_ids_expr(s, out);
                Self::collect_var_ids_expr(e, out);
            }
            ExprKind::ListComp(body, _, _, iter, cond, map) => {
                Self::collect_var_ids_expr(iter, out);
                Self::collect_var_ids_expr(body, out);
                if let Some(c) = cond { Self::collect_var_ids_expr(c, out); }
                if let Some(m) = map { Self::collect_var_ids_expr(m, out); }
            }
            _ => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::typer::Typer;

    fn parse(src: &str) -> crate::ast::Program {
        let tokens = Lexer::new(src).tokenize().unwrap();
        Parser::new(tokens).parse_program().unwrap()
    }

    fn verify(src: &str) -> Vec<OwnershipDiag> {
        let prog = parse(src);
        let mut typer = Typer::new();
        let hir = typer.lower_program(&prog).unwrap();
        let mut verifier = OwnershipVerifier::new();
        verifier.verify(&hir)
    }

    #[test]
    fn test_simple_program_no_errors() {
        let diags = verify("*main()\n    x is 42\n    log(x)\n");
        assert!(
            diags.is_empty(),
            "expected no ownership errors, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_rc_binding_no_errors() {
        let diags = verify("*main()\n    x is rc(42)\n    log(@x)\n");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_function_params_no_errors() {
        let diags = verify("*add(a: i64, b: i64) -> i64\n    a + b\n*main()\n    log(add(1, 2))\n");
        assert!(diags.is_empty());
    }
}
