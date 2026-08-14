use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/parallel.rs"]
mod parallel;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

struct Run {
    stdout: String,
    code: Option<i32>,
}

fn compile_and_run(src: &Path, opt: &str) -> Result<Run, String> {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("p.bin");
    let c = Command::new(jinnc())
        .arg(src)
        .arg("--opt")
        .arg(opt)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("invoke jinnc");
    if !c.status.success() {
        let stderr = String::from_utf8_lossy(&c.stderr);
        if stderr.contains("panicked at") || stderr.contains("internal compiler error") {
            return Err(format!("ICE at --opt {opt}: {}", first_lines(&stderr, 3)));
        }
        return Err(format!(
            "compile failed at --opt {opt}: {}",
            first_lines(&stderr, 3)
        ));
    }
    let r = Command::new(&bin)
        .current_dir(dir.path())
        .output()
        .expect("run");
    Ok(Run {
        stdout: String::from_utf8_lossy(&r.stdout).into_owned(),
        code: r.status.code(),
    })
}

fn first_lines(s: &str, n: usize) -> String {
    s.lines().take(n).collect::<Vec<_>>().join(" | ")
}

fn scrub(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '0' && chars.peek() == Some(&'x') {
            chars.next();
            while chars.peek().map(|d| d.is_ascii_hexdigit()).unwrap_or(false) {
                chars.next();
            }
            out.push_str("0xADDR");
        } else {
            out.push(c);
        }
    }
    out
}

fn jn_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map(|x| x == "jn").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

// Programs that do not compile for a recorded compiler limitation, with the
// diagnostic they must fail with. tests/programs_harness.rs keeps the same
// list and asserts the wording; here we only need to keep them out of the
// differential, which requires a running binary.
const UNSUPPORTED: &[&str] = &[];

fn is_unsupported(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|stem| UNSUPPORTED.contains(&stem))
}

fn differential_sweep(label: &str, files: Vec<PathBuf>, min_expected: usize) {
    let files: Vec<PathBuf> = files.into_iter().filter(|p| !is_unsupported(p)).collect();
    assert!(
        files.len() >= min_expected,
        "{label}: expected at least {min_expected} programs, found {}",
        files.len()
    );

    let results = parallel::par_map(files, |src| {
        let rel = src
            .strip_prefix(root())
            .unwrap_or(src)
            .display()
            .to_string();
        let a = match compile_and_run(src, "0") {
            Ok(r) => r,
            Err(e) => return Err(format!("{rel}: {e}")),
        };
        let b = match compile_and_run(src, "3") {
            Ok(r) => r,
            Err(e) => return Err(format!("{rel}: {e}")),
        };
        if scrub(&a.stdout) != scrub(&b.stdout) {
            return Err(format!(
                "{rel}: --opt 0 and --opt 3 disagree on stdout\n--- opt0 ---\n{}\n--- opt3 ---\n{}",
                a.stdout, b.stdout
            ));
        }
        if a.code != b.code {
            return Err(format!(
                "{rel}: --opt 0 exited {:?} but --opt 3 exited {:?}",
                a.code, b.code
            ));
        }
        Ok(())
    });

    let failures: Vec<String> = results.into_iter().filter_map(|r| r.err()).collect();
    assert!(
        failures.is_empty(),
        "{label}: {} program(s) failed:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn snippets_agree_across_opt_levels() {
    differential_sweep("snippets", jn_files(&root().join("snippets")), 300);
}

#[test]
fn test_programs_agree_across_opt_levels() {
    differential_sweep(
        "tests/programs",
        jn_files(&root().join("tests/programs")),
        50,
    );
}
