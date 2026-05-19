//! Numeric, string, raw string, and identifier lexing helpers.

use super::*;

impl<'s> Lexer<'s> {
    pub(in crate::lexer) fn lex_number(&mut self) -> Result<Spanned, LexError> {
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

    pub(in crate::lexer) fn lex_based(
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
        // Use u64::from_str_radix so values that bit-fit in i64 but exceed
        // i64::MAX (e.g. 0x180ec6d33cfd0aba) lex successfully and are
        // reinterpreted as the equivalent signed bit pattern.
        let val = u64::from_str_radix(&text, radix)
            .map(|u| u as i64)
            .map_err(|_| self.mkerr(&format!("bad base-{radix} literal")))?;
        Ok(Spanned {
            token: Token::Int(val),
            span: Span::new(start, self.pos, self.line, sc),
        })
    }

    pub(in crate::lexer) fn lex_string(&mut self) -> Result<Spanned, LexError> {
        let (start, sc) = (self.pos, self.col);
        self.advance(); // consume first '
        // Check for triple-quoted string '''...'''
        if self.pos + 1 < self.src.len()
            && self.src[self.pos] == b'\''
            && self.src[self.pos + 1] == b'\''
        {
            self.advance(); // consume second '
            self.advance(); // consume third '
            // Skip optional leading newline
            if self.pos < self.src.len() && self.src[self.pos] == b'\n' {
                self.line += 1;
                self.col = 0;
                self.pos += 1;
            }
            let mut val = String::new();
            while self.pos < self.src.len() {
                if self.src[self.pos] == b'\''
                    && self.pos + 2 < self.src.len()
                    && self.src[self.pos + 1] == b'\''
                    && self.src[self.pos + 2] == b'\''
                {
                    self.advance(); // consume first '
                    self.advance(); // consume second '
                    self.advance(); // consume third '
                    return Ok(Spanned {
                        token: Token::Str(val),
                        span: Span::new(start, self.pos, self.line, sc),
                    });
                }
                if self.src[self.pos] == b'\n' {
                    val.push('\n');
                    self.line += 1;
                    self.col = 0;
                    self.pos += 1;
                } else {
                    val.push(self.src[self.pos] as char);
                    self.advance();
                }
            }
            return self.err("unterminated triple-quoted string");
        }
        let mut val = String::new();
        let mut has_interp = false;
        while self.pos < self.src.len() && self.src[self.pos] != b'\'' {
            if self.src[self.pos] == b'\n' {
                return self.err("unterminated string");
            }
            if self.src[self.pos] == b'{' {
                // Heuristic: a `{` immediately followed by `"`, `'`, `{`, `}`,
                // a digit-other-than-an-identifier, or end-of-string is treated
                // as a literal brace (e.g. embedding JSON in a string). This
                // avoids accidentally entering interpolation for `'{"x":1}'`.
                let next = self.src.get(self.pos + 1).copied();
                let starts_interp = match next {
                    None => false,
                    Some(c) => {
                        c.is_ascii_alphabetic()
                            || c == b'_'
                            || c == b'('
                            || c == b'['
                            || c == b'$'
                            || c.is_ascii_digit()
                    }
                };
                if !starts_interp {
                    val.push('{');
                    self.advance();
                    continue;
                }
                has_interp = true;
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
                // Inline lex: lex tokens at current position tracking brace depth
                let mut depth = 1u32;
                while self.pos < self.src.len() && depth > 0 {
                    let ch = self.src[self.pos];
                    if ch == b'}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    if ch == b'\n' {
                        return self.err("unterminated interpolation");
                    }
                    if ch == b' ' {
                        self.advance();
                        continue;
                    }
                    let tok = self.lex_token()?;
                    if ch == b'{' {
                        depth += 1;
                    }
                    if !matches!(tok.token, Token::Newline | Token::Eof) {
                        self.pending.push(tok);
                    }
                }
                if depth > 0 {
                    return self.err("unterminated interpolation");
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
                    b'"' => val.push('"'),
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
        self.advance();
        if has_interp {
            let sp = Span::new(start, self.pos, self.line, sc);
            self.pending.push(Spanned {
                token: Token::Str(val),
                span: sp,
            });
            return Ok(self.pending.remove(0));
        }
        Ok(Spanned {
            token: Token::Str(val),
            span: Span::new(start, self.pos, self.line, sc),
        })
    }

    pub(in crate::lexer) fn lex_raw_string(&mut self) -> Result<Spanned, LexError> {
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

    pub(in crate::lexer) fn lex_ident(&mut self) -> Result<Spanned, LexError> {
        let (start, sc) = (self.pos, self.col);
        while self.pos < self.src.len()
            && (self.src[self.pos].is_ascii_alphanumeric() || self.src[self.pos] == b'_')
        {
            self.advance();
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        // P0-10: when the immediately preceding token was `.`, this
        // identifier is a member/method name. Don't promote it to a
        // language keyword \u2014 otherwise `ch.send(x)`, `xs.take()`, `obj.match`
        // etc. would tokenize a keyword in identifier position and the
        // parser would reject them.
        let tok = if self.after_dot {
            Token::Ident(Symbol::intern(text))
        } else {
            keyword(text).unwrap_or_else(|| Token::Ident(Symbol::intern(text)))
        };
        Ok(Spanned {
            token: tok,
            span: Span::new(start, self.pos, self.line, sc),
        })
    }
}
