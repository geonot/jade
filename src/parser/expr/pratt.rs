use super::placeholder::{contains_placeholder, replace_placeholder};
use super::{ParseError, Parser};
use crate::ast::*;
use crate::lexer::Token;

impl Parser {
    pub(in crate::parser) fn parse_ternary(&mut self) -> Result<Expr, ParseError> {
        let e = self.parse_pipeline()?;
        if self.check(Token::Question) {
            let sp = self.span();
            self.advance();

            if self.check(Token::Bang) {
                self.advance();
                let f = self.parse_pipeline()?;
                Ok(Expr::Ternary(
                    Box::new(e),
                    Box::new(Expr::Void(sp)),
                    Box::new(f),
                    sp,
                ))
            } else {
                let t = self.parse_pipeline()?;
                if self.check(Token::Bang) {
                    self.advance();
                    Ok(Expr::Ternary(
                        Box::new(e),
                        Box::new(t),
                        Box::new(self.parse_expr()?),
                        sp,
                    ))
                } else {
                    Ok(Expr::Ternary(
                        Box::new(e),
                        Box::new(t),
                        Box::new(Expr::Void(sp)),
                        sp,
                    ))
                }
            }
        } else if self.check(Token::Bang) && !self.suppress_bang_else {
            let sp = self.span();
            self.advance();
            let f = self.parse_pipeline()?;
            Ok(Expr::Ternary(
                Box::new(e),
                Box::new(Expr::Void(sp)),
                Box::new(f),
                sp,
            ))
        } else {
            Ok(e)
        }
    }

    pub(in crate::parser) fn parse_pipeline(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_or()?;
        while self.check(Token::Tilde) {
            let sp = self.span();
            self.advance();
            let rhs = self.parse_or()?;

            let rhs = if contains_placeholder(&rhs) && !matches!(rhs, Expr::Placeholder(_)) {
                let is_named_call_with_ph = matches!(&rhs, Expr::Call(callee, _, _) if matches!(callee.as_ref(), Expr::Ident(_, _)));
                if is_named_call_with_ph {
                    rhs
                } else {
                    let replaced = replace_placeholder(&rhs, "__ph");
                    Expr::Lambda(
                        vec![Param {
                            name: "__ph".into(),
                            ty: None,
                            default: None,
                            literal: None,
                            access_mod: None,
                            span: sp,
                        }],
                        None,
                        vec![Stmt::Expr(replaced)],
                        sp,
                    )
                }
            } else {
                rhs
            };
            e = Expr::Pipe(Box::new(e), Box::new(rhs), vec![], sp);
        }
        Ok(e)
    }

    binop!(parse_or,     parse_xor,    { Token::Or => BinOp::Or });
    binop!(parse_xor,    parse_and,    { Token::Xor => BinOp::BitXor });
    binop!(parse_and,    parse_eq,     { Token::And => BinOp::And });
    pub(in crate::parser) fn parse_eq(&mut self) -> Result<Expr, ParseError> {
        let mut l = self.parse_cmp()?;
        loop {
            let sp = self.span();
            match self.peek() {
                Token::Equals => {
                    self.advance();
                    let r = self.parse_cmp()?;
                    l = Expr::BinOp(Box::new(l), BinOp::Eq, Box::new(r), sp);
                }
                Token::Neq => {
                    self.advance();
                    let r = self.parse_cmp()?;
                    l = Expr::BinOp(Box::new(l), BinOp::Ne, Box::new(r), sp);
                }
                Token::Not if matches!(self.peek_at(1), Token::Equals) => {
                    self.advance();
                    self.advance();
                    let r = self.parse_cmp()?;
                    l = Expr::BinOp(Box::new(l), BinOp::Ne, Box::new(r), sp);
                }
                _ => break,
            }
        }
        Ok(l)
    }

    pub(in crate::parser) fn parse_cmp(&mut self) -> Result<Expr, ParseError> {
        let mut l = self.parse_bitor()?;

        let mut prev_right: Option<Expr> = None;
        loop {
            let sp = self.span();
            let op = match self.peek() {
                Token::Lt => Some(BinOp::Lt),
                Token::Gt => Some(BinOp::Gt),
                Token::LtEq => Some(BinOp::Le),
                Token::GtEq => Some(BinOp::Ge),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let r = self.parse_bitor()?;
                if let Some(pr) = prev_right.take() {
                    let right = Expr::BinOp(Box::new(pr), op, Box::new(r.clone()), sp);
                    l = Expr::BinOp(Box::new(l), BinOp::And, Box::new(right), sp);
                } else {
                    l = Expr::BinOp(Box::new(l), op, Box::new(r.clone()), sp);
                }
                prev_right = Some(r);
                continue;
            }
            match self.peek() {
                Token::In => {
                    self.advance();
                    let r = self.parse_bitor()?;
                    l = Expr::Method(Box::new(r), "contains".into(), vec![l], sp);
                }
                _ => break,
            }
        }
        Ok(l)
    }

    binop!(parse_bitor,  parse_bitxor, { Token::Pipe => BinOp::BitOr });
    binop!(parse_bitxor, parse_bitand, { Token::Caret => BinOp::BitXor });
    binop!(parse_bitand, parse_shift,  { Token::Ampersand => BinOp::BitAnd });
    binop!(parse_shift,  parse_add,    { Token::Shl => BinOp::Shl, Token::Shr => BinOp::Shr, Token::Ushr => BinOp::Ushr });
    binop!(parse_add,    parse_mul,    { Token::Plus => BinOp::Add, Token::Minus => BinOp::Sub });
    binop!(parse_mul,    parse_exp,    { Token::Star => BinOp::Mul, Token::Slash => BinOp::Div, Token::Percent => BinOp::Mod });

    pub(in crate::parser) fn parse_exp(&mut self) -> Result<Expr, ParseError> {
        let l = self.parse_unary()?;
        if self.check(Token::StarStar) {
            let sp = self.span();
            self.advance();
            let r = self.parse_exp()?;
            Ok(Expr::BinOp(Box::new(l), BinOp::Exp, Box::new(r), sp))
        } else {
            Ok(l)
        }
    }

    pub(in crate::parser) fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        let sp = self.span();
        match self.peek() {
            Token::Minus => {
                self.advance();
                Ok(Expr::UnaryOp(
                    UnaryOp::Neg,
                    Box::new(self.parse_unary()?),
                    sp,
                ))
            }
            Token::Not => {
                self.advance();
                Ok(Expr::UnaryOp(
                    UnaryOp::Not,
                    Box::new(self.parse_unary()?),
                    sp,
                ))
            }
            Token::Tilde => {
                self.advance();
                Ok(Expr::UnaryOp(
                    UnaryOp::BitNot,
                    Box::new(self.parse_unary()?),
                    sp,
                ))
            }
            Token::At => {
                self.advance();
                Ok(Expr::Deref(Box::new(self.parse_unary()?), sp))
            }
            _ => self.parse_postfix(),
        }
    }

    pub(in crate::parser) fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_primary()?;
        loop {
            match self.peek() {
                Token::Dot => {
                    let sp = self.span();
                    self.advance();
                    let f = self.ident()?;
                    if self.check(Token::LParen) {
                        self.advance();
                        let a = self.parse_args()?;
                        self.expect(Token::RParen)?;

                        let is_hof = f.with_str(|s| {
                            matches!(
                                s,
                                "map"
                                    | "filter"
                                    | "fold"
                                    | "reduce"
                                    | "each"
                                    | "for_each"
                                    | "any"
                                    | "all"
                                    | "find"
                                    | "count_by"
                                    | "group_by"
                                    | "sort_by"
                                    | "min_by"
                                    | "max_by"
                                    | "take_while"
                                    | "drop_while"
                                    | "flat_map"
                                    | "partition"
                            )
                        });
                        let a: Vec<Expr> = a
                            .into_iter()
                            .map(|arg| {
                                if is_hof
                                    && super::placeholder::contains_lambda_placeholder(&arg)
                                    && !matches!(arg, Expr::Placeholder(_))
                                    && !matches!(arg, Expr::Lambda(..))
                                {
                                    let asp = arg.span();
                                    let replaced =
                                        super::placeholder::replace_placeholder(&arg, "__ph");
                                    Expr::Lambda(
                                        vec![Param {
                                            name: "__ph".into(),
                                            ty: None,
                                            default: None,
                                            literal: None,
                                            access_mod: None,
                                            span: asp,
                                        }],
                                        None,
                                        vec![Stmt::Expr(replaced)],
                                        asp,
                                    )
                                } else {
                                    arg
                                }
                            })
                            .collect();
                        e = Expr::Method(Box::new(e), f, a, sp);
                    } else {
                        e = Expr::Field(Box::new(e), f, sp);
                    }
                }
                Token::LBracket => {
                    let sp = self.span();
                    self.advance();
                    let idx = self.parse_expr()?;
                    self.expect(Token::RBracket)?;
                    e = Expr::Index(Box::new(e), Box::new(idx), sp);
                }
                Token::AtKw => {
                    let sp = self.span();
                    self.advance();
                    let idx = self.parse_unary()?;
                    e = Expr::Index(Box::new(e), Box::new(idx), sp);
                }
                Token::LParen => {
                    let sp = self.span();
                    self.advance();
                    let a = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    e = Expr::Call(Box::new(e), a, sp);
                }
                Token::As => {
                    let sp = self.span();
                    self.advance();

                    if self.check(Token::Strict) {
                        self.advance();
                        e = Expr::StrictCast(Box::new(e), self.parse_type()?, sp);
                    } else if matches!(self.peek(), Token::Ident(s) if s == "json" || s == "map") {
                        if let Token::Ident(fmt) = self.peek() {
                            let fmt = fmt.clone();
                            self.advance();
                            e = Expr::AsFormat(Box::new(e), fmt, sp);
                        }
                    } else {
                        e = Expr::As(Box::new(e), self.parse_type()?, sp);
                    }
                }
                Token::From => {
                    let sp = self.span();
                    self.advance();
                    let start = self.parse_cmp()?;
                    self.expect(Token::To)?;
                    let end = self.parse_cmp()?;
                    e = Expr::Slice(Box::new(e), Box::new(start), Box::new(end), sp);
                }
                Token::Ident(kw) if kw == "where" => {
                    if let Expr::Ident(ref store_name, sp) = e {
                        let store = store_name.clone();
                        let filter = self.parse_store_filter()?;
                        e = Expr::StoreQuery(store, Box::new(filter), sp);
                    } else {
                        break;
                    }
                }
                Token::Of if matches!(e, Expr::Ident(_, _) | Expr::Lambda(..)) => {
                    let sp = self.span();
                    self.advance();
                    let arg = self.parse_primary()?;
                    e = Expr::OfCall(Box::new(e), Box::new(arg), sp);
                }
                _ => break,
            }
        }
        Ok(e)
    }
}
