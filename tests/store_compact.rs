//! Conformance for tombstone reclamation
//! (store-improvement.md item 6, task 2-31-10). Pins the documented semantics:
//!
//!   * `compact StoreName` rewrites the record file dropping every
//!     soft-deleted (tombstoned) row, shrinking the on-disk file;
//!   * live rows survive compaction with all field values intact and
//!     queries continue to resolve;
//!   * compaction preserves the schema fingerprint/version in the header so
//!     a subsequent open does not trip the migration guard;
//!   * the `@compact(threshold)` decorator auto-compacts once the number of
//!     tombstones reaches the threshold after a delete.

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

fn header(dir: &Path, store: &str) -> (i64, i64, i64) {
    let bytes = std::fs::read(dir.join(format!("{store}.store"))).unwrap();
    let count = i64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let fp = i64::from_le_bytes(bytes[24..32].try_into().unwrap());
    (count, bytes.len() as i64, fp)
}

#[test]
fn compact_reclaims_tombstones_and_keeps_live_rows() {
    let dir = tempfile::tempdir().unwrap();
    let src = "store people\n    name as String\n    age as i64\n\n*main\n    insert people 'zoe', 30\n    insert people 'amy', 25\n    insert people 'bob', 40\n    delete people where name equals 'amy'\n    log(count people)\n    compact people\n    log(count people)\n    r is people where name equals 'bob'\n    log r.age\n    z is people where name equals 'zoe'\n    log z.age\n";
    let bin = compile_in(dir.path(), "p", src);
    let r = run(dir.path(), &bin);
    assert!(
        r.status.success(),
        "run: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(out_lines(&r), vec!["2", "2", "40", "30"]);

    let (count, _len, fp) = header(dir.path(), "people");
    assert_eq!(count, 2, "compacted header should hold only live rows");
    assert_ne!(fp, 0, "compaction must preserve the schema fingerprint");
}

#[test]
fn compact_shrinks_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let no_compact = "store t\n    name as String\n    age as i64\n\n*main\n    insert t 'a', 1\n    insert t 'b', 2\n    insert t 'c', 3\n    insert t 'd', 4\n    delete t where name equals 'b'\n    delete t where name equals 'c'\n";
    let b1 = compile_in(dir.path(), "nc", no_compact);
    assert!(run(dir.path(), &b1).status.success());
    let (count_before, _, _) = header(dir.path(), "t");
    assert_eq!(count_before, 4, "soft delete keeps physical rows");

    let with_compact = "store t\n    name as String\n    age as i64\n\n*main\n    compact t\n";
    let b2 = compile_in(dir.path(), "wc", with_compact);
    assert!(run(dir.path(), &b2).status.success());
    let (count_after, _, fp) = header(dir.path(), "t");
    assert_eq!(count_after, 2, "compaction drops the two tombstones");
    assert_ne!(fp, 0);
}

#[test]
fn compact_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let writer = "store people\n    name as String\n    age as i64\n\n*main\n    insert people 'zoe', 30\n    insert people 'amy', 25\n    insert people 'bob', 40\n    delete people where name equals 'amy'\n    compact people\n";
    let wbin = compile_in(dir.path(), "writer", writer);
    assert!(run(dir.path(), &wbin).status.success());

    let reader = "store people\n    name as String\n    age as i64\n\n*main\n    log(count people)\n    r is people where name equals 'bob'\n    log r.age\n";
    let rbin = compile_in(dir.path(), "reader", reader);
    let r = run(dir.path(), &rbin);
    assert!(
        r.status.success(),
        "reader: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(out_lines(&r), vec!["2", "40"]);
}

#[test]
fn compact_with_no_tombstones_is_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let src = "store t\n    name as String\n    age as i64\n\n*main\n    insert t 'a', 1\n    insert t 'b', 2\n    compact t\n    log(count t)\n    r is t where name equals 'a'\n    log r.age\n";
    let bin = compile_in(dir.path(), "t", src);
    let r = run(dir.path(), &bin);
    assert!(
        r.status.success(),
        "run: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(out_lines(&r), vec!["2", "1"]);
    let (count, _, _) = header(dir.path(), "t");
    assert_eq!(count, 2);
}

#[test]
fn compact_policy_decorator_auto_reclaims_at_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let src = "store people @compact(2)\n    name as String\n    age as i64\n\n*main\n    insert people 'zoe', 30\n    insert people 'amy', 25\n    insert people 'bob', 40\n    insert people 'cat', 5\n    delete people where name equals 'amy'\n    log(count people)\n    delete people where name equals 'cat'\n    log(count people)\n    r is people where name equals 'bob'\n    log r.age\n";
    let bin = compile_in(dir.path(), "p", src);
    let r = run(dir.path(), &bin);
    assert!(
        r.status.success(),
        "run: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(out_lines(&r), vec!["3", "2", "40"]);

    let (count, _, fp) = header(dir.path(), "people");
    assert_eq!(count, 2, "auto-compaction should leave only live rows");
    assert_ne!(fp, 0);
}
