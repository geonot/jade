use std::path::PathBuf;
use std::process::Command;

#[test]
fn durable_rewrite_survives_crash_injection() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("durable_crash");
    let cc = Command::new("cc")
        .args(["-O1", "-g", "-Wall", "-Iruntime"])
        .args([
            "tests/durable_crash.c",
            "runtime/durable.c",
            "runtime/kv.c",
            "-o",
        ])
        .arg(&bin)
        .current_dir(&root)
        .output()
        .expect("invoke cc");
    assert!(
        cc.status.success(),
        "harness compile failed: {}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = Command::new(&bin)
        .arg(dir.path())
        .arg("25")
        .current_dir(dir.path())
        .output()
        .expect("run harness");
    assert!(
        out.status.success(),
        "probes failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
