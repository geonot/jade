use crate::ast::*;
use crate::lexer::Token;
use crate::types::Type;

use super::super::{ParseError, Parser};
use super::Either;

impl Parser {
    pub(in crate::parser) fn parse_type_def(&mut self) -> Result<TypeDef, ParseError> {
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
            let items = self.parse_indented(|p| {
                if p.check(Token::Star) {
                    Ok(Either::Method(p.parse_fn()?))
                } else {
                    Ok(Either::Field(p.parse_field()?))
                }
            })?;
            for item in items {
                match item {
                    Either::Field(f) => fields.push(f),
                    Either::Method(mut m) => {
                        Self::ensure_implicit_self(&mut m.params, m.span);
                        methods.push(m);
                    }
                }
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

    pub(in crate::parser) fn parse_layout_attrs(
        &mut self,
    ) -> Result<crate::ast::LayoutAttrs, ParseError> {
        let mut layout = crate::ast::LayoutAttrs::default();
        while self.check(Token::At) {
            self.advance();

            if self.check(Token::Strict) {
                self.advance();
                layout.strict = true;
                continue;
            }
            let attr = self.ident()?;
            if attr == "packed" {
                layout.packed = true;
            } else if attr == "align" {
                self.expect(Token::LParen)?;
                let n = match self.peek() {
                    Token::Int(n) => {
                        let v = *n as u32;
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
            } else if attr == "resource" {
                layout.resource = true;
            } else {
                return Err(self.error(&format!("unknown layout attribute: @{attr}")));
            }
        }
        Ok(layout)
    }

    pub(in crate::parser) fn parse_field(&mut self) -> Result<Field, ParseError> {
        let sp = self.span();
        let name = self.ident()?;
        let (ty, access_mod) = if self.check(Token::As) {
            self.advance();
            let am = self.try_parse_access_mod_at_type_pos();
            (Some(self.parse_type()?), am)
        } else {
            (None, None)
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
            access_mod,
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_enum_def(&mut self) -> Result<EnumDef, ParseError> {
        let sp = self.span();
        self.expect(Token::Enum)?;
        let name = self.ident()?;
        let (type_params, _) = self.parse_type_params();
        self.expect(Token::Newline)?;
        let variants = if self.check(Token::Indent) {
            self.parse_indented(Self::parse_variant)?
        } else {
            Vec::new()
        };
        Ok(EnumDef {
            name,
            type_params,
            variants,
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_variant(&mut self) -> Result<Variant, ParseError> {
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
        let mut discriminant = None;
        if self.check(Token::Is) {
            self.advance();
            if let Token::Int(n) = self.peek() {
                discriminant = Some(*n);
                self.advance();
            }
        }
        Ok(Variant {
            name,
            fields,
            discriminant,
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_vfield(&mut self) -> Result<VField, ParseError> {
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
                ty: self.ident_to_type(&n.as_str()),
            })
        }
    }

    pub(in crate::parser) fn parse_use_decl(&mut self) -> Result<UseDecl, ParseError> {
        let sp = self.span();
        self.expect(Token::Use)?;
        let mut path = vec![self.ident()?];
        while self.check(Token::Slash) {
            self.advance();
            path.push(self.ident()?);
        }

        let imports = if self.check(Token::LBracket) {
            self.advance();
            let mut names = Vec::new();
            while !self.check(Token::RBracket) && !self.eof() {
                names.push(self.ident()?);
                if !self.check(Token::RBracket) {
                    if self.check(Token::Comma) {
                        self.advance();
                    }
                }
            }
            self.expect(Token::RBracket)?;
            Some(names)
        } else if !self.check(Token::Newline)
            && !self.eof()
            && matches!(self.peek(), Token::Ident(_))
        {
            Some(vec![self.ident()?])
        } else {
            None
        };

        let alias = if self.check(Token::As) {
            self.advance();
            Some(self.ident()?)
        } else {
            None
        };
        Ok(UseDecl {
            path,
            imports,
            alias,
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_err_def(&mut self) -> Result<ErrDef, ParseError> {
        let sp = self.span();
        self.expect(Token::Err)?;
        let name = self.ident()?;
        self.expect(Token::Newline)?;
        let variants = self.parse_indented(|p| {
            let vsp = p.span();
            let vname = p.ident()?;
            let mut fields = Vec::new();
            if p.check(Token::LParen) {
                p.advance();
                while !p.check(Token::RParen) && !p.eof() {
                    fields.push(p.parse_type()?);
                    if !p.check(Token::RParen) {
                        p.expect(Token::Comma)?;
                    }
                }
                p.expect(Token::RParen)?;
            }
            Ok(ErrVariant {
                name: vname,
                fields,
                span: vsp,
            })
        })?;
        Ok(ErrDef {
            name,
            variants,
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_test_block(&mut self) -> Result<TestBlock, ParseError> {
        let sp = self.span();
        self.expect(Token::Test)?;
        let name = match self.peek() {
            Token::Str(s) => {
                let n = Symbol::intern(s);
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
}
