//! Conformance for the declarative store decorator table (task 2-31-12).
//! One table (in src/store_decorators.rs) drives parser, typer validation, and
//! docs. These tests pin the documented rejection diagnostics for nonsensical
//! decorator combinations and type-constrained field decorators.

use std::path::{Path, PathBuf};
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile_in(dir: &Path, src: &str) -> Result<PathBuf, String> {
    let jinn = dir.join("test.jn");
    let out = dir.join("test_bin");
    std::fs::write(&jinn, src).unwrap();
    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("jinnc failed to start");
    if output.status.success() {
        Ok(out)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn expect_compile_error(src: &str, needle: &str) {
    let dir = tempfile::tempdir().unwrap();
    match compile_in(dir.path(), src) {
        Ok(_) => panic!("expected compile error, but compilation succeeded\nsource:\n{src}"),
        Err(stderr) => assert!(
            stderr.contains(needle),
            "expected stderr to contain {needle:?}, got: {stderr}\nsource:\n{src}"
        ),
    }
}

fn expect_ok(src: &str) {
    let dir = tempfile::tempdir().unwrap();
    if let Err(e) = compile_in(dir.path(), src) {
        panic!("expected compilation to succeed, got: {e}\nsource:\n{src}");
    }
}

#[test]
fn rejects_kv_plus_vector() {
    expect_compile_error(
        "store t @kv @vector(8)\n    name as String\n\n*main\n    log 1\n",
        "@kv and @vector cannot be combined",
    );
}

#[test]
fn rejects_mem_plus_versioned() {
    expect_compile_error(
        "store t @mem @versioned\n    name as String\n\n*main\n    log 1\n",
        "@mem and @versioned cannot be combined",
    );
}

#[test]
fn rejects_unique_on_f64() {
    expect_compile_error(
        "store t\n    score as f64 @unique\n\n*main\n    log 1\n",
        "@unique on field 'score' is not allowed for `f64`",
    );
}

#[test]
fn rejects_increment_on_string() {
    expect_compile_error(
        "store t\n    id as String @increment\n\n*main\n    log 1\n",
        "@increment on field 'id' requires a numeric type",
    );
}

#[test]
fn rejects_unknown_store_decorator() {
    expect_compile_error(
        "store t @bogus\n    name as String\n\n*main\n    log 1\n",
        "unknown store decorator: @bogus",
    );
}

#[test]
fn rejects_unknown_field_decorator() {
    expect_compile_error(
        "store t\n    name as String @bogus\n\n*main\n    log 1\n",
        "unknown field decorator: @bogus",
    );
}

#[test]
fn accepts_sensible_combinations() {
    expect_ok(
        "store users\n    name as String @unique\n    age as i64 @index\n\n*main\n    log 1\n",
    );
}
