use crate::ast::*;
use crate::lexer::{Spanned, Token};
use crate::types::Type;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("line {line}:{col}: {msg}")]
    Error { line: u32, col: u32, msg: String },
}

pub struct Parser {
    tok: Vec<Spanned>,
    pos: usize,
}

macro_rules! binop {
    ($name:ident, $next:ident, { $($t:path => $op:expr),+ $(,)? }) => {
        fn $name(&mut self) -> Result<Expr, ParseError> {
            let mut l = self.$next()?;
            loop { let sp = self.span(); match self.peek() {
                $($t => { self.advance(); let r = self.$next()?;
                    l = Expr::BinOp(Box::new(l), $op, Box::new(r), sp); })+
                _ => break,
            }} Ok(l)
        }
    };
}

impl Parser {
    pub fn new(tok: Vec<Spanned>) -> Self {
        Self { tok, pos: 0 }
    }

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut decls = Vec::new();
        while !self.eof() {
            self.skip_nl();
            if self.eof() {
                break;
            }
            decls.push(self.parse_decl()?);
        }
        let mut prog = Program { decls };
        desugar_multi_clause_fns(&mut prog);
        Ok(prog)
    }

    fn parse_decl(&mut self) -> Result<Decl, ParseError> {
        match self.peek() {
            Token::Star => Ok(Decl::Fn(self.parse_fn()?)),
            Token::Type | Token::Pub => Ok(Decl::Type(self.parse_type_def()?)),
            Token::Enum => Ok(Decl::Enum(self.parse_enum_def()?)),
            Token::Extern => Ok(Decl::Extern(self.parse_extern()?)),
            Token::Use => Ok(Decl::Use(self.parse_use_decl()?)),
            Token::Err => Ok(Decl::ErrDef(self.parse_err_def()?)),
            Token::Test => Ok(Decl::Test(self.parse_test_block()?)),
            Token::Actor => Ok(Decl::Actor(self.parse_actor_def()?)),
            Token::Store => Ok(Decl::Store(self.parse_store_def()?)),
            Token::Trait => Ok(Decl::Trait(self.parse_trait_def()?)),
            Token::Impl => Ok(Decl::Impl(self.parse_impl_block()?)),
            _ => Err(self.error("expected *, type, enum, extern, use, err, test, actor, store, trait, or impl")),
        }
    }

    fn parse_type_params(&mut self) -> (Vec<String>, Vec<(String, Vec<String>)>) {
        let mut tp = Vec::new();
        let mut bounds = Vec::new();
        if self.check(Token::Of) {
            self.advance();
            if let Token::Ident(_) = self.peek() {
                let name = self.ident().unwrap();
                if self.check(Token::Colon) {
                    self.advance();
                    let mut bs = vec![self.ident().unwrap_or_default()];
                    while self.check(Token::Plus) {
                        self.advance();
                        bs.push(self.ident().unwrap_or_default());
                    }
                    bounds.push((name.clone(), bs));
                }
                tp.push(name);
                while self.check(Token::Comma) {
                    self.advance();
                    if let Token::Ident(_) = self.peek() {
                        let name = self.ident().unwrap();
                        if self.check(Token::Colon) {
                            self.advance();
                            let mut bs = vec![self.ident().unwrap_or_default()];
                            while self.check(Token::Plus) {
                                self.advance();
                                bs.push(self.ident().unwrap_or_default());
                            }
                            bounds.push((name.clone(), bs));
                        }
                        tp.push(name);
                    }
                }
            }
        }
        (tp, bounds)
    }

    fn parse_extern(&mut self) -> Result<ExternFn, ParseError> {
        let sp = self.span();
        self.expect(Token::Extern)?;
        self.expect(Token::Star)?;
        let name = self.ident()?;
        self.expect(Token::LParen)?;
        let mut params = Vec::new();
        let mut variadic = false;
        while !self.check(Token::RParen) && !self.eof() {
            if self.check(Token::Dot) {
                self.advance();
                self.expect(Token::Dot)?;
                self.expect(Token::Dot)?;
                variadic = true;
                break;
            }
            let pname = self.ident()?;
            self.expect(Token::Colon)?;
            let pty = self.parse_type()?;
            params.push((pname, pty));
            if !self.check(Token::RParen) && !variadic {
                self.expect(Token::Comma)?;
            }
        }
        self.expect(Token::RParen)?;
        let ret = if self.check(Token::Arrow) {
            self.advance();
            self.parse_type()?
        } else {
            Type::Void
        };
        self.skip_nl();
        Ok(ExternFn {
            name,
            params,
            ret,
            variadic,
            span: sp,
        })
    }

    fn parse_fn(&mut self) -> Result<Fn, ParseError> {
        let sp = self.span();
        self.expect(Token::Star)?;
        let name = self.ident()?;
        let (type_params, type_bounds) = self.parse_type_params();
        let mut params = Vec::new();

        if self.check(Token::LParen) {
            self.advance();
            let mut arg_idx = 0u32;
            while !self.check(Token::RParen) && !self.eof() {
                match self.peek() {
                    Token::Int(_) | Token::Float(_) | Token::True | Token::False | Token::Str(_) => {
                        let lit_sp = self.span();
                        let lit_expr = self.parse_literal_token()?;
                        params.push(Param {
                            name: format!("__arg{arg_idx}"),
                            ty: None,
                            default: None,
                            literal: Some(lit_expr),
                            span: lit_sp,
                        });
                    }
                    Token::Minus => {
                        let lit_sp = self.span();
                        let lit_expr = self.parse_unary()?;
                        params.push(Param {
                            name: format!("__arg{arg_idx}"),
                            ty: None,
                            default: None,
                            literal: Some(lit_expr),
                            span: lit_sp,
                        });
                    }
                    _ => {
                        params.push(self.parse_param(true)?);
                    }
                }
                arg_idx += 1;
                if !self.check(Token::RParen) {
                    self.expect(Token::Comma)?;
                }
            }
            self.expect(Token::RParen)?;
        } else {
            while !self.check(Token::Newline) && !self.check(Token::Arrow) && !self.check(Token::Is) && !self.eof() {
                match self.peek() {
                    Token::Int(_) | Token::Float(_) | Token::True | Token::False | Token::Str(_) => {
                        let lit_sp = self.span();
                        let lit_expr = self.parse_literal_token()?;
                        params.push(Param {
                            name: format!("__arg{}", params.len()),
                            ty: None,
                            default: None,
                            literal: Some(lit_expr),
                            span: lit_sp,
                        });
                    }
                    Token::Minus => {
                        let lit_sp = self.span();
                        let lit_expr = self.parse_unary()?;
                        params.push(Param {
                            name: format!("__arg{}", params.len()),
                            ty: None,
                            default: None,
                            literal: Some(lit_expr),
                            span: lit_sp,
                        });
                    }
                    _ => {
                        params.push(self.parse_param(false)?);
                    }
                }
                if self.check(Token::Comma) {
                    self.advance();
                }
            }
        }

        let ret = if self.check(Token::Arrow) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = if self.check(Token::Is) {
            self.advance();
            let expr = self.parse_expr()?;
            vec![Stmt::Expr(expr)]
        } else {
            self.expect(Token::Newline)?;
            self.parse_block()?
        };

        Ok(Fn {
            name,
            type_params,
            type_bounds,
            params,
            ret,
            body,
            span: sp,
        })
    }

    fn parse_param(&mut self, typed: bool) -> Result<Param, ParseError> {
        let sp = self.span();
        let name = self.ident()?;
        let ty = if typed && self.check(Token::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        let default = if typed && self.check(Token::Is) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Param {
            name,
            ty,
            default,
            literal: None,
            span: sp,
        })
    }

    fn parse_type_def(&mut self) -> Result<TypeDef, ParseError> {
        let sp = self.span();
        if self.check(Token::Pub) {
            self.advance();
        }
        self.expect(Token::Type)?;
        let name = self.ident()?;
        let (type_params, _) = self.parse_type_params();
        let layout = self.parse_layout_attrs()?;
        self.expect(Token::Newline)?;
        let (mut fields, mut methods) = (Vec::new(), Vec::new());
        if self.check(Token::Indent) {
            self.advance();
            while !self.check(Token::Dedent) && !self.eof() {
                self.skip_nl();
                if self.check(Token::Dedent) || self.eof() {
                    break;
                }
                if self.check(Token::Star) {
                    methods.push(self.parse_fn()?);
                } else {
                    fields.push(self.parse_field()?);
                    self.skip_nl();
                }
            }
            if self.check(Token::Dedent) {
                self.advance();
            }
        }
        Ok(TypeDef {
            name,
            type_params,
            fields,
            methods,
            layout,
            span: sp,
        })
    }

    fn parse_layout_attrs(&mut self) -> Result<crate::ast::LayoutAttrs, ParseError> {
        let mut layout = crate::ast::LayoutAttrs::default();
        while self.check(Token::At) {
            self.advance();
            let attr = self.ident()?;
            match attr.as_str() {
                "packed" => layout.packed = true,
                "strict" => layout.strict = true,
                "align" => {
                    self.expect(Token::LParen)?;
                    let n = match self.peek() {
                        Token::Int(n) => {
                            let v = n as u32;
                            self.advance();
                            v
                        }
                        _ => return Err(self.error("expected alignment value")),
                    };
                    self.expect(Token::RParen)?;
                    if !n.is_power_of_two() {
                        return Err(self.error("alignment must be a power of 2"));
                    }
                    layout.align = Some(n);
                }
                _ => return Err(self.error(&format!("unknown layout attribute: @{attr}"))),
            }
        }
        Ok(layout)
    }

    fn parse_field(&mut self) -> Result<Field, ParseError> {
        let sp = self.span();
        let name = self.ident()?;
        let ty = if self.check(Token::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        let default = if self.check(Token::Is) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Field {
            name,
            ty,
            default,
            span: sp,
        })
    }

    fn parse_enum_def(&mut self) -> Result<EnumDef, ParseError> {
        let sp = self.span();
        self.expect(Token::Enum)?;
        let name = self.ident()?;
        let (type_params, _) = self.parse_type_params();
        self.expect(Token::Newline)?;
        let mut variants = Vec::new();
        if self.check(Token::Indent) {
            self.advance();
            while !self.check(Token::Dedent) && !self.eof() {
                self.skip_nl();
                if self.check(Token::Dedent) || self.eof() {
                    break;
                }
                variants.push(self.parse_variant()?);
                self.skip_nl();
            }
            if self.check(Token::Dedent) {
                self.advance();
            }
        }
        Ok(EnumDef {
            name,
            type_params,
            variants,
            span: sp,
        })
    }

    fn parse_variant(&mut self) -> Result<Variant, ParseError> {
        let sp = self.span();
        let name = self.ident()?;
        let mut fields = Vec::new();
        if self.check(Token::LParen) {
            self.advance();
            while !self.check(Token::RParen) && !self.eof() {
                fields.push(self.parse_vfield()?);
                if !self.check(Token::RParen) {
                    self.expect(Token::Comma)?;
                }
            }
            self.expect(Token::RParen)?;
        }
        Ok(Variant {
            name,
            fields,
            span: sp,
        })
    }

    fn parse_vfield(&mut self) -> Result<VField, ParseError> {
        let n = self.ident()?;
        if self.check(Token::As) {
            self.advance();
            Ok(VField {
                name: Some(n),
                ty: self.parse_type()?,
            })
        } else {
            Ok(VField {
                name: None,
                ty: self.ident_to_type(&n),
            })
        }
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        self.expect(Token::Indent)?;
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
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek() {
            Token::If => {
                let i = self.parse_if()?;
                Ok(Stmt::If(i))
            }
            Token::While => self.parse_while(),
            Token::For => self.parse_for(),
            Token::Loop => {
                let sp = self.span();
                self.advance();
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
                    | Token::PercentEq
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
        match self.peek() {
            Token::PlusEq => {
                self.advance();
                Some(BinOp::Add)
            }
            Token::MinusEq => {
                self.advance();
                Some(BinOp::Sub)
            }
            Token::StarEq => {
                self.advance();
                Some(BinOp::Mul)
            }
            Token::SlashEq => {
                self.advance();
                Some(BinOp::Div)
            }
            Token::PercentEq => {
                self.advance();
                Some(BinOp::Mod)
            }
            Token::AmpEq => {
                self.advance();
                Some(BinOp::BitAnd)
            }
            Token::PipeEq => {
                self.advance();
                Some(BinOp::BitOr)
            }
            Token::CaretEq => {
                self.advance();
                Some(BinOp::BitXor)
            }
            Token::ShlEq => {
                self.advance();
                Some(BinOp::Shl)
            }
            Token::ShrEq => {
                self.advance();
                Some(BinOp::Shr)
            }
            _ => None,
        }
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
        Ok(Stmt::Bind(Bind {
            name,
            value: self.parse_expr()?,
            ty: None,
            span: sp,
        }))
    }

    fn parse_if(&mut self) -> Result<If, ParseError> {
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
        self.expect(Token::In)?;
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
        Ok(Stmt::For(For {
            bind,
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
        self.expect(Token::Indent)?;
        let mut arms = Vec::new();
        while !self.check(Token::Dedent) && !self.eof() {
            self.skip_nl();
            if self.check(Token::Dedent) || self.eof() {
                break;
            }
            arms.push(self.parse_arm()?);
            self.skip_nl();
        }
        if self.check(Token::Dedent) {
            self.advance();
        }
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

    fn parse_use_decl(&mut self) -> Result<UseDecl, ParseError> {
        let sp = self.span();
        self.expect(Token::Use)?;
        let mut path = vec![self.ident()?];
        while self.check(Token::Dot) {
            self.advance();
            path.push(self.ident()?);
        }
        Ok(UseDecl { path, span: sp })
    }

    fn parse_err_def(&mut self) -> Result<ErrDef, ParseError> {
        let sp = self.span();
        self.expect(Token::Err)?;
        let name = self.ident()?;
        self.expect(Token::Newline)?;
        self.expect(Token::Indent)?;
        let mut variants = Vec::new();
        while !self.check(Token::Dedent) && !self.eof() {
            self.skip_nl();
            if self.check(Token::Dedent) || self.eof() {
                break;
            }
            let vsp = self.span();
            let vname = self.ident()?;
            let mut fields = Vec::new();
            if self.check(Token::LParen) {
                self.advance();
                while !self.check(Token::RParen) && !self.eof() {
                    fields.push(self.parse_type()?);
                    if !self.check(Token::RParen) {
                        self.expect(Token::Comma)?;
                    }
                }
                self.expect(Token::RParen)?;
            }
            variants.push(ErrVariant {
                name: vname,
                fields,
                span: vsp,
            });
            self.skip_nl();
        }
        if self.check(Token::Dedent) {
            self.advance();
        }
        Ok(ErrDef {
            name,
            variants,
            span: sp,
        })
    }

    fn parse_test_block(&mut self) -> Result<TestBlock, ParseError> {
        let sp = self.span();
        self.expect(Token::Test)?;
        let name = match self.peek() {
            Token::Str(ref s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => return Err(self.error("test requires a string name")),
        };
        self.expect(Token::Newline)?;
        let body = self.parse_block()?;
        Ok(TestBlock { name, body, span: sp })
    }

    fn parse_actor_def(&mut self) -> Result<ActorDef, ParseError> {
        let sp = self.span();
        self.expect(Token::Actor)?;
        let name = self.ident()?;
        self.expect(Token::Newline)?;
        let mut fields = Vec::new();
        let mut handlers = Vec::new();
        if self.check(Token::Indent) {
            self.advance();
            while !self.check(Token::Dedent) && !self.eof() {
                self.skip_nl();
                if self.check(Token::Dedent) || self.eof() {
                    break;
                }
                if self.check(Token::At) {
                    handlers.push(self.parse_handler()?);
                } else {
                    fields.push(self.parse_field()?);
                    self.skip_nl();
                }
            }
            if self.check(Token::Dedent) {
                self.advance();
            }
        }
        Ok(ActorDef {
            name,
            fields,
            handlers,
            span: sp,
        })
    }

    fn parse_handler(&mut self) -> Result<Handler, ParseError> {
        let sp = self.span();
        self.expect(Token::At)?;
        let name = self.ident()?;
        let mut params = Vec::new();
        while !self.check(Token::Newline) && !self.check(Token::Is) && !self.eof() {
            params.push(self.parse_param(true)?);
            if self.check(Token::Comma) {
                self.advance();
            }
        }
        let body = if self.check(Token::Is) {
            self.advance();
            let expr = self.parse_expr()?;
            vec![Stmt::Expr(expr)]
        } else {
            self.expect(Token::Newline)?;
            self.parse_block()?
        };
        Ok(Handler {
            name,
            params,
            body,
            span: sp,
        })
    }

    fn parse_store_def(&mut self) -> Result<StoreDef, ParseError> {
        let sp = self.span();
        self.expect(Token::Store)?;
        let name = self.ident()?;
        self.expect(Token::Newline)?;
        let mut fields = Vec::new();
        if self.check(Token::Indent) {
            self.advance();
            while !self.check(Token::Dedent) && !self.eof() {
                self.skip_nl();
                if self.check(Token::Dedent) || self.eof() {
                    break;
                }
                fields.push(self.parse_field()?);
                self.skip_nl();
            }
            if self.check(Token::Dedent) {
                self.advance();
            }
        }
        Ok(StoreDef {
            name,
            fields,
            span: sp,
        })
    }

    fn parse_trait_def(&mut self) -> Result<TraitDef, ParseError> {
        let sp = self.span();
        self.expect(Token::Trait)?;
        let name = self.ident()?;
        let (type_params, _) = self.parse_type_params();
        self.expect(Token::Newline)?;
        let mut methods = Vec::new();
        let mut assoc_types = Vec::new();
        if self.check(Token::Indent) {
            self.advance();
            while !self.check(Token::Dedent) && !self.eof() {
                self.skip_nl();
                if self.check(Token::Dedent) || self.eof() {
                    break;
                }
                if self.check(Token::Type) {
                    self.advance();
                    assoc_types.push(self.ident()?);
                    self.skip_nl();
                } else {
                    methods.push(self.parse_trait_method()?);
                }
            }
            if self.check(Token::Dedent) {
                self.advance();
            }
        }
        Ok(TraitDef {
            name,
            type_params,
            assoc_types,
            methods,
            span: sp,
        })
    }

    fn parse_trait_method(&mut self) -> Result<TraitMethod, ParseError> {
        let sp = self.span();
        self.expect(Token::Star)?;
        let name = self.ident()?;
        let mut params = Vec::new();

        if self.check(Token::LParen) {
            self.advance();
            while !self.check(Token::RParen) && !self.eof() {
                params.push(self.parse_param(true)?);
                if !self.check(Token::RParen) {
                    self.expect(Token::Comma)?;
                }
            }
            self.expect(Token::RParen)?;
        } else {
            // Paren-free: parse 'self' first (untyped), then typed params
            while !self.check(Token::Newline) && !self.check(Token::Arrow) && !self.check(Token::Is) && !self.eof() {
                let is_self = matches!(self.peek(), Token::Ident(ref s) if s == "self");
                params.push(self.parse_param(!is_self)?);
                if self.check(Token::Comma) {
                    self.advance();
                }
            }
        }

        let ret = if self.check(Token::Arrow) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        let default_body = if self.check(Token::Is) {
            self.advance();
            let expr = self.parse_expr()?;
            Some(vec![Stmt::Expr(expr)])
        } else if self.check(Token::Newline) {
            self.advance();
            if self.check(Token::Indent) {
                Some(self.parse_block()?)
            } else {
                None
            }
        } else {
            None
        };

        Ok(TraitMethod {
            name,
            params,
            ret,
            default_body,
            span: sp,
        })
    }

    fn parse_impl_block(&mut self) -> Result<ImplBlock, ParseError> {
        let sp = self.span();
        self.expect(Token::Impl)?;
        let first_name = self.ident()?;
        let (trait_name, trait_type_args, type_name) = if self.check(Token::Of) {
            // `impl TraitName of TypeArg, ... for TypeName`
            self.advance();
            let mut type_args = vec![self.parse_type()?];
            while self.check(Token::Comma) {
                self.advance();
                type_args.push(self.parse_type()?);
            }
            self.expect(Token::For)?;
            let tn = self.ident()?;
            (Some(first_name), type_args, tn)
        } else if self.check(Token::For) {
            self.advance();
            let tn = self.ident()?;
            (Some(first_name), Vec::new(), tn)
        } else {
            (None, Vec::new(), first_name)
        };
        self.expect(Token::Newline)?;
        let mut methods = Vec::new();
        let mut assoc_type_bindings = Vec::new();
        if self.check(Token::Indent) {
            self.advance();
            while !self.check(Token::Dedent) && !self.eof() {
                self.skip_nl();
                if self.check(Token::Dedent) || self.eof() {
                    break;
                }
                if self.check(Token::Type) {
                    self.advance();
                    let aname = self.ident()?;
                    self.expect(Token::Is)?;
                    let aty = self.parse_type()?;
                    assoc_type_bindings.push((aname, aty));
                    self.skip_nl();
                } else {
                    methods.push(self.parse_fn()?);
                }
            }
            if self.check(Token::Dedent) {
                self.advance();
            }
        }
        Ok(ImplBlock {
            trait_name,
            trait_type_args,
            type_name,
            assoc_type_bindings,
            methods,
            span: sp,
        })
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

    /// Parse `set <store> where <filter> <field> <expr> [, <field> <expr>]*`
    fn parse_set_stmt(&mut self) -> Result<Stmt, ParseError> {
        let sp = self.span();
        self.expect(Token::Set)?;
        let store = self.ident()?;
        // Filter comes first: `where field op value`
        let filter = self.parse_store_filter()?;
        // Then field assignments: field expr [, field expr]*
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

    fn parse_store_filter(&mut self) -> Result<StoreFilter, ParseError> {
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
            match self.peek() {
                Token::And => {
                    self.advance();
                    let f = self.ident()?;
                    let o = self.parse_filter_op()?;
                    let v = self.parse_add()?;
                    extra.push((LogicalOp::And, StoreFilterCond { field: f, op: o, value: v }));
                }
                Token::Or => {
                    self.advance();
                    let f = self.ident()?;
                    let o = self.parse_filter_op()?;
                    let v = self.parse_add()?;
                    extra.push((LogicalOp::Or, StoreFilterCond { field: f, op: o, value: v }));
                }
                _ => break,
            }
        }
        Ok(StoreFilter { field, op, value, span: sp, extra })
    }

    fn parse_filter_op(&mut self) -> Result<BinOp, ParseError> {
        match self.peek() {
            Token::Equals => { self.advance(); Ok(BinOp::Eq) }
            Token::Isnt => { self.advance(); Ok(BinOp::Ne) }
            Token::Lt => { self.advance(); Ok(BinOp::Lt) }
            Token::Gt => { self.advance(); Ok(BinOp::Gt) }
            Token::LtEq => { self.advance(); Ok(BinOp::Le) }
            Token::GtEq => { self.advance(); Ok(BinOp::Ge) }
            _ => Err(self.error("expected comparison operator (equals, isnt, <, >, <=, >=)")),
        }
    }

    fn parse_pat(&mut self) -> Result<Pat, ParseError> {
        let first = self.parse_single_pat()?;
        // Or-pattern: `A or B or C`
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
            Token::Int(_) | Token::Float(_) | Token::Str(_) | Token::True | Token::False => {
                let lit = self.parse_primary()?;
                // Range pattern: `1 to 10`
                if self.check(Token::To) {
                    self.advance();
                    let hi = self.parse_primary()?;
                    Ok(Pat::Range(lit, hi, sp))
                } else {
                    Ok(Pat::Lit(lit))
                }
            }
            Token::LParen => {
                // Tuple pattern: (a, b, c)
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
                // Array pattern: [a, b, c]
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

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
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

    fn parse_pipeline(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_or()?;
        while self.check(Token::Tilde) {
            let sp = self.span();
            self.advance();
            let rhs = self.parse_or()?;
            e = Expr::Pipe(Box::new(e), Box::new(rhs), vec![], sp);
        }
        Ok(e)
    }

    binop!(parse_or,     parse_and,    { Token::Or => BinOp::Or });
    binop!(parse_and,    parse_eq,     { Token::And => BinOp::And });
    binop!(parse_eq,     parse_cmp,    { Token::Equals => BinOp::Eq, Token::Isnt => BinOp::Ne });
    binop!(parse_cmp,    parse_bitor,  { Token::Lt => BinOp::Lt, Token::Gt => BinOp::Gt, Token::LtEq => BinOp::Le, Token::GtEq => BinOp::Ge });
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

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
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
                    e = Expr::As(Box::new(e), self.parse_type()?, sp);
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
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let sp = self.span();
        match self.peek() {
            Token::Int(n) => {
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
                let arg = if self.check(Token::LParen) {
                    self.advance();
                    let a = self.parse_expr()?;
                    self.expect(Token::RParen)?;
                    a
                } else {
                    self.parse_expr()?
                };
                Ok(Expr::Call(
                    Box::new(Expr::Ident("log".into(), sp)),
                    vec![arg],
                    sp,
                ))
            }
            Token::If => Ok(Expr::IfExpr(Box::new(self.parse_if()?))),
            Token::Assert => {
                self.advance();
                let arg = if self.check(Token::LParen) {
                    self.advance();
                    let a = self.parse_expr()?;
                    self.expect(Token::RParen)?;
                    a
                } else {
                    self.parse_expr()?
                };
                Ok(Expr::Call(
                    Box::new(Expr::Ident("assert".into(), sp)),
                    vec![arg],
                    sp,
                ))
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
            Token::Ident(ref name) => {
                let name = name.clone();
                self.advance();
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
                let ret = if self.check(Token::Arrow) {
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
                // channel of Type(capacity) or channel of Type
                self.advance();
                self.expect(Token::Of)?;
                let elem_ty = self.parse_type()?;
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
                        // send ch, value
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
                        // receive ch as binding  OR  receive ch
                        self.advance();
                        let ch = self.parse_expr()?;
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
                        return Err(self.error("expected 'send', 'receive', or 'default' in select"));
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
                // Coroutine: `dispatch name\n  body` (dispatch + ident + newline means coroutine)
                if is_dispatch {
                    if let Token::Ident(_) = self.peek() {
                        // Look ahead: if ident is followed by newline (not comma), it's a coroutine
                        let next_idx = self.pos + 1;
                        if next_idx < self.tok.len() && matches!(self.tok[next_idx].token, Token::Newline) {
                            let name = self.ident()?;
                            self.expect(Token::Newline)?;
                            let body = self.parse_block()?;
                            return Ok(Expr::DispatchBlock(name, body, sp));
                        }
                    }
                    // `dispatch\n  body` (anonymous coroutine)
                    if self.check(Token::Newline) {
                        let next_idx = self.pos + 1;
                        if next_idx < self.tok.len() && matches!(self.tok[next_idx].token, Token::Indent) {
                            self.advance(); // consume newline
                            let body = self.parse_block()?;
                            return Ok(Expr::DispatchBlock("__anon".to_string(), body, sp));
                        }
                    }
                }
                // Actor send: `send/dispatch expr, @handler(args)`
                // Channel send: `send expr, value` (no @)
                let target = self.parse_expr()?;
                self.expect(Token::Comma)?;
                if self.check(Token::At) {
                    // Actor send
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
                    // Channel send
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
                // If next token is NOT newline, it's a channel receive: `receive ch`
                if !self.check(Token::Newline) {
                    let ch = self.parse_expr()?;
                    return Ok(Expr::ChannelRecv(Box::new(ch), sp));
                }
                // Otherwise it's actor receive with @handler arms
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

    fn parse_literal_token(&mut self) -> Result<Expr, ParseError> {
        let sp = self.span();
        match self.peek() {
            Token::Int(n) => { self.advance(); Ok(Expr::Int(n, sp)) }
            Token::Float(n) => { self.advance(); Ok(Expr::Float(n, sp)) }
            Token::True => { self.advance(); Ok(Expr::Bool(true, sp)) }
            Token::False => { self.advance(); Ok(Expr::Bool(false, sp)) }
            Token::Str(ref s) => { let v = s.clone(); self.advance(); Ok(Expr::Str(v, sp)) }
            _ => Err(self.error("expected literal")),
        }
    }

    fn parse_query_block(&mut self, source: Expr) -> Result<Expr, ParseError> {
        let sp = source.span();
        self.expect(Token::Query)?;
        self.expect(Token::Newline)?;
        self.expect(Token::Indent)?;
        let mut clauses = Vec::new();
        while !self.check(Token::Dedent) && !self.eof() {
            self.skip_nl();
            if self.check(Token::Dedent) || self.eof() {
                break;
            }
            clauses.push(self.parse_query_clause()?);
            self.skip_nl();
        }
        if self.check(Token::Dedent) {
            self.advance();
        }
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
                    if dir == "desc" { self.advance(); false }
                    else if dir == "asc" { self.advance(); true }
                    else { true }
                } else { true };
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

    fn is_field_init(&self) -> bool {
        matches!(self.peek(), Token::Ident(_))
            && self.pos + 1 < self.tok.len()
            && matches!(self.tok[self.pos + 1].token, Token::Is)
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut a = Vec::new();
        while !self.check(Token::RParen) && !self.eof() {
            a.push(self.parse_expr()?);
            if !self.check(Token::RParen) {
                self.expect(Token::Comma)?;
            }
        }
        Ok(a)
    }

    fn parse_type(&mut self) -> Result<Type, ParseError> {
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
                    if let Type::Struct(name) = t {
                        self.advance();
                        let mut args = vec![self.parse_type()?];
                        while self.check(Token::Comma) {
                            self.advance();
                            args.push(self.parse_type()?);
                        }
                        let mangled = format!(
                            "{}_{}",
                            name,
                            args.iter()
                                .map(|a| format!("{a}"))
                                .collect::<Vec<_>>()
                                .join("_")
                        );
                        Ok(Type::Struct(mangled))
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
                self.expect(Token::Arrow)?;
                let ret = self.parse_type()?;
                Ok(Type::Fn(params, Box::new(ret)))
            }
            _ => Err(self.error("expected type")),
        }
    }

    fn ident_to_type(&self, n: &str) -> Type {
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
            _ => Type::Struct(n.to_string()),
        }
    }

    /// Parse a string interpolation: 'hello {expr} world {expr2} end'
    /// Already consumed the first Str token; sp is the span of the opening string.
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

    fn peek(&self) -> Token {
        if self.pos < self.tok.len() {
            self.tok[self.pos].token.clone()
        } else {
            Token::Eof
        }
    }
    fn check(&self, t: Token) -> bool {
        std::mem::discriminant(&self.peek()) == std::mem::discriminant(&t)
    }
    fn advance(&mut self) {
        if self.pos < self.tok.len() {
            self.pos += 1;
        }
    }
    fn span(&self) -> Span {
        if self.pos < self.tok.len() {
            self.tok[self.pos].span
        } else {
            Span::dummy()
        }
    }
    fn eof(&self) -> bool {
        self.pos >= self.tok.len() || matches!(self.tok[self.pos].token, Token::Eof)
    }
    fn skip_nl(&mut self) {
        while self.check(Token::Newline) {
            self.advance();
        }
    }

    fn expect(&mut self, t: Token) -> Result<(), ParseError> {
        if self.check(t.clone()) {
            self.advance();
            Ok(())
        } else {
            Err(self.error(&format!("expected {t}, got {}", self.peek())))
        }
    }

    fn ident(&mut self) -> Result<String, ParseError> {
        match self.peek() {
            Token::Ident(n) => { self.advance(); Ok(n) }
            Token::Set => { self.advance(); Ok("set".into()) }
            _ => Err(self.error(&format!("expected identifier, got {}", self.peek())))
        }
    }

    fn error(&self, msg: &str) -> ParseError {
        let sp = self.span();
        ParseError::Error {
            line: sp.line,
            col: sp.col,
            msg: msg.into(),
        }
    }
}

/// Merge multiple function definitions with the same name (pattern-directed clauses)
/// into a single function with an if/elif/else chain that dispatches on literal params.
fn desugar_multi_clause_fns(prog: &mut Program) {
    // Collect indices of Fn decls grouped by name, preserving order.
    let mut name_indices: Vec<(String, Vec<usize>)> = Vec::new();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (i, decl) in prog.decls.iter().enumerate() {
        if let Decl::Fn(f) = decl {
            if let Some(&group_idx) = seen.get(&f.name) {
                name_indices[group_idx].1.push(i);
            } else {
                seen.insert(f.name.clone(), name_indices.len());
                name_indices.push((f.name.clone(), vec![i]));
            }
        }
    }

    // Find groups with >1 clause.
    let multi_groups: Vec<(String, Vec<usize>)> = name_indices
        .into_iter()
        .filter(|(_, indices)| indices.len() > 1)
        .collect();

    if multi_groups.is_empty() {
        return;
    }

    // For each multi-clause group, merge into a single Fn and mark the rest for removal.
    let mut to_remove: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (_, indices) in &multi_groups {
        let clauses: Vec<Fn> = indices
            .iter()
            .map(|&i| {
                if let Decl::Fn(f) = &prog.decls[i] {
                    f.clone()
                } else {
                    unreachable!()
                }
            })
            .collect();

        let merged = merge_fn_clauses(&clauses);

        // Replace the first occurrence, mark the rest for removal.
        prog.decls[indices[0]] = Decl::Fn(merged);
        for &i in &indices[1..] {
            to_remove.insert(i);
        }
    }

    // Remove merged-away decls in reverse order.
    let mut remove_sorted: Vec<usize> = to_remove.into_iter().collect();
    remove_sorted.sort_unstable_by(|a, b| b.cmp(a));
    for i in remove_sorted {
        prog.decls.remove(i);
    }
}

fn merge_fn_clauses(clauses: &[Fn]) -> Fn {
    let first = &clauses[0];
    let param_count = first.params.len();
    let sp = first.span;

    // Build unified params: use __argN for positions that have any literal in any clause,
    // otherwise use the name from the catch-all (last) clause or first named clause.
    let mut unified_params: Vec<Param> = Vec::new();
    for pi in 0..param_count {
        // Find a clause that has a real name (non-literal) for this position.
        let real_name = clauses
            .iter()
            .find_map(|c| {
                c.params.get(pi).and_then(|p| {
                    if p.literal.is_none() {
                        Some(p.name.clone())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| format!("__arg{pi}"));

        // Use the type from whichever clause has an explicit type.
        let ty = clauses.iter().find_map(|c| c.params.get(pi).and_then(|p| p.ty.clone()));

        unified_params.push(Param {
            name: real_name,
            ty,
            default: None,
            literal: None,
            span: sp,
        });
    }

    // Separate clauses with literal patterns (guarded) from the catch-all.
    let mut guarded: Vec<&Fn> = Vec::new();
    let mut catchall: Option<&Fn> = None;
    for c in clauses {
        if c.params.iter().any(|p| p.literal.is_some()) {
            guarded.push(c);
        } else {
            catchall = Some(c);
        }
    }

    // Build if/elif/else body.
    let build_cond = |clause: &Fn| -> Expr {
        let mut conds: Vec<Expr> = Vec::new();
        for (pi, p) in clause.params.iter().enumerate() {
            if let Some(ref lit) = p.literal {
                let arg_ref = Expr::Ident(unified_params[pi].name.clone(), sp);
                conds.push(Expr::BinOp(
                    Box::new(arg_ref),
                    BinOp::Eq,
                    Box::new(lit.clone()),
                    sp,
                ));
            }
        }
        conds
            .into_iter()
            .reduce(|a, b| Expr::BinOp(Box::new(a), BinOp::And, Box::new(b), sp))
            .unwrap()
    };

    let build_body = |clause: &Fn| -> Block {
        let mut body = Vec::new();
        // Bind named params that differ from unified names.
        for (pi, p) in clause.params.iter().enumerate() {
            if p.literal.is_none() && p.name != unified_params[pi].name {
                body.push(Stmt::Bind(crate::ast::Bind {
                    name: p.name.clone(),
                    value: Expr::Ident(unified_params[pi].name.clone(), sp),
                    ty: None,
                    span: sp,
                }));
            }
        }
        body.extend(clause.body.clone());
        body
    };

    let body = if guarded.is_empty() {
        // No literal params at all, just use the catch-all.
        catchall.map(|c| c.body.clone()).unwrap_or_default()
    } else {
        let first_guarded = guarded[0];
        let then_cond = build_cond(first_guarded);
        let then_body = build_body(first_guarded);
        let mut elifs: Vec<(Expr, Block)> = Vec::new();
        for g in &guarded[1..] {
            elifs.push((build_cond(g), build_body(g)));
        }
        let els = catchall.map(|c| build_body(c));

        vec![Stmt::Expr(Expr::IfExpr(Box::new(If {
            cond: then_cond,
            then: then_body,
            elifs,
            els,
            span: sp,
        })))]
    };

    Fn {
        name: first.name.clone(),
        type_params: first.type_params.clone(),
        type_bounds: first.type_bounds.clone(),
        params: unified_params,
        ret: first.ret.clone(),
        body,
        span: sp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    fn parse(s: &str) -> Program {
        let t = Lexer::new(s).tokenize().unwrap();
        Parser::new(t).parse_program().unwrap()
    }

    #[test]
    fn hello() {
        let p = parse("*main()\n    log('hello')\n");
        assert_eq!(p.decls.len(), 1);
        if let Decl::Fn(f) = &p.decls[0] {
            assert_eq!(f.name, "main");
            assert_eq!(f.params.len(), 0);
            assert_eq!(f.body.len(), 1);
        } else {
            panic!("expected fn");
        }
    }

    #[test]
    fn binding() {
        let p = parse("*main()\n    x is 42\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                assert_eq!(b.name, "x");
            } else {
                panic!("expected bind");
            }
        }
    }

    #[test]
    fn fibonacci() {
        let p = parse("*fibonacci(n)\n    n <= 1 ? n ! fibonacci(n - 1) + fibonacci(n - 2)\n");
        assert_eq!(p.decls.len(), 1);
        if let Decl::Fn(f) = &p.decls[0] {
            assert_eq!(f.name, "fibonacci");
            assert_eq!(f.params.len(), 1);
        }
    }

    #[test]
    fn if_else() {
        let p = parse("*main()\n    if true\n        log('yes')\n    else\n        log('no')\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::If(i) = &f.body[0] {
                assert!(i.els.is_some());
            } else {
                panic!("expected if");
            }
        }
    }

    #[test]
    fn struct_def() {
        let p = parse("type Point\n    x is 0.0\n    y is 0.0\n");
        if let Decl::Type(td) = &p.decls[0] {
            assert_eq!(td.name, "Point");
            assert_eq!(td.fields.len(), 2);
        } else {
            panic!("expected type def");
        }
    }

    #[test]
    fn enum_def() {
        let p = parse("enum Direction\n    North\n    South\n    East\n    West\n");
        if let Decl::Enum(ed) = &p.decls[0] {
            assert_eq!(ed.name, "Direction");
            assert_eq!(ed.variants.len(), 4);
        } else {
            panic!("expected enum");
        }
    }

    #[test]
    fn ternary() {
        let p = parse("*main()\n    x is 1 > 0 ? 42 ! 0\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                assert!(matches!(b.value, Expr::Ternary(_, _, _, _)));
            } else {
                panic!("expected bind");
            }
        }
    }

    #[test]
    fn while_stmt() {
        let p = parse("*main()\n    while true\n        log(1)\n");
        if let Decl::Fn(f) = &p.decls[0] {
            assert!(matches!(f.body[0], Stmt::While(_)));
        }
    }

    #[test]
    fn for_stmt() {
        let p = parse("*main()\n    for i in 10\n        log(i)\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::For(fo) = &f.body[0] {
                assert_eq!(fo.bind, "i");
            } else {
                panic!("expected for");
            }
        }
    }

    #[test]
    fn loop_break() {
        let p = parse("*main()\n    loop\n        break\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Loop(l) = &f.body[0] {
                assert!(matches!(l.body[0], Stmt::Break(_, _)));
            } else {
                panic!("expected loop");
            }
        }
    }

    #[test]
    fn return_stmt() {
        let p = parse("*foo()\n    return 42\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Ret(Some(Expr::Int(42, _)), _) = &f.body[0] {
            } else {
                panic!("expected return 42");
            }
        }
    }

    #[test]
    fn match_stmt() {
        let p = parse("*main()\n    match x\n        1 ? log(1)\n        _ ? log(0)\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Match(m) = &f.body[0] {
                assert_eq!(m.arms.len(), 2);
                assert!(matches!(m.arms[1].pat, Pat::Wild(_)));
            } else {
                panic!("expected match");
            }
        }
    }

    #[test]
    fn unary_ops() {
        let p = parse("*main()\n    x is -42\n    y is not true\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                assert!(matches!(b.value, Expr::UnaryOp(UnaryOp::Neg, _, _)));
            }
            if let Stmt::Bind(b) = &f.body[1] {
                assert!(matches!(b.value, Expr::UnaryOp(UnaryOp::Not, _, _)));
            }
        }
    }

    #[test]
    fn exponentiation() {
        let p = parse("*main()\n    x is 2 ** 3\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::BinOp(_, op, _, _) = &b.value {
                    assert_eq!(*op, BinOp::Exp);
                } else {
                    panic!("expected binop");
                }
            }
        }
    }

    #[test]
    fn exp_right_assoc() {
        let p = parse("*main()\n    x is 2 ** 3 ** 4\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::BinOp(l, BinOp::Exp, r, _) = &b.value {
                    assert!(matches!(l.as_ref(), Expr::Int(2, _)));
                    assert!(matches!(r.as_ref(), Expr::BinOp(_, BinOp::Exp, _, _)));
                } else {
                    panic!("expected exp");
                }
            }
        }
    }

    #[test]
    fn as_cast() {
        let p = parse("*main()\n    x is 42 as f64\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                assert!(matches!(b.value, Expr::As(_, Type::F64, _)));
            } else {
                panic!("expected bind");
            }
        }
    }

    #[test]
    fn array_literal() {
        let p = parse("*main()\n    x is [1, 2, 3]\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::Array(elems, _) = &b.value {
                    assert_eq!(elems.len(), 3);
                } else {
                    panic!("expected array");
                }
            }
        }
    }

    #[test]
    fn tuple_literal() {
        let p = parse("*main()\n    x is (1, 2, 3)\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::Tuple(elems, _) = &b.value {
                    assert_eq!(elems.len(), 3);
                } else {
                    panic!("expected tuple");
                }
            }
        }
    }

    #[test]
    fn fn_with_typed_params() {
        let p = parse("*add(a: i64, b: i64) -> i64\n    return a + b\n");
        if let Decl::Fn(f) = &p.decls[0] {
            assert_eq!(f.params[0].ty, Some(Type::I64));
            assert_eq!(f.params[1].ty, Some(Type::I64));
            assert_eq!(f.ret, Some(Type::I64));
        }
    }

    #[test]
    fn elif_chain() {
        let p = parse(
            "*main()\n    if true\n        log(1)\n    elif false\n        log(2)\n    elif true\n        log(3)\n    else\n        log(4)\n",
        );
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::If(i) = &f.body[0] {
                assert_eq!(i.elifs.len(), 2);
                assert!(i.els.is_some());
            } else {
                panic!("expected if");
            }
        }
    }

    #[test]
    fn multiple_fns() {
        let p = parse("*foo()\n    return 1\n\n*bar()\n    return 2\n\n*main()\n    log(foo())\n");
        assert_eq!(p.decls.len(), 3);
    }

    #[test]
    fn continue_stmt() {
        let p = parse("*main()\n    while true\n        continue\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::While(w) = &f.body[0] {
                assert!(matches!(w.body[0], Stmt::Continue(_)));
            }
        }
    }

    #[test]
    fn method_call() {
        let p = parse("*main()\n    x.foo(1, 2)\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Expr(Expr::Method(_, name, args, _)) = &f.body[0] {
                assert_eq!(name, "foo");
                assert_eq!(args.len(), 2);
            } else {
                panic!("expected method call");
            }
        }
    }

    #[test]
    fn field_access() {
        let p = parse("*main()\n    x.y\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Expr(Expr::Field(_, field, _)) = &f.body[0] {
                assert_eq!(field, "y");
            } else {
                panic!("expected field access");
            }
        }
    }

    #[test]
    fn struct_construction() {
        let p = parse("*main()\n    Point(x is 1, y is 2)\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Expr(Expr::Struct(name, fields, _)) = &f.body[0] {
                assert_eq!(name, "Point");
                assert_eq!(fields.len(), 2);
            } else {
                panic!("expected struct construction");
            }
        }
    }

    #[test]
    fn index_expr() {
        let p = parse("*main()\n    x[0]\n");
        if let Decl::Fn(f) = &p.decls[0] {
            assert!(matches!(f.body[0], Stmt::Expr(Expr::Index(_, _, _))));
        }
    }

    #[test]
    fn pipeline_simple() {
        let p = parse("*double(x: i64) -> i64\n    x * 2\n\n*main()\n    x is 10 ~ double\n");
        if let Decl::Fn(f) = &p.decls[1] {
            if let Stmt::Bind(b) = &f.body[0] {
                assert!(matches!(b.value, Expr::Pipe(_, _, _, _)));
            } else {
                panic!("expected bind with pipe");
            }
        }
    }

    #[test]
    fn pipeline_chain() {
        let p = parse(
            "*a(x: i64) -> i64\n    x\n\n*b(x: i64) -> i64\n    x\n\n*main()\n    x is 1 ~ a ~ b\n",
        );
        if let Decl::Fn(f) = &p.decls[2] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::Pipe(left, _, _, _) = &b.value {
                    assert!(matches!(left.as_ref(), Expr::Pipe(_, _, _, _)));
                } else {
                    panic!("expected chained pipe");
                }
            }
        }
    }

    #[test]
    fn pipeline_with_call() {
        let p = parse("*add(a: i64, b: i64) -> i64\n    a + b\n\n*main()\n    x is 10 ~ add(5)\n");
        if let Decl::Fn(f) = &p.decls[1] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::Pipe(_, right, _, _) = &b.value {
                    assert!(matches!(right.as_ref(), Expr::Call(_, _, _)));
                } else {
                    panic!("expected pipe with call");
                }
            }
        }
    }

    #[test]
    fn placeholder_in_call() {
        let p =
            parse("*mul(a: i64, b: i64) -> i64\n    a * b\n\n*main()\n    x is 10 ~ mul($, 3)\n");
        if let Decl::Fn(f) = &p.decls[1] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::Pipe(_, right, _, _) = &b.value {
                    if let Expr::Call(_, args, _) = right.as_ref() {
                        assert!(matches!(args[0], Expr::Placeholder(_)));
                    } else {
                        panic!("expected call with placeholder");
                    }
                }
            }
        }
    }

    #[test]
    fn lambda_expr() {
        let p = parse("*main()\n    f is *fn(x: i64) -> i64 x * 2\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::Lambda(params, ret, _, _) = &b.value {
                    assert_eq!(params.len(), 1);
                    assert_eq!(params[0].name, "x");
                    assert_eq!(*ret, Some(Type::I64));
                } else {
                    panic!("expected lambda");
                }
            }
        }
    }

    #[test]
    fn lambda_multi_param() {
        let p = parse("*main()\n    f is *fn(a: i64, b: i64) -> i64 a + b\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::Lambda(params, _, _, _) = &b.value {
                    assert_eq!(params.len(), 2);
                } else {
                    panic!("expected lambda");
                }
            }
        }
    }

    #[test]
    fn fn_type_annotation() {
        let p = parse("*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Some(Type::Fn(ptys, ret)) = &f.params[0].ty {
                assert_eq!(ptys.len(), 1);
                assert_eq!(ptys[0], Type::I64);
                assert_eq!(**ret, Type::I64);
            } else {
                panic!("expected fn type on param, got: {:?}", f.params[0].ty);
            }
        }
    }

    #[test]
    fn inline_body_parens() {
        let p = parse("*double(x) is x * 2\n");
        if let Decl::Fn(f) = &p.decls[0] {
            assert_eq!(f.name, "double");
            assert_eq!(f.params.len(), 1);
            assert_eq!(f.params[0].name, "x");
            assert_eq!(f.body.len(), 1);
            assert!(matches!(f.body[0], Stmt::Expr(Expr::BinOp(_, BinOp::Mul, _, _))));
        } else {
            panic!("expected fn");
        }
    }

    #[test]
    fn inline_body_paren_free() {
        let p = parse("*add a, b is a + b\n");
        if let Decl::Fn(f) = &p.decls[0] {
            assert_eq!(f.name, "add");
            assert_eq!(f.params.len(), 2);
            assert_eq!(f.params[0].name, "a");
            assert_eq!(f.params[1].name, "b");
            assert_eq!(f.body.len(), 1);
            assert!(matches!(f.body[0], Stmt::Expr(Expr::BinOp(_, BinOp::Add, _, _))));
        } else {
            panic!("expected fn");
        }
    }

    #[test]
    fn literal_param_single() {
        // *fib(0) is 0 / *fib(n) is n — merges into one fn with if
        let p = parse("*fib(0) is 0\n\n*fib(n)\n    n\n");
        assert_eq!(p.decls.len(), 1);
        if let Decl::Fn(f) = &p.decls[0] {
            assert_eq!(f.name, "fib");
            assert_eq!(f.params.len(), 1);
            assert!(f.params[0].literal.is_none()); // merged: unified param has no literal
            assert_eq!(f.body.len(), 1);
            // Body should be an IfExpr (desugared from clauses)
            assert!(matches!(f.body[0], Stmt::Expr(Expr::IfExpr(_))));
        } else {
            panic!("expected fn");
        }
    }

    #[test]
    fn literal_param_multi_clause() {
        let p = parse("*fib(0) is 0\n\n*fib(1) is 1\n\n*fib(n)\n    fib(n - 1) + fib(n - 2)\n");
        assert_eq!(p.decls.len(), 1); // all merged into one
        if let Decl::Fn(f) = &p.decls[0] {
            assert_eq!(f.name, "fib");
            if let Stmt::Expr(Expr::IfExpr(ref i)) = f.body[0] {
                assert_eq!(i.elifs.len(), 1); // one elif for *fib(1)
                assert!(i.els.is_some());     // else for *fib(n)
            } else {
                panic!("expected desugared if-expr");
            }
        }
    }

    #[test]
    fn query_block() {
        let p = parse("*main()\n    x is Users query\n        where age >= 18\n        limit 10\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::Query(source, clauses, _) = &b.value {
                    assert!(matches!(source.as_ref(), Expr::Ident(n, _) if n == "Users"));
                    assert_eq!(clauses.len(), 2);
                    assert!(matches!(clauses[0], QueryClause::Where(_, _)));
                    assert!(matches!(clauses[1], QueryClause::Limit(_, _)));
                } else {
                    panic!("expected query expr, got: {:?}", b.value);
                }
            } else {
                panic!("expected bind");
            }
        }
    }

    #[test]
    fn query_sort_clause() {
        let p = parse("*main()\n    x is Items query\n        sort name desc\n");
        if let Decl::Fn(f) = &p.decls[0] {
            if let Stmt::Bind(b) = &f.body[0] {
                if let Expr::Query(_, clauses, _) = &b.value {
                    if let QueryClause::Sort(field, asc, _) = &clauses[0] {
                        assert_eq!(field, "name");
                        assert!(!asc);
                    } else {
                        panic!("expected sort clause");
                    }
                }
            }
        }
    }
}
