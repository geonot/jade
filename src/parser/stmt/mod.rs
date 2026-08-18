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

            if matches!(stmt, Stmt::Bind(_))
                && self.pos > 0
                && !self.check(Token::Newline)
                && !self.check(Token::Dedent)
                && !self.eof()
                && !matches!(self.tok[self.pos - 1].token, Token::Dedent | Token::Newline)
            {
                return Err(self.error(
                    "unexpected token after binding — each statement must be on its own line",
                ));
            }

            for pre in self.pending_pre_stmts.drain(..).collect::<Vec<_>>() {
                items.push(pre);
            }

            items.push(stmt);
            for post in self.pending_post_stmts.drain(..).collect::<Vec<_>>() {
                items.push(post);
            }
            self.skip_nl();
        }
        Ok(items)
    }
}

mod bind;
mod control;
mod dispatch;
mod store;
