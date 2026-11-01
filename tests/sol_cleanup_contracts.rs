use std::process::Command;

fn expect(source: &str, expected: &str) {
    for opt in ["0", "3"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cleanup.jn");
        let binary = dir.path().join("cleanup");
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
        let output = Command::new("timeout")
            .args(["15s"])
            .arg(&binary)
            .current_dir(dir.path())
            .env("JINN_WORKERS", "2")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "opt {opt}: {:?}: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "opt {opt}"
        );
    }
}

#[test]
fn only_reached_registrations_run_in_reverse_order() {
    expect(
        "*go(x as i64)\n    if x is 0\n        return\n    defer log(10)\n    if x is 1\n        defer log(11)\n    else\n        defer log(12)\n    log(x)\n\n*main\n    go(0)\n    go(1)\n    go(2)\n",
        "1\n11\n10\n2\n12\n10\n",
    );
}

#[test]
fn loops_register_zero_or_many_times_and_retain_iteration_binders() {
    expect(
        "*go(n as i64)\n    for i in 0 to n\n        if i is 1\n            continue\n        defer log(i)\n        if i is 3\n            break\n    log(90)\n\n*main\n    go(0)\n    go(5)\n",
        "90\n90\n3\n2\n0\n",
    );
}

#[test]
fn heap_backed_loop_locals_survive_until_function_exit() {
    expect(
        "*main\n    for i in 0 to 4\n        xs is [i, i + 10]\n        defer log(xs[0] + xs[1])\n        xs.push(100)\n    log('body')\n",
        "body\n16\n14\n12\n10\n",
    );
}

#[test]
fn cleanup_observes_exit_state_and_keeps_heap_live() {
    expect(
        "*main\n    xs is [1]\n    defer log(xs.len())\n    xs.push(2)\n    xs.push(3)\n    n is 1\n    defer log(n)\n    n is 9\n    log('body')\n",
        "body\n9\n3\n",
    );
}

#[test]
fn nested_cleanup_and_recursive_calls_have_independent_frames() {
    expect(
        "*go(n as i64)\n    defer\n        defer log(n + 20)\n        log(n + 10)\n    if n > 0\n        go(n - 1)\n    log(n)\n\n*main\n    go(2)\n",
        "0\n10\n20\n1\n11\n21\n2\n12\n22\n",
    );
}

#[test]
fn lambda_cleanup_runs_on_return_and_preserves_captures() {
    expect(
        "*main\n    base is 40\n    f is |x as i64| returns i64 do\n        defer log(base + x)\n        return x + 1\n    end\n    log(f(2))\n",
        "42\n3\n",
    );
}

#[test]
fn cancellation_runs_only_registered_loop_obligations_once() {
    expect(
        "*main\n    ready is channel of i64(1)\n    done is channel of i64(4)\n    together work\n        dispatch\n            for i in 0 to 3\n                xs is [i + 10]\n                defer send done, xs[0]\n            send ready, 1\n            while true\n                nop\n        receive ready\n        stop work\n    log(receive done)\n    log(receive done)\n    log(receive done)\n",
        "12\n11\n10\n",
    );
}

#[test]
fn return_value_is_evaluated_before_cleanup() {
    expect(
        "*go returns i64\n    n is 3\n    defer\n        n is 7\n        log(n)\n    return n\n\n*main\n    log(go())\n",
        "7\n3\n",
    );
}

#[test]
fn propagation_runs_reached_cleanup_exactly_once() {
    expect(
        "err Failure\n    Bad\n\n*fail returns i64 ! Failure\n    err Bad\n\n*go returns i64 ! Failure\n    defer log('outer')\n    for i in 0 to 2\n        defer log(i)\n    value is fail()\n    defer log('unreached')\n    value\n\n*main\n    go() ? log($) !! log('caught')\n",
        "1\n0\nouter\ncaught\n",
    );
}

#[test]
fn deferred_closure_and_string_survive_lexical_scope() {
    expect(
        "*main\n    if true\n        text is to_string(42)\n        f is |x as i64| x + 1\n        defer\n            log(text)\n            log(f(7))\n    log('body')\n",
        "body\n42\n8\n",
    );
}

#[test]
fn cleanup_in_value_blocks_is_discovered_before_early_return() {
    expect(
        "*go(n as i64) returns i64\n    if n is 0\n        return 1\n    value is if n > 0\n        defer log(n)\n        7\n    else\n        8\n    value\n\n*main\n    log(go(0))\n    log(go(2))\n",
        "1\n2\n7\n",
    );
}

#[test]
fn tuple_binders_are_retained_by_deferred_reads() {
    expect(
        "*main\n    if true\n        (a, b) is (to_string(21), to_string(22))\n        defer\n            log(a)\n            log(b)\n    log('body')\n",
        "body\n21\n22\n",
    );
}

#[test]
fn cleanup_nonlocal_control_is_rejected() {
    for source in [
        "*main\n    defer\n        return\n",
        "*go returns Vec of i64\n    xs is [42]\n    defer log(xs[0])\n    return xs\n\n*main\n    values is go()\n    log(values[0])\n",
        "*go returns Vec of i64\n    xs is [42]\n    defer log(xs[0])\n    xs\n\n*main\n    values is go()\n    log(values[0])\n",
        "*go(flag as bool) returns Vec of i64\n    xs is [42]\n    defer log(xs[0])\n    return flag ? xs ! [0]\n\n*main\n    values is go(true)\n    log(values[0])\n",
        "*go(flag as bool) returns Vec of i64\n    xs is [42]\n    defer log(xs[0])\n    if flag\n        xs\n    else\n        [0]\n\n*main\n    values is go(true)\n    log(values[0])\n",
        "*main\n    values is [40, 41, 42]\n    if true\n        window is values.view(1, 3)\n        defer log(window[1])\n    moved is take values\n    log(moved[0])\n",
        "*consume(xs as take Vec of i64)\n    log(xs[0])\n\n*main\n    xs is [42]\n    defer consume(xs)\n",
        "*main\n    for i in 0 to 2\n        defer\n            break\n",
        "*main\n    for i in 0 to 2\n        defer\n            continue\n",
        "err Failure\n    Bad\n\n*go returns i64 ! Failure\n    defer\n        err Bad\n    1\n\n*main\n    go()\n",
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
            String::from_utf8_lossy(&output.stderr).contains("defer"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn cleanup_local_loops_and_handled_errors_complete_normally() {
    expect(
        "err Failure\n    Bad\n\n*fail returns i64 ! Failure\n    err Bad\n\n*main\n    defer\n        for i in 0 to 4\n            if i is 1\n                continue\n            log(i)\n            if i is 2\n                break\n        fail() ? log($) !! log('handled')\n    log('body')\n",
        "body\n0\n2\nhandled\n",
    );
}

#[test]
fn actor_cleanup_reads_current_state_and_message_parameters() {
    expect(
        "actor Worker\n    total\n    @work n as i64\n        defer\n            log(total)\n            log(n)\n        total is total + n\n\n*main\n    w is spawn Worker\n    w.work(3)\n    w.work(4)\n    stop w\n    join w\n",
        "3\n3\n7\n4\n",
    );
}

#[test]
fn each_loop_retains_heap_elements_beyond_source_scope() {
    expect(
        "*main\n    if true\n        values is [to_string(10), to_string(11), to_string(12)]\n        for text in values\n            defer log(text)\n    log('body')\n",
        "body\n12\n11\n10\n",
    );
}

#[test]
fn match_payloads_are_retained_for_cleanup() {
    expect(
        "enum Message\n    Text(String)\n\n*main\n    if true\n        message is Text(to_string(42))\n        match message\n            Text(text) ?\n                defer log(text)\n    log('body')\n",
        "body\n42\n",
    );
}

#[test]
fn captured_aggregates_support_rebinding_and_index_updates() {
    expect(
        "*main\n    xs is [1, 2]\n    defer log(xs[0] + xs[1])\n    xs[0] is 10\n    xs is [20, 30]\n    xs[1] is 40\n    log('body')\n",
        "body\n60\n",
    );
}

#[test]
fn captured_structs_support_field_updates_and_heap_fields() {
    expect(
        "type Record\n    text as String\n    number as i64\n\n*main\n    if true\n        record is Record(text is to_string(42), number is 1)\n        defer\n            log(record.text)\n            log(record.number)\n        record.number is 7\n    log('body')\n",
        "body\n42\n7\n",
    );
}

#[test]
fn owned_and_borrowed_parameters_remain_live_through_cleanup() {
    expect(
        "*go(xs as Vec of i64, text as String)\n    defer\n        log(xs[0])\n        log(text)\n    log('body')\n\n*main\n    values is [42]\n    text is to_string(43)\n    go(values, text)\n",
        "body\n42\n43\n",
    );
}

#[test]
fn separate_lexical_binders_with_the_same_name_are_independent() {
    expect(
        "*main\n    if true\n        text is to_string(10)\n        defer log(text)\n    if true\n        text is to_string(20)\n        defer log(text)\n    log('body')\n",
        "body\n20\n10\n",
    );
}

#[test]
fn cleanup_inside_elif_value_blocks_is_discovered() {
    expect(
        "*go(n as i64) returns i64\n    if n is 0\n        return 1\n    value is if n > 2\n        2\n    elif n > 1\n        defer log(n)\n        3\n    else\n        4\n    value\n\n*main\n    log(go(0))\n    log(go(2))\n",
        "1\n2\n3\n",
    );
}

#[test]
fn cleanup_can_invoke_a_locally_returning_lambda() {
    expect(
        "*main\n    defer\n        f is |x as i64| returns i64 do\n            return x + 1\n        end\n        log(f(41))\n    log('body')\n",
        "body\n42\n",
    );
}

#[test]
fn normal_generator_exhaustion_runs_registered_cleanup() {
    expect(
        "*items\n    defer log('done')\n    for i in 0 to 3\n        defer log(i)\n        yield i\n\n*main\n    for n in items()\n        log(n + 10)\n",
        "10\n11\n12\n2\n1\n0\ndone\n",
    );
}

#[test]
fn deferred_consuming_calls_can_take_an_explicit_local_copy() {
    expect(
        "*consume(xs as take Vec of i64)\n    log(xs[0])\n\n*main\n    xs is [42]\n    defer\n        local is copy xs\n        consume(local)\n    log('body')\n",
        "body\n42\n",
    );
}

#[test]
fn deferred_moved_field_keeps_its_owner_live() {
    expect(
        "type Record\n    values as Vec of i64\n\n*main\n    if true\n        record is Record(values is [41, 42])\n        row is record.values\n        defer log(row[1])\n    log('body')\n",
        "body\n42\n",
    );
}

#[test]
fn deferred_view_keeps_its_transitive_source_owner_live() {
    expect(
        "*main\n    if true\n        values is [40, 41, 42]\n        window is values.view(1, 3)\n        alias is window\n        defer log(alias[1])\n    log('body')\n",
        "body\n42\n",
    );
}

#[test]
fn returned_explicit_copy_survives_cleanup_of_original_owner() {
    expect(
        "*go returns Vec of i64\n    xs is [42]\n    defer log(xs[0])\n    result is copy xs\n    return result\n\n*main\n    values is go()\n    log(values[0])\n",
        "42\n42\n",
    );
}

#[test]
fn lambdas_from_distinct_functions_have_distinct_code() {
    expect(
        "*adder(n as i64)\n    |x as i64| x + n\n\n*multiplier(n as i64)\n    |x as i64| x * n\n\n*main\n    add is adder(10)\n    multiply is multiplier(3)\n    log(add(7))\n    log(multiply(7))\n",
        "17\n21\n",
    );
}
