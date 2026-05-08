//! HIR pretty-printer used by `--emit-hir`.

use super::*;

pub fn pretty_print(prog: &Program) -> String {
    let mut pp = PrettyPrinter {
        buf: String::with_capacity(4096),
        indent: 0,
    };
    pp.program(prog);
    pp.buf
}

struct PrettyPrinter {
    buf: String,
    indent: usize,
}

impl PrettyPrinter {
    fn line(&mut self, s: &str) {
        for _ in 0..self.indent {
            self.buf.push_str("  ");
        }
        self.buf.push_str(s);
        self.buf.push('\n');
    }

    fn push(&mut self) {
        self.indent += 1;
    }
    fn pop(&mut self) {
        self.indent -= 1;
    }

    fn program(&mut self, p: &Program) {
        for e in &p.externs {
            self.extern_fn(e);
        }
        for t in &p.types {
            self.type_def(t);
        }
        for e in &p.enums {
            self.enum_def(e);
        }
        for e in &p.err_defs {
            self.err_def(e);
        }
        for a in &p.actors {
            self.actor_def(a);
        }
        for s in &p.stores {
            self.store_def(s);
        }
        for ti in &p.trait_impls {
            self.trait_impl(ti);
        }
        for f in &p.fns {
            self.fn_def(f);
        }
    }

    fn extern_fn(&mut self, e: &ExternFn) {
        let params: Vec<String> = e.params.iter().map(|(n, t)| format!("{n}: {t}")).collect();
        let va = if e.variadic { ", ..." } else { "" };
        self.line(&format!(
            "extern fn {}({}{}) -> {} [d{}]",
            e.name,
            params.join(", "),
            va,
            e.ret,
            e.def_id.0
        ));
    }

    fn type_def(&mut self, t: &TypeDef) {
        self.line(&format!("type {} [d{}]:", t.name, t.def_id.0));
        self.push();
        for f in &t.fields {
            if let Some(d) = &f.default {
                self.line(&format!("{}: {} = {}", f.name, f.ty, self.expr_str(d)));
            } else {
                self.line(&format!("{}: {}", f.name, f.ty));
            }
        }
        for m in &t.methods {
            self.fn_def(m);
        }
        self.pop();
    }

    fn enum_def(&mut self, e: &EnumDef) {
        self.line(&format!("enum {} [d{}]:", e.name, e.def_id.0));
        self.push();
        for v in &e.variants {
            let fields: Vec<String> = v
                .fields
                .iter()
                .map(|f| {
                    if let Some(n) = &f.name {
                        format!("{n}: {}", f.ty)
                    } else {
                        format!("{}", f.ty)
                    }
                })
                .collect();
            if fields.is_empty() {
                self.line(&format!("{} = {}", v.name, v.tag));
            } else {
                self.line(&format!("{}({}) = {}", v.name, fields.join(", "), v.tag));
            }
        }
        self.pop();
    }

    fn err_def(&mut self, e: &ErrDef) {
        self.line(&format!("error {} [d{}]:", e.name, e.def_id.0));
        self.push();
        for v in &e.variants {
            let fields: Vec<String> = v.fields.iter().map(|t| format!("{t}")).collect();
            if fields.is_empty() {
                self.line(&format!("{} = {}", v.name, v.tag));
            } else {
                self.line(&format!("{}({}) = {}", v.name, fields.join(", "), v.tag));
            }
        }
        self.pop();
    }

    fn actor_def(&mut self, a: &ActorDef) {
        self.line(&format!("actor {} [d{}]:", a.name, a.def_id.0));
        self.push();
        for f in &a.fields {
            self.line(&format!("{}: {}", f.name, f.ty));
        }
        for h in &a.handlers {
            let params: Vec<String> = h
                .params
                .iter()
                .map(|p| format!("{}: {}", p.name, p.ty))
                .collect();
            if h.is_loop {
                let sleep = h
                    .loop_sleep_ms
                    .as_ref()
                    .map(|e| self.expr_str(e))
                    .unwrap_or_else(|| "0".to_string());
                self.line(&format!(
                    "loop {}({}) [sleep_ms={}]:",
                    h.name,
                    params.join(", "),
                    sleep
                ));
            } else {
                self.line(&format!(
                    "on {}({}) [tag={}]:",
                    h.name,
                    params.join(", "),
                    h.tag
                ));
            }
            self.push();
            self.block(&h.body);
            self.pop();
        }
        self.pop();
    }

    fn store_def(&mut self, s: &StoreDef) {
        self.line(&format!("store {} [d{}]:", s.name, s.def_id.0));
        self.push();
        for f in &s.fields {
            self.line(&format!("{}: {}", f.name, f.ty));
        }
        self.pop();
    }

    fn trait_impl(&mut self, ti: &TraitImpl) {
        let trait_part = ti
            .trait_name
            .map(|s| s.as_str())
            .unwrap_or_else(|| "_".to_string());
        self.line(&format!("impl {} for {}:", trait_part, ti.type_name));
        self.push();
        for m in &ti.methods {
            self.fn_def(m);
        }
        self.pop();
    }

    fn fn_def(&mut self, f: &Fn) {
        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| {
                let own = if p.ownership == Ownership::Owned {
                    String::new()
                } else {
                    format!(" {}", p.ownership)
                };
                format!("{}: {}{}", p.name, p.ty, own)
            })
            .collect();
        let gen_tag = f
            .generic_origin
            .map(|g| format!(" <{g}>"))
            .unwrap_or_default();
        self.line(&format!(
            "fn {}{}({}) -> {} [d{}]:",
            f.name,
            gen_tag,
            params.join(", "),
            f.ret,
            f.def_id.0
        ));
        self.push();
        self.block(&f.body);
        self.pop();
    }

    fn block(&mut self, blk: &Block) {
        for s in blk {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Bind(b) => {
                let own = if b.ownership == Ownership::Owned {
                    String::new()
                } else {
                    format!(" {}", b.ownership)
                };
                self.line(&format!(
                    "let {} [d{}]: {}{} = {}",
                    b.name,
                    b.def_id.0,
                    b.ty,
                    own,
                    self.expr_str(&b.value)
                ));
            }
            Stmt::TupleBind(binds, val, _) => {
                let names: Vec<String> = binds
                    .iter()
                    .map(|(id, n, t)| format!("{n}[d{}]: {t}", id.0))
                    .collect();
                self.line(&format!(
                    "let ({}) = {}",
                    names.join(", "),
                    self.expr_str(val)
                ));
            }
            Stmt::Assign(lhs, rhs, _) => {
                self.line(&format!("{} = {}", self.expr_str(lhs), self.expr_str(rhs)));
            }
            Stmt::Expr(e) => {
                self.line(&self.expr_str(e));
            }
            Stmt::If(i) => self.if_stmt(i),
            Stmt::While(w) => {
                self.line(&format!("while {}:", self.expr_str(&w.cond)));
                self.push();
                self.block(&w.body);
                self.pop();
            }
            Stmt::For(f) => {
                let end = f
                    .end
                    .as_ref()
                    .map(|e| format!(" to {}", self.expr_str(e)))
                    .unwrap_or_default();
                let step = f
                    .step
                    .as_ref()
                    .map(|e| format!(" step {}", self.expr_str(e)))
                    .unwrap_or_default();
                self.line(&format!(
                    "for {} [d{}] in {}{}{}:",
                    f.bind,
                    f.bind_id.0,
                    self.expr_str(&f.iter),
                    end,
                    step
                ));
                self.push();
                self.block(&f.body);
                self.pop();
            }
            Stmt::Loop(l) => {
                self.line("loop:");
                self.push();
                self.block(&l.body);
                self.pop();
            }
            Stmt::Ret(val, ty, _) => {
                let v = val
                    .as_ref()
                    .map(|e| format!(" {}", self.expr_str(e)))
                    .unwrap_or_default();
                self.line(&format!("ret{v} : {ty}"));
            }
            Stmt::Break(val, _) => {
                let v = val
                    .as_ref()
                    .map(|e| format!(" {}", self.expr_str(e)))
                    .unwrap_or_default();
                self.line(&format!("break{v}"));
            }
            Stmt::Continue(_) => {
                self.line("continue");
            }
            Stmt::Match(m) => {
                self.line(&format!("match {} : {}:", self.expr_str(&m.subject), m.ty));
                self.push();
                for arm in &m.arms {
                    let guard = arm
                        .guard
                        .as_ref()
                        .map(|g| format!(" if {}", self.expr_str(g)))
                        .unwrap_or_default();
                    self.line(&format!("{}{}:", self.pat_str(&arm.pat), guard));
                    self.push();
                    self.block(&arm.body);
                    self.pop();
                }
                self.pop();
            }
            Stmt::Asm(a) => {
                self.line(&format!("asm \"{}\"", a.template));
            }
            Stmt::Drop(id, name, ty, _) => {
                self.line(&format!("drop {} [d{}]: {}", name, id.0, ty));
            }
            Stmt::ErrReturn(e, ty, _) => {
                self.line(&format!("err_return {} : {}", self.expr_str(e), ty));
            }
            Stmt::Defer(body, _) => {
                self.line("defer:");
                self.push();
                self.block(body);
                self.pop();
            }
            Stmt::StoreInsert(name, vals, _) => {
                let vs: Vec<String> = vals.iter().map(|v| self.expr_str(v)).collect();
                self.line(&format!("store_insert {} ({})", name, vs.join(", ")));
            }
            Stmt::StoreDelete(name, _, _) => {
                self.line(&format!("store_delete {name} ..."));
            }
            Stmt::StoreDestroy(name, _, _) => {
                self.line(&format!("store_destroy {name} ..."));
            }
            Stmt::StoreRestore(name, _, _) => {
                self.line(&format!("store_restore {name} ..."));
            }
            Stmt::StoreSave(name, _) => {
                self.line(&format!("store_save {name}"));
            }
            Stmt::StoreSet(name, _, _, _) => {
                self.line(&format!("store_set {name} ..."));
            }
            Stmt::Transaction(blk, _) => {
                self.line("transaction:");
                self.push();
                self.block(blk);
                self.pop();
            }
            Stmt::ChannelClose(e, _) => {
                self.line(&format!("close {}", self.expr_str(e)));
            }
            Stmt::Stop(e, _) => {
                self.line(&format!("stop {}", self.expr_str(e)));
            }
            Stmt::SimFor(f, _) => {
                let end = f
                    .end
                    .as_ref()
                    .map(|e| format!(" to {}", self.expr_str(e)))
                    .unwrap_or_default();
                let step = f
                    .step
                    .as_ref()
                    .map(|e| format!(" step {}", self.expr_str(e)))
                    .unwrap_or_default();
                self.line(&format!(
                    "sim for {} [d{}] in {}{}{}:",
                    f.bind,
                    f.bind_id.0,
                    self.expr_str(&f.iter),
                    end,
                    step
                ));
                self.push();
                self.block(&f.body);
                self.pop();
            }
            Stmt::SimBlock(b, _) => {
                self.line("sim:");
                self.push();
                self.block(b);
                self.pop();
            }
            Stmt::UseLocal(path, imports, alias, _) => {
                let p = Symbol::join_vec(path, ".");
                let i = imports
                    .as_ref()
                    .map(|is| format!(" import {}", Symbol::join_vec(is, ", ")))
                    .unwrap_or_default();
                let a = alias
                    .as_ref()
                    .map(|a| format!(" as {a}"))
                    .unwrap_or_default();
                self.line(&format!("use {p}{i}{a}"));
            }
            Stmt::GlobalStore(name, e, _) => {
                self.line(&format!("global_store {name} = {}", self.expr_str(e)));
            }
        }
    }

    fn if_stmt(&mut self, i: &If) {
        self.line(&format!("if {}:", self.expr_str(&i.cond)));
        self.push();
        self.block(&i.then);
        self.pop();
        for (cond, blk) in &i.elifs {
            self.line(&format!("elif {}:", self.expr_str(cond)));
            self.push();
            self.block(blk);
            self.pop();
        }
        if let Some(els) = &i.els {
            self.line("else:");
            self.push();
            self.block(els);
            self.pop();
        }
    }

    fn pat_str(&self, p: &Pat) -> String {
        match p {
            Pat::Wild(_) => "_".into(),
            Pat::Bind(id, name, ty, _) => format!("{name}[d{}]: {ty}", id.0),
            Pat::Lit(e) => self.expr_str(e),
            Pat::Ctor(name, tag, pats, _) => {
                let ps: Vec<String> = pats.iter().map(|p| self.pat_str(p)).collect();
                format!("{name}#{tag}({})", ps.join(", "))
            }
            Pat::Or(pats, _) => {
                let ps: Vec<String> = pats.iter().map(|p| self.pat_str(p)).collect();
                ps.join(" | ")
            }
            Pat::Range(lo, hi, _) => format!("{} to {}", self.expr_str(lo), self.expr_str(hi)),
            Pat::Tuple(pats, _) => {
                let ps: Vec<String> = pats.iter().map(|p| self.pat_str(p)).collect();
                format!("({})", ps.join(", "))
            }
            Pat::Array(pats, _) => {
                let ps: Vec<String> = pats.iter().map(|p| self.pat_str(p)).collect();
                format!("[{}]", ps.join(", "))
            }
        }
    }

    fn expr_str(&self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(v) => format!("{v}"),
            ExprKind::Float(v) => format!("{v}"),
            ExprKind::Str(s) => format!("{s:?}"),
            ExprKind::Bool(b) => format!("{b}"),
            ExprKind::None => "none".into(),
            ExprKind::Void => "void".into(),
            ExprKind::Var(id, name) => format!("{name}[d{}]", id.0),
            ExprKind::GlobalLoad(name) => format!("global {name}"),
            ExprKind::FnRef(id, name) => format!("&fn {name}[d{}]", id.0),
            ExprKind::VariantRef(enum_n, var_n, tag) => format!("{enum_n}::{var_n}#{tag}"),
            ExprKind::BinOp(l, op, r) => {
                format!("({} {:?} {})", self.expr_str(l), op, self.expr_str(r))
            }
            ExprKind::UnaryOp(op, e) => format!("({:?} {})", op, self.expr_str(e)),
            ExprKind::Call(id, name, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("{}[d{}]({})", name, id.0, a.join(", "))
            }
            ExprKind::IndirectCall(f, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("({})({})", self.expr_str(f), a.join(", "))
            }
            ExprKind::Builtin(bf, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("@{:?}({})", bf, a.join(", "))
            }
            ExprKind::Method(recv, ty_name, meth, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!(
                    "{}.{ty_name}::{meth}({})",
                    self.expr_str(recv),
                    a.join(", ")
                )
            }
            ExprKind::StringMethod(recv, meth, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("{}.str::{meth}({})", self.expr_str(recv), a.join(", "))
            }
            ExprKind::DeferredMethod(recv, meth, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("{}.?::{meth}({})", self.expr_str(recv), a.join(", "))
            }
            ExprKind::VecMethod(recv, meth, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("{}.vec::{meth}({})", self.expr_str(recv), a.join(", "))
            }
            ExprKind::MapMethod(recv, meth, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("{}.map::{meth}({})", self.expr_str(recv), a.join(", "))
            }
            ExprKind::VecNew(elems) => {
                let a: Vec<String> = elems.iter().map(|a| self.expr_str(a)).collect();
                format!("vec[{}]", a.join(", "))
            }
            ExprKind::MapNew => "map{}".into(),
            ExprKind::SetNew => "set{}".into(),
            ExprKind::PQNew => "pq{}".into(),
            ExprKind::NDArrayNew(dims) => {
                format!(
                    "ndarray[{}]",
                    dims.iter()
                        .map(|d| self.expr_str(d))
                        .collect::<Vec<_>>()
                        .join(" by ")
                )
            }
            ExprKind::SIMDNew(elems) => {
                format!(
                    "simd({})",
                    elems
                        .iter()
                        .map(|e| self.expr_str(e))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            ExprKind::SetMethod(recv, meth, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("{}.set::{meth}({})", self.expr_str(recv), a.join(", "))
            }
            ExprKind::PQMethod(recv, meth, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("{}.pq::{meth}({})", self.expr_str(recv), a.join(", "))
            }
            ExprKind::Field(recv, name, idx) => format!("{}.{name}#{idx}", self.expr_str(recv)),
            ExprKind::Index(arr, idx) => format!("{}[{}]", self.expr_str(arr), self.expr_str(idx)),
            ExprKind::Ternary(c, t, f) => format!(
                "({} ? {} : {})",
                self.expr_str(c),
                self.expr_str(t),
                self.expr_str(f)
            ),
            ExprKind::Coerce(e, kind) => format!("coerce<{:?}>({})", kind, self.expr_str(e)),
            ExprKind::Cast(e, ty) => format!("cast<{ty}>({})", self.expr_str(e)),
            ExprKind::Array(elems) => {
                let a: Vec<String> = elems.iter().map(|a| self.expr_str(a)).collect();
                format!("[{}]", a.join(", "))
            }
            ExprKind::Tuple(elems) => {
                let a: Vec<String> = elems.iter().map(|a| self.expr_str(a)).collect();
                format!("({})", a.join(", "))
            }
            ExprKind::Struct(name, fields) => {
                let fs: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        let n = f
                            .name
                            .map(|s| s.as_str())
                            .unwrap_or_else(|| "_".to_string());
                        format!("{n}: {}", self.expr_str(&f.value))
                    })
                    .collect();
                format!("{name}{{{}}}", fs.join(", "))
            }
            ExprKind::VariantCtor(enum_n, var_n, tag, fields) => {
                let fs: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        let n = f
                            .name
                            .map(|s| s.as_str())
                            .unwrap_or_else(|| "_".to_string());
                        format!("{n}: {}", self.expr_str(&f.value))
                    })
                    .collect();
                format!("{enum_n}::{var_n}#{tag}{{{}}}", fs.join(", "))
            }
            ExprKind::IfExpr(i) => {
                format!("(if {} then ... else ...)", self.expr_str(&i.cond))
            }
            ExprKind::Pipe(val, id, name, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!(
                    "{} |> {}[d{}]({})",
                    self.expr_str(val),
                    name,
                    id.0,
                    a.join(", ")
                )
            }
            ExprKind::Block(blk) => {
                if blk.is_empty() {
                    "{}".into()
                } else {
                    format!("{{ ... {} stmts }}", blk.len())
                }
            }
            ExprKind::Lambda(params, body) => {
                let ps: Vec<String> = params
                    .iter()
                    .map(|p| format!("{}: {}", p.name, p.ty))
                    .collect();
                format!("\\({}) -> {{ {} stmts }}", ps.join(", "), body.len())
            }
            ExprKind::Ref(e) => format!("&{}", self.expr_str(e)),
            ExprKind::Deref(e) => format!("*{}", self.expr_str(e)),
            ExprKind::ListComp(expr, id, name, iter, cond, transform) => {
                let c = cond
                    .as_ref()
                    .map(|c| format!(" if {}", self.expr_str(c)))
                    .unwrap_or_default();
                let t = transform
                    .as_ref()
                    .map(|t| format!(" => {}", self.expr_str(t)))
                    .unwrap_or_default();
                format!(
                    "[{} for {}[d{}] in {}{}{}]",
                    self.expr_str(expr),
                    name,
                    id.0,
                    self.expr_str(iter),
                    c,
                    t
                )
            }
            ExprKind::Syscall(args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("syscall({})", a.join(", "))
            }
            ExprKind::Spawn(name) => format!("spawn {name}"),
            ExprKind::Send(recv, handler, actor, tag, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!(
                    "send {}.{actor}::{handler}#{tag}({})",
                    self.expr_str(recv),
                    a.join(", ")
                )
            }
            ExprKind::StoreQuery(name, _) => format!("store_query {name} ..."),
            ExprKind::StoreCount(name) => format!("store_count {name}"),
            ExprKind::StoreAll(name) => format!("store_all {name}"),
            ExprKind::StoreGet(name, key) => format!("store_get {name} {}", self.expr_str(key)),
            ExprKind::StoreFirst(name, _) => format!("store_first {name} ..."),
            ExprKind::StoreExists(name, _) => format!("store_exists {name} ..."),
            ExprKind::StoreDistinct(name, field) => format!("store_distinct {name}.{field}"),
            ExprKind::StoreSum(name, field) => format!("store_sum {name}.{field}"),
            ExprKind::StoreAvg(name, field) => format!("store_avg {name}.{field}"),
            ExprKind::StoreMin(name, field) => format!("store_min {name}.{field}"),
            ExprKind::StoreMax(name, field) => format!("store_max {name}.{field}"),
            ExprKind::StoreVersionCount(name, _) => format!("store_version_count {name}"),
            ExprKind::StoreHistory(name, _) => format!("store_history {name}"),
            ExprKind::StoreAtVersion(name, _, _) => format!("store_at_version {name}"),
            ExprKind::ViewCount(name, _) => format!("view_count {name}"),
            ExprKind::ViewAll(name, _) => format!("view_all {name}"),
            ExprKind::KvGet(name, _) => format!("kv_get {name}"),
            ExprKind::KvHas(name, _) => format!("kv_has {name}"),
            ExprKind::KvCount(name) => format!("kv_count {name}"),
            ExprKind::KvSet(name, _, _) => format!("kv_set {name}"),
            ExprKind::KvDel(name, _) => format!("kv_del {name}"),
            ExprKind::KvIncr(name, _, _) => format!("kv_incr {name}"),
            ExprKind::VecNearest(name, _, _) => format!("vec_nearest {name}"),
            ExprKind::VecInsert(name, _) => format!("vec_insert {name}"),
            ExprKind::VecCount(name) => format!("vec_count {name}"),
            ExprKind::BloomTest(store, field, _) => format!("bloom_test {store}.{field}"),
            ExprKind::FtsSearch(store, field, _) => format!("fts_search {store}.{field}"),
            ExprKind::FtsCount(store, field) => format!("fts_count {store}.{field}"),
            ExprKind::GraphFrom(name, _) => format!("graph_from {name}"),
            ExprKind::GraphTo(name, _) => format!("graph_to {name}"),
            ExprKind::TsLatest(name) => format!("ts_latest {name}"),
            ExprKind::CoroutineCreate(name, _) => format!("coroutine {name}"),
            ExprKind::CoroutineNext(e) => format!("{}.next()", self.expr_str(e)),
            ExprKind::Yield(e) => format!("yield {}", self.expr_str(e)),
            ExprKind::DynDispatch(obj, trait_name, method, _args) => {
                format!("({}).{trait_name}::{method}(...)", self.expr_str(obj))
            }
            ExprKind::DynCoerce(e, _ty, trait_name) => {
                format!("dyn {trait_name}({})", self.expr_str(e))
            }
            ExprKind::IterNext(var, ty, method) => {
                format!("{var}.{ty}_{method}()")
            }
            ExprKind::ChannelCreate(ty, cap) => {
                format!("channel of {ty}({})", self.expr_str(cap))
            }
            ExprKind::ChannelSend(ch, val) => {
                format!("chan_send({}, {})", self.expr_str(ch), self.expr_str(val))
            }
            ExprKind::ChannelRecv(ch) => {
                format!("chan_recv({})", self.expr_str(ch))
            }
            ExprKind::Select(arms, _) => {
                format!("select({} arms)", arms.len())
            }
            ExprKind::Unreachable => "unreachable".into(),
            ExprKind::StrictCast(e, ty) => format!("strict_cast<{ty}>({})", self.expr_str(e)),
            ExprKind::AsFormat(e, fmt) => format!("{} as {fmt}", self.expr_str(e)),
            ExprKind::AtomicLoad(e) => format!("atomic_load({})", self.expr_str(e)),
            ExprKind::AtomicStore(p, v) => {
                format!("atomic_store({}, {})", self.expr_str(p), self.expr_str(v))
            }
            ExprKind::AtomicAdd(p, v) => {
                format!("atomic_add({}, {})", self.expr_str(p), self.expr_str(v))
            }
            ExprKind::AtomicSub(p, v) => {
                format!("atomic_sub({}, {})", self.expr_str(p), self.expr_str(v))
            }
            ExprKind::AtomicCas(p, e, n) => format!(
                "atomic_cas({}, {}, {})",
                self.expr_str(p),
                self.expr_str(e),
                self.expr_str(n)
            ),
            ExprKind::Slice(obj, start, end) => format!(
                "{}[{} .. {}]",
                self.expr_str(obj),
                self.expr_str(start),
                self.expr_str(end)
            ),
            ExprKind::DequeNew => "deque()".into(),
            ExprKind::DequeMethod(recv, meth, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("{}.deque::{meth}({})", self.expr_str(recv), a.join(", "))
            }
            ExprKind::Grad(e) => format!("grad({})", self.expr_str(e)),
            ExprKind::Einsum(spec, args) => {
                let a: Vec<String> = args.iter().map(|a| self.expr_str(a)).collect();
                format!("einsum '{spec}' ({})", a.join(", "))
            }
            ExprKind::Builder(name, fields) => {
                let fs: Vec<String> = fields
                    .iter()
                    .map(|(n, v)| format!("{n}: {}", self.expr_str(v)))
                    .collect();
                format!("builder {name} {{ {} }}", fs.join(", "))
            }
            ExprKind::CowWrap(e) => format!("cow({})", self.expr_str(e)),
            ExprKind::CowClone(e) => format!("cow_clone({})", self.expr_str(e)),
            ExprKind::GeneratorCreate(_, name, _) => format!("generator {name}"),
            ExprKind::GeneratorNext(e) => format!("{}.next()", self.expr_str(e)),
            ExprKind::EnumUnwrap(e, _, _) => format!("{}.unwrap()", self.expr_str(e)),
            ExprKind::EnumIs(e, tag) => format!("{}.is_tag({})", self.expr_str(e), tag),
        }
    }
}
