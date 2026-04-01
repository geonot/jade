use crate::ast::*;
use crate::lexer::Token;
use crate::types::Type;

use super::{ParseError, Parser};

impl Parser {
    pub(super) fn parse_pat(&mut self) -> Result<Pat, ParseError> {
        let first = self.parse_single_pat()?;
        if self.check(Token::Or) {
            let sp = first.span();
            let mut pats = vec![first];
            while self.check(Token::Or) {
                self.advance();
                pats.push(self.parse_single_pat()?);
            }
            return Ok(Pat::Or(pats, sp));
        }
        Ok(first)
    }

    fn parse_single_pat(&mut self) -> Result<Pat, ParseError> {
        let sp = self.span();
        match self.peek() {
            Token::Ident(ref s) if s == "_" => {
                self.advance();
                Ok(Pat::Wild(sp))
            }
            Token::Ident(_) => {
                let name = self.ident()?;
                if self.check(Token::LParen) {
                    self.advance();
                    let mut sub = Vec::new();
                    while !self.check(Token::RParen) && !self.eof() {
                        sub.push(self.parse_pat()?);
                        if !self.check(Token::RParen) {
                            self.expect(Token::Comma)?;
                        }
                    }
                    self.expect(Token::RParen)?;
                    Ok(Pat::Ctor(name, sub, sp))
                } else {
                    Ok(Pat::Ident(name, sp))
                }
            }
            Token::Int(_) | Token::CharLit(_) | Token::Float(_) | Token::Str(_) | Token::True | Token::False => {
                let lit = self.parse_primary()?;
                if self.check(Token::To) {
                    self.advance();
                    let hi = self.parse_primary()?;
                    Ok(Pat::Range(lit, hi, sp))
                } else {
                    Ok(Pat::Lit(lit))
                }
            }
            Token::LParen => {
                self.advance();
                let mut pats = Vec::new();
                while !self.check(Token::RParen) && !self.eof() {
                    pats.push(self.parse_pat()?);
                    if !self.check(Token::RParen) {
                        self.expect(Token::Comma)?;
                    }
                }
                self.expect(Token::RParen)?;
                Ok(Pat::Tuple(pats, sp))
            }
            Token::LBracket => {
                self.advance();
                let mut pats = Vec::new();
                while !self.check(Token::RBracket) && !self.eof() {
                    pats.push(self.parse_pat()?);
                    if !self.check(Token::RBracket) {
                        self.expect(Token::Comma)?;
                    }
                }
                self.expect(Token::RBracket)?;
                Ok(Pat::Array(pats, sp))
            }
            _ => Err(self.error("expected pattern")),
        }
    }

    pub(super) fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        let e = self.parse_ternary()?;
        if self.check(Token::Query) {
            return self.parse_query_block(e);
        }
        Ok(e)
    }

    fn parse_ternary(&mut self) -> Result<Expr, ParseError> {
        let e = self.parse_pipeline()?;
        if self.check(Token::Question) {
            let sp = self.span();
            self.advance();
            let t = self.parse_expr()?;
            self.expect(Token::Bang)?;
            Ok(Expr::Ternary(
                Box::new(e),
                Box::new(t),
                Box::new(self.parse_expr()?),
                sp,
            ))
        } else {
            Ok(e)
        }
    }

    pub(super) fn parse_pipeline(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_or()?;
        while self.check(Token::Tilde) {
            let sp = self.span();
            self.advance();
            let rhs = self.parse_or()?;
            // If the RHS contains $ but is NOT just a bare $ or a call to a named function,
            // wrap it in an implicit lambda: `$ * 2` → `*fn(__ph) __ph * 2`
            let rhs = if contains_placeholder(&rhs) && !matches!(rhs, Expr::Placeholder(_)) {
                // If RHS is a Call(Ident(name), args) and $ appears only in args,
                // leave it for the typer's existing placeholder substitution.
                let is_named_call_with_ph = matches!(&rhs, Expr::Call(callee, _, _) if matches!(callee.as_ref(), Expr::Ident(_, _)));
                if is_named_call_with_ph {
                    rhs
                } else {
                    let replaced = replace_placeholder(&rhs, "__ph");
                    Expr::Call(
                        Box::new(Expr::Lambda(
                            vec![Param {
                                name: "__ph".into(),
                                ty: None,
                                default: None,
                                literal: None,
                                span: sp,
                            }],
                            None,
                            vec![Stmt::Expr(replaced)],
                            sp,
                        )),
                        vec![],
                        sp,
                    )
                }
            } else {
                rhs
            };
            e = Expr::Pipe(Box::new(e), Box::new(rhs), vec![], sp);
        }
        Ok(e)
    }

    binop!(parse_or,     parse_xor,    { Token::Or => BinOp::Or });
    binop!(parse_xor,    parse_and,    { Token::Xor => BinOp::BitXor });
    binop!(parse_and,    parse_eq,     { Token::And => BinOp::And });
    binop!(parse_eq,     parse_cmp,    { Token::Equals => BinOp::Eq, Token::Neq => BinOp::Ne });

    pub(super) fn parse_cmp(&mut self) -> Result<Expr, ParseError> {
        let mut l = self.parse_bitor()?;
        let mut chained: Option<(Expr, BinOp, Expr)> = None;
        loop {
            let sp = self.span();
            let op = match self.peek() {
                Token::Lt => Some(BinOp::Lt),
                Token::Gt => Some(BinOp::Gt),
                Token::LtEq => Some(BinOp::Le),
                Token::GtEq => Some(BinOp::Ge),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let r = self.parse_bitor()?;
                if let Some((prev_l, prev_op, ref prev_r)) = chained {
                    // Chained comparison: `a < b < c` → `a < b and b < c`
                    let left = Expr::BinOp(Box::new(prev_l), prev_op, Box::new(prev_r.clone()), sp);
                    let right = Expr::BinOp(Box::new(prev_r.clone()), op, Box::new(r.clone()), sp);
                    l = Expr::BinOp(Box::new(left), BinOp::And, Box::new(right), sp);
                    chained = Some((l.clone(), op, r));
                } else {
                    chained = Some((l.clone(), op, r.clone()));
                    l = Expr::BinOp(Box::new(l), op, Box::new(r), sp);
                }
                continue;
            }
            match self.peek() {
                Token::In => {
                    self.advance();
                    let r = self.parse_bitor()?;
                    l = Expr::Method(Box::new(r), "contains".into(), vec![l], sp);
                }
                _ => break,
            }
        }
        Ok(l)
    }

    binop!(parse_bitor,  parse_bitxor, { Token::Pipe => BinOp::BitOr });
    binop!(parse_bitxor, parse_bitand, { Token::Caret => BinOp::BitXor });
    binop!(parse_bitand, parse_shift,  { Token::Ampersand => BinOp::BitAnd });
    binop!(parse_shift,  parse_add,    { Token::Shl => BinOp::Shl, Token::Shr => BinOp::Shr });
    binop!(parse_add,    parse_mul,    { Token::Plus => BinOp::Add, Token::Minus => BinOp::Sub });
    binop!(parse_mul,    parse_exp,    { Token::Star => BinOp::Mul, Token::Slash => BinOp::Div, Token::Percent => BinOp::Mod });

    fn parse_exp(&mut self) -> Result<Expr, ParseError> {
        let l = self.parse_unary()?;
        if self.check(Token::StarStar) {
            let sp = self.span();
            self.advance();
            let r = self.parse_exp()?;
            Ok(Expr::BinOp(Box::new(l), BinOp::Exp, Box::new(r), sp))
        } else {
            Ok(l)
        }
    }

    pub(super) fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        let sp = self.span();
        match self.peek() {
            Token::Minus => {
                self.advance();
                Ok(Expr::UnaryOp(
                    UnaryOp::Neg,
                    Box::new(self.parse_unary()?),
                    sp,
                ))
            }
            Token::Not => {
                self.advance();
                Ok(Expr::UnaryOp(
                    UnaryOp::Not,
                    Box::new(self.parse_unary()?),
                    sp,
                ))
            }
            Token::Tilde => {
                self.advance();
                Ok(Expr::UnaryOp(
                    UnaryOp::BitNot,
                    Box::new(self.parse_unary()?),
                    sp,
                ))
            }
            Token::At => {
                self.advance();
                Ok(Expr::Deref(Box::new(self.parse_unary()?), sp))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_primary()?;
        loop {
            match self.peek() {
                Token::Dot => {
                    let sp = self.span();
                    self.advance();
                    let f = self.ident()?;
                    if self.check(Token::LParen) {
                        self.advance();
                        let a = self.parse_args()?;
                        self.expect(Token::RParen)?;
                        e = Expr::Method(Box::new(e), f, a, sp);
                    } else {
                        e = Expr::Field(Box::new(e), f, sp);
                    }
                }
                Token::LBracket => {
                    let sp = self.span();
                    self.advance();
                    let idx = self.parse_expr()?;
                    self.expect(Token::RBracket)?;
                    e = Expr::Index(Box::new(e), Box::new(idx), sp);
                }
                Token::AtKw => {
                    let sp = self.span();
                    self.advance();
                    let idx = self.parse_unary()?;
                    e = Expr::Index(Box::new(e), Box::new(idx), sp);
                }
                Token::LParen => {
                    let sp = self.span();
                    self.advance();
                    let a = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    e = Expr::Call(Box::new(e), a, sp);
                }
                Token::As => {
                    let sp = self.span();
                    self.advance();
                    // `as strict T` → strict narrowing cast
                    if self.check(Token::Strict) {
                        self.advance();
                        e = Expr::StrictCast(Box::new(e), self.parse_type()?, sp);
                    } else if matches!(self.peek(), Token::Ident(ref s) if s == "json" || s == "map") {
                        if let Token::Ident(fmt) = self.peek() {
                            self.advance();
                            e = Expr::AsFormat(Box::new(e), fmt, sp);
                        }
                    } else {
                        e = Expr::As(Box::new(e), self.parse_type()?, sp);
                    }
                }
                Token::From => {
                    let sp = self.span();
                    self.advance();
                    let start = self.parse_cmp()?;
                    self.expect(Token::To)?;
                    let end = self.parse_cmp()?;
                    e = Expr::Slice(Box::new(e), Box::new(start), Box::new(end), sp);
                }
                Token::Ident(ref kw) if kw == "where" => {
                    if let Expr::Ident(ref store_name, sp) = e {
                        let store = store_name.clone();
                        let filter = self.parse_store_filter()?;
                        e = Expr::StoreQuery(store, Box::new(filter), sp);
                    } else {
                        break;
                    }
                }
                Token::By if !self.suppress_by => {
                    let sp = e.span();
                    let mut dims = vec![e.clone()];
                    while self.check(Token::By) {
                        self.advance();
                        dims.push(self.parse_primary()?);
                    }
                    e = Expr::NDArray(dims, sp);
                }
                Token::Of if matches!(e, Expr::Ident(_, _) | Expr::Lambda(..)) => {
                    let sp = self.span();
                    self.advance();
                    let arg = self.parse_primary()?;
                    e = Expr::OfCall(Box::new(e), Box::new(arg), sp);
                }
                _ => break,
            }
        }
        Ok(e)
    }

    pub(super) fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let sp = self.span();
        match self.peek() {
            Token::Int(n) => {
                self.advance();
                Ok(Expr::Int(n, sp))
            }
            Token::CharLit(n) => {
                self.advance();
                Ok(Expr::Int(n, sp))
            }
            Token::Float(n) => {
                self.advance();
                Ok(Expr::Float(n, sp))
            }
            Token::Str(ref s) => {
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
                if self.check(Token::RBracket) {
                    self.advance();
                    return Ok(Expr::Array(Vec::new(), sp));
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
                        bind,
                        Box::new(iter_start),
                        iter_end,
                        cond,
                        sp,
                    ));
                }
                let mut v = vec![first];
                while self.check(Token::Comma) {
                    self.advance();
                    if self.check(Token::RBracket) {
                        break;
                    }
                    v.push(self.parse_expr()?);
                }
                self.expect(Token::RBracket)?;
                Ok(Expr::Array(v, sp))
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
                    Token::Str(ref s) => {
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
            Token::Deque => {
                self.advance();
                self.expect(Token::LParen)?;
                let mut elems = Vec::new();
                while !self.check(Token::RParen) && !self.eof() {
                    elems.push(self.parse_expr()?);
                    if !self.check(Token::RParen) {
                        self.expect(Token::Comma)?;
                    }
                }
                self.expect(Token::RParen)?;
                Ok(Expr::Deque(elems, sp))
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
                    Token::Str(ref s) => {
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
                // If followed by '(' treat as function call, not builder syntax
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
                        Ok(BuilderField { name: fname, value: fval, span: fsp })
                    })?;
                    Ok(Expr::Builder(name, fields, sp))
                }
            }
            Token::Ident(ref name) => {
                let name = name.clone();
                self.advance();
                if name == "SIMD" && self.check(Token::Of) {
                    self.advance();
                    let elem_ty = self.parse_type()?;
                    self.expect(Token::Comma)?;
                    let lanes = match self.peek() {
                        Token::Int(n) => { self.advance(); n as usize }
                        _ => return Err(self.error("expected lane count after SIMD of <type>,")),
                    };
                    self.expect(Token::LParen)?;
                    let mut elems = Vec::new();
                    while !self.check(Token::RParen) && !self.eof() {
                        elems.push(self.parse_expr()?);
                        if !self.check(Token::RParen) {
                            self.expect(Token::Comma)?;
                        }
                    }
                    self.expect(Token::RParen)?;
                    return Ok(Expr::SIMDLit(elem_ty, lanes, elems, sp));
                }
                if name.starts_with(|c: char| c.is_uppercase()) && self.check(Token::LParen) {
                    self.advance();
                    let mut fields = Vec::new();
                    while !self.check(Token::RParen) && !self.eof() {
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
                        if !self.check(Token::RParen) {
                            self.expect(Token::Comma)?;
                        }
                    }
                    self.expect(Token::RParen)?;
                    return Ok(Expr::Struct(name, fields, sp));
                }
                if name == "count" {
                    if let Token::Ident(_) = self.peek() {
                        let store = self.ident()?;
                        return Ok(Expr::StoreCount(store, sp));
                    }
                }
                if name == "all" {
                    if let Token::Ident(_) = self.peek() {
                        let store = self.ident()?;
                        return Ok(Expr::StoreAll(store, sp));
                    }
                }
                Ok(Expr::Ident(name, sp))
            }
            Token::Star
                if self.pos + 1 < self.tok.len()
                    && matches!(self.tok[self.pos + 1].token, Token::Fn) =>
            {
                self.advance();
                self.advance();
                self.expect(Token::LParen)?;
                let mut params = Vec::new();
                while !self.check(Token::RParen) && !self.eof() {
                    params.push(self.parse_param(true)?);
                    if !self.check(Token::RParen) {
                        self.expect(Token::Comma)?;
                    }
                }
                self.expect(Token::RParen)?;
                let ret = if self.check(Token::Arrow) || self.check(Token::Returns) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let body = if self.check(Token::Do) {
                    self.advance();
                    self.skip_nl();
                    let mut stmts = Vec::new();
                    while !self.check(Token::End) && !self.eof() {
                        self.skip_nl();
                        if self.check(Token::End) || self.eof() {
                            break;
                        }
                        if self.check(Token::Indent) || self.check(Token::Dedent) {
                            self.advance();
                            continue;
                        }
                        stmts.push(self.parse_stmt()?);
                        self.skip_nl();
                    }
                    self.expect(Token::End)?;
                    stmts
                } else {
                    vec![Stmt::Expr(self.parse_expr()?)]
                };
                Ok(Expr::Lambda(params, ret, body, sp))
            }
            Token::Dollar => {
                self.advance();
                Ok(Expr::Placeholder(sp))
            }
            Token::Percent => {
                self.advance();
                let inner = self.parse_primary()?;
                Ok(Expr::Ref(Box::new(inner), sp))
            }
            Token::Spawn => {
                self.advance();
                let name = self.ident()?;
                Ok(Expr::Spawn(name, sp))
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
                            return Ok(Expr::DispatchBlock("__anon".to_string(), body, sp));
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
                    Ok(Expr::Send(Box::new(target), handler, args, sp))
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

    pub(super) fn parse_literal_token(&mut self) -> Result<Expr, ParseError> {
        let sp = self.span();
        match self.peek() {
            Token::Int(n) => {
                self.advance();
                Ok(Expr::Int(n, sp))
            }
            Token::CharLit(n) => {
                self.advance();
                Ok(Expr::Int(n, sp))
            }
            Token::Float(n) => {
                self.advance();
                Ok(Expr::Float(n, sp))
            }
            Token::True => {
                self.advance();
                Ok(Expr::Bool(true, sp))
            }
            Token::False => {
                self.advance();
                Ok(Expr::Bool(false, sp))
            }
            Token::Str(ref s) => {
                let v = s.clone();
                self.advance();
                Ok(Expr::Str(v, sp))
            }
            _ => Err(self.error("expected literal")),
        }
    }

    fn parse_query_block(&mut self, source: Expr) -> Result<Expr, ParseError> {
        let sp = source.span();
        self.expect(Token::Query)?;
        self.expect(Token::Newline)?;
        let clauses = self.parse_indented(Self::parse_query_clause)?;
        Ok(Expr::Query(Box::new(source), clauses, sp))
    }

    fn parse_query_clause(&mut self) -> Result<QueryClause, ParseError> {
        let sp = self.span();
        let kw = self.ident()?;
        match kw.as_str() {
            "where" => {
                let cond = self.parse_expr()?;
                Ok(QueryClause::Where(cond, sp))
            }
            "limit" => {
                let n = self.parse_expr()?;
                Ok(QueryClause::Limit(n, sp))
            }
            "sort" => {
                let field = self.ident()?;
                let asc = if let Token::Ident(ref dir) = self.peek() {
                    if dir == "desc" {
                        self.advance();
                        false
                    } else if dir == "asc" {
                        self.advance();
                        true
                    } else {
                        true
                    }
                } else {
                    true
                };
                Ok(QueryClause::Sort(field, asc, sp))
            }
            "take" => {
                let n = self.parse_expr()?;
                Ok(QueryClause::Take(n, sp))
            }
            "skip" => {
                let n = self.parse_expr()?;
                Ok(QueryClause::Skip(n, sp))
            }
            "set" => {
                let field = self.ident()?;
                self.expect(Token::Is)?;
                let val = self.parse_expr()?;
                Ok(QueryClause::Set(field, val, sp))
            }
            "delete" => Ok(QueryClause::Delete(sp)),
            _ => Err(self.error(&format!("unknown query clause: {kw}"))),
        }
    }

    fn parse_builtin_call(&mut self, name: &str, sp: Span) -> Result<Expr, ParseError> {
        let arg = if self.check(Token::LParen) {
            self.advance();
            let a = self.parse_expr()?;
            self.expect(Token::RParen)?;
            a
        } else {
            self.parse_expr()?
        };
        Ok(Expr::Call(
            Box::new(Expr::Ident(name.into(), sp)),
            vec![arg],
            sp,
        ))
    }

    fn is_field_init(&self) -> bool {
        matches!(self.peek(), Token::Ident(_))
            && self.pos + 1 < self.tok.len()
            && matches!(self.tok[self.pos + 1].token, Token::Is)
    }

    fn is_named_arg(&self) -> bool {
        matches!(self.peek(), Token::Ident(_))
            && self.pos + 1 < self.tok.len()
            && matches!(self.tok[self.pos + 1].token, Token::Is)
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut a = Vec::new();
        while !self.check(Token::RParen) && !self.eof() {
            if self.check(Token::DotDotDot) {
                let sp = self.span();
                self.advance();
                let e = self.parse_expr()?;
                a.push(Expr::Spread(Box::new(e), sp));
            } else if self.is_named_arg() {
                let sp = self.span();
                let name = self.ident()?;
                self.expect(Token::Is)?;
                let val = self.parse_expr()?;
                a.push(Expr::NamedArg(name, Box::new(val), sp));
            } else {
                a.push(self.parse_expr()?);
            }
            if !self.check(Token::RParen) {
                self.expect(Token::Comma)?;
            }
        }
        Ok(a)
    }

    pub(super) fn parse_type(&mut self) -> Result<Type, ParseError> {
        match self.peek() {
            Token::Percent => {
                self.advance();
                let inner = self.parse_type()?;
                Ok(Type::Ptr(Box::new(inner)))
            }
            Token::Ident(ref n) => {
                if n == "dyn" {
                    self.advance();
                    if let Token::Ident(ref trait_name) = self.peek() {
                        let name = trait_name.clone();
                        self.advance();
                        return Ok(Type::DynTrait(name));
                    }
                    return Err(self.error("expected trait name after 'dyn'"));
                }
                let t = self.ident_to_type(n);
                self.advance();
                if self.check(Token::Of) {
                    if let Type::Struct(name, _) = t {
                        self.advance();
                        let arg = self.parse_type()?;
                        if name == "Vec" {
                            return Ok(Type::Vec(Box::new(arg)));
                        }
                        let mangled = format!("{name}_{arg}");
                        Ok(Type::Struct(mangled, vec![]))
                    } else {
                        Ok(t)
                    }
                } else {
                    Ok(t)
                }
            }
            Token::LParen => {
                self.advance();
                let mut params = Vec::new();
                while !self.check(Token::RParen) && !self.eof() {
                    params.push(self.parse_type()?);
                    if !self.check(Token::RParen) {
                        self.expect(Token::Comma)?;
                    }
                }
                self.expect(Token::RParen)?;
                if self.check(Token::Arrow) || self.check(Token::Returns) {
                    self.advance();
                } else {
                    self.expect(Token::Arrow)?;
                }
                let ret = self.parse_type()?;
                Ok(Type::Fn(params, Box::new(ret)))
            }
            _ => Err(self.error("expected type")),
        }
    }

    pub(super) fn ident_to_type(&self, n: &str) -> Type {
        match n {
            "i8" => Type::I8,
            "i16" => Type::I16,
            "i32" => Type::I32,
            "i64" => Type::I64,
            "u8" => Type::U8,
            "u16" => Type::U16,
            "u32" => Type::U32,
            "u64" => Type::U64,
            "f32" => Type::F32,
            "f64" => Type::F64,
            "bool" => Type::Bool,
            "void" => Type::Void,
            "String" => Type::String,
            s if s.len() == 1 && s.chars().next().map_or(false, |c| c.is_uppercase()) => {
                Type::Param(s.to_string())
            }
            _ => Type::Struct(n.to_string(), vec![]),
        }
    }

    fn parse_interp(&mut self, first: String, sp: Span) -> Result<Expr, ParseError> {
        let mut result: Expr = Expr::Str(first, sp);
        while self.check(Token::InterpStart) {
            self.advance();
            let expr = self.parse_expr()?;
            if !self.check(Token::InterpEnd) {
                return Err(self.error("expected closing } in string interpolation"));
            }
            self.advance();
            let interp_expr = Expr::Call(
                Box::new(Expr::Ident("to_string".into(), expr.span())),
                vec![expr],
                sp,
            );
            result = Expr::BinOp(Box::new(result), BinOp::Add, Box::new(interp_expr), sp);
            if let Token::Str(ref s) = self.peek() {
                let tail = s.clone();
                self.advance();
                if !tail.is_empty() {
                    result = Expr::BinOp(
                        Box::new(result),
                        BinOp::Add,
                        Box::new(Expr::Str(tail, sp)),
                        sp,
                    );
                }
            }
        }
        Ok(result)
    }
}

/// Check if an AST expression contains `$` (Placeholder) anywhere.
pub(super) fn contains_placeholder(expr: &Expr) -> bool {
    match expr {
        Expr::Placeholder(_) => true,
        Expr::BinOp(l, _, r, _) => contains_placeholder(l) || contains_placeholder(r),
        Expr::UnaryOp(_, e, _) => contains_placeholder(e),
        Expr::Call(f, args, _) => {
            contains_placeholder(f) || args.iter().any(contains_placeholder)
        }
        Expr::Method(obj, _, args, _) => {
            contains_placeholder(obj) || args.iter().any(contains_placeholder)
        }
        Expr::Field(e, _, _) => contains_placeholder(e),
        Expr::Index(a, b, _) => contains_placeholder(a) || contains_placeholder(b),
        Expr::Ternary(a, b, c, _) => {
            contains_placeholder(a) || contains_placeholder(b) || contains_placeholder(c)
        }
        Expr::As(e, _, _) => contains_placeholder(e),
        Expr::Ref(e, _) => contains_placeholder(e),
        Expr::Deref(e, _) => contains_placeholder(e),
        Expr::Array(elems, _) => elems.iter().any(contains_placeholder),
        Expr::Tuple(elems, _) => elems.iter().any(contains_placeholder),
        Expr::Pipe(l, r, _, _) => contains_placeholder(l) || contains_placeholder(r),
        _ => false,
    }
}

/// Replace all `$` (Placeholder) in an expression with `Ident(name)`.
pub(super) fn replace_placeholder(expr: &Expr, name: &str) -> Expr {
    match expr {
        Expr::Placeholder(sp) => Expr::Ident(name.into(), *sp),
        Expr::BinOp(l, op, r, sp) => Expr::BinOp(
            Box::new(replace_placeholder(l, name)),
            *op,
            Box::new(replace_placeholder(r, name)),
            *sp,
        ),
        Expr::UnaryOp(op, e, sp) => {
            Expr::UnaryOp(*op, Box::new(replace_placeholder(e, name)), *sp)
        }
        Expr::Call(f, args, sp) => Expr::Call(
            Box::new(replace_placeholder(f, name)),
            args.iter().map(|a| replace_placeholder(a, name)).collect(),
            *sp,
        ),
        Expr::Method(obj, m, args, sp) => Expr::Method(
            Box::new(replace_placeholder(obj, name)),
            m.clone(),
            args.iter().map(|a| replace_placeholder(a, name)).collect(),
            *sp,
        ),
        Expr::Field(e, f, sp) => {
            Expr::Field(Box::new(replace_placeholder(e, name)), f.clone(), *sp)
        }
        Expr::Index(a, b, sp) => Expr::Index(
            Box::new(replace_placeholder(a, name)),
            Box::new(replace_placeholder(b, name)),
            *sp,
        ),
        Expr::Ternary(a, b, c, sp) => Expr::Ternary(
            Box::new(replace_placeholder(a, name)),
            Box::new(replace_placeholder(b, name)),
            Box::new(replace_placeholder(c, name)),
            *sp,
        ),
        Expr::As(e, t, sp) => Expr::As(Box::new(replace_placeholder(e, name)), t.clone(), *sp),
        Expr::Ref(e, sp) => Expr::Ref(Box::new(replace_placeholder(e, name)), *sp),
        Expr::Deref(e, sp) => Expr::Deref(Box::new(replace_placeholder(e, name)), *sp),
        Expr::Array(elems, sp) => Expr::Array(
            elems.iter().map(|e| replace_placeholder(e, name)).collect(),
            *sp,
        ),
        Expr::Tuple(elems, sp) => Expr::Tuple(
            elems.iter().map(|e| replace_placeholder(e, name)).collect(),
            *sp,
        ),
        other => other.clone(),
    }
}

/// Check if any statement in a block contains `$`.
pub(super) fn contains_placeholder_in_block(block: &[Stmt]) -> bool {
    block.iter().any(|s| contains_placeholder_in_stmt(s))
}

fn contains_placeholder_in_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Expr(e) => contains_placeholder(e),
        Stmt::Bind(b) => contains_placeholder(&b.value),
        Stmt::Assign(lhs, rhs, _) => contains_placeholder(lhs) || contains_placeholder(rhs),
        Stmt::If(i) => {
            contains_placeholder(&i.cond)
                || i.then.iter().any(|s| contains_placeholder_in_stmt(s))
                || i.elifs
                    .iter()
                    .any(|(c, b)| contains_placeholder(c) || b.iter().any(|s| contains_placeholder_in_stmt(s)))
                || i.els
                    .as_ref()
                    .map_or(false, |b| b.iter().any(|s| contains_placeholder_in_stmt(s)))
        }
        Stmt::While(w) => {
            contains_placeholder(&w.cond) || w.body.iter().any(|s| contains_placeholder_in_stmt(s))
        }
        Stmt::For(f) => {
            contains_placeholder(&f.iter) || f.body.iter().any(|s| contains_placeholder_in_stmt(s))
        }
        Stmt::SimFor(f, _) => {
            contains_placeholder(&f.iter) || f.body.iter().any(|s| contains_placeholder_in_stmt(s))
        }
        Stmt::Loop(l) => l.body.iter().any(|s| contains_placeholder_in_stmt(s)),
        Stmt::Ret(Some(e), _) => contains_placeholder(e),
        Stmt::Break(Some(e), _) => contains_placeholder(e),
        Stmt::Match(m) => {
            contains_placeholder(&m.subject)
                || m.arms
                    .iter()
                    .any(|a| a.body.iter().any(|s| contains_placeholder_in_stmt(s)))
        }
        _ => false,
    }
}

/// Replace all `$` in a block with `Ident(name)`.
pub(super) fn replace_placeholder_in_block(block: &[Stmt], name: &str) -> Vec<Stmt> {
    block.iter().map(|s| replace_placeholder_in_stmt(s, name)).collect()
}

fn replace_placeholder_in_stmt(stmt: &Stmt, name: &str) -> Stmt {
    match stmt {
        Stmt::Expr(e) => Stmt::Expr(replace_placeholder(e, name)),
        Stmt::Bind(b) => Stmt::Bind(Bind {
            name: b.name.clone(),
            value: replace_placeholder(&b.value, name),
            ty: b.ty.clone(),
            span: b.span,
        }),
        Stmt::Assign(lhs, rhs, sp) => {
            Stmt::Assign(replace_placeholder(lhs, name), replace_placeholder(rhs, name), *sp)
        }
        Stmt::If(i) => Stmt::If(If {
            cond: replace_placeholder(&i.cond, name),
            then: replace_placeholder_in_block(&i.then, name),
            elifs: i
                .elifs
                .iter()
                .map(|(c, b)| (replace_placeholder(c, name), replace_placeholder_in_block(b, name)))
                .collect(),
            els: i.els.as_ref().map(|b| replace_placeholder_in_block(b, name)),
            span: i.span,
        }),
        Stmt::While(w) => Stmt::While(While {
            cond: replace_placeholder(&w.cond, name),
            body: replace_placeholder_in_block(&w.body, name),
            span: w.span,
        }),
        Stmt::For(f) => Stmt::For(For {
            label: f.label.clone(),
            bind: f.bind.clone(),
            bind2: f.bind2.clone(),
            iter: replace_placeholder(&f.iter, name),
            end: f.end.as_ref().map(|e| replace_placeholder(e, name)),
            step: f.step.as_ref().map(|e| replace_placeholder(e, name)),
            body: replace_placeholder_in_block(&f.body, name),
            span: f.span,
        }),
        Stmt::Loop(l) => Stmt::Loop(Loop {
            body: replace_placeholder_in_block(&l.body, name),
            span: l.span,
        }),
        Stmt::Ret(val, sp) => Stmt::Ret(val.as_ref().map(|e| replace_placeholder(e, name)), *sp),
        Stmt::Break(val, sp) => {
            Stmt::Break(val.as_ref().map(|e| replace_placeholder(e, name)), *sp)
        }
        Stmt::Match(m) => Stmt::Match(Match {
            subject: replace_placeholder(&m.subject, name),
            arms: m
                .arms
                .iter()
                .map(|a| Arm {
                    pat: a.pat.clone(),
                    guard: a.guard.as_ref().map(|e| replace_placeholder(e, name)),
                    body: replace_placeholder_in_block(&a.body, name),
                    span: a.span,
                })
                .collect(),
            span: m.span,
        }),
        other => other.clone(),
    }
}
