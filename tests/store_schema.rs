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

#[test]
fn matching_schema_reopens_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let src = "store items @simple\n    name as String\n    price as i64\n\n*main\n    insert items 'apple', 5\n    c is count items\n    log c\n";
    let bin = compile_in(dir.path(), "app", src);

    let first = run(dir.path(), &bin);
    assert!(first.status.success());
    assert_eq!(String::from_utf8_lossy(&first.stdout).trim(), "1");

    let second = run(dir.path(), &bin);
    assert!(
        second.status.success(),
        "reopen failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&second.stdout).trim(), "2");
}

#[test]
fn changed_schema_without_migration_aborts() {
    let dir = tempfile::tempdir().unwrap();
    let v1 = "store items @simple\n    name as String\n    price as i64\n\n*main\n    insert items 'apple', 5\n    log count items\n";
    let bin1 = compile_in(dir.path(), "v1", v1);
    let r1 = run(dir.path(), &bin1);
    assert!(r1.status.success());

    let v2 = "store items @simple\n    label as String\n    cost as i64\n\n*main\n    insert items 'banana', 9\n    log count items\n";
    let bin2 = compile_in(dir.path(), "v2", v2);
    let r2 = run(dir.path(), &bin2);

    assert!(
        !r2.status.success(),
        "expected schema-mismatch abort, but binary succeeded\nstdout: {}",
        String::from_utf8_lossy(&r2.stdout)
    );
    let stderr = String::from_utf8_lossy(&r2.stderr);
    assert!(
        stderr.contains("schema mismatch") && stderr.contains("items"),
        "diagnostic missing schema-mismatch detail:\n{stderr}"
    );
}

#[test]
fn migration_bridges_schema_change() {
    let dir = tempfile::tempdir().unwrap();
    let v1 = "store items @simple\n    name as String\n    price as i64\n\n*main\n    insert items 'apple', 5\n    log count items\n";
    let bin1 = compile_in(dir.path(), "v1", v1);
    let r1 = run(dir.path(), &bin1);
    assert!(r1.status.success());

    let v2 = "store items @simple\n    name as String\n    price as i64\n    stock as i64\n\nmigration 'add_stock' version 1\n    up\n        alter items\n            add stock as i64\n\n*main\n    insert items 'banana', 9, 3\n    log count items\n";
    let bin2 = compile_in(dir.path(), "v2", v2);
    let r2 = run(dir.path(), &bin2);
    assert!(
        r2.status.success(),
        "migration should bridge the schema change, but binary aborted:\n{}",
        String::from_utf8_lossy(&r2.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&r2.stdout).trim(), "2");

    let r3 = run(dir.path(), &bin2);
    assert!(
        r3.status.success(),
        "post-migration reopen failed:\n{}",
        String::from_utf8_lossy(&r3.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&r3.stdout).trim(), "3");
}

#[test]
fn migration_add_default_populates_existing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let v1 = "store items @simple\n    name as String\n\n*main\n    insert items 'widget'\n    log count items\n";
    let bin1 = compile_in(dir.path(), "v1", v1);
    let r1 = run(dir.path(), &bin1);
    assert!(r1.status.success());
    assert_eq!(String::from_utf8_lossy(&r1.stdout).trim(), "1");

    let v2 = "store items @simple\n    name as String\n    price as i64\n\nmigration 'add_price' version 1\n    up\n        alter items\n            add price as i64 default 42\n\n*main\n    match items where price equals 42\n        Ok(r) ? log r.name\n        Err(e) ? log 'default missing'\n    log count items\n";
    let bin2 = compile_in(dir.path(), "v2", v2);
    let r2 = run(dir.path(), &bin2);
    assert!(
        r2.status.success(),
        "migrated reopen failed:\n{}",
        String::from_utf8_lossy(&r2.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&r2.stdout).trim(),
        "widget\n1",
        "the pre-existing row must carry the migration default"
    );
}

#[test]
fn interrupted_migration_fingerprint_refuses_to_auto_adopt() {
    let dir = tempfile::tempdir().unwrap();
    let src = "store items @simple\n    name as String\n\n*main\n    insert items 'a'\n    log count items\n";
    let bin = compile_in(dir.path(), "app", src);
    let r1 = run(dir.path(), &bin);
    assert!(r1.status.success());

    let store = dir.path().join("items.store");
    let mut bytes = std::fs::read(&store).unwrap();
    bytes[24..32].copy_from_slice(&(-1i64).to_le_bytes());
    std::fs::write(&store, &bytes).unwrap();

    let r2 = run(dir.path(), &bin);
    assert!(
        !r2.status.success(),
        "a migration-in-progress fingerprint must refuse to open"
    );
    let stderr = String::from_utf8_lossy(&r2.stderr);
    assert!(
        stderr.contains("migration was interrupted"),
        "stderr: {stderr}"
    );
}

#[test]
fn inflated_header_count_refuses_to_read() {
    let dir = tempfile::tempdir().unwrap();
    let src = "store items @simple\n    name as String\n\n*main\n    insert items 'a'\n    log count items\n";
    let bin = compile_in(dir.path(), "app", src);
    let r1 = run(dir.path(), &bin);
    assert!(r1.status.success());

    let store = dir.path().join("items.store");
    let mut bytes = std::fs::read(&store).unwrap();
    bytes[8..16].copy_from_slice(&1_000_000i64.to_le_bytes());
    std::fs::write(&store, &bytes).unwrap();

    let r2 = run(dir.path(), &bin);
    assert!(
        !r2.status.success(),
        "an inflated header count must refuse to read, stdout: {}",
        String::from_utf8_lossy(&r2.stdout)
    );
    let stderr = String::from_utf8_lossy(&r2.stderr);
    assert!(stderr.contains("corrupt or truncated"), "stderr: {stderr}");
}
