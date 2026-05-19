use crate::ast::*;
use crate::lexer::Token;

use super::super::{ParseError, Parser};
use super::Either;

impl Parser {
    pub(in crate::parser) fn parse_trait_def(&mut self) -> Result<TraitDef, ParseError> {
        let sp = self.span();
        self.expect(Token::Trait)?;
        let name = self.ident()?;
        let (type_params, _) = self.parse_type_params();
        self.expect(Token::Newline)?;
        let (mut methods, mut assoc_types) = (Vec::new(), Vec::new());
        if self.check(Token::Indent) {
            let items = self.parse_indented(|p| {
                if p.check(Token::Type) {
                    p.advance();
                    let aname = p.ident()?;
                    Ok(Either::Field(aname))
                } else {
                    Ok(Either::Method(p.parse_trait_method()?))
                }
            })?;
            for item in items {
                match item {
                    Either::Field(name) => assoc_types.push(name),
                    Either::Method(m) => methods.push(m),
                }
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

    pub(in crate::parser) fn parse_trait_method(&mut self) -> Result<TraitMethod, ParseError> {
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
                && !self.check(Token::Returns)
                && !self.check(Token::Is)
                && !self.eof()
            {
                let is_self = matches!(self.peek(), Token::Ident(s) if s == "self");
                params.push(self.parse_param(!is_self)?);
                if self.check(Token::Comma) {
                    self.advance();
                }
            }
        }

        let ret = if self.check(Token::Returns) {
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

        Self::ensure_implicit_self(&mut params, sp);

        Ok(TraitMethod {
            name,
            params,
            ret,
            default_body,
            span: sp,
        })
    }

    pub(in crate::parser) fn ensure_implicit_self(params: &mut Vec<Param>, span: Span) {
        if params.first().map_or(true, |p| p.name.as_str() != "self") {
            params.insert(
                0,
                Param {
                    name: Symbol::intern("self"),
                    ty: None,
                    default: None,
                    literal: None,
                    access_mod: None,
                    span,
                },
            );
        }
    }

    pub(in crate::parser) fn parse_impl_block(&mut self) -> Result<ImplBlock, ParseError> {
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
            (Some(first_name), type_args, self.ident()?)
        } else if self.check(Token::For) {
            self.advance();
            (Some(first_name), Vec::new(), self.ident()?)
        } else {
            (None, Vec::new(), first_name)
        };
        self.expect(Token::Newline)?;
        let (mut methods, mut assoc_type_bindings) = (Vec::new(), Vec::new());
        if self.check(Token::Indent) {
            let items = self.parse_indented(|p| {
                if p.check(Token::Type) {
                    p.advance();
                    let aname = p.ident()?;
                    p.expect(Token::Is)?;
                    let aty = p.parse_type()?;
                    Ok(Either::Field((aname, aty)))
                } else {
                    Ok(Either::Method(p.parse_fn()?))
                }
            })?;
            for item in items {
                match item {
                    Either::Field(binding) => assoc_type_bindings.push(binding),
                    Either::Method(mut m) => {
                        Self::ensure_implicit_self(&mut m.params, m.span);
                        methods.push(m);
                    }
                }
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

    pub(in crate::parser) fn parse_supervisor_def(&mut self) -> Result<SupervisorDef, ParseError> {
        let sp = self.span();
        self.expect(Token::Supervisor)?;
        let name = self.ident()?;
        self.expect(Token::Newline)?;
        let mut strategy = SupervisorStrategy::OneForOne;
        let mut children = Vec::new();
        self.expect(Token::Indent)?;
        while !self.check(Token::Dedent) && !self.eof() {
            self.skip_nl();
            if self.check(Token::Dedent) || self.eof() {
                break;
            }
            let key = self.ident()?;
            if key == "strategy" {
                self.expect(Token::Is)?;
                let val = self.ident()?;
                if val == "one_for_one" {
                    strategy = SupervisorStrategy::OneForOne;
                } else if val == "one_for_all" {
                    strategy = SupervisorStrategy::OneForAll;
                } else if val == "rest_for_one" {
                    strategy = SupervisorStrategy::RestForOne;
                } else {
                    return Err(self.error(&format!(
                        "unknown supervisor strategy '{val}', expected one_for_one, one_for_all, or rest_for_one"
                    )));
                }
            } else if key == "children" {
                self.expect(Token::Newline)?;
                children = self.parse_indented(|p| p.ident())?;
            } else {
                return Err(self.error(&format!(
                    "unexpected supervisor field '{key}', expected 'strategy' or 'children'"
                )));
            }
            self.skip_nl();
        }
        if self.check(Token::Dedent) {
            self.advance();
        }
        Ok(SupervisorDef {
            name,
            strategy,
            children,
            span: sp,
        })
    }
}
