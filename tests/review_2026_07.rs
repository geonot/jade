//! Pins the JINN_REVIEW_2026_07 repro programs (task 8-4).
//!
//! Each test asserts the **currently observed bad outcome** and carries a
//! `FIXME(8-N)` naming the task that owns the fix. When that task lands, the
//! test is flipped to assert the correct behavior and the marker is removed.
//! `fixme_markers_do_not_outlive_their_tasks` fails the suite if a marker is
//! still present after its task's `.ryu/tasks/8-N.task` is `status: complete`,
//! so a "fixed" task cannot leave its regression test asserting the bug.

use std::path::PathBuf;
use std::process::{Command, Output};

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

// ─── Memory (§3) ────────────────────────────────────────────────────────────

/// §3.1 — returning an aggregate parameter used to double-free
/// (`free(): invalid pointer`). Fixed by task 8-6: `ident`'s parameter is
/// inferred consuming (memory-model.md M6), the call moves `a`, and
/// exactly one drop fires.
#[test]
fn review_3_1_returning_vec_parameter_runs_clean() {
    let c = compile(
        "*ident(v) returns Vec of i64\n    return v\n\n*main\n    a is vec(1, 2, 3)\n    s is ident(a)\n    log(s.length)\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "must run clean: {}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n");
}

/// Companion to §3.1: using the argument after the consuming call is a
/// use-after-move compile error naming the callee and the copy-binding
/// escape hatch, and cloning first keeps both values alive (8-6).
#[test]
fn review_3_1_use_after_consuming_call_is_rejected() {
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

/// Companion to §3.1: an aggregate bind inside a nested scope transfers
/// the drop obligation; the outer scope must not drop again (8-6's
/// consumed-set recursion fix).
#[test]
fn review_3_1_nested_scope_bind_single_drop() {
    let c = compile(
        "*main\n    a is vec(1, 2, 3)\n    if true\n        b is a\n        log(b.length)\n    log(7)\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "must run clean: {}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n7\n");
}

/// §3.2 — two dispatch tasks mutating one Vec corrupted the allocator;
/// under M8 (task 8-8) the second capture is a compile error whose
/// diagnostic names the alternatives (per-task values over a channel, or
/// an actor owning the value) rather than reading as a bare limitation.
#[test]
fn review_3_2_cross_task_shared_vec_is_rejected() {
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

/// §3.3 — `b is a` on a Vec was silent shared mutable aliasing; under D1
/// (task 8-7) aggregates move on assignment, so the later read of `a` is
/// a compile error naming the move site and the `copy` escape hatch.
#[test]
fn review_3_3_vec_assignment_is_rejected_as_use_after_move() {
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
    // The escape hatch keeps both values, independently.
    let c = compile(
        "*main\n    a is vec(1,2,3)\n    b is copy a\n    b.push(4)\n    log(a.length)\n    log(b.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n4\n");
}

// ─── Typer (§4) ─────────────────────────────────────────────────────────────

/// §4.6 — cross-type `equals` (String vs i64) was accepted and
/// segfaulted; under 8-16 equality operands unify, so it is a compile
/// error.
#[test]
fn review_4_6_cross_type_equals_is_rejected() {
    let c = compile("*main\n    if 'abc' equals 5\n        log('huh')\n");
    assert!(!c.ok(), "cross-type equals must be a type error");
    assert!(c.stderr().contains("type mismatch"), "{}", c.stderr());
}

/// §4.6 — declared `returns String`, body returns i64: a source-level
/// diagnostic naming the function and its span (task 8-17). It used to
/// fall through to the MIR verifier and be reported as "a compiler bug".
#[test]
fn review_4_6_declared_string_returns_i64() {
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

/// §4.6 — `'abc' + 1` is rejected by the typer as a source-level
/// diagnostic (8-16 surfaced operand unification; 8-17 owns rendering).
#[test]
fn review_4_6_string_plus_int() {
    let c = compile("*main\n    x is 'abc' + 1\n    log(x)\n");
    assert!(!c.ok(), "must not compile");
    let stderr = c.stderr();
    // Caught by the TYPER since 8-16 surfaced operand-unification
    // failures (previously it fell through to hir-validate).
    assert!(
        stderr.contains("type mismatch") || stderr.contains("operator"),
        "{stderr}"
    );
}

/// §4.6 — heterogeneous `vec()` type-checked and read back a leaked
/// pointer as an integer; under 8-16 push arguments unify with the
/// element type and the mismatch is surfaced, not swallowed.
#[test]
fn review_4_6_heterogeneous_vec_is_rejected() {
    let c = compile("*main\n    v is vec()\n    v.push(1)\n    v.push('two')\n    log(v.get(1))\n");
    assert!(!c.ok(), "heterogeneous vec must be a type error");
    assert!(c.stderr().contains("type mismatch"), "{}", c.stderr());
}

/// §4.7 — multi-clause arity mismatch is a normal span-carrying
/// diagnostic with a non-zero exit (task 8-17). It used to be delivered
/// as a Rust panic — and the panic escaped the driver with EXIT CODE 0.
#[test]
fn review_4_7_multi_clause_arity_is_a_diagnostic() {
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

/// Found while pinning: top-level reassignment used to expand the
/// self-referential const until the compiler's stack overflowed. Now a
/// clean acyclicity diagnostic (task 8-17: no ICE on user input).
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

// ─── Store (§5.4) ───────────────────────────────────────────────────────────

/// §5.4 — `for u in all users` segfaults at runtime.
/// `all <store>` yields a first-class row set (task 8-25, first half):
/// a real Vec of the store's records — bindable, iterable, `.length`-able.
/// It used to be a bare pointer with no length, so iteration crashed.
#[test]
fn review_5_4_all_store_iteration_works() {
    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    for u in all users\n        log(u.name)\n    rows is all users\n    log(rows.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "Alice\nBob\n2\n");
}

/// §5.4 — a query that matched nothing used to fabricate a zero row
/// (`name=[] age=0`), indistinguishable from real data. A query is now
/// `Result of <row>, StoreError` (task 8-25, decision D3): hit and miss
/// are distinct values, the quaternary collapses them, and write-through
/// still works on a match-bound row.
#[test]
fn review_5_4_query_miss_is_a_value_not_a_zero_row() {
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

/// §5.4 — the original review repro is now a compile error, both at the
/// bare bind (an unhandled fallible value in a non-fallible function)
/// and at a direct field read off the query result.
#[test]
fn review_5_4_unhandled_query_is_a_compile_error() {
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

/// §5.4 — in a fallible function the bind needs no ceremony: the row
/// comes out unwrapped and a miss propagates as `Err(Missing)`.
#[test]
fn review_5_4_query_miss_propagates_in_fallible_fn() {
    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*find(n as String) returns i64 ! StoreError\n    r is users where name equals n\n    r.age\n\n*main\n    insert users 'Alice', 30\n    match find('Alice')\n        Ok(a) ? log(a)\n        Err(e) ? log('unexpected miss')\n    match find('Zed')\n        Ok(a) ? log(a)\n        Err(e) ? log('propagated')\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "30\npropagated\n");
}

// ─── Marker lifecycle ───────────────────────────────────────────────────────

/// A `FIXME(8-N)` marker asserting observed-bad behavior must not outlive
/// its owning task: once `.ryu/tasks/8-N.task` says `status: complete`, the
/// test above it must have been flipped and the marker removed.
#[test]
fn fixme_markers_do_not_outlive_their_tasks() {
    let this = repo_root().join("tests").join("review_2026_07.rs");
    let src = std::fs::read_to_string(&this).unwrap();
    let mut stale: Vec<String> = Vec::new();
    for (idx, line) in src.lines().enumerate() {
        let Some(pos) = line.find("FIXME(8-") else {
            continue;
        };
        let rest = &line[pos + "FIXME(".len()..];
        let Some(end) = rest.find(')') else { continue };
        let task_id = &rest[..end]; // e.g. "8-6"
        let task_file = repo_root()
            .join(".ryu")
            .join("tasks")
            .join(format!("{task_id}.task"));
        let status_complete = std::fs::read_to_string(&task_file)
            .map(|t| t.lines().any(|l| l.trim() == "status: complete"))
            .unwrap_or(false);
        if status_complete {
            stale.push(format!(
                "tests/review_2026_07.rs:{}: FIXME({task_id}) but {} is complete — \
                 flip the test to assert the fixed behavior and drop the marker",
                idx + 1,
                task_file.display()
            ));
        }
    }
    assert!(stale.is_empty(), "{}", stale.join("\n"));
}

/// Task 8-17 — every diagnostic-producing input exits non-zero (the
/// multi-clause arity panic used to exit 0), and none is delivered as a
/// Rust panic or raw LLVM IR.
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
