use super::super::Typer;
use crate::ast;
use crate::hir;
use crate::types::Type;

impl Typer {
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
        let ends_with_jump = stmts.last().map_or(false, |s| {
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
        let ends_with_jump = stmts.last().map_or(false, |s| {
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

    pub(in crate::typer) fn finalize_loop_body_drops(&mut self, stmts: &mut Vec<hir::Stmt>) {
        let ends_with_jump = stmts.last().map_or(false, |s| {
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

    fn collect_block_consumed_ids(
        &mut self,
        stmts: &[hir::Stmt],
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        for s in stmts {
            match s {
                hir::Stmt::Expr(e) => self.collect_consumed_in_expr(e, out),

                hir::Stmt::Assign(_target, value, _) => {
                    let resolved = self.infer_ctx.resolve(&value.ty);
                    if Self::expr_type_needs_drop(&resolved) {
                        if let hir::ExprKind::Var(id, _) = &value.kind {
                            out.insert(*id);
                        }
                    }
                }
                hir::Stmt::Bind(b) => {
                    if !matches!(b.access_mod, Some(crate::ast::AccessMod::Copy)) {
                        let resolved = self.infer_ctx.resolve(&b.value.ty);
                        if Self::expr_type_needs_drop(&resolved) {
                            if let hir::ExprKind::Var(id, _) = &b.value.kind {
                                out.insert(*id);
                            }
                        }
                    }
                }
                _ => {}
            }
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
                        if Self::expr_type_needs_drop(&resolved)
                            && matches!(a.kind, hir::ExprKind::Var(_, _))
                        {
                            if let hir::ExprKind::Var(id, _) = &a.kind {
                                out.insert(*id);
                            }
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

            hir::ExprKind::Method(recv, ty_name, m_name, args) => {
                let mangled: crate::intern::Symbol =
                    format!("{}_{}", ty_name.as_str(), m_name.as_str()).into();
                let access = self.fn_param_access.get(&mangled).cloned();
                if let Some(access) = access {
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
            _ => {}
        }
    }

    fn expr_type_needs_drop(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Vec(_) | Type::Map(_, _) | Type::String | Type::Struct(_, _) | Type::Enum(_)
        )
    }

    pub(in crate::typer) fn record_take_moves_in_stmt(&mut self, s: &hir::Stmt) {
        match s {
            hir::Stmt::Bind(b) => {
                if matches!(b.access_mod, Some(crate::ast::AccessMod::Take))
                    && let hir::ExprKind::Var(id, _) = &b.value.kind
                {
                    let resolved = self.infer_ctx.resolve(&b.value.ty);
                    if Self::expr_type_needs_drop(&resolved) {
                        self.mark_var_moved(*id);
                    }
                }
                self.record_take_moves_in_expr(&b.value);
            }
            hir::Stmt::Expr(e)
            | hir::Stmt::Ret(Some(e), _, _)
            | hir::Stmt::ErrReturn(e, _, _)
            | hir::Stmt::Break(Some(e), _) => {
                self.record_take_moves_in_expr(e);
            }
            hir::Stmt::Assign(target, value, _) => {
                self.record_take_moves_in_expr(value);
                self.record_take_moves_in_expr(target);
            }

            _ => {}
        }
    }

    fn record_take_moves_in_expr(&mut self, expr: &hir::Expr) {
        match &expr.kind {
            hir::ExprKind::Call(_, name, args) => {
                if let Some(access) = self.fn_param_access.get(name).cloned() {
                    for (i, a) in args.iter().enumerate() {
                        if matches!(access.get(i), Some(Some(crate::ast::AccessMod::Take)))
                            && let hir::ExprKind::Var(id, _) = &a.kind
                        {
                            let resolved = self.infer_ctx.resolve(&a.ty);
                            if Self::expr_type_needs_drop(&resolved) {
                                self.mark_var_moved(*id);
                            }
                        }
                    }
                }
                for a in args {
                    self.record_take_moves_in_expr(a);
                }
            }
            hir::ExprKind::Method(recv, ty_name, m_name, args) => {
                let mangled: crate::intern::Symbol =
                    format!("{}_{}", ty_name.as_str(), m_name.as_str()).into();
                if let Some(access) = self.fn_param_access.get(&mangled).cloned() {
                    if matches!(access.first(), Some(Some(crate::ast::AccessMod::Take)))
                        && let hir::ExprKind::Var(id, _) = &recv.kind
                    {
                        let resolved = self.infer_ctx.resolve(&recv.ty);
                        if Self::expr_type_needs_drop(&resolved) {
                            self.mark_var_moved(*id);
                        }
                    }
                    for (i, a) in args.iter().enumerate() {
                        if matches!(access.get(i + 1), Some(Some(crate::ast::AccessMod::Take)))
                            && let hir::ExprKind::Var(id, _) = &a.kind
                        {
                            let resolved = self.infer_ctx.resolve(&a.ty);
                            if Self::expr_type_needs_drop(&resolved) {
                                self.mark_var_moved(*id);
                            }
                        }
                    }
                }
                self.record_take_moves_in_expr(recv);
                for a in args {
                    self.record_take_moves_in_expr(a);
                }
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                self.record_take_moves_in_expr(callee);
                for a in args {
                    self.record_take_moves_in_expr(a);
                }
            }
            hir::ExprKind::BinOp(l, _, r) | hir::ExprKind::Index(l, r) => {
                self.record_take_moves_in_expr(l);
                self.record_take_moves_in_expr(r);
            }
            hir::ExprKind::UnaryOp(_, x)
            | hir::ExprKind::Field(x, _, _)
            | hir::ExprKind::Cast(x, _)
            | hir::ExprKind::StrictCast(x, _)
            | hir::ExprKind::Coerce(x, _)
            | hir::ExprKind::Ref(x)
            | hir::ExprKind::Deref(x) => {
                self.record_take_moves_in_expr(x);
            }
            hir::ExprKind::Ternary(c, t, e) => {
                self.record_take_moves_in_expr(c);
                self.record_take_moves_in_expr(t);
                self.record_take_moves_in_expr(e);
            }
            hir::ExprKind::Tuple(xs)
            | hir::ExprKind::Array(xs)
            | hir::ExprKind::VecNew(xs)
            | hir::ExprKind::Builtin(_, xs) => {
                for x in xs {
                    self.record_take_moves_in_expr(x);
                }
            }
            _ => {}
        }
    }

    pub(in crate::typer) fn emit_scope_drops_excluding(
        &mut self,
        stmts: &mut Vec<hir::Stmt>,
        exclude: &std::collections::HashSet<crate::hir::DefId>,
    ) {
        let scope_entries: Vec<(crate::intern::Symbol, crate::typer::VarInfo)> =
            match self.scopes.last() {
                Some(s) => s.iter().map(|(n, v)| (n.clone(), v.clone())).collect(),
                None => return,
            };

        let mut consumed: std::collections::HashSet<crate::hir::DefId> = exclude.clone();
        self.collect_block_consumed_ids(stmts, &mut consumed);

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
                name.clone(),
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
            _ => {}
        }
    }

    pub(in crate::typer) fn collect_hir_var_ids_stmt(
        stmt: &hir::Stmt,
        out: &mut std::collections::HashSet<crate::hir::DefId>,
    ) {
        match stmt {
            hir::Stmt::Expr(e) | hir::Stmt::Bind(hir::Bind { value: e, .. }) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::Stmt::Ret(Some(e), _, _)
            | hir::Stmt::Break(Some(e), _)
            | hir::Stmt::ErrReturn(e, _, _) => {
                Self::collect_hir_var_ids_expr(e, out);
            }
            hir::Stmt::Assign(t, v, _) => {
                Self::collect_hir_var_ids_expr(t, out);
                Self::collect_hir_var_ids_expr(v, out);
            }
            hir::Stmt::If(i) => {
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
                if !visiting.insert(name.clone()) {
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
                if !visiting.insert(name.clone()) {
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

            Type::Alias(_, inner) | Type::Newtype(_, inner) => {
                self.needs_drop_inner(inner, visiting)
            }
            _ => false,
        }
    }

    fn struct_field_types(&self, name: &crate::intern::Symbol, args: &[Type]) -> Vec<Type> {
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
                        .map(|(p, t)| (p.clone(), t.clone()))
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
                name.clone(),
                ts.iter().map(|t| Self::subst_type(t, subs)).collect(),
            ),
            Type::Alias(name, inner) => {
                Type::Alias(name.clone(), Box::new(Self::subst_type(inner, subs)))
            }
            Type::Newtype(name, inner) => {
                Type::Newtype(name.clone(), Box::new(Self::subst_type(inner, subs)))
            }
            _ => ty.clone(),
        }
    }
}
