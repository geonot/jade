use super::super::Typer;
use crate::ast;
use crate::hir;
use crate::types::Type;

impl Typer {
    #[allow(clippy::only_used_in_recursion)]
    pub(in crate::typer) fn hir_tail_type(&self, body: &[hir::Stmt]) -> Option<Type> {
        let last = body
            .iter()
            .rev()
            .find(|s| !matches!(s, hir::Stmt::Drop(..)))?;
        match last {
            hir::Stmt::Expr(e) if e.ty != Type::Void => Some(e.ty.clone()),
            hir::Stmt::If(i) => {
                if i.els.is_some() {
                    self.hir_tail_type(&i.then)
                } else {
                    None
                }
            }
            hir::Stmt::Match(m) => {
                if let Some(arm) = m.arms.first() {
                    self.hir_tail_type(&arm.body)
                } else {
                    None
                }
            }
            hir::Stmt::Ret(Some(e), _, _) => Some(e.ty.clone()),
            _ => None,
        }
    }

    pub(in crate::typer) fn block_diverges(body: &[hir::Stmt]) -> bool {
        let Some(last) = body
            .iter()
            .rev()
            .find(|s| !matches!(s, hir::Stmt::Drop(..)))
        else {
            return false;
        };
        match last {
            hir::Stmt::Ret(..)
            | hir::Stmt::Break(..)
            | hir::Stmt::Continue(..)
            | hir::Stmt::ErrReturn(..) => true,
            hir::Stmt::If(i) => match &i.els {
                Some(els) => {
                    Self::block_diverges(&i.then)
                        && Self::block_diverges(els)
                        && i.elifs.iter().all(|(_, b)| Self::block_diverges(b))
                }
                None => false,
            },
            hir::Stmt::Match(m) => {
                !m.arms.is_empty() && m.arms.iter().all(|a| Self::block_diverges(&a.body))
            }
            _ => false,
        }
    }

    pub(in crate::typer) fn join_branch_type(&self, body: &[hir::Stmt]) -> Option<Type> {
        if Self::block_diverges(body) {
            return None;
        }
        self.hir_tail_type(body)
    }

    pub(in crate::typer) fn unify_join_arm(
        &mut self,
        join_ty: &Type,
        arm_ty: &Type,
        span: crate::ast::Span,
        construct: &'static str,
    ) {
        let reason: &'static str = if construct == "match" {
            "match arm result type"
        } else {
            "if branches"
        };
        if self
            .infer_ctx
            .unify_at_tolerant(join_ty, arm_ty, span, reason)
            .is_err()
        {
            let want = self.infer_ctx.resolve(join_ty);
            let got = self.infer_ctx.resolve(arm_ty);
            let (first, this) = if construct == "match" {
                ("the first arm", "this arm")
            } else {
                ("the first branch", "this branch")
            };
            let help = match (&want, &got) {
                (Type::String, t) | (t, Type::String) if t.is_num() => {
                    "\n  help: use `to_string(value)` so both produce a string"
                }
                _ => {
                    "\n  help: every branch used as a value must produce the same type; \
                     make them agree, or bind each branch separately"
                }
            };
            self.type_errors.push(format!(
                "{}: `{construct}` branches produce different types: {first} produces `{want}`, \
                 but {this} produces `{got}`{help}",
                span.loc(),
            ));
        }
    }

    pub(in crate::typer) fn lower_block(
        &mut self,
        block: &ast::Block,
        ret_ty: &Type,
    ) -> Result<hir::Block, String> {
        self.lower_block_with_tail(block, ret_ty, None)
    }

    pub(in crate::typer) fn lower_block_with_tail(
        &mut self,
        block: &ast::Block,
        ret_ty: &Type,
        tail_expected: Option<&Type>,
    ) -> Result<hir::Block, String> {
        self.push_scope();
        let mut stmts = self.lower_block_no_scope_with_tail(block, ret_ty, tail_expected)?;
        self.finalize_block_drops(&mut stmts);
        self.pop_scope();
        Ok(stmts)
    }

    pub(in crate::typer) fn finalize_block_drops(&mut self, stmts: &mut Vec<hir::Stmt>) {
        let ends_with_jump = stmts.last().is_some_and(|s| {
            matches!(
                s,
                hir::Stmt::Ret(..) | hir::Stmt::Break(..) | hir::Stmt::Continue(..)
            )
        });
        if ends_with_jump {
            let jump = stmts.pop().unwrap();

            let mut jump_refs = std::collections::HashSet::new();
            Self::collect_hir_var_ids_stmt(&jump, &mut jump_refs);
            self.emit_scope_drops_excluding(stmts, &jump_refs);
            stmts.push(jump);
        } else if let Some(hir::Stmt::Expr(_)) = stmts.last() {
            let tail = stmts.pop().unwrap();
            let mut tail_moves = std::collections::HashSet::new();
            if let hir::Stmt::Expr(te) = &tail {
                Self::collect_moved_var_ids(te, &mut tail_moves);
            }
            stmts.push(tail);
            self.emit_scope_drops_excluding(stmts, &tail_moves);
        } else {
            self.emit_scope_drops(stmts);
        }
    }

    pub(in crate::typer) fn finalize_block_drops_excluding(
        &mut self,
        stmts: &mut Vec<hir::Stmt>,
        extra: &std::collections::HashSet<crate::hir::DefId>,
    ) {
        let ends_with_jump = stmts.last().is_some_and(|s| {
            matches!(
                s,
                hir::Stmt::Ret(..) | hir::Stmt::Break(..) | hir::Stmt::Continue(..)
            )
        });
        if ends_with_jump {
            let jump = stmts.pop().unwrap();
            let mut excl = extra.clone();
            Self::collect_hir_var_ids_stmt(&jump, &mut excl);
            self.emit_scope_drops_excluding(stmts, &excl);
            stmts.push(jump);
        } else if let Some(hir::Stmt::Expr(_)) = stmts.last() {
            let tail = stmts.pop().unwrap();
            let mut excl = extra.clone();
            if let hir::Stmt::Expr(te) = &tail {
                Self::collect_moved_var_ids(te, &mut excl);
            }
            stmts.push(tail);
            self.emit_scope_drops_excluding(stmts, &excl);
        } else {
            self.emit_scope_drops_excluding(stmts, extra);
        }
    }

    pub(in crate::typer) fn finalize_loop_body_drops(&mut self, stmts: &mut Vec<hir::Stmt>) {
        let ends_with_jump = stmts.last().is_some_and(|s| {
            matches!(
                s,
                hir::Stmt::Ret(..) | hir::Stmt::Break(..) | hir::Stmt::Continue(..)
            )
        });
        if ends_with_jump {
            let jump = stmts.pop().unwrap();
            let mut jump_refs = std::collections::HashSet::new();
            Self::collect_hir_var_ids_stmt(&jump, &mut jump_refs);
            self.emit_scope_drops_excluding(stmts, &jump_refs);
            stmts.push(jump);
        } else {
            self.emit_scope_drops(stmts);
        }
    }

    pub(in crate::typer) fn collect_pat_bind_ids(
        pat: &hir::Pat,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match pat {
            hir::Pat::Bind(id, _, _, _) => {
                out.insert(*id);
            }
            hir::Pat::Ctor(_, _, sub, _)
            | hir::Pat::Or(sub, _)
            | hir::Pat::Tuple(sub, _)
            | hir::Pat::Array(sub, _) => {
                for p in sub {
                    Self::collect_pat_bind_ids(p, out);
                }
            }
            hir::Pat::Wild(_) | hir::Pat::Lit(_) | hir::Pat::Range(..) => {}
        }
    }

    pub(in crate::typer) fn emit_scope_drops(&mut self, stmts: &mut Vec<hir::Stmt>) {
        self.emit_scope_drops_excluding(stmts, &std::collections::HashSet::new());
    }

    fn collect_block_consumed_ids(
        &mut self,
        stmts: &[hir::Stmt],
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        for s in stmts {
            self.collect_consumed_in_stmt(s, out);
        }
    }

    fn collect_consumed_in_stmt(
        &mut self,
        s: &hir::Stmt,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match s {
            hir::Stmt::Expr(e) => self.collect_consumed_in_expr(e, out),

            hir::Stmt::Assign(target, value, _) => {
                let resolved = self.infer_ctx.resolve(&value.ty);
                if Self::expr_type_needs_drop(&resolved)
                    && let hir::ExprKind::Var(id, _) = &value.kind
                {
                    out.insert(*id);
                }
                self.collect_consumed_in_expr(value, out);
                self.collect_consumed_in_expr(target, out);
            }
            hir::Stmt::Bind(b) => {
                let resolved = self.infer_ctx.resolve(&b.value.ty);
                if Self::expr_type_needs_drop(&resolved)
                    && let hir::ExprKind::Var(id, _) = &b.value.kind
                {
                    out.insert(*id);
                }
                self.collect_consumed_in_expr(&b.value, out);
            }
            hir::Stmt::TupleBind(_, e, _)
            | hir::Stmt::Ret(Some(e), _, _)
            | hir::Stmt::ErrReturn(e, _, _)
            | hir::Stmt::Break(Some(e), _) => {
                self.collect_consumed_in_expr(e, out);
            }
            hir::Stmt::If(i) => {
                self.collect_consumed_in_expr(&i.cond, out);
                self.collect_block_consumed_ids(&i.then, out);
                for (c, b) in &i.elifs {
                    self.collect_consumed_in_expr(c, out);
                    self.collect_block_consumed_ids(b, out);
                }
                if let Some(b) = &i.els {
                    self.collect_block_consumed_ids(b, out);
                }
            }
            hir::Stmt::While(w) => {
                self.collect_consumed_in_expr(&w.cond, out);
                self.collect_block_consumed_ids(&w.body, out);
            }
            hir::Stmt::For(f) => {
                self.collect_consumed_in_expr(&f.iter, out);
                if let Some(e) = &f.end {
                    self.collect_consumed_in_expr(e, out);
                }
                if let Some(e) = &f.step {
                    self.collect_consumed_in_expr(e, out);
                }
                self.collect_block_consumed_ids(&f.body, out);
            }
            hir::Stmt::Loop(l) => self.collect_block_consumed_ids(&l.body, out),
            hir::Stmt::Match(m) => {
                self.collect_consumed_in_expr(&m.subject, out);
                for arm in &m.arms {
                    if let Some(g) = &arm.guard {
                        self.collect_consumed_in_expr(g, out);
                    }
                    self.collect_block_consumed_ids(&arm.body, out);
                }
            }
            hir::Stmt::Defer(b, _)
            | hir::Stmt::Transaction(b, _)
            | hir::Stmt::SimBlock(b, _)
            | hir::Stmt::Together(_, b, _, _, _) => {
                self.collect_block_consumed_ids(b, out);
            }
            hir::Stmt::SimFor(f, _) => {
                self.collect_consumed_in_expr(&f.iter, out);
                self.collect_block_consumed_ids(&f.body, out);
            }
            _ => {}
        }
    }

    fn collect_consumed_in_expr(
        &mut self,
        expr: &hir::Expr,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match &expr.kind {
            hir::ExprKind::VecMethod(_, meth, args) | hir::ExprKind::MapMethod(_, meth, args) => {
                let m_owned = meth.as_str();
                let m: &str = m_owned.as_ref();
                if matches!(
                    m,
                    "push"
                        | "push_back"
                        | "push_front"
                        | "insert"
                        | "append"
                        | "add"
                        | "put"
                        | "set"
                        | "enqueue"
                        | "send"
                ) {
                    for a in args {
                        let resolved = self.infer_ctx.resolve(&a.ty);
                        if Self::expr_type_needs_drop(&resolved) {
                            Self::collect_moved_var_ids(a, out);
                        }
                    }
                }
            }

            hir::ExprKind::Call(_, name, args) => {
                let access = self.fn_param_access.get(name).cloned();
                if let Some(access) = access {
                    for (i, a) in args.iter().enumerate() {
                        if matches!(access.get(i), Some(Some(crate::ast::AccessMod::Take)))
                            && let hir::ExprKind::Var(id, _) = &a.kind
                        {
                            let resolved = self.infer_ctx.resolve(&a.ty);
                            if Self::expr_type_needs_drop(&resolved) {
                                out.insert(*id);
                            }
                        }
                    }
                }

                for a in args {
                    self.collect_consumed_in_expr(a, out);
                }
            }

            hir::ExprKind::Method(recv, mangled_name, _m_name, args) => {
                let mangled: crate::intern::Symbol = *mangled_name;
                let access = self.fn_param_access.get(&mangled).cloned();
                if let Some(access) = access {
                    if matches!(access.first(), Some(Some(crate::ast::AccessMod::Take)))
                        && let hir::ExprKind::Var(id, _) = &recv.kind
                    {
                        let resolved = self.infer_ctx.resolve(&recv.ty);
                        if Self::expr_type_needs_drop(&resolved) {
                            out.insert(*id);
                        }
                    }
                    for (i, a) in args.iter().enumerate() {
                        if matches!(access.get(i + 1), Some(Some(crate::ast::AccessMod::Take)))
                            && let hir::ExprKind::Var(id, _) = &a.kind
                        {
                            let resolved = self.infer_ctx.resolve(&a.ty);
                            if Self::expr_type_needs_drop(&resolved) {
                                out.insert(*id);
                            }
                        }
                    }
                }
                self.collect_consumed_in_expr(recv, out);
                for a in args {
                    self.collect_consumed_in_expr(a, out);
                }
            }

            hir::ExprKind::BinOp(l, _, r) | hir::ExprKind::Index(l, r) => {
                self.collect_consumed_in_expr(l, out);
                self.collect_consumed_in_expr(r, out);
            }
            hir::ExprKind::UnaryOp(_, x)
            | hir::ExprKind::Field(x, _, _)
            | hir::ExprKind::Cast(x, _)
            | hir::ExprKind::StrictCast(x, _)
            | hir::ExprKind::Coerce(x, _)
            | hir::ExprKind::Ref(x)
            | hir::ExprKind::Deref(x) => {
                self.collect_consumed_in_expr(x, out);
            }
            hir::ExprKind::Ternary(c, t, e) => {
                self.collect_consumed_in_expr(c, out);
                self.collect_consumed_in_expr(t, out);
                self.collect_consumed_in_expr(e, out);
            }
            hir::ExprKind::Tuple(xs) | hir::ExprKind::Array(xs) | hir::ExprKind::VecNew(xs) => {
                for x in xs {
                    self.collect_ctor_captured_id(x, out);
                    self.collect_consumed_in_expr(x, out);
                }
            }
            hir::ExprKind::Builtin(_, xs) => {
                for x in xs {
                    self.collect_consumed_in_expr(x, out);
                }
            }
            hir::ExprKind::Struct(_, inits) | hir::ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    self.collect_ctor_captured_id(&fi.value, out);
                    self.collect_consumed_in_expr(&fi.value, out);
                }
            }
            hir::ExprKind::StringMethod(recv, _, args)
            | hir::ExprKind::DeferredMethod(recv, _, args) => {
                self.collect_consumed_in_expr(recv, out);
                for a in args {
                    self.collect_consumed_in_expr(a, out);
                }
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                self.collect_consumed_in_expr(callee, out);
                for a in args {
                    self.collect_consumed_in_expr(a, out);
                }
            }
            hir::ExprKind::Lambda(params, body) => {
                let mut ids: std::collections::HashSet<crate::hir::DefId> =
                    std::collections::HashSet::new();
                for st in body {
                    Self::collect_hir_var_ids_stmt_inner(st, &mut ids, true);
                }
                for p in params {
                    ids.remove(&p.def_id);
                }
                for id in ids {
                    let Some(ty) = self.find_var_by_id(id).map(|info| info.ty.clone()) else {
                        continue;
                    };
                    let was_strict = self.infer_ctx.is_strict();
                    self.infer_ctx.set_strict(false);
                    let resolved = self.infer_ctx.resolve(&ty);
                    self.infer_ctx.set_strict(was_strict);
                    if self.type_is_aggregate(&resolved) {
                        out.insert(id);
                    }
                }
            }
            hir::ExprKind::Pipe(e, _, name, rest) => {
                if let Some(access) = self.fn_param_access.get(name).cloned() {
                    let mut parts: Vec<&hir::Expr> = Vec::with_capacity(rest.len() + 1);
                    parts.push(e);
                    parts.extend(rest.iter());
                    for (i, a) in parts.iter().enumerate() {
                        if matches!(access.get(i), Some(Some(crate::ast::AccessMod::Take)))
                            && let hir::ExprKind::Var(id, _) = &a.kind
                        {
                            let resolved = self.infer_ctx.resolve(&a.ty);
                            if Self::expr_type_needs_drop(&resolved) {
                                out.insert(*id);
                            }
                        }
                    }
                }
                self.collect_consumed_in_expr(e, out);
                for a in rest {
                    self.collect_consumed_in_expr(a, out);
                }
            }
            hir::ExprKind::Block(stmts) => {
                for s in stmts {
                    self.collect_consumed_in_stmt(s, out);
                }
            }
            hir::ExprKind::IfExpr(i) => {
                self.collect_consumed_in_expr(&i.cond, out);
                self.collect_block_consumed_ids(&i.then, out);
                for (c, b) in &i.elifs {
                    self.collect_consumed_in_expr(c, out);
                    self.collect_block_consumed_ids(b, out);
                }
                if let Some(b) = &i.els {
                    self.collect_block_consumed_ids(b, out);
                }
            }
            _ => {}
        }
    }

    fn expr_type_needs_drop(ty: &Type) -> bool {
        match ty {
            Type::Vec(_)
            | Type::Map(_, _)
            | Type::String
            | Type::Struct(_, _)
            | Type::Enum(_)
            | Type::Row(_)
            | Type::Fn(_, _) => true,
            Type::Frozen(inner) => Self::expr_type_needs_drop(inner),
            _ => false,
        }
    }

    pub(in crate::typer) fn type_is_aggregate(&self, ty: &Type) -> bool {
        let mut visiting: std::collections::HashSet<crate::intern::Symbol> =
            std::collections::HashSet::new();
        self.type_is_aggregate_inner(ty, &mut visiting)
    }

    fn type_is_aggregate_inner(
        &self,
        ty: &Type,
        visiting: &mut std::collections::HashSet<crate::intern::Symbol>,
    ) -> bool {
        match ty {
            Type::Vec(_)
            | Type::Map(_, _)
            | Type::Coroutine(_)
            | Type::Generator(_)
            | Type::Fn(_, _) => true,
            Type::Struct(name, args) => {
                if self
                    .struct_attrs
                    .get(name)
                    .map(|a| a.resource)
                    .unwrap_or(false)
                {
                    return false;
                }
                if !visiting.insert(*name) {
                    return false;
                }
                let result = self
                    .struct_field_types(name, args)
                    .into_iter()
                    .any(|fty| self.type_is_aggregate_inner(&fty, visiting));
                visiting.remove(name);
                result
            }
            Type::Enum(name) => {
                if !visiting.insert(*name) {
                    return false;
                }
                let result = if let Some(variants) = self.enums.get(name) {
                    variants.iter().any(|(_vname, ftys)| {
                        ftys.iter()
                            .any(|t| self.type_is_aggregate_inner(t, visiting))
                    })
                } else {
                    false
                };
                visiting.remove(name);
                result
            }
            Type::Tuple(elts) => elts
                .iter()
                .any(|t| self.type_is_aggregate_inner(t, visiting)),
            Type::Array(elem, _) => self.type_is_aggregate_inner(elem, visiting),
            Type::Alias(_, inner) | Type::Newtype(_, inner) | Type::Frozen(inner) => {
                self.type_is_aggregate_inner(inner, visiting)
            }
            _ => false,
        }
    }

    pub(in crate::typer) fn peel_move_wrappers(e: &hir::Expr) -> &hir::Expr {
        match &e.kind {
            hir::ExprKind::Coerce(inner, _) | hir::ExprKind::Cast(inner, _) => {
                Self::peel_move_wrappers(inner)
            }
            _ => e,
        }
    }

    pub(in crate::typer) fn find_var_by_id(
        &self,
        id: crate::hir::DefId,
    ) -> Option<&crate::typer::VarInfo> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|s| s.values())
            .find(|v| v.def_id == id)
    }

    pub(in crate::typer) fn collect_aggregate_captures(
        &mut self,
        body: &[hir::Stmt],
        outer_ids: &std::collections::HashSet<crate::hir::DefId>,
    ) -> Vec<(crate::hir::DefId, crate::intern::Symbol)> {
        let mut used: std::collections::HashSet<crate::hir::DefId> =
            std::collections::HashSet::new();
        for st in body {
            Self::collect_hir_var_ids_stmt_inner(st, &mut used, true);
        }
        let mut candidates: Vec<(crate::hir::DefId, crate::intern::Symbol, Type)> = self
            .scopes
            .iter()
            .flat_map(|s| s.iter())
            .filter(|(_, info)| used.contains(&info.def_id) && outer_ids.contains(&info.def_id))
            .map(|(n, info)| (info.def_id, *n, info.ty.clone()))
            .collect();
        candidates.sort_by_key(|(id, _, _)| id.0);
        let mut captured = Vec::new();
        for (id, name, ty) in candidates {
            let was_strict = self.infer_ctx.is_strict();
            self.infer_ctx.set_strict(false);
            let resolved = self.infer_ctx.resolve(&ty);
            self.infer_ctx.set_strict(was_strict);
            if self.type_is_aggregate(&resolved) {
                captured.push((id, name));
            }
        }
        captured
    }

    pub(in crate::typer) fn mark_task_captures(
        &mut self,
        body: &[hir::Stmt],
        outer_ids: &std::collections::HashSet<crate::hir::DefId>,
        at: crate::ast::Span,
    ) -> Result<(), String> {
        self.reject_view_captures(body, outer_ids, at)?;
        for (id, name) in self.collect_aggregate_captures(body, outer_ids) {
            let in_together_outer = self
                .together_outer_ids
                .last()
                .is_some_and(|outer| outer.contains(&id));
            let var_ty = self.find_var_by_id(id).map(|info| info.ty.clone());
            let is_shared_frozen = in_together_outer
                && var_ty
                    .is_some_and(|t| matches!(self.infer_ctx.shallow_resolve(&t), Type::Frozen(_)));
            if is_shared_frozen {
                let pl = crate::typer::place::Place::var(id, name);
                let already = self.iter_borrowed.iter().any(|e| {
                    matches!(e.kind, crate::typer::BorrowKind::FrozenShare)
                        && e.place.root == pl.root
                });
                if !already {
                    self.iter_borrowed.push(crate::typer::BorrowEntry {
                        place: pl,
                        span: at,
                        kind: crate::typer::BorrowKind::FrozenShare,
                    });
                }
                continue;
            }
            self.mark_var_moved_checked(id, name, crate::typer::MoveReason::TaskCapture(at), at)?;
        }
        Ok(())
    }

    pub(in crate::typer) fn mark_closure_captures(
        &mut self,
        body: &[hir::Stmt],
        outer_ids: &std::collections::HashSet<crate::hir::DefId>,
        at: crate::ast::Span,
    ) -> Result<(), String> {
        self.reject_view_captures(body, outer_ids, at)?;
        let mut used: std::collections::HashSet<crate::hir::DefId> =
            std::collections::HashSet::new();
        for st in body {
            Self::collect_hir_var_ids_stmt_inner(st, &mut used, true);
        }
        let mut candidates: Vec<(
            crate::hir::DefId,
            crate::intern::Symbol,
            Type,
            crate::hir::Ownership,
        )> = self
            .scopes
            .iter()
            .flat_map(|s| s.iter())
            .filter(|(_, info)| used.contains(&info.def_id) && outer_ids.contains(&info.def_id))
            .map(|(n, info)| (info.def_id, *n, info.ty.clone(), info.ownership))
            .collect();
        candidates.sort_by_key(|(id, _, _, _)| id.0);
        for (id, name, ty, ownership) in candidates {
            let was_strict = self.infer_ctx.is_strict();
            self.infer_ctx.set_strict(false);
            let resolved = self.infer_ctx.resolve(&ty);
            self.infer_ctx.set_strict(was_strict);
            if self.type_has_resource_annotation(&resolved) {
                return Err(format!(
                    "{}: a closure cannot capture the @resource value `{}`: a resource \
                     has exactly one owner and a closure's call site is unknowable; \
                     pass it to the closure as a parameter instead",
                    at.loc(),
                    name,
                ));
            }
            if self.type_is_aggregate(&resolved) {
                if matches!(ownership, crate::hir::Ownership::Borrowed) {
                    return Err(format!(
                        "{}: a closure cannot capture `{}`: it is a borrow, not an \
                         owner (a parameter borrows unless the function consumes it), \
                         and a closure owns what it captures; capture a clone (bind \
                         `copy {}` to a fresh name), or pass it as a parameter",
                        at.loc(),
                        name,
                        name,
                    ));
                }
                self.mark_var_moved_checked(
                    id,
                    name,
                    crate::typer::MoveReason::ClosureCapture(at),
                    at,
                )?;
            }
        }
        Ok(())
    }

    pub(in crate::typer) fn reject_view_captures(
        &mut self,
        body: &[hir::Stmt],
        outer_ids: &std::collections::HashSet<crate::hir::DefId>,
        at: crate::ast::Span,
    ) -> Result<(), String> {
        let mut used: std::collections::HashSet<crate::hir::DefId> =
            std::collections::HashSet::new();
        for st in body {
            Self::collect_hir_var_ids_stmt_inner(st, &mut used, true);
        }
        let candidates: Vec<(crate::intern::Symbol, Type)> = self
            .scopes
            .iter()
            .flat_map(|s| s.iter())
            .filter(|(_, info)| used.contains(&info.def_id) && outer_ids.contains(&info.def_id))
            .map(|(n, info)| (*n, info.ty.clone()))
            .collect();
        for (name, ty) in candidates {
            let was_strict = self.infer_ctx.is_strict();
            self.infer_ctx.set_strict(false);
            let resolved = self.infer_ctx.resolve(&ty);
            self.infer_ctx.set_strict(was_strict);
            if crate::typer::expr::views::type_contains_view(&resolved) {
                return Err(format!(
                    "{}: `{}` is a view and cannot be captured by a task: it borrows \
                     memory owned by this frame; capture the owning container or a \
                     copied slice",
                    at.loc(),
                    name,
                ));
            }
        }
        Ok(())
    }

    pub(in crate::typer) fn record_take_moves_in_stmt(
        &mut self,
        s: &hir::Stmt,
    ) -> Result<(), String> {
        match s {
            hir::Stmt::Bind(b) => {
                let src = Self::peel_move_wrappers(&b.value);
                if let hir::ExprKind::Var(id, name) = &src.kind
                    && *id != b.def_id
                    && !matches!(b.ownership, crate::hir::Ownership::Borrowed)
                {
                    let resolved = self.infer_ctx.resolve(&src.ty);
                    match b.access_mod {
                        Some(crate::ast::AccessMod::Take) => {
                            if Self::expr_type_needs_drop(&resolved) {
                                self.mark_var_moved_checked(
                                    *id,
                                    *name,
                                    crate::typer::MoveReason::TakeExplicit,
                                    b.span,
                                )?;
                            }
                        }
                        None => {
                            if self.type_is_aggregate(&resolved) {
                                self.mark_var_moved_checked(
                                    *id,
                                    *name,
                                    crate::typer::MoveReason::AssignMove(b.name, b.span),
                                    b.span,
                                )?;
                            }
                        }
                        _ => {}
                    }
                }
                self.record_take_moves_in_expr(&b.value)?;
                self.clear_all_moved_for(b.def_id);
            }
            hir::Stmt::Ret(Some(e), _, span) => {
                let inner = Self::peel_move_wrappers(e);
                if let hir::ExprKind::Ref(pointee) = &inner.kind
                    && let hir::ExprKind::Var(id, name) = &Self::peel_move_wrappers(pointee).kind
                    && let Some(info) = self.find_var_by_id(*id)
                    && matches!(info.ownership, crate::hir::Ownership::Owned)
                {
                    return Err(format!(
                        "{}: returning reference to local variable `{}` — the pointee \
                         is dropped when the function returns; return the value itself \
                         to transfer ownership",
                        span.loc(),
                        name,
                    ));
                }
                self.suppress_move_marking += 1;
                let r = self.record_take_moves_in_expr(e);
                self.suppress_move_marking -= 1;
                r?;
            }
            hir::Stmt::ErrReturn(e, _, _) | hir::Stmt::Break(Some(e), _) => {
                self.suppress_move_marking += 1;
                let r = self.record_take_moves_in_expr(e);
                self.suppress_move_marking -= 1;
                r?;
            }
            hir::Stmt::Expr(e) => {
                self.record_take_moves_in_expr(e)?;
            }
            hir::Stmt::Assign(target, value, span) => {
                self.record_take_moves_in_expr(value)?;
                self.record_take_moves_in_expr(target)?;
                if let hir::ExprKind::Var(tid, tname) = &target.kind {
                    let src = Self::peel_move_wrappers(value);
                    if let hir::ExprKind::Var(id, name) = &src.kind
                        && id != tid
                    {
                        let resolved = self.infer_ctx.resolve(&src.ty);
                        if self.type_is_aggregate(&resolved) {
                            if self.find_var_by_id(*id).is_some_and(|v| {
                                matches!(v.ownership, crate::hir::Ownership::Borrowed)
                            }) {
                                return Err(format!(
                                    "{}: cannot assign `{}` to `{}`: `{}` is a borrow, \
                                     not an owner (a parameter borrows unless the \
                                     function consumes it); assign a clone \
                                     (`{} is copy {}`) instead",
                                    span.loc(),
                                    name,
                                    tname,
                                    name,
                                    tname,
                                    name,
                                ));
                            }
                            self.mark_var_moved_checked(
                                *id,
                                *name,
                                crate::typer::MoveReason::AssignMove(*tname, *span),
                                *span,
                            )?;
                        }
                    }
                    self.clear_all_moved_for(*tid);
                }
            }
            hir::Stmt::Defer(block, span) => {
                let mut ids: std::collections::HashSet<crate::hir::DefId> =
                    std::collections::HashSet::new();
                for st in block {
                    Self::collect_hir_var_ids_stmt(st, &mut ids);
                }
                for id in ids {
                    self.defer_read_vars.entry(id).or_insert(*span);
                }
            }

            _ => {}
        }
        Ok(())
    }

    fn is_container_element_read(&mut self, e: &hir::Expr) -> bool {
        match &e.kind {
            hir::ExprKind::VecMethod(_, m, _) => matches!(
                m.as_str().as_ref(),
                "get" | "first" | "last" | "at" | "front" | "back" | "peek"
            ),
            hir::ExprKind::MapMethod(_, m, _) => matches!(m.as_str().as_ref(), "get"),
            hir::ExprKind::Field(x, _, _) => self.is_container_element_read(x),
            hir::ExprKind::Index(base, _) => matches!(
                self.infer_ctx.shallow_resolve(&base.ty),
                Type::Vec(_) | Type::Map(_, _)
            ),
            _ => false,
        }
    }

    fn check_call_arg_aliasing(
        &mut self,
        callee: crate::intern::Symbol,
        access: &[Option<crate::ast::AccessMod>],
        parts: &[&hir::Expr],
        is_method: bool,
        span: crate::ast::Span,
    ) -> Result<(), String> {
        use crate::typer::place::{Place, place_of_expr};
        let mutates = self
            .fn_param_mutates
            .get(&callee)
            .cloned()
            .unwrap_or_default();
        struct ArgPlace {
            slot: usize,
            place: Place,
            consuming: bool,
            clonable: bool,
        }
        let mut places: Vec<ArgPlace> = Vec::new();
        for (i, a) in parts.iter().enumerate() {
            let src = Self::peel_move_wrappers(a);
            if let Some(place) = place_of_expr(src) {
                let resolved = self.infer_ctx.resolve(&src.ty);
                if !self.type_is_aggregate(&resolved) && !Self::expr_type_needs_drop(&resolved) {
                    continue;
                }
                places.push(ArgPlace {
                    slot: i,
                    place,
                    consuming: matches!(access.get(i), Some(Some(crate::ast::AccessMod::Take))),
                    clonable: resolved.is_value_clonable(),
                });
            }
        }
        let shown_callee = crate::typer::Typer::display_fn_name(&callee.as_str()).to_string();

        for p in &places {
            if p.place.is_root() {
                continue;
            }
            let is_recv = is_method && p.slot == 0;
            let is_mut = mutates.get(p.slot).copied().unwrap_or(false);
            if is_mut && is_recv {
                return Err(format!(
                    "{}: `{}` mutates its receiver, but the receiver here is the \
                     nested place `{}` — the method would act on a copy and the \
                     mutation would be silently lost; bind the value out, mutate \
                     it, and write it back",
                    span.loc(),
                    shown_callee,
                    p.place.render(),
                ));
            }
            if is_mut {
                return Err(format!(
                    "{}: cannot pass `{}` to `{}`, which mutates it through this \
                     parameter — a field or element read passes a copy, so the \
                     mutation would be silently lost; pass the whole variable, or \
                     bind the value out, mutate it, and write it back",
                    span.loc(),
                    p.place.render(),
                    shown_callee,
                ));
            }
            if p.consuming && (is_recv || !p.clonable) {
                return Err(format!(
                    "{}: `{}` is moved into this call to `{}` (consuming \
                     parameter), but it is a field or element of a value that \
                     still owns it — the call would create two owners; bind it \
                     out with `take` first, or pass a clone (`copy {}`)",
                    span.loc(),
                    p.place.render(),
                    shown_callee,
                    p.place.render(),
                ));
            }
        }

        for p in &places {
            if !p.consuming || !p.place.is_root() {
                continue;
            }
            for (j, a) in parts.iter().enumerate() {
                if j == p.slot {
                    continue;
                }
                let mut ids = std::collections::HashSet::new();
                Self::collect_hir_var_ids_expr(a, &mut ids);
                if ids.contains(&p.place.root) {
                    return Err(format!(
                        "{}: `{}` is moved into this call to `{}` (consuming parameter) \
                         and used again in the same argument list — the call would \
                         create two owners of one value; pass a clone (`copy {}`) for \
                         one of the uses",
                        span.loc(),
                        p.place.render(),
                        shown_callee,
                        p.place.render(),
                    ));
                }
            }
        }

        for (ai, a) in places.iter().enumerate() {
            for b in places.iter().skip(ai + 1) {
                if a.consuming || b.consuming {
                    continue;
                }
                if !a.place.overlaps(&b.place) {
                    continue;
                }
                let a_mut = mutates.get(a.slot).copied().unwrap_or(false);
                let b_mut = mutates.get(b.slot).copied().unwrap_or(false);
                if a_mut || b_mut {
                    if a.place.proj == b.place.proj {
                        return Err(format!(
                            "{}: `{}` is passed twice to `{}`, and the call mutates it \
                             through one of those parameters — the two parameters would \
                             alias one value while it is being modified; pass a clone \
                             (`copy {}`) for the read-only use",
                            span.loc(),
                            a.place.render(),
                            shown_callee,
                            a.place.render(),
                        ));
                    }
                    return Err(format!(
                        "{}: `{}` and `{}` overlap, and this call to `{}` mutates one \
                         of them — the parameters would alias one value while it is \
                         being modified; pass a clone (`copy`) for the read-only use",
                        span.loc(),
                        a.place.render(),
                        b.place.render(),
                        shown_callee,
                    ));
                }
            }
        }

        for p in &places {
            if !mutates.get(p.slot).copied().unwrap_or(false) {
                continue;
            }
            if let Some(entry) = self.iter_borrow_conflict(&p.place) {
                return Err(format!(
                    "{}: cannot pass `{}` to `{}`, which mutates it, while {}; {}",
                    span.loc(),
                    p.place.render(),
                    shown_callee,
                    entry.clause(&p.place),
                    entry.help(),
                ));
            }
        }
        Ok(())
    }

    fn record_take_moves_in_expr(&mut self, expr: &hir::Expr) -> Result<(), String> {
        match &expr.kind {
            hir::ExprKind::Call(_, name, args) => {
                if let Some(access) = self.fn_param_access.get(name).cloned() {
                    let parts: Vec<&hir::Expr> = args.iter().collect();
                    self.check_call_arg_aliasing(*name, &access, &parts, false, expr.span)?;
                    for (i, a) in args.iter().enumerate() {
                        if matches!(access.get(i), Some(Some(crate::ast::AccessMod::Take)))
                            && let hir::ExprKind::Var(id, vname) = &a.kind
                        {
                            let resolved = self.infer_ctx.resolve(&a.ty);
                            if Self::expr_type_needs_drop(&resolved) {
                                self.mark_var_moved_checked(
                                    *id,
                                    *vname,
                                    crate::typer::MoveReason::ConsumingCall(*name, i),
                                    a.span,
                                )?;
                            }
                        }
                    }
                }
                for a in args {
                    self.record_take_moves_in_expr(a)?;
                }
            }
            hir::ExprKind::VecMethod(recv, meth, args)
            | hir::ExprKind::MapMethod(recv, meth, args) => {
                let m_owned = meth.as_str();
                let m: &str = m_owned.as_ref();
                let is_mutating = crate::typer::mutate_infer::is_builtin_mutating_method(m);
                if is_mutating {
                    if self.is_container_element_read(recv) {
                        return Err(format!(
                            "{}: `{}` mutates a temporary copy — reading a container \
                             element copies it, so the mutation is silently lost; \
                             mutate through the owning container (e.g. `set`), or bind \
                             the element to a variable, modify it, and write it back",
                            expr.span.loc(),
                            m,
                        ));
                    }
                    if let Some(pl) = crate::typer::place::place_of_expr(recv)
                        && let Some(entry) = self.iter_borrow_conflict(&pl)
                    {
                        return Err(format!(
                            "{}: cannot call `{}` on `{}` while {} — the borrow would \
                             observe (or outlive) the modification; {}",
                            expr.span.loc(),
                            m,
                            pl.render(),
                            entry.clause(&pl),
                            entry.help(),
                        ));
                    }
                }
                if matches!(
                    m,
                    "push"
                        | "push_back"
                        | "push_front"
                        | "insert"
                        | "append"
                        | "add"
                        | "put"
                        | "set"
                        | "enqueue"
                ) {
                    for a in args {
                        if let hir::ExprKind::Var(id, vname) = &a.kind {
                            if let hir::ExprKind::Var(rid, _) = &recv.kind
                                && rid == id
                            {
                                return Err(format!(
                                    "{}: cannot `{}` `{}` into itself — the container \
                                     would own itself and the original binding would \
                                     dangle; insert a clone (`copy {}`) instead",
                                    expr.span.loc(),
                                    m,
                                    vname,
                                    vname,
                                ));
                            }
                            let resolved = self.infer_ctx.resolve(&a.ty);
                            let owned = !self.current_fn_param_ids.contains(id)
                                && self
                                    .find_var(&vname.as_str())
                                    .map(|v| matches!(v.ownership, crate::hir::Ownership::Owned))
                                    .unwrap_or(false);
                            if owned
                                && Self::expr_type_needs_drop(&resolved)
                                && !matches!(resolved, Type::String)
                            {
                                self.mark_var_moved_checked(
                                    *id,
                                    *vname,
                                    crate::typer::MoveReason::ContainerInsert(*meth, a.span),
                                    a.span,
                                )?;
                            }
                        }
                    }
                }
                self.record_take_moves_in_expr(recv)?;
                for a in args {
                    self.record_take_moves_in_expr(a)?;
                }
            }
            hir::ExprKind::Method(recv, mangled_name, m_name, args) => {
                let mangled: crate::intern::Symbol = *mangled_name;
                let recv_mutated = self
                    .fn_param_mutates
                    .get(&mangled)
                    .and_then(|s| s.first().copied())
                    .unwrap_or(false);
                if recv_mutated && self.is_container_element_read(recv) {
                    return Err(format!(
                        "{}: `{}` mutates its receiver, but the receiver here is a \
                         temporary copy — reading a container element copies it, so the \
                         mutation is silently lost; bind the element to a variable, \
                         modify it, and write it back",
                        expr.span.loc(),
                        m_name,
                    ));
                }
                if recv_mutated
                    && let Some(pl) = crate::typer::place::place_of_expr(recv)
                    && let Some(entry) = self.iter_borrow_conflict(&pl)
                {
                    return Err(format!(
                        "{}: cannot call `{}` on `{}` while {} — the method mutates \
                         its receiver; {}",
                        expr.span.loc(),
                        m_name,
                        pl.render(),
                        entry.clause(&pl),
                        entry.help(),
                    ));
                }
                if let Some(access) = self.fn_param_access.get(&mangled).cloned() {
                    let mut parts: Vec<&hir::Expr> = Vec::with_capacity(args.len() + 1);
                    parts.push(recv);
                    parts.extend(args.iter());
                    self.check_call_arg_aliasing(mangled, &access, &parts, true, expr.span)?;
                    if matches!(access.first(), Some(Some(crate::ast::AccessMod::Take)))
                        && let hir::ExprKind::Var(id, vname) = &recv.kind
                    {
                        let resolved = self.infer_ctx.resolve(&recv.ty);
                        if Self::expr_type_needs_drop(&resolved) {
                            self.mark_var_moved_checked(
                                *id,
                                *vname,
                                crate::typer::MoveReason::ConsumingCall(mangled, 0),
                                recv.span,
                            )?;
                        }
                    }
                    for (i, a) in args.iter().enumerate() {
                        if matches!(access.get(i + 1), Some(Some(crate::ast::AccessMod::Take)))
                            && let hir::ExprKind::Var(id, vname) = &a.kind
                        {
                            let resolved = self.infer_ctx.resolve(&a.ty);
                            if Self::expr_type_needs_drop(&resolved) {
                                self.mark_var_moved_checked(
                                    *id,
                                    *vname,
                                    crate::typer::MoveReason::ConsumingCall(mangled, i + 1),
                                    a.span,
                                )?;
                            }
                        }
                    }
                }
                self.record_take_moves_in_expr(recv)?;
                for a in args {
                    self.record_take_moves_in_expr(a)?;
                }
            }
            hir::ExprKind::ChannelSend(ch, v) => {
                let src = Self::peel_move_wrappers(v);
                if let hir::ExprKind::Var(id, vname) = &src.kind {
                    let resolved = self.infer_ctx.resolve(&src.ty);
                    if self.type_is_aggregate(&resolved) {
                        self.mark_var_moved_checked(
                            *id,
                            *vname,
                            crate::typer::MoveReason::Sent(expr.span),
                            expr.span,
                        )?;
                    }
                }
                self.record_take_moves_in_expr(ch)?;
                self.record_take_moves_in_expr(v)?;
            }

            hir::ExprKind::Send(actor, _, _, _, args) => {
                for a in args {
                    let src = Self::peel_move_wrappers(a);
                    if let hir::ExprKind::Var(id, vname) = &src.kind {
                        let resolved = self.infer_ctx.resolve(&src.ty);
                        if self.type_is_aggregate(&resolved) {
                            self.mark_var_moved_checked(
                                *id,
                                *vname,
                                crate::typer::MoveReason::TaskCapture(expr.span),
                                expr.span,
                            )?;
                        }
                    }
                }
                self.record_take_moves_in_expr(actor)?;
                for a in args {
                    self.record_take_moves_in_expr(a)?;
                }
            }
            hir::ExprKind::Spawn(_, inits) => {
                for (_, v) in inits {
                    let src = Self::peel_move_wrappers(v);
                    if let hir::ExprKind::Var(id, vname) = &src.kind {
                        let resolved = self.infer_ctx.resolve(&src.ty);
                        if self.type_is_aggregate(&resolved) {
                            self.mark_var_moved_checked(
                                *id,
                                *vname,
                                crate::typer::MoveReason::TaskCapture(expr.span),
                                expr.span,
                            )?;
                        }
                    }
                    self.record_take_moves_in_expr(v)?;
                }
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                self.record_take_moves_in_expr(callee)?;
                for a in args {
                    self.record_take_moves_in_expr(a)?;
                }
            }
            hir::ExprKind::BinOp(l, _, r) | hir::ExprKind::Index(l, r) => {
                self.record_take_moves_in_expr(l)?;
                self.record_take_moves_in_expr(r)?;
            }
            hir::ExprKind::UnaryOp(_, x)
            | hir::ExprKind::Field(x, _, _)
            | hir::ExprKind::Cast(x, _)
            | hir::ExprKind::StrictCast(x, _)
            | hir::ExprKind::Coerce(x, _)
            | hir::ExprKind::Ref(x)
            | hir::ExprKind::Deref(x) => {
                self.record_take_moves_in_expr(x)?;
            }
            hir::ExprKind::Ternary(c, t, e) => {
                self.record_take_moves_in_expr(c)?;
                let pre = self.snapshot_moved_fields();
                self.record_take_moves_in_expr(t)?;
                let t_end = self.snapshot_moved_fields();
                self.restore_moved_fields(pre.clone());
                self.record_take_moves_in_expr(e)?;
                let e_end = self.snapshot_moved_fields();
                self.restore_moved_fields(pre);
                self.merge_moved_fields_union(&[t_end, e_end]);
            }
            hir::ExprKind::Block(stmts) => {
                for s in stmts {
                    match s {
                        hir::Stmt::Bind(b) => self.record_take_moves_in_expr(&b.value)?,
                        other => self.record_take_moves_in_stmt(other)?,
                    }
                }
            }
            hir::ExprKind::Pipe(first, _, name, rest) => {
                if let Some(access) = self.fn_param_access.get(name).cloned() {
                    let mut parts: Vec<&hir::Expr> = Vec::with_capacity(rest.len() + 1);
                    parts.push(first);
                    parts.extend(rest.iter());
                    self.check_call_arg_aliasing(*name, &access, &parts, false, expr.span)?;
                    for (i, a) in parts.iter().enumerate() {
                        if matches!(access.get(i), Some(Some(crate::ast::AccessMod::Take)))
                            && let hir::ExprKind::Var(id, vname) = &a.kind
                        {
                            let resolved = self.infer_ctx.resolve(&a.ty);
                            if Self::expr_type_needs_drop(&resolved) {
                                self.mark_var_moved_checked(
                                    *id,
                                    *vname,
                                    crate::typer::MoveReason::ConsumingCall(*name, i),
                                    a.span,
                                )?;
                            }
                        }
                    }
                }
                self.record_take_moves_in_expr(first)?;
                for a in rest {
                    self.record_take_moves_in_expr(a)?;
                }
            }
            hir::ExprKind::StringMethod(recv, _, args)
            | hir::ExprKind::DeferredMethod(recv, _, args) => {
                self.record_take_moves_in_expr(recv)?;
                for a in args {
                    self.record_take_moves_in_expr(a)?;
                }
            }
            hir::ExprKind::Tuple(xs) | hir::ExprKind::Array(xs) | hir::ExprKind::VecNew(xs) => {
                for x in xs {
                    self.record_ctor_capture(x)?;
                    self.record_take_moves_in_expr(x)?;
                }
            }
            hir::ExprKind::Builtin(_, xs) => {
                for x in xs {
                    self.record_take_moves_in_expr(x)?;
                }
            }
            hir::ExprKind::Struct(_, fields) => {
                for fi in fields {
                    self.record_ctor_capture(&fi.value)?;
                    self.record_take_moves_in_expr(&fi.value)?;
                }
            }
            hir::ExprKind::VariantCtor(_, _, _, fields) => {
                for fi in fields {
                    self.record_ctor_capture(&fi.value)?;
                    self.record_take_moves_in_expr(&fi.value)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn collect_ctor_captured_id(
        &mut self,
        value: &hir::Expr,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        let src = Self::peel_move_wrappers(value);
        if let hir::ExprKind::Var(id, _) = &src.kind {
            let resolved = self.infer_ctx.resolve(&src.ty);
            if self.type_is_aggregate(&resolved) {
                out.insert(*id);
            }
        }
    }

    fn record_ctor_capture(&mut self, value: &hir::Expr) -> Result<(), String> {
        let src = Self::peel_move_wrappers(value);
        {
            let was_strict = self.infer_ctx.is_strict();
            self.infer_ctx.set_strict(false);
            let resolved = self.infer_ctx.resolve(&src.ty);
            self.infer_ctx.set_strict(was_strict);
            if crate::typer::expr::views::type_contains_view(&resolved) {
                return Err(format!(
                    "{}: a view cannot be stored in a constructed value — views are \
                     second-class borrows that never escape; store the owning \
                     container or a copied slice instead",
                    src.span.loc(),
                ));
            }
        }
        if let hir::ExprKind::Var(id, vname) = &src.kind {
            let resolved = self.infer_ctx.resolve(&src.ty);
            if self.type_is_aggregate(&resolved) {
                self.mark_var_moved_checked(
                    *id,
                    *vname,
                    crate::typer::MoveReason::CtorCapture(src.span),
                    src.span,
                )?;
            }
        }
        Ok(())
    }

    pub(in crate::typer) fn strip_drops_for(
        body: &mut Vec<hir::Stmt>,
        ids: &std::collections::HashSet<crate::hir::DefId>,
    ) {
        body.retain(|s| !matches!(s, hir::Stmt::Drop(id, _, _, _) if ids.contains(id)));
        for s in body.iter_mut() {
            match s {
                hir::Stmt::If(i) => {
                    Self::strip_drops_for(&mut i.then, ids);
                    for (_, b) in i.elifs.iter_mut() {
                        Self::strip_drops_for(b, ids);
                    }
                    if let Some(b) = i.els.as_mut() {
                        Self::strip_drops_for(b, ids);
                    }
                }
                hir::Stmt::Match(m) => {
                    for a in m.arms.iter_mut() {
                        Self::strip_drops_for(&mut a.body, ids);
                    }
                }
                hir::Stmt::While(w) => Self::strip_drops_for(&mut w.body, ids),
                hir::Stmt::For(f) => Self::strip_drops_for(&mut f.body, ids),
                hir::Stmt::Loop(l) => Self::strip_drops_for(&mut l.body, ids),
                _ => {}
            }
        }
    }

    fn expand_payload_bind_links(&self, ids: &mut std::collections::HashSet<crate::hir::DefId>) {
        let mut added: Vec<crate::hir::DefId> = Vec::new();
        for id in ids.iter() {
            let mut cur = *id;
            while let Some(pl) = self.payload_bind_subjects.get(&cur) {
                if ids.contains(&pl.root) || added.contains(&pl.root) {
                    break;
                }
                added.push(pl.root);
                cur = pl.root;
            }
        }
        ids.extend(added);
    }

    pub(in crate::typer) fn emit_scope_drops_excluding(
        &mut self,
        stmts: &mut Vec<hir::Stmt>,
        exclude: &std::collections::HashSet<crate::hir::DefId>,
    ) {
        let scope_entries: Vec<(crate::intern::Symbol, crate::typer::VarInfo)> =
            match self.scopes.last() {
                Some(s) => s.iter().map(|(n, v)| (*n, v.clone())).collect(),
                None => return,
            };

        let mut consumed: std::collections::HashSet<crate::hir::DefId> = exclude.clone();
        self.collect_block_consumed_ids(stmts, &mut consumed);
        self.expand_payload_bind_links(&mut consumed);

        let mut resolved_entries: Vec<(crate::intern::Symbol, crate::typer::VarInfo, Type)> =
            Vec::with_capacity(scope_entries.len());

        let was_strict = self.infer_ctx.is_strict();
        self.infer_ctx.set_strict(false);
        for (name, info) in scope_entries {
            let resolved = self.infer_ctx.resolve(&info.ty);
            resolved_entries.push((name, info, resolved));
        }
        self.infer_ctx.set_strict(was_strict);
        let mut drops: Vec<_> = resolved_entries
            .into_iter()
            .filter(|(_, info, resolved)| {
                self.needs_drop(resolved)
                    && !matches!(
                        info.ownership,
                        crate::hir::Ownership::Borrowed | crate::hir::Ownership::BorrowMut
                    )
                    && !consumed.contains(&info.def_id)
            })
            .collect();
        drops.sort_by_key(|(_, info, _)| std::cmp::Reverse(info.def_id.0));
        for (name, info, resolved) in drops {
            stmts.push(hir::Stmt::Drop(
                info.def_id,
                name,
                resolved,
                crate::ast::Span::dummy(),
            ));
        }
    }

    pub(in crate::typer) fn collect_moved_var_ids(
        expr: &hir::Expr,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match &expr.kind {
            hir::ExprKind::Var(id, _) => {
                out.insert(*id);
            }
            hir::ExprKind::Struct(_, inits) | hir::ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    Self::collect_moved_var_ids(&fi.value, out);
                }
            }
            hir::ExprKind::Tuple(es) | hir::ExprKind::Array(es) => {
                for e in es {
                    Self::collect_moved_var_ids(e, out);
                }
            }
            _ => {}
        }
    }

    pub(in crate::typer) fn collect_hir_var_ids_expr(
        expr: &hir::Expr,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match &expr.kind {
            hir::ExprKind::Var(id, _) => {
                out.insert(*id);
            }
            hir::ExprKind::BinOp(l, _, r) => {
                Self::collect_hir_var_ids_expr(l, out);
                Self::collect_hir_var_ids_expr(r, out);
            }
            hir::ExprKind::UnaryOp(_, e) => Self::collect_hir_var_ids_expr(e, out),
            hir::ExprKind::Call(_, _, args) => {
                for a in args {
                    Self::collect_hir_var_ids_expr(a, out);
                }
            }
            hir::ExprKind::Struct(_, inits) | hir::ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    Self::collect_hir_var_ids_expr(&fi.value, out);
                }
            }
            hir::ExprKind::IfExpr(i) => {
                Self::collect_hir_var_ids_expr(&i.cond, out);
                for s in &i.then {
                    Self::collect_hir_var_ids_stmt(s, out);
                }
                for (c, b) in &i.elifs {
                    Self::collect_hir_var_ids_expr(c, out);
                    for s in b {
                        Self::collect_hir_var_ids_stmt(s, out);
                    }
                }
                if let Some(b) = &i.els {
                    for s in b {
                        Self::collect_hir_var_ids_stmt(s, out);
                    }
                }
            }
            hir::ExprKind::Index(e, i) => {
                Self::collect_hir_var_ids_expr(e, out);
                Self::collect_hir_var_ids_expr(i, out);
            }
            hir::ExprKind::Field(e, _, _) => Self::collect_hir_var_ids_expr(e, out),
            hir::ExprKind::Method(e, _, _, args)
            | hir::ExprKind::StringMethod(e, _, args)
            | hir::ExprKind::DeferredMethod(e, _, args)
            | hir::ExprKind::VecMethod(e, _, args)
            | hir::ExprKind::MapMethod(e, _, args) => {
                Self::collect_hir_var_ids_expr(e, out);
                for a in args {
                    Self::collect_hir_var_ids_expr(a, out);
                }
            }
            hir::ExprKind::Tuple(es) | hir::ExprKind::Array(es) => {
                for e in es {
                    Self::collect_hir_var_ids_expr(e, out);
                }
            }
            hir::ExprKind::Block(stmts) => {
                for s in stmts {
                    Self::collect_hir_var_ids_stmt(s, out);
                }
            }
            hir::ExprKind::Lambda(_, body) => {
                for s in body {
                    Self::collect_hir_var_ids_stmt(s, out);
                }
            }
            hir::ExprKind::Ref(e) | hir::ExprKind::Deref(e) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::ExprKind::Pipe(e, _, _, rest) => {
                Self::collect_hir_var_ids_expr(e, out);
                for a in rest {
                    Self::collect_hir_var_ids_expr(a, out);
                }
            }
            hir::ExprKind::Cast(e, _) => Self::collect_hir_var_ids_expr(e, out),
            hir::ExprKind::Builtin(_, xs) | hir::ExprKind::VecNew(xs) => {
                for e in xs {
                    Self::collect_hir_var_ids_expr(e, out);
                }
            }
            hir::ExprKind::Coerce(e, _) | hir::ExprKind::StrictCast(e, _) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                Self::collect_hir_var_ids_expr(callee, out);
                for a in args {
                    Self::collect_hir_var_ids_expr(a, out);
                }
            }
            hir::ExprKind::Ternary(c, t, e) => {
                Self::collect_hir_var_ids_expr(c, out);
                Self::collect_hir_var_ids_expr(t, out);
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::ExprKind::ChannelSend(ch, v) => {
                Self::collect_hir_var_ids_expr(ch, out);
                Self::collect_hir_var_ids_expr(v, out);
            }
            _ => {}
        }
    }

    pub(in crate::typer) fn collect_hir_var_ids_stmt(
        stmt: &hir::Stmt,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        Self::collect_hir_var_ids_stmt_inner(stmt, out, false);
    }

    fn collect_hir_var_ids_stmt_inner(
        stmt: &hir::Stmt,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
        shield_copies: bool,
    ) {
        match stmt {
            hir::Stmt::Bind(b) => {
                if shield_copies
                    && matches!(b.access_mod, Some(crate::ast::AccessMod::Copy))
                    && matches!(
                        Self::peel_move_wrappers(&b.value).kind,
                        hir::ExprKind::Var(..)
                    )
                {
                    return;
                }
                Self::collect_hir_var_ids_expr(&b.value, out);
            }
            hir::Stmt::Expr(e) | hir::Stmt::TupleBind(_, e, _) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::Stmt::Ret(Some(e), _, _)
            | hir::Stmt::Break(Some(e), _)
            | hir::Stmt::ErrReturn(e, _, _)
            | hir::Stmt::ChannelClose(e, _)
            | hir::Stmt::Stop(e, _)
            | hir::Stmt::Join(e, _) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::Stmt::Assign(t, v, _) => {
                Self::collect_hir_var_ids_expr(t, out);
                Self::collect_hir_var_ids_expr(v, out);
            }
            hir::Stmt::If(i) => {
                Self::collect_hir_var_ids_expr(&i.cond, out);
                for s in &i.then {
                    Self::collect_hir_var_ids_stmt_inner(s, out, shield_copies);
                }
                for (c, b) in &i.elifs {
                    Self::collect_hir_var_ids_expr(c, out);
                    for s in b {
                        Self::collect_hir_var_ids_stmt_inner(s, out, shield_copies);
                    }
                }
                if let Some(b) = &i.els {
                    for s in b {
                        Self::collect_hir_var_ids_stmt_inner(s, out, shield_copies);
                    }
                }
            }
            hir::Stmt::While(w) => {
                Self::collect_hir_var_ids_expr(&w.cond, out);
                for s in &w.body {
                    Self::collect_hir_var_ids_stmt_inner(s, out, shield_copies);
                }
            }
            hir::Stmt::For(f) | hir::Stmt::SimFor(f, _) => {
                Self::collect_hir_var_ids_expr(&f.iter, out);
                if let Some(e) = &f.end {
                    Self::collect_hir_var_ids_expr(e, out);
                }
                if let Some(e) = &f.step {
                    Self::collect_hir_var_ids_expr(e, out);
                }
                for s in &f.body {
                    Self::collect_hir_var_ids_stmt_inner(s, out, shield_copies);
                }
            }
            hir::Stmt::Loop(l) => {
                for s in &l.body {
                    Self::collect_hir_var_ids_stmt_inner(s, out, shield_copies);
                }
            }
            hir::Stmt::Match(m) => {
                Self::collect_hir_var_ids_expr(&m.subject, out);
                for arm in &m.arms {
                    if let Some(g) = &arm.guard {
                        Self::collect_hir_var_ids_expr(g, out);
                    }
                    for s in &arm.body {
                        Self::collect_hir_var_ids_stmt_inner(s, out, shield_copies);
                    }
                }
            }
            hir::Stmt::Defer(b, _)
            | hir::Stmt::Transaction(b, _)
            | hir::Stmt::SimBlock(b, _)
            | hir::Stmt::Together(_, b, _, _, _) => {
                for s in b {
                    Self::collect_hir_var_ids_stmt_inner(s, out, shield_copies);
                }
            }
            hir::Stmt::StoreInsert(_, es, _) => {
                for e in es {
                    Self::collect_hir_var_ids_expr(e, out);
                }
            }
            hir::Stmt::StoreSet(_, sets, _, _) => {
                for (_, e) in sets {
                    Self::collect_hir_var_ids_expr(e, out);
                }
            }
            _ => {}
        }
    }

    pub(in crate::typer) fn needs_drop(&self, ty: &Type) -> bool {
        let mut visiting: std::collections::HashSet<crate::intern::Symbol> =
            std::collections::HashSet::new();
        self.needs_drop_inner(ty, &mut visiting)
    }

    fn needs_drop_inner(
        &self,
        ty: &Type,
        visiting: &mut std::collections::HashSet<crate::intern::Symbol>,
    ) -> bool {
        if matches!(
            ty,
            Type::String
                | Type::Vec(_)
                | Type::Map(_, _)
                | Type::Coroutine(_)
                | Type::Generator(_)
                | Type::Channel(_)
                | Type::Fn(_, _)
        ) {
            return true;
        }
        match ty {
            Type::Struct(name, args) => {
                if self
                    .struct_attrs
                    .get(name)
                    .map(|a| a.resource)
                    .unwrap_or(false)
                {
                    return true;
                }
                if !self.structs.contains_key(name) && self.enums.contains_key(name) {
                    return self.needs_drop_inner(&Type::Enum(*name), visiting);
                }
                if !visiting.insert(*name) {
                    return false;
                }

                let result = self
                    .struct_field_types(name, args)
                    .into_iter()
                    .any(|fty| self.needs_drop_inner(&fty, visiting));
                visiting.remove(name);
                result
            }
            Type::Enum(name) => {
                if !visiting.insert(*name) {
                    return false;
                }
                let result = if let Some(variants) = self.enums.get(name) {
                    variants.iter().any(|(_vname, ftys)| {
                        ftys.iter().any(|t| self.needs_drop_inner(t, visiting))
                    })
                } else {
                    false
                };
                visiting.remove(name);
                result
            }
            Type::Tuple(elts) => elts.iter().any(|t| self.needs_drop_inner(t, visiting)),
            Type::Array(elem, _) => self.needs_drop_inner(elem, visiting),
            Type::Row(name) => {
                let rec = crate::intern::Symbol::intern(&format!("__store_{name}"));
                self.needs_drop_inner(&Type::Struct(rec, vec![]), visiting)
            }
            Type::Alias(_, inner) | Type::Newtype(_, inner) | Type::Frozen(inner) => {
                self.needs_drop_inner(inner, visiting)
            }
            _ => false,
        }
    }

    pub(in crate::typer) fn struct_field_types(
        &self,
        name: &crate::intern::Symbol,
        args: &[Type],
    ) -> Vec<Type> {
        if let Some(fields) = self.structs.get(name) {
            if args.is_empty() {
                return fields.iter().map(|(_, ty)| ty.clone()).collect();
            }

            if let Some(generic_def) = self.generic_types.get(name) {
                let params = &generic_def.type_params;
                if params.len() == args.len() {
                    let subs: std::collections::HashMap<crate::intern::Symbol, Type> = params
                        .iter()
                        .zip(args.iter())
                        .map(|(p, t)| (*p, t.clone()))
                        .collect();
                    return fields
                        .iter()
                        .map(|(_, ty)| Self::subst_type(ty, &subs))
                        .collect();
                }
            }
            return fields.iter().map(|(_, ty)| ty.clone()).collect();
        }
        Vec::new()
    }

    fn subst_type(
        ty: &Type,
        subs: &std::collections::HashMap<crate::intern::Symbol, Type>,
    ) -> Type {
        match ty {
            Type::Param(name) => subs.get(name).cloned().unwrap_or_else(|| ty.clone()),
            Type::Vec(inner) => Type::Vec(Box::new(Self::subst_type(inner, subs))),
            Type::Coroutine(inner) => Type::Coroutine(Box::new(Self::subst_type(inner, subs))),
            Type::Generator(inner) => Type::Generator(Box::new(Self::subst_type(inner, subs))),
            Type::Channel(inner) => Type::Channel(Box::new(Self::subst_type(inner, subs))),
            Type::Map(k, v) => Type::Map(
                Box::new(Self::subst_type(k, subs)),
                Box::new(Self::subst_type(v, subs)),
            ),
            Type::Tuple(elts) => {
                Type::Tuple(elts.iter().map(|t| Self::subst_type(t, subs)).collect())
            }
            Type::Array(elem, n) => Type::Array(Box::new(Self::subst_type(elem, subs)), *n),
            Type::Struct(name, ts) => Type::Struct(
                *name,
                ts.iter().map(|t| Self::subst_type(t, subs)).collect(),
            ),
            Type::Alias(name, inner) => Type::Alias(*name, Box::new(Self::subst_type(inner, subs))),
            Type::Newtype(name, inner) => {
                Type::Newtype(*name, Box::new(Self::subst_type(inner, subs)))
            }
            Type::View(inner) => Type::View(Box::new(Self::subst_type(inner, subs))),
            Type::Frozen(inner) => Type::Frozen(Box::new(Self::subst_type(inner, subs))),
            _ => ty.clone(),
        }
    }
}
