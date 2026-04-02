use crate::ast::*;
use crate::lexer::Token;

use super::expr::{contains_placeholder_in_block, replace_placeholder_in_block};
use super::{ParseError, Parser};

impl Parser {
    pub(super) fn parse_block(&mut self) -> Result<Block, ParseError> {
        self.parse_indented(Self::parse_stmt)
    }

    pub(super) fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek() {
            Token::If => {
                let sp = self.span();
                self.advance();
                let subject = self.parse_expr()?;
                if self.check(Token::Is) {
                    self.advance();
                    let pat = self.parse_pat()?;
                    self.expect(Token::Newline)?;
                    let then_body = self.parse_block()?;
                    let else_body = if self.check(Token::Else) {
                        self.advance();
                        self.expect(Token::Newline)?;
                        self.parse_block()?
                    } else {
                        vec![]
                    };
                    let arms = vec![
                        Arm {
                            pat,
                            guard: None,
                            body: then_body,
                            span: sp,
                        },
                        Arm {
                            pat: Pat::Wild(sp),
                            guard: None,
                            body: else_body,
                            span: sp,
                        },
                    ];
                    Ok(Stmt::Match(Match {
                        subject,
                        arms,
                        span: sp,
                    }))
                } else {
                    self.expect(Token::Newline)?;
                    let then = self.parse_block()?;
                    let mut elifs = Vec::new();
                    while self.check(Token::Elif) {
                        self.advance();
                        let c = self.parse_expr()?;
                        self.expect(Token::Newline)?;
                        elifs.push((c, self.parse_block()?));
                    }
                    let els = if self.check(Token::Else) {
                        self.advance();
                        self.expect(Token::Newline)?;
                        Some(self.parse_block()?)
                    } else {
                        None
                    };
                    Ok(Stmt::If(If {
                        cond: subject,
                        then,
                        elifs,
                        els,
                        span: sp,
                    }))
                }
            }
            Token::While => self.parse_while(),
            Token::Until => {
                let sp = self.span();
                self.advance();
                let cond = self.parse_expr()?;
                self.expect(Token::Newline)?;
                Ok(Stmt::While(While {
                    cond: Expr::UnaryOp(UnaryOp::Not, Box::new(cond), sp),
                    body: self.parse_block()?,
                    span: sp,
                }))
            }
            Token::Unless => {
                let sp = self.span();
                self.advance();
                let cond = self.parse_expr()?;
                self.expect(Token::Newline)?;
                let body = self.parse_block()?;
                Ok(Stmt::If(If {
                    cond: Expr::UnaryOp(UnaryOp::Not, Box::new(cond), sp),
                    then: body,
                    elifs: vec![],
                    els: None,
                    span: sp,
                }))
            }
            Token::For => self.parse_for(),
            Token::Sim => {
                let sp = self.span();
                self.advance();
                if self.check(Token::For) {
                    self.advance();
                    let bind = self.ident()?;
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
                    Ok(Stmt::SimFor(For {
                        label: None,
                        bind,
                        bind2: None,
                        iter,
                        end,
                        step,
                        body: self.parse_block()?,
                        span: sp,
                    }, sp))
                } else {
                    // sim block: run statements in parallel
                    self.expect(Token::Newline)?;
                    let body = self.parse_block()?;
                    Ok(Stmt::SimBlock(body, sp))
                }
            }
            Token::Loop => {
                let sp = self.span();
                self.advance();
                // `loop items` → desugar to `for __ph in items` with $ → __ph
                if !self.check(Token::Newline) && !self.check(Token::Indent) && !self.eof() {
                    let iter = self.parse_expr()?;
                    self.expect(Token::Newline)?;
                    let body = self.parse_block()?;
                    // Replace any $ in the body with __ph
                    let body = if contains_placeholder_in_block(&body) {
                        replace_placeholder_in_block(&body, "__ph")
                    } else {
                        body
                    };
                    return Ok(Stmt::For(For {
                        label: None,
                        bind: "__ph".into(),
                        bind2: None,
                        iter,
                        end: None,
                        step: None,
                        body,
                        span: sp,
                    }));
                }
                self.expect(Token::Newline)?;
                Ok(Stmt::Loop(Loop {
                    body: self.parse_block()?,
                    span: sp,
                }))
            }
            Token::Return => {
                let sp = self.span();
                self.advance();
                let v = if !self.check(Token::Newline) && !self.check(Token::Dedent) && !self.eof()
                {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                Ok(Stmt::Ret(v, sp))
            }
            Token::Break => {
                let sp = self.span();
                self.advance();
                let v = if !self.check(Token::Newline)
                    && !self.check(Token::If)
                    && !self.check(Token::Dedent)
                    && !self.eof()
                {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                Ok(Stmt::Break(v, sp))
            }
            Token::Continue => {
                let sp = self.span();
                self.advance();
                Ok(Stmt::Continue(sp))
            }
            Token::Match => self.parse_match(),
            Token::Asm => self.parse_asm_stmt(),
            Token::Insert => self.parse_insert_stmt(),
            Token::Delete => self.parse_delete_stmt(),
            Token::Set => self.parse_set_stmt(),
            Token::Transaction => self.parse_transaction(),
            Token::Close => {
                let sp = self.span();
                self.advance();
                let ch = self.parse_expr()?;
                Ok(Stmt::ChannelClose(ch, sp))
            }
            Token::Stop => {
                let sp = self.span();
                self.advance();
                let target = self.parse_expr()?;
                Ok(Stmt::Stop(target, sp))
            }
            Token::Bang => {
                let sp = self.span();
                self.advance();
                let val = self.parse_expr()?;
                Ok(Stmt::ErrReturn(val, sp))
            }
            Token::Use => {
                let u = self.parse_use_decl()?;
                Ok(Stmt::UseLocal(u))
            }
            _ => {
                if self.is_tuple_bind() {
                    self.parse_tuple_bind()
                } else if self.is_bind() {
                    self.parse_bind()
                } else {
                    let expr = self.parse_expr()?;
                    if self.check(Token::Is) {
                        let sp = expr.span();
                        self.advance();
                        let val = self.parse_expr()?;
                        Ok(Stmt::Assign(expr, val, sp))
                    } else {
                        Ok(Stmt::Expr(expr))
                    }
                }
            }
        }
    }

    fn is_bind(&self) -> bool {
        matches!(self.peek(), Token::Ident(_))
            && self.pos + 1 < self.tok.len()
            && matches!(
                self.tok[self.pos + 1].token,
                Token::Is
                    | Token::PlusEq
                    | Token::MinusEq
                    | Token::StarEq
                    | Token::SlashEq
                    | Token::AmpEq
                    | Token::PipeEq
                    | Token::CaretEq
                    | Token::ShlEq
                    | Token::ShrEq
            )
    }

    fn is_tuple_bind(&self) -> bool {
        if !matches!(self.peek(), Token::Ident(_)) {
            return false;
        }
        let mut i = self.pos + 1;
        loop {
            if i >= self.tok.len() || !matches!(self.tok[i].token, Token::Comma) {
                return false;
            }
            i += 1;
            if i >= self.tok.len() || !matches!(self.tok[i].token, Token::Ident(_)) {
                return false;
            }
            i += 1;
            if i < self.tok.len() && matches!(self.tok[i].token, Token::Is) {
                return true;
            }
        }
    }

    fn parse_tuple_bind(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        let mut names = vec![self.ident()?];
        while self.check(Token::Comma) {
            self.advance();
            names.push(self.ident()?);
        }
        self.expect(Token::Is)?;
        let value = self.parse_expr()?;
        Ok(Stmt::TupleBind(names, value, sp))
    }

    fn aug_op(&mut self) -> Option<BinOp> {
        let op = match self.peek() {
            Token::PlusEq => BinOp::Add,
            Token::MinusEq => BinOp::Sub,
            Token::StarEq => BinOp::Mul,
            Token::SlashEq => BinOp::Div,
            Token::AmpEq => BinOp::BitAnd,
            Token::PipeEq => BinOp::BitOr,
            Token::CaretEq => BinOp::BitXor,
            Token::ShlEq => BinOp::Shl,
            Token::ShrEq => BinOp::Shr,
            _ => return None,
        };
        self.advance();
        Some(op)
    }

    fn parse_bind(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        let name = self.ident()?;
        if let Some(op) = self.aug_op() {
            let rhs = self.parse_expr()?;
            let rsp = rhs.span();
            return Ok(Stmt::Bind(Bind {
                name: name.clone(),
                value: Expr::BinOp(Box::new(Expr::Ident(name, sp)), op, Box::new(rhs), rsp),
                ty: None,
                span: sp,
            }));
        }
        self.expect(Token::Is)?;
        // Labeled loop: `outer is for i in items`
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
            return Ok(Stmt::For(For {
                label: Some(name),
                bind,
                bind2,
                iter,
                end,
                step,
                body: self.parse_block()?,
                span: sp,
            }));
        }
        Ok(Stmt::Bind(Bind {
            name,
            value: self.parse_expr()?,
            ty: None,
            span: sp,
        }))
    }

    pub(super) fn parse_if(&mut self) -> Result<If, ParseError> {
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
        let els = if self.check(Token::Else) {
            self.advance();
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

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
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

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::For)?;
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
            span: sp,
        }))
    }

    fn parse_match(&mut self) -> Result<Stmt, ParseError> {
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

    fn parse_arm(&mut self) -> Result<Arm, ParseError> {
        let sp = self.span();
        let pat = self.parse_pat()?;
        let guard = if self.check(Token::When) {
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
            vec![Stmt::Expr(self.parse_expr()?)]
        };
        Ok(Arm {
            pat,
            guard,
            body,
            span: sp,
        })
    }

    fn parse_asm_stmt(&mut self) -> Result<Stmt, ParseError> {
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
                    inputs.push((name.clone(), expr));
                } else {
                    outputs.push((name.clone(), format!("={{{name}}}")));
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
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(&t.token.to_string());
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

    fn parse_insert_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Insert)?;
        let store = self.ident()?;
        let mut values = vec![self.parse_expr()?];
        while self.check(Token::Comma) {
            self.advance();
            values.push(self.parse_expr()?);
        }
        Ok(Stmt::StoreInsert(store, values, sp))
    }

    fn parse_delete_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Delete)?;
        let store = self.ident()?;
        let filter = self.parse_store_filter()?;
        Ok(Stmt::StoreDelete(store, filter, sp))
    }

    fn parse_set_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Set)?;
        let store = self.ident()?;
        let filter = self.parse_store_filter()?;
        let mut assignments = Vec::new();
        loop {
            if self.check(Token::Newline) || self.check(Token::Eof) {
                break;
            }
            let field = self.ident()?;
            let value = self.parse_expr()?;
            assignments.push((field, value));
            if self.check(Token::Comma) {
                self.advance();
            }
        }
        if assignments.is_empty() {
            return Err(self.error("expected at least one field assignment in set statement"));
        }
        Ok(Stmt::StoreSet(store, assignments, filter, sp))
    }

    fn parse_transaction(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Transaction)?;
        self.expect(Token::Newline)?;
        let body = self.parse_block()?;
        Ok(Stmt::Transaction(body, sp))
    }

    pub(super) fn parse_store_filter(&mut self) -> Result<StoreFilter, ParseError> {
        let sp = self.span();
        let kw = self.ident()?;
        if kw != "where" {
            return Err(self.error("expected 'where'"));
        }
        let field = self.ident()?;
        let op = self.parse_filter_op()?;
        let value = self.parse_add()?;
        let mut extra = Vec::new();
        loop {
            let logical = match self.peek() {
                Token::And => LogicalOp::And,
                Token::Or => LogicalOp::Or,
                _ => break,
            };
            self.advance();
            let f = self.ident()?;
            let o = self.parse_filter_op()?;
            let v = self.parse_add()?;
            extra.push((
                logical,
                StoreFilterCond {
                    field: f,
                    op: o,
                    value: v,
                },
            ));
        }
        Ok(StoreFilter {
            field,
            op,
            value,
            span: sp,
            extra,
        })
    }

    fn parse_filter_op(&mut self) -> Result<BinOp, ParseError> {
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
}
