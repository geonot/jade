use crate::ast::*;
use crate::lexer::Token;
use crate::types::Type;

use super::{ParseError, Parser};

impl Parser {
    pub(super) fn parse_decl(&mut self) -> Result<Decl, ParseError> {
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
            Token::Ident(_) => {
                let sp = self.span();
                let name = self.ident()?;
                self.expect(Token::Is)?;
                let val = self.parse_expr()?;
                self.expect(Token::Newline)?;
                Ok(Decl::Const(name, val, sp))
            }
            _ => Err(self.error(
                "expected *, type, enum, extern, use, err, test, actor, store, trait, or impl",
            )),
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

    pub(super) fn parse_fn(&mut self) -> Result<Fn, ParseError> {
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
                    Token::Int(_)
                    | Token::Float(_)
                    | Token::True
                    | Token::False
                    | Token::Str(_) => {
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
            while !self.check(Token::Newline)
                && !self.check(Token::Arrow)
                && !self.check(Token::Is)
                && !self.eof()
            {
                match self.peek() {
                    Token::Int(_)
                    | Token::Float(_)
                    | Token::True
                    | Token::False
                    | Token::Str(_) => {
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

    pub(super) fn parse_param(&mut self, typed: bool) -> Result<Param, ParseError> {
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
        Ok(TestBlock {
            name,
            body,
            span: sp,
        })
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
            while !self.check(Token::Newline)
                && !self.check(Token::Arrow)
                && !self.check(Token::Is)
                && !self.eof()
            {
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
}
