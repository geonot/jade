use crate::ast::*;
use crate::lexer::Token;
use crate::types::Type;

use super::{ParseError, Parser};

pub(super) enum Either<A, B> {
    Field(A),
    Method(B),
}

fn is_useless_top_expr(e: &Expr) -> bool {
    match e {
        Expr::None(_)
        | Expr::Void(_)
        | Expr::Int(..)
        | Expr::Float(..)
        | Expr::Str(..)
        | Expr::Bool(..)
        | Expr::Ident(..)
        | Expr::BinOp(..)
        | Expr::UnaryOp(..)
        | Expr::Field(..)
        | Expr::Index(..)
        | Expr::Ternary(..)
        | Expr::As(..)
        | Expr::Array(..)
        | Expr::Tuple(..)
        | Expr::Struct(..)
        | Expr::Lambda(..)
        | Expr::Placeholder(..)
        | Expr::IndexPlaceholder(..)
        | Expr::Ref(..)
        | Expr::Deref(..)
        | Expr::ListComp(..)
        | Expr::StoreQuery(..)
        | Expr::StoreCount(..)
        | Expr::StoreAll(..)
        | Expr::StoreGet(..)
        | Expr::StoreFirst(..)
        | Expr::StoreExists(..)
        | Expr::StoreDistinct(..)
        | Expr::AsFormat(..)
        | Expr::StrictCast(..)
        | Expr::Slice(..)
        | Expr::NamedArg(..)
        | Expr::Spread(..)
        | Expr::Grad(..)
        | Expr::Einsum(..)
        | Expr::Builder(..)
        | Expr::QualifiedIdent(..)
        | Expr::Query(..) => true,

        Expr::Call(..)
        | Expr::Method(..)
        | Expr::Pipe(..)
        | Expr::Block(..)
        | Expr::IfExpr(..)
        | Expr::Embed(..)
        | Expr::Syscall(..)
        | Expr::Spawn(..)
        | Expr::Send(..)
        | Expr::Receive(..)
        | Expr::Yield(..)
        | Expr::DispatchBlock(..)
        | Expr::ChannelCreate(..)
        | Expr::ChannelSend(..)
        | Expr::ChannelRecv(..)
        | Expr::Select(..)
        | Expr::Unreachable(..)
        | Expr::OfCall(..) => false,
    }
}

impl Parser {
    pub(super) fn parse_decl(&mut self) -> Result<Decl, ParseError> {
        match self.peek() {
            Token::Star => Ok(Decl::Fn(self.parse_fn()?)),
            Token::At => {
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
                self.advance();
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

                if let Token::Ident(first) = self.peek() {
                    if first.as_str() == "const"
                        && self.pos + 2 < self.tok.len()
                        && matches!(self.tok[self.pos + 1].token, Token::Ident(_))
                        && matches!(self.tok[self.pos + 2].token, Token::Is)
                    {
                        self.advance();
                        let name = self.ident()?;
                        self.expect(Token::Is)?;
                        let val = self.parse_expr()?;
                        if self.check(Token::Newline) {
                            self.advance();
                        }
                        return Ok(Decl::Const(name, val, sp));
                    }
                }

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
                    let stmt = self.parse_stmt()?;
                    if let Stmt::Expr(e) = &stmt {
                        if is_useless_top_expr(e) {
                            return Err(self.error(
                                "bare expression at top level has no effect; expected a declaration (`*function`, `type`, `actor`, `store`, ...) or a statement with side effects (`log`, `print`, function call, assignment)",
                            ));
                        }
                    }
                    if self.check(Token::Newline) {
                        self.advance();
                    }
                    Ok(Decl::TopStmt(stmt))
                }
            }
            _ => {
                let save = self.pos;
                match self.parse_stmt() {
                    Ok(stmt) => {
                        if let Stmt::Expr(e) = &stmt {
                            if is_useless_top_expr(e) {
                                return Err(self.error(
                                    "bare expression at top level has no effect; expected a declaration (`*function`, `type`, `actor`, `store`, ...) or a statement with side effects (`log`, `print`, function call, assignment)",
                                ));
                            }
                        }
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
}
mod actor_store;
mod r#fn;
mod trait_sup;
mod types;
pub(super) mod yield_scan;
