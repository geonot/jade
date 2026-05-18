//! Recursive-descent parser. Tokens → AST.

use crate::ast::*;
use crate::lexer::{Spanned, Token};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("line {line}:{col}: {msg}")]
    Error { line: u32, col: u32, msg: String },
}

pub struct Parser {
    tok: Vec<Spanned>,
    pos: usize,
    errors: Vec<ParseError>,
    suppress_by: bool,
    /// When set, `parse_ternary` will not consume a trailing `! else_expr`
    /// as the ternary-else form. Used by statement-level parsers (e.g. the
    /// `is` binding) so they can recognize `a is x() ! Variant` as the
    /// guard-form sugar instead of as a ternary.
    suppress_bang_else: bool,
    /// Queue of statements that desugaring inside `parse_stmt` wants to
    /// splice into the enclosing block *before* the statement actually
    /// returned from `parse_stmt`. Used by Layer-2 sugar (`a is x() ! V`)
    /// where one source statement expands into several. `parse_block`
    /// drains this queue between statements.
    pending_pre_stmts: Vec<Stmt>,
    /// Same, but spliced *after* the returned statement.
    pending_post_stmts: Vec<Stmt>,
    gensym_counter: u32,
    depth: u32,
    /// Active loop labels in scope; used so `break LABEL` / `continue LABEL`
    /// in `parse_break`/`parse_continue` can recognize the label and emit a
    /// magic placeholder expression instead of a bare identifier (which
    /// would otherwise be looked up as a variable).
    label_stack: Vec<crate::intern::Symbol>,
}

macro_rules! binop {
    ($name:ident, $next:ident, { $($t:path => $op:expr),+ $(,)? }) => {
        pub(in crate::parser) fn $name(&mut self) -> Result<Expr, ParseError> {
            let mut l = self.$next()?;
            loop { let sp = self.span(); match self.peek() {
                $($t => { self.advance(); let r = self.$next()?;
                    l = Expr::BinOp(Box::new(l), $op, Box::new(r), sp); })+
                _ => break,
            }} Ok(l)
        }
    };
}

mod decl;
mod expr;
mod stmt;

impl Parser {
    pub fn new(tok: Vec<Spanned>) -> Self {
        Self {
            tok,
            pos: 0,
            errors: Vec::new(),
            suppress_by: false,
            suppress_bang_else: false,
            pending_pre_stmts: Vec::new(),
            pending_post_stmts: Vec::new(),
            gensym_counter: 0,
            depth: 0,
            label_stack: Vec::new(),
        }
    }

    pub(crate) fn gensym(&mut self, prefix: &str) -> String {
        let id = self.gensym_counter;
        self.gensym_counter += 1;
        format!("__{prefix}_{id}")
    }

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut decls = Vec::new();
        while !self.eof() {
            self.skip_nl();
            if self.eof() {
                break;
            }
            match self.parse_decl() {
                Ok(d) => decls.push(d),
                Err(e) => {
                    self.errors.push(e);
                    if self.errors.len() >= 20 {
                        break;
                    }
                    self.synchronize();
                }
            }
        }
        if !self.errors.is_empty() {
            // Render only the first error's location; collapse remaining
            // errors into the message body so we don't print a useless
            // `line 0:0:` wrapper above the real diagnostic.
            let msgs: Vec<String> = self.errors.iter().map(|e| e.to_string()).collect();
            let (line, col) = match &self.errors[0] {
                ParseError::Error { line, col, .. } => (*line, *col),
            };
            let body = if msgs.len() == 1 {
                msgs.into_iter().next().unwrap()
            } else {
                msgs.join("\n")
            };
            // Strip the redundant `line N:M:` prefix from the head message
            // since the outer ParseError already carries it.
            let body = body
                .strip_prefix(&format!("line {line}:{col}: "))
                .map(|s| s.to_string())
                .unwrap_or(body);
            return Err(ParseError::Error {
                line,
                col,
                msg: body,
            });
        }
        let mut prog = Program { decls };
        // If there are top-level statements and no explicit *main, wrap them into an implicit *main
        let has_explicit_main = prog
            .decls
            .iter()
            .any(|d| matches!(d, Decl::Fn(f) if f.name == "main"));
        let has_top_stmts = prog
            .decls
            .iter()
            .any(|d| matches!(d, Decl::TopStmt(_) | Decl::Const(..)));
        if has_top_stmts && !has_explicit_main {
            let mut body_stmts: Vec<Stmt> = Vec::new();
            let mut remaining_decls: Vec<Decl> = Vec::new();
            let mut main_span = Span::dummy();
            for d in prog.decls.drain(..) {
                match d {
                    Decl::TopStmt(stmt) => {
                        if main_span.line == 0 {
                            main_span = stmt.span();
                        }
                        body_stmts.push(stmt);
                    }
                    Decl::Const(name, val, sp) => {
                        // Keep constants as top-level Decl::Const so the typer
                        // registers them in self.consts (visible to type methods).
                        if main_span.line == 0 {
                            main_span = sp;
                        }
                        remaining_decls.push(Decl::Const(name, val, sp));
                    }
                    other => remaining_decls.push(other),
                }
            }
            if !body_stmts.is_empty() {
                remaining_decls.push(Decl::Fn(Fn {
                    name: Symbol::intern("main"),
                    type_params: Vec::new(),
                    type_bounds: Vec::new(),
                    params: Vec::new(),
                    ret: None,
                    error_types: Vec::new(),
                    body: body_stmts,
                    is_generator: false,
                    span: main_span,
                    attrs: FnAttrs::default(),
                }));
            }
            prog.decls = remaining_decls;
        }
        desugar_multi_clause_fns(&mut prog);
        Ok(prog)
    }

    /// Skip tokens until the next likely declaration boundary.
    fn synchronize(&mut self) {
        loop {
            if self.eof() {
                break;
            }
            match self.peek() {
                // Declaration-starting tokens
                Token::Star
                | Token::Type
                | Token::Enum
                | Token::Extern
                | Token::Use
                | Token::Actor
                | Token::Store
                | Token::Trait
                | Token::Impl
                | Token::Test
                | Token::Pub
                | Token::Supervisor
                | Token::Alias => break,
                Token::Newline => {
                    self.advance();
                    // After newline, check if next token starts a declaration
                    if self.eof() {
                        break;
                    }
                    match self.peek() {
                        Token::Star
                        | Token::Type
                        | Token::Enum
                        | Token::Extern
                        | Token::Use
                        | Token::Actor
                        | Token::Store
                        | Token::Trait
                        | Token::Impl
                        | Token::Test
                        | Token::Pub
                        | Token::Supervisor
                        | Token::Alias => break,
                        _ => {}
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    fn peek(&self) -> &Token {
        if self.pos < self.tok.len() {
            &self.tok[self.pos].token
        } else {
            &Token::Eof
        }
    }

    fn peek_at(&self, offset: usize) -> &Token {
        let idx = self.pos + offset;
        if idx < self.tok.len() {
            &self.tok[idx].token
        } else {
            &Token::Eof
        }
    }

    fn check(&self, t: Token) -> bool {
        if self.pos < self.tok.len() {
            std::mem::discriminant(&self.tok[self.pos].token) == std::mem::discriminant(&t)
        } else {
            matches!(t, Token::Eof)
        }
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

    /// Skip newlines, indents, and dedents inside parenthesized/bracketed contexts.
    fn skip_ws(&mut self) {
        while self.check(Token::Newline) || self.check(Token::Indent) || self.check(Token::Dedent) {
            self.advance();
        }
    }

    fn expect(&mut self, t: Token) -> Result<(), ParseError> {
        if self.check(Token::Eof) && !matches!(t, Token::Eof) {
            return Err(self.error(&format!("expected {t}, got EOF")));
        }
        if self.pos < self.tok.len()
            && std::mem::discriminant(&self.tok[self.pos].token) == std::mem::discriminant(&t)
        {
            self.advance();
            Ok(())
        } else {
            Err(self.error(&format!("expected {t}, got {}", self.peek())))
        }
    }

    fn ident(&mut self) -> Result<Symbol, ParseError> {
        if self.pos < self.tok.len() {
            match &self.tok[self.pos].token {
                Token::Ident(n) => {
                    let name = n.clone();
                    self.advance();
                    return Ok(name);
                }
                Token::Set => {
                    self.advance();
                    return Ok("set".into());
                }
                Token::Build => {
                    self.advance();
                    return Ok("build".into());
                }
                Token::From => {
                    self.advance();
                    return Ok("from".into());
                }
                Token::To => {
                    self.advance();
                    return Ok("to".into());
                }
                Token::Insert => {
                    self.advance();
                    return Ok("insert".into());
                }
                // `log` is a builtin call keyword but is also a perfectly
                // valid method/field name on user types (e.g. `obj.log()`,
                // which the typer dispatches to a user `log` method or to
                // the default `log(self)` formatter). Accept it here so
                // method-position parsing succeeds.
                Token::Log => {
                    self.advance();
                    return Ok("log".into());
                }
                // Logical/bitwise keyword operators are also valid as
                // function/method/field names (e.g. `*xor(...)`,
                // `obj.xor(...)`, `stream.and(...)`). At an
                // identifier-expected position there is no ambiguity with
                // the operator form.
                Token::Xor => {
                    self.advance();
                    return Ok("xor".into());
                }
                Token::And => {
                    self.advance();
                    return Ok("and".into());
                }
                Token::Or => {
                    self.advance();
                    return Ok("or".into());
                }
                Token::Not => {
                    self.advance();
                    return Ok("not".into());
                }
                _ => {}
            }
        }
        Err(self.error(&format!("expected identifier, got {}", self.peek())))
    }

    fn parse_body(&mut self) -> Result<Block, ParseError> {
        if self.check(Token::Is) {
            self.advance();
            Ok(vec![Stmt::Expr(self.parse_expr()?)])
        } else {
            self.expect(Token::Newline)?;
            self.parse_block()
        }
    }

    fn parse_indented<T>(
        &mut self,
        mut f: impl FnMut(&mut Self) -> Result<T, ParseError>,
    ) -> Result<Vec<T>, ParseError> {
        self.expect(Token::Indent)?;
        let mut items = Vec::new();
        while !self.check(Token::Dedent) && !self.eof() {
            self.skip_nl();
            if self.check(Token::Dedent) || self.eof() {
                break;
            }
            items.push(f(self)?);
            self.skip_nl();
        }
        if self.check(Token::Dedent) {
            self.advance();
        }
        Ok(items)
    }

    fn error(&self, msg: &str) -> ParseError {
        let sp = self.span();
        ParseError::Error {
            line: sp.line,
            col: sp.col,
            msg: msg.into(),
        }
    }

    // ── Access-modifier parsing helpers ────────────────────────────────
    //
    // The four access modifiers (`copy`, `ref`, `mut`, `take`) are
    // contextual keywords: the lexer emits them as plain `Ident` tokens
    // and the parser disambiguates them by position. They appear in
    // three grammatical slots:
    //
    //   1. After `is` in a bind:    `x is ref users.get(0)`
    //   2. After `as` in a typed
    //      parameter/field position: `f(items as ref Vec(Row))`
    //   3. Immediately after `for`
    //      before the binder:        `for ref user in users`
    //
    // To avoid breaking expressions whose head happens to be an
    // identifier named `copy`/`ref`/`mut`/`take`, the lookahead requires
    // the modifier to be followed by something that *cannot* be the
    // start of a call/index/method chain on the modifier itself.
    //
    // See `docs/access-semantics.md` \u00a72 for the full rationale.

    /// True if `name` is one of the four access-modifier keywords.
    fn is_access_mod_keyword(name: &str) -> bool {
        matches!(name, "copy" | "ref" | "mut" | "take")
    }

    fn ident_to_access_mod(name: &str) -> Option<crate::ast::AccessMod> {
        match name {
            "copy" => Some(crate::ast::AccessMod::Copy),
            "ref" => Some(crate::ast::AccessMod::Ref),
            "mut" => Some(crate::ast::AccessMod::Mut),
            "take" => Some(crate::ast::AccessMod::Take),
            _ => None,
        }
    }

    /// Try to consume an access modifier appearing immediately after `is`
    /// in a bind. The lookahead requires the next-next token to be a
    /// plausible expression-start that does NOT chain off the modifier
    /// itself (i.e. NOT `(`, `.`, `[`, etc.).
    pub(in crate::parser) fn try_parse_access_mod_after_is(
        &mut self,
    ) -> Option<crate::ast::AccessMod> {
        let name = match self.peek() {
            Token::Ident(n) if Self::is_access_mod_keyword(&n.as_str()) => n.as_str().to_string(),
            _ => return None,
        };
        // Disambiguate `x is copy(foo)` (call) from `x is copy foo` (modifier).
        match self.peek_at(1) {
            Token::LParen
            | Token::Dot
            | Token::LBracket
            | Token::DotDotDot
            | Token::Tilde
            | Token::Newline
            | Token::Eof => return None,
            _ => {}
        }
        let am = Self::ident_to_access_mod(&name)?;
        self.advance();
        Some(am)
    }

    /// Try to consume an access modifier appearing at a type-annotation
    /// position (after `as`, before a Type). Slightly different
    /// disambiguation: a Type must follow, so `ref(...)` is NOT a Type
    /// and we accept the modifier whenever the next-next token can start
    /// a Type identifier or a builtin type keyword.
    pub(in crate::parser) fn try_parse_access_mod_at_type_pos(
        &mut self,
    ) -> Option<crate::ast::AccessMod> {
        let name = match self.peek() {
            Token::Ident(n) if Self::is_access_mod_keyword(&n.as_str()) => n.as_str().to_string(),
            _ => return None,
        };
        // Next must look like the start of a Type.
        match self.peek_at(1) {
            Token::Ident(_) => {}
            // Builtin/keyword types that may follow.
            _ => return None,
        }
        let am = Self::ident_to_access_mod(&name)?;
        self.advance();
        Some(am)
    }

    /// Try to consume an access modifier at the for-loop binder slot:
    /// `for copy/ref/mut/take BINDER in EXPR`. The next-next token must
    /// be the binder name (an Ident).
    pub(in crate::parser) fn try_parse_access_mod_at_binder_pos(
        &mut self,
    ) -> Option<crate::ast::AccessMod> {
        let name = match self.peek() {
            Token::Ident(n) if Self::is_access_mod_keyword(&n.as_str()) => n.as_str().to_string(),
            _ => return None,
        };
        match self.peek_at(1) {
            Token::Ident(_) => {}
            _ => return None,
        }
        let am = Self::ident_to_access_mod(&name)?;
        self.advance();
        Some(am)
    }
}

fn desugar_multi_clause_fns(prog: &mut Program) {
    let mut name_indices: Vec<(Symbol, Vec<usize>)> = Vec::new();
    let mut seen: std::collections::HashMap<Symbol, usize> = std::collections::HashMap::new();

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

    let multi_groups: Vec<(Symbol, Vec<usize>)> = name_indices
        .into_iter()
        .filter(|(_, indices)| indices.len() > 1)
        .collect();

    if multi_groups.is_empty() {
        return;
    }

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

        prog.decls[indices[0]] = Decl::Fn(merged);
        for &i in &indices[1..] {
            to_remove.insert(i);
        }
    }

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

    for (i, c) in clauses.iter().enumerate().skip(1) {
        if c.params.len() != param_count {
            panic!(
                "{}: multi-clause function `{}` clause {} has {} parameters, but first clause has {}",
                c.span.loc(),
                first.name,
                i + 1,
                c.params.len(),
                param_count
            );
        }
    }

    let mut unified_params: Vec<Param> = Vec::new();
    for pi in 0..param_count {
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
            .unwrap_or_else(|| Symbol::intern(&format!("__arg{pi}")));

        let ty = clauses
            .iter()
            .find_map(|c| c.params.get(pi).and_then(|p| p.ty.clone()));

        unified_params.push(Param {
            name: real_name,
            ty,
            default: None,
            literal: None,
            access_mod: None,
            span: sp,
        });
    }

    let mut guarded: Vec<&Fn> = Vec::new();
    let mut catchall: Option<&Fn> = None;
    for c in clauses {
        if c.params.iter().any(|p| p.literal.is_some()) {
            guarded.push(c);
        } else {
            catchall = Some(c);
        }
    }

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
        for (pi, p) in clause.params.iter().enumerate() {
            if p.literal.is_none() && p.name != unified_params[pi].name {
                body.push(Stmt::Bind(crate::ast::Bind {
                    name: p.name.clone(),
                    value: Expr::Ident(unified_params[pi].name.clone(), sp),
                    ty: None,
                    atomic: false,
                    access_mod: None,
                    span: sp,
                }));
            }
        }
        body.extend(clause.body.clone());
        body
    };

    let body = if guarded.is_empty() {
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
        error_types: first.error_types.clone(),
        body,
        is_generator: false,
        span: sp,
        attrs: FnAttrs::default(),
    }
}

#[cfg(test)]
mod tests;
