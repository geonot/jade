use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use clap::{Parser as ClapParser, Subcommand};
use inkwell::OptimizationLevel;
use inkwell::context::Context;

use crate::ast::{Decl, Program, Stmt};
use crate::intern::Symbol;
use crate::cache::{Cache, build_package_map};
use crate::codegen::Compiler;
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


pub(super) fn cmd_fetch() {
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

pub(super) fn cmd_update() {
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

pub(super) fn project_config_to_package(cfg: ProjectConfig) -> Package {
    let name = cfg.name.unwrap_or_else(|| "unnamed".to_string());
    let version = cfg
        .version
        .as_deref()
        .and_then(|v| SemVer::parse(v).ok())
        .unwrap_or(SemVer {
            major: 0,
            minor: 1,
            patch: 0,
        });
    Package {
        name,
        version,
        author: None,
        requires: cfg.requires,
    }
}

pub(super) fn run_git(args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("git {:?}: {e}", args))?;
    if !out.status.success() {
        return Err(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub(super) fn cmd_package(output: Option<PathBuf>, no_archive: bool) {
    let project_path = PathBuf::from("project.jade");
    if !project_path.exists() {
        die("no project.jade found in current directory (run `jadec init` to create one)");
    }

    let cfg = ProjectConfig::from_file(&project_path)
        .unwrap_or_else(|e| die(&format!("project.jade: {e}")));
    let pkg = project_config_to_package(cfg);

    let pkg_out = output.unwrap_or_else(|| PathBuf::from("jade.pkg"));
    fs::write(&pkg_out, pkg.to_string_repr())
        .unwrap_or_else(|e| die(&format!("cannot write {}: {e}", pkg_out.display())));
    println!("wrote {}", pkg_out.display());

    if no_archive {
        return;
    }

    let dist_dir = PathBuf::from("dist");
    if !dist_dir.exists() {
        fs::create_dir_all(&dist_dir)
            .unwrap_or_else(|e| die(&format!("cannot create {}: {e}", dist_dir.display())));
    }
    let archive_name = format!("{}-{}.tar.gz", pkg.name, pkg.version);
    let archive_path = dist_dir.join(archive_name);

    let mut tar_args = vec![
        "-czf".to_string(),
        archive_path.to_string_lossy().to_string(),
        "project.jade".to_string(),
        pkg_out.to_string_lossy().to_string(),
    ];
    if PathBuf::from("source").is_dir() {
        tar_args.push("source".to_string());
    }
    if PathBuf::from("README.md").exists() {
        tar_args.push("README.md".to_string());
    }
    if PathBuf::from("LICENSE").exists() {
        tar_args.push("LICENSE".to_string());
    }

    let status = Command::new("tar")
        .args(tar_args.iter().map(|s| s.as_str()))
        .status()
        .unwrap_or_else(|e| die(&format!("tar invocation failed: {e}")));
    if !status.success() {
        die(&format!(
            "tar failed with exit code {}",
            status.code().unwrap_or(-1)
        ));
    }
    println!("created {}", archive_path.display());
}

pub(super) fn cmd_publish(push: bool, remote: Option<String>, force: bool) {
    let project_path = PathBuf::from("project.jade");
    if !project_path.exists() {
        die("no project.jade found in current directory (run `jadec init` to create one)");
    }

    let cfg = ProjectConfig::from_file(&project_path)
        .unwrap_or_else(|e| die(&format!("project.jade: {e}")));
    let pkg = project_config_to_package(cfg);
    let tag = format!("v{}", pkg.version);

    run_git(&["rev-parse", "--is-inside-work-tree"]).unwrap_or_else(|e| die(&e));

    if !force {
        let status_out = run_git(&["status", "--porcelain"])
            .unwrap_or_else(|e| die(&format!("cannot read git status: {e}")));
        if !status_out.trim().is_empty() {
            die("publish requires a clean git working tree (use --force to override)");
        }
    }

    let tag_exists = Command::new("git")
        .args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if tag_exists {
        if force {
            run_git(&["tag", "-d", &tag]).unwrap_or_else(|e| die(&e));
        } else {
            die(&format!("tag {tag} already exists (use --force to retag)"));
        }
    }

    run_git(&[
        "tag",
        "-a",
        &tag,
        "-m",
        &format!("jade publish {} {}", pkg.name, pkg.version),
    ])
    .unwrap_or_else(|e| die(&e));

    let remote_name = remote.unwrap_or_else(|| "origin".to_string());
    if push {
        run_git(&["push", &remote_name, &tag]).unwrap_or_else(|e| die(&e));
        println!("pushed tag {tag} to {remote_name}");
    }

    let remote_url = run_git(&["remote", "get-url", &remote_name]).ok();
    println!("published {} {} as git tag {}", pkg.name, pkg.version, tag);
    if let Some(url) = remote_url {
        println!(
            "consumer require line:\nrequire '{}' '{}' '{}'",
            pkg.name, url, pkg.version
        );
    } else {
        println!(
            "consumer require line:\nrequire '{}' '<git-url>' '{}'",
            pkg.name, pkg.version
        );
    }
}
