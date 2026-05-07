//! Parser arms for statements and blocks.

use crate::ast::*;
use crate::lexer::Token;

use super::expr::{
    contains_index_placeholder_in_block, contains_placeholder_in_block,
    replace_index_placeholder_in_block, replace_placeholder_in_block,
};
use super::{ParseError, Parser};

impl Parser {
    pub(super) fn parse_block(&mut self) -> Result<Block, ParseError> {
        // Custom inlined version of parse_indented that drains the pending
        // statement queues populated by Layer-2 sugar desugaring.
        self.expect(Token::Indent)?;
        let mut items: Block = Vec::new();
        while !self.check(Token::Dedent) && !self.eof() {
            self.skip_nl();
            if self.check(Token::Dedent) || self.eof() {
                break;
            }
            let stmt = self.parse_stmt()?;
            // Drain any pre-statements queued during this parse_stmt call.
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
                    | Token::UshrEq
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
            Token::UshrEq => BinOp::Ushr,
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
                atomic: false,
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
        // Layer 2 sugar: `a is RHS ! Variant` (guard) and
        // `a is RHS ? on_ok ! on_err` (handler chain). Both desugar at
        // parse-time into a small block of statements, wrapped in an
        // `if true { ... }` so we can return a single Stmt from this
        // function (jade has no Stmt::Block variant).
        //
        // We parse RHS with parse_pipeline so the trailing `?` and `!`
        // are visible to us (parse_expr/parse_ternary would consume them
        // as ternary operators).
        let value = self.parse_pipeline()?;
        // Re-attach `query` post-fix block so `r is users query ...`
        // continues to work (parse_expr does this for normal callers).
        let value = if self.check(Token::Query) {
            self.parse_query_block(value)?
        } else {
            value
        };

        // ── Form 1: `a is RHS ? on_ok [! on_err] [!! Variant]` ──────────
        // Only treat `?` as a handler-chain when RHS is a call expression
        // (otherwise `r is x > 3 ? "big" ! "small"` would lose its
        // standard ternary meaning).
        if self.check(Token::Question) && matches!(value, Expr::Call(..)) {
            self.advance();
            let on_ok = self.parse_pipeline()?;
            let (on_falsy_or_err, throw_variant): (Option<Expr>, Option<(Symbol, Span)>) =
                if self.check(Token::BangBang) {
                    self.advance();
                    let var_sp = self.span();
                    let v: Symbol = self.ident()?.into();
                    (None, Some((v, var_sp)))
                } else if self.check(Token::Bang) {
                    self.advance();
                    let arm_expr = self.parse_pipeline()?;
                    let throw = if self.check(Token::BangBang) {
                        self.advance();
                        let var_sp = self.span();
                        let v: Symbol = self.ident()?.into();
                        Some((v, var_sp))
                    } else {
                        None
                    };
                    (Some(arm_expr), throw)
                } else {
                    (None, None)
                };
            let tmp_name: Symbol = self.gensym("__hc").into();
            self.pending_pre_stmts.push(Stmt::Bind(Bind {
                name: tmp_name.clone(),
                value,
                ty: None,
                atomic: false,
                span: sp,
            }));
            // Build arms: same three-way logic as finish_bare_handler_chain but the
            // Ok arm always binds `name` (the user-visible bind target).
            let ok_arm;
            let err_arm;
            if let Some((variant_name, var_sp)) = throw_variant {
                // `!!` present: `!` is the falsy-Ok ternary-else, NOT an error handler.
                if let Some(falsy_expr) = on_falsy_or_err {
                    let v_name: Symbol = self.gensym("__v").into();
                    let ternary = Expr::Ternary(
                        Box::new(Expr::Ident(v_name.clone(), sp)),
                        Box::new(on_ok),
                        Box::new(falsy_expr),
                        sp,
                    );
                    ok_arm = Arm {
                        pat: Pat::Ctor(
                            "Ok".into(),
                            vec![Pat::Ident(v_name, sp)],
                            sp,
                        ),
                        guard: None,
                        body: vec![Stmt::Expr(ternary)],
                        span: sp,
                    };
                } else {
                    ok_arm = Arm {
                        pat: Pat::Ctor(
                            "Ok".into(),
                            vec![Pat::Ident(name.clone(), sp)],
                            sp,
                        ),
                        guard: None,
                        body: vec![Stmt::Expr(on_ok)],
                        span: sp,
                    };
                }
                err_arm = Arm {
                    pat: Pat::Wild(sp),
                    guard: None,
                    body: vec![Stmt::ErrReturn(Expr::Ident(variant_name, var_sp), var_sp)],
                    span: sp,
                };
            } else {
                // No `!!`: `! on_err` is the error/else handler with implicit `err` binding.
                ok_arm = Arm {
                    pat: Pat::Ctor(
                        "Ok".into(),
                        vec![Pat::Ident(name.clone(), sp)],
                        sp,
                    ),
                    guard: None,
                    body: vec![Stmt::Expr(on_ok)],
                    span: sp,
                };
                let err_pat = if on_falsy_or_err.is_some() {
                    Pat::Ident("err".into(), sp)
                } else {
                    Pat::Wild(sp)
                };
                err_arm = Arm {
                    pat: err_pat,
                    guard: None,
                    body: match on_falsy_or_err {
                        Some(err_expr) => vec![Stmt::Expr(err_expr)],
                        None => vec![],
                    },
                    span: sp,
                };
            }
            return Ok(Stmt::Match(Match {
                subject: Expr::Ident(tmp_name, sp),
                arms: vec![ok_arm, err_arm],
                span: sp,
            }));
        }

        // ── Form 1b: `a is RHS !! Variant` ──────────────────────────────
        // Bind a to the Ok payload; on any error throw Variant.
        // Desugars (via pre-stmts):
        //   __guard is RHS
        //   match __guard
        //       Ok(_) ?   (fall through)
        //       _     ?   ! Variant
        //   a is __guard
        if self.check(Token::BangBang) {
            self.advance();
            let var_sp = self.span();
            let variant_name = self.ident()?;
            let tmp_name: Symbol = self.gensym("__guard").into();
            self.pending_pre_stmts.push(Stmt::Bind(Bind {
                name: tmp_name.clone(),
                value,
                ty: None,
                atomic: false,
                span: sp,
            }));
            let ok_arm = Arm {
                pat: Pat::Ctor("Ok".into(), vec![Pat::Wild(sp)], sp),
                guard: None,
                body: vec![],
                span: sp,
            };
            let err_arm = Arm {
                pat: Pat::Wild(sp),
                guard: None,
                body: vec![Stmt::ErrReturn(Expr::Ident(variant_name, var_sp), var_sp)],
                span: sp,
            };
            self.pending_pre_stmts.push(Stmt::Match(Match {
                subject: Expr::Ident(tmp_name.clone(), sp),
                arms: vec![ok_arm, err_arm],
                span: sp,
            }));
            return Ok(Stmt::Bind(Bind {
                name,
                value: Expr::Ident(tmp_name, sp),
                ty: None,
                atomic: false,
                span: sp,
            }));
        }

        // ── Form 2: `a is RHS ! Variant` (guard form) ───────────────────
        // Desugar to (spliced into the enclosing block via pending queues):
        //   __guard is RHS
        //   match __guard
        //       Variant ?
        //           ! __guard
        //       _ ?
        //           (fall through)
        //   a is __guard      ← returned as the main Stmt
        //
        // The `!` must be followed immediately by a bare identifier (the
        // err-variant tag). We do NOT require an uppercase prefix — the
        // disambiguation against ternary-else is done by token shape:
        //   `a is x() ! Bad`        → guard (next is bare ident)
        //   `a is x ! "fallback"`   → ternary-else (next is a literal)
        //   `a is x ! foo()`        → ternary-else (ident followed by `(`)
        // If the named tag turns out not to be an err-variant, the typer
        // will produce an error when lowering the synthesized match arm.
        let next_is_bare_ident = self.check(Token::Bang)
            && self.pos + 1 < self.tok.len()
            && matches!(&self.tok[self.pos + 1].token, Token::Ident(_))
            && {
                let p2 = self.pos + 2;
                if p2 < self.tok.len() {
                    !matches!(
                        &self.tok[p2].token,
                        Token::LParen
                            | Token::LBracket
                            | Token::Dot
                            | Token::DotDotDot
                            | Token::Tilde
                    )
                } else {
                    true
                }
            };
        if next_is_bare_ident {
            if let Token::Ident(_) = &self.tok[self.pos + 1].token {
                {
                    {
                        self.advance(); // consume `!`
                        let var_sp = self.span();
                        let variant_name = self.ident()?;

                        let tmp_name: Symbol = self.gensym("__guard").into();
                        let bind_tmp = Stmt::Bind(Bind {
                            name: tmp_name.clone(),
                            value,
                            ty: None,
                            atomic: false,
                            span: sp,
                        });
                        let propagate_arm = Arm {
                            pat: Pat::Ctor(variant_name.clone(), vec![], var_sp),
                            guard: None,
                            body: vec![Stmt::ErrReturn(
                                Expr::Ident(tmp_name.clone(), sp),
                                sp,
                            )],
                            span: sp,
                        };
                        let fall_arm = Arm {
                            pat: Pat::Wild(sp),
                            guard: None,
                            body: vec![],
                            span: sp,
                        };
                        let match_stmt = Stmt::Match(Match {
                            subject: Expr::Ident(tmp_name.clone(), sp),
                            arms: vec![propagate_arm, fall_arm],
                            span: sp,
                        });
                        // Splice the temp-bind and match BEFORE the user-
                        // visible `a is __tmp` bind that we return.
                        self.pending_pre_stmts.push(bind_tmp);
                        self.pending_pre_stmts.push(match_stmt);
                        return Ok(Stmt::Bind(Bind {
                            name: name.clone(),
                            value: Expr::Ident(tmp_name, sp),
                            ty: None,
                            atomic: false,
                            span: sp,
                        }));
                    }
                }
            }
        }

        // ── Plain bind ──────────────────────────────────────────────────
        // If the RHS is followed by ternary operators that we did not
        // consume as sugar, re-parse them at this level to preserve the
        // standard ternary semantics for `is` bindings.
        let value = if self.check(Token::Question) {
            let qsp = self.span();
            self.advance();
            // `cond ? ! else_expr`
            if self.check(Token::Bang) {
                self.advance();
                let f = self.parse_pipeline()?;
                Expr::Ternary(
                    Box::new(value),
                    Box::new(Expr::Void(qsp)),
                    Box::new(f),
                    qsp,
                )
            } else {
                let t = self.parse_pipeline()?;
                if self.check(Token::Bang) {
                    self.advance();
                    let f = self.parse_expr()?;
                    Expr::Ternary(Box::new(value), Box::new(t), Box::new(f), qsp)
                } else {
                    Expr::Ternary(
                        Box::new(value),
                        Box::new(t),
                        Box::new(Expr::Void(qsp)),
                        qsp,
                    )
                }
            }
        } else if self.check(Token::Bang) && !self.suppress_bang_else {
            let bsp = self.span();
            self.advance();
            let f = self.parse_pipeline()?;
            Expr::Ternary(
                Box::new(value),
                Box::new(Expr::Void(bsp)),
                Box::new(f),
                bsp,
            )
        } else {
            value
        };
        Ok(Stmt::Bind(Bind {
            name,
            value,
            ty: None,
            atomic: false,
            span: sp,
        }))
    }

    /// Continue an expression after `parse_pipeline` has returned, applying
    /// the same ternary / query-block continuations that `parse_expr` would
    /// have. Used by callers that intercept tokens between the pipeline and
    /// the rest of the expression (e.g. the bare-statement handler chain).
    fn complete_expr_after_pipeline(&mut self, head: Expr) -> Result<Expr, ParseError> {
        // Ternary continuations. We mirror parse_ternary's logic.
        let value = if self.check(Token::Question) {
            let qsp = self.span();
            self.advance();
            if self.check(Token::Bang) {
                self.advance();
                let f = self.parse_pipeline()?;
                Expr::Ternary(
                    Box::new(head),
                    Box::new(Expr::Void(qsp)),
                    Box::new(f),
                    qsp,
                )
            } else {
                let t = self.parse_pipeline()?;
                if self.check(Token::Bang) {
                    self.advance();
                    let f = self.parse_expr()?;
                    Expr::Ternary(Box::new(head), Box::new(t), Box::new(f), qsp)
                } else {
                    Expr::Ternary(
                        Box::new(head),
                        Box::new(t),
                        Box::new(Expr::Void(qsp)),
                        qsp,
                    )
                }
            }
        } else if self.check(Token::Bang) && !self.suppress_bang_else {
            let bsp = self.span();
            self.advance();
            let f = self.parse_pipeline()?;
            Expr::Ternary(
                Box::new(head),
                Box::new(Expr::Void(bsp)),
                Box::new(f),
                bsp,
            )
        } else {
            head
        };
        // Query block (`expr query ...`) — parse_expr_inner does this after
        // the ternary layer.
        if self.check(Token::Query) {
            self.parse_query_block(value)
        } else {
            Ok(value)
        }
    }

    /// Bare `!! Variant` form: `expr !! Variant`.
    /// On any error from `head`, propagate as a freshly-constructed `Variant`.
    /// On Ok, silently fall through.
    fn finish_bare_bangbang(&mut self, head: Expr) -> Result<Stmt, ParseError> {
        let sp = head.span();
        debug_assert!(self.check(Token::BangBang));
        self.advance(); // consume `!!`
        let var_sp = self.span();
        let variant_name = self.ident()?;
        let tmp_name: Symbol = self.gensym("__hc").into();
        self.pending_pre_stmts.push(Stmt::Bind(Bind {
            name: tmp_name.clone(),
            value: head,
            ty: None,
            atomic: false,
            span: sp,
        }));
        let ok_arm = Arm {
            pat: Pat::Ctor("Ok".into(), vec![Pat::Wild(sp)], sp),
            guard: None,
            body: vec![],
            span: sp,
        };
        let err_arm = Arm {
            pat: Pat::Wild(sp),
            guard: None,
            body: vec![Stmt::ErrReturn(Expr::Ident(variant_name, var_sp), var_sp)],
            span: sp,
        };
        Ok(Stmt::Match(Match {
            subject: Expr::Ident(tmp_name, sp),
            arms: vec![ok_arm, err_arm],
            span: sp,
        }))
    }

    /// Bare-statement handler chain: `call() ? on_ok [! on_falsy] [!! Variant]`.
    /// Desugars to a `match` on the call's return value.
    ///
    /// - `call() ? on_ok`                   — ok: on_ok, err: no-op
    /// - `call() ? on_ok ! on_err`          — ok: on_ok, err: on_err (implicit `err` binding)
    /// - `call() ? on_ok !! Variant`        — ok: on_ok, err: throw Variant
    /// - `call() ? on_ok ! on_falsy !! V`   — truthy-ok: on_ok, falsy-ok: on_falsy, err: throw V
    ///
    /// When `!!` is present, `!` is the ternary-else (falsy non-error) branch, NOT an error
    /// handler. `!` is only the error handler (with implicit `err` binding) when `!!` is absent.
    fn finish_bare_handler_chain(&mut self, call: Expr) -> Result<Stmt, ParseError> {
        let sp = call.span();
        debug_assert!(self.check(Token::Question));
        self.advance(); // consume `?`
        let on_ok = self.parse_pipeline()?;
        // Parse the remaining arms.
        let (on_falsy_or_err, throw_variant): (Option<Expr>, Option<(Symbol, Span)>) =
            if self.check(Token::BangBang) {
                self.advance();
                let var_sp = self.span();
                let v: Symbol = self.ident()?.into();
                (None, Some((v, var_sp)))
            } else if self.check(Token::Bang) {
                self.advance();
                let arm_expr = self.parse_pipeline()?;
                let throw = if self.check(Token::BangBang) {
                    self.advance();
                    let var_sp = self.span();
                    let v: Symbol = self.ident()?.into();
                    Some((v, var_sp))
                } else {
                    None
                };
                (Some(arm_expr), throw)
            } else {
                (None, None)
            };
        // Bind the call to a temp so codegen has a stable subject.
        let tmp_name: Symbol = self.gensym("__hc").into();
        self.pending_pre_stmts.push(Stmt::Bind(Bind {
            name: tmp_name.clone(),
            value: call,
            ty: None,
            atomic: false,
            span: sp,
        }));
        // Build arms.
        let ok_arm;
        let err_arm;
        if let Some((variant_name, var_sp)) = throw_variant {
            // `!!` present: `!` (if present) is the falsy-Ok ternary-else, NOT an error handler.
            // Three-way desugar: truthy-ok → on_ok, falsy-ok → on_falsy, error → throw Variant.
            if let Some(falsy_expr) = on_falsy_or_err {
                let v_name: Symbol = self.gensym("__v").into();
                let ternary = Expr::Ternary(
                    Box::new(Expr::Ident(v_name.clone(), sp)),
                    Box::new(on_ok),
                    Box::new(falsy_expr),
                    sp,
                );
                ok_arm = Arm {
                    pat: Pat::Ctor("Ok".into(), vec![Pat::Ident(v_name, sp)], sp),
                    guard: None,
                    body: vec![Stmt::Expr(ternary)],
                    span: sp,
                };
            } else {
                ok_arm = Arm {
                    pat: Pat::Ctor("Ok".into(), vec![Pat::Wild(sp)], sp),
                    guard: None,
                    body: vec![Stmt::Expr(on_ok)],
                    span: sp,
                };
            }
            err_arm = Arm {
                pat: Pat::Wild(sp),
                guard: None,
                body: vec![Stmt::ErrReturn(Expr::Ident(variant_name, var_sp), var_sp)],
                span: sp,
            };
        } else {
            // No `!!`: `! on_err` is the error/else handler with implicit `err` binding.
            ok_arm = Arm {
                pat: Pat::Ctor("Ok".into(), vec![Pat::Wild(sp)], sp),
                guard: None,
                body: vec![Stmt::Expr(on_ok)],
                span: sp,
            };
            let err_pat = if on_falsy_or_err.is_some() {
                Pat::Ident("err".into(), sp)
            } else {
                Pat::Wild(sp)
            };
            err_arm = Arm {
                pat: err_pat,
                guard: None,
                body: match on_falsy_or_err {
                    Some(err_expr) => vec![Stmt::Expr(err_expr)],
                    None => vec![],
                },
                span: sp,
            };
        }
        Ok(Stmt::Match(Match {
            subject: Expr::Ident(tmp_name, sp),
            arms: vec![ok_arm, err_arm],
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
                    // Don't insert space before closing delimiters or comma
                    let no_space_before = matches!(
                        t.token,
                        Token::RParen | Token::RBracket | Token::Comma
                    );
                    // Don't insert space after opening delimiters
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

    fn parse_insert_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Insert)?;
        let store = self.ident()?;
        // Optional parenthesized form: `insert users (name is "alice", age is 30)`
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
        Ok(Stmt::StoreInsert(store, values, sp))
    }

    /// Parse one insert value: either `name is expr` (named) or a bare expr.
    fn parse_insert_value(&mut self) -> Result<crate::ast::FieldInit, ParseError> {
        // Look-ahead for `Ident is …` — but only when the rhs is a value
        // expression, not a relational comparison (so `users where age is 30`
        // is unaffected — `where` parses separately).
        if let (Token::Ident(name), Token::Is) = (self.peek().clone(), self.peek_at(1)) {
            // Reserve `where`/`from`/`to`/etc. as positional shorthands —
            // unlikely as field names but err on the side of acceptance.
            self.advance(); // ident
            self.advance(); // is
            let value = self.parse_expr()?;
            return Ok(crate::ast::FieldInit {
                name: Some(name),
                value,
            });
        }
        Ok(crate::ast::FieldInit {
            name: None,
            value: self.parse_expr()?,
        })
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
        let value = self.parse_bitor()?;
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
            let v = self.parse_bitor()?;
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

    fn parse_destroy_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.advance(); // consume 'destroy'
        let store = self.ident()?;
        let filter = self.parse_store_filter()?;
        Ok(Stmt::StoreDestroy(store, filter, sp))
    }

    fn parse_restore_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.advance(); // consume 'restore'
        let store = self.ident()?;
        let filter = self.parse_store_filter()?;
        Ok(Stmt::StoreRestore(store, filter, sp))
    }

    fn parse_save_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.advance(); // consume 'save'
        let store = self.ident()?;
        Ok(Stmt::StoreSave(store, sp))
    }
}
