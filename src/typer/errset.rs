use crate::ast::{self, Span};
use crate::intern::Symbol;
use crate::types::Type;

use super::Typer;

impl Typer {
    pub(in crate::typer) fn seed_inferred_fallibility(&mut self, fns: &[&ast::Fn]) {
        loop {
            let mut changed = false;
            for f in fns {
                if !f.error_types.is_empty() || f.name.as_str() == "main" {
                    continue;
                }
                let mut errs: std::collections::BTreeSet<Symbol> =
                    std::collections::BTreeSet::new();
                for s in &f.body {
                    self.scan_stmt_for_errors(s, &mut errs);
                }
                if errs.len() != 1 {
                    continue;
                }
                let err_enum = *errs.iter().next().unwrap();
                let Some((id, ptys, ret)) = self.fns.get(&f.name).cloned() else {
                    continue;
                };
                if matches!(&ret, Type::Struct(n, _) if n.as_str() == "Result") {
                    continue;
                }
                let resolved_ret = self.infer_ctx.resolve(&ret);
                if matches!(&resolved_ret, Type::Enum(n) if self.is_result_enum(*n)) {
                    continue;
                }
                let ok_ty = match &resolved_ret {
                    Type::TypeVar(_) => Type::Void,
                    other => other.clone(),
                };
                let mut tm = std::collections::HashMap::new();
                tm.insert(Symbol::intern("T"), ok_ty);
                tm.insert(Symbol::intern("E"), Type::Enum(err_enum));
                let Ok(mono) = self.monomorphize_enum("Result", &tm) else {
                    continue;
                };
                if matches!(&ret, Type::Enum(n) if *n == mono) {
                    continue;
                }
                self.fns.insert(f.name, (id, ptys, Type::Enum(mono)));
                changed = true;
            }
            if !changed {
                break;
            }
        }
    }

    fn err_enum_of_variant_expr(&self, e: &ast::Expr) -> Option<Symbol> {
        let variant = match e {
            ast::Expr::Ident(n, _) => *n,
            ast::Expr::QualifiedIdent(_, v, _) => *v,
            _ => return None,
        };
        let (en, _) = self.variant_tags.get(&variant)?;
        if self.err_enum_names.contains(en) {
            Some(*en)
        } else {
            None
        }
    }

    fn err_enum_of_callee(&self, name: Symbol) -> Option<Symbol> {
        let (_, _, ret) = self.fns.get(&name)?;
        let args = match ret {
            Type::Struct(n, a) if n.as_str() == "Result" => a,
            _ => return None,
        };
        match args.get(1) {
            Some(Type::Enum(en)) | Some(Type::Struct(en, _))
                if self.err_enum_names.contains(en) =>
            {
                Some(*en)
            }
            _ => None,
        }
    }

    fn scan_expr_for_errors(&self, e: &ast::Expr, errs: &mut std::collections::BTreeSet<Symbol>) {
        if let ast::Expr::Call(callee, args, _) = e {
            if let ast::Expr::Ident(name, _) = callee.as_ref()
                && let Some(en) = self.err_enum_of_callee(*name)
            {
                errs.insert(en);
            }
            self.scan_expr_for_errors(callee, errs);
            for a in args {
                self.scan_expr_for_errors(a, errs);
            }
        }
    }

    fn scan_stmt_for_errors(&self, s: &ast::Stmt, errs: &mut std::collections::BTreeSet<Symbol>) {
        match s {
            ast::Stmt::ErrReturn(e, _) => {
                if let Some(en) = self.err_enum_of_variant_expr(e) {
                    errs.insert(en);
                }
                self.scan_expr_for_errors(e, errs);
            }
            ast::Stmt::Bind(b) => self.scan_expr_for_errors(&b.value, errs),
            ast::Stmt::Expr(e) => self.scan_expr_for_errors(e, errs),
            ast::Stmt::Ret(Some(e), _) => self.scan_expr_for_errors(e, errs),
            ast::Stmt::If(i) => {
                for st in &i.then {
                    self.scan_stmt_for_errors(st, errs);
                }
                for (_, blk) in &i.elifs {
                    for st in blk {
                        self.scan_stmt_for_errors(st, errs);
                    }
                }
                if let Some(els) = &i.els {
                    for st in els {
                        self.scan_stmt_for_errors(st, errs);
                    }
                }
            }
            ast::Stmt::While(w) => {
                for st in &w.body {
                    self.scan_stmt_for_errors(st, errs);
                }
            }
            ast::Stmt::For(fr) => {
                for st in &fr.body {
                    self.scan_stmt_for_errors(st, errs);
                }
            }
            ast::Stmt::Loop(l) => {
                for st in &l.body {
                    self.scan_stmt_for_errors(st, errs);
                }
            }
            ast::Stmt::Match(m) => {
                for arm in &m.arms {
                    for st in &arm.body {
                        self.scan_stmt_for_errors(st, errs);
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn has_from_conversion(&self, src: Symbol, tgt: Symbol) -> bool {
        if src == tgt {
            return true;
        }
        let from_name: Symbol = format!("{tgt}_from_{src}").into();
        self.fns.contains_key(&from_name)
    }

    pub(crate) fn converts_into_union(&self, src: Symbol, union: &[Symbol]) -> Vec<Symbol> {
        union
            .iter()
            .copied()
            .filter(|tgt| self.has_from_conversion(src, *tgt))
            .collect()
    }

    pub(crate) fn check_error_soundness(
        &self,
        fn_name: Symbol,
        inferred: &[Symbol],
        declared: &[Symbol],
        span: Span,
    ) -> Result<(), String> {
        if declared.is_empty() {
            return Ok(());
        }
        for src in inferred {
            let targets = self.converts_into_union(*src, declared);
            if targets.is_empty() {
                let union = declared
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ");
                return Err(format!(
                    "function `{fn_name}` at {span:?} declares error union `! {union}` but may produce `{src}`, and no conversion `{src} -> {union}` exists. Add `| {src}` to the union, or add `impl From of {src} for <one of {union}>` with `*from(e as {src}) returns <that type>`."
                ));
            }
            if targets.len() > 1 && !declared.contains(src) {
                let opts = targets
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "ambiguous error conversion for `{src}` at {span:?} in function `{fn_name}`: it can convert into several declared error types ({opts}). Make the union unambiguous so only one target receives `{src}`."
                ));
            }
        }
        Ok(())
    }
}
