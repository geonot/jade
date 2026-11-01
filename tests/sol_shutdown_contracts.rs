use std::process::Command;

fn expect(source: &str, expected: &str) {
    for opt in ["0", "3"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shutdown.jn");
        let binary = dir.path().join("shutdown");
        std::fs::write(&path, source).unwrap();
        let compiled = Command::new(env!("CARGO_BIN_EXE_jinnc"))
            .arg(&path)
            .args(["--opt", opt, "-o"])
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "opt {opt}: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        for workers in ["1", "2", "4"] {
            let state = tempfile::tempdir().unwrap();
            let output = Command::new("timeout")
                .args(["20s"])
                .arg(&binary)
                .current_dir(state.path())
                .env("JINN_WORKERS", workers)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "opt {opt}, workers {workers}: {:?}: {}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                expected,
                "opt {opt}, workers {workers}"
            );
        }
    }
}

#[test]
fn helper_loop_cancellation_rolls_back_before_deferred_writes_and_releases_writer() {
    expect(
        "store ledger\n    value as i64\n\n*spin(ready, done) returns i64\n    defer send done, 7\n    transaction\n        insert ledger 10\n        send ready, 1\n        while true\n            nop\n    99\n\n*main\n    ready is channel of i64(1)\n    done is channel of i64(2)\n    together work\n        dispatch\n            value is spin(ready, done)\n            send done, value\n        receive ready\n        stop work\n    log(receive done)\n    log(count ledger)\n    transaction\n        insert ledger 42\n    log(count ledger)\n",
        "7\n0\n1\n",
    );
}

#[test]
fn cancelled_transactions_roll_back_before_deferred_transactions_begin() {
    expect(
        "store ledger\n    value as i64\n\n*record\n    transaction\n        insert ledger 7\n\n*spin(ready)\n    defer record()\n    transaction\n        insert ledger 10\n        send ready, 1\n        while true\n            nop\n\n*main\n    ready is channel of i64(1)\n    together work\n        dispatch\n            spin(ready)\n        receive ready\n        stop work\n    log(count ledger)\n    match ledger where value equals 7\n        Ok(row) ? log(row.value)\n        Err(error) ? log(-1)\n    transaction\n        insert ledger 42\n    log(count ledger)\n",
        "1\n7\n2\n",
    );
}

#[test]
fn opposite_multistore_acquisition_orders_are_serialized_without_deadlock() {
    expect(
        "store alpha\n    value as i64\n\nstore beta\n    value as i64\n\n*forward\n    for i in 0 to 12\n        transaction\n            insert alpha i\n            for j in 0 to 4\n                nop\n            insert beta i\n\n*reverse\n    for i in 0 to 12\n        transaction\n            insert beta i\n            for j in 0 to 4\n                nop\n            insert alpha i\n\n*main\n    together\n        dispatch\n            forward()\n        dispatch\n            reverse()\n    log(count alpha)\n    log(count beta)\n",
        "24\n24\n",
    );
}

#[test]
fn dropping_a_suspended_generator_runs_only_reached_cleanup() {
    expect(
        "*items\n    defer log('outer')\n    for i in 0 to 5\n        defer log(i)\n        yield i\n        log('resumed')\n\n*main\n    for value in items()\n        log(value + 10)\n        break\n    log('done')\n",
        "10\ndone\n0\nouter\n",
    );
}

#[test]
fn unstarted_generator_destruction_does_not_enter_user_body() {
    expect(
        "*items\n    log('body')\n    defer log('cleanup')\n    yield 1\n    nop\n\n*main\n    values is items()\n    log('done')\n",
        "done\n",
    );
}

#[test]
fn cancelled_helper_unwinds_nested_scopes_before_parent_cleanup() {
    expect(
        "*helper(ready, done)\n    defer send done, 2\n    together inner\n        dispatch\n            defer send done, 1\n            send ready, 1\n            while true\n                nop\n        while true\n            nop\n\n*main\n    ready is channel of i64(1)\n    done is channel of i64(2)\n    together work\n        dispatch\n            helper(ready, done)\n        receive ready\n        stop work\n    log(receive done)\n    log(receive done)\n",
        "1\n2\n",
    );
}

#[test]
fn generator_executing_inside_a_cancelled_task_unwinds_to_its_caller() {
    expect(
        "*items(ready, done)\n    defer send done, 1\n    send ready, 1\n    while true\n        nop\n    yield 1\n    nop\n\n*main\n    ready is channel of i64(1)\n    done is channel of i64(2)\n    together work\n        dispatch\n            defer send done, 2\n            for value in items(ready, done)\n                log(value)\n        receive ready\n        stop work\n    log(receive done)\n    log(receive done)\n",
        "1\n2\n",
    );
}

#[test]
fn moved_closures_and_owned_parameters_are_released_across_task_exits() {
    expect(
        "*use_it(f as take () returns i64, early as bool) returns i64\n    if early\n        return 1\n    f()\n\n*main\n    total is 0\n    for i in 0 to 300\n        xs is [i, 1]\n        f is || xs.sum()\n        total is total + use_it(take f, true)\n        ys is [i, 2]\n        g is || ys.sum()\n        together\n            dispatch\n                value is g()\n    log(total)\n",
        "300\n",
    );
}

#[test]
fn generator_owned_captures_survive_suspension_and_unstarted_destruction() {
    expect(
        "*items(xs as take Vec of i64)\n    yield xs[0]\n    yield xs[1]\n    nop\n\n*main\n    total is 0\n    for i in 0 to 300\n        unused is items([i, 1])\n        for value in items([i, 2])\n            total is total + value\n            break\n    log(total)\n",
        "44850\n",
    );
}

#[test]
fn cancelled_owned_parameters_release_branch_and_loop_values() {
    expect(
        "*work(xs as take Vec of String, ready, done)\n    defer send done, 1\n    text is to_string(1234567890123456789)\n    for i in 0 to 100\n        value is [text, text]\n        if i < 50\n            moved is take value\n    send ready, 1\n    while true\n        nop\n\n*main\n    for i in 0 to 30\n        ready is channel of i64(1)\n        done is channel of i64(1)\n        together running\n            dispatch\n                work([to_string(1234567890123456789)], ready, done)\n            receive ready\n            stop running\n        receive done\n    log('done')\n",
        "done\n",
    );
}

#[test]
fn closed_channels_drop_rejected_and_buffered_heap_payloads() {
    expect(
        "*main\n    for i in 0 to 300\n        values is channel of Vec of String(2)\n        send values, [to_string(1234567890123456789)]\n        close values\n        accepted is send values, [to_string(1234567890123456789)]\n        assert(not accepted)\n    log('done')\n",
        "done\n",
    );
}

#[test]
fn actor_stop_drains_owned_messages_and_repeated_join_is_safe() {
    expect(
        "actor Worker\n    @work xs as Vec of String\n        assert(xs.length equals 1)\n\n*main\n    for i in 0 to 80\n        w is spawn Worker\n        for j in 0 to 8\n            w.work([to_string(1234567890123456789)])\n        stop w\n        join w\n        join w\n        stop w\n    log('done')\n",
        "done\n",
    );
}

#[test]
fn rejected_actor_messages_and_heap_state_are_released() {
    expect(
        "actor Worker\n    words as Vec of String\n    @work xs as Vec of String\n        assert(xs.length equals 1)\n\n*main\n    for i in 0 to 100\n        w is spawn Worker(words is [to_string(1234567890123456789)])\n        stop w\n        join w\n        for j in 0 to 5\n            w.work([to_string(1234567890123456789)])\n    log('done')\n",
        "done\n",
    );
}

#[test]
fn owned_closures_in_generators_release_unstarted_and_suspended_environments() {
    expect(
        "*items(f as take () returns i64)\n    yield f()\n    yield f()\n    nop\n\n*main\n    total is 0\n    for i in 0 to 200\n        xs is [i, 1]\n        f is || xs.sum()\n        unused is items(take f)\n        ys is [i, 2]\n        g is || ys.sum()\n        for value in items(take g)\n            total is total + value\n            break\n    log(total)\n",
        "20300\n",
    );
}

#[test]
fn branch_loop_string_owners_and_borrowed_phi_inputs_are_released() {
    expect(
        "*trimmed(s as String, change as bool) returns i64\n    rest is s\n    if change\n        rest is rest.slice(1, rest.byte_count)\n    if rest.byte_count > 24\n        rest is rest.slice(1, rest.byte_count)\n    rest.byte_count\n\n*main\n    total is 0\n    for i in 0 to 300\n        text is to_string(1234567890123456789) + to_string(1234567890123456789)\n        total is total + trimmed(text, true) + trimmed(text, false)\n        out is ''\n        for j in 0 to 20\n            if j % 2 equals 0\n                out is out + text.slice(0, 1)\n            else\n                out is out + text.slice(1, 2) + text.slice(2, 3)\n        assert(out.byte_count equals 30)\n    log(total)\n",
        "21900\n",
    );
}

#[test]
fn selected_heap_payloads_are_typed_owned_and_released() {
    expect(
        "*main\n    for i in 0 to 300\n        texts is channel of String(2)\n        vectors is channel of Vec of String(2)\n        send texts, to_string(1234567890123456789)\n        select\n            receive texts as text\n                assert(text.length equals 19)\n            receive vectors as words\n                assert(false)\n        send vectors, [to_string(1234567890123456789)]\n        select\n            receive texts as text\n                assert(false)\n            receive vectors as words\n                assert(words.length equals 1)\n        send texts, to_string(1234567890123456789)\n        select\n            receive texts\n                nop\n    log('done')\n",
        "done\n",
    );
}

#[test]
fn cancelling_select_and_actor_queues_releases_owned_payloads() {
    expect(
        "actor Worker\n    @work xs as Vec of String\n        assert(xs.length equals 1)\n\n*main\n    for i in 0 to 200\n        texts is channel of String(2)\n        ready is channel of i64(1)\n        together work\n            w is spawn Worker\n            dispatch\n                send ready, 1\n                select\n                    receive texts as text\n                        assert(text.length equals 19)\n            receive ready\n            send texts, to_string(1234567890123456789)\n            for j in 0 to 8\n                w.work([to_string(1234567890123456789)])\n            stop work\n    log('done')\n",
        "done\n",
    );
}

#[test]
fn lexical_transaction_yield_and_consuming_closure_parameters_reject() {
    for (source, expected) in [
        (
            "*items\n    transaction\n        yield 1\n    nop\n\n*main\n    for value in items()\n        log(value)\n",
            "generator yield inside a transaction is unsupported",
        ),
        (
            "*main\n    f is |xs as take Vec of i64| xs.length\n    log(f([1]))\n",
            "consuming closure parameters are unsupported",
        ),
        (
            "*main\n    ch is channel of i64(1)\n    send ch, 1\n    select\n        receive ch as value\n            log(value)\n    log(value)\n",
            "undefined",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.jn");
        std::fs::write(&path, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_jinnc"))
            .arg("check")
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted: {source}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn generators_reject_retaining_borrowed_move_only_parameters() {
    for source in [
        "*items(f as const () returns i64)\n    yield f()\n    nop\n\n*main\n    xs is [1, 2]\n    f is || xs.sum()\n    for value in items(f)\n        log(value)\n",
        "*items(values as const Map of i64)\n    yield values.get('a')\n    nop\n\n*main\n    values as Map of i64 is map()\n    values.set('a', 7)\n    for value in items(values)\n        log(value)\n",
    ] {
        for opt in ["0", "3"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("invalid.jn");
            let binary = dir.path().join("invalid");
            std::fs::write(&path, source).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_jinnc"))
                .arg(&path)
                .args(["--opt", opt, "-o"])
                .arg(&binary)
                .output()
                .unwrap();
            assert!(!output.status.success(), "opt {opt}: accepted: {source}");
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("this type is move-only; transfer an owning value instead"),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!binary.exists());
        }
    }
}

#[test]
fn dynamically_blocking_transactions_fail_stop_instead_of_deadlocking() {
    for (source, expected) in [
        (
            "*wait(ch)\n    receive ch\n\n*main\n    ch is channel of i64(1)\n    transaction\n        wait(ch)\n",
            "blocking channel receive inside a transaction is unsupported",
        ),
        (
            "*wait(ch)\n    send ch, 2\n\n*main\n    ch is channel of i64(1)\n    send ch, 1\n    transaction\n        wait(ch)\n",
            "blocking channel send inside a transaction is unsupported",
        ),
        (
            "*items\n    yield 1\n    nop\n\n*main\n    transaction\n        for value in items()\n            log(value)\n",
            "generator resume inside a transaction is unsupported",
        ),
    ] {
        for opt in ["0", "3"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("blocking.jn");
            let binary = dir.path().join("blocking");
            std::fs::write(&path, source).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_jinnc"))
                .arg(&path)
                .args(["--opt", opt, "-o"])
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = Command::new("timeout")
                .arg("10s")
                .arg(&binary)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert_ne!(output.status.code(), Some(124));
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(expected),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
