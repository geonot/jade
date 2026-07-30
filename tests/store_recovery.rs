//! Task 8-23 — WAL replay on recovery. Before this task the WAL was
//! write-only: deleting the `.store` and keeping the `.wal` yielded zero
//! rows, and corrupting the WAL changed nothing anywhere. Store open now
//! performs recovery: replay committed WAL records missing from the data
//! file (upsert by `sid`), rewrite atomically, invalidate `.idx`
//! sidecars, then checkpoint — a WAL prefix is discardable only once its
//! effects are durably in the data file.

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
        "jinnc compilation failed:\n{}",
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

fn stdout_lines(o: &std::process::Output) -> Vec<String> {
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .collect()
}

const WRITER: &str = "store items\n    name as String\n    qty as i64\n\n*main\n    for i in 0 to 20\n        insert items 'w', i\n    log(count items)\n";
const READER: &str = "store items\n    name as String\n    qty as i64\n\n*main\n    log(count items)\n";

/// Probe: delete the `.store`, keep the `.wal` — every committed record
/// is restored from the log (this yielded 0 rows before 8-23).
#[test]
fn recovery_restores_store_from_wal_alone() {
    let dir = tempfile::tempdir().unwrap();
    let w = compile_in(dir.path(), "w", WRITER);
    let r = run(dir.path(), &w);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(stdout_lines(&r), vec!["20"]);

    std::fs::remove_file(dir.path().join("items.store")).unwrap();

    let rd = compile_in(dir.path(), "r", READER);
    let r = run(dir.path(), &rd);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(
        stdout_lines(&r),
        vec!["20"],
        "all rows must be restored from the WAL; stderr: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(
        stderr.contains("restoring"),
        "recovery must announce itself: {stderr}"
    );
}

/// Probe: delete the `.wal`, keep the `.store` — unchanged rows.
#[test]
fn store_without_wal_is_unaffected() {
    let dir = tempfile::tempdir().unwrap();
    let w = compile_in(dir.path(), "w", WRITER);
    run(dir.path(), &w);
    std::fs::remove_file(dir.path().join("items.wal")).unwrap();
    let rd = compile_in(dir.path(), "r", READER);
    let r = run(dir.path(), &rd);
    assert!(r.status.success());
    assert_eq!(stdout_lines(&r), vec!["20"]);
}

/// Probe: a stale data file (rolled back to an earlier state) is healed
/// forward from the WAL.
#[test]
fn stale_data_file_is_healed_from_wal() {
    let dir = tempfile::tempdir().unwrap();
    let w1 = compile_in(
        dir.path(),
        "w1",
        "store items\n    name as String\n    qty as i64\n\n*main\n    for i in 0 to 5\n        insert items 'a', i\n    log(count items)\n",
    );
    let r = run(dir.path(), &w1);
    assert_eq!(stdout_lines(&r), vec!["5"]);
    // Snapshot the 5-row data file, then write 15 more rows.
    let stale = std::fs::read(dir.path().join("items.store")).unwrap();
    let w2 = compile_in(
        dir.path(),
        "w2",
        "store items\n    name as String\n    qty as i64\n\n*main\n    for i in 0 to 15\n        insert items 'b', 100 + i\n    log(count items)\n",
    );
    let r = run(dir.path(), &w2);
    assert_eq!(stdout_lines(&r), vec!["20"]);
    // Roll the data file back to the stale 5-row snapshot; the WAL still
    // holds w2's 15 inserts (w2's open checkpointed w1's entries away).
    std::fs::write(dir.path().join("items.store"), &stale).unwrap();

    let rd = compile_in(dir.path(), "r", READER);
    let r = run(dir.path(), &rd);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(
        stdout_lines(&r),
        vec!["20"],
        "recovery must heal the stale file forward; stderr: {}",
        String::from_utf8_lossy(&r.stderr)
    );
}

/// Probe: corrupting WAL bytes is DETECTED and reported (it used to be
/// invisible — identical results to the uncorrupted control), and the
/// valid prefix is still usable.
#[test]
fn wal_corruption_is_detected_and_prefix_recovered() {
    let dir = tempfile::tempdir().unwrap();
    let w = compile_in(dir.path(), "w", WRITER);
    run(dir.path(), &w);

    let wal_path = dir.path().join("items.wal");
    let mut wal = std::fs::read(&wal_path).unwrap();
    assert!(wal.len() > 2064, "need a WAL long enough to corrupt at 2000");
    for b in &mut wal[2000..2064] {
        *b ^= 0xA5;
    }
    std::fs::write(&wal_path, &wal).unwrap();
    // Remove the store so recovery MUST lean on the (damaged) WAL.
    std::fs::remove_file(dir.path().join("items.store")).unwrap();

    let rd = compile_in(dir.path(), "r", READER);
    let r = run(dir.path(), &rd);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(
        stderr.contains("torn or corrupt"),
        "corruption must be detected and reported: {stderr}"
    );
    let lines = stdout_lines(&r);
    let n: i64 = lines[0].parse().unwrap();
    assert!(
        n > 0 && n < 20,
        "only the valid WAL prefix should be restored, got {n}"
    );
}

/// Probe: kill -9 mid-insert — the next open recovers to a consistent
/// state (runs clean; count equals whatever prefix of inserts became
/// durable, and a WAL-ahead record is replayed in rather than lost).
#[test]
fn kill_nine_mid_insert_recovers_consistently() {
    use std::os::unix::process::ExitStatusExt;
    let dir = tempfile::tempdir().unwrap();
    let w = compile_in(
        dir.path(),
        "w",
        "store items\n    name as String\n    qty as i64\n\n*main\n    i is 0\n    while true\n        insert items 'k', i\n        i is i + 1\n",
    );
    let mut child = Command::new(&w)
        .current_dir(dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let _ = Command::new("kill")
        .args(["-9", &child.id().to_string()])
        .status();
    let st = child.wait().unwrap();
    assert_eq!(st.signal(), Some(9));

    let rd = compile_in(dir.path(), "r", READER);
    let r = run(dir.path(), &rd);
    assert!(
        r.status.success(),
        "reader must recover cleanly: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    let n: i64 = stdout_lines(&r)[0].parse().unwrap();
    assert!(n > 0, "some inserts must have survived");
    // A second open agrees with the first (recovery is idempotent and
    // checkpointing made the state stable).
    let r2 = run(dir.path(), &rd);
    let n2: i64 = stdout_lines(&r2)[0].parse().unwrap();
    assert_eq!(n, n2, "recovered state must be stable across opens");
}

/// Task 8-25 (first half) — `all <store>` is a first-class row set:
/// iterate empty, iterate many, bind, `.length`, pass to a function,
/// and tombstoned rows are excluded.
#[test]
fn all_store_first_class_rows() {
    let dir = tempfile::tempdir().unwrap();
    let src = "store items\n    name as String\n    qty as i64\n\n*total(rows)\n    t is 0\n    for r in rows\n        t is t + r.qty\n    log(t)\n\n*main\n    e is all items\n    log(e.length)\n    insert items 'a', 1\n    insert items 'b', 2\n    insert items 'c', 4\n    delete items where name equals 'b'\n    rows is all items\n    log(rows.length)\n    total(rows)\n";
    let bin = compile_in(dir.path(), "p", src);
    let out = run(dir.path(), &bin);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_lines(&out), vec!["0", "2", "5"]);
}
