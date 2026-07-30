//! `jinn fmt` must never destroy source (task 8-2, decision D5).
//!
//! The formatter reprints from the AST and comments are not represented in
//! the AST yet (task 8-18 gives them a trivia channel). Until that lands,
//! `fmt` must refuse any file containing a comment rather than silently
//! deleting every one of them, and in-place modification must require an
//! explicit `--write` (the default prints to stdout).

use std::path::PathBuf;
use std::process::Command;

fn jinn() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinn"))
}

const COMMENTED: &str = "\
# leading comment
*main
    x is 1  # trailing comment
    # standalone comment
    log(x)
";

const UNCOMMENTED_UGLY: &str = "\
*main
    x    is    1
    log(x)
";

#[test]
fn fmt_write_refuses_commented_file_and_leaves_it_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("commented.jn");
    std::fs::write(&path, COMMENTED).unwrap();

    let out = Command::new(jinn())
        .args(["fmt", "--write"])
        .arg(&path)
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "fmt --write on a commented file must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("comment"),
        "diagnostic must say why it refused, got: {stderr}"
    );
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        after, COMMENTED,
        "a commented file must be byte-identical after `jinn fmt --write`"
    );
}

#[test]
fn fmt_default_prints_to_stdout_and_never_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ugly.jn");
    std::fs::write(&path, UNCOMMENTED_UGLY).unwrap();

    let out = Command::new(jinn()).arg("fmt").arg(&path).output().unwrap();

    assert!(out.status.success(), "fmt (stdout mode) should succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("x is 1"),
        "formatted output should go to stdout, got: {stdout}"
    );
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        after, UNCOMMENTED_UGLY,
        "without --write the file must not be modified"
    );
}

#[test]
fn fmt_write_formats_uncommented_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ugly.jn");
    std::fs::write(&path, UNCOMMENTED_UGLY).unwrap();

    let out = Command::new(jinn())
        .args(["fmt", "--write"])
        .arg(&path)
        .output()
        .unwrap();

    assert!(out.status.success(), "fmt --write on a clean file succeeds");
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("x is 1"),
        "file should be reformatted in place, got: {after}"
    );
}

#[test]
fn fmt_refuses_shebang_file() {
    // A shebang is source the AST does not carry, so reprinting would
    // delete it — same non-destructiveness rule as comments.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.jn");
    let src = "#!/usr/bin/env jinn\n*main\n    log(1)\n";
    std::fs::write(&path, src).unwrap();

    let out = Command::new(jinn())
        .args(["fmt", "--write"])
        .arg(&path)
        .output()
        .unwrap();

    assert!(!out.status.success());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), src);
}
