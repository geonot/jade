use crate::ast;
use crate::hir::{self, DefId, Ownership};
use crate::intern::Symbol;
use crate::types::Type;

use super::super::{Typer, VarInfo};

impl Typer {
    pub(crate) fn lower_block_no_scope(
        &mut self,
        block: &ast::Block,
        ret_ty: &Type,
    ) -> Result<hir::Block, String> {
        self.lower_block_no_scope_with_tail(block, ret_ty, None)
    }

    /// Lower a block without pushing a scope. If `tail_expected` is `Some(t)`,
    /// the final statement (when it is a bare `Expr`) is typed with `t` as
    /// the expected type, allowing literal width propagation (e.g. `42` as `u8`).
    ///
    /// `ret_ty` is the *function's* return type, used by `return` statements
    /// inside the block. It is intentionally separate from `tail_expected`:
    /// the tail of an inner `if`/`while`/`for` body is NOT the function's
    /// return value, so passing `ret_ty` as `tail_expected` for those would
    /// erroneously unify unrelated expressions with the function's ret type.
    pub(crate) fn lower_block_no_scope_with_tail(
        &mut self,
        block: &ast::Block,
        ret_ty: &Type,
        tail_expected: Option<&Type>,
    ) -> Result<hir::Block, String> {
        let deferred_snapshot = self.deferred_quantified_vars.len();
        let mut stmts = Vec::new();
        let block_len = block.len();
        for (idx, s) in block.iter().enumerate() {
            if idx == block_len - 1 {
                if let (Some(expected), crate::ast::Stmt::Expr(e)) = (tail_expected, s) {
                    let he = self.lower_expr_expected(e, Some(expected))?;
                    stmts.push(hir::Stmt::Expr(he));
                    continue;
                }
            }
            stmts.push(self.lower_stmt(s, ret_ty)?);
        }
        if self.deferred_quantified_vars.len() > deferred_snapshot {
            let vars_to_default: Vec<u32> = self
                .deferred_quantified_vars
                .drain(deferred_snapshot..)
                .collect();
            self.infer_ctx.default_quantified_vars(&vars_to_default);
        }
        Ok(stmts)
    }

    pub(crate) fn lower_if(&mut self, i: &ast::If, ret_ty: &Type) -> Result<hir::If, String> {
        let cond = self.lower_expr_expected(&i.cond, Some(&Type::Bool))?;

        let pre_if = self.snapshot_moved_fields();
        let then = self.lower_block(&i.then, ret_ty)?;
        let then_end = self.snapshot_moved_fields();
        let mut branch_ends: Vec<_> = vec![then_end];
        self.restore_moved_fields(pre_if.clone());
        let mut elifs = Vec::new();
        for (ec, eb) in &i.elifs {
            let hc = self.lower_expr_expected(ec, Some(&Type::Bool))?;
            let hb = self.lower_block(eb, ret_ty)?;
            branch_ends.push(self.snapshot_moved_fields());
            self.restore_moved_fields(pre_if.clone());
            elifs.push((hc, hb));
        }
        let els = i
            .els
            .as_ref()
            .map(|b| self.lower_block(b, ret_ty))
            .transpose()?;
        if els.is_some() {
            branch_ends.push(self.snapshot_moved_fields());
        }

        self.restore_moved_fields(pre_if);
        self.merge_moved_fields_union(&branch_ends);

        if let Some(ref else_block) = els {
            let mut common = Self::collect_block_new_binds(&then);
            for (_, elif_block) in &elifs {
                let eb = Self::collect_block_new_binds(elif_block);
                common.retain(|name, _| eb.contains_key(name));
            }
            let eb = Self::collect_block_new_binds(else_block);
            common.retain(|name, _| eb.contains_key(name));
            for (name, (def_id, ty, ownership)) in common {
                if self.find_var(&name.as_str()).is_none() {
                    self.define_var(
                        &name.as_str(),
                        VarInfo {
                            def_id,
                            ty,
                            ownership,
                            scheme: None,
                        },
                    );
                }
            }
        }

        Ok(hir::If {
            cond,
            then,
            elifs,
            els,
            span: i.span,
        })
    }

    fn collect_block_new_binds(
        block: &hir::Block,
    ) -> std::collections::HashMap<Symbol, (DefId, Type, Ownership)> {
        let mut binds = std::collections::HashMap::new();
        for stmt in block {
            if let hir::Stmt::Bind(b) = stmt {
                binds
                    .entry(b.name.clone())
                    .or_insert((b.def_id, b.ty.clone(), b.ownership));
            }
        }
        binds
    }

    pub(crate) fn lower_match(
        &mut self,
        m: &ast::Match,
        ret_ty: &Type,
    ) -> Result<hir::Match, String> {
        let subject = self.lower_expr(&m.subject)?;
        let subj_ty = subject.ty.clone();
        let mut arms = Vec::new();
        let mut first_arm_ty: Option<Type> = None;

        let pre_match = self.snapshot_moved_fields();
        let mut arm_ends: Vec<_> = Vec::new();
        for a in &m.arms {
            self.restore_moved_fields(pre_match.clone());
            self.push_scope();
            let pat = self.lower_pat(&a.pat, &subj_ty)?;
            let guard = a
                .guard
                .as_ref()
                .map(|g| self.lower_expr_expected(g, Some(&Type::Bool)))
                .transpose()?;
            let body = self.lower_block_no_scope(&a.body, ret_ty)?;
            if let Some(hir::Stmt::Expr(tail_expr)) = body.last() {
                if let Some(ref first_ty) = first_arm_ty {
                    let _ = self.infer_ctx.unify_at(
                        first_ty,
                        &tail_expr.ty,
                        a.span,
                        "match arm result type",
                    );
                } else {
                    first_arm_ty = Some(tail_expr.ty.clone());
                }
            }
            self.pop_scope();
            arm_ends.push(self.snapshot_moved_fields());
            arms.push(hir::Arm {
                pat,
                guard,
                body,
                span: a.span,
            });
        }

        self.restore_moved_fields(pre_match);
        self.merge_moved_fields_union(&arm_ends);
        let resolved_subj_ty = self.infer_ctx.resolve(&subj_ty);
        let result_ty = first_arm_ty
            .map(|t| self.infer_ctx.shallow_resolve(&t))
            .unwrap_or(Type::Void);
        let result = hir::Match {
            subject,
            arms,
            ty: result_ty,
            span: m.span,
        };

        self.check_exhaustiveness(&resolved_subj_ty, &result.arms, m.span)?;

        Ok(result)
    }

    pub(crate) fn lower_pat(
        &mut self,
        pat: &ast::Pat,
        expected_ty: &Type,
    ) -> Result<hir::Pat, String> {
        match pat {
            ast::Pat::Wild(span) => Ok(hir::Pat::Wild(*span)),
            ast::Pat::Ident(name, span) => {
                if let Some((en, tag)) = self.variant_tags.get(name).cloned() {
                    let enum_ty = Type::Enum(en.clone());
                    let _ = self.infer_ctx.unify_at(
                        expected_ty,
                        &enum_ty,
                        *span,
                        "match pattern implies enum type",
                    );
                    return Ok(hir::Pat::Ctor(name.as_str(), tag, vec![], *span));
                }
                let id = self.fresh_id();
                let ty = expected_ty.clone();
                self.define_var(
                    &name.as_str(),
                    VarInfo {
                        def_id: id,
                        ty: ty.clone(),
                        ownership: Self::ownership_for_type(&ty),
                        scheme: None,
                    },
                );
                Ok(hir::Pat::Bind(id, name.clone(), ty, *span))
            }
            ast::Pat::Lit(e) => {
                let he = self.lower_expr(e)?;
                Ok(hir::Pat::Lit(he))
            }
            ast::Pat::Ctor(name, sub_pats, span) => {
                let tag = self.variant_tags.get(name).map(|(_, t)| *t).unwrap_or(0);

                let enum_name = self.variant_tags.get(name).map(|(en, _)| en.clone());

                if let Some(ref en) = enum_name {
                    let enum_ty = Type::Enum(en.clone());
                    let _ = self.infer_ctx.unify_at(
                        expected_ty,
                        &enum_ty,
                        *span,
                        "match pattern implies enum type",
                    );
                }

                let field_tys: Vec<Type> = if let Some(ref en) = enum_name {
                    if let Some(variants) = self.enums.get(en) {
                        variants
                            .iter()
                            .find(|(vn, _)| vn == name)
                            .map(|(_, ftys)| ftys.clone())
                            .unwrap_or_default()
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                };

                let mut hpats = Vec::new();
                for (i, sp) in sub_pats.iter().enumerate() {
                    let ft = field_tys
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| self.infer_ctx.fresh_var());
                    hpats.push(self.lower_pat(sp, &ft)?);
                }
                Ok(hir::Pat::Ctor(name.as_str(), tag, hpats, *span))
            }
            ast::Pat::Or(pats, span) => {
                let mut hpats = Vec::new();
                for p in pats {
                    hpats.push(self.lower_pat(p, expected_ty)?);
                }
                Ok(hir::Pat::Or(hpats, *span))
            }
            ast::Pat::Range(lo, hi, span) => {
                let hlo = self.lower_expr(lo)?;
                let hhi = self.lower_expr(hi)?;
                Ok(hir::Pat::Range(Box::new(hlo), Box::new(hhi), *span))
            }
            ast::Pat::Tuple(pats, span) => {
                let tys = match expected_ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => (0..pats.len())
                        .map(|_| self.infer_ctx.fresh_var())
                        .collect(),
                };
                let mut hpats = Vec::new();
                for (i, p) in pats.iter().enumerate() {
                    let ety = tys
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| self.infer_ctx.fresh_var());
                    hpats.push(self.lower_pat(p, &ety)?);
                }
                Ok(hir::Pat::Tuple(hpats, *span))
            }
            ast::Pat::Array(pats, span) => {
                let elem_ty = match expected_ty {
                    Type::Array(et, _) => et.as_ref().clone(),
                    _ => self.infer_ctx.fresh_var(),
                };
                let mut hpats = Vec::new();
                for p in pats {
                    hpats.push(self.lower_pat(p, &elem_ty)?);
                }
                Ok(hir::Pat::Array(hpats, *span))
            }
        }
    }
}
