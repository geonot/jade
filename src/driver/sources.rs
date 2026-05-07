use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use clap::{Parser as ClapParser, Subcommand};
use inkwell::OptimizationLevel;
use inkwell::context::Context;

use crate::ast::{Decl, Program, Stmt};
use crate::intern::Symbol;
use crate::cache::{Cache, build_package_map};
use crate::codegen::Compiler;
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
use super::undef::collect_undefined_refs;


pub(super) fn decl_name(d: &Decl) -> Option<Symbol> {
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

pub(super) fn should_import_decl(d: &Decl, imports: &Option<Vec<Symbol>>) -> bool {
    match imports {
        None => true,
        Some(names) => {
            if let Some(name) = decl_name(d) {
                names.iter().any(|n| name == *n)
            } else {
                false
            }
        }
    }
}

pub(super) fn resolve_modules(
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

        // 1. Standard library (bundled with compiler)
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                candidates.push(exe_dir.join("std").join(format!("{name}.jade")));
                // Check parent dirs (handles target/release/ layout during development)
                if let Some(parent) = exe_dir.parent() {
                    candidates.push(parent.join("std").join(format!("{name}.jade")));
                    if let Some(grandparent) = parent.parent() {
                        candidates.push(grandparent.join("std").join(format!("{name}.jade")));
                    }
                }
            }
        }
        if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
            candidates.push(
                PathBuf::from(manifest)
                    .join("std")
                    .join(format!("{name}.jade")),
            );
        }
        candidates.push(base_dir.join("std").join(format!("{name}.jade")));

        // 2. Project source directory (use foo → source/foo.jade, use foo/bar → source/foo/bar.jade)
        candidates.push(base_dir.join(format!("{file_path}.jade")));
        // Also check parent of base_dir in case base_dir is source/ itself
        if let Some(project_root) = base_dir.parent() {
            candidates.push(
                project_root
                    .join("source")
                    .join(format!("{file_path}.jade")),
            );
        }

        // 3. Packages from project.jade / lock
        if let Some(pkg_path) = packages.get(&path[0]) {
            if path.len() > 1 {
                let rest = path_strs[1..].join("/");
                candidates.push(pkg_path.join("source").join(format!("{rest}.jade")));
                candidates.push(pkg_path.join("src").join(format!("{rest}.jade")));
            } else {
                candidates.push(pkg_path.join("source").join(format!("{}.jade", path[0])));
                candidates.push(pkg_path.join("src").join(format!("{}.jade", path[0])));
            }
        }

        // 4. JADE_PACKAGE_PATH directories
        if let Ok(pkg_paths) = std::env::var("JADE_PACKAGE_PATH") {
            for pkg_dir in pkg_paths.split(':') {
                let pkg_dir = PathBuf::from(pkg_dir);
                candidates.push(pkg_dir.join(format!("{file_path}.jade")));
            }
        }

        let candidate = candidates
            .into_iter()
            .find(|c| c.exists())
            .unwrap_or_else(|| die(&format!("module not found: {key}")));

        // Check for a cached .jadei interface file
        let jadei_path = candidate.with_extension("jadei");
        if jadei_path.exists() {
            // If the interface file is newer than the source, use it
            let src_meta = fs::metadata(&candidate).ok();
            let iface_meta = fs::metadata(&jadei_path).ok();
            let use_cache = match (src_meta, iface_meta) {
                (Some(sm), Some(im)) => im.modified().ok() >= sm.modified().ok(),
                _ => false,
            };
            if use_cache {
                if let Ok(iface) = crate::interface::InterfaceFile::read_from(&jadei_path) {
                    let importable: Vec<Decl> = iface
                        .to_decls()
                        .into_iter()
                        .filter(|d| should_import_decl(d, &imports))
                        .collect();
                    for pd in prefix_module(importable, &name.as_str()) {
                        prog.decls.push(pd);
                    }
                    continue;
                }
            }
        }

        let src = fs::read_to_string(&candidate)
            .unwrap_or_else(|e| die(&format!("cannot read {}: {e}", candidate.display())));
        let tokens = Lexer::new(&src)
            .tokenize()
            .unwrap_or_else(|e| die(&format!("{}: {e}", candidate.display())));
        let mut mod_prog = Parser::new(tokens)
            .parse_program()
            .unwrap_or_else(|e| die(&format!("{}: {e}", candidate.display())));
        resolve_modules(
            &mut mod_prog,
            candidate.parent().unwrap_or(base_dir),
            loaded,
            packages,
        );
        // Collect all importable declarations from the module
        let mut importable: Vec<Decl> = Vec::new();
        for d in mod_prog.decls {
            if matches!(d, Decl::Use(_)) {
                continue;
            }
            // The parser wraps module-level top-level statements into an implicit *main.
            // Skip the implicit main from imported modules; constants are already Decl::Const.
            if let Decl::Fn(ref f) = d {
                if f.name == "main" && f.params.is_empty() {
                    for stmt in &f.body {
                        if let Stmt::Bind(b) = stmt {
                            let cd = Decl::Const(b.name.clone(), b.value.clone(), b.span);
                            if should_import_decl(&cd, &imports) {
                                importable.push(cd);
                            }
                        }
                    }
                    continue;
                }
            }
            if should_import_decl(&d, &imports) {
                importable.push(d);
            }
        }
        for pd in prefix_module(importable, &name.as_str()) {
            prog.decls.push(pd);
        }
    }
}


pub(super) fn find_project_entry() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let project_jade = cwd.join("project.jade");
    if project_jade.exists() {
        let cfg = ProjectConfig::from_file(&project_jade)
            .unwrap_or_else(|e| die(&format!("project.jade: {e}")));
        if let Some(entry) = cfg.entry {
            let entry_path = cwd.join(&entry);
            if entry_path.exists() {
                return entry_path;
            }
            die(&format!("entry file not found: {entry}"));
        }
    }
    // Try source/main.jade (new convention), then src/main.jade (legacy)
    let source_main = cwd.join("source").join("main.jade");
    if source_main.exists() {
        return source_main;
    }
    let src_main = cwd.join("src").join("main.jade");
    if src_main.exists() {
        return src_main;
    }
    die(
        "no entry file found: create project.jade with `entry is 'source/main.jade'` or add source/main.jade",
    );
}


/// Find all .jade files in source_dir (recursively), excluding the entry file,
/// parse them, and merge their declarations into the program.
/// Recursively collect .jade files under a directory.
pub(super) fn collect_jade_files(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_jade_files(&path, files);
            } else if path.extension().map_or(false, |e| e == "jade") {
                files.push(path);
            }
        }
    }
}

/// Returns the set of module keys (e.g. "math_utils", "utils.strings") for merged files.
pub(super) fn merge_source_files(
    prog: &mut Program,
    source_dir: &std::path::Path,
    entry_canon: &std::path::Path,
) -> HashSet<Symbol> {
    let mut source_files = Vec::new();
    collect_jade_files(source_dir, &mut source_files);
    let mut merged_keys = HashSet::new();

    for file in source_files {
        let file_canon = file.canonicalize().unwrap_or_else(|_| file.clone());
        if file_canon == entry_canon {
            continue;
        }
        // Compute module key from relative path (e.g. source/utils/strings.jade → "utils.strings")
        if let Ok(rel) = file.strip_prefix(source_dir) {
            let key = rel
                .with_extension("")
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(".");
            merged_keys.insert(Symbol::intern(&key));
        }
        let src = match fs::read_to_string(&file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("warning: cannot read {}: {e}", file.display());
                continue;
            }
        };
        let tokens = match Lexer::new(&src).tokenize() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("warning: {}: {e}", file.display());
                continue;
            }
        };
        let mod_prog = match Parser::new(tokens).parse_program() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("warning: {}: {e}", file.display());
                continue;
            }
        };
        // Derive module name from file stem (e.g., "helpers.jade" → "helpers")
        let mod_name = file
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut importable: Vec<Decl> = Vec::new();
        for d in mod_prog.decls {
            if matches!(d, Decl::Use(_)) {
                continue; // Use decls in source files will be resolved via the entry
            }
            // Skip *main from non-entry files — only include type/fn/const/enum decls
            if let Decl::Fn(ref f) = d {
                if f.name == "main" {
                    // Extract any top-level constants from the implicit *main wrapper
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
    }
    merged_keys
}


/// Entity index: maps symbol names (functions, types, enums, consts) to the
/// file that defines them. Used for implicit (auto) module resolution.
pub(super) struct EntityIndex {
    /// symbol_name → file_path
    symbols: HashMap<Symbol, PathBuf>,
}

impl EntityIndex {
    fn new() -> Self {
        Self {
            symbols: HashMap::new(),
        }
    }

    /// Scan a directory recursively for .jade files and index their exported symbols.
    fn scan_dir(&mut self, dir: &std::path::Path) {
        fn collect(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        collect(&path, files);
                    } else if path.extension().map_or(false, |e| e == "jade") {
                        files.push(path);
                    }
                }
            }
        }
        let mut files = Vec::new();
        collect(dir, &mut files);
        for file in files {
            self.scan_file(&file);
        }
    }

    /// Index a single .jade file by extracting top-level declaration names.
    fn scan_file(&mut self, path: &std::path::Path) {
        let src = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return,
        };
        let tokens = match Lexer::new(&src).tokenize() {
            Ok(t) => t,
            Err(_) => return,
        };
        let prog = match Parser::new(tokens).parse_program() {
            Ok(p) => p,
            Err(_) => return,
        };
        for d in &prog.decls {
            if let Some(name) = decl_name(d) {
                if name != "main" {
                    self.symbols
                        .entry(name)
                        .or_insert_with(|| path.to_path_buf());
                }
            }
            // Also index enum variant names
            if let Decl::Enum(ed) = d {
                for v in &ed.variants {
                    self.symbols
                        .entry(v.name)
                        .or_insert_with(|| path.to_path_buf());
                }
            }
            // Also index method names (TypeName_method)
            if let Decl::Fn(f) = d {
                // Unwrap implicit main to find module-level constants
                if f.name == "main" && f.params.is_empty() {
                    for stmt in &f.body {
                        if let Stmt::Bind(b) = stmt {
                            self.symbols
                                .entry(b.name.clone())
                                .or_insert_with(|| path.to_path_buf());
                        }
                    }
                }
            }
        }
    }

    /// Build the full entity index from std lib, source dir, and package paths.
    pub(super) fn build(base_dir: &std::path::Path, packages: &HashMap<Symbol, PathBuf>) -> Self {
        let mut idx = Self::new();

        // 1. Standard library
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                let std_dir = exe_dir.join("std");
                if std_dir.is_dir() {
                    idx.scan_dir(&std_dir);
                }
                // Check parent dirs (handles target/release/ layout during development)
                if let Some(parent) = exe_dir.parent() {
                    let std_dir = parent.join("std");
                    if std_dir.is_dir() {
                        idx.scan_dir(&std_dir);
                    }
                    if let Some(grandparent) = parent.parent() {
                        let std_dir = grandparent.join("std");
                        if std_dir.is_dir() {
                            idx.scan_dir(&std_dir);
                        }
                    }
                }
            }
        }
        if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
            let std_dir = PathBuf::from(manifest).join("std");
            if std_dir.is_dir() {
                idx.scan_dir(&std_dir);
            }
        }
        let std_dir = base_dir.join("std");
        if std_dir.is_dir() {
            idx.scan_dir(&std_dir);
        }

        // 2. Project source directory
        let source_dir = base_dir.join("source");
        if source_dir.is_dir() {
            idx.scan_dir(&source_dir);
        } else if base_dir
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s == "source")
            .unwrap_or(false)
        {
            idx.scan_dir(base_dir);
        }

        // 3. Package directories
        for (_, pkg_path) in packages {
            let source = pkg_path.join("source");
            if source.is_dir() {
                idx.scan_dir(&source);
            }
            let src = pkg_path.join("src");
            if src.is_dir() {
                idx.scan_dir(&src);
            }
        }

        // 4. JADE_PACKAGE_PATH directories
        if let Ok(pkg_paths) = std::env::var("JADE_PACKAGE_PATH") {
            for pkg_dir in pkg_paths.split(':') {
                let pkg_dir = PathBuf::from(pkg_dir);
                if pkg_dir.is_dir() {
                    idx.scan_dir(&pkg_dir);
                }
            }
        }

        idx
    }
}


/// Auto-import modules based on undefined references found in the program.
/// Uses the entity index to find which files provide the needed symbols.
pub(super) fn resolve_implicit_imports(
    prog: &mut Program,
    base_dir: &std::path::Path,
    loaded: &mut HashSet<Symbol>,
    packages: &HashMap<Symbol, PathBuf>,
    entity_index: &EntityIndex,
) {
    let undefined = collect_undefined_refs(prog);
    if undefined.is_empty() {
        return;
    }
    if std::env::var("JADE_DEBUG_IMPORTS").is_ok() {
        eprintln!("[auto-import] undefined refs: {:?}", undefined);
    }

    // Find which files need to be imported
    let mut files_to_import: HashMap<PathBuf, Vec<String>> = HashMap::new();
    for name in &undefined {
        if let Some(file_path) = entity_index.symbols.get(name) {
            files_to_import
                .entry(file_path.clone())
                .or_default()
                .push(name.to_string());
        }
    }

    for (file_path, _symbols) in &files_to_import {
        // Check if already loaded via a module key
        let file_canon = file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone());
        let key = file_canon.to_string_lossy().to_string();
        if loaded.contains(&Symbol::intern(&key)) {
            if std::env::var("JADE_DEBUG_IMPORTS").is_ok() {
                eprintln!(
                    "[auto-import] SKIP (already loaded): {}",
                    file_path.display()
                );
            }
            continue;
        }
        if std::env::var("JADE_DEBUG_IMPORTS").is_ok() {
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

        // Derive module name from file path (e.g., "/path/to/json.jade" → "json")
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
    }
}


pub(super) fn load_packages(base_dir: &std::path::Path) -> HashMap<Symbol, PathBuf> {
    let project_root = find_project_root(base_dir).unwrap_or_else(|| base_dir.to_path_buf());
    let project_jade = project_root.join("project.jade");
    let requires = if project_jade.exists() {
        match ProjectConfig::from_file(&project_jade) {
            Ok(cfg) => cfg.requires,
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    if requires.is_empty() {
        return HashMap::new();
    }
    let pkg = Package {
        name: String::new(),
        version: SemVer {
            major: 0,
            minor: 0,
            patch: 0,
        },
        author: None,
        requires,
    };
    let lock_file = project_root.join("jade.lock");
    let existing_lock = if lock_file.exists() {
        Some(Lockfile::from_file(&lock_file).unwrap_or_else(|e| die(&format!("jade.lock: {e}"))))
    } else {
        None
    };
    let cache = Cache::new();
    let resolved = cache
        .resolve(&pkg, existing_lock.as_ref())
        .unwrap_or_else(|e| die(&format!("resolve: {e}")));
    let lock_content = resolved.write();
    fs::write(&lock_file, &lock_content).unwrap_or_else(|e| die(&format!("write lock: {e}")));
    build_package_map(&cache, &resolved)
}
