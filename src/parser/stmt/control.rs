use super::super::{ParseError, Parser};
use crate::ast::*;
use crate::lexer::Token;

impl Parser {
    pub(in crate::parser) fn parse_if(&mut self) -> Result<If, ParseError> {
        let sp = self.span();
        self.expect(Token::If)?;
        let cond = self.parse_expr()?;
        self.expect(Token::Newline)?;
        let then = self.parse_block()?;
        let mut elifs = Vec::new();
        while self.check(Token::Elif) {
            self.advance();
            let c = self.parse_expr()?;
            self.expect(Token::Newline)?;
            elifs.push((c, self.parse_block()?));
        }

        let mut consumed_else = false;
        loop {
            if self.check(Token::Elif) {
                self.advance();
                let c = self.parse_expr()?;
                self.expect(Token::Newline)?;
                elifs.push((c, self.parse_block()?));
                continue;
            }
            if self.check(Token::Else) {
                self.advance();
                if self.check(Token::If) {
                    self.advance();
                    let c = self.parse_expr()?;
                    self.expect(Token::Newline)?;
                    elifs.push((c, self.parse_block()?));
                    continue;
                }
                consumed_else = true;
                break;
            }
            break;
        }
        let els = if consumed_else {
            self.expect(Token::Newline)?;
            Some(self.parse_block()?)
        } else {
            None
        };
        Ok(If {
            cond,
            then,
            elifs,
            els,
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::While)?;
        let cond = self.parse_expr()?;
        self.expect(Token::Newline)?;
        Ok(Stmt::While(While {
            cond,
            body: self.parse_block()?,
            span: sp,
        }))
    }

    pub(in crate::parser) fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::For)?;

        let access_mod = self.try_parse_access_mod_at_binder_pos();
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
            self.suppress_by = true;
            let e = self.parse_expr()?;
            self.suppress_by = false;
            Some(e)
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
        Ok(Stmt::For(For {
            label: None,
            bind,
            bind2,
            iter,
            end,
            step,
            body: self.parse_block()?,
            access_mod,
            span: sp,
        }))
    }

    pub(in crate::parser) fn parse_match(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Match)?;
        let subject = self.parse_expr()?;
        self.expect(Token::Newline)?;
        let arms = self.parse_indented(Self::parse_arm)?;
        Ok(Stmt::Match(Match {
            subject,
            arms,
            span: sp,
        }))
    }

    pub(in crate::parser) fn parse_arm(&mut self) -> Result<Arm, ParseError> {
        let sp = self.span();
        let pat = self.parse_pat()?;
        let guard = if self.check(Token::When) || self.check(Token::If) {
            self.advance();
            Some(self.parse_pipeline()?)
        } else {
            None
        };
        self.expect(Token::Question)?;
        let body = if self.check(Token::Newline) {
            self.advance();
            self.parse_block()?
        } else {
            vec![self.parse_stmt()?]
        };
        Ok(Arm {
            pat,
            guard,
            body,
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_asm_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Asm)?;
        let mut outputs = Vec::new();
        let mut inputs = Vec::new();
        if self.check(Token::LParen) {
            self.advance();
            while !self.check(Token::RParen) && !self.eof() {
                let name = self.ident()?;
                if self.check(Token::Is) {
                    self.advance();
                    let expr = self.parse_expr()?;
                    inputs.push((name.as_str(), expr));
                } else {
                    outputs.push((name.as_str(), format!("={{{name}}}")));
                }
                if !self.check(Token::RParen) {
                    self.expect(Token::Comma)?;
                }
            }
            self.expect(Token::RParen)?;
        }
        self.expect(Token::Newline)?;
        self.expect(Token::Indent)?;
        let mut lines = Vec::new();
        while !self.check(Token::Dedent) && !self.eof() {
            self.skip_nl();
            if self.check(Token::Dedent) || self.eof() {
                break;
            }
            let mut line = String::new();
            while !self.check(Token::Newline) && !self.check(Token::Dedent) && !self.eof() {
                let t = &self.tok[self.pos];
                let tok_str = t.token.to_string();
                if !line.is_empty() {
                    let no_space_before =
                        matches!(t.token, Token::RParen | Token::RBracket | Token::Comma);

                    let no_space_after = line.ends_with('(') || line.ends_with('[');
                    if !no_space_before && !no_space_after {
                        line.push(' ');
                    }
                }
                line.push_str(&tok_str);
                self.advance();
            }
            if !line.is_empty() {
                lines.push(line);
            }
            self.skip_nl();
        }
        if self.check(Token::Dedent) {
            self.advance();
        }
        let template = lines.join("\n");
        Ok(Stmt::Asm(AsmBlock {
            template,
            outputs,
            inputs,
            clobbers: Vec::new(),
            span: sp,
        }))
    }
}
