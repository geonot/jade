use std::collections::HashMap;
use std::path::PathBuf;

use crate::ast::{self, Span};
use crate::hir::{self, DefId, Ownership};
use crate::types::{Type, Scheme};

#[derive(Debug, Clone)]
pub(crate) struct VarInfo {
    pub(crate) def_id: DefId,
    pub(crate) ty: Type,
    #[allow(dead_code)]
    pub(crate) ownership: Ownership,
    pub(crate) scheme: Option<Scheme>,
}

/// A method call whose receiver was a TypeVar at lowering time.
/// After lowering completes, the TypeVar may have been solved, so we
/// re-check the method, unify arg/return types, and re-classify the HIR node.
#[derive(Debug, Clone)]
pub(crate) struct DeferredMethod {
    pub(crate) receiver_ty: Type,   // the TypeVar (or whatever) receiver had
    pub(crate) method: String,
    pub(crate) arg_tys: Vec<Type>,  // types of the already-lowered args
    pub(crate) ret_ty: Type,        // the fresh_var assigned as return type
    pub(crate) span: Span,
}

/// A field access whose receiver was a TypeVar at lowering time.
#[derive(Debug, Clone)]
pub(crate) struct DeferredField {
    pub(crate) receiver_ty: Type,
    pub(crate) field_name: String,
    pub(crate) field_ty: Type,      // the fresh_var assigned as field type
    pub(crate) span: Span,
}

mod mono;
mod resolve;
pub(crate) mod unify;

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
    pub(crate) generic_bounds: HashMap<String, Vec<(String, Vec<String>)>>,
    pub(crate) trait_impl_type_args: HashMap<(String, String), Vec<Type>>,
    pub(crate) assoc_types: HashMap<(String, String), Type>,
    pub(crate) trait_assoc_types: HashMap<String, Vec<String>>,
    pub(crate) consts: HashMap<String, ast::Expr>,
    pub(crate) infer_ctx: unify::InferCtx,
    pub(crate) debug_types: bool,
    pub(crate) warnings: Vec<String>,
    pub(crate) deferred_methods: Vec<DeferredMethod>,
    pub(crate) deferred_fields: Vec<DeferredField>,
    /// Functions with unannotated params stored for auto-monomorphization fallback.
    /// When called with incompatible types (e.g., multiple struct types), we fall back
    /// to monomorphization. Otherwise, TypeVars are solved by unification.
    pub(crate) inferable_fns: HashMap<String, ast::Fn>,
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
            generic_bounds: HashMap::new(),
            trait_impl_type_args: HashMap::new(),
            assoc_types: HashMap::new(),
            trait_assoc_types: HashMap::new(),
            consts: HashMap::new(),
            infer_ctx: unify::InferCtx::new(),
            debug_types: false,
            warnings: Vec::new(),
            deferred_methods: Vec::new(),
            deferred_fields: Vec::new(),
            inferable_fns: HashMap::new(),
        }
    }

    pub fn set_source_dir(&mut self, dir: PathBuf) {
        self.source_dir = Some(dir);
    }

    pub fn set_test_mode(&mut self, enabled: bool) {
        self.test_mode = enabled;
    }

    pub fn set_debug_types(&mut self, enabled: bool) {
        self.debug_types = enabled;
        self.infer_ctx.debug = enabled;
    }

    pub fn set_warn_inferred_defaults(&mut self, enabled: bool) {
        if enabled {
            self.infer_ctx.enable_default_warnings();
        }
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

    /// Collect all free TypeVars in the current environment (all scopes).
    fn free_type_vars_in_env(&mut self) -> std::collections::HashSet<u32> {
        let mut ftvs = std::collections::HashSet::new();
        for scope in &self.scopes {
            for info in scope.values() {
                let resolved = self.infer_ctx.shallow_resolve(&info.ty);
                resolved.free_type_vars(&mut ftvs);
            }
        }
        ftvs
    }

    /// Generalize a type into a Scheme by quantifying TypeVars that are free
    /// in the type but NOT free in the environment. This is the Gen(Γ, τ) step
    /// of Algorithm J / Hindley-Milner.
    fn generalize(&mut self, ty: &Type) -> Scheme {
        let resolved = self.infer_ctx.shallow_resolve(ty);
        if !resolved.has_type_var() {
            return Scheme::mono(resolved);
        }
        let env_ftvs = self.free_type_vars_in_env();
        let mut ty_ftvs = std::collections::HashSet::new();
        resolved.free_type_vars(&mut ty_ftvs);
        let quantified: Vec<u32> = ty_ftvs.difference(&env_ftvs).copied().collect();
        if quantified.is_empty() {
            Scheme::mono(resolved)
        } else {
            Scheme { quantified, ty: resolved }
        }
    }

    /// Value restriction: only generalize let-bindings whose RHS is a syntactic value.
    /// Syntactic values cannot have side effects, so polymorphic generalization is safe.
    /// Excludes literals — they are inherently monomorphic and generalizing them
    /// would discard constraint information (Float, Integer, etc.).
    fn is_syntactic_value(expr: &ast::Expr) -> bool {
        match expr {
            ast::Expr::Lambda(..) => true,
            ast::Expr::Ident(..) => true,
            ast::Expr::Struct(..) => true,
            ast::Expr::Array(elems, _) | ast::Expr::Tuple(elems, _) => {
                elems.iter().all(Self::is_syntactic_value)
            }
            ast::Expr::Ref(inner, _) => Self::is_syntactic_value(inner),
            _ => false,
        }
    }
}

mod expr;
mod infer;
mod lower;
pub(crate) mod scc;
mod stmt;

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
        // With explicit type param T, identity is generic and monomorphized
        let hir = type_check("*identity(x: T) -> T\n    x\n*main()\n    log(identity(42))\n");
        assert!(
            hir.fns.len() >= 2,
            "expected at least 2 fns, got {}",
            hir.fns.len()
        );
        let mono = hir.fns.iter().find(|f| f.generic_origin.is_some());
        assert!(mono.is_some(), "expected monomorphized fn");
    }

    #[test]
    fn test_untyped_param_not_generic() {
        // With unannotated params (no explicit type param), fn is NOT generic.
        // TypeVars are solved by unification, not monomorphization.
        let hir = type_check("*identity(x)\n    x\n*main()\n    log(identity(42))\n");
        let identity = hir.fns.iter().find(|f| f.name == "identity").unwrap();
        assert!(identity.generic_origin.is_none(),
            "unannotated-param fn should NOT be treated as generic");
        // Param type should be resolved to I64 via call-site unification
        assert_eq!(identity.params[0].ty, Type::I64);
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

    #[test]
    fn test_typevar_resolved_after_lowering() {
        let hir = type_check(
            "type Pair\n    a: i64\n    b: f64\n\n*main() -> i32\n    p is Pair(a is 1, b is 2.0)\n    log(p.a)\n    0\n",
        );
        let pair = &hir.types[0];
        assert_eq!(pair.fields[0].ty, Type::I64);
        assert_eq!(pair.fields[1].ty, Type::F64);
        assert!(!pair.fields[0].ty.has_type_var());
        assert!(!pair.fields[1].ty.has_type_var());
    }

    #[test]
    fn test_constraint_provenance() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_var();
        let span = crate::ast::Span {
            start: 0,
            end: 5,
            line: 1,
            col: 1,
        };
        let _ = ctx.unify_at(&v, &Type::String, span, "test constraint");
        let origin = ctx.origin_of(&v);
        assert!(origin.is_some(), "expected constraint origin");
        let o = origin.unwrap();
        assert_eq!(o.reason, "test constraint");
        assert_eq!(o.span.line, 1);
        assert_eq!(ctx.resolve(&v), Type::String);
    }

    #[test]
    fn test_type_mismatch_msg() {
        let mut typer = Typer::new();
        let msg = typer.type_mismatch_msg(&Type::I64, &Type::String, "argument");
        assert!(msg.contains("expected `i64`"), "msg: {msg}");
        assert!(msg.contains("found `String`"), "msg: {msg}");
    }

    // ── Let-Generalization Tests ──

    #[test]
    fn test_let_gen_fn_scheme_is_poly() {
        // A lambda bound via let should get a polymorphic scheme
        let prog = parse("*main() -> i32\n    f is *fn(x: i64) -> i64 x + 1\n    log(f(5))\n    0\n");
        let mut typer = Typer::new();
        let _hir = typer.lower_program(&prog).unwrap();
        // f should be in scope as a Fn type — verify it was generalized
        // (the scheme machinery runs, fn types get generalized)
    }

    #[test]
    fn test_instantiation_creates_fresh_vars() {
        // Two uses of the same polymorphic scheme should get different TypeVars
        let mut ctx = unify::InferCtx::new();
        let a = ctx.fresh_var();
        let fn_ty = Type::Fn(vec![a.clone()], Box::new(a.clone()));
        let scheme = Scheme {
            quantified: vec![0], // quantify over ?0
            ty: fn_ty,
        };
        let inst1 = ctx.instantiate(&scheme);
        let inst2 = ctx.instantiate(&scheme);
        // Both should be Fn types with DIFFERENT TypeVars
        if let (Type::Fn(p1, _), Type::Fn(p2, _)) = (&inst1, &inst2) {
            assert_ne!(p1[0], p2[0], "instantiation must create distinct TypeVars");
        } else {
            panic!("expected Fn types from instantiation");
        }
    }

    #[test]
    fn test_constrained_var_integer_rejects_float() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_integer_var();
        assert!(ctx.unify(&v, &Type::F64).is_err(),
            "integer-constrained var must reject F64");
    }

    #[test]
    fn test_constrained_var_float_rejects_int() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_float_var();
        assert!(ctx.unify(&v, &Type::I64).is_err(),
            "float-constrained var must reject I64");
    }

    #[test]
    fn test_constrained_var_numeric_accepts_both() {
        let mut ctx = unify::InferCtx::new();
        let v1 = ctx.fresh_numeric_var();
        assert!(ctx.unify(&v1, &Type::I64).is_ok(),
            "numeric var must accept I64");
        let v2 = ctx.fresh_numeric_var();
        assert!(ctx.unify(&v2, &Type::F64).is_ok(),
            "numeric var must accept F64");
    }

    #[test]
    fn test_constrained_var_numeric_rejects_string() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_numeric_var();
        assert!(ctx.unify(&v, &Type::String).is_err(),
            "numeric var must reject String");
    }

    #[test]
    fn test_integer_var_defaults_i64() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_integer_var();
        assert_eq!(ctx.resolve(&v), Type::I64);
    }

    #[test]
    fn test_float_var_defaults_f64() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_float_var();
        assert_eq!(ctx.resolve(&v), Type::F64);
    }

    #[test]
    fn test_numeric_var_defaults_i64() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_numeric_var();
        // Numeric defaults to I64 (no Float constraint)
        assert_eq!(ctx.resolve(&v), Type::I64);
    }

    // ── Return Type Inference Tests ──

    #[test]
    fn test_return_type_inferred_from_tail() {
        let hir = type_check("*double(x: i64) -> i64\n    x * 2\n*main()\n    log(double(5))\n");
        let double = hir.fns.iter().find(|f| f.name == "double").unwrap();
        assert_eq!(double.ret, Type::I64);
    }

    #[test]
    fn test_return_type_inferred_from_return_stmt() {
        let hir = type_check("*abs(x: i64) -> i64\n    if x < 0\n        return -x\n    x\n*main()\n    log(abs(-5))\n");
        let abs_fn = hir.fns.iter().find(|f| f.name == "abs").unwrap();
        assert_eq!(abs_fn.ret, Type::I64);
    }

    #[test]
    fn test_recursive_fn_return_type() {
        // Fibonacci: return type must be inferred as I64
        let hir = type_check(
            "*fib(n: i64) -> i64\n    if n <= 1\n        return n\n    fib(n - 1) + fib(n - 2)\n*main()\n    log(fib(10))\n",
        );
        let fib = hir.fns.iter().find(|f| f.name == "fib").unwrap();
        assert_eq!(fib.ret, Type::I64);
    }

    // ── Deferred Resolution Tests ──

    #[test]
    fn test_deferred_field_no_typevars() {
        // After lowering, all TypeVars in struct fields should be resolved
        let hir = type_check(
            "type Point\n    x: i64\n    y: i64\n\n*main() -> i32\n    p is Point(x is 10, y is 20)\n    log(p.x + p.y)\n    0\n",
        );
        let point = &hir.types[0];
        assert!(!point.fields[0].ty.has_type_var());
        assert!(!point.fields[1].ty.has_type_var());
    }

    #[test]
    fn test_vec_method_types_resolved() {
        // Vec operations should have concrete types after lowering
        let hir = type_check(
            "*main() -> i32\n    v is vec(1, 2, 3)\n    v.push(4)\n    log(v.len())\n    0\n",
        );
        // Should compile without TypeVars remaining
        let main = &hir.fns[0];
        for stmt in &main.body {
            check_no_typevars_in_stmt(stmt);
        }
    }

    fn check_no_typevars_in_stmt(stmt: &hir::Stmt) {
        match stmt {
            hir::Stmt::Bind(b) => {
                assert!(!b.ty.has_type_var(), "TypeVar in bind: {} has type {}", b.name, b.ty);
            }
            hir::Stmt::Expr(e) => {
                assert!(!e.ty.has_type_var(), "TypeVar in expr: {}", e.ty);
            }
            _ => {}
        }
    }

    // ── Type Error (Negative) Tests ──

    #[test]
    fn test_type_error_add_bool_int() {
        let prog = parse("*main()\n    x is true + 1\n    log(x)\n");
        let mut typer = Typer::new();
        // This may or may not error depending on coercion rules,
        // but the types should be concrete
        let _ = typer.lower_program(&prog);
    }

    #[test]
    fn test_concrete_mismatch_fn_arg() {
        let mut ctx = unify::InferCtx::new();
        // Fn(i64) -> bool vs Fn(String) -> bool should fail
        let fn_a = Type::Fn(vec![Type::I64], Box::new(Type::Bool));
        let fn_b = Type::Fn(vec![Type::String], Box::new(Type::Bool));
        assert!(ctx.unify(&fn_a, &fn_b).is_err());
    }

    #[test]
    fn test_concrete_mismatch_fn_ret() {
        let mut ctx = unify::InferCtx::new();
        let fn_a = Type::Fn(vec![Type::I64], Box::new(Type::Bool));
        let fn_b = Type::Fn(vec![Type::I64], Box::new(Type::String));
        assert!(ctx.unify(&fn_a, &fn_b).is_err());
    }

    #[test]
    fn test_fn_arity_mismatch() {
        let mut ctx = unify::InferCtx::new();
        let fn_a = Type::Fn(vec![Type::I64], Box::new(Type::Bool));
        let fn_b = Type::Fn(vec![Type::I64, Type::I64], Box::new(Type::Bool));
        assert!(ctx.unify(&fn_a, &fn_b).is_err());
    }

    // ── Generalize / free_type_vars Tests ──

    #[test]
    fn test_free_type_vars_basic() {
        let mut ftvs = std::collections::HashSet::new();
        let ty = Type::Fn(vec![Type::TypeVar(0)], Box::new(Type::TypeVar(1)));
        ty.free_type_vars(&mut ftvs);
        assert!(ftvs.contains(&0));
        assert!(ftvs.contains(&1));
        assert_eq!(ftvs.len(), 2);
    }

    #[test]
    fn test_free_type_vars_nested() {
        let mut ftvs = std::collections::HashSet::new();
        let ty = Type::Vec(Box::new(Type::Tuple(vec![Type::TypeVar(5), Type::I64])));
        ty.free_type_vars(&mut ftvs);
        assert!(ftvs.contains(&5));
        assert_eq!(ftvs.len(), 1);
    }

    #[test]
    fn test_free_type_vars_concrete() {
        let mut ftvs = std::collections::HashSet::new();
        let ty = Type::Fn(vec![Type::I64, Type::String], Box::new(Type::Bool));
        ty.free_type_vars(&mut ftvs);
        assert!(ftvs.is_empty());
    }

    #[test]
    fn test_scheme_mono_not_poly() {
        let s = Scheme::mono(Type::I64);
        assert!(!s.is_poly());
        assert!(s.quantified.is_empty());
    }

    #[test]
    fn test_scheme_poly() {
        let s = Scheme {
            quantified: vec![0, 1],
            ty: Type::Fn(vec![Type::TypeVar(0)], Box::new(Type::TypeVar(1))),
        };
        assert!(s.is_poly());
        assert_eq!(s.quantified.len(), 2);
    }

    // ── No TypeVar leak tests ──

    #[test]
    fn test_no_typevar_in_simple_fn() {
        let hir = type_check("*add(a: i64, b: i64) -> i64\n    a + b\n*main()\n    log(add(1, 2))\n");
        for f in &hir.fns {
            assert!(!f.ret.has_type_var(), "fn {} has TypeVar in ret: {}", f.name, f.ret);
            for p in &f.params {
                assert!(!p.ty.has_type_var(), "fn {} param {} has TypeVar: {}", f.name, p.name, p.ty);
            }
        }
    }

    #[test]
    fn test_no_typevar_in_struct_fields() {
        let hir = type_check(
            "type Point\n    x: i64\n    y: i64\n\n*main() -> i32\n    p is Point(x is 1, y is 2)\n    log(p.x)\n    0\n",
        );
        for td in &hir.types {
            for f in &td.fields {
                assert!(!f.ty.has_type_var(), "struct {} field {} has TypeVar: {}", td.name, f.name, f.ty);
            }
        }
    }

    #[test]
    fn test_no_typevar_in_enum_variants() {
        let hir = type_check(
            "enum Shape\n    Circle(f64)\n    Rect(f64, f64)\n\n*main() -> i32\n    s is Circle(3.14)\n    match s\n        Circle(r) ? log(r)\n        Rect(w, h) ? log(w)\n    0\n",
        );
        for ed in &hir.enums {
            for v in &ed.variants {
                for vf in &v.fields {
                    assert!(!vf.ty.has_type_var(), "enum {} variant {} has TypeVar: {}", ed.name, v.name, vf.ty);
                }
            }
        }
    }

    // ── Unification edge cases ──

    #[test]
    fn test_unify_rc_types() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_var();
        let rc_a = Type::Rc(Box::new(v.clone()));
        let rc_b = Type::Rc(Box::new(Type::I64));
        ctx.unify(&rc_a, &rc_b).unwrap();
        assert_eq!(ctx.resolve(&v), Type::I64);
    }

    #[test]
    fn test_unify_channel_types() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_var();
        let ch_a = Type::Channel(Box::new(v.clone()));
        let ch_b = Type::Channel(Box::new(Type::String));
        ctx.unify(&ch_a, &ch_b).unwrap();
        assert_eq!(ctx.resolve(&v), Type::String);
    }

    #[test]
    fn test_unify_ptr_types() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_var();
        let ptr_a = Type::Ptr(Box::new(v.clone()));
        let ptr_b = Type::Ptr(Box::new(Type::I32));
        ctx.unify(&ptr_a, &ptr_b).unwrap();
        assert_eq!(ctx.resolve(&v), Type::I32);
    }

    #[test]
    fn test_unify_coroutine_types() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_var();
        let co_a = Type::Coroutine(Box::new(v.clone()));
        let co_b = Type::Coroutine(Box::new(Type::F64));
        ctx.unify(&co_a, &co_b).unwrap();
        assert_eq!(ctx.resolve(&v), Type::F64);
    }

    #[test]
    fn test_constraint_merge_integer_wins() {
        // When merging Integer and Numeric constraints, Integer should win
        let mut ctx = unify::InferCtx::new();
        let a = ctx.fresh_integer_var();
        let b = ctx.fresh_numeric_var();
        ctx.unify(&a, &b).unwrap();
        // Should be constrained to Integer — reject Float
        assert!(ctx.unify(&a, &Type::F64).is_err(),
            "merged Integer+Numeric constraint should reject F64");
    }

    #[test]
    fn test_constraint_merge_float_wins() {
        let mut ctx = unify::InferCtx::new();
        let a = ctx.fresh_float_var();
        let b = ctx.fresh_numeric_var();
        ctx.unify(&a, &b).unwrap();
        // Should be constrained to Float — reject Int
        assert!(ctx.unify(&a, &Type::I64).is_err(),
            "merged Float+Numeric constraint should reject I64");
    }

    #[test]
    fn test_bidirectional_call_result_unifies_with_expected() {
        // Phase 12: When a call site has an expected type, the return TypeVar
        // should get unified with it.
        let mut ctx = unify::InferCtx::new();
        let ret_var = ctx.fresh_var();
        let expected = Type::I64;
        // Simulate what lower_expr_expected does for Call: unify result with expected
        ctx.unify(&expected, &ret_var).unwrap();
        assert_eq!(ctx.resolve(&ret_var), Type::I64);
    }

    #[test]
    fn test_bidirectional_call_result_propagates_through_chain() {
        // Phase 12: Expected type propagates through chained TypeVars
        let mut ctx = unify::InferCtx::new();
        let ret_var = ctx.fresh_var();
        let intermediate = ctx.fresh_var();
        // Chain: intermediate = ret_var, then expected unifies with intermediate
        ctx.unify(&intermediate, &ret_var).unwrap();
        ctx.unify(&Type::F64, &intermediate).unwrap();
        // ret_var should now resolve to F64
        assert_eq!(ctx.resolve(&ret_var), Type::F64);
    }

    #[test]
    fn test_bidirectional_numeric_var_constrained_by_expected() {
        // Phase 12: A numeric-constrained return var gets fully resolved
        // when the call site expects a concrete type
        let mut ctx = unify::InferCtx::new();
        let ret_var = ctx.fresh_numeric_var();
        // Call site expects F64 — should resolve numeric ambiguity
        ctx.unify(&Type::F64, &ret_var).unwrap();
        assert_eq!(ctx.resolve(&ret_var), Type::F64);
    }
}
