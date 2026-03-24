//! Typer: AST → HIR lowering pass.
//!
//! Performs:
//! - Name resolution (every identifier → DefId)
//! - Type inference (bidirectional: synthesis + checking)
//! - Generic monomorphization
//! - Method resolution
//! - Coercion insertion
//!
//! The typer does NOT emit LLVM IR. It produces a fully typed HIR
//! that codegen can read without re-discovering types.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::ast::{self, BinOp, Span, UnaryOp};
use crate::hir::{self, CoercionKind, DefId, Ownership};
use crate::types::Type;

#[derive(Debug, Clone)]
pub(crate) struct VarInfo {
    pub(crate) def_id: DefId,
    pub(crate) ty: Type,
    #[allow(dead_code)]
    pub(crate) ownership: Ownership,
}

mod mono;
mod resolve;

pub struct Typer {
    pub(crate) next_id: u32,
    pub(crate) scopes: Vec<HashMap<String, VarInfo>>,
    pub(crate) fns: HashMap<String, (DefId, Vec<Type>, Type)>,
    pub(crate) structs: HashMap<String, Vec<(String, Type)>>,
    pub(crate) enums: HashMap<String, Vec<(String, Vec<Type>)>>,
    pub(crate) variant_tags: HashMap<String, (String, u32)>,
    pub(crate) generic_fns: HashMap<String, ast::Fn>,
    pub(crate) generic_enums: HashMap<String, ast::EnumDef>,
    pub(crate) generic_types: HashMap<String, ast::TypeDef>,
    pub(crate) methods: HashMap<String, Vec<ast::Fn>>,
    pub(crate) mono_fns: Vec<hir::Fn>,
    pub(crate) mono_enums: Vec<hir::EnumDef>,
    pub(crate) source_dir: Option<PathBuf>,
    pub(crate) test_mode: bool,
    pub(crate) actors: HashMap<String, (DefId, Vec<(String, Type)>, Vec<(String, Vec<Type>, u32)>)>,
    pub(crate) store_schemas: HashMap<String, Vec<(String, Type)>>,
    pub(crate) mono_depth: u32,
    pub(crate) traits: HashMap<String, Vec<TraitMethodSig>>,
    pub(crate) trait_impls: HashMap<String, Vec<String>>,
    infer_depth: Cell<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct TraitMethodSig {
    pub(crate) name: String,
    pub(crate) _params: Vec<(String, Option<Type>)>,
    pub(crate) _ret: Option<Type>,
    pub(crate) has_default: bool,
}

impl Typer {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            scopes: Vec::new(),
            fns: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            variant_tags: HashMap::new(),
            generic_fns: HashMap::new(),
            generic_enums: HashMap::new(),
            generic_types: HashMap::new(),
            methods: HashMap::new(),
            mono_fns: Vec::new(),
            mono_enums: Vec::new(),
            source_dir: None,
            test_mode: false,
            actors: HashMap::new(),
            store_schemas: HashMap::new(),
            mono_depth: 0,
            traits: HashMap::new(),
            trait_impls: HashMap::new(),
            infer_depth: Cell::new(0),
        }
    }

    pub fn set_source_dir(&mut self, dir: PathBuf) {
        self.source_dir = Some(dir);
    }

    pub fn set_test_mode(&mut self, enabled: bool) {
        self.test_mode = enabled;
    }

    fn fresh_id(&mut self) -> DefId {
        let id = DefId(self.next_id);
        self.next_id += 1;
        id
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define_var(&mut self, name: &str, info: VarInfo) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), info);
        }
    }

    fn find_var(&self, name: &str) -> Option<&VarInfo> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    fn update_var(&mut self, name: &str, info: VarInfo) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), info);
                return;
            }
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), info);
        }
    }

    fn resolve_ty(&self, ty: Type) -> Type {
        match &ty {
            Type::Struct(n) if self.enums.contains_key(n) => Type::Enum(n.clone()),
            _ => ty,
        }
    }

    fn ownership_for_type(ty: &Type) -> Ownership {
        match ty {
            Type::Rc(_) => Ownership::Rc,
            Type::Ptr(_) => Ownership::Raw,
            _ => Ownership::Owned,
        }
    }

    fn expr_ty_ast(&self, expr: &ast::Expr) -> Type {
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
                    // Builtin return types (not in self.fns)
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
                        // No explicit return — infer from body with substituted param types
                        // Guard against infinite recursion (e.g. fact(n) calling fact(n-1))
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
            ast::Expr::IfExpr(i) => {
                match i.then.last() {
                    Some(ast::Stmt::Expr(e)) => self.expr_ty_ast(e),
                    _ => Type::I64,
                }
            }
            ast::Expr::Block(stmts, _) => match stmts.last() {
                Some(ast::Stmt::Expr(e)) => self.expr_ty_ast(e),
                _ => Type::Void,
            },
            ast::Expr::Query(_, _, _) => Type::Void,
            ast::Expr::Spawn(_, _) => Type::I64,
            ast::Expr::Send(_, _, _, _) => Type::Void,
            ast::Expr::Receive(_, _) => Type::Void,
            ast::Expr::Yield(inner, _) => self.expr_ty_ast(inner),
            ast::Expr::DispatchBlock(_, body, _) => {
                match body.last() {
                    Some(ast::Stmt::Ret(Some(e), _)) => Type::Coroutine(Box::new(self.expr_ty_ast(e))),
                    Some(ast::Stmt::Expr(e)) => Type::Coroutine(Box::new(self.expr_ty_ast(e))),
                    _ => Type::Coroutine(Box::new(Type::I64)),
                }
            }
            ast::Expr::StoreQuery(_, _, _) | ast::Expr::StoreCount(_, _) | ast::Expr::StoreAll(_, _) => Type::I64,
            ast::Expr::ChannelCreate(ty, _, _) => Type::Channel(Box::new(ty.clone())),
            ast::Expr::ChannelSend(_, _, _) => Type::Void,
            ast::Expr::ChannelRecv(_, _) => Type::I64, // will be refined during lowering
            ast::Expr::Select(_, _, _) => Type::I64,
        }
    }

    fn infer_ret_ast(&self, f: &ast::Fn) -> Type {
        match f.body.last() {
            Some(ast::Stmt::Expr(e)) => self.expr_ty_ast(e),
            Some(ast::Stmt::Ret(Some(e), _)) => self.expr_ty_ast(e),
            Some(ast::Stmt::Match(_)) => Type::I64,
            _ => Type::Void,
        }
    }

    /// Re-infer return type after param types have been resolved.
    /// Temporarily binds params in scope so expr_ty_ast can see them.
    fn infer_ret_ast_with_params(&mut self, f: &ast::Fn, lookup_name: &str) -> Type {
        let ptys = if let Some((_, ptys, _)) = self.fns.get(lookup_name) {
            ptys.clone()
        } else {
            return self.infer_ret_ast(f);
        };

        // Methods have self prepended; skip it when binding param names
        let offset = if ptys.len() > f.params.len() { ptys.len() - f.params.len() } else { 0 };

        self.push_scope();
        for (i, p) in f.params.iter().enumerate() {
            if offset + i < ptys.len() {
                let info = VarInfo {
                    def_id: self.fresh_id(),
                    ty: ptys[offset + i].clone(),
                    ownership: Ownership::Owned,
                };
                self.define_var(&p.name, info);
            }
        }
        let ret = self.infer_ret_ast(f);
        self.pop_scope();
        ret
    }

    fn infer_coroutine_yield_type(&self, body: &[hir::Stmt]) -> Type {
        // Walk the HIR body to find yield statements and infer the type
        for stmt in body {
            if let Some(ty) = self.find_yield_type_stmt(stmt) {
                return ty;
            }
        }
        // Fallback: check last expression for return type
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
                for s in &i.then { if let Some(ty) = self.find_yield_type_stmt(s) { return Some(ty); } }
                for (_, blk) in &i.elifs { for s in blk { if let Some(ty) = self.find_yield_type_stmt(s) { return Some(ty); } } }
                if let Some(els) = &i.els { for s in els { if let Some(ty) = self.find_yield_type_stmt(s) { return Some(ty); } } }
                None
            }
            hir::Stmt::While(w) => { for s in &w.body { if let Some(ty) = self.find_yield_type_stmt(s) { return Some(ty); } } None }
            hir::Stmt::For(f) => { for s in &f.body { if let Some(ty) = self.find_yield_type_stmt(s) { return Some(ty); } } None }
            hir::Stmt::Loop(l) => { for s in &l.body { if let Some(ty) = self.find_yield_type_stmt(s) { return Some(ty); } } None }
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

    fn infer_dyn_method_ret(&self, trait_name: &str, method: &str) -> Type {
        // Look through trait_impls to find the return type for this method
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

    fn infer_field_ty(&self, f: &ast::Field) -> Type {
        f.default
            .as_ref()
            .map(|e| self.expr_ty_ast(e))
            .unwrap_or(Type::I64)
    }

    /// Return type for builtin functions (not registered in self.fns).
    fn builtin_ret_ty(name: &str) -> Option<Type> {
        match name {
            "__ln" | "__log2" | "__log10" | "__exp" | "__exp2" | "__powf"
            | "__copysign" | "__fma" | "__time_monotonic" => Some(Type::F64),
            "__fmt_float" | "__fmt_hex" | "__fmt_oct" | "__fmt_bin"
            | "__string_from_raw" | "__string_from_ptr" => Some(Type::String),
            "__get_args" => Some(Type::Vec(Box::new(Type::String))),
            "__file_exists" => Some(Type::Bool),
            "__sleep_ms" => Some(Type::Void),
            _ => None,
        }
    }

    /// Parameter types for builtin functions, for body-driven inference.
    fn builtin_param_tys(name: &str) -> Option<Vec<Type>> {
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

    fn needs_int_coercion(from: &Type, to: &Type) -> Option<CoercionKind> {
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

    fn coerce_binop_operands(&self, lhs: hir::Expr, rhs: hir::Expr) -> (hir::Expr, hir::Expr) {
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

    pub fn lower_program(&mut self, prog: &ast::Program) -> Result<hir::Program, String> {
        self.register_prelude_types();

        for d in &prog.decls {
            match d {
                ast::Decl::Fn(f) if Self::is_generic_fn(f) => {
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
                        .map(|f| {
                            (
                                f.name.clone(),
                                f.ty.clone().unwrap_or(Type::I64),
                            )
                        })
                        .collect();
                    self.structs.insert(format!("__store_{}", sd.name), fields.clone());
                    self.store_schemas.insert(sd.name.clone(), fields);
                }
                ast::Decl::Trait(td) => {
                    self.declare_trait_def(td);
                }
                ast::Decl::Impl(_) => {}
            }
        }

        // Register impl block methods (after all types and traits are declared)
        for d in &prog.decls {
            if let ast::Decl::Impl(ib) = d {
                self.declare_impl_block(ib)?;
            }
        }

        // Bidirectional param type inference: refine Type::Inferred slots
        // by analyzing function bodies and call sites.
        self.infer_param_types(prog);

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
                    self.fns.insert(fn_name.clone(), (test_id, vec![], Type::Void));
                    hir_fns.push(test_fn);
                    test_fns.push((tb.name.clone(), fn_name));
                }
                _ => {}
            }
        }

        // Lower actor definitions
        for d in &prog.decls {
            if let ast::Decl::Actor(ad) = d {
                let ha = self.lower_actor_def(ad)?;
                hir_actors.push(ha);
            }
        }

        // Lower store definitions
        for d in &prog.decls {
            if let ast::Decl::Store(sd) = d {
                let hs = self.lower_store_def(sd)?;
                hir_stores.push(hs);
            }
        }

        // Lower trait impl blocks
        let mut hir_trait_impls = Vec::new();
        for d in &prog.decls {
            if let ast::Decl::Impl(ib) = d {
                let hi = self.lower_impl_block(ib)?;
                hir_trait_impls.push(hi);
            }
        }

        if self.test_mode && !test_fns.is_empty() {
            let main_fn = self.build_test_runner(&test_fns);
            self.fns.insert("main".into(), (main_fn.def_id, vec![], Type::I32));
            hir_fns.push(main_fn);
        }

        hir_fns.extend(self.mono_fns.drain(..));
        hir_enums.extend(self.mono_enums.drain(..));

        Ok(hir::Program {
            fns: hir_fns,
            types: hir_types,
            enums: hir_enums,
            externs: hir_externs,
            err_defs: hir_err_defs,
            actors: hir_actors,
            stores: hir_stores,
            trait_impls: hir_trait_impls,
        })
    }

    fn lower_actor_def(&mut self, ad: &ast::ActorDef) -> Result<hir::ActorDef, String> {
        let (id, _, ref handler_info) = self
            .actors
            .get(&ad.name)
            .ok_or_else(|| format!("undeclared actor: {}", ad.name))?
            .clone();

        let fields: Vec<hir::Field> = ad
            .fields
            .iter()
            .map(|f| {
                let ty = f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f));
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
            // Bind actor state fields as variables
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
            for p in &h.params {
                let pid = self.fresh_id();
                let ty = p.ty.clone().unwrap_or(Type::I64);
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

    fn lower_store_def(&mut self, sd: &ast::StoreDef) -> Result<hir::StoreDef, String> {
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

    fn lower_impl_block(&mut self, ib: &ast::ImplBlock) -> Result<hir::TraitImpl, String> {
        let mut hir_methods = Vec::new();
        for m in &ib.methods {
            let hm = self.lower_method(&ib.type_name, m)?;
            hir_methods.push(hm);
        }
        Ok(hir::TraitImpl {
            trait_name: ib.trait_name.clone(),
            type_name: ib.type_name.clone(),
            methods: hir_methods,
            span: ib.span,
        })
    }

    fn lower_fn(&mut self, f: &ast::Fn) -> Result<hir::Fn, String> {
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

    fn lower_type_def(&mut self, td: &ast::TypeDef) -> Result<hir::TypeDef, String> {
        let id = self.fresh_id();
        let fields: Vec<hir::Field> = td
            .fields
            .iter()
            .map(|f| {
                let ty = f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f));
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

    fn lower_method(&mut self, type_name: &str, m: &ast::Fn) -> Result<hir::Fn, String> {
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

    fn lower_enum_def(&mut self, ed: &ast::EnumDef) -> hir::EnumDef {
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

    fn lower_extern(&self, ef: &ast::ExternFn) -> hir::ExternFn {
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

    fn lower_err_def(&mut self, ed: &ast::ErrDef) -> hir::ErrDef {
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

    fn lower_block(&mut self, block: &ast::Block, ret_ty: &Type) -> Result<hir::Block, String> {
        self.push_scope();
        let mut stmts = Vec::new();
        for s in block {
            let hs = self.lower_stmt(s, ret_ty)?;
            stmts.push(hs);
        }
        // Check if last statement is a return/break/continue — drops are unreachable after those
        let ends_with_jump = stmts.last().map_or(false, |s| {
            matches!(s, hir::Stmt::Ret(..) | hir::Stmt::Break(..) | hir::Stmt::Continue(..))
        });
        if ends_with_jump {
            // Insert drops BEFORE the jump so heap resources are freed on
            // early-exit paths (return / break / continue).
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
        // Emit drops in reverse definition order (LIFO).
        // Only drop types with heap resources that need cleanup.
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

    /// Returns true if a type has heap resources that need cleanup at scope exit.
    fn needs_drop(ty: &Type) -> bool {
        matches!(ty, Type::String | Type::Vec(_) | Type::Map(_, _) | Type::Rc(_) | Type::Weak(_))
    }

    // ── Exhaustiveness checking ──────────────────────────────────────

    fn check_exhaustiveness(&self, subject_ty: &Type, arms: &[hir::Arm], _span: Span) -> Result<(), String> {
        // Only guard-free arms contribute to exhaustiveness
        let pats: Vec<&hir::Pat> = arms.iter()
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

        // Warn about unreachable patterns (duplicates)
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
        // Flatten Or patterns
        let mut flat: Vec<&hir::Pat> = Vec::new();
        for p in pats {
            Self::flatten_or_pat(p, &mut flat);
        }

        // Wildcard or binding catches everything
        if flat.iter().any(|p| matches!(p, hir::Pat::Wild(_) | hir::Pat::Bind(..))) {
            return vec![];
        }

        // Resolve the type (Struct → Enum if applicable)
        let ty = self.resolve_ty(ty.clone());

        match &ty {
            Type::Enum(name) => {
                let variants = match self.enums.get(name) {
                    Some(v) => v,
                    None => return vec![],
                };
                let mut missing = Vec::new();
                for (vname, field_tys) in variants {
                    let sub_lists: Vec<&Vec<hir::Pat>> = flat.iter()
                        .filter_map(|p| match p {
                            hir::Pat::Ctor(n, _, subs, _) if n == vname => Some(subs),
                            _ => None,
                        })
                        .collect();

                    if sub_lists.is_empty() {
                        // Variant completely uncovered
                        if field_tys.is_empty() {
                            missing.push(vname.clone());
                        } else {
                            let fields = vec!["_"; field_tys.len()].join(", ");
                            missing.push(format!("{}({})", vname, fields));
                        }
                    } else if !field_tys.is_empty() {
                        // Recursively check each field position
                        for (i, ft) in field_tys.iter().enumerate() {
                            let col: Vec<&hir::Pat> = sub_lists.iter()
                                .filter_map(|subs| subs.get(i))
                                .collect();
                            let sub_missing = self.find_missing_patterns(&col, ft);
                            for sm in &sub_missing {
                                let fields: Vec<String> = field_tys.iter().enumerate()
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
                if !has_true { missing.push("true".to_string()); }
                if !has_false { missing.push("false".to_string()); }
                missing
            }
            Type::I64 | Type::F64 | Type::String => {
                // Infinite-domain types: always non-exhaustive without wildcard
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

    fn lower_stmt(&mut self, stmt: &ast::Stmt, ret_ty: &Type) -> Result<hir::Stmt, String> {
        match stmt {
            ast::Stmt::Bind(b) => {
                let value = self.lower_expr(&b.value)?;
                let ty = if let Some(ref ann) = b.ty {
                    self.resolve_ty(ann.clone())
                } else {
                    value.ty.clone()
                };
                let ownership = Self::ownership_for_type(&ty);
                let id = self.fresh_id();
                if let Some(existing) = self.find_var(&b.name) {
                    let id = existing.def_id;
                    let existing_ty = existing.ty.clone();
                    let value = self.maybe_coerce_to(value, &existing_ty);
                    self.update_var(
                        &b.name,
                        VarInfo {
                            def_id: id,
                            ty: existing_ty.clone(),
                            ownership,
                        },
                    );
                    Ok(hir::Stmt::Bind(hir::Bind {
                        def_id: id,
                        name: b.name.clone(),
                        value,
                        ty: existing_ty,
                        ownership,
                        span: b.span,
                    }))
                } else {
                    self.define_var(
                        &b.name,
                        VarInfo {
                            def_id: id,
                            ty: ty.clone(),
                            ownership,
                        },
                    );
                    Ok(hir::Stmt::Bind(hir::Bind {
                        def_id: id,
                        name: b.name.clone(),
                        value,
                        ty,
                        ownership,
                        span: b.span,
                    }))
                }
            }

            ast::Stmt::TupleBind(names, value, span) => {
                let hval = self.lower_expr(value)?;
                let tys = match &hval.ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => vec![Type::I64; names.len()],
                };
                let bindings: Vec<(DefId, String, Type)> = names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| {
                        let ty = tys.get(i).cloned().unwrap_or(Type::I64);
                        let id = self.fresh_id();
                        self.define_var(
                            n,
                            VarInfo {
                                def_id: id,
                                ty: ty.clone(),
                                ownership: Self::ownership_for_type(&ty),
                            },
                        );
                        (id, n.clone(), ty)
                    })
                    .collect();
                Ok(hir::Stmt::TupleBind(bindings, hval, *span))
            }

            ast::Stmt::Assign(target, value, span) => {
                let ht = self.lower_expr(target)?;
                let hv = self.lower_expr(value)?;
                Ok(hir::Stmt::Assign(ht, hv, *span))
            }

            ast::Stmt::Expr(e) => {
                let he = self.lower_expr(e)?;
                Ok(hir::Stmt::Expr(he))
            }

            ast::Stmt::If(i) => {
                let hi = self.lower_if(i, ret_ty)?;
                Ok(hir::Stmt::If(hi))
            }

            ast::Stmt::While(w) => {
                let cond = self.lower_expr(&w.cond)?;
                let body = self.lower_block(&w.body, ret_ty)?;
                Ok(hir::Stmt::While(hir::While {
                    cond,
                    body,
                    span: w.span,
                }))
            }

            ast::Stmt::For(f) => {
                let iter = self.lower_expr(&f.iter)?;
                let end = f.end.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                let step = f.step.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                let bind_ty = if end.is_some() || matches!(iter.ty, Type::I64) {
                    Type::I64
                } else {
                    match &iter.ty {
                        Type::Array(et, _) => *et.clone(),
                        Type::Ptr(et) => *et.clone(),
                        _ => Type::I64,
                    }
                };
                let bind_id = self.fresh_id();
                self.push_scope();
                self.define_var(
                    &f.bind,
                    VarInfo {
                        def_id: bind_id,
                        ty: bind_ty.clone(),
                        ownership: Ownership::Owned,
                    },
                );
                let body = self.lower_block_no_scope(&f.body, ret_ty)?;
                self.pop_scope();
                Ok(hir::Stmt::For(hir::For {
                    bind_id,
                    bind: f.bind.clone(),
                    bind_ty,
                    iter,
                    end,
                    step,
                    body,
                    span: f.span,
                }))
            }

            ast::Stmt::Loop(l) => {
                let body = self.lower_block(&l.body, ret_ty)?;
                Ok(hir::Stmt::Loop(hir::Loop { body, span: l.span }))
            }

            ast::Stmt::Ret(val, span) => {
                let hval = val.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                let hval = hval.map(|v| self.maybe_coerce_to(v, ret_ty));
                Ok(hir::Stmt::Ret(hval, ret_ty.clone(), *span))
            }

            ast::Stmt::Break(val, span) => {
                let hval = val.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                Ok(hir::Stmt::Break(hval, *span))
            }

            ast::Stmt::Continue(span) => Ok(hir::Stmt::Continue(*span)),

            ast::Stmt::Match(m) => {
                let hm = self.lower_match(m, ret_ty)?;
                Ok(hir::Stmt::Match(hm))
            }

            ast::Stmt::Asm(a) => {
                let inputs: Vec<(String, hir::Expr)> = a
                    .inputs
                    .iter()
                    .map(|(c, e)| Ok((c.clone(), self.lower_expr(e)?)))
                    .collect::<Result<_, String>>()?;
                Ok(hir::Stmt::Asm(hir::AsmBlock {
                    template: a.template.clone(),
                    outputs: a.outputs.clone(),
                    inputs,
                    clobbers: a.clobbers.clone(),
                    span: a.span,
                }))
            }

            ast::Stmt::ErrReturn(e, span) => {
                let he = self.lower_expr(e)?;
                Ok(hir::Stmt::ErrReturn(he, ret_ty.clone(), *span))
            }

            ast::Stmt::StoreInsert(store, values, span) => {
                let schema = self.store_schemas.get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                if values.len() != schema.len() {
                    return Err(format!(
                        "store '{store}' has {} fields but {} values given",
                        schema.len(), values.len()
                    ));
                }
                let mut hvalues = Vec::new();
                for (v, (_fname, _fty)) in values.iter().zip(schema.iter()) {
                    hvalues.push(self.lower_expr(v)?);
                }
                Ok(hir::Stmt::StoreInsert(store.clone(), hvalues, *span))
            }

            ast::Stmt::StoreDelete(store, filter, span) => {
                let schema = self.store_schemas.get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, store)?;
                Ok(hir::Stmt::StoreDelete(
                    store.clone(),
                    Box::new(hfilter),
                    *span,
                ))
            }

            ast::Stmt::StoreSet(store, assignments, filter, span) => {
                let schema = self.store_schemas.get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, store)?;
                // Validate and lower each field assignment
                let mut hassigns = Vec::new();
                for (fname, fval) in assignments {
                    if !schema.iter().any(|(n, _)| n == fname) {
                        return Err(format!("store '{store}' has no field '{fname}'"));
                    }
                    hassigns.push((fname.clone(), self.lower_expr(fval)?));
                }
                Ok(hir::Stmt::StoreSet(
                    store.clone(),
                    hassigns,
                    Box::new(hfilter),
                    *span,
                ))
            }

            ast::Stmt::Transaction(body, span) => {
                let hbody = self.lower_block(body, ret_ty)?;
                Ok(hir::Stmt::Transaction(hbody, *span))
            }

            ast::Stmt::ChannelClose(ch, span) => {
                let hch = self.lower_expr(ch)?;
                if !matches!(&hch.ty, Type::Channel(_)) {
                    return Err(format!("close: target must be a Channel, got {}", hch.ty));
                }
                Ok(hir::Stmt::ChannelClose(hch, *span))
            }

            ast::Stmt::Stop(target, span) => {
                let htarget = self.lower_expr(target)?;
                if !matches!(&htarget.ty, Type::ActorRef(_)) {
                    return Err(format!("stop: target must be an ActorRef, got {}", htarget.ty));
                }
                Ok(hir::Stmt::Stop(htarget, *span))
            }
        }
    }

    fn lower_store_filter(
        &mut self,
        filter: &ast::StoreFilter,
        schema: &[(String, Type)],
        store: &str,
    ) -> Result<hir::StoreFilter, String> {
        if !schema.iter().any(|(n, _)| n == &filter.field) {
            return Err(format!("store '{store}' has no field '{}'", filter.field));
        }
        let hvalue = self.lower_expr(&filter.value)?;
        let mut hextra = Vec::new();
        for (lop, cond) in &filter.extra {
            if !schema.iter().any(|(n, _)| n == &cond.field) {
                return Err(format!("store '{store}' has no field '{}'", cond.field));
            }
            let hv = self.lower_expr(&cond.value)?;
            hextra.push((*lop, hir::StoreFilterCond {
                field: cond.field.clone(),
                op: cond.op,
                value: hv,
            }));
        }
        Ok(hir::StoreFilter {
            field: filter.field.clone(),
            op: filter.op,
            value: hvalue,
            span: filter.span,
            extra: hextra,
        })
    }

    fn lower_block_no_scope(
        &mut self,
        block: &ast::Block,
        ret_ty: &Type,
    ) -> Result<hir::Block, String> {
        let mut stmts = Vec::new();
        for s in block {
            stmts.push(self.lower_stmt(s, ret_ty)?);
        }
        Ok(stmts)
    }

    fn lower_if(&mut self, i: &ast::If, ret_ty: &Type) -> Result<hir::If, String> {
        let cond = self.lower_expr(&i.cond)?;
        let then = self.lower_block(&i.then, ret_ty)?;
        let mut elifs = Vec::new();
        for (ec, eb) in &i.elifs {
            let hc = self.lower_expr(ec)?;
            let hb = self.lower_block(eb, ret_ty)?;
            elifs.push((hc, hb));
        }
        let els = i
            .els
            .as_ref()
            .map(|b| self.lower_block(b, ret_ty))
            .transpose()?;
        Ok(hir::If {
            cond,
            then,
            elifs,
            els,
            span: i.span,
        })
    }

    fn lower_match(&mut self, m: &ast::Match, ret_ty: &Type) -> Result<hir::Match, String> {
        let subject = self.lower_expr(&m.subject)?;
        let subj_ty = subject.ty.clone();
        let mut arms = Vec::new();
        for a in &m.arms {
            self.push_scope();
            let pat = self.lower_pat(&a.pat, &subj_ty)?;
            let guard = a.guard.as_ref().map(|g| self.lower_expr(g)).transpose()?;
            let body = self.lower_block_no_scope(&a.body, ret_ty)?;
            self.pop_scope();
            arms.push(hir::Arm {
                pat,
                guard,
                body,
                span: a.span,
            });
        }
        let result = hir::Match {
            subject,
            arms,
            ty: subj_ty.clone(),
            span: m.span,
        };

        self.check_exhaustiveness(&subj_ty, &result.arms, m.span)?;

        Ok(result)
    }

    fn lower_pat(&mut self, pat: &ast::Pat, expected_ty: &Type) -> Result<hir::Pat, String> {
        match pat {
            ast::Pat::Wild(span) => Ok(hir::Pat::Wild(*span)),
            ast::Pat::Ident(name, span) => {
                if let Some((_, tag)) = self.variant_tags.get(name).cloned() {
                    return Ok(hir::Pat::Ctor(name.clone(), tag, vec![], *span));
                }
                let id = self.fresh_id();
                let ty = expected_ty.clone();
                self.define_var(
                    name,
                    VarInfo {
                        def_id: id,
                        ty: ty.clone(),
                        ownership: Self::ownership_for_type(&ty),
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
                    let ft = field_tys.get(i).cloned().unwrap_or(Type::I64);
                    hpats.push(self.lower_pat(sp, &ft)?);
                }
                Ok(hir::Pat::Ctor(name.clone(), tag, hpats, *span))
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
                    _ => vec![Type::I64; pats.len()],
                };
                let mut hpats = Vec::new();
                for (i, p) in pats.iter().enumerate() {
                    let ety = tys.get(i).cloned().unwrap_or(Type::I64);
                    hpats.push(self.lower_pat(p, &ety)?);
                }
                Ok(hir::Pat::Tuple(hpats, *span))
            }
            ast::Pat::Array(pats, span) => {
                let elem_ty = match expected_ty {
                    Type::Array(et, _) => et.as_ref().clone(),
                    _ => Type::I64,
                };
                let mut hpats = Vec::new();
                for p in pats {
                    hpats.push(self.lower_pat(p, &elem_ty)?);
                }
                Ok(hir::Pat::Array(hpats, *span))
            }
        }
    }

    fn lower_expr(&mut self, expr: &ast::Expr) -> Result<hir::Expr, String> {
        match expr {
            ast::Expr::Int(n, span) => Ok(hir::Expr {
                kind: hir::ExprKind::Int(*n),
                ty: Type::I64,
                span: *span,
            }),

            ast::Expr::Float(n, span) => Ok(hir::Expr {
                kind: hir::ExprKind::Float(*n),
                ty: Type::F64,
                span: *span,
            }),

            ast::Expr::Str(s, span) => Ok(hir::Expr {
                kind: hir::ExprKind::Str(s.clone()),
                ty: Type::String,
                span: *span,
            }),

            ast::Expr::Bool(v, span) => Ok(hir::Expr {
                kind: hir::ExprKind::Bool(*v),
                ty: Type::Bool,
                span: *span,
            }),

            ast::Expr::None(span) => Ok(hir::Expr {
                kind: hir::ExprKind::None,
                ty: Type::I64,
                span: *span,
            }),

            ast::Expr::Void(span) => Ok(hir::Expr {
                kind: hir::ExprKind::Void,
                ty: Type::Void,
                span: *span,
            }),

            ast::Expr::Ident(name, span) => {
                if let Some((enum_name, tag)) = self.variant_tags.get(name).cloned() {
                    let is_unit = self
                        .enums
                        .get(&enum_name)
                        .and_then(|vs| vs.iter().find(|(vn, _)| vn == name))
                        .map(|(_, fs)| fs.is_empty())
                        .unwrap_or(false);
                    if is_unit {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(enum_name.clone(), name.clone(), tag),
                            ty: Type::Enum(enum_name),
                            span: *span,
                        });
                    }
                    if let Ok(Some(_mangled)) = self.try_monomorphize_generic_variant_bare(name) {
                        let (en2, tag2) = self
                            .variant_tags
                            .get(name)
                            .cloned()
                            .unwrap_or((enum_name.clone(), tag));
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(en2.clone(), name.clone(), tag2),
                            ty: Type::Enum(en2),
                            span: *span,
                        });
                    }
                } else if let Ok(Some(mangled)) = self.try_monomorphize_generic_variant_bare(name) {
                    if let Some((_, tag)) = self.variant_tags.get(name).cloned() {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(mangled.clone(), name.clone(), tag),
                            ty: Type::Enum(mangled),
                            span: *span,
                        });
                    }
                }
                if let Some(v) = self.find_var(name) {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Var(v.def_id, name.clone()),
                        ty: v.ty.clone(),
                        span: *span,
                    });
                }
                if let Some((id, ptys, ret)) = self.fns.get(name).cloned() {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::FnRef(id, name.clone()),
                        ty: Type::Fn(ptys, Box::new(ret)),
                        span: *span,
                    });
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::Var(DefId::BUILTIN, name.clone()),
                    ty: Type::I64,
                    span: *span,
                })
            }

            ast::Expr::BinOp(lhs, op, rhs, span) => {
                let hl = self.lower_expr(lhs)?;
                let hr = self.lower_expr(rhs)?;
                let (hl, hr) = self.coerce_binop_operands(hl, hr);
                let result_ty = match op {
                    BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::Le
                    | BinOp::Ge
                    | BinOp::And
                    | BinOp::Or => Type::Bool,
                    _ => hl.ty.clone(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::BinOp(Box::new(hl), *op, Box::new(hr)),
                    ty: result_ty,
                    span: *span,
                })
            }

            ast::Expr::UnaryOp(op, inner, span) => {
                let hi = self.lower_expr(inner)?;
                let ty = match op {
                    UnaryOp::Not => Type::Bool,
                    _ => hi.ty.clone(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::UnaryOp(*op, Box::new(hi)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Call(callee, args, span) => self.lower_call(callee, args, *span),

            ast::Expr::Method(obj, method, args, span) => {
                self.lower_method_call(obj, method, args, *span)
            }

            ast::Expr::Field(obj, field, span) => {
                let hobj = self.lower_expr(obj)?;
                let (ty, idx) = if let Type::Struct(ref name) = hobj.ty {
                    if let Some(fields) = self.structs.get(name) {
                        if let Some((i, (_, fty))) =
                            fields.iter().enumerate().find(|(_, (n, _))| n == field)
                        {
                            (fty.clone(), i)
                        } else {
                            (Type::I64, 0)
                        }
                    } else {
                        (Type::I64, 0)
                    }
                } else if matches!(hobj.ty, Type::String) && field == "length" {
                    (Type::I64, 0)
                } else {
                    (Type::I64, 0)
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Field(Box::new(hobj), field.clone(), idx),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Index(arr, idx, span) => {
                let harr = self.lower_expr(arr)?;
                let hidx = self.lower_expr(idx)?;
                let elem_ty = match &harr.ty {
                    Type::Array(et, _) => *et.clone(),
                    Type::Vec(et) => *et.clone(),
                    Type::Map(_, vt) => *vt.clone(),
                    Type::Tuple(tys) => tys.first().cloned().unwrap_or(Type::I64),
                    _ => Type::I64,
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Index(Box::new(harr), Box::new(hidx)),
                    ty: elem_ty,
                    span: *span,
                })
            }

            ast::Expr::Ternary(cond, then, els, span) => {
                let hc = self.lower_expr(cond)?;
                let ht = self.lower_expr(then)?;
                let he = self.lower_expr(els)?;
                let ty = ht.ty.clone();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Ternary(Box::new(hc), Box::new(ht), Box::new(he)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::As(inner, target_ty, span) => {
                let hi = self.lower_expr(inner)?;
                let ty = target_ty.clone();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Cast(Box::new(hi), ty.clone()),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Array(elems, span) => {
                let helems: Vec<hir::Expr> = elems
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                let et = helems.first().map(|e| e.ty.clone()).unwrap_or(Type::I64);
                let len = helems.len();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Array(helems),
                    ty: Type::Array(Box::new(et), len),
                    span: *span,
                })
            }

            ast::Expr::Tuple(elems, span) => {
                let helems: Vec<hir::Expr> = elems
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                let tys: Vec<Type> = helems.iter().map(|e| e.ty.clone()).collect();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Tuple(helems),
                    ty: Type::Tuple(tys),
                    span: *span,
                })
            }

            ast::Expr::Struct(name, inits, span) => {
                self.lower_struct_or_variant(name, inits, *span)
            }

            ast::Expr::IfExpr(i) => {
                let result_ty = match i.then.last() {
                    Some(ast::Stmt::Expr(e)) => self.expr_ty_ast(e),
                    _ => Type::Void,
                };
                let hi = self.lower_if(i, &result_ty)?;
                let ty = match hi.then.last() {
                    Some(hir::Stmt::Expr(e)) => e.ty.clone(),
                    _ => Type::Void,
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::IfExpr(Box::new(hi)),
                    ty,
                    span: i.span,
                })
            }

            ast::Expr::Pipe(left, right, extra_args, span) => {
                self.lower_pipe(left, right, extra_args, *span)
            }

            ast::Expr::Block(stmts, span) => {
                let hstmts = self.lower_block(stmts, &Type::Void)?;
                let ty = match hstmts.last() {
                    Some(hir::Stmt::Expr(e)) => e.ty.clone(),
                    _ => Type::Void,
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Block(hstmts),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Lambda(params, ret, body, span) => {
                self.lower_lambda(params, ret, body, *span)
            }

            ast::Expr::Placeholder(span) => Ok(hir::Expr {
                kind: hir::ExprKind::Void,
                ty: Type::I64,
                span: *span,
            }),

            ast::Expr::Ref(inner, span) => {
                let hi = self.lower_expr(inner)?;
                let ty = Type::Ptr(Box::new(hi.ty.clone()));
                Ok(hir::Expr {
                    kind: hir::ExprKind::Ref(Box::new(hi)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Deref(inner, span) => {
                let hi = self.lower_expr(inner)?;
                let ty = match &hi.ty {
                    Type::Ptr(inner_ty) | Type::Rc(inner_ty) => *inner_ty.clone(),
                    _ => Type::I64,
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Deref(Box::new(hi)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::ListComp(body_expr, var, iter_expr, cond, map_expr, span) => {
                let hiter = self.lower_expr(iter_expr)?;

                let bind_ty = match &hiter.ty {
                    Type::Array(et, _) | Type::Ptr(et) => *et.clone(),
                    _ => Type::I64,
                };
                let bind_id = self.fresh_id();
                self.push_scope();
                self.define_var(
                    var,
                    VarInfo {
                        def_id: bind_id,
                        ty: bind_ty,
                        ownership: Ownership::Owned,
                    },
                );
                let hbody = self.lower_expr(body_expr)?;
                let hcond = cond.as_ref().map(|c| self.lower_expr(c)).transpose()?;
                let hmap = map_expr.as_ref().map(|m| self.lower_expr(m)).transpose()?;
                self.pop_scope();

                let ty = Type::Ptr(Box::new(hbody.ty.clone()));
                Ok(hir::Expr {
                    kind: hir::ExprKind::ListComp(
                        Box::new(hbody),
                        bind_id,
                        var.clone(),
                        Box::new(hiter),
                        hcond.map(Box::new),
                        hmap.map(Box::new),
                    ),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Syscall(args, span) => {
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Syscall(hargs),
                    ty: Type::I64,
                    span: *span,
                })
            }

            ast::Expr::Embed(path, span) => {
                let base = self.source_dir.clone().unwrap_or_else(|| PathBuf::from("."));
                let file_path = base.join(path);
                let contents = std::fs::read_to_string(&file_path).map_err(|e| {
                    format!("embed '{}': {}", file_path.display(), e)
                })?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Str(contents),
                    ty: Type::String,
                    span: *span,
                })
            }
            ast::Expr::Query(_, _, span) => {
                Ok(hir::Expr {
                    kind: hir::ExprKind::Void,
                    ty: Type::Void,
                    span: *span,
                })
            }

            ast::Expr::StoreQuery(store, filter, span) => {
                let schema = self.store_schemas.get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, store)?;
                let struct_name = format!("__store_{store}");
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreQuery(
                        store.clone(),
                        Box::new(hfilter),
                    ),
                    ty: Type::Struct(struct_name),
                    span: *span,
                })
            }

            ast::Expr::StoreCount(store, span) => {
                if !self.store_schemas.contains_key(store) {
                    return Err(format!("unknown store '{store}'"));
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreCount(store.clone()),
                    ty: Type::I64,
                    span: *span,
                })
            }

            ast::Expr::StoreAll(store, span) => {
                if !self.store_schemas.contains_key(store) {
                    return Err(format!("unknown store '{store}'"));
                }
                let struct_name = format!("__store_{store}");
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreAll(store.clone()),
                    ty: Type::Ptr(Box::new(Type::Struct(struct_name))),
                    span: *span,
                })
            }

            ast::Expr::Spawn(name, span) => {
                if !self.actors.contains_key(name) {
                    return Err(format!("spawn: unknown actor '{name}'"));
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::Spawn(name.clone()),
                    ty: Type::ActorRef(name.clone()),
                    span: *span,
                })
            }

            ast::Expr::Send(target, handler, args, span) => {
                let htarget = self.lower_expr(target)?;
                let actor_name = match &htarget.ty {
                    Type::ActorRef(name) => name.clone(),
                    _ => return Err(format!("send: target must be an ActorRef, got {}", htarget.ty)),
                };
                let (_, _, ref handlers) = self
                    .actors
                    .get(&actor_name)
                    .ok_or_else(|| format!("send: unknown actor '{actor_name}'"))?
                    .clone();
                let (_, _, tag) = handlers
                    .iter()
                    .find(|(n, _, _)| n == handler)
                    .ok_or_else(|| format!("send: actor '{actor_name}' has no handler '@{handler}'"))?;
                let tag = *tag;
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Send(
                        Box::new(htarget),
                        actor_name,
                        handler.clone(),
                        tag,
                        hargs,
                    ),
                    ty: Type::Void,
                    span: *span,
                })
            }

            ast::Expr::Receive(_, span) => {
                // Receive is only valid inside actor handlers — for now stub it
                Ok(hir::Expr {
                    kind: hir::ExprKind::Void,
                    ty: Type::Void,
                    span: *span,
                })
            }

            ast::Expr::Yield(inner, span) => {
                let hi = self.lower_expr(inner)?;
                let ty = hi.ty.clone();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Yield(Box::new(hi)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::DispatchBlock(name, body, span) => {
                let hbody = self.lower_block_no_scope(body, &Type::Void)?;
                // Infer yield type from the body
                let yield_ty = self.infer_coroutine_yield_type(&hbody);
                Ok(hir::Expr {
                    kind: hir::ExprKind::CoroutineCreate(name.clone(), hbody),
                    ty: Type::Coroutine(Box::new(yield_ty)),
                    span: *span,
                })
            }

            ast::Expr::ChannelCreate(elem_ty, cap, span) => {
                let hcap = self.lower_expr(cap)?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::ChannelCreate(elem_ty.clone(), Box::new(hcap)),
                    ty: Type::Channel(Box::new(elem_ty.clone())),
                    span: *span,
                })
            }

            ast::Expr::ChannelSend(ch, val, span) => {
                let hch = self.lower_expr(ch)?;
                let elem_ty = match &hch.ty {
                    Type::Channel(t) => (**t).clone(),
                    _ => return Err(format!("send: target must be a Channel, got {}", hch.ty)),
                };
                let hval = self.lower_expr(val)?;
                let _ = elem_ty; // type checking can be added later
                Ok(hir::Expr {
                    kind: hir::ExprKind::ChannelSend(Box::new(hch), Box::new(hval)),
                    ty: Type::Void,
                    span: *span,
                })
            }

            ast::Expr::ChannelRecv(ch, span) => {
                let hch = self.lower_expr(ch)?;
                let elem_ty = match &hch.ty {
                    Type::Channel(t) => (**t).clone(),
                    _ => return Err(format!("receive: target must be a Channel, got {}", hch.ty)),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::ChannelRecv(Box::new(hch)),
                    ty: elem_ty,
                    span: *span,
                })
            }

            ast::Expr::Select(arms, default_body, span) => {
                let mut harms = Vec::new();
                for arm in arms {
                    let hch = self.lower_expr(&arm.chan)?;
                    let elem_ty = match &hch.ty {
                        Type::Channel(t) => (**t).clone(),
                        _ => return Err(format!("select: channel must be a Channel type, got {}", hch.ty)),
                    };
                    let hval = if let Some(ref v) = arm.value {
                        Some(self.lower_expr(v)?)
                    } else {
                        None
                    };
                    let bind_id = arm.binding.as_ref().map(|_| self.fresh_id());
                    if let (Some(name), Some(id)) = (&arm.binding, bind_id) {
                        self.define_var(name, VarInfo {
                            def_id: id,
                            ty: elem_ty.clone(),
                            ownership: hir::Ownership::Owned,
                        });
                    }
                    let hbody = self.lower_block_no_scope(&arm.body, &Type::Void)?;
                    harms.push(hir::SelectArm {
                        is_send: arm.is_send,
                        chan: hch,
                        value: hval,
                        binding: arm.binding.clone(),
                        bind_id,
                        elem_ty,
                        body: hbody,
                        span: arm.span,
                    });
                }
                let hdefault = if let Some(body) = default_body {
                    Some(self.lower_block_no_scope(body, &Type::Void)?)
                } else {
                    None
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Select(harms, hdefault),
                    ty: Type::I64,
                    span: *span,
                })
            }
        }
    }

    fn lower_call(
        &mut self,
        callee: &ast::Expr,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        if let ast::Expr::Ident(name, _) = callee {
            match name.as_str() {
                "assert" => {
                    if args.is_empty() {
                        return Err("assert requires a condition".into());
                    }
                    let hcond = self.lower_expr(&args[0])?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::Assert, vec![hcond]),
                        ty: Type::Void,
                        span,
                    });
                }
                "log" => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::Log, hargs),
                        ty: Type::Void,
                        span,
                    });
                }
                "to_string" => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::ToString, hargs),
                        ty: Type::String,
                        span,
                    });
                }
                "rc" if args.len() == 1 => {
                    let harg = self.lower_expr(&args[0])?;
                    let inner_ty = harg.ty.clone();
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::RcAlloc, vec![harg]),
                        ty: Type::Rc(Box::new(inner_ty)),
                        span,
                    });
                }
                "rc_retain" => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::RcRetain, hargs),
                        ty: Type::Void,
                        span,
                    });
                }
                "rc_release" => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::RcRelease, hargs),
                        ty: Type::Void,
                        span,
                    });
                }
                "weak" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let harg = self.lower_expr(&args[0])?;
                    let inner_ty = match &harg.ty {
                        Type::Rc(inner) => inner.as_ref().clone(),
                        _ => return Err(format!("weak() requires an rc value, got {}", harg.ty)),
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::WeakDowngrade, vec![harg]),
                        ty: Type::Weak(Box::new(inner_ty)),
                        span,
                    });
                }
                "weak_upgrade" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let harg = self.lower_expr(&args[0])?;
                    let inner_ty = match &harg.ty {
                        Type::Weak(inner) => inner.as_ref().clone(),
                        _ => return Err(format!("weak_upgrade() requires a weak value, got {}", harg.ty)),
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::WeakUpgrade, vec![harg]),
                        ty: Type::Rc(Box::new(inner_ty)),
                        span,
                    });
                }
                "volatile_load" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let harg = self.lower_expr(&args[0])?;
                    let inner_ty = match &harg.ty {
                        Type::Ptr(inner) => inner.as_ref().clone(),
                        _ => return Err(format!("volatile_load() requires a pointer, got {}", harg.ty)),
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::VolatileLoad, vec![harg]),
                        ty: inner_ty,
                        span,
                    });
                }
                "volatile_store" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let hptr = self.lower_expr(&args[0])?;
                    let hval = self.lower_expr(&args[1])?;
                    if !matches!(hptr.ty, Type::Ptr(_)) {
                        return Err(format!("volatile_store() first arg must be a pointer, got {}", hptr.ty));
                    }
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::VolatileStore, vec![hptr, hval]),
                        ty: Type::Void,
                        span,
                    });
                }
                "wrapping_add" | "wrapping_sub" | "wrapping_mul" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let lhs = self.lower_expr(&args[0])?;
                    let rhs = self.lower_expr(&args[1])?;
                    let ty = lhs.ty.clone();
                    let builtin = match name.as_str() {
                        "wrapping_add" => hir::BuiltinFn::WrappingAdd,
                        "wrapping_sub" => hir::BuiltinFn::WrappingSub,
                        "wrapping_mul" => hir::BuiltinFn::WrappingMul,
                        _ => unreachable!(),
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(builtin, vec![lhs, rhs]),
                        ty,
                        span,
                    });
                }
                "saturating_add" | "saturating_sub" | "saturating_mul" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let lhs = self.lower_expr(&args[0])?;
                    let rhs = self.lower_expr(&args[1])?;
                    let ty = lhs.ty.clone();
                    let builtin = match name.as_str() {
                        "saturating_add" => hir::BuiltinFn::SaturatingAdd,
                        "saturating_sub" => hir::BuiltinFn::SaturatingSub,
                        "saturating_mul" => hir::BuiltinFn::SaturatingMul,
                        _ => unreachable!(),
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(builtin, vec![lhs, rhs]),
                        ty,
                        span,
                    });
                }
                "checked_add" | "checked_sub" | "checked_mul" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let lhs = self.lower_expr(&args[0])?;
                    let rhs = self.lower_expr(&args[1])?;
                    let ty = lhs.ty.clone();
                    let builtin = match name.as_str() {
                        "checked_add" => hir::BuiltinFn::CheckedAdd,
                        "checked_sub" => hir::BuiltinFn::CheckedSub,
                        "checked_mul" => hir::BuiltinFn::CheckedMul,
                        _ => unreachable!(),
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(builtin, vec![lhs, rhs]),
                        ty: Type::Tuple(vec![ty, Type::Bool]),
                        span,
                    });
                }
                "signal_handle" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let hsig = self.lower_expr(&args[0])?;
                    let hhandler = self.lower_expr(&args[1])?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::SignalHandle, vec![hsig, hhandler]),
                        ty: Type::Void,
                        span,
                    });
                }
                "signal_raise" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hsig = self.lower_expr(&args[0])?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::SignalRaise, vec![hsig]),
                        ty: Type::I32,
                        span,
                    });
                }
                "signal_ignore" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hsig = self.lower_expr(&args[0])?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::SignalIgnore, vec![hsig]),
                        ty: Type::Void,
                        span,
                    });
                }
                "popcount" | "clz" | "ctz" | "rotate_left" | "rotate_right" | "bswap" => {
                    let builtin = match name.as_str() {
                        "popcount" => hir::BuiltinFn::Popcount,
                        "clz" => hir::BuiltinFn::Clz,
                        "ctz" => hir::BuiltinFn::Ctz,
                        "rotate_left" => hir::BuiltinFn::RotateLeft,
                        "rotate_right" => hir::BuiltinFn::RotateRight,
                        "bswap" => hir::BuiltinFn::Bswap,
                        _ => unreachable!(),
                    };
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(builtin, hargs),
                        ty: Type::I64,
                        span,
                    });
                }
                "__string_from_raw" if args.len() == 3 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::StringFromRaw, hargs), ty: Type::String, span });
                }
                "__string_from_ptr" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::StringFromPtr, hargs), ty: Type::String, span });
                }
                "__get_args" if args.is_empty() && !self.fns.contains_key(name) => {
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::GetArgs, vec![]), ty: Type::Vec(Box::new(Type::String)), span });
                }
                "__ln" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::Ln, hargs), ty: Type::F64, span });
                }
                "__log2" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::Log2, hargs), ty: Type::F64, span });
                }
                "__log10" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::Log10, hargs), ty: Type::F64, span });
                }
                "__exp" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::Exp, hargs), ty: Type::F64, span });
                }
                "__exp2" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::Exp2, hargs), ty: Type::F64, span });
                }
                "__powf" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::PowF, hargs), ty: Type::F64, span });
                }
                "__copysign" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::Copysign, hargs), ty: Type::F64, span });
                }
                "__fma" if args.len() == 3 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::Fma, hargs), ty: Type::F64, span });
                }
                "__fmt_float" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::FmtFloat, hargs), ty: Type::String, span });
                }
                "__fmt_hex" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::FmtHex, hargs), ty: Type::String, span });
                }
                "__fmt_oct" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::FmtOct, hargs), ty: Type::String, span });
                }
                "__fmt_bin" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::FmtBin, hargs), ty: Type::String, span });
                }
                "__time_monotonic" if args.is_empty() && !self.fns.contains_key(name) => {
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::TimeMonotonic, vec![]), ty: Type::F64, span });
                }
                "__sleep_ms" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::SleepMs, hargs), ty: Type::Void, span });
                }
                "__file_exists" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args.iter().map(|e| self.lower_expr(e)).collect::<Result<_, _>>()?;
                    return Ok(hir::Expr { kind: hir::ExprKind::Builtin(hir::BuiltinFn::FileExists, hargs), ty: Type::Bool, span });
                }
                "vec" if !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    let elem_ty = hargs.first().map(|a| a.ty.clone()).unwrap_or(Type::I64);
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::VecNew(hargs),
                        ty: Type::Vec(Box::new(elem_ty)),
                        span,
                    });
                }
                "map" if !self.fns.contains_key(name) => {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::MapNew,
                        ty: Type::Map(Box::new(Type::String), Box::new(Type::I64)),
                        span,
                    });
                }
                _ => {}
            }

            if let Some(gf) = self.generic_fns.get(name).cloned() {
                let arg_tys: Vec<Type> = args.iter().map(|e| self.expr_ty_ast(e)).collect();
                let mut type_map = HashMap::new();
                for (i, p) in gf.params.iter().enumerate() {
                    if let Some(Type::Param(tp)) = &p.ty {
                        if i < arg_tys.len() {
                            type_map.insert(tp.clone(), arg_tys[i].clone());
                        }
                    }
                }
                for tp in &gf.type_params {
                    type_map.entry(tp.clone()).or_insert(Type::I64);
                }
                let mangled = self.monomorphize_fn(name, &type_map)?;
                let (id, _, ret) = self.fns.get(&mangled).cloned().unwrap();
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Call(id, mangled, hargs),
                    ty: ret,
                    span,
                });
            }

            if let Some((id, param_tys, ret)) = self.fns.get(name).cloned() {
                let mut hargs: Vec<hir::Expr> = args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                // Coerce arguments to declared parameter types (e.g. Struct → dyn Trait)
                for (i, ha) in hargs.iter_mut().enumerate() {
                    if let Some(pt) = param_tys.get(i) {
                        let taken = std::mem::replace(ha, hir::Expr { kind: hir::ExprKind::Int(0), ty: Type::I64, span });
                        *ha = self.maybe_coerce_to(taken, pt);
                    }
                }
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Call(id, name.clone(), hargs),
                    ty: ret,
                    span,
                });
            }

            if let Some(v) = self.find_var(name) {
                if let Type::Fn(_, ret) = &v.ty {
                    let ret = *ret.clone();
                    let fn_expr = hir::Expr {
                        kind: hir::ExprKind::Var(v.def_id, name.clone()),
                        ty: v.ty.clone(),
                        span,
                    };
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::IndirectCall(Box::new(fn_expr), hargs),
                        ty: ret,
                        span,
                    });
                }
            }

            let _hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            return Err(format!("undefined function: '{name}'"));
        }

        let hcallee = self.lower_expr(callee)?;
        let ret = match &hcallee.ty {
            Type::Fn(_, ret) => *ret.clone(),
            _ => Type::I64,
        };
        let hargs: Vec<hir::Expr> = args
            .iter()
            .map(|e| self.lower_expr(e))
            .collect::<Result<_, _>>()?;
        Ok(hir::Expr {
            kind: hir::ExprKind::IndirectCall(Box::new(hcallee), hargs),
            ty: ret,
            span,
        })
    }

    fn lower_method_call(
        &mut self,
        obj: &ast::Expr,
        method: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        let obj_ty = self.expr_ty_ast(obj);

        if matches!(obj_ty, Type::String) {
            let hobj = self.lower_expr(obj)?;
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let ret_ty = match method {
                "contains" | "starts_with" | "ends_with" => Type::Bool,
                "char_at" | "len" | "find" => Type::I64,
                "slice" | "trim" | "trim_left" | "trim_right" | "replace" | "to_upper" | "to_lower" => Type::String,
                "split" => Type::Vec(Box::new(Type::String)),
                _ => Type::I64,
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::StringMethod(Box::new(hobj), method.to_string(), hargs),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Vec(ref elem_ty) = obj_ty {
            let hobj = self.lower_expr(obj)?;
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let ret_ty = match method {
                "push" | "clear" => Type::Void,
                "pop" | "get" | "remove" => *elem_ty.clone(),
                "len" => Type::I64,
                "set" => Type::Void,
                _ => return Err(format!("no method '{method}' on Vec")),
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::VecMethod(Box::new(hobj), method.to_string(), hargs),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Map(ref key_ty, ref val_ty) = obj_ty {
            let hobj = self.lower_expr(obj)?;
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let ret_ty = match method {
                "set" | "remove" | "clear" => Type::Void,
                "get" => *val_ty.clone(),
                "has" => Type::Bool,
                "len" => Type::I64,
                "keys" => Type::Vec(key_ty.clone()),
                "values" => Type::Vec(val_ty.clone()),
                _ => return Err(format!("no method '{method}' on Map")),
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::MapMethod(Box::new(hobj), method.to_string(), hargs),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Coroutine(ref yield_ty) = obj_ty {
            if method == "next" {
                let hobj = self.lower_expr(obj)?;
                return Ok(hir::Expr {
                    kind: hir::ExprKind::CoroutineNext(Box::new(hobj)),
                    ty: *yield_ty.clone(),
                    span,
                });
            }
            return Err(format!("no method '{method}' on Coroutine"));
        }

        if let Type::DynTrait(ref trait_name) = obj_ty {
            let hobj = self.lower_expr(obj)?;
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            // Look up the method's return type from any impl of this trait
            let ret_ty = self.infer_dyn_method_ret(trait_name, method);
            return Ok(hir::Expr {
                kind: hir::ExprKind::DynDispatch(
                    Box::new(hobj),
                    trait_name.clone(),
                    method.to_string(),
                    hargs,
                ),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Struct(ref type_name) = obj_ty {
            let method_name = format!("{type_name}_{method}");
            if let Some((_, _, ret)) = self.fns.get(&method_name).cloned() {
                let hobj = self.lower_expr(obj)?;
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Method(
                        Box::new(hobj),
                        method_name,
                        method.to_string(),
                        hargs,
                    ),
                    ty: ret,
                    span,
                });
            }
        }

        let hobj = self.lower_expr(obj)?;
        let hargs: Vec<hir::Expr> = args
            .iter()
            .map(|e| self.lower_expr(e))
            .collect::<Result<_, _>>()?;
        Ok(hir::Expr {
            kind: hir::ExprKind::StringMethod(Box::new(hobj), method.to_string(), hargs),
            ty: Type::I64,
            span,
        })
    }

    fn lower_struct_or_variant(
        &mut self,
        name: &str,
        inits: &[ast::FieldInit],
        span: Span,
    ) -> Result<hir::Expr, String> {
        if let Some((enum_name, tag)) = self.variant_tags.get(name).cloned() {
            let hinits: Vec<hir::FieldInit> = inits
                .iter()
                .map(|fi| {
                    Ok(hir::FieldInit {
                        name: fi.name.clone(),
                        value: self.lower_expr(&fi.value)?,
                    })
                })
                .collect::<Result<_, String>>()?;
            return Ok(hir::Expr {
                kind: hir::ExprKind::VariantCtor(enum_name.clone(), name.to_string(), tag, hinits),
                ty: Type::Enum(enum_name),
                span,
            });
        }

        if let Ok(Some(mangled)) = self.try_monomorphize_generic_variant(name, inits) {
            let (_, tag) = self
                .variant_tags
                .get(name)
                .cloned()
                .unwrap_or((mangled.clone(), 0));
            let hinits: Vec<hir::FieldInit> = inits
                .iter()
                .map(|fi| {
                    Ok(hir::FieldInit {
                        name: fi.name.clone(),
                        value: self.lower_expr(&fi.value)?,
                    })
                })
                .collect::<Result<_, String>>()?;
            return Ok(hir::Expr {
                kind: hir::ExprKind::VariantCtor(mangled.clone(), name.to_string(), tag, hinits),
                ty: Type::Enum(mangled),
                span,
            });
        }

        let hinits: Vec<hir::FieldInit> = inits
            .iter()
            .map(|fi| {
                Ok(hir::FieldInit {
                    name: fi.name.clone(),
                    value: self.lower_expr(&fi.value)?,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(hir::Expr {
            kind: hir::ExprKind::Struct(name.to_string(), hinits),
            ty: Type::Struct(name.to_string()),
            span,
        })
    }

    fn lower_pipe(
        &mut self,
        left: &ast::Expr,
        right: &ast::Expr,
        extra_args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        let hleft = self.lower_expr(left)?;
        if let ast::Expr::Ident(name, _) = right {
            if let Some(gf) = self.generic_fns.get(name).cloned() {
                let left_ty = hleft.ty.clone();
                let mut type_map = HashMap::new();
                if let Some(p) = gf.params.first() {
                    if let Some(Type::Param(tp)) = &p.ty {
                        type_map.insert(tp.clone(), left_ty);
                    }
                }
                for tp in &gf.type_params {
                    type_map.entry(tp.clone()).or_insert(Type::I64);
                }
                let mangled = self.monomorphize_fn(name, &type_map)?;
                let (id, _, ret) = self.fns.get(&mangled).cloned().unwrap();
                let mut all_args = vec![hleft];
                for a in extra_args {
                    all_args.push(self.lower_expr(a)?);
                }
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Call(id, mangled, all_args),
                    ty: ret,
                    span,
                });
            }
            if let Some((id, _, ret)) = self.fns.get(name).cloned() {
                let mut all_args = vec![hleft];
                for a in extra_args {
                    all_args.push(self.lower_expr(a)?);
                }
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Pipe(
                        Box::new(all_args.remove(0)),
                        id,
                        name.clone(),
                        all_args,
                    ),
                    ty: ret,
                    span,
                });
            }
            let hright = self.lower_expr(right)?;
            let ret = match &hright.ty {
                Type::Fn(_, r) => *r.clone(),
                _ => Type::I64,
            };
            let mut all_args = vec![hleft];
            for a in extra_args {
                all_args.push(self.lower_expr(a)?);
            }
            return Ok(hir::Expr {
                kind: hir::ExprKind::IndirectCall(Box::new(hright), all_args),
                ty: ret,
                span,
            });
        }

        if let ast::Expr::Call(callee, call_args, _) = right {
            if let ast::Expr::Ident(name, _) = callee.as_ref() {
                let has_placeholder = call_args
                    .iter()
                    .any(|a| matches!(a, ast::Expr::Placeholder(_)));
                let mut all_args = Vec::new();
                if has_placeholder {
                    for a in call_args {
                        if matches!(a, ast::Expr::Placeholder(_)) {
                            all_args.push(hleft.clone());
                        } else {
                            all_args.push(self.lower_expr(a)?);
                        }
                    }
                } else {
                    all_args.push(hleft.clone());
                    for a in call_args {
                        all_args.push(self.lower_expr(a)?);
                    }
                }
                if let Some(gf) = self.generic_fns.get(name).cloned() {
                    let left_ty = all_args[0].ty.clone();
                    let mut type_map = HashMap::new();
                    if let Some(p) = gf.params.first() {
                        if let Some(Type::Param(tp)) = &p.ty {
                            type_map.insert(tp.clone(), left_ty);
                        }
                    }
                    for tp in &gf.type_params {
                        type_map.entry(tp.clone()).or_insert(Type::I64);
                    }
                    let mangled = self.monomorphize_fn(name, &type_map)?;
                    let (id, _, ret) = self.fns.get(&mangled).cloned().unwrap();
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Call(id, mangled, all_args),
                        ty: ret,
                        span,
                    });
                }
                if let Some((id, _, ret)) = self.fns.get(name).cloned() {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Pipe(
                            Box::new(all_args.remove(0)),
                            id,
                            name.clone(),
                            all_args,
                        ),
                        ty: ret,
                        span,
                    });
                }
            }
        }

        let hright = self.lower_expr(right)?;
        let ret = match &hright.ty {
            Type::Fn(_, r) => *r.clone(),
            _ => Type::I64,
        };
        let mut all_args = vec![hleft];
        for a in extra_args {
            all_args.push(self.lower_expr(a)?);
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::IndirectCall(Box::new(hright), all_args),
            ty: ret,
            span,
        })
    }

    fn lower_lambda(
        &mut self,
        params: &[ast::Param],
        ret: &Option<Type>,
        body: &ast::Block,
        span: Span,
    ) -> Result<hir::Expr, String> {
        self.push_scope();
        let mut hparams = Vec::new();
        let mut ptys = Vec::new();
        for p in params {
            let pid = self.fresh_id();
            let ty = p.ty.clone().unwrap_or(Type::I64);
            ptys.push(ty.clone());
            let ownership = Self::ownership_for_type(&ty);
            self.define_var(
                &p.name,
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
                },
            );
            hparams.push(hir::Param {
                def_id: pid,
                name: p.name.clone(),
                ty,
                ownership,
                span: p.span,
            });
        }

        let ret_ty = ret.clone().unwrap_or_else(|| match body.last() {
            Some(ast::Stmt::Expr(e)) => self.expr_ty_ast(e),
            _ => Type::Void,
        });

        let hbody = self.lower_block_no_scope(body, &ret_ty)?;
        self.pop_scope();

        Ok(hir::Expr {
            kind: hir::ExprKind::Lambda(hparams, hbody),
            ty: Type::Fn(ptys, Box::new(ret_ty)),
            span,
        })
    }

    fn maybe_coerce_to(&self, expr: hir::Expr, target: &Type) -> hir::Expr {
        if &expr.ty == target {
            return expr;
        }
        if let Some(coercion) = Self::needs_int_coercion(&expr.ty, target) {
            let span = expr.span;
            return hir::Expr {
                kind: hir::ExprKind::Coerce(Box::new(expr), coercion),
                ty: target.clone(),
                span,
            };
        }
        if expr.ty.is_int() && target.is_float() {
            let span = expr.span;
            return hir::Expr {
                kind: hir::ExprKind::Coerce(
                    Box::new(expr),
                    CoercionKind::IntToFloat { signed: true },
                ),
                ty: target.clone(),
                span,
            };
        }
        if expr.ty.is_float() && target.is_int() {
            let span = expr.span;
            return hir::Expr {
                kind: hir::ExprKind::Coerce(
                    Box::new(expr),
                    CoercionKind::FloatToInt {
                        signed: target.is_signed(),
                    },
                ),
                ty: target.clone(),
                span,
            };
        }
        if expr.ty.is_float() && target.is_float() && expr.ty.bits() != target.bits() {
            let span = expr.span;
            let coercion = if expr.ty.bits() < target.bits() {
                CoercionKind::FloatWiden
            } else {
                CoercionKind::FloatNarrow
            };
            return hir::Expr {
                kind: hir::ExprKind::Coerce(Box::new(expr), coercion),
                ty: target.clone(),
                span,
            };
        }
        if expr.ty == Type::Bool && target.is_int() {
            let span = expr.span;
            return hir::Expr {
                kind: hir::ExprKind::Coerce(Box::new(expr), CoercionKind::BoolToInt),
                ty: target.clone(),
                span,
            };
        }
        // Struct → dyn Trait coercion
        if let Type::DynTrait(trait_name) = &target {
            if let Type::Struct(type_name) = &expr.ty {
                let tn = type_name.clone();
                let trn = trait_name.clone();
                let span = expr.span;
                return hir::Expr {
                    kind: hir::ExprKind::DynCoerce(
                        Box::new(expr),
                        tn,
                        trn,
                    ),
                    ty: target.clone(),
                    span,
                };
            }
        }
        expr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn parse(src: &str) -> ast::Program {
        let tokens = Lexer::new(src).tokenize().unwrap();
        Parser::new(tokens).parse_program().unwrap()
    }

    fn type_check(src: &str) -> hir::Program {
        let prog = parse(src);
        let mut typer = Typer::new();
        typer.lower_program(&prog).unwrap()
    }

    #[test]
    fn test_int_literal_typed() {
        let hir = type_check("*main()\n    log(42)\n");
        assert!(!hir.fns.is_empty());
        let main = &hir.fns[0];
        assert_eq!(main.name, "main");
        assert_eq!(main.ret, Type::I32);
    }

    #[test]
    fn test_variable_binding_typed() {
        let hir = type_check("*main()\n    x is 10\n    log(x)\n");
        let main = &hir.fns[0];
        if let hir::Stmt::Bind(b) = &main.body[0] {
            assert_eq!(b.name, "x");
            assert_eq!(b.ty, Type::I64);
        } else {
            panic!("expected bind");
        }
    }

    #[test]
    fn test_binop_typed() {
        let hir = type_check("*main()\n    log(1 + 2)\n");
        let main = &hir.fns[0];
        if let hir::Stmt::Expr(e) = &main.body[0] {
            if let hir::ExprKind::Builtin(hir::BuiltinFn::Log, args) = &e.kind {
                assert_eq!(args[0].ty, Type::I64);
            } else {
                panic!("expected log builtin");
            }
        }
    }

    #[test]
    fn test_comparison_returns_bool() {
        let hir = type_check("*main()\n    x is 1 equals 2\n    log(x)\n");
        let main = &hir.fns[0];
        if let hir::Stmt::Bind(b) = &main.body[0] {
            assert_eq!(b.ty, Type::Bool);
        }
    }

    #[test]
    fn test_string_typed() {
        let hir = type_check("*main()\n    x is \"hello\"\n    log(x)\n");
        let main = &hir.fns[0];
        if let hir::Stmt::Bind(b) = &main.body[0] {
            assert_eq!(b.ty, Type::String);
        }
    }

    #[test]
    fn test_function_call_typed() {
        let hir =
            type_check("*add(a: i64, b: i64) -> i64\n    a + b\n*main()\n    log(add(1, 2))\n");
        let add_fn = hir.fns.iter().find(|f| f.name == "add").unwrap();
        assert_eq!(add_fn.ret, Type::I64);
    }

    #[test]
    fn test_struct_typed() {
        let hir = type_check(
            "type Point\n    x: i64\n    y: i64\n\n*main() -> i32\n    p is Point(x is 1, y is 2)\n    log(p.x)\n    0\n",
        );
        assert!(!hir.types.is_empty());
        let point = &hir.types[0];
        assert_eq!(point.name, "Point");
        assert_eq!(point.fields.len(), 2);
    }

    #[test]
    fn test_enum_typed() {
        let hir = type_check(
            "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Red\n    match c\n        Red ? log(1)\n        Green ? log(2)\n        Blue ? log(3)\n    0\n",
        );
        assert!(!hir.enums.is_empty());
        let color = &hir.enums[0];
        assert_eq!(color.name, "Color");
        assert_eq!(color.variants.len(), 3);
    }

    #[test]
    fn test_generic_fn_monomorphized() {
        let hir = type_check("*identity(x)\n    x\n*main()\n    log(identity(42))\n");
        assert!(
            hir.fns.len() >= 2,
            "expected at least 2 fns, got {}",
            hir.fns.len()
        );
        let mono = hir.fns.iter().find(|f| f.generic_origin.is_some());
        assert!(mono.is_some(), "expected monomorphized fn");
    }

    #[test]
    fn test_lambda_typed() {
        let hir =
            type_check("*main() -> i32\n    f is *fn(x: i64) -> i64 x + 1\n    log(f(5))\n    0\n");
        let main = &hir.fns[0];
        if let hir::Stmt::Bind(b) = &main.body[0] {
            assert!(matches!(b.ty, Type::Fn(_, _)));
        }
    }

    #[test]
    fn test_ownership_default() {
        let hir = type_check("*main()\n    x is 42\n    log(x)\n");
        let main = &hir.fns[0];
        if let hir::Stmt::Bind(b) = &main.body[0] {
            assert_eq!(b.ownership, Ownership::Owned);
        }
    }

    #[test]
    fn test_rc_ownership() {
        let hir = type_check("*main()\n    x is rc(42)\n    log(@x)\n");
        let main = &hir.fns[0];
        if let hir::Stmt::Bind(b) = &main.body[0] {
            assert_eq!(b.ownership, Ownership::Rc);
            assert!(matches!(b.ty, Type::Rc(_)));
        }
    }
}
