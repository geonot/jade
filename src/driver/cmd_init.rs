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

use super::cli::*;
use super::project::*;

pub(super) fn cmd_init(name: Option<String>) {
    let pkg_name = name.unwrap_or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "myproject".into())
    });
    let project_path = PathBuf::from("project.jn");
    if project_path.exists() {
        die("project.jn already exists");
    }
    let project_content = format!(
        "name is '{}'\nversion is '0.1.0'\nentry is 'source/main.jn'\n",
        pkg_name
    );
    fs::write(&project_path, &project_content)
        .unwrap_or_else(|e| die(&format!("cannot write project.jn: {e}")));

    // Create source directory and main.jn
    let source_dir = PathBuf::from("source");
    if !source_dir.exists() {
        fs::create_dir_all(&source_dir)
            .unwrap_or_else(|e| die(&format!("cannot create source/: {e}")));
    }
    let main_path = source_dir.join("main.jn");
    if !main_path.exists() {
        fs::write(&main_path, "*main\n    log('hello world')\n")
            .unwrap_or_else(|e| die(&format!("cannot write source/main.jn: {e}")));
    }
    println!("initialized project '{pkg_name}'");
}
