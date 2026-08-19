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

    pub(crate) fn is_result_variant_expr(e: &ast::Expr) -> bool {
        let name = match e {
            ast::Expr::Call(callee, _, _) => match callee.as_ref() {
                ast::Expr::Ident(n, _) => n.as_str(),
                _ => return false,
            },
            ast::Expr::Struct(n, _, _) => n.as_str(),
            ast::Expr::Ident(n, _) => n.as_str(),
            _ => return false,
        };
        matches!(name.as_str(), "Ok" | "Err" | "Some" | "Nothing")
    }

    pub(crate) fn expr_is_fallible_producer(e: &ast::Expr) -> bool {
        matches!(
            e,
            ast::Expr::Call(..)
                | ast::Expr::Method(..)
                | ast::Expr::StoreInsert(..)
                | ast::Expr::StoreUpdate(..)
        )
    }

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
            if idx == block_len - 1
                && let (Some(expected), crate::ast::Stmt::Expr(e)) = (tail_expected, s)
            {
                let resolved_expected = self.infer_ctx.shallow_resolve(expected);
                if let Some(result_enum) = self.result_enum_of(&resolved_expected) {
                    let result_ty = Type::Enum(result_enum);
                    let he = if Self::is_result_variant_expr(e) {
                        self.lower_expr_expected(e, Some(&result_ty))?
                    } else if Self::expr_is_fallible_producer(e) {
                        self.lower_expr(e)?
                    } else {
                        let ok_inner = match &resolved_expected {
                            Type::Struct(n, args) if n.as_str() == "Result" && args.len() == 2 => {
                                args[0].clone()
                            }
                            _ => self.ok_inner_ty_pub(result_enum),
                        };
                        let he = self.lower_expr_expected(e, Some(&ok_inner))?;
                        if matches!(self.infer_ctx.shallow_resolve(&ok_inner), Type::Void) {
                            let het = self.infer_ctx.shallow_resolve(&he.ty);
                            let voidish = matches!(het, Type::Void)
                                || matches!(
                                    het,
                                    Type::TypeVar(v) if matches!(
                                        self.infer_ctx.constraint(v),
                                        super::super::unify::TypeConstraint::None
                                    )
                                );
                            if !voidish {
                                return Err(format!(
                                    "{}: a bare `! E` signature means `Result of Unit, E`, \
                                     so the function body must end in unit — this tail \
                                     expression has a value type; declare `returns T ! E` \
                                     to return a value, or drop the tail expression",
                                    he.span.loc(),
                                ));
                            }
                        }
                        he
                    };
                    let val_ty = self.infer_ctx.resolve(&he.ty);
                    let he = match self.result_enum_of(&val_ty) {
                        Some(val_enum) if val_enum == result_enum => he,
                        Some(_) => match self.implicit_propagate(he.clone())? {
                            Some(v) => self.auto_wrap_ok(v, result_enum),
                            None => he,
                        },
                        None => self.auto_wrap_ok(he, result_enum),
                    };
                    let stmt = hir::Stmt::Expr(he);
                    self.record_take_moves_in_stmt(&stmt)?;
                    stmts.push(stmt);
                    continue;
                }
                let he = self.lower_expr_expected(e, Some(expected))?;
                let stmt = hir::Stmt::Expr(he);
                self.record_take_moves_in_stmt(&stmt)?;
                stmts.push(stmt);
                continue;
            }
            if idx == block_len - 1
                && let (Some(expected), crate::ast::Stmt::StoreInsert(store, values, span)) =
                    (tail_expected, s)
            {
                let resolved_expected = self.infer_ctx.shallow_resolve(expected);
                if let Some(result_enum) = self.result_enum_of(&resolved_expected) {
                    let insert = self.lower_expr_store_insert(store, values, *span)?;
                    let he = match self.implicit_propagate(insert.clone())? {
                        Some(v) => self.auto_wrap_ok(v, result_enum),
                        None => insert,
                    };
                    let stmt = hir::Stmt::Expr(he);
                    self.record_take_moves_in_stmt(&stmt)?;
                    stmts.push(stmt);
                    continue;
                }
            }
            if idx == block_len - 1
                && let (Some(expected), crate::ast::Stmt::If(i)) = (tail_expected, s)
            {
                let hi = self.lower_if_with_tail(i, ret_ty, Some(expected))?;
                let stmt = hir::Stmt::If(hi);
                self.record_take_moves_in_stmt(&stmt)?;
                stmts.push(stmt);
                continue;
            }
            if idx == block_len - 1
                && let (Some(expected), crate::ast::Stmt::Match(m)) = (tail_expected, s)
            {
                let hm = self.lower_match_with_tail(m, ret_ty, Some(expected))?;
                let stmt = hir::Stmt::Match(hm);
                self.record_take_moves_in_stmt(&stmt)?;
                for p in std::mem::take(&mut self.pending_prelude_stmts) {
                    self.record_take_moves_in_stmt(&p)?;
                    stmts.push(p);
                }
                stmts.push(stmt);
                continue;
            }
            let stmt = self.lower_stmt(s, ret_ty)?;
            self.record_take_moves_in_stmt(&stmt)?;
            for p in std::mem::take(&mut self.pending_prelude_stmts) {
                self.record_take_moves_in_stmt(&p)?;
                stmts.push(p);
            }
            stmts.push(stmt);
            stmts.append(&mut self.pending_post_stmts);
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
        self.lower_if_with_tail(i, ret_ty, None)
    }

    fn lower_branch_with_tail(
        &mut self,
        b: &ast::Block,
        ret_ty: &Type,
        tail_expected: Option<&Type>,
    ) -> Result<hir::Block, String> {
        self.lower_block_with_tail(b, ret_ty, tail_expected)
    }

    pub(crate) fn lower_if_with_tail(
        &mut self,
        i: &ast::If,
        ret_ty: &Type,
        tail_expected: Option<&Type>,
    ) -> Result<hir::If, String> {
        let cond = self.lower_expr_expected(&i.cond, Some(&Type::Bool))?;
        self.record_take_moves_in_expr(&cond)?;

        let mut pre_if = self.snapshot_moved_fields();
        let then = self.lower_branch_with_tail(&i.then, ret_ty, tail_expected)?;
        let then_end = self.snapshot_moved_fields();
        let mut branch_ends: Vec<_> = vec![then_end];
        self.restore_moved_fields(pre_if.clone());
        let mut elifs = Vec::new();
        for (ec, eb) in &i.elifs {
            let hc = self.lower_expr_expected(ec, Some(&Type::Bool))?;
            self.record_take_moves_in_expr(&hc)?;
            pre_if = self.snapshot_moved_fields();
            let hb = self.lower_branch_with_tail(eb, ret_ty, tail_expected)?;
            branch_ends.push(self.snapshot_moved_fields());
            self.restore_moved_fields(pre_if.clone());
            elifs.push((hc, hb));
        }
        let els = match &i.els {
            Some(b) => Some(self.lower_branch_with_tail(b, ret_ty, tail_expected)?),
            None => None,
        };
        if els.is_some() {
            branch_ends.push(self.snapshot_moved_fields());
        }

        self.restore_moved_fields(pre_if);
        self.merge_moved_fields_union(&branch_ends);

        let mut result_ty = Type::Void;
        if tail_expected.is_some() && els.is_some() {
            let mut join_ty: Option<Type> = self.join_branch_type(&then);
            let mut rest: Vec<&hir::Block> = elifs.iter().map(|(_, b)| b).collect();
            if let Some(ref e) = els {
                rest.push(e);
            }
            let arms: Vec<Option<Type>> = rest.iter().map(|b| self.join_branch_type(b)).collect();
            for arm_ty in arms {
                match (&join_ty, arm_ty) {
                    (Some(j), Some(a)) => {
                        let j = j.clone();
                        self.unify_join_arm(&j, &a, i.span, "if");
                    }
                    (None, Some(a)) => join_ty = Some(a),
                    _ => {}
                }
            }
            if let Some(j) = join_ty {
                result_ty = self.infer_ctx.shallow_resolve(&j);
            }
        }

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
            ty: result_ty,
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
                    .entry(b.name)
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
        self.lower_match_with_tail(m, ret_ty, None)
    }

    fn adapt_propagate_match(&mut self, m: &ast::Match, subj_ty: &Type) -> ast::Match {
        let resolved = self.infer_ctx.resolve(subj_ty);
        let is_option = match &resolved {
            Type::Enum(n) => {
                let s = n.as_str();
                s.starts_with("Option_") || s == "Option"
            }
            Type::Struct(n, _) => n.as_str() == "Option",
            _ => false,
        };
        if !is_option {
            return m.clone();
        }
        let is_prop = m.arms.iter().any(|a| {
            matches!(&a.pat, ast::Pat::Ctor(n, ps, _)
                if n.as_str() == "Ok"
                    && matches!(ps.first(), Some(ast::Pat::Ident(b, _)) if b.as_str().contains("__prop_v_")))
        });
        if !is_prop {
            return m.clone();
        }
        let mut m2 = m.clone();
        for a in &mut m2.arms {
            if let ast::Pat::Ctor(n, ps, sp) = &a.pat {
                if n.as_str() == "Ok" {
                    a.pat = ast::Pat::Ctor("Some".into(), ps.clone(), *sp);
                } else if n.as_str() == "Err" {
                    let bsp = *sp;
                    a.pat = ast::Pat::Ctor("Nothing".into(), vec![], bsp);
                    a.body = vec![ast::Stmt::ErrReturn(
                        ast::Expr::Ident("Nothing".into(), bsp),
                        bsp,
                    )];
                }
            }
        }
        m2
    }

    pub(crate) fn lower_match_with_tail(
        &mut self,
        m: &ast::Match,
        ret_ty: &Type,
        tail_expected: Option<&Type>,
    ) -> Result<hir::Match, String> {
        let subject = self.lower_expr(&m.subject)?;
        self.record_take_moves_in_expr(&subject)?;
        let mut subject_prelude: Option<hir::Stmt> = None;
        let subject = if crate::typer::place::place_of_expr(&subject).is_none() {
            let resolved = {
                let was_strict = self.infer_ctx.is_strict();
                self.infer_ctx.set_strict(false);
                let r = self.infer_ctx.resolve(&subject.ty);
                self.infer_ctx.set_strict(was_strict);
                r
            };
            if self.needs_drop(&resolved) {
                let id = self.fresh_id();
                let nm: Symbol = format!("__match_subj_{}", id.0).into();
                let ty = subject.ty.clone();
                let span = subject.span;
                let ownership = Self::ownership_for_type(&ty);
                self.define_var(
                    &nm.as_str(),
                    VarInfo {
                        def_id: id,
                        ty: ty.clone(),
                        ownership,
                        scheme: None,
                    },
                );
                subject_prelude = Some(hir::Stmt::Bind(hir::Bind {
                    def_id: id,
                    name: nm,
                    value: subject,
                    ty: ty.clone(),
                    ownership,
                    atomic: false,
                    access_mod: None,
                    span,
                }));
                hir::Expr {
                    kind: hir::ExprKind::Var(id, nm),
                    ty,
                    span,
                }
            } else {
                subject
            }
        } else {
            subject
        };
        let subj_ty = subject.ty.clone();

        let m = self.adapt_propagate_match(m, &subj_ty);
        let m = &m;

        let mut arms = Vec::new();
        let mut first_arm_ty: Option<Type> = None;

        let pre_match = self.snapshot_moved_fields();
        let mut arm_ends: Vec<_> = Vec::new();
        for a in &m.arms {
            self.restore_moved_fields(pre_match.clone());
            self.push_scope();
            if let ast::Pat::Ident(name, span) = &a.pat
                && !self.variant_tags.contains_key(name)
                && !self.consts.contains_key(name)
                && self.find_var(&name.as_str()).is_some()
            {
                return Err(format!(
                    "{}: pattern `{}` always matches — it binds a new variable that \
                     shadows the existing `{}` rather than comparing against it; use \
                     `equals` (or a guard) to compare, or pick a fresh name to bind",
                    span.loc(),
                    name,
                    name
                ));
            }
            let pat = self.lower_pat(&a.pat, &subj_ty)?;
            let mut pat_binds = std::collections::HashSet::new();
            Self::collect_pat_bind_ids(&pat, &mut pat_binds);
            if let Some(subj_pl) = crate::typer::place::place_of_expr(&subject) {
                for bid in &pat_binds {
                    self.payload_bind_subjects.insert(*bid, subj_pl.clone());
                }
            }
            let guard = a
                .guard
                .as_ref()
                .map(|g| self.lower_expr_expected(g, Some(&Type::Bool)))
                .transpose()?;
            let arm_expected = first_arm_ty.as_ref().or(tail_expected);
            let mut body = self.lower_block_no_scope_with_tail(&a.body, ret_ty, arm_expected)?;
            if let Some(tail_ty) = self.join_branch_type(&body) {
                if let Some(ref first_ty) = first_arm_ty {
                    let first_ty = first_ty.clone();
                    self.unify_join_arm(&first_ty, &tail_ty, a.span, "match");
                } else {
                    first_arm_ty = Some(tail_ty);
                }
            }
            let consumed_binds: std::collections::HashSet<crate::hir::DefId> = pat_binds
                .iter()
                .filter(|b| !self.moves.entries_for(**b).is_empty())
                .cloned()
                .collect();
            let mut pat = pat;
            let mut wild_drops: Vec<hir::Stmt> = Vec::new();
            if !consumed_binds.is_empty()
                && let hir::Pat::Ctor(ref vname, _, ref mut subpats, _) = pat
            {
                let vsym = Symbol::intern(vname);
                let field_tys: Vec<Type> = self
                    .variant_tags
                    .get(&vsym)
                    .map(|(en, _)| *en)
                    .and_then(|en| {
                        self.enums.get(&en).and_then(|vs| {
                            vs.iter()
                                .find(|(vn, _)| *vn == vsym)
                                .map(|(_, ftys)| ftys.clone())
                        })
                    })
                    .unwrap_or_default();
                for (i, sp) in subpats.iter_mut().enumerate() {
                    if let hir::Pat::Wild(wspan) = sp {
                        let wspan = *wspan;
                        let Some(fty) = field_tys.get(i).cloned() else {
                            continue;
                        };
                        let resolved = {
                            let was_strict = self.infer_ctx.is_strict();
                            self.infer_ctx.set_strict(false);
                            let r = self.infer_ctx.resolve(&fty);
                            self.infer_ctx.set_strict(was_strict);
                            r
                        };
                        if self.needs_drop(&resolved) {
                            let id = self.fresh_id();
                            let nm: Symbol = format!("__unbound{i}").into();
                            *sp = hir::Pat::Bind(id, nm, fty, wspan);
                            wild_drops.push(hir::Stmt::Drop(id, nm, resolved, wspan));
                        }
                    }
                }
            }
            let arm_excl = if consumed_binds.is_empty() {
                &pat_binds
            } else {
                &consumed_binds
            };
            self.finalize_block_drops_excluding(&mut body, arm_excl);
            body.extend(wild_drops);
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

        if let Some(p) = subject_prelude {
            self.pending_prelude_stmts.push(p);
        }
        Ok(result)
    }

    fn same_enum_modulo_mono(&self, en: crate::intern::Symbol, resolved: &Type) -> bool {
        fn base(s: &str) -> &str {
            s.split("__G_").next().unwrap_or(s)
        }
        let en_str = en.as_str();
        match resolved {
            Type::Enum(n) => base(&n.as_str()) == base(&en_str),
            Type::Struct(n, _) if self.generic_enums.contains_key(n) => base(&en_str) == n.as_str(),
            _ => false,
        }
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
                    let resolved_expected = self.infer_ctx.resolve(expected_ty);
                    if !self.same_enum_modulo_mono(en, &resolved_expected) {
                        let enum_ty = Type::Enum(en);
                        let _ = self.infer_ctx.unify_at(
                            expected_ty,
                            &enum_ty,
                            *span,
                            "match pattern implies enum type",
                        );
                    }
                    return Ok(hir::Pat::Ctor(name.as_str(), tag, vec![], *span));
                }
                if self.find_var(&name.as_str()).is_none()
                    && let Some(const_expr) = self.consts.get(name).cloned()
                {
                    let he = self.lower_expr_expected(&const_expr, Some(expected_ty))?;
                    return Ok(hir::Pat::Lit(he));
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
                Ok(hir::Pat::Bind(id, *name, ty, *span))
            }
            ast::Pat::Lit(e) => {
                let he = self.lower_expr(e)?;
                Ok(hir::Pat::Lit(he))
            }
            ast::Pat::Ctor(name, sub_pats, span) => {
                let tag = self.variant_tags.get(name).map(|(_, t)| *t).unwrap_or(0);

                let resolved_expected = self.infer_ctx.resolve(expected_ty);
                let expected_enum = match &resolved_expected {
                    Type::Enum(n)
                        if self
                            .enums
                            .get(n)
                            .map(|vs| vs.iter().any(|(vn, _)| vn == name))
                            .unwrap_or(false) =>
                    {
                        Some(*n)
                    }
                    Type::Struct(n, args) if self.generic_enums.contains_key(n) => {
                        let ge = self.generic_enums.get(n).cloned().unwrap();
                        if args.len() == ge.type_params.len() {
                            let mut tm = std::collections::HashMap::new();
                            for (tp, ta) in ge.type_params.iter().zip(args.iter()) {
                                tm.insert(*tp, ta.clone());
                            }
                            self.monomorphize_enum(&n.as_str(), &tm).ok()
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                let enum_name =
                    expected_enum.or_else(|| self.variant_tags.get(name).map(|(en, _)| *en));

                if let Some(ref en) = enum_name
                    && !self.same_enum_modulo_mono(*en, &resolved_expected)
                {
                    let enum_ty = Type::Enum(*en);
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
