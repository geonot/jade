use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use clap::{Parser as ClapParser, Subcommand};
use inkwell::OptimizationLevel;
use inkwell::context::Context;

use crate::ast::{Decl, Program, Stmt};
use crate::cache::{Cache, build_package_map};
use crate::codegen::Compiler;
use crate::intern::Symbol;
use crate::lexer::Lexer;
use crate::lock::Lockfile;
use crate::ownership::OwnershipVerifier;
use crate::parser::Parser;
use crate::perceus::PerceusPass;
use crate::pkg::{Dependency, Package, SemVer};
use crate::resolve::prefix_module;
use crate::typer::Typer;

use super::cli::strip_codegen_prefix;
use super::cli::*;
use super::project::*;
use super::sources::{
    EntityIndex, load_packages, merge_source_files, resolve_implicit_imports, resolve_modules,
};

pub(super) fn compile_and_link(
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

    // Multi-file project: merge all .jn files from the source directory
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

    crate::comptime::fold_program(&mut hir_prog);

    let mut perceus = PerceusPass::new();
    let _hir_hints = perceus.optimize(&hir_prog);

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
    let mir_opt_level = match opt_level {
        0 => crate::mir::opt::OptLevel::None,
        1 => crate::mir::opt::OptLevel::Basic,
        _ => crate::mir::opt::OptLevel::Full,
    };
    let mut mir_prog = crate::mir::lower::lower_program(&hir_prog);
    for func in &mut mir_prog.functions {
        crate::mir::opt::optimize(func, mir_opt_level);
    }
    if emit_mir {
        print!("{}", crate::mir::printer::print_program(&mir_prog));
    }

    // ── Incremental compilation: check and report cache status ──
    if incremental {
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
    if fast_math {
        comp.set_fast_math(true);
    }
    if deterministic_fp {
        comp.set_deterministic_fp();
    }
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
        use crate::perceus::mir_perceus;
        comp.tune_empty_vec_growth_floor_from_mir(&mir_prog);
        let mir_hints = mir_perceus::run(&mut mir_prog);
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
