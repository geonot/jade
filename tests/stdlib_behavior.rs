use std::path::PathBuf;
use std::process::Command;

#[path = "support/parallel.rs"]
mod parallel;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn stdlib_behavior_suites_pass() {
    let dir = root().join("tests/stdlib");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read tests/stdlib")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("_tests.jn"))
        })
        .collect();
    files.sort();
    assert!(
        files.len() >= 12,
        "expected the stdlib behavior corpus, found {} files",
        files.len()
    );

    let failures: Vec<String> = parallel::par_map(files, |src| {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bin = tmp.path().join("t.bin");
        let name = src.file_stem().unwrap().to_string_lossy().to_string();
        let c = Command::new(jinnc())
            .arg(&src)
            .arg("--test")
            .arg("-o")
            .arg(&bin)
            .current_dir(tmp.path())
            .output()
            .expect("invoke jinnc");
        if !c.status.success() {
            return Some(format!(
                "{name}: compile failed:\n{}",
                String::from_utf8_lossy(&c.stderr)
            ));
        }
        let r = Command::new(&bin)
            .current_dir(tmp.path())
            .output()
            .expect("run test binary");
        if !r.status.success() {
            return Some(format!(
                "{name}: tests failed (status {:?}):\n{}\n{}",
                r.status,
                String::from_utf8_lossy(&r.stdout),
                String::from_utf8_lossy(&r.stderr)
            ));
        }
        None
    })
    .into_iter()
    .flatten()
    .collect();

    assert!(
        failures.is_empty(),
        "stdlib behavior suites failed:\n{}",
        failures.join("\n---\n")
    );
}
