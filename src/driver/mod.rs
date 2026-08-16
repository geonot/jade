mod cli;
mod cmd_init;
mod cmd_pkg;
mod pipeline;
mod project;
mod sources;

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use clap::Parser as ClapParser;
use inkwell::OptimizationLevel;
use inkwell::context::Context;

use crate::codegen::Compiler;
use crate::intern::Symbol;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::typer::Typer;

use cli::{Cli, Cmd, die, dirs_cache, strip_codegen_prefix};
use cmd_init::cmd_init;
use cmd_pkg::{cmd_fetch, cmd_package, cmd_publish, cmd_update};
use pipeline::compile_and_link;
use project::ProjectConfig;
pub(crate) use sources::resolve_modules;
use sources::{find_project_entry, load_packages};

fn init_tracing(cli: &Cli) {
    use tracing_subscriber::EnvFilter;

    let mut filter = if cli.debug {
        EnvFilter::new("jinnc=debug")
    } else if cli.verbose {
        EnvFilter::new("jinnc=info")
    } else {
        EnvFilter::new("warn")
    };
    if cli.debug_types {
        filter = filter.add_directive("jinnc::type=trace".parse().unwrap());
    }
    if cli.debug_drops {
        filter = filter.add_directive("jinnc::drops=trace".parse().unwrap());
    }
    if let Ok(env) = std::env::var("JINN_LOG")
        && let Ok(extra) = EnvFilter::try_new(env)
    {
        filter = extra;
    }

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(std::io::stderr)
        .without_time()
        .with_level(false)
        .try_init();
}

pub fn run() {
    let mut cli = Cli::parse();
    init_tracing(&cli);

    if let Some(Cmd::Compile {
        input,
        output,
        emit_ir,
        emit_llvm,
        emit_hir,
        emit_mir,
        emit_obj,
        opt,
        lto,
        lib,
        link,
        debug,
        test,
        emit_interface,
        dump_tokens,
        dump_ast,
        standalone,
        target,
        cpu,
        features,
    }) = cli.command
    {
        cli.input = Some(input);
        cli.output = output;
        cli.emit_ir = emit_ir;
        cli.emit_llvm = emit_llvm;
        cli.emit_hir = emit_hir;
        cli.emit_mir = emit_mir;
        cli.emit_obj = emit_obj;
        cli.opt = opt;
        cli.lto = lto;
        cli.lib = lib;
        cli.link = link;
        cli.debug = debug;
        cli.test = test;
        cli.emit_interface = emit_interface;
        cli.dump_tokens = dump_tokens;
        cli.dump_ast = dump_ast;
        cli.standalone = standalone;
        cli.target = target.or(cli.target);
        cli.cpu = cpu.or(cli.cpu);
        cli.features = features.or(cli.features);
        cli.command = None;
    }

    if let Some(cmd) = cli.command {
        match cmd {
            Cmd::Compile { .. } => unreachable!(),
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
                    cli.emit_mir,
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
                    for (path, bytes) in collect_sibling_sources(&entry) {
                        path.hash(&mut h);
                        bytes.hash(&mut h);
                    }
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
                        false,
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
                    false,
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

                let mut loaded: HashSet<Symbol> = HashSet::new();
                if let Ok(canon) = entry.canonicalize() {
                    loaded.insert(Symbol::intern(&canon.to_string_lossy()));
                }
                let packages = load_packages(base_dir);
                let mut std_files: HashSet<Symbol> = HashSet::new();
                resolve_modules(&mut prog, base_dir, &mut loaded, &packages, &mut std_files)
                    .unwrap_or_else(|e| die(&e));
                let mut typer = Typer::new();
                typer.set_source_dir(base_dir.to_path_buf());
                typer.set_std_files(std_files);

                match typer.lower_program(&prog) {
                    Ok(mut hir_prog) => {
                        let hir_errors = crate::hir_validate::HirValidator::validate(&hir_prog);
                        for e in &hir_errors {
                            eprintln!("{e}");
                        }
                        if !hir_errors.is_empty() {
                            die("check failed: HIR validation errors");
                        }
                        crate::comptime::fold_program(&mut hir_prog);
                        println!("check passed");
                    }
                    Err(e) => die(&format!("type error: {e}")),
                }
            }
            Cmd::Fmt { files, write } => {
                let targets: Vec<PathBuf> = if files.is_empty() {
                    fn collect_jinn_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
                        if let Ok(entries) = fs::read_dir(dir) {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                if path.is_dir() {
                                    let name = path.file_name().unwrap_or_default();
                                    if name != "target" && name != ".git" {
                                        collect_jinn_files(&path, out);
                                    }
                                } else if path.extension().is_some_and(|e| e == "jn") {
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
                let mut failed = false;
                for path in &targets {
                    match fs::read_to_string(path) {
                        Ok(src) => match crate::fmt::format_source(&src) {
                            Ok(formatted) => {
                                if write {
                                    if formatted != src {
                                        if let Err(e) = crate::fmt::format_source(&formatted) {
                                            eprintln!(
                                                "refusing to write {}: the formatted output no \
                                                 longer parses ({e}); this is a formatter bug — \
                                                 the file is unchanged",
                                                path.display()
                                            );
                                            failed = true;
                                        } else {
                                            fs::write(path, &formatted).unwrap_or_else(|e| {
                                                eprintln!("cannot write {}: {e}", path.display());
                                                failed = true;
                                            });
                                            println!("formatted {}", path.display());
                                        }
                                    }
                                } else {
                                    print!("{formatted}");
                                }
                            }
                            Err(e) => {
                                eprintln!("cannot format {}: {e}", path.display());
                                failed = true;
                            }
                        },
                        Err(e) => {
                            eprintln!("cannot read {}: {e}", path.display());
                            failed = true;
                        }
                    }
                }
                if failed {
                    std::process::exit(1);
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

    let input = resolve_project_input(input);
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

    if let Ok(canon) = input.canonicalize() {
        loaded.insert(Symbol::intern(&canon.to_string_lossy()));
    }

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

    let mut std_files: HashSet<Symbol> = HashSet::new();
    resolve_modules(&mut prog, base_dir, &mut loaded, &packages, &mut std_files)
        .unwrap_or_else(|e| die(&e));

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
    typer.set_std_files(std_files);
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
        Err(e) => die(&e),
    };

    if cli.lib
        && !cli.emit_hir
        && !cli.emit_llvm
        && !cli.emit_ir
        && !cli.emit_mir
        && !cli.emit_interface
    {
        let unresolved = typer.unresolved_exported_generics();
        if !unresolved.is_empty() {
            for f in &unresolved {
                eprintln!(
                    "error: exported function `{f}` has unannotated parameters whose types cannot be inferred without a call site; annotate them (e.g. `v as Vec of i64`) or declare a trait bound"
                );
            }
            die("library compile failed: unresolvable exported generics");
        }
        for w in typer.boundary_ownership_warnings(&prog) {
            eprintln!("{w}");
        }
    }

    if cli.emit_interface {
        let mod_name = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("module");
        let mut iface = crate::interface::InterfaceFile::from_decls(mod_name, &prog.decls);
        iface.annotate_ownership(&typer.fn_param_access, &typer.fn_param_mutates);
        let iface_path = input.with_extension("jni");
        if let Err(e) = iface.write_to(&iface_path) {
            die(&format!("interface: {e}"));
        }
    }

    if cli.emit_hir {
        print!("{}", crate::hir::pretty_print(&hir_prog));
        let hir_errors = crate::hir_validate::HirValidator::validate(&hir_prog);
        for e in &hir_errors {
            eprintln!("{e}");
        }
        if !hir_errors.is_empty() {
            die("compilation aborted due to HIR validation errors");
        }
        return;
    }

    let hir_errors = crate::hir_validate::HirValidator::validate(&hir_prog);
    for e in &hir_errors {
        eprintln!("{e}");
    }
    if !hir_errors.is_empty() {
        die("compilation aborted due to HIR validation errors");
    }

    crate::comptime::fold_program(&mut hir_prog);

    let mir_opt_level = match cli.opt {
        0 => crate::mir::opt::OptLevel::None,
        1 => crate::mir::opt::OptLevel::Basic,
        _ => crate::mir::opt::OptLevel::Full,
    };
    let mut mir_prog = crate::mir::lower::lower_program(&hir_prog);
    if pipeline::mir_verify_enabled()
        && let Err(errs) = crate::mir::verify::verify_program(&mir_prog)
    {
        for e in &errs {
            eprintln!("MIR verify (post-lower): {e}");
        }
        if std::env::var("JINN_MIR_VERIFY_SOFT").is_err() {
            die("MIR verification failed after lowering — this is a compiler bug");
        }
    }
    for func in &mut mir_prog.functions {
        crate::mir::opt::optimize(func, mir_opt_level);
    }
    if pipeline::mir_verify_enabled()
        && let Err(errs) = crate::mir::verify::verify_program(&mir_prog)
    {
        for e in &errs {
            eprintln!("MIR verify (post-opt): {e}");
        }
        die("MIR verification failed after optimization — this is a compiler bug");
    }

    if cli.strict_types {
        use crate::mir::{InstKind, Terminator};
        let _fn_names: std::collections::HashSet<Symbol> =
            mir_prog.functions.iter().map(|f| f.name).collect();
        for func in &mir_prog.functions {
            for bb in &func.blocks {
                for inst in &bb.insts {
                    if let InstKind::FnRef(ref name) = inst.kind
                        && let Some(dest) = inst.dest
                        && func.name == "main"
                        && matches!(bb.terminator, Terminator::Return(Some(v)) if v == dest)
                    {
                        die(&format!(
                            "codegen: bare function reference `{name}` has unresolved return type in main"
                        ));
                    }
                }
            }
        }
    }

    if cli.emit_mir {
        print!("{}", crate::mir::printer::print_program(&mir_prog));
        return;
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
        use crate::drops::mir_drops;
        comp.tune_empty_vec_growth_floor_from_mir(&mir_prog);
        let consuming: crate::drops::ConsumingMap = typer
            .fn_param_access
            .iter()
            .map(|(name, accs)| {
                (
                    *name,
                    accs.iter()
                        .map(|a| matches!(a, Some(crate::ast::AccessMod::Take)))
                        .collect(),
                )
            })
            .collect();
        let mir_hints = mir_drops::run(&mut mir_prog, &consuming).unwrap_or_else(|errors| {
            for e in errors {
                eprintln!("MIR drop verify: {e}");
            }
            die("MIR drop-linearity verification failed — this is a compiler bug");
        });
        if cli.debug_drops {
            eprintln!(
                "mir-drops: {} drops elided, {} drops sunk, {} drops fused, {} reuse pairs ({} bindings)",
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
    if comp.needs_pcre2 {
        if env!("JINN_HAS_PCRE2") != "1" {
            die("program uses std.regex but PCRE2 was not available when the compiler was built");
        }
        cc.arg("-lpcre2-8");
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

    if let Some(ref triple) = comp.target_triple {
        cc.arg(format!("--target={triple}"));
        if triple.contains("wasm") {
            cc = Command::new("clang");
            cc.arg(format!("--target={triple}"));
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

fn resolve_project_input(input: PathBuf) -> PathBuf {
    let project_jinn = if input.is_dir() {
        input.join("project.jn")
    } else if input.file_name().and_then(|n| n.to_str()) == Some("project.jn") {
        input.clone()
    } else {
        return input;
    };
    if !project_jinn.exists() {
        die(&format!(
            "no project.jn at {} (pass a .jn source file instead, or run `jinnc init`)",
            project_jinn.display()
        ));
    }
    let cfg = ProjectConfig::from_file(&project_jinn)
        .unwrap_or_else(|e| die(&format!("project.jn: {e}")));
    let project_dir = project_jinn.parent().unwrap_or(std::path::Path::new("."));
    if let Some(entry) = cfg.entry {
        let entry_path = project_dir.join(&entry);
        if !entry_path.exists() {
            die(&format!(
                "entry file not found: {} (declared in {})",
                entry_path.display(),
                project_jinn.display()
            ));
        }
        return entry_path;
    }
    for candidate in [
        project_dir.join("source").join("main.jn"),
        project_dir.join("src").join("main.jn"),
    ] {
        if candidate.exists() {
            return candidate;
        }
    }
    die(&format!(
        "{} has no `entry is …` declaration and no source/main.jn or src/main.jn fallback",
        project_jinn.display()
    ));
}

fn collect_sibling_sources(entry: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let root = match entry.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut stack = vec![root];
    let mut visited = 0usize;
    while let Some(dir) = stack.pop() {
        if visited > 4096 {
            break;
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for ent in rd.flatten() {
            let path = ent.path();
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("jn") {
                visited += 1;
                if let Ok(bytes) = fs::read(&path) {
                    out.push((path.display().to_string(), bytes));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
