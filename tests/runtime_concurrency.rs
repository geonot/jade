//! Conformance tests for the Tier 2 runtime-concurrency fixes
//! (remediation-2026-07.md tasks 8-10..8-14). Each test compiles and runs a
//! real program through `jinnc`, asserting exact output within a timeout —
//! the failure modes here are hangs, SIGSEGVs, and allocator aborts.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

struct Compiled {
    dir: tempfile::TempDir,
}

fn compile(src: &str) -> Compiled {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("prog.jn"), src).unwrap();
    let out = Command::new(jinnc())
        .arg("prog.jn")
        .arg("-o")
        .arg(dir.path().join("prog.bin"))
        .current_dir(dir.path())
        .output()
        .expect("invoke jinnc");
    assert!(
        out.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Compiled { dir }
}

impl Compiled {
    /// Run with a hard timeout; a hang is a failure, not a stuck suite.
    fn run_within(&self, secs: u64) -> Output {
        let mut child = Command::new(self.dir.path().join("prog.bin"))
            .current_dir(self.dir.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn program");
        let deadline = std::time::Instant::now() + Duration::from_secs(secs);
        loop {
            match child.try_wait().expect("try_wait") {
                Some(_) => return child.wait_with_output().expect("collect output"),
                None if std::time::Instant::now() >= deadline => {
                    let _ = child.kill();
                    let out = child.wait_with_output().expect("collect output");
                    panic!(
                        "program hung past {secs}s: stdout so far={:?} stderr={:?}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }
}

fn sorted_lines(out: &[u8]) -> Vec<i64> {
    let mut v: Vec<i64> = String::from_utf8_lossy(out)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    v.sort();
    v
}

/// Task 8-13 — `tl_gen_coro` was set on every `jinn_gen_resume` but cleared
/// only on the trampoline's first entry, so a worker that resumed a
/// generator more than once kept a stale value forever, and the next fresh
/// coroutine whose first run landed there executed the *generator's* entry
/// instead of its own (SIGSEGV, 3/3 before the fix). Round 1 poisons every
/// worker (more advancing tasks than workers, several resumes each); round
/// 2's fresh coroutines must still run their own bodies.
#[test]
fn generator_resume_does_not_poison_worker_tls() {
    let advance: String = (1..=8)
        .map(|i| format!("        dispatch\n            advance({i}000)\n"))
        .collect();
    let fresh: String = (11..=18)
        .map(|i| format!("        dispatch\n            fresh({i})\n"))
        .collect();
    let src = format!(
        "*counter()\n    n is 0\n    loop\n        yield n\n        n is n + 1\n\n\
         *advance(id as i64)\n    g is counter()\n    a is g.next()\n    b is g.next()\n    c is g.next()\n    log(a + b + c + id)\n\n\
         *fresh(id as i64)\n    log(id)\n\n\
         *main\n    together\n{advance}    together\n{fresh}    log(999)\n"
    );
    let c = compile(&src);
    for round in 0..3 {
        let out = c.run_within(10);
        assert!(
            out.status.success(),
            "round {round}: {:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let mut expected: Vec<i64> = (11..=18).collect();
        expected.extend([999]);
        expected.extend((1..=8).map(|i| i * 1000 + 3));
        expected.sort();
        assert_eq!(
            sorted_lines(&out.stdout),
            expected,
            "round {round}: wrong output"
        );
    }
}
