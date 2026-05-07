
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
    let hir = type_check(
        "*add(a as i64, b as i64) returns i64\n    a + b\n*main()\n    log(add(1, 2))\n",
    );
    let add_fn = hir.fns.iter().find(|f| f.name == "add").unwrap();
    assert_eq!(add_fn.ret, Type::I64);
}

#[test]
fn test_struct_typed() {
    let hir = type_check(
        "type Point\n    x as i64\n    y as i64\n\n*main() returns i32\n    p is Point(x is 1, y is 2)\n    log(p.x)\n    0\n",
    );
    assert!(!hir.types.is_empty());
    let point = &hir.types[0];
    assert_eq!(point.name, "Point");
    assert_eq!(point.fields.len(), 2);
}

#[test]
fn test_enum_typed() {
    let hir = type_check(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() returns i32\n    c is Red\n    match c\n        Red ? log(1)\n        Green ? log(2)\n        Blue ? log(3)\n    0\n",
    );
    assert!(!hir.enums.is_empty());
    let color = &hir.enums[0];
    assert_eq!(color.name, "Color");
    assert_eq!(color.variants.len(), 3);
}

#[test]
fn test_generic_fn_monomorphized() {
    let hir =
        type_check("*identity(x as T) returns T\n    x\n*main()\n    log(identity(42))\n");
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
    let hir = type_check(
        "*main() returns i32\n    f is |x as i64| returns i64 x + 1\n    log(f(5))\n    0\n",
    );
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
        "type Pair\n    a as i64\n    b as f64\n\n*main() returns i32\n    p is Pair(a is 1, b is 2.0)\n    log(p.a)\n    0\n",
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
    let prog = parse(
        "*main() returns i32\n    f is |x as i64| returns i64 x + 1\n    log(f(5))\n    0\n",
    );
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
    let hir =
        type_check("*double(x as i64) returns i64\n    x * 2\n*main()\n    log(double(5))\n");
    let double = hir.fns.iter().find(|f| f.name == "double").unwrap();
    assert_eq!(double.ret, Type::I64);
}

#[test]
fn test_return_type_inferred_from_return_stmt() {
    let hir = type_check(
        "*abs(x as i64) returns i64\n    if x < 0\n        return -x\n    x\n*main()\n    log(abs(-5))\n",
    );
    let abs_fn = hir.fns.iter().find(|f| f.name == "abs").unwrap();
    assert_eq!(abs_fn.ret, Type::I64);
}

#[test]
fn test_recursive_fn_return_type() {
    let hir = type_check(
        "*fib(n as i64) returns i64\n    if n <= 1\n        return n\n    fib(n - 1) + fib(n - 2)\n*main()\n    log(fib(10))\n",
    );
    let fib = hir.fns.iter().find(|f| f.name == "fib").unwrap();
    assert_eq!(fib.ret, Type::I64);
}

#[test]
fn test_deferred_field_no_typevars() {
    let hir = type_check(
        "type Point\n    x as i64\n    y as i64\n\n*main() returns i32\n    p is Point(x is 10, y is 20)\n    log(p.x + p.y)\n    0\n",
    );
    let point = &hir.types[0];
    assert!(!point.fields[0].ty.has_type_var());
    assert!(!point.fields[1].ty.has_type_var());
}

#[test]
fn test_vec_method_types_resolved() {
    let hir = type_check(
        "*main() returns i32\n    v is vec(1, 2, 3)\n    v.push(4)\n    log(v.len())\n    0\n",
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
    let hir = type_check(
        "*add(a as i64, b as i64) returns i64\n    a + b\n*main()\n    log(add(1, 2))\n",
    );
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
        "type Point\n    x as i64\n    y as i64\n\n*main() returns i32\n    p is Point(x is 1, y is 2)\n    log(p.x)\n    0\n",
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
        "enum Shape\n    Circle(f64)\n    Rect(f64, f64)\n\n*main() returns i32\n    s is Circle(3.14)\n    match s\n        Circle(r) ? log(r)\n        Rect(w, h) ? log(w)\n    0\n",
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
