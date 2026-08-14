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
fn has_many_traversal_returns_child_rows() {
    expect(
        "store owners\n    name as String\n    &pets as [pets]\n\nstore pets\n    name as String\n    &owner as owners\n\n*main\n    insert owners 'alice'\n    insert owners 'bob'\n    insert pets 'rex', 1\n    insert pets 'spot', 2\n    insert pets 'fido', 1\n    o is first owners where name eq 'alice'\n    log o.pets.length\n    for p in o.pets\n        log p.name\n    b is first owners where name eq 'bob'\n    log b.pets.length\n",
        "2\nrex\nfido\n1",
    );
}

#[test]
fn has_many_traversal_empty_when_no_children() {
    expect(
        "store owners\n    name as String\n    &pets as [pets]\n\nstore pets\n    name as String\n    &owner as owners\n\n*main\n    insert owners 'alice'\n    o is first owners where name eq 'alice'\n    log o.pets.length\n",
        "0",
    );
}

#[test]
fn has_many_traversal_requires_back_relation() {
    expect_compile_error(
        "store owners\n    name as String\n    &pets as [pets]\n\nstore pets\n    name as String\n\n*main\n    o is first owners where name eq 'alice'\n    log o.pets.length\n",
        "has no belongs-to relation back",
    );
}

#[test]
fn cascade_soft_delete_removes_children() {
    expect(
        "store owners\n    name as String\n    &pets as [pets] @cascade\n\nstore pets\n    name as String\n    &owner as owners\n\n*main\n    insert owners 'alice'\n    insert owners 'bob'\n    insert pets 'rex', 1\n    insert pets 'spot', 2\n    insert pets 'fido', 1\n    delete owners where name eq 'alice'\n    log count pets\n    log count owners\n",
        "1\n1",
    );
}

#[test]
fn cascade_destroy_hard_deletes_children() {
    expect(
        "store owners\n    name as String\n    &pets as [pets] @cascade\n\nstore pets\n    name as String\n    &owner as owners\n\n*main\n    insert owners 'alice'\n    insert pets 'rex', 1\n    insert pets 'spot', 1\n    destroy owners where name eq 'alice'\n    log count pets\n    log count owners\n",
        "0\n0",
    );
}

#[test]
fn cascade_is_transitive() {
    expect(
        "store owners\n    name as String\n    &pets as [pets] @cascade\n\nstore pets\n    name as String\n    &owner as owners\n    &toys as [toys] @cascade\n\nstore toys\n    label as String\n    &pet as pets\n\n*main\n    insert owners 'alice'\n    insert owners 'bob'\n    insert pets 'rex', 1\n    insert pets 'spot', 2\n    insert toys 'ball', 1\n    insert toys 'yarn', 2\n    delete owners where name eq 'alice'\n    log count pets\n    log count toys\n",
        "1\n1",
    );
}

#[test]
fn cascade_on_belongs_to_side_cascades_from_owner() {
    expect(
        "store owners\n    name as String\n\nstore pets\n    name as String\n    &owner as owners @cascade\n\n*main\n    insert owners 'alice'\n    insert owners 'bob'\n    insert pets 'rex', 1\n    insert pets 'spot', 2\n    delete owners where name eq 'alice'\n    log count pets\n",
        "1",
    );
}

#[test]
fn cascade_cycle_is_rejected() {
    expect_compile_error(
        "store a\n    name as String\n    &b as b @cascade\n\nstore b\n    name as String\n    &a as a @cascade\n\n*main\n    log 1\n",
        "@cascade cycle",
    );
}

#[test]
fn cascade_has_many_requires_back_relation() {
    expect_compile_error(
        "store owners\n    name as String\n    &pets as [pets] @cascade\n\nstore pets\n    name as String\n\n*main\n    log 1\n",
        "has no belongs-to relation back",
    );
}
