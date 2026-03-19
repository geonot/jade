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

use std::collections::HashMap;

use crate::ast::{self, BinOp, Span, UnaryOp};
use crate::hir::{self, CoercionKind, DefId, Ownership};
use crate::types::Type;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scope
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone)]
struct VarInfo {
    def_id: DefId,
    ty: Type,
    #[allow(dead_code)] // stored for HIR bind ownership propagation
    ownership: Ownership,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Typer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct Typer {
    next_id: u32,
    /// Scoped variable stack: each entry is a scope.
    scopes: Vec<HashMap<String, VarInfo>>,
    /// Function signatures: name → (DefId, param types, return type).
    fns: HashMap<String, (DefId, Vec<Type>, Type)>,
    /// Struct definitions: name → fields.
    structs: HashMap<String, Vec<(String, Type)>>,
    /// Enum definitions: name → variants.
    enums: HashMap<String, Vec<(String, Vec<Type>)>>,
    /// Variant → (enum_name, tag).
    variant_tags: HashMap<String, (String, u32)>,
    /// Generic function ASTs, stored for on-demand monomorphization.
    generic_fns: HashMap<String, ast::Fn>,
    /// Generic enum ASTs.
    generic_enums: HashMap<String, ast::EnumDef>,
    /// Generic type ASTs.
    generic_types: HashMap<String, ast::TypeDef>,
    /// Methods by type name.
    methods: HashMap<String, Vec<ast::Fn>>,
    /// Already-monomorphized generic functions (avoid re-emission).
    mono_fns: Vec<hir::Fn>,
    /// Already-monomorphized enum defs.
    mono_enums: Vec<hir::EnumDef>,
}

impl Typer {
    pub fn new() -> Self {
        Self {
            next_id: 1, // 0 is BUILTIN
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
        }
    }

    fn fresh_id(&mut self) -> DefId {
        let id = DefId(self.next_id);
        self.next_id += 1;
        id
    }

    // ── Scope management ─────────────────────────────────────────

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

    // ── Type helpers ─────────────────────────────────────────────

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

    // ── Generic helpers ──────────────────────────────────────────

    fn substitute_type(ty: &Type, type_map: &HashMap<String, Type>) -> Type {
        match ty {
            Type::Param(n) => type_map.get(n).cloned().unwrap_or_else(|| ty.clone()),
            Type::Array(inner, sz) => {
                Type::Array(Box::new(Self::substitute_type(inner, type_map)), *sz)
            }
            Type::Tuple(tys) => Type::Tuple(
                tys.iter()
                    .map(|t| Self::substitute_type(t, type_map))
                    .collect(),
            ),
            Type::Fn(ptys, ret) => Type::Fn(
                ptys.iter()
                    .map(|t| Self::substitute_type(t, type_map))
                    .collect(),
                Box::new(Self::substitute_type(ret, type_map)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(Self::substitute_type(inner, type_map))),
            Type::Rc(inner) => Type::Rc(Box::new(Self::substitute_type(inner, type_map))),
            _ => ty.clone(),
        }
    }

    fn mangle_generic(
        base: &str,
        type_map: &HashMap<String, Type>,
        type_params: &[String],
    ) -> String {
        let mut name = base.to_string();
        for tp in type_params {
            if let Some(ty) = type_map.get(tp) {
                name = format!("{name}_{ty}");
            }
        }
        name
    }

    fn effective_type_params(f: &ast::Fn) -> Vec<String> {
        if !f.type_params.is_empty() {
            return f.type_params.clone();
        }
        let mut tps = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            if p.ty.is_none() {
                let name = format!("__{i}");
                if !tps.contains(&name) {
                    tps.push(name);
                }
            }
            if let Some(ty) = &p.ty {
                Self::collect_type_params_from(ty, &mut tps);
            }
        }
        if let Some(ret) = &f.ret {
            Self::collect_type_params_from(ret, &mut tps);
        }
        tps
    }

    fn collect_type_params_from(ty: &Type, out: &mut Vec<String>) {
        match ty {
            Type::Param(n) => {
                if !out.contains(n) {
                    out.push(n.clone());
                }
            }
            Type::Array(inner, _) | Type::Ptr(inner) | Type::Rc(inner) => {
                Self::collect_type_params_from(inner, out);
            }
            Type::Tuple(tys) => {
                for t in tys {
                    Self::collect_type_params_from(t, out);
                }
            }
            Type::Fn(ptys, ret) => {
                for t in ptys {
                    Self::collect_type_params_from(t, out);
                }
                Self::collect_type_params_from(ret, out);
            }
            _ => {}
        }
    }

    fn is_generic_fn(f: &ast::Fn) -> bool {
        !Self::effective_type_params(f).is_empty()
    }

    fn normalize_generic_fn(f: &ast::Fn) -> ast::Fn {
        let mut gf = f.clone();
        gf.type_params = Self::effective_type_params(f);
        for (i, p) in gf.params.iter_mut().enumerate() {
            if p.ty.is_none() {
                p.ty = Some(Type::Param(format!("__{i}")));
            }
        }
        gf
    }

    // ── Type inference (synthesis) ───────────────────────────────
    // Mirrors codegen's expr_ty but is used during HIR construction.

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
                    // Float promotion: int op float → float
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
                        // For recursive generics, fall through to I64 rather than
                        // risking infinite recursion via infer_ret_ast → expr_ty_ast.
                        if let Some(ret) = &gf.ret {
                            return Self::substitute_type(ret, &type_map);
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
                    // Try to find the method return type
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
                // Type of if-expression is type of then branch's last expr
                match i.then.last() {
                    Some(ast::Stmt::Expr(e)) => self.expr_ty_ast(e),
                    _ => Type::I64,
                }
            }
            ast::Expr::Block(stmts, _) => match stmts.last() {
                Some(ast::Stmt::Expr(e)) => self.expr_ty_ast(e),
                _ => Type::Void,
            },
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

    fn infer_field_ty(&self, f: &ast::Field) -> Type {
        f.default
            .as_ref()
            .map(|e| self.expr_ty_ast(e))
            .unwrap_or(Type::I64)
    }

    // ── Coercion helpers ─────────────────────────────────────────

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
        // Float promotion: int op float → float op float
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
        // Float width promotion: f32 op f64 → f64 op f64
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
        // Int width promotion
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

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Main entry point: lower entire program
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    pub fn lower_program(&mut self, prog: &ast::Program) -> Result<hir::Program, String> {
        self.register_prelude_types();

        // Pass 1: register all declarations (names, types, signatures)
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
            }
        }

        // Pass 2: lower function and method bodies
        let mut hir_fns = Vec::new();
        let mut hir_types = Vec::new();
        let mut hir_enums = Vec::new();
        let mut hir_externs = Vec::new();
        let mut hir_err_defs = Vec::new();

        for d in &prog.decls {
            match d {
                ast::Decl::Fn(f) if !Self::is_generic_fn(f) => {
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
                _ => {}
            }
        }

        // Append monomorphized generics (produced on-demand during lowering)
        hir_fns.extend(self.mono_fns.drain(..));
        hir_enums.extend(self.mono_enums.drain(..));

        Ok(hir::Program {
            fns: hir_fns,
            types: hir_types,
            enums: hir_enums,
            externs: hir_externs,
            err_defs: hir_err_defs,
        })
    }

    // ── Declaration registration ─────────────────────────────────

    fn register_prelude_types(&mut self) {
        let s = Span::dummy();
        self.generic_enums
            .entry("Option".into())
            .or_insert_with(|| ast::EnumDef {
                name: "Option".into(),
                type_params: vec!["T".into()],
                variants: vec![
                    ast::Variant {
                        name: "Some".into(),
                        fields: vec![ast::VField {
                            name: None,
                            ty: Type::Param("T".into()),
                        }],
                        span: s,
                    },
                    ast::Variant {
                        name: "Nothing".into(),
                        fields: vec![],
                        span: s,
                    },
                ],
                span: s,
            });
        self.generic_enums
            .entry("Result".into())
            .or_insert_with(|| ast::EnumDef {
                name: "Result".into(),
                type_params: vec!["T".into(), "E".into()],
                variants: vec![
                    ast::Variant {
                        name: "Ok".into(),
                        fields: vec![ast::VField {
                            name: None,
                            ty: Type::Param("T".into()),
                        }],
                        span: s,
                    },
                    ast::Variant {
                        name: "Err".into(),
                        fields: vec![ast::VField {
                            name: None,
                            ty: Type::Param("E".into()),
                        }],
                        span: s,
                    },
                ],
                span: s,
            });
    }

    fn declare_fn_sig(&mut self, f: &ast::Fn) {
        let ptys: Vec<Type> = f
            .params
            .iter()
            .map(|p| p.ty.clone().unwrap_or(Type::I64))
            .collect();
        let ret = if f.name == "main" {
            Type::I32
        } else {
            f.ret.clone().unwrap_or_else(|| self.infer_ret_ast(f))
        };
        let id = self.fresh_id();
        self.fns.insert(f.name.clone(), (id, ptys, ret));
    }

    fn declare_method_sig(&mut self, type_name: &str, m: &ast::Fn) {
        let method_name = format!("{type_name}_{}", m.name);
        let self_ty = Type::Struct(type_name.to_string());
        let mut ptys = vec![self_ty];
        for p in &m.params {
            ptys.push(p.ty.clone().unwrap_or(Type::I64));
        }
        let ret = m.ret.clone().unwrap_or_else(|| self.infer_ret_ast(m));
        let id = self.fresh_id();
        self.fns.insert(method_name, (id, ptys, ret));
    }

    fn declare_type_def(&mut self, td: &ast::TypeDef) {
        let fields: Vec<(String, Type)> = td
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)),
                )
            })
            .collect();
        self.structs.insert(td.name.clone(), fields);
    }

    fn declare_enum_def(&mut self, ed: &ast::EnumDef) {
        let mut variants = Vec::new();
        for (tag, v) in ed.variants.iter().enumerate() {
            let ftys: Vec<Type> = v.fields.iter().map(|f| f.ty.clone()).collect();
            self.variant_tags
                .insert(v.name.clone(), (ed.name.clone(), tag as u32));
            variants.push((v.name.clone(), ftys));
        }
        self.enums.insert(ed.name.clone(), variants);
    }

    fn declare_extern_sig(&mut self, ef: &ast::ExternFn) {
        let ptys: Vec<Type> = ef.params.iter().map(|(_, t)| t.clone()).collect();
        let id = self.fresh_id();
        self.fns.insert(ef.name.clone(), (id, ptys, ef.ret.clone()));
    }

    fn declare_err_def_sig(&mut self, ed: &ast::ErrDef) {
        let mut variants = Vec::new();
        for (tag, v) in ed.variants.iter().enumerate() {
            let ftys = v.fields.clone();
            self.variant_tags
                .insert(v.name.clone(), (ed.name.clone(), tag as u32));
            variants.push((v.name.clone(), ftys));
        }
        self.enums.insert(ed.name.clone(), variants);
    }

    // ── Monomorphization ─────────────────────────────────────────

    fn monomorphize_fn(
        &mut self,
        name: &str,
        type_map: &HashMap<String, Type>,
    ) -> Result<String, String> {
        let gf = self
            .generic_fns
            .get(name)
            .ok_or_else(|| format!("no generic fn: {name}"))?
            .clone();
        let mangled = Self::mangle_generic(name, type_map, &gf.type_params);
        if self.fns.contains_key(&mangled) {
            return Ok(mangled);
        }
        let ptys: Vec<Type> = gf
            .params
            .iter()
            .map(|p| {
                let base = p.ty.clone().unwrap_or(Type::I64);
                Self::substitute_type(&base, type_map)
            })
            .collect();
        let ret = gf
            .ret
            .clone()
            .map(|r| Self::substitute_type(&r, type_map))
            .unwrap_or_else(|| {
                let inferred = self.infer_ret_ast(&gf);
                Self::substitute_type(&inferred, type_map)
            });
        let id = self.fresh_id();
        self.fns
            .insert(mangled.clone(), (id, ptys.clone(), ret.clone()));

        // Lower the generic body with substituted types
        let mono_fn = self.lower_generic_fn_body(&gf, &mangled, id, &ptys, &ret, name)?;
        self.mono_fns.push(mono_fn);
        Ok(mangled)
    }

    fn lower_generic_fn_body(
        &mut self,
        gf: &ast::Fn,
        mangled: &str,
        def_id: DefId,
        ptys: &[Type],
        ret: &Type,
        origin: &str,
    ) -> Result<hir::Fn, String> {
        // Save scope state
        let saved_scopes = std::mem::take(&mut self.scopes);
        self.push_scope();

        let mut params = Vec::new();
        for (i, p) in gf.params.iter().enumerate() {
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

        let body = self.lower_block(&gf.body, ret)?;

        self.pop_scope();
        self.scopes = saved_scopes;

        Ok(hir::Fn {
            def_id,
            name: mangled.to_string(),
            params,
            ret: ret.clone(),
            body,
            span: gf.span,
            generic_origin: Some(origin.to_string()),
        })
    }

    fn monomorphize_enum(
        &mut self,
        name: &str,
        type_map: &HashMap<String, Type>,
    ) -> Result<String, String> {
        let ge = self
            .generic_enums
            .get(name)
            .ok_or_else(|| format!("no generic enum: {name}"))?
            .clone();
        let mangled = Self::mangle_generic(name, type_map, &ge.type_params);
        if self.enums.contains_key(&mangled) {
            return Ok(mangled);
        }
        // Register in enums + variant_tags
        let mut variants = Vec::new();
        let mut hir_variants = Vec::new();
        for (tag, v) in ge.variants.iter().enumerate() {
            let ftys: Vec<Type> = v
                .fields
                .iter()
                .map(|f| Self::substitute_type(&f.ty, type_map))
                .collect();
            self.variant_tags
                .insert(v.name.clone(), (mangled.clone(), tag as u32));
            let hv = hir::Variant {
                name: v.name.clone(),
                fields: ftys
                    .iter()
                    .enumerate()
                    .map(|(fi, fty)| hir::VField {
                        name: v.fields.get(fi).and_then(|f| f.name.clone()),
                        ty: fty.clone(),
                    })
                    .collect(),
                tag: tag as u32,
                span: v.span,
            };
            hir_variants.push(hv);
            variants.push((v.name.clone(), ftys));
        }
        self.enums.insert(mangled.clone(), variants);
        let hed = hir::EnumDef {
            def_id: self.fresh_id(),
            name: mangled.clone(),
            variants: hir_variants,
            span: ge.span,
        };
        self.mono_enums.push(hed);
        Ok(mangled)
    }

    fn try_monomorphize_generic_variant(
        &mut self,
        variant_name: &str,
        inits: &[ast::FieldInit],
    ) -> Result<Option<String>, String> {
        let found = self.generic_enums.iter().find_map(|(ename, edef)| {
            edef.variants
                .iter()
                .find(|v| v.name == variant_name)
                .map(|v| (ename.clone(), edef.clone(), v.clone()))
        });
        let (enum_name, edef, variant) = match found {
            Some(f) => f,
            None => return Ok(None),
        };
        let mut type_map = HashMap::new();
        for (i, field) in variant.fields.iter().enumerate() {
            if let Type::Param(ref p) = field.ty {
                if let Some(init) = inits.get(i) {
                    type_map.insert(p.clone(), self.expr_ty_ast(&init.value));
                }
            }
        }
        if type_map.is_empty() && !edef.type_params.is_empty() {
            for tp in &edef.type_params {
                type_map.entry(tp.clone()).or_insert(Type::I64);
            }
        }
        let mangled = self.monomorphize_enum(&enum_name, &type_map)?;
        Ok(Some(mangled))
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Lowering: AST → HIR
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

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

    fn lower_type_def(&mut self, td: &ast::TypeDef) -> Result<hir::TypeDef, String> {
        let id = self.fresh_id();
        let fields: Vec<hir::Field> = td
            .fields
            .iter()
            .map(|f| {
                let ty = f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f));
                let default = f.default.as_ref().map(|e| {
                    // Lower default expression (best-effort, no scope needed)
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

        // `self` parameter (first)
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

    fn lower_enum_def(&self, ed: &ast::EnumDef) -> hir::EnumDef {
        let id = DefId(self.next_id); // peek (don't bump for immutable method)
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

    fn lower_err_def(&self, ed: &ast::ErrDef) -> hir::ErrDef {
        let id = DefId(self.next_id);
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

    // ── Block lowering ───────────────────────────────────────────

    fn lower_block(&mut self, block: &ast::Block, ret_ty: &Type) -> Result<hir::Block, String> {
        self.push_scope();
        let mut stmts = Vec::new();
        for s in block {
            let hs = self.lower_stmt(s, ret_ty)?;
            stmts.push(hs);
        }
        self.pop_scope();
        Ok(stmts)
    }

    // ── Statement lowering ───────────────────────────────────────

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
                // Check if var already exists (reassignment) or new binding
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
                    // Range-based for: bind is an integer
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
        }
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

    // ── If lowering ──────────────────────────────────────────────

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

    // ── Match lowering ───────────────────────────────────────────

    fn lower_match(&mut self, m: &ast::Match, ret_ty: &Type) -> Result<hir::Match, String> {
        let subject = self.lower_expr(&m.subject)?;
        let subj_ty = subject.ty.clone();
        let mut arms = Vec::new();
        for a in &m.arms {
            self.push_scope();
            let pat = self.lower_pat(&a.pat, &subj_ty)?;
            let body = self.lower_block_no_scope(&a.body, ret_ty)?;
            self.pop_scope();
            arms.push(hir::Arm {
                pat,
                body,
                span: a.span,
            });
        }
        Ok(hir::Match {
            subject,
            arms,
            ty: subj_ty,
            span: m.span,
        })
    }

    fn lower_pat(&mut self, pat: &ast::Pat, expected_ty: &Type) -> Result<hir::Pat, String> {
        match pat {
            ast::Pat::Wild(span) => Ok(hir::Pat::Wild(*span)),
            ast::Pat::Ident(name, span) => {
                // Check if this identifier is a known variant (unit constructor)
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

                // Find the variant field types for sub-pattern typing
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
        }
    }

    // ── Expression lowering ──────────────────────────────────────

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
                // 1. Check if it's a variant reference
                if let Some((enum_name, tag)) = self.variant_tags.get(name).cloned() {
                    // Check if it's a unit variant (no fields)
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
                    // Non-unit variant used as an ident — try generic monomorphization
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
                    // Variant from a generic enum (e.g. Nothing from Option<T>)
                    if let Some((_, tag)) = self.variant_tags.get(name).cloned() {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(mangled.clone(), name.clone(), tag),
                            ty: Type::Enum(mangled),
                            span: *span,
                        });
                    }
                }
                // 2. Check variables
                if let Some(v) = self.find_var(name) {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Var(v.def_id, name.clone()),
                        ty: v.ty.clone(),
                        span: *span,
                    });
                }
                // 3. Check functions
                if let Some((id, ptys, ret)) = self.fns.get(name).cloned() {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::FnRef(id, name.clone()),
                        ty: Type::Fn(ptys, Box::new(ret)),
                        span: *span,
                    });
                }
                // 4. Fallback
                Ok(hir::Expr {
                    kind: hir::ExprKind::Var(DefId::BUILTIN, name.clone()),
                    ty: Type::I64,
                    span: *span,
                })
            }

            ast::Expr::BinOp(lhs, op, rhs, span) => {
                let hl = self.lower_expr(lhs)?;
                let hr = self.lower_expr(rhs)?;
                // Insert width/promotion coercions
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
                // Infer result type from then-branch tail expression
                let result_ty = match i.then.last() {
                    Some(ast::Stmt::Expr(e)) => self.expr_ty_ast(e),
                    _ => Type::Void,
                };
                let hi = self.lower_if(i, &result_ty)?;
                // Derive type from lowered then-block
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
        }
    }

    // ── Call lowering ────────────────────────────────────────────

    fn lower_call(
        &mut self,
        callee: &ast::Expr,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        // Named call
        if let ast::Expr::Ident(name, _) = callee {
            // Builtins
            match name.as_str() {
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
                    // weak() takes an rc value and creates a weak reference
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
                    // checked ops return a tuple (result, overflowed)
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
                _ => {}
            }

            // Generic function call
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
                // Fill any remaining type params with I64
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

            // Known function
            if let Some((id, _, ret)) = self.fns.get(name).cloned() {
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Call(id, name.clone(), hargs),
                    ty: ret,
                    span,
                });
            }

            // Variable-as-fn-pointer
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

            // Fallback: assume it returns I64
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            return Ok(hir::Expr {
                kind: hir::ExprKind::Call(DefId::BUILTIN, name.clone(), hargs),
                ty: Type::I64,
                span,
            });
        }

        // Indirect call (expression as callee)
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

    // ── Method call lowering ─────────────────────────────────────

    fn lower_method_call(
        &mut self,
        obj: &ast::Expr,
        method: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        let obj_ty = self.expr_ty_ast(obj);

        // String methods
        if matches!(obj_ty, Type::String) {
            let hobj = self.lower_expr(obj)?;
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let ret_ty = match method {
                "contains" | "starts_with" | "ends_with" => Type::Bool,
                "char_at" | "len" => Type::I64,
                "slice" => Type::String,
                _ => Type::I64,
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::StringMethod(Box::new(hobj), method.to_string(), hargs),
                ty: ret_ty,
                span,
            });
        }

        // Struct methods
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

        // Fallback
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

    // ── Struct/Variant construction ──────────────────────────────

    fn lower_struct_or_variant(
        &mut self,
        name: &str,
        inits: &[ast::FieldInit],
        span: Span,
    ) -> Result<hir::Expr, String> {
        // Check if it's an enum variant
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

        // Try generic enum variant monomorphization
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

        // Regular struct
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

    // ── Pipe lowering ────────────────────────────────────────────

    fn lower_pipe(
        &mut self,
        left: &ast::Expr,
        right: &ast::Expr,
        extra_args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        let hleft = self.lower_expr(left)?;
        // Right side should be a function name
        if let ast::Expr::Ident(name, _) = right {
            // Check if generic
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
            // Fallback: try as indirect call
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

        // Non-ident right side — check for Call with function name
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
                // Check if generic
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

        // Fallback: non-ident right side
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

    // ── Lambda lowering ──────────────────────────────────────────

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

    // ── Coercion insertion ───────────────────────────────────────

    fn maybe_coerce_to(&self, expr: hir::Expr, target: &Type) -> hir::Expr {
        if &expr.ty == target {
            return expr;
        }
        // Int width coercion (i8→i64, etc.)
        if let Some(coercion) = Self::needs_int_coercion(&expr.ty, target) {
            let span = expr.span;
            return hir::Expr {
                kind: hir::ExprKind::Coerce(Box::new(expr), coercion),
                ty: target.clone(),
                span,
            };
        }
        // Int → Float promotion
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
        // Float → Int truncation
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
        // Float width coercion (f32↔f64)
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
        // Bool → Int promotion
        if expr.ty == Type::Bool && target.is_int() {
            let span = expr.span;
            return hir::Expr {
                kind: hir::ExprKind::Coerce(Box::new(expr), CoercionKind::BoolToInt),
                ty: target.clone(),
                span,
            };
        }
        expr
    }

    // ── Helper for bare variant monomorphization ─────────────────

    fn try_monomorphize_generic_variant_bare(
        &mut self,
        variant_name: &str,
    ) -> Result<Option<String>, String> {
        let found = self.generic_enums.iter().find_map(|(ename, edef)| {
            edef.variants
                .iter()
                .find(|v| v.name == variant_name)
                .map(|v| (ename.clone(), edef.clone(), v.clone()))
        });
        let (enum_name, edef, _variant) = match found {
            Some(f) => f,
            None => return Ok(None),
        };
        let mut type_map = HashMap::new();
        for tp in &edef.type_params {
            type_map.entry(tp.clone()).or_insert(Type::I64);
        }
        let mangled = self.monomorphize_enum(&enum_name, &type_map)?;
        Ok(Some(mangled))
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
        // First stmt should be a bind
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
        // Should have main + monomorphized identity_i64
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
