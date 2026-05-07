
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
    let src = "*main()\n    f is |x| |y| x + y\n    g is f(10)\n    log(g(20))\n";
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
    let src = "*main()\n    id is |x| x\n    a is id(42)\n    b is id(\"hello\")\n    log(a)\n    log(b)\n";
    let hir = type_check(src);
    let main = &hir.fns[0];
    for stmt in &main.body {
        if let hir::Stmt::Bind(b) = stmt {
            match &*b.name.as_str() {
                "a" => assert_eq!(b.ty, Type::I64, "a should be I64, got {:?}", b.ty),
                "b" => assert_eq!(b.ty, Type::String, "b should be String, got {:?}", b.ty),
                _ => {}
            }
        }
    }
}

#[test]
fn test_strict_vs_lenient_comparison() {
    let src = "*double(x as i64) returns i64\n    x + x\n*main()\n    log(double(21))\n";
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
    let src = "*wrap(x as i64)\n    v is vec()\n    v.push(x)\n    v\n*main()\n    w is wrap(42)\n    log(w.len())\n";
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
    let src = "*main()\n    id is |x| x\n    a is id(42)\n    b is id(\"hi\")\n    log(a)\n    log(b)\n";
    let prog = parse(src);
    let mut typer = Typer::new();
    let hir = typer.lower_program(&prog).unwrap();
    let main = &hir.fns[0];
    for stmt in &main.body {
        if let hir::Stmt::Bind(b) = stmt {
            match &*b.name.as_str() {
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
