#![cfg(test)]

use super::*;
use crate::lexer::Lexer;
use crate::types::Type;
fn parse(s: &str) -> Program {
    let t = Lexer::new(s).tokenize().unwrap();
    Parser::new(t).parse_program().unwrap()
}

#[test]
fn hello() {
    let p = parse("*main()\n    log('hello')\n");
    assert_eq!(p.decls.len(), 1);
    if let Decl::Fn(f) = &p.decls[0] {
        assert_eq!(f.name, "main");
        assert_eq!(f.params.len(), 0);
        assert_eq!(f.body.len(), 1);
    } else {
        panic!("expected fn");
    }
}

#[test]
fn binding() {
    let p = parse("*main()\n    x is 42\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            assert_eq!(b.name, "x");
        } else {
            panic!("expected bind");
        }
    }
}

#[test]
fn fibonacci() {
    let p = parse("*fibonacci(n)\n    n <= 1 ? n ! fibonacci(n - 1) + fibonacci(n - 2)\n");
    assert_eq!(p.decls.len(), 1);
    if let Decl::Fn(f) = &p.decls[0] {
        assert_eq!(f.name, "fibonacci");
        assert_eq!(f.params.len(), 1);
    }
}

#[test]
fn if_else() {
    let p = parse("*main()\n    if true\n        log('yes')\n    else\n        log('no')\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::If(i) = &f.body[0] {
            assert!(i.els.is_some());
        } else {
            panic!("expected if");
        }
    }
}

#[test]
fn struct_def() {
    let p = parse("type Point\n    x is 0.0\n    y is 0.0\n");
    if let Decl::Type(td) = &p.decls[0] {
        assert_eq!(td.name, "Point");
        assert_eq!(td.fields.len(), 2);
    } else {
        panic!("expected type def");
    }
}

#[test]
fn enum_def() {
    let p = parse("enum Direction\n    North\n    South\n    East\n    West\n");
    if let Decl::Enum(ed) = &p.decls[0] {
        assert_eq!(ed.name, "Direction");
        assert_eq!(ed.variants.len(), 4);
    } else {
        panic!("expected enum");
    }
}

#[test]
fn ternary() {
    let p = parse("*main()\n    x is 1 > 0 ? 42 ! 0\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            assert!(matches!(b.value, Expr::Ternary(_, _, _, _)));
        } else {
            panic!("expected bind");
        }
    }
}

#[test]
fn while_stmt() {
    let p = parse("*main()\n    while true\n        log(1)\n");
    if let Decl::Fn(f) = &p.decls[0] {
        assert!(matches!(f.body[0], Stmt::While(_)));
    }
}

#[test]
fn for_stmt() {
    let p = parse("*main()\n    for i in 10\n        log(i)\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::For(fo) = &f.body[0] {
            assert_eq!(fo.bind, "i");
        } else {
            panic!("expected for");
        }
    }
}

#[test]
fn c_style_loop_desugars_to_bind_then_while() {
    // `loop(0, $ < 3, $ + 1)` should desugar to:
    //     __cph_N is 0
    //     while __cph_N < 3
    //         log(__cph_N)
    //         __cph_N is __cph_N + 1
    let p = parse("*main()\n    loop(0, $ < 3, $ + 1)\n        log($)\n");
    if let Decl::Fn(f) = &p.decls[0] {
        assert_eq!(f.body.len(), 2, "expected bind + while");
        let ph = if let Stmt::Bind(b) = &f.body[0] {
            assert!(matches!(b.value, Expr::Int(0, _)));
            b.name.as_str().to_string()
        } else {
            panic!("expected bind, got {:?}", f.body[0]);
        };
        if let Stmt::While(w) = &f.body[1] {
            // cond: ph < 3
            if let Expr::BinOp(l, _, _, _) = &w.cond {
                if let Expr::Ident(n, _) = l.as_ref() {
                    assert_eq!(n.as_str(), ph);
                } else {
                    panic!("cond lhs not ident");
                }
            } else {
                panic!("cond not binop");
            }
            // body: [log(ph), assign ph = ph + 1]
            assert_eq!(w.body.len(), 2);
            assert!(matches!(w.body[1], Stmt::Assign(..)));
        } else {
            panic!("expected while");
        }
    }
}

#[test]
fn c_style_loop_paren_less_desugars() {
    // Same desugar as the paren'd form but without the parens.
    let p = parse("*main()\n    loop 0, $ < 3, $ + 1\n        log($)\n");
    if let Decl::Fn(f) = &p.decls[0] {
        assert_eq!(f.body.len(), 2, "expected bind + while");
        let ph = if let Stmt::Bind(b) = &f.body[0] {
            assert!(matches!(b.value, Expr::Int(0, _)));
            b.name.as_str().to_string()
        } else {
            panic!("expected bind");
        };
        if let Stmt::While(w) = &f.body[1] {
            if let Expr::BinOp(l, _, _, _) = &w.cond {
                if let Expr::Ident(n, _) = l.as_ref() {
                    assert_eq!(n.as_str(), ph);
                }
            }
            assert_eq!(w.body.len(), 2);
            assert!(matches!(w.body[1], Stmt::Assign(..)));
        } else {
            panic!("expected while");
        }
    } else {
        panic!("expected fn decl");
    }
}

#[test]
fn loop_iter_with_index_placeholder() {
    // `loop arr` with `$$` in body should produce For with bind2 set.
    let p = parse("*main()\n    arr is [1, 2]\n    loop arr\n        log($$)\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::For(fo) = &f.body[1] {
            assert!(fo.bind2.is_some(), "expected index bind");
        } else {
            panic!("expected for, got {:?}", f.body[1]);
        }
    }
}

#[test]
fn loop_break() {
    let p = parse("*main()\n    loop\n        break\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Loop(l) = &f.body[0] {
            assert!(matches!(l.body[0], Stmt::Break(_, _)));
        } else {
            panic!("expected loop");
        }
    }
}

#[test]
fn return_stmt() {
    let p = parse("*foo()\n    return 42\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Ret(Some(Expr::Int(42, _)), _) = &f.body[0] {
        } else {
            panic!("expected return 42");
        }
    }
}

#[test]
fn match_stmt() {
    let p = parse("*main()\n    match x\n        1 ? log(1)\n        _ ? log(0)\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Match(m) = &f.body[0] {
            assert_eq!(m.arms.len(), 2);
            assert!(matches!(m.arms[1].pat, Pat::Wild(_)));
        } else {
            panic!("expected match");
        }
    }
}

#[test]
fn unary_ops() {
    let p = parse("*main()\n    x is -42\n    y is not true\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            assert!(matches!(b.value, Expr::UnaryOp(UnaryOp::Neg, _, _)));
        }
        if let Stmt::Bind(b) = &f.body[1] {
            assert!(matches!(b.value, Expr::UnaryOp(UnaryOp::Not, _, _)));
        }
    }
}

#[test]
fn exponentiation() {
    let p = parse("*main()\n    x is 2 pow 3\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            if let Expr::BinOp(_, op, _, _) = &b.value {
                assert_eq!(*op, BinOp::Exp);
            } else {
                panic!("expected binop");
            }
        }
    }
}

#[test]
fn exp_right_assoc() {
    let p = parse("*main()\n    x is 2 pow 3 pow 4\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            if let Expr::BinOp(l, BinOp::Exp, r, _) = &b.value {
                assert!(matches!(l.as_ref(), Expr::Int(2, _)));
                assert!(matches!(r.as_ref(), Expr::BinOp(_, BinOp::Exp, _, _)));
            } else {
                panic!("expected exp");
            }
        }
    }
}

#[test]
fn as_cast() {
    let p = parse("*main()\n    x is 42 as f64\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            assert!(matches!(b.value, Expr::As(_, Type::F64, _)));
        } else {
            panic!("expected bind");
        }
    }
}

#[test]
fn array_literal() {
    // `[1,2,3]` desugars to a vector literal — `vector(1,2,3)`.
    let p = parse("*main()\n    x is [1, 2, 3]\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            if let Expr::Call(callee, args, _) = &b.value {
                if let Expr::Ident(name, _) = callee.as_ref() {
                    assert_eq!(name, "vector");
                    assert_eq!(args.len(), 3);
                    return;
                }
            }
            panic!("expected vector(...) call, got {:?}", b.value);
        }
    }
}

#[test]
fn tuple_literal() {
    let p = parse("*main()\n    x is (1, 2, 3)\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            if let Expr::Tuple(elems, _) = &b.value {
                assert_eq!(elems.len(), 3);
            } else {
                panic!("expected tuple");
            }
        }
    }
}

#[test]
fn fn_with_typed_params() {
    let p = parse("*add(a as i64, b as i64) returns i64\n    return a + b\n");
    if let Decl::Fn(f) = &p.decls[0] {
        assert_eq!(f.params[0].ty, Some(Type::I64));
        assert_eq!(f.params[1].ty, Some(Type::I64));
        assert_eq!(f.ret, Some(Type::I64));
    }
}

#[test]
fn elif_chain() {
    let p = parse(
        "*main()\n    if true\n        log(1)\n    elif false\n        log(2)\n    elif true\n        log(3)\n    else\n        log(4)\n",
    );
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::If(i) = &f.body[0] {
            assert_eq!(i.elifs.len(), 2);
            assert!(i.els.is_some());
        } else {
            panic!("expected if");
        }
    }
}

#[test]
fn multiple_fns() {
    let p = parse("*foo()\n    return 1\n\n*bar()\n    return 2\n\n*main()\n    log(foo())\n");
    assert_eq!(p.decls.len(), 3);
}

#[test]
fn continue_stmt() {
    let p = parse("*main()\n    while true\n        continue\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::While(w) = &f.body[0] {
            assert!(matches!(w.body[0], Stmt::Continue(_)));
        }
    }
}

#[test]
fn method_call() {
    let p = parse("*main()\n    x.foo(1, 2)\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Expr(Expr::Method(_, name, args, _)) = &f.body[0] {
            assert_eq!(name, "foo");
            assert_eq!(args.len(), 2);
        } else {
            panic!("expected method call");
        }
    }
}

#[test]
fn field_access() {
    let p = parse("*main()\n    x.y\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Expr(Expr::Field(_, field, _)) = &f.body[0] {
            assert_eq!(field, "y");
        } else {
            panic!("expected field access");
        }
    }
}

#[test]
fn struct_construction() {
    let p = parse("*main()\n    Point(x is 1, y is 2)\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Expr(Expr::Struct(name, fields, _)) = &f.body[0] {
            assert_eq!(name, "Point");
            assert_eq!(fields.len(), 2);
        } else {
            panic!("expected struct construction");
        }
    }
}

#[test]
fn index_expr() {
    let p = parse("*main()\n    x[0]\n");
    if let Decl::Fn(f) = &p.decls[0] {
        assert!(matches!(f.body[0], Stmt::Expr(Expr::Index(_, _, _))));
    }
}

#[test]
fn pipeline_simple() {
    let p = parse("*double(x as i64) returns i64\n    x * 2\n\n*main()\n    x is 10 ~ double\n");
    if let Decl::Fn(f) = &p.decls[1] {
        if let Stmt::Bind(b) = &f.body[0] {
            assert!(matches!(b.value, Expr::Pipe(_, _, _, _)));
        } else {
            panic!("expected bind with pipe");
        }
    }
}

#[test]
fn pipeline_chain() {
    let p = parse(
        "*a(x as i64) returns i64\n    x\n\n*b(x as i64) returns i64\n    x\n\n*main()\n    x is 1 ~ a ~ b\n",
    );
    if let Decl::Fn(f) = &p.decls[2] {
        if let Stmt::Bind(b) = &f.body[0] {
            if let Expr::Pipe(left, _, _, _) = &b.value {
                assert!(matches!(left.as_ref(), Expr::Pipe(_, _, _, _)));
            } else {
                panic!("expected chained pipe");
            }
        }
    }
}

#[test]
fn pipeline_with_call() {
    let p =
        parse("*add(a as i64, b as i64) returns i64\n    a + b\n\n*main()\n    x is 10 ~ add(5)\n");
    if let Decl::Fn(f) = &p.decls[1] {
        if let Stmt::Bind(b) = &f.body[0] {
            if let Expr::Pipe(_, right, _, _) = &b.value {
                assert!(matches!(right.as_ref(), Expr::Call(_, _, _)));
            } else {
                panic!("expected pipe with call");
            }
        }
    }
}

#[test]
fn placeholder_in_call() {
    let p = parse(
        "*mul(a as i64, b as i64) returns i64\n    a * b\n\n*main()\n    x is 10 ~ mul($, 3)\n",
    );
    if let Decl::Fn(f) = &p.decls[1] {
        if let Stmt::Bind(b) = &f.body[0] {
            if let Expr::Pipe(_, right, _, _) = &b.value {
                if let Expr::Call(_, args, _) = right.as_ref() {
                    assert!(matches!(args[0], Expr::Placeholder(_)));
                } else {
                    panic!("expected call with placeholder");
                }
            }
        }
    }
}

#[test]
fn lambda_expr() {
    let p = parse("*main()\n    f is |x as i64| returns i64 x * 2\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            if let Expr::Lambda(params, ret, _, _) = &b.value {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].name, "x");
                assert_eq!(*ret, Some(Type::I64));
            } else {
                panic!("expected lambda");
            }
        }
    }
}

#[test]
fn lambda_multi_param() {
    let p = parse("*main()\n    f is |a as i64, b as i64| returns i64 a + b\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            if let Expr::Lambda(params, _, _, _) = &b.value {
                assert_eq!(params.len(), 2);
            } else {
                panic!("expected lambda");
            }
        }
    }
}

#[test]
fn fn_type_annotation() {
    let p = parse("*apply(f as (i64) returns i64, x as i64) returns i64\n    f(x)\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Some(Type::Fn(ptys, ret)) = &f.params[0].ty {
            assert_eq!(ptys.len(), 1);
            assert_eq!(ptys[0], Type::I64);
            assert_eq!(**ret, Type::I64);
        } else {
            panic!("expected fn type on param, got: {:?}", f.params[0].ty);
        }
    }
}

#[test]
fn inline_body_parens() {
    let p = parse("*double(x) is x * 2\n");
    if let Decl::Fn(f) = &p.decls[0] {
        assert_eq!(f.name, "double");
        assert_eq!(f.params.len(), 1);
        assert_eq!(f.params[0].name, "x");
        assert_eq!(f.body.len(), 1);
        assert!(matches!(
            f.body[0],
            Stmt::Expr(Expr::BinOp(_, BinOp::Mul, _, _))
        ));
    } else {
        panic!("expected fn");
    }
}

#[test]
fn inline_body_paren_free() {
    let p = parse("*add a, b is a + b\n");
    if let Decl::Fn(f) = &p.decls[0] {
        assert_eq!(f.name, "add");
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name, "a");
        assert_eq!(f.params[1].name, "b");
        assert_eq!(f.body.len(), 1);
        assert!(matches!(
            f.body[0],
            Stmt::Expr(Expr::BinOp(_, BinOp::Add, _, _))
        ));
    } else {
        panic!("expected fn");
    }
}

#[test]
fn literal_param_single() {
    let p = parse("*fib(0) is 0\n\n*fib(n)\n    n\n");
    assert_eq!(p.decls.len(), 1);
    if let Decl::Fn(f) = &p.decls[0] {
        assert_eq!(f.name, "fib");
        assert_eq!(f.params.len(), 1);
        assert!(f.params[0].literal.is_none());
        assert_eq!(f.body.len(), 1);
        assert!(matches!(f.body[0], Stmt::Expr(Expr::IfExpr(_))));
    } else {
        panic!("expected fn");
    }
}

#[test]
fn literal_param_multi_clause() {
    let p = parse("*fib(0) is 0\n\n*fib(1) is 1\n\n*fib(n)\n    fib(n - 1) + fib(n - 2)\n");
    assert_eq!(p.decls.len(), 1);
    if let Decl::Fn(f) = &p.decls[0] {
        assert_eq!(f.name, "fib");
        if let Stmt::Expr(Expr::IfExpr(ref i)) = f.body[0] {
            assert_eq!(i.elifs.len(), 1);
            assert!(i.els.is_some());
        } else {
            panic!("expected desugared if-expr");
        }
    }
}

#[test]
fn query_block() {
    let p = parse("*main()\n    x is Users query\n        where age >= 18\n        limit 10\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            if let Expr::Query(source, clauses, _) = &b.value {
                assert!(matches!(source.as_ref(), Expr::Ident(n, _) if n == "Users"));
                assert_eq!(clauses.len(), 2);
                assert!(matches!(clauses[0], QueryClause::Where(_, _)));
                assert!(matches!(clauses[1], QueryClause::Limit(_, _)));
            } else {
                panic!("expected query expr, got: {:?}", b.value);
            }
        } else {
            panic!("expected bind");
        }
    }
}

#[test]
fn query_sort_clause() {
    let p = parse("*main()\n    x is Items query\n        sort name desc\n");
    if let Decl::Fn(f) = &p.decls[0] {
        if let Stmt::Bind(b) = &f.body[0] {
            if let Expr::Query(_, clauses, _) = &b.value {
                if let QueryClause::Sort(field, asc, _) = &clauses[0] {
                    assert_eq!(field, "name");
                    assert!(!asc);
                } else {
                    panic!("expected sort clause");
                }
            }
        }
    }
}
