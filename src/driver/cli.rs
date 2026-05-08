// Compiler driver / CLI entry point. See `docs/architecture.md` for the pipeline overview.
// (Regular comment, not a `//!` doc comment, because this file is `include!`d by `src/bin/jinn.rs`.)

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

#[derive(ClapParser)]
#[command(name = "jinnc", version = "0.0.0", about = "The Jinn compiler")]
pub(super) struct Cli {
    #[command(subcommand)]
    pub(super) command: Option<Cmd>,

    pub(super) input: Option<PathBuf>,
    #[arg(short, long, default_value = "a.out")]
    pub(super) output: PathBuf,
    #[arg(long, hide = true)]
    pub(super) emit_ir: bool,
    #[arg(long)]
    pub(super) emit_llvm: bool,
    #[arg(long)]
    pub(super) emit_hir: bool,
    #[arg(long)]
    pub(super) emit_mir: bool,
    #[arg(long)]
    pub(super) emit_obj: bool,
    #[arg(long, default_value = "3")]
    pub(super) opt: u8,
    #[arg(long)]
    pub(super) lto: bool,
    #[arg(long)]
    pub(super) lib: bool,
    #[arg(long)]
    pub(super) link: Vec<PathBuf>,
    #[arg(long)]
    pub(super) debug: bool,
    #[arg(long)]
    pub(super) debug_types: bool,
    #[arg(long)]
    pub(super) debug_perceus: bool,
    #[arg(long, default_value_t = true)]
    pub(super) warn_inferred_defaults: bool,
    #[arg(long)]
    pub(super) no_warn_inferred_defaults: bool,
    #[arg(long)]
    pub(super) strict_types: bool,
    #[arg(long)]
    pub(super) lenient: bool,
    #[arg(long)]
    pub(super) pedantic: bool,
    #[arg(long)]
    pub(super) test: bool,
    #[arg(long)]
    pub(super) emit_interface: bool,
    #[arg(long)]
    pub(super) dump_tokens: bool,
    #[arg(long)]
    pub(super) dump_ast: bool,
    /// Enable fast-math optimizations (nnan, ninf, nsz, arcp, contract, afn, reassoc)
    #[arg(long)]
    pub(super) fast_math: bool,
    /// Guarantee deterministic floating-point results (disable FP reordering)
    #[arg(long)]
    pub(super) deterministic_fp: bool,
    /// Enable incremental compilation (cache unchanged function artifacts)
    #[arg(long, hide = true)]
    pub(super) incremental: bool,
    /// Number of parallel codegen threads (0 = auto-detect)
    #[arg(long, default_value = "0")]
    pub(super) threads: usize,
    /// Target triple for cross-compilation (e.g., aarch64-unknown-linux-gnu)
    #[arg(long)]
    pub(super) target: Option<String>,
    /// Target CPU name (e.g., cortex-a53, skylake)
    #[arg(long)]
    pub(super) cpu: Option<String>,
    /// Target CPU features (e.g., +avx2,+sse4.2)
    #[arg(long)]
    pub(super) features: Option<String>,
    /// Standalone mode: no runtime, no libc dependency
    #[arg(long)]
    pub(super) standalone: bool,
}

#[derive(Subcommand)]
pub(super) enum Cmd {
    Init {
        name: Option<String>,
    },
    Fetch,
    Update,
    /// Compile the project (uses project.jn entry if available)
    Build {
        #[arg(short, long, default_value = "a.out")]
        output: Option<PathBuf>,
        #[arg(long)]
        opt: Option<u8>,
        #[arg(long)]
        lto: bool,
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
    },
    /// Generate jinn.pkg and (by default) a source archive in dist/
    Package {
        /// Output manifest path (default: ./jinn.pkg)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Generate only jinn.pkg (skip dist/*.tar.gz creation)
        #[arg(long)]
        no_archive: bool,
    },
    /// Tag the current git repo as v<version> for dependency publishing
    Publish {
        /// Also push the created tag to remote (default remote: origin)
        #[arg(long)]
        push: bool,
        /// Git remote to push to (default: origin)
        #[arg(long)]
        remote: Option<String>,
        /// Force retag if tag already exists and skip clean-tree check
        #[arg(long)]
        force: bool,
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
    /// Format Jinn source files
    Fmt {
        /// Files to format (default: all .jn files in current directory)
        files: Vec<PathBuf>,
    },
    /// Generate Jinn extern declarations from a C header file
    Bind {
        /// Path to the C header file
        header: PathBuf,
    },
}

pub(super) fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

pub(super) fn dirs_cache() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("jinn")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache").join("jinn")
    } else {
        PathBuf::from(".jinn_cache")
    }
}

pub(super) fn find_project_root(start_dir: &std::path::Path) -> Option<PathBuf> {
    let mut dir = start_dir.to_path_buf();
    loop {
        if dir.join("project.jn").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}
