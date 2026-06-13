use super::super::{ParseError, Parser};
use crate::ast::*;
use crate::lexer::Token;

impl Parser {
    pub(in crate::parser) fn parse_insert_expr(&mut self) -> Result<Expr, ParseError> {
        let sp = self.span();
        self.expect(Token::Insert)?;
        let store = self.ident()?;

        let parens = self.check(Token::LParen);
        if parens {
            self.advance();
        }
        let mut values = vec![self.parse_insert_value()?];
        while self.check(Token::Comma) {
            self.advance();
            values.push(self.parse_insert_value()?);
        }
        if parens {
            self.expect(Token::RParen)?;
        }
        Ok(Expr::StoreInsert(store, values, sp))
    }

    pub(in crate::parser) fn parse_insert_stmt(&mut self) -> Result<Stmt, ParseError> {
        let e = self.parse_insert_expr()?;
        let Expr::StoreInsert(store, values, sp) = e else {
            unreachable!()
        };
        if self.at_multiline_arms() {
            let q = self.parse_multiline_handler_arms(Expr::StoreInsert(store, values, sp))?;
            return Ok(Stmt::Expr(q));
        }
        if matches!(self.peek(), Token::Question | Token::BangBang) {
            let q = self.parse_stmt_handler_arms(Expr::StoreInsert(store, values, sp))?;
            return Ok(Stmt::Expr(q));
        }
        Ok(Stmt::StoreInsert(store, values, sp))
    }

    pub(in crate::parser) fn parse_insert_value(
        &mut self,
    ) -> Result<crate::ast::FieldInit, ParseError> {
        if let (Token::Ident(name), Token::Is) = (self.peek().clone(), self.peek_at(1)) {
            self.advance();
            self.advance();
            let value = self.parse_pipeline()?;
            return Ok(crate::ast::FieldInit {
                name: Some(name),
                value,
            });
        }
        Ok(crate::ast::FieldInit {
            name: None,
            value: self.parse_pipeline()?,
        })
    }

    pub(in crate::parser) fn parse_delete_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Delete)?;
        let store = self.ident()?;
        let filter = self.parse_store_filter()?;
        Ok(Stmt::StoreDelete(store, filter, sp))
    }

    pub(in crate::parser) fn parse_set_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Set)?;
        let store = self.ident()?;
        let filter = self.parse_store_filter()?;
        let mut assignments = Vec::new();
        loop {
            if self.check(Token::Newline) || self.check(Token::Eof) {
                break;
            }
            if matches!(self.peek(), Token::Question | Token::BangBang) {
                break;
            }
            let field = self.ident()?;
            let value = self.parse_pipeline()?;
            assignments.push((field, value));
            if self.check(Token::Comma) {
                self.advance();
            }
        }
        if assignments.is_empty() {
            return Err(self.error("expected at least one field assignment in set statement"));
        }
        if self.at_multiline_arms() {
            let q = self.parse_multiline_handler_arms(Expr::StoreUpdate(
                store,
                assignments,
                Box::new(filter),
                sp,
            ))?;
            return Ok(Stmt::Expr(q));
        }
        if matches!(self.peek(), Token::Question | Token::BangBang) {
            let q = self.parse_stmt_handler_arms(Expr::StoreUpdate(
                store,
                assignments,
                Box::new(filter),
                sp,
            ))?;
            return Ok(Stmt::Expr(q));
        }
        Ok(Stmt::StoreSet(store, assignments, filter, sp))
    }

    pub(in crate::parser) fn parse_transaction(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Transaction)?;
        self.expect(Token::Newline)?;
        let body = self.parse_block()?;
        Ok(Stmt::Transaction(body, sp))
    }

    pub(in crate::parser) fn parse_together(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Together)?;
        let name = if let Token::Ident(_) = self.peek() {
            Some(self.ident()?)
        } else {
            None
        };
        self.expect(Token::Newline)?;
        let body = self.parse_block()?;
        Ok(Stmt::Together(name, body, sp))
    }

    pub(in crate::parser) fn parse_store_filter(&mut self) -> Result<StoreFilter, ParseError> {
        let sp = self.span();
        let kw = self.ident()?;
        if kw != "where" {
            return Err(self.error("expected 'where'"));
        }
        let mut flat: Vec<(LogicalOp, StoreFilterCond)> = Vec::new();
        loop {
            let logical = if flat.is_empty() {
                LogicalOp::And
            } else {
                match self.peek() {
                    Token::And => {
                        self.advance();
                        LogicalOp::And
                    }
                    Token::Or => {
                        self.advance();
                        LogicalOp::Or
                    }
                    _ => break,
                }
            };
            self.parse_filter_group(logical, &mut flat)?;
        }
        let seen_and = flat.iter().skip(1).any(|(l, _)| *l == LogicalOp::And);
        let seen_or = flat.iter().skip(1).any(|(l, _)| *l == LogicalOp::Or);
        if seen_and && seen_or {
            return Err(self.error(
                "mixed 'and'/'or' in a where clause is ambiguous; group with parentheses",
            ));
        }
        let (_, head) = flat.remove(0);
        Ok(StoreFilter {
            field: head.field,
            op: head.op,
            value: head.value,
            pred: head.pred,
            span: sp,
            extra: flat,
        })
    }

    fn parse_filter_group(
        &mut self,
        logical: LogicalOp,
        out: &mut Vec<(LogicalOp, StoreFilterCond)>,
    ) -> Result<(), ParseError> {
        if self.check(Token::LParen) {
            self.advance();
            let mut first = true;
            loop {
                let inner = if first {
                    first = false;
                    logical
                } else {
                    match self.peek() {
                        Token::And => {
                            self.advance();
                            LogicalOp::And
                        }
                        Token::Or => {
                            self.advance();
                            LogicalOp::Or
                        }
                        _ => break,
                    }
                };
                self.parse_filter_group(inner, out)?;
            }
            self.expect(Token::RParen)?;
            return Ok(());
        }
        self.parse_filter_cond(logical, out)
    }

    fn parse_filter_cond(
        &mut self,
        logical: LogicalOp,
        out: &mut Vec<(LogicalOp, StoreFilterCond)>,
    ) -> Result<(), ParseError> {
        let field = self.ident()?;
        if let Token::Ident(word) = self.peek().clone() {
            match word.as_str().as_str() {
                "contains" => {
                    self.advance();
                    let v = self.parse_bitor()?;
                    out.push((
                        logical,
                        StoreFilterCond {
                            field,
                            op: BinOp::Eq,
                            value: v,
                            pred: crate::ast::FilterPred::Contains,
                        },
                    ));
                    return Ok(());
                }
                "starts_with" => {
                    self.advance();
                    let v = self.parse_bitor()?;
                    out.push((
                        logical,
                        StoreFilterCond {
                            field,
                            op: BinOp::Eq,
                            value: v,
                            pred: crate::ast::FilterPred::StartsWith,
                        },
                    ));
                    return Ok(());
                }
                "ends_with" => {
                    self.advance();
                    let v = self.parse_bitor()?;
                    out.push((
                        logical,
                        StoreFilterCond {
                            field,
                            op: BinOp::Eq,
                            value: v,
                            pred: crate::ast::FilterPred::EndsWith,
                        },
                    ));
                    return Ok(());
                }
                "between" => {
                    self.advance();
                    let lo = self.parse_bitor()?;
                    self.expect(Token::And)?;
                    let hi = self.parse_bitor()?;
                    out.push((
                        logical,
                        StoreFilterCond {
                            field,
                            op: BinOp::Ge,
                            value: lo,
                            pred: crate::ast::FilterPred::Cmp,
                        },
                    ));
                    out.push((
                        LogicalOp::And,
                        StoreFilterCond {
                            field,
                            op: BinOp::Le,
                            value: hi,
                            pred: crate::ast::FilterPred::Cmp,
                        },
                    ));
                    return Ok(());
                }
                _ => {}
            }
        }
        if self.check(Token::In) {
            self.advance();
            self.expect(Token::LBracket)?;
            let mut values = Vec::new();
            if !self.check(Token::RBracket) {
                values.push(self.parse_bitor()?);
                while self.check(Token::Comma) {
                    self.advance();
                    values.push(self.parse_bitor()?);
                }
            }
            self.expect(Token::RBracket)?;
            if values.is_empty() {
                return Err(self.error("'in' filter requires at least one value"));
            }
            for (i, v) in values.into_iter().enumerate() {
                out.push((
                    if i == 0 { logical } else { LogicalOp::Or },
                    StoreFilterCond {
                        field,
                        op: BinOp::Eq,
                        value: v,
                        pred: crate::ast::FilterPred::Cmp,
                    },
                ));
            }
            return Ok(());
        }
        let op = self.parse_filter_op()?;
        let value = self.parse_bitor()?;
        out.push((
            logical,
            StoreFilterCond {
                field,
                op,
                value,
                pred: crate::ast::FilterPred::Cmp,
            },
        ));
        Ok(())
    }

    pub(in crate::parser) fn parse_filter_op(&mut self) -> Result<BinOp, ParseError> {
        let op = match self.peek() {
            Token::Equals => BinOp::Eq,
            Token::Neq => BinOp::Ne,
            Token::Lt => BinOp::Lt,
            Token::Gt => BinOp::Gt,
            Token::LtEq => BinOp::Le,
            Token::GtEq => BinOp::Ge,
            _ => return Err(self.error("expected comparison operator")),
        };
        self.advance();
        Ok(op)
    }

    pub(in crate::parser) fn parse_destroy_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.advance();
        let store = self.ident()?;
        let filter = self.parse_store_filter()?;
        Ok(Stmt::StoreDestroy(store, filter, sp))
    }

    pub(in crate::parser) fn parse_restore_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.advance();
        let store = self.ident()?;
        let filter = self.parse_store_filter()?;
        Ok(Stmt::StoreRestore(store, filter, sp))
    }

    pub(in crate::parser) fn parse_save_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.advance();
        let store = self.ident()?;
        Ok(Stmt::StoreSave(store, sp))
    }

    pub(in crate::parser) fn parse_compact_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.advance();
        let store = self.ident()?;
        Ok(Stmt::StoreCompact(store, sp))
    }
}
