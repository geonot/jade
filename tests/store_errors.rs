use std::path::{Path, PathBuf};
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile_in(dir: &Path, src: &str) -> PathBuf {
    let jinn = dir.join("test.jn");
    let out = dir.join("test_bin");
    std::fs::write(&jinn, src).unwrap();
    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("jinnc failed to start");
    assert!(
        output.status.success(),
        "jinnc compilation failed for:\n{src}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    out
}

fn expect(src: &str, expected: &str) {
    let dir = tempfile::tempdir().unwrap();
    let bin = compile_in(dir.path(), src);
    let output = Command::new(&bin)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        "binary exited with {:?}\nstderr: {}\nsource:\n{src}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        expected.trim(),
        "source:\n{src}"
    );
}

fn expect_trap(src: &str, needle: &str) {
    let dir = tempfile::tempdir().unwrap();
    let bin = compile_in(dir.path(), src);
    let output = Command::new(&bin)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        !output.status.success(),
        "expected trap, but binary succeeded\nsource:\n{src}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(needle),
        "expected stderr to contain {needle:?}, got: {}\nsource:\n{src}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn handled_insert_ok_yields_sid() {
    expect(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Ann', 30 ? log($) !! log(-1)\n    insert users 'Bob', 25 ? log($) !! log(-1)\n",
        "1\n2",
    );
}

#[test]
fn handled_insert_duplicate_yields_err() {
    expect(
        "store emails\n    addr as String @unique\n    name as String\n\n*main\n    insert emails 'a@b.com', 'Alice' ? log($) !! log(-1)\n    insert emails 'a@b.com', 'Bob' ? log($) !! log(-1)\n    log(count emails)\n",
        "1\n-1\n1",
    );
}

#[test]
fn bare_insert_duplicate_traps() {
    expect_trap(
        "store emails\n    addr as String @unique\n    name as String\n\n*main\n    insert emails 'a@b.com', 'Alice'\n    insert emails 'a@b.com', 'Bob'\n",
        "store 'emails': duplicate value for @unique field 'addr'",
    );
}

#[test]
fn bare_insert_empty_required_traps() {
    expect_trap(
        "store regs\n    code as String @required\n    val as i64\n\n*main\n    insert regs '', 3\n",
        "store 'regs': missing @required field 'code'",
    );
}

#[test]
fn store_error_variants_matchable() {
    expect(
        "store regs\n    code as String @unique @required\n    val as i64\n\n*add(c as String, v as i64) returns Result of i64, StoreError\n    insert regs c, v !! err\n    Ok(1)\n\n*main\n    match add('x', 1)\n        Ok(v) ? log(v)\n        Err(e) ? match e\n            Duplicate ? log(-1)\n            Missing ? log(-2)\n            Constraint ? log(-3)\n            Io ? log(-4)\n    match add('x', 2)\n        Ok(v) ? log(v)\n        Err(e) ? match e\n            Duplicate ? log(-1)\n            Missing ? log(-2)\n            Constraint ? log(-3)\n            Io ? log(-4)\n    match add('', 3)\n        Ok(v) ? log(v)\n        Err(e) ? match e\n            Duplicate ? log(-1)\n            Missing ? log(-2)\n            Constraint ? log(-3)\n            Io ? log(-4)\n    log(count regs)\n",
        "1\n-1\n-3\n1",
    );
}

#[test]
fn handled_set_yields_updated_count() {
    expect(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Ann', 30\n    insert users 'Bo', 30\n    set users where age equals 30 age 31 ? log($) !! log(-7)\n    match users where name equals 'Ann'\n        Ok(r) ? log(r.age)\n        Err(e) ? log(0 - 1)\n",
        "2\n31",
    );
}

#[test]
fn handled_set_no_match_yields_missing() {
    expect(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Ann', 30\n    set users where name equals 'Zed' age 99 ? log($) !! log(-7)\n    match users where name equals 'Ann'\n        Ok(r) ? log(r.age)\n        Err(e) ? log(0 - 1)\n",
        "-7\n30",
    );
}

#[test]
fn multiline_arms_on_insert_and_set() {
    expect(
        "store t9\n    name as String @unique\n    v as i64\n\n*main\n    insert t9 'a', 1\n    insert t9 'a', 2\n        ? log($)\n        !! log(-1)\n    set t9 where name equals 'zzz' v 5\n        ? log($)\n        !! log(-9)\n    log(count t9)\n",
        "-1\n-9\n1",
    );
}

#[test]
fn err_only_arm_runs_on_failure_only() {
    expect(
        "store w1\n    name as String @unique\n    v as i64\n\n*main\n    insert w1 'a', 1 !! log(-1)\n    insert w1 'a', 2 !! log(-1)\n    log(count w1)\n",
        "-1\n1",
    );
}

#[test]
fn insert_error_propagates_with_bare_err() {
    expect(
        "store accs\n    code as String @unique\n    v as i64\n\n*add(c as String) returns Result of i64, StoreError\n    sid is insert accs c, 1 ? $ !! err\n    Ok(sid)\n\n*main\n    match add('k')\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match add('k')\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n",
        "1\n-1",
    );
}

#[test]
fn user_declared_store_error_overrides_builtin() {
    expect(
        "err StoreError\n    Duplicate\n    Missing\n    Constraint\n    Io\n    Custom\n\n*f() returns Result of i64, StoreError\n    Err(Custom)\n\n*main\n    match f()\n        Ok(v) ? log(v)\n        Err(e) ? match e\n            Custom ? log(42)\n            _ ? log(0)\n",
        "42",
    );
}

#[test]
fn handled_insert_inside_transaction_rolls_back_on_propagation() {
    expect(
        "store inv\n    code as String @unique\n    qty as i64\n\n*restock(c as String) returns Result of i64, StoreError\n    transaction\n        insert inv c, 10 !! err\n        insert inv c, 20 !! err\n    Ok(1)\n\n*main\n    match restock('a')\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    log(count inv)\n",
        "-1\n0",
    );
}
