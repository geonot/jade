use std::path::PathBuf;

use clap::{Parser as ClapParser, Subcommand};


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
    #[arg(short = 'v', long)]
    pub(super) verbose: bool,
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

    #[arg(long)]
    pub(super) fast_math: bool,

    #[arg(long)]
    pub(super) deterministic_fp: bool,

    #[arg(long, hide = true)]
    pub(super) incremental: bool,

    #[arg(long, default_value = "0")]
    pub(super) threads: usize,

    #[arg(long)]
    pub(super) target: Option<String>,

    #[arg(long)]
    pub(super) cpu: Option<String>,

    #[arg(long)]
    pub(super) features: Option<String>,

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

    Build {
        #[arg(short, long, default_value = "a.out")]
        output: Option<PathBuf>,
        #[arg(long)]
        opt: Option<u8>,
        #[arg(long)]
        lto: bool,

        #[arg(long)]
        target: Option<String>,

        #[arg(long)]
        cpu: Option<String>,

        #[arg(long)]
        features: Option<String>,

        #[arg(long)]
        standalone: bool,
    },

    Package {
        #[arg(short, long)]
        output: Option<PathBuf>,

        #[arg(long)]
        no_archive: bool,
    },

    Publish {
        #[arg(long)]
        push: bool,

        #[arg(long)]
        remote: Option<String>,

        #[arg(long)]
        force: bool,
    },

    Run {
        file: Option<PathBuf>,

        #[arg(last = true)]
        args: Vec<String>,
    },

    Test,

    Check,

    Fmt {
        files: Vec<PathBuf>,
    },

    Bind {
        header: PathBuf,
    },
}

pub(super) fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

pub(crate) fn strip_codegen_prefix(s: &str) -> String {
    let trimmed = s.trim_start();
    for p in &[
        "mir_codegen: ",
        "mir-codegen: ",
        "hir: ",
        "hir_validate: ",
        "typer: ",
        "codegen: ",
    ] {
        if let Some(rest) = trimmed.strip_prefix(p) {
            return rest.to_string();
        }
    }
    s.to_string()
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
