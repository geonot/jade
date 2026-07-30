//! Conformance for the single shared builtin-method registry.
//!
//! `src/builtin_methods.rs` is the one place that declares which names exist
//! on Vec/Map/String receivers. Both the typer (return-type inference) and the
//! codegen (instruction selection) consult it; codegen `match`es the registry
//! enums exhaustively, so adding a method in the registry without wiring
//! codegen breaks the build. These tests pin the runtime behaviour of the
//! methods that previously drifted between typer and codegen.

use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile_and_run(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    let out = dir.path().join("test_bin");
    std::fs::write(&jinn, src).unwrap();
    let status = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jinnc failed to start");
    assert!(status.success(), "jinnc compilation failed for:\n{src}");
    let output = Command::new(&out)
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        "binary exited with {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn expect(src: &str, expected: &str) {
    assert_eq!(
        compile_and_run(src).trim(),
        expected.trim(),
        "source:\n{src}"
    );
}

#[test]
fn vec_chain_concatenates() {
    expect(
        "*main\n    c is [1, 2].chain([3, 4])\n    log(c.len())\n    log(c[3])\n",
        "4\n4",
    );
}

#[test]
fn vec_flatten_one_level() {
    expect(
        "*main\n    c is [[1, 2], [3], [4, 5, 6]].flatten()\n    log(c.len())\n    log(c[5])\n",
        "6\n6",
    );
}

#[test]
fn vec_enumerate_pairs_index() {
    expect(
        "*main\n    c is [10, 20, 30].enumerate()\n    log(c.len())\n",
        "3",
    );
}

#[test]
fn map_keys_and_values() {
    expect(
        "*main\n    m is map()\n    m.set('a', 1)\n    m.set('b', 2)\n    m.set('c', 3)\n    log(m.keys().len())\n    log(m.values().len())\n",
        "3\n3",
    );
}

#[test]
fn vec_shift_first_last_distinct() {
    expect(
        "*main\n    v is [10, 20, 30]\n    log(v.first())\n    log(v.last())\n    log(v.shift())\n    log(v.len())\n",
        "10\n30\n10\n2",
    );
}

#[test]
fn registry_names_round_trip() {
    use jinnc::builtin_methods::{MapMethod, StrMethod, VecMethod};

    for name in [
        "push",
        "pop",
        "shift",
        "first",
        "last",
        "get",
        "set",
        "remove",
        "clear",
        "len",
        "count",
        "is_empty",
        "contains",
        "sum",
        "join",
        "take",
        "skip",
        "slice",
        "collect",
        "reverse",
        "sort",
        "flatten",
        "enumerate",
        "chain",
        "zip",
        "map",
        "filter",
        "fold",
        "find",
        "any",
        "all",
    ] {
        assert!(
            VecMethod::from_name(name).is_some(),
            "vec method `{name}` missing"
        );
    }
    assert!(VecMethod::from_name("nope").is_none());

    for name in [
        "set", "get", "has", "contains", "remove", "clear", "len", "count", "keys", "values",
    ] {
        assert!(
            MapMethod::from_name(name).is_some(),
            "map method `{name}` missing"
        );
    }
    assert!(MapMethod::from_name("nope").is_none());

    for name in [
        "len",
        "length",
        "byte_count",
        "contains",
        "starts_with",
        "ends_with",
        "char_at",
        "find",
        "slice",
        "trim",
        "trim_left",
        "trim_right",
        "replace",
        "to_upper",
        "to_lower",
        "repeat",
        "split",
        "lines",
        "is_empty",
    ] {
        assert!(
            StrMethod::from_name(name).is_some(),
            "string method `{name}` missing"
        );
    }
    assert!(StrMethod::from_name("nope").is_none());
}
