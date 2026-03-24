use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::ast;
use crate::hir::{self, DefId, Ownership};
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
    infer_depth: Cell<u8>,
    pub(crate) infer_ctx: unify::InferCtx,
    pub(crate) debug_types: bool,
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
            infer_depth: Cell::new(0),
            infer_ctx: unify::InferCtx::new(),
            debug_types: false,
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
}

mod expr;
mod infer;
mod lower;
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
}
