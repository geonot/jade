#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };
    let mut lx = jinnc::lexer::Lexer::new(s);
    let Ok(toks) = lx.tokenize() else { return };
    let mut p = jinnc::parser::Parser::new(toks);
    let _ = p.parse_program();
});
