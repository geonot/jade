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

fn accepts_and_prints(src: &str, expected: &str) {
    let c = compile(src);
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(
        run.status.success(),
        "must run clean, got {:?}\nstderr: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        expected.trim(),
        "source:\n{src}"
    );
}

fn rejects(src: &str, needles: &[&str]) {
    let c = compile(src);
    assert!(!c.ok(), "must not compile:\n{src}");
    let stderr = c.stderr();
    for n in needles {
        assert!(stderr.contains(n), "missing {n:?} in:\n{stderr}");
    }
}

#[test]
fn scalar_captures_copy_at_creation() {
    accepts_and_prints(
        "*main\n    n is 10\n    addn is |x| x + n\n    n is 99\n    log(addn(5))\n    log(n)\n",
        "15\n99",
    );
}

#[test]
fn string_captures_clone_and_both_stay_live() {
    accepts_and_prints(
        "*main\n    name is 'ada'\n    greet is || name\n    log(greet())\n    log(name)\n",
        "ada\nada",
    );
}

#[test]
fn aggregate_capture_moves_and_later_use_is_rejected() {
    rejects(
        "*main\n    xs is vector(1, 2, 3)\n    total is || xs.sum()\n    xs.push(100)\n    log(total())\n",
        &[
            "used after being captured by the closure",
            "owns its environment",
        ],
    );
}

#[test]
fn capture_then_consuming_call_is_rejected_instead_of_segfaulting() {
    rejects(
        "*eat(v as take Vec of i64) returns i64\n    v.sum()\n\n*main\n    xs is vector(1, 2, 3)\n    total is || xs.sum()\n    log(eat(take xs))\n    log(total())\n",
        &["used after being captured by the closure"],
    );
}

#[test]
fn owned_environment_survives_repeated_calls() {
    accepts_and_prints(
        "*main\n    xs is vector(1, 2, 3)\n    total is || xs.sum()\n    log(total())\n    log(total())\n",
        "6\n6",
    );
}

#[test]
fn copy_first_keeps_the_original_usable() {
    accepts_and_prints(
        "*main\n    xs is vector(1, 2, 3)\n    snapshot is copy xs\n    total is || snapshot.sum()\n    xs.push(4)\n    log(total())\n    log(xs.sum())\n",
        "6\n10",
    );
}

#[test]
fn two_closures_cannot_capture_one_aggregate() {
    rejects(
        "*main\n    xs is vector(1, 2)\n    f is || xs.sum()\n    g is || xs.length\n    log(f() + g())\n",
        &["used after being captured by the closure"],
    );
}

#[test]
fn returning_a_closure_over_a_local_aggregate_works() {
    accepts_and_prints(
        "*make_counter() returns () returns i64\n    xs is vector(1, 2, 3)\n    || xs.sum()\n\n*main\n    f is make_counter()\n    log(f())\n    log(f())\n",
        "6\n6",
    );
}

#[test]
fn closure_assignment_moves_the_closure() {
    rejects(
        "*main\n    xs is vector(1, 2)\n    f is || xs.sum()\n    g is f\n    h is f\n    log(1)\n",
        &["use of moved value `f`"],
    );
}

#[test]
fn calling_a_moved_closure_is_rejected() {
    rejects(
        "*main\n    xs is vector(1, 2)\n    f is || xs.sum()\n    g is f\n    log(f())\n",
        &["cannot call `f`", "moves like any aggregate"],
    );
}

#[test]
fn closure_capturing_closure_moves_the_inner_one() {
    accepts_and_prints(
        "*main\n    xs is vector(1, 2, 3)\n    f is || xs.sum()\n    g is || f() + 1\n    log(g())\n",
        "7",
    );
}

#[test]
fn closure_moves_into_a_task_and_runs_there() {
    accepts_and_prints(
        "*main\n    xs is vector(1, 2, 3)\n    f is || xs.sum()\n    together\n        dispatch\n            log(f())\n",
        "6",
    );
}

#[test]
fn closure_used_after_task_capture_is_rejected() {
    rejects(
        "*main\n    xs is vector(1, 2)\n    f is || xs.sum()\n    together\n        dispatch\n            log(f())\n    log(f())\n",
        &["moved into"],
    );
}

#[test]
fn closure_cannot_capture_a_borrowed_parameter_aggregate() {
    rejects(
        "*peek(v as Vec of i64, n as i64) returns i64\n    m is n\n    f is || v.length\n    f()\n\n*main\n    xs is vector(1, 2)\n    log(peek(xs, 1))\n    log(xs.length)\n",
        &["a closure cannot capture `v`", "borrow"],
    );
}

#[test]
fn closure_cannot_capture_a_view() {
    rejects(
        "*scan(v as View of i64) returns i64\n    f is || v.length\n    f()\n\n*main\n    xs is vector(1, 2)\n    log(scan(xs))\n",
        &["is a view and cannot be captured"],
    );
}

#[test]
fn passing_a_closure_to_a_function_parameter_borrows_it() {
    accepts_and_prints(
        "*apply(f as (i64) returns i64, x as i64) returns i64\n    f(x)\n\n*main\n    n is 10\n    addn is |x| x + n\n    log(apply(addn, 5))\n    log(apply(addn, 6))\n",
        "15\n16",
    );
}

#[test]
fn string_capture_in_a_task_is_not_truncated() {
    accepts_and_prints(
        "*main\n    name is 'a long string that spills the sso buffer'\n    together\n        dispatch\n            log(name.length)\n",
        "40",
    );
}

#[test]
fn generator_arguments_are_consumed_at_creation() {
    accepts_and_prints(
        "*emit(xs as Vec of i64)\n    for x in xs\n        yield x\n\n*main\n    xs is vector(1, 2, 3)\n    g is emit(xs)\n    log(g.next())\n    log(g.next())\n    log(g.next())\n",
        "1\n2\n3",
    );
    rejects(
        "*emit(xs as Vec of i64)\n    for x in xs\n        yield x\n\n*main\n    xs is vector(1, 2, 3)\n    g is emit(xs)\n    log(g.next())\n    log(xs.length)\n",
        &["use of moved value `xs`", "emit"],
    );
    rejects(
        "*eat(v as take Vec of i64) returns i64\n    v.sum()\n\n*emit(xs as Vec of i64)\n    for x in xs\n        yield x\n\n*main\n    xs is vector(1, 2, 3)\n    g is emit(xs)\n    log(g.next())\n    log(eat(take xs))\n    log(g.next())\n",
        &["use of moved value `xs`"],
    );
}

#[test]
fn calling_through_a_function_parameter_taints_needs() {
    rejects(
        "*apply(f as (i64) returns i64, x as i64) returns i64 needs pure\n    f(x)\n\n*main\n    log(apply(|x| x + 1, 5))\n",
        &["a call through a function value"],
    );
    rejects(
        "*helper(f as (i64) returns i64) returns i64\n    f(1)\n\n*outer(g as (i64) returns i64) returns i64 needs pure\n    helper(g)\n\n*main\n    log(outer(|x| x))\n",
        &["a call through a function value", "outer -> helper"],
    );
    accepts_and_prints(
        "*apply(f as (i64) returns i64, x as i64) returns i64\n    f(x)\n\n*main\n    log(apply(|x| x + 1, 5))\n",
        "6",
    );
}

#[test]
fn frozen_payloads_to_actor_handlers_are_rejected_with_guidance() {
    rejects(
        "actor Sink\n    total as i64\n\n    @add v as Vec of i64\n        total is total + v.sum()\n\n*main\n    s is spawn Sink\n    xs is vector(1, 2)\n    fz is freeze xs\n    s.add(fz)\n    stop s\n    join s\n",
        &["cannot send a frozen value", "send a copy"],
    );
}

#[test]
fn loop_and_match_binders_are_capturable_by_closures_and_tasks() {
    accepts_and_prints(
        "*main\n    total is 0\n    for i in 0 to 3\n        f is |x| x + i\n        total is total + f(10)\n    log total\n",
        "33",
    );
    accepts_and_prints(
        "enum E\n    A(i64)\n    B\n\n*main\n    e is A(7)\n    match e\n        A(n) ?\n            f is |x| x + n\n            log f(1)\n        B ? log 0\n",
        "8",
    );
    accepts_and_prints_sorted(
        "*work(id as i64)\n    log(id)\n\n*main\n    base is 100\n    together\n        for i in 0 to 3\n            dispatch\n                work(base + i)\n",
        &["100", "101", "102"],
    );
    accepts_and_prints_sorted(
        "enum E\n    A(i64)\n    B\n\n*work(id as i64)\n    log(id)\n\n*main\n    e is A(7)\n    match e\n        A(n) ?\n            together\n                dispatch\n                    work(n)\n        B ? log 0\n",
        &["7"],
    );
}

fn accepts_and_prints_sorted(src: &str, expected: &[&str]) {
    let c = compile(src);
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(
        run.status.success(),
        "must run clean, got {:?}\nstderr: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let mut got: Vec<String> = String::from_utf8_lossy(&run.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    got.sort();
    let mut want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(got, want, "source:\n{src}");
}

#[test]
fn a_void_returning_lambda_lowers_to_a_void_return() {
    accepts_and_prints(
        "*apply(f as (i64) returns void, n as i64)\n    f(n)\n\n*main\n    apply(|x| log x, 5)\n",
        "5",
    );
}
