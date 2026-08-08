use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/parallel.rs"]
mod parallel;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

const EXPERIMENTAL: &[&str] = &["test.jn"];

fn frontend_check(path: &Path) -> Result<(), String> {
    let output = Command::new(jinnc())
        .arg(path)
        .arg("--lib")
        .arg("--emit-hir")
        .output()
        .expect("jinnc failed to start");
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn import_and_link_check(module: &str) -> Result<(), String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("importer.jn");
    std::fs::write(
        &src,
        format!("use std/{module}\n\n*main\n    log(1)\n    0\n"),
    )
    .expect("write");
    let bin = dir.path().join("importer.bin");
    let output = Command::new(jinnc())
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .current_dir(dir.path())
        .output()
        .expect("jinnc failed to start");
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let run = Command::new(&bin)
        .current_dir(dir.path())
        .output()
        .expect("run importer");
    if !run.status.success() {
        return Err(format!(
            "importer linked but exited {:?}\n{}",
            run.status.code(),
            String::from_utf8_lossy(&run.stderr)
        ));
    }
    Ok(())
}

fn stable_modules() -> Vec<PathBuf> {
    let std_dir = Path::new("std");
    assert!(
        std_dir.is_dir(),
        "std/ directory not found (cwd must be the crate root)"
    );
    let mut entries: Vec<_> = std::fs::read_dir(std_dir)
        .expect("read std/")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "jn").unwrap_or(false))
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            !EXPERIMENTAL.contains(&name.as_str())
        })
        .collect();
    entries.sort();
    assert!(
        !entries.is_empty(),
        "no std modules were found — directory layout changed?"
    );
    entries
}

fn report(gate: &str, failures: Vec<String>) {
    assert!(
        failures.is_empty(),
        "{} std module(s) failed the alpha-stable {gate} gate:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn std_stable_subset_frontend_checks() {
    let results = parallel::par_map(stable_modules(), |path| {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        frontend_check(path).map_err(|e| format!("  {name}:\n{}", indent(&e)))
    });
    report(
        "frontend",
        results.into_iter().filter_map(|r| r.err()).collect(),
    );
}

#[test]
fn std_stable_subset_imports_and_links() {
    let results = parallel::par_map(stable_modules(), |path| {
        let module = path.file_stem().unwrap().to_string_lossy().into_owned();
        import_and_link_check(&module).map_err(|e| format!("  {module}:\n{}", indent(&e)))
    });
    report(
        "import-and-link",
        results.into_iter().filter_map(|r| r.err()).collect(),
    );
}

#[test]
fn std_experimental_list_is_accurate() {
    for name in EXPERIMENTAL {
        let path = Path::new("std").join(name);
        assert!(path.is_file(), "EXPERIMENTAL lists missing module `{name}`");
        let module = path.file_stem().unwrap().to_string_lossy().into_owned();
        assert!(
            frontend_check(&path).is_err() || import_and_link_check(&module).is_err(),
            "`{module}` now passes both alpha-stable gates — remove it from \
             EXPERIMENTAL so it is covered by the stable subset"
        );
    }
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
