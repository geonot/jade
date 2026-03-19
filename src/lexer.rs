use crate::ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Int(i64),
    Float(f64),
    Str(String),
    True,
    False,
    None,
    Ident(String),
    Is,
    Isnt,
    Equals,
    And,
    Or,
    Not,
    If,
    Elif,
    Else,
    While,
    For,
    In,
    Loop,
    Break,
    Continue,
    Return,
    Match,
    When,
    Type,
    Enum,
    Err,
    Pub,
    Use,
    As,
    From,
    To,
    By,
    Array,
    Asm,
    Unsafe,
    Volatile,
    Signal,
    Weak,
    Extern,
    Fn,
    Do,
    End,
    Log,
    Of,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Pipe,
    Caret,
    Ampersand,
    At,
    Tilde,
    Shl,
    Shr,
    Lt,
    Gt,
    LtEq,
    GtEq,
    Question,
    Bang,
    Arrow,
    Dollar,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Colon,
    StarStar,
    Dot,
    Hash,
    Newline,
    Indent,
    Dedent,
    Eof,
    InterpStart,
    InterpEnd,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int(n) => write!(f, "{n}"),
            Self::Float(n) => write!(f, "{n}"),
            Self::Str(s) => write!(f, "'{s}'"),
            Self::True => f.write_str("true"),
            Self::False => f.write_str("false"),
            Self::None => f.write_str("none"),
            Self::Ident(s) => f.write_str(s),
            Self::Is => f.write_str("is"),
            Self::Isnt => f.write_str("isnt"),
            Self::Equals => f.write_str("equals"),
            Self::And => f.write_str("and"),
            Self::Or => f.write_str("or"),
            Self::Not => f.write_str("not"),
            Self::If => f.write_str("if"),
            Self::Elif => f.write_str("elif"),
            Self::Else => f.write_str("else"),
            Self::While => f.write_str("while"),
            Self::For => f.write_str("for"),
            Self::In => f.write_str("in"),
            Self::Loop => f.write_str("loop"),
            Self::Break => f.write_str("break"),
            Self::Continue => f.write_str("continue"),
            Self::Return => f.write_str("return"),
            Self::Match => f.write_str("match"),
            Self::When => f.write_str("when"),
            Self::Type => f.write_str("type"),
            Self::Enum => f.write_str("enum"),
            Self::Err => f.write_str("err"),
            Self::Pub => f.write_str("pub"),
            Self::Use => f.write_str("use"),
            Self::As => f.write_str("as"),
            Self::From => f.write_str("from"),
            Self::To => f.write_str("to"),
            Self::By => f.write_str("by"),
            Self::Array => f.write_str("array"),
            Self::Asm => f.write_str("asm"),
            Self::Unsafe => f.write_str("unsafe"),
            Self::Volatile => f.write_str("volatile"),
            Self::Signal => f.write_str("signal"),
            Self::Weak => f.write_str("weak"),
            Self::Extern => f.write_str("extern"),
            Self::Fn => f.write_str("fn"),
            Self::Do => f.write_str("do"),
            Self::End => f.write_str("end"),
            Self::Log => f.write_str("log"),
            Self::Of => f.write_str("of"),
            Self::Plus => f.write_str("+"),
            Self::Minus => f.write_str("-"),
            Self::Star => f.write_str("*"),
            Self::Slash => f.write_str("/"),
            Self::Percent => f.write_str("%"),
            Self::Pipe => f.write_str("|"),
            Self::Caret => f.write_str("^"),
            Self::Ampersand => f.write_str("&"),
            Self::At => f.write_str("@"),
            Self::Tilde => f.write_str("~"),
            Self::Dollar => f.write_str("$"),
            Self::Shl => f.write_str("<<"),
            Self::Shr => f.write_str(">>"),
            Self::Lt => f.write_str("<"),
            Self::Gt => f.write_str(">"),
            Self::LtEq => f.write_str("<="),
            Self::GtEq => f.write_str(">="),
            Self::Question => f.write_str("?"),
            Self::Bang => f.write_str("!"),
            Self::StarStar => f.write_str("**"),
            Self::Arrow => f.write_str("->"),
            Self::LParen => f.write_str("("),
            Self::RParen => f.write_str(")"),
            Self::LBracket => f.write_str("["),
            Self::RBracket => f.write_str("]"),
            Self::Comma => f.write_str(","),
            Self::Colon => f.write_str(":"),
            Self::Dot => f.write_str("."),
            Self::Hash => f.write_str("#"),
            Self::Newline => f.write_str("NEWLINE"),
            Self::Indent => f.write_str("INDENT"),
            Self::Dedent => f.write_str("DEDENT"),
            Self::Eof => f.write_str("EOF"),
            Self::InterpStart => f.write_str("INTERP_START"),
            Self::InterpEnd => f.write_str("INTERP_END"),
            Self::PlusEq => f.write_str("+="),
            Self::MinusEq => f.write_str("-="),
            Self::StarEq => f.write_str("*="),
            Self::SlashEq => f.write_str("/="),
            Self::PercentEq => f.write_str("%="),
            Self::AmpEq => f.write_str("&="),
            Self::PipeEq => f.write_str("|="),
            Self::CaretEq => f.write_str("^="),
            Self::ShlEq => f.write_str("<<="),
            Self::ShrEq => f.write_str(">>="),
        }
    }
}

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
}

fn keyword(s: &str) -> Option<Token> {
    Some(match s {
        "is" => Token::Is,
        "isnt" => Token::Isnt,
        "equals" => Token::Equals,
        "and" => Token::And,
        "or" => Token::Or,
        "not" => Token::Not,
        "if" => Token::If,
        "elif" => Token::Elif,
        "else" => Token::Else,
        "while" => Token::While,
        "for" => Token::For,
        "in" => Token::In,
        "loop" => Token::Loop,
        "break" => Token::Break,
        "continue" => Token::Continue,
        "return" => Token::Return,
        "match" => Token::Match,
        "when" => Token::When,
        "type" => Token::Type,
        "enum" => Token::Enum,
        "err" => Token::Err,
        "pub" => Token::Pub,
        "use" => Token::Use,
        "as" => Token::As,
        "from" => Token::From,
        "to" => Token::To,
        "by" => Token::By,
        "array" => Token::Array,
        "asm" => Token::Asm,
        "unsafe" => Token::Unsafe,
        "volatile" => Token::Volatile,
        "signal" => Token::Signal,
        "weak" => Token::Weak,
        "extern" => Token::Extern,
        "fn" => Token::Fn,
        "do" => Token::Do,
        "end" => Token::End,
        "log" => Token::Log,
        "of" => Token::Of,
        "true" => Token::True,
        "false" => Token::False,
        "none" => Token::None,
        _ => return Option::None,
    })
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
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Spanned>, LexError> {
        let mut out = Vec::new();
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
            out.push(self.lex_token()?);
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
        Ok(out)
    }

    /// Tokenize a fragment (expression inside interpolation) — no indent/dedent handling.
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

        if self.pos + 2 < self.src.len() {
            let three = match (ch, self.src[self.pos + 1], self.src[self.pos + 2]) {
                (b'<', b'<', b'=') => Some(Token::ShlEq),
                (b'>', b'>', b'=') => Some(Token::ShrEq),
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
                (b'*', b'*') => Some(Token::StarStar),
                (b'<', b'<') => Some(Token::Shl),
                (b'>', b'>') => Some(Token::Shr),
                (b'<', b'=') => Some(Token::LtEq),
                (b'>', b'=') => Some(Token::GtEq),
                (b'-', b'>') => Some(Token::Arrow),
                (b'+', b'=') => Some(Token::PlusEq),
                (b'-', b'=') => Some(Token::MinusEq),
                (b'*', b'=') => Some(Token::StarEq),
                (b'/', b'=') => Some(Token::SlashEq),
                (b'%', b'=') => Some(Token::PercentEq),
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
            b'$' => Token::Dollar,
            b'<' => Token::Lt,
            b'>' => Token::Gt,
            b'?' => Token::Question,
            b'!' => Token::Bang,
            b'(' => Token::LParen,
            b')' => Token::RParen,
            b'[' => Token::LBracket,
            b']' => Token::RBracket,
            b',' => Token::Comma,
            b':' => Token::Colon,
            b'.' => Token::Dot,
            b'*' => Token::Star,
            _ => return self.err(&format!("unexpected character: '{}'", ch as char)),
        };
        self.advance();
        Ok(Spanned {
            token: tok,
            span: Span::new(start, self.pos, self.line, sc),
        })
    }

    fn lex_number(&mut self) -> Result<Spanned, LexError> {
        let (start, sc) = (self.pos, self.col);

        if self.src[self.pos] == b'0' && self.pos + 1 < self.src.len() {
            match self.src[self.pos + 1] {
                b'x' | b'X' => return self.lex_based(start, sc, 16, |b| b.is_ascii_hexdigit()),
                b'b' | b'B' => return self.lex_based(start, sc, 2, |b| b == b'0' || b == b'1'),
                b'o' | b'O' => return self.lex_based(start, sc, 8, |b| (b'0'..=b'7').contains(&b)),
                _ => {}
            }
        }

        while self.pos < self.src.len()
            && (self.src[self.pos].is_ascii_digit() || self.src[self.pos] == b'_')
        {
            self.advance();
        }

        let mut float = false;
        if self.pos < self.src.len()
            && self.src[self.pos] == b'.'
            && self.pos + 1 < self.src.len()
            && self.src[self.pos + 1].is_ascii_digit()
        {
            float = true;
            self.advance();
            while self.pos < self.src.len()
                && (self.src[self.pos].is_ascii_digit() || self.src[self.pos] == b'_')
            {
                self.advance();
            }
        }

        if self.pos < self.src.len() && matches!(self.src[self.pos], b'e' | b'E') {
            float = true;
            self.advance();
            if self.pos < self.src.len() && matches!(self.src[self.pos], b'+' | b'-') {
                self.advance();
            }
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                self.advance();
            }
        }

        let text: String = self.src[start..self.pos]
            .iter()
            .filter(|&&b| b != b'_')
            .map(|&b| b as char)
            .collect();
        let sp = Span::new(start, self.pos, self.line, sc);
        if float {
            Ok(Spanned {
                token: Token::Float(
                    text.parse()
                        .map_err(|_| self.mkerr(&format!("bad float: {text}")))?,
                ),
                span: sp,
            })
        } else {
            Ok(Spanned {
                token: Token::Int(
                    text.parse()
                        .map_err(|_| self.mkerr(&format!("bad int: {text}")))?,
                ),
                span: sp,
            })
        }
    }

    fn lex_based(
        &mut self,
        start: usize,
        sc: u32,
        radix: u32,
        valid: impl Fn(u8) -> bool,
    ) -> Result<Spanned, LexError> {
        self.advance();
        self.advance();
        let ns = self.pos;
        while self.pos < self.src.len() && (valid(self.src[self.pos]) || self.src[self.pos] == b'_')
        {
            self.advance();
        }
        if self.pos == ns {
            let prefix = match radix {
                16 => 'x',
                2 => 'b',
                _ => 'o',
            };
            return self.err(&format!("expected digits after 0{prefix}"));
        }
        let text: String = self.src[ns..self.pos]
            .iter()
            .filter(|&&b| b != b'_')
            .map(|&b| b as char)
            .collect();
        let val = i64::from_str_radix(&text, radix)
            .map_err(|_| self.mkerr(&format!("bad base-{radix} literal")))?;
        Ok(Spanned {
            token: Token::Int(val),
            span: Span::new(start, self.pos, self.line, sc),
        })
    }

    fn lex_string(&mut self) -> Result<Spanned, LexError> {
        let (start, sc) = (self.pos, self.col);
        self.advance();
        let mut val = String::new();
        let mut has_interp = false;
        while self.pos < self.src.len() && self.src[self.pos] != b'\'' {
            if self.src[self.pos] == b'\n' {
                return self.err("unterminated string");
            }
            if self.src[self.pos] == b'{' {
                has_interp = true;
                // Push the string part accumulated so far
                let sp = Span::new(start, self.pos, self.line, sc);
                if !val.is_empty() || self.pending.is_empty() || !has_interp {
                    self.pending.push(Spanned {
                        token: Token::Str(std::mem::take(&mut val)),
                        span: sp,
                    });
                } else if val.is_empty() {
                    self.pending.push(Spanned {
                        token: Token::Str(String::new()),
                        span: sp,
                    });
                }
                self.pending.push(Spanned {
                    token: Token::InterpStart,
                    span: sp,
                });
                self.advance(); // skip '{'
                // Tokenize the expression inside {}
                let mut depth = 1u32;
                let expr_start = self.pos;
                while self.pos < self.src.len() && depth > 0 {
                    match self.src[self.pos] {
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        b'\n' => return self.err("unterminated interpolation"),
                        _ => {}
                    }
                    if depth > 0 {
                        self.advance();
                    }
                }
                if depth > 0 {
                    return self.err("unterminated interpolation");
                }
                // Lex the inner expression
                let inner_src = &self.src[expr_start..self.pos];
                let inner_str = std::str::from_utf8(inner_src).unwrap();
                let mut inner_lexer = Lexer::new(inner_str);
                let inner_tokens = inner_lexer.lex_all()?;
                for t in inner_tokens {
                    if !matches!(t.token, Token::Newline | Token::Eof) {
                        self.pending.push(t);
                    }
                }
                let isp = Span::new(self.pos, self.pos + 1, self.line, self.col);
                self.pending.push(Spanned {
                    token: Token::InterpEnd,
                    span: isp,
                });
                self.advance(); // skip '}'
                continue;
            }
            if self.src[self.pos] == b'\\' {
                self.advance();
                if self.pos >= self.src.len() {
                    return self.err("unterminated escape");
                }
                match self.src[self.pos] {
                    b'n' => val.push('\n'),
                    b't' => val.push('\t'),
                    b'r' => val.push('\r'),
                    b'\\' => val.push('\\'),
                    b'\'' => val.push('\''),
                    b'0' => val.push('\0'),
                    b'{' => val.push('{'),
                    b'}' => val.push('}'),
                    o => return self.err(&format!("unknown escape: \\{}", o as char)),
                }
            } else {
                val.push(self.src[self.pos] as char);
            }
            self.advance();
        }
        if self.pos >= self.src.len() {
            return self.err("unterminated string");
        }
        self.advance(); // skip closing '
        if has_interp {
            let sp = Span::new(start, self.pos, self.line, sc);
            self.pending.push(Spanned {
                token: Token::Str(val),
                span: sp,
            });
            // Return the first pending token
            return Ok(self.pending.remove(0));
        }
        Ok(Spanned {
            token: Token::Str(val),
            span: Span::new(start, self.pos, self.line, sc),
        })
    }

    fn lex_raw_string(&mut self) -> Result<Spanned, LexError> {
        let (start, sc) = (self.pos, self.col);
        self.advance();
        let mut val = String::new();
        while self.pos < self.src.len() && self.src[self.pos] != b'"' {
            if self.src[self.pos] == b'\n' {
                return self.err("unterminated raw string");
            }
            val.push(self.src[self.pos] as char);
            self.advance();
        }
        if self.pos >= self.src.len() {
            return self.err("unterminated raw string");
        }
        self.advance();
        Ok(Spanned {
            token: Token::Str(val),
            span: Span::new(start, self.pos, self.line, sc),
        })
    }

    fn lex_ident(&mut self) -> Result<Spanned, LexError> {
        let (start, sc) = (self.pos, self.col);
        while self.pos < self.src.len()
            && (self.src[self.pos].is_ascii_alphanumeric() || self.src[self.pos] == b'_')
        {
            self.advance();
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        let tok = keyword(text).unwrap_or_else(|| Token::Ident(text.to_string()));
        Ok(Spanned {
            token: tok,
            span: Span::new(start, self.pos, self.line, sc),
        })
    }

    fn advance(&mut self) {
        self.pos += 1;
        self.col += 1;
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
mod tests {
    use super::*;
    fn lex(s: &str) -> Vec<Token> {
        Lexer::new(s)
            .tokenize()
            .unwrap()
            .into_iter()
            .map(|t| t.token)
            .collect()
    }

    #[test]
    fn hello() {
        let t = lex("*main()\n    log('hello')\n");
        assert_eq!(
            &t[..13],
            &[
                Token::Star,
                Token::Ident("main".into()),
                Token::LParen,
                Token::RParen,
                Token::Newline,
                Token::Indent,
                Token::Log,
                Token::LParen,
                Token::Str("hello".into()),
                Token::RParen,
                Token::Newline,
                Token::Dedent,
                Token::Eof
            ]
        );
    }

    #[test]
    fn bases() {
        let t = lex("0xFF 0b1010 0o77 42 1_000");
        assert_eq!(t[0], Token::Int(255));
        assert_eq!(t[1], Token::Int(10));
        assert_eq!(t[2], Token::Int(63));
        assert_eq!(t[3], Token::Int(42));
        assert_eq!(t[4], Token::Int(1000));
    }

    #[test]
    fn floats() {
        let t = lex("3.14 1e-3 2.5e10");
        assert_eq!(t[0], Token::Float(3.14));
        assert_eq!(t[1], Token::Float(1e-3));
        assert_eq!(t[2], Token::Float(2.5e10));
    }

    #[test]
    fn kw() {
        let t = lex("is isnt equals and or not");
        assert_eq!(
            &t[..6],
            &[
                Token::Is,
                Token::Isnt,
                Token::Equals,
                Token::And,
                Token::Or,
                Token::Not
            ]
        );
    }

    #[test]
    fn indent() {
        let t = lex("if true\n    x is 1\n    y is 2\n");
        assert_eq!(t[0], Token::If);
        assert_eq!(t[2], Token::Newline);
        assert_eq!(t[3], Token::Indent);
        assert_eq!(t[12], Token::Dedent);
    }

    #[test]
    fn bind() {
        let t = lex("x is 42");
        assert_eq!(
            &t[..3],
            &[Token::Ident("x".into()), Token::Is, Token::Int(42)]
        );
    }

    #[test]
    fn two_char_ops() {
        let t = lex("<< >> <= >= -> **");
        assert_eq!(t[0], Token::Shl);
        assert_eq!(t[1], Token::Shr);
        assert_eq!(t[2], Token::LtEq);
        assert_eq!(t[3], Token::GtEq);
        assert_eq!(t[4], Token::Arrow);
        assert_eq!(t[5], Token::StarStar);
    }

    #[test]
    fn single_ops() {
        let t = lex("+ - * / % | ^ & ~ < > ? ! ( ) [ ] , : .");
        assert_eq!(t[0], Token::Plus);
        assert_eq!(t[1], Token::Minus);
        assert_eq!(t[2], Token::Star);
        assert_eq!(t[3], Token::Slash);
        assert_eq!(t[4], Token::Percent);
        assert_eq!(t[5], Token::Pipe);
        assert_eq!(t[6], Token::Caret);
        assert_eq!(t[7], Token::Ampersand);
        assert_eq!(t[8], Token::Tilde);
        assert_eq!(t[9], Token::Lt);
        assert_eq!(t[10], Token::Gt);
        assert_eq!(t[11], Token::Question);
        assert_eq!(t[12], Token::Bang);
        assert_eq!(t[13], Token::LParen);
        assert_eq!(t[14], Token::RParen);
        assert_eq!(t[15], Token::LBracket);
        assert_eq!(t[16], Token::RBracket);
        assert_eq!(t[17], Token::Comma);
        assert_eq!(t[18], Token::Colon);
        assert_eq!(t[19], Token::Dot);
    }

    #[test]
    fn strings() {
        let t = lex("'hello' 'world'");
        assert_eq!(t[0], Token::Str("hello".into()));
        assert_eq!(t[1], Token::Str("world".into()));
    }

    #[test]
    fn string_escapes() {
        let t = lex(r"'hello\nworld'");
        assert_eq!(t[0], Token::Str("hello\nworld".into()));
        let t = lex(r"'tab\there'");
        assert_eq!(t[0], Token::Str("tab\there".into()));
        let t = lex(r"'null\0end'");
        assert_eq!(t[0], Token::Str("null\0end".into()));
    }

    #[test]
    fn raw_string() {
        let t = lex(r#""no \n escapes""#);
        assert_eq!(t[0], Token::Str(r"no \n escapes".into()));
    }

    #[test]
    fn all_keywords() {
        let t = lex(
            "if elif else while for in loop break continue return match when type enum err pub use as from to by array unsafe extern fn do end log",
        );
        let expected = [
            Token::If,
            Token::Elif,
            Token::Else,
            Token::While,
            Token::For,
            Token::In,
            Token::Loop,
            Token::Break,
            Token::Continue,
            Token::Return,
            Token::Match,
            Token::When,
            Token::Type,
            Token::Enum,
            Token::Err,
            Token::Pub,
            Token::Use,
            Token::As,
            Token::From,
            Token::To,
            Token::By,
            Token::Array,
            Token::Unsafe,
            Token::Extern,
            Token::Fn,
            Token::Do,
            Token::End,
            Token::Log,
        ];
        for (i, e) in expected.iter().enumerate() {
            assert_eq!(&t[i], e, "keyword mismatch at {i}");
        }
    }

    #[test]
    fn comments() {
        let t = lex("x is 1 # this is a comment\ny is 2");
        assert_eq!(t[0], Token::Ident("x".into()));
        assert_eq!(t[2], Token::Int(1));
        assert_eq!(t[4], Token::Ident("y".into()));
    }

    #[test]
    fn nested_indent() {
        let t = lex("if true\n    if false\n        x is 1\n");
        let mut indents = 0;
        let mut dedents = 0;
        for tok in &t {
            match tok {
                Token::Indent => indents += 1,
                Token::Dedent => dedents += 1,
                _ => {}
            }
        }
        assert_eq!(indents, 2);
        assert_eq!(dedents, 2);
    }

    #[test]
    fn scientific_notation() {
        let t = lex("1e3 2.5E-10 3e+2");
        assert_eq!(t[0], Token::Float(1e3));
        assert_eq!(t[1], Token::Float(2.5e-10));
        assert_eq!(t[2], Token::Float(3e2));
    }

    #[test]
    fn booleans() {
        let t = lex("true false none");
        assert_eq!(t[0], Token::True);
        assert_eq!(t[1], Token::False);
        assert_eq!(t[2], Token::None);
    }

    #[test]
    fn tab_error() {
        let r = Lexer::new("*main()\n\tlog(1)\n").tokenize();
        assert!(r.is_err());
    }

    #[test]
    fn unterminated_string() {
        let r = Lexer::new("'hello\n").tokenize();
        assert!(r.is_err());
    }
}
