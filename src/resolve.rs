use crate::ast::{self, Decl, Expr, Pat, Stmt};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::{HashMap, HashSet};

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
    let (types, ambiguous) = (HashMap::new(), HashSet::new());
    let mut r = Renamer::new(&rename_map, &types, &ambiguous);
    decls
        .into_iter()
        .map(|mut d| {
            match &mut d {
                Decl::Fn(f) => {
                    if let Some(new) = rename_map.get(&f.name) {
                        f.name = *new;
                    }
                }
                Decl::Const(name, _, _) => {
                    if let Some(new) = rename_map.get(name) {
                        *name = *new;
                    }
                }
                _ => {}
            }
            r.rewrite_decl(&mut d);
            d
        })
        .collect()
}

pub fn qualify_colliding_types(prog: &mut ast::Program) -> Result<(), String> {
    let mut sites: HashMap<Symbol, Vec<(Option<Symbol>, ast::Span)>> = HashMap::new();
    let mut order: Vec<Symbol> = Vec::new();
    let mut note = |name: Symbol, file: Option<Symbol>, span: ast::Span| {
        let v = sites.entry(name).or_insert_with(|| {
            order.push(name);
            Vec::new()
        });
        if !v.iter().any(|(f, _)| *f == file) {
            v.push((file, span));
        }
    };
    for d in &prog.decls {
        match d {
            Decl::Type(td) => note(td.name, td.span.file, td.span),
            Decl::Enum(ed) => {
                note(ed.name, ed.span.file, ed.span);
                for v in &ed.variants {
                    note(v.name, ed.span.file, v.span);
                }
            }
            _ => {}
        }
    }
    let colliding: Vec<Symbol> = order.into_iter().filter(|n| sites[n].len() > 1).collect();
    if colliding.is_empty() {
        return Ok(());
    }
    let mut tags: HashMap<Option<Symbol>, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut maps: HashMap<Option<Symbol>, HashMap<Symbol, Symbol>> = HashMap::new();
    for name in &colliding {
        for (file, _) in &sites[name] {
            let tag = tags
                .entry(*file)
                .or_insert_with(|| {
                    let stem: String = file
                        .map(|f| {
                            std::path::Path::new(&*f.as_str())
                                .file_stem()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        })
                        .unwrap_or_default()
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric())
                        .collect();
                    let stem = if stem.is_empty() {
                        "m".to_string()
                    } else {
                        stem
                    };
                    let mut tag = stem.clone();
                    let mut k = 2;
                    while !used.insert(tag.clone()) {
                        tag = format!("{stem}{k}");
                        k += 1;
                    }
                    tag
                })
                .clone();
            maps.entry(*file)
                .or_default()
                .insert(*name, Symbol::intern(&format!("{name}__M_{tag}")));
        }
    }
    let ambiguous: HashSet<Symbol> = colliding.iter().copied().collect();
    let (renames, empty) = (HashMap::new(), HashMap::new());
    for d in &mut prog.decls {
        let span = d.span();
        let types = maps.get(&span.file).unwrap_or(&empty);
        let mut r = Renamer::new(&renames, types, &ambiguous);
        r.rewrite_decl(d);
        if let Some(name) = r.hits.first() {
            let decls: Vec<String> = sites[name].iter().map(|(_, s)| s.loc()).collect();
            return Err(format!(
                "{}: `{}` is ambiguous — it is declared by more than one module ({}) and \
                 this module declares none of them; each module sees its own declaration, \
                 so use the value through its module's functions or rename one declaration",
                span.loc(),
                name,
                decls.join(", ")
            ));
        }
    }
    Ok(())
}

struct Renamer<'a> {
    renames: &'a HashMap<Symbol, Symbol>,
    types: &'a HashMap<Symbol, Symbol>,
    ambiguous: &'a HashSet<Symbol>,
    hits: Vec<Symbol>,
    scopes: Vec<Vec<Symbol>>,
    shadowed: HashMap<Symbol, u32>,
}

impl<'a> Renamer<'a> {
    fn new(
        renames: &'a HashMap<Symbol, Symbol>,
        types: &'a HashMap<Symbol, Symbol>,
        ambiguous: &'a HashSet<Symbol>,
    ) -> Self {
        Renamer {
            renames,
            types,
            ambiguous,
            hits: Vec::new(),
            scopes: Vec::new(),
            shadowed: HashMap::new(),
        }
    }

    fn nominal(&mut self, n: &mut Symbol) {
        if let Some(new) = self.types.get(n) {
            *n = *new;
        } else if self.ambiguous.contains(n) {
            self.hits.push(*n);
        }
    }

    fn value_name(&mut self, n: &mut Symbol) {
        if self.shadowed.contains_key(n) {
            return;
        }
        if let Some(new) = self.renames.get(n) {
            *n = *new;
        } else {
            self.nominal(n);
        }
    }

    fn rewrite_type(&mut self, t: &mut Type) {
        match t {
            Type::Struct(n, args) => {
                self.nominal(n);
                for a in args {
                    self.rewrite_type(a);
                }
            }
            Type::Enum(n) => self.nominal(n),
            Type::Alias(_, i)
            | Type::Newtype(_, i)
            | Type::Array(i, _)
            | Type::Vec(i)
            | Type::Ptr(i)
            | Type::Coroutine(i)
            | Type::Channel(i)
            | Type::Generator(i)
            | Type::View(i)
            | Type::Frozen(i) => self.rewrite_type(i),
            Type::Map(k, v) => {
                self.rewrite_type(k);
                self.rewrite_type(v);
            }
            Type::Tuple(ts) => {
                for t in ts {
                    self.rewrite_type(t);
                }
            }
            Type::Fn(ps, r) => {
                for p in ps {
                    self.rewrite_type(p);
                }
                self.rewrite_type(r);
            }
            _ => {}
        }
    }

    fn rewrite_params(&mut self, params: &mut [ast::Param]) {
        for p in params {
            if let Some(t) = &mut p.ty {
                self.rewrite_type(t);
            }
        }
    }

    fn rewrite_fn(&mut self, f: &mut ast::Fn) {
        self.rewrite_params(&mut f.params);
        if let Some(t) = &mut f.ret {
            self.rewrite_type(t);
        }
        for t in &mut f.error_types {
            self.rewrite_type(t);
        }
        let pnames: Vec<Symbol> = f.params.iter().map(|p| p.name).collect();
        self.in_fn_scope(&pnames, |r| {
            r.rewrite_block(&mut f.body);
            for p in &mut f.params {
                if let Some(def) = &mut p.default {
                    r.rewrite_expr(def);
                }
            }
        });
    }

    fn rewrite_fields(&mut self, fields: &mut [ast::Field]) {
        for fld in fields {
            if let Some(t) = &mut fld.ty {
                self.rewrite_type(t);
            }
            if let Some(def) = &mut fld.default {
                self.rewrite_expr(def);
            }
        }
    }

    fn rewrite_decl(&mut self, d: &mut Decl) {
        match d {
            Decl::Fn(f) => self.rewrite_fn(f),
            Decl::Const(_, e, _) | Decl::Global(_, e, _) => self.rewrite_expr(e),
            Decl::Type(td) => {
                self.nominal(&mut td.name);
                self.rewrite_fields(&mut td.fields);
                for m in &mut td.methods {
                    self.rewrite_fn(m);
                }
            }
            Decl::Enum(ed) => {
                self.nominal(&mut ed.name);
                for v in &mut ed.variants {
                    self.nominal(&mut v.name);
                    for f in &mut v.fields {
                        self.rewrite_type(&mut f.ty);
                    }
                }
            }
            Decl::ErrDef(ed) => {
                for v in &mut ed.variants {
                    for t in &mut v.fields {
                        self.rewrite_type(t);
                    }
                }
            }
            Decl::Impl(ib) => {
                self.nominal(&mut ib.type_name);
                for t in &mut ib.trait_type_args {
                    self.rewrite_type(t);
                }
                for (_, t) in &mut ib.assoc_type_bindings {
                    self.rewrite_type(t);
                }
                for m in &mut ib.methods {
                    self.rewrite_fn(m);
                }
            }
            Decl::Actor(ad) => {
                self.rewrite_fields(&mut ad.fields);
                for h in &mut ad.handlers {
                    self.rewrite_params(&mut h.params);
                    let pnames: Vec<Symbol> = h.params.iter().map(|p| p.name).collect();
                    self.in_fn_scope(&pnames, |r| {
                        if let Some(e) = &mut h.loop_sleep_ms {
                            r.rewrite_expr(e);
                        }
                        r.rewrite_block(&mut h.body);
                    });
                }
            }
            Decl::Trait(td) => {
                for m in &mut td.methods {
                    self.rewrite_params(&mut m.params);
                    if let Some(t) = &mut m.ret {
                        self.rewrite_type(t);
                    }
                    for t in &mut m.error_types {
                        self.rewrite_type(t);
                    }
                    if let Some(body) = &mut m.default_body {
                        let pnames: Vec<Symbol> = m.params.iter().map(|p| p.name).collect();
                        self.in_fn_scope(&pnames, |r| r.rewrite_block(body));
                    }
                }
            }
            Decl::Extern(ef) => {
                for (_, t) in &mut ef.params {
                    self.rewrite_type(t);
                }
                self.rewrite_type(&mut ef.ret);
            }
            Decl::Store(sd) => {
                for f in &mut sd.fields {
                    if let Some(t) = &mut f.ty {
                        self.rewrite_type(t);
                    }
                    if let Some(def) = &mut f.default {
                        self.rewrite_expr(def);
                    }
                }
                for m in &mut sd.methods {
                    self.rewrite_fn(m);
                }
            }
            Decl::TypeAlias(_, t, _) | Decl::Newtype(_, t, _) => self.rewrite_type(t),
            Decl::Test(tb) => self.in_fn_scope(&[], |r| r.rewrite_block(&mut tb.body)),
            Decl::TopStmt(st) => self.in_fn_scope(&[], |r| r.rewrite_stmt(st)),
            Decl::Use(_) | Decl::Supervisor(_) | Decl::Migration(_) | Decl::View(_) => {}
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
            Pat::Ident(s, _) => self.value_name(s),
            Pat::Ctor(name, ps, _) => {
                self.nominal(name);
                for p in ps {
                    self.rewrite_pat(p);
                }
            }
            Pat::Or(ps, _) | Pat::Tuple(ps, _) | Pat::Array(ps, _) => {
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
                if let Some(t) = &mut b.ty {
                    self.rewrite_type(t);
                }
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
            Expr::Ident(name, _) => self.value_name(name),
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
            Expr::As(e, t, _) | Expr::StrictCast(e, t, _) => {
                self.rewrite_type(t);
                self.rewrite_expr(e);
            }
            Expr::Ref(e, _)
            | Expr::Deref(e, _)
            | Expr::Freeze(e, _)
            | Expr::Yield(e, _)
            | Expr::Grad(e, _)
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
                self.nominal(name);
                for f in fields {
                    self.rewrite_expr(&mut f.value);
                }
            }
            Expr::Builder(name, fields, _) => {
                self.nominal(name);
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
            Expr::Lambda(params, ret, body, _) => {
                self.rewrite_params(params);
                if let Some(t) = ret {
                    self.rewrite_type(t);
                }
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
            Expr::ChannelCreate(t, e, _) => {
                if let Some(t) = t {
                    self.rewrite_type(t);
                }
                self.rewrite_expr(e);
            }
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
            | Expr::StoreDistinct(_, _, _) => {}
            Expr::QualifiedIdent(a, b, _) => {
                self.nominal(a);
                self.nominal(b);
            }
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
