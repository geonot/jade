use crate::ast;
use std::collections::{HashMap, HashSet};

/// Extract direct call targets from an AST expression.
fn collect_calls_expr(expr: &ast::Expr, calls: &mut HashSet<String>) {
    match expr {
        ast::Expr::Call(callee, args, _) => {
            if let ast::Expr::Ident(name, _) = callee.as_ref() {
                calls.insert(name.clone());
            }
            collect_calls_expr(callee, calls);
            for a in args {
                collect_calls_expr(a, calls);
            }
        }
        ast::Expr::BinOp(l, _, r, _) => {
            collect_calls_expr(l, calls);
            collect_calls_expr(r, calls);
        }
        ast::Expr::UnaryOp(_, e, _)
        | ast::Expr::As(e, _, _)
        | ast::Expr::Ref(e, _)
        | ast::Expr::Deref(e, _)
        | ast::Expr::Yield(e, _)
        | ast::Expr::ChannelRecv(e, _)
        | ast::Expr::Field(e, _, _) => {
            collect_calls_expr(e, calls);
        }
        ast::Expr::Method(recv, _, args, _)
        | ast::Expr::Pipe(recv, _, args, _)
        | ast::Expr::Send(recv, _, args, _) => {
            collect_calls_expr(recv, calls);
            for a in args {
                collect_calls_expr(a, calls);
            }
        }
        ast::Expr::Index(a, b, _) | ast::Expr::ChannelSend(a, b, _) => {
            collect_calls_expr(a, calls);
            collect_calls_expr(b, calls);
        }
        ast::Expr::Ternary(c, t, f, _) => {
            collect_calls_expr(c, calls);
            collect_calls_expr(t, calls);
            collect_calls_expr(f, calls);
        }
        ast::Expr::Array(elems, _) | ast::Expr::Tuple(elems, _) | ast::Expr::Syscall(elems, _) => {
            for e in elems {
                collect_calls_expr(e, calls);
            }
        }
        ast::Expr::Struct(_, fields, _) => {
            for f in fields {
                collect_calls_expr(&f.value, calls);
            }
        }
        ast::Expr::IfExpr(i) => {
            collect_calls_expr(&i.cond, calls);
            collect_calls_block(&i.then, calls);
            for (c, b) in &i.elifs {
                collect_calls_expr(c, calls);
                collect_calls_block(b, calls);
            }
            if let Some(el) = &i.els {
                collect_calls_block(el, calls);
            }
        }
        ast::Expr::Block(block, _) => collect_calls_block(block, calls),
        ast::Expr::Lambda(_, _, body, _) => collect_calls_block(body, calls),
        ast::Expr::ListComp(body, _, iter, cond, map, _) => {
            collect_calls_expr(body, calls);
            collect_calls_expr(iter, calls);
            if let Some(c) = cond {
                collect_calls_expr(c, calls);
            }
            if let Some(m) = map {
                collect_calls_expr(m, calls);
            }
        }
        ast::Expr::Select(arms, default, _) => {
            for arm in arms {
                collect_calls_expr(&arm.chan, calls);
                if let Some(v) = &arm.value {
                    collect_calls_expr(v, calls);
                }
                collect_calls_block(&arm.body, calls);
            }
            if let Some(b) = default {
                collect_calls_block(b, calls);
            }
        }
        ast::Expr::ChannelCreate(_, cap, _) => collect_calls_expr(cap, calls),
        _ => {}
    }
}

fn collect_calls_stmt(stmt: &ast::Stmt, calls: &mut HashSet<String>) {
    match stmt {
        ast::Stmt::Bind(b) => {
            collect_calls_expr(&b.value, calls);
        }
        ast::Stmt::TupleBind(_, e, _) => collect_calls_expr(e, calls),
        ast::Stmt::Assign(_, e, _) => collect_calls_expr(e, calls),
        ast::Stmt::Expr(e) => collect_calls_expr(e, calls),
        ast::Stmt::Ret(v, _) => {
            if let Some(e) = v {
                collect_calls_expr(e, calls);
            }
        }
        ast::Stmt::If(i) => {
            collect_calls_expr(&i.cond, calls);
            collect_calls_block(&i.then, calls);
            for (c, b) in &i.elifs {
                collect_calls_expr(c, calls);
                collect_calls_block(b, calls);
            }
            if let Some(el) = &i.els {
                collect_calls_block(el, calls);
            }
        }
        ast::Stmt::While(w) => {
            collect_calls_expr(&w.cond, calls);
            collect_calls_block(&w.body, calls);
        }
        ast::Stmt::For(f) => {
            collect_calls_expr(&f.iter, calls);
            if let Some(e) = &f.end {
                collect_calls_expr(e, calls);
            }
            if let Some(s) = &f.step {
                collect_calls_expr(s, calls);
            }
            collect_calls_block(&f.body, calls);
        }
        ast::Stmt::Loop(l) => collect_calls_block(&l.body, calls),
        ast::Stmt::Break(v, _) => {
            if let Some(e) = v {
                collect_calls_expr(e, calls);
            }
        }
        ast::Stmt::Match(m) => {
            collect_calls_expr(&m.subject, calls);
            for arm in &m.arms {
                if let Some(g) = &arm.guard {
                    collect_calls_expr(g, calls);
                }
                collect_calls_block(&arm.body, calls);
            }
        }
        _ => {}
    }
}

fn collect_calls_block(block: &[ast::Stmt], calls: &mut HashSet<String>) {
    for s in block {
        collect_calls_stmt(s, calls);
    }
}

/// Build a call graph: map from function name to set of called function names.
pub(crate) fn build_call_graph(fns: &[&ast::Fn]) -> HashMap<String, HashSet<String>> {
    let fn_names: HashSet<&str> = fns.iter().map(|f| f.name.as_str()).collect();
    let mut graph = HashMap::new();
    for f in fns {
        let mut calls = HashSet::new();
        collect_calls_block(&f.body, &mut calls);
        // Only keep edges to known functions
        calls.retain(|c| fn_names.contains(c.as_str()));
        graph.insert(f.name.clone(), calls);
    }
    graph
}

/// Tarjan's SCC algorithm. Returns SCCs in reverse topological order
/// (dependencies before dependents).
pub(crate) fn tarjan_scc(graph: &HashMap<String, HashSet<String>>) -> Vec<Vec<String>> {
    struct State {
        index_counter: usize,
        stack: Vec<String>,
        on_stack: HashSet<String>,
        indices: HashMap<String, usize>,
        lowlinks: HashMap<String, usize>,
        result: Vec<Vec<String>>,
    }

    fn strongconnect(v: &str, graph: &HashMap<String, HashSet<String>>, state: &mut State) {
        let idx = state.index_counter;
        state.index_counter += 1;
        state.indices.insert(v.to_string(), idx);
        state.lowlinks.insert(v.to_string(), idx);
        state.stack.push(v.to_string());
        state.on_stack.insert(v.to_string());

        if let Some(neighbors) = graph.get(v) {
            for w in neighbors {
                if !state.indices.contains_key(w.as_str()) {
                    strongconnect(w, graph, state);
                    let w_low = state.lowlinks[w.as_str()];
                    let v_low = state.lowlinks.get_mut(v).unwrap();
                    if w_low < *v_low {
                        *v_low = w_low;
                    }
                } else if state.on_stack.contains(w.as_str()) {
                    let w_idx = state.indices[w.as_str()];
                    let v_low = state.lowlinks.get_mut(v).unwrap();
                    if w_idx < *v_low {
                        *v_low = w_idx;
                    }
                }
            }
        }

        let v_low = state.lowlinks[v];
        let v_idx = state.indices[v];
        if v_low == v_idx {
            let mut component = Vec::new();
            loop {
                let w = state.stack.pop().unwrap();
                state.on_stack.remove(&w);
                component.push(w.clone());
                if w == v {
                    break;
                }
            }
            state.result.push(component);
        }
    }

    let mut state = State {
        index_counter: 0,
        stack: Vec::new(),
        on_stack: HashSet::new(),
        indices: HashMap::new(),
        lowlinks: HashMap::new(),
        result: Vec::new(),
    };

    for v in graph.keys() {
        if !state.indices.contains_key(v.as_str()) {
            strongconnect(v, graph, &mut state);
        }
    }

    // Tarjan's naturally produces SCCs in topological order (sinks first)
    state.result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_cycles() {
        let mut graph = HashMap::new();
        graph.insert("a".into(), HashSet::from(["b".into()]));
        graph.insert("b".into(), HashSet::from(["c".into()]));
        graph.insert("c".into(), HashSet::new());
        let sccs = tarjan_scc(&graph);
        assert!(sccs.iter().all(|scc| scc.len() == 1));
    }

    #[test]
    fn test_simple_cycle() {
        let mut graph = HashMap::new();
        graph.insert("a".into(), HashSet::from(["b".into()]));
        graph.insert("b".into(), HashSet::from(["a".into()]));
        let sccs = tarjan_scc(&graph);
        let mutual: Vec<_> = sccs.iter().filter(|s| s.len() > 1).collect();
        assert_eq!(mutual.len(), 1);
        assert!(mutual[0].contains(&"a".into()));
        assert!(mutual[0].contains(&"b".into()));
    }

    #[test]
    fn test_self_recursion() {
        let mut graph = HashMap::new();
        graph.insert("f".into(), HashSet::from(["f".into()]));
        let sccs = tarjan_scc(&graph);
        assert_eq!(sccs.len(), 1);
        assert_eq!(sccs[0], vec!["f".to_string()]);
    }

    #[test]
    fn test_topological_order() {
        let mut graph = HashMap::new();
        graph.insert("main".into(), HashSet::from(["helper".into()]));
        graph.insert("helper".into(), HashSet::new());
        let sccs = tarjan_scc(&graph);
        let names: Vec<_> = sccs.iter().map(|s| s[0].as_str()).collect();
        let helper_pos = names.iter().position(|&n| n == "helper").unwrap();
        let main_pos = names.iter().position(|&n| n == "main").unwrap();
        assert!(helper_pos < main_pos, "helper should come before main");
    }
}
