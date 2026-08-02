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

fn expect(src: &str, expected: &str) {
    let dir = tempfile::tempdir().unwrap();
    let bin = compile_in(dir.path(), src).unwrap_or_else(|e| {
        panic!("jinnc compilation failed for:\n{src}\nstderr: {e}");
    });
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

#[test]
fn belongs_to_traversal_resolves_target_row() {
    expect(
        "store owners\n    name as String\n\nstore pets\n    name as String\n    &owner as owners\n\n*main\n    insert owners 'alice'\n    insert owners 'bob'\n    insert pets 'rex', 2\n    insert pets 'spot', 1\n    p is first pets where name eq 'rex'\n    log p.owner.name\n    q is first pets where name eq 'spot'\n    log q.owner.name\n",
        "bob\nalice",
    );
}

#[test]
fn relation_column_is_storable_and_queryable() {
    expect(
        "store owners\n    name as String\n\nstore pets\n    name as String\n    &owner as owners\n\n*main\n    insert owners 'alice'\n    insert owners 'bob'\n    insert pets 'rex', 2\n    n is count pets where owner eq 2\n    log n\n",
        "1",
    );
}

#[test]
fn cascade_decorator_compiles() {
    expect(
        "store owners\n    name as String\n\nstore pets\n    name as String\n    &owner as owners @cascade\n\n*main\n    insert owners 'alice'\n    insert pets 'rex', 1\n    p is first pets where name eq 'rex'\n    log p.owner.name\n",
        "alice",
    );
}

#[test]
fn has_many_is_not_directly_traversable() {
    expect_compile_error(
        "store owners\n    name as String\n    &pets as [pets]\n\nstore pets\n    name as String\n    &owner as owners\n\n*main\n    o is first owners where name eq 'alice'\n    log o.pets\n",
        "has-many relation",
    );
}
