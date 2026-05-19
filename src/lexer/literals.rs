use super::*;

/// Decode the UTF-8 codepoint starting at `src[*pos]` and append it
/// (preserving the original bytes) to `val`, then advance `*pos` past
/// the entire codepoint and bump `*col` once. Continuation bytes do
/// not increment the column count, matching `Lexer::advance`.
fn push_utf8_at(val: &mut String, src: &[u8], pos: &mut usize, col: &mut u32) {
    let b = src[*pos];
    let n = if b < 0x80 {
        1
    } else if b < 0xC0 {
        1 // stray continuation byte; treat as one byte
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    };
    let end = (*pos + n).min(src.len());
    match std::str::from_utf8(&src[*pos..end]) {
        Ok(s) => val.push_str(s),
        Err(_) => val.push('\u{FFFD}'),
    }
    *pos = end;
    *col += 1;
}

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
        self.advance();

        if self.pos + 1 < self.src.len()
            && self.src[self.pos] == b'\''
            && self.src[self.pos + 1] == b'\''
        {
            self.advance();
            self.advance();

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
                    self.advance();
                    self.advance();
                    self.advance();
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
                    push_utf8_at(&mut val, self.src, &mut self.pos, &mut self.col);
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
                self.advance();

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
                self.advance();
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
                self.advance();
            } else {
                push_utf8_at(&mut val, self.src, &mut self.pos, &mut self.col);
            }
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
