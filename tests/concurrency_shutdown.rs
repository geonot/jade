//! End-to-end tests for Jinn's concurrency & shutdown semantics.
//!
//! These compile real `.jn` programs through `jinnc` and run them, asserting
//! deterministic output and clean exit. They are the executable backing for
//! the contract documented in `docs/concurrency.md`.
//!
//! The channel tests drive `send`/`receive`/`close` from `*main` (the
//! main-thread, non-coroutine context), arranged so no operation ever has to
//! park — making them fully deterministic and timing-independent. The actor
//! tests are liveness/regression guards: they prove a program with actors
//! shuts down cleanly (no hang, exit 0), including the historically tricky
//! cases of `stop` and of a daemon actor left parked on `receive`.

use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

/// Compile `src` to a temp binary, run it, and return its stdout. Panics with
/// a useful message if compilation or execution fails (non-zero exit).
fn compile_and_run(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    let out = dir.path().join("test_bin");
    std::fs::write(&jinn, src).unwrap();
    let status = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jinnc failed to start");
    assert!(status.success(), "jinnc compilation failed for:\n{src}");
    let output = Command::new(&out)
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        "binary exited with {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn expect(src: &str, expected: &str) {
    let got = compile_and_run(src);
    assert_eq!(got.trim(), expected.trim(), "source:\n{src}");
}

// ── Channel semantics ───────────────────────────────────────────────────

/// `send`/`receive` preserve FIFO order and lose nothing: enqueue 0..5 into a
/// channel with slack capacity, then drain it; the sum must be 0+1+2+3+4 = 10.
#[test]
fn channel_fifo_roundtrip() {
    expect(
        "\
*main
    ch is channel of i64(8)
    for i in 0 to 5
        send ch, i
    total is 0
    for i in 0 to 5
        v is receive ch
        total is total + v
    log(total)
",
        "10",
    );
}

/// A capacity-1 channel forces strict ping-pong: each `send` is immediately
/// followed by a `receive`, so the buffer is never full and nothing is lost.
/// Sum of 0..6 = 0+1+2+3+4+5 = 15.
#[test]
fn channel_capacity_one_interleaved() {
    expect(
        "\
*main
    ch is channel of i64(1)
    total is 0
    for i in 0 to 6
        send ch, i
        v is receive ch
        total is total + v
    log(total)
",
        "15",
    );
}

/// Every value enqueued before `close` is still delivered: send 0..8, close,
/// then receive 8 values. `receive` drains the buffer even after close. Sum of
/// 0..8 = 28.
#[test]
fn channel_receive_drains_buffer() {
    expect(
        "\
*main
    ch is channel of i64(16)
    for i in 0 to 8
        send ch, i
    close ch
    total is 0
    for i in 0 to 8
        v is receive ch
        total is total + v
    log(total)
",
        "28",
    );
}

// ── Actor shutdown ──────────────────────────────────────────────────────

/// A message actor: `*main` enqueues messages, then `stop`s the actor (closes
/// its mailbox), then returns. The program must shut down cleanly — the
/// scheduler joins its workers without hanging — and print `*main`'s output.
/// This is the regression guard for the historical actor-shutdown deadlock.
#[test]
fn actor_processes_then_stops() {
    expect(
        "\
actor Worker
    sum

    @work n
        sum is sum + n

*main
    w is spawn Worker
    w.work(10)
    w.work(32)
    stop w
    log('ok')
",
        "ok",
    );
}

/// A daemon actor left parked on `receive` (no `stop`) does NOT block program
/// exit: `jinn_sched_run` only waits for non-daemon coroutines, and the worker
/// loop abandons the parked daemon at shutdown. The program must still exit 0.
#[test]
fn actor_without_stop_still_exits() {
    expect(
        "\
actor Worker
    sum

    @work n
        sum is sum + n

*main
    w is spawn Worker
    w.work(10)
    log('ok')
",
        "ok",
    );
}
