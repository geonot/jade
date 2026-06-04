//! Conformance for the canonical Option/Result prelude (docs/error-effects.md
//! §2). Pins the combinator method surfaces shared by typer and codegen:
//! Option{is_some,is_none,unwrap,unwrap_or,map,and_then,ok_or},
//! Result{is_ok,is_err,unwrap,unwrap_or,map,map_err,and_then,ok,err}.

use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile_and_run(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    let out = dir.path().join("test_bin");
    std::fs::write(&jinn, src).unwrap();
    let status = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jinnc failed to start");
    assert!(status.success(), "jinnc compilation failed for:\n{src}");
    let output = Command::new(&out)
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        "binary exited with {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn expect(src: &str, expected: &str) {
    assert_eq!(compile_and_run(src).trim(), expected.trim(), "source:\n{src}");
}

#[test]
fn option_predicates_and_unwrap() {
    expect(
        "*main()\n    a is Some(5)\n    log(a.is_some())\n    log(a.is_none())\n    log(a.unwrap())\n    log(a.unwrap_or(99))\n    n is Nothing\n    log(n.unwrap_or(42))\n",
        "1\n0\n5\n5\n42",
    );
}

#[test]
fn option_map_and_then_ok_or() {
    expect(
        "*main()\n    a is Some(5)\n    log(a.map(|x| x * 2).unwrap())\n    log(a.and_then(|x| Some(x * 10)).unwrap())\n    log(a.ok_or(7).unwrap())\n",
        "10\n50\n5",
    );
}

#[test]
fn option_map_short_circuits_on_nothing() {
    expect(
        "*parse(x as i64) returns Option of i64\n    if x > 0\n        Some(x)\n    else\n        Nothing\n\n*main()\n    log(parse(5).map(|x| x + 1).unwrap_or(0))\n    log(parse(-1).map(|x| x + 1).unwrap_or(-99))\n",
        "6\n-99",
    );
}

#[test]
fn result_predicates_and_unwrap() {
    expect(
        "*ok(x as i64) returns Result of i64, i64\n    Ok(x)\n\n*er(x as i64) returns Result of i64, i64\n    Err(x)\n\n*main()\n    a is ok(5)\n    log(a.is_ok())\n    log(a.is_err())\n    log(a.unwrap())\n    log(a.unwrap_or(99))\n    e is er(8)\n    log(e.is_err())\n    log(e.unwrap_or(77))\n",
        "1\n0\n5\n5\n1\n77",
    );
}

#[test]
fn result_map_map_err_and_then() {
    expect(
        "*ok(x as i64) returns Result of i64, i64\n    Ok(x)\n\n*main()\n    a is ok(5)\n    log(a.map(|x| x * 2).unwrap())\n    log(a.map_err(|e| e + 1).unwrap())\n    log(a.and_then(|x| ok(x * 10)).unwrap())\n",
        "10\n5\n50",
    );
}

#[test]
fn result_ok_and_err_projections() {
    expect(
        "*ok(x as i64) returns Result of i64, i64\n    Ok(x)\n\n*er(x as i64) returns Result of i64, i64\n    Err(x)\n\n*main()\n    log(ok(5).ok().unwrap())\n    log(er(8).err().unwrap())\n    log(ok(5).err().is_none())\n    log(er(8).ok().is_none())\n",
        "5\n8\n1\n1",
    );
}
