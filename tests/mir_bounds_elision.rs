use std::path::PathBuf;
use std::process::Command;

fn jadec() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jadec"))
}

fn compile_and_emit_mir(src: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let jade = dir.path().join("test.jade");
    std::fs::write(&jade, src).expect("write source");

    let output = Command::new(jadec())
        .arg(&jade)
        .arg("--emit-mir")
        .output()
        .expect("jadec failed to start");

    assert!(
        output.status.success(),
        "jadec compilation failed\nstderr: {}\nsource:\n{}",
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
