use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use clap::{Parser as ClapParser, Subcommand};
use inkwell::OptimizationLevel;
use inkwell::context::Context;

use crate::ast::{Decl, Program, Stmt};
use crate::cache::{Cache, build_package_map};
use crate::codegen::Compiler;
use crate::intern::Symbol;
use crate::lexer::Lexer;
use crate::lock::Lockfile;
use crate::ownership::OwnershipVerifier;
use crate::parser::Parser;
use crate::perceus::PerceusPass;
use crate::pkg::{Dependency, Package, SemVer};
use crate::resolve::prefix_module;
use crate::typer::Typer;

use super::cli::*;
use super::project::*;

/// Collect all identifiers referenced in the program (function calls, type refs,
/// variable refs, struct constructors, etc.) that are not defined by the program itself.
pub(super) fn collect_undefined_refs(prog: &Program) -> HashSet<Symbol> {
    let mut defined = HashSet::new();
    let mut referenced = HashSet::new();

    // Collect defined names
    for d in &prog.decls {
        match d {
            Decl::Fn(f) => {
                defined.insert(f.name.clone());
            }
            Decl::Type(t) => {
                defined.insert(t.name.clone());
            }
            Decl::Enum(e) => {
                defined.insert(e.name.clone());
                for v in &e.variants {
                    defined.insert(v.name.clone());
                }
            }
            Decl::Extern(e) => {
                defined.insert(e.name.clone());
            }
            Decl::ErrDef(e) => {
                defined.insert(e.name.clone());
            }
            Decl::Actor(a) => {
                defined.insert(a.name.clone());
            }
            Decl::Store(s) => {
                defined.insert(s.name.clone());
            }
            Decl::Trait(t) => {
                defined.insert(t.name.clone());
            }
            Decl::Const(name, _, _) => {
                defined.insert(name.clone());
            }
            Decl::Impl(_) | Decl::Use(_) | Decl::Test(_) => {}
            Decl::Supervisor(s) => {
                defined.insert(s.name.clone());
            }
            Decl::TypeAlias(name, _, _) | Decl::Newtype(name, _, _) => {
                defined.insert(name.clone());
            }
            Decl::TopStmt(_) => {}
            Decl::Migration(m) => {
                defined.insert(m.name.clone());
            }
            Decl::View(v) => {
                defined.insert(v.name.clone());
            }
            Decl::Global(name, _, _) => {
                defined.insert(name.clone());
            }
        }
    }

    // Walk all expressions to find referenced identifiers
    fn walk_expr(e: &crate::ast::Expr, refs: &mut HashSet<Symbol>, defs: &mut HashSet<Symbol>) {
        use crate::ast::Expr;
        match e {
            Expr::Ident(name, _) => {
                refs.insert(name.clone());
            }
            Expr::Call(callee, args, _) => {
                walk_expr(callee, refs, defs);
                for a in args {
                    walk_expr(a, refs, defs);
                }
            }
            Expr::Method(obj, _method, args, _) => {
                walk_expr(obj, refs, defs);
                for a in args {
                    walk_expr(a, refs, defs);
                }
            }
            Expr::BinOp(l, _, r, _) => {
                walk_expr(l, refs, defs);
                walk_expr(r, refs, defs);
            }
            Expr::UnaryOp(_, e, _) => walk_expr(e, refs, defs),
            Expr::IfExpr(if_expr) => {
                walk_expr(&if_expr.cond, refs, defs);
                walk_block(&if_expr.then, refs, defs);
                for (c, b) in &if_expr.elifs {
                    walk_expr(c, refs, defs);
                    walk_block(b, refs, defs);
                }
                if let Some(eb) = &if_expr.els {
                    walk_block(eb, refs, defs);
                }
            }
            Expr::Array(elems, _)
            | Expr::Tuple(elems, _)
            | Expr::NDArray(elems, _)
            | Expr::Deque(elems, _) => {
                for e in elems {
                    walk_expr(e, refs, defs);
                }
            }
            Expr::Struct(name, inits, _) => {
                refs.insert(name.clone());
                for fi in inits {
                    walk_expr(&fi.value, refs, defs);
                }
            }
            Expr::Index(a, i, _) => {
                walk_expr(a, refs, defs);
                walk_expr(i, refs, defs);
            }
            Expr::Field(obj, _, _) => walk_expr(obj, refs, defs),
            Expr::Lambda(params, _, body, _) => {
                for p in params {
                    defs.insert(p.name.clone());
                }
                walk_block(body, refs, defs);
            }
            Expr::Pipe(l, r, extra, _) => {
                walk_expr(l, refs, defs);
                walk_expr(r, refs, defs);
                for a in extra {
                    walk_expr(a, refs, defs);
                }
            }
            Expr::As(e, _, _) | Expr::StrictCast(e, _, _) | Expr::AsFormat(e, _, _) => {
                walk_expr(e, refs, defs)
            }
            Expr::Block(stmts, _) => walk_block(stmts, refs, defs),
            Expr::Ref(e, _) | Expr::Deref(e, _) | Expr::Yield(e, _) | Expr::Grad(e, _) => {
                walk_expr(e, refs, defs)
            }
            Expr::Spawn(name, inits, _) => {
                refs.insert(name.clone());
                for (_, v) in inits {
                    walk_expr(v, refs, defs);
                }
            }
            Expr::Ternary(c, t, f, _) => {
                walk_expr(c, refs, defs);
                walk_expr(t, refs, defs);
                walk_expr(f, refs, defs);
            }
            Expr::ListComp(body, var, iter, filter, map, _) => {
                defs.insert(var.clone().into());
                walk_expr(body, refs, defs);
                walk_expr(iter, refs, defs);
                if let Some(f) = filter {
                    walk_expr(f, refs, defs);
                }
                if let Some(m) = map {
                    walk_expr(m, refs, defs);
                }
            }
            Expr::Slice(a, lo, hi, _) => {
                walk_expr(a, refs, defs);
                walk_expr(lo, refs, defs);
                walk_expr(hi, refs, defs);
            }
            Expr::ChannelCreate(_, sz, _) => walk_expr(sz, refs, defs),
            Expr::ChannelSend(ch, val, _) => {
                walk_expr(ch, refs, defs);
                walk_expr(val, refs, defs);
            }
            Expr::ChannelRecv(ch, _) => walk_expr(ch, refs, defs),
            Expr::Send(obj, _, args, _) => {
                walk_expr(obj, refs, defs);
                for a in args {
                    walk_expr(a, refs, defs);
                }
            }
            Expr::NamedArg(_, e, _) | Expr::Spread(e, _) => walk_expr(e, refs, defs),
            Expr::OfCall(a, b, _) => {
                if let Expr::Ident(name, _) = a.as_ref() {
                    match &*name.as_str() {
                        "fields" | "size" | "type" => {
                            walk_expr(b, refs, defs);
                            return;
                        }
                        _ => {}
                    }
                }
                walk_expr(a, refs, defs);
                walk_expr(b, refs, defs);
            }
            Expr::Builder(name, fields, _) => {
                refs.insert(name.clone());
                for f in fields {
                    walk_expr(&f.value, refs, defs);
                }
            }
            Expr::Einsum(_, args, _) | Expr::Syscall(args, _) => {
                for a in args {
                    walk_expr(a, refs, defs);
                }
            }
            Expr::SIMDLit(_, _, elems, _) => {
                for e in elems {
                    walk_expr(e, refs, defs);
                }
            }
            Expr::Select(arms, default, _) => {
                for arm in arms {
                    walk_expr(&arm.chan, refs, defs);
                    if let Some(v) = &arm.value {
                        walk_expr(v, refs, defs);
                    }
                    walk_block(&arm.body, refs, defs);
                }
                if let Some(d) = default {
                    walk_block(d, refs, defs);
                }
            }
            Expr::Receive(arms, _) => {
                for arm in arms {
                    walk_block(&arm.body, refs, defs);
                }
            }
            Expr::DispatchBlock(_, body, _) => walk_block(body, refs, defs),
            Expr::Query(base, _clauses, _) => {
                walk_expr(base, refs, defs);
            }
            _ => {} // Int, Float, Str, Bool, None, Void, Embed, Placeholder, etc.
        }
    }

    fn walk_pat(p: &crate::ast::Pat, refs: &mut HashSet<Symbol>, defs: &mut HashSet<Symbol>) {
        use crate::ast::Pat;
        match p {
            Pat::Ctor(name, pats, _) => {
                refs.insert(name.clone());
                for p in pats {
                    walk_pat(p, refs, defs);
                }
            }
            Pat::Or(pats, _) | Pat::Tuple(pats, _) | Pat::Array(pats, _) => {
                for p in pats {
                    walk_pat(p, refs, defs);
                }
            }
            Pat::Lit(e) => walk_expr(e, refs, defs),
            Pat::Ident(name, _) => {
                defs.insert(name.clone());
            }
            _ => {} // Wild, Range
        }
    }

    fn walk_block(
        stmts: &[crate::ast::Stmt],
        refs: &mut HashSet<Symbol>,
        defs: &mut HashSet<Symbol>,
    ) {
        for s in stmts {
            walk_stmt(s, refs, defs);
        }
    }

    fn walk_stmt(s: &crate::ast::Stmt, refs: &mut HashSet<Symbol>, defs: &mut HashSet<Symbol>) {
        use crate::ast::Stmt;
        match s {
            Stmt::Expr(e) => walk_expr(e, refs, defs),
            Stmt::Bind(b) => {
                defs.insert(b.name.clone());
                walk_expr(&b.value, refs, defs);
                if let Some(ty) = &b.ty {
                    walk_type(ty, refs);
                }
            }
            Stmt::Assign(l, r, _) => {
                walk_expr(l, refs, defs);
                walk_expr(r, refs, defs);
            }
            Stmt::Ret(Some(e), _) | Stmt::ErrReturn(e, _) | Stmt::Break(Some(e), _) => {
                walk_expr(e, refs, defs)
            }
            Stmt::Ret(None, _) | Stmt::Break(None, _) | Stmt::Continue(_) | Stmt::Nop(_) => {}
            Stmt::If(if_s) => {
                walk_expr(&if_s.cond, refs, defs);
                walk_block(&if_s.then, refs, defs);
                for (c, b) in &if_s.elifs {
                    walk_expr(c, refs, defs);
                    walk_block(b, refs, defs);
                }
                if let Some(eb) = &if_s.els {
                    walk_block(eb, refs, defs);
                }
            }
            Stmt::While(w) => {
                walk_expr(&w.cond, refs, defs);
                walk_block(&w.body, refs, defs);
            }
            Stmt::For(f) => {
                defs.insert(f.bind.clone());
                if let Some(b2) = &f.bind2 {
                    defs.insert(b2.clone());
                }
                walk_expr(&f.iter, refs, defs);
                if let Some(end) = &f.end {
                    walk_expr(end, refs, defs);
                }
                if let Some(step) = &f.step {
                    walk_expr(step, refs, defs);
                }
                walk_block(&f.body, refs, defs);
            }
            Stmt::Loop(l) => walk_block(&l.body, refs, defs),
            Stmt::Match(m) => {
                walk_expr(&m.subject, refs, defs);
                for arm in &m.arms {
                    walk_pat(&arm.pat, refs, defs);
                    if let Some(g) = &arm.guard {
                        walk_expr(g, refs, defs);
                    }
                    walk_block(&arm.body, refs, defs);
                }
            }
            Stmt::TupleBind(names, e, _) => {
                for n in names {
                    defs.insert(n.clone());
                }
                walk_expr(e, refs, defs);
            }
            Stmt::ChannelClose(e, _) | Stmt::Stop(e, _) => walk_expr(e, refs, defs),
            Stmt::StoreInsert(_, exprs, _) => {
                for fi in exprs {
                    walk_expr(&fi.value, refs, defs);
                }
            }
            Stmt::Transaction(body, _) | Stmt::SimBlock(body, _) => walk_block(body, refs, defs),
            Stmt::SimFor(f, _) => {
                walk_expr(&f.iter, refs, defs);
                walk_block(&f.body, refs, defs);
            }
            _ => {} // Asm, StoreDelete, StoreSet, UseLocal
        }
    }

    fn walk_type(ty: &crate::types::Type, refs: &mut HashSet<Symbol>) {
        use crate::types::Type;
        match ty {
            Type::Struct(name, args) => {
                refs.insert(name.clone());
                for a in args {
                    walk_type(a, refs);
                }
            }
            Type::Enum(name) => {
                refs.insert(name.clone());
            }
            Type::Vec(inner)
            | Type::Ptr(inner)
            | Type::Rc(inner)
            | Type::Weak(inner)
            | Type::Channel(inner)
            | Type::Set(inner)
            | Type::PriorityQueue(inner)
            | Type::Coroutine(inner)
            | Type::Deque(inner)
            | Type::Cow(inner)
            | Type::Generator(inner) => {
                walk_type(inner, refs);
            }
            Type::Map(k, v) => {
                walk_type(k, refs);
                walk_type(v, refs);
            }
            Type::Array(inner, _) => walk_type(inner, refs),
            Type::Tuple(elems) => {
                for e in elems {
                    walk_type(e, refs);
                }
            }
            Type::Fn(params, ret) => {
                for p in params {
                    walk_type(p, refs);
                }
                walk_type(ret, refs);
            }
            Type::NDArray(inner, _) | Type::SIMD(inner, _) => walk_type(inner, refs),
            Type::Alias(_, inner) | Type::Newtype(_, inner) => walk_type(inner, refs),
            Type::ActorRef(name) => {
                refs.insert(name.clone());
            }
            Type::DynTrait(name) => {
                refs.insert(name.clone());
            }
            _ => {} // primitives, TypeVar, etc.
        }
    }

    // Walk all function bodies in the program
    for d in &prog.decls {
        match d {
            Decl::Fn(f) => {
                // Register parameter names as defined (local scope)
                for p in &f.params {
                    defined.insert(p.name.clone());
                }
                for s in &f.body {
                    walk_stmt(s, &mut referenced, &mut defined);
                }
                // Check return type
                if let Some(ret) = &f.ret {
                    walk_type(ret, &mut referenced);
                }
                // Check param types
                for p in &f.params {
                    if let Some(ty) = &p.ty {
                        walk_type(ty, &mut referenced);
                    }
                }
            }
            Decl::Type(td) => {
                for field in &td.fields {
                    if let Some(ty) = &field.ty {
                        walk_type(ty, &mut referenced);
                    }
                }
                for m in &td.methods {
                    for p in &m.params {
                        defined.insert(p.name.clone());
                    }
                    for s in &m.body {
                        walk_stmt(s, &mut referenced, &mut defined);
                    }
                }
            }
            Decl::Impl(ib) => {
                for m in &ib.methods {
                    for p in &m.params {
                        defined.insert(p.name.clone());
                    }
                    for s in &m.body {
                        walk_stmt(s, &mut referenced, &mut defined);
                    }
                }
            }
            Decl::Actor(ad) => {
                for h in &ad.handlers {
                    for p in &h.params {
                        defined.insert(p.name.clone());
                    }
                    if let Some(sleep_ms) = &h.loop_sleep_ms {
                        walk_expr(sleep_ms, &mut referenced, &mut defined);
                    }
                    for s in &h.body {
                        walk_stmt(s, &mut referenced, &mut defined);
                    }
                }
            }
            _ => {}
        }
    }

    // Built-in names that should never trigger auto-import
    let builtins: HashSet<&str> = [
        "log",
        "print",
        "println",
        "assert",
        "len",
        "push",
        "pop",
        "append",
        "range",
        "input",
        "exit",
        "panic",
        "type_of",
        "size_of",
        "fields",
        "size",
        "type",
        "true",
        "false",
        "None",
        "Some",
        "Nothing",
        "Ok",
        "Err",
        "Vec",
        "Map",
        "Set",
        "String",
        "Array",
        "Channel",
        "Deque",
        "int",
        "float",
        "str",
        "bool",
        "void",
        "i8",
        "i16",
        "i32",
        "i64",
        "u8",
        "u16",
        "u32",
        "u64",
        "f32",
        "f64",
        "self",
        "main",
        "vec",
        "map",
        "set",
        "to_string",
        "to_int",
        "to_float",
        "to_bool",
        "abs",
        "min",
        "max",
        "sqrt",
        "floor",
        "ceil",
        "round",
        "not",
        "and",
        "or",
        "mod",
    ]
    .iter()
    .copied()
    .collect();

    referenced
        .difference(&defined)
        .filter(|name| !builtins.contains(&*name.as_str()))
        .cloned()
        .collect()
}
