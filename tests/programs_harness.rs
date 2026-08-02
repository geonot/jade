use std::path::{Path, PathBuf};
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

const KNOWN_ICE: &[&str] = &["compiler_pipeline"];

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

    for name in &names {
        if KNOWN_ICE.contains(&name.as_str()) {
            continue;
        }
        let snap = expected_dir.join(format!("{name}.out"));
        let Ok(want) = std::fs::read_to_string(&snap) else {
            failures.push(format!(
                "{name}.jn has no expected-output snapshot — add \
                 tests/programs/expected/{name}.out or delete the program \
                 with a reason"
            ));
            continue;
        };

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
        let want = scrub(&want);
        match run_program(&progs_dir.join(format!("{name}.jn"))).map(|g| scrub(&g)) {
            Ok(got) if got == want => checked += 1,
            Ok(got) => failures.push(format!(
                "{name}.jn output drifted from its snapshot:\n--- want ---\n{want}\n--- got ---\n{got}"
            )),
            Err(e) => failures.push(format!("{name}.jn: {e}")),
        }
    }

    for e in std::fs::read_dir(&expected_dir).unwrap().flatten() {
        let stem = e.path().file_stem().unwrap().to_string_lossy().into_owned();
        if !names.contains(&stem) {
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

#[test]
fn known_ice_still_reproduces() {
    for name in KNOWN_ICE {
        let src = root().join(format!("tests/programs/{name}.jn"));
        let dir = tempfile::tempdir().unwrap();
        let c = Command::new(jinnc())
            .arg(&src)
            .arg("-o")
            .arg(dir.path().join("p.bin"))
            .output()
            .unwrap();
        assert!(
            !c.status.success(),
            "{name}.jn now compiles — wire it into the snapshot corpus and \
             remove it from KNOWN_ICE"
        );
    }
}
