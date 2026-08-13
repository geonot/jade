use std::collections::{HashMap, HashSet};

use crate::ast::{self, Expr, Stmt};
use crate::intern::Symbol;
use crate::types::Type;

const CONSUMING_METHODS: &[&str] = &[
    "push",
    "push_back",
    "push_front",
    "insert",
    "append",
    "add",
    "put",
    "set",
    "enqueue",
    "send",
];

fn annotated_non_consumable(ty: &Option<Type>) -> bool {
    match ty {
        None => false,
        Some(t) => matches!(
            t,
            Type::String
                | Type::I8
                | Type::I16
                | Type::I32
                | Type::I64
                | Type::U8
                | Type::U16
                | Type::U32
                | Type::U64
                | Type::F32
                | Type::F64
                | Type::Bool
                | Type::Void
                | Type::Ptr(_)
        ),
    }
}

type AliasMap = HashMap<Symbol, HashSet<usize>>;

pub(in crate::typer) struct ScanCtx {
    pub(in crate::typer) facts: HashMap<Symbol, Symbol>,
    pub(in crate::typer) fields: HashSet<Symbol>,
}

#[derive(Default)]
struct Escapes {
    sites: HashMap<usize, (ast::Span, bool)>,
}

impl Escapes {
    fn record(&mut self, slots: HashSet<usize>, span: ast::Span, cond: bool) {
        for i in slots {
            match self.sites.get(&i) {
                Some((_, false)) => {}
                Some((_, true)) if cond => {}
                _ => {
                    self.sites.insert(i, (span, cond));
                }
            }
        }
    }
}

impl crate::typer::Typer {
    pub(crate) fn infer_consuming_params(&mut self, fns: &[&ast::Fn]) {
        let method_items: Vec<(Symbol, Symbol, ast::Fn)> = self
            .methods
            .iter()
            .flat_map(|(ty, ms)| {
                let ty = *ty;
                ms.iter().map(move |m| {
                    let mangled: Symbol = format!("{}_{}", ty.as_str(), m.name.as_str()).into();
                    (mangled, ty, m.clone())
                })
            })
            .collect();
        loop {
            let mut changed = false;
            for f in fns {
                changed |= self.consume_scan_one(f.name, f, 0, None);
            }
            for (mangled, ty, m) in &method_items {
                changed |= self.consume_scan_one(*mangled, m, 1, Some(*ty));
            }
            if !changed {
                break;
            }
        }
        self.infer_mutating_params(fns, &method_items);
    }

    pub(in crate::typer) fn is_user_type_name(&self, n: Symbol) -> bool {
        self.structs.contains_key(&n)
            || self.enums.contains_key(&n)
            || self.methods.contains_key(&n)
    }

    pub(in crate::typer) fn user_ty_of_annotation(&self, t: &Type) -> Option<Symbol> {
        match t {
            Type::Struct(n, _) | Type::Enum(n) if self.is_user_type_name(*n) => Some(*n),
            _ => None,
        }
    }

    fn user_ctor_ty(&self, e: &Expr) -> Option<Symbol> {
        match e {
            Expr::Call(callee, _, _) => match &**callee {
                Expr::Ident(n, _) if self.is_user_type_name(*n) => Some(*n),
                _ => None,
            },
            Expr::Struct(n, _, _) if self.is_user_type_name(*n) => Some(*n),
            Expr::As(inner, _, _) => self.user_ctor_ty(inner),
            _ => None,
        }
    }

    fn struct_field_user_ty(&self, ty: Symbol, field: Symbol) -> Option<Symbol> {
        self.structs.get(&ty).and_then(|fs| {
            fs.iter()
                .find(|(n, _)| *n == field)
                .and_then(|(_, t)| self.user_ty_of_annotation(t))
        })
    }

    pub(in crate::typer) fn recv_user_ty(
        &self,
        recv: &Expr,
        facts: &HashMap<Symbol, Symbol>,
    ) -> Option<Symbol> {
        match recv {
            Expr::Ident(n, _) => facts.get(n).copied(),
            Expr::Field(base, f, _) => self
                .recv_user_ty(base, facts)
                .and_then(|t| self.struct_field_user_ty(t, *f)),
            Expr::As(x, _, _) => self.recv_user_ty(x, facts),
            _ => None,
        }
    }

    pub(in crate::typer) fn self_field_names(&self, self_ty: Symbol) -> HashSet<Symbol> {
        self.structs
            .get(&self_ty)
            .map(|fs| fs.iter().map(|(n, _)| *n).collect())
            .unwrap_or_default()
    }

    fn kill_pat_binders(p: &ast::Pat, events: &mut HashMap<Symbol, Vec<Option<Symbol>>>) {
        match p {
            ast::Pat::Ident(n, _) => {
                events.entry(*n).or_default().push(None);
            }
            ast::Pat::Ctor(_, ps, _)
            | ast::Pat::Or(ps, _)
            | ast::Pat::Tuple(ps, _)
            | ast::Pat::Array(ps, _) => {
                for q in ps {
                    Self::kill_pat_binders(q, events);
                }
            }
            _ => {}
        }
    }

    fn collect_bind_events(
        &self,
        block: &[Stmt],
        events: &mut HashMap<Symbol, Vec<Option<Symbol>>>,
    ) {
        for s in block {
            match s {
                Stmt::Bind(b) => {
                    let fact =
                        b.ty.as_ref()
                            .and_then(|t| self.user_ty_of_annotation(t))
                            .or_else(|| self.user_ctor_ty(&b.value));
                    events.entry(b.name).or_default().push(fact);
                }
                Stmt::TupleBind(names, _, _) => {
                    for n in names {
                        events.entry(*n).or_default().push(None);
                    }
                }
                Stmt::Assign(Expr::Ident(n, _), _, _) => {
                    events.entry(*n).or_default().push(None);
                }
                Stmt::If(i) => {
                    self.collect_bind_events(&i.then, events);
                    for (_, b) in &i.elifs {
                        self.collect_bind_events(b, events);
                    }
                    if let Some(b) = &i.els {
                        self.collect_bind_events(b, events);
                    }
                }
                Stmt::While(w) => self.collect_bind_events(&w.body, events),
                Stmt::For(f) | Stmt::SimFor(f, _) => {
                    events.entry(f.bind).or_default().push(None);
                    if let Some(b2) = f.bind2 {
                        events.entry(b2).or_default().push(None);
                    }
                    self.collect_bind_events(&f.body, events);
                }
                Stmt::Loop(l) => self.collect_bind_events(&l.body, events),
                Stmt::Match(m) => {
                    for arm in &m.arms {
                        Self::kill_pat_binders(&arm.pat, events);
                        self.collect_bind_events(&arm.body, events);
                    }
                }
                Stmt::Defer(b, _) | Stmt::Transaction(b, _) | Stmt::SimBlock(b, _) => {
                    self.collect_bind_events(b, events);
                }
                Stmt::Together(_, b, _, _) => self.collect_bind_events(b, events),
                _ => {}
            }
        }
    }

    pub(in crate::typer) fn receiver_facts(
        &self,
        f: &ast::Fn,
        self_ty: Option<Symbol>,
    ) -> HashMap<Symbol, Symbol> {
        let mut events: HashMap<Symbol, Vec<Option<Symbol>>> = HashMap::new();
        if let Some(t) = self_ty {
            events.entry("self".into()).or_default().push(Some(t));
            if let Some(fields) = self.structs.get(&t) {
                for (fname, fty) in fields {
                    let fact = self.user_ty_of_annotation(fty);
                    events.entry(*fname).or_default().push(fact);
                }
            }
        }
        for p in &f.params {
            let fact = p.ty.as_ref().and_then(|t| self.user_ty_of_annotation(t));
            events.entry(p.name).or_default().push(fact);
        }
        self.collect_bind_events(&f.body, &mut events);
        let mut facts: HashMap<Symbol, Symbol> = HashMap::new();
        for (name, evs) in &events {
            if evs.len() == 1
                && let Some(Some(t)) = evs.first()
            {
                facts.insert(*name, *t);
            }
        }
        facts
    }

    fn consume_scan_one(
        &mut self,
        fname: Symbol,
        f: &ast::Fn,
        offset: usize,
        self_ty: Option<Symbol>,
    ) -> bool {
        let mut alias: AliasMap = AliasMap::new();
        let mut slot = offset;
        for p in &f.params {
            if offset == 1 && p.name.as_str() == "self" {
                continue;
            }
            let this_slot = slot;
            slot += 1;
            if p.access_mod.is_some() || annotated_non_consumable(&p.ty) {
                continue;
            }
            alias.insert(p.name, HashSet::from([this_slot]));
        }
        if alias.is_empty() {
            return false;
        }
        let facts = self.receiver_facts(f, self_ty);
        let param_names: HashSet<Symbol> = f.params.iter().map(|p| p.name).collect();
        let fields: HashSet<Symbol> = self_ty
            .map(|t| {
                self.self_field_names(t)
                    .difference(&param_names)
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        let ctx = ScanCtx { facts, fields };
        let mut escaping = Escapes::default();
        let returns_value = f.ret.is_some() || ret_is_inferred(f);
        self.scan_block(
            &f.body,
            &mut alias,
            &ctx,
            &mut escaping,
            returns_value,
            false,
        );

        let mut changed = false;
        for (i, (span, cond)) in escaping.sites {
            let mut flipped = false;
            let mut n_slots = 0;
            if let Some(accs) = self.fn_param_access.get_mut(&fname)
                && let Some(slot) = accs.get_mut(i)
                && slot.is_none()
            {
                *slot = Some(ast::AccessMod::Take);
                n_slots = accs.len();
                flipped = true;
            }
            if flipped {
                let sites = self
                    .fn_param_consume_sites
                    .entry(fname)
                    .or_insert_with(|| vec![None; n_slots]);
                if sites.len() < n_slots {
                    sites.resize(n_slots, None);
                }
                if let Some(s) = sites.get_mut(i) {
                    *s = Some((span, cond));
                }
                changed = true;
            }
        }
        changed
    }

    pub fn boundary_ownership_warnings(&self, prog: &ast::Program) -> Vec<String> {
        let mut out = Vec::new();
        for d in &prog.decls {
            match d {
                ast::Decl::Fn(f) => self.boundary_warn_fn(f.name, f, 0, &mut out),
                ast::Decl::Type(td) => {
                    for m in &td.methods {
                        let mangled: Symbol =
                            format!("{}_{}", td.name.as_str(), m.name.as_str()).into();
                        self.boundary_warn_fn(mangled, m, 1, &mut out);
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn boundary_warn_fn(&self, key: Symbol, f: &ast::Fn, offset: usize, out: &mut Vec<String>) {
        let Some(sites) = self.fn_param_consume_sites.get(&key) else {
            return;
        };
        let mut slot = offset;
        for p in &f.params {
            if offset == 1 && p.name.as_str() == "self" {
                continue;
            }
            let this_slot = slot;
            slot += 1;
            if let Some(Some((span, cond))) = sites.get(this_slot) {
                let key_owned = key.as_str();
                let shown = Self::display_fn_name(&key_owned);
                let when = if *cond { " on a conditional path" } else { "" };
                out.push(format!(
                    "{}: warning: exported function `{}` consumes its parameter `{}` by \
                     inference (it escapes at {}{}), so callers move their argument and a \
                     body edit can silently change call sites downstream; declare it \
                     `{} as take ...` to make the contract explicit at the boundary",
                    f.span.loc(),
                    shown,
                    p.name,
                    span.loc(),
                    when,
                    p.name,
                ));
            }
        }
    }

    pub(in crate::typer) fn any_user_method_consumes(&self, method: Symbol, slot: usize) -> bool {
        self.methods.iter().any(|(ty, ms)| {
            ms.iter().any(|m| m.name == method) && {
                let mangled: Symbol = format!("{}_{}", ty.as_str(), method.as_str()).into();
                self.fn_param_access
                    .get(&mangled)
                    .map(|accs| matches!(accs.get(slot), Some(Some(ast::AccessMod::Take))))
                    .unwrap_or(false)
            }
        })
    }

    fn scan_block(
        &self,
        block: &[Stmt],
        alias: &mut AliasMap,
        ctx: &ScanCtx,
        escaping: &mut Escapes,
        tail_returns: bool,
        cond: bool,
    ) {
        let last = block.len().saturating_sub(1);
        for (idx, s) in block.iter().enumerate() {
            let is_tail = tail_returns && idx == last;
            self.scan_stmt(s, alias, ctx, escaping, is_tail, cond);
        }
    }

    fn scan_stmt(
        &self,
        s: &Stmt,
        alias: &mut AliasMap,
        ctx: &ScanCtx,
        escaping: &mut Escapes,
        is_tail: bool,
        cond: bool,
    ) {
        match s {
            Stmt::Bind(b) => {
                self.scan_expr_sinks(&b.value, alias, ctx, escaping, cond);
                if ctx.fields.contains(&b.name) {
                    escaping.record(Self::expr_alias(&b.value, alias), b.value.span(), cond);
                }
                let set = Self::expr_alias(&b.value, alias);
                if !set.is_empty() {
                    alias.entry(b.name).or_default().extend(set);
                }
            }
            Stmt::TupleBind(_, e, _) => self.scan_expr_sinks(e, alias, ctx, escaping, cond),
            Stmt::Assign(target, value, _) => {
                self.scan_expr_sinks(value, alias, ctx, escaping, cond);
                self.scan_expr_sinks(target, alias, ctx, escaping, cond);
                match target {
                    Expr::Ident(n, _) if !ctx.fields.contains(n) => {
                        let set = Self::expr_alias(value, alias);
                        if !set.is_empty() {
                            alias.entry(*n).or_default().extend(set);
                        }
                    }

                    _ => {
                        escaping.record(Self::expr_alias(value, alias), value.span(), cond);
                    }
                }
            }
            Stmt::Expr(e) => {
                self.scan_expr_sinks(e, alias, ctx, escaping, cond);
                if is_tail {
                    escaping.record(Self::expr_alias(e, alias), e.span(), cond);
                }
            }
            Stmt::Ret(Some(e), _) | Stmt::ErrReturn(e, _) | Stmt::Break(Some(e), _) => {
                self.scan_expr_sinks(e, alias, ctx, escaping, cond);
                escaping.record(Self::expr_alias(e, alias), e.span(), cond);
            }
            Stmt::If(i) => {
                self.scan_expr_sinks(&i.cond, alias, ctx, escaping, cond);
                self.scan_block(&i.then, alias, ctx, escaping, is_tail, true);
                for (c, b) in &i.elifs {
                    self.scan_expr_sinks(c, alias, ctx, escaping, cond);
                    self.scan_block(b, alias, ctx, escaping, is_tail, true);
                }
                if let Some(b) = &i.els {
                    self.scan_block(b, alias, ctx, escaping, is_tail, true);
                }
            }
            Stmt::While(w) => {
                self.scan_expr_sinks(&w.cond, alias, ctx, escaping, cond);
                self.scan_block(&w.body, alias, ctx, escaping, false, true);
            }
            Stmt::For(f) | Stmt::SimFor(f, _) => {
                self.scan_expr_sinks(&f.iter, alias, ctx, escaping, cond);
                if let Some(e) = &f.end {
                    self.scan_expr_sinks(e, alias, ctx, escaping, cond);
                }
                if let Some(e) = &f.step {
                    self.scan_expr_sinks(e, alias, ctx, escaping, cond);
                }
                self.scan_block(&f.body, alias, ctx, escaping, false, true);
            }
            Stmt::Loop(l) => self.scan_block(&l.body, alias, ctx, escaping, false, cond),
            Stmt::Match(m) => {
                self.scan_expr_sinks(&m.subject, alias, ctx, escaping, cond);
                for arm in &m.arms {
                    if let Some(g) = &arm.guard {
                        self.scan_expr_sinks(g, alias, ctx, escaping, cond);
                    }
                    self.scan_block(&arm.body, alias, ctx, escaping, is_tail, true);
                }
            }
            Stmt::Defer(b, _) | Stmt::Transaction(b, _) | Stmt::SimBlock(b, _) => {
                self.scan_block(b, alias, ctx, escaping, false, cond);
            }
            Stmt::Together(_, b, _, _) => self.scan_block(b, alias, ctx, escaping, false, cond),
            Stmt::StoreInsert(_, inits, _) => {
                for fi in inits {
                    self.scan_expr_sinks(&fi.value, alias, ctx, escaping, cond);
                }
            }
            Stmt::StoreSet(_, sets, _, _) => {
                for (_, e) in sets {
                    self.scan_expr_sinks(e, alias, ctx, escaping, cond);
                }
            }
            Stmt::ChannelClose(e, _) | Stmt::Stop(e, _) | Stmt::Join(e, _) => {
                self.scan_expr_sinks(e, alias, ctx, escaping, cond);
            }
            _ => {}
        }
    }

    fn expr_alias(e: &Expr, alias: &AliasMap) -> HashSet<usize> {
        match e {
            Expr::Ident(n, _) => alias.get(n).cloned().unwrap_or_default(),
            Expr::Ternary(_, t, els, _) => {
                let mut s = Self::expr_alias(t, alias);
                s.extend(Self::expr_alias(els, alias));
                s
            }
            Expr::Quaternary(subj, ok, nothing, err, _) => {
                let mut s = Self::expr_alias(subj, alias);
                for arm in [ok, nothing, err].into_iter().flatten() {
                    s.extend(Self::expr_alias(arm, alias));
                }
                s
            }
            Expr::As(inner, _, _) => Self::expr_alias(inner, alias),
            Expr::Struct(_, inits, _) => {
                let mut s = HashSet::new();
                for fi in inits {
                    s.extend(Self::expr_alias(&fi.value, alias));
                }
                s
            }
            Expr::Array(es, _) | Expr::Tuple(es, _) => {
                let mut s = HashSet::new();
                for x in es {
                    s.extend(Self::expr_alias(x, alias));
                }
                s
            }
            _ => HashSet::new(),
        }
    }

    fn scan_expr_sinks(
        &self,
        e: &Expr,
        alias: &AliasMap,
        ctx: &ScanCtx,
        escaping: &mut Escapes,
        cond: bool,
    ) {
        match e {
            Expr::Call(callee, args, _) => {
                if let Expr::Ident(cn, _) = &**callee
                    && matches!(cn.as_str().as_str(), "vec" | "vector")
                {
                    for a in args {
                        escaping.record(Self::expr_alias(a, alias), a.span(), cond);
                    }
                }
                if !args.iter().any(|a| matches!(a, Expr::NamedArg(..))) {
                    let fname = match &**callee {
                        Expr::Ident(n, _) => Some(*n),
                        Expr::QualifiedIdent(_, n, _) => Some(*n),
                        _ => None,
                    };
                    if let Some(fname) = fname
                        && let Some(access) = self.fn_param_access.get(&fname)
                    {
                        for (j, a) in args.iter().enumerate() {
                            if matches!(access.get(j), Some(Some(ast::AccessMod::Take))) {
                                escaping.record(Self::expr_alias(a, alias), a.span(), cond);
                            }
                        }
                    }
                }
                for a in args {
                    self.scan_expr_sinks(a, alias, ctx, escaping, cond);
                }
            }
            Expr::Method(recv, name, args, _) => {
                match self.recv_user_ty(recv, &ctx.facts) {
                    Some(tyname) => {
                        let mangled: Symbol =
                            format!("{}_{}", tyname.as_str(), name.as_str()).into();
                        if let Some(access) = self.fn_param_access.get(&mangled) {
                            for (j, a) in args.iter().enumerate() {
                                if matches!(access.get(j + 1), Some(Some(ast::AccessMod::Take))) {
                                    escaping.record(Self::expr_alias(a, alias), a.span(), cond);
                                }
                            }
                        }
                    }
                    None => {
                        if CONSUMING_METHODS.contains(&&*name.as_str()) {
                            for a in args {
                                escaping.record(Self::expr_alias(a, alias), a.span(), cond);
                            }
                        } else {
                            for (j, a) in args.iter().enumerate() {
                                if self.any_user_method_consumes(*name, j + 1) {
                                    escaping.record(Self::expr_alias(a, alias), a.span(), cond);
                                }
                            }
                        }
                    }
                }
                self.scan_expr_sinks(recv, alias, ctx, escaping, cond);
                for a in args {
                    self.scan_expr_sinks(a, alias, ctx, escaping, cond);
                }
            }
            Expr::Pipe(lhs, target, rest, _) => {
                if let Expr::Ident(fname, _) = &**target
                    && let Some(access) = self.fn_param_access.get(fname)
                    && matches!(access.first(), Some(Some(ast::AccessMod::Take)))
                {
                    escaping.record(Self::expr_alias(lhs, alias), lhs.span(), cond);
                }
                self.scan_expr_sinks(lhs, alias, ctx, escaping, cond);
                for a in rest {
                    self.scan_expr_sinks(a, alias, ctx, escaping, cond);
                }
            }
            Expr::ChannelSend(ch, v, _) => {
                escaping.record(Self::expr_alias(v, alias), v.span(), cond);
                self.scan_expr_sinks(ch, alias, ctx, escaping, cond);
                self.scan_expr_sinks(v, alias, ctx, escaping, cond);
            }
            Expr::Send(actor, _, args, _) => {
                for a in args {
                    escaping.record(Self::expr_alias(a, alias), a.span(), cond);
                    self.scan_expr_sinks(a, alias, ctx, escaping, cond);
                }
                self.scan_expr_sinks(actor, alias, ctx, escaping, cond);
            }
            Expr::Spawn(_, inits, _) => {
                for (_, v) in inits {
                    escaping.record(Self::expr_alias(v, alias), v.span(), cond);
                    self.scan_expr_sinks(v, alias, ctx, escaping, cond);
                }
            }
            Expr::Yield(v, _) => {
                escaping.record(Self::expr_alias(v, alias), v.span(), cond);
                self.scan_expr_sinks(v, alias, ctx, escaping, cond);
            }

            Expr::BinOp(l, _, r, _) | Expr::Index(l, r, _) | Expr::OfCall(l, r, _) => {
                self.scan_expr_sinks(l, alias, ctx, escaping, cond);
                self.scan_expr_sinks(r, alias, ctx, escaping, cond);
            }
            Expr::UnaryOp(_, x, _)
            | Expr::Field(x, _, _)
            | Expr::As(x, _, _)
            | Expr::StrictCast(x, _, _)
            | Expr::Ref(x, _)
            | Expr::Deref(x, _)
            | Expr::Spread(x, _)
            | Expr::Grad(x, _)
            | Expr::AsFormat(x, _, _)
            | Expr::NamedArg(_, x, _) => {
                self.scan_expr_sinks(x, alias, ctx, escaping, cond);
            }
            Expr::Ternary(c, t, els, _) => {
                self.scan_expr_sinks(c, alias, ctx, escaping, cond);
                self.scan_expr_sinks(t, alias, ctx, escaping, true);
                self.scan_expr_sinks(els, alias, ctx, escaping, true);
            }
            Expr::Quaternary(subj, ok, nothing, err, _) => {
                self.scan_expr_sinks(subj, alias, ctx, escaping, cond);
                for arm in [ok, nothing, err].into_iter().flatten() {
                    self.scan_expr_sinks(arm, alias, ctx, escaping, true);
                }
            }
            Expr::Array(es, _) | Expr::Tuple(es, _) => {
                for x in es {
                    escaping.record(Self::expr_alias(x, alias), x.span(), cond);
                    self.scan_expr_sinks(x, alias, ctx, escaping, cond);
                }
            }
            Expr::Syscall(es, _) | Expr::Einsum(_, es, _) => {
                for x in es {
                    self.scan_expr_sinks(x, alias, ctx, escaping, cond);
                }
            }
            Expr::Struct(_, inits, _) => {
                for fi in inits {
                    escaping.record(Self::expr_alias(&fi.value, alias), fi.value.span(), cond);
                    self.scan_expr_sinks(&fi.value, alias, ctx, escaping, cond);
                }
            }
            Expr::Slice(a, b, c, _) => {
                self.scan_expr_sinks(a, alias, ctx, escaping, cond);
                self.scan_expr_sinks(b, alias, ctx, escaping, cond);
                self.scan_expr_sinks(c, alias, ctx, escaping, cond);
            }

            _ => {}
        }
    }
}

fn ret_is_inferred(f: &ast::Fn) -> bool {
    f.name.as_str() != "main"
}
