use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/audit_alpha/{name}")).unwrap()
}

fn compile_source(src: &str, extra_args: &[&str]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    let out = dir.path().join("test_bin");
    std::fs::write(&jinn, src).unwrap();
    let mut cmd = Command::new(jinnc());
    for arg in extra_args {
        cmd.arg(arg);
    }
    let status = cmd
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jinnc failed to start");
    assert!(status.success(), "jinnc compilation failed for:\n{src}");
    (dir, out)
}

fn run_fixture(name: &str) -> String {
    let src = fixture(name);
    let (dir, out) = compile_source(&src, &[]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        "binary exited with {:?}\nstderr: {}\nsource:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
        src
    );
    String::from_utf8(output.stdout).unwrap()
}

fn expect_fixture(name: &str, expected: &str) {
    let got = run_fixture(name);
    assert_eq!(got.trim(), expected.trim(), "fixture: {name}");
}

fn expect_compile_fail(name: &str, needles: &[&str]) {
    let src = fixture(name);
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    let out = dir.path().join("test_bin");
    std::fs::write(&jinn, &src).unwrap();
    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("jinnc failed to start");
    assert!(
        !output.status.success(),
        "expected compile failure for:\n{src}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for needle in needles {
        assert!(
            stderr.contains(needle),
            "expected diagnostic to contain `{needle}`; stderr was:\n{stderr}"
        );
    }
}

fn expect_runtime_fail(name: &str) {
    let src = fixture(name);
    let (dir, out) = compile_source(&src, &[]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        !output.status.success(),
        "expected runtime failure; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn alpha_audit_core_semantics() {
    expect_fixture(
        "core_semantics.jn",
        "37\n255\n64\n1024\n2.500000\nalpha-audit\n10\n720",
    );
}

#[test]
fn alpha_audit_types_patterns_ownership() {
    expect_fixture(
        "types_patterns_ownership.jn",
        "6\nrestored\n15\n9\n7\nscope-end\nresource-dropped",
    );
}

#[test]
fn alpha_audit_store_channels_actors() {
    expect_fixture("store_channels_actors.jn", "250\n333\n333\n11\n42");
}

#[test]
fn alpha_audit_generic_empty_enum() {
    expect_fixture("generic_empty_enum.jn", "9\n7");
}

#[test]
fn alpha_audit_generic_struct_param() {
    expect_fixture("generic_struct_param.jn", "42");
}

#[test]
fn alpha_audit_recursive_generic_enum() {
    expect_fixture("recursive_generic_enum.jn", "15\n4");
}

#[test]
fn alpha_audit_soft_keyword_idents() {
    expect_fixture("soft_keyword_idents.jn", "20\n105");
}

#[test]
fn alpha_audit_bracket_list_type() {
    expect_fixture("bracket_list_type.jn", "60\n15");
}

#[test]
fn alpha_audit_chr_builtin() {
    expect_fixture("chr_builtin.jn", "ABC\n7");
}

#[test]
fn alpha_audit_soft_keyword_nouns() {
    expect_fixture("soft_keyword_nouns.jn", "7\n8");
}

#[test]
fn alpha_audit_word_operator_idents() {
    expect_fixture("word_operator_idents.jn", "116");
}

#[test]
fn alpha_audit_std_test_mode() {
    let src = fixture("std_test_mode.jn");
    let (dir, out) = compile_source(&src, &["--test"]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("test binary failed to start");
    assert!(output.status.success(), "test binary failed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("test math and assertions ..."), "{stdout}");
    assert!(stdout.contains("test float guardrails ..."), "{stdout}");
    assert!(!stdout.contains("main-should-not-run"), "{stdout}");
}

#[test]
fn alpha_audit_negative_compile_cases() {
    expect_compile_fail("negative_partial_move.jn", &["use of moved field", "`p.a`"]);
    expect_compile_fail("negative_resource_copy.jn", &["resource"]);
    expect_compile_fail("negative_resource_channel.jn", &["resource", "thread"]);
    expect_compile_fail(
        "negative_type_mismatch.jn",
        &["type mismatch", "i64", "string"],
    );
    expect_compile_fail(
        "negative_return_of_borrowed.jn",
        &["returning reference to local variable", "`x`"],
    );
}

#[test]
fn alpha_audit_runtime_bounds_case() {
    expect_runtime_fail("runtime_bounds_fail.jn");
}

#[test]
fn alpha_audit_eprint_to_stderr() {
    let src = "*main\n    \
        eprint('hello stderr')\n    \
        eprint(42)\n    \
        print('stdout only')\n";
    let (dir, out) = compile_source(src, &[]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(output.status.success(), "binary exited non-zero");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        stdout, "stdout only\n",
        "stdout should hold only print output"
    );
    assert_eq!(
        stderr, "hello stderr\n42\n",
        "stderr should hold eprint output with trailing newlines"
    );
}

#[test]
fn alpha_audit_native_stack_overflow_diagnostic() {
    let src = r#"*blow(n)
    if n > 100000000
        return n
    m is blow(n + 1)
    return m + n

*main
    x is blow(0)
    log(x)
    0
"#;
    let (dir, out) = compile_source(src, &["--opt", "0"]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(134),
        "expected exit 134 from stack-overflow handler; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("stack overflow (native thread)"),
        "expected specific native stack-overflow diagnostic; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("ulimit -s"),
        "expected remediation advice mentioning the OS stack limit; stderr:\n{stderr}"
    );
}

fn run_expect_trap(src: &str, needle: &str) {
    let (dir, out) = compile_source(src, &[]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let aborted = output.status.code() == Some(134) || {
        use std::os::unix::process::ExitStatusExt;
        output.status.signal() == Some(6)
    };
    assert!(
        aborted,
        "expected abort (exit 134 / SIGABRT); status: {:?}\nstdout: {}\nstderr: {stderr}",
        output.status,
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        stderr.contains(needle),
        "expected trap diagnostic containing `{needle}`; stderr:\n{stderr}"
    );
}

#[test]
fn alpha_audit_div_by_zero_traps() {
    run_expect_trap(
        "*main\n    a is 10\n    b is 0\n    log(a / b)\n",
        "integer division by zero",
    );
    run_expect_trap(
        "*main\n    a is 10\n    b is 0\n    log(a % b)\n",
        "integer remainder by zero",
    );
    run_expect_trap(
        "*main\n    a is 10 as u64\n    b is 0 as u64\n    log(a / b)\n",
        "integer division by zero",
    );
}

#[test]
fn alpha_audit_int_min_div_neg_one_traps() {
    run_expect_trap(
        "*main\n    a is -9223372036854775808\n    b is -1\n    log(a / b)\n",
        "signed integer overflow in division",
    );
}

#[test]
fn alpha_audit_vec_oob_diagnostic() {
    run_expect_trap(
        "*main\n    v is [10, 20]\n    log(v[5])\n",
        "vec index out of bounds",
    );
}

#[test]
fn alpha_audit_generator_compiles_and_runs() {
    let src = "*counts(n as i64)\n    \
        for i from 0 to n\n        \
        yield i\n\n\
        *main\n    \
        for v in counts(5)\n        \
        log(v)\n";
    let (dir, out) = compile_source(src, &[]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        "generator program failed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "0\n1\n2\n3\n4\n",
        "generator must yield 0..5"
    );
}

#[test]
fn alpha_audit_take_in_argument_position() {
    let src = "*consume(v as [i64])\n    \
        log(v[0])\n\n\
        *main\n    \
        v is [7, 8, 9]\n    \
        consume(take v)\n";
    let (dir, out) = compile_source(src, &[]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        "take-in-arg program failed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "7\n");
}

#[test]
fn alpha_audit_bare_toplevel_expression_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    std::fs::write(&jinn, "42\n").unwrap();
    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(dir.path().join("test_bin"))
        .output()
        .expect("jinnc failed to start");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bare expression at top level"),
        "expected top-level-expression diagnostic; stderr:\n{stderr}"
    );
}

#[test]
fn alpha_audit_missing_main_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    std::fs::write(&jinn, "*helper\n    log(1)\n").unwrap();
    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(dir.path().join("test_bin"))
        .output()
        .expect("jinnc failed to start");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("program has no `*main` function"),
        "expected missing-main diagnostic; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("undefined reference"),
        "missing main must not surface as a linker error; stderr:\n{stderr}"
    );
}

#[test]
fn alpha_audit_keyword_method_names_after_dot() {
    let src = "*main\n    \
        ch is channel of i64\n    \
        ok is ch.send(1)\n    \
        log(ok)\n";
    let (dir, out) = compile_source(src, &[]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        ".send() program failed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n");

    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    std::fs::write(&jinn, "*main\n    ch is channel of i64\n    ch.close()\n").unwrap();
    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(dir.path().join("test_bin"))
        .output()
        .expect("jinnc failed to start");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("close {ch}"),
        "expected targeted close-statement redirect; stderr:\n{stderr}"
    );
}
