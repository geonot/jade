//! Implicit auto-import analysis and qualified module reference discovery.

use super::*;

/// Auto-import modules based on undefined references found in the program.
/// Uses the entity index to find which files provide the needed symbols.
pub(in crate::driver) fn resolve_implicit_imports(
    prog: &mut Program,
    base_dir: &std::path::Path,
    loaded: &mut HashSet<Symbol>,
    packages: &HashMap<Symbol, PathBuf>,
    entity_index: &EntityIndex,
) {
    let module_refs = collect_qualified_module_refs(prog);
    if module_refs.is_empty() {
        return;
    }
    if std::env::var("JINN_DEBUG_IMPORTS").is_ok() {
        eprintln!("[auto-import] qualified module refs: {:?}", module_refs);
    }

    let explicit_modules: HashSet<Symbol> = prog
        .decls
        .iter()
        .filter_map(|d| {
            if let Decl::Use(u) = d {
                Some(
                    u.alias
                        .clone()
                        .unwrap_or_else(|| u.path.last().cloned().unwrap_or_default()),
                )
            } else {
                None
            }
        })
        .collect();

    // Find which module files need to be imported. Bare names intentionally
    // never trigger implicit imports; users must write `module.symbol` or an
    // explicit `use` declaration.
    let mut files_to_import: HashMap<PathBuf, Vec<String>> = HashMap::new();
    for module in &module_refs {
        if explicit_modules.contains(module) {
            continue;
        }
        if let Some(file_path) = entity_index.modules.get(module) {
            files_to_import
                .entry(file_path.clone())
                .or_default()
                .push(module.to_string());
        }
    }

    for (file_path, _symbols) in &files_to_import {
        // Check if already loaded via a module key
        let file_canon = file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone());
        let key = file_canon.to_string_lossy().to_string();
        if loaded.contains(&Symbol::intern(&key)) {
            if std::env::var("JINN_DEBUG_IMPORTS").is_ok() {
                eprintln!(
                    "[auto-import] SKIP (already loaded): {}",
                    file_path.display()
                );
            }
            continue;
        }
        if std::env::var("JINN_DEBUG_IMPORTS").is_ok() {
            eprintln!(
                "[auto-import] IMPORTING: {} for {:?}",
                file_path.display(),
                _symbols
            );
        }
        loaded.insert(Symbol::intern(&key));

        let src = match fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let tokens = match Lexer::new(&src).tokenize() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let mut mod_prog = match Parser::new(tokens).parse_program() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Recursively resolve this module's explicit imports
        resolve_modules(
            &mut mod_prog,
            file_path.parent().unwrap_or(base_dir),
            loaded,
            packages,
        );

        // Derive module name from file path (e.g., "/path/to/json.jn" → "json")
        let mod_name = file_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut importable: Vec<Decl> = Vec::new();
        for d in mod_prog.decls {
            if matches!(d, Decl::Use(_)) {
                continue;
            }
            if let Decl::Fn(ref f) = d {
                if f.name == "main" && f.params.is_empty() {
                    // Unwrap implicit main constants
                    for stmt in &f.body {
                        if let Stmt::Bind(b) = stmt {
                            importable.push(Decl::Const(b.name.clone(), b.value.clone(), b.span));
                        }
                    }
                    continue;
                }
            }
            importable.push(d);
        }
        for pd in prefix_module(importable, &mod_name) {
            prog.decls.push(pd);
        }
        prog.decls.push(Decl::Use(crate::ast::UseDecl {
            path: vec![Symbol::intern(&mod_name)],
            imports: None,
            alias: None,
            span: crate::ast::Span::dummy(),
        }));
    }
}

fn collect_qualified_module_refs(prog: &Program) -> HashSet<Symbol> {
    let mut defined = HashSet::new();
    let mut modules = HashSet::new();

    for d in &prog.decls {
        if let Some(name) = decl_name(d) {
            defined.insert(name);
        }
    }

    fn walk_expr(e: &crate::ast::Expr, modules: &mut HashSet<Symbol>, defs: &mut HashSet<Symbol>) {
        use crate::ast::Expr;
        match e {
            Expr::Method(obj, _, args, _) => {
                if let Expr::Ident(name, _) = obj.as_ref() {
                    if !defs.contains(name) {
                        modules.insert(name.clone());
                    }
                } else {
                    walk_expr(obj, modules, defs);
                }
                for arg in args {
                    walk_expr(arg, modules, defs);
                }
            }
            Expr::Field(obj, _, _) => {
                if let Expr::Ident(name, _) = obj.as_ref() {
                    if !defs.contains(name) {
                        modules.insert(name.clone());
                    }
                } else {
                    walk_expr(obj, modules, defs);
                }
            }
            Expr::Call(callee, args, _) => {
                walk_expr(callee, modules, defs);
                for arg in args {
                    walk_expr(arg, modules, defs);
                }
            }
            Expr::BinOp(l, _, r, _) => {
                walk_expr(l, modules, defs);
                walk_expr(r, modules, defs);
            }
            Expr::UnaryOp(_, e, _) => walk_expr(e, modules, defs),
            Expr::IfExpr(if_expr) => {
                walk_expr(&if_expr.cond, modules, defs);
                walk_block(&if_expr.then, modules, defs);
                for (cond, body) in &if_expr.elifs {
                    walk_expr(cond, modules, defs);
                    walk_block(body, modules, defs);
                }
                if let Some(body) = &if_expr.els {
                    walk_block(body, modules, defs);
                }
            }
            Expr::Array(elems, _)
            | Expr::Tuple(elems, _)
            | Expr::NDArray(elems, _)
            | Expr::Deque(elems, _)
            | Expr::Syscall(elems, _) => {
                for elem in elems {
                    walk_expr(elem, modules, defs);
                }
            }
            Expr::Struct(_, fields, _) => {
                for field in fields {
                    walk_expr(&field.value, modules, defs);
                }
            }
            Expr::Builder(_, fields, _) => {
                for field in fields {
                    walk_expr(&field.value, modules, defs);
                }
            }
            Expr::Index(a, b, _) => {
                walk_expr(a, modules, defs);
                walk_expr(b, modules, defs);
            }
            Expr::Lambda(params, _, body, _) => {
                let mut local_defs = defs.clone();
                for param in params {
                    local_defs.insert(param.name.clone());
                }
                walk_block(body, modules, &mut local_defs);
            }
            Expr::Pipe(l, r, extra, _) => {
                walk_expr(l, modules, defs);
                walk_expr(r, modules, defs);
                for arg in extra {
                    walk_expr(arg, modules, defs);
                }
            }
            Expr::As(e, _, _)
            | Expr::StrictCast(e, _, _)
            | Expr::AsFormat(e, _, _)
            | Expr::Ref(e, _)
            | Expr::Deref(e, _)
            | Expr::Yield(e, _)
            | Expr::Grad(e, _)
            | Expr::NamedArg(_, e, _)
            | Expr::Spread(e, _) => walk_expr(e, modules, defs),
            Expr::Block(stmts, _) | Expr::DispatchBlock(_, stmts, _) => {
                walk_block(stmts, modules, defs)
            }
            Expr::Ternary(c, t, f, _) => {
                walk_expr(c, modules, defs);
                walk_expr(t, modules, defs);
                walk_expr(f, modules, defs);
            }
            Expr::ListComp(body, var, iter, filter, map, _) => {
                walk_expr(iter, modules, defs);
                let mut local_defs = defs.clone();
                local_defs.insert(var.clone().into());
                walk_expr(body, modules, &mut local_defs);
                if let Some(filter) = filter {
                    walk_expr(filter, modules, &mut local_defs);
                }
                if let Some(map) = map {
                    walk_expr(map, modules, &mut local_defs);
                }
            }
            Expr::Slice(a, lo, hi, _) => {
                walk_expr(a, modules, defs);
                walk_expr(lo, modules, defs);
                walk_expr(hi, modules, defs);
            }
            Expr::ChannelCreate(_, size, _) => walk_expr(size, modules, defs),
            Expr::ChannelSend(ch, val, _) => {
                walk_expr(ch, modules, defs);
                walk_expr(val, modules, defs);
            }
            Expr::ChannelRecv(ch, _) => walk_expr(ch, modules, defs),
            Expr::Send(obj, _, args, _) => {
                walk_expr(obj, modules, defs);
                for arg in args {
                    walk_expr(arg, modules, defs);
                }
            }
            Expr::OfCall(a, b, _) => {
                walk_expr(a, modules, defs);
                walk_expr(b, modules, defs);
            }
            Expr::Einsum(_, args, _) | Expr::SIMDLit(_, _, args, _) => {
                for arg in args {
                    walk_expr(arg, modules, defs);
                }
            }
            Expr::Select(arms, default, _) => {
                for arm in arms {
                    walk_expr(&arm.chan, modules, defs);
                    if let Some(value) = &arm.value {
                        walk_expr(value, modules, defs);
                    }
                    walk_block(&arm.body, modules, defs);
                }
                if let Some(default) = default {
                    walk_block(default, modules, defs);
                }
            }
            Expr::Receive(arms, _) => {
                for arm in arms {
                    walk_block(&arm.body, modules, defs);
                }
            }
            Expr::Query(base, clauses, _) => {
                walk_expr(base, modules, defs);
                for clause in clauses {
                    match clause {
                        crate::ast::QueryClause::Where(expr, _)
                        | crate::ast::QueryClause::Limit(expr, _)
                        | crate::ast::QueryClause::Take(expr, _)
                        | crate::ast::QueryClause::Skip(expr, _) => walk_expr(expr, modules, defs),
                        crate::ast::QueryClause::Set(_, expr, _) => walk_expr(expr, modules, defs),
                        _ => {}
                    }
                }
            }
            Expr::StoreQuery(_, filter, _)
            | Expr::StoreFirst(_, filter, _)
            | Expr::StoreExists(_, filter, _) => walk_store_filter(filter, modules, defs),
            Expr::StoreCount(_, filter, _) => {
                if let Some(filter) = filter {
                    walk_store_filter(filter, modules, defs);
                }
            }
            Expr::StoreGet(_, id, _) => walk_expr(id, modules, defs),
            Expr::StoreAll(_, _) | Expr::StoreDistinct(_, _, _) | Expr::Unreachable(_) => {}
            Expr::Ident(_, _)
            | Expr::Int(_, _)
            | Expr::Float(_, _)
            | Expr::Str(_, _)
            | Expr::Bool(_, _)
            | Expr::None(_)
            | Expr::Void(_)
            | Expr::Placeholder(_)
            | Expr::IndexPlaceholder(_)
            | Expr::QualifiedIdent(_, _, _)
            | Expr::Embed(_, _) => {}
            Expr::Spawn(name, _) => {
                if !defs.contains(name) {
                    modules.insert(name.clone());
                }
            }
        }
    }

    fn walk_block(
        stmts: &[crate::ast::Stmt],
        modules: &mut HashSet<Symbol>,
        defs: &mut HashSet<Symbol>,
    ) {
        for stmt in stmts {
            walk_stmt(stmt, modules, defs);
        }
    }

    fn walk_store_filter(
        filter: &crate::ast::StoreFilter,
        modules: &mut HashSet<Symbol>,
        defs: &mut HashSet<Symbol>,
    ) {
        walk_expr(&filter.value, modules, defs);
        for (_, cond) in &filter.extra {
            walk_expr(&cond.value, modules, defs);
        }
    }

    fn walk_stmt(
        stmt: &crate::ast::Stmt,
        modules: &mut HashSet<Symbol>,
        defs: &mut HashSet<Symbol>,
    ) {
        use crate::ast::Stmt;
        match stmt {
            Stmt::Expr(e) => walk_expr(e, modules, defs),
            Stmt::Bind(b) => {
                walk_expr(&b.value, modules, defs);
                defs.insert(b.name.clone());
            }
            Stmt::Assign(l, r, _) => {
                walk_expr(l, modules, defs);
                walk_expr(r, modules, defs);
            }
            Stmt::Ret(Some(e), _) | Stmt::ErrReturn(e, _) | Stmt::Break(Some(e), _) => {
                walk_expr(e, modules, defs)
            }
            Stmt::If(if_s) => {
                walk_expr(&if_s.cond, modules, defs);
                walk_block(&if_s.then, modules, defs);
                for (cond, body) in &if_s.elifs {
                    walk_expr(cond, modules, defs);
                    walk_block(body, modules, defs);
                }
                if let Some(body) = &if_s.els {
                    walk_block(body, modules, defs);
                }
            }
            Stmt::While(w) => {
                walk_expr(&w.cond, modules, defs);
                walk_block(&w.body, modules, defs);
            }
            Stmt::For(f) => {
                walk_expr(&f.iter, modules, defs);
                if let Some(end) = &f.end {
                    walk_expr(end, modules, defs);
                }
                if let Some(step) = &f.step {
                    walk_expr(step, modules, defs);
                }
                let mut local_defs = defs.clone();
                local_defs.insert(f.bind.clone());
                if let Some(bind2) = &f.bind2 {
                    local_defs.insert(bind2.clone());
                }
                walk_block(&f.body, modules, &mut local_defs);
            }
            Stmt::Loop(l) => walk_block(&l.body, modules, defs),
            Stmt::Match(m) => {
                walk_expr(&m.subject, modules, defs);
                for arm in &m.arms {
                    if let Some(guard) = &arm.guard {
                        walk_expr(guard, modules, defs);
                    }
                    walk_block(&arm.body, modules, defs);
                }
            }
            Stmt::TupleBind(names, e, _) => {
                walk_expr(e, modules, defs);
                for name in names {
                    defs.insert(name.clone());
                }
            }
            Stmt::ChannelClose(e, _) | Stmt::Stop(e, _) => walk_expr(e, modules, defs),
            Stmt::StoreInsert(_, exprs, _) => {
                for field in exprs {
                    walk_expr(&field.value, modules, defs);
                }
            }
            Stmt::StoreSet(_, pairs, filter, _) => {
                for (_, expr) in pairs {
                    walk_expr(expr, modules, defs);
                }
                walk_store_filter(filter, modules, defs);
            }
            Stmt::Transaction(body, _) | Stmt::SimBlock(body, _) | Stmt::Defer(body, _) => {
                walk_block(body, modules, defs)
            }
            Stmt::SimFor(f, _) => {
                walk_expr(&f.iter, modules, defs);
                let mut local_defs = defs.clone();
                local_defs.insert(f.bind.clone());
                walk_block(&f.body, modules, &mut local_defs);
            }
            Stmt::Ret(None, _)
            | Stmt::Break(None, _)
            | Stmt::Continue(_)
            | Stmt::Asm(_)
            | Stmt::StoreSave(_, _)
            | Stmt::StoreDelete(_, _, _)
            | Stmt::StoreDestroy(_, _, _)
            | Stmt::StoreRestore(_, _, _)
            | Stmt::UseLocal(_) => {}
        }
    }

    for d in &prog.decls {
        match d {
            Decl::Fn(f) => {
                let mut local_defs = defined.clone();
                for param in &f.params {
                    local_defs.insert(param.name.clone());
                }
                walk_block(&f.body, &mut modules, &mut local_defs);
            }
            Decl::Type(td) => {
                for method in &td.methods {
                    let mut local_defs = defined.clone();
                    for param in &method.params {
                        local_defs.insert(param.name.clone());
                    }
                    walk_block(&method.body, &mut modules, &mut local_defs);
                }
            }
            Decl::Impl(ib) => {
                for method in &ib.methods {
                    let mut local_defs = defined.clone();
                    for param in &method.params {
                        local_defs.insert(param.name.clone());
                    }
                    walk_block(&method.body, &mut modules, &mut local_defs);
                }
            }
            Decl::Actor(ad) => {
                for handler in &ad.handlers {
                    let mut local_defs = defined.clone();
                    for param in &handler.params {
                        local_defs.insert(param.name.clone());
                    }
                    if let Some(sleep_ms) = &handler.loop_sleep_ms {
                        walk_expr(sleep_ms, &mut modules, &mut local_defs);
                    }
                    walk_block(&handler.body, &mut modules, &mut local_defs);
                }
            }
            Decl::Test(test) => {
                let mut local_defs = defined.clone();
                walk_block(&test.body, &mut modules, &mut local_defs);
            }
            Decl::TopStmt(stmt) => {
                let mut local_defs = defined.clone();
                walk_stmt(stmt, &mut modules, &mut local_defs);
            }
            _ => {}
        }
    }

    modules
}
