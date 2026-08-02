use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

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
        .current_dir(dir.path())
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
