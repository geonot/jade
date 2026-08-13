use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile_and_run(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    let out = dir.path().join("test_bin");
    std::fs::write(&jinn, src).unwrap();
    let status = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jinnc failed to start");
    assert!(status.success(), "jinnc compilation failed for:\n{src}");
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        "binary exited with {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn expect(src: &str, expected: &str) {
    let got = compile_and_run(src);
    assert_eq!(got.trim(), expected.trim(), "source:\n{src}");
}

#[test]
fn length_is_scalar_count_ascii() {
    expect("*main\n    log(\"hello\".length)", "5");
}

#[test]
fn length_is_scalar_count_accented() {
    expect("*main\n    log(\"héllo\".length)", "5");
}

#[test]
fn length_is_scalar_count_emoji() {
    expect("*main\n    log(\"😀\".length)", "1");
    expect("*main\n    log(\"a😀b\".length)", "3");
}

#[test]
fn length_empty() {
    expect("*main\n    log(\"\".length)", "0");
}

#[test]
fn len_method_matches_length() {
    expect("*main\n    log(\"héllo\".len())", "5");
}

#[test]
fn byte_count_is_storage_length() {
    expect("*main\n    log(\"hello\".byte_count)", "5");
    expect("*main\n    log(\"héllo\".byte_count)", "6");
    expect("*main\n    log(\"😀\".byte_count)", "4");
}

#[test]
fn byte_count_diverges_from_length_for_multibyte() {
    let src = "*main\n    s is \"héllo\"\n    log(s.byte_count - s.length)";
    expect(src, "1");
}

#[test]
fn char_at_returns_bytes() {
    expect("*main\n    log(\"héllo\".char_at(0))", "104");
    expect("*main\n    log(\"héllo\".char_at(1))", "195");
    expect("*main\n    log(\"héllo\".char_at(2))", "169");
}

#[test]
fn index_matches_char_at() {
    expect("*main\n    s is \"héllo\"\n    log(s[1])", "195");
}

#[test]
fn byte_iteration_uses_byte_count() {
    let src = "*main\n    s is \"héllo\"\n    sum is 0\n    \
        for i from 0 to s.byte_count\n        sum is sum + s.char_at(i)\n    log(sum)";
    let expected = ('h' as i64 + 195 + 169 + 'l' as i64 + 'l' as i64 + 'o' as i64).to_string();
    expect(src, &expected);
}

#[test]
fn slice_is_byte_based() {
    expect("*main\n    log(\"héllo\".slice(0, 1))", "h");
    expect("*main\n    log(\"héllo\".slice(3, 6))", "llo");
}

#[test]
fn equality_is_byte_exact() {
    expect(
        "*main\n    if \"héllo\" equals \"héllo\"\n        log(1)\n    else\n        log(0)",
        "1",
    );
}

#[test]
fn concat_preserves_utf8() {
    expect("*main\n    log(\"hé\" + \"llo\")", "héllo");
    expect("*main\n    log((\"hé\" + \"llo\").length)", "5");
}

#[test]
fn contains_finds_multibyte_substring() {
    expect(
        "*main\n    if \"héllo\".contains(\"éll\")\n        log(1)\n    else\n        log(0)",
        "1",
    );
}

#[test]
fn upper_lower_ascii_only() {
    expect("*main\n    log(\"héllo\".to_upper())", "HéLLO");
    expect("*main\n    log(\"HÉLLO\".to_lower())", "hÉllo");
}

#[test]
fn chr_encodes_ascii() {
    expect("*main\n    log(chr(65) + chr(66) + chr(67))", "ABC");
}

#[test]
fn chr_utf8_encodes_two_byte_scalar() {
    expect(
        "*main\n    s is chr(233)\n    log(s.byte_count)\n    log(s.char_at(0))\n    log(s.char_at(1))\n    log(s)",
        "2\n195\n169\né",
    );
}

#[test]
fn chr_utf8_encodes_four_byte_scalar() {
    expect(
        "*main\n    s is chr(128512)\n    log(s.byte_count)\n    log(s.length)\n    log(s)",
        "4\n1\n😀",
    );
}

#[test]
fn chr_replaces_invalid_scalars() {
    expect(
        "*main\n    a is chr(55296)\n    b is chr(1114112)\n    log(a.byte_count)\n    log(a equals b ? 1 ! 0)\n    log(a equals chr(65533) ? 1 ! 0)",
        "3\n1\n1",
    );
}

#[test]
fn byte_builds_raw_bytes() {
    expect(
        "*main\n    s is byte(200)\n    log(s.byte_count)\n    log(s.char_at(0))\n    t is byte(456)\n    log(t.char_at(0))",
        "1\n200\n200",
    );
}

#[test]
fn chr_rejects_non_integer_argument() {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("bad.jn");
    std::fs::write(&jinn, "*main\n    log(chr(\"x\"))\n").unwrap();
    let out = Command::new(jinnc())
        .arg(&jinn)
        .arg("--emit-hir")
        .output()
        .expect("jinnc failed to start");
    assert!(!out.status.success(), "chr(\"x\") must not type-check");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("integer"),
        "diagnostic must mention the integer requirement: {stderr}"
    );
}

#[test]
fn uuid_v7_formats_after_byte_fix() {
    expect(
        "use uuid\nuse strings\n\n*main\n    u is uuid.v7()\n    log(u.byte_count)\n    log(u.char_at(14) equals 55 ? 1 ! 0)\n    log(uuid.is_valid(u) ? 1 ! 0)",
        "36\n1\n1",
    );
}

#[test]
fn url_percent_round_trips_non_ascii() {
    expect(
        "use url\n\n*main\n    e is url.percent_encode(\"é\")\n    log(e)\n    d is url.percent_decode(e)\n    log(d equals \"é\" ? 1 ! 0)",
        "%C3%A9\n1",
    );
}

#[test]
fn codec_hex_round_trips_high_bytes() {
    expect(
        "use codec\n\n*main\n    raw is codec.from_hex(\"c3a9\")\n    log(raw.byte_count)\n    log(codec.to_hex(raw))",
        "2\nc3a9",
    );
}

#[test]
fn strings_case_mapping_survives_multibyte_input() {
    expect(
        "use strings\n\n*main\n    log(strings.to_lower(\"HéLLO\"))\n    log(strings.to_upper(\"héllo\"))",
        "héllo\nHéLLO",
    );
}
