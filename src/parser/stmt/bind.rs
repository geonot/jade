use super::super::expr::{
    contains_index_placeholder_in_block, contains_placeholder_in_block,
    replace_index_placeholder_in_block, replace_placeholder_in_block,
};
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
                name: name.clone(),
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
            declared_ty = Some(self.parse_type()?);
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
            self.label_stack.push(name.clone().into());
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

        let value = self.parse_pipeline()?;

        let value = if self.check(Token::Query) {
            self.parse_query_block(value)?
        } else {
            value
        };

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
                access_mod: None,
                span: sp,
            }));

            let ok_arm;
            let err_arm;
            if let Some((variant_name, var_sp)) = throw_variant {
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
                        pat: Pat::Ctor("Ok".into(), vec![Pat::Ident(name.clone(), sp)], sp),
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
                ok_arm = Arm {
                    pat: Pat::Ctor("Ok".into(), vec![Pat::Ident(name.clone(), sp)], sp),
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
                access_mod: None,
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
                access_mod: None,
                span: sp,
            }));
        }

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
                        self.advance();
                        let var_sp = self.span();
                        let variant_name = self.ident()?;

                        let tmp_name: Symbol = self.gensym("__guard").into();
                        let bind_tmp = Stmt::Bind(Bind {
                            name: tmp_name.clone(),
                            value,
                            ty: None,
                            atomic: false,
                            access_mod: None,
                            span: sp,
                        });
                        let propagate_arm = Arm {
                            pat: Pat::Ctor(variant_name.clone(), vec![], var_sp),
                            guard: None,
                            body: vec![Stmt::ErrReturn(Expr::Ident(tmp_name.clone(), sp), sp)],
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

                        self.pending_pre_stmts.push(bind_tmp);
                        self.pending_pre_stmts.push(match_stmt);
                        return Ok(Stmt::Bind(Bind {
                            name: name.clone(),
                            value: Expr::Ident(tmp_name, sp),
                            ty: None,
                            atomic: false,
                            access_mod: None,
                            span: sp,
                        }));
                    }
                }
            }
        }

        let value = if self.check(Token::Question) {
            let qsp = self.span();
            self.advance();

            if self.check(Token::Bang) {
                self.advance();
                let f = self.parse_pipeline()?;
                Expr::Ternary(Box::new(value), Box::new(Expr::Void(qsp)), Box::new(f), qsp)
            } else {
                let t = self.parse_pipeline()?;
                if self.check(Token::Bang) {
                    self.advance();
                    let f = self.parse_expr()?;
                    Expr::Ternary(Box::new(value), Box::new(t), Box::new(f), qsp)
                } else {
                    Expr::Ternary(Box::new(value), Box::new(t), Box::new(Expr::Void(qsp)), qsp)
                }
            }
        } else if self.check(Token::Bang) && !self.suppress_bang_else {
            let bsp = self.span();
            self.advance();
            let f = self.parse_pipeline()?;
            Expr::Ternary(Box::new(value), Box::new(Expr::Void(bsp)), Box::new(f), bsp)
        } else {
            value
        };
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

    pub(in crate::parser) fn finish_bare_bangbang(
        &mut self,
        head: Expr,
    ) -> Result<Stmt, ParseError> {
        let sp = head.span();
        debug_assert!(self.check(Token::BangBang));
        self.advance();
        let var_sp = self.span();
        let variant_name = self.ident()?;
        let tmp_name: Symbol = self.gensym("__hc").into();
        self.pending_pre_stmts.push(Stmt::Bind(Bind {
            name: tmp_name.clone(),
            value: head,
            ty: None,
            atomic: false,
            access_mod: None,
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

    pub(in crate::parser) fn finish_bare_handler_chain(
        &mut self,
        call: Expr,
    ) -> Result<Stmt, ParseError> {
        let sp = call.span();
        debug_assert!(self.check(Token::Question));
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
            value: call,
            ty: None,
            atomic: false,
            access_mod: None,
            span: sp,
        }));

        let ok_arm;
        let err_arm;
        if let Some((variant_name, var_sp)) = throw_variant {
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
}
