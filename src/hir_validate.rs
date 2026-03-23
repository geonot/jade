use std::collections::HashMap;

use crate::ast::Span;
use crate::hir::{self, DefId};
use crate::types::Type;

/// Validates the HIR program between typer output and codegen.
/// Catches bugs that would otherwise surface as cryptic LLVM errors.
pub struct HirValidator {
    fn_defs: HashMap<u32, Span>, // top-level DefId uniqueness (fns, types, enums, externs)
    fn_sigs: HashMap<u32, (String, usize)>, // DefId → (name, param_count)
    errors: Vec<String>,
}

impl HirValidator {
    pub fn validate(prog: &hir::Program) -> Vec<String> {
        let mut v = HirValidator {
            fn_defs: HashMap::new(),
            fn_sigs: HashMap::new(),
            errors: Vec::new(),
        };

        // Pre-collect function signatures for arity checking
        for f in &prog.fns {
            v.fn_sigs.insert(f.def_id.0, (f.name.clone(), f.params.len()));
        }
        for ext in &prog.externs {
            v.fn_sigs.insert(ext.def_id.0, (ext.name.clone(), ext.params.len()));
        }

        // Validate each top-level item
        for f in &prog.fns {
            v.check_top_level_def(f.def_id, &f.name, f.span);
            v.validate_fn(f);
        }
        for td in &prog.types {
            v.check_top_level_def(td.def_id, &td.name, td.span);
            for m in &td.methods {
                v.check_top_level_def(m.def_id, &m.name, m.span);
                v.validate_fn(m);
            }
        }
        for ed in &prog.enums {
            v.check_top_level_def(ed.def_id, &ed.name, ed.span);
        }
        for ext in &prog.externs {
            v.check_top_level_def(ext.def_id, &ext.name, ext.span);
        }
        for actor in &prog.actors {
            v.check_top_level_def(actor.def_id, &actor.name, actor.span);
            for h in &actor.handlers {
                v.validate_block(&h.body);
            }
        }
        for ti in &prog.trait_impls {
            for m in &ti.methods {
                v.check_top_level_def(m.def_id, &m.name, m.span);
                v.validate_fn(m);
            }
        }

        v.errors
    }

    fn check_top_level_def(&mut self, id: DefId, name: &str, span: Span) {
        if id == DefId::BUILTIN {
            return;
        }
        if let Some(prev) = self.fn_defs.insert(id.0, span) {
            self.errors.push(format!(
                "duplicate top-level DefId d{} for `{}` at line {} (previously at line {})",
                id.0, name, span.line, prev.line
            ));
        }
    }

    fn validate_fn(&mut self, f: &hir::Fn) {
        self.validate_block(&f.body);
    }

    fn validate_block(&mut self, block: &hir::Block) {
        let mut saw_terminator = false;
        for (i, stmt) in block.iter().enumerate() {
            if saw_terminator {
                self.errors.push(format!(
                    "unreachable statement at line {} (after return/break/continue)",
                    stmt_span(stmt).line
                ));
                break;
            }
            self.validate_stmt(stmt);
            if matches!(stmt, hir::Stmt::Ret(..) | hir::Stmt::Break(..) | hir::Stmt::Continue(..)) {
                // Drop statements after terminators are OK (emitted by scope cleanup)
                let remaining = &block[i + 1..];
                if remaining.iter().any(|s| !matches!(s, hir::Stmt::Drop(..))) {
                    saw_terminator = true;
                }
            }
        }
    }

    fn validate_stmt(&mut self, stmt: &hir::Stmt) {
        match stmt {
            hir::Stmt::Bind(b) => {
                self.validate_expr(&b.value);
                // Type consistency: binding type should match value type
                if b.ty != b.value.ty && b.value.ty != Type::Void {
                    self.errors.push(format!(
                        "type mismatch in binding `{}` at line {}: declared {:?} but value is {:?}",
                        b.name, b.span.line, b.ty, b.value.ty
                    ));
                }
            }
            hir::Stmt::TupleBind(_, expr, _) => {
                self.validate_expr(expr);
            }
            hir::Stmt::Assign(target, value, _) => {
                self.validate_expr(target);
                self.validate_expr(value);
            }
            hir::Stmt::Expr(e) => self.validate_expr(e),
            hir::Stmt::If(i) => {
                self.validate_expr(&i.cond);
                self.validate_block(&i.then);
                for (c, b) in &i.elifs {
                    self.validate_expr(c);
                    self.validate_block(b);
                }
                if let Some(b) = &i.els {
                    self.validate_block(b);
                }
            }
            hir::Stmt::While(w) => {
                self.validate_expr(&w.cond);
                self.validate_block(&w.body);
            }
            hir::Stmt::For(f) => {
                self.validate_expr(&f.iter);
                if let Some(e) = &f.end {
                    self.validate_expr(e);
                }
                if let Some(s) = &f.step {
                    self.validate_expr(s);
                }
                self.validate_block(&f.body);
            }
            hir::Stmt::Loop(l) => self.validate_block(&l.body),
            hir::Stmt::Ret(e, _, _) => {
                if let Some(expr) = e {
                    self.validate_expr(expr);
                }
            }
            hir::Stmt::Break(e, _) => {
                if let Some(expr) = e {
                    self.validate_expr(expr);
                }
            }
            hir::Stmt::Continue(_) => {}
            hir::Stmt::Match(m) => {
                self.validate_expr(&m.subject);
                for arm in &m.arms {
                    self.validate_pat(&arm.pat);
                    if let Some(g) = &arm.guard {
                        self.validate_expr(g);
                    }
                    self.validate_block(&arm.body);
                }
            }
            hir::Stmt::Drop(_, _, _, _) => {}
            hir::Stmt::Asm(_) => {}
            hir::Stmt::ErrReturn(e, _, _) => self.validate_expr(e),
            hir::Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    self.validate_expr(e);
                }
            }
            hir::Stmt::StoreDelete(_, _, _) => {}
            hir::Stmt::StoreSet(_, updates, _, _) => {
                for (_, e) in updates {
                    self.validate_expr(e);
                }
            }
            hir::Stmt::Transaction(block, _) => self.validate_block(block),
            hir::Stmt::ChannelClose(e, _) => self.validate_expr(e),
            hir::Stmt::Stop(e, _) => self.validate_expr(e),
        }
    }

    fn validate_expr(&mut self, expr: &hir::Expr) {
        match &expr.kind {
            hir::ExprKind::BinOp(lhs, op, rhs) => {
                self.validate_expr(lhs);
                self.validate_expr(rhs);
                // Arithmetic ops require matching operand types
                use crate::ast::BinOp::*;
                match op {
                    Add | Sub | Mul | Div | Mod | Lt | Gt | Le | Ge => {
                        if lhs.ty != rhs.ty {
                            self.errors.push(format!(
                                "BinOp {:?} type mismatch at line {}: lhs {:?} vs rhs {:?}",
                                op, expr.span.line, lhs.ty, rhs.ty
                            ));
                        }
                    }
                    _ => {}
                }
            }
            hir::ExprKind::UnaryOp(_, inner) => self.validate_expr(inner),
            hir::ExprKind::Call(id, name, args) => {
                for a in args {
                    self.validate_expr(a);
                }
                // Check arity for known functions (skip variadics/builtins/closures)
                if let Some((_, expected)) = self.fn_sigs.get(&id.0) {
                    if args.len() != *expected {
                        self.errors.push(format!(
                            "call to `{}` at line {}: expected {} args, got {}",
                            name, expr.span.line, expected, args.len()
                        ));
                    }
                }
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                self.validate_expr(callee);
                for a in args {
                    self.validate_expr(a);
                }
            }
            hir::ExprKind::Builtin(_, args) => {
                for a in args {
                    self.validate_expr(a);
                }
            }
            hir::ExprKind::Method(obj, _, _, args) => {
                self.validate_expr(obj);
                for a in args {
                    self.validate_expr(a);
                }
            }
            hir::ExprKind::StringMethod(obj, _, args)
            | hir::ExprKind::VecMethod(obj, _, args)
            | hir::ExprKind::MapMethod(obj, _, args) => {
                self.validate_expr(obj);
                for a in args {
                    self.validate_expr(a);
                }
            }
            hir::ExprKind::VecNew(elems) => {
                for e in elems {
                    self.validate_expr(e);
                }
            }
            hir::ExprKind::Field(obj, _, _) => self.validate_expr(obj),
            hir::ExprKind::Index(arr, idx) => {
                self.validate_expr(arr);
                self.validate_expr(idx);
            }
            hir::ExprKind::Ternary(c, t, f) => {
                self.validate_expr(c);
                self.validate_expr(t);
                self.validate_expr(f);
            }
            hir::ExprKind::Coerce(inner, _) | hir::ExprKind::Cast(inner, _) => {
                self.validate_expr(inner);
            }
            hir::ExprKind::Array(elems) | hir::ExprKind::Tuple(elems) => {
                for e in elems {
                    self.validate_expr(e);
                }
            }
            hir::ExprKind::Struct(_, fields) | hir::ExprKind::VariantCtor(_, _, _, fields) => {
                for f in fields {
                    self.validate_expr(&f.value);
                }
            }
            hir::ExprKind::IfExpr(i) => {
                self.validate_expr(&i.cond);
                self.validate_block(&i.then);
                for (c, b) in &i.elifs {
                    self.validate_expr(c);
                    self.validate_block(b);
                }
                if let Some(b) = &i.els {
                    self.validate_block(b);
                }
            }
            hir::ExprKind::Pipe(lhs, _, _, args) => {
                self.validate_expr(lhs);
                for a in args {
                    self.validate_expr(a);
                }
            }
            hir::ExprKind::Block(block) => self.validate_block(block),
            hir::ExprKind::Lambda(_, body) => self.validate_block(body),
            hir::ExprKind::Ref(inner) | hir::ExprKind::Deref(inner) => {
                self.validate_expr(inner);
            }
            hir::ExprKind::ListComp(body, _id, _, iter, end, cond) => {
                self.validate_expr(body);
                self.validate_expr(iter);
                if let Some(e) = end {
                    self.validate_expr(e);
                }
                if let Some(c) = cond {
                    self.validate_expr(c);
                }
            }
            hir::ExprKind::Send(target, _, _, _, args) => {
                self.validate_expr(target);
                for a in args {
                    self.validate_expr(a);
                }
            }
            hir::ExprKind::DynDispatch(obj, _, _, args) => {
                self.validate_expr(obj);
                for a in args {
                    self.validate_expr(a);
                }
            }
            hir::ExprKind::DynCoerce(inner, _, _) => self.validate_expr(inner),
            hir::ExprKind::CoroutineCreate(_, stmts) => {
                self.validate_block(stmts);
            }
            hir::ExprKind::CoroutineNext(inner) | hir::ExprKind::Yield(inner) => {
                self.validate_expr(inner);
            }
            hir::ExprKind::Syscall(args) => {
                for a in args {
                    self.validate_expr(a);
                }
            }
            hir::ExprKind::StoreQuery(_, _) => {}
            // Leaf expressions — no sub-expressions to validate
            hir::ExprKind::Int(_)
            | hir::ExprKind::Float(_)
            | hir::ExprKind::Str(_)
            | hir::ExprKind::Bool(_)
            | hir::ExprKind::None
            | hir::ExprKind::Void
            | hir::ExprKind::Var(_, _)
            | hir::ExprKind::FnRef(_, _)
            | hir::ExprKind::VariantRef(_, _, _)
            | hir::ExprKind::MapNew
            | hir::ExprKind::Spawn(_)
            | hir::ExprKind::StoreCount(_)
            | hir::ExprKind::StoreAll(_)
            | hir::ExprKind::IterNext(_, _, _) => {}
            hir::ExprKind::ChannelCreate(_, cap) => self.validate_expr(cap),
            hir::ExprKind::ChannelSend(ch, val) => {
                self.validate_expr(ch);
                self.validate_expr(val);
            }
            hir::ExprKind::ChannelRecv(ch) => self.validate_expr(ch),
            hir::ExprKind::Select(arms, default_body) => {
                for arm in arms {
                    self.validate_expr(&arm.chan);
                    if let Some(val) = &arm.value {
                        self.validate_expr(val);
                    }
                    self.validate_block(&arm.body);
                }
                if let Some(body) = default_body {
                    self.validate_block(body);
                }
            }
        }
    }

    fn validate_pat(&mut self, pat: &hir::Pat) {
        match pat {
            hir::Pat::Bind(_, _, _, _) => {}
            hir::Pat::Ctor(_, _, subs, _) => {
                for s in subs {
                    self.validate_pat(s);
                }
            }
            hir::Pat::Or(pats, _) => {
                for p in pats {
                    self.validate_pat(p);
                }
            }
            hir::Pat::Tuple(pats, _) | hir::Pat::Array(pats, _) => {
                for p in pats {
                    self.validate_pat(p);
                }
            }
            hir::Pat::Lit(e) => self.validate_expr(e),
            hir::Pat::Wild(_) => {}
            hir::Pat::Range(lo, hi, _) => {
                self.validate_expr(lo);
                self.validate_expr(hi);
            }
        }
    }
}

fn stmt_span(stmt: &hir::Stmt) -> Span {
    match stmt {
        hir::Stmt::Bind(b) => b.span,
        hir::Stmt::TupleBind(_, _, s) => *s,
        hir::Stmt::Assign(_, _, s) => *s,
        hir::Stmt::Expr(e) => e.span,
        hir::Stmt::If(i) => i.span,
        hir::Stmt::While(w) => w.span,
        hir::Stmt::For(f) => f.span,
        hir::Stmt::Loop(l) => l.span,
        hir::Stmt::Ret(_, _, s) => *s,
        hir::Stmt::Break(_, s) => *s,
        hir::Stmt::Continue(s) => *s,
        hir::Stmt::Match(m) => m.span,
        hir::Stmt::Asm(a) => a.span,
        hir::Stmt::Drop(_, _, _, s) => *s,
        hir::Stmt::ErrReturn(_, _, s) => *s,
        hir::Stmt::StoreInsert(_, _, s) => *s,
        hir::Stmt::StoreDelete(_, _, s) => *s,
        hir::Stmt::StoreSet(_, _, _, s) => *s,
        hir::Stmt::Transaction(_, s) => *s,
        hir::Stmt::ChannelClose(_, s) => *s,
        hir::Stmt::Stop(_, s) => *s,
    }
}
