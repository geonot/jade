//! Conformance for real WAL-backed store transactions (store-improvement.md
//! item 1, task 2-31-1). Pins the documented semantics from jinn.md:
//!
//!   * a `transaction` block that completes normally commits its writes as
//!     one durable batch (group commit);
//!   * an error escaping the block (`err` propagation) rolls back every
//!     store mutation made inside it — inserts, sets, deletes, and index
//!     updates alike;
//!   * `return` leaving the block commits work done so far;
//!   * a runtime trap inside the block rolls back before aborting, so a
//!     restarted process observes the pre-transaction state;
//!   * nested `transaction` blocks join the outermost one: inner commits
//!     are deferred and any rollback aborts the whole nest;
//!   * rollback restores data files byte-exactly, so sids and indexes
//!     assigned inside the aborted transaction are reused afterwards.

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

fn run_in(dir: &Path, bin: &Path) -> std::process::Output {
    Command::new(bin)
        .current_dir(dir)
        .output()
        .expect("compiled binary failed to start")
}

fn expect(src: &str, expected: &str) {
    let dir = tempfile::tempdir().unwrap();
    let bin = compile_in(dir.path(), "t", src);
    let output = run_in(dir.path(), &bin);
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

#[test]
fn commit_on_normal_completion() {
    expect(
        "store ledger\n    amount as i64\n\n*main\n    transaction\n        insert ledger 10\n        insert ledger 20\n        insert ledger 30\n    log(count ledger)\n",
        "3",
    );
}

#[test]
fn rollback_on_escaping_error_discards_inserts() {
    expect(
        "err OpErr\n    Boom\n    S(StoreError)\n\nimpl From of StoreError for OpErr\n    *from(e as StoreError) returns OpErr is S(e)\n\nstore ledger\n    amount as i64\n\n*apply(ok as bool) returns Result of i64, OpErr\n    transaction\n        insert ledger 10\n        insert ledger 20\n        if not ok\n            err Boom\n        insert ledger 30\n    Ok(count ledger)\n\n*main\n    match apply(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    log(count ledger)\n    match apply(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n",
        "-1\n0\n3",
    );
}

#[test]
fn rollback_restores_sets_and_deletes() {
    expect(
        "err XferErr\n    Bad\n\nstore accts\n    name as String\n    bal as i64\n\n*xfer(ok as bool) returns Result of i64, XferErr\n    transaction\n        set accts where name equals 'a' bal 50\n        delete accts where name equals 'b'\n        if not ok\n            err Bad\n    Ok(1)\n\n*main\n    insert accts 'a', 100\n    insert accts 'b', 200\n    match xfer(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    r is accts where name equals 'a'\n    log r.bal\n    log(count accts)\n    match xfer(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    r2 is accts where name equals 'a'\n    log r2.bal\n    log(count accts)\n",
        "-1\n100\n2\n1\n50\n1",
    );
}

#[test]
fn rollback_restores_secondary_index() {
    expect(
        "err TErr\n    Nope\n    S(StoreError)\n\nimpl From of StoreError for TErr\n    *from(e as StoreError) returns TErr is S(e)\n\nstore people\n    name as String @index\n    age as i64\n\n*addp(ok as bool) returns Result of i64, TErr\n    transaction\n        insert people 'zoe', 30\n        if not ok\n            err Nope\n    Ok(1)\n\n*main\n    match addp(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    log(count people)\n    r is people where name equals 'zoe'\n    log r.age\n    match addp(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    r2 is people where name equals 'zoe'\n    log r2.age\n",
        "-1\n0\n0\n1\n30",
    );
}

#[test]
fn return_mid_block_commits_partial_work() {
    expect(
        "store ledger\n    amount as i64\n\n*add(halt as bool) returns i64\n    transaction\n        insert ledger 10\n        if halt\n            return -5\n        insert ledger 20\n    1\n\n*main\n    log(add(true))\n    log(count ledger)\n    log(add(false))\n    log(count ledger)\n",
        "-5\n1\n1\n3",
    );
}

#[test]
fn nested_transactions_join_outermost() {
    expect(
        "err NE\n    Whoops\n    S(StoreError)\n\nimpl From of StoreError for NE\n    *from(e as StoreError) returns NE is S(e)\n\nstore stock\n    qty as i64\n\n*inner(ok as bool) returns Result of i64, NE\n    transaction\n        insert stock 2\n        if not ok\n            err Whoops\n    Ok(2)\n\n*outer(ok as bool) returns Result of i64, NE\n    transaction\n        insert stock 1\n        v is inner(ok)\n        insert stock 3\n    Ok(v)\n\n*main\n    match outer(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    log(count stock)\n    match outer(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    log(count stock)\n",
        "-1\n0\n2\n3",
    );
}

#[test]
fn trap_inside_transaction_rolls_back_on_disk_state() {
    let dir = tempfile::tempdir().unwrap();
    let writer = compile_in(
        dir.path(),
        "writer",
        "store ledger\n    amount as i64\n\n*main\n    insert ledger 1\n    transaction\n        insert ledger 10\n        insert ledger 20\n        x is 0\n        log(100 / x)\n",
    );
    let reader = compile_in(
        dir.path(),
        "reader",
        "store ledger\n    amount as i64\n\n*main\n    log(count ledger)\n",
    );

    let w = run_in(dir.path(), &writer);
    assert!(!w.status.success(), "writer should trap");
    assert!(
        String::from_utf8_lossy(&w.stderr).contains("integer division by zero"),
        "expected div-by-zero trap, got: {}",
        String::from_utf8_lossy(&w.stderr)
    );

    let r = run_in(dir.path(), &reader);
    assert!(
        r.status.success(),
        "reader failed: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&r.stdout).trim(),
        "1",
        "transaction writes must not survive a trap"
    );
}

#[test]
fn committed_transaction_survives_process_restart() {
    let dir = tempfile::tempdir().unwrap();
    let writer = compile_in(
        dir.path(),
        "writer",
        "store ledger\n    amount as i64\n\n*main\n    transaction\n        insert ledger 10\n        insert ledger 20\n    log(count ledger)\n",
    );
    let reader = compile_in(
        dir.path(),
        "reader",
        "store ledger\n    amount as i64\n\n*main\n    log(count ledger)\n    r is ledger where amount equals 20\n    log r.amount\n",
    );

    let w = run_in(dir.path(), &writer);
    assert!(
        w.status.success(),
        "writer failed: {}",
        String::from_utf8_lossy(&w.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&w.stdout).trim(), "2");

    let r = run_in(dir.path(), &reader);
    assert!(
        r.status.success(),
        "reader failed: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&r.stdout).trim(), "2\n20");
}

#[test]
fn rollback_then_retry_succeeds_cleanly() {
    expect(
        "err RErr\n    Fail\n    S(StoreError)\n\nimpl From of StoreError for RErr\n    *from(e as StoreError) returns RErr is S(e)\n\nstore jobs\n    pri as i64\n\n*enqueue(ok as bool) returns Result of i64, RErr\n    transaction\n        insert jobs 7\n        if not ok\n            err Fail\n    Ok(7)\n\n*main\n    match enqueue(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match enqueue(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match enqueue(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    log(count jobs)\n    r is jobs where pri equals 7\n    log r.pri\n",
        "-1\n-1\n7\n1\n7",
    );
}

/// Task 8-24 — transaction state is per-coroutine: two concurrent tasks
/// each in their own `transaction` (on their own stores) commit
/// independently; neither corrupts the other's tracking and neither
/// disables the other's durability. Before 8-24 the depth/list were
/// process-global with no lock.
#[test]
fn concurrent_transactions_commit_independently() {
    let src = "store alpha\n    v as i64\n\nstore beta\n    v as i64\n\n*fill_alpha()\n    transaction\n        for i in 0 to 50\n            insert alpha i\n\n*fill_beta()\n    transaction\n        for i in 0 to 80\n            insert beta i\n\n*main\n    together\n        dispatch\n            fill_alpha()\n        dispatch\n            fill_beta()\n    log(count alpha)\n    log(count beta)\n";
    expect(src, "50\n80");
}

/// Task 8-24 — one task's rollback cannot touch another task's
/// committed store.
#[test]
fn rollback_in_one_task_leaves_other_store_committed() {
    let src = "err OpErr\n    Boom\n    S(StoreError)\n\nimpl From of StoreError for OpErr\n    *from(e as StoreError) returns OpErr is S(e)\n\nstore good\n    v as i64\n\nstore bad\n    v as i64\n\n*fill_good()\n    transaction\n        for i in 0 to 30\n            insert good i\n\n*fill_bad() returns Result of i64, OpErr\n    transaction\n        for i in 0 to 30\n            insert bad i\n        err Boom\n    Ok(0)\n\n*main\n    together\n        dispatch\n            fill_good()\n        dispatch\n            match fill_bad()\n                Ok(v) ? log(v)\n                Err(e) ? log(0 - 1)\n    log(count good)\n    log(count bad)\n";
    let dir = tempfile::tempdir().unwrap();
    let bin = compile_in(dir.path(), "p", src);
    let out = run_in(dir.path(), &bin);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines.contains(&"30"), "good must commit fully: {stdout}");
    assert!(lines.contains(&"0"), "bad must roll back fully: {stdout}");
}
