use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile_and_emit_mir(src: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let jinn = dir.path().join("test.jn");
    std::fs::write(&jinn, src).expect("write source");

    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("--emit-mir")
        .output()
        .expect("jinnc failed to start");

    assert!(
        output.status.success(),
        "jinnc compilation failed\nstderr: {}\nsource:\n{}",
        String::from_utf8_lossy(&output.stderr),
        src
    );

    String::from_utf8(output.stdout).expect("utf8 MIR output")
}

#[test]
fn foreach_over_collection_uses_index_unchecked() {
    let src = "*main()\n    xs is [1, 2, 3, 4]\n    sum is 0\n    for x in xs\n        sum is sum + x\n    log(sum)\n";
    let mir = compile_and_emit_mir(src);

    assert!(
        mir.contains("index_unchecked"),
        "expected bounds-proven foreach index to lower as index_unchecked\nMIR:\n{}",
        mir
    );
}

#[test]
fn sim_for_over_collection_uses_index_unchecked() {
    let src = "*main()\n    xs is [1, 2, 3, 4]\n    sim for x in xs\n        log(x)\n";
    let mir = compile_and_emit_mir(src);

    assert!(
        mir.contains("index_unchecked"),
        "expected bounds-proven sim-for index to lower as index_unchecked\nMIR:\n{}",
        mir
    );
}

fn emit_llvm(src: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let jinn = dir.path().join("test.jn");
    std::fs::write(&jinn, src).expect("write source");
    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("--emit-llvm")
        .output()
        .expect("jinnc failed to start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 IR")
}

#[test]
fn loop_bounded_index_arithmetic_elides_overflow_checks() {
    let src = "*main\n    n is 1500\n    total is 0\n    for i in 0 to n\n        for k in 0 to n\n            total is wrapping_add(total, (i * n + k) * (k * n + i))\n    log(total)\n";
    let ir = emit_llvm(src);
    assert!(
        !ir.contains("with.overflow"),
        "range-proven index arithmetic must not carry overflow checks\nIR:\n{ir}"
    );
}

#[test]
fn unbounded_arithmetic_keeps_overflow_checks() {
    let src = "*step(n)\n    3 * n + 1\n*main\n    log(step(7))\n";
    let ir = emit_llvm(src);
    assert!(
        ir.contains("with.overflow"),
        "arithmetic on unconstrained parameters must stay checked\nIR:\n{ir}"
    );
}

#[test]
fn for_counters_lower_to_ssa_not_slots() {
    let src = "*main\n    t is 0\n    for i in 0 to 10\n        t is t + i\n    log(t)\n";
    let mir = compile_and_emit_mir(src);
    assert!(
        !mir.contains("store i#") && !mir.contains("load i#"),
        "for-loop counter should be an SSA phi, not a memory slot\nMIR:\n{mir}"
    );
}
