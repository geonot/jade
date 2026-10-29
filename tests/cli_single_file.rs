use std::path::PathBuf;
use std::process::{Command, Stdio};

fn jinn() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinn"))
}

fn run(dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(jinn())
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("invoke jinn");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

const SRC: &str = "*add(a as i64, b as i64) returns i64\n    a + b\n\ntest 'adds'\n    assert add(1, 2) equals EXPECTED\n\n*main\n    log(add(1, 1))\n";

#[test]
fn jinn_test_accepts_a_file_and_reports_failures_off_tty() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("t.jn"), SRC.replace("EXPECTED", "3")).unwrap();
    let (ok, out) = run(dir.path(), &["test", "t.jn"]);
    assert!(ok, "passing test file failed:\n{out}");
    assert!(out.contains("1 test passed"), "{out}");

    let (ok, out) = run(dir.path(), &["check", "t.jn"]);
    assert!(ok, "check on a file failed:\n{out}");

    std::fs::write(dir.path().join("t.jn"), SRC.replace("EXPECTED", "4")).unwrap();
    let (ok, out) = run(dir.path(), &["test", "t.jn"]);
    assert!(!ok, "failing test reported success:\n{out}");
    assert!(out.contains("test adds"), "{out}");
    assert!(out.contains("assertion failed: at t.jn:5:5"), "{out}");
    assert!(
        std::fs::read_dir(dir.path()).unwrap().count() == 1,
        "jinn test left artifacts in the working directory"
    );

    let (ok, out) = run(dir.path(), &["test", "missing.jn"]);
    assert!(!ok && out.contains("file not found: missing.jn"), "{out}");
}
