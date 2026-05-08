//! Parser arms for expressions including precedence climbing.

use crate::ast::*;
use crate::lexer::Token;

use super::{ParseError, Parser};

impl Parser {
    pub(in crate::parser) fn parse_pat(&mut self) -> Result<Pat, ParseError> {
        let first = self.parse_single_pat()?;
        if self.check(Token::Or) {
            let sp = first.span();
            let mut pats = vec![first];
            while self.check(Token::Or) {
                self.advance();
                pats.push(self.parse_single_pat()?);
            }
            return Ok(Pat::Or(pats, sp));
        }
        Ok(first)
    }

    pub(in crate::parser) fn parse_single_pat(&mut self) -> Result<Pat, ParseError> {
        let sp = self.span();
        match self.peek() {
            Token::Ident(s) if s == "_" => {
                self.advance();
                Ok(Pat::Wild(sp))
            }
            Token::Ident(_) => {
                let name = self.ident()?;
                if self.check(Token::LParen) {
                    self.advance();
                    let mut sub = Vec::new();
                    while !self.check(Token::RParen) && !self.eof() {
                        sub.push(self.parse_pat()?);
                        if !self.check(Token::RParen) {
                            self.expect(Token::Comma)?;
                        }
                    }
                    self.expect(Token::RParen)?;
                    Ok(Pat::Ctor(name, sub, sp))
                } else {
                    Ok(Pat::Ident(name, sp))
                }
            }
            Token::Int(_)
            | Token::CharLit(_)
            | Token::Float(_)
            | Token::Str(_)
            | Token::True
            | Token::False => {
                let lit = self.parse_primary()?;
                if self.check(Token::To) {
                    self.advance();
                    let hi = self.parse_primary()?;
                    Ok(Pat::Range(lit, hi, sp))
                } else {
                    Ok(Pat::Lit(lit))
                }
            }
            Token::LParen => {
                self.advance();
                let mut pats = Vec::new();
                while !self.check(Token::RParen) && !self.eof() {
                    pats.push(self.parse_pat()?);
                    if !self.check(Token::RParen) {
                        self.expect(Token::Comma)?;
                    }
                }
                self.expect(Token::RParen)?;
                Ok(Pat::Tuple(pats, sp))
            }
            Token::LBracket => {
                self.advance();
                let mut pats = Vec::new();
                while !self.check(Token::RBracket) && !self.eof() {
                    pats.push(self.parse_pat()?);
                    if !self.check(Token::RBracket) {
                        self.expect(Token::Comma)?;
                    }
                }
                self.expect(Token::RBracket)?;
                Ok(Pat::Array(pats, sp))
            }
            _ => Err(self.error("expected pattern")),
        }
    }

    pub(in crate::parser) fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.depth += 1;
        if self.depth > 256 {
            self.depth -= 1;
            return Err(self.error("expression nesting depth limit exceeded (256 levels)"));
        }
        let result = self.parse_expr_inner();
        self.depth -= 1;
        result
    }

    pub(in crate::parser) fn parse_expr_inner(&mut self) -> Result<Expr, ParseError> {
        let e = self.parse_ternary()?;
        if self.check(Token::Query) {
            return self.parse_query_block(e);
        }
        Ok(e)
    }
}

mod parse_primary;
mod placeholder;
mod pratt;
mod primary;

pub(super) use placeholder::*;
