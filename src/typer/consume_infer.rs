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

impl crate::typer::Typer {
    pub(crate) fn infer_consuming_params(&mut self, fns: &[&ast::Fn]) {
        let method_items: Vec<(Symbol, ast::Fn)> = self
            .methods
            .iter()
            .flat_map(|(ty, ms)| {
                ms.iter().map(move |m| {
                    let mangled: Symbol = format!("{}_{}", ty.as_str(), m.name.as_str()).into();
                    (mangled, m.clone())
                })
            })
            .collect();
        loop {
            let mut changed = false;
            for f in fns {
                changed |= self.consume_scan_one(f.name, f, 0);
            }
            for (mangled, m) in &method_items {
                changed |= self.consume_scan_one(*mangled, m, 1);
            }
            if !changed {
                break;
            }
        }
        self.infer_mutating_params(fns, &method_items);
    }

    fn consume_scan_one(&mut self, fname: Symbol, f: &ast::Fn, offset: usize) -> bool {
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
        let mut escaping: HashSet<usize> = HashSet::new();
        let returns_value = f.ret.is_some() || ret_is_inferred(f);
        self.scan_block(&f.body, &mut alias, &mut escaping, returns_value);

        let mut changed = false;
        for i in escaping {
            if let Some(accs) = self.fn_param_access.get_mut(&fname)
                && let Some(slot) = accs.get_mut(i)
                && slot.is_none()
            {
                *slot = Some(ast::AccessMod::Take);
                changed = true;
            }
        }
        changed
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
        escaping: &mut HashSet<usize>,
        tail_returns: bool,
    ) {
        let last = block.len().saturating_sub(1);
        for (idx, s) in block.iter().enumerate() {
            let is_tail = tail_returns && idx == last;
            self.scan_stmt(s, alias, escaping, is_tail);
        }
    }

    fn scan_stmt(
        &self,
        s: &Stmt,
        alias: &mut AliasMap,
        escaping: &mut HashSet<usize>,
        is_tail: bool,
    ) {
        match s {
            Stmt::Bind(b) => {
                self.scan_expr_sinks(&b.value, alias, escaping);
                let set = Self::expr_alias(&b.value, alias);
                if !set.is_empty() {
                    alias.entry(b.name).or_default().extend(set);
                }
            }
            Stmt::TupleBind(_, e, _) => self.scan_expr_sinks(e, alias, escaping),
            Stmt::Assign(target, value, _) => {
                self.scan_expr_sinks(value, alias, escaping);
                self.scan_expr_sinks(target, alias, escaping);
                match target {
                    Expr::Ident(n, _) => {
                        let set = Self::expr_alias(value, alias);
                        if !set.is_empty() {
                            alias.entry(*n).or_default().extend(set);
                        }
                    }

                    _ => {
                        escaping.extend(Self::expr_alias(value, alias));
                    }
                }
            }
            Stmt::Expr(e) => {
                self.scan_expr_sinks(e, alias, escaping);
                if is_tail {
                    escaping.extend(Self::expr_alias(e, alias));
                }
            }
            Stmt::Ret(Some(e), _) | Stmt::ErrReturn(e, _) | Stmt::Break(Some(e), _) => {
                self.scan_expr_sinks(e, alias, escaping);
                escaping.extend(Self::expr_alias(e, alias));
            }
            Stmt::If(i) => {
                self.scan_expr_sinks(&i.cond, alias, escaping);
                self.scan_block(&i.then, alias, escaping, is_tail);
                for (c, b) in &i.elifs {
                    self.scan_expr_sinks(c, alias, escaping);
                    self.scan_block(b, alias, escaping, is_tail);
                }
                if let Some(b) = &i.els {
                    self.scan_block(b, alias, escaping, is_tail);
                }
            }
            Stmt::While(w) => {
                self.scan_expr_sinks(&w.cond, alias, escaping);
                self.scan_block(&w.body, alias, escaping, false);
            }
            Stmt::For(f) | Stmt::SimFor(f, _) => {
                self.scan_expr_sinks(&f.iter, alias, escaping);
                if let Some(e) = &f.end {
                    self.scan_expr_sinks(e, alias, escaping);
                }
                if let Some(e) = &f.step {
                    self.scan_expr_sinks(e, alias, escaping);
                }
                self.scan_block(&f.body, alias, escaping, false);
            }
            Stmt::Loop(l) => self.scan_block(&l.body, alias, escaping, false),
            Stmt::Match(m) => {
                self.scan_expr_sinks(&m.subject, alias, escaping);
                for arm in &m.arms {
                    if let Some(g) = &arm.guard {
                        self.scan_expr_sinks(g, alias, escaping);
                    }
                    self.scan_block(&arm.body, alias, escaping, is_tail);
                }
            }
            Stmt::Defer(b, _) | Stmt::Transaction(b, _) | Stmt::SimBlock(b, _) => {
                self.scan_block(b, alias, escaping, false);
            }
            Stmt::Together(_, b, _, _) => self.scan_block(b, alias, escaping, false),
            Stmt::StoreInsert(_, inits, _) => {
                for fi in inits {
                    self.scan_expr_sinks(&fi.value, alias, escaping);
                }
            }
            Stmt::StoreSet(_, sets, _, _) => {
                for (_, e) in sets {
                    self.scan_expr_sinks(e, alias, escaping);
                }
            }
            Stmt::ChannelClose(e, _) | Stmt::Stop(e, _) | Stmt::Join(e, _) => {
                self.scan_expr_sinks(e, alias, escaping);
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

    fn scan_expr_sinks(&self, e: &Expr, alias: &AliasMap, escaping: &mut HashSet<usize>) {
        match e {
            Expr::Call(callee, args, _) => {
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
                                escaping.extend(Self::expr_alias(a, alias));
                            }
                        }
                    }
                }
                for a in args {
                    self.scan_expr_sinks(a, alias, escaping);
                }
            }
            Expr::Method(recv, name, args, _) => {
                if CONSUMING_METHODS.contains(&&*name.as_str()) {
                    for a in args {
                        escaping.extend(Self::expr_alias(a, alias));
                    }
                } else {
                    for (j, a) in args.iter().enumerate() {
                        if self.any_user_method_consumes(*name, j + 1) {
                            escaping.extend(Self::expr_alias(a, alias));
                        }
                    }
                }
                self.scan_expr_sinks(recv, alias, escaping);
                for a in args {
                    self.scan_expr_sinks(a, alias, escaping);
                }
            }
            Expr::Pipe(lhs, target, rest, _) => {
                if let Expr::Ident(fname, _) = &**target
                    && let Some(access) = self.fn_param_access.get(fname)
                    && matches!(access.first(), Some(Some(ast::AccessMod::Take)))
                {
                    escaping.extend(Self::expr_alias(lhs, alias));
                }
                self.scan_expr_sinks(lhs, alias, escaping);
                for a in rest {
                    self.scan_expr_sinks(a, alias, escaping);
                }
            }
            Expr::ChannelSend(ch, v, _) => {
                escaping.extend(Self::expr_alias(v, alias));
                self.scan_expr_sinks(ch, alias, escaping);
                self.scan_expr_sinks(v, alias, escaping);
            }
            Expr::Send(actor, _, args, _) => {
                for a in args {
                    escaping.extend(Self::expr_alias(a, alias));
                    self.scan_expr_sinks(a, alias, escaping);
                }
                self.scan_expr_sinks(actor, alias, escaping);
            }
            Expr::Spawn(_, inits, _) => {
                for (_, v) in inits {
                    escaping.extend(Self::expr_alias(v, alias));
                    self.scan_expr_sinks(v, alias, escaping);
                }
            }
            Expr::Yield(v, _) => {
                escaping.extend(Self::expr_alias(v, alias));
                self.scan_expr_sinks(v, alias, escaping);
            }

            Expr::BinOp(l, _, r, _) | Expr::Index(l, r, _) | Expr::OfCall(l, r, _) => {
                self.scan_expr_sinks(l, alias, escaping);
                self.scan_expr_sinks(r, alias, escaping);
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
                self.scan_expr_sinks(x, alias, escaping);
            }
            Expr::Ternary(c, t, els, _) => {
                self.scan_expr_sinks(c, alias, escaping);
                self.scan_expr_sinks(t, alias, escaping);
                self.scan_expr_sinks(els, alias, escaping);
            }
            Expr::Quaternary(subj, ok, nothing, err, _) => {
                self.scan_expr_sinks(subj, alias, escaping);
                for arm in [ok, nothing, err].into_iter().flatten() {
                    self.scan_expr_sinks(arm, alias, escaping);
                }
            }
            Expr::Array(es, _)
            | Expr::Tuple(es, _)
            | Expr::Syscall(es, _)
            | Expr::Einsum(_, es, _) => {
                for x in es {
                    self.scan_expr_sinks(x, alias, escaping);
                }
            }
            Expr::Struct(_, inits, _) => {
                for fi in inits {
                    self.scan_expr_sinks(&fi.value, alias, escaping);
                }
            }
            Expr::Slice(a, b, c, _) => {
                self.scan_expr_sinks(a, alias, escaping);
                self.scan_expr_sinks(b, alias, escaping);
                self.scan_expr_sinks(c, alias, escaping);
            }

            _ => {}
        }
    }
}

fn ret_is_inferred(f: &ast::Fn) -> bool {
    f.name.as_str() != "main"
}
