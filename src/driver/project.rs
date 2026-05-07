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


#[derive(Debug, Default)]
pub(super) struct ProjectConfig {
    pub(super) name: Option<String>,
    pub(super) version: Option<String>,
    pub(super) entry: Option<String>,
    pub(super) opt: Option<u8>,
    pub(super) lto: Option<bool>,
    pub(super) requires: Vec<Dependency>,
}

impl ProjectConfig {
    pub(super) fn from_file(path: &std::path::Path) -> Result<Self, String> {
        use crate::ast::{Expr, Stmt};
        let src =
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let tokens = Lexer::new(&src).tokenize().map_err(|e| format!("{e}"))?;
        let prog = Parser::new(tokens)
            .parse_program()
            .map_err(|e| format!("{e}"))?;
        let mut cfg = ProjectConfig::default();
        for decl in &prog.decls {
            if let Decl::Fn(f) = decl {
                for stmt in &f.body {
                    match stmt {
                        Stmt::Assign(Expr::Ident(name, _), val, _) => {
                            Self::set_field(&mut cfg, &name.as_str(), val);
                        }
                        // require 'name' 'url' 'version'
                        Stmt::Expr(Expr::Call(callee, args, _))
                            if matches!(callee.as_ref(), Expr::Ident(n, _) if n == "require")
                                && args.len() == 3 =>
                        {
                            if let (Expr::Str(name, _), Expr::Str(url, _), Expr::Str(ver, _)) =
                                (&args[0], &args[1], &args[2])
                            {
                                let version = SemVer::parse(ver)
                                    .map_err(|e| format!("project.jade require: {e}"))?;
                                cfg.requires.push(Dependency {
                                    name: name.clone(),
                                    url: url.clone(),
                                    version,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // Also check top-level const bindings: `name is 'foo'`
        for decl in &prog.decls {
            if let Decl::Const(name, val, _) = decl {
                Self::set_field(&mut cfg, &name.as_str(), val);
            }
        }
        Ok(cfg)
    }

    fn set_field(cfg: &mut ProjectConfig, name: &str, val: &crate::ast::Expr) {
        use crate::ast::Expr;
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
