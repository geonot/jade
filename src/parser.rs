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
        Ok(Program { decls })
    }

    fn parse_decl(&mut self) -> Result<Decl, ParseError> {
        match self.peek() {
            Token::Star => Ok(Decl::Fn(self.parse_fn()?)),
            Token::Type | Token::Pub => Ok(Decl::Type(self.parse_type_def()?)),
            Token::Enum => Ok(Decl::Enum(self.parse_enum_def()?)),
            Token::Extern => Ok(Decl::Extern(self.parse_extern()?)),
            Token::Use => Ok(Decl::Use(self.parse_use_decl()?)),
            Token::Err => Ok(Decl::ErrDef(self.parse_err_def()?)),
            _ => Err(self.error("expected *, type, enum, extern, use, or err")),
        }
    }

    fn parse_type_params(&mut self) -> Vec<String> {
        let mut tp = Vec::new();
        if self.check(Token::Of) {
            self.advance();
            if let Token::Ident(_) = self.peek() {
                tp.push(self.ident().unwrap());
                while self.check(Token::Comma) {
                    self.advance();
                    if let Token::Ident(_) = self.peek() {
                        tp.push(self.ident().unwrap());
                    }
                }
            }
        }
        tp
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
        let type_params = self.parse_type_params();
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
            while !self.check(Token::Newline) && !self.check(Token::Arrow) && !self.eof() {
                params.push(self.parse_param(false)?);
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
        self.expect(Token::Newline)?;
        let body = self.parse_block()?;
        Ok(Fn {
            name,
            type_params,
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
        let default = if self.check(Token::Is) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Param {
            name,
            ty,
            default,
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
        let type_params = self.parse_type_params();
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
            span: sp,
        })
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
        let type_params = self.parse_type_params();
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
        // x, y is ...  →  Ident Comma Ident (Comma Ident)* Is
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
            Token::PlusEq => { self.advance(); Some(BinOp::Add) }
            Token::MinusEq => { self.advance(); Some(BinOp::Sub) }
            Token::StarEq => { self.advance(); Some(BinOp::Mul) }
            Token::SlashEq => { self.advance(); Some(BinOp::Div) }
            Token::PercentEq => { self.advance(); Some(BinOp::Mod) }
            Token::AmpEq => { self.advance(); Some(BinOp::BitAnd) }
            Token::PipeEq => { self.advance(); Some(BinOp::BitOr) }
            Token::CaretEq => { self.advance(); Some(BinOp::BitXor) }
            Token::ShlEq => { self.advance(); Some(BinOp::Shl) }
            Token::ShrEq => { self.advance(); Some(BinOp::Shr) }
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
                value: Expr::BinOp(
                    Box::new(Expr::Ident(name, sp)),
                    op,
                    Box::new(rhs),
                    rsp,
                ),
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
        self.expect(Token::Question)?;
        let body = if self.check(Token::Newline) {
            self.advance();
            self.parse_block()?
        } else {
            vec![Stmt::Expr(self.parse_expr()?)]
        };
        Ok(Arm {
            pat,
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

    fn parse_pat(&mut self) -> Result<Pat, ParseError> {
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
                Ok(Pat::Lit(self.parse_primary()?))
            }
            _ => Err(self.error("expected pattern")),
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_ternary()
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
                // Check for string interpolation: Str InterpStart expr InterpEnd Str ...
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
            _ => Err(self.error(&format!("unexpected token: {}", self.peek()))),
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
        // Build a chain of BinOp::Add expressions for string concatenation
        let mut result: Expr = Expr::Str(first, sp);
        while self.check(Token::InterpStart) {
            self.advance(); // consume InterpStart
            let expr = self.parse_expr()?;
            if !self.check(Token::InterpEnd) {
                return Err(self.error("expected closing } in string interpolation"));
            }
            self.advance(); // consume InterpEnd
            // Wrap the expression in a to_string call for non-string types
            let interp_expr = Expr::Call(
                Box::new(Expr::Ident("to_string".into(), expr.span())),
                vec![expr],
                sp,
            );
            result = Expr::BinOp(Box::new(result), BinOp::Add, Box::new(interp_expr), sp);
            // Check for the next string part
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
        if let Token::Ident(n) = self.peek() {
            self.advance();
            Ok(n)
        } else {
            Err(self.error(&format!("expected identifier, got {}", self.peek())))
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
}
