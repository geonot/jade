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
    expect_compile_fail("negative_partial_move.jn", &["moved", "`a`"]);
    expect_compile_fail("negative_resource_copy.jn", &["resource"]);
    expect_compile_fail("negative_resource_channel.jn", &["resource", "thread"]);
    expect_compile_fail(
        "negative_type_mismatch.jn",
        &["type mismatch", "i64", "string"],
    );
    expect_compile_fail("negative_generic_empty_enum.jn", &["FieldGet", "__tag"]);
}

#[test]
fn alpha_audit_runtime_bounds_case() {
    expect_runtime_fail("runtime_bounds_fail.jn");
}
