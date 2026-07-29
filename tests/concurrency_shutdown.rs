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

/// `send` is an expression yielding a `bool`: `true` when the value was
/// delivered, `false` when the channel was closed and the value dropped. A
/// send to an open channel reports `true`; after `close`, every send reports
/// `false` (and drops the value, so a subsequent drain sees nothing new).
#[test]
fn send_after_close_is_observable() {
    expect(
        "\
*main
    ch is channel of i64(8)
    before is send ch, 1
    close ch
    after is send ch, 2
    if before
        log(\"open:delivered\")
    else
        log(\"open:dropped\")
    if after
        log(\"closed:delivered\")
    else
        log(\"closed:dropped\")
    drained is 0
    for i in 0 to 4
        v is receive ch
        drained is drained + v
    log(drained)
",
        "open:delivered\nclosed:dropped\n1",
    );
}

/// A bare `send` statement remains fire-and-forget: its boolean result may be
/// ignored without ceremony, including after the channel is closed.
#[test]
fn bare_send_ignores_result() {
    expect(
        "\
*main
    ch is channel of i64(4)
    send ch, 7
    close ch
    send ch, 99
    v is receive ch
    log(v)
",
        "7",
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

/// `join` parks the caller until the target actor's mailbox is closed and its
/// handler loop has fully exited. After `stop` (which closes the mailbox and
/// lets the message actor drain) a `join` returns once the actor is done.
#[test]
fn join_after_stop_completes() {
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
    join w
    log('joined')
",
        "joined",
    );
}

/// `join` is idempotent: once the actor has exited, the completion latch stays
/// set, so a second `join` on the same actor returns immediately. The program
/// must not hang or deadlock.
#[test]
fn join_twice_is_idempotent() {
    expect(
        "\
actor Worker
    sum

    @work n
        sum is sum + n

*main
    w is spawn Worker
    w.work(5)
    stop w
    join w
    join w
    log('twice-ok')
",
        "twice-ok",
    );
}

/// `stop` is graceful **stop-and-drain**, not a hard kill — for a *loop
/// actor* too. The loop actor polls with `try_recv`, which keeps returning
/// buffered messages even after the mailbox is closed, and only reports
/// end-of-stream once the buffer is empty *and* closed. So every message
/// enqueued before `stop` is still dispatched. Here 1..9 are enqueued, then
/// `stop` + `join`; the running totals prove all eight were processed before
/// exit (final total 1+2+..+8 = 36).
#[test]
fn loop_actor_stop_drains_all_messages() {
    expect(
        "\
actor Counter
    total
    *loop 0
        nop
    *add n
        total is total + n
        log(total)

*main
    c is spawn Counter
    for i in 1 to 9
        c.add(i)
    stop c
    join c
    log('done')
",
        "1\n3\n6\n10\n15\n21\n28\n36\ndone",
    );
}

/// Cooperative preemption: a tight loop inside an actor handler does NOT
/// starve sibling actors. The compiler injects `jinn_sched_yield` at loop
/// back-edges in coroutine/actor contexts (the `inject_yields` MIR pass), so
/// the spinning `Spinner` keeps swapping back to its worker and the `Pinger`
/// gets to run. Both must complete and the program must exit cleanly (no
/// hang). Output order between the two actors is not asserted — only that all
/// three lines appear and the program terminates.
#[test]
fn tight_loop_actor_does_not_starve_siblings() {
    let out = compile_and_run(
        "\
actor Spinner
    acc
    *work n
        i is 0
        while i < n
            acc is acc + 1
            i is i + 1
        log('spinner-done')

actor Pinger
    *ping
        log('pinger-done')

*main
    s is spawn Spinner
    p is spawn Pinger
    s.work(5000000)
    p.ping()
    stop s
    stop p
    join s
    join p
    log('all-done')
",
    );
    assert!(out.contains("spinner-done"), "missing spinner-done:\n{out}");
    assert!(out.contains("pinger-done"), "missing pinger-done:\n{out}");
    assert!(out.contains("all-done"), "missing all-done:\n{out}");
}

// ── Structured concurrency: `together` scopes ───────────────────────────

/// `together` joins all child dispatches before control leaves the block.
/// Five anonymous `dispatch` tasks each send `1` on a channel; the code after
/// the scope drains exactly five values (sum 5), proving the scope waited for
/// every child to complete. No `usleep`, no manual join.
#[test]
fn scope_joins_all_dispatches() {
    expect(
        "\
*main
    done is channel of i64(16)
    together
        for i in 0 to 5
            dispatch
                send done, 1
    total is 0
    for i in 0 to 5
        total is total + receive done
    log(total)
",
        "5",
    );
}

/// A `spawn` inside a `together` is a *scope-owned* actor: non-daemon, its
/// mailbox auto-closed at block exit. Because a message actor drains before
/// exiting, every message sent before the block end is processed before
/// control passes the scope — first-class stop-and-drain with no ceremony.
#[test]
fn scope_owned_actor_drains_on_exit() {
    expect(
        "\
actor Counter
    total

    @add n
        total is total + n
        log(total)

*main
    together
        c is spawn Counter
        c.add(1)
        c.add(2)
        c.add(3)
    log('after-scope')
",
        "1\n3\n6\nafter-scope",
    );
}

/// `spawn` *outside* any scope is unchanged: a daemon, fire-and-forget actor
/// that does not block program exit. This is the backward-compatibility guard
/// — structured concurrency is strictly opt-in.
#[test]
fn daemon_spawn_outside_scope_unchanged() {
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

// ── Cancellation (task 2-6-4) ───────────────────────────────────────────

/// `stop <scope>` cancels a named `together` scope: a child task delivers a
/// fixed prefix (handshaking over `ready` so the test is timing-independent),
/// then would send forever. After the prefix is observed, `stop s` cancels the
/// task at its next channel operation, so the infinite tail never runs and the
/// program exits cleanly having consumed exactly the prefix.
#[test]
fn stop_scope_cancels_without_drain() {
    expect(
        "\
*main
    out is channel of i64(64)
    ready is channel of i64(1)
    together s
        dispatch
            for i in 0 to 4
                send out, i
            send ready, 1
            for j in 0 to 1000000
                send out, 99
        x is receive ready
        stop s
    count is 0
    while count < 4
        v is receive out
        count is count + 1
    log(count)
",
        "4",
    );
}

/// A `defer` inside a cancelled task still runs: cancellation unwinds through
/// the body's deferred cleanup before the task exits, exactly like an early
/// return.
#[test]
fn defer_runs_on_cancellation() {
    expect(
        "\
*main
    sig is channel of i64(1)
    together s
        dispatch
            defer log('cleaned')
            for i in 0 to 1000000
                send sig, i
        stop s
    log('done')
",
        "cleaned\ndone",
    );
}

/// Regression guard: a child that is *already parked* on a full channel when
/// the scope is cancelled must still observe the cancellation and unwind.
///
/// The `usleep` is load-bearing — it makes the child reach `send` and park
/// before `stop s` runs. Waking it goes through the global inject queue (the
/// waker is `*main`, not a worker), so the child is almost always resumed on a
/// *different* worker than the one it parked on. That migration used to hang:
/// the runtime cached the `tl_worker` TLS block address in a callee-saved
/// register, and `jinn_context_swap` restores those registers as part of the
/// coroutine context, so after migration `jinn_chan_send` kept reading the
/// original thread's worker. That worker is idle, its `current` is NULL, which
/// reads as "not on a coroutine" — so the send took the non-coroutine backoff
/// path and spun forever, never re-checking cancellation. Without the fix this
/// hangs ~90% of the time; see `jinn_worker_self` in runtime/jinn_rt.h.
#[test]
fn parked_sender_observes_cancellation_after_worker_migration() {
    expect(
        "\
extern *usleep(us as i32) returns i32

*main
    sig is channel of i64(1)
    together s
        dispatch
            defer log('cleaned')
            for i in 0 to 1000000
                send sig, i
        extern.usleep(20000)
        stop s
    log('done')
",
        "cleaned\ndone",
    );
}

// ── Scope error propagation (task 2-6-5) ────────────────────────────────

/// A child task that propagates an error records it on the scope; the failing
/// scope re-raises in the enclosing (fallible) function. `work` returns
/// `Result of i64, Boom`, the dispatched child raises `Bad`, and `main` matches
/// the surfaced `Err` — the scope-site re-raise carries the child's error out.
#[test]
fn failing_child_surfaces_error() {
    expect(
        "\
err Boom
    Bad

*risky() returns i64 ! Boom
    err Bad

*work() returns i64 ! Boom
    together
        dispatch
            x is risky() ? $ !! err
            log(1)
    0

*main()
    match work()
        Ok(v) ? log(v)
        Err(e) ? log(99)
",
        "99",
    );
}

/// A scope re-raises with `From` conversion (R1/R3): the child raises `Boom`,
/// but the enclosing function's error type is `AppError`, and an
/// `impl From of Boom for AppError` exists. The scope-site re-raise must apply
/// that conversion, surfacing the child's error wrapped as `AppError::Wrapped`.
#[test]
fn scope_error_propagates_with_from() {
    expect(
        "\
err Boom
    Bad

err Other
    Nope

err AppError
    Wrapped(Boom)
    Via(Other)

impl From of Boom for AppError
    *from(e as Boom) returns AppError
        Wrapped(e)

*risky() returns i64 ! Boom
    err Bad

*work() returns i64 ! AppError
    together
        dispatch
            x is risky() ? $ !! err
            log(1)
    0

*main()
    match work()
        Ok(v) ? log(v)
        Err(e) ? match e
            Wrapped(_) ? log(42)
            Via(_) ? log(7)
",
        "42",
    );
}

/// A `!!` handler arm on a `together` block (E4) handles the scope's error in
/// place instead of propagating it. `err` is bound to the surfaced error value;
/// here the handler logs `99` and the scope is considered handled, so the
/// enclosing function continues and returns `Ok(0)`.
#[test]
fn scope_err_handler_binds_err() {
    expect(
        "\
err Boom
    Bad

*risky() returns i64 ! Boom
    err Bad

*work() returns i64 ! Boom
    together
        dispatch
            x is risky() ? $ !! err
            log(1)
    !! log(99)
    0

*main()
    match work()
        Ok(v) ? log(v)
        Err(e) ? log(-1)
",
        "99\n0",
    );
}

/// A failing child cancels its siblings (E1): the failing task records the
/// error and cancels the scope, so a looping sibling unwinds at its next
/// suspension point instead of running to completion. The would-be million
/// iterations never finish; the error surfaces and the program exits cleanly.
#[test]
fn failing_child_cancels_siblings() {
    expect(
        "\
err Boom
    Bad

*risky() returns i64 ! Boom
    err Bad

*work() returns i64 ! Boom
    progress is channel of i64(64)
    together
        dispatch
            x is risky() ? $ !! err
            log(0)
        dispatch
            for i in 0 to 1000000
                send progress, i
    0

*main()
    match work()
        Ok(v) ? log(v)
        Err(e) ? log(99)
",
        "99",
    );
}

/// A `together` with both a `?` (success) arm and a `!!` (error) arm runs the
/// arms with quaternary semantics: the `?` arm fires only when the scope joined
/// cleanly, the `!!` arm only when a child failed — never both. Here the child
/// fails, so only `99` is logged (not `7`), and `work` returns `Ok(0)`.
#[test]
fn scope_quaternary_error_runs_only_err_arm() {
    expect(
        "\
err Boom
    Bad

*risky() returns i64 ! Boom
    err Bad

*work() returns i64 ! Boom
    together
        dispatch
            x is risky() ? $ !! err
            log(1)
    ? log(7)
    !! log(99)
    0

*main()
    match work()
        Ok(v) ? log(v)
        Err(e) ? log(-1)
",
        "99\n0",
    );
}

/// Mirror of the above on the success path: the child succeeds, the scope joins
/// cleanly, so only the `?` arm fires (logs `7`) and the `!!` arm is skipped.
#[test]
fn scope_quaternary_ok_runs_only_ok_arm() {
    expect(
        "\
err Boom
    Bad

*risky() returns i64 ! Boom
    42

*work() returns i64 ! Boom
    together
        dispatch
            x is risky() ? $ !! err
            log(1)
    ? log(7)
    !! log(99)
    0

*main()
    match work()
        Ok(v) ? log(v)
        Err(e) ? log(-1)
",
        "1\n7\n0",
    );
}
