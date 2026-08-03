use std::path::PathBuf;
use std::process::{Command, Output};

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

struct Compiled {
    dir: tempfile::TempDir,
    out: Output,
}

impl Compiled {
    fn ok(&self) -> bool {
        self.out.status.success()
    }
    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.out.stderr).into_owned()
    }
    fn run(&self) -> Output {
        Command::new(self.dir.path().join("prog.bin"))
            .current_dir(self.dir.path())
            .output()
            .expect("run compiled program")
    }
}

fn compile(src: &str) -> Compiled {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("prog.jn");
    std::fs::write(&file, src).unwrap();
    let out = Command::new(jinnc())
        .arg("prog.jn")
        .arg("-o")
        .arg(dir.path().join("prog.bin"))
        .current_dir(dir.path())
        .output()
        .expect("invoke jinnc");
    Compiled { dir, out }
}

fn exit_desc(o: &Output) -> String {
    format!(
        "status={:?} stdout={:?} stderr={:?}",
        o.status,
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn ownership_returning_vec_parameter_runs_clean() {
    let c = compile(
        "*ident(v) returns Vec of i64\n    return v\n\n*main\n    a is vec(1, 2, 3)\n    s is ident(a)\n    log(s.length)\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "must run clean: {}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n");
}

#[test]
fn ownership_use_after_consuming_call_is_rejected() {
    let c = compile(
        "*ident(v) returns Vec of i64\n    return v\n\n*main\n    a is vec(1, 2, 3)\n    s is ident(a)\n    log(a.length)\n",
    );
    assert!(!c.ok(), "use-after-move must not compile");
    let stderr = c.stderr();
    assert!(
        stderr.contains("moved value `a`") && stderr.contains("`ident`"),
        "diagnostic must name the move and the consuming callee: {stderr}"
    );

    let ok = compile(
        "*ident(v) returns Vec of i64\n    return v\n\n*main\n    a is vec(1, 2, 3)\n    a2 is copy a\n    s is ident(a2)\n    log(s.length)\n    log(a.length)\n",
    );
    assert!(
        ok.ok(),
        "copy-binding escape hatch must compile: {}",
        ok.stderr()
    );
    let run = ok.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n3\n");
}

#[test]
fn ownership_nested_scope_bind_single_drop() {
    let c = compile(
        "*main\n    a is vec(1, 2, 3)\n    if true\n        b is a\n        log(b.length)\n    log(7)\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "must run clean: {}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n7\n");
}

#[test]
fn ownership_cross_task_shared_vec_is_rejected() {
    let c = compile(
        "*pusher(v, base)\n    for i in 0 to 20000\n        v.push(base + i)\n\n*main\n    shared is vec()\n    together\n        dispatch\n            pusher(shared, 0)\n        dispatch\n            pusher(shared, 1000000)\n    log(shared.length)\n",
    );
    assert!(!c.ok(), "must be rejected under M8 (memory-model.md)");
    let stderr = c.stderr();
    assert!(
        stderr.contains("`shared` used after being moved into a concurrent task")
            && stderr.contains("channel")
            && stderr.contains("actor"),
        "diagnostic must name the message-passing alternatives: {stderr}"
    );
}

#[test]
fn ownership_vec_assignment_is_rejected_as_use_after_move() {
    let c = compile(
        "*main\n    a is vec(1,2,3)\n    b is a\n    b.push(4)\n    log(a.length)\n    log(b.length)\n",
    );
    assert!(!c.ok(), "must be rejected under D1 (memory-model.md M1)");
    let stderr = c.stderr();
    assert!(
        stderr.contains("use of moved value `a`")
            && stderr.contains("`b is a`")
            && stderr.contains("copy"),
        "diagnostic must name the move site and the copy escape hatch: {stderr}"
    );

    let c = compile(
        "*main\n    a is vec(1,2,3)\n    b is copy a\n    b.push(4)\n    log(a.length)\n    log(b.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n4\n");
}

#[test]
fn typing_cross_type_equals_is_rejected() {
    let c = compile("*main\n    if 'abc' equals 5\n        log('huh')\n");
    assert!(!c.ok(), "cross-type equals must be a type error");
    assert!(c.stderr().contains("type mismatch"), "{}", c.stderr());
}

#[test]
fn typing_declared_string_returns_i64() {
    let c = compile("*f(x as i64) returns String\n    x + 1\n\n*main\n    log(f(1))\n");
    assert!(!c.ok(), "must not compile");
    let stderr = c.stderr();
    assert!(
        stderr.contains("`f`") && stderr.contains("returns") && stderr.contains("type mismatch"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("compiler bug") && !stderr.contains("MIR verify"),
        "a plain source error must not carry ICE wording: {stderr}"
    );
}

#[test]
fn typing_string_plus_int() {
    let c = compile("*main\n    x is 'abc' + 1\n    log(x)\n");
    assert!(!c.ok(), "must not compile");
    let stderr = c.stderr();

    assert!(
        stderr.contains("type mismatch") || stderr.contains("operator"),
        "{stderr}"
    );
}

#[test]
fn typing_heterogeneous_vec_is_rejected() {
    let c = compile("*main\n    v is vec()\n    v.push(1)\n    v.push('two')\n    log(v.get(1))\n");
    assert!(!c.ok(), "heterogeneous vec must be a type error");
    assert!(c.stderr().contains("type mismatch"), "{}", c.stderr());
}

#[test]
fn typing_multi_clause_arity_is_a_diagnostic() {
    let c = compile("*f(0) is 0\n*f a, b is a + b\n\n*main\n    log(f(1, 2))\n");
    assert!(!c.ok(), "must not compile (and must exit non-zero)");
    let stderr = c.stderr();
    assert!(
        stderr.contains("multi-clause function `f`") && stderr.contains("parameters"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("RUST_BACKTRACE") && !stderr.contains("panicked"),
        "must not be a Rust panic: {stderr}"
    );
}

#[test]
fn top_level_reassignment_is_cleanly_diagnosed() {
    let c = compile("x is 41\nx is x + 1\nlog(x)\n");
    assert!(!c.ok(), "self-referential top-level const must be rejected");
    let stderr = c.stderr();
    assert!(stderr.contains("defined in terms of itself"), "{stderr}");
    assert!(
        !stderr.contains("stack overflow") && !stderr.contains("RUST_BACKTRACE"),
        "must be a diagnostic, not a crash: {stderr}"
    );
}

#[test]
fn store_all_iteration_works() {
    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    for u in all users\n        log(u.name)\n    rows is all users\n    log(rows.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "Alice\nBob\n2\n");
}

#[test]
fn store_query_miss_is_a_value_not_a_zero_row() {
    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    match users where name equals 'Alice'\n        Ok(r) ?\n            log(r.name)\n            log(r.age)\n        Err(e) ? log('hit expected')\n    match users where age > 100\n        Ok(r) ? log(r.name)\n        Err(e) ? log('miss is a miss')\n    q is users where age > 100 ? $.age ! 0 - 1\n    log(q)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "Alice\n30\nmiss is a miss\n-1\n"
    );
}

#[test]
fn store_unhandled_query_is_a_compile_error() {
    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*main\n    missing is users where age > 100\n    log('name=[{missing.name}] age={missing.age}')\n",
    );
    assert!(!c.ok(), "bare bind of a query must not compile");
    let stderr = c.stderr();
    assert!(
        stderr.contains("query result")
            || stderr.contains("propagat")
            || stderr.contains("fallible"),
        "{stderr}"
    );

    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*main\n    log((users where age > 100).age)\n",
    );
    assert!(!c.ok(), "field read on a query result must not compile");
    let stderr = c.stderr();
    assert!(stderr.contains("query result"), "{stderr}");
}

#[test]
fn store_query_miss_propagates_in_fallible_fn() {
    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*find(n as String) returns i64 ! StoreError\n    r is users where name equals n\n    r.age\n\n*main\n    insert users 'Alice', 30\n    match find('Alice')\n        Ok(a) ? log(a)\n        Err(e) ? log('unexpected miss')\n    match find('Zed')\n        Ok(a) ? log(a)\n        Err(e) ? log('propagated')\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "30\npropagated\n");
}

#[test]
fn every_diagnostic_input_exits_nonzero() {
    let bad = [
        "*f(0) is 0\n*f a, b is a + b\n\n*main\n    log(f(1, 2))\n",
        "x is 41\nx is x + 1\nlog(x)\n",
        "*f(x as i64) returns String\n    x + 1\n\n*main\n    log(f(1))\n",
        "*add(a as i64, b as i64) returns i64\n    a + b\n\n*main\n    log(add('one', 2))\n",
        "*main\n    if 'abc' equals 5\n        log('huh')\n",
        "*main\n    v is vec()\n    v.push(1)\n    v.push('two')\n",
        "*main\n    log(oops(((\n",
    ];
    for src in bad {
        let c = compile(src);
        assert!(!c.ok(), "must exit non-zero for:\n{src}");
        let stderr = c.stderr();
        assert!(
            !stderr.contains("RUST_BACKTRACE")
                && !stderr.contains("panicked at")
                && !stderr.contains("Call parameter type"),
            "diagnostic must not be a panic or raw IR for:\n{src}\n{stderr}"
        );
    }
}
