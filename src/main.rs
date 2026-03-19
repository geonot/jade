use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use clap::Parser as ClapParser;
use inkwell::OptimizationLevel;
use inkwell::context::Context;

use jadec::ast::{Decl, Program};
use jadec::codegen::Compiler;
use jadec::lexer::Lexer;
use jadec::ownership::OwnershipVerifier;
use jadec::parser::Parser;
use jadec::perceus::PerceusPass;
use jadec::typer::Typer;

#[derive(ClapParser)]
#[command(name = "jadec", version = "0.0.0", about = "The Jade compiler")]
struct Cli {
    input: PathBuf,
    #[arg(short, long, default_value = "a.out")]
    output: PathBuf,
    #[arg(long)]
    emit_ir: bool,
    #[arg(long, default_value = "3")]
    opt: u8,
    #[arg(long)]
    lto: bool,
}

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn resolve_modules(prog: &mut Program, base_dir: &std::path::Path, loaded: &mut HashSet<String>) {
    let uses: Vec<Vec<String>> = prog
        .decls
        .iter()
        .filter_map(|d| {
            if let Decl::Use(u) = d {
                Some(u.path.clone())
            } else {
                None
            }
        })
        .collect();
    for path in uses {
        let key = path.join(".");
        if loaded.contains(&key) {
            continue;
        }
        loaded.insert(key.clone());
        let file_path = path.join("/");
        let mut candidate = base_dir.join(format!("{file_path}.jade"));
        if !candidate.exists() {
            candidate = base_dir
                .join("std")
                .join(format!("{}.jade", path.last().unwrap()));
        }
        if !candidate.exists() {
            die(&format!("module not found: {key}"));
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
        );
        for d in mod_prog.decls {
            if !matches!(d, Decl::Use(_)) {
                prog.decls.push(d);
            }
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let src = fs::read_to_string(&cli.input)
        .unwrap_or_else(|e| die(&format!("cannot read {}: {e}", cli.input.display())));
    let tokens = Lexer::new(&src)
        .tokenize()
        .unwrap_or_else(|e| die(&format!("{e}")));
    let mut prog = Parser::new(tokens)
        .parse_program()
        .unwrap_or_else(|e| die(&format!("{e}")));

    let base_dir = cli
        .input
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut loaded = HashSet::new();
    resolve_modules(&mut prog, base_dir, &mut loaded);

    // ── HIR: type check → Perceus optimization → ownership verification ──
    // Runs alongside existing codegen; does not block compilation.
    {
        let mut typer = Typer::new();
        match typer.lower_program(&prog) {
            Ok(hir_prog) => {
                // Phase C: Perceus optimizations
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

                // Ownership verification
                let mut verifier = OwnershipVerifier::new();
                let diags = verifier.verify(&hir_prog);
                for d in &diags {
                    eprintln!(
                        "ownership: {} (line {}): {}",
                        match d.kind {
                            jadec::ownership::DiagKind::UseAfterMove => "error",
                            jadec::ownership::DiagKind::DoubleMutableBorrow => "error",
                            jadec::ownership::DiagKind::MoveOfBorrowed => "error",
                            jadec::ownership::DiagKind::InvalidRcDeref => "error",
                            jadec::ownership::DiagKind::ReturnOfBorrowed => "error",
                            jadec::ownership::DiagKind::Warning => "warning",
                        },
                        d.span.line,
                        d.message
                    );
                }
            }
            Err(e) => {
                eprintln!("hir: {e}");
            }
        }
    }

    let ctx = Context::create();
    let name = cli
        .input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "main".into());
    let mut comp = Compiler::new(&ctx, &name);
    comp.set_source(&src);
    if let Err(e) = comp.compile_program(&prog) {
        die(&format!("codegen: {e}"));
    }

    if cli.emit_ir {
        println!("{}", comp.emit_ir());
        return;
    }

    let opt = match cli.opt {
        0 => OptimizationLevel::None,
        1 => OptimizationLevel::Less,
        2 => OptimizationLevel::Default,
        3 => OptimizationLevel::Aggressive,
        _ => die("opt must be 0-3"),
    };
    let obj = cli.output.with_extension("o");
    if let Err(e) = comp.emit_object(&obj, opt) {
        die(&format!("emit: {e}"));
    }

    let mut cc = Command::new("cc");
    cc.arg(&obj).arg("-o").arg(&cli.output).arg("-lm");
    if cli.lto {
        cc.arg("-flto");
    }
    let status = cc.status();
    let _ = fs::remove_file(&obj);
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => die(&format!("linker failed: {}", s.code().unwrap_or(-1))),
        Err(e) => die(&format!("linker: {e}")),
    }
}
