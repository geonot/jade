#![cfg(test)]

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
            Token::Ident(Symbol::intern("main")),
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
    let t = lex("is neq equals and or not");
    assert_eq!(
        &t[..6],
        &[
            Token::Is,
            Token::Neq,
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
        &[Token::Ident(Symbol::intern("x")), Token::Is, Token::Int(42)]
    );
}

#[test]
fn two_char_ops() {
    let t = lex("<< >> <= >=");
    assert_eq!(t[0], Token::Shl);
    assert_eq!(t[1], Token::Shr);
    assert_eq!(t[2], Token::LtEq);
    assert_eq!(t[3], Token::GtEq);
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
        "if elif else while for in loop break continue return match when type enum err pub use as from to by array extern do end log",
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
        Token::Extern,
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
    assert_eq!(t[0], Token::Ident(Symbol::intern("x")));
    assert_eq!(t[2], Token::Int(1));
    assert_eq!(t[4], Token::Ident(Symbol::intern("y")));
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
