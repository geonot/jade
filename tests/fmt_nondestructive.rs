//! `jinn fmt` must never destroy source (task 8-2, decision D5; task
//! 8-18 made comments lexer trivia carried through the printer). A
//! commented file formats WITH its comments — every comment text, its
//! position kind (leading/trailing/standalone), and blank-line grouping
//! survive, and formatting is idempotent. In-place modification still
//! requires an explicit `--write` (the default prints to stdout).

use std::path::PathBuf;
use std::process::Command;

fn jinn() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinn"))
}

const COMMENTED: &str = "\
# leading comment
*main
    x is 1  # trailing comment
    # standalone comment
    log(x)
";

const UNCOMMENTED_UGLY: &str = "\
*main
    x    is    1
    log(x)
";

#[test]
fn fmt_write_preserves_comments_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("commented.jn");
    std::fs::write(&path, COMMENTED).unwrap();

    let out = Command::new(jinn())
        .args(["fmt", "--write"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fmt --write on a commented file must succeed (8-18): {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let once = std::fs::read_to_string(&path).unwrap();
    assert!(once.contains("# leading comment"), "{once}");
    assert!(once.contains("# trailing comment"), "{once}");
    assert!(once.contains("# standalone comment"), "{once}");
    // Trailing stays trailing (same line as its code).
    assert!(
        once.lines()
            .any(|l| l.contains("x is 1") && l.contains("# trailing comment")),
        "trailing comment must stay on its statement's line: {once}"
    );

    let out = Command::new(jinn())
        .args(["fmt", "--write"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let twice = std::fs::read_to_string(&path).unwrap();
    assert_eq!(once, twice, "fmt must be idempotent on commented files");
}

#[test]
fn fmt_default_prints_to_stdout_and_never_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ugly.jn");
    std::fs::write(&path, UNCOMMENTED_UGLY).unwrap();

    let out = Command::new(jinn()).arg("fmt").arg(&path).output().unwrap();

    assert!(out.status.success(), "fmt (stdout mode) should succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("x is 1"),
        "formatted output should go to stdout, got: {stdout}"
    );
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        after, UNCOMMENTED_UGLY,
        "without --write the file must not be modified"
    );
}

#[test]
fn fmt_write_formats_uncommented_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ugly.jn");
    std::fs::write(&path, UNCOMMENTED_UGLY).unwrap();

    let out = Command::new(jinn())
        .args(["fmt", "--write"])
        .arg(&path)
        .output()
        .unwrap();

    assert!(out.status.success(), "fmt --write on a clean file succeeds");
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("x is 1"),
        "file should be reformatted in place, got: {after}"
    );
}

#[test]
fn fmt_preserves_shebang() {
    // A shebang is lexer trivia like a comment; it must survive at the
    // top of the file.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.jn");
    let src = "#!/usr/bin/env jinn\n*main\n    log(1)\n";
    std::fs::write(&path, src).unwrap();

    let out = Command::new(jinn())
        .args(["fmt", "--write"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.starts_with("#!/usr/bin/env jinn"),
        "shebang must stay first: {after}"
    );
}

/// Task 8-18 round-trip property over the whole snippets corpus: for
/// every parseable snippet, `fmt` succeeds, is idempotent, and preserves
/// the multiset of comment texts.
#[test]
fn fmt_roundtrip_over_snippets_corpus() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("snippets");
    let mut checked = 0usize;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().map(|x| x == "jn").unwrap_or(false) {
                let src = std::fs::read_to_string(&p).unwrap();
                let comments = |s: &str| -> Vec<String> {
                    s.lines()
                        .filter_map(|l| l.find('#').map(|i| l[i..].trim_end().to_string()))
                        .collect::<Vec<_>>()
                };
                let once = match jinnc::fmt::format_source(&src) {
                    Ok(o) => o,
                    Err(_) => continue, // unparseable corpus entries are out of scope
                };
                let twice = jinnc::fmt::format_source(&once)
                    .unwrap_or_else(|e| panic!("reformat failed for {}: {e}", p.display()));
                assert_eq!(once, twice, "fmt not idempotent for {}", p.display());
                let mut a = comments(&src);
                let mut b = comments(&once);
                a.sort();
                b.sort();
                assert_eq!(a, b, "comments not preserved for {}", p.display());
                checked += 1;
            }
        }
    }
    assert!(
        checked > 300,
        "corpus scan looks wrong: only {checked} files"
    );
}
