
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
    let src = "*add(a as i64, b as i64) returns i64\n    a + b\n*main()\n    log(add(1, 2))\n";
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
    let src = "*add1(x as i64) returns i64\n    x + 1\n*apply(f, x)\n    f(x)\n*main()\n    log(apply(add1, 42))\n";
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
    let src = "*inc(x as i64) returns i64\n    x + 1\n*dbl(x as i64) returns i64\n    x * 2\n*compose(f, g, x)\n    f(g(x))\n*main()\n    log(compose(inc, dbl, 20))\n";
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
    let src = "*inc(x as i64) returns i64\n    x + 1\n*apply_twice(f, x)\n    f(f(x))\n*main()\n    log(apply_twice(inc, 40))\n";
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
    let src = "*inc(x as i64) returns i64\n    x + 1\n*apply(f, x)\n    f(x)\n*main()\n    log(apply(inc, 42))\n";
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
    let src = "*main()\n    f is |x| x + 1\n    log(f(5))\n";
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
    let src = "*main()\n    f is |x| x + 1.0\n    log(f(2.5))\n";
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
    let src = "*main()\n    f is |x| x + 1\n    result is f(42)\n    log(result)\n";
    let hir = type_check(src);
    let main = &hir.fns[0];
    if let hir::Stmt::Bind(b) = &main.body[1] {
        assert_eq!(b.name, "result");
        assert_eq!(b.ty, Type::I64, "result should be I64, got {:?}", b.ty);
    }
}

#[test]
fn test_lambda_passed_to_hof_infers_type() {
    let src = "*apply(f as (i64) returns i64, x as i64) returns i64\n    f(x)\n*main()\n    log(apply(|x| x + 1, 5))\n";
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
    let src = "*inc(x as i64) returns i64\n    x + 1\n*apply_inc(x)\n    inc(x)\n*main()\n    log(apply_inc(5))\n";
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
    let src = "*square(x as i64)\n    x * x\n*main()\n    log(square(5))\n";
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
        "*abs(x as i64)\n    if x < 0\n        return -x\n    x\n*main()\n    log(abs(-5))\n";
    let hir = type_check(src);
    let abs_fn = hir.fns.iter().find(|f| f.name == "abs").unwrap();
    assert_eq!(abs_fn.ret, Type::I64);
}
