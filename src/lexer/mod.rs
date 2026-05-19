//! Source → token stream. The KEYWORDS table is the single source of truth for reserved words; see `docs/lexer/keywords.md`.

use crate::ast::Span;
use crate::intern::Symbol;
use std::collections::HashMap;
use std::sync::LazyLock;

mod literals;
mod token;
pub use token::*;

#[derive(Debug, Clone)]
pub struct Spanned {
    pub token: Token,
    pub span: Span,
}

#[derive(Debug, thiserror::Error)]
pub enum LexError {
    #[error("line {line}: {msg}")]
    Error { line: u32, col: u32, msg: String },
}

pub struct Lexer<'s> {
    src: &'s [u8],
    pos: usize,
    line: u32,
    col: u32,
    indents: Vec<u32>,
    pending: Vec<Spanned>,
    sol: bool,
    nl: bool,
    file: Option<crate::intern::Symbol>,
    /// True iff the most recently emitted significant token was `Dot`.
    /// When set, the next identifier is lexed as a plain `Ident` without
    /// keyword promotion — so e.g. `ch.send(x)`, `xs.take()`, `m.match()`
    /// see the method name as an identifier and not as the language keyword.
    after_dot: bool,
}

static KEYWORDS: LazyLock<HashMap<&'static str, Token>> = LazyLock::new(|| {
    let entries: &[(&str, Token)] = &[
        ("is", Token::Is),
        ("neq", Token::Neq),
        ("equals", Token::Equals),
        ("eq", Token::Equals),
        ("unless", Token::Unless),
        ("until", Token::Until),
        ("returns", Token::Returns),
        ("mod", Token::Percent),
        ("lt", Token::Lt),
        ("gt", Token::Gt),
        ("lte", Token::LtEq),
        ("gte", Token::GtEq),
        ("nlt", Token::GtEq),
        ("ngt", Token::LtEq),
        ("ngte", Token::Lt),
        ("nlte", Token::Gt),
        ("and", Token::And),
        ("or", Token::Or),
        ("not", Token::Not),
        ("if", Token::If),
        ("elif", Token::Elif),
        ("else", Token::Else),
        ("while", Token::While),
        ("for", Token::For),
        ("in", Token::In),
        ("loop", Token::Loop),
        ("break", Token::Break),
        ("continue", Token::Continue),
        ("return", Token::Return),
        ("match", Token::Match),
        ("when", Token::When),
        ("type", Token::Type),
        ("enum", Token::Enum),
        ("err", Token::Err),
        ("pub", Token::Pub),
        ("use", Token::Use),
        ("as", Token::As),
        ("at", Token::AtKw),
        ("from", Token::From),
        ("to", Token::To),
        ("by", Token::By),
        ("asm", Token::Asm),
        ("extern", Token::Extern),
        ("do", Token::Do),
        ("end", Token::End),
        ("log", Token::Log),
        ("of", Token::Of),
        ("test", Token::Test),
        ("embed", Token::Embed),
        ("assert", Token::Assert),
        ("query", Token::Query),
        ("store", Token::Store),
        ("migration", Token::Migration),
        ("insert", Token::Insert),
        ("delete", Token::Delete),
        ("set", Token::Set),
        ("transaction", Token::Transaction),
        ("view", Token::View),
        ("actor", Token::Actor),
        ("spawn", Token::Spawn),
        ("send", Token::Send),
        ("receive", Token::Receive),
        ("trait", Token::Trait),
        ("impl", Token::Impl),
        ("dispatch", Token::Dispatch),
        ("yield", Token::Yield),
        ("channel", Token::Channel),
        ("close", Token::Close),
        ("select", Token::Select),
        ("stop", Token::Stop),
        ("default", Token::Default),
        ("sim", Token::Sim),
        ("supervisor", Token::Supervisor),
        ("atomic", Token::Atomic),
        ("strict", Token::Strict),
        ("xor", Token::Xor),
        ("unreachable", Token::Unreachable),
        ("alias", Token::Alias),
        ("defer", Token::Defer),
        ("grad", Token::Grad),
        ("einsum", Token::Einsum),
        ("build", Token::Build),
        ("syscall", Token::Syscall),
        ("global", Token::Global),
        ("nop", Token::Nop),
        ("pow", Token::StarStar),
        ("true", Token::True),
        ("false", Token::False),
        ("none", Token::None),
    ];
    entries.iter().cloned().collect()
});

fn keyword(s: &str) -> Option<Token> {
    KEYWORDS.get(s).cloned()
}

impl<'s> Lexer<'s> {
    pub fn new(src: &'s str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
            indents: vec![0],
            pending: Vec::new(),
            sol: true,
            nl: false,
            file: None,
            after_dot: false,
        }
    }

    /// Tag every span this lexer produces with `file`. Use this when the
    /// source comes from a known on-disk file so diagnostics include the
    /// filename — essential for multi-file projects.
    pub fn with_file(mut self, file: crate::intern::Symbol) -> Self {
        self.file = Some(file);
        self
    }

    pub fn tokenize(&mut self) -> Result<Vec<Spanned>, LexError> {
        let mut out = Vec::new();

        // Skip shebang line (e.g. #!/usr/bin/env jinnc run)
        if self.pos == 0 && self.src.len() >= 2 && self.src[0] == b'#' && self.src[1] == b'!' {
            while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                self.advance();
            }
            if self.pos < self.src.len() {
                // consume the newline
                self.line += 1;
                self.col = 0;
                self.pos += 1;
            }
        }

        loop {
            if !self.pending.is_empty() {
                out.append(&mut self.pending);
            }
            if self.pos >= self.src.len() {
                break;
            }

            if self.sol {
                self.handle_indent(&mut out)?;
                self.sol = false;
                continue;
            }

            let ch = self.src[self.pos];
            match ch {
                b' ' => {
                    self.advance();
                    continue;
                }
                b'#' => {
                    self.skip_line();
                    continue;
                }
                b'\r' => {
                    self.advance();
                    continue;
                }
                b'\t' => return self.err("tabs are not allowed; use spaces"),
                b'\n' => {
                    if !self.nl {
                        out.push(self.spanned(Token::Newline));
                        self.nl = true;
                    }
                    self.advance();
                    self.line += 1;
                    self.col = 1;
                    self.sol = true;
                    continue;
                }
                _ => {}
            }
            self.nl = false;
            let tok = self.lex_token()?;
            // P0-10: after a `Dot`, the next identifier is a member/method
            // name and must NOT be promoted to a language keyword. The
            // promotion is suppressed inside `lex_ident` via `after_dot`.
            self.after_dot = matches!(tok.token, Token::Dot);
            out.push(tok);
        }

        if !self.nl && !out.is_empty() {
            out.push(Spanned {
                token: Token::Newline,
                span: self.here(),
            });
        }
        while self.indents.len() > 1 {
            self.indents.pop();
            out.push(Spanned {
                token: Token::Dedent,
                span: self.here(),
            });
        }
        out.push(Spanned {
            token: Token::Eof,
            span: self.here(),
        });
        if let Some(file) = self.file {
            for sp in &mut out {
                sp.span.file = Some(file);
            }
        }
        Ok(out)
    }

    pub fn lex_all(&mut self) -> Result<Vec<Spanned>, LexError> {
        let mut out = Vec::new();
        while self.pos < self.src.len() {
            let ch = self.src[self.pos];
            if ch == b' ' {
                self.advance();
                continue;
            }
            if ch == b'\n' || ch == b'\r' {
                break;
            }
            out.push(self.lex_token()?);
        }
        if let Some(file) = self.file {
            for sp in &mut out {
                sp.span.file = Some(file);
            }
        }
        Ok(out)
    }

    fn handle_indent(&mut self, out: &mut Vec<Spanned>) -> Result<(), LexError> {
        let mut spaces: u32 = 0;
        while self.pos < self.src.len() && self.src[self.pos] == b' ' {
            spaces += 1;
            self.pos += 1;
        }
        self.col = spaces + 1;

        if self.pos >= self.src.len() || matches!(self.src[self.pos], b'\n' | b'\r' | b'#') {
            return Ok(());
        }

        let cur = *self.indents.last().unwrap();
        let sp = Span::new(self.pos, self.pos, self.line, 1);
        if spaces > cur {
            self.indents.push(spaces);
            out.push(Spanned {
                token: Token::Indent,
                span: sp,
            });
        } else if spaces < cur {
            while *self.indents.last().unwrap() > spaces {
                self.indents.pop();
                out.push(Spanned {
                    token: Token::Dedent,
                    span: sp,
                });
            }
            if *self.indents.last().unwrap() != spaces {
                return Err(LexError::Error {
                    line: self.line,
                    col: 1,
                    msg: format!(
                        "inconsistent indentation: expected {}, got {spaces}",
                        self.indents.last().unwrap()
                    ),
                });
            }
        }
        Ok(())
    }

    fn lex_token(&mut self) -> Result<Spanned, LexError> {
        let (start, sc) = (self.pos, self.col);
        let ch = self.src[self.pos];

        if ch.is_ascii_digit() {
            return self.lex_number();
        }
        if ch == b'\'' {
            return self.lex_string();
        }
        if ch == b'"' {
            return self.lex_raw_string();
        }
        if ch.is_ascii_alphabetic() || ch == b'_' {
            return self.lex_ident();
        }

        // Four-character tokens
        if self.pos + 3 < self.src.len() {
            if let (b'>', b'>', b'>', b'=') = (
                ch,
                self.src[self.pos + 1],
                self.src[self.pos + 2],
                self.src[self.pos + 3],
            ) {
                self.advance();
                self.advance();
                self.advance();
                self.advance();
                return Ok(Spanned {
                    token: Token::UshrEq,
                    span: Span::new(start, self.pos, self.line, sc),
                });
            }
        }

        if self.pos + 2 < self.src.len() {
            let three = match (ch, self.src[self.pos + 1], self.src[self.pos + 2]) {
                (b'<', b'<', b'=') => Some(Token::ShlEq),
                (b'>', b'>', b'=') => Some(Token::ShrEq),
                (b'>', b'>', b'>') => Some(Token::Ushr),
                _ => Option::None,
            };
            if let Some(tok) = three {
                self.advance();
                self.advance();
                self.advance();
                return Ok(Spanned {
                    token: tok,
                    span: Span::new(start, self.pos, self.line, sc),
                });
            }
        }

        if self.pos + 1 < self.src.len() {
            let two = match (ch, self.src[self.pos + 1]) {
                (b'<', b'<') => Some(Token::Shl),
                (b'>', b'>') => Some(Token::Shr),
                (b'<', b'=') => Some(Token::LtEq),
                (b'>', b'=') => Some(Token::GtEq),
                (b'+', b'=') => Some(Token::PlusEq),
                (b'-', b'=') => Some(Token::MinusEq),
                (b'*', b'=') => Some(Token::StarEq),
                (b'/', b'=') => Some(Token::SlashEq),
                (b'&', b'=') => Some(Token::AmpEq),
                (b'|', b'=') => Some(Token::PipeEq),
                (b'^', b'=') => Some(Token::CaretEq),
                _ => Option::None,
            };
            if let Some(tok) = two {
                self.advance();
                self.advance();
                return Ok(Spanned {
                    token: tok,
                    span: Span::new(start, self.pos, self.line, sc),
                });
            }
        }

        let tok = match ch {
            b'+' => Token::Plus,
            b'-' => Token::Minus,
            b'/' => Token::Slash,
            b'%' => Token::Percent,
            b'|' => Token::Pipe,
            b'^' => Token::Caret,
            b'&' => Token::Ampersand,
            b'@' => Token::At,
            b'~' => Token::Tilde,
            b'$' => {
                if self.pos + 1 < self.src.len() && self.src[self.pos + 1] == b'$' {
                    self.advance();
                    Token::DollarDollar
                } else {
                    Token::Dollar
                }
            }
            b'<' => Token::Lt,
            b'>' => Token::Gt,
            b'?' => Token::Question,
            b'!' => {
                if self.pos + 1 < self.src.len() && self.src[self.pos + 1] == b'!' {
                    self.advance();
                    Token::BangBang
                } else {
                    Token::Bang
                }
            }
            b'(' => Token::LParen,
            b')' => Token::RParen,
            b'[' => Token::LBracket,
            b']' => Token::RBracket,
            b',' => Token::Comma,
            b':' => {
                if self.pos + 1 < self.src.len() {
                    let next = self.src[self.pos + 1];
                    if next == b'\\' {
                        // Escape sequence char literal :  \n \t \r \\ \0
                        if self.pos + 2 < self.src.len() {
                            let esc = self.src[self.pos + 2];
                            let val = match esc {
                                b'n' => Some(b'\n' as i64),
                                b't' => Some(b'\t' as i64),
                                b'r' => Some(b'\r' as i64),
                                b'\\' => Some(b'\\' as i64),
                                b'0' => Some(0i64),
                                _ => None,
                            };
                            if let Some(v) = val {
                                self.advance(); // skip :
                                self.advance(); // skip \\
                                self.advance(); // skip escape char
                                return Ok(Spanned {
                                    token: Token::CharLit(v),
                                    span: Span::new(start, self.pos, self.line, sc),
                                });
                            }
                        }
                    } else if next != b' ' && next != b'\n' && next != b'\r' {
                        // Check for single-char literal: char after next must be
                        // whitespace, punctuation, or EOF
                        let after = if self.pos + 2 < self.src.len() {
                            self.src[self.pos + 2]
                        } else {
                            b' ' // treat EOF as whitespace
                        };
                        let is_boundary = after == b' '
                            || after == b'\n'
                            || after == b'\r'
                            || after == b')'
                            || after == b']'
                            || after == b'}'
                            || after == b','
                            || after == b':'
                            || after == b'\t'
                            || after == b'!'
                            || after == b'?'
                            || after == b';'
                            || after == b'.'
                            || self.pos + 2 >= self.src.len();
                        if is_boundary {
                            let val = next as i64;
                            self.advance(); // skip :
                            self.advance(); // skip char
                            return Ok(Spanned {
                                token: Token::CharLit(val),
                                span: Span::new(start, self.pos, self.line, sc),
                            });
                        }
                    }
                }
                Token::Colon
            }
            b'.' => {
                if self.pos + 2 < self.src.len()
                    && self.src[self.pos + 1] == b'.'
                    && self.src[self.pos + 2] == b'.'
                {
                    self.advance();
                    self.advance();
                    Token::DotDotDot
                } else {
                    Token::Dot
                }
            }
            b'*' => Token::Star,
            _ => return self.err(&format!("unexpected character: '{}'", ch as char)),
        };
        self.advance();
        Ok(Spanned {
            token: tok,
            span: Span::new(start, self.pos, self.line, sc),
        })
    }

    fn advance(&mut self) {
        // Only count column for non-continuation UTF-8 bytes
        if self.pos < self.src.len() && (self.src[self.pos] & 0xC0) != 0x80 {
            self.col += 1;
        }
        self.pos += 1;
    }
    fn skip_line(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
            self.advance();
        }
    }
    fn here(&self) -> Span {
        Span::new(self.pos, self.pos, self.line, self.col)
    }
    fn spanned(&self, token: Token) -> Spanned {
        Spanned {
            token,
            span: self.here(),
        }
    }
    fn mkerr(&self, msg: &str) -> LexError {
        LexError::Error {
            line: self.line,
            col: self.col,
            msg: msg.into(),
        }
    }
    fn err<T>(&self, msg: &str) -> Result<T, LexError> {
        Err(self.mkerr(msg))
    }
}

#[cfg(test)]
mod tests;
