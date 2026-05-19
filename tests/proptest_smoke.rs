use jinnc::lexer::Lexer;
use jinnc::parser::Parser as JinnParser;
use proptest::prelude::*;

fn safe_chars() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            Just(b' '),
            Just(b'\n'),
            (b'a'..=b'z'),
            (b'A'..=b'Z'),
            (b'0'..=b'9'),
            prop_oneof![
                Just(b'+'),
                Just(b'-'),
                Just(b'*'),
                Just(b'/'),
                Just(b'('),
                Just(b')'),
                Just(b','),
                Just(b'.'),
                Just(b':'),
                Just(b'<'),
                Just(b'>'),
                Just(b'='),
                Just(b'_'),
                Just(b'!'),
            ],
        ],
        0..200usize,
    )
    .prop_map(|v| String::from_utf8(v).unwrap())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn lexer_never_panics(s in safe_chars()) {

        let mut lx = Lexer::new(&s);
        let _ = lx.tokenize();
    }

    #[test]
    fn parser_never_panics_on_lexer_output(s in safe_chars()) {
        let mut lx = Lexer::new(&s);
        if let Ok(tokens) = lx.tokenize() {
            let mut p = JinnParser::new(tokens);
            let _ = p.parse_program();
        }
    }
}

#[test]
fn known_good_program_parses() {
    let src = "*main\n    log(1)\n";
    let mut lx = Lexer::new(src);
    let toks = lx.tokenize().expect("lex");
    let mut p = JinnParser::new(toks);
    p.parse_program().expect("parse");
}
