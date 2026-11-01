use super::super::*;
use super::Lowerer;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::{HashMap, HashSet};

impl Lowerer {
    pub(super) fn lower_expr_closure(&mut self, expr: &hir::Expr) -> ValueId {
        let ExprKind::Lambda(params, body, moved) = &expr.kind else {
            unreachable!("lower_expr_closure called for non-lambda expression")
        };

        let span = expr.span;
        let ty = expr.ty.clone();
        let lambda_name = format!("lambda.{}.{}", self.func.name, self.func.next_value);

        let param_ids: HashSet<_> = params.iter().map(|p| p.def_id).collect();
        let mut refs = VarRefs::default();
        collect_var_refs_block(body, &mut refs);
        let mut ref_names: Vec<_> = refs.vars.into_iter().collect();
        ref_names.sort_by_key(|(id, _)| id.0);

        let mut capture_info = Vec::new();
        for (id, name) in &ref_names {
            let key = self.var_key(*id, *name);
            if !param_ids.contains(id)
                && let Some(cap_ty) = self.var_types.get(&key).cloned()
            {
                let val = self.read_var(key, self.current_block, cap_ty.clone(), span);
                let val = if !moved.contains(id) && !cap_ty.is_trivially_droppable() {
                    self.emit(InstKind::Clone(val, cap_ty.clone()), cap_ty.clone(), span)
                } else {
                    val
                };
                capture_info.push((*id, *name, val, cap_ty));
            }
        }
        let capture_vals: Vec<ValueId> = capture_info.iter().map(|(_, _, v, _)| *v).collect();

        let ret_ty = if let Type::Fn(_, ret) = &ty {
            *ret.clone()
        } else {
            Type::I64
        };

        let mut lambda_lowerer = Lowerer::new(&lambda_name, crate::hir::DefId(0), span);
        lambda_lowerer.func.ret_ty = ret_ty.clone();

        for (id, cap_name, _, cap_ty) in &capture_info {
            let val = lambda_lowerer.new_value();
            lambda_lowerer.func.params.push(Param {
                value: val,
                name: *cap_name,
                ty: cap_ty.clone(),
                by_ref: false,
                owned: false,
            });
            lambda_lowerer.var_types.insert(*cap_name, cap_ty.clone());
            lambda_lowerer.cleanup_param_ids.insert(*cap_name, *id);
            let entry = lambda_lowerer.func.entry;
            lambda_lowerer
                .current_def
                .entry(entry)
                .or_default()
                .insert(*cap_name, val);
        }

        for p in params {
            let val = lambda_lowerer.new_value();
            lambda_lowerer.func.params.push(Param {
                value: val,
                name: p.name,
                ty: p.ty.clone(),
                by_ref: false,
                owned: matches!(p.ownership, hir::Ownership::Owned)
                    && !matches!(p.access_mod, Some(crate::ast::AccessMod::Copy)),
            });
            lambda_lowerer.var_types.insert(p.name, p.ty.clone());
            lambda_lowerer.cleanup_param_ids.insert(p.name, p.def_id);
            if matches!(p.ownership, hir::Ownership::Owned) {
                lambda_lowerer.cleanup_owned_params.insert(p.def_id);
            }
            let entry = lambda_lowerer.func.entry;
            lambda_lowerer
                .current_def
                .entry(entry)
                .or_default()
                .insert(p.name, val);
        }

        for p in params {
            if matches!(p.access_mod, Some(crate::ast::AccessMod::Copy))
                && !p.ty.is_trivially_droppable()
                && p.ty.is_value_clonable()
            {
                let value = lambda_lowerer.read_var(
                    p.name,
                    lambda_lowerer.func.entry,
                    p.ty.clone(),
                    p.span,
                );
                let value =
                    lambda_lowerer.emit(InstKind::Clone(value, p.ty.clone()), p.ty.clone(), p.span);
                lambda_lowerer.write_var(p.name, lambda_lowerer.func.entry, value);
            }
        }

        super::finish_body(&mut lambda_lowerer, body, &ret_ty, span, false);

        self.lambda_fns.push(lambda_lowerer.func);
        self.lambda_fns.append(&mut lambda_lowerer.lambda_fns);

        self.emit(
            InstKind::ClosureCreate(Symbol::intern(&lambda_name), capture_vals),
            ty,
            span,
        )
    }
}

#[derive(Default)]
pub(super) struct VarRefs {
    pub(super) vars: HashSet<(hir::DefId, Symbol)>,

    pub(super) addr_taken: HashSet<Symbol>,
    pub(super) deferred_vars: HashSet<hir::DefId>,
    pub(super) borrow_sources: HashMap<hir::DefId, HashSet<hir::DefId>>,
    pub(super) has_defer: bool,
}

impl VarRefs {
    fn insert(&mut self, r: (hir::DefId, Symbol)) {
        self.vars.insert(r);
    }

    fn absorb(&mut self, other: VarRefs) {
        self.vars.extend(other.vars);
        self.addr_taken.extend(other.addr_taken);
        self.deferred_vars.extend(other.deferred_vars);
        for (id, sources) in other.borrow_sources {
            self.borrow_sources.entry(id).or_default().extend(sources);
        }
        self.has_defer |= other.has_defer;
    }
}

pub(super) fn collect_addr_taken_block(body: &[hir::Stmt], out: &mut HashSet<Symbol>) {
    let mut refs = VarRefs::default();
    collect_var_refs_block(body, &mut refs);
    out.extend(refs.addr_taken);
}

pub(super) fn collect_var_refs_block(body: &[hir::Stmt], refs: &mut VarRefs) {
    for stmt in body {
        collect_var_refs_stmt(stmt, refs);
    }
}

fn collect_var_refs_stmt(stmt: &hir::Stmt, refs: &mut VarRefs) {
    match stmt {
        hir::Stmt::Bind(b) => {
            let mut inner = VarRefs::default();
            collect_var_refs_expr(&b.value, &mut inner);
            if b.ownership.is_borrow() || matches!(b.ty, Type::View(_)) {
                refs.borrow_sources
                    .entry(b.def_id)
                    .or_default()
                    .extend(inner.vars.iter().map(|(id, _)| *id));
            }
            refs.absorb(inner);
        }
        hir::Stmt::Assign(target, value, _) => {
            collect_var_refs_expr(target, refs);
            collect_var_refs_expr(value, refs);
        }
        hir::Stmt::Expr(expr) => collect_var_refs_expr(expr, refs),
        hir::Stmt::If(if_stmt) => {
            collect_var_refs_expr(&if_stmt.cond, refs);
            collect_var_refs_block(&if_stmt.then, refs);
            for (cond, body) in &if_stmt.elifs {
                collect_var_refs_expr(cond, refs);
                collect_var_refs_block(body, refs);
            }
            if let Some(else_body) = &if_stmt.els {
                collect_var_refs_block(else_body, refs);
            }
        }
        hir::Stmt::While(while_stmt) => {
            collect_var_refs_expr(&while_stmt.cond, refs);
            collect_var_refs_block(&while_stmt.body, refs);
        }
        hir::Stmt::For(for_stmt) | hir::Stmt::SimFor(for_stmt, _) => {
            let mut inner = VarRefs::default();
            collect_var_refs_expr(&for_stmt.iter, &mut inner);
            for id in [Some(for_stmt.bind_id), for_stmt.bind2_id]
                .into_iter()
                .flatten()
            {
                refs.borrow_sources
                    .entry(id)
                    .or_default()
                    .extend(inner.vars.iter().map(|(source, _)| *source));
            }
            refs.absorb(inner);
            if let Some(end) = &for_stmt.end {
                collect_var_refs_expr(end, refs);
            }
            if let Some(step) = &for_stmt.step {
                collect_var_refs_expr(step, refs);
            }
            collect_var_refs_block(&for_stmt.body, refs);
        }
        hir::Stmt::Loop(loop_stmt) => collect_var_refs_block(&loop_stmt.body, refs),
        hir::Stmt::Ret(Some(expr), _, _) => collect_var_refs_expr(expr, refs),
        hir::Stmt::Match(match_stmt) => {
            let mut inner = VarRefs::default();
            collect_var_refs_expr(&match_stmt.subject, &mut inner);
            for arm in &match_stmt.arms {
                collect_pattern_sources(&arm.pat, &inner, refs);
                if let Some(guard) = &arm.guard {
                    collect_var_refs_expr(guard, refs);
                }
                collect_var_refs_block(&arm.body, refs);
            }
            refs.absorb(inner);
        }
        hir::Stmt::Defer(body, _) => {
            let mut inner = VarRefs::default();
            collect_var_refs_block(body, &mut inner);
            refs.deferred_vars
                .extend(inner.vars.iter().map(|(id, _)| *id));
            refs.has_defer = true;
            refs.absorb(inner);
        }
        hir::Stmt::GlobalStore(_, expr, _) => collect_var_refs_expr(expr, refs),
        hir::Stmt::Break(Some(expr), _)
        | hir::Stmt::ErrReturn(expr, _, _)
        | hir::Stmt::ChannelClose(expr, _)
        | hir::Stmt::Stop(expr, _)
        | hir::Stmt::Join(expr, _) => collect_var_refs_expr(expr, refs),
        hir::Stmt::ScopeCancel(_, _) => {}
        hir::Stmt::TupleBind(_, expr, _) => collect_var_refs_expr(expr, refs),
        hir::Stmt::SimBlock(body, _) | hir::Stmt::Transaction(body, _) => {
            collect_var_refs_block(body, refs);
        }
        hir::Stmt::Together(_, body, _, handler, _) => {
            collect_var_refs_block(body, refs);
            if let Some(handler) = handler {
                if let Some(body) = &handler.ok_arm {
                    collect_var_refs_block(body, refs);
                }
                if let Some(body) = &handler.err_arm {
                    collect_var_refs_block(body, refs);
                }
            }
        }
        hir::Stmt::StoreInsert(_, exprs, _) => {
            for expr in exprs {
                collect_var_refs_expr(expr, refs);
            }
        }
        hir::Stmt::StoreSet(_, updates, filter, _) => {
            for (_, expr) in updates {
                collect_var_refs_expr(expr, refs);
            }
            collect_filter_refs(filter, refs);
        }
        hir::Stmt::StoreDelete(_, filter, _)
        | hir::Stmt::StoreDestroy(_, filter, _)
        | hir::Stmt::StoreRestore(_, filter, _) => collect_filter_refs(filter, refs),
        hir::Stmt::Asm(asm) => {
            for (_, expr) in &asm.inputs {
                collect_var_refs_expr(expr, refs);
            }
        }
        _ => {}
    }
}

fn collect_pattern_sources(pat: &hir::Pat, sources: &VarRefs, refs: &mut VarRefs) {
    match pat {
        hir::Pat::Bind(id, _, _, _) => {
            refs.borrow_sources
                .entry(*id)
                .or_default()
                .extend(sources.vars.iter().map(|(id, _)| *id));
        }
        hir::Pat::Ctor(_, _, pats, _)
        | hir::Pat::Or(pats, _)
        | hir::Pat::Tuple(pats, _)
        | hir::Pat::Array(pats, _) => {
            for pat in pats {
                collect_pattern_sources(pat, sources, refs);
            }
        }
        hir::Pat::Wild(_) | hir::Pat::Lit(_) | hir::Pat::Range(..) => {}
    }
}

fn collect_filter_refs(filter: &hir::StoreFilter, refs: &mut VarRefs) {
    collect_var_refs_expr(&filter.value, refs);
    for (_, cond) in &filter.extra {
        collect_var_refs_expr(&cond.value, refs);
    }
}

fn collect_var_refs_expr(expr: &hir::Expr, refs: &mut VarRefs) {
    match &expr.kind {
        ExprKind::Var(id, name) => {
            refs.insert((*id, *name));
        }
        ExprKind::BinOp(left, _, right) => {
            collect_var_refs_expr(left, refs);
            collect_var_refs_expr(right, refs);
        }
        ExprKind::Ref(inner) => {
            if let ExprKind::Var(_, name) = &inner.kind
                && Lowerer::addressable_scalar(&inner.ty)
            {
                refs.addr_taken.insert(*name);
            }
            collect_var_refs_expr(inner, refs);
        }
        ExprKind::UnaryOp(_, inner)
        | ExprKind::Deref(inner)
        | ExprKind::Cast(inner, _)
        | ExprKind::StrictCast(inner, _)
        | ExprKind::Coerce(inner, _) => collect_var_refs_expr(inner, refs),
        ExprKind::Call(_, _, args)
        | ExprKind::Array(args)
        | ExprKind::Tuple(args)
        | ExprKind::VecNew(args)
        | ExprKind::Syscall(args) => {
            for arg in args {
                collect_var_refs_expr(arg, refs);
            }
        }
        ExprKind::IndirectCall(func, args) => {
            collect_var_refs_expr(func, refs);
            for arg in args {
                collect_var_refs_expr(arg, refs);
            }
        }
        ExprKind::Method(obj, _, _, args)
        | ExprKind::StringMethod(obj, _, args)
        | ExprKind::VecMethod(obj, _, args)
        | ExprKind::MapMethod(obj, _, args)
        | ExprKind::DeferredMethod(obj, _, args) => {
            collect_var_refs_expr(obj, refs);
            for arg in args {
                collect_var_refs_expr(arg, refs);
            }
        }
        ExprKind::Field(obj, _, _) => collect_var_refs_expr(obj, refs),
        ExprKind::Index(array, index) => {
            collect_var_refs_expr(array, refs);
            collect_var_refs_expr(index, refs);
        }
        ExprKind::IfExpr(if_expr) => {
            collect_var_refs_expr(&if_expr.cond, refs);
            collect_var_refs_block(&if_expr.then, refs);
            for (cond, body) in &if_expr.elifs {
                collect_var_refs_expr(cond, refs);
                collect_var_refs_block(body, refs);
            }
            if let Some(else_body) = &if_expr.els {
                collect_var_refs_block(else_body, refs);
            }
        }
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            collect_var_refs_expr(cond, refs);
            collect_var_refs_expr(then_expr, refs);
            collect_var_refs_expr(else_expr, refs);
        }
        ExprKind::Struct(_, fields) | ExprKind::VariantCtor(_, _, _, fields) => {
            for field in fields {
                collect_var_refs_expr(&field.value, refs);
            }
        }
        ExprKind::Select(arms, default) => {
            for arm in arms {
                collect_var_refs_expr(&arm.chan, refs);
                if let Some(value) = &arm.value {
                    collect_var_refs_expr(value, refs);
                }
                collect_var_refs_block(&arm.body, refs);
            }
            if let Some(default_body) = default {
                collect_var_refs_block(default_body, refs);
            }
        }
        ExprKind::Send(obj, _, _, _, args) | ExprKind::Pipe(obj, _, _, args) => {
            collect_var_refs_expr(obj, refs);
            for arg in args {
                collect_var_refs_expr(arg, refs);
            }
        }
        ExprKind::Builtin(_, args) => {
            for arg in args {
                collect_var_refs_expr(arg, refs);
            }
        }
        ExprKind::ChannelSend(chan, value)
        | ExprKind::AtomicStore(chan, value)
        | ExprKind::AtomicAdd(chan, value)
        | ExprKind::AtomicSub(chan, value) => {
            collect_var_refs_expr(chan, refs);
            collect_var_refs_expr(value, refs);
        }
        ExprKind::ChannelRecv(inner)
        | ExprKind::GeneratorNext(inner)
        | ExprKind::EnumUnwrap(inner, _, _)
        | ExprKind::EnumIs(inner, _)
        | ExprKind::CoroutineNext(inner)
        | ExprKind::Yield(inner)
        | ExprKind::AsFormat(inner, _)
        | ExprKind::AtomicLoad(inner)
        | ExprKind::Grad(inner) => collect_var_refs_expr(inner, refs),
        ExprKind::Slice(inner, lo, hi) => {
            collect_var_refs_expr(inner, refs);
            collect_var_refs_expr(lo, refs);
            collect_var_refs_expr(hi, refs);
        }
        ExprKind::ChannelCreate(_, cap) => collect_var_refs_expr(cap, refs),
        ExprKind::ListComp(body, _, _, iter, end, cond) => {
            collect_var_refs_expr(body, refs);
            collect_var_refs_expr(iter, refs);
            if let Some(end) = end {
                collect_var_refs_expr(end, refs);
            }
            if let Some(cond) = cond {
                collect_var_refs_expr(cond, refs);
            }
        }
        ExprKind::CoroutineCreate(_, stmts, _) | ExprKind::GeneratorCreate(_, _, stmts, _) => {
            let mut inner = VarRefs::default();
            collect_var_refs_block(stmts, &mut inner);
            inner.addr_taken.clear();
            inner.deferred_vars.clear();
            inner.borrow_sources.clear();
            inner.has_defer = false;
            refs.absorb(inner);
        }
        ExprKind::AtomicCas(ptr, expected, replacement) => {
            collect_var_refs_expr(ptr, refs);
            collect_var_refs_expr(expected, refs);
            collect_var_refs_expr(replacement, refs);
        }
        ExprKind::Einsum(_, args) => {
            for arg in args {
                collect_var_refs_expr(arg, refs);
            }
        }
        ExprKind::Spawn(_, fields) | ExprKind::Builder(_, fields) => {
            for (_, expr) in fields {
                collect_var_refs_expr(expr, refs);
            }
        }
        ExprKind::StoreQuery(_, filter)
        | ExprKind::StoreAllWhere(_, filter)
        | ExprKind::StoreFirst(_, filter)
        | ExprKind::StoreExists(_, filter)
        | ExprKind::ViewCount(_, filter)
        | ExprKind::ViewAll(_, filter) => collect_filter_refs(filter, refs),
        ExprKind::StoreQueryGroup(_, _, _, Some(filter)) => collect_filter_refs(filter, refs),
        ExprKind::StoreGet(_, expr)
        | ExprKind::StoreVersionCount(_, expr)
        | ExprKind::StoreHistory(_, expr)
        | ExprKind::KvGet(_, expr)
        | ExprKind::KvHas(_, expr)
        | ExprKind::KvDel(_, expr)
        | ExprKind::VecInsert(_, expr)
        | ExprKind::BloomTest(_, _, expr)
        | ExprKind::FtsSearch(_, _, expr)
        | ExprKind::GraphFrom(_, expr)
        | ExprKind::GraphTo(_, expr) => collect_var_refs_expr(expr, refs),
        ExprKind::StoreAtVersion(_, first, second)
        | ExprKind::KvSet(_, first, second)
        | ExprKind::KvIncr(_, first, second)
        | ExprKind::VecNearest(_, first, second) => {
            collect_var_refs_expr(first, refs);
            collect_var_refs_expr(second, refs);
        }
        ExprKind::Block(stmts) => collect_var_refs_block(stmts, refs),
        ExprKind::Lambda(inner_params, stmts, _) => {
            let mut inner = VarRefs::default();
            collect_var_refs_block(stmts, &mut inner);
            inner
                .vars
                .retain(|(id, _)| !inner_params.iter().any(|p| p.def_id == *id));

            inner.borrow_sources.clear();
            inner.addr_taken.clear();
            inner.deferred_vars.clear();
            inner.has_defer = false;
            refs.absorb(inner);
        }
        _ => {}
    }
}
