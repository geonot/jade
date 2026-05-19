use super::{ParseError, Parser};
use crate::ast::*;
use crate::lexer::Token;
use crate::types::Type;

impl Parser {
    pub(in crate::parser) fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let sp = self.span();
        match self.peek() {
            Token::Err => {
                self.advance();
                Ok(Expr::Ident("err".into(), sp))
            }

            Token::Extern
                if self.pos + 1 < self.tok.len()
                    && matches!(self.tok[self.pos + 1].token, Token::Dot) =>
            {
                self.advance();
                Ok(Expr::Ident("extern".into(), sp))
            }

            Token::Type
                if self.pos + 1 < self.tok.len()
                    && matches!(self.tok[self.pos + 1].token, Token::Of) =>
            {
                self.advance();
                self.advance();
                let arg = self.parse_primary()?;
                Ok(Expr::OfCall(
                    Box::new(Expr::Ident("type".into(), sp)),
                    Box::new(arg),
                    sp,
                ))
            }
            Token::Int(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Int(n, sp))
            }
            Token::CharLit(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Int(n, sp))
            }
            Token::Float(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Float(n, sp))
            }
            Token::Str(s) => {
                let v = s.clone();
                self.advance();
                if self.check(Token::InterpStart) {
                    return self.parse_interp(v, sp);
                }
                Ok(Expr::Str(v, sp))
            }
            Token::True => {
                self.advance();
                Ok(Expr::Bool(true, sp))
            }
            Token::False => {
                self.advance();
                Ok(Expr::Bool(false, sp))
            }
            Token::None => {
                self.advance();
                Ok(Expr::None(sp))
            }
            Token::LParen => {
                self.advance();
                if self.check(Token::RParen) {
                    self.advance();
                    return Ok(Expr::Void(sp));
                }
                let e = self.parse_expr()?;
                if self.check(Token::Comma) {
                    let mut v = vec![e];
                    while self.check(Token::Comma) {
                        self.advance();
                        if self.check(Token::RParen) {
                            break;
                        }
                        v.push(self.parse_expr()?);
                    }
                    self.expect(Token::RParen)?;
                    return Ok(Expr::Tuple(v, sp));
                }
                self.expect(Token::RParen)?;
                Ok(e)
            }
            Token::LBracket => {
                self.advance();
                self.skip_ws();
                if self.check(Token::RBracket) {
                    self.advance();

                    return Ok(Expr::Call(
                        Box::new(Expr::Ident("vector".into(), sp)),
                        Vec::new(),
                        sp,
                    ));
                }
                let first = self.parse_expr()?;
                if self.check(Token::For) {
                    self.advance();
                    let bind = self.ident()?;
                    self.expect(Token::In)?;
                    let iter_start = self.parse_expr()?;
                    let iter_end = if self.check(Token::To) {
                        self.advance();
                        Some(Box::new(self.parse_expr()?))
                    } else {
                        None
                    };
                    let cond = if self.check(Token::If) {
                        self.advance();
                        Some(Box::new(self.parse_expr()?))
                    } else {
                        None
                    };
                    self.expect(Token::RBracket)?;
                    return Ok(Expr::ListComp(
                        Box::new(first),
                        bind.as_str(),
                        Box::new(iter_start),
                        iter_end,
                        cond,
                        sp,
                    ));
                }
                let mut v = vec![first];
                while self.check(Token::Comma) {
                    self.advance();
                    self.skip_ws();
                    if self.check(Token::RBracket) {
                        break;
                    }
                    v.push(self.parse_expr()?);
                }
                self.skip_ws();
                self.expect(Token::RBracket)?;

                Ok(Expr::Call(
                    Box::new(Expr::Ident("vector".into(), sp)),
                    v,
                    sp,
                ))
            }
            Token::Log => {
                self.advance();
                self.parse_builtin_call("log", sp)
            }
            Token::If => Ok(Expr::IfExpr(Box::new(self.parse_if()?))),
            Token::Assert => {
                self.advance();
                self.parse_builtin_call("assert", sp)
            }
            Token::Embed => {
                self.advance();
                let path = match self.peek() {
                    Token::Str(s) => {
                        let p = s.clone();
                        self.advance();
                        p
                    }
                    _ => return Err(self.error("embed requires a string literal path")),
                };
                Ok(Expr::Embed(path, sp))
            }
            Token::Unreachable => {
                self.advance();
                Ok(Expr::Unreachable(sp))
            }
            Token::Grad => {
                self.advance();
                self.expect(Token::LParen)?;
                let inner = self.parse_expr()?;
                self.expect(Token::RParen)?;
                Ok(Expr::Grad(Box::new(inner), sp))
            }
            Token::Einsum => {
                self.advance();
                let spec = match self.peek() {
                    Token::Str(s) => {
                        let v = s.clone();
                        self.advance();
                        v
                    }
                    _ => return Err(self.error("einsum requires a string spec")),
                };
                self.expect(Token::Comma)?;
                let mut args = vec![self.parse_expr()?];
                while self.check(Token::Comma) {
                    self.advance();
                    args.push(self.parse_expr()?);
                }
                Ok(Expr::Einsum(spec, args, sp))
            }
            Token::Build => {
                let next = if self.pos + 1 < self.tok.len() {
                    &self.tok[self.pos + 1].token
                } else {
                    &Token::Eof
                };
                if matches!(next, Token::LParen) {
                    self.advance();
                    Ok(Expr::Ident("build".into(), sp))
                } else {
                    self.advance();
                    let name = self.ident()?;
                    self.expect(Token::Newline)?;
                    let fields = self.parse_indented(|p| {
                        let fsp = p.span();
                        let fname = p.ident()?;
                        p.expect(Token::Is)?;
                        let fval = p.parse_expr()?;
                        Ok(BuilderField {
                            name: fname,
                            value: fval,
                            span: fsp,
                        })
                    })?;
                    Ok(Expr::Builder(name, fields, sp))
                }
            }
            Token::Ident(name) => {
                let name = name.clone();
                self.advance();

                // P0-5: `take EXPR` in expression position. The borrow/ownership
                // story for parameters with `take` access mod is already
                // enforced at the binding (parameter) side — passing a Var to
                // a `take` parameter moves it. Accept the `take EXPR` syntax
                // at the call site as a transparent marker (no AST node yet;
                // future P0-5 extension may add Expr::Take for richer moves
                // like `take v.field`).
                if name.with_str(|s| s == "take")
                    && matches!(
                        self.peek(),
                        Token::Ident(_) | Token::LParen | Token::Star
                    )
                {
                    return self.parse_unary();
                }

                if (name.with_str(|s| s == "vector" || s == "vec")) && self.check(Token::LBracket) {
                    self.advance();
                    self.skip_ws();
                    let mut elems = Vec::new();
                    if !self.check(Token::RBracket) {
                        elems.push(self.parse_expr()?);
                        while self.check(Token::Comma) {
                            self.advance();
                            self.skip_ws();
                            if self.check(Token::RBracket) {
                                break;
                            }
                            elems.push(self.parse_expr()?);
                        }
                        self.skip_ws();
                    }
                    self.expect(Token::RBracket)?;
                    return Ok(Expr::Call(Box::new(Expr::Ident(name, sp)), elems, sp));
                }
                if name.with_str(|s| s.starts_with(|c: char| c.is_uppercase()))
                    && self.check(Token::LParen)
                {
                    self.advance();
                    let mut fields = Vec::new();
                    while !self.check(Token::RParen) && !self.eof() {
                        self.skip_ws();
                        if self.check(Token::RParen) {
                            break;
                        }
                        if self.is_field_init() {
                            let n = self.ident()?;
                            self.expect(Token::Is)?;
                            fields.push(FieldInit {
                                name: Some(n),
                                value: self.parse_expr()?,
                            });
                        } else {
                            fields.push(FieldInit {
                                name: None,
                                value: self.parse_expr()?,
                            });
                        }
                        self.skip_ws();
                        if !self.check(Token::RParen) {
                            self.expect(Token::Comma)?;
                            self.skip_ws();
                        }
                    }
                    self.skip_ws();
                    self.expect(Token::RParen)?;
                    return Ok(Expr::Struct(name, fields, sp));
                }
                if name == "count" {
                    if let Token::Ident(_) = self.peek() {
                        let store = self.ident()?;
                        let filter = if matches!(self.peek(), Token::Ident(s) if s == "where") {
                            Some(Box::new(self.parse_store_filter()?))
                        } else {
                            None
                        };
                        return Ok(Expr::StoreCount(store, filter, sp));
                    }
                }
                if name == "all" {
                    if let Token::Ident(_) = self.peek() {
                        let store = self.ident()?;
                        return Ok(Expr::StoreAll(store, sp));
                    }
                }
                if name == "get" {
                    if let Token::Ident(_) = self.peek() {
                        let store = self.ident()?;
                        let key = self.parse_expr()?;
                        return Ok(Expr::StoreGet(store, Box::new(key), sp));
                    }
                }
                if name == "first" {
                    if let Token::Ident(_) = self.peek() {
                        let store = self.ident()?;
                        let filter = self.parse_store_filter()?;
                        return Ok(Expr::StoreFirst(store, Box::new(filter), sp));
                    }
                }
                if name == "exists" {
                    if let Token::Ident(_) = self.peek() {
                        let store = self.ident()?;
                        let filter = self.parse_store_filter()?;
                        return Ok(Expr::StoreExists(store, Box::new(filter), sp));
                    }
                }
                if name == "distinct" {
                    if let Token::Ident(_) = self.peek() {
                        let store = self.ident()?;
                        let field = self.ident()?;
                        return Ok(Expr::StoreDistinct(store, field, sp));
                    }
                }

                if matches!(self.peek(), Token::Colon) {
                    if let Token::Ident(_) = self.peek_at(1) {
                        self.advance();
                        let variant = self.ident()?;
                        return Ok(Expr::QualifiedIdent(name, variant, sp));
                    }
                }
                Ok(Expr::Ident(name, sp))
            }
            Token::Pipe => {
                self.advance();
                let mut params = Vec::new();
                while !self.check(Token::Pipe) && !self.eof() {
                    params.push(self.parse_param(true)?);
                    if !self.check(Token::Pipe) {
                        self.expect(Token::Comma)?;
                    }
                }
                self.expect(Token::Pipe)?;
                let ret = if self.check(Token::Returns) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let body = if self.check(Token::Indent) {
                    self.advance();
                    let mut stmts = Vec::new();
                    while !self.check(Token::Dedent) && !self.eof() {
                        self.skip_nl();
                        if self.check(Token::Dedent) || self.eof() {
                            break;
                        }
                        stmts.push(self.parse_stmt()?);
                        self.skip_nl();
                    }
                    if self.check(Token::Dedent) {
                        self.advance();
                    }
                    stmts
                } else if self.check(Token::Do) {
                    self.advance();
                    self.skip_nl();

                    if self.check(Token::Indent) {
                        self.advance();
                    }
                    let mut stmts = Vec::new();
                    while !self.check(Token::End) && !self.check(Token::Dedent) && !self.eof() {
                        self.skip_nl();
                        if self.check(Token::End) || self.check(Token::Dedent) || self.eof() {
                            break;
                        }
                        stmts.push(self.parse_stmt()?);
                        self.skip_nl();
                    }
                    if self.check(Token::Dedent) {
                        self.advance();
                    }
                    self.expect(Token::End)?;
                    stmts
                } else {
                    vec![Stmt::Expr(self.parse_expr()?)]
                };
                Ok(Expr::Lambda(params, ret, body, sp))
            }
            Token::Syscall => {
                self.advance();
                self.expect(Token::LParen)?;
                let mut args = Vec::new();
                if !self.check(Token::RParen) {
                    args.push(self.parse_expr()?);
                    while self.check(Token::Comma) {
                        self.advance();
                        if self.check(Token::RParen) {
                            break;
                        }
                        args.push(self.parse_expr()?);
                    }
                }
                self.expect(Token::RParen)?;
                Ok(Expr::Syscall(args, sp))
            }
            Token::Dollar => {
                self.advance();
                Ok(Expr::Placeholder(sp))
            }
            Token::DollarDollar => {
                self.advance();
                Ok(Expr::IndexPlaceholder(sp))
            }
            Token::Percent => {
                self.advance();
                let inner = self.parse_primary()?;
                Ok(Expr::Ref(Box::new(inner), sp))
            }
            Token::Spawn => {
                self.advance();
                let mut name = self.ident()?;

                while self.check(Token::Dot) {
                    self.advance();
                    name = self.ident()?;
                }

                let mut inits: Vec<(crate::intern::Symbol, Expr)> = Vec::new();
                if self.check(Token::LParen) {
                    self.advance();
                    while !self.check(Token::RParen) && !self.eof() {
                        let fname = self.ident()?;
                        if !self.check(Token::Is) {
                            return Err(self.error(
                                "spawn init args must be `field is value` (struct-literal style)",
                            ));
                        }
                        self.advance();
                        let val = self.parse_expr()?;
                        inits.push((fname, val));
                        if self.check(Token::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.expect(Token::RParen)?;
                }
                Ok(Expr::Spawn(name, inits, sp))
            }
            Token::Channel => {
                self.advance();
                let elem_ty = if self.check(Token::Of) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let cap = if self.check(Token::LParen) {
                    self.advance();
                    let c = self.parse_expr()?;
                    self.expect(Token::RParen)?;
                    c
                } else {
                    Expr::Int(16, sp)
                };
                Ok(Expr::ChannelCreate(elem_ty, Box::new(cap), sp))
            }
            Token::Select => {
                self.advance();
                self.expect(Token::Newline)?;
                self.expect(Token::Indent)?;
                let mut arms = Vec::new();
                let mut default_body = None;
                while !self.check(Token::Dedent) && !self.eof() {
                    self.skip_nl();
                    if self.check(Token::Dedent) || self.eof() {
                        break;
                    }
                    let arm_sp = self.span();
                    if self.check(Token::Default) {
                        self.advance();
                        self.expect(Token::Newline)?;
                        default_body = Some(self.parse_block()?);
                    } else if self.check(Token::Send) {
                        self.advance();
                        let ch = self.parse_expr()?;
                        self.expect(Token::Comma)?;
                        let val = self.parse_expr()?;
                        self.expect(Token::Newline)?;
                        let body = self.parse_block()?;
                        arms.push(crate::ast::SelectArm {
                            is_send: true,
                            chan: ch,
                            value: Some(val),
                            binding: None,
                            body,
                            span: arm_sp,
                        });
                    } else if self.check(Token::Receive) {
                        self.advance();
                        let ch_name = self.ident()?;
                        let ch = Expr::Ident(ch_name, arm_sp);
                        let binding = if self.check(Token::As) {
                            self.advance();
                            Some(self.ident()?)
                        } else {
                            None
                        };
                        self.expect(Token::Newline)?;
                        let body = self.parse_block()?;
                        arms.push(crate::ast::SelectArm {
                            is_send: false,
                            chan: ch,
                            value: None,
                            binding,
                            body,
                            span: arm_sp,
                        });
                    } else {
                        return Err(
                            self.error("expected 'send', 'receive', or 'default' in select")
                        );
                    }
                }
                if self.check(Token::Dedent) {
                    self.advance();
                }
                Ok(Expr::Select(arms, default_body, sp))
            }
            Token::Send | Token::Dispatch => {
                let is_dispatch = matches!(self.peek(), Token::Dispatch);
                self.advance();
                if is_dispatch {
                    if let Token::Ident(_) = self.peek() {
                        let next_idx = self.pos + 1;
                        if next_idx < self.tok.len()
                            && matches!(self.tok[next_idx].token, Token::Newline)
                        {
                            let name = self.ident()?;
                            self.expect(Token::Newline)?;
                            let body = self.parse_block()?;
                            return Ok(Expr::DispatchBlock(name, body, sp));
                        }
                    }
                    if self.check(Token::Newline) {
                        let next_idx = self.pos + 1;
                        if next_idx < self.tok.len()
                            && matches!(self.tok[next_idx].token, Token::Indent)
                        {
                            self.advance();
                            let body = self.parse_block()?;
                            return Ok(Expr::DispatchBlock("__anon".into(), body, sp));
                        }
                    }
                }
                let target = self.parse_expr()?;
                self.expect(Token::Comma)?;
                if self.check(Token::At) {
                    self.advance();
                    let handler = self.ident()?;
                    let mut args = Vec::new();
                    if self.check(Token::LParen) {
                        self.advance();
                        while !self.check(Token::RParen) && !self.eof() {
                            args.push(self.parse_expr()?);
                            if !self.check(Token::RParen) {
                                self.expect(Token::Comma)?;
                            }
                        }
                        self.expect(Token::RParen)?;
                    }
                    if is_dispatch {
                        Ok(Expr::Method(Box::new(target), handler, args, sp))
                    } else {
                        Ok(Expr::Send(Box::new(target), handler, args, sp))
                    }
                } else {
                    let val = self.parse_expr()?;
                    Ok(Expr::ChannelSend(Box::new(target), Box::new(val), sp))
                }
            }
            Token::Yield => {
                self.advance();
                let val = self.parse_expr()?;
                Ok(Expr::Yield(Box::new(val), sp))
            }
            Token::Receive => {
                self.advance();
                if !self.check(Token::Newline) {
                    let ch = self.parse_expr()?;
                    return Ok(Expr::ChannelRecv(Box::new(ch), sp));
                }
                self.expect(Token::Newline)?;
                self.expect(Token::Indent)?;
                let mut arms = Vec::new();
                while !self.check(Token::Dedent) && !self.eof() {
                    self.skip_nl();
                    if self.check(Token::Dedent) || self.eof() {
                        break;
                    }
                    let arm_sp = self.span();
                    self.expect(Token::At)?;
                    let handler_name = self.ident()?;
                    let mut bindings = Vec::new();
                    if self.check(Token::LParen) {
                        self.advance();
                        while !self.check(Token::RParen) && !self.eof() {
                            bindings.push(self.ident()?);
                            if !self.check(Token::RParen) {
                                self.expect(Token::Comma)?;
                            }
                        }
                        self.expect(Token::RParen)?;
                    }
                    self.expect(Token::Newline)?;
                    let body = self.parse_block()?;
                    arms.push(ReceiveArm {
                        handler: handler_name,
                        bindings,
                        body,
                        span: arm_sp,
                    });
                }
                if self.check(Token::Dedent) {
                    self.advance();
                }
                Ok(Expr::Receive(arms, sp))
            }
            _ => Err(self.error(&format!("unexpected token: {}", self.peek()))),
        }
    }
}
