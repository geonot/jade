use std::process::Command;

fn expect(source: &str, expected: &str) {
    for opt in ["0", "3"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.jn");
        let binary = dir.path().join("layout");
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
            "opt {opt}: {:?}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "opt {opt}"
        );
    }
}

#[test]
fn captured_closure_and_following_enum_field_do_not_overlap() {
    expect(
        "enum Action\n    Apply((i64) returns i64, i64)\n\n*run(action as Action) returns i64\n    match action\n        Apply(f, n) ? f(n)\n\n*main\n    base is 40\n    action is Apply(|x as i64| x + base, 2)\n    log(run(action))\n    log(run(action))\n",
        "42\n42\n",
    );
}

#[test]
fn aggregate_enum_payload_is_stored_as_value_not_address() {
    expect(
        "type Small\n    flag as i8\n    count as i64\n\nenum Envelope\n    Item(Small, String, i64)\n\n*read(e as Envelope)\n    match e\n        Item(value, text, tail) ?\n            log(value.flag)\n            log(value.count)\n            log(text)\n            log(tail)\n\n*main\n    e is Item(Small(7, 42), to_string(1234), 99)\n    read(e)\n    second is copy e\n    read(second)\n",
        "7\n42\n1234\n99\n7\n42\n1234\n99\n",
    );
}

#[test]
fn tuple_payload_padding_and_trailing_heap_field_are_consistent() {
    expect(
        "enum Packet\n    Data((i8, i64, i8), String)\n\n*read(p as Packet)\n    match p\n        Data(values, text) ?\n            (a, b, c) is values\n            log(a)\n            log(b)\n            log(c)\n            log(text)\n\n*main\n    p is Data((1 as i8, 42, 3 as i8), to_string(999))\n    read(p)\n    q is copy p\n    read(q)\n",
        "1\n42\n3\n999\n1\n42\n3\n999\n",
    );
}

#[test]
fn forward_nominal_layouts_and_aggregate_returns_are_resolved() {
    expect(
        "type Outer\n    inner as Inner\n    tail as i8\n\ntype Inner\n    value as i64\n    marker as i8\n\n*make returns Outer\n    Outer(Inner(42, 7), 9)\n\n*main\n    value is make()\n    log(value.inner.value)\n    log(value.inner.marker)\n    log(value.tail)\n",
        "42\n7\n9\n",
    );
}

#[test]
fn recursive_enum_slots_share_clone_drop_and_extraction_layout() {
    expect(
        "enum Chain\n    End\n    Link(String, Chain, i8)\n\n*show(c as Chain)\n    match c\n        End ? log('end')\n        Link(text, next, marker) ?\n            log(text)\n            log(marker)\n            show(next)\n\n*main\n    c is Link(to_string(11), Link(to_string(22), End(), 2), 1)\n    d is copy c\n    show(c)\n    show(d)\n",
        "11\n1\n22\n2\nend\n11\n1\n22\n2\nend\n",
    );
}

#[test]
fn aggregate_channel_elements_preserve_full_native_stride() {
    expect(
        "type Message\n    small as i8\n    text as String\n    last as i64\n\n*main\n    ch is channel of Message(2)\n    send ch, Message(7, to_string(42), 99)\n    send ch, Message(8, to_string(43), 100)\n    first is receive ch\n    second is receive ch\n    log(first.small)\n    log(first.text)\n    log(first.last)\n    log(second.small)\n    log(second.text)\n    log(second.last)\n",
        "7\n42\n99\n8\n43\n100\n",
    );
}

#[test]
fn nested_move_only_payloads_reject_copy_during_check_and_native_compile() {
    let declarations = "enum Action\n    Apply((i64) returns i64, i64)\n\ntype Wrapper\n    action as Action\n\nenum Chain\n    End\n    Link(Chain, Action)\n\n";
    for body in [
        "*main\n    base is 40\n    action is Apply(|x as i64| x + base, 2)\n    duplicate is copy action\n",
        "*main\n    base is 40\n    wrapped is Wrapper(Apply(|x as i64| x + base, 2))\n    duplicate is copy wrapped\n",
        "*main\n    base is 40\n    chain is Link(End(), Apply(|x as i64| x + base, 2))\n    duplicate is copy chain\n",
        "*consume(action as copy Action)\n    log(1)\n\n*main\n    log(2)\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("copy.jn");
        std::fs::write(&path, format!("{declarations}{body}")).unwrap();
        for check in [true, false] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_jinnc"));
            if check {
                cmd.arg("check");
            }
            let output = cmd.arg(&path).output().unwrap();
            assert!(
                !output.status.success(),
                "copy unexpectedly accepted: {body}"
            );
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("cannot `copy`") && error.contains("deep-copy is not defined"),
                "{error}"
            );
            assert!(
                !error.contains("panicked") && !error.contains("clone_value:"),
                "{error}"
            );
        }
    }
}
