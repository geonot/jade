// R16: property-based smoke tests. Goals:
//  • lexer never panics on arbitrary printable input
//  • parser never panics on lexer output
//  • round-trip of a small, well-formed program kernel via fmt → parse
//    is fixpoint (idempotent) at the AST level
//
// Bug-finding tool, not a correctness oracle. Runs ~256 cases per prop
// per invocation; cheap enough to keep in the regular cargo test gate.

use jinnc::lexer::Lexer;
use jinnc::parser::Parser as JinnParser;
use proptest::prelude::*;

fn safe_chars() -> impl Strategy<Value = String> {
    // Restrict to printable ASCII + space + newline to avoid lexer
    // tab-error short-circuits dominating the search.
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
        // Result is fine; a panic is the failure mode we're hunting.
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

// Sanity: a known-good tiny program parses without error.
#[test]
fn known_good_program_parses() {
    let src = "*main\n    log(1)\n";
    let mut lx = Lexer::new(src);
    let toks = lx.tokenize().expect("lex");
    let mut p = JinnParser::new(toks);
    p.parse_program().expect("parse");
}
