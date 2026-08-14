use crate::ast::{self, Decl, Expr, Pat, Stmt};
use crate::intern::Symbol;
use std::collections::HashMap;

pub fn flatten_module(decls: Vec<Decl>, module: &str) -> Vec<Decl> {
    let mut rename_map: HashMap<Symbol, Symbol> = HashMap::new();
    for d in &decls {
        match d {
            Decl::Fn(f) => {
                if f.name.contains_str("_") && f.name.as_str().starts_with(char::is_uppercase) {
                    continue;
                }
                rename_map.insert(f.name, Symbol::intern(&format!("{}_{}", module, f.name)));
            }
            Decl::Const(name, _, _) => {
                rename_map.insert(*name, Symbol::intern(&format!("{}_{}", module, name)));
            }
            _ => {}
        }
    }

    let mut r = Renamer::new(&rename_map);
    decls
        .into_iter()
        .map(|d| match d {
            Decl::Fn(mut f) => {
                if let Some(new) = rename_map.get(&f.name) {
                    f.name = *new;
                }
                let pnames: Vec<Symbol> = f.params.iter().map(|p| p.name).collect();
                r.in_fn_scope(&pnames, |r| {
                    r.rewrite_block(&mut f.body);
                    for p in &mut f.params {
                        if let Some(ref mut def) = p.default {
                            r.rewrite_expr(def);
                        }
                    }
                });
                Decl::Fn(f)
            }
            Decl::Const(name, mut expr, span) => {
                let new_name = rename_map.get(&name).copied().unwrap_or(name);
                r.rewrite_expr(&mut expr);
                Decl::Const(new_name, expr, span)
            }
            Decl::Global(name, mut expr, span) => {
                r.rewrite_expr(&mut expr);
                Decl::Global(name, expr, span)
            }
            Decl::Type(mut td) => {
                for fld in &mut td.fields {
                    if let Some(ref mut def) = fld.default {
                        r.rewrite_expr(def);
                    }
                }
                for m in &mut td.methods {
                    let pnames: Vec<Symbol> = m.params.iter().map(|p| p.name).collect();
                    r.in_fn_scope(&pnames, |r| r.rewrite_block(&mut m.body));
                }
                Decl::Type(td)
            }
            Decl::Impl(mut ib) => {
                for m in &mut ib.methods {
                    let pnames: Vec<Symbol> = m.params.iter().map(|p| p.name).collect();
                    r.in_fn_scope(&pnames, |r| r.rewrite_block(&mut m.body));
                }
                Decl::Impl(ib)
            }
            Decl::Actor(mut ad) => {
                for fld in &mut ad.fields {
                    if let Some(ref mut def) = fld.default {
                        r.rewrite_expr(def);
                    }
                }
                for h in &mut ad.handlers {
                    let pnames: Vec<Symbol> = h.params.iter().map(|p| p.name).collect();
                    r.in_fn_scope(&pnames, |r| {
                        if let Some(ref mut e) = h.loop_sleep_ms {
                            r.rewrite_expr(e);
                        }
                        r.rewrite_block(&mut h.body);
                    });
                }
                Decl::Actor(ad)
            }
            Decl::Test(mut tb) => {
                r.in_fn_scope(&[], |r| r.rewrite_block(&mut tb.body));
                Decl::Test(tb)
            }
            other => other,
        })
        .collect()
}

struct Renamer<'a> {
    renames: &'a HashMap<Symbol, Symbol>,
    scopes: Vec<Vec<Symbol>>,
    shadowed: HashMap<Symbol, u32>,
}

impl<'a> Renamer<'a> {
    fn new(renames: &'a HashMap<Symbol, Symbol>) -> Self {
        Renamer {
            renames,
            scopes: Vec::new(),
            shadowed: HashMap::new(),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        for name in self.scopes.pop().expect("scope underflow") {
            match self.shadowed.get_mut(&name) {
                Some(1) => {
                    self.shadowed.remove(&name);
                }
                Some(n) => *n -= 1,
                None => unreachable!("unshadowing a name that was never shadowed"),
            }
        }
    }

    fn shadow(&mut self, name: Symbol) {
        self.scopes
            .last_mut()
            .expect("shadow outside any scope")
            .push(name);
        *self.shadowed.entry(name).or_insert(0) += 1;
    }

    fn rewrite_pat(&mut self, pat: &mut Pat) {
        match pat {
            Pat::Ident(s, _) => {
                if let Some(new) = self.lookup(*s) {
                    *s = new;
                }
            }
            Pat::Ctor(_, ps, _) | Pat::Or(ps, _) | Pat::Tuple(ps, _) | Pat::Array(ps, _) => {
                for p in ps {
                    self.rewrite_pat(p);
                }
            }
            Pat::Wild(_) | Pat::Lit(_) | Pat::Range(_, _, _) => {}
        }
    }

    fn lookup(&self, name: Symbol) -> Option<Symbol> {
        if self.shadowed.contains_key(&name) {
            return None;
        }
        self.renames.get(&name).copied()
    }

    fn in_fn_scope(&mut self, params: &[Symbol], f: impl FnOnce(&mut Self)) {
        self.push_scope();
        for p in params {
            self.shadow(*p);
        }
        f(self);
        self.pop_scope();
    }

    fn rewrite_block(&mut self, block: &mut ast::Block) {
        self.push_scope();
        for stmt in block.iter_mut() {
            self.rewrite_stmt(stmt);
        }
        self.pop_scope();
    }

    fn rewrite_stmt(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Bind(b) => {
                self.rewrite_expr(&mut b.value);
                self.shadow(b.name);
            }
            Stmt::TupleBind(names, e, _) => {
                self.rewrite_expr(e);
                for n in names.iter() {
                    self.shadow(*n);
                }
            }
            Stmt::Assign(l, r, _) => {
                self.rewrite_expr(l);
                self.rewrite_expr(r);
            }
            Stmt::Expr(e) => self.rewrite_expr(e),
            Stmt::If(i) => self.rewrite_if(i),
            Stmt::While(w) => {
                self.rewrite_expr(&mut w.cond);
                self.rewrite_block(&mut w.body);
            }
            Stmt::For(f) => {
                self.rewrite_expr(&mut f.iter);
                if let Some(ref mut e) = f.end {
                    self.rewrite_expr(e);
                }
                if let Some(ref mut e) = f.step {
                    self.rewrite_expr(e);
                }
                self.push_scope();
                self.shadow(f.bind);
                if let Some(b2) = f.bind2 {
                    self.shadow(b2);
                }
                self.rewrite_block(&mut f.body);
                self.pop_scope();
            }
            Stmt::Loop(l) => self.rewrite_block(&mut l.body),
            Stmt::Ret(e, _) => {
                if let Some(e) = e {
                    self.rewrite_expr(e);
                }
            }
            Stmt::Break(e, _) => {
                if let Some(e) = e {
                    self.rewrite_expr(e);
                }
            }
            Stmt::Match(m) => {
                self.rewrite_expr(&mut m.subject);
                for arm in &mut m.arms {
                    self.push_scope();
                    self.rewrite_pat(&mut arm.pat);
                    let mut binders = Vec::new();
                    collect_pat_binders(&arm.pat, &mut binders);
                    for b in binders {
                        self.shadow(b);
                    }
                    if let Some(ref mut g) = arm.guard {
                        self.rewrite_expr(g);
                    }
                    self.rewrite_block(&mut arm.body);
                    self.pop_scope();
                }
            }
            Stmt::ErrReturn(e, _) => self.rewrite_expr(e),
            Stmt::Defer(b, _) => self.rewrite_block(b),
            Stmt::StoreInsert(_, exprs, _) => {
                for fi in exprs {
                    self.rewrite_expr(&mut fi.value);
                }
            }
            Stmt::StoreSet(_, pairs, _, _) => {
                for (_, e) in pairs {
                    self.rewrite_expr(e);
                }
            }
            Stmt::Transaction(block, _) | Stmt::SimBlock(block, _) => self.rewrite_block(block),
            Stmt::Together(name, block, handler, _) => {
                self.push_scope();
                if let Some(n) = name {
                    self.shadow(*n);
                }
                for stmt in block.iter_mut() {
                    self.rewrite_stmt(stmt);
                }
                if let Some(ref mut e) = handler.ok_arm {
                    self.rewrite_expr(e);
                }
                if let Some(ref mut e) = handler.err_arm {
                    self.rewrite_expr(e);
                }
                self.pop_scope();
            }
            Stmt::SimFor(f, _) => {
                self.rewrite_expr(&mut f.iter);
                if let Some(ref mut e) = f.end {
                    self.rewrite_expr(e);
                }
                if let Some(ref mut e) = f.step {
                    self.rewrite_expr(e);
                }
                self.push_scope();
                self.shadow(f.bind);
                if let Some(b2) = f.bind2 {
                    self.shadow(b2);
                }
                self.rewrite_block(&mut f.body);
                self.pop_scope();
            }
            Stmt::ChannelClose(e, _) | Stmt::Stop(e, _) | Stmt::Join(e, _) => self.rewrite_expr(e),
            Stmt::Continue(_)
            | Stmt::Nop(_)
            | Stmt::Asm(_)
            | Stmt::StoreSave(_, _)
            | Stmt::StoreCompact(_, _)
            | Stmt::StoreDelete(_, _, _)
            | Stmt::StoreDestroy(_, _, _)
            | Stmt::StoreRestore(_, _, _)
            | Stmt::UseLocal(_) => {}
        }
    }

    fn rewrite_if(&mut self, i: &mut ast::If) {
        self.rewrite_expr(&mut i.cond);
        self.rewrite_block(&mut i.then);
        for (c, b) in &mut i.elifs {
            self.rewrite_expr(c);
            self.rewrite_block(b);
        }
        if let Some(ref mut b) = i.els {
            self.rewrite_block(b);
        }
    }

    fn rewrite_filter(&mut self, filter: &mut ast::StoreFilter) {
        self.rewrite_expr(&mut filter.value);
        for (_, cond) in &mut filter.extra {
            self.rewrite_expr(&mut cond.value);
        }
    }

    fn rewrite_expr(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Ident(name, _) => {
                if let Some(new) = self.lookup(*name) {
                    *name = new;
                }
            }
            Expr::Call(callee, args, _) => {
                self.rewrite_expr(callee);
                for a in args {
                    self.rewrite_expr(a);
                }
            }
            Expr::Method(obj, _, args, _) => {
                self.rewrite_expr(obj);
                for a in args {
                    self.rewrite_expr(a);
                }
            }
            Expr::Field(obj, _, _) => self.rewrite_expr(obj),
            Expr::BinOp(l, _, r, _) => {
                self.rewrite_expr(l);
                self.rewrite_expr(r);
            }
            Expr::UnaryOp(_, e, _) => self.rewrite_expr(e),
            Expr::Index(a, b, _) => {
                self.rewrite_expr(a);
                self.rewrite_expr(b);
            }
            Expr::Ternary(a, b, c, _) => {
                self.rewrite_expr(a);
                self.rewrite_expr(b);
                self.rewrite_expr(c);
            }
            Expr::Quaternary(subj, ok, nothing, err, _) => {
                self.rewrite_expr(subj);
                if let Some(ok) = ok {
                    self.rewrite_expr(ok);
                }
                if let Some(nothing) = nothing {
                    self.rewrite_expr(nothing);
                }
                if let Some(err) = err {
                    self.rewrite_expr(err);
                }
            }
            Expr::As(e, _, _)
            | Expr::Ref(e, _)
            | Expr::Deref(e, _)
            | Expr::Freeze(e, _)
            | Expr::Yield(e, _)
            | Expr::Grad(e, _)
            | Expr::StrictCast(e, _, _)
            | Expr::Spread(e, _) => {
                self.rewrite_expr(e);
            }
            Expr::Array(es, _) | Expr::Tuple(es, _) | Expr::Syscall(es, _) => {
                for e in es {
                    self.rewrite_expr(e);
                }
            }
            Expr::Struct(name, fields, span) => {
                if let Some(renamed) = self.lookup(*name) {
                    let mut args: Vec<Expr> = Vec::with_capacity(fields.len());
                    for fi in fields.drain(..) {
                        let mut v = fi.value;
                        self.rewrite_expr(&mut v);
                        match fi.name {
                            Some(n) => args.push(Expr::NamedArg(n, Box::new(v), *span)),
                            None => args.push(v),
                        }
                    }
                    *expr = Expr::Call(Box::new(Expr::Ident(renamed, *span)), args, *span);
                    return;
                }
                for f in fields {
                    self.rewrite_expr(&mut f.value);
                }
            }
            Expr::Builder(_, fields, _) => {
                for f in fields {
                    self.rewrite_expr(&mut f.value);
                }
            }
            Expr::IfExpr(i) => self.rewrite_if(i),
            Expr::Pipe(a, b, args, _) => {
                self.rewrite_expr(a);
                self.rewrite_expr(b);
                for e in args {
                    self.rewrite_expr(e);
                }
            }
            Expr::Block(block, _) => self.rewrite_block(block),
            Expr::Lambda(params, _, body, _) => {
                self.push_scope();
                for p in params.iter() {
                    self.shadow(p.name);
                }
                self.rewrite_block(body);
                self.pop_scope();
            }
            Expr::ListComp(body, binder, iter, cond, end, _) => {
                self.rewrite_expr(iter);
                if let Some(e) = end {
                    self.rewrite_expr(e);
                }
                self.push_scope();
                self.shadow(Symbol::intern(binder));
                self.rewrite_expr(body);
                if let Some(c) = cond {
                    self.rewrite_expr(c);
                }
                self.pop_scope();
            }
            Expr::Query(e, _, _) => self.rewrite_expr(e),
            Expr::Send(target, _, args, _) => {
                self.rewrite_expr(target);
                for a in args {
                    self.rewrite_expr(a);
                }
            }
            Expr::ChannelSend(a, b, _) => {
                self.rewrite_expr(a);
                self.rewrite_expr(b);
            }
            Expr::ChannelRecv(e, _) => self.rewrite_expr(e),
            Expr::ChannelCreate(_, e, _) => self.rewrite_expr(e),
            Expr::Select(arms, default, _) => {
                for arm in arms {
                    self.rewrite_expr(&mut arm.chan);
                    if let Some(ref mut v) = arm.value {
                        self.rewrite_expr(v);
                    }
                    self.push_scope();
                    if let Some(b) = arm.binding {
                        self.shadow(b);
                    }
                    self.rewrite_block(&mut arm.body);
                    self.pop_scope();
                }
                if let Some(b) = default {
                    self.rewrite_block(b);
                }
            }
            Expr::Slice(a, b, c, _) => {
                self.rewrite_expr(a);
                self.rewrite_expr(b);
                self.rewrite_expr(c);
            }
            Expr::OfCall(a, b, _) => {
                self.rewrite_expr(a);
                self.rewrite_expr(b);
            }
            Expr::NamedArg(_, e, _) => self.rewrite_expr(e),
            Expr::AsFormat(e, _, _) => self.rewrite_expr(e),
            Expr::Einsum(_, es, _) => {
                for e in es {
                    self.rewrite_expr(e);
                }
            }
            Expr::StoreGet(_, e, _) => self.rewrite_expr(e),
            Expr::StoreInsert(_, fis, _) => {
                for fi in fis {
                    self.rewrite_expr(&mut fi.value);
                }
            }
            Expr::StoreUpdate(_, pairs, filter, _) => {
                for (_, e) in pairs {
                    self.rewrite_expr(e);
                }
                self.rewrite_filter(filter);
            }
            Expr::StoreQuery(_, filter, _)
            | Expr::StoreFirst(_, filter, _)
            | Expr::StoreExists(_, filter, _) => self.rewrite_filter(filter),
            Expr::StoreCount(_, filter, _) => {
                if let Some(f) = filter {
                    self.rewrite_filter(f);
                }
            }
            Expr::Spawn(_, inits, _) => {
                for (_, e) in inits {
                    self.rewrite_expr(e);
                }
            }
            Expr::DispatchBlock(_, block, _) => self.rewrite_block(block),
            Expr::Receive(arms, _) => {
                for arm in arms {
                    self.push_scope();
                    for b in &arm.bindings {
                        self.shadow(*b);
                    }
                    self.rewrite_block(&mut arm.body);
                    self.pop_scope();
                }
            }

            Expr::None(_)
            | Expr::Void(_)
            | Expr::Int(_, _)
            | Expr::Float(_, _)
            | Expr::Str(_, _)
            | Expr::Bool(_, _)
            | Expr::Placeholder(_)
            | Expr::IndexPlaceholder(_)
            | Expr::Embed(_, _)
            | Expr::Unreachable(_)
            | Expr::StoreAll(_, _)
            | Expr::StoreDistinct(_, _, _)
            | Expr::QualifiedIdent(_, _, _) => {}
        }
    }
}

fn collect_pat_binders(pat: &Pat, out: &mut Vec<Symbol>) {
    match pat {
        Pat::Ident(s, _) => out.push(*s),
        Pat::Ctor(_, ps, _) | Pat::Or(ps, _) | Pat::Tuple(ps, _) | Pat::Array(ps, _) => {
            for p in ps {
                collect_pat_binders(p, out);
            }
        }
        Pat::Wild(_) | Pat::Lit(_) | Pat::Range(_, _, _) => {}
    }
}
