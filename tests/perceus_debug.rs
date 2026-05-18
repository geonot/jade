//! Integration test for the MIR-Perceus pass pipeline reporting via
//! `--debug-perceus`.
//!
//! The previous HIR-level analyzer (`perceus: ...` line) has been retired;
//! the only stats line emitted today is `mir-perceus: ...`. Tests focus on
//! the passes that the current MIR lowering reliably exercises.

use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
struct MirSummary {
    drops_elided: u32,
    drops_sunk: u32,
    drops_fused: u32,
    reuse_pairs: u32,
    borrows_promoted: u32,
    bindings: u32,
}

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn leading_u32(part: &str) -> Option<u32> {
    part.split_whitespace().next()?.parse().ok()
}

fn parse_bindings(part: &str) -> Option<u32> {
    let start = part.rfind('(')? + 1;
    let tail = &part[start..];
    let end = tail.find(" bindings")?;
    tail[..end].parse().ok()
}

fn parse_mir_summary(line: &str) -> Option<MirSummary> {
    let payload = line.strip_prefix("mir-perceus: ")?.trim();
    let parts: Vec<&str> = payload.split(", ").collect();
    if parts.len() != 5 {
        return None;
    }
    Some(MirSummary {
        drops_elided: leading_u32(parts[0])?,
        drops_sunk: leading_u32(parts[1])?,
        drops_fused: leading_u32(parts[2])?,
        reuse_pairs: leading_u32(parts[3])?,
        borrows_promoted: leading_u32(parts[4])?,
        bindings: parse_bindings(parts[4])?,
    })
}

fn compile(src: &str) -> (Option<MirSummary>, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let jinn = dir.path().join("case.jn");
    let out = dir.path().join("case_bin");
    std::fs::write(&jinn, src).expect("write case source");

    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .arg("--debug-perceus")
        .output()
        .expect("jinnc failed to start");

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    // Linker errors are fine for these tests — we only care about the
    // perceus stats line that the compiler prints before linking.
    let summary = stderr
        .lines()
        .find(|l| l.starts_with("mir-perceus: "))
        .and_then(parse_mir_summary);
    (summary, stderr)
}

fn require(summary: Option<MirSummary>, stderr: &str) -> MirSummary {
    summary.unwrap_or_else(|| panic!("missing or malformed mir-perceus summary:\n{stderr}"))
}

#[test]
fn perceus_stats_line_is_emitted() {
    let src = "*main() returns i32\n    0\n";
    let (summary, stderr) = compile(src);
    let s = require(summary, &stderr);
    assert!(
        s.bindings > 0,
        "expected MIR-perceus to analyze >0 bindings, got {s:?}"
    );
}

#[test]
fn drop_fusion_coalesces_consecutive_rc_drops() {
    let src = "*main() returns i32\n    x is rc(10)\n    y is rc(20)\n    z is rc(30)\n    log(@x + @y + @z)\n    0\n";
    let (summary, stderr) = compile(src);
    let s = require(summary, &stderr);
    assert!(
        s.drops_fused >= 2,
        "expected drop fusion to coalesce >=2 drops in a triple-rc scope, got {s:?}\n{stderr}"
    );
}

#[test]
fn perceus_stats_are_nondestructive_for_trivial_main() {
    // Pure scalar main: nothing should explode, drops_fused stays at 0
    // because i32 lowering never emits Drop instructions for trivials.
    let src = "*main() returns i32\n    a is 1\n    b is 2\n    log(a + b)\n    0\n";
    let (summary, stderr) = compile(src);
    let s = require(summary, &stderr);
    assert_eq!(
        s.drops_fused, 0,
        "unexpected fusion in scalar program: {s:?}"
    );
}
