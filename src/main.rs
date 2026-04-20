use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use clap::{Parser as ClapParser, Subcommand};
use inkwell::OptimizationLevel;
use inkwell::context::Context;

use jadec::ast::{Decl, Program, Stmt};
use jadec::intern::Symbol;
use jadec::cache::{Cache, build_package_map};
use jadec::codegen::Compiler;
use jadec::lexer::Lexer;
use jadec::lock::Lockfile;
use jadec::ownership::OwnershipVerifier;
use jadec::parser::Parser;
use jadec::perceus::PerceusPass;
use jadec::pkg::{Dependency, Package, SemVer};
use jadec::resolve::prefix_module;
use jadec::typer::Typer;

#[derive(ClapParser)]
#[command(name = "jadec", version = "0.0.0", about = "The Jade compiler")]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,

    input: Option<PathBuf>,
    #[arg(short, long, default_value = "a.out")]
    output: PathBuf,
    #[arg(long, hide = true)]
    emit_ir: bool,
    #[arg(long)]
    emit_llvm: bool,
    #[arg(long)]
    emit_hir: bool,
    #[arg(long)]
    emit_mir: bool,
    #[arg(long)]
    emit_obj: bool,
    #[arg(long, default_value = "3")]
    opt: u8,
    #[arg(long)]
    lto: bool,
    #[arg(long)]
    lib: bool,
    #[arg(long)]
    link: Vec<PathBuf>,
    #[arg(long)]
    debug: bool,
    #[arg(long)]
    debug_types: bool,
    #[arg(long, default_value_t = true)]
    warn_inferred_defaults: bool,
    #[arg(long)]
    no_warn_inferred_defaults: bool,
    #[arg(long)]
    strict_types: bool,
    #[arg(long)]
    lenient: bool,
    #[arg(long)]
    pedantic: bool,
    #[arg(long)]
    test: bool,
    #[arg(long)]
    emit_interface: bool,
    #[arg(long)]
    dump_tokens: bool,
    #[arg(long)]
    dump_ast: bool,
    /// Enable fast-math optimizations (nnan, ninf, nsz, arcp, contract, afn, reassoc)
    #[arg(long)]
    fast_math: bool,
    /// Guarantee deterministic floating-point results (disable FP reordering)
    #[arg(long)]
    deterministic_fp: bool,
    /// Enable incremental compilation (cache unchanged function artifacts)
    #[arg(long, hide = true)]
    incremental: bool,
    /// Number of parallel codegen threads (0 = auto-detect)
    #[arg(long, default_value = "0")]
    threads: usize,
    /// Target triple for cross-compilation (e.g., aarch64-unknown-linux-gnu)
    #[arg(long)]
    target: Option<String>,
    /// Target CPU name (e.g., cortex-a53, skylake)
    #[arg(long)]
    cpu: Option<String>,
    /// Target CPU features (e.g., +avx2,+sse4.2)
    #[arg(long)]
    features: Option<String>,
    /// Standalone mode: no runtime, no libc dependency
    #[arg(long)]
    standalone: bool,
}

#[derive(Subcommand)]
enum Cmd {
    Init {
        name: Option<String>,
    },
    Fetch,
    Update,
    /// Compile the project (uses project.jade entry if available)
    Build {
        #[arg(short, long, default_value = "a.out")]
        output: Option<PathBuf>,
        #[arg(long)]
        opt: Option<u8>,
        #[arg(long)]
        lto: bool,
    },
    /// Compile and run a file or the project
    Run {
        /// Source file to run (optional — uses project entry if omitted)
        file: Option<PathBuf>,
        /// Arguments to pass to the program
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Run project tests
    Test,
    /// Type-check without codegen
    Check,
    /// Format Jade source files
    Fmt {
        /// Files to format (default: all .jade files in current directory)
        files: Vec<PathBuf>,
    },
    /// Generate Jade extern declarations from a C header file
    Bind {
        /// Path to the C header file
        header: PathBuf,
    },
}

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn dirs_cache() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("jade")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache").join("jade")
    } else {
        PathBuf::from(".jade_cache")
    }
}

fn decl_name(d: &Decl) -> Option<Symbol> {
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

fn should_import_decl(d: &Decl, imports: &Option<Vec<Symbol>>) -> bool {
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

fn resolve_modules(
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
                if let Ok(iface) = jadec::interface::InterfaceFile::read_from(&jadei_path) {
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

#[derive(Debug, Default)]
struct ProjectConfig {
    name: Option<String>,
    version: Option<String>,
    entry: Option<String>,
    opt: Option<u8>,
    lto: Option<bool>,
    requires: Vec<Dependency>,
}

impl ProjectConfig {
    fn from_file(path: &std::path::Path) -> Result<Self, String> {
        use jadec::ast::{Expr, Stmt};
        let src =
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let tokens = Lexer::new(&src).tokenize().map_err(|e| format!("{e}"))?;
        let prog = Parser::new(tokens)
            .parse_program()
            .map_err(|e| format!("{e}"))?;
        let mut cfg = ProjectConfig::default();
        for decl in &prog.decls {
            if let Decl::Fn(f) = decl {
                for stmt in &f.body {
                    match stmt {
                        Stmt::Assign(Expr::Ident(name, _), val, _) => {
                            Self::set_field(&mut cfg, &name.as_str(), val);
                        }
                        // require 'name' 'url' 'version'
                        Stmt::Expr(Expr::Call(callee, args, _))
                            if matches!(callee.as_ref(), Expr::Ident(n, _) if n == "require")
                                && args.len() == 3 =>
                        {
                            if let (Expr::Str(name, _), Expr::Str(url, _), Expr::Str(ver, _)) =
                                (&args[0], &args[1], &args[2])
                            {
                                let version = SemVer::parse(ver)
                                    .map_err(|e| format!("project.jade require: {e}"))?;
                                cfg.requires.push(Dependency {
                                    name: name.clone(),
                                    url: url.clone(),
                                    version,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // Also check top-level const bindings: `name is 'foo'`
        for decl in &prog.decls {
            if let Decl::Const(name, val, _) = decl {
                Self::set_field(&mut cfg, &name.as_str(), val);
            }
        }
        Ok(cfg)
    }

    fn set_field(cfg: &mut ProjectConfig, name: &str, val: &jadec::ast::Expr) {
        use jadec::ast::Expr;
        match name {
            "name" => {
                if let Expr::Str(s, _) = val {
                    cfg.name = Some(s.clone());
                }
            }
            "version" => {
                if let Expr::Str(s, _) = val {
                    cfg.version = Some(s.clone());
                }
            }
            "entry" => {
                if let Expr::Str(s, _) = val {
                    cfg.entry = Some(s.clone());
                }
            }
            "opt" => {
                if let Expr::Int(n, _) = val {
                    cfg.opt = Some(*n as u8);
                }
            }
            "lto" => {
                if let Expr::Bool(b, _) = val {
                    cfg.lto = Some(*b);
                }
            }
            _ => {}
        }
    }
}

fn cmd_init(name: Option<String>) {
    let pkg_name = name.unwrap_or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "myproject".into())
    });
    let project_path = PathBuf::from("project.jade");
    if project_path.exists() {
        die("project.jade already exists");
    }
    let project_content = format!(
        "name is '{}'\nversion is '0.1.0'\nentry is 'source/main.jade'\n",
        pkg_name
    );
    fs::write(&project_path, &project_content)
        .unwrap_or_else(|e| die(&format!("cannot write project.jade: {e}")));

    // Create source directory and main.jade
    let source_dir = PathBuf::from("source");
    if !source_dir.exists() {
        fs::create_dir_all(&source_dir)
            .unwrap_or_else(|e| die(&format!("cannot create source/: {e}")));
    }
    let main_path = source_dir.join("main.jade");
    if !main_path.exists() {
        fs::write(&main_path, "*main\n    log('hello world')\n")
            .unwrap_or_else(|e| die(&format!("cannot write source/main.jade: {e}")));
    }
    println!("initialized project '{pkg_name}'");
}

fn cmd_fetch() {
    let project_path = PathBuf::from("project.jade");
    if !project_path.exists() {
        die("no project.jade found in current directory (run `jadec init` to create one)");
    }
    let cfg = ProjectConfig::from_file(&project_path)
        .unwrap_or_else(|e| die(&format!("project.jade: {e}")));
    if cfg.requires.is_empty() {
        println!("no dependencies to fetch");
        return;
    }
    let pkg = Package {
        name: cfg.name.unwrap_or_default(),
        version: cfg
            .version
            .and_then(|v| SemVer::parse(&v).ok())
            .unwrap_or(SemVer {
                major: 0,
                minor: 0,
                patch: 0,
            }),
        author: None,
        requires: cfg.requires,
    };
    let cache = Cache::new();
    let lock_path = PathBuf::from("jade.lock");
    let existing_lock = if lock_path.exists() {
        Some(Lockfile::from_file(&lock_path).unwrap_or_else(|e| die(&format!("jade.lock: {e}"))))
    } else {
        None
    };
    let resolved = cache
        .resolve(&pkg, existing_lock.as_ref())
        .unwrap_or_else(|e| die(&format!("resolve: {e}")));
    let lock_content = resolved.write();
    fs::write(&lock_path, &lock_content).unwrap_or_else(|e| die(&format!("write lock: {e}")));
    println!("fetched {} dependencies", pkg.requires.len());
}

fn cmd_update() {
    let project_path = PathBuf::from("project.jade");
    if !project_path.exists() {
        die("no project.jade found in current directory (run `jadec init` to create one)");
    }
    let cfg = ProjectConfig::from_file(&project_path)
        .unwrap_or_else(|e| die(&format!("project.jade: {e}")));
    if cfg.requires.is_empty() {
        println!("no dependencies to update");
        return;
    }
    let pkg = Package {
        name: cfg.name.unwrap_or_default(),
        version: cfg
            .version
            .and_then(|v| SemVer::parse(&v).ok())
            .unwrap_or(SemVer {
                major: 0,
                minor: 0,
                patch: 0,
            }),
        author: None,
        requires: cfg.requires,
    };
    let lock_path = PathBuf::from("jade.lock");
    let _ = fs::remove_file(&lock_path);
    let cache = Cache::new();
    let resolved = cache
        .resolve(&pkg, None)
        .unwrap_or_else(|e| die(&format!("resolve: {e}")));
    let lock_content = resolved.write();
    fs::write(&lock_path, &lock_content).unwrap_or_else(|e| die(&format!("write lock: {e}")));
    println!("updated {} dependencies", pkg.requires.len());
}

fn find_project_entry() -> PathBuf {
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
/// Returns the set of module keys (e.g. "math_utils", "utils.strings") for merged files.
fn merge_source_files(
    prog: &mut Program,
    source_dir: &std::path::Path,
    entry_canon: &std::path::Path,
) -> HashSet<Symbol> {
    fn collect_jade_files(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
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
struct EntityIndex {
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
    fn build(base_dir: &std::path::Path, packages: &HashMap<Symbol, PathBuf>) -> Self {
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

/// Collect all identifiers referenced in the program (function calls, type refs,
/// variable refs, struct constructors, etc.) that are not defined by the program itself.
fn collect_undefined_refs(prog: &Program) -> HashSet<Symbol> {
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
    fn walk_expr(e: &jadec::ast::Expr, refs: &mut HashSet<Symbol>, defs: &mut HashSet<Symbol>) {
        use jadec::ast::Expr;
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
            Expr::Ref(e, _) | Expr::Deref(e, _) | Expr::Yield(e, _) | Expr::Grad(e, _) | Expr::Try(e, _) => {
                walk_expr(e, refs, defs)
            }
            Expr::Spawn(name, _) => {
                refs.insert(name.clone());
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
            Expr::Query(base, clauses, _) => {
                walk_expr(base, refs, defs);
                for c in clauses {
                    match c {
                        jadec::ast::QueryClause::Where(e, _)
                        | jadec::ast::QueryClause::Limit(e, _)
                        | jadec::ast::QueryClause::Take(e, _)
                        | jadec::ast::QueryClause::Skip(e, _) => walk_expr(e, refs, defs),
                        jadec::ast::QueryClause::Set(_, e, _) => walk_expr(e, refs, defs),
                        _ => {}
                    }
                }
            }
            _ => {} // Int, Float, Str, Bool, None, Void, Embed, Placeholder, etc.
        }
    }

    fn walk_pat(p: &jadec::ast::Pat, refs: &mut HashSet<Symbol>, defs: &mut HashSet<Symbol>) {
        use jadec::ast::Pat;
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
        stmts: &[jadec::ast::Stmt],
        refs: &mut HashSet<Symbol>,
        defs: &mut HashSet<Symbol>,
    ) {
        for s in stmts {
            walk_stmt(s, refs, defs);
        }
    }

    fn walk_stmt(s: &jadec::ast::Stmt, refs: &mut HashSet<Symbol>, defs: &mut HashSet<Symbol>) {
        use jadec::ast::Stmt;
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
            Stmt::Ret(None, _) | Stmt::Break(None, _) | Stmt::Continue(_) => {}
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
                for e in exprs {
                    walk_expr(e, refs, defs);
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

    fn walk_type(ty: &jadec::types::Type, refs: &mut HashSet<Symbol>) {
        use jadec::types::Type;
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

/// Auto-import modules based on undefined references found in the program.
/// Uses the entity index to find which files provide the needed symbols.
fn resolve_implicit_imports(
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

fn load_packages(base_dir: &std::path::Path) -> HashMap<Symbol, PathBuf> {
    let project_jade = base_dir.join("project.jade");
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
    let lock_file = base_dir.join("jade.lock");
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

fn compile_and_link(
    input: &std::path::Path,
    output: &std::path::Path,
    opt_level: u8,
    lto: bool,
    test_mode: bool,
    _bench: bool,
    fast_math: bool,
    deterministic_fp: bool,
    emit_mir: bool,
    incremental: bool,
) {
    let src = fs::read_to_string(input)
        .unwrap_or_else(|e| die(&format!("cannot read {}: {e}", input.display())));
    let tokens = Lexer::new(&src)
        .tokenize()
        .unwrap_or_else(|e| die(&format!("{e}")));
    let mut prog = Parser::new(tokens)
        .parse_program()
        .unwrap_or_else(|e| die(&format!("{e}")));

    // Multi-file project: merge all .jade files from the source directory
    let base_dir = input.parent().unwrap_or(std::path::Path::new("."));
    let input_canon = input.canonicalize().unwrap_or_else(|_| input.to_path_buf());
    let merged = merge_source_files(&mut prog, base_dir, &input_canon);

    let mut loaded: HashSet<Symbol> = merged;
    // Prevent auto-import from re-importing the entry file itself
    loaded.insert(Symbol::intern(&input_canon.to_string_lossy()));
    let packages = load_packages(base_dir);
    resolve_modules(&mut prog, base_dir, &mut loaded, &packages);
    let entity_index = EntityIndex::build(base_dir, &packages);
    resolve_implicit_imports(&mut prog, base_dir, &mut loaded, &packages, &entity_index);

    let mut typer = Typer::new();
    typer.set_source_dir(base_dir.to_path_buf());
    if test_mode {
        typer.set_test_mode(true);
    }
    let mut hir_prog = match typer.lower_program(&prog) {
        Ok(p) => p,
        Err(e) => die(&format!("hir: {e}")),
    };

    jadec::comptime::fold_program(&mut hir_prog);

    let mut perceus = PerceusPass::new();
    let _hir_hints = perceus.optimize(&hir_prog);

    let mut verifier = OwnershipVerifier::new();
    let diags = verifier.verify(&hir_prog);
    let mut has_hard_error = false;
    for d in &diags {
        let level = match d.kind {
            jadec::ownership::DiagKind::UseAfterMove => {
                has_hard_error = true;
                "error"
            }
            jadec::ownership::DiagKind::DoubleMutableBorrow => {
                has_hard_error = true;
                "error"
            }
            jadec::ownership::DiagKind::MoveOfBorrowed => {
                has_hard_error = true;
                "error"
            }
            jadec::ownership::DiagKind::InvalidRcDeref => {
                has_hard_error = true;
                "error"
            }
            jadec::ownership::DiagKind::ReturnOfBorrowed => {
                has_hard_error = true;
                "error"
            }
            jadec::ownership::DiagKind::WeakUpgradeWithoutCheck => "warning",
            jadec::ownership::DiagKind::Warning => "warning",
        };
        eprintln!("ownership: {} (line {}): {}", level, d.span.line, d.message);
    }
    if has_hard_error {
        die("compilation aborted due to ownership errors");
    }

    // ── MIR pass: HIR → MIR → optimize → (optional print) ──
    let mir_opt_level = match opt_level {
        0 => jadec::mir::opt::OptLevel::None,
        1 => jadec::mir::opt::OptLevel::Basic,
        _ => jadec::mir::opt::OptLevel::Full,
    };
    let mut mir_prog = jadec::mir::lower::lower_program(&hir_prog);
    for func in &mut mir_prog.functions {
        jadec::mir::opt::optimize(func, mir_opt_level);
    }
    if emit_mir {
        print!("{}", jadec::mir::printer::print_program(&mir_prog));
    }

    // ── Incremental compilation: check and report cache status ──
    if incremental {
        let incr_cache = jadec::incr::ArtifactCache::new();
        let (dirty, _keys) = jadec::incr::compute_dirty_set(&hir_prog, &incr_cache);
        if dirty.is_empty() {
            eprintln!("incr: all functions up to date");
        } else {
            eprintln!(
                "incr: {} of {} functions need recompilation",
                dirty.len(),
                hir_prog.fns.len()
            );
        }
    }

    let ctx = Context::create();
    let name = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "main".into());
    let mut comp = Compiler::new(&ctx, &name);
    comp.init_tbaa();
    comp.set_source(&src);
    if fast_math {
        comp.set_fast_math(true);
    }
    if deterministic_fp {
        comp.set_deterministic_fp();
    }

    {
        use jadec::codegen::mir_codegen::MirCodegen;
        use jadec::perceus::mir_perceus;
        let mir_hints = mir_perceus::analyze_mir_program(&mir_prog);
        let mut mir_cg = MirCodegen::new(&mut comp);
        if let Err(e) = mir_cg.compile_program(&mir_prog, &hir_prog, mir_hints) {
            die(&format!("mir-codegen: {e}"));
        }
    }

    let opt = match opt_level {
        0 => OptimizationLevel::None,
        1 => OptimizationLevel::Less,
        2 => OptimizationLevel::Default,
        3 => OptimizationLevel::Aggressive,
        _ => die("opt must be 0-3"),
    };

    let obj = output.with_extension("o");
    if let Err(e) = comp.emit_object(&obj, opt) {
        die(&format!("emit object: {e}"));
    }

    let mut cc = Command::new("cc");
    cc.arg(&obj).arg("-o").arg(output);
    if comp.needs_runtime {
        let rt_dir = env!("JADE_RT_DIR");
        cc.arg("-L").arg(rt_dir).arg("-ljade_rt").arg("-lpthread");
    }
    if comp.needs_ssl {
        if env!("JADE_HAS_SSL") != "1" {
            die("program uses std.tls or std.crypto but OpenSSL was not available when the compiler was built");
        }
        let rt_dir = env!("JADE_RT_DIR");
        cc.arg("-L").arg(rt_dir).arg("-ljade_ssl").arg("-lssl").arg("-lcrypto");
    }
    if comp.needs_sqlite {
        if env!("JADE_HAS_SQLITE") != "1" {
            die("program uses std.sqlite but SQLite3 was not available when the compiler was built");
        }
        let rt_dir = env!("JADE_RT_DIR");
        cc.arg("-L").arg(rt_dir).arg("-ljade_sqlite").arg("-lsqlite3");
    }
    cc.arg("-lm");
    if lto {
        cc.arg("-flto");
    }
    let status = cc.status();
    let _ = fs::remove_file(&obj);
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => die(&format!("linker failed with {s}")),
        Err(e) => die(&format!("cc: {e}")),
    }
}

fn main() {
    let cli = Cli::parse();

    if let Some(cmd) = cli.command {
        match cmd {
            Cmd::Init { name } => cmd_init(name),
            Cmd::Fetch => cmd_fetch(),
            Cmd::Update => cmd_update(),
            Cmd::Build { output, opt, lto } => {
                let entry = find_project_entry();
                let out = output.unwrap_or_else(|| PathBuf::from("a.out"));
                let opt_level = opt.unwrap_or(3);
                compile_and_link(
                    &entry,
                    &out,
                    opt_level,
                    lto,
                    false,
                    false,
                    cli.fast_math,
                    cli.deterministic_fp,
                    cli.emit_mir,
                    cli.incremental,
                );
            }
            Cmd::Run { file, args } => {
                let entry = match file {
                    Some(f) => {
                        if !f.exists() {
                            die(&format!("file not found: {}", f.display()));
                        }
                        f
                    }
                    None => find_project_entry(),
                };
                // Use content hash for caching compiled binaries
                let src_bytes = fs::read(&entry).unwrap_or_default();
                let hash = {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    src_bytes.hash(&mut h);
                    h.finish()
                };
                let cache_dir = dirs_cache();
                let _ = fs::create_dir_all(&cache_dir);
                let cached_bin = cache_dir.join(format!("jade_run_{:016x}", hash));
                if !cached_bin.exists() {
                    compile_and_link(
                        &entry,
                        &cached_bin,
                        2,
                        false,
                        false,
                        false,
                        cli.fast_math,
                        cli.deterministic_fp,
                        false,
                        cli.incremental,
                    );
                }
                let status = Command::new(&cached_bin).args(&args).status();
                match status {
                    Ok(s) => std::process::exit(s.code().unwrap_or(1)),
                    Err(e) => die(&format!("run failed: {e}")),
                }
            }
            Cmd::Test => {
                let entry = find_project_entry();
                compile_and_link(
                    &entry,
                    &PathBuf::from("./.jade_test_tmp"),
                    0,
                    false,
                    true,
                    false,
                    cli.fast_math,
                    cli.deterministic_fp,
                    false,
                    cli.incremental,
                );
                let status = Command::new("./.jade_test_tmp").status();
                let _ = fs::remove_file("./.jade_test_tmp");
                match status {
                    Ok(s) if s.success() => println!("all tests passed"),
                    Ok(s) => std::process::exit(s.code().unwrap_or(1)),
                    Err(e) => die(&format!("test failed: {e}")),
                }
            }
            Cmd::Check => {
                let entry = find_project_entry();
                let src = fs::read_to_string(&entry)
                    .unwrap_or_else(|e| die(&format!("cannot read {}: {e}", entry.display())));
                let tokens = Lexer::new(&src)
                    .tokenize()
                    .unwrap_or_else(|e| die(&format!("{e}")));
                let mut prog = Parser::new(tokens)
                    .parse_program()
                    .unwrap_or_else(|e| die(&format!("{e}")));
                let base_dir = entry.parent().unwrap_or(std::path::Path::new("."));
                let input_canon = entry.canonicalize().unwrap_or_else(|_| entry.clone());
                let merged = merge_source_files(&mut prog, base_dir, &input_canon);
                let mut loaded: HashSet<Symbol> = merged;
                let packages = load_packages(base_dir);
                resolve_modules(&mut prog, base_dir, &mut loaded, &packages);
                let entity_index = EntityIndex::build(base_dir, &packages);
                resolve_implicit_imports(
                    &mut prog,
                    base_dir,
                    &mut loaded,
                    &packages,
                    &entity_index,
                );
                let mut typer = Typer::new();
                typer.set_source_dir(base_dir.to_path_buf());
                match typer.lower_program(&prog) {
                    Ok(_) => println!("check passed"),
                    Err(e) => die(&format!("type error: {e}")),
                }
            }
            Cmd::Fmt { files } => {
                let targets: Vec<PathBuf> = if files.is_empty() {
                    // Find all .jade files in current directory recursively
                    fn collect_jade_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
                        if let Ok(entries) = fs::read_dir(dir) {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                if path.is_dir() {
                                    let name = path.file_name().unwrap_or_default();
                                    if name != "target" && name != ".git" {
                                        collect_jade_files(&path, out);
                                    }
                                } else if path.extension().map_or(false, |e| e == "jade") {
                                    out.push(path);
                                }
                            }
                        }
                    }
                    let mut found = Vec::new();
                    collect_jade_files(std::path::Path::new("."), &mut found);
                    found
                } else {
                    files
                };
                for path in &targets {
                    match fs::read_to_string(path) {
                        Ok(src) => match jadec::fmt::format_source(&src) {
                            Ok(formatted) => {
                                if formatted != src {
                                    fs::write(path, &formatted).unwrap_or_else(|e| {
                                        eprintln!("cannot write {}: {e}", path.display())
                                    });
                                    println!("formatted {}", path.display());
                                }
                            }
                            Err(e) => eprintln!("cannot format {}: {e}", path.display()),
                        },
                        Err(e) => eprintln!("cannot read {}: {e}", path.display()),
                    }
                }
            }
            Cmd::Bind { header } => match jadec::bind::bind_header(&header) {
                Ok(output) => print!("{output}"),
                Err(e) => die(&e),
            },
        }
        return;
    }

    let input = cli.input.unwrap_or_else(|| die("no input file provided"));
    let src = fs::read_to_string(&input)
        .unwrap_or_else(|e| die(&format!("cannot read {}: {e}", input.display())));
    let tokens = Lexer::new(&src)
        .tokenize()
        .unwrap_or_else(|e| die(&format!("{e}")));

    if cli.dump_tokens {
        for tok in &tokens {
            println!("{}:{} {}", tok.span.line, tok.span.col, tok.token);
        }
        return;
    }

    let mut prog = Parser::new(tokens)
        .parse_program()
        .unwrap_or_else(|e| die(&format!("{e}")));

    if cli.dump_ast {
        for decl in &prog.decls {
            println!("{decl:#?}");
        }
        return;
    }

    let base_dir = input.parent().unwrap_or_else(|| std::path::Path::new("."));
    let mut loaded = HashSet::new();
    // Prevent auto-import from re-importing the entry file itself
    if let Ok(canon) = input.canonicalize() {
        loaded.insert(Symbol::intern(&canon.to_string_lossy()));
    }

    // Load project.jade if present
    let project_jade = base_dir.join("project.jade");
    let project_config = if project_jade.exists() {
        Some(
            ProjectConfig::from_file(&project_jade)
                .unwrap_or_else(|e| die(&format!("project.jade: {e}"))),
        )
    } else {
        None
    };

    let packages = load_packages(base_dir);

    resolve_modules(&mut prog, base_dir, &mut loaded, &packages);
    let entity_index = EntityIndex::build(base_dir, &packages);
    resolve_implicit_imports(&mut prog, base_dir, &mut loaded, &packages, &entity_index);

    let mut typer = Typer::new();
    typer.set_source_dir(base_dir.to_path_buf());
    if cli.test {
        typer.set_test_mode(true);
    }
    if cli.debug_types {
        typer.set_debug_types(true);
    }
    if cli.warn_inferred_defaults && !cli.no_warn_inferred_defaults {
        typer.set_warn_inferred_defaults(true);
    }
    if cli.strict_types {
        typer.set_strict_types(true);
    }
    if cli.lenient {
        typer.set_lenient(true);
    }
    if cli.pedantic {
        typer.set_pedantic(true);
    }
    let mut hir_prog = match typer.lower_program(&prog) {
        Ok(hir_prog) => hir_prog,
        Err(e) => die(&format!("hir: {e}")),
    };

    if cli.emit_interface {
        let mod_name = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("module");
        let iface = jadec::interface::InterfaceFile::from_decls(mod_name, &prog.decls);
        let iface_path = input.with_extension("jadei");
        if let Err(e) = iface.write_to(&iface_path) {
            die(&format!("interface: {e}"));
        }
    }

    if cli.emit_hir {
        print!("{}", jadec::hir::pretty_print(&hir_prog));
        return;
    }

    let hir_errors = jadec::hir_validate::HirValidator::validate(&hir_prog);
    for e in &hir_errors {
        eprintln!("hir-validate: {e}");
    }
    if !hir_errors.is_empty() {
        die("compilation aborted due to HIR validation errors");
    }

    jadec::comptime::fold_program(&mut hir_prog);

    let mut perceus = PerceusPass::new();
    let hints = perceus.optimize(&hir_prog);
    if hints.stats.drops_elided > 0
        || hints.stats.reuse_sites > 0
        || hints.stats.borrows_promoted > 0
        || hints.stats.fbip_sites > 0
        || hints.stats.tail_reuse_sites > 0
        || hints.stats.speculative_reuse_sites > 0
        || hints.stats.pool_hints_found > 0
    {
        eprintln!(
            "perceus: {} drops elided, {} reuse, {} borrow→move, {} fbip, {} tail-reuse, {} speculative, {} pool-hints ({} bindings)",
            hints.stats.drops_elided,
            hints.stats.reuse_sites,
            hints.stats.borrows_promoted,
            hints.stats.fbip_sites,
            hints.stats.tail_reuse_sites,
            hints.stats.speculative_reuse_sites,
            hints.stats.pool_hints_found,
            hints.stats.total_bindings_analyzed,
        );
    }

    let mut verifier = OwnershipVerifier::new();
    let diags = verifier.verify(&hir_prog);
    let mut has_hard_error = false;
    for d in &diags {
        let level = match d.kind {
            jadec::ownership::DiagKind::UseAfterMove => {
                has_hard_error = true;
                "error"
            }
            jadec::ownership::DiagKind::DoubleMutableBorrow => {
                has_hard_error = true;
                "error"
            }
            jadec::ownership::DiagKind::MoveOfBorrowed => {
                has_hard_error = true;
                "error"
            }
            jadec::ownership::DiagKind::InvalidRcDeref => {
                has_hard_error = true;
                "error"
            }
            jadec::ownership::DiagKind::ReturnOfBorrowed => {
                has_hard_error = true;
                "error"
            }
            jadec::ownership::DiagKind::WeakUpgradeWithoutCheck => "warning",
            jadec::ownership::DiagKind::Warning => "warning",
        };
        eprintln!("ownership: {} (line {}): {}", level, d.span.line, d.message);
    }
    if has_hard_error {
        die("compilation aborted due to ownership errors");
    }

    // ── MIR pass: HIR → MIR → optimize → (optional print) ──
    let mir_opt_level = match cli.opt {
        0 => jadec::mir::opt::OptLevel::None,
        1 => jadec::mir::opt::OptLevel::Basic,
        _ => jadec::mir::opt::OptLevel::Full,
    };
    let mut mir_prog = jadec::mir::lower::lower_program(&hir_prog);
    for func in &mut mir_prog.functions {
        jadec::mir::opt::optimize(func, mir_opt_level);
    }

    // ── Strict-types: reject FnRef to polymorphic functions that aren't called ──
    if cli.strict_types {
        use jadec::mir::{InstKind, Terminator};
        let _fn_names: std::collections::HashSet<Symbol> =
            mir_prog.functions.iter().map(|f| f.name.clone()).collect();
        for func in &mir_prog.functions {
            for bb in &func.blocks {
                for inst in &bb.insts {
                    if let InstKind::FnRef(ref name) = inst.kind {
                        // If main returns a bare FnRef, its type won't match the expected i32 return.
                        // Check if the function return type is not compatible with FnRef usage.
                        if let Some(dest) = inst.dest {
                            // Check if this FnRef is used as a return value from main
                            if func.name == "main" {
                                if matches!(bb.terminator, Terminator::Return(Some(v)) if v == dest)
                                {
                                    die(&format!(
                                        "codegen: bare function reference `{name}` has unresolved return type in main"
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if cli.emit_mir {
        print!("{}", jadec::mir::printer::print_program(&mir_prog));
        return;
    }

    // ── Incremental compilation: check cache status ──
    if cli.incremental {
        let incr_cache = jadec::incr::ArtifactCache::new();
        let (dirty, _keys) = jadec::incr::compute_dirty_set(&hir_prog, &incr_cache);
        if dirty.is_empty() {
            eprintln!("incr: all functions up to date");
        } else {
            eprintln!(
                "incr: {} of {} functions need recompilation",
                dirty.len(),
                hir_prog.fns.len()
            );
        }
    }

    let ctx = Context::create();
    let name = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "main".into());
    let mut comp = Compiler::new(&ctx, &name);
    comp.init_tbaa();
    comp.set_source(&src);
    if cli.lib {
        comp.set_lib_mode();
    }
    if cli.debug {
        let filename = input.to_string_lossy().to_string();
        comp.enable_debug(&filename);
    }
    if cli.fast_math {
        comp.set_fast_math(true);
    }
    if cli.deterministic_fp {
        comp.set_deterministic_fp();
    }
    if let Some(ref target) = cli.target {
        comp.target_triple = Some(target.clone());
    }
    if let Some(ref cpu) = cli.cpu {
        comp.target_cpu = Some(cpu.clone());
    }
    if let Some(ref features) = cli.features {
        comp.target_features = Some(features.clone());
    }
    if cli.standalone {
        comp.standalone = true;
    }

    {
        use jadec::codegen::mir_codegen::MirCodegen;
        use jadec::perceus::mir_perceus;
        let mir_hints = mir_perceus::analyze_mir_program(&mir_prog);
        if mir_hints.stats.drops_elided > 0
            || mir_hints.stats.reuse_sites > 0
            || mir_hints.stats.borrows_promoted > 0
            || mir_hints.stats.fbip_sites > 0
            || mir_hints.stats.tail_reuse_sites > 0
            || mir_hints.stats.speculative_reuse_sites > 0
            || mir_hints.stats.pool_hints_found > 0
        {
            eprintln!(
                "mir-perceus: {} drops elided, {} reuse, {} borrow→move, {} fbip, {} tail-reuse, {} speculative, {} pool-hints, {} drops-fused ({} bindings)",
                mir_hints.stats.drops_elided,
                mir_hints.stats.reuse_sites,
                mir_hints.stats.borrows_promoted,
                mir_hints.stats.fbip_sites,
                mir_hints.stats.tail_reuse_sites,
                mir_hints.stats.speculative_reuse_sites,
                mir_hints.stats.pool_hints_found,
                mir_hints.stats.drops_fused,
                mir_hints.stats.total_bindings_analyzed,
            );
        }
        let mut mir_cg = MirCodegen::new(&mut comp);
        if let Err(e) = mir_cg.compile_program(&mir_prog, &hir_prog, mir_hints) {
            die(&format!("mir-codegen: {e}"));
        }
    }

    if cli.emit_ir {
        println!("{}", comp.emit_ir());
        return;
    }

    let opt_level = project_config
        .as_ref()
        .and_then(|p| p.opt)
        .unwrap_or(cli.opt);
    let opt = match opt_level {
        0 => OptimizationLevel::None,
        1 => OptimizationLevel::Less,
        2 => OptimizationLevel::Default,
        3 => OptimizationLevel::Aggressive,
        _ => die("opt must be 0-3"),
    };

    if cli.emit_llvm {
        match comp.emit_ir_optimized(opt) {
            Ok(ir) => println!("{ir}"),
            Err(e) => die(&format!("opt: {e}")),
        }
        return;
    }

    if cli.emit_obj {
        let obj = if cli.output.extension().is_some() {
            cli.output.clone()
        } else {
            cli.output.with_extension("o")
        };
        if let Err(e) = comp.emit_object(&obj, opt) {
            die(&format!("emit: {e}"));
        }
        return;
    }

    let obj = cli.output.with_extension("o");
    if let Err(e) = comp.emit_object(&obj, opt) {
        die(&format!("emit: {e}"));
    }

    let mut cc = Command::new("cc");
    cc.arg(&obj).arg("-o").arg(&cli.output);
    if comp.needs_runtime {
        let rt_dir = env!("JADE_RT_DIR");
        cc.arg("-L").arg(rt_dir).arg("-ljade_rt").arg("-lpthread");
    }
    if comp.needs_ssl {
        if env!("JADE_HAS_SSL") != "1" {
            die("program uses std.tls or std.crypto but OpenSSL was not available when the compiler was built");
        }
        let rt_dir = env!("JADE_RT_DIR");
        cc.arg("-L").arg(rt_dir).arg("-ljade_ssl").arg("-lssl").arg("-lcrypto");
    }
    if comp.needs_sqlite {
        if env!("JADE_HAS_SQLITE") != "1" {
            die("program uses std.sqlite but SQLite3 was not available when the compiler was built");
        }
        let rt_dir = env!("JADE_RT_DIR");
        cc.arg("-L").arg(rt_dir).arg("-ljade_sqlite").arg("-lsqlite3");
    }
    cc.arg("-lm");
    for extra in &cli.link {
        cc.arg(extra);
    }
    let use_lto = project_config
        .as_ref()
        .and_then(|p| p.lto)
        .unwrap_or(cli.lto);
    if use_lto {
        cc.arg("-flto");
    }
    if cli.debug {
        cc.arg("-g");
    }
    // Cross-compilation: use appropriate linker for target
    if let Some(ref triple) = comp.target_triple {
        cc.arg(&format!("--target={triple}"));
        if triple.contains("wasm") {
            // WASM targets: emit .wasm, use clang with wasm target or wasm-ld
            cc = Command::new("clang");
            cc.arg(&format!("--target={triple}"));
            cc.arg(&obj).arg("-o").arg(&cli.output);
            if !comp.standalone {
                cc.arg("-lc");
            } else {
                cc.arg("-nostdlib")
                    .arg("-Wl,--no-entry")
                    .arg("-Wl,--export-all");
            }
        }
    }
    let status = cc.status();
    let _ = fs::remove_file(&obj);
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => die(&format!("linker failed: {}", s.code().unwrap_or(-1))),
        Err(e) => die(&format!("linker: {e}")),
    }
}
