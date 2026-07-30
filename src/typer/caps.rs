use std::collections::HashMap;

use crate::ast::{self, CapAnnot};
use crate::cap_sites;
use crate::caps::{CapRow, CapSet, Capability};
use crate::intern::Symbol;

use super::scc;

#[derive(Debug)]
pub(in crate::typer) struct CapAnalysis {
    #[allow(dead_code)]
    pub rows: HashMap<Symbol, CapRow>,
}

fn parse_annot(a: &CapAnnot) -> Result<Capability, String> {
    let cap = match a.class.as_str() {
        "fs.read" => Capability::FsRead(a.scope.clone()),
        "fs.write" => Capability::FsWrite(a.scope.clone()),
        "net.client" => Capability::NetClient,
        "net.server" => Capability::NetServer,
        "process.spawn" => Capability::ProcessSpawn,
        "env.read" => Capability::EnvRead,
        "clock" => Capability::Clock,
        "time" => Capability::Time,
        "random" => Capability::Random,
        "ffi.unsafe" => Capability::FfiUnsafe,
        "state" => Capability::State(a.scope.clone()),
        other => {
            return Err(format!(
                "unknown capability class `{other}` at {:?}; the capability set is closed (caps.md §1.1)",
                a.span
            ));
        }
    };
    Ok(cap)
}

fn declared_set(needs: &[CapAnnot]) -> Result<CapSet, String> {
    let mut set = CapSet::new();
    for a in needs {
        set.insert(parse_annot(a)?);
    }
    Ok(set)
}

fn literal_path(arg: Option<&ast::Expr>) -> Option<String> {
    match arg {
        Some(ast::Expr::Str(s, _)) => Some(s.clone()),
        _ => None,
    }
}

fn dotted_path(e: &ast::Expr) -> Option<String> {
    match e {
        ast::Expr::Ident(n, _) => Some(n.to_string()),
        ast::Expr::Field(recv, f, _) => Some(format!("{}.{}", dotted_path(recv)?, f)),
        _ => None,
    }
}

fn record_site(sym: &str, args: &[ast::Expr], out: &mut CapSet) {
    if let Some(site) = cap_sites::lookup(sym) {
        let path = site
            .path_arg
            .and_then(|i| args.get(i))
            .and_then(|a| literal_path(Some(a)));
        out.insert(cap_sites::capability_of(site, path.as_deref()));
    }
}

fn primitive_caps_expr(e: &ast::Expr, out: &mut CapSet) {
    match e {
        ast::Expr::Call(callee, args, _) => {
            let sym = match callee.as_ref() {
                ast::Expr::Ident(n, _) => Some(n.to_string()),
                ast::Expr::QualifiedIdent(m, n, _) => Some(format!("{m}.{n}")),
                other => dotted_path(other),
            };
            if let Some(sym) = sym {
                record_site(&sym, args, out);
            }
            primitive_caps_expr(callee, out);
            for a in args {
                primitive_caps_expr(a, out);
            }
        }
        ast::Expr::Method(recv, method, args, _) => {
            if let Some(base) = dotted_path(recv) {
                record_site(&format!("{base}.{method}"), args, out);
            }
            primitive_caps_expr(recv, out);
            for a in args {
                primitive_caps_expr(a, out);
            }
        }
        _ => walk_expr(e, &mut |sub| primitive_caps_expr(sub, out)),
    }
}

fn walk_expr(e: &ast::Expr, f: &mut impl FnMut(&ast::Expr)) {
    match e {
        ast::Expr::BinOp(l, _, r, _)
        | ast::Expr::Index(l, r, _)
        | ast::Expr::ChannelSend(l, r, _) => {
            f(l);
            f(r);
        }
        ast::Expr::UnaryOp(_, e, _)
        | ast::Expr::As(e, _, _)
        | ast::Expr::Ref(e, _)
        | ast::Expr::Deref(e, _)
        | ast::Expr::Yield(e, _)
        | ast::Expr::ChannelRecv(e, _)
        | ast::Expr::Field(e, _, _) => f(e),
        ast::Expr::Method(recv, _, args, _) | ast::Expr::Send(recv, _, args, _) => {
            f(recv);
            for a in args {
                f(a);
            }
        }
        ast::Expr::Pipe(recv, func, args, _) => {
            f(recv);
            f(func);
            for a in args {
                f(a);
            }
        }
        ast::Expr::Ternary(c, t, e, _) => {
            f(c);
            f(t);
            f(e);
        }
        ast::Expr::Quaternary(s, ok, no, er, _) => {
            f(s);
            if let Some(x) = ok {
                f(x);
            }
            if let Some(x) = no {
                f(x);
            }
            if let Some(x) = er {
                f(x);
            }
        }
        ast::Expr::Array(es, _) | ast::Expr::Tuple(es, _) | ast::Expr::Syscall(es, _) => {
            for x in es {
                f(x);
            }
        }
        ast::Expr::Struct(_, fields, _) => {
            for fl in fields {
                f(&fl.value);
            }
        }
        _ => {}
    }
}

fn primitive_caps_stmt(s: &ast::Stmt, out: &mut CapSet) {
    let mut visit = |e: &ast::Expr| primitive_caps_expr(e, out);
    match s {
        ast::Stmt::Bind(b) => visit(&b.value),
        ast::Stmt::TupleBind(_, e, _)
        | ast::Stmt::Assign(_, e, _)
        | ast::Stmt::Expr(e)
        | ast::Stmt::Ret(Some(e), _)
        | ast::Stmt::ErrReturn(e, _)
        | ast::Stmt::Break(Some(e), _) => visit(e),
        ast::Stmt::If(i) => {
            visit(&i.cond);
            primitive_caps_block(&i.then, out);
            for (c, b) in &i.elifs {
                primitive_caps_expr(c, out);
                primitive_caps_block(b, out);
            }
            if let Some(el) = &i.els {
                primitive_caps_block(el, out);
            }
        }
        ast::Stmt::While(w) => {
            visit(&w.cond);
            primitive_caps_block(&w.body, out);
        }
        ast::Stmt::For(fr) => {
            visit(&fr.iter);
            primitive_caps_block(&fr.body, out);
        }
        ast::Stmt::Loop(l) => primitive_caps_block(&l.body, out),
        ast::Stmt::Match(m) => {
            visit(&m.subject);
            for arm in &m.arms {
                primitive_caps_block(&arm.body, out);
            }
        }
        _ => {}
    }
}

fn primitive_caps_block(block: &[ast::Stmt], out: &mut CapSet) {
    for s in block {
        primitive_caps_stmt(s, out);
    }
}

pub(in crate::typer) fn analyze(fns: &[&ast::Fn]) -> Result<CapAnalysis, String> {
    let call_graph = scc::build_call_graph(fns);
    let sccs = scc::tarjan_scc(&call_graph);

    let fn_lookup: HashMap<Symbol, &ast::Fn> = fns.iter().map(|f| (f.name, *f)).collect();

    let mut primitive: HashMap<Symbol, CapSet> = HashMap::new();
    for f in fns {
        let mut set = CapSet::new();
        primitive_caps_block(&f.body, &mut set);
        primitive.insert(f.name, set);
    }

    let mut rows: HashMap<Symbol, CapRow> = HashMap::new();
    for scc_group in &sccs {
        loop {
            let mut changed = false;
            for name in scc_group {
                let mut acc = primitive.get(name).cloned().unwrap_or_default();
                if let Some(callees) = call_graph.get(name) {
                    let mut sorted: Vec<&Symbol> = callees.iter().collect();
                    sorted.sort();
                    for c in sorted {
                        if let Some(r) = rows.get(c) {
                            acc.join(&r.caps);
                        }
                    }
                }
                let entry = rows.entry(*name).or_default();
                let before = entry.caps.len();
                entry.caps.join(&acc);
                if entry.caps.len() != before {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    for f in fns {
        let Some(needs) = &f.needs else { continue };
        let declared = declared_set(needs)?;
        let derived = rows.get(&f.name).cloned().unwrap_or_default();
        let uncovered = derived.caps.contained_in(&declared);
        if let Some(bad) = uncovered.first() {
            let path = shortest_intro_path(f.name, bad, &call_graph, &primitive, &fn_lookup);
            return Err(format!(
                "function `{}` declares `needs {}` but uses `{}`{}",
                f.name,
                declared.render(),
                bad.render(),
                path,
            ));
        }
    }

    Ok(CapAnalysis { rows })
}

fn shortest_intro_path(
    start: Symbol,
    cap: &Capability,
    graph: &HashMap<Symbol, std::collections::HashSet<Symbol>>,
    primitive: &HashMap<Symbol, CapSet>,
    fn_lookup: &HashMap<Symbol, &ast::Fn>,
) -> String {
    use std::collections::VecDeque;
    let introduces = |n: &Symbol| primitive.get(n).map(|s| s.satisfies(cap)).unwrap_or(false);
    let mut q: VecDeque<Vec<Symbol>> = VecDeque::new();
    let mut seen: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
    q.push_back(vec![start]);
    seen.insert(start);
    while let Some(path) = q.pop_front() {
        let last = *path.last().unwrap();
        if introduces(&last) {
            let chain = path
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" -> ");
            return format!(" (introduced via {chain})");
        }
        if let Some(callees) = graph.get(&last) {
            let mut sorted: Vec<&Symbol> = callees.iter().collect();
            sorted.sort();
            for c in sorted {
                if fn_lookup.contains_key(c) && seen.insert(*c) {
                    let mut next = path.clone();
                    next.push(*c);
                    q.push_back(next);
                }
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn fns_of(src: &str) -> Vec<ast::Fn> {
        let t = Lexer::new(src).tokenize().expect("lex");
        let prog = Parser::new(t).parse_program().expect("parse");
        prog.decls
            .into_iter()
            .filter_map(|d| match d {
                ast::Decl::Fn(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    fn run(src: &str) -> Result<CapAnalysis, String> {
        let owned = fns_of(src);
        let refs: Vec<&ast::Fn> = owned.iter().collect();
        analyze(&refs)
    }

    #[test]
    fn pure_fn_infers_empty() {
        let a = run("*add a, b returns i64\n    a + b\n").unwrap();
        let row = a.rows.get(&Symbol::intern("add")).unwrap();
        assert!(row.caps.is_empty());
    }

    #[test]
    fn declared_pure_passes() {
        assert!(run("*add a, b returns i64 needs pure\n    a + b\n").is_ok());
    }

    #[test]
    fn unknown_cap_class_rejected() {
        let err = run("*f needs teleport\n    1\n").unwrap_err();
        assert!(err.contains("unknown capability"));
    }

    #[test]
    fn primitive_site_introduces_cap() {
        let a = run("*f\n    std.net.connect(host)\n").unwrap();
        let row = a.rows.get(&Symbol::intern("f")).unwrap();
        assert!(row.caps.satisfies(&Capability::NetClient));
    }

    #[test]
    fn cap_propagates_through_call_graph() {
        let src = "*lo\n    std.net.connect(host)\n*hi\n    lo()\n";
        let a = run(src).unwrap();
        let hi = a.rows.get(&Symbol::intern("hi")).unwrap();
        assert!(hi.caps.satisfies(&Capability::NetClient));
    }

    #[test]
    fn declared_narrower_than_derived_is_rejected() {
        let src = "*f needs fs.read\n    std.net.connect(host)\n";
        let err = run(src).unwrap_err();
        assert!(err.contains("net.client"), "diag: {err}");
        assert!(err.contains("introduced via"), "diag: {err}");
    }

    #[test]
    fn scoped_write_satisfies_wider_needs() {
        let src = "*f needs fs.write './out'\n    std.fs.write_file('./out/log', data)\n";
        assert!(run(src).is_ok());
    }

    #[test]
    fn scoped_write_outside_declared_scope_rejected() {
        let src = "*f needs fs.write './out'\n    std.fs.write_file('./etc', data)\n";
        assert!(run(src).is_err());
    }
}
