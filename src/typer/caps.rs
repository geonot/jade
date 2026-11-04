use std::collections::{HashMap, HashSet, VecDeque};

use crate::ast::{self, CapAnnot};
use crate::cap_sites::{self, ExternCaps};
use crate::caps::{CapSet, Capability};
use crate::hir::{self, DefId, Expr, ExprKind, Stmt};
use crate::intern::Symbol;
use crate::types::Type;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Callable {
    Function(DefId),
    Handler(DefId, u32),
}

struct Item<'a> {
    id: Callable,
    name: String,
    body: &'a hir::Block,
    needs: Option<&'a [CapAnnot]>,
    span: ast::Span,
    extra: Vec<&'a Expr>,
}

#[derive(Default)]
struct Collected {
    caps: CapSet,
    callees: HashSet<Callable>,
}

struct Context<'a> {
    functions: HashMap<DefId, &'a hir::Fn>,
    methods: HashMap<Symbol, DefId>,
    externs: HashMap<DefId, Symbol>,
    apertures: HashMap<DefId, &'static cap_sites::ApertureSite>,
    actors: HashMap<Symbol, &'a hir::ActorDef>,
    stores: Vec<&'a hir::StoreDef>,
    globals: HashMap<Symbol, &'a Expr>,
    types: HashMap<Symbol, &'a hir::TypeDef>,
    enums: HashMap<Symbol, &'a hir::EnumDef>,
}

struct Scanner<'a, 'p> {
    ctx: &'a Context<'p>,
    out: Collected,
    globals_seen: HashSet<Symbol>,
}

fn parse_annot(a: &CapAnnot) -> Result<Capability, String> {
    Ok(match a.class.as_str() {
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
                "{}: unknown capability class `{other}`; the capability set is closed",
                a.span.loc()
            ));
        }
    })
}

fn declared_set(needs: &[CapAnnot]) -> Result<CapSet, String> {
    needs.iter().map(parse_annot).collect()
}

pub(in crate::typer) fn validate_annotations(prog: &ast::Program) -> Result<(), String> {
    for decl in &prog.decls {
        let functions: &[ast::Fn] = match decl {
            ast::Decl::Fn(f) => std::slice::from_ref(f),
            ast::Decl::Type(t) => &t.methods,
            ast::Decl::Impl(i) => &i.methods,
            ast::Decl::Store(s) => &s.methods,
            _ => &[],
        };
        for f in functions {
            if let Some(needs) = &f.needs {
                declared_set(needs)?;
            }
        }
    }
    Ok(())
}

fn literal_str(e: Option<&Expr>) -> Option<&str> {
    match e.map(|e| &e.kind) {
        Some(ExprKind::Str(s)) => Some(s),
        _ => None,
    }
}

impl Scanner<'_, '_> {
    fn store(&mut self, name: Symbol) {
        let path = Some(format!("./{name}.store"));
        self.out.caps.insert(Capability::FsRead(path.clone()));
        self.out.caps.insert(Capability::FsWrite(path));
    }

    fn store_hooks(&mut self, name: Symbol, insert: bool) {
        if let Some(store) = self.ctx.stores.iter().find(|s| s.name == name) {
            for decorator in &store.decorators {
                let hook = match decorator {
                    ast::StoreDecorator::BeforeInsert(f) | ast::StoreDecorator::AfterInsert(f)
                        if insert =>
                    {
                        Some(*f)
                    }
                    ast::StoreDecorator::BeforeDelete(f) | ast::StoreDecorator::AfterDelete(f)
                        if !insert =>
                    {
                        Some(*f)
                    }
                    _ => None,
                };
                if let Some(hook) = hook {
                    self.method(hook, &[]);
                }
            }
        }
    }

    fn store_delete(&mut self, name: Symbol) {
        let mut pending = vec![name];
        let mut seen = HashSet::new();
        while let Some(parent) = pending.pop() {
            if !seen.insert(parent) {
                continue;
            }
            self.store(parent);
            self.store_hooks(parent, false);
            for store in &self.ctx.stores {
                for rel in &store.relations {
                    let (owner, child) = if rel.is_has_many {
                        (store.name, rel.target)
                    } else {
                        (rel.target, store.name)
                    };
                    if rel.cascade && owner == parent {
                        pending.push(child);
                    }
                }
            }
        }
    }

    fn drop_type(&mut self, ty: &Type, seen: &mut HashSet<Symbol>) {
        match ty {
            Type::Struct(name, _) | Type::Enum(name) => {
                if !seen.insert(*name) {
                    return;
                }
                if let Some(t) = self.ctx.types.get(name) {
                    if t.layout.resource {
                        let method = Symbol::intern(&format!("{name}_drop"));
                        if self.ctx.methods.contains_key(&method) {
                            self.method(method, &[]);
                        }
                    }
                    for field in &t.fields {
                        self.drop_type(&field.ty, seen);
                    }
                } else if let Some(e) = self.ctx.enums.get(name) {
                    for variant in &e.variants {
                        for field in &variant.fields {
                            self.drop_type(&field.ty, seen);
                        }
                    }
                }
            }
            Type::Vec(inner)
            | Type::Array(inner, _)
            | Type::Alias(_, inner)
            | Type::Newtype(_, inner)
            | Type::Frozen(inner) => self.drop_type(inner, seen),
            Type::Map(key, value) => {
                self.drop_type(key, seen);
                self.drop_type(value, seen);
            }
            Type::Tuple(fields) => {
                for field in fields {
                    self.drop_type(field, seen);
                }
            }
            _ => {}
        }
    }

    fn classed(&mut self, caps: &[cap_sites::ClassedArg], args: &[&Expr]) {
        for (class, path_arg) in caps {
            let path = path_arg.and_then(|i| literal_str(args.get(i).copied()));
            self.out.caps.insert(cap_sites::capability_of(*class, path));
        }
    }

    fn open_mode(&mut self, args: &[&Expr], path: usize, mode: usize) {
        cap_sites::open_mode_caps(
            literal_str(args.get(path).copied()),
            literal_str(args.get(mode).copied()),
            &mut |c| self.out.caps.insert(c),
        );
    }

    fn call(&mut self, id: DefId, name: Symbol, args: &[&Expr]) {
        if let Some(site) = self.ctx.apertures.get(&id) {
            self.classed(site.caps, args);
            if let Some((path, mode)) = site.open_mode {
                self.open_mode(args, path, mode);
            }
        } else if self.ctx.functions.contains_key(&id) {
            self.out.callees.insert(Callable::Function(id));
        } else if let Some(name) = self.ctx.externs.get(&id) {
            match cap_sites::classify_extern(&name.as_str()) {
                ExternCaps::Effect(caps) => self.classed(caps, args),
                ExternCaps::OpenMode { path_arg, mode_arg } => {
                    self.open_mode(args, path_arg, mode_arg)
                }
                ExternCaps::Benign => {}
                ExternCaps::Unknown => self.out.caps.insert(Capability::FfiUnsafe),
            }
        } else if id == DefId::BUILTIN {
            let name = name.as_str();
            if let Some(store) = self.ctx.stores.iter().find(|s| {
                name == format!("__store_insert_status_{}", s.name)
                    || name.starts_with(&format!("__store_set_status_{}__", s.name))
            }) {
                self.store(store.name);
                if name == format!("__store_insert_status_{}", store.name) {
                    self.store_hooks(store.name, true);
                }
            } else {
                self.out.caps.insert(Capability::FfiUnsafe);
            }
        } else {
            self.out.caps.insert(Capability::FfiUnsafe);
        }
    }

    fn method(&mut self, name: Symbol, args: &[&Expr]) {
        if let Some(id) = self.ctx.methods.get(&name) {
            self.call(*id, name, args);
        } else {
            self.out.caps.insert(Capability::IndirectCall);
        }
    }

    fn display(&mut self, ty: &Type) {
        match ty {
            Type::Struct(name, _) => {
                let method = Symbol::intern(&format!("{name}_display"));
                if self.ctx.methods.contains_key(&method) {
                    self.method(method, &[]);
                }
            }
            Type::Vec(inner) | Type::Array(inner, _) => self.display(inner),
            _ => {}
        }
    }

    fn exprs(&mut self, args: &[Expr]) {
        for a in args {
            self.expr(a);
        }
    }

    fn filter(&mut self, filter: &hir::StoreFilter) {
        self.expr(&filter.value);
        for (_, cond) in &filter.extra {
            self.expr(&cond.value);
        }
    }

    fn conditional(&mut self, i: &hir::If) {
        self.expr(&i.cond);
        self.block(&i.then);
        for (cond, body) in &i.elifs {
            self.expr(cond);
            self.block(body);
        }
        if let Some(body) = &i.els {
            self.block(body);
        }
    }

    fn pattern(&mut self, p: &hir::Pat) {
        match p {
            hir::Pat::Lit(e) => self.expr(e),
            hir::Pat::Range(a, b, _) => {
                self.expr(a);
                self.expr(b);
            }
            hir::Pat::Ctor(_, _, ps, _)
            | hir::Pat::Tuple(ps, _)
            | hir::Pat::Array(ps, _)
            | hir::Pat::Or(ps, _) => {
                for p in ps {
                    self.pattern(p);
                }
            }
            hir::Pat::Wild(_) | hir::Pat::Bind(..) => {}
        }
    }

    fn block(&mut self, body: &hir::Block) {
        for s in body {
            match s {
                Stmt::Bind(b) => self.expr(&b.value),
                Stmt::TupleBind(_, e, _)
                | Stmt::Expr(e)
                | Stmt::Ret(Some(e), _, _)
                | Stmt::Break(Some(e), _)
                | Stmt::ErrReturn(e, _, _)
                | Stmt::ChannelClose(e, _)
                | Stmt::Stop(e, _)
                | Stmt::Join(e, _) => self.expr(e),
                Stmt::Assign(a, b, _) => {
                    self.expr(a);
                    self.expr(b);
                }
                Stmt::GlobalStore(name, e, _) => {
                    self.out.caps.insert(Capability::State(Some(name.as_str())));
                    self.expr(e);
                }
                Stmt::If(i) => self.conditional(i),
                Stmt::While(w) => {
                    self.expr(&w.cond);
                    self.block(&w.body);
                }
                Stmt::For(f) | Stmt::SimFor(f, _) => {
                    self.expr(&f.iter);
                    for e in [&f.end, &f.step].into_iter().flatten() {
                        self.expr(e);
                    }
                    self.block(&f.body);
                }
                Stmt::Loop(l) => self.block(&l.body),
                Stmt::Match(m) => {
                    self.expr(&m.subject);
                    for arm in &m.arms {
                        self.pattern(&arm.pat);
                        if let Some(g) = &arm.guard {
                            self.expr(g);
                        }
                        self.block(&arm.body);
                    }
                }
                Stmt::Asm(a) => {
                    self.out.caps.insert(Capability::FfiUnsafe);
                    for (_, e) in &a.inputs {
                        self.expr(e);
                    }
                }
                Stmt::Defer(b, _) | Stmt::Transaction(b, _) | Stmt::SimBlock(b, _) => self.block(b),
                Stmt::Together(_, b, _, handler, _) => {
                    self.block(b);
                    if let Some(h) = handler {
                        for b in [&h.ok_arm, &h.err_arm].into_iter().flatten() {
                            self.block(b);
                        }
                    }
                }
                Stmt::StoreInsert(s, args, _) => {
                    self.store(*s);
                    self.store_hooks(*s, true);
                    self.exprs(args);
                }
                Stmt::StoreDelete(s, f, _) | Stmt::StoreDestroy(s, f, _) => {
                    self.store_delete(*s);
                    self.filter(f);
                }
                Stmt::StoreRestore(s, f, _) => {
                    self.store(*s);
                    self.filter(f);
                }
                Stmt::StoreSet(s, updates, filter, _) => {
                    self.store(*s);
                    for (_, e) in updates {
                        self.expr(e);
                    }
                    self.filter(filter);
                }
                Stmt::StoreSave(s, _) | Stmt::StoreCompact(s, _) => self.store(*s),
                Stmt::Drop(_, _, ty, _) => self.drop_type(ty, &mut HashSet::new()),
                Stmt::Ret(None, _, _)
                | Stmt::Break(None, _)
                | Stmt::Continue(_)
                | Stmt::Nop(_)
                | Stmt::ScopeCancel(..)
                | Stmt::UseLocal(..) => {}
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        use ExprKind::*;
        match &e.kind {
            Call(id, name, args) => {
                self.call(*id, *name, &args.iter().collect::<Vec<_>>());
                self.exprs(args);
            }
            FnRef(id, name) => self.call(*id, *name, &[]),
            IndirectCall(callee, args) => {
                self.out.caps.insert(Capability::IndirectCall);
                self.expr(callee);
                self.exprs(args);
            }
            Method(recv, name, _, args) => {
                let all = std::iter::once(recv.as_ref())
                    .chain(args.iter())
                    .collect::<Vec<_>>();
                self.method(*name, &all);
                self.expr(recv);
                self.exprs(args);
            }
            Pipe(recv, id, name, args) => {
                let all = std::iter::once(recv.as_ref())
                    .chain(args.iter())
                    .collect::<Vec<_>>();
                self.call(*id, *name, &all);
                self.expr(recv);
                self.exprs(args);
            }
            DeferredMethod(recv, _, args) => {
                self.out.caps.insert(Capability::IndirectCall);
                self.expr(recv);
                self.exprs(args);
            }
            Builtin(b, args) => {
                match b {
                    hir::BuiltinFn::TimeMonotonic => self.out.caps.insert(Capability::Time),
                    hir::BuiltinFn::FileExists => self.out.caps.insert(Capability::FsRead(
                        literal_str(args.first()).map(str::to_owned),
                    )),
                    hir::BuiltinFn::GetArgs => self.out.caps.insert(Capability::EnvRead),
                    hir::BuiltinFn::SignalHandle
                    | hir::BuiltinFn::SignalRaise
                    | hir::BuiltinFn::SignalIgnore
                    | hir::BuiltinFn::SignalDefault
                    | hir::BuiltinFn::SignalKill
                    | hir::BuiltinFn::VolatileLoad
                    | hir::BuiltinFn::VolatileStore => self.out.caps.insert(Capability::FfiUnsafe),
                    hir::BuiltinFn::Log
                    | hir::BuiltinFn::Print
                    | hir::BuiltinFn::Eprint
                    | hir::BuiltinFn::ToString => {
                        for arg in args {
                            self.display(&arg.ty);
                        }
                    }
                    _ => {}
                }
                self.exprs(args);
            }
            StringMethod(recv, _, args) | VecMethod(recv, _, args) | MapMethod(recv, _, args) => {
                self.expr(recv);
                self.exprs(args);
            }
            BinOp(a, _, b)
            | Index(a, b)
            | ChannelSend(a, b)
            | AtomicStore(a, b)
            | AtomicAdd(a, b)
            | AtomicSub(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            UnaryOp(_, a)
            | Field(a, _, _)
            | Coerce(a, _)
            | Cast(a, _)
            | Ref(a)
            | Deref(a)
            | CoroutineNext(a)
            | Yield(a)
            | ChannelRecv(a)
            | StrictCast(a, _)
            | AtomicLoad(a)
            | Grad(a)
            | GeneratorNext(a)
            | EnumUnwrap(a, _, _)
            | EnumIs(a, _)
            | ChannelCreate(_, a) => self.expr(a),
            AsFormat(a, _) => {
                self.display(&a.ty);
                self.expr(a);
            }
            Ternary(a, b, c) | Slice(a, b, c) | AtomicCas(a, b, c) => {
                self.expr(a);
                self.expr(b);
                self.expr(c);
            }
            VecNew(args) | Array(args) | Tuple(args) | Einsum(_, args) => self.exprs(args),
            Syscall(args) => {
                self.out.caps.insert(Capability::FfiUnsafe);
                self.exprs(args);
            }
            Struct(_, fields) | VariantCtor(_, _, _, fields) => {
                for f in fields {
                    self.expr(&f.value);
                }
            }
            Builder(_, fields) => {
                for (_, e) in fields {
                    self.expr(e);
                }
            }
            IfExpr(i) => self.conditional(i),
            Block(b) | Lambda(_, b, _) | CoroutineCreate(_, b, _) | GeneratorCreate(_, _, b, _) => {
                self.block(b)
            }
            ListComp(body, _, _, iter, cond, map) => {
                self.expr(body);
                self.expr(iter);
                for e in [cond, map].into_iter().flatten() {
                    self.expr(e);
                }
            }
            Spawn(actor, inits) => {
                if let Some(actor) = self.ctx.actors.get(actor) {
                    for h in &actor.handlers {
                        self.out
                            .callees
                            .insert(Callable::Handler(actor.def_id, h.tag));
                    }
                    for f in &actor.fields {
                        if let Some(e) = &f.default {
                            self.expr(e);
                        }
                    }
                } else {
                    self.out.caps.insert(Capability::IndirectCall);
                }
                for (_, e) in inits {
                    self.expr(e);
                }
            }
            Send(recv, actor, _, tag, args) => {
                if let Some(actor) = self.ctx.actors.get(actor) {
                    self.out
                        .callees
                        .insert(Callable::Handler(actor.def_id, *tag));
                } else {
                    self.out.caps.insert(Capability::IndirectCall);
                }
                self.expr(recv);
                self.exprs(args);
            }
            StoreQuery(s, f)
            | StoreAllWhere(s, f)
            | StoreFirst(s, f)
            | StoreExists(s, f)
            | ViewCount(s, f)
            | ViewAll(s, f) => {
                self.store(*s);
                self.filter(f);
            }
            StoreCount(s)
            | StoreAll(s)
            | StoreDistinct(s, _)
            | StoreGroup(s, _, _, _)
            | StoreSum(s, _)
            | StoreAvg(s, _)
            | StoreMin(s, _)
            | StoreMax(s, _)
            | KvCount(s)
            | VecCount(s)
            | FtsCount(s, _)
            | TsLatest(s) => self.store(*s),
            StoreGet(s, e)
            | StoreVersionCount(s, e)
            | StoreHistory(s, e)
            | KvGet(s, e)
            | KvHas(s, e)
            | KvDel(s, e)
            | VecInsert(s, e)
            | BloomTest(s, _, e)
            | FtsSearch(s, _, e)
            | GraphFrom(s, e)
            | GraphTo(s, e) => {
                self.store(*s);
                self.expr(e);
            }
            StoreAtVersion(s, a, b) | KvSet(s, a, b) | KvIncr(s, a, b) | VecNearest(s, a, b) => {
                self.store(*s);
                self.expr(a);
                self.expr(b);
            }
            StoreQueryGroup(s, _, _, filter) => {
                self.store(*s);
                if let Some(f) = filter {
                    self.filter(f);
                }
            }
            IterNext(_, ty, method) => self.method(Symbol::intern(&format!("{ty}_{method}")), &[]),
            Select(arms, default) => {
                for a in arms {
                    self.expr(&a.chan);
                    if let Some(v) = &a.value {
                        self.expr(v);
                    }
                    self.block(&a.body);
                }
                if let Some(b) = default {
                    self.block(b);
                }
            }
            GlobalLoad(name) => {
                self.out.caps.insert(Capability::State(Some(name.as_str())));
                if self.globals_seen.insert(*name)
                    && let Some(e) = self.ctx.globals.get(name)
                {
                    self.expr(e);
                }
            }
            Int(_) | Float(_) | Str(_) | Bool(_) | None | Void | Var(..) | VariantRef(..)
            | MapNew | Unreachable | SizeOf(_) => {}
        }
    }
}

pub(in crate::typer) fn analyze(
    program: &hir::Program,
    std_files: &HashSet<Symbol>,
) -> Result<(), String> {
    let functions: Vec<&hir::Fn> = program
        .fns
        .iter()
        .chain(program.types.iter().flat_map(|t| &t.methods))
        .chain(program.trait_impls.iter().flat_map(|i| &i.methods))
        .chain(program.stores.iter().flat_map(|s| &s.methods))
        .collect();
    let mut ctx = Context {
        functions: functions.iter().map(|f| (f.def_id, *f)).collect(),
        methods: functions.iter().map(|f| (f.name, f.def_id)).collect(),
        externs: program.externs.iter().map(|f| (f.def_id, f.name)).collect(),
        apertures: HashMap::new(),
        actors: program.actors.iter().map(|a| (a.name, a)).collect(),
        stores: program.stores.iter().collect(),
        globals: program.globals.iter().map(|g| (g.name, &g.init)).collect(),
        types: program.types.iter().map(|t| (t.name, t)).collect(),
        enums: program.enums.iter().map(|e| (e.name, e)).collect(),
    };
    ctx.stores
        .sort_by_key(|s| std::cmp::Reverse(s.name.as_str().len()));
    for site in cap_sites::APERTURE_SITES {
        let name = Symbol::intern(&site.symbol.replace('.', "_"));
        if let Some(id) = ctx.methods.get(&name)
            && let Some(f) = ctx.functions.get(id)
            && f.span.file.is_some_and(|file| std_files.contains(&file))
        {
            ctx.apertures.insert(*id, site);
        }
    }
    let mut items: Vec<Item<'_>> = functions
        .iter()
        .map(|f| Item {
            id: Callable::Function(f.def_id),
            name: f.name.as_str(),
            body: &f.body,
            needs: f.needs.as_deref(),
            span: f.span,
            extra: f.params.iter().filter_map(|p| p.default.as_ref()).collect(),
        })
        .collect();
    for actor in &program.actors {
        for handler in &actor.handlers {
            items.push(Item {
                id: Callable::Handler(actor.def_id, handler.tag),
                name: format!("{}.{}", actor.name, handler.name),
                body: &handler.body,
                needs: None,
                span: handler.span,
                extra: handler.loop_sleep_ms.iter().collect(),
            });
        }
    }
    let indices: HashMap<Callable, usize> = items
        .iter()
        .enumerate()
        .map(|(i, item)| (item.id, i))
        .collect();
    let mut primitive = Vec::new();
    let mut graph = Vec::new();
    let mut callers = vec![Vec::new(); items.len()];
    for (i, item) in items.iter().enumerate() {
        let mut scanner = Scanner {
            ctx: &ctx,
            out: Collected::default(),
            globals_seen: HashSet::new(),
        };
        scanner.block(item.body);
        for e in &item.extra {
            scanner.expr(e);
        }
        if item.name == "main" {
            for global in &program.globals {
                scanner.expr(&global.init);
            }
        }
        let mut edges = Vec::new();
        for callee in &scanner.out.callees {
            if let Some(&callee) = indices.get(callee) {
                edges.push(callee);
            } else {
                scanner.out.caps.insert(Capability::IndirectCall);
            }
        }
        edges.sort_by(|a, b| items[*a].name.cmp(&items[*b].name));
        for &callee in &edges {
            callers[callee].push(i);
        }
        graph.push(edges);
        primitive.push(scanner.out.caps);
    }
    let mut rows = primitive.clone();
    let mut pending: VecDeque<usize> = (0..items.len()).collect();
    let mut queued = vec![true; items.len()];
    while let Some(callee) = pending.pop_front() {
        queued[callee] = false;
        let effects = rows[callee].clone();
        for &caller in &callers[callee] {
            let before = rows[caller].len();
            rows[caller].join(&effects);
            if before != rows[caller].len() && !queued[caller] {
                pending.push_back(caller);
                queued[caller] = true;
            }
        }
    }
    for (i, item) in items.iter().enumerate() {
        let Some(needs) = item.needs else {
            continue;
        };
        let declared = declared_set(needs)?;
        if let Some(bad) = rows[i].contained_in(&declared).first() {
            let path = shortest_intro_path(i, bad, &graph, &primitive, &items);
            return Err(format!(
                "{}: function `{}` declares `needs {}` but uses `{}`{}",
                item.span.loc(),
                item.name,
                declared.render(),
                bad.render(),
                path
            ));
        }
    }
    Ok(())
}

fn shortest_intro_path(
    start: usize,
    cap: &Capability,
    graph: &[Vec<usize>],
    primitive: &[CapSet],
    items: &[Item<'_>],
) -> String {
    let mut pending = VecDeque::from([vec![start]]);
    let mut seen = HashSet::from([start]);
    while let Some(path) = pending.pop_front() {
        let last = *path.last().unwrap();
        if primitive[last].satisfies(cap) {
            return format!(
                " (introduced via {})",
                path.iter()
                    .map(|&i| items[i].name.as_str())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            );
        }
        for &callee in &graph[last] {
            if seen.insert(callee) {
                let mut next = path.clone();
                next.push(callee);
                pending.push_back(next);
            }
        }
    }
    String::new()
}
