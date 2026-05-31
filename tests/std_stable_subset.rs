//! Alpha-stable standard-library subset gate.
//!
//! Every module under `std/` is expected to pass the compiler **frontend**
//! (lex → parse → type-check → HIR) when compiled in library mode. This test
//! enforces that contract so a regression in any alpha-stable std module fails
//! CI immediately.
//!
//! The check uses `jinnc <module> --lib --emit-hir`, which runs the frontend
//! only (no LLVM codegen, no linking, no execution) and is therefore fast and
//! safe to run over the whole directory.
//!
//! Modules listed in [`EXPERIMENTAL`] are explicitly excluded from the stable
//! subset because they rely on language features that are not yet implemented.
//! Each exclusion must carry a justification comment. When a feature lands,
//! drop the corresponding entry here so the module is gated like the rest.

use std::path::{Path, PathBuf};
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

/// Std modules deliberately outside the alpha-stable subset.
///
/// * `test.jn` — uses `try`/`rescue` exception handling, which has no
///   lexer/parser/HIR support yet. Experimental until exceptions land.
const EXPERIMENTAL: &[&str] = &["test.jn"];

/// Frontend-check a single std module: `jinnc <path> --lib --emit-hir`.
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

/// Guard against bit-rot in [`EXPERIMENTAL`]: every listed module must exist
/// and must *actually still fail* the frontend gate. When a module is fixed,
/// the entry has to be removed — this keeps the exclusion list honest.
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
