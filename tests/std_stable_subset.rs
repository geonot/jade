use std::path::{Path, PathBuf};
use std::process::Command;

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

#[test]
fn std_stable_subset_frontend_checks() {
    let std_dir = Path::new("std");
    assert!(
        std_dir.is_dir(),
        "std/ directory not found (cwd must be the crate root)"
    );

    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();

    let mut entries: Vec<_> = std::fs::read_dir(std_dir)
        .expect("read std/")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "jn").unwrap_or(false))
        .collect();
    entries.sort();

    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if EXPERIMENTAL.contains(&name.as_str()) {
            continue;
        }
        checked += 1;
        if let Err(stderr) = frontend_check(&path) {
            failures.push(format!("  {name}:\n{}", indent(&stderr)));
        }
    }

    assert!(
        checked > 0,
        "no std modules were checked — directory layout changed?"
    );
    assert!(
        failures.is_empty(),
        "{} std module(s) failed the alpha-stable frontend gate:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn std_experimental_list_is_accurate() {
    for name in EXPERIMENTAL {
        let path = Path::new("std").join(name);
        assert!(path.is_file(), "EXPERIMENTAL lists missing module `{name}`");
        assert!(
            frontend_check(&path).is_err(),
            "`{name}` now passes the frontend gate — remove it from EXPERIMENTAL \
             so it is covered by the stable subset"
        );
    }
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
