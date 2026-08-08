use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/parallel.rs"]
mod parallel;

fn jinn() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinn"))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn app_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root().join("apps"))
        .expect("read apps/")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("project.jn").is_file())
        .collect();
    dirs.sort();
    dirs
}

fn build_and_run(dir: &Path) -> Result<(), String> {
    let out = tempfile::tempdir().expect("tempdir");
    let bin = out.path().join("app.bin");
    let c = Command::new(jinn())
        .arg("build")
        .arg("-o")
        .arg(&bin)
        .current_dir(dir)
        .output()
        .expect("invoke jinn build");
    if !c.status.success() {
        return Err(format!(
            "build failed:\n{}",
            String::from_utf8_lossy(&c.stderr)
        ));
    }
    let r = Command::new(&bin)
        .current_dir(out.path())
        .output()
        .expect("run app");
    if !r.status.success() {
        return Err(format!(
            "exited {:?}\nstdout:\n{}\nstderr:\n{}",
            r.status.code(),
            String::from_utf8_lossy(&r.stdout),
            String::from_utf8_lossy(&r.stderr)
        ));
    }
    Ok(())
}

#[test]
fn every_app_builds_and_runs() {
    let dirs = app_dirs();
    assert!(
        dirs.len() >= 20,
        "apps/ looks wrong: found only {} projects",
        dirs.len()
    );

    let results = parallel::par_map(dirs, |dir| {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        build_and_run(dir).map_err(|e| format!("  {name}: {e}"))
    });

    let failures: Vec<String> = results.into_iter().filter_map(|r| r.err()).collect();
    assert!(
        failures.is_empty(),
        "{} app(s) failed to build and run:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
