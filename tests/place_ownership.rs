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
fn binding_a_borrowed_param_borrows_instead_of_double_freeing() {
    accepts_and_prints(
        "*peek(v as Vec of i64) returns i64\n    s is v\n    s.length\n\n*main\n    a is vec(1, 2, 3)\n    n is peek(a)\n    log(n)\n    log(a.length)\n",
        "3\n3",
    );
}

#[test]
fn two_borrowed_args_of_one_value_run_clean() {
    accepts_and_prints(
        "*both(x as Vec of i64, y as Vec of i64) returns i64\n    sx is x\n    sy is y\n    sx.length + sy.length\n\n*main\n    a is vec(1, 2, 3)\n    log(both(a, a))\n",
        "6",
    );
}

#[test]
fn take_from_a_borrowed_param_is_rejected() {
    rejects(
        "*grab(v as Vec of i64) returns i64\n    s is take v\n    s.length\n\n*main\n    a is vec(1)\n    log(grab(a))\n",
        &["cannot `take` from `v`", "borrow"],
    );
}

#[test]
fn rebinding_a_local_to_a_borrowed_param_runs_clean() {
    accepts_and_prints(
        "*shuffle(v as Vec of i64) returns i64\n    s is vec(9)\n    s is v\n    s.length\n\n*main\n    a is vec(1)\n    log(shuffle(a))\n    log(a.length)\n",
        "1\n1",
    );
}

#[test]
fn mutating_call_with_field_argument_is_rejected() {
    rejects(
        "type S\n    a as Vec of i64\n\n*app2(x as Vec of i64)\n    x.push(77)\n\n*main\n    s is S(a is vec(1))\n    app2(s.a)\n    log(s.a.length)\n",
        &[
            "mutates it through this parameter",
            "silently lost",
            "`s.a`",
        ],
    );
}

#[test]
fn mutating_method_on_nested_receiver_is_rejected() {
    rejects(
        "type Counter\n    n as i64\n\n    *bump\n        self.n is self.n + 1\n\ntype Holder\n    c as Counter\n\n*main\n    h is Holder(c is Counter(n is 0))\n    h.c.bump()\n    log(h.c.n)\n",
        &["mutates its receiver", "`h.c`", "copy"],
    );
}

#[test]
fn take_of_a_nested_place_is_rejected_not_a_segfault() {
    rejects(
        "type Inner\n    data as Vec of i64\n\ntype Outer\n    inner as Inner\n\n*main\n    o is Outer(inner is Inner(data is vec(1, 2, 3)))\n    x is take o.inner.data\n    log(x.length)\n",
        &["`take` of the nested place `o.inner.data`"],
    );
}

#[test]
fn ctor_capture_makes_a_param_consuming() {
    rejects(
        "type Box2\n    v as Vec of i64\n\n*wrap(v as Vec of i64) returns i64\n    b is Box2(v is v)\n    b.v.length\n\n*main\n    a is vec(1, 2, 3)\n    n is wrap(a)\n    log(n)\n    log(a.length)\n",
        &["use of moved value `a`", "`wrap`"],
    );
}

#[test]
fn method_that_stores_its_argument_is_consuming() {
    rejects(
        "type Sink\n    items as Vec of i64\n\n    *swallow(v as Vec of i64)\n        self.items is v\n\n*main\n    s is Sink(items is vec())\n    a is vec(1, 2, 3)\n    s.swallow(a)\n    log(a.length)\n",
        &["use of moved value `a`", "swallow"],
    );
}

#[test]
fn mutating_an_iterated_field_place_is_rejected() {
    rejects(
        "type S\n    items as Vec of i64\n\n*main\n    s is S(items is vec(1, 2, 3))\n    for x in s.items\n        s.items.push(9)\n    log(s.items.length)\n",
        &["cannot call `push` on `s.items`", "is iterating"],
    );
}

#[test]
fn moving_the_root_of_an_iterated_field_place_is_rejected() {
    rejects(
        "type S\n    items as Vec of i64\n\n*main\n    s is S(items is vec(1, 2, 3))\n    for x in s.items\n        w is s\n    log(1)\n",
        &["cannot move `s`", "is iterating", "`s.items`"],
    );
}

#[test]
fn sibling_field_mutation_during_iteration_is_allowed() {
    accepts_and_prints(
        "type S\n    a as Vec of i64\n    b as Vec of i64\n\n*main\n    s is S(a is vec(1, 2), b is vec())\n    for x in s.a\n        s.b.push(x)\n    log(s.b.length)\n",
        "2",
    );
}

#[test]
fn map_iteration_takes_a_borrow() {
    rejects(
        "*main\n    m is map()\n    m.set('a', 1)\n    m.set('b', 2)\n    for k, v in m\n        m.set('c', 9)\n    log(m.count)\n",
        &["cannot call `set` on `m`", "is iterating"],
    );
}

#[test]
fn iter_trait_loop_takes_a_borrow() {
    rejects(
        "type UpTo\n    cur as i64\n    max as i64\n\nimpl Iter for UpTo\n    *next(self) returns Option of i64\n        if self.cur >= self.max\n            return Nothing\n        v is self.cur\n        self.cur is self.cur + 1\n        Some(v)\n\n*main\n    u is UpTo(cur is 0, max is 3)\n    total is 0\n    for x in u\n        total is total + x\n        u.max is 10\n    log(total)\n",
        &["cannot assign to `u.max`", "is iterating"],
    );
}

#[test]
fn assigning_to_the_iterated_place_is_rejected() {
    rejects(
        "type S\n    items as Vec of i64\n\n*main\n    s is S(items is vec(1, 2))\n    for x in s.items\n        s.items is vec(9)\n    log(s.items.length)\n",
        &["cannot assign to `s.items`", "is iterating"],
    );
}

#[test]
fn reading_a_deeper_place_under_a_moved_field_is_rejected() {
    rejects(
        "type Inner\n    data as Vec of i64\n\ntype Outer\n    inner as Inner\n\n*main\n    o is Outer(inner is Inner(data is vec(1)))\n    x is o.inner\n    log(o.inner.data.length)\n",
        &["use of moved field"],
    );
}

#[test]
fn disjoint_field_reads_in_one_call_are_allowed() {
    accepts_and_prints(
        "type S\n    a as Vec of i64\n    b as Vec of i64\n\n*total(x as Vec of i64, y as Vec of i64) returns i64\n    x.length + y.length\n\n*main\n    s is S(a is vec(1), b is vec(2, 3))\n    log(total(s.a, s.b))\n",
        "3",
    );
}

#[test]
fn container_swap_idiom_stays_legal() {
    accepts_and_prints(
        "*main\n    v is vec(10, 20, 30)\n    v.set(0, v.get(2))\n    log(v.get(0))\n",
        "30",
    );
}

#[test]
fn move_while_iterating_a_variable_is_rejected() {
    rejects(
        "*main\n    v is vec(1, 2, 3)\n    for x in v\n        w is v\n    log(1)\n",
        &["cannot move `v`", "is iterating it"],
    );
}

#[test]
fn passing_a_variable_twice_to_a_mutating_call_is_rejected() {
    rejects(
        "*app(x as Vec of i64, y as Vec of i64)\n    x.push(1)\n\n*main\n    v is vec(1)\n    app(v, v)\n    log(v.length)\n",
        &["passed twice", "mutates it"],
    );
}

#[test]
fn heap_and_priority_queue_order_correctly() {
    accepts_and_prints(
        "use collections\n\n*main\n    h is collections.new_heap()\n    h.push(5)\n    h.push(1)\n    h.push(3)\n    log(h.pop())\n    log(h.pop())\n    log(h.pop())\n    q is collections.new_priority_queue()\n    q.push(9, 90)\n    q.push(2, 20)\n    q.push(5, 50)\n    e is q.pop()\n    log(e.value)\n",
        "1\n3\n5\n20",
    );
}
