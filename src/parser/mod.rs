use crate::ast::*;
use crate::lexer::{Spanned, Token};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("line {line}:{col}: {msg}")]
    Error { line: u32, col: u32, msg: String },
}

pub struct Parser {
    tok: Vec<Spanned>,
    pos: usize,
}

macro_rules! binop {
    ($name:ident, $next:ident, { $($t:path => $op:expr),+ $(,)? }) => {
        pub(super) fn $name(&mut self) -> Result<Expr, ParseError> {
            let mut l = self.$next()?;
            loop { let sp = self.span(); match self.peek() {
                $($t => { self.advance(); let r = self.$next()?;
                    l = Expr::BinOp(Box::new(l), $op, Box::new(r), sp); })+
                _ => break,
            }} Ok(l)
        }
    };
}

mod decl;
mod expr;
mod stmt;

impl Parser {
    pub fn new(tok: Vec<Spanned>) -> Self {
        Self { tok, pos: 0 }
    }

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut decls = Vec::new();
        while !self.eof() {
            self.skip_nl();
            if self.eof() {
                break;
            }
            decls.push(self.parse_decl()?);
        }
        let mut prog = Program { decls };
        desugar_multi_clause_fns(&mut prog);
        Ok(prog)
    }

    fn peek(&self) -> Token {
        if self.pos < self.tok.len() {
            self.tok[self.pos].token.clone()
        } else {
            Token::Eof
        }
    }

    fn check(&self, t: Token) -> bool {
        if self.pos < self.tok.len() {
            std::mem::discriminant(&self.tok[self.pos].token) == std::mem::discriminant(&t)
        } else {
            matches!(t, Token::Eof)
        }
    }

    fn advance(&mut self) {
        if self.pos < self.tok.len() {
            self.pos += 1;
        }
    }

    fn span(&self) -> Span {
        if self.pos < self.tok.len() {
            self.tok[self.pos].span
        } else {
            Span::dummy()
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.tok.len() || matches!(self.tok[self.pos].token, Token::Eof)
    }

    fn skip_nl(&mut self) {
        while self.check(Token::Newline) {
            self.advance();
        }
    }

    fn expect(&mut self, t: Token) -> Result<(), ParseError> {
        if self.check(Token::Eof) && !matches!(t, Token::Eof) {
            return Err(self.error(&format!("expected {t}, got EOF")));
        }
        if self.pos < self.tok.len()
            && std::mem::discriminant(&self.tok[self.pos].token) == std::mem::discriminant(&t)
        {
            self.advance();
            Ok(())
        } else {
            Err(self.error(&format!("expected {t}, got {}", self.peek())))
        }
    }

    fn ident(&mut self) -> Result<String, ParseError> {
        if self.pos < self.tok.len() {
            match &self.tok[self.pos].token {
                Token::Ident(n) => {
                    let name = n.clone();
                    self.advance();
                    return Ok(name);
                }
                Token::Set => {
                    self.advance();
                    return Ok("set".into());
                }
                _ => {}
            }
        }
        Err(self.error(&format!("expected identifier, got {}", self.peek())))
    }

    fn parse_body(&mut self) -> Result<Block, ParseError> {
        if self.check(Token::Is) {
            self.advance();
            Ok(vec![Stmt::Expr(self.parse_expr()?)])
        } else {
            self.expect(Token::Newline)?;
            self.parse_block()
        }
    }

    fn parse_indented<T>(
        &mut self,
        mut f: impl FnMut(&mut Self) -> Result<T, ParseError>,
    ) -> Result<Vec<T>, ParseError> {
        self.expect(Token::Indent)?;
        let mut items = Vec::new();
        while !self.check(Token::Dedent) && !self.eof() {
            self.skip_nl();
            if self.check(Token::Dedent) || self.eof() {
                break;
            }
            items.push(f(self)?);
            self.skip_nl();
        }
        if self.check(Token::Dedent) {
            self.advance();
        }
        Ok(items)
    }

    fn error(&self, msg: &str) -> ParseError {
        let sp = self.span();
        ParseError::Error {
            line: sp.line,
            col: sp.col,
            msg: msg.into(),
        }
    }
}

fn desugar_multi_clause_fns(prog: &mut Program) {
    let mut name_indices: Vec<(String, Vec<usize>)> = Vec::new();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (i, decl) in prog.decls.iter().enumerate() {
        if let Decl::Fn(f) = decl {
            if let Some(&group_idx) = seen.get(&f.name) {
                name_indices[group_idx].1.push(i);
            } else {
                seen.insert(f.name.clone(), name_indices.len());
                name_indices.push((f.name.clone(), vec![i]));
            }
        }
    }

    let multi_groups: Vec<(String, Vec<usize>)> = name_indices
        .into_iter()
        .filter(|(_, indices)| indices.len() > 1)
        .collect();

    if multi_groups.is_empty() {
        return;
    }

    let mut to_remove: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (_, indices) in &multi_groups {
        let clauses: Vec<Fn> = indices
            .iter()
            .map(|&i| {
                if let Decl::Fn(f) = &prog.decls[i] {
                    f.clone()
                } else {
                    unreachable!()
                }
            })
            .collect();

        let merged = merge_fn_clauses(&clauses);

        prog.decls[indices[0]] = Decl::Fn(merged);
        for &i in &indices[1..] {
            to_remove.insert(i);
        }
    }

    let mut remove_sorted: Vec<usize> = to_remove.into_iter().collect();
    remove_sorted.sort_unstable_by(|a, b| b.cmp(a));
    for i in remove_sorted {
        prog.decls.remove(i);
    }
}

fn merge_fn_clauses(clauses: &[Fn]) -> Fn {
    let first = &clauses[0];
    let param_count = first.params.len();
    let sp = first.span;

    let mut unified_params: Vec<Param> = Vec::new();
    for pi in 0..param_count {
        let real_name = clauses
            .iter()
            .find_map(|c| {
                c.params.get(pi).and_then(|p| {
                    if p.literal.is_none() {
                        Some(p.name.clone())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| format!("__arg{pi}"));

        let ty = clauses
            .iter()
            .find_map(|c| c.params.get(pi).and_then(|p| p.ty.clone()));

        unified_params.push(Param {
            name: real_name,
            ty,
            default: None,
            literal: None,
            span: sp,
        });
    }

    let mut guarded: Vec<&Fn> = Vec::new();
    let mut catchall: Option<&Fn> = None;
    for c in clauses {
        if c.params.iter().any(|p| p.literal.is_some()) {
            guarded.push(c);
        } else {
            catchall = Some(c);
        }
    }

    let build_cond = |clause: &Fn| -> Expr {
        let mut conds: Vec<Expr> = Vec::new();
        for (pi, p) in clause.params.iter().enumerate() {
            if let Some(ref lit) = p.literal {
                let arg_ref = Expr::Ident(unified_params[pi].name.clone(), sp);
                conds.push(Expr::BinOp(
                    Box::new(arg_ref),
                    BinOp::Eq,
                    Box::new(lit.clone()),
                    sp,
                ));
            }
        }
        conds
            .into_iter()
            .reduce(|a, b| Expr::BinOp(Box::new(a), BinOp::And, Box::new(b), sp))
            .unwrap()
    };

    let build_body = |clause: &Fn| -> Block {
        let mut body = Vec::new();
        for (pi, p) in clause.params.iter().enumerate() {
            if p.literal.is_none() && p.name != unified_params[pi].name {
                body.push(Stmt::Bind(crate::ast::Bind {
                    name: p.name.clone(),
                    value: Expr::Ident(unified_params[pi].name.clone(), sp),
                    ty: None,
                    span: sp,
                }));
            }
        }
        body.extend(clause.body.clone());
        body
    };

    let body = if guarded.is_empty() {
        catchall.map(|c| c.body.clone()).unwrap_or_default()
    } else {
        let first_guarded = guarded[0];
        let then_cond = build_cond(first_guarded);
        let then_body = build_body(first_guarded);
        let mut elifs: Vec<(Expr, Block)> = Vec::new();
        for g in &guarded[1..] {
            elifs.push((build_cond(g), build_body(g)));
        }
        let els = catchall.map(|c| build_body(c));

        vec![Stmt::Expr(Expr::IfExpr(Box::new(If {
            cond: then_cond,
            then: then_body,
            elifs,
            els,
            span: sp,
        })))]
    };

    Fn {
        name: first.name.clone(),
        type_params: first.type_params.clone(),
        type_bounds: first.type_bounds.clone(),
        params: unified_params,
        ret: first.ret.clone(),
        body,
        is_generator: false,
        span: sp,
    }
}

#[cfg(test)]
mod tests {
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
        let p = parse("*main()\n    x is 2 ** 3\n");
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
        let p = parse("*main()\n    x is 2 ** 3 ** 4\n");
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
        let p = parse("*main()\n    x is [1, 2, 3]\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::Array(elems, _) = &b.value {
                    assert_eq!(elems.len(), 3);
                } else {
                    panic!("expected array");
                }
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
        let p = parse("*add(a: i64, b: i64) -> i64\n    return a + b\n");
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
        let p = parse("*double(x: i64) -> i64\n    x * 2\n\n*main()\n    x is 10 ~ double\n");
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
            "*a(x: i64) -> i64\n    x\n\n*b(x: i64) -> i64\n    x\n\n*main()\n    x is 1 ~ a ~ b\n",
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
        let p = parse("*add(a: i64, b: i64) -> i64\n    a + b\n\n*main()\n    x is 10 ~ add(5)\n");
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
        let p =
            parse("*mul(a: i64, b: i64) -> i64\n    a * b\n\n*main()\n    x is 10 ~ mul($, 3)\n");
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
        let p = parse("*main()\n    f is *fn(x: i64) -> i64 x * 2\n");
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
        let p = parse("*main()\n    f is *fn(a: i64, b: i64) -> i64 a + b\n");
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
        let p = parse("*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n");
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
}
