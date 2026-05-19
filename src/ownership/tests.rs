#![cfg(test)]

use super::*;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::typer::Typer;

fn parse(src: &str) -> crate::ast::Program {
    let tokens = Lexer::new(src).tokenize().unwrap();
    Parser::new(tokens).parse_program().unwrap()
}

fn verify(src: &str) -> Vec<OwnershipDiag> {
    let prog = parse(src);
    let mut typer = Typer::new();
    let hir = typer.lower_program(&prog).unwrap();
    let mut verifier = OwnershipVerifier::new();
    verifier.verify(&hir)
}

#[test]
fn test_simple_program_no_errors() {
    let diags = verify("*main()\n    x is 42\n    log(x)\n");
    assert!(
        diags.is_empty(),
        "expected no ownership errors, got: {:?}",
        diags
    );
}

#[test]
fn test_rc_binding_no_errors() {
    let diags = verify("type Foo\n    x as i64\n\n*main()\n    f is Foo(x is 42)\n    log(f.x)\n");
    assert!(
        diags.is_empty(),
        "expected no ownership errors, got: {:?}",
        diags
    );
}

#[test]
fn test_function_params_no_errors() {
    let diags =
        verify("*add(a as i64, b as i64) returns i64\n    a + b\n*main()\n    log(add(1, 2))\n");
    assert!(diags.is_empty());
}
