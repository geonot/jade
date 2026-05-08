use crate::ast::*;
use crate::lexer::Token;
use crate::types::Type;

use super::super::{ParseError, Parser};
use super::yield_scan::body_contains_yield;

impl Parser {
    pub(in crate::parser) fn parse_fn_attrs(&mut self) -> Result<FnAttrs, ParseError> {
        let mut attrs = FnAttrs::default();
        while self.check(Token::At) {
            self.advance();
            let attr = self.ident()?;
            if attr == "inline" {
                attrs.inline = true;
            } else if attr == "noinline" {
                attrs.noinline = true;
            } else if attr == "cold" {
                attrs.cold = true;
            } else if attr == "hot" {
                attrs.hot = true;
            } else {
                return Err(self.error(&format!("unknown function attribute: @{attr}")));
            }
        }
        Ok(attrs)
    }

    pub(in crate::parser) fn parse_type_params(
        &mut self,
    ) -> (Vec<Symbol>, Vec<(Symbol, Vec<Symbol>)>) {
        let mut tp = Vec::new();
        let mut bounds = Vec::new();
        if !self.check(Token::Of) {
            return (tp, bounds);
        }
        self.advance();
        while let Token::Ident(_) = self.peek() {
            let name = self.ident().unwrap();
            if self.check(Token::Colon) {
                self.advance();
                let mut bs = Vec::new();
                match self.ident() {
                    Ok(b) => bs.push(b),
                    Err(_) => break,
                }
                while self.check(Token::Plus) {
                    self.advance();
                    match self.ident() {
                        Ok(b) => bs.push(b),
                        Err(_) => break,
                    }
                }
                bounds.push((name, bs));
            }
            tp.push(name);
            if !self.check(Token::Comma) {
                break;
            }
            self.advance();
        }
        (tp, bounds)
    }

    pub(in crate::parser) fn parse_extern(&mut self) -> Result<ExternFn, ParseError> {
        let sp = self.span();
        self.expect(Token::Extern)?;
        self.expect(Token::Star)?;
        let name = self.ident()?;
        self.expect(Token::LParen)?;
        let mut params = Vec::new();
        let mut variadic = false;
        while !self.check(Token::RParen) && !self.eof() {
            if self.check(Token::Dot) {
                self.advance();
                self.expect(Token::Dot)?;
                self.expect(Token::Dot)?;
                variadic = true;
                break;
            }
            let pname = self.ident()?;
            self.expect(Token::As)?;
            let pty = self.parse_type()?;
            params.push((pname, pty));
            if !self.check(Token::RParen) && !variadic {
                self.expect(Token::Comma)?;
            }
        }
        self.expect(Token::RParen)?;
        let ret = if self.check(Token::Returns) {
            self.advance();
            self.parse_type()?
        } else {
            Type::Void
        };
        self.skip_nl();
        Ok(ExternFn {
            name,
            params,
            ret,
            variadic,
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_fn(&mut self) -> Result<Fn, ParseError> {
        let sp = self.span();
        self.expect(Token::Star)?;
        let name = self.ident()?;
        let (type_params, type_bounds) = self.parse_type_params();
        let mut params = Vec::new();

        if self.check(Token::LParen) {
            self.advance();
            while !self.check(Token::RParen) && !self.eof() {
                params.push(self.parse_fn_param(params.len(), true)?);
                if !self.check(Token::RParen) {
                    self.expect(Token::Comma)?;
                }
            }
            self.expect(Token::RParen)?;
        } else {
            while !self.check(Token::Newline)
                && !self.check(Token::Returns)
                && !self.check(Token::Is)
                && !self.eof()
            {
                params.push(self.parse_fn_param(params.len(), false)?);
                if self.check(Token::Comma) {
                    self.advance();
                }
            }
        }

        let ret = if self.check(Token::Returns) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        // Optional error union: `returns T ! E1 ! E2 ...` or `! E1 ! E2 ...`
        // Each `! Ident` after the return type names an err-type that this
        // function may early-return via `! Variant`.
        let mut error_types = Vec::new();
        while self.check(Token::Bang) {
            self.advance();
            error_types.push(self.parse_type()?);
        }

        let body = self.parse_body()?;
        let is_generator = body_contains_yield(&body);

        Ok(Fn {
            name,
            type_params,
            type_bounds,
            params,
            ret,
            error_types,
            body,
            is_generator,
            attrs: FnAttrs::default(),
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_fn_param(
        &mut self,
        idx: usize,
        typed: bool,
    ) -> Result<Param, ParseError> {
        match self.peek() {
            Token::Int(_)
            | Token::CharLit(_)
            | Token::Float(_)
            | Token::True
            | Token::False
            | Token::Str(_) => {
                let lit_sp = self.span();
                let lit_expr = self.parse_literal_token()?;
                Ok(Param {
                    name: Symbol::intern(&format!("__arg{idx}")),
                    ty: None,
                    default: None,
                    literal: Some(lit_expr),
                    span: lit_sp,
                })
            }
            Token::Minus => {
                let lit_sp = self.span();
                let lit_expr = self.parse_unary()?;
                Ok(Param {
                    name: Symbol::intern(&format!("__arg{idx}")),
                    ty: None,
                    default: None,
                    literal: Some(lit_expr),
                    span: lit_sp,
                })
            }
            _ => self.parse_param(typed),
        }
    }

    pub(in crate::parser) fn parse_param(&mut self, typed: bool) -> Result<Param, ParseError> {
        let sp = self.span();
        let name = self.ident()?;
        let ty = if typed && self.check(Token::As) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        let default = if typed && self.check(Token::Is) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Param {
            name,
            ty,
            default,
            literal: None,
            span: sp,
        })
    }
}
