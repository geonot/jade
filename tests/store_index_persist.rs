use std::path::{Path, PathBuf};
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile_in(dir: &Path, name: &str, src: &str) -> PathBuf {
    let jinn = dir.join(format!("{name}.jn"));
    let out = dir.join(name);
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

fn run(dir: &Path, bin: &Path) -> std::process::Output {
    Command::new(bin)
        .current_dir(dir)
        .output()
        .expect("compiled binary failed to start")
}

fn out_lines(o: &std::process::Output) -> Vec<String> {
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .collect()
}

#[test]
fn string_index_persists_and_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let writer = "store people\n    name as String @index\n    age as i64\n\n*main\n    insert people 'zoe', 30\n    insert people 'amy', 25\n    insert people 'bob', 40\n";
    let wbin = compile_in(dir.path(), "writer", writer);
    let w = run(dir.path(), &wbin);
    assert!(
        w.status.success(),
        "writer: {}",
        String::from_utf8_lossy(&w.stderr)
    );

    assert!(dir.path().join("people.name.idx").exists());

    let reader = "store people\n    name as String @index\n    age as i64\n\n*main\n    match people where name equals 'bob'\n        Ok(r) ? log(r.age)\n        Err(e) ? log(0 - 1)\n    match people where name equals 'amy'\n        Ok(r) ? log(r.age)\n        Err(e) ? log(0 - 1)\n";
    let rbin = compile_in(dir.path(), "reader", reader);
    let r = run(dir.path(), &rbin);
    assert!(
        r.status.success(),
        "reader: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(out_lines(&r), vec!["40", "25"]);
}

#[test]
fn missing_index_rebuilds_from_store() {
    let dir = tempfile::tempdir().unwrap();
    let writer = "store people\n    name as String @index\n    age as i64\n\n*main\n    insert people 'zoe', 30\n    insert people 'amy', 25\n    insert people 'bob', 40\n";
    let wbin = compile_in(dir.path(), "writer", writer);
    assert!(run(dir.path(), &wbin).status.success());

    std::fs::remove_file(dir.path().join("people.name.idx")).unwrap();

    let reader = "store people\n    name as String @index\n    age as i64\n\n*main\n    match people where name equals 'bob'\n        Ok(r) ? log(r.age)\n        Err(e) ? log(0 - 1)\n    match people where name equals 'zoe'\n        Ok(r) ? log(r.age)\n        Err(e) ? log(0 - 1)\n";
    let rbin = compile_in(dir.path(), "reader", reader);
    let r = run(dir.path(), &rbin);
    assert!(
        r.status.success(),
        "reader: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(out_lines(&r), vec!["40", "30"]);
    assert!(dir.path().join("people.name.idx").exists());
}

#[test]
fn corrupt_index_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let writer = "store people\n    name as String @index\n    age as i64\n\n*main\n    insert people 'zoe', 30\n    insert people 'amy', 25\n    insert people 'bob', 40\n";
    let wbin = compile_in(dir.path(), "writer", writer);
    assert!(run(dir.path(), &wbin).status.success());

    let idx_path = dir.path().join("people.name.idx");
    let mut bytes = std::fs::read(&idx_path).unwrap();
    for b in bytes.iter_mut().skip(8).take(16) {
        *b ^= 0xFF;
    }
    std::fs::write(&idx_path, &bytes).unwrap();

    let reader = "store people\n    name as String @index\n    age as i64\n\n*main\n    match people where name equals 'amy'\n        Ok(r) ? log(r.age)\n        Err(e) ? log(0 - 1)\n";
    let rbin = compile_in(dir.path(), "reader", reader);
    let r = run(dir.path(), &rbin);
    assert!(
        r.status.success(),
        "reader: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(out_lines(&r), vec!["25"]);
}

#[test]
fn integer_index_rebuilds_from_store() {
    let dir = tempfile::tempdir().unwrap();
    let writer = "store nums\n    key as i64 @index\n    val as i64\n\n*main\n    insert nums 100, 1\n    insert nums 200, 2\n    insert nums 300, 3\n";
    let wbin = compile_in(dir.path(), "writer", writer);
    assert!(run(dir.path(), &wbin).status.success());

    std::fs::remove_file(dir.path().join("nums.key.idx")).unwrap();

    let reader = "store nums\n    key as i64 @index\n    val as i64\n\n*main\n    match nums where key equals 200\n        Ok(r) ? log(r.val)\n        Err(e) ? log(0 - 1)\n";
    let rbin = compile_in(dir.path(), "reader", reader);
    let r = run(dir.path(), &rbin);
    assert!(
        r.status.success(),
        "reader: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(out_lines(&r), vec!["2"]);
}

#[test]
fn rebuild_skips_soft_deleted_records() {
    let dir = tempfile::tempdir().unwrap();
    let writer = "store people\n    name as String @index\n    age as i64\n\n*main\n    insert people 'zoe', 30\n    insert people 'amy', 25\n    delete people where name equals 'amy'\n";
    let wbin = compile_in(dir.path(), "writer", writer);
    assert!(run(dir.path(), &wbin).status.success());

    std::fs::remove_file(dir.path().join("people.name.idx")).unwrap();

    let reader = "store people\n    name as String @index\n    age as i64\n\n*main\n    match people where name equals 'zoe'\n        Ok(r) ? log(r.age)\n        Err(e) ? log(0 - 1)\n    c is count people\n    log c\n";
    let rbin = compile_in(dir.path(), "reader", reader);
    let r = run(dir.path(), &rbin);
    assert!(
        r.status.success(),
        "reader: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(out_lines(&r), vec!["30", "1"]);
}
