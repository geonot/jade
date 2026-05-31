use super::ConstVal;
use super::fold::{fold_float_op, fold_int_op};
use crate::ast::{BinOp, Span, UnaryOp};
use crate::hir::{self, Block, Expr, ExprKind, Stmt};
use crate::intern::Symbol;
use std::collections::HashMap;

pub(super) fn try_eval_pure_call(
    name: &str,
    args: &[Expr],
    pure_fns: &HashMap<Symbol, hir::Fn>,
    depth: u32,
) -> Option<Expr> {
    if depth > 100 {
        return None;
    }
    let func = pure_fns.get(&Symbol::intern(name))?;

    let const_args: Vec<_> = args
        .iter()
        .map(|a| match &a.kind {
            ExprKind::Int(v) => Some(ConstVal::Int(*v)),
            ExprKind::Float(v) => Some(ConstVal::Float(*v)),
            ExprKind::Bool(v) => Some(ConstVal::Bool(*v)),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;

    let mut env: HashMap<hir::DefId, ConstVal> = HashMap::new();
    for (param, val) in func.params.iter().zip(const_args.iter()) {
        env.insert(param.def_id, val.clone());
    }

    eval_block(&func.body, &mut env, pure_fns, depth + 1).map(|val| {
        val.to_expr(
            func.ret.clone(),
            args.first().map_or(
                Span {
                    start: 0,
                    end: 0,
                    line: 0,
                    col: 0,
                    file: None,
                },
                |a| a.span,
            ),
        )
    })
}

pub(super) fn eval_block(
    block: &Block,
    env: &mut HashMap<hir::DefId, ConstVal>,
    pure_fns: &HashMap<Symbol, hir::Fn>,
    depth: u32,
) -> Option<ConstVal> {
    for stmt in block {
        match stmt {
            Stmt::Bind(b) => {
                let val = eval_expr(&b.value, env, pure_fns, depth)?;
                env.insert(b.def_id, val);
            }
            Stmt::Ret(Some(e), _, _) => {
                return Some(eval_expr(e, env, pure_fns, depth)?);
            }
            Stmt::Ret(None, _, _) => return Some(ConstVal::Void),
            Stmt::If(i) => {
                let cond = eval_expr(&i.cond, env, pure_fns, depth)?;
                if let ConstVal::Bool(true) = cond {
                    if let Some(v) = eval_block(&i.then, env, pure_fns, depth) {
                        return Some(v);
                    }
                } else {
                    for (ec, eb) in &i.elifs {
                        let ec_val = eval_expr(ec, env, pure_fns, depth)?;
                        if let ConstVal::Bool(true) = ec_val {
                            if let Some(v) = eval_block(eb, env, pure_fns, depth) {
                                return Some(v);
                            }
                        }
                    }
                    if let Some(els) = &i.els {
                        if let Some(v) = eval_block(els, env, pure_fns, depth) {
                            return Some(v);
                        }
                    }
                }
            }
            Stmt::Expr(e) => {
                eval_expr(e, env, pure_fns, depth)?;
            }
            _ => return None,
        }
    }
    None
}

pub(super) fn eval_expr(
    expr: &Expr,
    env: &mut HashMap<hir::DefId, ConstVal>,
    pure_fns: &HashMap<Symbol, hir::Fn>,
    depth: u32,
) -> Option<ConstVal> {
    match &expr.kind {
        ExprKind::Int(v) => Some(ConstVal::Int(*v)),
        ExprKind::Float(v) => Some(ConstVal::Float(*v)),
        ExprKind::Bool(v) => Some(ConstVal::Bool(*v)),
        ExprKind::Void => Some(ConstVal::Void),
        ExprKind::Var(id, _) => env.get(id).cloned(),
        ExprKind::BinOp(l, op, r) => {
            let lv = eval_expr(l, env, pure_fns, depth)?;
            let rv = eval_expr(r, env, pure_fns, depth)?;
            eval_binop(lv, *op, rv)
        }
        ExprKind::UnaryOp(op, e) => {
            let v = eval_expr(e, env, pure_fns, depth)?;
            match (op, v) {
                (UnaryOp::Neg, ConstVal::Int(n)) => Some(ConstVal::Int(n.wrapping_neg())),
                (UnaryOp::Neg, ConstVal::Float(n)) => Some(ConstVal::Float(-n)),
                (UnaryOp::Not, ConstVal::Bool(b)) => Some(ConstVal::Bool(!b)),
                _ => None,
            }
        }
        ExprKind::Call(_, name, args) => {
            let eval_args: Vec<Expr> = args
                .iter()
                .map(|a| {
                    eval_expr(a, env, pure_fns, depth).map(|v| v.to_expr(a.ty.clone(), a.span))
                })
                .collect::<Option<Vec<_>>>()?;
            let result = try_eval_pure_call(&name.as_str(), &eval_args, pure_fns, depth)?;
            match &result.kind {
                ExprKind::Int(v) => Some(ConstVal::Int(*v)),
                ExprKind::Float(v) => Some(ConstVal::Float(*v)),
                ExprKind::Bool(v) => Some(ConstVal::Bool(*v)),
                _ => None,
            }
        }
        ExprKind::Ternary(c, t, f) => {
            let cv = eval_expr(c, env, pure_fns, depth)?;
            match cv {
                ConstVal::Bool(true) => eval_expr(t, env, pure_fns, depth),
                ConstVal::Bool(false) => eval_expr(f, env, pure_fns, depth),
                _ => None,
            }
        }
        _ => None,
    }
}

pub(super) fn eval_binop(l: ConstVal, op: BinOp, r: ConstVal) -> Option<ConstVal> {
    match (l, r) {
        (ConstVal::Int(a), ConstVal::Int(b)) => {
            let v = fold_int_op(a, op, b)?;
            match v {
                ExprKind::Int(n) => Some(ConstVal::Int(n)),
                ExprKind::Bool(b) => Some(ConstVal::Bool(b)),
                _ => None,
            }
        }
        (ConstVal::Float(a), ConstVal::Float(b)) => {
            let v = fold_float_op(a, op, b)?;
            match v {
                ExprKind::Float(n) => Some(ConstVal::Float(n)),
                ExprKind::Bool(b) => Some(ConstVal::Bool(b)),
                _ => None,
            }
        }
        (ConstVal::Bool(a), ConstVal::Bool(b)) => match op {
            BinOp::And => Some(ConstVal::Bool(a && b)),
            BinOp::Or => Some(ConstVal::Bool(a || b)),
            _ => None,
        },
        _ => None,
    }
}
