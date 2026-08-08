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

const UNSUPPORTED: &[(&str, &str)] = &[(
    "compiler_pipeline",
    "joins values of different shapes at a control-flow merge",
)];

fn run_program(src: &Path) -> Result<String, String> {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("p.bin");
    let c = Command::new(jinnc())
        .arg(src)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("invoke jinnc");
    if !c.status.success() {
        return Err(format!(
            "compile failed: {}",
            String::from_utf8_lossy(&c.stderr)
        ));
    }
    let r = Command::new(&bin)
        .current_dir(dir.path())
        .output()
        .expect("run");
    if !r.status.success() {
        return Err(format!(
            "run failed ({:?}): {}",
            r.status,
            String::from_utf8_lossy(&r.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&r.stdout).into_owned())
}

#[test]
fn every_program_is_wired_and_matches_its_snapshot() {
    let progs_dir = root().join("tests/programs");
    let expected_dir = progs_dir.join("expected");
    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;

    let mut names: Vec<String> = std::fs::read_dir(&progs_dir)
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().map(|x| x == "jn").unwrap_or(false) {
                Some(p.file_stem().unwrap().to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect();
    names.sort();

    let scrub = |s: &str| -> String {
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
    };

    let todo: Vec<String> = names
        .iter()
        .filter(|n| !UNSUPPORTED.iter().any(|(u, _)| u == n))
        .cloned()
        .collect();

    let results = parallel::par_map(todo, |name| {
        let snap = expected_dir.join(format!("{name}.out"));
        let Ok(want) = std::fs::read_to_string(&snap) else {
            return Err(format!(
                "{name}.jn has no expected-output snapshot — add \
                 tests/programs/expected/{name}.out or delete the program \
                 with a reason"
            ));
        };
        let want = scrub(&want);
        match run_program(&progs_dir.join(format!("{name}.jn"))).map(|g| scrub(&g)) {
            Ok(got) if got == want => Ok(()),
            Ok(got) => Err(format!(
                "{name}.jn output drifted from its snapshot:\n--- want ---\n{want}\n--- got ---\n{got}"
            )),
            Err(e) => Err(format!("{name}.jn: {e}")),
        }
    });

    for r in results {
        match r {
            Ok(()) => checked += 1,
            Err(e) => failures.push(e),
        }
    }

    for (name, want_diag) in UNSUPPORTED {
        let src = root().join(format!("tests/programs/{name}.jn"));
        let dir = tempfile::tempdir().unwrap();
        let c = Command::new(jinnc())
            .arg(&src)
            .arg("-o")
            .arg(dir.path().join("p.bin"))
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&c.stderr);
        if c.status.success() {
            failures.push(format!(
                "{name}.jn now compiles — give it a snapshot in \
                 tests/programs/expected/ and drop it from UNSUPPORTED"
            ));
        } else if stderr.contains("panicked at") {
            failures.push(format!(
                "{name}.jn panics the compiler; it must fail with a diagnostic, \
                 not an ICE:\n{stderr}"
            ));
        } else if !stderr.contains(want_diag) {
            failures.push(format!(
                "{name}.jn failed for a different reason than recorded \
                 ({want_diag:?}):\n{stderr}"
            ));
        }
    }

    for e in std::fs::read_dir(&expected_dir).unwrap().flatten() {
        let stem = e.path().file_stem().unwrap().to_string_lossy().into_owned();
        if !names.contains(&stem) && !UNSUPPORTED.iter().any(|(u, _)| *u == stem) {
            failures.push(format!("orphaned snapshot expected/{stem}.out"));
        }
    }

    assert!(
        failures.is_empty(),
        "{} program(s) failed:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    assert!(
        checked > 80,
        "corpus scan looks wrong: only {checked} programs"
    );
}
