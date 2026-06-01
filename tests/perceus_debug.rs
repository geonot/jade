use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct MirSummary {
    drops_elided: u32,
    drops_sunk: u32,
    drops_fused: u32,
    reuse_pairs: u32,
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
    if parts.len() != 4 {
        return None;
    }
    Some(MirSummary {
        drops_elided: leading_u32(parts[0])?,
        drops_sunk: leading_u32(parts[1])?,
        drops_fused: leading_u32(parts[2])?,
        reuse_pairs: leading_u32(parts[3])?,
        bindings: parse_bindings(parts[3])?,
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
fn drop_fusion_coalesces_consecutive_scope_exit_drops() {
    // Value-semantics drop-fusion probe (replaces the removed rc() variant).
    // Three owned heap vectors are borrowed by a single call, so each one's
    // last use is that call; their scope-exit drops land consecutively and
    // fuse into one DropMany run.
    let src = "*sink3(a as Vec of i64, b as Vec of i64, c as Vec of i64) returns i64\n    a.len() + b.len() + c.len()\n\n*main() returns i32\n    x is [1, 2, 3]\n    y is [4, 5, 6]\n    z is [7, 8, 9]\n    log(sink3(x, y, z))\n    0\n";
    let (summary, stderr) = compile(src);
    let s = require(summary, &stderr);
    assert!(
        s.drops_fused >= 2,
        "expected drop fusion to coalesce >=2 consecutive scope-exit drops, got {s:?}\n{stderr}"
    );
}

#[test]
fn perceus_stats_are_nondestructive_for_trivial_main() {
    let src = "*main() returns i32\n    a is 1\n    b is 2\n    log(a + b)\n    0\n";
    let (summary, stderr) = compile(src);
    let s = require(summary, &stderr);
    assert_eq!(
        s.drops_fused, 0,
        "unexpected fusion in scalar program: {s:?}"
    );
}
