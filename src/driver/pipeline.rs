use std::collections::HashSet;
use std::fs;
use std::process::Command;

use inkwell::OptimizationLevel;
use inkwell::context::Context;

use crate::codegen::Compiler;
use crate::intern::Symbol;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::pkg::SemVer;
use crate::pkgid::{PackageRecord, PkgId, ScopePath, compute_semantic_hash};
use crate::typer::Typer;

use super::cli::strip_codegen_prefix;
use super::cli::*;
use super::project::ProjectConfig;
use super::sources::{
    flatten_workspace, load_packages_with_ids, resolve_modules, resolve_scoped_pkg_ids,
};

fn mir_verify_enabled() -> bool {
    match std::env::var("JINN_MIR_VERIFY").as_deref() {
        Ok("0") => false,
        Ok(_) => true,
        Err(_) => true,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_and_link(
    input: &std::path::Path,
    output: &std::path::Path,
    opt_level: u8,
    lto: bool,
    test_mode: bool,
    _bench: bool,
    emit_mir: bool,
    target: Option<&str>,
    cpu: Option<&str>,
    features: Option<&str>,
    standalone: bool,
) {
    let src = fs::read_to_string(input)
        .unwrap_or_else(|e| die(&format!("cannot read {}: {e}", input.display())));
    let file_sym = Symbol::intern(&input.display().to_string());
    let tokens = Lexer::new(&src)
        .with_file(file_sym)
        .tokenize()
        .unwrap_or_else(|e| die(&format!("{e}")));
    let mut prog = Parser::new(tokens)
        .parse_program()
        .unwrap_or_else(|e| die(&format!("{e}")));

    let base_dir = input.parent().unwrap_or(std::path::Path::new("."));
    let input_canon = input.canonicalize().unwrap_or_else(|_| input.to_path_buf());

    let mut loaded: HashSet<Symbol> = HashSet::new();
    loaded.insert(Symbol::intern(&input_canon.to_string_lossy()));
    let (packages, pkg_id_map) = load_packages_with_ids(base_dir);
    let mut std_files: HashSet<Symbol> = HashSet::new();
    resolve_modules(&mut prog, base_dir, &mut loaded, &packages, &mut std_files);

    if !standalone && !test_mode {
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

    let pkg_name = input
        .file_stem()
        .map(|s| Symbol::intern(&s.to_string_lossy()))
        .unwrap_or_else(|| Symbol::intern("main"));
    let root_pkg_id = {
        let mut all_sources: Vec<u8> = src.as_bytes().to_vec();
        for path in &loaded {
            if let Ok(extra) = std::fs::read(path.as_str()) {
                all_sources.extend_from_slice(&extra);
            }
        }
        let hash = compute_semantic_hash(
            pkg_name,
            ScopePath::root(),
            &SemVer {
                major: 0,
                minor: 0,
                patch: 0,
            },
            &all_sources,
            &[],
        );
        PkgId::intern(PackageRecord {
            name: pkg_name,
            owner_scope: ScopePath::root(),
            version: SemVer {
                major: 0,
                minor: 0,
                patch: 0,
            },
            semantic_hash: hash,
            visibility: crate::pkgid::Visibility::Public,
        })
    };

    let mut typer = Typer::new();
    typer.set_source_dir(base_dir.to_path_buf());
    typer.set_std_files(std_files);
    typer.set_root_pkg_id(root_pkg_id);
    typer.set_dep_pkg_ids(pkg_id_map);
    let scoped_use_map = {
        let root_deps = {
            let proj_jn = base_dir.join("project.jn");
            if proj_jn.exists() {
                ProjectConfig::from_file(&proj_jn)
                    .map(|c| c.requires)
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        };
        match flatten_workspace(pkg_name, base_dir, &root_deps, &packages) {
            Ok(dag) => resolve_scoped_pkg_ids(&dag),

            Err(e) if e.starts_with("multi-version coexistence of") => die(&e),
            Err(_) => Default::default(),
        }
    };
    typer.set_scoped_use_map(scoped_use_map);
    if test_mode {
        typer.set_test_mode(true);
    }
    let mut hir_prog = match typer.lower_program(&prog) {
        Ok(p) => p,
        Err(e) => die(&e),
    };

    let hir_errors = crate::hir_validate::HirValidator::validate(&hir_prog);
    for e in &hir_errors {
        eprintln!("{e}");
    }
    if !hir_errors.is_empty() {
        die("compilation aborted due to HIR validation errors");
    }

    crate::comptime::fold_program(&mut hir_prog);

    let mir_opt_level = match opt_level {
        0 => crate::mir::opt::OptLevel::None,
        1 => crate::mir::opt::OptLevel::Basic,
        _ => crate::mir::opt::OptLevel::Full,
    };
    let mut mir_prog = crate::mir::lower::lower_program(&hir_prog);
    if mir_verify_enabled()
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
    if mir_verify_enabled()
        && let Err(errs) = crate::mir::verify::verify_program(&mir_prog)
    {
        for e in &errs {
            eprintln!("MIR verify (post-opt): {e}");
        }
        die("MIR verification failed after optimization — this is a compiler bug");
    }
    if emit_mir {
        print!("{}", crate::mir::printer::print_program(&mir_prog));
    }

    let ctx = Context::create();
    let name = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "main".into());
    let mut comp = Compiler::new(&ctx, &name);
    comp.init_tbaa();
    comp.set_source(&src);
    if let Some(t) = target {
        comp.target_triple = Some(t.to_string());
    }
    if let Some(c) = cpu {
        comp.target_cpu = Some(c.to_string());
    }
    if let Some(f) = features {
        comp.target_features = Some(f.to_string());
    }
    if standalone {
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
        if let Err(e) = comp.compile_program(&mir_prog, &hir_prog, mir_hints) {
            die(&strip_codegen_prefix(&e.to_string()));
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
    if lto {
        cc.arg("-flto");
    }
    if let Some(triple) = target {
        cc.arg(format!("--target={triple}"));
        if triple.contains("wasm") {
            cc = Command::new("clang");
            cc.arg(format!("--target={triple}"));
            cc.arg(&obj).arg("-o").arg(output);
            if !standalone {
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
        Ok(s) => die(&format!("linker failed with {s}")),
        Err(e) => die(&format!("cc: {e}")),
    }
}
