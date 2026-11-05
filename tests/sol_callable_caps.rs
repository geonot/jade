use std::process::Command;

fn check(source: &str, missing: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("caps.jn");
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_jinnc"))
        .arg(&path)
        .arg("--emit-hir")
        .current_dir(dir.path())
        .output()
        .unwrap();
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    if let Some(missing) = missing {
        assert!(!output.status.success(), "accepted {source}");
        assert!(diagnostic.contains(missing), "{diagnostic}");
        assert!(diagnostic.contains("caps.jn:"), "{diagnostic}");
    } else {
        assert!(output.status.success(), "{diagnostic}");
    }
}

fn run(source: &str, expected: &str) {
    for opt in ["0", "3"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("caps.jn");
        let binary = dir.path().join("caps");
        std::fs::write(&path, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_jinnc"))
            .arg(&path)
            .args(["--opt", opt, "-o"])
            .arg(&binary)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new("timeout")
            .arg("15s")
            .arg(&binary)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    }
}

#[test]
fn unrelated_method_with_same_spelling_does_not_pollute_pure_call() {
    run(
        "use io\n\ntype Quiet\n    n as i64\n    *hit returns i64\n        self.n\n\ntype Loud\n    n as i64\n    *hit returns i64\n        io.write_file('wrong.txt', 'x')\n        self.n\n\n*read(q as Quiet) returns i64 needs pure\n    q.hit()\n\n*main\n    q is Quiet(42)\n    log(read(q))\n",
        "42\n",
    );
}

#[test]
fn user_callable_shadowing_an_extern_keeps_both_callees_distinct() {
    run(
        "use io\n\ntype Quiet\n    n as i64\n    *hit returns i64\n        self.n\n\n*read(q as Quiet) returns i64\n    q.hit()\n\n*main\n    q is Quiet(42)\n    log(read(q))\n    bad is extern.write(-1, %'x', 1)\n    if bad < 0\n        log('extern-reached-libc')\n",
        "42\nextern-reached-libc\n",
    );
}

#[test]
fn receiver_named_like_std_module_does_not_gain_aperture_authority() {
    run(
        "type Local\n    n as i64\n    *write_file(path as String, data as String) returns i64\n        self.n\n\n*read(io as Local) returns i64 needs pure\n    io.write_file('x', 'y')\n\n*main\n    value is Local(42)\n    log(read(value))\n",
        "42\n",
    );
}

#[test]
fn instantiated_generic_uses_only_selected_method() {
    run(
        "use io\n\ntype Quiet\n    n as i64\n    *hit returns i64\n        self.n\n\ntype Loud\n    n as i64\n    *hit returns i64\n        io.write_file('wrong.txt', 'x')\n        self.n\n\n*read of T(q as T) returns i64 needs pure\n    q.hit()\n\n*main\n    q is Quiet(42)\n    log(read(q))\n",
        "42\n",
    );
}

#[test]
fn instantiated_generic_preserves_declared_capability_ceiling() {
    check(
        "use io\n\n*write of T(x as T) returns T needs pure\n    io.write_file('generic.txt', 'x')\n    x\n\n*main\n    log(write(42))\n",
        Some("fs.write 'generic.txt'"),
    );
}

#[test]
fn same_named_actor_handlers_are_disjoint() {
    run(
        "use io\n\nactor Quiet\n    n as i64\n    @hit\n        nop\n\nactor Loud\n    n as i64\n    @hit\n        io.write_file('wrong.txt', 'x')\n\n*send_quiet(q as Quiet) needs pure\n    q.hit()\n\n*main\n    q is spawn Quiet\n    send_quiet(q)\n    stop q\n    join q\n    log(42)\n",
        "42\n",
    );
}

#[test]
fn raw_extern_direct_call_cannot_escape_classification() {
    check(
        "extern *mystery(x as i64) returns i64\n\n*f returns i64 needs pure\n    mystery(1)\n\n*main\n    log(f())\n",
        Some("ffi.unsafe"),
    );
}

#[test]
fn classified_extern_direct_call_keeps_effect() {
    check(
        "extern *time(p as i64) returns i64\n\n*f returns i64 needs pure\n    time(0)\n\n*main\n    log(f())\n",
        Some("clock"),
    );
}

#[test]
fn benign_extern_is_still_capability_free() {
    run(
        "extern *abs(n as i32) returns i32\n\n*f returns i32 needs ffi.unsafe\n    extern.abs(-42 as i32)\n\n*main\n    log(f())\n",
        "42\n",
    );
    check(
        "extern *fabs(n as f64) returns f64\n\n*f returns f64 needs pure\n    extern.fabs(-1.0)\n\n*main\n    log(f())\n",
        None,
    );
}

#[test]
fn recursive_call_graph_propagates_effect_to_fixed_point() {
    check(
        "use io\n\n*a(n as i64) returns i64 needs pure\n    n > 0 ? b(n - 1) ! 0\n\n*b(n as i64) returns i64\n    n > 0 ? a(n - 1) ! write()\n\n*write returns i64\n    io.write_file('recursive.txt', 'x')\n    0\n\n*main\n    log(a(3))\n",
        Some("fs.write 'recursive.txt'"),
    );
}

#[test]
fn match_binding_cannot_hide_callable_effect() {
    check(
        "use io\n\nenum Action\n    Run(() returns bool)\n\n*write returns bool\n    io.write_file('match.txt', 'x')\n\n*invoke(a as Action) returns bool needs pure\n    match a\n        Run(f) ? f()\n\n*main\n    action is Run(write)\n    log(invoke(action))\n",
        Some("function value"),
    );
}

#[test]
fn default_argument_effect_is_visible_at_resolved_call() {
    check(
        "use io\n\n*write returns i64\n    io.write_file('default.txt', 'x')\n    1\n\n*value(n as i64 is write()) returns i64\n    n\n\n*f returns i64 needs pure\n    value()\n\n*main\n    log(f())\n",
        Some("fs.write 'default.txt'"),
    );
}

#[test]
fn unknown_capability_in_uninstantiated_generic_is_rejected() {
    check(
        "*f of T(x as T) returns T needs teleport\n    x\n\n*main\n    nop\n",
        Some("unknown capability"),
    );
}

#[test]
fn kv_method_cannot_hide_store_access() {
    check(
        "store counters @kv\n    key as String\n    val as i64\n\n*f needs pure\n    counters.set('x', 42)\n\n*main\n    f()\n",
        Some("./counters.store"),
    );
}

#[test]
fn store_relation_requires_target_store_capability() {
    check(
        "store authors\n    name as String\n\nstore posts\n    title as String\n    &author as authors\n\n*f needs fs.read './posts.store', fs.write './posts.store'\n    row is get posts 1\n    log(row.author.name)\n\n*main\n    f()\n",
        Some("./authors.store"),
    );
}

#[test]
fn implicit_display_call_propagates_user_method_effects() {
    check(
        "use io\n\ntype Value\n    n as i64\n    *display returns String\n        io.write_file('display.txt', 'x')\n        'value'\n\n*f(v as Value) needs pure\n    log(v)\n\n*main\n    v is Value(42)\n    f(v)\n",
        Some("fs.write 'display.txt'"),
    );
}

#[test]
fn method_needs_annotation_is_checked_after_resolution() {
    check(
        "use io\n\ntype Value\n    n as i64\n    *hit needs pure\n        io.write_file('method.txt', 'x')\n\n*main\n    v is Value(42)\n    v.hit()\n",
        Some("fs.write 'method.txt'"),
    );
}

#[test]
fn std_aperture_spelling_does_not_authorize_user_definition() {
    check(
        "extern *mystery() returns i64\n\n*io_write_file(path as String, data as String) returns i64\n    extern.mystery()\n\n*f returns i64 needs fs.write\n    io_write_file('out.txt', 'x')\n\n*main\n    log(f())\n",
        Some("ffi.unsafe"),
    );
}

#[test]
fn capability_free_arithmetic_runs() {
    run(
        "*add(a as i64, b as i64) returns i64 needs pure\n    a + b\n\n*main\n    log(add(19, 23))\n",
        "42\n",
    );
}

#[test]
fn dynamic_path_does_not_satisfy_scoped_permission() {
    check(
        "use io\n\n*f(path as String) needs fs.write './out'\n    io.write_file(path, 'x')\n\n*main\n    f('./out/file')\n",
        Some("uses `fs.write`"),
    );
}

#[test]
fn syscall_and_lambda_body_effects_are_checked() {
    check(
        "*f needs pure\n    syscall(39)\n\n*main\n    f()\n",
        Some("ffi.unsafe"),
    );
    check(
        "extern *connect(fd as i64, addr as i64, len as i64) returns i64\n\n*f needs pure\n    g is |x as i64| extern.connect(x, 0, 0)\n    g(1)\n\n*main\n    f()\n",
        Some("net.client"),
    );
}

#[test]
fn environment_mutation_requires_state() {
    check(
        "extern *setenv(k as String, v as String, overwrite as i32) returns i32\n\n*f returns i32 needs pure\n    extern.setenv('K', 'V', 1 as i32)\n\n*main\n    log(f())\n",
        Some("state <env>"),
    );
}

#[test]
fn unreachable_branch_is_checked_before_optimization() {
    check(
        "use io\n\n*f needs pure\n    if false\n        io.write_file('branch.txt', 'x')\n\n*main\n    f()\n",
        Some("fs.write 'branch.txt'"),
    );
}

#[test]
fn uninstantiated_generic_has_no_executable_capability_row() {
    check(
        "use io\n\n*f of T(x as T) returns T needs pure\n    io.write_file('unused.txt', 'x')\n    x\n\n*main\n    nop\n",
        None,
    );
}

#[test]
fn open_mode_and_literal_path_bound_the_aperture() {
    check(
        "use io\n\n*f needs fs.read './input.txt'\n    file is io.open('./input.txt', 'r')\n\n*main\n    nop\n",
        None,
    );
    check(
        "use io\n\n*f needs fs.read './input.txt'\n    file is io.open('./input.txt', 'w')\n\n*main\n    nop\n",
        Some("fs.write './input.txt'"),
    );
    check(
        "use io\n\n*f(mode as String) needs fs.read './input.txt'\n    file is io.open('./input.txt', mode)\n\n*main\n    nop\n",
        Some("fs.write './input.txt'"),
    );
}

#[test]
fn global_load_and_initialization_cannot_hide_effects() {
    check(
        "global counter is 0\n\n*f returns i64 needs pure\n    counter\n\n*main\n    log(f())\n",
        Some("state <counter>"),
    );
    check(
        "use io\n\nglobal ready is io.write_file('global.txt', 'x')\n\n*main needs pure\n    nop\n",
        Some("fs.write 'global.txt'"),
    );
}

#[test]
fn store_insert_hooks_propagate_effects_for_both_lowerings() {
    for statement in ["insert items 42", "result is insert items 42 ? $ !! 0"] {
        let source = format!(
            "use io\n\n*hook\n    io.write_file('hook.txt', 'x')\n\nstore items @simple @before_insert(hook)\n    n as i64\n\n*f needs fs.read './items.store', fs.write './items.store'\n    {statement}\n\n*main\n    f()\n"
        );
        check(&source, Some("fs.write 'hook.txt'"));
    }
}

#[test]
fn store_delete_cascades_and_child_hooks_are_checked() {
    for operation in ["delete", "destroy"] {
        let source = format!(
            "use io\n\n*hook\n    io.write_file('cascade.txt', 'x')\n\nstore owners\n    name as String\n\nstore pets @after_delete(hook)\n    name as String\n    &owner as owners @cascade\n\n*f needs fs.read './owners.store', fs.write './owners.store'\n    {operation} owners where name eq 'alice'\n\n*main\n    nop\n"
        );
        check(&source, Some("./pets.store"));
        check(
            &source.replace(
                "fs.write './owners.store'",
                "fs.write './owners.store', fs.read './pets.store', fs.write './pets.store'",
            ),
            Some("fs.write 'cascade.txt'"),
        );
    }
}

#[test]
fn store_restore_does_not_delete_children_or_invoke_delete_hooks() {
    run(
        "use io\n\n*hook\n    io.write_file('wrong.txt', 'x')\n\nstore owners @before_delete(hook)\n    name as String\n\nstore pets\n    name as String\n    &owner as owners @cascade\n\n*f needs fs.read './owners.store', fs.write './owners.store'\n    restore owners where name eq 'alice'\n\n*main\n    f()\n    log(42)\n",
        "42\n",
    );
}

#[test]
fn implicit_resource_drop_propagates_method_effects() {
    check(
        "use io\n\ntype Guard @resource\n    n as i64\n    *drop\n        io.write_file('drop.txt', 'x')\n\n*f needs pure\n    guard is Guard(42)\n    log(guard.n)\n\n*main\n    f()\n",
        Some("fs.write 'drop.txt'"),
    );
}

#[test]
fn nested_resource_drop_propagates_method_effects() {
    check(
        "use io\n\ntype Guard @resource\n    n as i64\n    *drop\n        io.write_file('drop.txt', 'x')\n\ntype Boxed\n    guard as Guard\n\n*f needs pure\n    value is Boxed(Guard(42))\n    log(value.guard.n)\n\n*main\n    f()\n",
        Some("fs.write 'drop.txt'"),
    );
}

#[test]
fn enum_formatting_does_not_call_payload_display() {
    run(
        "use io\n\ntype Value\n    n as i64\n    *display returns String\n        io.write_file('wrong.txt', 'x')\n        'value'\n\nenum Wrapped\n    ValueCase(Value)\n\n*f(v as Wrapped) needs pure\n    log(to_string(v))\n\n*main\n    f(ValueCase(Value(42)))\n",
        "ValueCase\n",
    );
}
