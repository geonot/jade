use super::{ParseError, Parser};
use crate::ast::*;
use crate::lexer::Token;
use crate::types::Type;

impl Parser {
    pub(in crate::parser) fn parse_literal_token(&mut self) -> Result<Expr, ParseError> {
        let sp = self.span();
        match self.peek() {
            Token::Int(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Int(n, sp))
            }
            Token::CharLit(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Int(n, sp))
            }
            Token::Float(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Float(n, sp))
            }
            Token::True => {
                self.advance();
                Ok(Expr::Bool(true, sp))
            }
            Token::False => {
                self.advance();
                Ok(Expr::Bool(false, sp))
            }
            Token::Str(s) => {
                let v = s.clone();
                self.advance();
                Ok(Expr::Str(v, sp))
            }
            _ => Err(self.error("expected literal")),
        }
    }

    pub(in crate::parser) fn parse_query_block(
        &mut self,
        source: Expr,
    ) -> Result<Expr, ParseError> {
        let sp = source.span();
        self.expect(Token::Query)?;
        self.expect(Token::Newline)?;
        let clauses = self.parse_indented(Self::parse_query_clause)?;
        Ok(Expr::Query(Box::new(source), clauses, sp))
    }

    pub(crate) fn parse_query_clause(&mut self) -> Result<QueryClause, ParseError> {
        let sp = self.span();

        if self.check(Token::Delete) {
            self.advance();
            return Ok(QueryClause::Delete(sp));
        }
        let kw = self.ident()?;
        match &*kw.as_str() {
            "where" => {
                let cond = self.parse_expr()?;
                Ok(QueryClause::Where(cond, sp))
            }
            "limit" => {
                let n = self.parse_expr()?;
                Ok(QueryClause::Limit(n, sp))
            }
            "sort" => {
                let field = self.ident()?;
                let asc = if let Token::Ident(dir) = self.peek() {
                    if dir == "desc" {
                        self.advance();
                        false
                    } else if dir == "asc" {
                        self.advance();
                        true
                    } else {
                        true
                    }
                } else {
                    true
                };
                Ok(QueryClause::Sort(field, asc, sp))
            }
            "take" => {
                let n = self.parse_expr()?;
                Ok(QueryClause::Take(n, sp))
            }
            "skip" => {
                let n = self.parse_expr()?;
                Ok(QueryClause::Skip(n, sp))
            }
            "set" => {
                let field = self.ident()?;
                self.expect(Token::Is)?;
                let val = self.parse_expr()?;
                Ok(QueryClause::Set(field, val, sp))
            }
            _ => Err(self.error(&format!("unknown query clause: {kw}"))),
        }
    }

    pub(in crate::parser) fn parse_builtin_call(
        &mut self,
        name: &str,
        sp: Span,
    ) -> Result<Expr, ParseError> {
        let arg = if self.check(Token::LParen) {
            self.advance();
            let a = self.parse_expr()?;
            self.expect(Token::RParen)?;
            a
        } else {
            self.parse_expr()?
        };
        Ok(Expr::Call(
            Box::new(Expr::Ident(name.into(), sp)),
            vec![arg],
            sp,
        ))
    }

    pub(in crate::parser) fn is_field_init(&self) -> bool {
        matches!(self.peek(), Token::Ident(_))
            && self.pos + 1 < self.tok.len()
            && matches!(self.tok[self.pos + 1].token, Token::Is)
    }

    pub(in crate::parser) fn is_named_arg(&self) -> bool {
        matches!(self.peek(), Token::Ident(_))
            && self.pos + 1 < self.tok.len()
            && matches!(self.tok[self.pos + 1].token, Token::Is)
    }

    pub(in crate::parser) fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut a = Vec::new();
        while !self.check(Token::RParen) && !self.eof() {
            self.skip_ws();
            if self.check(Token::RParen) {
                break;
            }
            if self.check(Token::DotDotDot) {
                let sp = self.span();
                self.advance();
                let e = self.parse_expr()?;
                a.push(Expr::Spread(Box::new(e), sp));
            } else if self.is_named_arg() {
                let sp = self.span();
                let name = self.ident()?;
                self.expect(Token::Is)?;
                let val = self.parse_expr()?;
                a.push(Expr::NamedArg(name, Box::new(val), sp));
            } else {
                a.push(self.parse_expr()?);
            }
            self.skip_ws();
            if !self.check(Token::RParen) {
                self.expect(Token::Comma)?;
                self.skip_ws();
            }
        }
        Ok(a)
    }

    pub(in crate::parser) fn parse_type(&mut self) -> Result<Type, ParseError> {
        match self.peek() {
            Token::Percent => {
                self.advance();
                let inner = self.parse_type()?;
                Ok(Type::Ptr(Box::new(inner)))
            }
            Token::Ident(n) => {
                let t = n.with_str(|s| self.ident_to_type(s));
                self.advance();
                if self.check(Token::Of) {
                    if let Type::Struct(name, _) = t {
                        self.advance();
                        let arg = self.parse_type()?;
                        if name == "Vec" {
                            return Ok(Type::Vec(Box::new(arg)));
                        }
                        if name == "Map" {
                            return Ok(Type::Map(Box::new(Type::String), Box::new(arg)));
                        }
                        let mangled = format!("{name}_{arg}");
                        Ok(Type::Struct(mangled.into(), vec![]))
                    } else {
                        Ok(t)
                    }
                } else {
                    Ok(t)
                }
            }
            Token::LParen => {
                self.advance();
                let mut params = Vec::new();
                while !self.check(Token::RParen) && !self.eof() {
                    params.push(self.parse_type()?);
                    if !self.check(Token::RParen) {
                        self.expect(Token::Comma)?;
                    }
                }
                self.expect(Token::RParen)?;
                if self.check(Token::Returns) {
                    self.advance();
                    let ret = self.parse_type()?;
                    return Ok(Type::Fn(params, Box::new(ret)));
                }

                Ok(match params.len() {
                    0 => Type::Void,
                    1 => params.into_iter().next().unwrap(),
                    _ => Type::Tuple(params),
                })
            }
            _ => Err(self.error("expected type")),
        }
    }

    pub(in crate::parser) fn ident_to_type(&self, n: &str) -> Type {
        match n {
            "i8" => Type::I8,
            "i16" => Type::I16,
            "i32" => Type::I32,
            "int" | "i64" => Type::I64,
            "u8" => Type::U8,
            "u16" => Type::U16,
            "u32" => Type::U32,
            "u64" => Type::U64,
            "f32" => Type::F32,
            "float" | "f64" => Type::F64,
            "bool" => Type::Bool,
            "void" => Type::Void,
            "str" | "String" => Type::String,
            s if s.len() == 1 && s.chars().next().map_or(false, |c| c.is_uppercase()) => {
                Type::Param(s.into())
            }
            _ => Type::Struct(n.into(), vec![]),
        }
    }

    pub(in crate::parser) fn parse_interp(
        &mut self,
        first: String,
        sp: Span,
    ) -> Result<Expr, ParseError> {
        let mut result: Expr = Expr::Str(first, sp);
        while self.check(Token::InterpStart) {
            self.advance();
            let expr = self.parse_expr()?;
            if !self.check(Token::InterpEnd) {
                return Err(self.error("expected closing } in string interpolation"));
            }
            self.advance();
            let interp_expr = Expr::Call(
                Box::new(Expr::Ident("to_string".into(), expr.span())),
                vec![expr],
                sp,
            );
            result = Expr::BinOp(Box::new(result), BinOp::Add, Box::new(interp_expr), sp);
            if let Token::Str(s) = self.peek() {
                let tail = s.clone();
                self.advance();
                if !tail.is_empty() {
                    result = Expr::BinOp(
                        Box::new(result),
                        BinOp::Add,
                        Box::new(Expr::Str(tail, sp)),
                        sp,
                    );
                }
            }
        }
        Ok(result)
    }
}
