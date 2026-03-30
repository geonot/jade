use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use clap::{Parser as ClapParser, Subcommand};
use inkwell::OptimizationLevel;
use inkwell::context::Context;

use jadec::ast::{Decl, Program};
use jadec::cache::{Cache, build_package_map};
use jadec::codegen::Compiler;
use jadec::lexer::Lexer;
use jadec::lock::Lockfile;
use jadec::ownership::OwnershipVerifier;
use jadec::parser::Parser;
use jadec::perceus::PerceusPass;
use jadec::pkg::Package;
use jadec::typer::Typer;

#[derive(ClapParser)]
#[command(name = "jadec", version = "0.0.0", about = "The Jade compiler")]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,

    input: Option<PathBuf>,
    #[arg(short, long, default_value = "a.out")]
    output: PathBuf,
    #[arg(long)]
    emit_ir: bool,
    #[arg(long)]
    emit_llvm: bool,
    #[arg(long)]
    emit_hir: bool,
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
    #[arg(short = 'g', long)]
    debug: bool,
    #[arg(long)]
    debug_types: bool,
    #[arg(long)]
    warn_inferred_defaults: bool,
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
}

#[derive(Subcommand)]
enum Cmd {
    Init { name: Option<String> },
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
    /// Compile and run the project
    Run {
        /// Arguments to pass to the program
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

fn decl_name(d: &Decl) -> Option<&str> {
    match d {
        Decl::Fn(f) => Some(&f.name),
        Decl::Type(t) => Some(&t.name),
        Decl::Enum(e) => Some(&e.name),
        Decl::Extern(e) => Some(&e.name),
        Decl::ErrDef(e) => Some(&e.name),
        Decl::Actor(a) => Some(&a.name),
        Decl::Store(s) => Some(&s.name),
        Decl::Trait(t) => Some(&t.name),
        Decl::Const(name, _, _) => Some(name),
        Decl::Impl(i) => Some(&i.type_name),
        Decl::Test(_) | Decl::Use(_) => None,
        Decl::Supervisor(s) => Some(&s.name),
        Decl::TypeAlias(name, _, _) | Decl::Newtype(name, _, _) => Some(name),
    }
}

fn should_import_decl(d: &Decl, imports: &Option<Vec<String>>) -> bool {
    match imports {
        None => true,
        Some(names) => {
            if let Some(name) = decl_name(d) {
                names.iter().any(|n| n == name)
            } else {
                false
            }
        }
    }
}

fn resolve_modules(
    prog: &mut Program,
    base_dir: &std::path::Path,
    loaded: &mut HashSet<String>,
    packages: &HashMap<String, PathBuf>,
) {
    let uses: Vec<(Vec<String>, Option<Vec<String>>)> = prog
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
        let key = path.join(".");
        if loaded.contains(&key) {
            continue;
        }
        loaded.insert(key.clone());
        let file_path = path.join("/");
        let name = path.last().unwrap();
        let mut candidates = Vec::new();
        if let Some(pkg_path) = packages.get(&path[0]) {
            if path.len() > 1 {
                let rest = path[1..].join("/");
                candidates.push(pkg_path.join("src").join(format!("{rest}.jade")));
            } else {
                candidates.push(pkg_path.join("src").join(format!("{}.jade", path[0])));
            }
        }
        candidates.push(base_dir.join(format!("{file_path}.jade")));
        candidates.push(base_dir.join("std").join(format!("{name}.jade")));
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                candidates.push(exe_dir.join("std").join(format!("{name}.jade")));
            }
        }
        if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
            candidates.push(
                PathBuf::from(manifest)
                    .join("std")
                    .join(format!("{name}.jade")),
            );
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
                (Some(sm), Some(im)) => {
                    im.modified().ok() >= sm.modified().ok()
                }
                _ => false,
            };
            if use_cache {
                if let Ok(iface) = jadec::interface::InterfaceFile::read_from(&jadei_path) {
                    for d in iface.to_decls() {
                        if should_import_decl(&d, &imports) {
                            prog.decls.push(d);
                        }
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
        for d in mod_prog.decls {
            if !matches!(d, Decl::Use(_)) && should_import_decl(&d, &imports) {
                prog.decls.push(d);
            }
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
}

impl ProjectConfig {
    fn from_file(path: &std::path::Path) -> Result<Self, String> {
        use jadec::ast::{Expr, Stmt};
        let src = fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let tokens = Lexer::new(&src).tokenize().map_err(|e| format!("{e}"))?;
        let prog = Parser::new(tokens)
            .parse_program()
            .map_err(|e| format!("{e}"))?;
        let mut cfg = ProjectConfig::default();
        for decl in &prog.decls {
            if let Decl::Fn(f) = decl {
                for stmt in &f.body {
                    if let Stmt::Assign(Expr::Ident(name, _), val, _) = stmt {
                        Self::set_field(&mut cfg, name, val);
                    }
                }
            }
        }
        // Also check top-level const bindings: `name is 'foo'`
        for decl in &prog.decls {
            if let Decl::Const(name, val, _) = decl {
                Self::set_field(&mut cfg, name, val);
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
    let pkg_path = PathBuf::from("jade.pkg");
    if pkg_path.exists() {
        die("jade.pkg already exists");
    }
    let pkg = Package {
        name: pkg_name.clone(),
        version: jadec::pkg::SemVer {
            major: 0,
            minor: 1,
            patch: 0,
        },
        author: None,
        requires: Vec::new(),
    };
    fs::write(&pkg_path, pkg.to_string_repr())
        .unwrap_or_else(|e| die(&format!("cannot write jade.pkg: {e}")));
    println!("created jade.pkg for {pkg_name}");
}

fn cmd_fetch() {
    let pkg_path = PathBuf::from("jade.pkg");
    if !pkg_path.exists() {
        die("no jade.pkg found in current directory");
    }
    let pkg = Package::from_file(&pkg_path).unwrap_or_else(|e| die(&format!("jade.pkg: {e}")));
    if pkg.requires.is_empty() {
        println!("no dependencies to fetch");
        return;
    }
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
    let pkg_path = PathBuf::from("jade.pkg");
    if !pkg_path.exists() {
        die("no jade.pkg found in current directory");
    }
    let pkg = Package::from_file(&pkg_path).unwrap_or_else(|e| die(&format!("jade.pkg: {e}")));
    if pkg.requires.is_empty() {
        println!("no dependencies to update");
        return;
    }
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
    let default = cwd.join("src").join("main.jade");
    if default.exists() {
        return default;
    }
    die("no entry file found: create project.jade with `entry is 'src/main.jade'` or add src/main.jade");
}

fn load_packages(base_dir: &std::path::Path) -> HashMap<String, PathBuf> {
    let pkg_file = base_dir.join("jade.pkg");
    if pkg_file.exists() {
        let pkg = Package::from_file(&pkg_file).unwrap_or_else(|e| die(&format!("jade.pkg: {e}")));
        let lock_file = base_dir.join("jade.lock");
        let existing_lock = if lock_file.exists() {
            Some(Lockfile::from_file(&lock_file).unwrap_or_else(|e| die(&format!("jade.lock: {e}"))))
        } else {
            None
        };
        if pkg.requires.is_empty() {
            HashMap::new()
        } else {
            let cache = Cache::new();
            let resolved = cache
                .resolve(&pkg, existing_lock.as_ref())
                .unwrap_or_else(|e| die(&format!("resolve: {e}")));
            let lock_content = resolved.write();
            fs::write(&lock_file, &lock_content)
                .unwrap_or_else(|e| die(&format!("write lock: {e}")));
            build_package_map(&cache, &resolved)
        }
    } else {
        HashMap::new()
    }
}

fn compile_and_link(input: &std::path::Path, output: &std::path::Path, opt_level: u8, lto: bool, test_mode: bool, _bench: bool) {
    let src = fs::read_to_string(input)
        .unwrap_or_else(|e| die(&format!("cannot read {}: {e}", input.display())));
    let tokens = Lexer::new(&src)
        .tokenize()
        .unwrap_or_else(|e| die(&format!("{e}")));
    let mut prog = Parser::new(tokens)
        .parse_program()
        .unwrap_or_else(|e| die(&format!("{e}")));
    let base_dir = input.parent().unwrap_or(std::path::Path::new("."));
    let mut loaded = HashSet::new();
    let packages = load_packages(base_dir);
    resolve_modules(&mut prog, base_dir, &mut loaded, &packages);

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
    let hints = perceus.optimize(&hir_prog);

    let mut verifier = OwnershipVerifier::new();
    let diags = verifier.verify(&hir_prog);
    let mut has_hard_error = false;
    for d in &diags {
        let is_err = !matches!(d.kind, jadec::ownership::DiagKind::WeakUpgradeWithoutCheck | jadec::ownership::DiagKind::Warning);
        if is_err { has_hard_error = true; }
        let level = if is_err { "error" } else { "warning" };
        eprintln!("ownership: {} (line {}): {}", level, d.span.line, d.message);
    }
    if has_hard_error {
        die("compilation aborted due to ownership errors");
    }

    let ctx = Context::create();
    let name = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "main".into());
    let mut comp = Compiler::new(&ctx, &name);
    comp.set_source(&src);
    if let Err(e) = comp.compile_program(&hir_prog, hints) {
        die(&format!("codegen: {e}"));
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
    cc.arg(&obj).arg("-o").arg(output).arg("-lm");
    if comp.needs_runtime {
        let rt_dir = env!("JADE_RT_DIR");
        cc.arg("-L").arg(rt_dir).arg("-ljade_rt").arg("-lpthread");
    }
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
                compile_and_link(&entry, &out, opt_level, lto, false, false);
            }
            Cmd::Run { args } => {
                let entry = find_project_entry();
                let out = PathBuf::from("./.jade_run_tmp");
                compile_and_link(&entry, &out, 2, false, false, false);
                let status = Command::new(&out).args(&args).status();
                let _ = fs::remove_file(&out);
                match status {
                    Ok(s) => std::process::exit(s.code().unwrap_or(1)),
                    Err(e) => die(&format!("run failed: {e}")),
                }
            }
            Cmd::Test => {
                let entry = find_project_entry();
                compile_and_link(&entry, &PathBuf::from("./.jade_test_tmp"), 0, false, true, false);
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
                let mut loaded = HashSet::new();
                let packages = load_packages(base_dir);
                resolve_modules(&mut prog, base_dir, &mut loaded, &packages);
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
                        Ok(src) => {
                            match jadec::fmt::format_source(&src) {
                                Ok(formatted) => {
                                    if formatted != src {
                                        fs::write(path, &formatted)
                                            .unwrap_or_else(|e| eprintln!("cannot write {}: {e}", path.display()));
                                        println!("formatted {}", path.display());
                                    }
                                }
                                Err(e) => eprintln!("cannot format {}: {e}", path.display()),
                            }
                        }
                        Err(e) => eprintln!("cannot read {}: {e}", path.display()),
                    }
                }
            }
            Cmd::Bind { header } => {
                match jadec::bind::bind_header(&header) {
                    Ok(output) => print!("{output}"),
                    Err(e) => die(&e),
                }
            }
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

    // Load project.jade if present
    let project_jade = base_dir.join("project.jade");
    let project_config = if project_jade.exists() {
        Some(ProjectConfig::from_file(&project_jade)
            .unwrap_or_else(|e| die(&format!("project.jade: {e}"))))
    } else {
        None
    };

    let pkg_file = base_dir.join("jade.pkg");
    let packages = if pkg_file.exists() {
        let pkg = Package::from_file(&pkg_file).unwrap_or_else(|e| die(&format!("jade.pkg: {e}")));
        let lock_file = base_dir.join("jade.lock");
        let existing_lock = if lock_file.exists() {
            Some(
                Lockfile::from_file(&lock_file).unwrap_or_else(|e| die(&format!("jade.lock: {e}"))),
            )
        } else {
            None
        };
        if pkg.requires.is_empty() {
            HashMap::new()
        } else {
            let cache = Cache::new();
            let resolved = cache
                .resolve(&pkg, existing_lock.as_ref())
                .unwrap_or_else(|e| die(&format!("resolve: {e}")));
            let lock_content = resolved.write();
            fs::write(&lock_file, &lock_content)
                .unwrap_or_else(|e| die(&format!("write lock: {e}")));
            build_package_map(&cache, &resolved)
        }
    } else {
        HashMap::new()
    };

    resolve_modules(&mut prog, base_dir, &mut loaded, &packages);

    let mut typer = Typer::new();
    typer.set_source_dir(base_dir.to_path_buf());
    if cli.test {
        typer.set_test_mode(true);
    }
    if cli.debug_types {
        typer.set_debug_types(true);
    }
    if cli.warn_inferred_defaults {
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
    {
        eprintln!(
            "perceus: {} drops elided, {} reuse, {} borrow→move, {} fbip, {} tail-reuse, {} speculative ({} bindings)",
            hints.stats.drops_elided,
            hints.stats.reuse_sites,
            hints.stats.borrows_promoted,
            hints.stats.fbip_sites,
            hints.stats.tail_reuse_sites,
            hints.stats.speculative_reuse_sites,
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

    let ctx = Context::create();
    let name = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "main".into());
    let mut comp = Compiler::new(&ctx, &name);
    comp.set_source(&src);
    if cli.lib {
        comp.set_lib_mode();
    }
    if cli.debug {
        let filename = input.to_string_lossy().to_string();
        comp.enable_debug(&filename);
    }
    if let Err(e) = comp.compile_program(&hir_prog, hints) {
        die(&format!("codegen: {e}"));
    }

    if cli.emit_ir {
        println!("{}", comp.emit_ir());
        return;
    }

    let opt_level = project_config.as_ref().and_then(|p| p.opt).unwrap_or(cli.opt);
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
    cc.arg(&obj).arg("-o").arg(&cli.output).arg("-lm");
    if comp.needs_runtime {
        let rt_dir = env!("JADE_RT_DIR");
        cc.arg("-L").arg(rt_dir).arg("-ljade_rt").arg("-lpthread");
    }
    for extra in &cli.link {
        cc.arg(extra);
    }
    let use_lto = project_config.as_ref().and_then(|p| p.lto).unwrap_or(cli.lto);
    if use_lto {
        cc.arg("-flto");
    }
    if cli.debug {
        cc.arg("-g");
    }
    let status = cc.status();
    let _ = fs::remove_file(&obj);
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => die(&format!("linker failed: {}", s.code().unwrap_or(-1))),
        Err(e) => die(&format!("linker: {e}")),
    }
}
