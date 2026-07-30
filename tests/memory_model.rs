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
