//! Inferred consuming parameters (task 8-6, memory-model.md rule M6).
//!
//! A function whose body lets a parameter's buffer *leave* the function —
//! returned (directly or through an alias chain / aggregate literal),
//! pushed into a container, sent on a channel or to an actor, or passed on
//! to another consuming parameter — must own that parameter: the caller's
//! argument moves into the call exactly as an explicit `take` would.
//! Without this, the caller and its own result binding both believe they
//! own one buffer and the scope-exit drops double-free it (review §3.1).
//!
//! The analysis runs on the AST, after every signature has populated
//! `fn_param_access` and before any body is lowered, so call sites and
//! callees see one consistent answer regardless of lowering order. It is a
//! whole-program fixpoint: marking `f`'s parameter consuming can make a
//! caller `g` that forwards its own parameter to `f` consuming too.
//!
//! Deliberately conservative in both directions and documented as such:
//!
//! - flow-insensitive over-approximation (an escape on any path marks the
//!   parameter), which can turn a caller's later use into a use-after-move
//!   error — the D1-correct outcome;
//! - structural under-approximation (escapes through lambda bodies, block
//!   expressions, or named-argument calls are not tracked), which leaves
//!   those double-frees to task 8-7's single flow-sensitive analysis.
//!
//! Parameters with an explicit access modifier are never touched, and
//! parameters explicitly annotated as scalars or `String` are skipped:
//! scalars cannot double-free, and `String` is a value type (deep copy)
//! per memory-model.md §1.

use std::collections::{HashMap, HashSet};

use crate::ast::{self, Expr, Stmt};
use crate::intern::Symbol;
use crate::types::Type;

/// Method names that transfer ownership of their arguments into the
/// receiver. Mirrors the list in `typer/lower/block.rs`
/// (`collect_consumed_in_expr`).
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

/// Alias state for one function body: local name → set of parameter
/// indices whose buffer the name may hold. Union-only (never narrowed by a
/// rebind), which keeps the analysis a sound over-approximation across
/// branches without flow tracking.
type AliasMap = HashMap<Symbol, HashSet<usize>>;

impl crate::typer::Typer {
    /// Entry point, called from `lower_program` before function lowering.
    pub(crate) fn infer_consuming_params(&mut self, fns: &[&ast::Fn]) {
        loop {
            let mut changed = false;
            for f in fns {
                let mut alias: AliasMap = AliasMap::new();
                for (i, p) in f.params.iter().enumerate() {
                    if p.access_mod.is_some() || annotated_non_consumable(&p.ty) {
                        continue;
                    }
                    alias.insert(p.name, HashSet::from([i]));
                }
                if alias.is_empty() {
                    continue;
                }
                let mut escaping: HashSet<usize> = HashSet::new();
                let returns_value = f.ret.is_some() || ret_is_inferred(f);
                self.scan_block(&f.body, &mut alias, &mut escaping, returns_value);

                for i in escaping {
                    if let Some(accs) = self.fn_param_access.get_mut(&f.name)
                        && let Some(slot) = accs.get_mut(i)
                        && slot.is_none()
                    {
                        *slot = Some(ast::AccessMod::Take);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
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
                    // Storing into a field/index/deref of anything: the
                    // buffer now lives inside another value; treat as an
                    // escape (its new home may outlive the call frame).
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

    /// Parameter indices whose buffer the expression's *value* may hold.
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

    /// Walk every subexpression, recording parameters that flow into a
    /// consuming position: a `take` (explicit or already-inferred) call
    /// argument, a container-push argument, a channel send, an actor send,
    /// a spawn initializer, or a yield.
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
                }
                self.scan_expr_sinks(recv, alias, escaping);
                for a in args {
                    self.scan_expr_sinks(a, alias, escaping);
                }
            }
            Expr::Pipe(lhs, target, rest, _) => {
                // `v ~ f` is `f(v, ...)`; treat the piped value like the
                // first positional argument of the target.
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

            // Pure structural recursion.
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
            // Lambda bodies and block expressions are not tracked (see
            // module docs); their escapes are 8-7's to catch.
            _ => {}
        }
    }
}

fn ret_is_inferred(f: &ast::Fn) -> bool {
    // A function with no `returns` clause may still return a value via a
    // tail expression; treat its tail as returning unless it is `main`.
    f.name.as_str() != "main"
}
