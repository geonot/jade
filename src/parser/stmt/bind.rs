use super::super::{ParseError, Parser};
use crate::ast::*;
use crate::lexer::Token;

impl Parser {
    pub(in crate::parser) fn parse_bind(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        let name = self.ident()?;
        if let Some(op) = self.aug_op() {
            let rhs = self.parse_expr()?;
            let rsp = rhs.span();
            return Ok(Stmt::Bind(Bind {
                name,
                value: Expr::BinOp(Box::new(Expr::Ident(name, sp)), op, Box::new(rhs), rsp),
                ty: None,
                atomic: false,
                access_mod: None,
                span: sp,
            }));
        }

        let mut declared_ty: Option<crate::types::Type> = None;
        if self.check(Token::As) {
            self.advance();
            declared_ty = Some(self.parse_type_multi()?);
        }
        self.expect(Token::Is)?;

        let access_mod = self.try_parse_access_mod_after_is();

        if self.check(Token::For) {
            self.advance();
            let bind = self.ident()?;
            let bind2 = if self.check(Token::Comma) {
                self.advance();
                Some(self.ident()?)
            } else {
                None
            };
            if self.check(Token::From) {
                self.advance();
            } else {
                self.expect(Token::In)?;
            }
            let iter = self.parse_expr()?;
            let end = if self.check(Token::To) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            let step = if self.check(Token::By) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.expect(Token::Newline)?;
            self.label_stack.push(name);
            let body = self.parse_block()?;
            self.label_stack.pop();
            return Ok(Stmt::For(For {
                label: Some(name),
                bind,
                bind2,
                iter,
                end,
                step,
                body,
                access_mod,
                span: sp,
            }));
        }

        let value = if self.check(Token::Insert) {
            self.parse_insert_expr()?
        } else {
            self.parse_pipeline()?
        };

        let value = if self.check(Token::Query) {
            self.parse_query_block(value)?
        } else {
            value
        };

        let bang_ok = !matches!(self.peek(), Token::Bang) || !self.suppress_bang_else;
        if bang_ok && matches!(self.peek(), Token::Question | Token::Bang | Token::BangBang) {
            let handled = self.parse_stmt_handler_arms(value)?;
            return Ok(Stmt::Bind(Bind {
                name,
                value: handled,
                ty: declared_ty,
                atomic: false,
                access_mod,
                span: sp,
            }));
        }

        Ok(Stmt::Bind(Bind {
            name,
            value,
            ty: declared_ty,
            atomic: false,
            access_mod,
            span: sp,
        }))
    }

    pub(in crate::parser) fn complete_expr_after_pipeline(
        &mut self,
        head: Expr,
    ) -> Result<Expr, ParseError> {
        let value = if self.check(Token::Question) {
            let qsp = self.span();
            self.advance();
            if self.check(Token::Bang) {
                self.advance();
                let f = self.parse_pipeline()?;
                Expr::Ternary(Box::new(head), Box::new(Expr::Void(qsp)), Box::new(f), qsp)
            } else {
                let t = self.parse_pipeline()?;
                if self.check(Token::Bang) {
                    self.advance();
                    let f = self.parse_expr()?;
                    Expr::Ternary(Box::new(head), Box::new(t), Box::new(f), qsp)
                } else {
                    Expr::Ternary(Box::new(head), Box::new(t), Box::new(Expr::Void(qsp)), qsp)
                }
            }
        } else if self.check(Token::Bang) && !self.suppress_bang_else {
            let bsp = self.span();
            self.advance();
            let f = self.parse_pipeline()?;
            Expr::Ternary(Box::new(head), Box::new(Expr::Void(bsp)), Box::new(f), bsp)
        } else {
            head
        };

        if self.check(Token::Query) {
            self.parse_query_block(value)
        } else {
            Ok(value)
        }
    }
}
