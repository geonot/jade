use crate::ast::*;
use crate::lexer::Token;
use crate::types::Type;

use super::super::{ParseError, Parser};
use super::Either;

impl Parser {
    pub(in crate::parser) fn parse_actor_def(&mut self) -> Result<ActorDef, ParseError> {
        let sp = self.span();
        self.expect(Token::Actor)?;
        let name = self.ident()?;
        self.expect(Token::Newline)?;
        let (mut fields, mut handlers) = (Vec::new(), Vec::new());
        if self.check(Token::Indent) {
            let items = self.parse_indented(|p| {
                if p.check(Token::At) || p.check(Token::Star) {
                    Ok(Either::Method(p.parse_handler()?))
                } else {
                    Ok(Either::Field(p.parse_field()?))
                }
            })?;
            for item in items {
                match item {
                    Either::Field(f) => fields.push(f),
                    Either::Method(h) => handlers.push(h),
                }
            }
        }
        let loop_count = handlers.iter().filter(|h| h.is_loop).count();
        if loop_count > 1 {
            return Err(self.error("actor may define at most one *loop handler"));
        }
        Ok(ActorDef {
            name,
            fields,
            handlers,
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_handler(&mut self) -> Result<Handler, ParseError> {
        let sp = self.span();
        let mut is_loop = false;
        let mut loop_sleep_ms = None;

        let name = if self.check(Token::At) {
            self.advance();
            self.ident()?
        } else {
            self.expect(Token::Star)?;
            if self.check(Token::Loop) {
                self.advance();
                is_loop = true;
                Symbol::intern("loop")
            } else {
                self.ident()?
            }
        };

        let mut params = Vec::new();
        if is_loop {
            if !self.check(Token::Newline) && !self.check(Token::Is) && !self.eof() {
                loop_sleep_ms = Some(self.parse_expr()?);
            }
        } else {
            // Allow `*name returns T` with no params (no parens, no params before `returns`).
            while !self.check(Token::Newline)
                && !self.check(Token::Is)
                && !self.check(Token::Returns)
                && !self.eof()
            {
                params.push(self.parse_param(true)?);
                if self.check(Token::Comma) {
                    self.advance();
                }
            }
        }
        let body = self.parse_body()?;
        Ok(Handler {
            name,
            params,
            is_loop,
            loop_sleep_ms,
            body,
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_store_def(&mut self) -> Result<StoreDef, ParseError> {
        let sp = self.span();
        self.expect(Token::Store)?;
        let name = self.ident()?;

        // Parse store-level decorators: @simple, @mem, @transient, @versioned, @graph, @kv, @vector(N), @timeseries(field)
        let mut decorators = Vec::new();
        while self.check(Token::At) {
            self.advance();
            let attr = self.ident()?;
            if attr == "simple" {
                decorators.push(crate::ast::StoreDecorator::Simple);
            } else if attr == "mem" {
                decorators.push(crate::ast::StoreDecorator::Mem);
            } else if attr == "transient" {
                decorators.push(crate::ast::StoreDecorator::Transient);
            } else if attr == "versioned" {
                decorators.push(crate::ast::StoreDecorator::Versioned);
            } else if attr == "graph" {
                decorators.push(crate::ast::StoreDecorator::Graph);
            } else if attr == "kv" {
                decorators.push(crate::ast::StoreDecorator::Kv);
            } else if attr == "vector" {
                self.expect(Token::LParen)?;
                let n = match self.peek() {
                    Token::Int(v) => {
                        let n = *v as u64;
                        self.advance();
                        n
                    }
                    _ => return Err(self.error("expected vector dimension")),
                };
                self.expect(Token::RParen)?;
                decorators.push(crate::ast::StoreDecorator::Vector(n));
            } else if attr == "timeseries" {
                self.expect(Token::LParen)?;
                let field = self.ident()?;
                self.expect(Token::RParen)?;
                decorators.push(crate::ast::StoreDecorator::TimeSeries(field));
            } else if attr == "before_insert" {
                self.expect(Token::LParen)?;
                let fname = self.ident()?;
                self.expect(Token::RParen)?;
                decorators.push(crate::ast::StoreDecorator::BeforeInsert(fname));
            } else if attr == "after_insert" {
                self.expect(Token::LParen)?;
                let fname = self.ident()?;
                self.expect(Token::RParen)?;
                decorators.push(crate::ast::StoreDecorator::AfterInsert(fname));
            } else if attr == "before_delete" {
                self.expect(Token::LParen)?;
                let fname = self.ident()?;
                self.expect(Token::RParen)?;
                decorators.push(crate::ast::StoreDecorator::BeforeDelete(fname));
            } else if attr == "after_delete" {
                self.expect(Token::LParen)?;
                let fname = self.ident()?;
                self.expect(Token::RParen)?;
                decorators.push(crate::ast::StoreDecorator::AfterDelete(fname));
            } else if attr == "column" {
                decorators.push(crate::ast::StoreDecorator::Column);
            } else {
                return Err(self.error(&format!("unknown store decorator: @{attr}")));
            }
        }

        self.expect(Token::Newline)?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
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
                    fields.push(self.parse_store_field()?);
                    self.skip_nl();
                }
            }
            if self.check(Token::Dedent) {
                self.advance();
            }
        }
        Ok(StoreDef {
            name,
            decorators,
            fields,
            methods,
            span: sp,
        })
    }

    pub(in crate::parser) fn parse_store_field(
        &mut self,
    ) -> Result<crate::ast::StoreField, ParseError> {
        let sp = self.span();

        // Check for relationship prefix: &
        let is_relation = if self.check(Token::Ampersand) {
            self.advance();
            true
        } else {
            false
        };

        let name = self.ident()?;
        let (ty, is_has_many) = if self.check(Token::As) {
            self.advance();
            // Check for [Type] (has-many relation)
            if self.check(Token::LBracket) {
                self.advance();
                let inner = self.parse_type()?;
                self.expect(Token::RBracket)?;
                (Some(inner), true)
            } else {
                (Some(self.parse_type()?), false)
            }
        } else {
            (None, false)
        };

        // Parse field-level decorators: @index, @unique, @sorted, @transient, @increment, @required, @versioned, @default(val)
        // Also relation decorators: @cascade, @lazy
        let mut field_decorators = Vec::new();
        while self.check(Token::At) {
            self.advance();
            let attr = self.ident()?;
            if attr == "index" {
                field_decorators.push(crate::ast::FieldDecorator::Index);
            } else if attr == "unique" {
                field_decorators.push(crate::ast::FieldDecorator::Unique);
            } else if attr == "sorted" {
                field_decorators.push(crate::ast::FieldDecorator::Sorted);
            } else if attr == "transient" {
                field_decorators.push(crate::ast::FieldDecorator::Transient);
            } else if attr == "increment" {
                field_decorators.push(crate::ast::FieldDecorator::Increment);
            } else if attr == "required" {
                field_decorators.push(crate::ast::FieldDecorator::Required);
            } else if attr == "versioned" {
                field_decorators.push(crate::ast::FieldDecorator::Versioned);
            } else if attr == "cascade" {
                field_decorators.push(crate::ast::FieldDecorator::Cascade);
            } else if attr == "lazy" {
                field_decorators.push(crate::ast::FieldDecorator::Lazy);
            } else if attr == "bloom" {
                field_decorators.push(crate::ast::FieldDecorator::Bloom);
            } else if attr == "search" {
                field_decorators.push(crate::ast::FieldDecorator::Search);
            } else if attr == "default" {
                self.expect(Token::LParen)?;
                // Read the default value as a string token
                let val = match self.peek() {
                    Token::Str(s) => {
                        let v = s.clone();
                        self.advance();
                        v
                    }
                    Token::Int(n) => {
                        let v = n.to_string();
                        self.advance();
                        v
                    }
                    Token::Float(f) => {
                        let v = f.to_string();
                        self.advance();
                        v
                    }
                    Token::True => {
                        self.advance();
                        "true".to_string()
                    }
                    Token::False => {
                        self.advance();
                        "false".to_string()
                    }
                    _ => return Err(self.error("expected default value")),
                };
                self.expect(Token::RParen)?;
                field_decorators.push(crate::ast::FieldDecorator::Default(val));
            } else {
                return Err(self.error(&format!("unknown field decorator: @{attr}")));
            }
        }

        let default = if self.check(Token::Is) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };

        Ok(crate::ast::StoreField {
            name,
            ty,
            default,
            decorators: field_decorators,
            is_relation,
            is_has_many,
            span: sp,
        })
    }

    /// Parse `migration 'name' version N` block with indented up/down containing alter ops.
    pub(in crate::parser) fn parse_migration_def(
        &mut self,
    ) -> Result<crate::ast::MigrationDef, ParseError> {
        use crate::ast::MigrationDef;
        let sp = self.span();
        self.expect(Token::Migration)?;

        // migration 'name' version N
        let name = match self.peek() {
            Token::Str(s) => {
                let n = Symbol::intern(s);
                self.advance();
                n
            }
            _ => return Err(self.error("expected migration name string")),
        };

        // expect 'version' identifier
        match self.peek() {
            Token::Ident(s) if s == "version" => {
                self.advance();
            }
            _ => return Err(self.error("expected 'version'")),
        }

        let version = match self.peek() {
            Token::Int(n) => {
                let v = *n;
                self.advance();
                v
            }
            _ => return Err(self.error("expected version number")),
        };

        self.expect(Token::Newline)?;

        let mut up = Vec::new();
        let mut down = Vec::new();

        if self.check(Token::Indent) {
            let blocks = self.parse_indented(|p| {
                let ident = p.ident()?;
                p.expect(Token::Newline)?;
                if ident == "up" || ident == "down" {
                    let mut alter_ops = Vec::new();
                    if p.check(Token::Indent) {
                        let ops = p.parse_indented(|p2| p2.parse_alter_op())?;
                        alter_ops = ops;
                    }
                    Ok((ident, alter_ops))
                } else {
                    Err(p.error(&format!("expected 'up' or 'down', got '{ident}'")))
                }
            })?;
            for (dir, ops) in blocks {
                if dir == "up" {
                    up = ops;
                } else if dir == "down" {
                    down = ops;
                }
            }
        }

        Ok(MigrationDef {
            name,
            version,
            up,
            down,
            span: sp,
        })
    }

    /// Parse `view Name from source_store\n    where ...\n    ...`
    pub(in crate::parser) fn parse_view_def(&mut self) -> Result<crate::ast::ViewDef, ParseError> {
        let sp = self.span();
        self.expect(Token::View)?;
        let name = self.ident()?;
        self.expect(Token::From)?;
        let source = self.ident()?;
        self.expect(Token::Newline)?;

        let clauses = if self.check(Token::Indent) {
            self.parse_indented(Self::parse_query_clause)?
        } else {
            Vec::new()
        };

        Ok(crate::ast::ViewDef {
            name,
            source,
            clauses,
            span: sp,
        })
    }

    /// Parse `alter <store_name>` with indented add/drop/rename actions.
    pub(in crate::parser) fn parse_alter_op(&mut self) -> Result<crate::ast::AlterOp, ParseError> {
        use crate::ast::{AlterAction, AlterOp};
        // expect 'alter' identifier
        match self.peek() {
            Token::Ident(s) if s == "alter" => {
                self.advance();
            }
            _ => return Err(self.error("expected 'alter'")),
        }
        let store_name = self.ident()?;
        self.expect(Token::Newline)?;

        let mut actions = Vec::new();
        if self.check(Token::Indent) {
            let acts = self.parse_indented(|p| {
                let action_name = p.ident()?;
                if action_name == "add" {
                    let field_name = p.ident()?;
                    p.expect(Token::As)?;
                    let ty = p.parse_type()?;
                    // optional: default <value>
                    let default = if p.check(Token::Default) {
                        p.advance();
                        Some(p.parse_expr()?)
                    } else {
                        None
                    };
                    Ok(AlterAction::Add {
                        name: field_name.as_str(),
                        ty,
                        default,
                    })
                } else if action_name == "drop" {
                    let field_name = p.ident()?;
                    Ok(AlterAction::Drop {
                        name: field_name.as_str(),
                    })
                } else if action_name == "rename" {
                    let from = p.ident()?;
                    p.expect(Token::To)?;
                    let to = p.ident()?;
                    Ok(AlterAction::Rename {
                        from: from.as_str(),
                        to: to.as_str(),
                    })
                } else {
                    Err(p.error(&format!(
                        "expected 'add', 'drop', or 'rename', got '{action_name}'"
                    )))
                }
            })?;
            actions = acts;
        }

        Ok(AlterOp {
            store_name,
            actions,
        })
    }
}
