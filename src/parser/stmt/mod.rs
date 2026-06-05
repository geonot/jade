use crate::ast::*;
use crate::lexer::Token;

use super::{ParseError, Parser};

impl Parser {
    pub(in crate::parser) fn parse_block(&mut self) -> Result<Block, ParseError> {
        self.expect(Token::Indent)?;
        let items = self.parse_block_items(true)?;
        if self.check(Token::Dedent) {
            self.advance();
        }
        Ok(items)
    }

    fn parse_block_items(&mut self, _top: bool) -> Result<Block, ParseError> {
        let mut items: Block = Vec::new();
        while !self.check(Token::Dedent) && !self.eof() {
            self.skip_nl();
            if self.check(Token::Dedent) || self.eof() {
                break;
            }
            let stmt = self.parse_stmt()?;

            for pre in self.pending_pre_stmts.drain(..).collect::<Vec<_>>() {
                items.push(pre);
            }

            if self.pending_propagates.is_empty() {
                items.push(stmt);
                for post in self.pending_post_stmts.drain(..).collect::<Vec<_>>() {
                    items.push(post);
                }
                self.skip_nl();
                continue;
            }

            let propagates = std::mem::take(&mut self.pending_propagates);
            let mut tail: Block = vec![stmt];
            for post in self.pending_post_stmts.drain(..).collect::<Vec<_>>() {
                tail.push(post);
            }
            self.skip_nl();
            let rest = self.parse_block_items(false)?;
            tail.extend(rest);

            let wrapped = self.wrap_propagates(propagates, tail);
            items.extend(wrapped);
            break;
        }
        Ok(items)
    }

    fn wrap_propagates(
        &mut self,
        propagates: Vec<super::PendingPropagate>,
        tail: Block,
    ) -> Block {
        let mut body = tail;
        for p in propagates.into_iter().rev() {
            let sp = p.span;
            let ok_arm = Arm {
                pat: Pat::Ctor("Ok".into(), vec![Pat::Ident(p.val_name, sp)], sp),
                guard: None,
                body,
                span: sp,
            };
            let err_arm = Arm {
                pat: Pat::Ctor("Err".into(), vec![Pat::Ident(p.err_name, sp)], sp),
                guard: None,
                body: vec![Stmt::ErrReturn(Expr::Ident(p.err_name, sp), sp)],
                span: sp,
            };
            body = vec![Stmt::Match(Match {
                subject: p.subject,
                arms: vec![ok_arm, err_arm],
                span: sp,
            })];
        }
        body
    }
}

mod bind;
mod control;
mod dispatch;
mod store;
