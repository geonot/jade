use super::*;

pub(in crate::driver) fn decl_name(d: &Decl) -> Option<Symbol> {
    match d {
        Decl::Fn(f) => Some(f.name),
        Decl::Type(t) => Some(t.name),
        Decl::Enum(e) => Some(e.name),
        Decl::Extern(e) => Some(e.name),
        Decl::ErrDef(e) => Some(e.name),
        Decl::Actor(a) => Some(a.name),
        Decl::Store(s) => Some(s.name),
        Decl::Trait(t) => Some(t.name),
        Decl::Const(name, _, _) => Some(*name),
        Decl::Impl(i) => Some(i.type_name),
        Decl::Test(_) | Decl::Use(_) => None,
        Decl::Supervisor(s) => Some(s.name),
        Decl::TypeAlias(name, _, _) | Decl::Newtype(name, _, _) => Some(*name),
        Decl::TopStmt(_) => None,
        Decl::Migration(m) => Some(m.name),
        Decl::View(v) => Some(v.name),
        Decl::Global(name, _, _) => Some(*name),
    }
}

pub(in crate::driver) fn should_import_decl(d: &Decl, imports: &Option<Vec<Symbol>>) -> bool {
    match imports {
        None => true,
        Some(names) => {
            if let Some(name) = decl_name(d) {
                names.contains(&name)
            } else {
                false
            }
        }
    }
}

pub(in crate::driver) fn resolve_modules(
    prog: &mut Program,
    base_dir: &std::path::Path,
    loaded: &mut HashSet<Symbol>,
    packages: &HashMap<Symbol, PathBuf>,
) {
    let uses: Vec<(Vec<Symbol>, Option<Vec<Symbol>>)> = prog
        .decls
        .iter()
        .filter_map(|d| {
            if let Decl::Use(u) = d {
                Some((u.path.clone(), u.imports.clone()))
            } else {
                None
            }
        })
        .collect();
    for (path, imports) in uses {
        let path_strs: Vec<String> = path.iter().map(|s| s.as_str()).collect();
        let key = Symbol::intern(&path_strs.join("."));
        if loaded.contains(&key) {
            continue;
        }
        loaded.insert(key);
        let file_path = path_strs.join("/");
        let name = path.last().unwrap();
        let mut candidates = Vec::new();

        candidates.push(base_dir.join(format!("{file_path}.jn")));
        if let Some(project_root) = base_dir.parent() {
            candidates.push(project_root.join("source").join(format!("{file_path}.jn")));
        }

        if let Ok(exe) = std::env::current_exe()
            && let Some(exe_dir) = exe.parent()
        {
            candidates.push(exe_dir.join("std").join(format!("{name}.jn")));

            if let Some(parent) = exe_dir.parent() {
                candidates.push(parent.join("std").join(format!("{name}.jn")));
                if let Some(grandparent) = parent.parent() {
                    candidates.push(grandparent.join("std").join(format!("{name}.jn")));
                }
            }
        }
        if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
            candidates.push(
                PathBuf::from(manifest)
                    .join("std")
                    .join(format!("{name}.jn")),
            );
        }
        candidates.push(base_dir.join("std").join(format!("{name}.jn")));

        if let Some(pkg_path) = packages.get(&path[0]) {
            if path.len() > 1 {
                let rest = path_strs[1..].join("/");
                candidates.push(pkg_path.join("source").join(format!("{rest}.jn")));
                candidates.push(pkg_path.join("src").join(format!("{rest}.jn")));
            } else {
                candidates.push(pkg_path.join("source").join(format!("{}.jn", path[0])));
                candidates.push(pkg_path.join("src").join(format!("{}.jn", path[0])));
            }
        }

        if let Ok(pkg_paths) = std::env::var("JINN_PACKAGE_PATH") {
            for pkg_dir in pkg_paths.split(':') {
                let pkg_dir = PathBuf::from(pkg_dir);
                candidates.push(pkg_dir.join(format!("{file_path}.jn")));
            }
        }

        let candidate = candidates
            .into_iter()
            .find(|c| c.exists())
            .unwrap_or_else(|| die(&format!("module not found: {key}")));

        let jni_path = candidate.with_extension("jni");
        if jni_path.exists()
            && interface_reuse_enabled()
            && let Ok(iface) = crate::interface::InterfaceFile::read_from(&jni_path)
        {
            let importable: Vec<Decl> = iface
                .to_decls()
                .into_iter()
                .filter(|d| should_import_decl(d, &imports))
                .collect();
            for pd in flatten_module(importable, &name.as_str()) {
                prog.decls.push(pd);
            }
            continue;
        }

        let src = fs::read_to_string(&candidate)
            .unwrap_or_else(|e| die(&format!("cannot read {}: {e}", candidate.display())));
        let file_sym = Symbol::intern(&candidate.display().to_string());
        let tokens = Lexer::new(&src)
            .with_file(file_sym)
            .tokenize()
            .unwrap_or_else(|e| die(&format!("{}: {e}", candidate.display())));
        let mut mod_prog = Parser::new(tokens)
            .parse_program()
            .unwrap_or_else(|e| die(&format!("{}: {e}", candidate.display())));

        let own_decl_count = mod_prog.decls.len();
        resolve_modules(
            &mut mod_prog,
            candidate.parent().unwrap_or(base_dir),
            loaded,
            packages,
        );

        let all_decls = std::mem::take(&mut mod_prog.decls);
        let mut own_importable: Vec<Decl> = Vec::new();
        let mut transitive: Vec<Decl> = Vec::new();
        for (i, d) in all_decls.into_iter().enumerate() {
            if matches!(d, Decl::Use(_)) {
                transitive.push(d);
                continue;
            }

            if i >= own_decl_count {
                transitive.push(d);
                continue;
            }

            if let Decl::Fn(ref f) = d
                && f.name == "main"
                && f.params.is_empty()
            {
                for stmt in &f.body {
                    if let Stmt::Bind(b) = stmt {
                        let cd = Decl::Const(b.name, b.value.clone(), b.span);
                        if should_import_decl(&cd, &imports) {
                            own_importable.push(cd);
                        }
                    }
                }
                continue;
            }
            if should_import_decl(&d, &imports) {
                own_importable.push(d);
            }
        }
        for pd in flatten_module(own_importable, &name.as_str()) {
            prog.decls.push(pd);
        }
        for d in transitive {
            prog.decls.push(d);
        }
    }
}

pub(in crate::driver) fn find_project_entry() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let project_jinn = cwd.join("project.jn");
    if project_jinn.exists() {
        let cfg = ProjectConfig::from_file(&project_jinn)
            .unwrap_or_else(|e| die(&format!("project.jn: {e}")));
        if let Some(entry) = cfg.entry {
            let entry_path = cwd.join(&entry);
            if entry_path.exists() {
                return entry_path;
            }
            die(&format!("entry file not found: {entry}"));
        }
    }

    let source_main = cwd.join("source").join("main.jn");
    if source_main.exists() {
        return source_main;
    }
    let src_main = cwd.join("src").join("main.jn");
    if src_main.exists() {
        return src_main;
    }
    die(
        "no entry file found: create project.jn with `entry is 'source/main.jn'` or add source/main.jn",
    );
}

fn interface_reuse_enabled() -> bool {
    std::env::var("JINN_USE_INTERFACE_FILES").as_deref() == Ok("1")
}
