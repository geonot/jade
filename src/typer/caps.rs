use std::collections::{HashMap, HashSet};

use crate::ast::{self, CapAnnot};
use crate::cap_sites::{self, ExternCaps};
use crate::caps::{CapSet, Capability};
use crate::intern::Symbol;

use super::scc;

pub(in crate::typer) struct CapItem<'a> {
    pub name: Symbol,
    pub bare_method: Option<Symbol>,
    pub fun: &'a ast::Fn,
}

#[derive(Debug)]
pub(in crate::typer) struct CapAnalysis {
    #[allow(dead_code)]
    pub rows: HashMap<Symbol, CapSet>,
}

#[derive(Default)]
pub(in crate::typer) struct CapContext {
    pub store_names: HashSet<Symbol>,
    pub actor_items: HashMap<Symbol, Vec<Symbol>>,
}

fn parse_annot(a: &CapAnnot) -> Result<Capability, String> {
    let cap = match a.class.as_str() {
        "fs.read" => Capability::FsRead(a.scope.clone()),
        "fs.write" => Capability::FsWrite(a.scope.clone()),
        "net.client" => Capability::NetClient,
        "net.server" => Capability::NetServer,
        "process.spawn" => Capability::ProcessSpawn,
        "env.read" => Capability::EnvRead,
        "clock" => Capability::Clock,
        "time" => Capability::Time,
        "random" => Capability::Random,
        "ffi.unsafe" => Capability::FfiUnsafe,
        "state" => Capability::State(a.scope.clone()),
        other => {
            return Err(format!(
                "unknown capability class `{other}` at {:?}; the capability set is closed (caps.md §1.1)",
                a.span
            ));
        }
    };
    Ok(cap)
}

fn declared_set(needs: &[CapAnnot]) -> Result<CapSet, String> {
    let mut set = CapSet::new();
    for a in needs {
        set.insert(parse_annot(a)?);
    }
    Ok(set)
}

fn literal_str(e: Option<&ast::Expr>) -> Option<&str> {
    match e {
        Some(ast::Expr::Str(s, _)) => Some(s.as_str()),
        _ => None,
    }
}

#[derive(Default)]
struct Collected {
    caps: CapSet,
    callees: HashSet<Symbol>,
    method_calls: HashSet<Symbol>,
    suppressed: HashSet<Symbol>,
}

struct Scanner<'a> {
    out: Collected,
    trusted_apertures: &'a HashSet<Symbol>,
    param_names: HashSet<Symbol>,
    opaque_locals: HashSet<Symbol>,
    local_alias: HashMap<Symbol, Symbol>,
    ctx: &'a CapContext,
}

impl Scanner<'_> {
    fn store_op(&mut self, store: Symbol) {
        let path = Some(format!("./{store}.store"));
        self.out.caps.insert(Capability::FsRead(path.clone()));
        self.out.caps.insert(Capability::FsWrite(path));
    }

    fn classed_caps(&mut self, caps: &[(cap_sites::CapClass, Option<usize>)], args: &[ast::Expr]) {
        for (class, path_arg) in caps {
            let path = path_arg.and_then(|i| literal_str(args.get(i)));
            self.out.caps.insert(cap_sites::capability_of(*class, path));
        }
    }

    fn extern_call(&mut self, name: &str, args: &[ast::Expr]) {
        match cap_sites::classify_extern(name) {
            ExternCaps::Effect(caps) => self.classed_caps(caps, args),
            ExternCaps::OpenMode { path_arg, mode_arg } => {
                let path = literal_str(args.get(path_arg));
                let mode = literal_str(args.get(mode_arg));
                cap_sites::open_mode_caps(path, mode, &mut |c| {
                    self.out.caps.insert(c);
                });
            }
            ExternCaps::Benign => {}
            ExternCaps::Unknown => {
                self.out.caps.insert(Capability::FfiUnsafe);
            }
        }
    }

    fn module_call(&mut self, base: Symbol, method: Symbol, args: &[ast::Expr]) {
        let flattened = Symbol::intern(&format!("{base}_{method}"));
        if let Some(site) = cap_sites::aperture(&format!("{base}.{method}")) {
            self.classed_caps(site.caps, args);
            if let Some((path_arg, mode_arg)) = site.open_mode {
                let path = literal_str(args.get(path_arg));
                let mode = literal_str(args.get(mode_arg));
                cap_sites::open_mode_caps(path, mode, &mut |c| {
                    self.out.caps.insert(c);
                });
            }
            if self.trusted_apertures.contains(&flattened) {
                self.out.suppressed.insert(flattened);
                return;
            }
        }
        self.out.callees.insert(flattened);
    }

    fn scan_block(&mut self, block: &[ast::Stmt]) {
        for s in block {
            self.scan_stmt(s);
        }
    }

    fn scan_filter(&mut self, f: &ast::StoreFilter) {
        self.scan_expr(&f.value);
        for (_, c) in &f.extra {
            self.scan_expr(&c.value);
        }
    }

    fn scan_stmt(&mut self, s: &ast::Stmt) {
        match s {
            ast::Stmt::Bind(b) => {
                match &b.value {
                    ast::Expr::Lambda(..) => {
                        self.opaque_locals.remove(&b.name);
                        self.local_alias.remove(&b.name);
                    }
                    ast::Expr::Ident(src, _) => {
                        self.opaque_locals.remove(&b.name);
                        self.local_alias.insert(b.name, *src);
                    }
                    ast::Expr::Field(..)
                    | ast::Expr::Index(..)
                    | ast::Expr::Call(..)
                    | ast::Expr::Method(..) => {
                        self.local_alias.remove(&b.name);
                        self.opaque_locals.insert(b.name);
                    }
                    _ => {}
                }
                self.scan_expr(&b.value)
            }
            ast::Stmt::TupleBind(_, e, _)
            | ast::Stmt::Expr(e)
            | ast::Stmt::Ret(Some(e), _)
            | ast::Stmt::ErrReturn(e, _)
            | ast::Stmt::Break(Some(e), _)
            | ast::Stmt::ChannelClose(e, _)
            | ast::Stmt::Stop(e, _)
            | ast::Stmt::Join(e, _) => self.scan_expr(e),
            ast::Stmt::Assign(lhs, rhs, _) => {
                self.scan_expr(lhs);
                self.scan_expr(rhs);
            }
            ast::Stmt::If(i) => {
                self.scan_expr(&i.cond);
                self.scan_block(&i.then);
                for (c, b) in &i.elifs {
                    self.scan_expr(c);
                    self.scan_block(b);
                }
                if let Some(el) = &i.els {
                    self.scan_block(el);
                }
            }
            ast::Stmt::While(w) => {
                self.scan_expr(&w.cond);
                self.scan_block(&w.body);
            }
            ast::Stmt::For(f) | ast::Stmt::SimFor(f, _) => {
                self.scan_expr(&f.iter);
                if let Some(e) = &f.end {
                    self.scan_expr(e);
                }
                if let Some(e) = &f.step {
                    self.scan_expr(e);
                }
                self.scan_block(&f.body);
            }
            ast::Stmt::Loop(l) => self.scan_block(&l.body),
            ast::Stmt::Match(m) => {
                self.scan_expr(&m.subject);
                for arm in &m.arms {
                    if let Some(g) = &arm.guard {
                        self.scan_expr(g);
                    }
                    self.scan_block(&arm.body);
                }
            }
            ast::Stmt::Asm(_) => {
                self.out.caps.insert(Capability::FfiUnsafe);
            }
            ast::Stmt::Defer(b, _) | ast::Stmt::Transaction(b, _) | ast::Stmt::SimBlock(b, _) => {
                self.scan_block(b)
            }
            ast::Stmt::Together(_, b, handler, _) => {
                self.scan_block(b);
                if let Some(e) = &handler.ok_arm {
                    self.scan_expr(e);
                }
                if let Some(e) = &handler.err_arm {
                    self.scan_expr(e);
                }
            }
            ast::Stmt::StoreInsert(store, inits, _) => {
                self.store_op(*store);
                for fi in inits {
                    self.scan_expr(&fi.value);
                }
            }
            ast::Stmt::StoreDelete(store, f, _)
            | ast::Stmt::StoreDestroy(store, f, _)
            | ast::Stmt::StoreRestore(store, f, _) => {
                self.store_op(*store);
                self.scan_filter(f);
            }
            ast::Stmt::StoreSet(store, updates, f, _) => {
                self.store_op(*store);
                for (_, e) in updates {
                    self.scan_expr(e);
                }
                self.scan_filter(f);
            }
            ast::Stmt::StoreSave(store, _) | ast::Stmt::StoreCompact(store, _) => {
                self.store_op(*store);
            }
            ast::Stmt::Ret(None, _)
            | ast::Stmt::Break(None, _)
            | ast::Stmt::Continue(_)
            | ast::Stmt::Nop(_)
            | ast::Stmt::UseLocal(_) => {}
        }
    }

    fn scan_expr(&mut self, e: &ast::Expr) {
        match e {
            ast::Expr::Call(callee, args, _) => {
                match callee.as_ref() {
                    ast::Expr::Ident(name, _) => {
                        if self.param_names.contains(name) || self.opaque_locals.contains(name) {
                            self.out.caps.insert(Capability::IndirectCall);
                        } else if let Some(src) = self.local_alias.get(name) {
                            let src = *src;
                            self.out.callees.insert(src);
                        } else {
                            self.out.callees.insert(*name);
                        }
                    }
                    ast::Expr::Field(..) | ast::Expr::Index(..) => {
                        self.out.caps.insert(Capability::IndirectCall);
                        self.scan_expr(callee);
                    }
                    _ => {
                        self.scan_expr(callee);
                    }
                }
                for a in args {
                    self.scan_expr(a);
                }
            }
            ast::Expr::Method(recv, method, args, _) => {
                if let ast::Expr::Ident(base, _) = recv.as_ref() {
                    if base.as_str() == "extern" {
                        self.extern_call(&method.as_str(), args);
                    } else {
                        self.module_call(*base, *method, args);
                        self.out.method_calls.insert(*method);
                    }
                } else {
                    self.out.method_calls.insert(*method);
                    self.scan_expr(recv);
                }
                for a in args {
                    self.scan_expr(a);
                }
            }
            ast::Expr::Pipe(recv, func, args, _) => {
                if let ast::Expr::Ident(name, _) = func.as_ref() {
                    self.out.callees.insert(*name);
                } else {
                    self.scan_expr(func);
                }
                self.scan_expr(recv);
                for a in args {
                    self.scan_expr(a);
                }
            }
            ast::Expr::Syscall(args, _) => {
                self.out.caps.insert(Capability::FfiUnsafe);
                for a in args {
                    self.scan_expr(a);
                }
            }
            ast::Expr::BinOp(l, _, r, _)
            | ast::Expr::Index(l, r, _)
            | ast::Expr::ChannelSend(l, r, _)
            | ast::Expr::OfCall(l, r, _) => {
                self.scan_expr(l);
                self.scan_expr(r);
            }
            ast::Expr::UnaryOp(_, e, _)
            | ast::Expr::As(e, _, _)
            | ast::Expr::StrictCast(e, _, _)
            | ast::Expr::Ref(e, _)
            | ast::Expr::Deref(e, _)
            | ast::Expr::Freeze(e, _)
            | ast::Expr::Yield(e, _)
            | ast::Expr::ChannelRecv(e, _)
            | ast::Expr::Field(e, _, _)
            | ast::Expr::AsFormat(e, _, _)
            | ast::Expr::NamedArg(_, e, _)
            | ast::Expr::Spread(e, _)
            | ast::Expr::Grad(e, _)
            | ast::Expr::ChannelCreate(_, e, _) => self.scan_expr(e),
            ast::Expr::StoreGet(store, e, _) => {
                self.store_op(*store);
                self.scan_expr(e);
            }
            ast::Expr::Ternary(c, t, f, _) => {
                self.scan_expr(c);
                self.scan_expr(t);
                self.scan_expr(f);
            }
            ast::Expr::Quaternary(s, ok, no, er, _) => {
                self.scan_expr(s);
                for x in [ok, no, er].into_iter().flatten() {
                    self.scan_expr(x);
                }
            }
            ast::Expr::Slice(a, b, c, _) => {
                self.scan_expr(a);
                self.scan_expr(b);
                self.scan_expr(c);
            }
            ast::Expr::Array(es, _) | ast::Expr::Tuple(es, _) | ast::Expr::Einsum(_, es, _) => {
                for x in es {
                    self.scan_expr(x);
                }
            }
            ast::Expr::Struct(_, fields, _) => {
                for fl in fields {
                    self.scan_expr(&fl.value);
                }
            }
            ast::Expr::Builder(_, fields, _) => {
                for fl in fields {
                    self.scan_expr(&fl.value);
                }
            }
            ast::Expr::IfExpr(i) => {
                self.scan_expr(&i.cond);
                self.scan_block(&i.then);
                for (c, b) in &i.elifs {
                    self.scan_expr(c);
                    self.scan_block(b);
                }
                if let Some(el) = &i.els {
                    self.scan_block(el);
                }
            }
            ast::Expr::Block(b, _) | ast::Expr::DispatchBlock(_, b, _) => self.scan_block(b),
            ast::Expr::Lambda(params, _, body, _) => {
                for p in params {
                    if let Some(d) = &p.default {
                        self.scan_expr(d);
                    }
                }
                self.scan_block(body);
            }
            ast::Expr::ListComp(body, _, iter, cond, map, _) => {
                self.scan_expr(body);
                self.scan_expr(iter);
                if let Some(c) = cond {
                    self.scan_expr(c);
                }
                if let Some(m) = map {
                    self.scan_expr(m);
                }
            }
            ast::Expr::Query(subject, clauses, _) => {
                if let ast::Expr::Ident(n, _) = subject.as_ref()
                    && self.ctx.store_names.contains(n)
                {
                    self.store_op(*n);
                }
                self.scan_expr(subject);
                for cl in clauses {
                    match cl {
                        ast::QueryClause::Where(e, _)
                        | ast::QueryClause::Limit(e, _)
                        | ast::QueryClause::Take(e, _)
                        | ast::QueryClause::Skip(e, _)
                        | ast::QueryClause::Set(_, e, _) => self.scan_expr(e),
                        ast::QueryClause::Sort(_, _, _)
                        | ast::QueryClause::Delete(_)
                        | ast::QueryClause::Group(_, _)
                        | ast::QueryClause::Select(_, _) => {}
                    }
                }
            }
            ast::Expr::StoreQuery(store, f, _)
            | ast::Expr::StoreFirst(store, f, _)
            | ast::Expr::StoreExists(store, f, _) => {
                self.store_op(*store);
                self.scan_filter(f);
            }
            ast::Expr::StoreCount(store, f, _) => {
                self.store_op(*store);
                if let Some(f) = f {
                    self.scan_filter(f);
                }
            }
            ast::Expr::StoreInsert(store, inits, _) => {
                self.store_op(*store);
                for fi in inits {
                    self.scan_expr(&fi.value);
                }
            }
            ast::Expr::StoreUpdate(store, updates, f, _) => {
                self.store_op(*store);
                for (_, e) in updates {
                    self.scan_expr(e);
                }
                self.scan_filter(f);
            }
            ast::Expr::Spawn(actor, inits, _) => {
                if let Some(handler_items) = self.ctx.actor_items.get(actor) {
                    for h in handler_items {
                        self.out.callees.insert(*h);
                    }
                }
                for (_, e) in inits {
                    self.scan_expr(e);
                }
            }
            ast::Expr::Send(recv, handler, args, _) => {
                self.out.method_calls.insert(*handler);
                self.scan_expr(recv);
                for a in args {
                    self.scan_expr(a);
                }
            }
            ast::Expr::Receive(arms, _) => {
                for arm in arms {
                    self.scan_block(&arm.body);
                }
            }
            ast::Expr::Select(arms, default, _) => {
                for arm in arms {
                    self.scan_expr(&arm.chan);
                    if let Some(v) = &arm.value {
                        self.scan_expr(v);
                    }
                    self.scan_block(&arm.body);
                }
                if let Some(b) = default {
                    self.scan_block(b);
                }
            }
            ast::Expr::StoreAll(store, _) | ast::Expr::StoreDistinct(store, _, _) => {
                self.store_op(*store);
            }
            ast::Expr::None(_)
            | ast::Expr::Void(_)
            | ast::Expr::Int(_, _)
            | ast::Expr::Float(_, _)
            | ast::Expr::Str(_, _)
            | ast::Expr::Bool(_, _)
            | ast::Expr::Ident(_, _)
            | ast::Expr::Placeholder(_)
            | ast::Expr::IndexPlaceholder(_)
            | ast::Expr::Embed(_, _)
            | ast::Expr::Unreachable(_)
            | ast::Expr::QualifiedIdent(_, _, _) => {}
        }
    }
}

pub(in crate::typer) fn analyze(
    items: &[CapItem],
    std_files: &HashSet<Symbol>,
    ctx: &CapContext,
) -> Result<CapAnalysis, String> {
    let item_lookup: HashMap<Symbol, &CapItem> = items.iter().map(|it| (it.name, it)).collect();

    let mut method_index: HashMap<Symbol, Vec<Symbol>> = HashMap::new();
    for it in items {
        if let Some(m) = it.bare_method {
            method_index.entry(m).or_default().push(it.name);
        }
    }

    let mut trusted_apertures: HashSet<Symbol> = HashSet::new();
    for site in cap_sites::APERTURE_SITES {
        let flattened = Symbol::intern(&site.symbol.replace('.', "_"));
        if let Some(it) = item_lookup.get(&flattened)
            && it.fun.span.file.is_some_and(|f| std_files.contains(&f))
        {
            trusted_apertures.insert(flattened);
        }
    }

    let mut primitive: HashMap<Symbol, CapSet> = HashMap::new();
    let mut graph: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
    for it in items {
        let mut scanner = Scanner {
            out: Collected::default(),
            trusted_apertures: &trusted_apertures,
            param_names: it.fun.params.iter().map(|p| p.name).collect(),
            opaque_locals: HashSet::new(),
            local_alias: HashMap::new(),
            ctx,
        };
        scanner.scan_block(&it.fun.body);
        let mut collected = scanner.out;
        let mut edges: HashSet<Symbol> = collected
            .callees
            .drain()
            .filter(|c| item_lookup.contains_key(c))
            .collect();
        for m in &collected.method_calls {
            if let Some(targets) = method_index.get(m) {
                for t in targets {
                    if !collected.suppressed.contains(t) {
                        edges.insert(*t);
                    }
                }
            }
        }
        primitive.insert(it.name, collected.caps);
        graph.insert(it.name, edges);
    }

    let sccs = scc::tarjan_scc(&graph);
    let mut rows: HashMap<Symbol, CapSet> = HashMap::new();
    for scc_group in &sccs {
        loop {
            let mut changed = false;
            for name in scc_group {
                let mut acc = primitive.get(name).cloned().unwrap_or_default();
                if let Some(callees) = graph.get(name) {
                    let mut sorted: Vec<&Symbol> = callees.iter().collect();
                    sorted.sort();
                    for c in sorted {
                        if let Some(r) = rows.get(c) {
                            acc.join(r);
                        }
                    }
                }
                let entry = rows.entry(*name).or_default();
                let before = entry.len();
                entry.join(&acc);
                if entry.len() != before {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    for it in items {
        let Some(needs) = &it.fun.needs else { continue };
        let declared = declared_set(needs)?;
        let derived = rows.get(&it.name).cloned().unwrap_or_default();
        let uncovered = derived.contained_in(&declared);
        if let Some(bad) = uncovered.first() {
            let path = shortest_intro_path(it.name, bad, &graph, &primitive, &item_lookup);
            return Err(format!(
                "{}: function `{}` declares `needs {}` but uses `{}`{}",
                it.fun.span.loc(),
                it.name,
                declared.render(),
                bad.render(),
                path,
            ));
        }
    }

    Ok(CapAnalysis { rows })
}

fn shortest_intro_path(
    start: Symbol,
    cap: &Capability,
    graph: &HashMap<Symbol, HashSet<Symbol>>,
    primitive: &HashMap<Symbol, CapSet>,
    item_lookup: &HashMap<Symbol, &CapItem>,
) -> String {
    use std::collections::VecDeque;
    let introduces = |n: &Symbol| primitive.get(n).map(|s| s.satisfies(cap)).unwrap_or(false);
    let mut q: VecDeque<Vec<Symbol>> = VecDeque::new();
    let mut seen: HashSet<Symbol> = HashSet::new();
    q.push_back(vec![start]);
    seen.insert(start);
    while let Some(path) = q.pop_front() {
        let last = *path.last().unwrap();
        if introduces(&last) {
            let chain = path
                .iter()
                .map(|s| s.as_str().replace("__handler_", "."))
                .collect::<Vec<_>>()
                .join(" -> ");
            return format!(" (introduced via {chain})");
        }
        if let Some(callees) = graph.get(&last) {
            let mut sorted: Vec<&Symbol> = callees.iter().collect();
            sorted.sort();
            for c in sorted {
                if item_lookup.contains_key(c) && seen.insert(*c) {
                    let mut next = path.clone();
                    next.push(*c);
                    q.push_back(next);
                }
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn parse(src: &str) -> Vec<ast::Decl> {
        let t = Lexer::new(src).tokenize().expect("lex");
        Parser::new(t).parse_program().expect("parse").decls
    }

    fn items_of(decls: &[ast::Decl]) -> Vec<CapItem<'_>> {
        let mut items = Vec::new();
        for d in decls {
            match d {
                ast::Decl::Fn(f) => items.push(CapItem {
                    name: f.name,
                    bare_method: None,
                    fun: f,
                }),
                ast::Decl::Type(td) => {
                    for m in &td.methods {
                        items.push(CapItem {
                            name: Symbol::intern(&format!("{}_{}", td.name, m.name)),
                            bare_method: Some(m.name),
                            fun: m,
                        });
                    }
                }
                ast::Decl::Impl(ib) => {
                    for m in &ib.methods {
                        items.push(CapItem {
                            name: Symbol::intern(&format!("{}_{}", ib.type_name, m.name)),
                            bare_method: Some(m.name),
                            fun: m,
                        });
                    }
                }
                _ => {}
            }
        }
        items
    }

    fn run(src: &str) -> Result<CapAnalysis, String> {
        let decls = parse(src);
        let items = items_of(&decls);
        analyze(&items, &HashSet::new(), &CapContext::default())
    }

    #[test]
    fn pure_fn_infers_empty() {
        let a = run("*add a, b returns i64\n    a + b\n").unwrap();
        let row = a.rows.get(&Symbol::intern("add")).unwrap();
        assert!(row.is_empty());
    }

    #[test]
    fn declared_pure_passes() {
        assert!(run("*add a, b returns i64 needs pure\n    a + b\n").is_ok());
    }

    #[test]
    fn unknown_cap_class_rejected() {
        let err = run("*f needs teleport\n    1\n").unwrap_err();
        assert!(err.contains("unknown capability"));
    }

    #[test]
    fn extern_site_introduces_cap() {
        let a = run("*f\n    extern.connect(fd, addr, len)\n").unwrap();
        let row = a.rows.get(&Symbol::intern("f")).unwrap();
        assert!(row.satisfies(&Capability::NetClient));
    }

    #[test]
    fn unknown_extern_is_ffi_unsafe() {
        let a = run("*f\n    extern.launch_missiles()\n").unwrap();
        let row = a.rows.get(&Symbol::intern("f")).unwrap();
        assert!(row.satisfies(&Capability::FfiUnsafe));
    }

    #[test]
    fn benign_extern_introduces_nothing() {
        let a = run("*f\n    extern.memcpy(a, b, n)\n").unwrap();
        let row = a.rows.get(&Symbol::intern("f")).unwrap();
        assert!(row.is_empty());
    }

    #[test]
    fn cap_propagates_through_call_graph() {
        let src = "*lo\n    extern.connect(fd, addr, len)\n*hi\n    lo()\n";
        let a = run(src).unwrap();
        let hi = a.rows.get(&Symbol::intern("hi")).unwrap();
        assert!(hi.satisfies(&Capability::NetClient));
    }

    #[test]
    fn declared_narrower_than_derived_is_rejected() {
        let src = "*f needs fs.read\n    extern.connect(fd, addr, len)\n";
        let err = run(src).unwrap_err();
        assert!(err.contains("net.client"), "diag: {err}");
        assert!(err.contains("introduced via"), "diag: {err}");
    }

    #[test]
    fn needs_pure_rejects_aperture_write() {
        let src = "*sneaky returns i64 needs pure\n    io.write_file('canary.txt', 'x')\n    0\n";
        let err = run(src).unwrap_err();
        assert!(err.contains("fs.write 'canary.txt'"), "diag: {err}");
    }

    #[test]
    fn scoped_write_satisfies_wider_needs() {
        let src = "*f needs fs.write './out'\n    io.write_file('./out/log', data)\n";
        assert!(run(src).is_ok());
    }

    #[test]
    fn scoped_write_outside_declared_scope_rejected() {
        let src = "*f needs fs.write './out'\n    io.write_file('./etc', data)\n";
        assert!(run(src).is_err());
    }

    #[test]
    fn non_literal_path_is_unscoped() {
        let src = "*f needs fs.write './out'\n    io.write_file(p, data)\n";
        let err = run(src).unwrap_err();
        assert!(err.contains("fs.write"), "diag: {err}");
    }

    #[test]
    fn method_bucket_propagates_caps() {
        let src = "type W\n    n as i64\n\n    *hit\n        extern.connect(fd, addr, len)\n\n*f w needs pure\n    w.hit()\n";
        let err = run(src).unwrap_err();
        assert!(err.contains("net.client"), "diag: {err}");
    }

    #[test]
    fn generic_fn_bodies_are_scanned() {
        let src =
            "*leak of T(x as T)\n    extern.connect(fd, addr, len)\n*f needs pure\n    leak(1)\n";
        let err = run(src).unwrap_err();
        assert!(err.contains("net.client"), "diag: {err}");
    }

    #[test]
    fn syscall_is_ffi_unsafe() {
        let src = "*f needs pure\n    syscall(60, 0)\n";
        let err = run(src).unwrap_err();
        assert!(err.contains("ffi.unsafe"), "diag: {err}");
    }

    #[test]
    fn lambda_bodies_are_scanned() {
        let src = "*f needs pure\n    g is |x| extern.connect(x, addr, len)\n    g(1)\n";
        let err = run(src).unwrap_err();
        assert!(err.contains("net.client"), "diag: {err}");
    }

    #[test]
    fn env_write_is_state_env() {
        let src = "*f needs pure\n    extern.setenv(k, v, 1)\n";
        let err = run(src).unwrap_err();
        assert!(err.contains("state"), "diag: {err}");
    }

    #[test]
    fn aperture_row_replaces_trusted_std_body() {
        let src = "\
*io_write_file(path as String, content as String) returns bool
    extern.connect(fd, addr, len)
    true

*f needs fs.write './out'
    io.write_file('./out/log', data)
";
        let decls = parse(src);
        let file = Symbol::intern("std/io.jn");
        let mut std_files = HashSet::new();
        std_files.insert(file);
        let items = items_of(&decls);
        assert!(
            analyze(&items, &std_files, &CapContext::default()).is_err(),
            "without provenance the body's caps must join through the edge"
        );
        let mut owned: Vec<ast::Decl> = decls.clone();
        for d in &mut owned {
            if let ast::Decl::Fn(f) = d {
                f.span.file = Some(file);
            }
        }
        let items3 = items_of(&owned);
        assert!(analyze(&items3, &std_files, &CapContext::default()).is_ok());
    }
}
