//! Conformance tests for `docs/memory-model.md` (decision D1).
//!
//! Each test is named for the M-rule it pins and follows the
//! `tests/access_semantics.rs` pattern: real programs through `jinnc`,
//! asserting exact runtime output or the diagnostic lead line. Rules whose
//! enforcement has not landed yet (M1/M3/M4 rejection — task 8-7, M8 —
//! task 8-8) are pinned as observed-bad in `tests/review_2026_07.rs`
//! instead, and their conformance tests are added here when they flip.

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
    fn run_stdout(&self) -> String {
        let out = Command::new(self.dir.path().join("prog.bin"))
            .current_dir(self.dir.path())
            .output()
            .expect("run compiled program");
        assert!(
            out.status.success(),
            "program must run clean: status={:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

fn compile(src: &str) -> Compiled {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("prog.jn"), src).unwrap();
    let out = Command::new(jinnc())
        .arg("prog.jn")
        .arg("-o")
        .arg(dir.path().join("prog.bin"))
        .current_dir(dir.path())
        .output()
        .expect("invoke jinnc");
    Compiled { dir, out }
}

/// M6 — a parameter returned by the callee is inferred consuming; the
/// §3.1 program compiles, runs clean, and drops the one buffer once.
#[test]
fn m6_consuming_param_inferred_from_return() {
    let c = compile(
        "*ident(v) returns Vec of i64\n    return v\n\n*main\n    a is vec(1, 2, 3)\n    s is ident(a)\n    log(s.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "3\n");
}

/// M6 — using the argument after the consuming call is rejected, and the
/// diagnostic names the callee (not a `take` the user never wrote).
#[test]
fn m6_use_after_consuming_call_names_callee() {
    let c = compile(
        "*ident(v) returns Vec of i64\n    return v\n\n*main\n    a is vec(1, 2, 3)\n    s is ident(a)\n    log(a.length)\n",
    );
    assert!(!c.ok());
    let stderr = c.stderr();
    assert!(
        stderr.contains("use of moved value `a`") && stderr.contains("`ident`"),
        "{stderr}"
    );
}

/// M6 — inference follows alias chains inside the callee
/// (`w is v; return w` consumes `v`).
#[test]
fn m6_alias_chain_in_callee_consumes() {
    let c = compile(
        "*second(v) returns Vec of i64\n    w is v\n    return w\n\n*main\n    a is vec(1, 2, 3)\n    s is second(a)\n    log(s.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "3\n");
}

/// M6 — inference is transitive across calls: `f` forwards its parameter
/// to consuming `g`, so `f` is consuming too and the whole chain is
/// single-drop.
#[test]
fn m6_transitive_consuming_through_forwarding() {
    let c = compile(
        "*g(v) returns Vec of i64\n    return v\n\n*f(v) returns Vec of i64\n    return g(v)\n\n*main\n    a is vec(4, 5)\n    s is f(a)\n    log(s.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "2\n");
}

/// M6 — a borrowing helper stays borrowing: the caller keeps ownership
/// and sees in-place mutation (unchanged behavior, access-semantics §6.1).
#[test]
fn m6_non_escaping_param_still_borrows() {
    let c = compile(
        "*push_one(v)\n    v.push(1)\n\n*main\n    xs is vec(1, 2, 3)\n    push_one(xs)\n    log(xs.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "4\n");
}

/// M6 escape hatch — an explicit `copy` binding before the consuming call
/// keeps the original usable.
#[test]
fn m6_copy_binding_escape_hatch() {
    let c = compile(
        "*ident(v) returns Vec of i64\n    return v\n\n*main\n    a is vec(1, 2, 3)\n    a2 is copy a\n    s is ident(a2)\n    log(s.length)\n    log(a.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "3\n3\n");
}

/// M7 — returning a local transfers ownership to the caller (one drop).
#[test]
fn m7_return_local_transfers_ownership() {
    let c = compile(
        "*make() returns Vec of i64\n    m is vec(9, 8)\n    return m\n\n*main\n    s is make()\n    log(s.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "2\n");
}

/// Nested-scope aggregate binds transfer the drop obligation out of the
/// inner scope; no double free (8-6 consumed-set recursion).
#[test]
fn nested_scope_bind_transfers_drop() {
    let c = compile(
        "*main\n    a is vec(1, 2, 3)\n    if true\n        b is a\n        log(b.length)\n    log(7)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "3\n7\n");
}

/// The recursive real-world shape the review called out: merge sort built
/// from consuming helpers sorts correctly with no allocator abort.
#[test]
fn m6_merge_sort_shape_runs_clean() {
    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("snippets/101-200/s107.jn"),
    )
    .expect("s107.jn");
    let c = compile(&src);
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "1\n2\n3\n4\n5\n6\n7\n8\n9\n");
}

/// M1 — aggregates move on assignment: the §3.3 program is rejected with
/// the D1 diagnostic naming the move site and the `copy` escape hatch.
#[test]
fn m1_move_on_assign_rejects_use_of_source() {
    let c = compile("*main\n    a is vec(1, 2, 3)\n    b is a\n    log(a.length)\n");
    assert!(!c.ok());
    let stderr = c.stderr();
    assert!(
        stderr.contains("use of moved value `a`")
            && stderr.contains("`b is a`")
            && stderr.contains("copy"),
        "{stderr}"
    );
}

/// M1 — the moved-to binding owns the one buffer; the program runs clean.
#[test]
fn m1_move_on_assign_new_owner_runs_clean() {
    let c = compile("*main\n    a is vec(1, 2, 3)\n    b is a\n    b.push(4)\n    log(b.length)\n");
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "4\n");
}

/// M2 — reassignment revives the tombstone (identical to the `take` rule).
#[test]
fn m2_reassignment_revives() {
    let c = compile(
        "*main\n    a is vec(1)\n    b is a\n    a is vec(2, 3)\n    log(a.length)\n    log(b.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "2\n1\n");
}

/// M1 + branches — a move on any branch tombstones after the join
/// (union merge), so the later read is rejected even with no else.
#[test]
fn m1_move_in_branch_rejected_after_join() {
    let c = compile(
        "*main\n    a is vec(1)\n    x is 1\n    if x > 0\n        b is a\n        log(b.length)\n    log(a.length)\n",
    );
    assert!(!c.ok());
    assert!(
        c.stderr().contains("use of moved value `a`"),
        "{}",
        c.stderr()
    );
}

/// M1 + loops — an aggregate assignment inside a loop body would re-move
/// the tombstone on the next iteration; rejected by the loop check
/// (including `while`, which previously skipped it).
#[test]
fn m1_move_in_while_loop_rejected() {
    let c =
        compile("*main\n    a is vec(1)\n    while true\n        b is a\n        log(b.length)\n");
    assert!(!c.ok());
    assert!(
        c.stderr().contains("moved inside a loop body"),
        "{}",
        c.stderr()
    );
}

/// M3 — a plain bind of an aggregate struct field is a partial move, as
/// `take` is: the field read is rejected, siblings stay readable.
#[test]
fn m3_field_bind_is_partial_move() {
    let src_bad = "type Bag\n    items as Vec of i64\n    label as String\n\n*main\n    b is Bag(items is vec(1), label is 'x')\n    v is b.items\n    log(b.items.length)\n";
    let c = compile(src_bad);
    assert!(!c.ok());
    assert!(c.stderr().contains("use of moved field"), "{}", c.stderr());

    let src_ok = "type Bag\n    items as Vec of i64\n    label as String\n\n*main\n    b is Bag(items is vec(1), label is 'x')\n    v is b.items\n    log(b.label)\n    log(v.length)\n";
    let c = compile(src_ok);
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "x\n1\n");
}

/// M4 — binding an aggregate container element is rejected (it would
/// alias the container's memory); `copy` and `take` are the escapes.
#[test]
fn m4_aggregate_element_bind_rejected_with_escapes() {
    let c = compile(
        "*main\n    grid is vec()\n    grid.push(vec(1, 2))\n    row is grid.get(0)\n    log(row.length)\n",
    );
    assert!(!c.ok());
    assert!(
        c.stderr().contains("cannot bind aggregate element"),
        "{}",
        c.stderr()
    );

    for escape in ["copy", "take"] {
        let c = compile(&format!(
            "*main\n    grid is vec()\n    grid.push(vec(1, 2))\n    row is {escape} grid.get(0)\n    log(row.length)\n"
        ));
        assert!(c.ok(), "{escape}: {}", c.stderr());
        assert_eq!(c.run_stdout(), "2\n", "{escape}");
    }
}

/// M4 — scalar elements bind freely (they copy).
#[test]
fn m4_scalar_element_bind_is_legal() {
    let c = compile("*main\n    nums is vec(7, 8)\n    x is nums.get(1)\n    log(x)\n");
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "8\n");
}

/// M9 — `send ch, v` moves an aggregate; the sender's later read is
/// rejected with a diagnostic naming the send.
#[test]
fn m9_channel_send_tombstones_sender() {
    let c = compile(
        "*main\n    ch is channel of Vec of i64(4)\n    v is vec(1, 2)\n    send ch, v\n    log(v.length)\n",
    );
    assert!(!c.ok());
    let stderr = c.stderr();
    assert!(
        stderr.contains("use of moved value `v`") && stderr.contains("sent on a channel"),
        "{stderr}"
    );
}

/// M10 — moving a value that a registered `defer` reads is rejected; the
/// same defer with no later move runs after the scope body.
#[test]
fn m10_defer_read_blocks_move() {
    let c = compile(
        "*main\n    buf is vec(1, 2)\n    defer\n        log(buf.length)\n    b is buf\n    log(b.length)\n",
    );
    assert!(!c.ok());
    let stderr = c.stderr();
    assert!(
        stderr.contains("cannot move `buf`") && stderr.contains("defer"),
        "{stderr}"
    );

    let c =
        compile("*main\n    buf is vec(1, 2)\n    defer\n        log(buf.length)\n    log(7)\n");
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "7\n2\n");
}

/// The ported return-of-borrowed check (formerly src/ownership): a
/// function returning `%local` is rejected — the pointee dies with the
/// frame.
#[test]
fn return_of_reference_to_local_rejected() {
    let c = compile("*f() returns %i64\n    x is 5\n    return %x\n\n*main\n    log(1)\n");
    assert!(!c.ok());
    assert!(
        c.stderr().contains("returning reference to local variable"),
        "{}",
        c.stderr()
    );
}

/// M8 — the §3.2 race: two dispatch tasks capturing one Vec is a
/// use-after-move at the second capture, with the actionable diagnostic.
#[test]
fn m8_second_task_capture_rejected() {
    let c = compile(
        "*pusher(v, base)\n    for i in 0 to 100\n        v.push(base + i)\n\n*main\n    shared is vec()\n    together\n        dispatch\n            pusher(shared, 0)\n        dispatch\n            pusher(shared, 1000)\n",
    );
    assert!(!c.ok());
    let stderr = c.stderr();
    assert!(
        stderr.contains("`shared` used after being moved into a concurrent task")
            && stderr.contains("channel")
            && stderr.contains("actor"),
        "{stderr}"
    );
}

/// M8 — any later use in the parent after a single task capture is also
/// a use-after-move.
#[test]
fn m8_parent_use_after_capture_rejected() {
    let c = compile(
        "*pusher(v, base)\n    for i in 0 to 100\n        v.push(base + i)\n\n*main\n    shared is vec()\n    together\n        dispatch\n            pusher(shared, 0)\n    log(shared.length)\n",
    );
    assert!(!c.ok());
    assert!(
        c.stderr()
            .contains("`shared` used after being moved into a concurrent task"),
        "{}",
        c.stderr()
    );
}

/// M8 — the equivalent correct program: per-task vectors merged over a
/// channel compiles and runs.
#[test]
fn m8_per_task_vectors_merged_over_channel_runs() {
    let c = compile(
        "*fill(v, base as i64, ch)\n    for i in 0 to 100\n        v.push(base + i)\n    send ch, v\n\n*main\n    ch is channel of Vec of i64(4)\n    together\n        dispatch\n            a is vec()\n            fill(a, 0, ch)\n        dispatch\n            b is vec()\n            fill(b, 1000, ch)\n    x is receive ch\n    y is receive ch\n    log(x.length + y.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "200\n");
}

/// M8 — the other correct shape: a single actor owns the aggregate and
/// receives messages.
#[test]
fn m8_actor_owned_aggregate_runs() {
    let c = compile(
        "actor Collector\n    items as Vec of i64\n\n    @add n as i64\n        items.push(n)\n\n*main\n    c is spawn Collector(items is vec())\n    c.add(1)\n    c.add(2)\n    stop c\n    join c\n    log(9)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "9\n");
}

/// M8 — `sim for` iterations are each a task, so capturing an outer
/// aggregate is a hard error naming the alternatives.
#[test]
fn m8_sim_for_capture_rejected() {
    let c =
        compile("*main\n    shared is vec()\n    sim for i in 0 to 4\n        shared.push(i)\n");
    assert!(!c.ok());
    assert!(
        c.stderr().contains("cannot be captured by `sim for`"),
        "{}",
        c.stderr()
    );
}

/// M8 — an actor message payload moves into the actor's task.
#[test]
fn m8_actor_payload_moves() {
    let c = compile(
        "actor Sink\n    total as i64\n\n    @eat v as Vec of i64\n        total is total + v.length\n\n*main\n    s is spawn Sink(total is 0)\n    v is vec(1, 2)\n    s.eat(v)\n    log(v.length)\n",
    );
    assert!(!c.ok());
    assert!(
        c.stderr()
            .contains("`v` used after being moved into a concurrent task"),
        "{}",
        c.stderr()
    );
}

/// M8 — a `spawn` initializer from a variable moves the aggregate into
/// the actor.
#[test]
fn m8_spawn_initializer_moves() {
    let c = compile(
        "actor Holder\n    items as Vec of i64\n\n    @noop n as i64\n        items.push(n)\n\n*main\n    v is vec(1)\n    h is spawn Holder(items is v)\n    log(v.length)\n",
    );
    assert!(!c.ok());
    assert!(
        c.stderr()
            .contains("`v` used after being moved into a concurrent task"),
        "{}",
        c.stderr()
    );
}

/// M8 — a `copy` capture shares a snapshot legally; the parent's value
/// is untouched.
#[test]
fn m8_copy_capture_is_legal() {
    let c = compile(
        "*consume(v)\n    log(v.length)\n\n*main\n    shared is vec(1, 2)\n    together\n        dispatch\n            c is copy shared\n            consume(c)\n    log(shared.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "2\n2\n");
}

/// M2 — consume-and-rebind is the canonical builder idiom: passing an
/// aggregate to a consuming call and rebinding the result to the same
/// name re-initializes the binding, so the next iteration is legal.
/// Regression: the post-lowering move pass re-marked the argument as
/// moved without observing that the bind target was re-initialized,
/// which rejected the shape six sample apps are written in.
#[test]
fn m2_consume_and_rebind_same_name_is_legal() {
    let c = compile(
        "*grow(v as [i64], x as i64) returns [i64]\n    v.push(x)\n    return v\n\n*main\n    g is [1]\n    g is grow(g, 2)\n    g is grow(g, 3)\n    log(g.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "3\n");
}

/// M2 — the revival holds across a loop body, which is the shape the
/// sample apps use: consume the accumulator and rebind it each iteration.
#[test]
fn m2_consume_and_rebind_in_loop_is_legal() {
    let c = compile(
        "*grow(v as [i64], x as i64) returns [i64]\n    v.push(x)\n    return v\n\n*main\n    g is [0]\n    i is 1\n    while i < 4\n        g is grow(g, i)\n        i is i + 1\n    log(g.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(c.run_stdout(), "4\n");
}

/// M2 — revival is precise: rebinding a *different* name still leaves
/// the consumed source tombstoned.
#[test]
fn m2_rebind_of_other_name_does_not_revive_source() {
    let c = compile(
        "*grow(v as [i64], x as i64) returns [i64]\n    v.push(x)\n    return v\n\n*main\n    g is [1]\n    h is grow(g, 2)\n    log(g.length)\n",
    );
    assert!(!c.ok());
    assert!(
        c.stderr().contains("use of moved value `g`"),
        "{}",
        c.stderr()
    );
}

/// A constructor call naming a type that does not exist must be a
/// diagnostic, not a silently-fabricated struct. Regression: the typer's
/// struct-literal fallthrough invented `Type::Struct(name)` for any
/// unknown name, so `x is Bogus()` compiled and ran.
#[test]
fn unknown_constructor_is_rejected() {
    let c = compile("*main\n    x is TotallyUndefinedThing()\n    log('ok')\n");
    assert!(!c.ok(), "unknown constructor must not compile");
    assert!(
        c.stderr().contains("unknown type or variant"),
        "{}",
        c.stderr()
    );
}

/// The rejection carries a suggestion when a near-miss exists.
#[test]
fn unknown_constructor_suggests_nearest_type() {
    let c = compile("type Point\n    a as i64\n\n*main\n    x is Poimt(a is 1)\n    log(x.a)\n");
    assert!(!c.ok());
    assert!(
        c.stderr().contains("did you mean `Point`?"),
        "{}",
        c.stderr()
    );
}

/// `None` is the prelude spelling in several other languages; Jinn's is
/// `Nothing`. Say so directly instead of "unknown variant".
#[test]
fn foreign_prelude_spelling_names_the_jinn_one() {
    let c = compile(
        "*find(x as i64) returns Option of i64\n    if x < 0\n        return None()\n    Some(x)\n\n*main\n    log(1)\n",
    );
    assert!(!c.ok());
    assert!(
        c.stderr().contains("Jinn spells this `Nothing`"),
        "{}",
        c.stderr()
    );
}

/// The canonical Option shape compiles once spelled correctly.
#[test]
fn option_nothing_and_some_typecheck() {
    let c = compile(
        "*find(x as i64) returns Option of i64\n    if x < 0\n        return Nothing()\n    Some(x)\n\n*main\n    r is find(5)\n    log(1)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
}
