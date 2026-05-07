//! Parser arms for top-level declarations (fn, type, store, actor, import).

use crate::ast::*;
use crate::lexer::Token;
use crate::types::Type;

use super::{ParseError, Parser};

enum Either<A, B> {
    Field(A),
    Method(B),
}

fn body_contains_yield(body: &[Stmt]) -> bool {
    body.iter().any(|s| stmt_has_yield(s))
}

fn stmt_has_yield(s: &Stmt) -> bool {
    match s {
        Stmt::Expr(e) | Stmt::Ret(Some(e), _) | Stmt::Break(Some(e), _) => expr_has_yield(e),
        Stmt::If(i) => {
            expr_has_yield(&i.cond)
                || body_contains_yield(&i.then)
                || i.elifs
                    .iter()
                    .any(|(c, b)| expr_has_yield(c) || body_contains_yield(b))
                || i.els.as_ref().is_some_and(|b| body_contains_yield(b))
        }
        Stmt::While(w) => expr_has_yield(&w.cond) || body_contains_yield(&w.body),
        Stmt::For(f) => body_contains_yield(&f.body),
        Stmt::Loop(l) => body_contains_yield(&l.body),
        Stmt::Match(m) => m.arms.iter().any(|a| body_contains_yield(&a.body)),
        _ => false,
    }
}

fn expr_has_yield(e: &Expr) -> bool {
    matches!(e, Expr::Yield(_, _))
}

impl Parser {
    fn parse_fn_attrs(&mut self) -> Result<FnAttrs, ParseError> {
        let mut attrs = FnAttrs::default();
        while self.check(Token::At) {
            self.advance();
            let attr = self.ident()?;
            if attr == "inline" {
                attrs.inline = true;
            } else if attr == "noinline" {
                attrs.noinline = true;
            } else if attr == "cold" {
                attrs.cold = true;
            } else if attr == "hot" {
                attrs.hot = true;
            } else {
                return Err(self.error(&format!("unknown function attribute: @{attr}")));
            }
        }
        Ok(attrs)
    }

    pub(super) fn parse_decl(&mut self) -> Result<Decl, ParseError> {
        match self.peek() {
            Token::Star => Ok(Decl::Fn(self.parse_fn()?)),
            Token::At => {
                // Function annotations: @inline, @noinline, @cold, @hot
                let attrs = self.parse_fn_attrs()?;
                self.skip_nl();
                if self.check(Token::Star) {
                    let mut f = self.parse_fn()?;
                    f.attrs = attrs;
                    Ok(Decl::Fn(f))
                } else {
                    Err(self.error("expected function declaration after @annotation"))
                }
            }
            Token::Type | Token::Pub => Ok(Decl::Type(self.parse_type_def()?)),
            Token::Enum => Ok(Decl::Enum(self.parse_enum_def()?)),
            Token::Extern => Ok(Decl::Extern(self.parse_extern()?)),
            Token::Use => Ok(Decl::Use(self.parse_use_decl()?)),
            Token::Err => Ok(Decl::ErrDef(self.parse_err_def()?)),
            Token::Test => Ok(Decl::Test(self.parse_test_block()?)),
            Token::Actor => Ok(Decl::Actor(self.parse_actor_def()?)),
            Token::Store => Ok(Decl::Store(self.parse_store_def()?)),
            Token::Migration => Ok(Decl::Migration(self.parse_migration_def()?)),
            Token::View => Ok(Decl::View(self.parse_view_def()?)),
            Token::Trait => Ok(Decl::Trait(self.parse_trait_def()?)),
            Token::Impl => Ok(Decl::Impl(self.parse_impl_block()?)),
            Token::Supervisor => Ok(Decl::Supervisor(self.parse_supervisor_def()?)),
            Token::Global => {
                let sp = self.span();
                self.advance(); // consume 'global'
                let name = self.ident()?;
                self.expect(Token::Is)?;
                let val = self.parse_expr()?;
                if self.check(Token::Newline) {
                    self.advance();
                }
                Ok(Decl::Global(name, val, sp))
            }
            Token::Alias => {
                let sp = self.span();
                self.advance();
                let name = self.ident()?;
                self.expect(Token::Is)?;
                let ty = self.parse_type()?;
                Ok(Decl::TypeAlias(name, ty, sp))
            }
            Token::Ident(_) => {
                let sp = self.span();
                // Look ahead: if next token is `Is` and the one after the ident is directly `Is`,
                // this is a const/binding. Otherwise it's a top-level statement (expr, method call, etc.)
                if self.pos + 1 < self.tok.len()
                    && matches!(self.tok[self.pos + 1].token, Token::Is)
                {
                    let name = self.ident()?;
                    self.expect(Token::Is)?;
                    let val = self.parse_expr()?;
                    if self.check(Token::Newline) {
                        self.advance();
                    }
                    Ok(Decl::Const(name, val, sp))
                } else {
                    // Top-level statement: expression statement, method call, assignment, etc.
                    let stmt = self.parse_stmt()?;
                    if self.check(Token::Newline) {
                        self.advance();
                    }
                    Ok(Decl::TopStmt(stmt))
                }
            }
            _ => {
                // Try to parse as a top-level expression/statement
                // This handles keywords like `log`, `print`, `assert`, and literals
                let save = self.pos;
                match self.parse_stmt() {
                    Ok(stmt) => {
                        if self.check(Token::Newline) {
                            self.advance();
                        }
                        Ok(Decl::TopStmt(stmt))
                    }
                    Err(_) => {
                        self.pos = save;
                        Err(self.error(
                            "expected *, type, enum, extern, use, err, test, actor, store, trait, or impl",
                        ))
                    }
                }
            }
        }
    }

    fn parse_type_params(&mut self) -> (Vec<Symbol>, Vec<(Symbol, Vec<Symbol>)>) {
        let mut tp = Vec::new();
        let mut bounds = Vec::new();
        if !self.check(Token::Of) {
            return (tp, bounds);
        }
        self.advance();
        while let Token::Ident(_) = self.peek() {
            let name = self.ident().unwrap();
            if self.check(Token::Colon) {
                self.advance();
                let mut bs = Vec::new();
                match self.ident() {
                    Ok(b) => bs.push(b),
                    Err(_) => break,
                }
                while self.check(Token::Plus) {
                    self.advance();
                    match self.ident() {
                        Ok(b) => bs.push(b),
                        Err(_) => break,
                    }
                }
                bounds.push((name, bs));
            }
            tp.push(name);
            if !self.check(Token::Comma) {
                break;
            }
            self.advance();
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
            self.expect(Token::As)?;
            let pty = self.parse_type()?;
            params.push((pname, pty));
            if !self.check(Token::RParen) && !variadic {
                self.expect(Token::Comma)?;
            }
        }
        self.expect(Token::RParen)?;
        let ret = if self.check(Token::Returns) {
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
            while !self.check(Token::RParen) && !self.eof() {
                params.push(self.parse_fn_param(params.len(), true)?);
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
                params.push(self.parse_fn_param(params.len(), false)?);
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

        // Optional error union: `returns T ! E1 ! E2 ...` or `! E1 ! E2 ...`
        // Each `! Ident` after the return type names an err-type that this
        // function may early-return via `! Variant`.
        let mut error_types = Vec::new();
        while self.check(Token::Bang) {
            self.advance();
            error_types.push(self.parse_type()?);
        }

        let body = self.parse_body()?;
        let is_generator = body_contains_yield(&body);

        Ok(Fn {
            name,
            type_params,
            type_bounds,
            params,
            ret,
            error_types,
            body,
            is_generator,
            attrs: FnAttrs::default(),
            span: sp,
        })
    }

    fn parse_fn_param(&mut self, idx: usize, typed: bool) -> Result<Param, ParseError> {
        match self.peek() {
            Token::Int(_)
            | Token::CharLit(_)
            | Token::Float(_)
            | Token::True
            | Token::False
            | Token::Str(_) => {
                let lit_sp = self.span();
                let lit_expr = self.parse_literal_token()?;
                Ok(Param {
                    name: Symbol::intern(&format!("__arg{idx}")),
                    ty: None,
                    default: None,
                    literal: Some(lit_expr),
                    span: lit_sp,
                })
            }
            Token::Minus => {
                let lit_sp = self.span();
                let lit_expr = self.parse_unary()?;
                Ok(Param {
                    name: Symbol::intern(&format!("__arg{idx}")),
                    ty: None,
                    default: None,
                    literal: Some(lit_expr),
                    span: lit_sp,
                })
            }
            _ => self.parse_param(typed),
        }
    }

    pub(super) fn parse_param(&mut self, typed: bool) -> Result<Param, ParseError> {
        let sp = self.span();
        let name = self.ident()?;
        let ty = if typed && self.check(Token::As) {
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
                    Either::Method(m) => methods.push(m),
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

    fn parse_layout_attrs(&mut self) -> Result<crate::ast::LayoutAttrs, ParseError> {
        let mut layout = crate::ast::LayoutAttrs::default();
        while self.check(Token::At) {
            self.advance();
            // Handle keywords that are also layout attribute names
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
            } else {
                return Err(self.error(&format!("unknown layout attribute: @{attr}")));
            }
        }
        Ok(layout)
    }

    fn parse_field(&mut self) -> Result<Field, ParseError> {
        let sp = self.span();
        let name = self.ident()?;
        let ty = if self.check(Token::As) {
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
                ty: self.ident_to_type(&n.as_str()),
            })
        }
    }

    pub(super) fn parse_use_decl(&mut self) -> Result<UseDecl, ParseError> {
        let sp = self.span();
        self.expect(Token::Use)?;
        let mut path = vec![self.ident()?];
        while self.check(Token::Slash) {
            self.advance();
            path.push(self.ident()?);
        }
        // Selective imports: `use foo bar` or `use foo [bar, baz]`
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
        // Import alias: `use long_module as lmn`
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

    fn parse_err_def(&mut self) -> Result<ErrDef, ParseError> {
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

    fn parse_test_block(&mut self) -> Result<TestBlock, ParseError> {
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

    fn parse_actor_def(&mut self) -> Result<ActorDef, ParseError> {
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

    fn parse_handler(&mut self) -> Result<Handler, ParseError> {
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
            while !self.check(Token::Newline) && !self.check(Token::Is) && !self.eof() {
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

    fn parse_store_def(&mut self) -> Result<StoreDef, ParseError> {
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

    fn parse_store_field(&mut self) -> Result<crate::ast::StoreField, ParseError> {
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
    fn parse_migration_def(&mut self) -> Result<crate::ast::MigrationDef, ParseError> {
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
    fn parse_view_def(&mut self) -> Result<crate::ast::ViewDef, ParseError> {
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
    fn parse_alter_op(&mut self) -> Result<crate::ast::AlterOp, ParseError> {
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
                    Ok(AlterAction::Drop { name: field_name.as_str() })
                } else if action_name == "rename" {
                    let from = p.ident()?;
                    p.expect(Token::To)?;
                    let to = p.ident()?;
                    Ok(AlterAction::Rename { from: from.as_str(), to: to.as_str() })
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

    fn parse_trait_def(&mut self) -> Result<TraitDef, ParseError> {
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
                    Either::Method(m) => methods.push(m),
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

    fn parse_supervisor_def(&mut self) -> Result<SupervisorDef, ParseError> {
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
