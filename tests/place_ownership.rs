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

#[test]
fn consuming_call_inside_quaternary_arm_tombstones() {
    rejects(
        "*eat(v as Vec of i64) returns Vec of i64\n    v\n\n*find(f as bool) returns Option of i64\n    if f\n        return Some(1)\n    Nothing\n\n*main\n    v is vec(1, 2, 3)\n    o is find(true)\n    n is o ? eat(v).length !! 0\n    log(n)\n    log(v.length)\n",
        &["use of moved value `v`", "eat"],
    );
}

#[test]
fn consuming_call_in_both_ternary_arms_is_one_move() {
    accepts_and_prints(
        "*eat(v as Vec of i64) returns Vec of i64\n    v\n\n*main\n    v is vec(1, 2, 3)\n    c is true\n    n is c ? eat(v).length ! eat(v).length\n    log(n)\n",
        "3",
    );
}

#[test]
fn consuming_pipe_tombstones_instead_of_segfaulting() {
    rejects(
        "*eat(v as Vec of i64) returns Vec of i64\n    v\n\n*main\n    v is vec(1, 2, 3)\n    n is v ~ eat\n    log(n.length)\n    log(v.length)\n",
        &["use of moved value `v`", "eat"],
    );
}

#[test]
fn idiomatic_field_store_makes_method_consuming() {
    rejects(
        "type Sink\n    data as Vec of i64\n\n    *swallow(x as Vec of i64)\n        data is x\n\n*main\n    s is Sink(data is vec())\n    a is vec(1, 2, 3)\n    s.swallow(a)\n    log(a.length)\n",
        &["use of moved value `a`"],
    );
}

#[test]
fn idiomatic_field_write_makes_method_mutating() {
    rejects(
        "type Counter\n    total as i64\n\n    *bump(x as i64)\n        total is total + x\n\ntype Holder\n    c as Counter\n\n*main\n    h is Holder(c is Counter(total is 0))\n    h.c.bump(5)\n    log(h.c.total)\n",
        &["mutates its receiver", "nested place `h.c`"],
    );
}

#[test]
fn user_set_method_no_longer_matches_builtin_name_bucket() {
    accepts_and_prints(
        "type Gauge\n    n as i64\n\n    *set(x as Vec of i64)\n        n is x.length\n\n*stash(g as Gauge, x as Vec of i64)\n    g.set(x)\n\n*main\n    g is Gauge(n is 0)\n    v is vec(1, 2, 3)\n    stash(g, v)\n    log(v.length)\n",
        "3",
    );
}

#[test]
fn mutation_during_pending_defer_observes_exit_state() {
    accepts_and_prints(
        "*main\n    v is vec(1)\n    defer log(v.length)\n    i is 0\n    while i < 100\n        v.push(i)\n        i is i + 1\n    log('built')\n",
        "built\n101",
    );
}

#[test]
fn value_assertion_on_aggregate_struct_is_rejected() {
    rejects(
        "type Config @value\n    items as Vec of i64\n\n*main\n    log(1)\n",
        &["asserted `@value`", "field `items`", "aggregate"],
    );
}

#[test]
fn aggregate_assertion_on_value_struct_is_rejected() {
    rejects(
        "type Point @aggregate\n    x as i64\n    y as i64\n\n*main\n    log(1)\n",
        &["asserted `@aggregate`", "value"],
    );
}

#[test]
fn correct_category_assertions_are_accepted() {
    accepts_and_prints(
        "type Point @value\n    x as i64\n    y as i64\n\ntype Bag @aggregate\n    items as Vec of i64\n\n*main\n    p is Point(x is 1, y is 2)\n    q is p\n    b is Bag(items is vec(7))\n    log(p.x + q.y + b.items.length)\n",
        "4",
    );
}

#[test]
fn arena_generational_handles_end_to_end() {
    accepts_and_prints(
        "use arena\n\ntype Node\n    label as String\n    next as i64\n\n*main\n    a is Arena(slots is vec(Node(label is 'seed', next is -1)), gens is vec(0), live is vec(false), free is vec(0), count is 0)\n    h1 is a.insert(Node(label is 'one', next is -1))\n    h2 is a.insert(Node(label is 'two', next is -1))\n    log(a.get(h1).label)\n    ok is a.remove(h1)\n    if a.contains(h1)\n        log('bug')\n    else\n        log('dead')\n    h3 is a.insert(Node(label is 'three', next is -1))\n    log(a.get(h3).label)\n    log(a.get(h2).label)\n    log(a.size())\n",
        "one\ndead\nthree\ntwo\n2",
    );
}

#[test]
fn lib_compile_warns_on_inferred_consuming_boundary() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("prog.jn");
    std::fs::write(
        &file,
        "*eat(v as Vec of i64) returns Vec of i64\n    v\n\n*explicit(v as take Vec of i64) returns i64\n    v.length\n",
    )
    .unwrap();
    let out = Command::new(jinnc())
        .arg("prog.jn")
        .arg("--lib")
        .arg("--emit-obj")
        .arg("-o")
        .arg(dir.path().join("prog"))
        .current_dir(dir.path())
        .output()
        .expect("invoke jinnc");
    assert!(out.status.success(), "lib compile must succeed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("consumes its parameter `v` by inference"),
        "missing boundary warning in:\n{stderr}"
    );
    assert!(
        !stderr.contains("`explicit` consumes"),
        "explicit take must not warn:\n{stderr}"
    );
}

#[test]
fn match_payload_rewrap_returns_owned_value_and_runs_clean() {
    accepts_and_prints(
        "type Obj\n    keys as Vec of String\n\nenum JV\n    JNum(i64)\n    JObj(Obj)\n\n*set(obj as JV, key as String) returns JV\n    match obj\n        JObj(o) ?\n            o.keys.push(key)\n            JObj(o)\n        _ ? obj\n\n*main\n    o is JObj(Obj(keys is vec()))\n    o is set(o, 'k')\n    o is set(o, 'j')\n    match o\n        JObj(x) ? log(x.keys.length)\n        _ ? log(-1)\n",
        "2",
    );
}

#[test]
fn match_payload_string_return_moves_out_of_the_subject() {
    accepts_and_prints(
        "enum V\n    S(String)\n    N(i64)\n\n*unwrap(v as V) returns String\n    match v\n        S(s) ? s\n        N(n) ? 'none'\n\n*main\n    v is S('a-long-enough-string-to-defeat-sso-inline-storage')\n    log(unwrap(v))\n",
        "a-long-enough-string-to-defeat-sso-inline-storage",
    );
}

#[test]
fn subject_use_after_payload_consumed_is_rejected() {
    rejects(
        "type Obj\n    keys as Vec of String\n\nenum JV\n    JNum(i64)\n    JObj(Obj)\n\n*main\n    v is JObj(Obj(keys is vec()))\n    match v\n        JObj(o) ?\n            w is JObj(o)\n            log(w.length)\n        _ ? nop\n    match v\n        JObj(o2) ? log(o2.keys.length)\n        _ ? nop\n",
        &["use of moved value `v`", "moved into a constructor"],
    );
}

#[test]
fn consuming_call_in_if_condition_is_move_tracked() {
    rejects(
        "*eat(v as take Vec of i64) returns bool\n    v.length > 0\n\n*main\n    a is vec(1, 2, 3)\n    if eat(a)\n        log('ate')\n    log(a.length)\n",
        &["use of moved value `a`", "`eat`"],
    );
}

#[test]
fn consuming_call_in_elif_condition_is_move_tracked() {
    rejects(
        "*eat(v as take Vec of i64) returns bool\n    v.length > 9\n\n*main\n    a is vec(1, 2, 3)\n    if false\n        log('no')\n    elif eat(a)\n        log('yes')\n    log(a.length)\n",
        &["use of moved value `a`", "`eat`"],
    );
}

#[test]
fn consuming_call_in_while_condition_is_rejected() {
    rejects(
        "*eat(v as take Vec of i64) returns bool\n    v.length > 3\n\n*main\n    a is vec(1, 2, 3)\n    while eat(a)\n        log('spin')\n    log(a.length)\n",
        &["moved inside a loop"],
    );
}

#[test]
fn consuming_call_in_match_scrutinee_is_move_tracked() {
    rejects(
        "enum Size\n    Small\n    Big\n\n*eat(v as take Vec of i64) returns Size\n    v.length > 2 ? Size.Big ! Size.Small\n\n*main\n    a is vec(1, 2, 3)\n    match eat(a)\n        Big ? log('big')\n        _ ? log('small')\n    log(a.length)\n",
        &["use of moved value `a`", "`eat`"],
    );
}

#[test]
fn consuming_call_in_for_iter_is_move_tracked() {
    rejects(
        "*wrap(v as take Vec of i64) returns Vec of i64\n    v\n\n*main\n    a is vec(1, 2, 3)\n    for x in wrap(a)\n        log(x)\n    log(a.length)\n",
        &["use of moved value `a`", "`wrap`"],
    );
}

#[test]
fn multi_level_projection_bind_is_rejected_not_aliased() {
    rejects(
        "type Inner\n    items as Vec of i64\n\ntype Outer\n    inner as Inner\n\n*main\n    o is Outer(inner is Inner(items is vec(1, 2, 3)))\n    v is o.inner.items\n    v.push(99)\n    log(o.inner.items.length)\n",
        &["`take` of the nested place `o.inner.items`", "copy"],
    );
}

#[test]
fn unannotated_param_rebind_borrows_instead_of_minting_an_owner() {
    accepts_and_prints(
        "*f(v)\n    s is v\n    log(s.length)\n\n*main\n    a is vec(1, 2, 3)\n    f(a)\n    log(a.length)\n",
        "3\n3",
    );
}

#[test]
fn take_from_unannotated_borrowed_param_is_rejected() {
    rejects(
        "*f(v)\n    s is take v\n    log(s.length)\n\n*main\n    a is vec(1, 2, 3)\n    f(a)\n",
        &["cannot `take` from `v`", "borrow"],
    );
}

#[test]
fn dollar_outside_a_handler_arm_is_rejected() {
    rejects(
        "*main\n    x is $ + 1\n    log(x)\n",
        &["`$` has no value here"],
    );
}
