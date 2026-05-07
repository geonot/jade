//! Variable-id collectors used by capture analysis.

use super::OwnershipVerifier;
use crate::hir::*;

impl OwnershipVerifier {
    pub(super) fn collect_var_ids_block(block: &Block, out: &mut std::collections::HashSet<DefId>) {
        for stmt in block {
            Self::collect_var_ids_stmt(stmt, out);
        }
    }

    pub(super) fn collect_var_ids_stmt(stmt: &Stmt, out: &mut std::collections::HashSet<DefId>) {
        match stmt {
            Stmt::Expr(e) | Stmt::Bind(Bind { value: e, .. }) => Self::collect_var_ids_expr(e, out),
            Stmt::TupleBind(_, v, _) => Self::collect_var_ids_expr(v, out),
            Stmt::Assign(t, v, _) => {
                Self::collect_var_ids_expr(t, out);
                Self::collect_var_ids_expr(v, out);
            }
            Stmt::If(i) => {
                Self::collect_var_ids_expr(&i.cond, out);
                Self::collect_var_ids_block(&i.then, out);
                for (c, b) in &i.elifs {
                    Self::collect_var_ids_expr(c, out);
                    Self::collect_var_ids_block(b, out);
                }
                if let Some(b) = &i.els {
                    Self::collect_var_ids_block(b, out);
                }
            }
            Stmt::While(w) => {
                Self::collect_var_ids_expr(&w.cond, out);
                Self::collect_var_ids_block(&w.body, out);
            }
            Stmt::For(f) => {
                Self::collect_var_ids_expr(&f.iter, out);
                Self::collect_var_ids_block(&f.body, out);
            }
            Stmt::Loop(l) => Self::collect_var_ids_block(&l.body, out),
            Stmt::Match(m) => {
                Self::collect_var_ids_expr(&m.subject, out);
                for arm in &m.arms {
                    if let Some(ref g) = arm.guard {
                        Self::collect_var_ids_expr(g, out);
                    }
                    Self::collect_var_ids_block(&arm.body, out);
                }
            }
            Stmt::Ret(Some(e), _, _) | Stmt::Break(Some(e), _) | Stmt::ErrReturn(e, _, _) => {
                Self::collect_var_ids_expr(e, out);
            }
            _ => {}
        }
    }

    pub(super) fn collect_var_ids_expr(e: &Expr, out: &mut std::collections::HashSet<DefId>) {
        match &e.kind {
            ExprKind::Var(def_id, _) => {
                out.insert(*def_id);
            }
            ExprKind::BinOp(l, _, r) | ExprKind::Index(l, r) => {
                Self::collect_var_ids_expr(l, out);
                Self::collect_var_ids_expr(r, out);
            }
            ExprKind::UnaryOp(_, inner)
            | ExprKind::Coerce(inner, _)
            | ExprKind::Cast(inner, _)
            | ExprKind::Ref(inner)
            | ExprKind::Deref(inner) => {
                Self::collect_var_ids_expr(inner, out);
            }
            ExprKind::Call(_, _, args) | ExprKind::Builtin(_, args) | ExprKind::Syscall(args) => {
                for a in args {
                    Self::collect_var_ids_expr(a, out);
                }
            }
            ExprKind::IndirectCall(callee, args) => {
                Self::collect_var_ids_expr(callee, out);
                for a in args {
                    Self::collect_var_ids_expr(a, out);
                }
            }
            ExprKind::Method(obj, _, _, args)
            | ExprKind::StringMethod(obj, _, args)
            | ExprKind::DeferredMethod(obj, _, args)
            | ExprKind::VecMethod(obj, _, args)
            | ExprKind::MapMethod(obj, _, args) => {
                Self::collect_var_ids_expr(obj, out);
                for a in args {
                    Self::collect_var_ids_expr(a, out);
                }
            }
            ExprKind::Field(e, _, _) => Self::collect_var_ids_expr(e, out),
            ExprKind::Ternary(c, t, f) => {
                Self::collect_var_ids_expr(c, out);
                Self::collect_var_ids_expr(t, out);
                Self::collect_var_ids_expr(f, out);
            }
            ExprKind::Array(es) | ExprKind::Tuple(es) | ExprKind::VecNew(es) => {
                for e in es {
                    Self::collect_var_ids_expr(e, out);
                }
            }
            ExprKind::Struct(_, inits) | ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    Self::collect_var_ids_expr(&fi.value, out);
                }
            }
            ExprKind::Lambda(_, body)
            | ExprKind::CoroutineCreate(_, body)
            | ExprKind::GeneratorCreate(_, _, body) => {
                Self::collect_var_ids_block(body, out);
            }
            ExprKind::Block(b) => Self::collect_var_ids_block(b, out),
            ExprKind::IfExpr(i) => {
                Self::collect_var_ids_expr(&i.cond, out);
                Self::collect_var_ids_block(&i.then, out);
                for (c, b) in &i.elifs {
                    Self::collect_var_ids_expr(c, out);
                    Self::collect_var_ids_block(b, out);
                }
                if let Some(b) = &i.els {
                    Self::collect_var_ids_block(b, out);
                }
            }
            ExprKind::Pipe(left, _, _, extra) => {
                Self::collect_var_ids_expr(left, out);
                for a in extra {
                    Self::collect_var_ids_expr(a, out);
                }
            }
            ExprKind::ChannelSend(ch, val) => {
                Self::collect_var_ids_expr(ch, out);
                Self::collect_var_ids_expr(val, out);
            }
            ExprKind::ChannelRecv(ch) => Self::collect_var_ids_expr(ch, out),
            ExprKind::Send(t, _, _, _, args) => {
                Self::collect_var_ids_expr(t, out);
                for a in args {
                    Self::collect_var_ids_expr(a, out);
                }
            }
            ExprKind::Slice(o, s, e) => {
                Self::collect_var_ids_expr(o, out);
                Self::collect_var_ids_expr(s, out);
                Self::collect_var_ids_expr(e, out);
            }
            ExprKind::ListComp(body, _, _, iter, cond, map) => {
                Self::collect_var_ids_expr(iter, out);
                Self::collect_var_ids_expr(body, out);
                if let Some(c) = cond {
                    Self::collect_var_ids_expr(c, out);
                }
                if let Some(m) = map {
                    Self::collect_var_ids_expr(m, out);
                }
            }
            _ => {}
        }
    }
}
