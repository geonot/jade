use std::collections::HashMap;

use crate::ast::{self, BinOp, UnaryOp};
use crate::hir::{self, CoercionKind};
use crate::types::Type;

use super::Typer;

impl Typer {
    pub(crate) fn expr_ty_ast(&self, expr: &ast::Expr) -> Type {
        match expr {
            ast::Expr::Int(_, _) => Type::I64,
            ast::Expr::Float(_, _) => Type::F64,
            ast::Expr::Str(_, _) => Type::String,
            ast::Expr::Bool(_, _) => Type::Bool,
            ast::Expr::None(_) => Type::I64,
            ast::Expr::Void(_) => Type::Void,
            ast::Expr::Ident(n, _) => {
                if let Some(v) = self.find_var(n) {
                    v.ty.clone()
                } else if let Some((enum_name, _)) = self.variant_tags.get(n) {
                    Type::Enum(enum_name.clone())
                } else if let Some((_, ptys, ret)) = self.fns.get(n) {
                    Type::Fn(ptys.clone(), Box::new(ret.clone()))
                } else {
                    Type::I64
                }
            }
            ast::Expr::BinOp(l, op, r, _) => match op {
                BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::Le
                | BinOp::Ge
                | BinOp::And
                | BinOp::Or => Type::Bool,
                _ => {
                    let lt = self.expr_ty_ast(l);
                    let rt = self.expr_ty_ast(r);
                    if lt.is_int() && rt.is_float() {
                        rt
                    } else if lt.is_float() && rt.is_int() {
                        lt
                    } else {
                        lt
                    }
                }
            },
            ast::Expr::UnaryOp(UnaryOp::Not, _, _) => Type::Bool,
            ast::Expr::UnaryOp(_, e, _) => self.expr_ty_ast(e),
            ast::Expr::Call(callee, args, _) => {
                if let ast::Expr::Ident(n, _) = callee.as_ref() {
                    if n == "rc" && args.len() == 1 {
                        return Type::Rc(Box::new(self.expr_ty_ast(&args[0])));
                    }
                    if n == "to_string" {
                        return Type::String;
                    }
                    if let Some((_, _, r)) = self.fns.get(n.as_str()) {
                        return r.clone();
                    }
                    if let Some(ty) = Self::builtin_ret_ty(n) {
                        return ty;
                    }
                    if let Some(gf) = self.generic_fns.get(n.as_str()) {
                        let gf = gf.clone();
                        let arg_tys: Vec<Type> = args.iter().map(|e| self.expr_ty_ast(e)).collect();
                        let mut type_map = HashMap::new();
                        for (i, p) in gf.params.iter().enumerate() {
                            if let Some(Type::Param(tp)) = &p.ty {
                                if i < arg_tys.len() {
                                    type_map.insert(tp.clone(), arg_tys[i].clone());
                                }
                            }
                        }
                        let mangled = Self::mangle_generic(n, &type_map, &gf.type_params);
                        if let Some((_, _, r)) = self.fns.get(&mangled) {
                            return r.clone();
                        }
                        if let Some(ret) = &gf.ret {
                            return Self::substitute_type(ret, &type_map);
                        }
                        let depth = self.infer_depth.get();
                        if depth < 4 {
                            self.infer_depth.set(depth + 1);
                            let inferred = self.infer_ret_ast(&gf);
                            self.infer_depth.set(depth);
                            let subst = Self::substitute_type(&inferred, &type_map);
                            if subst != Type::I64 {
                                return subst;
                            }
                        }
                    }
                }
                Type::I64
            }
            ast::Expr::Ternary(_, t, _, _) => self.expr_ty_ast(t),
            ast::Expr::As(_, ty, _) => ty.clone(),
            ast::Expr::Struct(name, _, _) => {
                if let Some((enum_name, _)) = self.variant_tags.get(name.as_str()) {
                    Type::Enum(enum_name.clone())
                } else {
                    Type::Struct(name.clone())
                }
            }
            ast::Expr::Field(obj, f, _) => {
                if let Type::Struct(n) = self.expr_ty_ast(obj) {
                    if let Some(fields) = self.structs.get(&n) {
                        if let Some((_, ty)) = fields.iter().find(|(name, _)| name == f) {
                            return ty.clone();
                        }
                    }
                }
                Type::I64
            }
            ast::Expr::Array(elems, _) => {
                let et = elems
                    .first()
                    .map(|e| self.expr_ty_ast(e))
                    .unwrap_or(Type::I64);
                Type::Array(Box::new(et), elems.len())
            }
            ast::Expr::Tuple(elems, _) => {
                Type::Tuple(elems.iter().map(|e| self.expr_ty_ast(e)).collect())
            }
            ast::Expr::Index(arr, _, _) => match self.expr_ty_ast(arr) {
                Type::Array(et, _) => *et,
                Type::Vec(et) => *et,
                Type::Map(_, vt) => *vt,
                Type::Tuple(tys) => tys.first().cloned().unwrap_or(Type::I64),
                _ => Type::I64,
            },
            ast::Expr::Lambda(params, ret, body, _) => {
                let ptys: Vec<Type> = params
                    .iter()
                    .map(|p| p.ty.clone().unwrap_or(Type::I64))
                    .collect();
                let ret_ty = ret.clone().unwrap_or_else(|| match body.last() {
                    Some(ast::Stmt::Expr(e)) => self.expr_ty_ast(e),
                    _ => Type::Void,
                });
                Type::Fn(ptys, Box::new(ret_ty))
            }
            ast::Expr::Pipe(_, right, _, _) => match self.expr_ty_ast(right) {
                Type::Fn(_, ret) => *ret,
                _ => Type::I64,
            },
            ast::Expr::Placeholder(_) => Type::I64,
            ast::Expr::Ref(inner, _) => Type::Ptr(Box::new(self.expr_ty_ast(inner))),
            ast::Expr::Deref(inner, _) => match self.expr_ty_ast(inner) {
                Type::Ptr(inner_ty) | Type::Rc(inner_ty) => *inner_ty,
                _ => Type::I64,
            },
            ast::Expr::ListComp(body, _, _, _, _, _) => Type::Ptr(Box::new(self.expr_ty_ast(body))),
            ast::Expr::Syscall(_, _) => Type::I64,
            ast::Expr::Embed(_, _) => Type::String,
            ast::Expr::Method(obj, m, _, _) => {
                let obj_ty = self.expr_ty_ast(obj);
                if matches!(obj_ty, Type::String) {
                    match m.as_str() {
                        "contains" | "starts_with" | "ends_with" => Type::Bool,
                        "char_at" => Type::I64,
                        "slice" => Type::String,
                        "len" => Type::I64,
                        _ => Type::I64,
                    }
                } else if let Type::Struct(ref type_name) = obj_ty {
                    let method_name = format!("{type_name}_{m}");
                    if let Some((_, _, ret)) = self.fns.get(&method_name) {
                        ret.clone()
                    } else {
                        Type::I64
                    }
                } else {
                    Type::I64
                }
            }
            ast::Expr::IfExpr(i) => match i.then.last() {
                Some(ast::Stmt::Expr(e)) => self.expr_ty_ast(e),
                _ => Type::I64,
            },
            ast::Expr::Block(stmts, _) => match stmts.last() {
                Some(ast::Stmt::Expr(e)) => self.expr_ty_ast(e),
                _ => Type::Void,
            },
            ast::Expr::Query(_, _, _) => Type::Void,
            ast::Expr::Spawn(_, _) => Type::I64,
            ast::Expr::Send(_, _, _, _) => Type::Void,
            ast::Expr::Receive(_, _) => Type::Void,
            ast::Expr::Yield(inner, _) => self.expr_ty_ast(inner),
            ast::Expr::DispatchBlock(_, body, _) => match body.last() {
                Some(ast::Stmt::Ret(Some(e), _)) => Type::Coroutine(Box::new(self.expr_ty_ast(e))),
                Some(ast::Stmt::Expr(e)) => Type::Coroutine(Box::new(self.expr_ty_ast(e))),
                _ => Type::Coroutine(Box::new(Type::I64)),
            },
            ast::Expr::StoreQuery(_, _, _)
            | ast::Expr::StoreCount(_, _)
            | ast::Expr::StoreAll(_, _) => Type::I64,
            ast::Expr::ChannelCreate(ty, _, _) => Type::Channel(Box::new(ty.clone())),
            ast::Expr::ChannelSend(_, _, _) => Type::Void,
            ast::Expr::ChannelRecv(_, _) => Type::I64,
            ast::Expr::Select(_, _, _) => Type::I64,
        }
    }

    pub(crate) fn infer_ret_ast(&self, f: &ast::Fn) -> Type {
        let mut locals = HashMap::new();
        for s in &f.body {
            if let ast::Stmt::Bind(b) = s {
                let ty = self.expr_ty_ast_with_locals(&b.value, &locals);
                locals.insert(b.name.clone(), ty);
            }
        }
        let mut found = Type::Void;
        self.collect_ret_types_body(&f.body, &mut found, &locals);
        found
    }

    fn collect_ret_types_body(
        &self,
        body: &[ast::Stmt],
        found: &mut Type,
        locals: &HashMap<String, Type>,
    ) {
        for (i, s) in body.iter().enumerate() {
            let is_last = i + 1 == body.len();
            match s {
                ast::Stmt::Ret(Some(e), _) => {
                    let ty = self.expr_ty_ast_with_locals(e, locals);
                    Self::pick_better_ret(found, &ty);
                }
                ast::Stmt::Expr(e) if is_last => {
                    let ty = self.expr_ty_ast_with_locals(e, locals);
                    Self::pick_better_ret(found, &ty);
                }
                ast::Stmt::If(iff) => {
                    self.collect_ret_types_body(&iff.then, found, locals);
                    for (_, blk) in &iff.elifs {
                        self.collect_ret_types_body(blk, found, locals);
                    }
                    if let Some(els) = &iff.els {
                        self.collect_ret_types_body(els, found, locals);
                    }
                    if is_last {
                        if let Some(ty) = self.tail_type_of_if(iff, locals) {
                            Self::pick_better_ret(found, &ty);
                        }
                    }
                }
                ast::Stmt::While(w) => self.collect_ret_types_body(&w.body, found, locals),
                ast::Stmt::For(fl) => self.collect_ret_types_body(&fl.body, found, locals),
                ast::Stmt::Loop(l) => self.collect_ret_types_body(&l.body, found, locals),
                ast::Stmt::Match(m) => {
                    for arm in &m.arms {
                        self.collect_ret_types_body(&arm.body, found, locals);
                    }
                }
                _ => {}
            }
        }
    }

    fn expr_ty_ast_with_locals(&self, expr: &ast::Expr, locals: &HashMap<String, Type>) -> Type {
        if let ast::Expr::Ident(n, _) = expr {
            if let Some(ty) = locals.get(n) {
                return ty.clone();
            }
        }
        if let ast::Expr::BinOp(l, op, r, _) = expr {
            use ast::BinOp::*;
            match op {
                Eq | Ne | Lt | Gt | Le | Ge | And | Or => return Type::Bool,
                _ => {
                    let lt = self.expr_ty_ast_with_locals(l, locals);
                    let rt = self.expr_ty_ast_with_locals(r, locals);
                    if lt == Type::String || rt == Type::String {
                        return Type::String;
                    }
                    if lt.is_int() && rt.is_float() {
                        return rt;
                    }
                    if lt.is_float() && rt.is_int() {
                        return lt;
                    }
                    return lt;
                }
            }
        }
        self.expr_ty_ast(expr)
    }

    fn tail_type_of_if(&self, iff: &ast::If, locals: &HashMap<String, Type>) -> Option<Type> {
        let then_ty = self.tail_type_of_block(&iff.then, locals);
        if let Some(ty) = &then_ty {
            if *ty != Type::I64 && *ty != Type::Void {
                return Some(ty.clone());
            }
        }
        if let Some(els) = &iff.els {
            let else_ty = self.tail_type_of_block(els, locals);
            if let Some(ty) = &else_ty {
                if *ty != Type::I64 && *ty != Type::Void {
                    return Some(ty.clone());
                }
            }
        }
        for (_, blk) in &iff.elifs {
            let ty = self.tail_type_of_block(blk, locals);
            if let Some(ty) = &ty {
                if *ty != Type::I64 && *ty != Type::Void {
                    return Some(ty.clone());
                }
            }
        }
        then_ty
    }

    fn tail_type_of_block(
        &self,
        body: &[ast::Stmt],
        locals: &HashMap<String, Type>,
    ) -> Option<Type> {
        match body.last() {
            Some(ast::Stmt::Expr(e)) => Some(self.expr_ty_ast_with_locals(e, locals)),
            Some(ast::Stmt::Ret(Some(e), _)) => Some(self.expr_ty_ast_with_locals(e, locals)),
            Some(ast::Stmt::If(iff)) => self.tail_type_of_if(iff, locals),
            _ => None,
        }
    }

    pub(crate) fn pick_better_ret(current: &mut Type, candidate: &Type) {
        if *current == Type::Void
            || (*current == Type::I64 && *candidate != Type::I64 && *candidate != Type::Void)
        {
            *current = candidate.clone();
        }
    }

    pub(crate) fn infer_ret_ast_with_params(&mut self, f: &ast::Fn, lookup_name: &str) -> Type {
        let ptys = if let Some((_, ptys, _)) = self.fns.get(lookup_name) {
            ptys.clone()
        } else {
            return self.infer_ret_ast(f);
        };

        let offset = if ptys.len() > f.params.len() {
            ptys.len() - f.params.len()
        } else {
            0
        };

        self.push_scope();
        for (i, p) in f.params.iter().enumerate() {
            if offset + i < ptys.len() {
                let info = super::VarInfo {
                    def_id: self.fresh_id(),
                    ty: ptys[offset + i].clone(),
                    ownership: crate::hir::Ownership::Owned,
                };
                self.define_var(&p.name, info);
            }
        }
        let ret = self.infer_ret_ast(f);
        self.pop_scope();
        ret
    }

    pub(crate) fn refine_ret_from_body(&self, declared: &Type, body: &[hir::Stmt]) -> Type {
        if *declared != Type::Void && *declared != Type::I64 {
            return declared.clone();
        }
        let mut best = declared.clone();
        self.collect_hir_ret_types(body, &mut best);
        if let Some(hir::Stmt::Expr(e)) = body.last() {
            if e.ty != Type::Void && e.ty != Type::I64 {
                if best == Type::Void || best == Type::I64 {
                    best = e.ty.clone();
                }
            }
        }
        best
    }

    fn collect_hir_ret_types(&self, body: &[hir::Stmt], best: &mut Type) {
        for stmt in body {
            match stmt {
                hir::Stmt::Ret(Some(e), _, _) => {
                    if *best == Type::Void || (*best == Type::I64 && e.ty != Type::I64) {
                        *best = e.ty.clone();
                    }
                }
                hir::Stmt::If(i) => {
                    self.collect_hir_ret_types(&i.then, best);
                    for (_, blk) in &i.elifs {
                        self.collect_hir_ret_types(blk, best);
                    }
                    if let Some(els) = &i.els {
                        self.collect_hir_ret_types(els, best);
                    }
                }
                hir::Stmt::While(w) => self.collect_hir_ret_types(&w.body, best),
                hir::Stmt::For(f) => self.collect_hir_ret_types(&f.body, best),
                hir::Stmt::Loop(l) => self.collect_hir_ret_types(&l.body, best),
                hir::Stmt::Match(m) => {
                    for arm in &m.arms {
                        self.collect_hir_ret_types(&arm.body, best);
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn infer_coroutine_yield_type(&self, body: &[hir::Stmt]) -> Type {
        for stmt in body {
            if let Some(ty) = self.find_yield_type_stmt(stmt) {
                return ty;
            }
        }
        if let Some(hir::Stmt::Ret(Some(e), ty, _)) = body.last() {
            let _ = e;
            return ty.clone();
        }
        Type::I64
    }

    fn find_yield_type_stmt(&self, stmt: &hir::Stmt) -> Option<Type> {
        match stmt {
            hir::Stmt::Expr(e) => self.find_yield_type_expr(e),
            hir::Stmt::If(i) => {
                for s in &i.then {
                    if let Some(ty) = self.find_yield_type_stmt(s) {
                        return Some(ty);
                    }
                }
                for (_, blk) in &i.elifs {
                    for s in blk {
                        if let Some(ty) = self.find_yield_type_stmt(s) {
                            return Some(ty);
                        }
                    }
                }
                if let Some(els) = &i.els {
                    for s in els {
                        if let Some(ty) = self.find_yield_type_stmt(s) {
                            return Some(ty);
                        }
                    }
                }
                None
            }
            hir::Stmt::While(w) => {
                for s in &w.body {
                    if let Some(ty) = self.find_yield_type_stmt(s) {
                        return Some(ty);
                    }
                }
                None
            }
            hir::Stmt::For(f) => {
                for s in &f.body {
                    if let Some(ty) = self.find_yield_type_stmt(s) {
                        return Some(ty);
                    }
                }
                None
            }
            hir::Stmt::Loop(l) => {
                for s in &l.body {
                    if let Some(ty) = self.find_yield_type_stmt(s) {
                        return Some(ty);
                    }
                }
                None
            }
            hir::Stmt::Ret(Some(e), _, _) => Some(e.ty.clone()),
            _ => None,
        }
    }

    fn find_yield_type_expr(&self, e: &hir::Expr) -> Option<Type> {
        if let hir::ExprKind::Yield(inner) = &e.kind {
            return Some(inner.ty.clone());
        }
        None
    }

    pub(crate) fn infer_dyn_method_ret(&self, trait_name: &str, method: &str) -> Type {
        for (type_name, impls) in &self.trait_impls {
            if impls.contains(&trait_name.to_string()) {
                let fn_name = format!("{type_name}_{method}");
                if let Some((_, _, ret)) = self.fns.get(&fn_name) {
                    return ret.clone();
                }
            }
        }
        Type::I64
    }

    pub(crate) fn infer_field_ty(&mut self, f: &ast::Field) -> Type {
        f.default
            .as_ref()
            .map(|e| self.expr_ty_ast(e))
            .unwrap_or_else(|| self.infer_ctx.fresh_var())
    }

    pub(crate) fn builtin_ret_ty(name: &str) -> Option<Type> {
        match name {
            "__ln" | "__log2" | "__log10" | "__exp" | "__exp2" | "__powf" | "__copysign"
            | "__fma" | "__time_monotonic" => Some(Type::F64),
            "__fmt_float" | "__fmt_hex" | "__fmt_oct" | "__fmt_bin" | "__string_from_raw"
            | "__string_from_ptr" => Some(Type::String),
            "__get_args" => Some(Type::Vec(Box::new(Type::String))),
            "__file_exists" => Some(Type::Bool),
            "__sleep_ms" => Some(Type::Void),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn builtin_param_tys(name: &str) -> Option<Vec<Type>> {
        match name {
            "__ln" | "__log2" | "__log10" | "__exp" | "__exp2" => Some(vec![Type::F64]),
            "__powf" | "__copysign" => Some(vec![Type::F64, Type::F64]),
            "__fma" => Some(vec![Type::F64, Type::F64, Type::F64]),
            "__fmt_float" => Some(vec![Type::F64, Type::I64]),
            "__fmt_hex" | "__fmt_oct" | "__fmt_bin" | "__sleep_ms" => Some(vec![Type::I64]),
            "__string_from_ptr" => Some(vec![Type::Ptr(Box::new(Type::I8))]),
            "__string_from_raw" => Some(vec![Type::Ptr(Box::new(Type::I8)), Type::I64, Type::I64]),
            "__file_exists" => Some(vec![Type::String]),
            _ => None,
        }
    }

    pub(crate) fn needs_int_coercion(from: &Type, to: &Type) -> Option<CoercionKind> {
        if !from.is_int() || !to.is_int() {
            return None;
        }
        let fb = from.bits();
        let tb = to.bits();
        if fb == tb {
            return None;
        }
        if fb < tb {
            Some(CoercionKind::IntWiden {
                from_bits: fb,
                to_bits: tb,
                signed: from.is_signed(),
            })
        } else {
            Some(CoercionKind::IntTrunc {
                from_bits: fb,
                to_bits: tb,
            })
        }
    }

    pub(crate) fn coerce_binop_operands(
        &self,
        lhs: hir::Expr,
        rhs: hir::Expr,
    ) -> (hir::Expr, hir::Expr) {
        let lt = lhs.ty.clone();
        let rt = rhs.ty.clone();
        if lt.is_int() && rt.is_float() {
            let span = lhs.span;
            return (
                hir::Expr {
                    kind: hir::ExprKind::Coerce(
                        Box::new(lhs),
                        CoercionKind::IntToFloat {
                            signed: lt.is_signed(),
                        },
                    ),
                    ty: rt,
                    span,
                },
                rhs,
            );
        }
        if lt.is_float() && rt.is_int() {
            let span = rhs.span;
            return (
                lhs,
                hir::Expr {
                    kind: hir::ExprKind::Coerce(
                        Box::new(rhs),
                        CoercionKind::IntToFloat {
                            signed: rt.is_signed(),
                        },
                    ),
                    ty: lt,
                    span,
                },
            );
        }
        if lt.is_float() && rt.is_float() && lt.bits() != rt.bits() {
            if lt.bits() < rt.bits() {
                let span = lhs.span;
                return (
                    hir::Expr {
                        kind: hir::ExprKind::Coerce(Box::new(lhs), CoercionKind::FloatWiden),
                        ty: rt,
                        span,
                    },
                    rhs,
                );
            } else {
                let span = rhs.span;
                return (
                    lhs,
                    hir::Expr {
                        kind: hir::ExprKind::Coerce(Box::new(rhs), CoercionKind::FloatWiden),
                        ty: lt,
                        span,
                    },
                );
            }
        }
        if !lt.is_int() || !rt.is_int() || lt.bits() == rt.bits() {
            return (lhs, rhs);
        }
        if lt.bits() > rt.bits() {
            let coercion = CoercionKind::IntWiden {
                from_bits: rt.bits(),
                to_bits: lt.bits(),
                signed: rt.is_signed(),
            };
            let span = rhs.span;
            (
                lhs,
                hir::Expr {
                    kind: hir::ExprKind::Coerce(Box::new(rhs), coercion),
                    ty: lt,
                    span,
                },
            )
        } else {
            let coercion = CoercionKind::IntWiden {
                from_bits: lt.bits(),
                to_bits: rt.bits(),
                signed: lt.is_signed(),
            };
            let span = lhs.span;
            (
                hir::Expr {
                    kind: hir::ExprKind::Coerce(Box::new(lhs), coercion),
                    ty: rt,
                    span,
                },
                rhs,
            )
        }
    }
}
