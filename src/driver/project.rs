use std::fs;

use crate::ast::Decl;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::pkg::{Dependency, SemVer};

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
        let tokens = Lexer::new(&src)
            .with_file(crate::intern::Symbol::intern(&path.display().to_string()))
            .tokenize()
            .map_err(|e| format!("{e}"))?;
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

                        Stmt::Expr(Expr::Call(callee, args, _))
                            if matches!(callee.as_ref(), Expr::Ident(n, _) if n == "require")
                                && args.len() == 3 =>
                        {
                            if let (Expr::Str(name, _), Expr::Str(url, _), Expr::Str(ver, _)) =
                                (&args[0], &args[1], &args[2])
                            {
                                let version = SemVer::parse(ver)
                                    .map_err(|e| format!("project.jn require: {e}"))?;
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
