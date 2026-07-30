//! Task 8-15 (decision D4) — explicit module resolution. A compile sees
//! exactly one entry file plus the transitive closure of its explicit
//! `use` declarations. Directory-tree absorption and identifier-driven
//! implicit import are gone: what a program means no longer depends on
//! what else happens to be in the directory.

use std::path::{Path, PathBuf};
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile(dir: &Path, entry: &str) -> std::process::Output {
    Command::new(jinnc())
        .arg(entry)
        .arg("-o")
        .arg(dir.join("prog.bin"))
        .current_dir(dir)
        .output()
        .expect("invoke jinnc")
}

fn run(dir: &Path) -> String {
    let out = Command::new(dir.join("prog.bin"))
        .current_dir(dir)
        .output()
        .expect("run");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const ENTRY: &str = "*main\n    x is 41\n    log(x + 1)\n";

/// A sibling file with a syntax error must leave the target compile
/// unaffected (it used to surface as `warning:` lines, and its decls
/// were silently merged).
#[test]
fn sibling_with_syntax_error_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.jn"), ENTRY).unwrap();
    std::fs::write(dir.path().join("broken.jn"), "*oops(((((\n    ???\n").unwrap();
    let out = compile(dir.path(), "main.jn");
    assert!(
        out.status.success(),
        "sibling syntax error leaked into the compile: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("broken.jn"),
        "no diagnostic may reference a file outside the use-closure: {stderr}"
    );
    assert_eq!(run(dir.path()), "42");
}

/// A sibling declaring a colliding type must not change what the entry
/// file's names mean.
#[test]
fn sibling_with_colliding_type_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let entry = "type Vec3\n    x as f64\n    y as f64\n    z as f64\n\n*main\n    v is Vec3(x is 1.5, y is 0.0, z is 0.0)\n    log(v.x)\n";
    std::fs::write(dir.path().join("main.jn"), entry).unwrap();
    std::fs::write(
        dir.path().join("other.jn"),
        "type Vec3\n    a as i64\n    b as i64\n",
    )
    .unwrap();
    let out = compile(dir.path(), "main.jn");
    assert!(
        out.status.success(),
        "colliding sibling type leaked in: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(run(dir.path()), "1.500000");
}

/// A sibling declaring a store must not create that store's files or
/// affect the target compile.
#[test]
fn sibling_with_store_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.jn"), ENTRY).unwrap();
    std::fs::write(
        dir.path().join("stored.jn"),
        "store ghosts\n    name as String\n\n*seed()\n    insert ghosts 'boo'\n",
    )
    .unwrap();
    let out = compile(dir.path(), "main.jn");
    assert!(out.status.success());
    assert_eq!(run(dir.path()), "42");
    assert!(
        !dir.path().join("ghosts.store").exists(),
        "a never-imported sibling store must not materialize"
    );
}

/// `use sibling` still resolves a module beside the entry file — the
/// explicit path keeps working, including its diagnostics.
#[test]
fn explicit_use_of_sibling_still_resolves() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.jn"),
        "use helper\n\n*main\n    log(helper.triple(14))\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("helper.jn"),
        "*triple(n as i64) returns i64\n    n * 3\n",
    )
    .unwrap();
    let out = compile(dir.path(), "main.jn");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(run(dir.path()), "42");
}

/// A qualified reference to an importable module without `use` is an
/// error naming the fix — never a silent auto-import.
#[test]
fn unimported_sibling_module_is_an_error_naming_use() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.jn"),
        "*main\n    log(helper.triple(14))\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("helper.jn"),
        "*triple(n as i64) returns i64\n    n * 3\n",
    )
    .unwrap();
    let out = compile(dir.path(), "main.jn");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`use helper`"),
        "diagnostic must name the fix: {stderr}"
    );
}
