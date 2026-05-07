//! Module/path resolution: maps qualified names to defining items.

use crate::intern::Symbol;
use std::collections::HashMap;
use crate::ast::{self, Decl, Expr, Stmt};

/// Prefix a declaration's function/const name with a module name for qualified access.
/// ONLY the prefixed version is emitted — bare names are NOT registered.
/// Module functions are accessed exclusively via `module.fn()` syntax.
/// Externs are kept as-is (accessed via `extern.fn()` syntax).
/// Types, Enums, ErrDefs, Impls are NOT prefixed — they use structural names.
/// Prefix an entire module's declarations: rename functions/constants to
/// `module_name` and rewrite all intra-module references in bodies so that
/// recursive and sibling calls resolve to the prefixed names.
pub fn prefix_module(decls: Vec<Decl>, module: &str) -> Vec<Decl> {
    // 1. Collect all renameable names (functions + constants) defined in the module
    let mut rename_map: HashMap<Symbol, String> = HashMap::new();
    for d in &decls {
        match d {
            Decl::Fn(f) => {
                // Skip methods (TypeName_method) — they're already mangled
                if f.name.contains_str("_") && f.name.as_str().starts_with(char::is_uppercase) {
                    continue;
                }
                rename_map.insert(f.name.clone(), format!("{}_{}", module, f.name));
            }
            Decl::Const(name, _, _) => {
                rename_map.insert(name.clone(), format!("{}_{}", module, name));
            }
            _ => {}
        }
    }

    // 2. Rewrite each declaration
    decls
        .into_iter()
        .map(|d| match d {
            Decl::Fn(mut f) => {
                if let Some(new) = rename_map.get(&f.name) {
                    f.name = Symbol::intern(new);
                }
                // Exclude parameter names from renaming to avoid
                // shadowing params with sibling function names
                let mut fn_renames = rename_map.clone();
                for p in &f.params {
                    fn_renames.remove(&p.name);
                }
                rewrite_block(&mut f.body, &fn_renames);
                // Also rewrite bodies of default param exprs
                for p in &mut f.params {
                    if let Some(ref mut def) = p.default {
                        rewrite_expr(def, &fn_renames);
                    }
                }
                Decl::Fn(f)
            }
            Decl::Const(name, mut expr, span) => {
                let new_name = rename_map.get(&name).cloned().unwrap_or(name.as_str());
                rewrite_expr(&mut expr, &rename_map);
                Decl::Const(Symbol::intern(&new_name), expr, span)
            }
            Decl::Type(mut td) => {
                for m in &mut td.methods {
                    let mut mr = rename_map.clone();
                    for p in &m.params {
                        mr.remove(&p.name);
                    }
                    rewrite_block(&mut m.body, &mr);
                }
                Decl::Type(td)
            }
            Decl::Impl(mut ib) => {
                for m in &mut ib.methods {
                    let mut mr = rename_map.clone();
                    for p in &m.params {
                        mr.remove(&p.name);
                    }
                    rewrite_block(&mut m.body, &mr);
                }
                Decl::Impl(ib)
            }
            other => other,
        })
        .collect()
}

pub fn rewrite_block(block: &mut ast::Block, renames: &HashMap<Symbol, String>) {
    for stmt in block.iter_mut() {
        rewrite_stmt(stmt, renames);
    }
}

pub fn rewrite_stmt(stmt: &mut Stmt, renames: &HashMap<Symbol, String>) {
    match stmt {
        Stmt::Bind(b) => rewrite_expr(&mut b.value, renames),
        Stmt::TupleBind(_, e, _) => rewrite_expr(e, renames),
        Stmt::Assign(l, r, _) => {
            rewrite_expr(l, renames);
            rewrite_expr(r, renames);
        }
        Stmt::Expr(e) => rewrite_expr(e, renames),
        Stmt::If(i) => rewrite_if(i, renames),
        Stmt::While(w) => {
            rewrite_expr(&mut w.cond, renames);
            rewrite_block(&mut w.body, renames);
        }
        Stmt::For(f) => {
            rewrite_expr(&mut f.iter, renames);
            rewrite_block(&mut f.body, renames);
        }
        Stmt::Loop(l) => rewrite_block(&mut l.body, renames),
        Stmt::Ret(e, _) => {
            if let Some(e) = e {
                rewrite_expr(e, renames);
            }
        }
        Stmt::Break(e, _) => {
            if let Some(e) = e {
                rewrite_expr(e, renames);
            }
        }
        Stmt::Match(m) => {
            rewrite_expr(&mut m.subject, renames);
            for arm in &mut m.arms {
                if let Some(ref mut g) = arm.guard {
                    rewrite_expr(g, renames);
                }
                rewrite_block(&mut arm.body, renames);
            }
        }
        Stmt::ErrReturn(e, _) => rewrite_expr(e, renames),
        Stmt::Defer(b, _) => rewrite_block(b, renames),
        Stmt::StoreInsert(_, exprs, _) => {
            for fi in exprs {
                rewrite_expr(&mut fi.value, renames);
            }
        }
        Stmt::StoreSet(_, pairs, _, _) => {
            for (_, e) in pairs {
                rewrite_expr(e, renames);
            }
        }
        Stmt::Transaction(block, _) | Stmt::SimBlock(block, _) => rewrite_block(block, renames),
        Stmt::SimFor(f, _) => {
            rewrite_expr(&mut f.iter, renames);
            rewrite_block(&mut f.body, renames);
        }
        Stmt::ChannelClose(e, _) | Stmt::Stop(e, _) => rewrite_expr(e, renames),
        Stmt::Continue(_)
        | Stmt::Asm(_)
        | Stmt::StoreSave(_, _)
        | Stmt::StoreDelete(_, _, _)
        | Stmt::StoreDestroy(_, _, _)
        | Stmt::StoreRestore(_, _, _)
        | Stmt::UseLocal(_) => {}
    }
}

pub fn rewrite_if(i: &mut ast::If, renames: &HashMap<Symbol, String>) {
    rewrite_expr(&mut i.cond, renames);
    rewrite_block(&mut i.then, renames);
    for (c, b) in &mut i.elifs {
        rewrite_expr(c, renames);
        rewrite_block(b, renames);
    }
    if let Some(ref mut b) = i.els {
        rewrite_block(b, renames);
    }
}

pub fn rewrite_expr(expr: &mut Expr, renames: &HashMap<Symbol, String>) {
    match expr {
        Expr::Ident(name, _) => {
            if let Some(new) = renames.get(name) {
                *name = Symbol::intern(new);
            }
        }
        Expr::Call(callee, args, _) => {
            rewrite_expr(callee, renames);
            for a in args {
                rewrite_expr(a, renames);
            }
        }
        Expr::Method(obj, _, args, _) => {
            rewrite_expr(obj, renames);
            for a in args {
                rewrite_expr(a, renames);
            }
        }
        Expr::Field(obj, _, _) => rewrite_expr(obj, renames),
        Expr::BinOp(l, _, r, _) => {
            rewrite_expr(l, renames);
            rewrite_expr(r, renames);
        }
        Expr::UnaryOp(_, e, _) => rewrite_expr(e, renames),
        Expr::Index(a, b, _) => {
            rewrite_expr(a, renames);
            rewrite_expr(b, renames);
        }
        Expr::Ternary(a, b, c, _) => {
            rewrite_expr(a, renames);
            rewrite_expr(b, renames);
            rewrite_expr(c, renames);
        }
        Expr::As(e, _, _)
        | Expr::Ref(e, _)
        | Expr::Deref(e, _)
        | Expr::Yield(e, _)
        | Expr::Grad(e, _)
        | Expr::StrictCast(e, _, _)
        | Expr::Spread(e, _) => {
            rewrite_expr(e, renames);
        }
        Expr::Array(es, _)
        | Expr::Tuple(es, _)
        | Expr::Syscall(es, _)
        | Expr::NDArray(es, _)
        | Expr::Deque(es, _) => {
            for e in es {
                rewrite_expr(e, renames);
            }
        }
        Expr::SIMDLit(_, _, es, _) => {
            for e in es {
                rewrite_expr(e, renames);
            }
        }
        Expr::Struct(_, fields, _) => {
            for f in fields {
                rewrite_expr(&mut f.value, renames);
            }
        }
        Expr::Builder(_, fields, _) => {
            for f in fields {
                rewrite_expr(&mut f.value, renames);
            }
        }
        Expr::IfExpr(i) => rewrite_if(i, renames),
        Expr::Pipe(a, b, args, _) => {
            rewrite_expr(a, renames);
            rewrite_expr(b, renames);
            for e in args {
                rewrite_expr(e, renames);
            }
        }
        Expr::Block(block, _) => rewrite_block(block, renames),
        Expr::Lambda(_, _, body, _) => rewrite_block(body, renames),
        Expr::ListComp(body, _, iter, cond, end, _) => {
            rewrite_expr(body, renames);
            rewrite_expr(iter, renames);
            if let Some(c) = cond {
                rewrite_expr(c, renames);
            }
            if let Some(e) = end {
                rewrite_expr(e, renames);
            }
        }
        Expr::Query(e, _, _) => rewrite_expr(e, renames),
        Expr::Send(target, _, args, _) => {
            rewrite_expr(target, renames);
            for a in args {
                rewrite_expr(a, renames);
            }
        }
        Expr::ChannelSend(a, b, _) => {
            rewrite_expr(a, renames);
            rewrite_expr(b, renames);
        }
        Expr::ChannelRecv(e, _) => rewrite_expr(e, renames),
        Expr::ChannelCreate(_, e, _) => rewrite_expr(e, renames),
        Expr::Select(arms, default, _) => {
            for arm in arms {
                if let Some(ref mut v) = arm.value {
                    rewrite_expr(v, renames);
                }
                rewrite_block(&mut arm.body, renames);
            }
            if let Some(b) = default {
                rewrite_block(b, renames);
            }
        }
        Expr::Slice(a, b, c, _) => {
            rewrite_expr(a, renames);
            rewrite_expr(b, renames);
            rewrite_expr(c, renames);
        }
        Expr::OfCall(a, b, _) => {
            rewrite_expr(a, renames);
            rewrite_expr(b, renames);
        }
        Expr::NamedArg(_, e, _) => rewrite_expr(e, renames),
        Expr::AsFormat(e, _, _) => rewrite_expr(e, renames),
        Expr::Einsum(_, es, _) => {
            for e in es {
                rewrite_expr(e, renames);
            }
        }
        Expr::StoreGet(_, e, _) => rewrite_expr(e, renames),
        Expr::Receive(arms, _) => {
            for arm in arms {
                rewrite_block(&mut arm.body, renames);
            }
        }
        // Literals and leaf nodes — nothing to rewrite
        Expr::None(_)
        | Expr::Void(_)
        | Expr::Int(_, _)
        | Expr::Float(_, _)
        | Expr::Str(_, _)
        | Expr::Bool(_, _)
        | Expr::Placeholder(_)
        | Expr::IndexPlaceholder(_)
        | Expr::Embed(_, _)
        | Expr::Unreachable(_)
        | Expr::Spawn(_, _)
        | Expr::StoreQuery(_, _, _)
        | Expr::StoreCount(_, _, _)
        | Expr::StoreAll(_, _)
        | Expr::StoreFirst(_, _, _)
        | Expr::StoreExists(_, _, _)
        | Expr::StoreDistinct(_, _, _)
        | Expr::DispatchBlock(_, _, _)
        | Expr::QualifiedIdent(_, _, _) => {}
    }
}
