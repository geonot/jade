use crate::ast::*;
use crate::lexer::Token;
use super::super::expr::{
    contains_index_placeholder_in_block, contains_placeholder_in_block,
    replace_index_placeholder_in_block, replace_placeholder_in_block,
};
use super::super::{ParseError, Parser};

impl Parser {
    pub(in crate::parser) fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
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
                    Ok(Stmt::SimFor(
                        For {
                            label: None,
                            bind,
                            bind2: None,
                            iter,
                            end,
                            step,
                            body: self.parse_block()?,
                            span: sp,
                        },
                        sp,
                    ))
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
                // `loop items` → desugar to `for __ph_N in items` with $ → __ph_N, $$ → __ph_N_idx
                if !self.check(Token::Newline) && !self.check(Token::Indent) && !self.eof() {
                    let iter = self.parse_expr()?;
                    // `loop start to end` → range loop with $ as value, $$ as index
                    let (end, step) = if self.check(Token::To) {
                        self.advance();
                        self.suppress_by = true;
                        let e = self.parse_expr()?;
                        self.suppress_by = false;
                        let s = if self.check(Token::By) {
                            self.advance();
                            Some(self.parse_expr()?)
                        } else {
                            None
                        };
                        (Some(e), s)
                    } else {
                        (None, None)
                    };
                    self.expect(Token::Newline)?;
                    let body = self.parse_block()?;
                    let ph_name = self.gensym("ph");
                    let ph_idx_name = format!("{ph_name}_idx");
                    // Replace any $ in the body with the unique placeholder
                    let body = if contains_placeholder_in_block(&body) {
                        replace_placeholder_in_block(&body, &ph_name)
                    } else {
                        body
                    };
                    // Replace any $$ in the body with the unique index placeholder
                    let has_idx = contains_index_placeholder_in_block(&body);
                    let body = if has_idx {
                        replace_index_placeholder_in_block(&body, &ph_idx_name)
                    } else {
                        body
                    };
                    return Ok(Stmt::For(For {
                        label: None,
                        bind: ph_name.into(),
                        bind2: if has_idx {
                            Some(ph_idx_name.into())
                        } else {
                            None
                        },
                        iter,
                        end,
                        step,
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
            Token::BangBang => {
                let sp = self.span();
                self.advance();
                let val = self.parse_expr()?;
                Ok(Stmt::ErrReturn(val, sp))
            }
            Token::Defer => {
                let sp = self.span();
                self.advance();
                // `defer` followed by either an indented block or a single
                // inline statement (`defer close(fd)`).
                let body = if self.check(Token::Newline) {
                    self.expect(Token::Newline)?;
                    self.parse_block()?
                } else {
                    vec![self.parse_stmt()?]
                };
                Ok(Stmt::Defer(body, sp))
            }
            Token::Use => {
                let u = self.parse_use_decl()?;
                Ok(Stmt::UseLocal(u))
            }
            Token::Atomic => {
                let sp = self.span();
                self.advance();
                let name = self.ident()?;
                if let Some(op) = self.aug_op() {
                    let rhs = self.parse_expr()?;
                    let rsp = rhs.span();
                    return Ok(Stmt::Bind(Bind {
                        name: name.clone(),
                        value: Expr::BinOp(Box::new(Expr::Ident(name, sp)), op, Box::new(rhs), rsp),
                        ty: None,
                        atomic: true,
                        span: sp,
                    }));
                }
                self.expect(Token::Is)?;
                let value = self.parse_expr()?;
                Ok(Stmt::Bind(Bind {
                    name,
                    value,
                    ty: None,
                    atomic: true,
                    span: sp,
                }))
            }
            _ => {
                // Store statement keywords parsed as identifiers
                if let Token::Ident(kw) = self.peek() {
                    let kw = kw.clone();
                    match &*kw.as_str() {
                        "destroy" => return self.parse_destroy_stmt(),
                        "restore" => return self.parse_restore_stmt(),
                        "save" => return self.parse_save_stmt(),
                        _ => {}
                    }
                }
                if self.is_tuple_bind() {
                    self.parse_tuple_bind()
                } else if self.is_bind() {
                    self.parse_bind()
                } else {
                    // Bare-statement handler chain: `call() ? on_ok ! on_err`.
                    // We parse via parse_pipeline so the trailing `?` and
                    // `!` remain visible at this level. If neither sugar
                    // shape applies we splice the rest of the expression
                    // back together and behave exactly as `parse_expr`.
                    let head = self.parse_pipeline()?;
                    if self.check(Token::Question)
                        && matches!(head, Expr::Call(..))
                    {
                        return self.finish_bare_handler_chain(head);
                    }
                    if self.check(Token::BangBang) {
                        return self.finish_bare_bangbang(head);
                    }
                    let head_sp = head.span();
                    let expr = self.complete_expr_after_pipeline(head)?;
                    if self.check(Token::Is) {
                        self.advance();
                        let val = self.parse_expr()?;
                        Ok(Stmt::Assign(expr, val, head_sp))
                    } else {
                        Ok(Stmt::Expr(expr))
                    }
                }
            }
        }
    }

    pub(in crate::parser) fn is_bind(&self) -> bool {
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
                    | Token::UshrEq
            )
    }

    pub(in crate::parser) fn is_tuple_bind(&self) -> bool {
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

    pub(in crate::parser) fn parse_tuple_bind(&mut self) -> Result<Stmt, ParseError> {
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

    pub(in crate::parser) fn aug_op(&mut self) -> Option<BinOp> {
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
            Token::UshrEq => BinOp::Ushr,
            _ => return None,
        };
        self.advance();
        Some(op)
    }

}