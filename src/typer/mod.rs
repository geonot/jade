use std::collections::HashMap;
use std::path::PathBuf;

use crate::ast::{self, Span};
use crate::hir::{self, DefId, Ownership};
use crate::types::{Scheme, Type};

#[derive(Debug, Clone)]
pub(crate) struct VarInfo {
    pub(crate) def_id: DefId,
    pub(crate) ty: Type,
    #[allow(dead_code)]
    pub(crate) ownership: Ownership,
    pub(crate) scheme: Option<Scheme>,
}

#[derive(Debug, Clone)]
pub(crate) struct DeferredMethod {
    pub(crate) receiver_ty: Type,
    pub(crate) method: String,
    pub(crate) arg_tys: Vec<Type>,
    pub(crate) ret_ty: Type,
    pub(crate) span: Span,
}

#[derive(Debug, Clone)]
pub(crate) struct DeferredField {
    pub(crate) receiver_ty: Type,
    pub(crate) field_name: String,
    pub(crate) field_ty: Type,
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
    pub(crate) mono_types: Vec<hir::TypeDef>,
    pub(crate) inferred_field_structs: std::collections::HashSet<String>,
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
    pub(crate) deferred_quantified_vars: Vec<u32>,
    pub(crate) field_constraints: HashMap<u32, Vec<(String, Type)>>,
    pub(crate) inferable_fns: HashMap<String, ast::Fn>,
    pub(crate) fn_schemes: HashMap<String, (Vec<u32>, Vec<Type>, Type)>,
    pub(crate) unannotated_struct_fields: Vec<(String, String, Type, Span)>,
    pub(crate) poly_lambda_asts: HashMap<String, (Vec<ast::Param>, Option<Type>, ast::Block, Span)>,
    pub(crate) type_errors: Vec<String>,
    pub(crate) fn_param_names: HashMap<String, Vec<String>>,
    pub(crate) fn_defaults: HashMap<String, Vec<Option<ast::Expr>>>,
    pub(crate) current_method_type: Option<String>,
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
            mono_types: Vec::new(),
            inferred_field_structs: std::collections::HashSet::new(),
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
            deferred_quantified_vars: Vec::new(),
            field_constraints: HashMap::new(),
            inferable_fns: HashMap::new(),
            fn_schemes: HashMap::new(),
            unannotated_struct_fields: Vec::new(),
            poly_lambda_asts: HashMap::new(),
            type_errors: Vec::new(),
            fn_param_names: HashMap::new(),
            fn_defaults: HashMap::new(),
            current_method_type: None,
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

    pub fn set_strict_types(&mut self, enabled: bool) {
        if enabled {
            self.infer_ctx.enable_strict_types();
        }
    }

    pub fn set_lenient(&mut self, enabled: bool) {
        if enabled {
            self.infer_ctx.disable_strict_types();
        }
    }

    pub fn set_pedantic(&mut self, enabled: bool) {
        if enabled {
            self.infer_ctx.set_pedantic(true);
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
            Type::Struct(n, _) if self.enums.contains_key(n) => Type::Enum(n.clone()),
            _ => ty,
        }
    }

    fn collect_unify_error(&mut self, result: Result<(), String>) {
        if let Err(e) = result {
            self.type_errors.push(e);
        }
    }

    pub(crate) fn make_coerce(
        expr: hir::Expr,
        coercion: hir::CoercionKind,
        target_ty: Type,
    ) -> hir::Expr {
        let span = expr.span;
        hir::Expr {
            kind: hir::ExprKind::Coerce(Box::new(expr), coercion),
            ty: target_ty,
            span,
        }
    }

    fn ownership_for_type(ty: &Type) -> Ownership {
        match ty {
            Type::Rc(_) => Ownership::Rc,
            Type::Ptr(_) => Ownership::Raw,
            _ => Ownership::Owned,
        }
    }

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

    fn generalize(&mut self, ty: &Type) -> Scheme {
        let resolved = self.infer_ctx.canonicalize_type(ty);
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
            Scheme {
                quantified,
                ty: resolved,
            }
        }
    }

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

    pub(crate) fn string_method_ret_ty(method: &str) -> Option<Type> {
        match method {
            "contains" | "starts_with" | "ends_with" => Some(Type::Bool),
            "matches" => Some(Type::Bool),
            "char_at" | "len" | "find" => Some(Type::I64),
            "slice" | "trim" | "trim_left" | "trim_right" | "replace" | "to_upper" | "to_lower" | "repeat" | "replace_re" => {
                Some(Type::String)
            }
            "split" | "lines" => Some(Type::Vec(Box::new(Type::String))),
            "find_all" => Some(Type::Vec(Box::new(Type::String))),
            "is_empty" => Some(Type::Bool),
            _ => None,
        }
    }

    /// Returns true if the method name is exclusive to String (not shared with Vec/Map/Struct).
    /// Used to immediately constrain TypeVar receivers to String.
    pub(crate) fn is_string_exclusive_method(method: &str) -> bool {
        matches!(
            method,
            "contains"
                | "starts_with"
                | "ends_with"
                | "char_at"
                | "find"
                | "slice"
                | "trim"
                | "trim_left"
                | "trim_right"
                | "replace"
                | "to_upper"
                | "to_lower"
                | "split"
                | "lines"
                | "repeat"
                | "is_empty"
                | "matches"
                | "find_all"
                | "replace_re"
        )
    }

    pub(crate) fn vec_method_ret_ty(method: &str, elem_ty: &Type) -> Option<Type> {
        match method {
            "push" | "clear" | "set" => Some(Type::Void),
            "pop" | "get" | "remove" => Some(elem_ty.clone()),
            "len" | "count" => Some(Type::I64),
            "take" | "skip" | "flatten" | "collect" | "reverse" | "sort" => {
                Some(Type::Vec(Box::new(elem_ty.clone())))
            }
            "sum" => Some(elem_ty.clone()),
            "contains" => Some(Type::Bool),
            "join" => Some(Type::String),
            "enumerate" => {
                Some(Type::Vec(Box::new(Type::Tuple(vec![Type::I64, elem_ty.clone()]))))
            }
            _ => None,
        }
    }

    pub(crate) fn map_method_ret_ty(method: &str, key_ty: &Type, val_ty: &Type) -> Option<Type> {
        match method {
            "set" | "remove" | "clear" => Some(Type::Void),
            "get" => Some(val_ty.clone()),
            "has" | "contains" => Some(Type::Bool),
            "len" => Some(Type::I64),
            "keys" => Some(Type::Vec(Box::new(key_ty.clone()))),
            "values" => Some(Type::Vec(Box::new(val_ty.clone()))),
            _ => None,
        }
    }

    pub(crate) fn set_method_ret_ty(method: &str, elem_ty: &Type) -> Option<Type> {
        match method {
            "add" | "remove" | "clear" => Some(Type::Void),
            "contains" => Some(Type::Bool),
            "len" => Some(Type::I64),
            "union" | "difference" | "intersection" => Some(Type::Set(Box::new(elem_ty.clone()))),
            "to_vec" => Some(Type::Vec(Box::new(elem_ty.clone()))),
            _ => None,
        }
    }
}

mod builtins;
mod call;
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
    fn test_untyped_param_is_implicit_generic() {
        let hir = type_check("*identity(x)\n    x\n*main()\n    log(identity(42))\n");
        let identity = hir
            .fns
            .iter()
            .find(|f| f.name.starts_with("identity"))
            .unwrap();
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
    fn test_let_gen_fn_scheme_is_poly() {
        let prog =
            parse("*main() -> i32\n    f is *fn(x: i64) -> i64 x + 1\n    log(f(5))\n    0\n");
        let mut typer = Typer::new();
        let _hir = typer.lower_program(&prog).unwrap();
    }

    #[test]
    fn test_instantiation_creates_fresh_vars() {
        let mut ctx = unify::InferCtx::new();
        let a = ctx.fresh_var();
        let fn_ty = Type::Fn(vec![a.clone()], Box::new(a.clone()));
        let scheme = Scheme {
            quantified: vec![0],
            ty: fn_ty,
        };
        let inst1 = ctx.instantiate(&scheme);
        let inst2 = ctx.instantiate(&scheme);
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
        assert!(
            ctx.unify(&v, &Type::F64).is_err(),
            "integer-constrained var must reject F64"
        );
    }

    #[test]
    fn test_constrained_var_float_rejects_int() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_float_var();
        assert!(
            ctx.unify(&v, &Type::I64).is_err(),
            "float-constrained var must reject I64"
        );
    }

    #[test]
    fn test_constrained_var_numeric_accepts_both() {
        let mut ctx = unify::InferCtx::new();
        let v1 = ctx.fresh_numeric_var();
        assert!(
            ctx.unify(&v1, &Type::I64).is_ok(),
            "numeric var must accept I64"
        );
        let v2 = ctx.fresh_numeric_var();
        assert!(
            ctx.unify(&v2, &Type::F64).is_ok(),
            "numeric var must accept F64"
        );
    }

    #[test]
    fn test_constrained_var_numeric_rejects_string() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_numeric_var();
        assert!(
            ctx.unify(&v, &Type::String).is_err(),
            "numeric var must reject String"
        );
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
        assert_eq!(ctx.resolve(&v), Type::I64);
    }

    #[test]
    fn test_return_type_inferred_from_tail() {
        let hir = type_check("*double(x: i64) -> i64\n    x * 2\n*main()\n    log(double(5))\n");
        let double = hir.fns.iter().find(|f| f.name == "double").unwrap();
        assert_eq!(double.ret, Type::I64);
    }

    #[test]
    fn test_return_type_inferred_from_return_stmt() {
        let hir = type_check(
            "*abs(x: i64) -> i64\n    if x < 0\n        return -x\n    x\n*main()\n    log(abs(-5))\n",
        );
        let abs_fn = hir.fns.iter().find(|f| f.name == "abs").unwrap();
        assert_eq!(abs_fn.ret, Type::I64);
    }

    #[test]
    fn test_recursive_fn_return_type() {
        let hir = type_check(
            "*fib(n: i64) -> i64\n    if n <= 1\n        return n\n    fib(n - 1) + fib(n - 2)\n*main()\n    log(fib(10))\n",
        );
        let fib = hir.fns.iter().find(|f| f.name == "fib").unwrap();
        assert_eq!(fib.ret, Type::I64);
    }

    #[test]
    fn test_deferred_field_no_typevars() {
        let hir = type_check(
            "type Point\n    x: i64\n    y: i64\n\n*main() -> i32\n    p is Point(x is 10, y is 20)\n    log(p.x + p.y)\n    0\n",
        );
        let point = &hir.types[0];
        assert!(!point.fields[0].ty.has_type_var());
        assert!(!point.fields[1].ty.has_type_var());
    }

    #[test]
    fn test_vec_method_types_resolved() {
        let hir = type_check(
            "*main() -> i32\n    v is vec(1, 2, 3)\n    v.push(4)\n    log(v.len())\n    0\n",
        );
        let main = &hir.fns[0];
        for stmt in &main.body {
            check_no_typevars_in_stmt(stmt);
        }
    }

    fn check_no_typevars_in_stmt(stmt: &hir::Stmt) {
        match stmt {
            hir::Stmt::Bind(b) => {
                assert!(
                    !b.ty.has_type_var(),
                    "TypeVar in bind: {} has type {}",
                    b.name,
                    b.ty
                );
            }
            hir::Stmt::Expr(e) => {
                assert!(!e.ty.has_type_var(), "TypeVar in expr: {}", e.ty);
            }
            _ => {}
        }
    }

    #[test]
    fn test_type_error_add_bool_int() {
        let prog = parse("*main()\n    x is true + 1\n    log(x)\n");
        let mut typer = Typer::new();
        let _ = typer.lower_program(&prog);
    }

    #[test]
    fn test_concrete_mismatch_fn_arg() {
        let mut ctx = unify::InferCtx::new();
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

    #[test]
    fn test_no_typevar_in_simple_fn() {
        let hir =
            type_check("*add(a: i64, b: i64) -> i64\n    a + b\n*main()\n    log(add(1, 2))\n");
        for f in &hir.fns {
            assert!(
                !f.ret.has_type_var(),
                "fn {} has TypeVar in ret: {}",
                f.name,
                f.ret
            );
            for p in &f.params {
                assert!(
                    !p.ty.has_type_var(),
                    "fn {} param {} has TypeVar: {}",
                    f.name,
                    p.name,
                    p.ty
                );
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
                assert!(
                    !f.ty.has_type_var(),
                    "struct {} field {} has TypeVar: {}",
                    td.name,
                    f.name,
                    f.ty
                );
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
                    assert!(
                        !vf.ty.has_type_var(),
                        "enum {} variant {} has TypeVar: {}",
                        ed.name,
                        v.name,
                        vf.ty
                    );
                }
            }
        }
    }

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
        let mut ctx = unify::InferCtx::new();
        let a = ctx.fresh_integer_var();
        let b = ctx.fresh_numeric_var();
        ctx.unify(&a, &b).unwrap();
        assert!(
            ctx.unify(&a, &Type::F64).is_err(),
            "merged Integer+Numeric constraint should reject F64"
        );
    }

    #[test]
    fn test_constraint_merge_float_wins() {
        let mut ctx = unify::InferCtx::new();
        let a = ctx.fresh_float_var();
        let b = ctx.fresh_numeric_var();
        ctx.unify(&a, &b).unwrap();
        assert!(
            ctx.unify(&a, &Type::I64).is_err(),
            "merged Float+Numeric constraint should reject I64"
        );
    }

    #[test]
    fn test_bidirectional_call_result_unifies_with_expected() {
        let mut ctx = unify::InferCtx::new();
        let ret_var = ctx.fresh_var();
        let expected = Type::I64;
        ctx.unify(&expected, &ret_var).unwrap();
        assert_eq!(ctx.resolve(&ret_var), Type::I64);
    }

    #[test]
    fn test_bidirectional_call_result_propagates_through_chain() {
        let mut ctx = unify::InferCtx::new();
        let ret_var = ctx.fresh_var();
        let intermediate = ctx.fresh_var();
        ctx.unify(&intermediate, &ret_var).unwrap();
        ctx.unify(&Type::F64, &intermediate).unwrap();
        assert_eq!(ctx.resolve(&ret_var), Type::F64);
    }

    #[test]
    fn test_bidirectional_numeric_var_constrained_by_expected() {
        let mut ctx = unify::InferCtx::new();
        let ret_var = ctx.fresh_numeric_var();
        ctx.unify(&Type::F64, &ret_var).unwrap();
        assert_eq!(ctx.resolve(&ret_var), Type::F64);
    }

    #[test]
    fn test_strict_types_errors_on_unconstrained_typevar() {
        let mut ctx = unify::InferCtx::new();
        ctx.enable_strict_types();
        let span = crate::ast::Span {
            start: 0,
            end: 0,
            line: 5,
            col: 3,
        };
        let v = ctx.fresh_var();
        let v2 = ctx.fresh_var();
        let _ = ctx.unify_at(&v, &v2, span, "test binding");
        let resolved = ctx.resolve(&v);
        assert_eq!(resolved, Type::I64);
        let errors = ctx.drain_strict_errors();
        assert!(
            !errors.is_empty(),
            "strict mode should produce errors for unconstrained TypeVar"
        );
        assert!(
            errors[0].contains("ambiguous type"),
            "error should mention ambiguity: {}",
            errors[0]
        );
    }

    #[test]
    fn test_strict_types_allows_integer_default() {
        let mut ctx = unify::InferCtx::new();
        ctx.enable_strict_types();
        let v = ctx.fresh_integer_var();
        let resolved = ctx.resolve(&v);
        assert_eq!(resolved, Type::I64);
        let errors = ctx.drain_strict_errors();
        assert!(
            errors.is_empty(),
            "Integer→I64 should be allowed in strict mode"
        );
    }

    #[test]
    fn test_strict_types_allows_float_default() {
        let mut ctx = unify::InferCtx::new();
        ctx.enable_strict_types();
        let v = ctx.fresh_float_var();
        let resolved = ctx.resolve(&v);
        assert_eq!(resolved, Type::F64);
        let errors = ctx.drain_strict_errors();
        assert!(
            errors.is_empty(),
            "Float→F64 should be allowed in strict mode"
        );
    }

    #[test]
    fn test_strict_types_numeric_defaults_with_warning() {
        let mut ctx = unify::InferCtx::new();
        ctx.enable_strict_types();
        ctx.enable_default_warnings();
        let span = crate::ast::Span {
            start: 0,
            end: 0,
            line: 10,
            col: 1,
        };
        let v = ctx.fresh_numeric_var();
        let v2 = ctx.fresh_var();
        let _ = ctx.unify_at(&v, &v2, span, "numeric op");
        let resolved = ctx.resolve(&v);
        assert_eq!(resolved, Type::I64, "Numeric should default to I64");
        let errors = ctx.drain_strict_errors();
        assert!(
            errors.is_empty(),
            "Numeric should NOT produce strict errors (now a warning): {:?}",
            errors
        );
        let warnings = ctx.drain_default_warnings();
        assert!(
            !warnings.is_empty(),
            "Numeric should produce a default warning"
        );
        assert!(
            warnings[0].contains("numeric type defaults to i64"),
            "warning: {}",
            warnings[0]
        );
    }

    #[test]
    fn test_strict_types_no_error_for_solved_vars() {
        let mut ctx = unify::InferCtx::new();
        ctx.enable_strict_types();
        let v = ctx.fresh_var();
        ctx.unify(&v, &Type::String).unwrap();
        let resolved = ctx.resolve(&v);
        assert_eq!(resolved, Type::String);
        let errors = ctx.drain_strict_errors();
        assert!(
            errors.is_empty(),
            "solved TypeVars should not produce errors"
        );
    }

    #[test]
    fn test_strict_types_integration_well_typed_program() {
        let src = "*main()\n    x is 42\n    log(x)\n";
        let prog = parse(src);
        let mut typer = Typer::new();
        typer.set_strict_types(true);
        let result = typer.lower_program(&prog);
        assert!(
            result.is_ok(),
            "well-typed program should succeed in strict mode: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_strict_types_integration_annotated_fn() {
        let src = "*add(a: i64, b: i64) -> i64\n    a + b\n*main()\n    log(add(1, 2))\n";
        let prog = parse(src);
        let mut typer = Typer::new();
        typer.set_strict_types(true);
        let result = typer.lower_program(&prog);
        assert!(
            result.is_ok(),
            "annotated function should succeed in strict mode: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_error_message_type_mismatch_has_provenance() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_var();
        let span1 = crate::ast::Span {
            start: 0,
            end: 0,
            line: 3,
            col: 5,
        };
        let span2 = crate::ast::Span {
            start: 0,
            end: 0,
            line: 7,
            col: 10,
        };
        ctx.unify_at(&v, &Type::String, span1, "bind annotation")
            .unwrap();
        let err = ctx
            .unify_at(&v, &Type::I64, span2, "function argument")
            .unwrap_err();
        assert!(
            err.contains("line 7:10"),
            "error should contain error span: {err}"
        );
        assert!(
            err.contains("function argument"),
            "error should contain reason: {err}"
        );
    }

    #[test]
    fn test_error_message_suggests_cast_for_int_float() {
        let mut ctx = unify::InferCtx::new();
        let span = crate::ast::Span {
            start: 0,
            end: 0,
            line: 5,
            col: 1,
        };
        let err = ctx
            .unify_at(&Type::I64, &Type::F64, span, "binary operands")
            .unwrap_err();
        assert!(err.contains("as"), "error should suggest a cast: {err}");
    }

    #[test]
    fn test_error_message_suggests_to_string() {
        let mut ctx = unify::InferCtx::new();
        let span = crate::ast::Span {
            start: 0,
            end: 0,
            line: 5,
            col: 1,
        };
        let err = ctx
            .unify_at(&Type::String, &Type::I64, span, "function argument")
            .unwrap_err();
        assert!(
            err.contains("to_string") || err.contains("check that the argument"),
            "error should suggest conversion: {err}"
        );
    }

    #[test]
    fn test_error_message_binary_operand_help() {
        let mut ctx = unify::InferCtx::new();
        let span = crate::ast::Span {
            start: 0,
            end: 0,
            line: 3,
            col: 1,
        };
        let err = ctx
            .unify_at(&Type::String, &Type::Bool, span, "binary operands")
            .unwrap_err();
        assert!(err.contains("help:"), "error should have help text: {err}");
    }

    #[test]
    fn test_constrain_typevar_to_numeric() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_var();
        let span = crate::ast::Span {
            start: 0,
            end: 0,
            line: 1,
            col: 1,
        };
        ctx.constrain(&v, unify::TypeConstraint::Numeric, span, "arithmetic")
            .unwrap();
        let err = ctx.unify(&v, &Type::String);
        assert!(
            err.is_err(),
            "Numeric-constrained TypeVar should reject String"
        );
        let mut ctx2 = unify::InferCtx::new();
        let v2 = ctx2.fresh_var();
        ctx2.constrain(&v2, unify::TypeConstraint::Numeric, span, "arithmetic")
            .unwrap();
        assert!(ctx2.unify(&v2, &Type::I64).is_ok());
    }

    #[test]
    fn test_constrain_typevar_to_integer() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_var();
        let span = crate::ast::Span {
            start: 0,
            end: 0,
            line: 1,
            col: 1,
        };
        ctx.constrain(&v, unify::TypeConstraint::Integer, span, "bitwise")
            .unwrap();
        assert!(ctx.unify(&v, &Type::F64).is_err());
        let mut ctx2 = unify::InferCtx::new();
        let v2 = ctx2.fresh_var();
        ctx2.constrain(&v2, unify::TypeConstraint::Integer, span, "bitwise")
            .unwrap();
        assert!(ctx2.unify(&v2, &Type::I64).is_ok());
    }

    #[test]
    fn test_constrain_already_solved_validates() {
        let mut ctx = unify::InferCtx::new();
        let v = ctx.fresh_var();
        let span = crate::ast::Span {
            start: 0,
            end: 0,
            line: 1,
            col: 1,
        };
        ctx.unify(&v, &Type::String).unwrap();
        let err = ctx.constrain(&v, unify::TypeConstraint::Numeric, span, "arithmetic");
        assert!(err.is_err(), "should reject String for Numeric constraint");
    }

    #[test]
    fn test_arithmetic_operators_constrain_typevar_params() {
        let src = "*add(a, b)\n    a + b\n*main()\n    log(add(3, 4))\n";
        let prog = parse(src);
        let mut typer = Typer::new();
        let hir = typer.lower_program(&prog).unwrap();
        let add_fn = hir.fns.iter().find(|f| f.name.starts_with("add")).unwrap();
        assert!(
            add_fn.params[0].ty.is_num(),
            "add param 'a' should be numeric: {:?}",
            add_fn.params[0].ty
        );
    }

    #[test]
    fn test_operator_constraints_allow_float_arithmetic() {
        let src = "*mul(a, b)\n    a * b\n*main()\n    log(mul(2.5, 3.0))\n";
        let hir = type_check(src);
        let mul_fn = hir.fns.iter().find(|f| f.name.starts_with("mul")).unwrap();
        assert!(
            mul_fn.params[0].ty.is_float(),
            "param should be float: {:?}",
            mul_fn.params[0].ty
        );
    }

    #[test]
    fn test_string_concat_not_broken_by_constraints() {
        let src = "*main()\n    s is \"hello\" + \" world\"\n    log(s)\n";
        let hir = type_check(src);
        let main = &hir.fns[0];
        if let hir::Stmt::Bind(b) = &main.body[0] {
            assert_eq!(b.ty, Type::String, "string concat should produce String");
        }
    }

    #[test]
    fn test_bitwise_ops_constrain_to_integer() {
        let src = "*bitop(a, b)\n    a & b\n*main()\n    log(bitop(0xFF, 0x0F))\n";
        let hir = type_check(src);
        let bitop = hir
            .fns
            .iter()
            .find(|f| f.name.starts_with("bitop"))
            .unwrap();
        assert!(
            bitop.params[0].ty.is_int(),
            "bitwise param should be integer: {:?}",
            bitop.params[0].ty
        );
    }

    #[test]
    fn test_mutual_recursion_is_even_is_odd() {
        let src = "*is_even(n)\n    if n equals 0\n        return 1\n    is_odd(n - 1)\n\n*is_odd(n)\n    if n equals 0\n        return 0\n    is_even(n - 1)\n\n*main()\n    log(is_even(10))\n";
        let hir = type_check(src);
        let is_even = hir
            .fns
            .iter()
            .find(|f| f.name.starts_with("is_even"))
            .unwrap();
        let is_odd = hir
            .fns
            .iter()
            .find(|f| f.name.starts_with("is_odd"))
            .unwrap();
        assert_eq!(
            is_even.params[0].ty,
            Type::I64,
            "is_even param type: {:?}",
            is_even.params[0].ty
        );
        assert_eq!(
            is_even.ret,
            Type::I64,
            "is_even return type: {:?}",
            is_even.ret
        );
        assert_eq!(
            is_odd.params[0].ty,
            Type::I64,
            "is_odd param type: {:?}",
            is_odd.params[0].ty
        );
        assert_eq!(
            is_odd.ret,
            Type::I64,
            "is_odd return type: {:?}",
            is_odd.ret
        );
    }

    #[test]
    fn test_mutual_recursion_no_typevars_remain() {
        let src = "*ping(n)\n    if n equals 0\n        return 0\n    pong(n - 1)\n\n*pong(n)\n    if n equals 0\n        return 0\n    ping(n - 1)\n\n*main()\n    log(ping(5))\n";
        let hir = type_check(src);
        for f in &hir.fns {
            assert!(
                !f.ret.has_type_var(),
                "{} has TypeVar in return: {:?}",
                f.name,
                f.ret
            );
            for p in &f.params {
                assert!(
                    !p.ty.has_type_var(),
                    "{} param {} has TypeVar: {:?}",
                    f.name,
                    p.name,
                    p.ty
                );
            }
        }
    }

    #[test]
    fn test_scc_three_way_mutual_recursion() {
        let src = "*f1(n)\n    if n equals 0\n        return 0\n    f2(n - 1)\n\n*f2(n)\n    if n equals 0\n        return 0\n    f3(n - 1)\n\n*f3(n)\n    if n equals 0\n        return 0\n    f1(n - 1)\n\n*main()\n    log(f1(9))\n";
        let hir = type_check(src);
        let f1 = hir.fns.iter().find(|f| f.name.starts_with("f1")).unwrap();
        let f2 = hir.fns.iter().find(|f| f.name.starts_with("f2")).unwrap();
        let f3 = hir.fns.iter().find(|f| f.name.starts_with("f3")).unwrap();
        assert_eq!(f1.ret, Type::I64);
        assert_eq!(f2.ret, Type::I64);
        assert_eq!(f3.ret, Type::I64);
        assert_eq!(f1.params[0].ty, Type::I64);
        assert_eq!(f2.params[0].ty, Type::I64);
        assert_eq!(f3.params[0].ty, Type::I64);
    }

    #[test]
    fn test_implicit_generic_identity_multi_type() {
        let src =
            "*identity(x)\n    x\n*main()\n    log(identity(42))\n    log(identity(\"hello\"))\n";
        let hir = type_check(src);
        let id_fns: Vec<_> = hir
            .fns
            .iter()
            .filter(|f| f.name.starts_with("identity"))
            .collect();
        assert!(
            id_fns.len() >= 2,
            "expected at least 2 identity monomorphizations, got {}: {:?}",
            id_fns.len(),
            id_fns.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let has_i64 = id_fns.iter().any(|f| f.params[0].ty == Type::I64);
        let has_string = id_fns.iter().any(|f| f.params[0].ty == Type::String);
        assert!(has_i64, "expected I64-specialized identity");
        assert!(has_string, "expected String-specialized identity");
    }

    #[test]
    fn test_implicit_generic_single_type() {
        let src = "*double(x)\n    x + x\n*main()\n    log(double(21))\n";
        let hir = type_check(src);
        let dbl_fns: Vec<_> = hir
            .fns
            .iter()
            .filter(|f| f.name.starts_with("double"))
            .collect();
        assert_eq!(
            dbl_fns.len(),
            1,
            "should have exactly one monomorphized version"
        );
        assert_eq!(dbl_fns[0].params[0].ty, Type::I64);
    }

    #[test]
    fn test_implicit_generic_no_typevars_in_output() {
        let src = "*swap(a, b)\n    b\n*main()\n    log(swap(1, 2))\n";
        let hir = type_check(src);
        for f in &hir.fns {
            assert!(
                !f.ret.has_type_var(),
                "{} has TypeVar in return: {:?}",
                f.name,
                f.ret
            );
            for p in &f.params {
                assert!(
                    !p.ty.has_type_var(),
                    "{} param {} has TypeVar: {:?}",
                    f.name,
                    p.name,
                    p.ty
                );
            }
        }
    }

    #[test]
    fn test_hof_apply_infers_fn_param() {
        let src = "*add1(x: i64) -> i64\n    x + 1\n*apply(f, x)\n    f(x)\n*main()\n    log(apply(add1, 42))\n";
        let hir = type_check(src);
        let apply_fn = hir
            .fns
            .iter()
            .find(|f| f.name.starts_with("apply"))
            .unwrap();
        assert!(
            matches!(&apply_fn.params[0].ty, Type::Fn(_, _)),
            "f should be Fn type, got {:?}",
            apply_fn.params[0].ty
        );
        assert_eq!(apply_fn.params[1].ty, Type::I64);
        assert_eq!(apply_fn.ret, Type::I64);
    }

    #[test]
    fn test_hof_compose_infers_two_fn_params() {
        let src = "*inc(x: i64) -> i64\n    x + 1\n*dbl(x: i64) -> i64\n    x * 2\n*compose(f, g, x)\n    f(g(x))\n*main()\n    log(compose(inc, dbl, 20))\n";
        let hir = type_check(src);
        let compose_fn = hir
            .fns
            .iter()
            .find(|f| f.name.starts_with("compose"))
            .unwrap();
        assert!(
            matches!(&compose_fn.params[0].ty, Type::Fn(_, _)),
            "f should be Fn, got {:?}",
            compose_fn.params[0].ty
        );
        assert!(
            matches!(&compose_fn.params[1].ty, Type::Fn(_, _)),
            "g should be Fn, got {:?}",
            compose_fn.params[1].ty
        );
        assert_eq!(compose_fn.params[2].ty, Type::I64);
        assert_eq!(compose_fn.ret, Type::I64);
    }

    #[test]
    fn test_hof_apply_twice() {
        let src = "*inc(x: i64) -> i64\n    x + 1\n*apply_twice(f, x)\n    f(f(x))\n*main()\n    log(apply_twice(inc, 40))\n";
        let hir = type_check(src);
        let at_fn = hir
            .fns
            .iter()
            .find(|f| f.name.starts_with("apply_twice"))
            .unwrap();
        assert!(matches!(&at_fn.params[0].ty, Type::Fn(_, _)));
        assert_eq!(at_fn.ret, Type::I64);
    }

    #[test]
    fn test_hof_no_typevars_remain() {
        let src = "*inc(x: i64) -> i64\n    x + 1\n*apply(f, x)\n    f(x)\n*main()\n    log(apply(inc, 42))\n";
        let hir = type_check(src);
        for f in &hir.fns {
            assert!(
                !f.ret.has_type_var(),
                "{} has TypeVar in return: {:?}",
                f.name,
                f.ret
            );
            for p in &f.params {
                assert!(
                    !p.ty.has_type_var(),
                    "{} param {} has TypeVar: {:?}",
                    f.name,
                    p.name,
                    p.ty
                );
            }
        }
    }

    #[test]
    fn test_lambda_standalone_unannotated_param_integer() {
        let src = "*main()\n    f is *fn(x) x + 1\n    log(f(5))\n";
        let prog = parse(src);
        let mut typer = Typer::new();
        let hir = typer.lower_program(&prog).unwrap();
        let main = &hir.fns[0];
        if let hir::Stmt::Bind(b) = &main.body[0] {
            assert_eq!(b.name, "f");
            if let Type::Fn(ptys, ret) = &b.ty {
                assert_eq!(ptys[0], Type::I64, "lambda param should infer to I64");
                assert_eq!(**ret, Type::I64, "lambda return should infer to I64");
            } else {
                panic!("expected Fn type for f, got {:?}", b.ty);
            }
        }
    }

    #[test]
    fn test_lambda_standalone_unannotated_param_float() {
        let src = "*main()\n    f is *fn(x) x + 1.0\n    log(f(2.5))\n";
        let prog = parse(src);
        let mut typer = Typer::new();
        let hir = typer.lower_program(&prog).unwrap();
        let main = &hir.fns[0];
        if let hir::Stmt::Bind(b) = &main.body[0] {
            if let Type::Fn(ptys, _) = &b.ty {
                assert!(
                    ptys[0].is_float(),
                    "lambda param should be float: {:?}",
                    ptys[0]
                );
            }
        }
    }

    #[test]
    fn test_lambda_let_bound_then_called() {
        let src = "*main()\n    f is *fn(x) x + 1\n    result is f(42)\n    log(result)\n";
        let hir = type_check(src);
        let main = &hir.fns[0];
        if let hir::Stmt::Bind(b) = &main.body[1] {
            assert_eq!(b.name, "result");
            assert_eq!(b.ty, Type::I64, "result should be I64, got {:?}", b.ty);
        }
    }

    #[test]
    fn test_lambda_passed_to_hof_infers_type() {
        let src = "*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n*main()\n    log(apply(*fn(x) x + 1, 5))\n";
        let hir = type_check(src);
        let apply_fn = hir.fns.iter().find(|f| f.name == "apply").unwrap();
        assert_eq!(apply_fn.ret, Type::I64);
    }

    #[test]
    fn test_fn_scheme_quantified_exempt_from_strict() {
        let src = "*identity(x)\n    x\n*main()\n    log(identity(42))\n";
        let prog = parse(src);
        let mut typer = Typer::new();
        typer.set_strict_types(true);
        let result = typer.lower_program(&prog);
        assert!(
            result.is_ok(),
            "scheme-quantified params should not error in strict mode: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_fn_scheme_polymorphic_identity_strict() {
        let src =
            "*identity(x)\n    x\n*main()\n    log(identity(42))\n    log(identity(\"hello\"))\n";
        let prog = parse(src);
        let mut typer = Typer::new();
        typer.set_strict_types(true);
        let result = typer.lower_program(&prog);
        assert!(
            result.is_ok(),
            "polymorphic multi-use should work in strict mode: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_fn_numeric_param_defaults_in_strict() {
        let src = "*double(x)\n    x + x\n*main()\n    log(double(21))\n";
        let prog = parse(src);
        let mut typer = Typer::new();
        typer.set_strict_types(true);
        let result = typer.lower_program(&prog);
        assert!(
            result.is_ok(),
            "*double(x) x+x should work in strict mode: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_cross_function_constraint_flow() {
        let src = "*inc(x: i64) -> i64\n    x + 1\n*apply_inc(x)\n    inc(x)\n*main()\n    log(apply_inc(5))\n";
        let hir = type_check(src);
        let apply_inc = hir
            .fns
            .iter()
            .find(|f| f.name.starts_with("apply_inc"))
            .unwrap();
        assert_eq!(
            apply_inc.params[0].ty,
            Type::I64,
            "x should be constrained to I64 via inc()"
        );
        assert_eq!(apply_inc.ret, Type::I64);
    }

    #[test]
    fn test_struct_field_inferred_from_constructor() {
        let src =
            "type Point\n    x\n    y\n\n*main()\n    p is Point(x is 1, y is 2)\n    log(p.x)\n";
        let prog = parse(src);
        let mut typer = Typer::new();
        typer.set_lenient(true);
        let result = typer.lower_program(&prog);
        assert!(
            result.is_ok(),
            "struct field inference from constructor should work: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_return_type_inference_no_annotation() {
        let src = "*square(x: i64)\n    x * x\n*main()\n    log(square(5))\n";
        let hir = type_check(src);
        let square = hir.fns.iter().find(|f| f.name == "square").unwrap();
        assert_eq!(
            square.ret,
            Type::I64,
            "return type should be inferred as I64"
        );
    }

    #[test]
    fn test_multipath_return_type_inference() {
        let src =
            "*abs(x: i64)\n    if x < 0\n        return -x\n    x\n*main()\n    log(abs(-5))\n";
        let hir = type_check(src);
        let abs_fn = hir.fns.iter().find(|f| f.name == "abs").unwrap();
        assert_eq!(abs_fn.ret, Type::I64);
    }

    #[test]
    fn test_mutual_recursion_unannotated_params() {
        let src = "*is_even(n)\n    if n equals 0\n        return 1\n    is_odd(n - 1)\n\n*is_odd(n)\n    if n equals 0\n        return 0\n    is_even(n - 1)\n\n*main()\n    log(is_even(4))\n";
        let hir = type_check(src);
        let is_even = hir
            .fns
            .iter()
            .find(|f| f.name.starts_with("is_even"))
            .unwrap();
        let is_odd = hir
            .fns
            .iter()
            .find(|f| f.name.starts_with("is_odd"))
            .unwrap();
        assert_eq!(is_even.params[0].ty, Type::I64);
        assert_eq!(is_odd.params[0].ty, Type::I64);
    }

    #[test]
    fn test_nested_lambda_inference() {
        let src = "*main()\n    f is *fn(x) *fn(y) x + y\n    g is f(10)\n    log(g(20))\n";
        let hir = type_check(src);
        let main = &hir.fns[0];
        if let hir::Stmt::Bind(b) = &main.body[0] {
            assert_eq!(b.name, "f");
            assert!(!b.ty.has_type_var(), "f type has TypeVar: {:?}", b.ty);
        }
    }

    #[test]
    fn test_scc_cycle_with_typevar_params() {
        let src = "*a(x)\n    if x equals 0\n        return 0\n    b(x - 1) + 1\n\n*b(x)\n    if x equals 0\n        return 0\n    a(x - 1) + 1\n\n*main()\n    log(a(5))\n";
        let hir = type_check(src);
        let a_fn = hir.fns.iter().find(|f| f.name.starts_with("a")).unwrap();
        let b_fn = hir.fns.iter().find(|f| f.name.starts_with("b")).unwrap();
        assert_eq!(a_fn.params[0].ty, Type::I64);
        assert_eq!(b_fn.params[0].ty, Type::I64);
        assert_eq!(a_fn.ret, Type::I64);
        assert_eq!(b_fn.ret, Type::I64);
    }

    #[test]
    fn test_poly_multi_use_identity() {
        let src = "*main()\n    id is *fn(x) x\n    a is id(42)\n    b is id(\"hello\")\n    log(a)\n    log(b)\n";
        let hir = type_check(src);
        let main = &hir.fns[0];
        for stmt in &main.body {
            if let hir::Stmt::Bind(b) = stmt {
                match b.name.as_str() {
                    "a" => assert_eq!(b.ty, Type::I64, "a should be I64, got {:?}", b.ty),
                    "b" => assert_eq!(b.ty, Type::String, "b should be String, got {:?}", b.ty),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn test_strict_vs_lenient_comparison() {
        let src = "*double(x: i64) -> i64\n    x + x\n*main()\n    log(double(21))\n";
        let prog = parse(src);

        let mut typer_lenient = Typer::new();
        let result_lenient = typer_lenient.lower_program(&prog);
        assert!(
            result_lenient.is_ok(),
            "lenient mode failed: {:?}",
            result_lenient.err()
        );

        let prog2 = parse(src);
        let mut typer_strict = Typer::new();
        typer_strict.set_strict_types(true);
        let result_strict = typer_strict.lower_program(&prog2);
        assert!(
            result_strict.is_ok(),
            "strict mode failed on fully annotated program: {:?}",
            result_strict.err()
        );
    }

    #[test]
    fn test_strict_rejects_completely_unconstrained() {
        let src = "*unused_param(x)\n    42\n*main()\n    log(unused_param(0))\n";
        let prog = parse(src);
        let mut typer = Typer::new();
        typer.set_strict_types(true);
        let result = typer.lower_program(&prog);
        assert!(
            result.is_ok(),
            "param constrained by call site should work: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_scheme_instantiation_with_container() {
        let src = "*wrap(x: i64)\n    v is vec()\n    v.push(x)\n    v\n*main()\n    w is wrap(42)\n    log(w.len())\n";
        let hir = type_check(src);
        let wrap_fn = hir.fns.iter().find(|f| f.name == "wrap").unwrap();
        assert_eq!(wrap_fn.params[0].ty, Type::I64);
    }

    #[test]
    fn test_vec_push_constrains_element_type() {
        let src = "*main()\n    v is vec()\n    v.push(42)\n    log(v.len())\n";
        let hir = type_check(src);
        let main = &hir.fns[0];
        for stmt in &main.body {
            if let hir::Stmt::Bind(b) = stmt {
                if b.name == "v" {
                    assert!(
                        !b.ty.has_type_var(),
                        "vec should have resolved element type: {:?}",
                        b.ty
                    );
                }
            }
        }
    }

    #[test]
    fn test_value_restriction_syntactic_value() {
        let src = "*main()\n    id is *fn(x) x\n    a is id(42)\n    b is id(\"hi\")\n    log(a)\n    log(b)\n";
        let prog = parse(src);
        let mut typer = Typer::new();
        let hir = typer.lower_program(&prog).unwrap();
        let main = &hir.fns[0];
        for stmt in &main.body {
            if let hir::Stmt::Bind(b) = stmt {
                match b.name.as_str() {
                    "a" => assert_eq!(b.ty, Type::I64, "a should be I64"),
                    "b" => assert_eq!(b.ty, Type::String, "b should be String"),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn test_unannotated_fn_called_with_string() {
        let src = "*echo(x)\n    x\n*main()\n    log(echo(\"hello\"))\n";
        let hir = type_check(src);
        let echo = hir.fns.iter().find(|f| f.name.starts_with("echo")).unwrap();
        assert_eq!(echo.params[0].ty, Type::String);
        assert_eq!(echo.ret, Type::String);
    }

    #[test]
    fn test_unannotated_fn_called_with_bool() {
        let src =
            "*negate(x)\n    if x\n        return 0\n    1\n*main()\n    log(negate(1 equals 1))\n";
        let hir = type_check(src);
        let negate = hir
            .fns
            .iter()
            .find(|f| f.name.starts_with("negate"))
            .unwrap();
        assert_eq!(negate.params[0].ty, Type::Bool);
    }
}
