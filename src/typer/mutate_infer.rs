use std::collections::{HashMap, HashSet};

use crate::ast::{self, Expr, Stmt};
use crate::intern::Symbol;

const BUILTIN_MUTATING_METHODS: &[&str] = &[
    "push",
    "push_back",
    "push_front",
    "pop",
    "pop_back",
    "pop_front",
    "insert",
    "remove",
    "delete",
    "set",
    "put",
    "add",
    "clear",
    "append",
    "extend",
    "sort",
    "sort_by",
    "reverse",
    "enqueue",
    "dequeue",
    "truncate",
    "swap",
    "fill",
    "retain",
];

pub(in crate::typer) fn is_builtin_mutating_method(name: &str) -> bool {
    BUILTIN_MUTATING_METHODS.contains(&name)
}

type AliasMap = HashMap<Symbol, HashSet<usize>>;

impl crate::typer::Typer {
    pub(crate) fn infer_mutating_params(
        &mut self,
        fns: &[&ast::Fn],
        methods: &[(Symbol, ast::Fn)],
    ) {
        for f in fns {
            self.fn_param_mutates
                .entry(f.name)
                .or_insert_with(|| vec![false; f.params.len()]);
        }
        for (mangled, m) in methods {
            let slots = 1 + m
                .params
                .iter()
                .filter(|p| p.name.as_str() != "self")
                .count();
            self.fn_param_mutates
                .entry(*mangled)
                .or_insert_with(|| vec![false; slots]);
        }
        loop {
            let mut changed = false;
            for f in fns {
                changed |= self.mutate_scan_one(f.name, f, 0);
            }
            for (mangled, m) in methods {
                changed |= self.mutate_scan_one(*mangled, m, 1);
            }
            if !changed {
                break;
            }
        }
    }

    fn mutate_scan_one(&mut self, fname: Symbol, f: &ast::Fn, offset: usize) -> bool {
        let mut alias: AliasMap = AliasMap::new();
        if offset == 1 {
            alias.insert("self".into(), HashSet::from([0]));
        }
        let mut slot = offset;
        for p in &f.params {
            if p.name.as_str() == "self" {
                continue;
            }
            alias.insert(p.name, HashSet::from([slot]));
            slot += 1;
        }
        let mut mutated: HashSet<usize> = HashSet::new();
        self.mutate_scan_block(&f.body, &mut alias, &mut mutated);
        let mut changed = false;
        if let Some(slots) = self.fn_param_mutates.get_mut(&fname) {
            for i in mutated {
                if let Some(s) = slots.get_mut(i)
                    && !*s
                {
                    *s = true;
                    changed = true;
                }
            }
        }
        changed
    }

    fn mutate_scan_block(
        &self,
        block: &[Stmt],
        alias: &mut AliasMap,
        mutated: &mut HashSet<usize>,
    ) {
        for s in block {
            self.mutate_scan_stmt(s, alias, mutated);
        }
    }

    fn mutate_scan_stmt(&self, s: &Stmt, alias: &mut AliasMap, mutated: &mut HashSet<usize>) {
        match s {
            Stmt::Bind(b) => {
                self.mutate_scan_expr(&b.value, alias, mutated);
                let set = Self::mutate_expr_alias(&b.value, alias);
                if !set.is_empty() {
                    alias.entry(b.name).or_default().extend(set);
                }
            }
            Stmt::TupleBind(_, e, _) => self.mutate_scan_expr(e, alias, mutated),
            Stmt::Assign(target, value, _) => {
                self.mutate_scan_expr(value, alias, mutated);
                self.mutate_scan_expr(target, alias, mutated);
                match target {
                    Expr::Ident(n, _) => {
                        let set = Self::mutate_expr_alias(value, alias);
                        if !set.is_empty() {
                            alias.entry(*n).or_default().extend(set);
                        }
                    }
                    _ => {
                        if let Some(root) = Self::lvalue_root(target)
                            && let Some(set) = alias.get(&root)
                        {
                            mutated.extend(set.iter().copied());
                        }
                    }
                }
            }
            Stmt::Expr(e)
            | Stmt::Ret(Some(e), _)
            | Stmt::ErrReturn(e, _)
            | Stmt::Break(Some(e), _) => {
                self.mutate_scan_expr(e, alias, mutated);
            }
            Stmt::If(i) => {
                self.mutate_scan_expr(&i.cond, alias, mutated);
                self.mutate_scan_block(&i.then, alias, mutated);
                for (c, b) in &i.elifs {
                    self.mutate_scan_expr(c, alias, mutated);
                    self.mutate_scan_block(b, alias, mutated);
                }
                if let Some(b) = &i.els {
                    self.mutate_scan_block(b, alias, mutated);
                }
            }
            Stmt::While(w) => {
                self.mutate_scan_expr(&w.cond, alias, mutated);
                self.mutate_scan_block(&w.body, alias, mutated);
            }
            Stmt::For(f) | Stmt::SimFor(f, _) => {
                self.mutate_scan_expr(&f.iter, alias, mutated);
                if let Some(e) = &f.end {
                    self.mutate_scan_expr(e, alias, mutated);
                }
                if let Some(e) = &f.step {
                    self.mutate_scan_expr(e, alias, mutated);
                }
                self.mutate_scan_block(&f.body, alias, mutated);
            }
            Stmt::Loop(l) => self.mutate_scan_block(&l.body, alias, mutated),
            Stmt::Match(m) => {
                self.mutate_scan_expr(&m.subject, alias, mutated);
                for arm in &m.arms {
                    if let Some(g) = &arm.guard {
                        self.mutate_scan_expr(g, alias, mutated);
                    }
                    self.mutate_scan_block(&arm.body, alias, mutated);
                }
            }
            Stmt::Defer(b, _) | Stmt::Transaction(b, _) | Stmt::SimBlock(b, _) => {
                self.mutate_scan_block(b, alias, mutated);
            }
            Stmt::Together(_, b, _, _) => self.mutate_scan_block(b, alias, mutated),
            Stmt::StoreInsert(_, inits, _) => {
                for fi in inits {
                    self.mutate_scan_expr(&fi.value, alias, mutated);
                }
            }
            Stmt::StoreSet(_, sets, _, _) => {
                for (_, e) in sets {
                    self.mutate_scan_expr(e, alias, mutated);
                }
            }
            Stmt::ChannelClose(e, _) | Stmt::Stop(e, _) | Stmt::Join(e, _) => {
                self.mutate_scan_expr(e, alias, mutated);
            }
            _ => {}
        }
    }

    fn lvalue_root(e: &Expr) -> Option<Symbol> {
        match e {
            Expr::Ident(n, _) => Some(*n),
            Expr::Field(x, _, _) | Expr::Index(x, _, _) | Expr::Deref(x, _) => Self::lvalue_root(x),
            _ => None,
        }
    }

    fn mutate_expr_alias(e: &Expr, alias: &AliasMap) -> HashSet<usize> {
        match e {
            Expr::Ident(n, _) => alias.get(n).cloned().unwrap_or_default(),
            Expr::Ternary(_, t, els, _) => {
                let mut s = Self::mutate_expr_alias(t, alias);
                s.extend(Self::mutate_expr_alias(els, alias));
                s
            }
            Expr::As(inner, _, _) => Self::mutate_expr_alias(inner, alias),
            _ => HashSet::new(),
        }
    }

    pub(in crate::typer) fn any_user_method_mutates(&self, method: Symbol, slot: usize) -> bool {
        self.methods.iter().any(|(ty, ms)| {
            ms.iter().any(|m| m.name == method) && {
                let mangled: Symbol = format!("{}_{}", ty.as_str(), method.as_str()).into();
                self.fn_param_mutates
                    .get(&mangled)
                    .map(|slots| slots.get(slot).copied().unwrap_or(false))
                    .unwrap_or(false)
            }
        })
    }

    fn mutate_scan_expr(&self, e: &Expr, alias: &AliasMap, mutated: &mut HashSet<usize>) {
        match e {
            Expr::Call(callee, args, _) => {
                if !args.iter().any(|a| matches!(a, Expr::NamedArg(..))) {
                    let fname = match &**callee {
                        Expr::Ident(n, _) => Some(*n),
                        Expr::QualifiedIdent(_, n, _) => Some(*n),
                        _ => None,
                    };
                    if let Some(fname) = fname
                        && let Some(slots) = self.fn_param_mutates.get(&fname)
                    {
                        for (j, a) in args.iter().enumerate() {
                            if slots.get(j).copied().unwrap_or(false) {
                                mutated.extend(Self::mutate_expr_alias(a, alias));
                            }
                        }
                    }
                }
                for a in args {
                    self.mutate_scan_expr(a, alias, mutated);
                }
            }
            Expr::Method(recv, name, args, _) => {
                if is_builtin_mutating_method(&name.as_str())
                    || self.any_user_method_mutates(*name, 0)
                {
                    mutated.extend(Self::mutate_expr_alias(recv, alias));
                }
                for (j, a) in args.iter().enumerate() {
                    if self.any_user_method_mutates(*name, j + 1) {
                        mutated.extend(Self::mutate_expr_alias(a, alias));
                    }
                }
                self.mutate_scan_expr(recv, alias, mutated);
                for a in args {
                    self.mutate_scan_expr(a, alias, mutated);
                }
            }
            Expr::Pipe(lhs, target, rest, _) => {
                if let Expr::Ident(fname, _) = &**target
                    && let Some(slots) = self.fn_param_mutates.get(fname)
                    && slots.first().copied().unwrap_or(false)
                {
                    mutated.extend(Self::mutate_expr_alias(lhs, alias));
                }
                self.mutate_scan_expr(lhs, alias, mutated);
                for a in rest {
                    self.mutate_scan_expr(a, alias, mutated);
                }
            }
            Expr::ChannelSend(ch, v, _) => {
                self.mutate_scan_expr(ch, alias, mutated);
                self.mutate_scan_expr(v, alias, mutated);
            }
            Expr::Send(actor, _, args, _) => {
                self.mutate_scan_expr(actor, alias, mutated);
                for a in args {
                    self.mutate_scan_expr(a, alias, mutated);
                }
            }
            Expr::Spawn(_, inits, _) => {
                for (_, v) in inits {
                    self.mutate_scan_expr(v, alias, mutated);
                }
            }
            Expr::Yield(v, _) => self.mutate_scan_expr(v, alias, mutated),
            Expr::BinOp(l, _, r, _) | Expr::Index(l, r, _) | Expr::OfCall(l, r, _) => {
                self.mutate_scan_expr(l, alias, mutated);
                self.mutate_scan_expr(r, alias, mutated);
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
                self.mutate_scan_expr(x, alias, mutated);
            }
            Expr::Ternary(c, t, els, _) => {
                self.mutate_scan_expr(c, alias, mutated);
                self.mutate_scan_expr(t, alias, mutated);
                self.mutate_scan_expr(els, alias, mutated);
            }
            Expr::Quaternary(subj, ok, nothing, err, _) => {
                self.mutate_scan_expr(subj, alias, mutated);
                for arm in [ok, nothing, err].into_iter().flatten() {
                    self.mutate_scan_expr(arm, alias, mutated);
                }
            }
            Expr::Array(es, _)
            | Expr::Tuple(es, _)
            | Expr::Syscall(es, _)
            | Expr::Einsum(_, es, _) => {
                for x in es {
                    self.mutate_scan_expr(x, alias, mutated);
                }
            }
            Expr::Struct(_, inits, _) => {
                for fi in inits {
                    self.mutate_scan_expr(&fi.value, alias, mutated);
                }
            }
            Expr::Slice(a, b, c, _) => {
                self.mutate_scan_expr(a, alias, mutated);
                self.mutate_scan_expr(b, alias, mutated);
                self.mutate_scan_expr(c, alias, mutated);
            }
            _ => {}
        }
    }
}
