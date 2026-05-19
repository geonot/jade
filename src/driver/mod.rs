//! Compiler driver: CLI dispatch and pipeline orchestration.

mod cli;
mod cmd_init;
mod cmd_pkg;
mod pipeline;
mod project;
mod sources;
mod undef;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use clap::Parser as ClapParser;
use inkwell::OptimizationLevel;
use inkwell::context::Context;

use crate::ast::{Decl, Program, Stmt};
use crate::cache::Cache;
use crate::codegen::Compiler;
use crate::intern::Symbol;
use crate::lexer::Lexer;
use crate::lock::Lockfile;
use crate::ownership::OwnershipVerifier;
use crate::parser::Parser;
use crate::perceus::PerceusPass;
use crate::typer::Typer;

use cli::{Cli, Cmd, die, dirs_cache, find_project_root, strip_codegen_prefix};
use cmd_init::cmd_init;
use cmd_pkg::{cmd_fetch, cmd_package, cmd_publish, cmd_update};
use pipeline::compile_and_link;
use project::ProjectConfig;
use sources::{
    EntityIndex, collect_jinn_files, find_project_entry, load_packages, merge_source_files,
    resolve_implicit_imports, resolve_modules,
};

pub fn run() {
    let cli = Cli::parse();

    if let Some(cmd) = cli.command {
        match cmd {
            Cmd::Init { name } => cmd_init(name),
            Cmd::Fetch => cmd_fetch(),
            Cmd::Update => cmd_update(),
            Cmd::Build {
                output,
                opt,
                lto,
                target,
                cpu,
                features,
                standalone,
            } => {
                let entry = find_project_entry();
                let out = output.unwrap_or_else(|| PathBuf::from("a.out"));
                let opt_level = opt.unwrap_or(3);
                let chosen_target = target.as_deref().or(cli.target.as_deref());
                let chosen_cpu = cpu.as_deref().or(cli.cpu.as_deref());
                let chosen_features = features.as_deref().or(cli.features.as_deref());
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
                    chosen_target,
                    chosen_cpu,
                    chosen_features,
                    standalone || cli.standalone,
                );
            }
            Cmd::Package { output, no_archive } => cmd_package(output, no_archive),
            Cmd::Publish {
                push,
                remote,
                force,
            } => cmd_publish(push, remote, force),
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
                // Cache compiled binaries by a hash of the source bytes
                // PLUS a fingerprint of the compiler itself, so that rebuilding
                // jinnc invalidates all cached binaries (otherwise stale
                // binaries silently mask compiler fixes — see CONTRIBUTING).
                let src_bytes = fs::read(&entry).unwrap_or_default();
                let compiler_fp: (u64, u64) = std::env::current_exe()
                    .ok()
                    .and_then(|p| fs::metadata(&p).ok())
                    .map(|m| {
                        let size = m.len();
                        let mtime = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_nanos() as u64)
                            .unwrap_or(0);
                        (size, mtime)
                    })
                    .unwrap_or((0, 0));
                let hash = {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    src_bytes.hash(&mut h);
                    env!("CARGO_PKG_VERSION").hash(&mut h);
                    compiler_fp.hash(&mut h);
                    h.finish()
                };
                let cache_dir = dirs_cache();
                let _ = fs::create_dir_all(&cache_dir);
                let cached_bin = cache_dir.join(format!("jinn_run_{:016x}", hash));
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
                        cli.target.as_deref(),
                        cli.cpu.as_deref(),
                        cli.features.as_deref(),
                        cli.standalone,
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
                    &PathBuf::from("./.jinn_test_tmp"),
                    0,
                    false,
                    true,
                    false,
                    cli.fast_math,
                    cli.deterministic_fp,
                    false,
                    cli.incremental,
                    cli.target.as_deref(),
                    cli.cpu.as_deref(),
                    cli.features.as_deref(),
                    cli.standalone,
                );
                let status = Command::new("./.jinn_test_tmp").status();
                let _ = fs::remove_file("./.jinn_test_tmp");
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
                    .with_file(Symbol::intern(&entry.display().to_string()))
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
                    // Find all .jn files in current directory recursively
                    fn collect_jinn_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
                        if let Ok(entries) = fs::read_dir(dir) {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                if path.is_dir() {
                                    let name = path.file_name().unwrap_or_default();
                                    if name != "target" && name != ".git" {
                                        collect_jinn_files(&path, out);
                                    }
                                } else if path.extension().map_or(false, |e| e == "jn") {
                                    out.push(path);
                                }
                            }
                        }
                    }
                    let mut found = Vec::new();
                    collect_jinn_files(std::path::Path::new("."), &mut found);
                    found
                } else {
                    files
                };
                for path in &targets {
                    match fs::read_to_string(path) {
                        Ok(src) => match crate::fmt::format_source(&src) {
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
            Cmd::Bind { header } => match crate::bind::bind_header(&header) {
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
        .with_file(Symbol::intern(&input.display().to_string()))
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

    // Load project.jn if present
    let project_jinn = base_dir.join("project.jn");
    let project_config = if project_jinn.exists() {
        Some(
            ProjectConfig::from_file(&project_jinn)
                .unwrap_or_else(|e| die(&format!("project.jn: {e}"))),
        )
    } else {
        None
    };

    let packages = load_packages(base_dir);

    resolve_modules(&mut prog, base_dir, &mut loaded, &packages);
    let entity_index = EntityIndex::build(base_dir, &packages);
    resolve_implicit_imports(&mut prog, base_dir, &mut loaded, &packages, &entity_index);

    // P0-9: enforce that an executable program has a `*main` entry point.
    if !cli.lib && !cli.test && !cli.standalone {
        let has_main = prog
            .decls
            .iter()
            .any(|d| matches!(d, crate::ast::Decl::Fn(f) if f.name == "main"));
        if !has_main {
            die(&format!(
                "{}: program has no `*main` function (use `--lib` to compile as a library or `--standalone` for freestanding mode)",
                input.display()
            ));
        }
    }

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
        let iface = crate::interface::InterfaceFile::from_decls(mod_name, &prog.decls);
        let iface_path = input.with_extension("jni");
        if let Err(e) = iface.write_to(&iface_path) {
            die(&format!("interface: {e}"));
        }
    }

    if cli.emit_hir {
        print!("{}", crate::hir::pretty_print(&hir_prog));
        return;
    }

    let hir_errors = crate::hir_validate::HirValidator::validate(&hir_prog);
    for e in &hir_errors {
        eprintln!("hir-validate: {e}");
    }
    if !hir_errors.is_empty() {
        die("compilation aborted due to HIR validation errors");
    }

    crate::comptime::fold_program(&mut hir_prog);

    // ── R11: HIR-level Perceus has been retired; the MIR-level Perceus
    // pipeline below is now the sole source of refcount transformations and
    // statistics. `--debug-perceus` prints the MIR-level stats line, which
    // is emitted unconditionally when the flag is set (see below).

    let mut verifier = OwnershipVerifier::new();
    let diags = verifier.verify(&hir_prog);
    let mut has_hard_error = false;
    for d in &diags {
        let level = match d.kind {
            crate::ownership::DiagKind::UseAfterMove => {
                has_hard_error = true;
                "error"
            }
            crate::ownership::DiagKind::DoubleMutableBorrow => {
                has_hard_error = true;
                "error"
            }
            crate::ownership::DiagKind::MoveOfBorrowed => {
                has_hard_error = true;
                "error"
            }
            crate::ownership::DiagKind::InvalidRcDeref => {
                has_hard_error = true;
                "error"
            }
            crate::ownership::DiagKind::ReturnOfBorrowed => {
                has_hard_error = true;
                "error"
            }
            crate::ownership::DiagKind::Warning => "warning",
        };
        eprintln!("ownership: {} (line {}): {}", level, d.span.line, d.message);
    }
    if has_hard_error {
        die("compilation aborted due to ownership errors");
    }

    // ── MIR pass: HIR → MIR → optimize → (optional print) ──
    let mir_opt_level = match cli.opt {
        0 => crate::mir::opt::OptLevel::None,
        1 => crate::mir::opt::OptLevel::Basic,
        _ => crate::mir::opt::OptLevel::Full,
    };
    let mut mir_prog = crate::mir::lower::lower_program(&hir_prog);
    for func in &mut mir_prog.functions {
        crate::mir::opt::optimize(func, mir_opt_level);
    }

    // ── Strict-types: reject FnRef to polymorphic functions that aren't called ──
    if cli.strict_types {
        use crate::mir::{InstKind, Terminator};
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
        print!("{}", crate::mir::printer::print_program(&mir_prog));
        return;
    }

    // ── Incremental compilation: check cache status ──
    if cli.incremental {
        let incr_cache = crate::incr::ArtifactCache::new();
        let (dirty, _keys) = crate::incr::compute_dirty_set(&hir_prog, &incr_cache);
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
        use crate::perceus::mir_perceus;
        comp.tune_empty_vec_growth_floor_from_mir(&mir_prog);
        let mir_hints = mir_perceus::run(&mut mir_prog);
        if cli.debug_perceus
            || mir_hints.stats.drops_elided > 0
            || mir_hints.stats.reuse_sites > 0
            || mir_hints.stats.drops_fused > 0
            || mir_hints.stats.last_use_tracked > 0
        {
            eprintln!(
                "mir-perceus: {} drops elided, {} drops sunk, {} drops fused, {} reuse pairs ({} bindings)",
                mir_hints.stats.drops_elided,
                mir_hints.stats.last_use_tracked,
                mir_hints.stats.drops_fused,
                mir_hints.stats.reuse_sites,
                mir_hints.stats.total_bindings_analyzed,
            );
        }
        if let Err(e) = comp.compile_program(&mir_prog, &hir_prog, mir_hints) {
            die(&strip_codegen_prefix(&e.to_string()));
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
        let rt_dir = env!("JINN_RT_DIR");
        cc.arg("-L").arg(rt_dir).arg("-ljinn_rt").arg("-lpthread");
    }
    if comp.needs_ssl {
        if env!("JINN_HAS_SSL") != "1" {
            die(
                "program uses std.tls or std.crypto but OpenSSL was not available when the compiler was built",
            );
        }
        let rt_dir = env!("JINN_RT_DIR");
        cc.arg("-L")
            .arg(rt_dir)
            .arg("-ljinn_ssl")
            .arg("-lssl")
            .arg("-lcrypto");
    }
    if comp.needs_sqlite {
        if env!("JINN_HAS_SQLITE") != "1" {
            die(
                "program uses std.sqlite but SQLite3 was not available when the compiler was built",
            );
        }
        let rt_dir = env!("JINN_RT_DIR");
        cc.arg("-L")
            .arg(rt_dir)
            .arg("-ljinn_sqlite")
            .arg("-lsqlite3");
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
