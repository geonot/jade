use crate::ast::*;
use crate::lexer::Token;

use super::expr::{
    contains_index_placeholder_in_block, contains_placeholder_in_block,
    replace_index_placeholder_in_block, replace_placeholder_in_block,
};
use super::{ParseError, Parser};

impl Parser {
    pub(in crate::parser) fn parse_block(&mut self) -> Result<Block, ParseError> {
        self.expect(Token::Indent)?;
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
            items.push(stmt);
            for post in self.pending_post_stmts.drain(..).collect::<Vec<_>>() {
                items.push(post);
            }
            self.skip_nl();
        }
        if self.check(Token::Dedent) {
            self.advance();
        }
        Ok(items)
    }
}

mod bind;
mod control;
mod dispatch;
mod store;
