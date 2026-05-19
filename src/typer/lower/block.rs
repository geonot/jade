//! Extracted lowering steps.

#![allow(unused_imports, unused_variables)]

use std::collections::{HashMap, HashSet};

use super::super::unify;
use super::super::{DeferredField, DeferredMethod, Typer, VarInfo};
use crate::ast::{self, Span};
use crate::hir::{self, CoercionKind, DefId, ExprKind, Ownership};
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn hir_tail_type(&self, body: &[hir::Stmt]) -> Option<Type> {
        let last = body
            .iter()
            .rev()
            .find(|s| !matches!(s, hir::Stmt::Drop(..)))?;
        match last {
            hir::Stmt::Expr(e) if e.ty != Type::Void => Some(e.ty.clone()),
            hir::Stmt::If(i) => {
                if i.els.is_some() {
                    self.hir_tail_type(&i.then)
                } else {
                    None
                }
            }
            hir::Stmt::Match(m) => {
                if let Some(arm) = m.arms.first() {
                    self.hir_tail_type(&arm.body)
                } else {
                    None
                }
            }
            hir::Stmt::Ret(Some(e), _, _) => Some(e.ty.clone()),
            _ => None,
        }
    }

    pub(in crate::typer) fn lower_block(
        &mut self,
        block: &ast::Block,
        ret_ty: &Type,
    ) -> Result<hir::Block, String> {
        self.push_scope();
        let mut stmts = self.lower_block_no_scope(block, ret_ty)?;
        self.finalize_block_drops(&mut stmts);
        self.pop_scope();
        Ok(stmts)
    }

    /// Finalize the scope-exit drop emission for a block whose statements
    /// have already been lowered with `lower_block_no_scope`. Encapsulates
    /// the tail-expression / jump handling that `lower_block` does so it
    /// can be reused by `lower_fn_deferred` (and other sites where params
    /// must live in the body's scope to receive proper drop emission).
    pub(in crate::typer) fn finalize_block_drops(&mut self, stmts: &mut Vec<hir::Stmt>) {
        let ends_with_jump = stmts.last().map_or(false, |s| {
            matches!(
                s,
                hir::Stmt::Ret(..) | hir::Stmt::Break(..) | hir::Stmt::Continue(..)
            )
        });
        if ends_with_jump {
            let jump = stmts.pop().unwrap();
            // Collect variable IDs referenced in the jump expression so we
            // don't drop them before they're consumed by the return/break.
            let mut jump_refs = std::collections::HashSet::new();
            Self::collect_hir_var_ids_stmt(&jump, &mut jump_refs);
            self.emit_scope_drops_excluding(stmts, &jump_refs);
            stmts.push(jump);
        } else if let Some(hir::Stmt::Expr(_)) = stmts.last() {
            // Implicit return: emit drops AFTER the tail evaluates, excluding
            // any var that was *moved* into the tail (struct ctor / tuple /
            // bare var). Vars merely referenced (via field access, method
            // call, etc.) still need to be dropped, but the drop must happen
            // AFTER the tail has read them. Emitting before-tail would free
            // storage the tail still needs.
            let tail = stmts.pop().unwrap();
            let mut tail_moves = std::collections::HashSet::new();
            if let hir::Stmt::Expr(te) = &tail {
                Self::collect_moved_var_ids(te, &mut tail_moves);
            }
            stmts.push(tail);
            self.emit_scope_drops_excluding(stmts, &tail_moves);
        } else {
            self.emit_scope_drops(stmts);
        }
    }

    pub(in crate::typer) fn emit_scope_drops(&mut self, stmts: &mut Vec<hir::Stmt>) {
        self.emit_scope_drops_excluding(stmts, &std::collections::HashSet::new());
    }

    /// Collect variable IDs that have been moved (consumed) by a statement-
    /// level expression earlier in the same block. Currently we recognise
    /// container "consume on push" methods: pushing/inserting a heap-typed
    /// bare variable into a Vec/Set/Map/PQ moves the value into the container,
    /// so it must not be dropped at scope exit (the container owns the
    /// underlying storage now).
    fn collect_block_consumed_ids(
        &mut self,
        stmts: &[hir::Stmt],
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        for s in stmts {
            match s {
                hir::Stmt::Expr(e) => self.collect_consumed_in_expr(e, out),
                // `x is y` (Assign) and `let x = y` (Bind) where `y` is a
                // bare variable of heap-managed type *moves* y into x. The
                // sole owner becomes x; y must not be dropped at scope exit.
                // Without this, both x and y get dropped and the underlying
                // buffer is freed twice (or x is left dangling if y's drop
                // runs first inside a loop body). Resolve any inference
                // variables before checking — the RHS type at this point may
                // still be a TypeVar that resolves to a heap type.
                hir::Stmt::Assign(_target, value, _) => {
                    let resolved = self.infer_ctx.resolve(&value.ty);
                    if Self::expr_type_needs_drop(&resolved) {
                        if let hir::ExprKind::Var(id, _) = &value.kind {
                            out.insert(*id);
                        }
                    }
                }
                hir::Stmt::Bind(b) => {
                    let resolved = self.infer_ctx.resolve(&b.value.ty);
                    if Self::expr_type_needs_drop(&resolved) {
                        if let hir::ExprKind::Var(id, _) = &b.value.kind {
                            out.insert(*id);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_consumed_in_expr(
        &mut self,
        expr: &hir::Expr,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match &expr.kind {
            hir::ExprKind::VecMethod(_, meth, args) | hir::ExprKind::MapMethod(_, meth, args) => {
                let m_owned = meth.as_str();
                let m: &str = m_owned.as_ref();
                if matches!(
                    m,
                    "push"
                        | "push_back"
                        | "push_front"
                        | "insert"
                        | "append"
                        | "add"
                        | "put"
                        | "set"
                        | "enqueue"
                        | "send"
                ) {
                    for a in args {
                        let resolved = self.infer_ctx.resolve(&a.ty);
                        if Self::expr_type_needs_drop(&resolved)
                            && matches!(a.kind, hir::ExprKind::Var(_, _))
                        {
                            if let hir::ExprKind::Var(id, _) = &a.kind {
                                out.insert(*id);
                            }
                        }
                    }
                }
            }
            // Stage B: callee-sig aware consume tracking. For each call
            // argument whose corresponding callee parameter is `take`,
            // mark the bare-Var argument as consumed so the caller's
            // scope-exit drop is suppressed. This implements the move
            // semantics required by `docs/access-semantics.md` §4.3 / §4.6
            // for explicit-`take` parameters, without breaking the (still
            // borrow-by-default) behavior for unannotated heap params.
            hir::ExprKind::Call(_, name, args) => {
                let access = self.fn_param_access.get(name).cloned();
                if let Some(access) = access {
                    for (i, a) in args.iter().enumerate() {
                        if matches!(access.get(i), Some(Some(crate::ast::AccessMod::Take)))
                            && let hir::ExprKind::Var(id, _) = &a.kind
                        {
                            let resolved = self.infer_ctx.resolve(&a.ty);
                            if Self::expr_type_needs_drop(&resolved) {
                                out.insert(*id);
                            }
                        }
                    }
                }
                // Walk into nested call args (e.g. f(g(x))). For a nested
                // call's bare-Var arg, `g(x)` already handles `x` via its
                // own callee-sig check; the outer call sees `g(x)` as the
                // arg expression (not a Var), so no double-handling.
                for a in args {
                    self.collect_consumed_in_expr(a, out);
                }
            }
            // Method dispatch on a struct/trait method: look up the
            // mangled `Type_method` signature in `fns` (which is the same
            // form `declare_method_sig_impl` registers under).
            hir::ExprKind::Method(recv, ty_name, m_name, args) => {
                let mangled: crate::intern::Symbol =
                    format!("{}_{}", ty_name.as_str(), m_name.as_str()).into();
                let access = self.fn_param_access.get(&mangled).cloned();
                if let Some(access) = access {
                    // access[0] corresponds to the synthetic `self` param.
                    // User-supplied args start at index 1.
                    for (i, a) in args.iter().enumerate() {
                        if matches!(access.get(i + 1), Some(Some(crate::ast::AccessMod::Take)))
                            && let hir::ExprKind::Var(id, _) = &a.kind
                        {
                            let resolved = self.infer_ctx.resolve(&a.ty);
                            if Self::expr_type_needs_drop(&resolved) {
                                out.insert(*id);
                            }
                        }
                    }
                }
                self.collect_consumed_in_expr(recv, out);
                for a in args {
                    self.collect_consumed_in_expr(a, out);
                }
            }
            _ => {}
        }
    }

    fn expr_type_needs_drop(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Vec(_)
                | Type::Map(_, _)
                | Type::String
                | Type::Struct(_, _)
                | Type::Enum(_)
        )
    }

    pub(in crate::typer) fn emit_scope_drops_excluding(
        &mut self,
        stmts: &mut Vec<hir::Stmt>,
        exclude: &std::collections::HashSet<crate::hir::DefId>,
    ) {
        // Snapshot scope entries up-front so we can take &mut self for resolve.
        let scope_entries: Vec<(crate::intern::Symbol, crate::typer::VarInfo)> =
            match self.scopes.last() {
                Some(s) => s.iter().map(|(n, v)| (n.clone(), v.clone())).collect(),
                None => return,
            };
        // In addition to the caller-supplied exclusion set (e.g. tail-expr
        // moves), also exclude any scope variable that was already moved into
        // a container by a prior stmt-level call (e.g. `g.push(row)`).
        let mut consumed: std::collections::HashSet<crate::hir::DefId> = exclude.clone();
        self.collect_block_consumed_ids(stmts, &mut consumed);
        // Resolve any inference variables so that needs_drop sees the concrete
        // type (e.g. Vec(_) rather than TypeVar). Without this, owned heap
        // locals whose Bind site introduced a fresh TyVar would be skipped by
        // needs_drop and silently leak. Pre-resolve all entries up-front to
        // satisfy the borrow checker (resolve needs &mut self.infer_ctx).
        let mut resolved_entries: Vec<(crate::intern::Symbol, crate::typer::VarInfo, Type)> =
            Vec::with_capacity(scope_entries.len());
        // Drop-emission resolves binding types only to decide which need a
        // matching `Drop` stmt. Unsolved type variables here are not a
        // diagnostic concern of this pass — they belong to inferable-fn
        // bodies that will either get fully solved by the post-lowering
        // resolver or replaced wholesale by monomorphization. Silence
        // strict-mode emissions for the duration of these resolves.
        let was_strict = self.infer_ctx.is_strict();
        self.infer_ctx.set_strict(false);
        for (name, info) in scope_entries {
            let resolved = self.infer_ctx.resolve(&info.ty);
            resolved_entries.push((name, info, resolved));
        }
        self.infer_ctx.set_strict(was_strict);
        let mut drops: Vec<_> = resolved_entries
            .into_iter()
            .filter(|(_, info, resolved)| {
                self.needs_drop(resolved)
                    && !matches!(
                        info.ownership,
                        crate::hir::Ownership::Borrowed | crate::hir::Ownership::BorrowMut
                    )
                    && !consumed.contains(&info.def_id)
            })
            .collect();
        drops.sort_by_key(|(_, info, _)| std::cmp::Reverse(info.def_id.0));
        for (name, info, resolved) in drops {
            stmts.push(hir::Stmt::Drop(
                info.def_id,
                name.clone(),
                resolved,
                crate::ast::Span::dummy(),
            ));
        }
    }

    /// Collect variable IDs that are *moved* (consumed) by an expression.
    /// Only struct constructors, tuple literals, and bare variable references
    /// count as moves. Method calls, field accesses, etc. borrow their receiver.
    pub(in crate::typer) fn collect_moved_var_ids(
        expr: &hir::Expr,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match &expr.kind {
            hir::ExprKind::Var(id, _) => {
                out.insert(*id);
            }
            hir::ExprKind::Struct(_, inits) | hir::ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    Self::collect_moved_var_ids(&fi.value, out);
                }
            }
            hir::ExprKind::Tuple(es) | hir::ExprKind::Array(es) => {
                for e in es {
                    Self::collect_moved_var_ids(e, out);
                }
            }
            _ => {}
        }
    }

    pub(in crate::typer) fn collect_hir_var_ids_expr(
        expr: &hir::Expr,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match &expr.kind {
            hir::ExprKind::Var(id, _) => {
                out.insert(*id);
            }
            hir::ExprKind::BinOp(l, _, r) => {
                Self::collect_hir_var_ids_expr(l, out);
                Self::collect_hir_var_ids_expr(r, out);
            }
            hir::ExprKind::UnaryOp(_, e) => Self::collect_hir_var_ids_expr(e, out),
            hir::ExprKind::Call(_, _, args) => {
                for a in args {
                    Self::collect_hir_var_ids_expr(a, out);
                }
            }
            hir::ExprKind::Struct(_, inits) | hir::ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    Self::collect_hir_var_ids_expr(&fi.value, out);
                }
            }
            hir::ExprKind::IfExpr(i) => {
                Self::collect_hir_var_ids_expr(&i.cond, out);
                for s in &i.then {
                    Self::collect_hir_var_ids_stmt(s, out);
                }
                for (c, b) in &i.elifs {
                    Self::collect_hir_var_ids_expr(c, out);
                    for s in b {
                        Self::collect_hir_var_ids_stmt(s, out);
                    }
                }
                if let Some(b) = &i.els {
                    for s in b {
                        Self::collect_hir_var_ids_stmt(s, out);
                    }
                }
            }
            hir::ExprKind::Index(e, i) => {
                Self::collect_hir_var_ids_expr(e, out);
                Self::collect_hir_var_ids_expr(i, out);
            }
            hir::ExprKind::Field(e, _, _) => Self::collect_hir_var_ids_expr(e, out),
            hir::ExprKind::Method(e, _, _, args)
            | hir::ExprKind::StringMethod(e, _, args)
            | hir::ExprKind::DeferredMethod(e, _, args)
            | hir::ExprKind::VecMethod(e, _, args)
            | hir::ExprKind::MapMethod(e, _, args) => {
                Self::collect_hir_var_ids_expr(e, out);
                for a in args {
                    Self::collect_hir_var_ids_expr(a, out);
                }
            }
            hir::ExprKind::Tuple(es) | hir::ExprKind::Array(es) => {
                for e in es {
                    Self::collect_hir_var_ids_expr(e, out);
                }
            }
            hir::ExprKind::Block(stmts) => {
                for s in stmts {
                    Self::collect_hir_var_ids_stmt(s, out);
                }
            }
            hir::ExprKind::Lambda(_, body) => {
                for s in body {
                    Self::collect_hir_var_ids_stmt(s, out);
                }
            }
            hir::ExprKind::Ref(e) | hir::ExprKind::Deref(e) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::ExprKind::Pipe(e, _, _, rest) => {
                Self::collect_hir_var_ids_expr(e, out);
                for a in rest {
                    Self::collect_hir_var_ids_expr(a, out);
                }
            }
            hir::ExprKind::Cast(e, _) => Self::collect_hir_var_ids_expr(e, out),
            _ => {}
        }
    }

    pub(in crate::typer) fn collect_hir_var_ids_stmt(
        stmt: &hir::Stmt,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match stmt {
            hir::Stmt::Expr(e) | hir::Stmt::Bind(hir::Bind { value: e, .. }) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::Stmt::Ret(Some(e), _, _)
            | hir::Stmt::Break(Some(e), _)
            | hir::Stmt::ErrReturn(e, _, _) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::Stmt::Assign(t, v, _) => {
                Self::collect_hir_var_ids_expr(t, out);
                Self::collect_hir_var_ids_expr(v, out);
            }
            hir::Stmt::If(i) => {
                Self::collect_hir_var_ids_expr(&i.cond, out);
                for s in &i.then {
                    Self::collect_hir_var_ids_stmt(s, out);
                }
                for (c, b) in &i.elifs {
                    Self::collect_hir_var_ids_expr(c, out);
                    for s in b {
                        Self::collect_hir_var_ids_stmt(s, out);
                    }
                }
                if let Some(b) = &i.els {
                    for s in b {
                        Self::collect_hir_var_ids_stmt(s, out);
                    }
                }
            }
            _ => {}
        }
    }

    pub(in crate::typer) fn needs_drop(&self, ty: &Type) -> bool {
        let mut visiting: std::collections::HashSet<crate::intern::Symbol> =
            std::collections::HashSet::new();
        self.needs_drop_inner(ty, &mut visiting)
    }

    /// Recursive implementation of `needs_drop` with cycle protection.
    /// A `visiting` set carries the names of struct/enum types currently on
    /// the inspection stack — a self- or mutually-referential type is treated
    /// as not needing a drop *at the cycle point* (the heap edge is usually
    /// an `Rc`/`Weak`/box which is itself in the base-case heap-owning set).
    fn needs_drop_inner(
        &self,
        ty: &Type,
        visiting: &mut std::collections::HashSet<crate::intern::Symbol>,
    ) -> bool {
        // Base-case heap-owning types: their owned bindings always require
        // scope-exit drop.
        if matches!(
            ty,
            Type::String
                | Type::Vec(_)
                | Type::Map(_, _)
                | Type::Coroutine(_)
                | Type::Generator(_)
                | Type::Channel(_)
        ) {
            return true;
        }
        match ty {
            // A user-defined struct needs drop when:
            //   * it is `@resource` (auto `*drop` fires at scope exit), OR
            //   * any of its fields recursively needs drop (so the heap
            //     storage carried by those fields is reclaimed).
            //
            // This is the canonical Stage-C recursion: combined with
            // Stage-A's borrow-by-default for unannotated heap params and
            // Stage-B's `take`-aware call-arg consume tracking, the caller
            // becomes the sole drop site for the aggregate's heap edges,
            // so recursing here no longer double-frees.
            //
            // Cycle protection: a self- or mutually-referential struct is
            // treated as not-needing-drop *at the cycle point*. Real heap
            // edges in such graphs go through `Rc`/`Weak`/`Box`, which are
            // base-case heap types handled above.
            Type::Struct(name, args) => {
                if self
                    .struct_attrs
                    .get(name)
                    .map(|a| a.resource)
                    .unwrap_or(false)
                {
                    return true;
                }
                if !visiting.insert(name.clone()) {
                    return false;
                }
                // Look up the struct's field types. Substitute generic
                // type args if any. `structs` stores the canonical field
                // shape; if a generic instantiation, also try a mangled
                // mono name.
                let result = self
                    .struct_field_types(name, args)
                    .into_iter()
                    .any(|fty| self.needs_drop_inner(&fty, visiting));
                visiting.remove(name);
                result
            }
            Type::Enum(name) => {
                if !visiting.insert(name.clone()) {
                    return false;
                }
                let result = if let Some(variants) = self.enums.get(name) {
                    variants.iter().any(|(_vname, ftys)| {
                        ftys.iter().any(|t| self.needs_drop_inner(t, visiting))
                    })
                } else {
                    false
                };
                visiting.remove(name);
                result
            }
            Type::Tuple(elts) => elts.iter().any(|t| self.needs_drop_inner(t, visiting)),
            Type::Array(elem, _) => self.needs_drop_inner(elem, visiting),
            // Alias/Newtype are transparent wrappers — preserve the
            // underlying type's drop classification.
            Type::Alias(_, inner) | Type::Newtype(_, inner) => {
                self.needs_drop_inner(inner, visiting)
            }
            _ => false,
        }
    }

    /// Resolve a struct name (+ optional type args) to its field types,
    /// substituting the type args into any generic field. Returns an
    /// empty Vec for unknown structs (safe under-approximation: caller
    /// treats no fields as no drop edges).
    fn struct_field_types(&self, name: &crate::intern::Symbol, args: &[Type]) -> Vec<Type> {
        if let Some(fields) = self.structs.get(name) {
            // If the stored shape has no generic params, return as-is.
            if args.is_empty() {
                return fields.iter().map(|(_, ty)| ty.clone()).collect();
            }
            // Otherwise, substitute generic type params using the stored
            // generic def's param order.
            if let Some(generic_def) = self.generic_types.get(name) {
                let params = &generic_def.type_params;
                if params.len() == args.len() {
                    let subs: std::collections::HashMap<crate::intern::Symbol, Type> = params
                        .iter()
                        .zip(args.iter())
                        .map(|(p, t)| (p.clone(), t.clone()))
                        .collect();
                    return fields
                        .iter()
                        .map(|(_, ty)| Self::subst_type(ty, &subs))
                        .collect();
                }
            }
            return fields.iter().map(|(_, ty)| ty.clone()).collect();
        }
        Vec::new()
    }

    /// Substitute generic type parameters in `ty` using `subs`.
    fn subst_type(
        ty: &Type,
        subs: &std::collections::HashMap<crate::intern::Symbol, Type>,
    ) -> Type {
        match ty {
            Type::Param(name) => subs.get(name).cloned().unwrap_or_else(|| ty.clone()),
            Type::Vec(inner) => Type::Vec(Box::new(Self::subst_type(inner, subs))),
            Type::Coroutine(inner) => Type::Coroutine(Box::new(Self::subst_type(inner, subs))),
            Type::Generator(inner) => Type::Generator(Box::new(Self::subst_type(inner, subs))),
            Type::Channel(inner) => Type::Channel(Box::new(Self::subst_type(inner, subs))),
            Type::Map(k, v) => Type::Map(
                Box::new(Self::subst_type(k, subs)),
                Box::new(Self::subst_type(v, subs)),
            ),
            Type::Tuple(elts) => {
                Type::Tuple(elts.iter().map(|t| Self::subst_type(t, subs)).collect())
            }
            Type::Array(elem, n) => Type::Array(Box::new(Self::subst_type(elem, subs)), *n),
            Type::Struct(name, ts) => Type::Struct(
                name.clone(),
                ts.iter().map(|t| Self::subst_type(t, subs)).collect(),
            ),
            Type::Alias(name, inner) => {
                Type::Alias(name.clone(), Box::new(Self::subst_type(inner, subs)))
            }
            Type::Newtype(name, inner) => {
                Type::Newtype(name.clone(), Box::new(Self::subst_type(inner, subs)))
            }
            _ => ty.clone(),
        }
    }
}
