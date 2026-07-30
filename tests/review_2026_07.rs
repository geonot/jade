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

/// §3.2 — two dispatch tasks mutating one Vec corrupt the allocator.
/// FIXME(8-8): must be rejected at compile time with an actionable
/// use-after-move diagnostic naming channels/actors as the alternative.
#[test]
fn review_3_2_cross_task_shared_vec_races() {
    let c = compile(
        "*pusher(v, base)\n    for i in 0 to 20000\n        v.push(base + i)\n\n*main\n    shared is vec()\n    together\n        dispatch\n            pusher(shared, 0)\n        dispatch\n            pusher(shared, 1000000)\n    log(shared.length)\n",
    );
    assert!(
        c.ok(),
        "OBSERVED-BAD: compiles today with no diagnostic. If this now fails \
         to compile, task 8-8 has landed — flip this test to assert the \
         rejection diagnostic. {}",
        c.stderr()
    );
    // The race aborts essentially every run; allow retries so scheduler luck
    // cannot green the suite.
    let crashed = (0..5).any(|_| !c.run().status.success());
    assert!(
        crashed,
        "expected the shared-Vec race to crash at least once in 5 runs; if it \
         is now stable, re-examine whether the program became safe (8-8) or \
         the race merely got harder to hit"
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

/// §4.6 — cross-type `equals` (String vs i64) is accepted and segfaults.
/// FIXME(8-16): must be a type error at compile time.
#[test]
fn review_4_6_cross_type_equals_segfaults() {
    let c = compile("*main\n    if 'abc' equals 5\n        log('huh')\n");
    assert!(
        c.ok(),
        "OBSERVED-BAD: the typer accepts String-vs-i64 `equals` today. If it \
         is now rejected, task 8-16 has landed — flip this test. {}",
        c.stderr()
    );
    let run = c.run();
    assert!(
        !run.status.success(),
        "expected the accepted-but-ill-typed comparison to crash at runtime: {}",
        exit_desc(&run)
    );
}

/// §4.6 — declared `returns String`, body returns i64. Rejected since the
/// review, but by the MIR verifier as "this is a compiler bug" instead of a
/// source-level type diagnostic.
/// FIXME(8-17): must be a proper `Diagnostic` with the function's span; no
/// internal-error wording on a plain source type error.
#[test]
fn review_4_6_declared_string_returns_i64() {
    let c = compile("*f(x as i64) returns String\n    x + 1\n\n*main\n    log(f(1))\n");
    assert!(!c.ok(), "must not compile");
    let stderr = c.stderr();
    assert!(
        stderr.contains("compiler bug") || stderr.contains("MIR verify"),
        "OBSERVED-BAD: currently reported as an internal MIR-verify failure. \
         If this is now a source diagnostic, task 8-17 has landed — flip this \
         test to assert the diagnostic and the absence of ICE wording. {stderr}"
    );
}

/// §4.6 — `'abc' + 1` is rejected since the review, but by the HIR validator
/// (`hir-validate:` prefix) rather than the typer's diagnostic path.
/// FIXME(8-17): must be a source-level diagnostic from the typer.
#[test]
fn review_4_6_string_plus_int() {
    let c = compile("*main\n    x is 'abc' + 1\n    log(x)\n");
    assert!(!c.ok(), "must not compile");
    let stderr = c.stderr();
    assert!(
        stderr.contains("hir-validate"),
        "OBSERVED-BAD: currently caught by hir-validate, not the typer. If \
         the message changed, check whether 8-17 landed and flip. {stderr}"
    );
}

/// §4.6 — heterogeneous `vec()` type-checks and reading element 1 as an
/// integer yields a leaked pointer value.
/// FIXME(8-16): the second `push` must be a type error.
#[test]
fn review_4_6_heterogeneous_vec() {
    let c = compile("*main\n    v is vec()\n    v.push(1)\n    v.push('two')\n    log(v.get(1))\n");
    assert!(
        c.ok(),
        "OBSERVED-BAD: heterogeneous vec type-checks today. If it is now \
         rejected, task 8-16 has landed — flip this test. {}",
        c.stderr()
    );
    let run = c.run();
    assert!(
        run.status.success(),
        "currently runs to completion printing a leaked pointer as an integer: {}",
        exit_desc(&run)
    );
}

/// §4.7 — multi-clause arity mismatch delivers a correct message as a Rust
/// panic (RUST_BACKTRACE hint, panic exit code).
/// FIXME(8-17): must be a normal diagnostic — non-zero (non-panic) exit, no
/// backtrace hint, span-carrying rendering.
#[test]
fn review_4_7_multi_clause_arity_panics() {
    let c = compile("*f(0) is 0\n*f a, b is a + b\n\n*main\n    log(f(1, 2))\n");
    assert!(!c.ok(), "must not compile");
    let stderr = c.stderr();
    assert!(
        stderr.contains("RUST_BACKTRACE"),
        "OBSERVED-BAD: the arity mismatch is currently reported via a Rust \
         panic. If the backtrace hint is gone, task 8-17 has landed — flip \
         this test to assert a clean diagnostic. {stderr}"
    );
}

/// Found while pinning: top-level statements with a reassignment crash the
/// compiler with a stack overflow (3-line program, no `*main`).
/// FIXME(8-17): must be either accepted or cleanly diagnosed; never an ICE.
#[test]
fn top_level_reassignment_overflows_compiler_stack() {
    let c = compile("x is 41\nx is x + 1\nlog(x)\n");
    assert!(
        !c.ok(),
        "OBSERVED-BAD: currently dies (stack overflow / abort). If this now \
         compiles or errors cleanly, task 8-17 (no ICE on user input) has \
         landed — flip this test. {}",
        c.stderr()
    );
}

// ─── Store (§5.4) ───────────────────────────────────────────────────────────

/// §5.4 — `for u in all users` segfaults at runtime.
/// FIXME(8-25): `all <store>` must yield an iterable row set.
#[test]
fn review_5_4_all_store_iteration_segfaults() {
    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    for u in all users\n        log(u.name)\n",
    );
    assert!(c.ok(), "compiles today: {}", c.stderr());
    let run = c.run();
    assert!(
        !run.status.success(),
        "OBSERVED-BAD: `for u in all users` currently crashes. If it now \
         works, task 8-25 has landed — flip this test to assert the two rows \
         are printed. {}",
        exit_desc(&run)
    );
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
