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

fn compile_fails(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    let out = dir.path().join("test_bin");
    std::fs::write(&jinn, src).unwrap();
    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("jinnc failed to start");
    assert!(
        !output.status.success(),
        "expected compilation to fail, but it succeeded:\n{src}"
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const FILE_ERR: &str = "err FileError\n    NotFound\n    Denied\n\n";

#[test]
#[ignore = "task 2-4-7: migrate `?>` corpus to quaternary `!! err` / implicit propagation"]
fn propagate_happy_path_passes_value_through() {
    expect(
        &format!(
            "{FILE_ERR}*read(ok as bool) returns Result of i64, FileError\n    if ok\n        Ok(42)\n    else\n        Err(NotFound)\n\n*load(ok as bool) returns Result of i64, FileError\n    raw is read(ok)?>\n    Ok(raw + 1)\n\n*main()\n    match load(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n"
        ),
        "43",
    );
}

#[test]
#[ignore = "task 2-4-7: migrate `?>` corpus to quaternary `!! err` / implicit propagation"]
fn propagate_early_exits_on_err() {
    expect(
        &format!(
            "{FILE_ERR}*read(ok as bool) returns Result of i64, FileError\n    if ok\n        Ok(42)\n    else\n        Err(NotFound)\n\n*load(ok as bool) returns Result of i64, FileError\n    raw is read(ok)?>\n    Ok(raw + 1)\n\n*main()\n    match load(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n"
        ),
        "-1",
    );
}

#[test]
#[ignore = "task 2-4-7: migrate `?>` corpus to quaternary `!! err` / implicit propagation"]
fn propagate_converts_via_from_across_layers() {
    let src = "err FileError\n    NotFound\n\nerr NetError\n    Timeout\n\nerr AppError\n    Io(FileError)\n    Net(NetError)\n\nimpl From of FileError for AppError\n    *from(e as FileError) returns AppError\n        Io(e)\n\nimpl From of NetError for AppError\n    *from(e as NetError) returns AppError\n        Net(e)\n\n*fetch(ok as bool) returns Result of i64, NetError\n    if ok\n        Ok(7)\n    else\n        Err(Timeout)\n\n*save(ok as bool) returns Result of i64, FileError\n    if ok\n        Ok(1)\n    else\n        Err(NotFound)\n\n*backup(a as bool, b as bool) returns Result of i64, AppError\n    data is fetch(a)?>\n    n is save(b)?>\n    Ok(data + n)\n\n*main()\n    match backup(true, true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match backup(false, true)\n        Ok(v) ? log(0)\n        Err(e) ? match e\n            Net(_) ? log(2)\n            Io(_) ? log(3)\n    match backup(true, false)\n        Ok(v) ? log(0)\n        Err(e) ? match e\n            Net(_) ? log(2)\n            Io(_) ? log(3)\n";
    expect(src, "8\n2\n3");
}

#[test]
#[ignore = "task 2-4-7: migrate `?>` corpus to quaternary `!! err` / implicit propagation"]
fn propagate_on_option_returns_nothing() {
    expect(
        "*find(ok as bool) returns Option of i64\n    if ok\n        Some(5)\n    else\n        Nothing\n\n*chain(ok as bool) returns Option of i64\n    v is find(ok)?>\n    Some(v + 1)\n\n*main()\n    log(chain(true).unwrap_or(-1))\n    log(chain(false).unwrap_or(-1))\n",
        "6\n-1",
    );
}

#[test]
#[ignore = "task 2-4-7: migrate `?>` corpus to quaternary `!! err` / implicit propagation"]
fn propagate_outside_fallible_fn_is_error() {
    let out = compile_fails(
        "err E\n    Oops\n\n*read() returns Result of i64, E\n    Err(Oops)\n\n*bad() returns i64\n    x is read()?>\n    x\n\n*main()\n    log(0)\n",
    );
    assert!(
        out.contains("?>") && out.contains("Result"),
        "expected `?>`-outside-fallible diagnostic, got:\n{out}"
    );
}

#[test]
fn bang_early_return_still_works() {
    expect(
        "err MyErr\n    Oops\n\n*f(ok as bool) returns Result of i64, MyErr\n    if ok\n        Ok(1)\n    else\n        ! Oops\n\n*main()\n    match f(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-9)\n",
        "-9",
    );
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

#[test]
#[ignore = "task 2-4-7: migrate `?>` corpus to quaternary `!! err` / implicit propagation"]
fn propagate_missing_conversion_is_error() {
    let out = compile_fails(
        "err FileError\n    NotFound\n\nerr NetError\n    Timeout\n\n*read(ok as bool) returns Result of i64, FileError\n    if ok\n        Ok(1)\n    else\n        Err(NotFound)\n\n*load(ok as bool) returns Result of i64, NetError\n    raw is read(ok)?>\n    Ok(raw)\n\n*main()\n    log(0)\n",
    );
    assert!(
        out.contains("no conversion `FileError -> NetError`")
            && out.contains("impl From of FileError for NetError"),
        "expected precise missing-conversion diagnostic, got:\n{out}"
    );
}

#[test]
fn bang_undeclared_error_is_error() {
    let out = compile_fails(
        "err FileError\n    NotFound\n\nerr NetError\n    Timeout\n\n*load(ok as bool) returns Result of i64, NetError\n    if ok\n        Ok(1)\n    else\n        ! NotFound\n\n*main()\n    log(0)\n",
    );
    assert!(
        out.contains("no conversion `FileError -> NetError`")
            || out.contains("FileError"),
        "expected undeclared-error/soundness diagnostic, got:\n{out}"
    );
}

#[test]
#[ignore = "task 2-4-7: migrate `?>` corpus to quaternary `!! err` / implicit propagation"]
fn propagate_reflexive_conversion_needs_no_impl() {
    expect(
        "err FileError\n    NotFound\n\n*read(ok as bool) returns Result of i64, FileError\n    if ok\n        Ok(7)\n    else\n        Err(NotFound)\n\n*load(ok as bool) returns Result of i64, FileError\n    raw is read(ok)?>\n    Ok(raw + 1)\n\n*main()\n    match load(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match load(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n",
        "8\n-1",
    );
}
