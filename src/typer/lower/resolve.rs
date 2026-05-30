use super::super::Typer;
use crate::hir;
use crate::intern::Symbol;
use crate::types::Type;

pub(super) fn type_references_name(ty: &Type, name: Symbol) -> bool {
    match ty {
        Type::Struct(n, args) => *n == name || args.iter().any(|a| type_references_name(a, name)),
        Type::Alias(n, inner) | Type::Newtype(n, inner) => {
            *n == name || type_references_name(inner, name)
        }
        Type::Vec(inner)
        | Type::Ptr(inner)
        | Type::Channel(inner)
        | Type::Coroutine(inner)
        | Type::Generator(inner) => type_references_name(inner, name),
        Type::Map(k, v) => type_references_name(k, name) || type_references_name(v, name),
        Type::Array(inner, _) => type_references_name(inner, name),
        Type::Tuple(elems) => elems.iter().any(|e| type_references_name(e, name)),
        Type::Fn(params, ret) => {
            params.iter().any(|p| type_references_name(p, name)) || type_references_name(ret, name)
        }
        Type::Enum(n) => *n == name,
        _ => false,
    }
}

impl Typer {
    pub(in crate::typer) fn normalize_actor_refs(
        ty: Type,
        actors: &std::collections::HashSet<Symbol>,
    ) -> Type {
        match ty {
            Type::Struct(name, args) if args.is_empty() && actors.contains(&name) => {
                Type::ActorRef(name)
            }
            Type::Struct(name, args) => {
                let args = args
                    .into_iter()
                    .map(|a| Self::normalize_actor_refs(a, actors))
                    .collect();
                Type::Struct(name, args)
            }
            Type::Vec(inner) => Type::Vec(Box::new(Self::normalize_actor_refs(*inner, actors))),
            Type::Array(inner, n) => {
                Type::Array(Box::new(Self::normalize_actor_refs(*inner, actors)), n)
            }
            Type::Map(k, v) => Type::Map(
                Box::new(Self::normalize_actor_refs(*k, actors)),
                Box::new(Self::normalize_actor_refs(*v, actors)),
            ),
            Type::Channel(inner) => {
                Type::Channel(Box::new(Self::normalize_actor_refs(*inner, actors)))
            }
            Type::Coroutine(inner) => {
                Type::Coroutine(Box::new(Self::normalize_actor_refs(*inner, actors)))
            }
            Type::Generator(inner) => {
                Type::Generator(Box::new(Self::normalize_actor_refs(*inner, actors)))
            }
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .into_iter()
                    .map(|e| Self::normalize_actor_refs(e, actors))
                    .collect(),
            ),
            Type::Fn(params, ret) => Type::Fn(
                params
                    .into_iter()
                    .map(|p| Self::normalize_actor_refs(p, actors))
                    .collect(),
                Box::new(Self::normalize_actor_refs(*ret, actors)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(Self::normalize_actor_refs(*inner, actors))),
            Type::Alias(name, inner) => {
                Type::Alias(name, Box::new(Self::normalize_actor_refs(*inner, actors)))
            }
            Type::Newtype(name, inner) => {
                Type::Newtype(name, Box::new(Self::normalize_actor_refs(*inner, actors)))
            }
            other => other,
        }
    }

    pub(in crate::typer) fn reclassify_method_call(&mut self, expr: &mut hir::Expr) {
        let (recv_ty, method) = match &expr.kind {
            hir::ExprKind::DeferredMethod(recv, m, _) => (recv.ty.clone(), m.clone()),
            _ => return,
        };
        match &recv_ty {
            Type::Vec(_) => {
                if let hir::ExprKind::DeferredMethod(recv, method, args) =
                    std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                {
                    expr.kind = hir::ExprKind::VecMethod(recv, method, args);
                }
            }
            Type::Map(_, _) => {
                if let hir::ExprKind::DeferredMethod(recv, method, args) =
                    std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                {
                    expr.kind = hir::ExprKind::MapMethod(recv, method, args);
                }
            }
            Type::Struct(type_name, _) => {
                let method_name = format!("{}_{}", type_name, method);
                if self.fns.contains_key(&method_name) {
                    if let hir::ExprKind::DeferredMethod(recv, _method_str, args) =
                        std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                    {
                        expr.kind = hir::ExprKind::Method(recv, method_name.into(), method, args);
                    }
                }
            }
            Type::Ptr(inner) => {
                if let Type::Struct(type_name, _) = inner.as_ref() {
                    let method_name = format!("{}_{}", type_name, method);
                    if self.fns.contains_key(&method_name) {
                        if let hir::ExprKind::DeferredMethod(recv, _method_str, args) =
                            std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                        {
                            expr.kind =
                                hir::ExprKind::Method(recv, method_name.into(), method, args);
                        }
                    }
                }
            }
            Type::Coroutine(_) if method == "next" => {
                if let hir::ExprKind::DeferredMethod(recv, _, _) =
                    std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                {
                    expr.kind = hir::ExprKind::CoroutineNext(recv);
                }
            }
            Type::F64 | Type::F32 => {
                let float_methods = [
                    "sqrt",
                    "abs",
                    "floor",
                    "ceil",
                    "round",
                    "trunc",
                    "sin",
                    "cos",
                    "tan",
                    "asin",
                    "acos",
                    "atan",
                    "sinh",
                    "cosh",
                    "tanh",
                    "exp",
                    "exp2",
                    "ln",
                    "log2",
                    "log10",
                    "cbrt",
                    "recip",
                    "signum",
                    "pow",
                    "atan2",
                    "copysign",
                    "min",
                    "max",
                    "clamp",
                    "is_nan",
                    "is_infinite",
                    "is_finite",
                    "to_int",
                ];
                if float_methods.iter().any(|m| method == *m) {
                    if let hir::ExprKind::DeferredMethod(recv, _method_str, args) =
                        std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                    {
                        let mut all_args = vec![*recv];
                        all_args.extend(args);
                        let ret_ty = match &*method.as_str() {
                            "is_nan" | "is_infinite" | "is_finite" => Type::Bool,
                            "to_int" => Type::I64,
                            _ => recv_ty.clone(),
                        };
                        expr.ty = ret_ty;
                        expr.kind =
                            hir::ExprKind::Builtin(hir::BuiltinFn::FloatMethod(method), all_args);
                    }
                }
            }
            Type::Channel(_) if method == "send" || method == "recv" || method == "close" => {
                if let hir::ExprKind::DeferredMethod(recv, method, args) =
                    std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                {
                    expr.kind = hir::ExprKind::StringMethod(recv, method, args);
                }
            }
            Type::I64
            | Type::I32
            | Type::I16
            | Type::I8
            | Type::U64
            | Type::U32
            | Type::U16
            | Type::U8 => {
                let int_methods = ["abs", "to_float", "to_str", "min", "max", "clamp"];
                if int_methods.iter().any(|m| method == *m) {
                    if let hir::ExprKind::DeferredMethod(recv, _method_str, args) =
                        std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                    {
                        let mut all_args = vec![*recv];
                        all_args.extend(args);
                        expr.kind =
                            hir::ExprKind::Builtin(hir::BuiltinFn::CharMethod(method), all_args);
                    }
                }
            }
            Type::String => {
                if let hir::ExprKind::DeferredMethod(recv, method, args) =
                    std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                {
                    expr.kind = hir::ExprKind::StringMethod(recv, method, args);
                }
            }
            _ => {}
        }
    }

    pub(in crate::typer) fn resolve_all_types(&mut self, prog: &mut hir::Program) {
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
            if let Some(sfields) = self.structs.get_mut(&td.name) {
                for (i, field) in td.fields.iter().enumerate() {
                    if let Some(sf) = sfields.get_mut(i) {
                        sf.1 = field.ty.clone();
                    }
                }
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
                if let Some(sleep_ms) = &mut h.loop_sleep_ms {
                    self.resolve_expr(sleep_ms);
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
        for g in &mut prog.globals {
            g.ty = self.infer_ctx.resolve(&g.ty);
            self.resolve_expr(&mut g.init);
        }
    }

    pub(in crate::typer) fn resolve_fn(&mut self, f: &mut hir::Fn) {
        f.ret = self.infer_ctx.resolve(&f.ret);
        for p in &mut f.params {
            p.ty = self.infer_ctx.resolve(&p.ty);
        }
        self.resolve_block(&mut f.body);
    }

    pub(in crate::typer) fn resolve_block(&mut self, block: &mut hir::Block) {
        for stmt in block {
            self.resolve_stmt(stmt);
        }
    }

    pub(in crate::typer) fn resolve_stmt(&mut self, stmt: &mut hir::Stmt) {
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
                if let Some(ref mut ty2) = f.bind2_ty {
                    *ty2 = self.infer_ctx.resolve(ty2);
                }
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
            hir::Stmt::Nop(_) => {}
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
            hir::Stmt::Defer(body, _) => self.resolve_block(body),
            hir::Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    self.resolve_expr(e);
                }
            }
            hir::Stmt::StoreDelete(_, filter, _) => {
                self.resolve_filter(filter);
            }
            hir::Stmt::StoreDestroy(_, filter, _) => {
                self.resolve_filter(filter);
            }
            hir::Stmt::StoreRestore(_, filter, _) => {
                self.resolve_filter(filter);
            }
            hir::Stmt::StoreSave(_, _) => {}
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
            hir::Stmt::SimFor(f, _) => {
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
            hir::Stmt::SimBlock(b, _) => {
                self.resolve_block(b);
            }
            hir::Stmt::UseLocal(_, _, _, _) => {}
            hir::Stmt::GlobalStore(_, e, _) => {
                self.resolve_expr(e);
            }
        }
    }

    pub(in crate::typer) fn resolve_expr(&mut self, expr: &mut hir::Expr) {
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
            | hir::ExprKind::GlobalLoad(_)
            | hir::ExprKind::StoreAll(_) => {}
            hir::ExprKind::Var(_, _) | hir::ExprKind::VariantRef(_, _, _) => {}
            hir::ExprKind::FnRef(_, _) => {
                if let hir::ExprKind::FnRef(ref mut id, ref mut name) = expr.kind {
                    let has_poly_scheme = self
                        .fn_schemes
                        .get(&*name)
                        .map_or(false, |s| !s.0.is_empty());
                    if has_poly_scheme {
                        if let Type::Fn(ref param_tys, _) = expr.ty {
                            if expr.ty.has_type_var() {
                            } else if let Some(inf_fn) = self.inferable_fns.get(&*name).cloned() {
                                let normalized = Self::normalize_inferable_fn(&inf_fn);
                                let type_map =
                                    self.build_type_map(&name.as_str(), &normalized, param_tys);
                                if let Ok(mangled) = self.monomorphize_fn(&name.as_str(), &type_map)
                                {
                                    if let Some((mid, _, _)) = self.fns.get(&mangled).cloned() {
                                        *id = mid;
                                        *name = mangled.into();
                                    }
                                }
                            }
                        }
                    }
                }
            }
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
            hir::ExprKind::StringMethod(recv, _, args) => {
                self.resolve_expr(recv);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::DeferredMethod(recv, _, args) => {
                self.resolve_expr(recv);
                for a in args {
                    self.resolve_expr(a);
                }
                self.reclassify_method_call(expr);
            }
            hir::ExprKind::VecMethod(recv, _, args) | hir::ExprKind::MapMethod(recv, _, args) => {
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
            hir::ExprKind::Field(e, _field_name, _idx) => {
                self.resolve_expr(e);
                if let hir::ExprKind::Field(ref inner, ref fname, ref mut field_idx) = expr.kind {
                    let recv_ty = &inner.ty;
                    if let Type::Struct(name, _) = recv_ty {
                        if let Some(fields) = self.structs.get(name) {
                            if let Some((i, _)) =
                                fields.iter().enumerate().find(|(_, (n, _))| n == fname)
                            {
                                *field_idx = i;
                            }
                        }
                    }
                }
            }
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
            hir::ExprKind::Spawn(_, _) => {}
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
            hir::ExprKind::StoreQuery(_, filter) => self.resolve_filter(filter),
            hir::ExprKind::ViewCount(_, filter) | hir::ExprKind::ViewAll(_, filter) => {
                self.resolve_filter(filter)
            }
            hir::ExprKind::StoreFirst(_, filter) => self.resolve_filter(filter),
            hir::ExprKind::StoreExists(_, filter) => self.resolve_filter(filter),
            hir::ExprKind::StoreGet(_, key) => self.resolve_expr(key),
            hir::ExprKind::StoreDistinct(_, _)
            | hir::ExprKind::StoreSum(_, _)
            | hir::ExprKind::StoreAvg(_, _)
            | hir::ExprKind::StoreMin(_, _)
            | hir::ExprKind::StoreMax(_, _)
            | hir::ExprKind::StoreVersionCount(_, _)
            | hir::ExprKind::StoreHistory(_, _)
            | hir::ExprKind::StoreAtVersion(_, _, _) => {}
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
            hir::ExprKind::Unreachable => {}
            hir::ExprKind::StrictCast(e, ty) => {
                self.resolve_expr(e);
                *ty = self.infer_ctx.resolve(ty);
            }
            hir::ExprKind::AsFormat(e, _) | hir::ExprKind::AtomicLoad(e) => self.resolve_expr(e),
            hir::ExprKind::AtomicStore(a, b)
            | hir::ExprKind::AtomicAdd(a, b)
            | hir::ExprKind::AtomicSub(a, b) => {
                self.resolve_expr(a);
                self.resolve_expr(b);
            }
            hir::ExprKind::AtomicCas(p, e, n) => {
                self.resolve_expr(p);
                self.resolve_expr(e);
                self.resolve_expr(n);
            }
            hir::ExprKind::Slice(obj, start, end) => {
                self.resolve_expr(obj);
                self.resolve_expr(start);
                self.resolve_expr(end);
            }
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
            hir::ExprKind::Grad(e)
            | hir::ExprKind::GeneratorNext(e)
            | hir::ExprKind::EnumUnwrap(e, _, _)
            | hir::ExprKind::EnumIs(e, _) => {
                self.resolve_expr(e);
            }
            hir::ExprKind::Einsum(_, args) => {
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::Builder(_, fields) => {
                for (_, v) in fields {
                    self.resolve_expr(v);
                }
            }
            hir::ExprKind::GeneratorCreate(_, _, stmts, _) => {
                for s in stmts {
                    self.resolve_stmt(s);
                }
            }
            hir::ExprKind::KvGet(_, e)
            | hir::ExprKind::KvHas(_, e)
            | hir::ExprKind::KvDel(_, e) => self.resolve_expr(e),
            hir::ExprKind::KvSet(_, k, v) | hir::ExprKind::KvIncr(_, k, v) => {
                self.resolve_expr(k);
                self.resolve_expr(v);
            }
            hir::ExprKind::KvCount(_) | hir::ExprKind::TsLatest(_) => {}
            hir::ExprKind::VecNearest(_, v, k) => {
                self.resolve_expr(v);
                self.resolve_expr(k);
            }
            hir::ExprKind::VecInsert(_, v) => self.resolve_expr(v),
            hir::ExprKind::VecCount(_) => {}
            hir::ExprKind::BloomTest(_, _, v) => self.resolve_expr(v),
            hir::ExprKind::FtsSearch(_, _, v) => self.resolve_expr(v),
            hir::ExprKind::FtsCount(_, _) => {}
            hir::ExprKind::GraphFrom(_, e) | hir::ExprKind::GraphTo(_, e) => self.resolve_expr(e),
        }
    }

    pub(in crate::typer) fn resolve_pat(&mut self, pat: &mut hir::Pat) {
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

    pub(in crate::typer) fn resolve_filter(&mut self, filter: &mut hir::StoreFilter) {
        self.resolve_expr(&mut filter.value);
        for (_, cond) in &mut filter.extra {
            self.resolve_expr(&mut cond.value);
        }
    }
}
