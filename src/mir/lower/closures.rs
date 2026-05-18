use super::super::*;
use super::Lowerer;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::HashSet;

impl Lowerer {
    pub(super) fn lower_expr_closure(&mut self, expr: &hir::Expr) -> ValueId {
        let ExprKind::Lambda(params, body) = &expr.kind else {
            unreachable!("lower_expr_closure called for non-lambda expression")
        };

        let span = expr.span;
        let ty = expr.ty.clone();
        let lambda_name = format!("lambda.{}", self.func.next_value);

        let param_names: HashSet<Symbol> = params.iter().map(|p| p.name).collect();
        let mut refs = HashSet::new();
        collect_var_refs_block(body, &mut refs);

        let mut capture_info: Vec<(Symbol, ValueId, Type)> = Vec::new();
        for name in &refs {
            if !param_names.contains(name) {
                if let Some(&val) = self.var_map.get(name) {
                    let cap_ty = self.value_type(val);
                    capture_info.push((*name, val, cap_ty));
                }
            }
        }
        let capture_vals: Vec<ValueId> = capture_info.iter().map(|(_, v, _)| *v).collect();

        let ret_ty = if let Type::Fn(_, ret) = &ty {
            *ret.clone()
        } else {
            Type::I64
        };

        let mut lambda_lowerer = Lowerer::new(&lambda_name, crate::hir::DefId(0), span);
        lambda_lowerer.func.ret_ty = ret_ty;

        for (cap_name, _, cap_ty) in &capture_info {
            let val = lambda_lowerer.new_value();
            lambda_lowerer.func.params.push(Param {
                value: val,
                name: *cap_name,
                ty: cap_ty.clone(),
            });
            lambda_lowerer.var_map.insert(*cap_name, val);
        }

        for p in params {
            let val = lambda_lowerer.new_value();
            lambda_lowerer.func.params.push(Param {
                value: val,
                name: p.name,
                ty: p.ty.clone(),
            });
            lambda_lowerer.var_map.insert(p.name, val);
        }

        // Same tail-expression auto-clone treatment as top-level fn body
        // lowering: identify the implicit-return position and use
        // `lower_expr_owned` so a heap-typed field/index read at the tail
        // produces an independently-owned value rather than aliasing
        // storage about to be scope-exit dropped.
        let tail_idx: Option<usize> = body
            .iter()
            .enumerate()
            .rev()
            .find(|(_, s)| {
                !matches!(
                    s,
                    hir::Stmt::Drop(..)
                        | hir::Stmt::Ret(..)
                        | hir::Stmt::Break(..)
                        | hir::Stmt::Continue(..)
                        | hir::Stmt::ErrReturn(..)
                )
            })
            .map(|(i, _)| i);
        let mut last = lambda_lowerer.emit(InstKind::Void, Type::Void, span);
        for (idx, stmt) in body.iter().enumerate() {
            let v = if Some(idx) == tail_idx {
                if let hir::Stmt::Expr(e) = stmt {
                    lambda_lowerer.lower_expr_owned(e)
                } else {
                    lambda_lowerer.lower_stmt(stmt)
                }
            } else {
                lambda_lowerer.lower_stmt(stmt)
            };
            if !matches!(stmt, hir::Stmt::Drop(..)) {
                last = v;
            }
        }
        if !lambda_lowerer.current_block_has_terminator() {
            lambda_lowerer.set_terminator(Terminator::Return(Some(last)));
        }

        self.lambda_fns.push(lambda_lowerer.func);
        self.lambda_fns.append(&mut lambda_lowerer.lambda_fns);

        self.emit(
            InstKind::ClosureCreate(Symbol::intern(&lambda_name), capture_vals),
            ty,
            span,
        )
    }
}

fn collect_var_refs_block(body: &[hir::Stmt], refs: &mut HashSet<Symbol>) {
    for stmt in body {
        collect_var_refs_stmt(stmt, refs);
    }
}

fn collect_var_refs_stmt(stmt: &hir::Stmt, refs: &mut HashSet<Symbol>) {
    match stmt {
        hir::Stmt::Bind(b) => collect_var_refs_expr(&b.value, refs),
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
        hir::Stmt::For(for_stmt) => {
            collect_var_refs_expr(&for_stmt.iter, refs);
            collect_var_refs_block(&for_stmt.body, refs);
        }
        hir::Stmt::Loop(loop_stmt) => collect_var_refs_block(&loop_stmt.body, refs),
        hir::Stmt::Ret(Some(expr), _, _) => collect_var_refs_expr(expr, refs),
        hir::Stmt::Match(match_stmt) => {
            collect_var_refs_expr(&match_stmt.subject, refs);
            for arm in &match_stmt.arms {
                collect_var_refs_block(&arm.body, refs);
            }
        }
        hir::Stmt::Break(Some(expr), _)
        | hir::Stmt::ErrReturn(expr, _, _)
        | hir::Stmt::ChannelClose(expr, _)
        | hir::Stmt::Stop(expr, _) => collect_var_refs_expr(expr, refs),
        hir::Stmt::TupleBind(_, expr, _) => collect_var_refs_expr(expr, refs),
        hir::Stmt::SimFor(sim_for, _) => {
            collect_var_refs_expr(&sim_for.iter, refs);
            collect_var_refs_block(&sim_for.body, refs);
        }
        hir::Stmt::SimBlock(body, _) | hir::Stmt::Transaction(body, _) => {
            collect_var_refs_block(body, refs);
        }
        hir::Stmt::StoreInsert(_, exprs, _) => {
            for expr in exprs {
                collect_var_refs_expr(expr, refs);
            }
        }
        hir::Stmt::StoreSet(_, updates, _, _) => {
            for (_, expr) in updates {
                collect_var_refs_expr(expr, refs);
            }
        }
        _ => {}
    }
}

fn collect_var_refs_expr(expr: &hir::Expr, refs: &mut HashSet<Symbol>) {
    match &expr.kind {
        ExprKind::Var(_, name) => {
            refs.insert(*name);
        }
        ExprKind::BinOp(left, _, right) => {
            collect_var_refs_expr(left, refs);
            collect_var_refs_expr(right, refs);
        }
        ExprKind::UnaryOp(_, inner)
        | ExprKind::Ref(inner)
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
        ExprKind::CoroutineCreate(_, stmts) => collect_var_refs_block(stmts, refs),
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
        ExprKind::Builder(_, fields) => {
            for (_, expr) in fields {
                collect_var_refs_expr(expr, refs);
            }
        }
        ExprKind::Block(stmts) | ExprKind::Lambda(_, stmts) => collect_var_refs_block(stmts, refs),
        _ => {}
    }
}
