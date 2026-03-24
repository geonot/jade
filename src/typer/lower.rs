use std::collections::HashMap;

use crate::ast::{self, Span};
use crate::hir::{self, DefId, Ownership};
use crate::types::Type;

use super::{Typer, VarInfo};

impl Typer {
    pub fn lower_program(&mut self, prog: &ast::Program) -> Result<hir::Program, String> {
        if self.debug_types {
            eprintln!("[type:pipeline] starting type inference and HIR lowering");
        }
        self.register_prelude_types();

        for d in &prog.decls {
            match d {
                ast::Decl::Fn(f) if Self::is_generic_fn(f) => {
                    if !f.type_bounds.is_empty() {
                        self.generic_bounds
                            .insert(f.name.clone(), f.type_bounds.clone());
                    }
                    self.generic_fns
                        .insert(f.name.clone(), Self::normalize_generic_fn(f));
                }
                ast::Decl::Fn(f) => {
                    self.declare_fn_sig(f);
                }
                ast::Decl::Type(td) if !td.type_params.is_empty() => {
                    self.generic_types.insert(td.name.clone(), td.clone());
                }
                ast::Decl::Type(td) => {
                    for m in &td.methods {
                        self.methods
                            .entry(td.name.clone())
                            .or_default()
                            .push(m.clone());
                    }
                    self.declare_type_def(td);
                    for m in &td.methods {
                        self.declare_method_sig(&td.name, m);
                    }
                }
                ast::Decl::Enum(ed) if !ed.type_params.is_empty() => {
                    self.generic_enums.insert(ed.name.clone(), ed.clone());
                }
                ast::Decl::Enum(ed) => {
                    self.declare_enum_def(ed);
                }
                ast::Decl::Extern(ef) => {
                    self.declare_extern_sig(ef);
                }
                ast::Decl::Use(_) => {}
                ast::Decl::ErrDef(ed) => {
                    self.declare_err_def_sig(ed);
                }
                ast::Decl::Test(_) => {}
                ast::Decl::Actor(ad) => {
                    self.declare_actor_def(ad);
                }
                ast::Decl::Store(sd) => {
                    let fields: Vec<(String, Type)> = sd
                        .fields
                        .iter()
                        .map(|f| (f.name.clone(), f.ty.clone().unwrap_or(Type::I64)))
                        .collect();
                    self.structs
                        .insert(format!("__store_{}", sd.name), fields.clone());
                    self.store_schemas.insert(sd.name.clone(), fields);
                }
                ast::Decl::Trait(td) => {
                    self.declare_trait_def(td);
                }
                ast::Decl::Impl(_) => {}
                ast::Decl::Const(name, expr, _) => {
                    self.consts.insert(name.clone(), expr.clone());
                }
            }
        }

        for d in &prog.decls {
            if let ast::Decl::Impl(ib) = d {
                self.declare_impl_block(ib)?;
            }
        }

        if self.debug_types {
            eprintln!("[type:pipeline] running bidirectional parameter inference");
        }
        self.infer_param_types(prog);

        if self.debug_types {
            eprintln!("[type:pipeline] lowering declarations to HIR");
        }
        let mut hir_fns = Vec::new();
        let mut hir_types = Vec::new();
        let mut hir_enums = Vec::new();
        let mut hir_externs = Vec::new();
        let mut hir_err_defs = Vec::new();
        let mut hir_actors = Vec::new();
        let mut hir_stores = Vec::new();
        let mut test_fns: Vec<(String, String)> = Vec::new();

        for d in &prog.decls {
            match d {
                ast::Decl::Fn(f) if !Self::is_generic_fn(f) => {
                    if self.test_mode && f.name == "main" {
                        continue;
                    }
                    let hfn = self.lower_fn(f)?;
                    hir_fns.push(hfn);
                }
                ast::Decl::Type(td) if td.type_params.is_empty() => {
                    let htd = self.lower_type_def(td)?;
                    hir_types.push(htd);
                }
                ast::Decl::Enum(ed) if ed.type_params.is_empty() => {
                    let hed = self.lower_enum_def(ed);
                    hir_enums.push(hed);
                }
                ast::Decl::Extern(ef) => {
                    let hef = self.lower_extern(ef);
                    hir_externs.push(hef);
                }
                ast::Decl::ErrDef(ed) => {
                    let hed = self.lower_err_def(ed);
                    hir_err_defs.push(hed);
                }
                ast::Decl::Test(tb) if self.test_mode => {
                    let fn_name = format!("__test_{}", test_fns.len());
                    let test_fn = self.lower_test_block(tb, &fn_name)?;
                    let test_id = test_fn.def_id;
                    self.fns
                        .insert(fn_name.clone(), (test_id, vec![], Type::Void));
                    hir_fns.push(test_fn);
                    test_fns.push((tb.name.clone(), fn_name));
                }
                _ => {}
            }
        }

        for d in &prog.decls {
            if let ast::Decl::Actor(ad) = d {
                let ha = self.lower_actor_def(ad)?;
                hir_actors.push(ha);
            }
        }

        for d in &prog.decls {
            if let ast::Decl::Store(sd) = d {
                let hs = self.lower_store_def(sd)?;
                hir_stores.push(hs);
            }
        }

        let mut hir_trait_impls = Vec::new();
        for d in &prog.decls {
            if let ast::Decl::Impl(ib) = d {
                let hi = self.lower_impl_block(ib)?;
                hir_trait_impls.push(hi);
            }
        }

        if self.test_mode && !test_fns.is_empty() {
            let main_fn = self.build_test_runner(&test_fns);
            self.fns
                .insert("main".into(), (main_fn.def_id, vec![], Type::I32));
            hir_fns.push(main_fn);
        }

        hir_fns.extend(self.mono_fns.drain(..));
        hir_enums.extend(self.mono_enums.drain(..));

        let mut program = hir::Program {
            fns: hir_fns,
            types: hir_types,
            enums: hir_enums,
            externs: hir_externs,
            err_defs: hir_err_defs,
            actors: hir_actors,
            stores: hir_stores,
            trait_impls: hir_trait_impls,
        };
        self.resolve_all_types(&mut program);
        if self.debug_types {
            eprintln!(
                "[type:pipeline] complete: {} fns, {} types, {} enums",
                program.fns.len(),
                program.types.len(),
                program.enums.len()
            );
        }
        Ok(program)
    }

    fn resolve_all_types(&mut self, prog: &mut hir::Program) {
        for f in &mut prog.fns {
            self.resolve_fn(f);
        }
        for td in &mut prog.types {
            for field in &mut td.fields {
                field.ty = self.infer_ctx.resolve(&field.ty);
                if let Some(def) = &mut field.default {
                    self.resolve_expr(def);
                }
            }
            for m in &mut td.methods {
                self.resolve_fn(m);
            }
        }
        for ed in &mut prog.enums {
            for v in &mut ed.variants {
                for vf in &mut v.fields {
                    vf.ty = self.infer_ctx.resolve(&vf.ty);
                }
            }
        }
        for ef in &mut prog.externs {
            ef.ret = self.infer_ctx.resolve(&ef.ret);
            for (_, ty) in &mut ef.params {
                *ty = self.infer_ctx.resolve(ty);
            }
        }
        for errdef in &mut prog.err_defs {
            for v in &mut errdef.variants {
                for ft in &mut v.fields {
                    *ft = self.infer_ctx.resolve(ft);
                }
            }
        }
        for ad in &mut prog.actors {
            for field in &mut ad.fields {
                field.ty = self.infer_ctx.resolve(&field.ty);
                if let Some(def) = &mut field.default {
                    self.resolve_expr(def);
                }
            }
            for h in &mut ad.handlers {
                for p in &mut h.params {
                    p.ty = self.infer_ctx.resolve(&p.ty);
                }
                self.resolve_block(&mut h.body);
            }
        }
        for sd in &mut prog.stores {
            for field in &mut sd.fields {
                field.ty = self.infer_ctx.resolve(&field.ty);
            }
        }
        for ti in &mut prog.trait_impls {
            for m in &mut ti.methods {
                self.resolve_fn(m);
            }
        }
    }

    fn resolve_fn(&mut self, f: &mut hir::Fn) {
        f.ret = self.infer_ctx.resolve(&f.ret);
        for p in &mut f.params {
            p.ty = self.infer_ctx.resolve(&p.ty);
        }
        self.resolve_block(&mut f.body);
    }

    fn resolve_block(&mut self, block: &mut hir::Block) {
        for stmt in block {
            self.resolve_stmt(stmt);
        }
    }

    fn resolve_stmt(&mut self, stmt: &mut hir::Stmt) {
        match stmt {
            hir::Stmt::Bind(b) => {
                b.ty = self.infer_ctx.resolve(&b.ty);
                self.resolve_expr(&mut b.value);
            }
            hir::Stmt::TupleBind(bindings, expr, _) => {
                for (_, _, ty) in bindings {
                    *ty = self.infer_ctx.resolve(ty);
                }
                self.resolve_expr(expr);
            }
            hir::Stmt::Assign(lhs, rhs, _) => {
                self.resolve_expr(lhs);
                self.resolve_expr(rhs);
            }
            hir::Stmt::Expr(e) => self.resolve_expr(e),
            hir::Stmt::If(if_stmt) => {
                self.resolve_expr(&mut if_stmt.cond);
                self.resolve_block(&mut if_stmt.then);
                for (cond, block) in &mut if_stmt.elifs {
                    self.resolve_expr(cond);
                    self.resolve_block(block);
                }
                if let Some(els) = &mut if_stmt.els {
                    self.resolve_block(els);
                }
            }
            hir::Stmt::While(w) => {
                self.resolve_expr(&mut w.cond);
                self.resolve_block(&mut w.body);
            }
            hir::Stmt::For(f) => {
                f.bind_ty = self.infer_ctx.resolve(&f.bind_ty);
                self.resolve_expr(&mut f.iter);
                if let Some(end) = &mut f.end {
                    self.resolve_expr(end);
                }
                if let Some(step) = &mut f.step {
                    self.resolve_expr(step);
                }
                self.resolve_block(&mut f.body);
            }
            hir::Stmt::Loop(l) => {
                self.resolve_block(&mut l.body);
            }
            hir::Stmt::Ret(expr, ty, _) => {
                *ty = self.infer_ctx.resolve(ty);
                if let Some(e) = expr {
                    self.resolve_expr(e);
                }
            }
            hir::Stmt::Break(expr, _) => {
                if let Some(e) = expr {
                    self.resolve_expr(e);
                }
            }
            hir::Stmt::Continue(_) => {}
            hir::Stmt::Match(m) => {
                self.resolve_expr(&mut m.subject);
                m.ty = self.infer_ctx.resolve(&m.ty);
                for arm in &mut m.arms {
                    self.resolve_pat(&mut arm.pat);
                    if let Some(g) = &mut arm.guard {
                        self.resolve_expr(g);
                    }
                    self.resolve_block(&mut arm.body);
                }
            }
            hir::Stmt::Asm(_) => {}
            hir::Stmt::Drop(_, _, ty, _) => {
                *ty = self.infer_ctx.resolve(ty);
            }
            hir::Stmt::ErrReturn(e, ty, _) => {
                *ty = self.infer_ctx.resolve(ty);
                self.resolve_expr(e);
            }
            hir::Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    self.resolve_expr(e);
                }
            }
            hir::Stmt::StoreDelete(_, filter, _) => {
                self.resolve_filter(filter);
            }
            hir::Stmt::StoreSet(_, updates, filter, _) => {
                for (_, e) in updates {
                    self.resolve_expr(e);
                }
                self.resolve_filter(filter);
            }
            hir::Stmt::Transaction(block, _) => {
                self.resolve_block(block);
            }
            hir::Stmt::ChannelClose(e, _) => self.resolve_expr(e),
            hir::Stmt::Stop(e, _) => self.resolve_expr(e),
        }
    }

    fn resolve_expr(&mut self, expr: &mut hir::Expr) {
        expr.ty = self.infer_ctx.resolve(&expr.ty);
        match &mut expr.kind {
            hir::ExprKind::Int(_)
            | hir::ExprKind::Float(_)
            | hir::ExprKind::Str(_)
            | hir::ExprKind::Bool(_)
            | hir::ExprKind::None
            | hir::ExprKind::Void
            | hir::ExprKind::MapNew
            | hir::ExprKind::StoreCount(_)
            | hir::ExprKind::StoreAll(_) => {}
            hir::ExprKind::Var(_, _)
            | hir::ExprKind::FnRef(_, _)
            | hir::ExprKind::VariantRef(_, _, _) => {}
            hir::ExprKind::BinOp(l, _, r) => {
                self.resolve_expr(l);
                self.resolve_expr(r);
            }
            hir::ExprKind::UnaryOp(_, e) => self.resolve_expr(e),
            hir::ExprKind::Call(_, _, args) => {
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                self.resolve_expr(callee);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::Builtin(_, args) => {
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::Method(recv, _, _, args) => {
                self.resolve_expr(recv);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::StringMethod(recv, _, args)
            | hir::ExprKind::VecMethod(recv, _, args)
            | hir::ExprKind::MapMethod(recv, _, args) => {
                self.resolve_expr(recv);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::VecNew(args) => {
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::Field(e, _, _) => self.resolve_expr(e),
            hir::ExprKind::Index(arr, idx) => {
                self.resolve_expr(arr);
                self.resolve_expr(idx);
            }
            hir::ExprKind::Ternary(c, t, f) => {
                self.resolve_expr(c);
                self.resolve_expr(t);
                self.resolve_expr(f);
            }
            hir::ExprKind::Coerce(e, _) => self.resolve_expr(e),
            hir::ExprKind::Cast(e, ty) => {
                self.resolve_expr(e);
                *ty = self.infer_ctx.resolve(ty);
            }
            hir::ExprKind::Array(elems) | hir::ExprKind::Tuple(elems) => {
                for e in elems {
                    self.resolve_expr(e);
                }
            }
            hir::ExprKind::Struct(_, fields) | hir::ExprKind::VariantCtor(_, _, _, fields) => {
                for fi in fields {
                    self.resolve_expr(&mut fi.value);
                }
            }
            hir::ExprKind::IfExpr(if_stmt) => {
                self.resolve_expr(&mut if_stmt.cond);
                self.resolve_block(&mut if_stmt.then);
                for (cond, block) in &mut if_stmt.elifs {
                    self.resolve_expr(cond);
                    self.resolve_block(block);
                }
                if let Some(els) = &mut if_stmt.els {
                    self.resolve_block(els);
                }
            }
            hir::ExprKind::Pipe(e, _, _, args) => {
                self.resolve_expr(e);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::Block(block) => self.resolve_block(block),
            hir::ExprKind::Lambda(params, body) => {
                for p in params {
                    p.ty = self.infer_ctx.resolve(&p.ty);
                }
                self.resolve_block(body);
            }
            hir::ExprKind::Ref(e) | hir::ExprKind::Deref(e) => self.resolve_expr(e),
            hir::ExprKind::ListComp(body, _, _, iter, cond, map) => {
                self.resolve_expr(body);
                self.resolve_expr(iter);
                if let Some(c) = cond {
                    self.resolve_expr(c);
                }
                if let Some(m) = map {
                    self.resolve_expr(m);
                }
            }
            hir::ExprKind::Syscall(args) => {
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::Spawn(_) => {}
            hir::ExprKind::Send(recv, _, _, _, args) => {
                self.resolve_expr(recv);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::CoroutineCreate(_, stmts) => {
                self.resolve_block(stmts);
            }
            hir::ExprKind::CoroutineNext(e) | hir::ExprKind::Yield(e) => {
                self.resolve_expr(e);
            }
            hir::ExprKind::DynDispatch(obj, _, _, args) => {
                self.resolve_expr(obj);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::DynCoerce(e, _, _) => self.resolve_expr(e),
            hir::ExprKind::StoreQuery(_, filter) => self.resolve_filter(filter),
            hir::ExprKind::IterNext(_, _, _) => {}
            hir::ExprKind::ChannelCreate(ty, cap) => {
                *ty = self.infer_ctx.resolve(ty);
                self.resolve_expr(cap);
            }
            hir::ExprKind::ChannelSend(ch, val) => {
                self.resolve_expr(ch);
                self.resolve_expr(val);
            }
            hir::ExprKind::ChannelRecv(ch) => self.resolve_expr(ch),
            hir::ExprKind::Select(arms, default) => {
                for arm in arms {
                    arm.elem_ty = self.infer_ctx.resolve(&arm.elem_ty);
                    self.resolve_expr(&mut arm.chan);
                    if let Some(v) = &mut arm.value {
                        self.resolve_expr(v);
                    }
                    self.resolve_block(&mut arm.body);
                }
                if let Some(block) = default {
                    self.resolve_block(block);
                }
            }
        }
    }

    fn resolve_pat(&mut self, pat: &mut hir::Pat) {
        match pat {
            hir::Pat::Wild(_) => {}
            hir::Pat::Bind(_, _, ty, _) => {
                *ty = self.infer_ctx.resolve(ty);
            }
            hir::Pat::Lit(e) => self.resolve_expr(e),
            hir::Pat::Ctor(_, _, pats, _)
            | hir::Pat::Tuple(pats, _)
            | hir::Pat::Array(pats, _)
            | hir::Pat::Or(pats, _) => {
                for p in pats {
                    self.resolve_pat(p);
                }
            }
            hir::Pat::Range(lo, hi, _) => {
                self.resolve_expr(lo);
                self.resolve_expr(hi);
            }
        }
    }

    fn resolve_filter(&mut self, filter: &mut hir::StoreFilter) {
        self.resolve_expr(&mut filter.value);
        for (_, cond) in &mut filter.extra {
            self.resolve_expr(&mut cond.value);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn type_mismatch_msg(
        &mut self,
        expected: &Type,
        found: &Type,
        context: &str,
    ) -> String {
        let expected_resolved = self.infer_ctx.resolve(expected);
        let found_resolved = self.infer_ctx.resolve(found);
        let mut msg =
            format!("{context}: expected `{expected_resolved}`, found `{found_resolved}`");
        if let Some(origin) = self.infer_ctx.origin_of(expected) {
            msg.push_str(&format!(
                " (expected type constrained at line {} by {})",
                origin.span.line, origin.reason
            ));
        }
        if let Some(origin) = self.infer_ctx.origin_of(found) {
            msg.push_str(&format!(
                " (found type constrained at line {} by {})",
                origin.span.line, origin.reason
            ));
        }
        msg
    }

    pub(crate) fn lower_actor_def(&mut self, ad: &ast::ActorDef) -> Result<hir::ActorDef, String> {
        let (id, ref declared_fields, ref handler_info) = self
            .actors
            .get(&ad.name)
            .ok_or_else(|| format!("undeclared actor: {}", ad.name))?
            .clone();

        let fields: Vec<hir::Field> = ad
            .fields
            .iter()
            .map(|f| {
                let ty = declared_fields
                    .iter()
                    .find(|(n, _)| n == &f.name)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)));
                let default = f.default.as_ref().map(|e| {
                    self.lower_expr(e).unwrap_or_else(|_| hir::Expr {
                        kind: hir::ExprKind::Int(0),
                        ty: Type::I64,
                        span: e.span(),
                    })
                });
                hir::Field {
                    name: f.name.clone(),
                    ty,
                    default,
                    span: f.span,
                }
            })
            .collect();

        let mut hir_handlers = Vec::new();
        for (i, h) in ad.handlers.iter().enumerate() {
            self.push_scope();
            for f in &fields {
                let fid = self.fresh_id();
                self.define_var(
                    &f.name,
                    VarInfo {
                        def_id: fid,
                        ty: f.ty.clone(),
                        ownership: Ownership::Owned,
                    },
                );
            }
            let mut params = Vec::new();
            let declared_ptys = &handler_info[i].1;
            for (pi, p) in h.params.iter().enumerate() {
                let pid = self.fresh_id();
                let ty = p.ty.clone().unwrap_or_else(|| {
                    declared_ptys
                        .get(pi)
                        .map(|t| self.infer_ctx.resolve(t))
                        .unwrap_or(Type::I64)
                });
                let ownership = Self::ownership_for_type(&ty);
                self.define_var(
                    &p.name,
                    VarInfo {
                        def_id: pid,
                        ty: ty.clone(),
                        ownership,
                    },
                );
                params.push(hir::Param {
                    def_id: pid,
                    name: p.name.clone(),
                    ty,
                    ownership,
                    span: p.span,
                });
            }
            let body = self.lower_block(&h.body, &Type::Void)?;
            self.pop_scope();
            hir_handlers.push(hir::HandlerDef {
                name: h.name.clone(),
                params,
                body,
                tag: handler_info[i].2,
                span: h.span,
            });
        }

        Ok(hir::ActorDef {
            def_id: id,
            name: ad.name.clone(),
            fields,
            handlers: hir_handlers,
            span: ad.span,
        })
    }

    pub(crate) fn lower_store_def(&mut self, sd: &ast::StoreDef) -> Result<hir::StoreDef, String> {
        let id = self.fresh_id();
        let fields: Vec<hir::Field> = sd
            .fields
            .iter()
            .map(|f| hir::Field {
                name: f.name.clone(),
                ty: f.ty.clone().unwrap_or(Type::I64),
                default: None,
                span: f.span,
            })
            .collect();
        Ok(hir::StoreDef {
            def_id: id,
            name: sd.name.clone(),
            fields,
            span: sd.span,
        })
    }

    pub(crate) fn lower_impl_block(
        &mut self,
        ib: &ast::ImplBlock,
    ) -> Result<hir::TraitImpl, String> {
        let mut hir_methods = Vec::new();
        let is_iter_impl = ib.trait_name.as_deref() == Some("Iter");
        for m in &ib.methods {
            let hm = if is_iter_impl {
                self.lower_method_by_ptr(&ib.type_name, m)?
            } else {
                self.lower_method(&ib.type_name, m)?
            };
            hir_methods.push(hm);
        }
        Ok(hir::TraitImpl {
            trait_name: ib.trait_name.clone(),
            trait_type_args: ib.trait_type_args.clone(),
            type_name: ib.type_name.clone(),
            methods: hir_methods,
            span: ib.span,
        })
    }

    pub(crate) fn lower_fn(&mut self, f: &ast::Fn) -> Result<hir::Fn, String> {
        let (id, ptys, ret) = self
            .fns
            .get(&f.name)
            .ok_or_else(|| format!("undeclared function: {}", f.name))?
            .clone();

        self.push_scope();
        let mut params = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            let pid = self.fresh_id();
            let ty = ptys[i].clone();
            let ownership = Self::ownership_for_type(&ty);
            self.define_var(
                &p.name,
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
                },
            );
            params.push(hir::Param {
                def_id: pid,
                name: p.name.clone(),
                ty,
                ownership,
                span: p.span,
            });
        }
        let body = self.lower_block(&f.body, &ret)?;
        self.pop_scope();

        let ret = if f.ret.is_none() && f.name != "main" {
            self.refine_ret_from_body(&ret, &body)
        } else {
            ret
        };

        Ok(hir::Fn {
            def_id: id,
            name: f.name.clone(),
            params,
            ret,
            body,
            span: f.span,
            generic_origin: None,
        })
    }

    fn lower_test_block(&mut self, tb: &ast::TestBlock, fn_name: &str) -> Result<hir::Fn, String> {
        let id = self.fresh_id();
        self.push_scope();
        let body = self.lower_block(&tb.body, &Type::Void)?;
        self.pop_scope();
        Ok(hir::Fn {
            def_id: id,
            name: fn_name.to_string(),
            params: vec![],
            ret: Type::Void,
            body,
            span: tb.span,
            generic_origin: None,
        })
    }

    fn build_test_runner(&mut self, tests: &[(String, String)]) -> hir::Fn {
        let id = self.fresh_id();
        let s = Span::dummy();
        let mut body: hir::Block = Vec::new();
        for (display_name, fn_name) in tests {
            body.push(hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::Builtin(
                    hir::BuiltinFn::Log,
                    vec![hir::Expr {
                        kind: hir::ExprKind::Str(format!("test {display_name} ...")),
                        ty: Type::String,
                        span: s,
                    }],
                ),
                ty: Type::Void,
                span: s,
            }));
            let test_id = self.fns.get(fn_name).unwrap().0;
            body.push(hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::Call(test_id, fn_name.clone(), vec![]),
                ty: Type::Void,
                span: s,
            }));
            body.push(hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::Builtin(
                    hir::BuiltinFn::Log,
                    vec![hir::Expr {
                        kind: hir::ExprKind::Str("  ok".into()),
                        ty: Type::String,
                        span: s,
                    }],
                ),
                ty: Type::Void,
                span: s,
            }));
        }
        hir::Fn {
            def_id: id,
            name: "main".into(),
            params: vec![],
            ret: Type::I32,
            body,
            span: s,
            generic_origin: None,
        }
    }

    pub(crate) fn lower_type_def(&mut self, td: &ast::TypeDef) -> Result<hir::TypeDef, String> {
        let id = self.fresh_id();
        let declared_fields = self.structs.get(&td.name).cloned().unwrap_or_default();
        let fields: Vec<hir::Field> = td
            .fields
            .iter()
            .map(|f| {
                let ty = declared_fields
                    .iter()
                    .find(|(n, _)| n == &f.name)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)));
                let default = f.default.as_ref().map(|e| {
                    let lowered = self.lower_expr(e).unwrap_or_else(|_| hir::Expr {
                        kind: hir::ExprKind::Int(0),
                        ty: Type::I64,
                        span: e.span(),
                    });
                    let _ =
                        self.infer_ctx
                            .unify_at(&ty, &lowered.ty, f.span, "field default value");
                    lowered
                });
                hir::Field {
                    name: f.name.clone(),
                    ty,
                    default,
                    span: f.span,
                }
            })
            .collect();

        let mut hir_methods = Vec::new();
        for m in &td.methods {
            let method_name = format!("{}_{}", td.name, m.name);
            if self.fns.contains_key(&method_name) {
                let hm = self.lower_method(&td.name, m)?;
                hir_methods.push(hm);
            }
        }

        Ok(hir::TypeDef {
            def_id: id,
            name: td.name.clone(),
            fields,
            methods: hir_methods,
            layout: td.layout.clone(),
            span: td.span,
        })
    }

    pub(crate) fn lower_method(&mut self, type_name: &str, m: &ast::Fn) -> Result<hir::Fn, String> {
        let method_name = format!("{type_name}_{}", m.name);
        let (id, ptys, ret) = self
            .fns
            .get(&method_name)
            .ok_or_else(|| format!("undeclared method: {method_name}"))?
            .clone();

        self.push_scope();
        let mut params = Vec::new();

        let self_id = self.fresh_id();
        let self_ty = ptys[0].clone();
        self.define_var(
            "self",
            VarInfo {
                def_id: self_id,
                ty: self_ty.clone(),
                ownership: Ownership::Borrowed,
            },
        );
        params.push(hir::Param {
            def_id: self_id,
            name: "self".to_string(),
            ty: self_ty,
            ownership: Ownership::Borrowed,
            span: m.span,
        });

        for (i, p) in m.params.iter().enumerate() {
            let pid = self.fresh_id();
            let ty = ptys[i + 1].clone();
            let ownership = Self::ownership_for_type(&ty);
            self.define_var(
                &p.name,
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
                },
            );
            params.push(hir::Param {
                def_id: pid,
                name: p.name.clone(),
                ty,
                ownership,
                span: p.span,
            });
        }

        let body = self.lower_block(&m.body, &ret)?;
        self.pop_scope();

        Ok(hir::Fn {
            def_id: id,
            name: method_name,
            params,
            ret,
            body,
            span: m.span,
            generic_origin: None,
        })
    }

    pub(crate) fn lower_method_by_ptr(
        &mut self,
        type_name: &str,
        m: &ast::Fn,
    ) -> Result<hir::Fn, String> {
        let method_name = format!("{type_name}_{}", m.name);
        let (id, ptys, ret) = self
            .fns
            .get(&method_name)
            .ok_or_else(|| format!("undeclared method: {method_name}"))?
            .clone();

        self.push_scope();
        let mut params = Vec::new();

        let self_id = self.fresh_id();
        let self_ty = ptys[0].clone();
        self.define_var(
            "self",
            VarInfo {
                def_id: self_id,
                ty: self_ty.clone(),
                ownership: Ownership::Borrowed,
            },
        );
        params.push(hir::Param {
            def_id: self_id,
            name: "self".to_string(),
            ty: self_ty,
            ownership: Ownership::Borrowed,
            span: m.span,
        });

        for (i, p) in m.params.iter().filter(|p| p.name != "self").enumerate() {
            let pid = self.fresh_id();
            let ty = ptys[i + 1].clone();
            let ownership = Self::ownership_for_type(&ty);
            self.define_var(
                &p.name,
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
                },
            );
            params.push(hir::Param {
                def_id: pid,
                name: p.name.clone(),
                ty,
                ownership,
                span: p.span,
            });
        }

        let body = self.lower_block(&m.body, &ret)?;
        self.pop_scope();

        Ok(hir::Fn {
            def_id: id,
            name: method_name,
            params,
            ret,
            body,
            span: m.span,
            generic_origin: None,
        })
    }

    pub(crate) fn lower_enum_def(&mut self, ed: &ast::EnumDef) -> hir::EnumDef {
        let id = self.fresh_id();
        let variants: Vec<hir::Variant> = ed
            .variants
            .iter()
            .enumerate()
            .map(|(tag, v)| hir::Variant {
                name: v.name.clone(),
                fields: v
                    .fields
                    .iter()
                    .map(|f| hir::VField {
                        name: f.name.clone(),
                        ty: f.ty.clone(),
                    })
                    .collect(),
                tag: tag as u32,
                span: v.span,
            })
            .collect();
        hir::EnumDef {
            def_id: id,
            name: ed.name.clone(),
            variants,
            span: ed.span,
        }
    }

    pub(crate) fn lower_extern(&self, ef: &ast::ExternFn) -> hir::ExternFn {
        let (id, _, _) = self
            .fns
            .get(&ef.name)
            .cloned()
            .unwrap_or_else(|| (DefId::BUILTIN, vec![], Type::Void));
        hir::ExternFn {
            def_id: id,
            name: ef.name.clone(),
            params: ef.params.clone(),
            ret: ef.ret.clone(),
            variadic: ef.variadic,
            span: ef.span,
        }
    }

    pub(crate) fn lower_err_def(&mut self, ed: &ast::ErrDef) -> hir::ErrDef {
        let id = self.fresh_id();
        let variants: Vec<hir::ErrVariant> = ed
            .variants
            .iter()
            .enumerate()
            .map(|(tag, v)| hir::ErrVariant {
                name: v.name.clone(),
                fields: v.fields.clone(),
                tag: tag as u32,
                span: v.span,
            })
            .collect();
        hir::ErrDef {
            def_id: id,
            name: ed.name.clone(),
            variants,
            span: ed.span,
        }
    }

    pub(crate) fn type_implements_trait(&self, type_name: &str, trait_name: &str) -> bool {
        self.trait_impls
            .get(type_name)
            .map(|impls| impls.contains(&trait_name.to_string()))
            .unwrap_or(false)
    }

    pub(crate) fn iter_element_type(&self, type_name: &str) -> Type {
        if let Some(args) = self
            .trait_impl_type_args
            .get(&(type_name.into(), "Iter".into()))
        {
            if let Some(t) = args.first() {
                return t.clone();
            }
        }
        let fn_name = format!("{type_name}_next");
        if let Some((_, _, ret)) = self.fns.get(&fn_name) {
            if let Type::Enum(ename) = ret {
                if let Some(stripped) = ename.strip_prefix("Option_") {
                    return match stripped {
                        "i64" => Type::I64,
                        "f64" => Type::F64,
                        "bool" => Type::Bool,
                        "String" => Type::String,
                        other => Type::Struct(other.into()),
                    };
                }
            }
        }
        Type::I64
    }

    pub(crate) fn desugar_for_iter(
        &mut self,
        f: &ast::For,
        iter_expr: hir::Expr,
        type_name: String,
        elem_ty: Type,
        ret_ty: &Type,
    ) -> Result<hir::Stmt, String> {
        let span = f.span;

        let mut option_type_map = HashMap::new();
        option_type_map.insert("T".into(), elem_ty.clone());
        let option_enum_name = self.monomorphize_enum("Option", &option_type_map)?;

        let some_tag = self.variant_tags.get("Some").map(|(_, t)| *t).unwrap_or(0);
        let nothing_tag = self
            .variant_tags
            .get("Nothing")
            .map(|(_, t)| *t)
            .unwrap_or(1);

        let iter_bind_id = self.fresh_id();
        let iter_var_name = format!("__iter_{}", f.bind);

        self.define_var(
            &iter_var_name,
            VarInfo {
                def_id: iter_bind_id,
                ty: iter_expr.ty.clone(),
                ownership: Ownership::Owned,
            },
        );

        let bind_stmt = hir::Stmt::Bind(hir::Bind {
            def_id: iter_bind_id,
            name: iter_var_name.clone(),
            value: iter_expr.clone(),
            ty: iter_expr.ty.clone(),
            ownership: Ownership::Owned,
            span,
        });

        let method_name = format!("{type_name}_next");
        let ret = Type::Enum(option_enum_name.clone());
        if let Some(entry) = self.fns.get_mut(&method_name) {
            entry.2 = ret.clone();
        }

        let next_call = hir::Expr {
            kind: hir::ExprKind::IterNext(iter_var_name.clone(), type_name.clone(), "next".into()),
            ty: ret,
            span,
        };

        let bind_id = self.fresh_id();
        let some_pat = hir::Pat::Ctor(
            "Some".into(),
            some_tag,
            vec![hir::Pat::Bind(
                bind_id,
                f.bind.clone(),
                elem_ty.clone(),
                span,
            )],
            span,
        );
        let nothing_pat = hir::Pat::Ctor("Nothing".into(), nothing_tag, vec![], span);

        self.push_scope();
        self.define_var(
            &f.bind,
            VarInfo {
                def_id: bind_id,
                ty: elem_ty.clone(),
                ownership: Ownership::Owned,
            },
        );
        let body = self.lower_block_no_scope(&f.body, ret_ty)?;
        self.pop_scope();

        let some_arm = hir::Arm {
            pat: some_pat,
            guard: None,
            body,
            span,
        };
        let nothing_arm = hir::Arm {
            pat: nothing_pat,
            guard: None,
            body: vec![hir::Stmt::Break(None, span)],
            span,
        };

        let match_stmt = hir::Stmt::Match(hir::Match {
            subject: next_call,
            arms: vec![some_arm, nothing_arm],
            ty: Type::Void,
            span,
        });

        let loop_stmt = hir::Stmt::Loop(hir::Loop {
            body: vec![match_stmt],
            span,
        });

        Ok(hir::Stmt::Expr(hir::Expr {
            kind: hir::ExprKind::Block(vec![bind_stmt, loop_stmt]),
            ty: Type::Void,
            span,
        }))
    }

    pub(crate) fn lower_block(
        &mut self,
        block: &ast::Block,
        ret_ty: &Type,
    ) -> Result<hir::Block, String> {
        self.push_scope();
        let mut stmts = Vec::new();
        for s in block {
            let hs = self.lower_stmt(s, ret_ty)?;
            stmts.push(hs);
        }
        let ends_with_jump = stmts.last().map_or(false, |s| {
            matches!(
                s,
                hir::Stmt::Ret(..) | hir::Stmt::Break(..) | hir::Stmt::Continue(..)
            )
        });
        if ends_with_jump {
            let jump = stmts.pop().unwrap();
            self.emit_scope_drops(&mut stmts);
            stmts.push(jump);
        } else {
            self.emit_scope_drops(&mut stmts);
        }
        self.pop_scope();
        Ok(stmts)
    }

    fn emit_scope_drops(&self, stmts: &mut Vec<hir::Stmt>) {
        let scope = match self.scopes.last() {
            Some(s) => s,
            None => return,
        };
        let mut drops: Vec<_> = scope
            .iter()
            .filter(|(_, info)| Self::needs_drop(&info.ty))
            .collect();
        drops.sort_by_key(|(_, info)| std::cmp::Reverse(info.def_id.0));
        for (name, info) in drops {
            stmts.push(hir::Stmt::Drop(
                info.def_id,
                name.clone(),
                info.ty.clone(),
                crate::ast::Span::dummy(),
            ));
        }
    }

    pub(crate) fn needs_drop(ty: &Type) -> bool {
        matches!(
            ty,
            Type::String | Type::Vec(_) | Type::Map(_, _) | Type::Rc(_) | Type::Weak(_)
        )
    }

    pub(crate) fn check_exhaustiveness(
        &self,
        subject_ty: &Type,
        arms: &[hir::Arm],
        _span: Span,
    ) -> Result<(), String> {
        let pats: Vec<&hir::Pat> = arms
            .iter()
            .filter(|a| a.guard.is_none())
            .map(|a| &a.pat)
            .collect();

        let missing = self.find_missing_patterns(&pats, subject_ty);
        if !missing.is_empty() {
            let missing_str = missing.join(", ");
            let ty_name = match subject_ty {
                Type::Enum(n) => format!("`{n}`"),
                Type::Bool => "Bool".to_string(),
                _ => format!("{:?}", subject_ty),
            };
            return Err(format!(
                "non-exhaustive match on {ty_name}: missing {missing_str}"
            ));
        }

        if let Type::Enum(_) = subject_ty {
            let mut seen: Vec<&str> = Vec::new();
            for arm in arms {
                if let hir::Pat::Ctor(n, _, subs, _) = &arm.pat {
                    if subs.is_empty() && seen.contains(&n.as_str()) {
                        eprintln!("warning: unreachable pattern `{n}` — already matched above");
                    }
                    if subs.is_empty() {
                        seen.push(n.as_str());
                    }
                }
            }
        }

        Ok(())
    }

    fn find_missing_patterns(&self, pats: &[&hir::Pat], ty: &Type) -> Vec<String> {
        let mut flat: Vec<&hir::Pat> = Vec::new();
        for p in pats {
            Self::flatten_or_pat(p, &mut flat);
        }

        if flat
            .iter()
            .any(|p| matches!(p, hir::Pat::Wild(_) | hir::Pat::Bind(..)))
        {
            return vec![];
        }

        let ty = self.resolve_ty(ty.clone());

        match &ty {
            Type::Enum(name) => {
                let variants = match self.enums.get(name) {
                    Some(v) => v,
                    None => return vec![],
                };
                let mut missing = Vec::new();
                for (vname, field_tys) in variants {
                    let sub_lists: Vec<&Vec<hir::Pat>> = flat
                        .iter()
                        .filter_map(|p| match p {
                            hir::Pat::Ctor(n, _, subs, _) if n == vname => Some(subs),
                            _ => None,
                        })
                        .collect();

                    if sub_lists.is_empty() {
                        if field_tys.is_empty() {
                            missing.push(vname.clone());
                        } else {
                            let fields = vec!["_"; field_tys.len()].join(", ");
                            missing.push(format!("{}({})", vname, fields));
                        }
                    } else if !field_tys.is_empty() {
                        for (i, ft) in field_tys.iter().enumerate() {
                            let col: Vec<&hir::Pat> =
                                sub_lists.iter().filter_map(|subs| subs.get(i)).collect();
                            let sub_missing = self.find_missing_patterns(&col, ft);
                            for sm in &sub_missing {
                                let fields: Vec<String> = field_tys
                                    .iter()
                                    .enumerate()
                                    .map(|(j, _)| if j == i { sm.clone() } else { "_".to_string() })
                                    .collect();
                                missing.push(format!("{}({})", vname, fields.join(", ")));
                            }
                        }
                    }
                }
                missing
            }
            Type::Bool => {
                let has_true = flat.iter().any(|p| match p {
                    hir::Pat::Lit(e) => matches!(e.kind, hir::ExprKind::Bool(true)),
                    _ => false,
                });
                let has_false = flat.iter().any(|p| match p {
                    hir::Pat::Lit(e) => matches!(e.kind, hir::ExprKind::Bool(false)),
                    _ => false,
                });
                let mut missing = Vec::new();
                if !has_true {
                    missing.push("true".to_string());
                }
                if !has_false {
                    missing.push("false".to_string());
                }
                missing
            }
            Type::I64 | Type::F64 | Type::String => {
                vec!["_".to_string()]
            }
            _ => vec![],
        }
    }

    fn flatten_or_pat<'a>(pat: &'a hir::Pat, out: &mut Vec<&'a hir::Pat>) {
        match pat {
            hir::Pat::Or(pats, _) => {
                for p in pats {
                    Self::flatten_or_pat(p, out);
                }
            }
            _ => out.push(pat),
        }
    }
}
