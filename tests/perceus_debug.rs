use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
struct PerceusSummary {
    drops_elided: u32,
    reuse: u32,
    borrow_to_move: u32,
    fbip: u32,
    tail_reuse: u32,
    speculative: u32,
    pool_hints: u32,
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

fn parse_summary_line(line: &str, prefix: &str, expect_mir: bool) -> Option<PerceusSummary> {
    let payload = line.strip_prefix(prefix)?.trim();
    let parts: Vec<&str> = payload.split(", ").collect();

    let expected_parts = if expect_mir { 8 } else { 7 };
    if parts.len() != expected_parts {
        return None;
    }

    let drops_elided = leading_u32(parts[0])?;
    let reuse = leading_u32(parts[1])?;
    let borrow_to_move = leading_u32(parts[2])?;
    let fbip = leading_u32(parts[3])?;
    let tail_reuse = leading_u32(parts[4])?;
    let speculative = leading_u32(parts[5])?;
    let pool_hints = leading_u32(parts[6])?;

    let bindings_src = if expect_mir {
        let _ = leading_u32(parts[7])?;
        parts[7]
    } else {
        parts[6]
    };

    let bindings = parse_bindings(bindings_src)?;

    Some(PerceusSummary {
        drops_elided,
        reuse,
        borrow_to_move,
        fbip,
        tail_reuse,
        speculative,
        pool_hints,
        bindings,
    })
}

fn compile_and_collect_summaries(
    src: &str,
) -> (Option<PerceusSummary>, Option<PerceusSummary>, String) {
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
    assert!(
        output.status.success(),
        "jinnc compilation failed:\n{}\nsource:\n{}",
        stderr,
        src
    );

    let perceus = stderr
        .lines()
        .find(|l| l.starts_with("perceus: "))
        .and_then(|l| parse_summary_line(l, "perceus: ", false));

    let mir = stderr
        .lines()
        .find(|l| l.starts_with("mir-perceus: "))
        .and_then(|l| parse_summary_line(l, "mir-perceus: ", true));

    (perceus, mir, stderr)
}

fn require_perceus(summary: Option<PerceusSummary>, stderr: &str) -> PerceusSummary {
    summary.unwrap_or_else(|| panic!("missing or malformed perceus summary:\n{stderr}"))
}

fn require_mir(summary: Option<PerceusSummary>, stderr: &str) -> PerceusSummary {
    summary.unwrap_or_else(|| panic!("missing or malformed mir-perceus summary:\n{stderr}"))
}

#[test]
fn perceus_debug_drop_elision_visible() {
    let src =
        "*main() returns i32\n    a is 1\n    b is 2\n    c is 3\n    log(a + b + c)\n    0\n";
    let (perceus, _, stderr) = compile_and_collect_summaries(src);
    let p = require_perceus(perceus, &stderr);
    assert!(p.drops_elided > 0, "expected drops elided > 0, got {p:?}");
    assert!(p.bindings > 0, "expected bindings tracked > 0, got {p:?}");
}

#[test]
fn perceus_debug_reuse_visible() {
    let src =
        "*main() returns i32\n    x is rc(10)\n    y is rc(20)\n    log(@y)\n    log(@y)\n    0\n";
    let (perceus, mir, stderr) = compile_and_collect_summaries(src);
    let p = require_perceus(perceus, &stderr);
    let m = require_mir(mir, &stderr);
    assert!(p.reuse > 0, "expected HIR reuse > 0, got {p:?}");
    assert!(m.reuse > 0, "expected MIR reuse > 0, got {m:?}");
}

#[test]
fn perceus_debug_speculative_reuse_visible() {
    let src = "*main() returns i32\n    x is rc(10)\n    log(@x)\n    y is rc(20)\n    log(@y)\n    log(@y)\n    0\n";
    let (perceus, _, stderr) = compile_and_collect_summaries(src);
    let p = require_perceus(perceus, &stderr);
    assert!(
        p.speculative > 0,
        "expected speculative reuse > 0, got {p:?}"
    );
}

#[test]
fn perceus_debug_borrow_promotion_visible() {
    let src = "*main() returns i32\n    x is 42\n    p is %x\n    log(@p)\n    0\n";
    let (perceus, _, stderr) = compile_and_collect_summaries(src);
    let p = require_perceus(perceus, &stderr);
    assert!(
        p.borrow_to_move > 0,
        "expected borrow-to-move > 0, got {p:?}"
    );
}

#[test]
fn perceus_debug_fbip_visible() {
    let src = "enum State\n    Idle\n    Running(i64)\n\n*step(s as State)\n    match s\n        Idle ? Running(0)\n        Running(n) ? Running(n + 1)\n\n*main() returns i32\n    s is Idle\n    s is step(s)\n    log(0)\n    0\n";
    let (perceus, _, stderr) = compile_and_collect_summaries(src);
    let p = require_perceus(perceus, &stderr);
    assert!(p.fbip > 0, "expected fbip > 0, got {p:?}");
}

#[test]
fn perceus_debug_tail_reuse_visible() {
    let src = "type Boxed\n    v as i64\n\n*rebuild(x as Boxed) returns Boxed\n    Boxed(v is x.v + 1)\n\n*main() returns i32\n    bx is Boxed(v is 0)\n    b2 is rebuild(bx)\n    log(b2.v)\n    0\n";
    let (perceus, mir, stderr) = compile_and_collect_summaries(src);
    let p = require_perceus(perceus, &stderr);
    let m = require_mir(mir, &stderr);
    assert!(p.tail_reuse > 0, "expected HIR tail-reuse > 0, got {p:?}");
    assert!(m.tail_reuse > 0, "expected MIR tail-reuse > 0, got {m:?}");
}

#[test]
fn perceus_debug_pool_hints_visible() {
    let src = "*main() returns i32\n    i is 0\n    while i < 20\n        x is rc(i)\n        log(@x)\n        i is i + 1\n    0\n";
    let (perceus, mir, stderr) = compile_and_collect_summaries(src);
    let p = require_perceus(perceus, &stderr);
    let m = require_mir(mir, &stderr);
    assert!(p.pool_hints > 0, "expected HIR pool-hints > 0, got {p:?}");
    assert!(m.pool_hints > 0, "expected MIR pool-hints > 0, got {m:?}");
}

#[test]
fn perceus_debug_combo_borrow_fbip_pool() {
    let src = "enum State\n    Idle\n    Running(i64)\n\n*step(s as State)\n    match s\n        Idle ? Running(0)\n        Running(n) ? Running(n + 1)\n\n*main() returns i32\n    x is 1\n    p is %x\n    log(@p)\n    s is Idle\n    s is step(s)\n    i is 0\n    while i < 5\n        t is rc(i)\n        log(@t)\n        i is i + 1\n    0\n";
    let (perceus, _, stderr) = compile_and_collect_summaries(src);
    let p = require_perceus(perceus, &stderr);
    assert!(p.drops_elided > 0, "expected drops > 0 in combo, got {p:?}");
    assert!(p.borrow_to_move > 0, "expected borrow in combo, got {p:?}");
    assert!(p.fbip > 0, "expected fbip in combo, got {p:?}");
    assert!(p.pool_hints > 0, "expected pool hints in combo, got {p:?}");
}

#[test]
fn perceus_debug_combo_reuse_tail() {
    let src = "type Boxed\n    v as i64\n\n*rebuild(x as Boxed) returns Boxed\n    Boxed(v is x.v + 1)\n\n*main() returns i32\n    x is rc(10)\n    y is rc(20)\n    log(@y)\n    bx is Boxed(v is 0)\n    b2 is rebuild(bx)\n    log(b2.v)\n    0\n";
    let (perceus, mir, stderr) = compile_and_collect_summaries(src);
    let p = require_perceus(perceus, &stderr);
    let m = require_mir(mir, &stderr);
    assert!(p.reuse > 0, "expected reuse in combo, got {p:?}");
    assert!(p.tail_reuse > 0, "expected tail-reuse in combo, got {p:?}");
    assert!(m.reuse > 0, "expected MIR reuse in combo, got {m:?}");
    assert!(
        m.tail_reuse > 0,
        "expected MIR tail-reuse in combo, got {m:?}"
    );
}
