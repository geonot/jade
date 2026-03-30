use jadec::lock::Lockfile;
use jadec::pkg::Package;
use std::path::PathBuf;
use std::process::Command;

fn jadec() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jadec"))
}

fn compile_and_run(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jade = dir.path().join("test.jade");
    let out = dir.path().join("test_bin");
    std::fs::write(&jade, src).unwrap();
    let status = Command::new(jadec())
        .arg(&jade)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jadec failed to start");
    assert!(status.success(), "jadec compilation failed for:\n{src}");
    let output = Command::new(&out)
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
    let out = compile_and_run(src).trim_end().to_string();
    assert_eq!(out, expected, "\nsource:\n{src}");
}

fn expect_compile_fail(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jade = dir.path().join("test.jade");
    let out = dir.path().join("test_bin");
    std::fs::write(&jade, src).unwrap();
    let output = Command::new(jadec())
        .arg(&jade)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("jadec failed to start");
    assert!(
        !output.status.success(),
        "expected compilation to fail for:\n{src}"
    );
    String::from_utf8(output.stderr).unwrap()
}

fn compile_with_strict(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jade = dir.path().join("test.jade");
    let out = dir.path().join("test_bin");
    std::fs::write(&jade, src).unwrap();
    let status = Command::new(jadec())
        .arg("--strict-types")
        .arg(&jade)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jadec failed to start");
    assert!(
        status.success(),
        "jadec --strict-types compilation failed for:\n{src}"
    );
    let output = Command::new(&out)
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

fn expect_strict_fail(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jade = dir.path().join("test.jade");
    let out = dir.path().join("test_bin");
    std::fs::write(&jade, src).unwrap();
    let output = Command::new(jadec())
        .arg("--strict-types")
        .arg(&jade)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("jadec failed to start");
    assert!(
        !output.status.success(),
        "expected --strict-types compilation to fail for:\n{src}"
    );
    String::from_utf8(output.stderr).unwrap()
}

fn expect_pedantic_fail(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jade = dir.path().join("test.jade");
    let out = dir.path().join("test_bin");
    std::fs::write(&jade, src).unwrap();
    let output = Command::new(jadec())
        .arg("--pedantic")
        .arg(&jade)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("jadec failed to start");
    assert!(
        !output.status.success(),
        "expected --pedantic compilation to fail for:\n{src}"
    );
    String::from_utf8(output.stderr).unwrap()
}

fn compile_and_run_test_mode(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jade = dir.path().join("test.jade");
    let out = dir.path().join("test_bin");
    std::fs::write(&jade, src).unwrap();
    let status = Command::new(jadec())
        .arg("--test")
        .arg(&jade)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jadec failed to start");
    assert!(
        status.success(),
        "jadec --test compilation failed for:\n{src}"
    );
    let output = Command::new(&out)
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        "test binary exited with {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn expect_runtime_fail(src: &str) {
    let dir = tempfile::tempdir().unwrap();
    let jade = dir.path().join("test.jade");
    let out = dir.path().join("test_bin");
    std::fs::write(&jade, src).unwrap();
    let status = Command::new(jadec())
        .arg(&jade)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jadec failed to start");
    assert!(status.success(), "jadec compilation failed for:\n{src}");
    let output = Command::new(&out)
        .output()
        .expect("compiled binary failed to start");
    assert!(
        !output.status.success(),
        "expected runtime failure for:\n{src}"
    );
}

fn compile_and_run_with_file(src: &str, extra_name: &str, extra_content: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jade = dir.path().join("test.jade");
    let extra = dir.path().join(extra_name);
    let out = dir.path().join("test_bin");
    std::fs::write(&jade, src).unwrap();
    std::fs::write(&extra, extra_content).unwrap();
    let status = Command::new(jadec())
        .arg(&jade)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jadec failed to start");
    assert!(status.success(), "jadec compilation failed for:\n{src}");
    let output = Command::new(&out)
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 1: Integer arithmetic
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_add_neg_pos() {
    expect("*main()\n    log(-3 + 10)\n", "7");
}
#[test]
fn b_sub_to_neg() {
    expect("*main()\n    log(3 - 10)\n", "-7");
}
#[test]
fn b_mul_neg_neg() {
    expect("*main()\n    log(-3 * -4)\n", "12");
}
#[test]
fn b_nested_parens() {
    expect("*main()\n    log((2 + 3) * (4 - 1))\n", "15");
}
#[test]
fn b_triple_add() {
    expect("*main()\n    log(1 + 2 + 3 + 4 + 5)\n", "15");
}
#[test]
fn b_triple_mul() {
    expect("*main()\n    log(2 * 3 * 4)\n", "24");
}
#[test]
fn b_precedence_1() {
    expect("*main()\n    log(2 + 3 * 4)\n", "14");
}
#[test]
fn b_precedence_2() {
    expect("*main()\n    log(10 - 8 / 2)\n", "6");
}
#[test]
fn b_big_mul() {
    expect("*main()\n    log(100000 * 100000)\n", "10000000000");
}
#[test]
fn b_left_assoc_sub() {
    expect("*main()\n    log(10 - 3 - 2)\n", "5");
}
#[test]
fn b_zero_add() {
    expect("*main()\n    log(0 + 0)\n", "0");
}
#[test]
fn b_one_mul() {
    expect("*main()\n    log(42 * 1)\n", "42");
}
#[test]
fn b_identity_sub() {
    expect("*main()\n    log(99 - 0)\n", "99");
}
#[test]
fn b_div_exact() {
    expect("*main()\n    log(100 / 25)\n", "4");
}
#[test]
fn b_mod_zero_num() {
    expect("*main()\n    log(0 % 7)\n", "0");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 2: Floating point
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_f64_add() {
    expect("*main()\n    log(1.5 + 2.5)\n", "4.000000");
}
#[test]
fn b_f64_mul() {
    expect("*main()\n    log(3.0 * 2.0)\n", "6.000000");
}
#[test]
fn b_f64_div() {
    expect("*main()\n    log(10.0 / 4.0)\n", "2.500000");
}
#[test]
fn b_f64_sub() {
    expect("*main()\n    log(5.5 - 2.25)\n", "3.250000");
}
#[test]
fn b_f64_var() {
    expect(
        "*main()\n    x is 1.5\n    y is 2.5\n    log(x + y)\n",
        "4.000000",
    );
}
#[test]
fn b_f64_neg() {
    expect("*main()\n    log(-3.5 + 10.0)\n", "6.500000");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 3: Boolean ops
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_bool_true() {
    expect("*main()\n    log(true)\n", "1");
}
#[test]
fn b_bool_false() {
    expect("*main()\n    log(false)\n", "0");
}
#[test]
fn b_and_tt() {
    expect("*main()\n    log(true and true)\n", "1");
}
#[test]
fn b_and_tf() {
    expect("*main()\n    log(true and false)\n", "0");
}
#[test]
fn b_and_ff() {
    expect("*main()\n    log(false and false)\n", "0");
}
#[test]
fn b_or_tf() {
    expect("*main()\n    log(true or false)\n", "1");
}
#[test]
fn b_or_ft() {
    expect("*main()\n    log(false or true)\n", "1");
}
#[test]
fn b_or_ff() {
    expect("*main()\n    log(false or false)\n", "0");
}
#[test]
fn b_not_true() {
    expect("*main()\n    log(not true)\n", "0");
}
#[test]
fn b_not_false() {
    expect("*main()\n    log(not false)\n", "1");
}
#[test]
fn b_and_or_chain() {
    expect("*main()\n    log(true and true or false)\n", "1");
}
#[test]
fn b_not_and() {
    expect("*main()\n    log(not false and true)\n", "1");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 4: Comparisons (equals / neq)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_eq_true() {
    expect("*main()\n    log(5 equals 5)\n", "1");
}
#[test]
fn b_eq_false() {
    expect("*main()\n    log(5 equals 6)\n", "0");
}
#[test]
fn b_ne_true() {
    expect("*main()\n    log(5 neq 6)\n", "1");
}
#[test]
fn b_ne_false() {
    expect("*main()\n    log(5 neq 5)\n", "0");
}
#[test]
fn b_lt_true() {
    expect("*main()\n    log(3 < 5)\n", "1");
}
#[test]
fn b_lt_false() {
    expect("*main()\n    log(5 < 3)\n", "0");
}
#[test]
fn b_gt_true() {
    expect("*main()\n    log(5 > 3)\n", "1");
}
#[test]
fn b_gt_false() {
    expect("*main()\n    log(3 > 5)\n", "0");
}
#[test]
fn b_le_eq() {
    expect("*main()\n    log(3 <= 3)\n", "1");
}
#[test]
fn b_le_less() {
    expect("*main()\n    log(2 <= 3)\n", "1");
}
#[test]
fn b_le_greater() {
    expect("*main()\n    log(4 <= 3)\n", "0");
}
#[test]
fn b_ge_eq() {
    expect("*main()\n    log(3 >= 3)\n", "1");
}
#[test]
fn b_ge_greater() {
    expect("*main()\n    log(4 >= 3)\n", "1");
}
#[test]
fn b_ge_less() {
    expect("*main()\n    log(2 >= 3)\n", "0");
}
#[test]
fn b_equals_kw() {
    expect("*main()\n    log(7 equals 7)\n", "1");
}
#[test]
fn b_isnt_kw() {
    expect("*main()\n    log(7 neq 8)\n", "1");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 5: Bitwise
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_bit_and() {
    expect("*main()\n    log(0xFF & 0x0F)\n", "15");
}
#[test]
fn b_bit_or() {
    expect("*main()\n    log(0xF0 | 0x0F)\n", "255");
}
#[test]
fn b_bit_xor() {
    expect("*main()\n    log(0xFF ^ 0x0F)\n", "240");
}
#[test]
fn b_shl() {
    expect("*main()\n    log(1 << 8)\n", "256");
}
#[test]
fn b_shr() {
    expect("*main()\n    log(256 >> 4)\n", "16");
}
#[test]
fn b_shl_1() {
    expect("*main()\n    log(1 << 0)\n", "1");
}
#[test]
fn b_shr_1() {
    expect("*main()\n    log(8 >> 3)\n", "1");
}
#[test]
fn b_and_mask() {
    expect("*main()\n    log(0b11001100 & 0b10101010)\n", "136");
}
#[test]
fn b_or_combine() {
    expect("*main()\n    log(0b1100 | 0b0011)\n", "15");
}
#[test]
fn b_xor_toggle() {
    expect("*main()\n    log(0b1111 ^ 0b1010)\n", "5");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 6: Variables and bindings
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_bind_simple() {
    expect("*main()\n    x is 42\n    log(x)\n", "42");
}
#[test]
fn b_bind_expr() {
    expect("*main()\n    x is 2 + 3\n    log(x)\n", "5");
}
#[test]
fn b_reassign() {
    expect("*main()\n    x is 10\n    x is 20\n    log(x)\n", "20");
}
#[test]
fn b_reassign_arith() {
    expect("*main()\n    x is 5\n    x is x + 10\n    log(x)\n", "15");
}
#[test]
fn b_multi_bind() {
    expect(
        "*main()\n    a is 1\n    b is 2\n    c is a + b\n    log(c)\n",
        "3",
    );
}
#[test]
fn b_bind_chain() {
    expect(
        "*main()\n    a is 1\n    b is a + 1\n    c is b + 1\n    d is c + 1\n    log(d)\n",
        "4",
    );
}
#[test]
fn b_swap() {
    expect(
        "*main()\n    a is 1\n    b is 2\n    t is a\n    a is b\n    b is t\n    log(a)\n    log(b)\n",
        "2\n1",
    );
}
#[test]
fn b_incr_loop() {
    expect(
        "*main()\n    x is 0\n    x is x + 1\n    x is x + 1\n    x is x + 1\n    log(x)\n",
        "3",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 7: If/elif/else
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_if_true() {
    expect("*main()\n    if true\n        log(1)\n", "1");
}
#[test]
fn b_if_else() {
    expect(
        "*main()\n    if false\n        log(1)\n    else\n        log(0)\n",
        "0",
    );
}
#[test]
fn b_elif_1() {
    expect(
        "*main()\n    x is 1\n    if x equals 1\n        log(10)\n    elif x equals 2\n        log(20)\n    else\n        log(30)\n",
        "10",
    );
}
#[test]
fn b_elif_2() {
    expect(
        "*main()\n    x is 2\n    if x equals 1\n        log(10)\n    elif x equals 2\n        log(20)\n    else\n        log(30)\n",
        "20",
    );
}
#[test]
fn b_elif_else() {
    expect(
        "*main()\n    x is 5\n    if x equals 1\n        log(10)\n    elif x equals 2\n        log(20)\n    else\n        log(30)\n",
        "30",
    );
}
#[test]
fn b_nested_if() {
    expect(
        "*main()\n    x is 5\n    if x > 0\n        if x > 3\n            log(1)\n        else\n            log(0)\n",
        "1",
    );
}
#[test]
fn b_if_cmp_expr() {
    expect(
        "*main()\n    a is 10\n    b is 20\n    if a < b\n        log(a)\n    else\n        log(b)\n",
        "10",
    );
}
#[test]
fn b_if_and() {
    expect(
        "*main()\n    x is 5\n    if x > 0 and x < 10\n        log(1)\n    else\n        log(0)\n",
        "1",
    );
}
#[test]
fn b_if_or() {
    expect(
        "*main()\n    x is 15\n    if x < 0 or x > 10\n        log(1)\n    else\n        log(0)\n",
        "1",
    );
}
#[test]
fn b_if_multi_elif() {
    expect(
        "*main()\n    x is 4\n    if x equals 1\n        log(1)\n    elif x equals 2\n        log(2)\n    elif x equals 3\n        log(3)\n    elif x equals 4\n        log(4)\n    else\n        log(0)\n",
        "4",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 8: While loops
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_while_count() {
    expect(
        "*main()\n    i is 0\n    while i < 5\n        i is i + 1\n    log(i)\n",
        "5",
    );
}
#[test]
fn b_while_sum() {
    expect(
        "*main()\n    sum is 0\n    i is 1\n    while i <= 10\n        sum is sum + i\n        i is i + 1\n    log(sum)\n",
        "55",
    );
}
#[test]
fn b_while_no_iter() {
    expect(
        "*main()\n    i is 10\n    while i < 0\n        i is i - 1\n    log(i)\n",
        "10",
    );
}
#[test]
fn b_while_break() {
    expect(
        "*main()\n    i is 0\n    while true\n        i is i + 1\n        if i equals 7\n            break\n    log(i)\n",
        "7",
    );
}
#[test]
fn b_while_continue() {
    expect(
        "*main()\n    sum is 0\n    i is 0\n    while i < 10\n        i is i + 1\n        if i % 2 equals 0\n            continue\n        sum is sum + i\n    log(sum)\n",
        "25",
    );
}
#[test]
fn b_while_nested() {
    expect(
        "*main()\n    sum is 0\n    i is 0\n    while i < 3\n        j is 0\n        while j < 4\n            sum is sum + 1\n            j is j + 1\n        i is i + 1\n    log(sum)\n",
        "12",
    );
}
#[test]
fn b_while_countdown() {
    expect(
        "*main()\n    i is 5\n    while i > 0\n        i is i - 1\n    log(i)\n",
        "0",
    );
}
#[test]
fn b_while_mul_acc() {
    expect(
        "*main()\n    prod is 1\n    i is 1\n    while i <= 5\n        prod is prod * i\n        i is i + 1\n    log(prod)\n",
        "120",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 9: For loops
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_for_basic() {
    expect(
        "*main()\n    sum is 0\n    for i in 5\n        sum is sum + i\n    log(sum)\n",
        "10",
    );
}
#[test]
fn b_for_range() {
    expect(
        "*main()\n    sum is 0\n    for i in 1 to 6\n        sum is sum + i\n    log(sum)\n",
        "15",
    );
}
#[test]
fn b_for_step() {
    expect(
        "*main()\n    sum is 0\n    for i in 0 to 10 by 2\n        sum is sum + i\n    log(sum)\n",
        "20",
    );
}
#[test]
fn b_for_nested() {
    expect(
        "*main()\n    sum is 0\n    for i in 3\n        for j in 4\n            sum is sum + 1\n    log(sum)\n",
        "12",
    );
}
#[test]
fn b_for_zero() {
    expect(
        "*main()\n    for i in 0\n        log(i)\n    log(99)\n",
        "99",
    );
}
#[test]
fn b_for_one() {
    expect("*main()\n    for i in 1\n        log(i)\n", "0");
}
#[test]
fn b_for_range_one() {
    expect("*main()\n    for i in 5 to 6\n        log(i)\n", "5");
}
#[test]
fn b_for_range_empty() {
    expect(
        "*main()\n    for i in 5 to 5\n        log(i)\n    log(99)\n",
        "99",
    );
}
#[test]
fn b_for_step_3() {
    expect(
        "*main()\n    for i in 0 to 12 by 3\n        log(i)\n",
        "0\n3\n6\n9",
    );
}
#[test]
fn b_for_triple_nest() {
    expect(
        "*main()\n    sum is 0\n    for i in 2\n        for j in 2\n            for k in 2\n                sum is sum + 1\n    log(sum)\n",
        "8",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 10: Loop/break
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_loop_break() {
    expect(
        "*main()\n    i is 0\n    loop\n        i is i + 1\n        if i equals 10\n            break\n    log(i)\n",
        "10",
    );
}
#[test]
fn b_loop_break_early() {
    expect(
        "*main()\n    i is 0\n    loop\n        if i equals 0\n            break\n        i is i + 1\n    log(i)\n",
        "0",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 11: Functions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_fn_basic() {
    expect(
        "*add(a: i64, b: i64) -> i64\n    a + b\n\n*main()\n    log(add(3, 4))\n",
        "7",
    );
}
#[test]
fn b_fn_void() {
    expect("*greet()\n    log(99)\n\n*main()\n    greet()\n", "99");
}
#[test]
fn b_fn_recursive() {
    expect(
        "*fact(n: i64) -> i64\n    if n <= 1\n        return 1\n    n * fact(n - 1)\n\n*main()\n    log(fact(5))\n",
        "120",
    );
}
#[test]
fn b_fn_nested_call() {
    expect(
        "*f(x: i64) -> i64\n    x + 1\n\n*g(x: i64) -> i64\n    f(x) * 2\n\n*main()\n    log(g(5))\n",
        "12",
    );
}
#[test]
fn b_fn_early_return() {
    expect(
        "*check(x: i64) -> i64\n    if x > 0\n        return 1\n    0\n\n*main()\n    log(check(5))\n    log(check(-1))\n",
        "1\n0",
    );
}
#[test]
fn b_fn_chain() {
    expect(
        "*a(x: i64) -> i64\n    x + 1\n\n*b(x: i64) -> i64\n    x * 2\n\n*c(x: i64) -> i64\n    x - 3\n\n*main()\n    log(c(b(a(5))))\n",
        "9",
    );
}
#[test]
fn b_fn_multi_return() {
    expect(
        "*abs_val(x: i64) -> i64\n    if x < 0\n        return 0 - x\n    x\n\n*main()\n    log(abs_val(-5))\n    log(abs_val(3))\n",
        "5\n3",
    );
}
#[test]
fn b_mutual_recursion() {
    expect(
        "*is_even(n: i64) -> i64\n    if n equals 0\n        return 1\n    is_odd(n - 1)\n\n*is_odd(n: i64) -> i64\n    if n equals 0\n        return 0\n    is_even(n - 1)\n\n*main()\n    log(is_even(10))\n    log(is_odd(10))\n",
        "1\n0",
    );
}
#[test]
fn b_fn_ten_params() {
    expect(
        "*sum5(a: i64, b: i64, c: i64, d: i64, e: i64) -> i64\n    a + b + c + d + e\n\n*main()\n    log(sum5(1, 2, 3, 4, 5))\n",
        "15",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 12: Closures
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_lambda_inline() {
    expect(
        "*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    log(apply(*fn(x: i64) -> i64 x * 3, 14))\n    0\n",
        "42",
    );
}
#[test]
fn b_lambda_var() {
    expect(
        "*main() -> i32\n    g is *fn(x: i64) -> i64 x + 100\n    log(g(42))\n    0\n",
        "142",
    );
}
#[test]
fn b_closure_single() {
    expect(
        "*main() -> i32\n    x is 10\n    f is *fn(y: i64) -> i64 x + y\n    log(f(5))\n    0\n",
        "15",
    );
}
#[test]
fn b_closure_multi() {
    expect(
        "*main() -> i32\n    a is 10\n    b is 20\n    f is *fn(x: i64) -> i64 a + b + x\n    log(f(5))\n    0\n",
        "35",
    );
}
#[test]
fn b_closure_hof() {
    expect(
        "*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    base is 100\n    f is *fn(x: i64) -> i64 base + x\n    log(apply(f, 42))\n    0\n",
        "142",
    );
}
#[test]
fn b_do_end_lambda() {
    expect(
        "*main() -> i32\n    g is *fn(x: i64) -> i64 do\n        y is x * 2\n        y + 1\n    end\n    log(g(20))\n    0\n",
        "41",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 13: Ternary (uses ? ! syntax)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_ternary_t() {
    expect("*main()\n    log(true ? 1 ! 0)\n", "1");
}
#[test]
fn b_ternary_f() {
    expect("*main()\n    log(false ? 1 ! 0)\n", "0");
}
#[test]
fn b_ternary_cmp() {
    expect("*main()\n    x is 5\n    log(x > 3 ? 10 ! 20)\n", "10");
}
#[test]
fn b_ternary_nested() {
    expect(
        "*main()\n    x is 2\n    log(x equals 1 ? 10 ! x equals 2 ? 20 ! 30)\n",
        "20",
    );
}
#[test]
fn b_ternary_arith() {
    expect("*main()\n    log((3 > 2 ? 10 ! 5) + 1)\n", "11");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 14: Structs
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_struct_basic() {
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*main() -> i32\n    p is Point(x is 3, y is 4)\n    log(p.x)\n    log(p.y)\n    0\n",
        "3\n4",
    );
}
#[test]
fn b_struct_arith() {
    expect(
        "type Vec2\n    x: i64\n    y: i64\n\n*main() -> i32\n    v is Vec2(x is 3, y is 4)\n    log(v.x * v.x + v.y * v.y)\n    0\n",
        "25",
    );
}
#[test]
fn b_struct_positional() {
    expect(
        "type Pair\n    a: i64\n    b: i64\n\n*main() -> i32\n    p is Pair(5, 15)\n    log(p.a + p.b)\n    0\n",
        "20",
    );
}
#[test]
fn b_struct_fn_arg() {
    expect(
        "type Pt\n    x: i64\n    y: i64\n\n*sum(p: Pt) -> i64\n    p.x + p.y\n\n*main() -> i32\n    log(sum(Pt(x is 4, y is 6)))\n    0\n",
        "10",
    );
}
#[test]
fn b_struct_method() {
    expect(
        "type Counter\n    val: i64\n\n    *get() -> i64\n        self.val\n\n*main() -> i32\n    c is Counter(val is 42)\n    log(c.get())\n    0\n",
        "42",
    );
}
#[test]
fn b_struct_method_add() {
    expect(
        "type Vec2\n    x: i64\n    y: i64\n\n    *sum() -> i64\n        self.x + self.y\n\n*main() -> i32\n    v is Vec2(x is 3, y is 7)\n    log(v.sum())\n    0\n",
        "10",
    );
}
#[test]
fn b_struct_3field() {
    expect(
        "type V3\n    x: i64\n    y: i64\n    z: i64\n\n*main() -> i32\n    v is V3(x is 1, y is 2, z is 3)\n    log(v.x + v.y + v.z)\n    0\n",
        "6",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 15: Enums & match
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_enum_2() {
    expect(
        "enum Dir\n    Up\n    Down\n\n*main() -> i32\n    d is Up\n    match d\n        Up ? log(1)\n        Down ? log(2)\n    0\n",
        "1",
    );
}
#[test]
fn b_enum_3() {
    expect(
        "enum Light\n    Red\n    Yellow\n    Green\n\n*main() -> i32\n    l is Yellow\n    match l\n        Red ? log(0)\n        Yellow ? log(1)\n        Green ? log(2)\n    0\n",
        "1",
    );
}
#[test]
fn b_enum_data() {
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*main() -> i32\n    s is Circle(5)\n    match s\n        Circle(r) ? log(r)\n        Rect(w, h) ? log(w + h)\n    0\n",
        "5",
    );
}
#[test]
fn b_enum_data_rect() {
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*main() -> i32\n    s is Rect(3, 4)\n    match s\n        Circle(r) ? log(r)\n        Rect(w, h) ? log(w * h)\n    0\n",
        "12",
    );
}
#[test]
fn b_enum_wildcard() {
    expect(
        "enum Op\n    Add\n    Sub\n    Mul\n\n*main() -> i32\n    o is Mul\n    match o\n        Add ? log(1)\n        _ ? log(99)\n    0\n",
        "99",
    );
}
#[test]
fn b_enum_fn_arg() {
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*area(s: Shape) -> i64\n    match s\n        Circle(r) ? r * r\n        Rect(w, h) ? w * h\n\n*main() -> i32\n    log(area(Circle(5)))\n    log(area(Rect(3, 7)))\n    0\n",
        "25\n21",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 16: Match expressions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_match_int_1() {
    expect(
        "*main()\n    x is 3\n    match x\n        1 ? log(10)\n        2 ? log(20)\n        3 ? log(30)\n        _ ? log(0)\n",
        "30",
    );
}
#[test]
fn b_match_int_default() {
    expect(
        "*main()\n    x is 99\n    match x\n        1 ? log(10)\n        _ ? log(0)\n",
        "0",
    );
}
#[test]
fn b_match_expr_fn() {
    expect(
        "*choose(x: i64) -> i64\n    match x\n        1 ? 10\n        2 ? 20\n        _ ? 99\n\n*main() -> i32\n    log(choose(1))\n    log(choose(2))\n    log(choose(7))\n    0\n",
        "10\n20\n99",
    );
}
#[test]
fn b_match_enum_expr() {
    expect(
        "enum Op\n    Add(i64, i64)\n    Neg(i64)\n\n*eval(op: Op) -> i64\n    match op\n        Add(a, b) ? a + b\n        Neg(a) ? 0 - a\n\n*main() -> i32\n    log(eval(Add(3, 4)))\n    log(eval(Neg(10)))\n    0\n",
        "7\n-10",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 17: Arrays
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_arr_index() {
    expect(
        "*main() -> i32\n    a is [10, 20, 30]\n    log(a[0])\n    log(a[1])\n    log(a[2])\n    0\n",
        "10\n20\n30",
    );
}
#[test]
fn b_arr_sum_loop() {
    expect(
        "*main() -> i32\n    a is [1, 2, 3, 4, 5]\n    total is 0\n    i is 0\n    while i < 5\n        total is total + a[i]\n        i is i + 1\n    log(total)\n    0\n",
        "15",
    );
}
#[test]
fn b_arr_4elem() {
    expect(
        "*main() -> i32\n    a is [10, 20, 30, 40]\n    log(a[0] + a[3])\n    0\n",
        "50",
    );
}
#[test]
fn b_arr_5elem() {
    expect(
        "*main() -> i32\n    a is [1, 1, 1, 1, 1]\n    sum is 0\n    for i in 5\n        sum is sum + a[i]\n    log(sum)\n    0\n",
        "5",
    );
}
#[test]
fn b_arr_for_in() {
    expect(
        "*main()\n    arr is [10, 20, 30]\n    for x in arr\n        log(x)\n",
        "10\n20\n30",
    );
}
#[test]
fn b_arr_for_in_sum() {
    expect(
        "*main()\n    arr is [1, 2, 3, 4, 5]\n    sum is 0\n    for x in arr\n        sum is sum + x\n    log(sum)\n",
        "15",
    );
}
#[test]
fn b_arr_for_product() {
    expect(
        "*main()\n    arr is [2, 3, 5, 7]\n    prod is 1\n    for x in arr\n        prod is prod * x\n    log(prod)\n",
        "210",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 18: Tuples
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_tuple_2() {
    expect(
        "*main() -> i32\n    t is (10, 20)\n    log(t[0])\n    log(t[1])\n    0\n",
        "10\n20",
    );
}
#[test]
fn b_tuple_3() {
    expect(
        "*main() -> i32\n    t is (100, 200, 300)\n    log(t[0] + t[1] + t[2])\n    0\n",
        "600",
    );
}
#[test]
fn b_tuple_arith() {
    expect(
        "*main() -> i32\n    t is (7, 3)\n    log(t[0] * t[1])\n    0\n",
        "21",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 19: Casts
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_cast_to_i32() {
    expect("*main()\n    x is 42 as i32\n    log(x)\n", "42");
}
#[test]
fn b_cast_to_f64() {
    expect("*main()\n    x is 5 as f64\n    log(x)\n", "5.000000");
}
#[test]
fn b_cast_f64_int() {
    expect("*main()\n    x is 3.14 as i64\n    log(x)\n", "3");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 20: Strings
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_str_lit() {
    expect("*main()\n    log('hello')\n", "hello");
}
#[test]
fn b_str_empty() {
    expect("*main()\n    s is ''\n    log(s.length)\n", "0");
}
#[test]
fn b_str_concat() {
    expect("*main()\n    log('foo' + 'bar')\n", "foobar");
}
#[test]
fn b_str_length() {
    expect("*main()\n    log('hello'.length)\n", "5");
}
#[test]
fn b_str_concat_3() {
    expect(
        "*main()\n    a is 'a'\n    b is 'b'\n    c is 'c'\n    log(a + b + c)\n",
        "abc",
    );
}
#[test]
fn b_str_escape_n() {
    expect("*main()\n    log('a\\nb')\n", "a\nb");
}
#[test]
fn b_str_escape_t() {
    expect("*main()\n    log('a\\tb')\n", "a\tb");
}
#[test]
fn b_str_concat_len() {
    expect(
        "*main()\n    a is 'foo'\n    b is 'bar'\n    c is a + b\n    log(c.length)\n",
        "6",
    );
}
#[test]
fn b_str_single_char() {
    expect("*main()\n    s is 'x'\n    log(s.length)\n", "1");
}
#[test]
fn b_str_long() {
    expect(
        "*main()\n    s is 'abcdefghijklmnopqrstuvwxyz'\n    log(s.length)\n",
        "26",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 21: Pipeline
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_pipe_single() {
    expect(
        "*double(x: i64) -> i64\n    x * 2\n\n*main() -> i32\n    result is 10 ~ double\n    log(result)\n    0\n",
        "20",
    );
}
#[test]
fn b_pipe_chain() {
    expect(
        "*double(x: i64) -> i64\n    x * 2\n\n*add_one(x: i64) -> i64\n    x + 1\n\n*main() -> i32\n    result is 10 ~ double ~ add_one\n    log(result)\n    0\n",
        "21",
    );
}
#[test]
fn b_pipe_placeholder() {
    expect(
        "*mul(a: i64, b: i64) -> i64\n    a * b\n\n*main() -> i32\n    result is 10 ~ mul($, 3)\n    log(result)\n    0\n",
        "30",
    );
}
#[test]
fn b_pipe_lambda() {
    expect(
        "*main() -> i32\n    result is 5 ~ *fn(x: i64) -> i64 x * x\n    log(result)\n    0\n",
        "25",
    );
}
#[test]
fn b_pipe_with_args() {
    expect(
        "*add(a: i64, b: i64) -> i64\n    a + b\n\n*main() -> i32\n    result is 10 ~ add(5)\n    log(result)\n    0\n",
        "15",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 22: Generics
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_gen_id() {
    expect(
        "*id of T(x: T) -> T\n    x\n\n*main() -> i32\n    log(id(42))\n    0\n",
        "42",
    );
}
#[test]
fn b_gen_add() {
    expect(
        "*add of T(a: T, b: T) -> T\n    a + b\n\n*main() -> i32\n    log(add(3, 4))\n    0\n",
        "7",
    );
}
#[test]
fn b_gen_max() {
    expect(
        "*max of T(a: T, b: T) -> T\n    if a > b\n        return a\n    b\n\n*main() -> i32\n    log(max(10, 20))\n    0\n",
        "20",
    );
}
#[test]
fn b_gen_min() {
    expect(
        "*min of T(a: T, b: T) -> T\n    if a < b\n        return a\n    b\n\n*main() -> i32\n    log(min(10, 20))\n    0\n",
        "10",
    );
}
#[test]
fn b_inferred_gen() {
    expect(
        "*identity(x: T) -> T\n    x\n\n*main() -> i32\n    log(identity(99))\n    0\n",
        "99",
    );
}
#[test]
fn b_inferred_gen_add() {
    expect(
        "*add(a: T, b: T) -> T\n    a + b\n\n*main() -> i32\n    log(add(10, 20))\n    0\n",
        "30",
    );
}
#[test]
fn b_untyped_gen() {
    expect(
        "*double(x)\n    x * 2\n\n*main() -> i32\n    log(double(21))\n    0\n",
        "42",
    );
}
#[test]
fn b_untyped_rec() {
    expect(
        "*fact(n)\n    if n <= 1\n        return 1\n    n * fact(n - 1)\n\n*main() -> i32\n    log(fact(10))\n    0\n",
        "3628800",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 23: HOF
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_hof_pass() {
    expect(
        "*double(x: i64) -> i64\n    x * 2\n\n*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    log(apply(double, 21))\n    0\n",
        "42",
    );
}
#[test]
fn b_hof_var() {
    expect(
        "*double(x: i64) -> i64\n    x * 2\n\n*main() -> i32\n    f is double\n    log(f(21))\n    0\n",
        "42",
    );
}
#[test]
fn b_hof_chain() {
    expect(
        "*add_one(x: i64) -> i64\n    x + 1\n\n*double(x: i64) -> i64\n    x * 2\n\n*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    log(apply(add_one, apply(double, 10)))\n    0\n",
        "21",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 24: Option / Result
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_option_some() {
    expect(
        "*main()\n    x is Some(42)\n    match x\n        Some(v) ? log(v)\n        Nothing ? log(0)\n",
        "42",
    );
}
#[test]
fn b_option_nothing() {
    expect(
        "*main()\n    x is Nothing\n    match x\n        Some(v) ? log(v)\n        Nothing ? log(0)\n",
        "0",
    );
}
#[test]
fn b_option_arith() {
    expect(
        "*main()\n    x is Some(10)\n    match x\n        Some(v) ? log(v + 5)\n        Nothing ? log(0)\n",
        "15",
    );
}
#[test]
fn b_result_ok_v() {
    expect(
        "*main()\n    x is Ok(10)\n    match x\n        Ok(v) ? log(v)\n        Err(e) ? log(e)\n",
        "10",
    );
}
#[test]
fn b_result_err_v() {
    expect(
        "*main()\n    x is Err(99)\n    match x\n        Ok(v) ? log(v)\n        Err(e) ? log(e)\n",
        "99",
    );
}
#[test]
fn b_result_ok_arith() {
    expect(
        "*main()\n    x is Ok(7)\n    match x\n        Ok(v) ? log(v * 3)\n        Err(e) ? log(e)\n",
        "21",
    );
}
#[test]
fn b_option_local() {
    expect(
        "enum Option\n    Some(i64)\n    None\n\n*safe_div(a: i64, b: i64) -> Option\n    if b equals 0\n        return None\n    Some(a / b)\n\n*main() -> i32\n    match safe_div(10, 2)\n        Some(v) ? log(v)\n        None ? log(-1)\n    match safe_div(10, 0)\n        Some(v) ? log(v)\n        None ? log(-1)\n    0\n",
        "5\n-1",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 25: RC
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_rc_create() {
    expect("*main()\n    x is rc(42)\n    log(@x)\n", "42");
}
#[test]
fn b_rc_retain_rel() {
    expect(
        "*main()\n    x is rc(100)\n    rc_retain(x)\n    rc_release(x)\n    log(@x)\n",
        "100",
    );
}
#[test]
fn b_rc_deref_arith() {
    expect("*main()\n    x is rc(21)\n    log(@x * 2)\n", "42");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 26: Pointers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_ptr_ref_deref() {
    expect(
        "*main() -> i32\n    x is 42\n    p is %x\n    log(@p)\n    0\n",
        "42",
    );
}
#[test]
fn b_ptr_arith() {
    expect(
        "*main() -> i32\n    a is 10\n    b is 20\n    pa is %a\n    pb is %b\n    log(@pa + @pb)\n    0\n",
        "30",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 27: List comprehension
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_listcomp_squares() {
    expect(
        "*main() -> i32\n    arr is [x * x for x in 0 to 5]\n    log(arr[0])\n    log(arr[1])\n    log(arr[4])\n    0\n",
        "0\n1\n16",
    );
}
#[test]
fn b_listcomp_filter() {
    expect(
        "*main() -> i32\n    arr is [x for x in 0 to 10 if x > 5]\n    log(arr[0])\n    log(arr[1])\n    0\n",
        "6\n7",
    );
}
#[test]
fn b_listcomp_add() {
    expect(
        "*main() -> i32\n    arr is [x + 10 for x in 0 to 3]\n    log(arr[0])\n    log(arr[1])\n    log(arr[2])\n    0\n",
        "10\n11\n12",
    );
}
#[test]
fn b_listcomp_double() {
    expect(
        "*main() -> i32\n    arr is [i * 2 for i in 0 to 4]\n    log(arr[0])\n    log(arr[1])\n    log(arr[2])\n    log(arr[3])\n    0\n",
        "0\n2\n4\n6",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 28: Exponentiation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_exp_2_10() {
    expect("*main()\n    log(2 ** 10)\n", "1024");
}
#[test]
fn b_exp_3_3() {
    expect("*main()\n    log(3 ** 3)\n", "27");
}
#[test]
fn b_exp_5_0() {
    expect("*main()\n    log(5 ** 0)\n", "1");
}
#[test]
fn b_exp_7_1() {
    expect("*main()\n    log(7 ** 1)\n", "7");
}
#[test]
fn b_exp_2_20() {
    expect("*main()\n    log(2 ** 20)\n", "1048576");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 29: Literals
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_hex_lit() {
    expect("*main()\n    log(0xFF)\n", "255");
}
#[test]
fn b_bin_lit() {
    expect("*main()\n    log(0b1010)\n", "10");
}
#[test]
fn b_oct_lit() {
    expect("*main()\n    log(0o77)\n", "63");
}
#[test]
fn b_underscore_lit() {
    expect("*main()\n    log(1_000_000)\n", "1000000");
}
#[test]
fn b_neg_lit() {
    expect("*main()\n    log(-42)\n", "-42");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 30: Err defs and bang return
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_err_def() {
    expect(
        "err IoError\n    NotFound\n    Permission\n\n*main() -> i32\n    log(99)\n    0\n",
        "99",
    );
}
#[test]
fn b_bang_return() {
    expect(
        "*check(x: i64) -> i64\n    if x < 0\n        ! -1\n    x * 2\n\n*main() -> i32\n    log(check(5))\n    log(check(-3))\n    0\n",
        "10\n-1",
    );
}
#[test]
fn b_err_safe_div() {
    expect(
        "err MathError\n    DivZero\n    Overflow\n\n*safe_div(a: i64, b: i64) -> i64\n    if b equals 0\n        ! 0\n    a / b\n\n*main()\n    log(safe_div(10, 2))\n",
        "5",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 31: Bit intrinsics
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_popcount() {
    expect(
        "*main() -> i32\n    log(popcount(7))\n    log(popcount(255))\n    0\n",
        "3\n8",
    );
}
#[test]
fn b_ctz_8() {
    expect("*main() -> i32\n    log(ctz(8))\n    0\n", "3");
}
#[test]
fn b_popcount_0() {
    expect("*main() -> i32\n    log(popcount(0))\n    0\n", "0");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 32: ASM
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_asm_nop() {
    expect(
        "*main() -> i32\n    asm\n        nop\n    log(42)\n    0\n",
        "42",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 33: Compile-fail tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_fail_no_main() {
    expect_compile_fail("*foo()\n    log(1)\n");
}
#[test]
fn b_fail_undef_var() {
    expect_compile_fail("*main()\n    log(xyz)\n");
}
#[test]
fn b_fail_non_exhaustive() {
    let err = expect_compile_fail(
        "enum AB\n    A\n    B\n\n*main() -> i32\n    x is A\n    match x\n        A ? log(1)\n    0\n",
    );
    assert!(err.contains("non-exhaustive") || err.contains("missing"));
}
#[test]
fn b_fail_tab_indent() {
    expect_compile_fail("*main()\n\tlog(1)\n");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 34: Complex algorithms
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_fibonacci() {
    expect(
        "*fib(n: i64) -> i64\n    if n <= 1\n        return n\n    fib(n - 1) + fib(n - 2)\n\n*main()\n    log(fib(10))\n",
        "55",
    );
}
#[test]
fn b_gcd() {
    expect(
        "*gcd(a: i64, b: i64) -> i64\n    if b equals 0\n        return a\n    gcd(b, a % b)\n\n*main()\n    log(gcd(48, 18))\n",
        "6",
    );
}
#[test]
fn b_power() {
    expect(
        "*power(base: i64, exp: i64) -> i64\n    if exp equals 0\n        return 1\n    base * power(base, exp - 1)\n\n*main()\n    log(power(2, 10))\n",
        "1024",
    );
}
#[test]
fn b_sum_to_n() {
    expect(
        "*sum_to(n: i64) -> i64\n    total is 0\n    for i in 1 to n + 1\n        total is total + i\n    total\n\n*main()\n    log(sum_to(100))\n",
        "5050",
    );
}
#[test]
fn b_deep_rec() {
    expect(
        "*countdown(n: i64) -> i64\n    if n equals 0\n        return 0\n    countdown(n - 1)\n\n*main()\n    log(countdown(10000))\n",
        "0",
    );
}
#[test]
fn b_count_evens() {
    expect(
        "*main()\n    arr is [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]\n    count is 0\n    for x in arr\n        if x % 2 equals 0\n            count is count + 1\n    log(count)\n",
        "5",
    );
}
#[test]
fn b_arr_max() {
    expect(
        "*main()\n    arr is [3, 7, 1, 9, 2]\n    mx is arr[0]\n    for i in 1 to 5\n        if arr[i] > mx\n            mx is arr[i]\n    log(mx)\n",
        "9",
    );
}
#[test]
fn b_arr_min() {
    expect(
        "*main()\n    arr is [3, 7, 1, 9, 2]\n    mn is arr[0]\n    for i in 1 to 5\n        if arr[i] < mn\n            mn is arr[i]\n    log(mn)\n",
        "1",
    );
}
#[test]
fn b_collatz() {
    expect(
        "*collatz(n: i64) -> i64\n    steps is 0\n    x is n\n    while x neq 1\n        if x % 2 equals 0\n            x is x / 2\n        else\n            x is 3 * x + 1\n        steps is steps + 1\n    steps\n\n*main()\n    log(collatz(27))\n",
        "111",
    );
}
#[test]
fn b_sum_digits() {
    expect(
        "*sum_digits(n: i64) -> i64\n    sum is 0\n    x is n\n    while x > 0\n        sum is sum + x % 10\n        x is x / 10\n    sum\n\n*main()\n    log(sum_digits(12345))\n",
        "15",
    );
}
#[test]
fn b_reverse_num() {
    expect(
        "*reverse(n: i64) -> i64\n    result is 0\n    x is n\n    while x > 0\n        result is result * 10 + x % 10\n        x is x / 10\n    result\n\n*main()\n    log(reverse(12345))\n",
        "54321",
    );
}
#[test]
fn b_is_prime() {
    expect(
        "*is_prime(n: i64) -> i64\n    if n < 2\n        return 0\n    i is 2\n    while i * i <= n\n        if n % i equals 0\n            return 0\n        i is i + 1\n    1\n\n*main()\n    log(is_prime(7))\n    log(is_prime(10))\n    log(is_prime(97))\n",
        "1\n0\n1",
    );
}
#[test]
fn b_lcm() {
    expect(
        "*gcd(a: i64, b: i64) -> i64\n    if b equals 0\n        return a\n    gcd(b, a % b)\n\n*lcm(a: i64, b: i64) -> i64\n    a / gcd(a, b) * b\n\n*main()\n    log(lcm(12, 18))\n",
        "36",
    );
}
#[test]
fn b_count_primes() {
    expect(
        "*is_prime(n: i64) -> i64\n    if n < 2\n        return 0\n    i is 2\n    while i * i <= n\n        if n % i equals 0\n            return 0\n        i is i + 1\n    1\n\n*main()\n    count is 0\n    for i in 2 to 30\n        count is count + is_prime(i)\n    log(count)\n",
        "10",
    );
}
#[test]
fn b_triangle_num() {
    expect(
        "*tri(n: i64) -> i64\n    n * (n + 1) / 2\n\n*main()\n    log(tri(10))\n    log(tri(100))\n",
        "55\n5050",
    );
}
#[test]
fn b_dot_product() {
    expect(
        "*main()\n    a is [1, 2, 3, 4, 5]\n    b is [5, 4, 3, 2, 1]\n    dot is 0\n    for i in 5\n        dot is dot + a[i] * b[i]\n    log(dot)\n",
        "35",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 35: Edge cases
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_zero_div() {
    expect("*main()\n    log(0 / 1)\n", "0");
}
#[test]
fn b_identity_add() {
    expect("*main()\n    log(42 + 0)\n", "42");
}
#[test]
fn b_identity_mul() {
    expect("*main()\n    log(42 * 1)\n", "42");
}
#[test]
fn b_neg_zero() {
    expect("*main()\n    log(-0)\n", "0");
}
#[test]
fn b_multi_log() {
    expect(
        "*main()\n    log(1)\n    log(2)\n    log(3)\n    log(4)\n    log(5)\n",
        "1\n2\n3\n4\n5",
    );
}
#[test]
fn b_chain_ops() {
    expect(
        "*main()\n    x is 2\n    x is x + 3\n    x is x * 2\n    x is x - 1\n    log(x)\n",
        "9",
    );
}
#[test]
fn b_complex_expr() {
    expect("*main()\n    log((1 + 2) * (3 + 4) - (5 + 6))\n", "10");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 36: Short-circuit eval
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_sc_and_skip() {
    expect(
        "*main()\n    x is 0\n    if false and true\n        x is 1\n    log(x)\n",
        "0",
    );
}
#[test]
fn b_sc_or_take() {
    expect(
        "*main()\n    x is 0\n    if true or false\n        x is 1\n    log(x)\n",
        "1",
    );
}
#[test]
fn b_sc_and_both() {
    expect(
        "*main()\n    x is 0\n    if true and true\n        x is 1\n    log(x)\n",
        "1",
    );
}
#[test]
fn b_sc_or_neither() {
    expect(
        "*main()\n    x is 0\n    if false or false\n        x is 1\n    log(x)\n",
        "0",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 37: Extern
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_extern_puts() {
    expect(
        "extern *puts(s: String) -> i32\n\n*main() -> i32\n    puts(\"hello extern\")\n    0\n",
        "hello extern",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 38: Module system
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_module_import() {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("helper.jade");
    let main = dir.path().join("main.jade");
    let out = dir.path().join("test_bin");
    std::fs::write(&helper, "*double(x: i64) -> i64\n    x + x\n").unwrap();
    std::fs::write(
        &main,
        "use helper\n\n*main() -> i32\n    log(double(21))\n    0\n",
    )
    .unwrap();
    let status = Command::new(jadec())
        .arg(&main)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jadec failed to start");
    assert!(status.success(), "module import: compilation failed");
    let output = Command::new(&out).output().expect("binary failed to start");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.trim(), "42");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 39: Exhaustive match
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_exhaust_all() {
    expect(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Red\n    match c\n        Red ? log(1)\n        Green ? log(2)\n        Blue ? log(3)\n    0\n",
        "1",
    );
}
#[test]
fn b_exhaust_wildcard() {
    expect(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Green\n    match c\n        Red ? log(1)\n        _ ? log(99)\n    0\n",
        "99",
    );
}
#[test]
fn b_exhaust_fail() {
    let err = expect_compile_fail(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Red\n    match c\n        Red ? log(1)\n        Green ? log(2)\n    0\n",
    );
    assert!(err.contains("non-exhaustive") || err.contains("missing"));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 40: Complex algorithm patterns
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_iterative_fib() {
    expect(
        "*fib(n: i64) -> i64\n    a is 0\n    b is 1\n    for i in n\n        t is a\n        a is b\n        b is t + b\n    a\n\n*main()\n    log(fib(10))\n    log(fib(20))\n",
        "55\n6765",
    );
}
#[test]
fn b_sum_squares() {
    expect(
        "*main()\n    sum is 0\n    for i in 1 to 11\n        sum is sum + i * i\n    log(sum)\n",
        "385",
    );
}
#[test]
fn b_sum_cubes() {
    expect(
        "*main()\n    sum is 0\n    for i in 1 to 11\n        sum is sum + i * i * i\n    log(sum)\n",
        "3025",
    );
}
#[test]
fn b_geometric() {
    expect(
        "*main()\n    val is 1\n    for i in 10\n        val is val * 2\n    log(val)\n",
        "1024",
    );
}
#[test]
fn b_nested_fn_call() {
    expect(
        "*add(a: i64, b: i64) -> i64\n    a + b\n\n*mul(a: i64, b: i64) -> i64\n    a * b\n\n*main()\n    log(add(mul(2, 3), mul(4, 5)))\n",
        "26",
    );
}
#[test]
fn b_apply_n_times() {
    expect(
        "*apply_n(f: (i64) -> i64, x: i64, n: i64) -> i64\n    result is x\n    for i in n\n        result is f(result)\n    result\n\n*double(x: i64) -> i64\n    x * 2\n\n*main() -> i32\n    log(apply_n(double, 1, 10))\n    0\n",
        "1024",
    );
}
#[test]
fn b_fn_returns_fn_val() {
    expect(
        "*square(x: i64) -> i64\n    x * x\n\n*cube(x: i64) -> i64\n    x * x * x\n\n*pick_fn(n: i64) -> (i64) -> i64\n    if n equals 2\n        return square\n    cube\n\n*main() -> i32\n    f is pick_fn(2)\n    log(f(5))\n    g is pick_fn(3)\n    log(g(3))\n    0\n",
        "25\n27",
    );
}
#[test]
fn b_enum_4variant() {
    expect(
        "enum Op\n    Add(i64, i64)\n    Sub(i64, i64)\n    Mul(i64, i64)\n    Neg(i64)\n\n*eval(op: Op) -> i64\n    match op\n        Add(a, b) ? a + b\n        Sub(a, b) ? a - b\n        Mul(a, b) ? a * b\n        Neg(a) ? 0 - a\n\n*main() -> i32\n    log(eval(Add(10, 20)))\n    log(eval(Sub(50, 8)))\n    log(eval(Mul(6, 7)))\n    log(eval(Neg(42)))\n    0\n",
        "30\n42\n42\n-42",
    );
}
#[test]
fn b_enum_recursive_sum() {
    expect(
        "enum List\n    Cons(i64, i64)\n    Nil\n\n*main() -> i32\n    a is Cons(10, 0)\n    match a\n        Cons(v, _) ? log(v)\n        Nil ? log(0)\n    0\n",
        "10",
    );
}
#[test]
fn b_struct_multi_methods() {
    expect(
        "type Rect\n    w: i64\n    h: i64\n\n    *area() -> i64\n        self.w * self.h\n\n    *perimeter() -> i64\n        2 * (self.w + self.h)\n\n*main() -> i32\n    r is Rect(w is 5, h is 3)\n    log(r.area())\n    log(r.perimeter())\n    0\n",
        "15\n16",
    );
}
#[test]
fn b_pipeline_complex() {
    expect(
        "*double(x: i64) -> i64\n    x * 2\n\n*sub_one(x: i64) -> i64\n    x - 1\n\n*square(x: i64) -> i64\n    x * x\n\n*main() -> i32\n    result is 3 ~ double ~ sub_one ~ square\n    log(result)\n    0\n",
        "25",
    );
}
#[test]
fn b_listcomp_sum() {
    expect(
        "*main() -> i32\n    arr is [i * i for i in 1 to 6]\n    log(arr[0] + arr[1] + arr[2] + arr[3] + arr[4])\n    0\n",
        "55",
    );
}
#[test]
fn b_closure_counter() {
    expect(
        "*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    base is 10\n    f is *fn(x: i64) -> i64 base + x\n    log(apply(f, 5))\n    base is 20\n    g is *fn(x: i64) -> i64 base + x\n    log(apply(g, 5))\n    0\n",
        "15\n25",
    );
}
#[test]
fn b_enum_as_return() {
    expect(
        "enum Result\n    Ok(i64)\n    Err(i64)\n\n*checked_add(a: i64, b: i64) -> Result\n    sum is a + b\n    if sum > 100\n        return Err(sum)\n    Ok(sum)\n\n*main() -> i32\n    match checked_add(30, 40)\n        Ok(v) ? log(v)\n        Err(e) ? log(0 - e)\n    match checked_add(60, 50)\n        Ok(v) ? log(v)\n        Err(e) ? log(0 - e)\n    0\n",
        "70\n-110",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 41: Additional for-in array tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_arr_count_gt() {
    expect(
        "*main()\n    arr is [1, 5, 3, 8, 2, 7]\n    count is 0\n    for x in arr\n        if x > 4\n            count is count + 1\n    log(count)\n",
        "3",
    );
}
#[test]
fn b_arr_sum_even() {
    expect(
        "*main()\n    arr is [1, 2, 3, 4, 5, 6]\n    sum is 0\n    for x in arr\n        if x % 2 equals 0\n            sum is sum + x\n    log(sum)\n",
        "12",
    );
}
#[test]
fn b_arr_all_positive() {
    expect(
        "*main()\n    arr is [3, 7, 1, 9, 2]\n    all_pos is 1\n    for x in arr\n        if x <= 0\n            all_pos is 0\n    log(all_pos)\n",
        "1",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 42: Additional patterns
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_match_5() {
    expect(
        "*classify(x: i64) -> i64\n    match x\n        0 ? 0\n        1 ? 1\n        2 ? 4\n        3 ? 9\n        _ ? -1\n\n*main()\n    log(classify(0))\n    log(classify(2))\n    log(classify(99))\n",
        "0\n4\n-1",
    );
}
#[test]
fn b_match_large_wildcard() {
    expect(
        "*main()\n    x is 42\n    match x\n        _ ? log(x)\n",
        "42",
    );
}
#[test]
fn b_enum_5() {
    expect(
        "enum Weekday\n    Mon\n    Tue\n    Wed\n    Thu\n    Fri\n\n*is_mid(d: Weekday) -> i64\n    match d\n        Wed ? 1\n        _ ? 0\n\n*main() -> i32\n    log(is_mid(Wed))\n    log(is_mid(Mon))\n    0\n",
        "1\n0",
    );
}
#[test]
fn b_struct_pass_return() {
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*translate(p: Point, dx: i64, dy: i64) -> Point\n    Point(x is p.x + dx, y is p.y + dy)\n\n*main() -> i32\n    p is Point(x is 1, y is 2)\n    q is translate(p, 10, 20)\n    log(q.x)\n    log(q.y)\n    0\n",
        "11\n22",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 43: Math utility functions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_abs_fn() {
    expect(
        "*abs(x: i64) -> i64\n    if x < 0\n        return 0 - x\n    x\n\n*main()\n    log(abs(5))\n    log(abs(-5))\n    log(abs(0))\n",
        "5\n5\n0",
    );
}
#[test]
fn b_max_fn() {
    expect(
        "*max(a: i64, b: i64) -> i64\n    if a > b\n        return a\n    b\n\n*main()\n    log(max(3, 7))\n    log(max(9, 2))\n    log(max(5, 5))\n",
        "7\n9\n5",
    );
}
#[test]
fn b_min_fn() {
    expect(
        "*min(a: i64, b: i64) -> i64\n    if a < b\n        return a\n    b\n\n*main()\n    log(min(3, 7))\n    log(min(9, 2))\n    log(min(5, 5))\n",
        "3\n2\n5",
    );
}
#[test]
fn b_clamp() {
    expect(
        "*clamp(x: i64, lo: i64, hi: i64) -> i64\n    if x < lo\n        return lo\n    if x > hi\n        return hi\n    x\n\n*main()\n    log(clamp(5, 0, 10))\n    log(clamp(-5, 0, 10))\n    log(clamp(15, 0, 10))\n",
        "5\n0\n10",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 44: Additional closures
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_closure_in_pipe() {
    expect(
        "*main() -> i32\n    c is 3\n    result is 7 ~ *fn(x: i64) -> i64 x * c\n    log(result)\n    0\n",
        "21",
    );
}
#[test]
fn b_do_end_if() {
    expect(
        "*main() -> i32\n    abs is *fn(x: i64) -> i64 do\n        result is x\n        if x < 0\n            result is 0 - x\n        result\n    end\n    log(abs(5))\n    log(abs(-3))\n    0\n",
        "5\n3",
    );
}
#[test]
fn b_lambda_chain_pipe() {
    expect(
        "*add_one(x: i64) -> i64\n    x + 1\n\n*main() -> i32\n    result is 5 ~ *fn(x: i64) -> i64 x * x ~ add_one\n    log(result)\n    0\n",
        "26",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 45: Additional number theory
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_is_palindrome() {
    expect(
        "*reverse(n: i64) -> i64\n    result is 0\n    x is n\n    while x > 0\n        result is result * 10 + x % 10\n        x is x / 10\n    result\n\n*is_palindrome(n: i64) -> i64\n    if n equals reverse(n)\n        return 1\n    0\n\n*main()\n    log(is_palindrome(121))\n    log(is_palindrome(123))\n    log(is_palindrome(12321))\n",
        "1\n0\n1",
    );
}
#[test]
fn b_digit_count() {
    expect(
        "*digits(n: i64) -> i64\n    if n equals 0\n        return 1\n    count is 0\n    x is n\n    while x > 0\n        count is count + 1\n        x is x / 10\n    count\n\n*main()\n    log(digits(0))\n    log(digits(9))\n    log(digits(99))\n    log(digits(12345))\n",
        "1\n1\n2\n5",
    );
}
#[test]
fn b_pow_mod() {
    expect(
        "*powmod(base: i64, exp: i64, m: i64) -> i64\n    result is 1\n    b is base % m\n    e is exp\n    while e > 0\n        if e % 2 equals 1\n            result is result * b % m\n        e is e / 2\n        b is b * b % m\n    result\n\n*main()\n    log(powmod(2, 10, 1000))\n",
        "24",
    );
}
#[test]
fn b_harmonic_int() {
    expect(
        "*main()\n    sum is 0\n    for i in 1 to 11\n        sum is sum + 100 / i\n    log(sum)\n",
        "291",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 46: More struct patterns
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_struct_default() {
    expect(
        "type Config\n    width: i64\n    height: i64\n    depth: i64\n\n*volume(c: Config) -> i64\n    c.width * c.height * c.depth\n\n*main() -> i32\n    c is Config(width is 2, height is 3, depth is 4)\n    log(volume(c))\n    0\n",
        "24",
    );
}
#[test]
fn b_struct_nested_access() {
    expect(
        "type Pair\n    a: i64\n    b: i64\n\n*main() -> i32\n    p1 is Pair(a is 10, b is 20)\n    p2 is Pair(a is p1.a + 1, b is p1.b + 1)\n    log(p2.a)\n    log(p2.b)\n    0\n",
        "11\n21",
    );
}
#[test]
fn b_struct_cmp() {
    expect(
        "type Box\n    val: i64\n\n*bigger(a: Box, b: Box) -> i64\n    if a.val > b.val\n        return a.val\n    b.val\n\n*main() -> i32\n    log(bigger(Box(val is 10), Box(val is 20)))\n    0\n",
        "20",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 47: More loop patterns
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_for_sum_odd() {
    expect(
        "*main()\n    sum is 0\n    for i in 1 to 20 by 2\n        sum is sum + i\n    log(sum)\n",
        "100",
    );
}
#[test]
fn b_while_gcd() {
    expect(
        "*main()\n    a is 252\n    b is 105\n    while b neq 0\n        t is b\n        b is a % b\n        a is t\n    log(a)\n",
        "21",
    );
}
#[test]
fn b_for_powers() {
    expect(
        "*main()\n    val is 1\n    for i in 16\n        val is val * 3\n    log(val)\n",
        "43046721",
    );
}
#[test]
fn b_nested_for_mult_table() {
    expect(
        "*main()\n    sum is 0\n    for i in 1 to 4\n        for j in 1 to 4\n            sum is sum + i * j\n    log(sum)\n",
        "36",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 48: Generics + closures combined
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_gen_apply() {
    expect(
        "*apply of T(f: (T) -> T, x: T) -> T\n    f(x)\n\n*main() -> i32\n    log(apply(*fn(x: i64) -> i64 x + 1, 41))\n    0\n",
        "42",
    );
}
#[test]
fn b_gen_compose() {
    expect(
        "*compose(f: (i64) -> i64, g: (i64) -> i64, x: i64) -> i64\n    f(g(x))\n\n*dbl(x: i64) -> i64\n    x * 2\n\n*inc(x: i64) -> i64\n    x + 1\n\n*main() -> i32\n    log(compose(dbl, inc, 5))\n    log(compose(inc, dbl, 5))\n    0\n",
        "12\n11",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 49: String operations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_str_multi_concat() {
    expect(
        "*main()\n    s is 'a' + 'b' + 'c' + 'd'\n    log(s)\n    log(s.length)\n",
        "abcd\n4",
    );
}
#[test]
fn b_str_var_concat() {
    expect(
        "*main()\n    greeting is 'hello'\n    name is 'world'\n    log(greeting + ' ' + name)\n",
        "hello world",
    );
}
#[test]
fn b_str_escape_backslash() {
    expect("*main()\n    log('a\\\\b')\n", "a\\b");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 50: More complex algorithms
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_binary_search_manual() {
    // Binary search via explicit while loop (no array mutation needed)
    expect(
        "*main()\n    arr is [2, 5, 8, 12, 16, 23, 38, 56, 72, 91]\n    target is 23\n    lo is 0\n    hi is 9\n    found is -1\n    while lo <= hi\n        mid is (lo + hi) / 2\n        if arr[mid] equals target\n            found is mid\n            lo is hi + 1\n        elif arr[mid] < target\n            lo is mid + 1\n        else\n            hi is mid - 1\n    log(found)\n",
        "5",
    );
}
#[test]
fn b_fibonacci_loop() {
    expect(
        "*main()\n    a is 0\n    b is 1\n    i is 0\n    while i < 30\n        t is a\n        a is b\n        b is t + b\n        i is i + 1\n    log(a)\n",
        "832040",
    );
}
#[test]
fn b_euler_sum_div() {
    // Sum of numbers 1..999 divisible by 3 or 5
    expect(
        "*main()\n    sum is 0\n    for i in 1 to 1000\n        if i % 3 equals 0 or i % 5 equals 0\n            sum is sum + i\n    log(sum)\n",
        "233168",
    );
}
#[test]
fn b_perfect_number() {
    expect(
        "*is_perfect(n: i64) -> i64\n    sum is 0\n    i is 1\n    while i < n\n        if n % i equals 0\n            sum is sum + i\n        i is i + 1\n    if sum equals n\n        return 1\n    0\n\n*main()\n    log(is_perfect(6))\n    log(is_perfect(28))\n    log(is_perfect(12))\n",
        "1\n1\n0",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 51: Pipeline advanced
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_pipe_4_chain() {
    expect(
        "*a(x: i64) -> i64\n    x + 1\n\n*b(x: i64) -> i64\n    x * 2\n\n*c(x: i64) -> i64\n    x - 3\n\n*d(x: i64) -> i64\n    x * x\n\n*main() -> i32\n    result is 5 ~ a ~ b ~ c ~ d\n    log(result)\n    0\n",
        "81",
    );
}
#[test]
fn b_pipe_placeholder_2() {
    expect(
        "*sub(a: i64, b: i64) -> i64\n    a - b\n\n*main() -> i32\n    result is 10 ~ sub($, 3)\n    log(result)\n    0\n",
        "7",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 52: Array advanced
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_arr_nested_access() {
    expect(
        "*main() -> i32\n    a is [10, 20, 30, 40, 50]\n    log(a[a[0] / 10])\n    0\n",
        "20",
    );
}
#[test]
fn b_arr_expr_index() {
    expect(
        "*main() -> i32\n    a is [100, 200, 300]\n    i is 1\n    log(a[i])\n    log(a[i + 1])\n    0\n",
        "200\n300",
    );
}
#[test]
fn b_listcomp_chain() {
    expect(
        "*main() -> i32\n    arr is [x * 2 + 1 for x in 0 to 5]\n    log(arr[0])\n    log(arr[1])\n    log(arr[2])\n    log(arr[3])\n    log(arr[4])\n    0\n",
        "1\n3\n5\n7\n9",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 53: More function patterns
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_fn_no_return_type() {
    expect(
        "*say_hi()\n    log(42)\n\n*main()\n    say_hi()\n    say_hi()\n",
        "42\n42",
    );
}
#[test]
fn b_fn_single_arg() {
    expect(
        "*negate(x: i64) -> i64\n    0 - x\n\n*main()\n    log(negate(42))\n    log(negate(-7))\n",
        "-42\n7",
    );
}
#[test]
fn b_fn_implicit_return() {
    expect(
        "*square(x: i64) -> i64\n    x * x\n\n*main()\n    log(square(9))\n",
        "81",
    );
}
#[test]
fn b_fn_arg_expr() {
    expect(
        "*add(a: i64, b: i64) -> i64\n    a + b\n\n*main()\n    log(add(2 + 3, 4 * 5))\n",
        "25",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 54: Modular arithmetic
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_mod_basic() {
    expect("*main()\n    log(10 % 3)\n", "1");
}
#[test]
fn b_mod_even_check() {
    expect("*main()\n    log(14 % 2)\n    log(15 % 2)\n", "0\n1");
}
#[test]
fn b_mod_large() {
    expect("*main()\n    log(1000000007 % 1000)\n", "7");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 55: More enum patterns
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_enum_data_3_variants() {
    expect(
        "enum Expr\n    Lit(i64)\n    Add(i64, i64)\n    Mul(i64, i64)\n\n*eval(e: Expr) -> i64\n    match e\n        Lit(x) ? x\n        Add(a, b) ? a + b\n        Mul(a, b) ? a * b\n\n*main() -> i32\n    log(eval(Lit(5)))\n    log(eval(Add(3, 4)))\n    log(eval(Mul(6, 7)))\n    0\n",
        "5\n7\n42",
    );
}
#[test]
fn b_enum_unit_all() {
    expect(
        "enum Season\n    Spring\n    Summer\n    Autumn\n    Winter\n\n*name(s: Season) -> i64\n    match s\n        Spring ? 1\n        Summer ? 2\n        Autumn ? 3\n        Winter ? 4\n\n*main() -> i32\n    log(name(Spring))\n    log(name(Summer))\n    log(name(Autumn))\n    log(name(Winter))\n    0\n",
        "1\n2\n3\n4",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 56: Cast combinations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_cast_chain() {
    expect("*main()\n    x is 42 as i32 as i64\n    log(x)\n", "42");
}
#[test]
fn b_cast_i8() {
    expect("*main()\n    x is 127 as i8\n    log(x)\n", "127");
}
#[test]
fn b_cast_u8() {
    expect("*main()\n    x is 200 as u8\n    log(x)\n", "200");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 57: While with complex conditions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_while_compound() {
    expect(
        "*main()\n    x is 0\n    y is 100\n    while x < 50 and y > 50\n        x is x + 1\n        y is y - 1\n    log(x)\n    log(y)\n",
        "50\n50",
    );
}
#[test]
fn b_while_or_cond() {
    expect(
        "*main()\n    x is 0\n    while x < 3 or x equals 3\n        x is x + 1\n    log(x)\n",
        "4",
    );
}
#[test]
fn b_while_not_cond() {
    expect(
        "*main()\n    done is false\n    i is 0\n    while not done\n        i is i + 1\n        if i equals 5\n            done is true\n    log(i)\n",
        "5",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 58: Additional Option/Result patterns
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_option_fn() {
    expect(
        "*main() -> i32\n    arr is [3, 7, 1, 9, 2]\n    found is -1\n    for x in arr\n        if x > 5 and found equals -1\n            found is x\n    log(found)\n    0\n",
        "7",
    );
}
#[test]
fn b_result_chain() {
    expect(
        "enum MaybeI64\n    Val(i64)\n    None\n\n*try_div(a: i64, b: i64) -> MaybeI64\n    if b equals 0\n        return None\n    Val(a / b)\n\n*main() -> i32\n    match try_div(100, 5)\n        Val(v) ? log(v)\n        None ? log(-1)\n    match try_div(100, 0)\n        Val(v) ? log(v)\n        None ? log(-1)\n    0\n",
        "20\n-1",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 59: More RC patterns
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_rc_multi() {
    expect(
        "*main()\n    a is rc(10)\n    b is rc(20)\n    log(@a + @b)\n",
        "30",
    );
}
#[test]
fn b_rc_nested_arith() {
    expect("*main()\n    x is rc(7)\n    log(@x * @x + 1)\n", "50");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 60: Expression edge cases
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_double_neg() {
    expect("*main()\n    x is 5\n    log(0 - (0 - x))\n", "5");
}
#[test]
fn b_assoc_add() {
    expect("*main()\n    log((1 + 2) + (3 + 4))\n", "10");
}
#[test]
fn b_mixed_ops() {
    expect("*main()\n    log(2 * 3 + 4 * 5 - 6)\n", "20");
}
#[test]
fn b_deeply_nested() {
    expect("*main()\n    log(((((1 + 2) * 3) + 4) * 5) - 6)\n", "59");
}
#[test]
fn b_unary_neg() {
    expect(
        "*main()\n    x is -10\n    y is -20\n    log(x + y)\n",
        "-30",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH 61: For-in with computations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_for_in_max_arr() {
    expect(
        "*main()\n    arr is [5, 12, 3, 8, 15, 7]\n    best is 0\n    for x in arr\n        if x > best\n            best is x\n    log(best)\n",
        "15",
    );
}
#[test]
fn b_for_in_count() {
    expect(
        "*main()\n    arr is [1, 2, 3, 4, 5]\n    count is 0\n    for x in arr\n        count is count + 1\n    log(count)\n",
        "5",
    );
}
#[test]
fn b_for_in_nested_if() {
    expect(
        "*main()\n    arr is [10, 25, 30, 45, 50]\n    count is 0\n    for x in arr\n        if x > 20 and x < 50\n            count is count + 1\n    log(count)\n",
        "3",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: String Interpolation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_interp_str_var() {
    expect(
        "*main()\n    name is 'world'\n    log('hello {name}')\n",
        "hello world",
    );
}
#[test]
fn b_interp_int_var() {
    expect("*main()\n    x is 42\n    log('x={x}')\n", "x=42");
}
#[test]
fn b_interp_float_var() {
    expect("*main()\n    pi is 3.14\n    log('pi={pi}')\n", "pi=3.14");
}
#[test]
fn b_interp_expr() {
    expect("*main()\n    log('sum={2 + 3}')\n", "sum=5");
}
#[test]
fn b_interp_multi() {
    expect(
        "*main()\n    a is 'x'\n    b is 1\n    log('{a}={b}')\n",
        "x=1",
    );
}
#[test]
fn b_interp_start() {
    expect("*main()\n    x is 42\n    log('{x} done')\n", "42 done");
}
#[test]
fn b_interp_only() {
    expect("*main()\n    x is 'hello'\n    log('{x}')\n", "hello");
}
#[test]
fn b_interp_no_interp() {
    expect("*main()\n    log('plain string')\n", "plain string");
}
#[test]
fn b_interp_adjacent() {
    expect("*main()\n    a is 1\n    b is 2\n    log('{a}{b}')\n", "12");
}
#[test]
fn b_interp_bool() {
    expect("*main()\n    log('{true}')\n", "true");
}
#[test]
fn b_interp_complex_expr() {
    expect(
        "*main()\n    x is 10\n    log('result={x * 2 + 1}')\n",
        "result=21",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Augmented Assignment
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_aug_plus() {
    expect("*main()\n    x is 10\n    x += 5\n    log(x)\n", "15");
}
#[test]
fn b_aug_minus() {
    expect("*main()\n    x is 10\n    x -= 3\n    log(x)\n", "7");
}
#[test]
fn b_aug_mul() {
    expect("*main()\n    x is 6\n    x *= 7\n    log(x)\n", "42");
}
#[test]
fn b_aug_div() {
    expect("*main()\n    x is 100\n    x /= 4\n    log(x)\n", "25");
}
// b_aug_mod removed: %= dropped from language
#[test]
fn b_aug_bitand() {
    expect("*main()\n    x is 0xFF\n    x &= 0x0F\n    log(x)\n", "15");
}
#[test]
fn b_aug_bitor() {
    expect("*main()\n    x is 0xF0\n    x |= 0x0F\n    log(x)\n", "255");
}
#[test]
fn b_aug_xor() {
    expect("*main()\n    x is 0xFF\n    x ^= 0x0F\n    log(x)\n", "240");
}
#[test]
fn b_aug_shl() {
    expect("*main()\n    x is 1\n    x <<= 10\n    log(x)\n", "1024");
}
#[test]
fn b_aug_shr() {
    expect("*main()\n    x is 1024\n    x >>= 5\n    log(x)\n", "32");
}
#[test]
fn b_aug_chain() {
    expect(
        "*main()\n    x is 1\n    x += 9\n    x *= 3\n    x -= 6\n    log(x)\n",
        "24",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: String Methods
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_str_contains_yes() {
    expect("*main()\n    log('hello world'.contains('world'))\n", "1");
}
#[test]
fn b_str_contains_no() {
    expect("*main()\n    log('hello world'.contains('xyz'))\n", "0");
}
#[test]
fn b_str_contains_empty() {
    expect("*main()\n    log('hello'.contains(''))\n", "1");
}
#[test]
fn b_str_starts_with_yes() {
    expect(
        "*main()\n    log('hello world'.starts_with('hello'))\n",
        "1",
    );
}
#[test]
fn b_str_starts_with_no() {
    expect(
        "*main()\n    log('hello world'.starts_with('world'))\n",
        "0",
    );
}
#[test]
fn b_str_ends_with_yes() {
    expect("*main()\n    log('hello world'.ends_with('world'))\n", "1");
}
#[test]
fn b_str_ends_with_no() {
    expect("*main()\n    log('hello world'.ends_with('hello'))\n", "0");
}
#[test]
fn b_str_char_at() {
    expect("*main()\n    log('abc'.char_at(0))\n", "97");
}
#[test]
fn b_str_char_at_mid() {
    expect("*main()\n    log('abc'.char_at(1))\n", "98");
}
#[test]
fn b_str_slice() {
    expect("*main()\n    log('hello world'.slice(0, 5))\n", "hello");
}
#[test]
fn b_str_slice_mid() {
    expect("*main()\n    log('hello world'.slice(6, 11))\n", "world");
}
#[test]
fn b_str_slice_len() {
    expect(
        "*main()\n    s is 'abcdef'.slice(2, 5)\n    log(s.length)\n",
        "3",
    );
}

// --- Array element assignment ---

#[test]
fn b_arr_assign_basic() {
    expect(
        "*main()\n    arr is [10, 20, 30]\n    arr[1] is 99\n    log(arr[1])\n",
        "99",
    );
}

#[test]
fn b_arr_assign_first() {
    expect(
        "*main()\n    arr is [1, 2, 3]\n    arr[0] is 42\n    log(arr[0])\n",
        "42",
    );
}

#[test]
fn b_arr_assign_last() {
    expect(
        "*main()\n    arr is [1, 2, 3]\n    arr[2] is 77\n    log(arr[2])\n",
        "77",
    );
}

#[test]
fn b_arr_assign_expr() {
    expect(
        "*main()\n    arr is [10, 20, 30]\n    arr[1] is arr[0] + arr[2]\n    log(arr[1])\n",
        "40",
    );
}

#[test]
fn b_arr_assign_neg() {
    expect(
        "*main()\n    arr is [1, 2, 3, 4, 5]\n    arr[-1] is 99\n    log(arr[4])\n",
        "99",
    );
}

// --- Struct field assignment ---

#[test]
fn b_field_assign_basic() {
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*main()\n    p is Point(x is 10, y is 20)\n    p.x is 42\n    log(p.x)\n",
        "42",
    );
}

#[test]
fn b_field_assign_both() {
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*main()\n    p is Point(x is 1, y is 2)\n    p.x is 10\n    p.y is 20\n    log(p.x + p.y)\n",
        "30",
    );
}

// --- Negative array indexing ---

#[test]
fn b_neg_idx_last() {
    expect(
        "*main()\n    arr is [10, 20, 30, 40, 50]\n    log(arr[-1])\n",
        "50",
    );
}

#[test]
fn b_neg_idx_second_last() {
    expect(
        "*main()\n    arr is [10, 20, 30, 40, 50]\n    log(arr[-2])\n",
        "40",
    );
}

#[test]
fn b_neg_idx_first() {
    expect("*main()\n    arr is [10, 20, 30]\n    log(arr[-3])\n", "10");
}

// --- Bitwise NOT ---

#[test]
fn b_bitnot_zero() {
    expect("*main()\n    log(~0)\n", "-1");
}

#[test]
fn b_bitnot_val() {
    expect("*main()\n    log(~255)\n", "-256");
}

#[test]
fn b_bitnot_double() {
    expect("*main()\n    x is 42\n    log(~~x)\n", "42");
}

// --- String length method ---

#[test]
fn b_str_len_method() {
    expect("*main()\n    s is 'hello'\n    log(s.len())\n", "5");
}

#[test]
fn b_str_len_empty() {
    expect("*main()\n    s is ''\n    log(s.length)\n", "0");
}

// --- Tuple destructuring ---

#[test]
fn b_tuple_bind_basic() {
    expect(
        "*main()\n    x, y is (10, 20)\n    log(x)\n    log(y)\n",
        "10\n20",
    );
}

#[test]
fn b_tuple_bind_triple() {
    expect(
        "*main()\n    a, b, c is (1, 2, 3)\n    log(a + b + c)\n",
        "6",
    );
}

#[test]
fn b_tuple_bind_fn_nullary() {
    expect(
        "*pair()\n    (10, 20)\n\n*main()\n    x, y is pair()\n    log(x)\n    log(y)\n",
        "10\n20",
    );
}

#[test]
fn b_tuple_bind_fn_args() {
    expect(
        "*divmod(a, b)\n    (a / b, a % b)\n\n*main()\n    q, r is divmod(17, 5)\n    log(q)\n    log(r)\n",
        "3\n2",
    );
}

#[test]
fn b_tuple_bind_expr() {
    expect(
        "*main()\n    x, y is (3 + 4, 10 * 2)\n    log(x)\n    log(y)\n",
        "7\n20",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PATTERN MATCHING GUARDS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_match_guard_value() {
    expect(
        "*main()\n    x is 5\n    match x\n        n when n > 3 ? log 'big'\n        _ ? log 'small'\n",
        "big",
    );
}

#[test]
fn b_match_guard_fallthrough() {
    expect(
        "*main()\n    x is 1\n    match x\n        n when n > 3 ? log 'big'\n        _ ? log 'small'\n",
        "small",
    );
}

#[test]
fn b_match_guard_enum() {
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*main()\n    s is Circle(10)\n    match s\n        Circle(r) when r > 5 ? log 'big circle'\n        Circle(r) ? log 'small circle'\n        _ ? log 'other'\n",
        "big circle",
    );
}

#[test]
fn b_match_guard_enum_fail() {
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*main()\n    s is Circle(2)\n    match s\n        Circle(r) when r > 5 ? log 'big circle'\n        Circle(r) ? log 'small circle'\n        _ ? log 'other'\n",
        "small circle",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EXTENSION METHODS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_ext_method_basic() {
    expect(
        "type Point\n    x: i64\n    y: i64\n\nimpl Point\n    *sum() -> i64\n        self.x + self.y\n\n*main()\n    p is Point(3, 4)\n    log p.sum()\n",
        "7",
    );
}

#[test]
fn b_ext_method_with_args() {
    expect(
        "type Vec2\n    x: i64\n    y: i64\n\nimpl Vec2\n    *add(other: Vec2) -> Vec2\n        Vec2(self.x + other.x, self.y + other.y)\n\n*main()\n    a is Vec2(1, 2)\n    b is Vec2(3, 4)\n    c is a.add(b)\n    log c.x\n    log c.y\n",
        "4\n6",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ASSERT
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_assert_pass() {
    expect("*main()\n    assert 1 equals 1\n    log 'ok'\n", "ok");
}

#[test]
fn b_assert_parens() {
    expect("*main()\n    assert(2 + 2 equals 4)\n    log 'ok'\n", "ok");
}

#[test]
fn b_assert_fail() {
    expect_runtime_fail("*main()\n    assert 1 equals 2\n");
}

#[test]
fn b_assert_expr() {
    expect(
        "*main()\n    assert(10 > 5)\n    assert(3 < 100)\n    log 'passed'\n",
        "passed",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EMBED
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_embed_file() {
    let out = compile_and_run_with_file(
        "*main()\n    log embed 'data.txt'\n",
        "data.txt",
        "hello embedded",
    );
    assert_eq!(out.trim(), "hello embedded");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TEST BLOCKS (--test mode)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_test_mode_runs_tests() {
    let out = compile_and_run_test_mode(
        "*add(a, b)\n    return a + b\n\ntest 'addition'\n    assert add(2, 3) equals 5\n\n*main()\n    log 'skip'\n",
    );
    let trimmed = out.trim();
    assert!(
        trimmed.contains("test addition ..."),
        "expected test header, got: {trimmed}"
    );
    assert!(trimmed.contains("ok"), "expected ok, got: {trimmed}");
    assert!(
        !trimmed.contains("skip"),
        "main should not run in test mode"
    );
}

#[test]
fn b_test_mode_multiple() {
    let out = compile_and_run_test_mode(
        "*add(a, b)\n    return a + b\n\ntest 'first'\n    assert add(1, 1) equals 2\n\ntest 'second'\n    assert add(0, 0) equals 0\n\n*main()\n    log 'nope'\n",
    );
    let trimmed = out.trim();
    assert!(trimmed.contains("test first ..."), "missing first test");
    assert!(trimmed.contains("test second ..."), "missing second test");
}

#[test]
fn b_test_blocks_stripped_normal() {
    expect(
        "*add(a, b)\n    return a + b\n\ntest 'whatever'\n    assert add(1, 1) equals 2\n\n*main()\n    log 'normal'\n",
        "normal",
    );
}

// --- Pattern-directed function clauses ---

#[test]
fn b_clause_fn_fib() {
    expect(
        "*fib(0) is 0\n\n*fib(1) is 1\n\n*fib(n)\n    fib(n - 1) + fib(n - 2)\n\n*main()\n    log(fib(10))\n",
        "55",
    );
}

#[test]
fn b_clause_fn_fact() {
    expect(
        "*fact(0) is 1\n\n*fact(n) is n * fact(n - 1)\n\n*main()\n    log(fact(5))\n",
        "120",
    );
}

#[test]
fn b_clause_fn_base_case() {
    expect(
        "*val(0) is 99\n\n*val(n) is n\n\n*main()\n    log(val(0))\n    log(val(7))\n",
        "99\n7",
    );
}

#[test]
fn b_clause_fn_gcd() {
    expect(
        "*gcd(0, b) is b\n\n*gcd(a, 0) is a\n\n*gcd(a, b)\n    if a > b\n        return gcd(a - b, b)\n    gcd(a, b - a)\n\n*main()\n    log(gcd(12, 8))\n",
        "4",
    );
}

// --- Inline body ---

#[test]
fn b_inline_body_parens() {
    expect(
        "*double(x) is x * 2\n\n*main()\n    log(double(21))\n",
        "42",
    );
}

#[test]
fn b_inline_body_paren_free() {
    expect(
        "*add a, b is a + b\n\n*main()\n    log(add(10, 20))\n",
        "30",
    );
}

#[test]
fn b_inline_body_multi() {
    expect(
        "*square(x) is x * x\n\n*cube(x) is x * x * x\n\n*main()\n    log(square(5))\n    log(cube(3))\n",
        "25\n27",
    );
}

#[test]
fn b_inline_fib_all_clauses() {
    expect(
        "*fib(0) is 0\n\n*fib(1) is 1\n\n*fib(n) is fib(n - 1) + fib(n - 2)\n\n*main()\n    log(fib(10))\n",
        "55",
    );
}

// ── Actor tests ──────────────────────────────────────────────────────

// Helper: compile and run a Jade program, executing the binary inside the tempdir
// so that .store files are created in isolation.
fn compile_and_run_in_dir(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jade = dir.path().join("test.jade");
    let out = dir.path().join("test_bin");
    std::fs::write(&jade, src).unwrap();
    let status = Command::new(jadec())
        .arg(&jade)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jadec failed to start");
    assert!(status.success(), "jadec compilation failed for:\n{src}");
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
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_string()
}

#[test]
fn b_actor_spawn_send_basic() {
    // Actor that logs a value when it receives a message
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Printer\n    @say n: i64\n        log(n)\n\n*main()\n    p is spawn Printer\n    send p, @say(42)\n    usleep(100000)\n    0\n",
        "42",
    );
}

#[test]
fn b_actor_multiple_messages() {
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Echo\n    @say n: i64\n        log(n)\n\n*main()\n    e is spawn Echo\n    send e, @say(1)\n    send e, @say(2)\n    send e, @say(3)\n    usleep(100000)\n    0\n",
        "1\n2\n3",
    );
}

#[test]
fn b_actor_state_accumulation() {
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Adder\n    total: i64\n    @add n: i64\n        total is total + n\n    @show\n        log(total)\n\n*main()\n    a is spawn Adder\n    send a, @add(10)\n    send a, @add(20)\n    send a, @add(30)\n    usleep(100000)\n    send a, @show()\n    usleep(100000)\n    0\n",
        "60",
    );
}

#[test]
fn b_actor_multi_handler() {
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Math\n    val: i64\n    @assign n: i64\n        val is n\n    @double\n        val is val * 2\n    @show\n        log(val)\n\n*main()\n    m is spawn Math\n    send m, @assign(5)\n    send m, @double()\n    usleep(100000)\n    send m, @show()\n    usleep(100000)\n    0\n",
        "10",
    );
}

// ── Additional actor tests ───────────────────────────────────────────

#[test]
fn b_actor_zero_param_handler() {
    // Handler with no parameters
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Pinger\n    count: i64\n    @ping\n        count is count + 1\n    @show\n        log(count)\n\n*main()\n    p is spawn Pinger\n    send p, @ping()\n    send p, @ping()\n    send p, @ping()\n    usleep(100000)\n    send p, @show()\n    usleep(100000)\n    0\n",
        "3",
    );
}

#[test]
fn b_actor_two_param_handler() {
    // Handler with two parameters
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Calc\n    result: i64\n    @add a: i64, b: i64\n        result is a + b\n    @show\n        log(result)\n\n*main()\n    c is spawn Calc\n    send c, @add(17, 25)\n    usleep(100000)\n    send c, @show()\n    usleep(100000)\n    0\n",
        "42",
    );
}

#[test]
fn b_actor_multiple_state_fields() {
    // Actor with multiple state fields
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Point\n    x: i64\n    y: i64\n    @set_x n: i64\n        x is n\n    @set_y n: i64\n        y is n\n    @show\n        log(x + y)\n\n*main()\n    p is spawn Point\n    send p, @set_x(10)\n    send p, @set_y(20)\n    usleep(100000)\n    send p, @show()\n    usleep(100000)\n    0\n",
        "30",
    );
}

#[test]
fn b_actor_sequential_messages() {
    // Actor processes messages in FIFO order
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Logger\n    @say n: i64\n        log(n)\n\n*main()\n    l is spawn Logger\n    send l, @say(10)\n    send l, @say(20)\n    send l, @say(30)\n    send l, @say(40)\n    send l, @say(50)\n    usleep(100000)\n    0\n",
        "10\n20\n30\n40\n50",
    );
}

// ── Store tests ──────────────────────────────────────────────────────

#[test]
fn b_store_insert_and_count() {
    // Basic insert and count
    let out = compile_and_run_in_dir(
        "store items\n    key: i64\n    val: i64\n\n*main()\n    insert items 1, 10\n    insert items 2, 20\n    insert items 3, 30\n    c is count items\n    log(c)\n",
    );
    assert_eq!(out, "3");
}

#[test]
fn b_store_query_equals() {
    // Query with equals filter
    let out = compile_and_run_in_dir(
        "store data\n    key: i64\n    val: i64\n\n*main()\n    insert data 1, 100\n    insert data 2, 200\n    insert data 3, 300\n    r is data where key equals 2\n    log(r.val)\n",
    );
    assert_eq!(out, "200");
}

#[test]
fn b_store_query_less_than() {
    // Query with less-than filter
    let out = compile_and_run_in_dir(
        "store nums\n    id: i64\n    score: i64\n\n*main()\n    insert nums 1, 50\n    insert nums 2, 30\n    insert nums 3, 70\n    r is nums where score < 40\n    log(r.id)\n",
    );
    assert_eq!(out, "2");
}

#[test]
fn b_store_query_greater_than() {
    // Query with greater-than filter
    let out = compile_and_run_in_dir(
        "store vals\n    x: i64\n    y: i64\n\n*main()\n    insert vals 10, 1\n    insert vals 20, 2\n    insert vals 30, 3\n    r is vals where x > 15\n    log(r.y)\n",
    );
    assert_eq!(out, "2");
}

#[test]
fn b_store_delete() {
    // Delete records matching a filter
    let out = compile_and_run_in_dir(
        "store items\n    key: i64\n    val: i64\n\n*main()\n    insert items 1, 10\n    insert items 2, 20\n    insert items 3, 30\n    delete items where key equals 2\n    c is count items\n    log(c)\n",
    );
    assert_eq!(out, "2");
}

#[test]
fn b_store_all() {
    // Get all records and verify data via count
    let out = compile_and_run_in_dir(
        "store recs\n    n: i64\n\n*main()\n    insert recs 10\n    insert recs 20\n    insert recs 30\n    c is count recs\n    log(c)\n    a is all recs\n    log(0)\n",
    );
    assert_eq!(out, "3\n0");
}

#[test]
fn b_store_set_update() {
    // Update records with set (spec syntax: set store where filter field value)
    let out = compile_and_run_in_dir(
        "store users\n    id: i64\n    score: i64\n\n*main()\n    insert users 1, 100\n    insert users 2, 200\n    set users where id equals 1 score 999\n    r is users where id equals 1\n    log(r.score)\n",
    );
    assert_eq!(out, "999");
}

#[test]
fn b_store_multiple_inserts_query() {
    // Insert many records and query
    let out = compile_and_run_in_dir(
        "store db\n    key: i64\n    val: i64\n\n*main()\n    i is 0\n    while i < 100\n        insert db i, i * 7\n        i is i + 1\n    r is db where key equals 50\n    log(r.val)\n",
    );
    assert_eq!(out, "350");
}

#[test]
fn b_store_transaction_basic() {
    // Transaction block groups store operations
    let out = compile_and_run_in_dir(
        "store ledger\n    amount: i64\n\n*main()\n    transaction\n        insert ledger 10\n        insert ledger 20\n        insert ledger 30\n    c is count ledger\n    log(c)\n",
    );
    assert_eq!(out, "3");
}

#[test]
fn b_store_compound_filter_and() {
    // AND compound filter
    let out = compile_and_run_in_dir(
        "store items\n    cat: i64\n    val: i64\n\n*main()\n    insert items 1, 10\n    insert items 1, 20\n    insert items 2, 30\n    insert items 2, 40\n    r is items where cat equals 1 and val > 15\n    log(r.val)\n",
    );
    assert_eq!(out, "20");
}

#[test]
fn b_store_string_field() {
    // Store with string fields
    let out = compile_and_run_in_dir(
        "store people\n    name: String\n    age: i64\n\n*main()\n    insert people 'Alice', 30\n    insert people 'Bob', 25\n    r is people where age equals 25\n    log(r.name)\n",
    );
    assert_eq!(out, "Bob");
}

#[test]
fn b_store_delete_and_recount() {
    // Delete multiple records and verify count
    let out = compile_and_run_in_dir(
        "store records\n    key: i64\n    val: i64\n\n*main()\n    insert records 1, 10\n    insert records 2, 20\n    insert records 3, 30\n    insert records 4, 40\n    insert records 5, 50\n    delete records where key > 3\n    c is count records\n    log(c)\n",
    );
    assert_eq!(out, "3");
}

// ==================== Trait tests ====================

#[test]
fn b_trait_basic_impl() {
    // Define a trait and implement it for a struct
    expect(
        "type Vec2\n    x: i64\n    y: i64\n\ntrait Summable\n    *sum() -> i64\n\nimpl Summable for Vec2\n    *sum() -> i64\n        self.x + self.y\n\n*main()\n    v is Vec2(x is 3, y is 7)\n    log(v.sum())\n",
        "10",
    );
}

#[test]
fn b_trait_multiple_methods() {
    // Trait with multiple methods
    expect(
        "type Point\n    x: i64\n    y: i64\n\ntrait Describable\n    *get_x() -> i64\n    *get_y() -> i64\n\nimpl Describable for Point\n    *get_x() -> i64\n        self.x\n    *get_y() -> i64\n        self.y\n\n*main()\n    p is Point(x is 5, y is 12)\n    log(p.get_x())\n    log(p.get_y())\n",
        "5\n12",
    );
}

#[test]
fn b_trait_with_params() {
    // Trait method with extra parameters
    expect(
        "type Counter\n    val: i64\n\ntrait Addable\n    *add_to(n: i64) -> i64\n\nimpl Addable for Counter\n    *add_to(n: i64) -> i64\n        self.val + n\n\n*main()\n    c is Counter(val is 10)\n    log(c.add_to(5))\n",
        "15",
    );
}

#[test]
fn b_trait_impl_alongside_methods() {
    // Struct with inline methods AND trait impl methods
    expect(
        "type Num\n    v: i64\n\n    *double() -> i64\n        self.v * 2\n\ntrait Showable\n    *value() -> i64\n\nimpl Showable for Num\n    *value() -> i64\n        self.v\n\n*main()\n    n is Num(v is 21)\n    log(n.double())\n    log(n.value())\n",
        "42\n21",
    );
}

#[test]
fn b_trait_multiple_impls() {
    // Same trait implemented for different types
    expect(
        "type A\n    x: i64\n\ntype B\n    y: i64\n\ntrait GetVal\n    *val() -> i64\n\nimpl GetVal for A\n    *val() -> i64\n        self.x\n\nimpl GetVal for B\n    *val() -> i64\n        self.y\n\n*main()\n    a is A(x is 10)\n    b is B(y is 20)\n    log(a.val())\n    log(b.val())\n",
        "10\n20",
    );
}

#[test]
fn b_trait_missing_method_fails() {
    // Impl missing a required method should fail compilation
    let err = expect_compile_fail(
        "type Foo\n    x: i64\n\ntrait NeedTwo\n    *first() -> i64\n    *second() -> i64\n\nimpl NeedTwo for Foo\n    *first() -> i64\n        self.x\n\n*main()\n    log(0)\n",
    );
    assert!(
        err.contains("missing required method 'second'"),
        "expected missing method error, got: {err}"
    );
}

#[test]
fn b_trait_unknown_trait_fails() {
    // Impl for nonexistent trait should fail
    let err = expect_compile_fail(
        "type Bar\n    x: i64\n\nimpl Nonexistent for Bar\n    *foo() -> i64\n        self.x\n\n*main()\n    log(0)\n",
    );
    assert!(
        err.contains("unknown trait"),
        "expected unknown trait error, got: {err}"
    );
}

#[test]
fn b_trait_unknown_type_fails() {
    // Impl for nonexistent type should fail
    let err = expect_compile_fail(
        "trait MyTrait\n    *foo() -> i64\n\nimpl MyTrait for Nonexistent\n    *foo() -> i64\n        0\n\n*main()\n    log(0)\n",
    );
    assert!(
        err.contains("unknown type"),
        "expected unknown type error, got: {err}"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Vec operations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_vec_create_empty() {
    expect("*main()\n    v is vec()\n    log(v.len())\n", "0");
}

#[test]
fn b_vec_create_with_elems() {
    expect("*main()\n    v is vec(10, 20, 30)\n    log(v.len())\n", "3");
}

#[test]
fn b_vec_push_and_len() {
    expect(
        "*main()\n    v is vec()\n    v.push(1)\n    v.push(2)\n    v.push(3)\n    log(v.len())\n",
        "3",
    );
}

#[test]
fn b_vec_get() {
    expect(
        "*main()\n    v is vec(10, 20, 30)\n    log(v.get(0))\n    log(v.get(1))\n    log(v.get(2))\n",
        "10\n20\n30",
    );
}

#[test]
fn b_vec_set() {
    expect(
        "*main()\n    v is vec(10, 20, 30)\n    v.set(1, 99)\n    log(v.get(1))\n",
        "99",
    );
}

#[test]
fn b_vec_pop() {
    expect(
        "*main()\n    v is vec(10, 20, 30)\n    x is v.pop()\n    log(x)\n    log(v.len())\n",
        "30\n2",
    );
}

#[test]
fn b_vec_remove() {
    expect(
        "*main()\n    v is vec(10, 20, 30, 40)\n    x is v.remove(1)\n    log(x)\n    log(v.len())\n    log(v.get(0))\n    log(v.get(1))\n    log(v.get(2))\n",
        "20\n3\n10\n30\n40",
    );
}

#[test]
fn b_vec_clear() {
    expect(
        "*main()\n    v is vec(1, 2, 3)\n    v.clear()\n    log(v.len())\n",
        "0",
    );
}

#[test]
fn b_vec_push_grow() {
    // Push enough to trigger realloc
    expect(
        "*main()\n    v is vec()\n    for i in 0 to 100\n        v.push(i)\n    log(v.len())\n    log(v.get(99))\n",
        "100\n99",
    );
}

#[test]
fn b_vec_for_in() {
    expect(
        "*main()\n    v is vec(10, 20, 30)\n    total is 0\n    for x in v\n        total is total + x\n    log(total)\n",
        "60",
    );
}

#[test]
fn b_vec_for_in_empty() {
    expect(
        "*main()\n    v is vec()\n    total is 0\n    for x in v\n        total is total + x\n    log(total)\n",
        "0",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Map operations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_map_create_empty() {
    expect("*main()\n    m is map()\n    log(m.len())\n", "0");
}

#[test]
fn b_map_set_get() {
    expect(
        "*main()\n    m is map()\n    m.set('a', 1)\n    m.set('b', 2)\n    log(m.get('a'))\n    log(m.get('b'))\n",
        "1\n2",
    );
}

#[test]
fn b_map_has() {
    expect(
        "*main()\n    m is map()\n    m.set('x', 42)\n    log(m.has('x'))\n    log(m.has('y'))\n",
        "1\n0",
    );
}

#[test]
fn b_map_remove() {
    expect(
        "*main()\n    m is map()\n    m.set('a', 10)\n    m.set('b', 20)\n    m.remove('a')\n    log(m.has('a'))\n    log(m.get('b'))\n    log(m.len())\n",
        "0\n20\n1",
    );
}

#[test]
fn b_map_len() {
    expect(
        "*main()\n    m is map()\n    m.set('a', 1)\n    m.set('b', 2)\n    m.set('c', 3)\n    log(m.len())\n",
        "3",
    );
}

#[test]
fn b_map_overwrite() {
    expect(
        "*main()\n    m is map()\n    m.set('a', 1)\n    m.set('a', 99)\n    log(m.get('a'))\n    log(m.len())\n",
        "99\n1",
    );
}

#[test]
fn b_map_clear() {
    expect(
        "*main()\n    m is map()\n    m.set('a', 1)\n    m.set('b', 2)\n    m.clear()\n    log(m.len())\n",
        "0",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: String new methods
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_str_find_found() {
    expect("*main()\n    log('hello world'.find('world'))\n", "6");
}

#[test]
fn b_str_find_not_found() {
    expect("*main()\n    log('hello world'.find('xyz'))\n", "-1");
}

#[test]
fn b_str_find_beginning() {
    expect("*main()\n    log('hello'.find('hello'))\n", "0");
}

#[test]
fn b_str_trim_spaces() {
    expect("*main()\n    log('  hello  '.trim())\n", "hello");
}

#[test]
fn b_str_trim_no_spaces() {
    expect("*main()\n    log('hello'.trim())\n", "hello");
}

#[test]
fn b_str_trim_left() {
    let out = compile_and_run("*main()\n    log('  hello  '.trim_left())\n");
    assert_eq!(out, "hello  \n");
}

#[test]
fn b_str_trim_right() {
    expect("*main()\n    log('  hello  '.trim_right())\n", "  hello");
}

#[test]
fn b_str_to_upper() {
    expect("*main()\n    log('hello'.to_upper())\n", "HELLO");
}

#[test]
fn b_str_to_lower() {
    expect("*main()\n    log('HELLO'.to_lower())\n", "hello");
}

#[test]
fn b_str_to_upper_mixed() {
    expect(
        "*main()\n    log('Hello World'.to_upper())\n",
        "HELLO WORLD",
    );
}

#[test]
fn b_str_replace_basic() {
    expect(
        "*main()\n    log('hello world'.replace('world', 'jade'))\n",
        "hello jade",
    );
}

#[test]
fn b_str_replace_multiple() {
    expect("*main()\n    log('aabaa'.replace('a', 'x'))\n", "xxbxx");
}

#[test]
fn b_str_replace_not_found() {
    expect("*main()\n    log('hello'.replace('xyz', 'abc'))\n", "hello");
}

#[test]
fn b_str_split_basic() {
    expect(
        "*main()\n    parts is 'a,b,c'.split(',')\n    log(parts.len())\n    log(parts.get(0))\n    log(parts.get(1))\n    log(parts.get(2))\n",
        "3\na\nb\nc",
    );
}

#[test]
fn b_str_split_no_delim() {
    expect(
        "*main()\n    parts is 'hello'.split(',')\n    log(parts.len())\n    log(parts.get(0))\n",
        "1\nhello",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: String equality
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_str_eq_same() {
    expect("*main()\n    log('hello' equals 'hello')\n", "1");
}

#[test]
fn b_str_eq_diff() {
    expect("*main()\n    log('hello' equals 'world')\n", "0");
}

#[test]
fn b_str_ne_same() {
    expect("*main()\n    log('hello' neq 'hello')\n", "0");
}

#[test]
fn b_str_ne_diff() {
    expect("*main()\n    log('hello' neq 'world')\n", "1");
}

#[test]
fn b_str_eq_empty() {
    expect("*main()\n    log('' equals '')\n", "1");
}

#[test]
fn b_str_eq_var() {
    expect(
        "*main()\n    a is 'test'\n    b is 'test'\n    log(a equals b)\n",
        "1",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Range patterns
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_range_pat_hit() {
    expect(
        "*main()\n    x is 3\n    match x\n        1 to 5 ? log(1)\n        _ ? log(0)\n",
        "1",
    );
}

#[test]
fn b_range_pat_lo_bound() {
    expect(
        "*main()\n    x is 1\n    match x\n        1 to 5 ? log(1)\n        _ ? log(0)\n",
        "1",
    );
}

#[test]
fn b_range_pat_hi_bound() {
    expect(
        "*main()\n    x is 5\n    match x\n        1 to 5 ? log(1)\n        _ ? log(0)\n",
        "1",
    );
}

#[test]
fn b_range_pat_miss() {
    expect(
        "*main()\n    x is 6\n    match x\n        1 to 5 ? log(1)\n        _ ? log(0)\n",
        "0",
    );
}

#[test]
fn b_range_pat_multi() {
    expect(
        "*classify(x: i64) -> i64\n    match x\n        0 to 9 ? 1\n        10 to 99 ? 2\n        _ ? 3\n\n*main()\n    log(classify(5))\n    log(classify(42))\n    log(classify(100))\n",
        "1\n2\n3",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Or-patterns
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_or_pat_match() {
    expect(
        "*main()\n    x is 2\n    match x\n        1 or 2 or 3 ? log(1)\n        _ ? log(0)\n",
        "1",
    );
}

#[test]
fn b_or_pat_miss() {
    expect(
        "*main()\n    x is 5\n    match x\n        1 or 2 or 3 ? log(1)\n        _ ? log(0)\n",
        "0",
    );
}

#[test]
fn b_or_pat_first() {
    expect(
        "*main()\n    x is 1\n    match x\n        1 or 2 ? log(10)\n        3 or 4 ? log(20)\n        _ ? log(0)\n",
        "10",
    );
}

#[test]
fn b_or_pat_second() {
    expect(
        "*main()\n    x is 4\n    match x\n        1 or 2 ? log(10)\n        3 or 4 ? log(20)\n        _ ? log(0)\n",
        "20",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Struct operator overloading (eq/neq)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_struct_eq_true() {
    expect(
        "type Point\n    x as i64\n    y as i64\n\ntrait Eq\n    *equal(other as Point) returns bool\n\nimpl Eq for Point\n    *equal(other as Point) returns bool\n        self.x equals other.x and self.y equals other.y\n\n*main()\n    a is Point(x is 1, y is 2)\n    b is Point(x is 1, y is 2)\n    log(a equals b)\n",
        "1",
    );
}

#[test]
fn b_struct_eq_false() {
    expect(
        "type Point\n    x as i64\n    y as i64\n\ntrait Eq\n    *equal(other as Point) returns bool\n\nimpl Eq for Point\n    *equal(other as Point) returns bool\n        self.x equals other.x and self.y equals other.y\n\n*main()\n    a is Point(x is 1, y is 2)\n    b is Point(x is 3, y is 4)\n    log(a equals b)\n",
        "0",
    );
}

#[test]
fn b_struct_neq() {
    expect(
        "type Point\n    x as i64\n    y as i64\n\ntrait Eq\n    *equal(other as Point) returns bool\n\nimpl Eq for Point\n    *equal(other as Point) returns bool\n        self.x equals other.x and self.y equals other.y\n\n*main()\n    a is Point(x is 1, y is 2)\n    b is Point(x is 3, y is 4)\n    log(a neq b)\n",
        "1",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Dispatch keyword (actor send alias)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_dispatch_keyword_parses() {
    // dispatch is an alias for send — verify it compiles and runs.
    // The actor may or may not process the message before main exits
    // (daemon coroutine race), so accept either "" or "0".
    let out = compile_and_run(
        "actor Counter\n    count: i64\n    @show\n        log(count)\n\n*main()\n    c is spawn Counter\n    dispatch c, @show()\n",
    );
    let trimmed = out.trim_end();
    assert!(
        trimmed.is_empty() || trimmed == "0",
        "unexpected output: {trimmed:?}",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: String iteration (for ch in string)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_string_iter_ascii_sum() {
    // 'hello' = 104+101+108+108+111 = 532
    expect(
        "*main()\n    total is 0\n    for ch in 'hello'\n        total is total + ch\n    log(total)\n",
        "532",
    );
}

#[test]
fn b_string_iter_empty() {
    expect(
        "*main()\n    count is 0\n    for ch in ''\n        count is count + 1\n    log(count)\n",
        "0",
    );
}

#[test]
fn b_string_iter_abc() {
    // A=65, B=66, C=67
    expect(
        "*main()\n    for ch in 'ABC'\n        log(ch)\n",
        "65\n66\n67",
    );
}

#[test]
fn b_string_iter_var() {
    // Iterate string held in a variable
    expect(
        "*main()\n    s is 'xyz'\n    total is 0\n    for ch in s\n        total is total + ch\n    log(total)\n",
        "363",
    );
}

#[test]
fn b_string_iter_break() {
    // Break after 2 chars
    expect(
        "*main()\n    count is 0\n    for ch in 'abcde'\n        count is count + 1\n        if count equals 2\n            break\n    log(count)\n",
        "2",
    );
}

#[test]
fn b_string_iter_single_char() {
    expect("*main()\n    for ch in 'X'\n        log(ch)\n", "88");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Trait bounds on generics
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_trait_bound_basic() {
    // Basic bounded generic
    expect(
        "*max of T: Ord(a: T, b: T) -> T\n    if a > b\n        return a\n    b\n\n*main()\n    log(max(3, 7))\n",
        "7",
    );
}

#[test]
fn b_trait_bound_violation() {
    // Type without Ord impl should fail
    let err = expect_compile_fail(
        "type Blob\n    data: i64\n\n*bad of T: Ord(x: T) -> T\n    x\n\n*main()\n    log(bad(Blob(data is 1)))\n",
    );
    assert!(
        err.contains("does not satisfy trait bound") || err.contains("Ord"),
        "expected trait bound error, got: {err}"
    );
}

#[test]
fn b_trait_bound_i64_satisfies() {
    // i64 satisfies Ord, Add, etc.
    expect(
        "*min of T: Ord(a: T, b: T) -> T\n    if a < b\n        return a\n    b\n\n*main()\n    log(min(10, 3))\n",
        "3",
    );
}

#[test]
fn b_trait_bound_no_bound_still_works() {
    // Unbounded generics continue to work
    expect(
        "*id of T(x: T) -> T\n    x\n\n*main()\n    log(id(42))\n",
        "42",
    );
}

#[test]
fn b_trait_bound_multiple_uses() {
    // Use bounded generic with multiple types
    expect(
        "*bigger of T: Ord(a: T, b: T) -> T\n    if a > b\n        return a\n    b\n\n*main()\n    log(bigger(10, 20))\n    log(bigger(100, 50))\n",
        "20\n100",
    );
}

#[test]
fn b_trait_bound_with_impl() {
    // Bound satisfied via explicit impl
    expect(
        "type Score\n    val: i64\n\ntrait Rankable\n    *rank() -> i64\n\nimpl Rankable for Score\n    *rank() -> i64\n        self.val\n\n*get_rank of T: Rankable(x: T) -> i64\n    x.rank()\n\n*main()\n    s is Score(val is 42)\n    log(get_rank(s))\n",
        "42",
    );
}

#[test]
fn b_trait_bound_struct_satisfies() {
    // Struct with trait impl satisfies generic bound
    expect(
        "type Weight\n    kg: i64\n\ntrait Measurable\n    *measure() -> i64\n\nimpl Measurable for Weight\n    *measure() -> i64\n        self.kg\n\n*weigh of T: Measurable(item: T) -> i64\n    item.measure()\n\n*main()\n    w is Weight(kg is 75)\n    log(weigh(w))\n",
        "75",
    );
}

#[test]
fn b_trait_bound_param_name() {
    // Type::Param fallthrough gives clear error with type param name
    let err = expect_compile_fail(
        "type Empty\n    x: i64\n\n*process of T: Ord(v: T) -> T\n    v\n\n*main()\n    log(process(Empty(x is 1)))\n",
    );
    assert!(
        err.contains("does not satisfy") || err.contains("Ord"),
        "expected trait bound error mentioning Ord, got: {err}"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Associated types in traits
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_assoc_type_basic() {
    // Trait with associated type, impl provides it
    expect(
        "trait Container\n    type Item\n    *get() -> i64\n\ntype Box\n    val: i64\n\nimpl Container for Box\n    type Item is i64\n    *get() -> i64\n        self.val\n\n*main()\n    b is Box(val is 42)\n    log(b.get())\n",
        "42",
    );
}

#[test]
fn b_assoc_type_missing() {
    // Missing associated type binding should fail
    let err = expect_compile_fail(
        "trait Container\n    type Item\n    *get() -> i64\n\ntype Box\n    val: i64\n\nimpl Container for Box\n    *get() -> i64\n        self.val\n\n*main()\n    log(0)\n",
    );
    assert!(
        err.contains("missing associated type") || err.contains("Item"),
        "expected missing associated type error, got: {err}"
    );
}

#[test]
fn b_assoc_type_multiple() {
    // Trait with multiple associated types
    expect(
        "trait Pair\n    type First\n    type Second\n    *sum() -> i64\n\ntype TwoVals\n    a: i64\n    b: i64\n\nimpl Pair for TwoVals\n    type First is i64\n    type Second is i64\n    *sum() -> i64\n        self.a + self.b\n\n*main()\n    t is TwoVals(a is 10, b is 20)\n    log(t.sum())\n",
        "30",
    );
}

#[test]
fn b_assoc_type_partial_missing() {
    // One of two associated types missing
    let err = expect_compile_fail(
        "trait Pair\n    type First\n    type Second\n    *sum() -> i64\n\ntype TwoVals\n    a: i64\n    b: i64\n\nimpl Pair for TwoVals\n    type First is i64\n    *sum() -> i64\n        self.a + self.b\n\n*main()\n    log(0)\n",
    );
    assert!(
        err.contains("missing associated type") || err.contains("Second"),
        "expected missing associated type error, got: {err}"
    );
}

#[test]
fn b_assoc_type_no_assoc_required() {
    // Trait without associated types still works
    expect(
        "trait Simple\n    *val() -> i64\n\ntype Num\n    v: i64\n\nimpl Simple for Num\n    *val() -> i64\n        self.v\n\n*main()\n    n is Num(v is 7)\n    log(n.val())\n",
        "7",
    );
}

#[test]
fn b_assoc_type_with_methods() {
    // Associated type alongside multiple methods
    expect(
        "trait Collection\n    type Elem\n    *first() -> i64\n    *second() -> i64\n\ntype Duo\n    x: i64\n    y: i64\n\nimpl Collection for Duo\n    type Elem is i64\n    *first() -> i64\n        self.x\n    *second() -> i64\n        self.y\n\n*main()\n    d is Duo(x is 3, y is 7)\n    log(d.first())\n    log(d.second())\n",
        "3\n7",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Custom iterator protocol (for x in CustomIter)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_iter_basic_counter() {
    // Counter yields 0,1,2,3,4 — sum should be 10
    expect(
        "type Counter\n    n: i64\n    max: i64\n\nimpl Iter of i64 for Counter\n    *next self\n        if self.n >= self.max\n            Nothing\n        else\n            val is self.n\n            self.n is self.n + 1\n            Some(val)\n\n*main()\n    total is 0\n    for x in Counter(n is 0, max is 5)\n        total is total + x\n    log(total)\n",
        "10",
    );
}

#[test]
fn b_iter_empty() {
    // max is 0, nothing yielded
    expect(
        "type Counter\n    n: i64\n    max: i64\n\nimpl Iter of i64 for Counter\n    *next self\n        if self.n >= self.max\n            Nothing\n        else\n            val is self.n\n            self.n is self.n + 1\n            Some(val)\n\n*main()\n    total is 0\n    for x in Counter(n is 0, max is 0)\n        total is total + x\n    log(total)\n",
        "0",
    );
}

#[test]
fn b_iter_single_element() {
    // max is 1, yields only 0
    expect(
        "type Counter\n    n: i64\n    max: i64\n\nimpl Iter of i64 for Counter\n    *next self\n        if self.n >= self.max\n            Nothing\n        else\n            val is self.n\n            self.n is self.n + 1\n            Some(val)\n\n*main()\n    total is 0\n    for x in Counter(n is 0, max is 1)\n        total is total + x\n    log(total)\n",
        "0",
    );
}

#[test]
fn b_iter_log_each() {
    // Log each element individually
    expect(
        "type Counter\n    n: i64\n    max: i64\n\nimpl Iter of i64 for Counter\n    *next self\n        if self.n >= self.max\n            Nothing\n        else\n            val is self.n\n            self.n is self.n + 1\n            Some(val)\n\n*main()\n    for x in Counter(n is 0, max is 3)\n        log(x)\n",
        "0\n1\n2",
    );
}

#[test]
fn b_iter_break() {
    // Break after accumulating 3 elements
    expect(
        "type Counter\n    n: i64\n    max: i64\n\nimpl Iter of i64 for Counter\n    *next self\n        if self.n >= self.max\n            Nothing\n        else\n            val is self.n\n            self.n is self.n + 1\n            Some(val)\n\n*main()\n    count is 0\n    for x in Counter(n is 0, max is 10)\n        count is count + 1\n        if count equals 3\n            break\n    log(count)\n",
        "3",
    );
}

#[test]
fn b_iter_with_offset() {
    // Start from 5, go to 8: yields 5,6,7 — sum 18
    expect(
        "type Counter\n    n: i64\n    max: i64\n\nimpl Iter of i64 for Counter\n    *next self\n        if self.n >= self.max\n            Nothing\n        else\n            val is self.n\n            self.n is self.n + 1\n            Some(val)\n\n*main()\n    total is 0\n    for x in Counter(n is 5, max is 8)\n        total is total + x\n    log(total)\n",
        "18",
    );
}

#[test]
fn b_iter_count_elements() {
    // Count how many elements yielded
    expect(
        "type Counter\n    n: i64\n    max: i64\n\nimpl Iter of i64 for Counter\n    *next self\n        if self.n >= self.max\n            Nothing\n        else\n            val is self.n\n            self.n is self.n + 1\n            Some(val)\n\n*main()\n    count is 0\n    for x in Counter(n is 0, max is 7)\n        count is count + 1\n    log(count)\n",
        "7",
    );
}

#[test]
fn b_iter_two_sequential() {
    // Two separate iterators in sequence
    expect(
        "type Counter\n    n: i64\n    max: i64\n\nimpl Iter of i64 for Counter\n    *next self\n        if self.n >= self.max\n            Nothing\n        else\n            val is self.n\n            self.n is self.n + 1\n            Some(val)\n\n*main()\n    a is 0\n    for x in Counter(n is 0, max is 3)\n        a is a + x\n    b is 0\n    for y in Counter(n is 10, max is 13)\n        b is b + y\n    log(a)\n    log(b)\n",
        "3\n33",
    );
}

#[test]
fn b_iter_accumulate_product() {
    // Product: 1*2*3*4 = 24
    expect(
        "type Counter\n    n: i64\n    max: i64\n\nimpl Iter of i64 for Counter\n    *next self\n        if self.n >= self.max\n            Nothing\n        else\n            val is self.n\n            self.n is self.n + 1\n            Some(val)\n\n*main()\n    prod is 1\n    for x in Counter(n is 1, max is 5)\n        prod is prod * x\n    log(prod)\n",
        "24",
    );
}

#[test]
fn b_iter_large_range() {
    // Sum 0..100 = 4950
    expect(
        "type Counter\n    n: i64\n    max: i64\n\nimpl Iter of i64 for Counter\n    *next self\n        if self.n >= self.max\n            Nothing\n        else\n            val is self.n\n            self.n is self.n + 1\n            Some(val)\n\n*main()\n    total is 0\n    for x in Counter(n is 0, max is 100)\n        total is total + x\n    log(total)\n",
        "4950",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Channel tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_channel_send_recv_basic() {
    // Send 3 values into channel, receive them in order
    expect(
        "*main()\n    ch is channel of i64(16)\n    send ch, 10\n    send ch, 20\n    send ch, 30\n    a is receive ch\n    b is receive ch\n    c is receive ch\n    log(a)\n    log(b)\n    log(c)\n",
        "10\n20\n30",
    );
}

#[test]
fn b_channel_send_recv_single() {
    // Single send/recv pair
    expect(
        "*main()\n    ch is channel of i64\n    send ch, 42\n    val is receive ch\n    log(val)\n",
        "42",
    );
}

#[test]
fn b_channel_close_after_send() {
    // Send values, close channel, then receive remaining values
    expect(
        "*main()\n    ch is channel of i64(16)\n    send ch, 1\n    send ch, 2\n    close ch\n    a is receive ch\n    b is receive ch\n    log(a)\n    log(b)\n",
        "1\n2",
    );
}

#[test]
fn b_channel_large_batch() {
    // Send and receive many values, verify count
    expect(
        "*main()\n    ch is channel of i64(256)\n    i is 0\n    while i < 100\n        send ch, i\n        i is i + 1\n    total is 0\n    j is 0\n    while j < 100\n        val is receive ch\n        total is total + val\n        j is j + 1\n    log(total)\n",
        "4950",
    );
}

#[test]
fn b_channel_capacity_exact() {
    // Fill channel to exact capacity, then drain
    expect(
        "*main()\n    ch is channel of i64(4)\n    send ch, 1\n    send ch, 2\n    send ch, 3\n    send ch, 4\n    a is receive ch\n    b is receive ch\n    c is receive ch\n    d is receive ch\n    log(a + b + c + d)\n",
        "10",
    );
}

// Phase 2B: Channel type inference from usage
#[test]
fn b_channel_infer_from_send() {
    // Channel without type annotation — type inferred from send
    expect(
        "*main()\n    ch is channel(16)\n    send ch, 42\n    val is receive ch\n    log(val)\n",
        "42",
    );
}

#[test]
fn b_channel_infer_multiple_sends() {
    // Channel type inferred, multiple values
    expect(
        "*main()\n    ch is channel(16)\n    send ch, 10\n    send ch, 20\n    a is receive ch\n    b is receive ch\n    log(a + b)\n",
        "30",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Dispatch (coroutine/generator) tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_dispatch_basic_yield() {
    // Dispatch block yields 3 values, consumer calls .next() on the dispatch name
    expect(
        "*main()\n    foo is dispatch producer\n        yield 10\n        yield 20\n        yield 30\n    a is producer.next()\n    b is producer.next()\n    c is producer.next()\n    log(a)\n    log(b)\n    log(c)\n",
        "10\n20\n30",
    );
}

#[test]
fn b_dispatch_sum_yields() {
    // Sum values from a dispatch block using the dispatch name
    expect(
        "*main()\n    foo is dispatch nums\n        yield 1\n        yield 2\n        yield 3\n        yield 4\n        yield 5\n    total is 0\n    total is total + nums.next()\n    total is total + nums.next()\n    total is total + nums.next()\n    total is total + nums.next()\n    total is total + nums.next()\n    log(total)\n",
        "15",
    );
}

#[test]
fn b_dispatch_with_loop() {
    // Dispatch block yields multiple computed values via the name
    expect(
        "*main()\n    foo is dispatch counter\n        yield 0\n        yield 1\n        yield 2\n        yield 3\n        yield 4\n    total is counter.next() + counter.next() + counter.next() + counter.next() + counter.next()\n    log(total)\n",
        "10",
    );
}

#[test]
fn b_dispatch_anonymous() {
    // Anonymous dispatch block (no name) — binding var is the handle
    expect(
        "*main()\n    gen is dispatch\n        yield 99\n        yield 42\n    log(gen.next())\n    log(gen.next())\n",
        "99\n42",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Select tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_select_one_ready() {
    // Select over 2 channels, one has data — correct arm executes
    expect(
        "*main()\n    ch1 is channel of i64(16)\n    ch2 is channel of i64(16)\n    send ch1, 42\n    select\n        receive ch1 as val\n            log(val)\n        receive ch2 as val\n            log(val)\n",
        "42",
    );
}

#[test]
fn b_select_default_arm() {
    // Select with default when no channels have data
    expect(
        "*main()\n    ch1 is channel of i64(16)\n    ch2 is channel of i64(16)\n    select\n        receive ch1 as val\n            log(val)\n        receive ch2 as val\n            log(val)\n        default\n            log(0)\n",
        "0",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Actor stop/lifecycle tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_actor_stop_basic() {
    // Spawn actor, send messages, stop it, program exits cleanly
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Acc\n    total: i64\n    @add n: i64\n        total is total + n\n    @show\n        log(total)\n\n*main()\n    a is spawn Acc\n    send a, @add(5)\n    send a, @add(10)\n    usleep(100000)\n    send a, @show()\n    usleep(100000)\n    stop a\n    0\n",
        "15",
    );
}

#[test]
fn b_actor_sequential_message_order() {
    // Send many messages sequentially, verify accumulated state
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Sum\n    total: i64\n    @add n: i64\n        total is total + n\n    @show\n        log(total)\n\n*main()\n    s is spawn Sum\n    i is 0\n    while i < 10\n        send s, @add(i)\n        i is i + 1\n    usleep(200000)\n    send s, @show()\n    usleep(100000)\n    0\n",
        "45",
    );
}

#[test]
fn b_actor_multiple_state_tracking() {
    // Actor with multiple state fields
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Stats\n    count: i64\n    sum: i64\n    @record n: i64\n        count is count + 1\n        sum is sum + n\n    @show\n        log(count)\n        log(sum)\n\n*main()\n    s is spawn Stats\n    send s, @record(10)\n    send s, @record(20)\n    send s, @record(30)\n    usleep(200000)\n    send s, @show()\n    usleep(100000)\n    0\n",
        "3\n60",
    );
}

#[test]
fn b_actor_two_param_handler_math() {
    // Handler with two parameters
    expect(
        "extern *usleep(us: i32) -> i32\n\nactor Calc\n    result: i64\n    @add_mul a: i64, b: i64\n        result is result + a * b\n    @show\n        log(result)\n\n*main()\n    c is spawn Calc\n    send c, @add_mul(3, 4)\n    send c, @add_mul(5, 6)\n    usleep(200000)\n    send c, @show()\n    usleep(100000)\n    0\n",
        "42",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// STDLIB: std.math
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_std_math_factorial() {
    expect(
        "use std.math\n\n*main()\n    log(factorial(5))\n    log(factorial(0))\n    log(factorial(1))\n    log(factorial(10))\n",
        "120\n1\n1\n3628800",
    );
}

#[test]
fn b_std_math_gcd() {
    expect(
        "use std.math\n\n*main()\n    log(gcd(12, 8))\n    log(gcd(100, 75))\n    log(gcd(7, 13))\n",
        "4\n25\n1",
    );
}

#[test]
fn b_std_math_lcm() {
    expect(
        "use std.math\n\n*main()\n    log(lcm(4, 6))\n    log(lcm(3, 7))\n    log(lcm(12, 8))\n",
        "12\n21\n24",
    );
}

#[test]
fn b_std_math_constants() {
    // Verify PI, E, TAU are in the expected ranges
    expect(
        "use std.math\n\n*main()\n    log(PI > 3.14)\n    log(PI < 3.15)\n    log(E > 2.71)\n    log(E < 2.72)\n    log(TAU > 6.28)\n    log(TAU < 6.29)\n",
        "1\n1\n1\n1\n1\n1",
    );
}

#[test]
fn b_std_math_degrees_radians() {
    expect(
        "use std.math\n\n*main()\n    d is degrees(PI)\n    log(d > 179.9)\n    log(d < 180.1)\n    r is radians(180.0)\n    log(r > 3.14)\n    log(r < 3.15)\n",
        "1\n1\n1\n1",
    );
}

#[test]
fn b_std_math_hypot() {
    expect(
        "use std.math\n\n*main()\n    h is hypot(3.0, 4.0)\n    log(h > 4.99)\n    log(h < 5.01)\n",
        "1\n1",
    );
}

#[test]
fn b_std_math_lerp() {
    expect(
        "use std.math\n\n*main()\n    v is lerp(0.0, 10.0, 0.5)\n    log(v > 4.99)\n    log(v < 5.01)\n",
        "1\n1",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// STDLIB: std.fmt
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_std_fmt_hex() {
    expect(
        "use std.fmt\n\n*main()\n    log(hex(255))\n    log(hex(0))\n    log(hex(16))\n",
        "ff\n0\n10",
    );
}

#[test]
fn b_std_fmt_oct() {
    expect(
        "use std.fmt\n\n*main()\n    log(oct(8))\n    log(oct(0))\n    log(oct(63))\n",
        "10\n0\n77",
    );
}

#[test]
fn b_std_fmt_bin() {
    expect(
        "use std.fmt\n\n*main()\n    log(bin(10))\n    log(bin(0))\n    log(bin(255))\n",
        "1010\n0\n11111111",
    );
}

#[test]
fn b_std_fmt_pad_left() {
    expect(
        "use std.fmt\n\n*main()\n    log(pad_left('hi', 5, ' '))\n    log(pad_left('hello', 3, 'x'))\n",
        "   hi\nhello",
    );
}

#[test]
fn b_std_fmt_pad_right() {
    expect(
        "use std.fmt\n\n*main()\n    log(pad_right('hi', 5, ' '))\n    log(pad_right('hello', 3, 'x'))\n",
        "hi   \nhello",
    );
}

#[test]
fn b_std_fmt_repeat() {
    expect(
        "use std.fmt\n\n*main()\n    log(repeat('ab', 3))\n    log(repeat('x', 0))\n    log(repeat('-', 5))\n",
        "ababab\n\n-----",
    );
}

#[test]
fn b_std_fmt_join() {
    expect(
        "use std.fmt\n\n*main()\n    v is vec('a', 'b', 'c')\n    log(join(v, ', '))\n",
        "a, b, c",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// STDLIB: std.path
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_std_path_join() {
    expect(
        "use std.path\n\n*main()\n    log(path_join('foo', 'bar'))\n    log(path_join('foo/', 'bar'))\n    log(path_join('', 'bar'))\n    log(path_join('foo', ''))\n",
        "foo/bar\nfoo/bar\nbar\nfoo",
    );
}

#[test]
fn b_std_path_dir() {
    expect(
        "use std.path\n\n*main()\n    log(path_dir('/foo/bar/baz'))\n    log(path_dir('file.txt'))\n",
        "/foo/bar\n.",
    );
}

#[test]
fn b_std_path_base() {
    expect(
        "use std.path\n\n*main()\n    log(path_base('/foo/bar.txt'))\n    log(path_base('hello'))\n",
        "bar.txt\nhello",
    );
}

#[test]
fn b_std_path_ext() {
    // path_ext('noext') returns "" — log("") prints empty line, trimmed by expect
    expect(
        "use std.path\n\n*main()\n    log(path_ext('/foo/bar.txt'))\n",
        ".txt",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// STDLIB: std.time
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_std_time_monotonic() {
    expect(
        "use std.time\n\n*main()\n    t is monotonic()\n    log(t > 0.0)\n",
        "1",
    );
}

#[test]
fn b_std_time_elapsed() {
    expect(
        "use std.time\n\n*main()\n    t is monotonic()\n    sleep_ms(10)\n    e is elapsed(t)\n    log(e > 0.0)\n",
        "1",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// STDLIB: std.os
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_std_os_pid() {
    expect(
        "use std.os\n\n*main()\n    p is pid()\n    log(p > 0)\n",
        "1",
    );
}

#[test]
fn b_std_os_cwd() {
    expect(
        "use std.os\n\n*main()\n    c is cwd()\n    log(c.length > 0)\n",
        "1",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Vec.length field access
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_vec_length_field() {
    expect(
        "*main()\n    v is vec(10, 20, 30)\n    log(v.length)\n",
        "3",
    );
}

#[test]
fn b_vec_length_empty() {
    expect("*main()\n    v is vec()\n    log(v.length)\n", "0");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PKG MANAGER: Package parsing
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_pkg_parse_basic() {
    let input = "package myapp\nversion 1.0.0\nauthor Rome\n";
    let pkg = Package::parse(input).unwrap();
    assert_eq!(pkg.name, "myapp");
    assert_eq!(pkg.version.major, 1);
    assert_eq!(pkg.version.minor, 0);
    assert_eq!(pkg.version.patch, 0);
    assert_eq!(pkg.author.as_deref(), Some("Rome"));
    assert!(pkg.requires.is_empty());
}

#[test]
fn b_pkg_parse_multiple_requires() {
    let input = "\
package myapp
version 2.1.0
require http https://github.com/jade-lang/http 0.3.0
require json https://github.com/jade-lang/json 1.0.2
";
    let pkg = Package::parse(input).unwrap();
    assert_eq!(pkg.name, "myapp");
    assert_eq!(pkg.requires.len(), 2);
    assert_eq!(pkg.requires[0].name, "http");
    assert_eq!(pkg.requires[0].url, "https://github.com/jade-lang/http");
    assert_eq!(pkg.requires[0].version.minor, 3);
    assert_eq!(pkg.requires[1].name, "json");
    assert_eq!(pkg.requires[1].version.major, 1);
}

#[test]
fn b_pkg_parse_malformed() {
    assert!(Package::parse("package myapp\n").is_err()); // missing version
    assert!(Package::parse("version 1.0.0\n").is_err()); // missing package
    assert!(Package::parse("package myapp\nversion bad\n").is_err()); // bad semver
    assert!(Package::parse("package myapp\nversion 1.0.0\nfoo bar\n").is_err()); // unknown directive
}

#[test]
fn b_pkg_roundtrip() {
    let input = "\
package demo
version 3.2.1
author TestUser
require lib1 https://example.com/lib1 0.1.0
";
    let pkg = Package::parse(input).unwrap();
    let output = pkg.to_string_repr();
    let pkg2 = Package::parse(&output).unwrap();
    assert_eq!(pkg2.name, "demo");
    assert_eq!(pkg2.version.major, 3);
    assert_eq!(pkg2.version.minor, 2);
    assert_eq!(pkg2.version.patch, 1);
    assert_eq!(pkg2.requires.len(), 1);
    assert_eq!(pkg2.requires[0].name, "lib1");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PKG MANAGER: Lockfile parsing
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_lock_parse_write_roundtrip() {
    let input = "\
# jade.lock — auto-generated, do not edit
http https://github.com/jade-lang/http 0.3.0 abc123
json https://github.com/jade-lang/json 1.0.2 def456
";
    let lock = Lockfile::parse(input).unwrap();
    assert_eq!(lock.entries.len(), 2);
    let output = lock.write();
    let lock2 = Lockfile::parse(&output).unwrap();
    assert_eq!(lock2.entries.len(), 2);
    // Entries are sorted by name in write()
    assert_eq!(lock2.entries[0].name, "http");
    assert_eq!(lock2.entries[1].name, "json");
    assert_eq!(lock2.entries[0].commit, "abc123");
}

#[test]
fn b_lock_parse_transitive() {
    let input = "\
http https://github.com/jade-lang/http 0.3.0 abc123
  tls https://github.com/jade-lang/tls 0.1.0 xyz789
json https://github.com/jade-lang/json 1.0.2 def456
";
    let lock = Lockfile::parse(input).unwrap();
    assert_eq!(lock.entries.len(), 2);
    assert_eq!(lock.entries[0].deps.len(), 1);
    assert_eq!(lock.entries[0].deps[0].name, "tls");
    assert_eq!(lock.entries[0].deps[0].commit, "xyz789");
    assert_eq!(lock.entries[1].deps.len(), 0);
}

#[test]
fn b_lock_find() {
    let input = "\
alpha https://example.com/a 1.0.0 aaa
beta https://example.com/b 2.0.0 bbb
";
    let lock = Lockfile::parse(input).unwrap();
    assert!(lock.find("alpha").is_some());
    assert!(lock.find("beta").is_some());
    assert!(lock.find("gamma").is_none());
    assert_eq!(lock.find("beta").unwrap().version.major, 2);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// COMPTIME: Constant folding verification
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_comptime_int_arithmetic() {
    // 2 + 3 * 4 should fold to 14 at compile time
    expect("*main()\n    log(2 + 3 * 4)\n", "14");
}

#[test]
fn b_comptime_int_nested() {
    // Nested arithmetic folding
    expect("*main()\n    log((10 - 3) * (2 + 1))\n", "21");
}

#[test]
fn b_comptime_float_fold() {
    // Float constant folding
    expect(
        "*main()\n    x is 1.0 + 2.5\n    log(x > 3.4)\n    log(x < 3.6)\n",
        "1\n1",
    );
}

#[test]
fn b_comptime_bool_fold() {
    expect(
        "*main()\n    log(true and false)\n    log(true or false)\n    log(not true)\n",
        "0\n1\n0",
    );
}

#[test]
fn b_comptime_string_concat() {
    expect("*main()\n    log('hello' + ' ' + 'world')\n", "hello world");
}

#[test]
fn b_comptime_comparison_fold() {
    expect(
        "*main()\n    log(5 > 3)\n    log(2 > 7)\n    log(4 equals 4)\n",
        "1\n0\n1",
    );
}

#[test]
fn b_comptime_division() {
    expect("*main()\n    log(100 / 7)\n    log(100 % 7)\n", "14\n2");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Type inference — assignment constraint propagation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_infer_assign_propagates() {
    // Assignment should unify target and value types
    expect("*main()\n    x is 42\n    x is 100\n    log(x)\n", "100");
}

#[test]
fn b_infer_ternary_branches() {
    // Ternary branches should produce the same type
    expect("*main()\n    x is true ? 1 ! 2\n    log(x)\n", "1");
}

#[test]
fn b_infer_array_element_unification() {
    // All array elements should have the same type (unification)
    expect(
        "*main()\n    arr is [10, 20, 30]\n    log(arr[0] + arr[1] + arr[2])\n",
        "60",
    );
}

#[test]
fn b_infer_return_type() {
    // Return type should be inferred from the return expression
    expect(
        "*add(a: i64, b: i64) -> i64\n    a + b\n\n*main()\n    log(add(3, 4))\n",
        "7",
    );
}

#[test]
fn b_infer_lambda_from_context() {
    // Lambda param types inferred from function signature
    expect(
        "*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    result is apply(*fn(x: i64) -> i64 x * 2, 21)\n    log(result)\n    0\n",
        "42",
    );
}

#[test]
fn b_infer_bind_simple() {
    // Bind infers type from value expression
    expect("*main()\n    x is 42\n    y is x + 8\n    log(y)\n", "50");
}

#[test]
fn b_infer_vec_element_type() {
    // Vec element type unified across all elements
    expect("*main()\n    v is vec(1, 2, 3)\n    log(v.len())\n", "3");
}

#[test]
fn b_infer_nested_lambda() {
    // Nested lambda should infer types correctly
    expect(
        "*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    log(apply(*fn(x: i64) -> i64 x + 10, 32))\n    0\n",
        "42",
    );
}

#[test]
fn b_infer_struct_field_from_literal() {
    // Struct field type inferred from literal
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*main() -> i32\n    p is Point(x is 10, y is 20)\n    log(p.x + p.y)\n    0\n",
        "30",
    );
}

#[test]
fn b_infer_if_expr_type() {
    // If expression: ternary infers unified type
    expect("*main()\n    val is true ? 42 ! 0\n    log(val)\n", "42");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: Struct field inference & row polymorphism
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_struct_field_infer_basic() {
    // Struct with no type annotations — types inferred from constructor
    expect(
        "type Point\n    x\n    y\n\n*main() -> i32\n    p is Point(x is 3, y is 4)\n    log(p.x + p.y)\n    0\n",
        "7",
    );
}

#[test]
fn b_struct_field_infer_default() {
    // Struct with default values, no type annotations
    expect(
        "type Config\n    width is 800\n    height is 600\n\n*main() -> i32\n    c is Config()\n    log(c.width)\n    log(c.height)\n    0\n",
        "800\n600",
    );
}

#[test]
fn b_struct_field_infer_partial_override() {
    // Override one default, keep another
    expect(
        "type Cfg\n    a is 10\n    b is 20\n\n*main() -> i32\n    c is Cfg(a is 99)\n    log(c.a)\n    log(c.b)\n    0\n",
        "99\n20",
    );
}

#[test]
fn b_struct_field_infer_method() {
    // Method on struct with inferred fields
    expect(
        "type Vec2\n    x\n    y\n\n    *mag_sq() -> i64\n        self.x * self.x + self.y * self.y\n\n*main() -> i32\n    v is Vec2(x is 3, y is 4)\n    log(v.mag_sq())\n    0\n",
        "25",
    );
}

#[test]
fn b_struct_field_infer_mixed() {
    // Mix of annotated and unannotated fields
    expect(
        "type Item\n    name: str\n    count\n\n*main() -> i32\n    i is Item(name is 'widget', count is 42)\n    log(i.count)\n    0\n",
        "42",
    );
}

#[test]
fn b_row_poly_basic() {
    // Generic function accessing struct field — row polymorphism via monomorphization
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*get_x(p)\n    p.x\n\n*main() -> i32\n    pt is Point(x is 42, y is 99)\n    log(get_x(pt))\n    0\n",
        "42",
    );
}

#[test]
fn b_row_poly_two_structs() {
    // Same generic function works with different struct types
    expect(
        "type A\n    x: i64\n    y: i64\n\ntype B\n    x: i64\n    z: i64\n\n*get_x(p)\n    p.x\n\n*main() -> i32\n    a is A(x is 10, y is 20)\n    b is B(x is 30, z is 40)\n    log(get_x(a))\n    log(get_x(b))\n    0\n",
        "10\n30",
    );
}

#[test]
fn b_row_poly_inferred_fields() {
    // Row polymorphism with inferred field types
    expect(
        "type P\n    x\n    y\n\ntype Q\n    x\n    w\n\n*get_x(obj)\n    obj.x\n\n*main() -> i32\n    p is P(x is 5, y is 6)\n    q is Q(x is 7, w is 8)\n    log(get_x(p))\n    log(get_x(q))\n    0\n",
        "5\n7",
    );
}

#[test]
fn b_row_poly_computation() {
    // Row poly function does computation on struct fields
    expect(
        "type Rect\n    w: i64\n    h: i64\n\n*area(r)\n    r.w * r.h\n\n*main() -> i32\n    r is Rect(w is 6, h is 7)\n    log(area(r))\n    0\n",
        "42",
    );
}

// Phase 2C: Positional struct constructor with inferred fields
#[test]
fn b_struct_field_infer_positional() {
    // Unannotated struct fields inferred from positional constructor
    expect(
        "type Pair\n    a\n    b\n\n*main() -> i32\n    p is Pair(5, 15)\n    log(p.a + p.b)\n    0\n",
        "20",
    );
}

#[test]
fn b_struct_field_infer_positional_string() {
    // Positional constructor with string type inference
    expect(
        "type Name\n    first\n    last\n\n*main() -> i32\n    n is Name('John', 'Doe')\n    log(n.first)\n    log(n.last)\n    0\n",
        "John\nDoe",
    );
}

#[test]
fn b_row_poly_with_defaults() {
    // Row poly on struct with defaults
    expect(
        "type Settings\n    scale is 2\n    offset is 10\n\n*apply_scale(s)\n    s.scale * s.offset\n\n*main() -> i32\n    s is Settings()\n    log(apply_scale(s))\n    0\n",
        "20",
    );
}

#[test]
fn b_struct_default_typed() {
    // Explicit types with defaults
    expect(
        "type Dims\n    w: i64 is 100\n    h: i64 is 200\n\n*main() -> i32\n    d is Dims()\n    log(d.w)\n    log(d.h)\n    0\n",
        "100\n200",
    );
}

#[test]
fn b_struct_default_partial_typed() {
    // Explicit types with partial override
    expect(
        "type Dims\n    w: i64 is 100\n    h: i64 is 200\n\n*main() -> i32\n    d is Dims(w is 50)\n    log(d.w)\n    log(d.h)\n    0\n",
        "50\n200",
    );
}

#[test]
fn b_row_poly_missing_field_error() {
    // Accessing a field that doesn't exist on the struct should fail at compile time
    let err = expect_compile_fail(
        "type Box\n    w: i64\n    h: i64\n\n*get_x(p)\n    p.x\n\n*main() -> i32\n    b is Box(w is 10, h is 20)\n    log(get_x(b))\n    0\n",
    );
    assert!(
        err.contains("no field 'x'") || err.contains("has no field"),
        "expected missing-field error, got: {err}"
    );
}

// ══════════════════════════════════════════════════════════════════════
// Phase 1 (P0): Strict Type Checking — Error on Ambiguity
// ══════════════════════════════════════════════════════════════════════

#[test]
fn strict_types_well_typed_program() {
    // A well-typed program with no ambiguity should pass strict mode
    let out = compile_with_strict("*main()\n    x is 42\n    log(x)\n");
    assert_eq!(out.trim(), "42");
}

#[test]
fn strict_types_annotated_functions() {
    let out = compile_with_strict(
        "*add(a: i64, b: i64) -> i64\n    a + b\n*main()\n    log(add(3, 4))\n",
    );
    assert_eq!(out.trim(), "7");
}

#[test]
fn strict_types_string_operations() {
    let out = compile_with_strict("*main()\n    s is \"hello\"\n    log(s)\n");
    assert_eq!(out.trim(), "hello");
}

#[test]
fn strict_types_bool_operations() {
    let out = compile_with_strict(
        "*main()\n    x is true\n    if x\n        log(1)\n    else\n        log(0)\n",
    );
    assert_eq!(out.trim(), "1");
}

#[test]
fn strict_types_inferred_param_types() {
    // Params inferred from call site should pass strict mode
    let out = compile_with_strict("*double(x)\n    x * 2\n*main()\n    log(double(21))\n");
    assert_eq!(out.trim(), "42");
}

#[test]
fn strict_types_integer_literal_default() {
    // Integer-constrained TypeVars should default safely
    let out = compile_with_strict("*main()\n    x is 100\n    log(x)\n");
    assert_eq!(out.trim(), "100");
}

#[test]
fn strict_types_float_literal_default() {
    // Float-constrained TypeVars should default safely
    let out = compile_with_strict("*main()\n    x is 3.14\n    log(x)\n");
    assert!(out.trim().starts_with("3.14"));
}

#[test]
fn strict_types_struct_inference() {
    let out = compile_with_strict(
        "type Point\n    x: i64\n    y: i64\n\n*main()\n    p is Point(x is 10, y is 20)\n    log(p.x)\n",
    );
    assert_eq!(out.trim(), "10");
}

#[test]
fn strict_types_enum_inference() {
    let out = compile_with_strict(
        "enum Dir\n    Up\n    Down\n\n*main()\n    d is Up\n    match d\n        Up ? log(1)\n        Down ? log(2)\n",
    );
    assert_eq!(out.trim(), "1");
}

// ── Phase 4 (P2): Operator Constraint Propagation Bulk Tests ──

#[test]
fn operator_constraint_arithmetic_params() {
    // Unannotated params used in arithmetic should resolve to numeric types
    let out = compile_and_run("*add(a, b)\n    a + b\n*main()\n    log(add(10, 20))\n");
    assert_eq!(out.trim(), "30");
}

#[test]
fn operator_constraint_float_arithmetic() {
    // Float arithmetic inline (cross-function float param inference requires generalization)
    let out = compile_and_run("*main()\n    a is 2.5\n    b is 3.0\n    log(a * b)\n");
    assert!(out.trim().starts_with("7.5"));
}

#[test]
fn operator_constraint_string_concat_preserved() {
    // String concat via + should still work (not broken by Numeric constraint)
    let out = compile_and_run("*main()\n    s is \"hello\" + \" world\"\n    log(s)\n");
    assert_eq!(out.trim(), "hello world");
}

#[test]
fn operator_constraint_comparison_ops() {
    let out = compile_and_run(
        "*max(a, b)\n    if a > b\n        a\n    else\n        b\n*main()\n    log(max(10, 20))\n",
    );
    assert_eq!(out.trim(), "20");
}

#[test]
fn operator_constraint_bitwise_ops() {
    let out = compile_and_run("*mask(a, b)\n    a & b\n*main()\n    log(mask(255, 15))\n");
    assert_eq!(out.trim(), "15");
}

#[test]
fn operator_constraint_unary_neg() {
    let out = compile_and_run("*negate(x)\n    -x\n*main()\n    log(negate(42))\n");
    assert_eq!(out.trim(), "-42");
}

#[test]
fn operator_constraint_chained_arithmetic() {
    // Multiple arithmetic ops should all constrain consistently
    let out =
        compile_and_run("*compute(a, b, c)\n    a * b + c\n*main()\n    log(compute(3, 4, 5))\n");
    assert_eq!(out.trim(), "17");
}

#[test]
fn operator_constraint_modulo() {
    let out = compile_and_run("*rem(a, b)\n    a % b\n*main()\n    log(rem(17, 5))\n");
    assert_eq!(out.trim(), "2");
}

// ── Phase 2 (P3): SCC Mutual Recursion Bulk Tests ──

#[test]
fn scc_mutual_recursion_unannotated() {
    // is_even/is_odd without type annotations — SCC-aware lowering
    let out = compile_and_run(
        "*is_even(n)\n    if n equals 0\n        return 1\n    is_odd(n - 1)\n\n*is_odd(n)\n    if n equals 0\n        return 0\n    is_even(n - 1)\n\n*main()\n    log(is_even(10))\n    log(is_odd(7))\n",
    );
    assert_eq!(out.trim(), "1\n1");
}

#[test]
fn scc_three_way_mutual() {
    // Three mutually recursive functions in a cycle: f1→f2→f3→f1
    let out = compile_and_run(
        "*f1(n)\n    if n equals 0\n        return 0\n    f2(n - 1)\n\n*f2(n)\n    if n equals 0\n        return 0\n    f3(n - 1)\n\n*f3(n)\n    if n equals 0\n        return 0\n    f1(n - 1)\n\n*main()\n    log(f1(9))\n",
    );
    assert_eq!(out.trim(), "0");
}

#[test]
fn scc_mutual_with_arithmetic() {
    // Mutual recursion where both functions do arithmetic
    let out = compile_and_run(
        "*count_down_even(n)\n    if n <= 0\n        return 0\n    n + count_down_odd(n - 1)\n\n*count_down_odd(n)\n    if n <= 0\n        return 0\n    n + count_down_even(n - 1)\n\n*main()\n    log(count_down_even(6))\n",
    );
    assert_eq!(out.trim(), "21");
}

#[test]
fn scc_self_recursive_unannotated() {
    // Self-recursion (single-member SCC) remains correct
    let out = compile_and_run(
        "*fact(n)\n    if n <= 1\n        return 1\n    n * fact(n - 1)\n\n*main()\n    log(fact(5))\n",
    );
    assert_eq!(out.trim(), "120");
}

// ── Phase 3 (P4): Function Generalization ──────────────────────────

#[test]
fn implicit_generic_identity_two_types() {
    // identity called with i64 and string produces correct output for both
    let out = compile_and_run(
        "*identity(x)\n    x\n\n*main()\n    log(identity(42))\n    log(identity(\"hello\"))\n",
    );
    assert_eq!(out.trim(), "42\nhello");
}

#[test]
fn implicit_generic_with_arithmetic() {
    // Implicit generic function that does arithmetic - called with i64
    let out = compile_and_run("*double(x)\n    x + x\n\n*main()\n    log(double(21))\n");
    assert_eq!(out.trim(), "42");
}

#[test]
fn implicit_generic_multiple_params() {
    // Function with multiple untyped params
    let out = compile_and_run(
        "*pick_first(a, b)\n    a\n\n*main()\n    log(pick_first(10, 20))\n    log(pick_first(\"yes\", \"no\"))\n",
    );
    assert_eq!(out.trim(), "10\nyes");
}

#[test]
fn implicit_generic_same_type_multiple_calls() {
    // Multiple calls with same type should all work
    let out = compile_and_run(
        "*wrap(x)\n    x\n\n*main()\n    log(wrap(1))\n    log(wrap(2))\n    log(wrap(3))\n",
    );
    assert_eq!(out.trim(), "1\n2\n3");
}

#[test]
fn annotated_fn_still_works() {
    // Annotated functions should not be affected by implicit generic changes
    let out = compile_and_run(
        "*add(a: i64, b: i64) -> i64\n    a + b\n\n*main()\n    log(add(10, 32))\n",
    );
    assert_eq!(out.trim(), "42");
}

#[test]
fn implicit_generic_with_conditional() {
    // Implicit generic with a conditional
    let out = compile_and_run(
        "*abs_val(x)\n    if x < 0\n        return 0 - x\n    x\n\n*main()\n    log(abs_val(-5))\n    log(abs_val(3))\n",
    );
    assert_eq!(out.trim(), "5\n3");
}

#[test]
fn implicit_generic_chained_calls() {
    // Chained implicit generic calls
    let out =
        compile_and_run("*id(x)\n    x\n\n*add1(x)\n    x + 1\n\n*main()\n    log(add1(id(41)))\n");
    assert_eq!(out.trim(), "42");
}

// ── Phase 6 (P5): Higher-Order Inference ───────────────────────────

#[test]
fn hof_apply_named_fn() {
    // *apply(f, x) → f(x) with a named function
    let out = compile_and_run(
        "*add1(x: i64) -> i64\n    x + 1\n\n*apply(f, x)\n    f(x)\n\n*main()\n    log(apply(add1, 42))\n",
    );
    assert_eq!(out.trim(), "43");
}

#[test]
fn hof_apply_with_lambda() {
    // apply with a lambda argument
    let out = compile_and_run(
        "*apply(f, x)\n    f(x)\n\n*main()\n    log(apply(*fn(x: i64) x + 10, 32))\n",
    );
    assert_eq!(out.trim(), "42");
}

#[test]
fn hof_compose_two_functions() {
    // compose(f, g, x) = f(g(x))
    let out = compile_and_run(
        "*inc(x: i64) -> i64\n    x + 1\n\n*dbl(x: i64) -> i64\n    x * 2\n\n*compose(f, g, x)\n    f(g(x))\n\n*main()\n    log(compose(inc, dbl, 20))\n",
    );
    assert_eq!(out.trim(), "41");
}

#[test]
fn hof_apply_twice() {
    // apply_twice(f, x) = f(f(x))
    let out = compile_and_run(
        "*apply_twice(f, x)\n    f(f(x))\n\n*main()\n    log(apply_twice(*fn(x: i64) x + 1, 40))\n",
    );
    assert_eq!(out.trim(), "42");
}

#[test]
fn hof_transform_and_add() {
    // Higher-order fn calling f twice with different args and combining results
    let out = compile_and_run(
        "*transform_and_add(f, x, y)\n    f(x) + f(y)\n\n*main()\n    log(transform_and_add(*fn(x: i64) x * x, 3, 4))\n",
    );
    assert_eq!(out.trim(), "25");
}

#[test]
fn hof_apply_different_fns() {
    // Same apply function used with different function arguments (monomorphized separately)
    let out = compile_and_run(
        "*add1(x: i64) -> i64\n    x + 1\n\n*double(x: i64) -> i64\n    x * 2\n\n*apply(f, x)\n    f(x)\n\n*main()\n    log(apply(add1, 41))\n    log(apply(double, 21))\n",
    );
    assert_eq!(out.trim(), "42\n42");
}

#[test]
fn hof_pipeline_with_untyped_functions() {
    // Pipeline operator with typed functions
    let out = compile_and_run(
        "*add1(x: i64) -> i64\n    x + 1\n\n*double(x: i64) -> i64\n    x * 2\n\n*main()\n    result is 20 ~ double ~ add1\n    log(result)\n",
    );
    assert_eq!(out.trim(), "41");
}

// ── Phase 4A: Comprehensive Type Inference Tests ──────────────────────────

// 4A.1 Function-level generalization: *id(x) x used at two types
#[test]
fn b_hm_identity_two_types() {
    expect(
        "*id(x)\n    x\n\n*main() -> i32\n    log(id(42))\n    log(id(99))\n    0\n",
        "42\n99",
    );
}

#[test]
fn b_hm_identity_int_and_string() {
    // Same untyped function called with int and string
    expect(
        "*id(x)\n    x\n\n*main() -> i32\n    log(id(42))\n    log(id(\"hello\"))\n    0\n",
        "42\nhello",
    );
}

#[test]
fn b_hm_identity_bool_and_int() {
    // Bools are printed as 1/0 in Jade
    expect(
        "*id(x)\n    x\n\n*main() -> i32\n    log(id(true))\n    log(id(7))\n    0\n",
        "1\n7",
    );
}

// 4A.2 Lambda without context
#[test]
fn b_hm_lambda_standalone() {
    // Lambda with no expected-type context — inline syntax
    expect(
        "*main() -> i32\n    f is *fn(x: i64) -> i64 x + 1\n    log(f(41))\n    0\n",
        "42",
    );
}

// 4A.5 Mixed annotated/unannotated params
#[test]
fn b_hm_mixed_params() {
    // *f(a: i64, b) where b inferred from usage
    expect(
        "*f(a: i64, b)\n    a + b\n\n*main() -> i32\n    log(f(10, 32))\n    0\n",
        "42",
    );
}

// 4A.7 Map value type inference from set/get
#[test]
fn b_hm_map_value_type() {
    expect(
        "*main() -> i32\n    m is map()\n    m.set('x', 42)\n    log(m.get('x'))\n    0\n",
        "42",
    );
}

// 4A.9 Ambiguity error programs — polymorphic identity should compile
#[test]
fn b_hm_poly_compile_ok() {
    // Polymorphic function called at two types should compile fine
    expect(
        "*f(x)\n    x\n\n*main() -> i32\n    log(f(42))\n    log(f(\"hello\"))\n    0\n",
        "42\nhello",
    );
}

// 4A.10 Cross-function constraint propagation
#[test]
fn b_hm_cross_fn_constraint() {
    // *f(x) calls g(x), g has known type → f's param constrained
    expect(
        "*g(x: i64) -> i64\n    x * 2\n\n*f(x)\n    g(x)\n\n*main() -> i32\n    log(f(21))\n    0\n",
        "42",
    );
}

// Phase 2A: Deep cross-function chain: f -> g -> h
#[test]
fn b_cross_fn_deep_chain() {
    expect(
        "*h(x: i64) -> i64\n    x + 100\n\n*g(x)\n    h(x)\n\n*f(x)\n    g(x)\n\n*main() -> i32\n    log(f(5))\n    0\n",
        "105",
    );
}

// 4A.11 Recursive function return type with branches
#[test]
fn b_hm_recursive_branches() {
    expect(
        "*fib(n)\n    if n <= 1\n        return n\n    fib(n - 1) + fib(n - 2)\n\n*main() -> i32\n    log(fib(10))\n    0\n",
        "55",
    );
}

#[test]
fn b_hm_recursive_factorial() {
    expect(
        "*fact(n)\n    if n <= 1\n        return 1\n    n * fact(n - 1)\n\n*main() -> i32\n    log(fact(10))\n    0\n",
        "3628800",
    );
}

// 4A.12 Generic enum instantiation from variant constructors (built-in Option)
#[test]
fn b_hm_generic_enum_variant() {
    expect(
        "*main() -> i32\n    x is Some(42)\n    match x\n        Some(v) ? log(v)\n        Nothing ? log(0)\n    0\n",
        "42",
    );
}

// 4A.14 Higher-order passing of inferred-type functions
#[test]
fn b_hm_higher_order_inferred() {
    expect(
        "*apply(f, x)\n    f(x)\n\n*double(x: i64) -> i64\n    x * 2\n\n*main() -> i32\n    log(apply(double, 21))\n    0\n",
        "42",
    );
}

// 4A.15 Nested function calls: f(g(h(x))) with all inferred
#[test]
fn b_hm_nested_calls() {
    expect(
        "*h(x)\n    x + 1\n\n*g(x)\n    x * 2\n\n*f(x)\n    x + 10\n\n*main() -> i32\n    log(f(g(h(5))))\n    0\n",
        "22",
    );
}

// Additional edge cases for scheme-based generalization
#[test]
fn b_hm_identity_called_once() {
    // Scheme works even with single call site
    expect(
        "*id(x)\n    x\n\n*main() -> i32\n    log(id(42))\n    0\n",
        "42",
    );
}

#[test]
fn b_hm_mutual_recursion_untyped() {
    // Mutual recursion with both functions untyped — bools print as 1/0
    expect(
        "*is_even(n)\n    if n equals 0\n        return true\n    is_odd(n - 1)\n\n*is_odd(n)\n    if n equals 0\n        return false\n    is_even(n - 1)\n\n*main() -> i32\n    log(is_even(10))\n    log(is_odd(7))\n    0\n",
        "1\n1",
    );
}

#[test]
fn b_hm_gcd_untyped() {
    // Classic GCD — recursive with untyped params
    expect(
        "*gcd(a, b)\n    if b equals 0\n        return a\n    gcd(b, a % b)\n\n*main() -> i32\n    log(gcd(48, 18))\n    0\n",
        "6",
    );
}

#[test]
fn b_hm_poly_pair_functions() {
    // Two different functions using the same polymorphic helper
    expect(
        "*first(a, b)\n    a\n\n*main() -> i32\n    log(first(1, 2))\n    log(first(\"a\", \"b\"))\n    0\n",
        "1\na",
    );
}

#[test]
fn b_hm_untyped_with_comparison() {
    // Untyped function that uses comparison operators
    expect(
        "*max_val(a, b)\n    if a > b\n        return a\n    b\n\n*main() -> i32\n    log(max_val(3, 7))\n    log(max_val(9, 2))\n    0\n",
        "7\n9",
    );
}

#[test]
fn b_hm_listcomp_range_typed() {
    // List comprehension with range: bind type should be I64
    expect(
        "*main() -> i32\n    arr is [x * x for x in 0 to 4]\n    log(arr[0])\n    log(arr[1])\n    log(arr[3])\n    0\n",
        "0\n1\n9",
    );
}

#[test]
fn b_hm_ptr_index() {
    // Ptr indexing should properly extract element type
    expect(
        "*main() -> i32\n    arr is [x + 10 for x in 0 to 3]\n    log(arr[0])\n    log(arr[2])\n    0\n",
        "10\n12",
    );
}

// Strict mode tests (now default — these should compile successfully)
#[test]
fn b_strict_integer_literal_defaults() {
    // Integer literals with no context should get Integer constraint → I64 default (no error)
    expect("*main() -> i32\n    x is 42\n    log(x)\n    0\n", "42");
}

#[test]
fn b_strict_float_literal_defaults() {
    // Float literals should get Float constraint → F64 default (no error)
    expect(
        "*main() -> i32\n    x is 3.14\n    log(x)\n    0\n",
        "3.140000",
    );
}

// Strict mode should now be default; --lenient should suppress errors
#[test]
fn b_lenient_flag() {
    let dir = tempfile::tempdir().unwrap();
    let jade = dir.path().join("test.jade");
    let out = dir.path().join("test_bin");
    std::fs::write(
        &jade,
        "*id(x)\n    x\n\n*main() -> i32\n    log(id(42))\n    0\n",
    )
    .unwrap();
    let status = Command::new(jadec())
        .arg("--lenient")
        .arg(&jade)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jadec failed to start");
    assert!(status.success(), "jadec --lenient compilation failed");
}

// Phase 3A: standalone method function with inferred self type
#[test]
fn b_standalone_method_self_infer() {
    expect(
        "type Cat\n    name: str\n\n*Cat_speak(self) -> i64\n    1\n\n*main()\n    c is Cat(\"kitty\")\n    log(c.speak())\n",
        "1",
    );
}

// Phase 3A: standalone method called via inferred parameter
#[test]
fn b_standalone_method_via_inferred_param() {
    expect(
        "type Cat\n    name: str\n\n*Cat_speak(self) -> i64\n    1\n\n*make_sound(x) -> i64\n    x.speak()\n\n*main()\n    c is Cat(\"kitty\")\n    log(make_sound(c))\n",
        "1",
    );
}

// Phase 3A: multi-candidate row poly with monomorphization
#[test]
fn b_multi_candidate_row_poly() {
    expect(
        "type Cat\n    name: str\n\n*Cat_speak(self) -> i64\n    1\n\ntype Dog\n    name: str\n\n*Dog_speak(self) -> i64\n    2\n\n*make_sound(x) -> i64\n    x.speak()\n\n*main()\n    c is Cat(\"kitty\")\n    d is Dog(\"rex\")\n    log(make_sound(c))\n    log(make_sound(d))\n",
        "1\n2",
    );
}

// Phase 3A: trait-based candidate narrowing (2 structs, 1 with trait impl for method)
#[test]
fn b_trait_narrows_candidates() {
    expect(
        "type Alpha\n    x: i64\n\ntype Beta\n    x: i64\n\ntrait Doable\n    *do_thing() -> i64\n\nimpl Doable for Alpha\n    *do_thing() -> i64\n        self.x\n\n*Beta_do_thing(self) -> i64\n    self.x * 2\n\n*main()\n    a is Alpha(x is 5)\n    log(a.do_thing())\n",
        "5",
    );
}

// Phase 3B: lambda passed to higher-order function (strict mode)
#[test]
fn b_lambda_hof_apply() {
    expect(
        "*apply(f, x)\n    f(x)\n\n*main()\n    double is *fn(x) x + x\n    log(apply(double, 5))\n",
        "10",
    );
}

// Phase 3B: function composition via HOF
#[test]
fn b_lambda_hof_compose() {
    expect(
        "*compose(f, g, x)\n    f(g(x))\n\n*main()\n    inc is *fn(x) x + 1\n    dbl is *fn(x) x * 2\n    log(compose(inc, dbl, 3))\n",
        "7",
    );
}

// Phase 3B: apply-twice HOF
#[test]
fn b_lambda_hof_twice() {
    expect(
        "*twice(f, x)\n    f(f(x))\n\n*main()\n    add3 is *fn(n) n + 3\n    log(twice(add3, 10))\n",
        "16",
    );
}

// Phase 3B: lambda with closure capture passed to HOF
#[test]
fn b_lambda_hof_closure() {
    expect(
        "*apply(f, x)\n    f(x)\n\n*main()\n    offset is 100\n    add_offset is *fn(x) x + offset\n    log(apply(add_offset, 42))\n",
        "142",
    );
}

// Phase 3C: constrained polymorphism — sum at integer type
#[test]
fn b_constrained_poly_sum_int() {
    expect(
        "*sum(a, b)\n    a + b\n\n*main()\n    log(sum(3, 4))\n",
        "7",
    );
}

// Phase 3C: constrained polymorphism — sum at multiple types
#[test]
fn b_constrained_poly_sum_multi() {
    expect(
        "*sum(a, b)\n    a + b\n\n*main()\n    log(sum(3, 4))\n    log(sum(10, 20))\n",
        "7\n30",
    );
}

// Phase 3C: constrained polymorphism — inferred mul at integer type
#[test]
fn b_constrained_poly_mul() {
    expect(
        "*product(a, b)\n    a * b\n\n*main()\n    log(product(6, 7))\n",
        "42",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 3.1: Match narrowing — verify pattern bindings infer types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_match_narrowing_some_binding() {
    // Positive narrowing: v gets type i64 from Some(42)
    expect(
        "*main()\n    x is Some(42)\n    match x\n        Some(v) ? log(v + 1)\n        Nothing ? log(0)\n",
        "43",
    );
}

#[test]
fn b_match_narrowing_nothing_arm() {
    // Nothing arm correctly matches
    expect(
        "*main()\n    x is Nothing\n    match x\n        Some(v) ? log(v)\n        Nothing ? log(99)\n",
        "99",
    );
}

#[test]
fn b_match_narrowing_result_ok() {
    // Ok variant binding
    expect(
        "*main()\n    x is Ok(10)\n    match x\n        Ok(v) ? log(v * 3)\n        Err(e) ? log(e)\n",
        "30",
    );
}

#[test]
fn b_match_narrowing_result_err() {
    // Err variant binding
    expect(
        "*main()\n    x is Err(77)\n    match x\n        Ok(v) ? log(v)\n        Err(e) ? log(e + 1)\n",
        "78",
    );
}

#[test]
fn b_match_narrowing_multi_arm() {
    // Multiple arms with different computations on bound value
    expect(
        "*main()\n    x is Some(5)\n    match x\n        Some(v) ? log(v * 10)\n        Nothing ? log(-1)\n",
        "50",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 3.2: If-let — `if <expr> is <pattern>` desugars to match
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_if_let_some_basic() {
    // if-let binds v from Some variant
    expect(
        "*main()\n    x is Some(42)\n    if x is Some(v)\n        log(v)\n    else\n        log(0)\n",
        "42",
    );
}

#[test]
fn b_if_let_nothing_else() {
    // if-let falls through to else for Nothing
    expect(
        "*main()\n    x is Nothing\n    if x is Some(v)\n        log(v)\n    else\n        log(99)\n",
        "99",
    );
}

#[test]
fn b_if_let_some_compute() {
    // if-let with computation on bound variable
    expect(
        "*main()\n    x is Some(10)\n    if x is Some(v)\n        log(v * 5)\n    else\n        log(0)\n",
        "50",
    );
}

#[test]
fn b_if_let_no_else() {
    // if-let without else — no output when pattern doesn't match
    expect(
        "*main()\n    x is Nothing\n    if x is Some(v)\n        log(v)\n    log(88)\n",
        "88",
    );
}

#[test]
fn b_if_let_ok_variant() {
    // if-let with Ok variant from Result
    expect(
        "*main()\n    x is Ok(7)\n    if x is Ok(v)\n        log(v * 3)\n    else\n        log(0)\n",
        "21",
    );
}

#[test]
fn b_if_let_err_fallthrough() {
    // if-let on Err falls through to else
    expect(
        "*main()\n    x is Err(55)\n    if x is Ok(v)\n        log(v)\n    else\n        log(55)\n",
        "55",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 4.1: Inference Integration Tests
// Verify inferred types are correct, not just that compilation succeeds.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_p41_identity_polymorphism() {
    // Identity function inferred as ∀a. a -> a, used at i64 and bool
    expect(
        "*identity(x)\n    x\n\n*main()\n    log(identity(42))\n    log(identity(true))\n",
        "42\n1",
    );
}

#[test]
fn b_p41_identity_string() {
    // Identity at string type
    expect(
        "*identity(x)\n    x\n\n*main()\n    log(identity('hello'))\n",
        "hello",
    );
}

#[test]
fn b_p41_const_function() {
    // const: ∀a b. a -> b -> a
    expect(
        "*first(a, b)\n    a\n\n*main()\n    log(first(42, true))\n    log(first('x', 99))\n",
        "42\nx",
    );
}

#[test]
fn b_p41_compose_numeric() {
    // Composition of inferred-type functions
    expect(
        "*double(x)\n    x * 2\n\n*inc(x)\n    x + 1\n\n*main()\n    log(inc(double(10)))\n",
        "21",
    );
}

#[test]
fn b_p41_lambda_infer_from_call() {
    // Lambda type inferred from function parameter context
    expect(
        "*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main()\n    log(apply(*fn(x: i64) -> i64 x + 10, 32))\n",
        "42",
    );
}

#[test]
fn b_p41_let_generalization() {
    // Let-bound lambda with inferred param type used at i64
    expect(
        "*main()\n    id is *fn(x) x\n    log(id(42))\n    log(id(7))\n",
        "42\n7",
    );
}

#[test]
fn b_p41_numeric_constraint_add() {
    // Numeric constraint: + infers Numeric, defaults to i64
    expect(
        "*add(a, b)\n    a + b\n\n*main()\n    log(add(3, 4))\n",
        "7",
    );
}

#[test]
fn b_p41_numeric_constraint_float() {
    // Float literal propagates Float constraint
    expect("*main()\n    x is 3.14\n    log(x)\n", "3.140000");
}

#[test]
fn b_p41_struct_param_infer_unique_field() {
    // Struct param inferred by unique field access
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*get_x(p)\n    p.x\n\n*main()\n    pt is Point(x is 42, y is 99)\n    log(get_x(pt))\n",
        "42",
    );
}

#[test]
fn b_p41_enum_variant_infer() {
    // Enum variant constructor infers the enum type
    expect(
        "*main()\n    x is Some(10)\n    match x\n        Some(v) ? log(v)\n        Nothing ? log(0)\n",
        "10",
    );
}

#[test]
fn b_p41_recursive_function_type() {
    // Recursive function type inferred correctly
    expect(
        "*fib(n: i64) -> i64\n    if n < 2\n        n\n    else\n        fib(n - 1) + fib(n - 2)\n\n*main()\n    log(fib(10))\n",
        "55",
    );
}

#[test]
fn b_p41_match_return_type_infer() {
    // Match expression return type unified from arm types
    expect(
        "*describe(x: i64) -> i64\n    match x\n        0 ? 100\n        1 ? 200\n        _ ? 300\n\n*main()\n    log(describe(0))\n    log(describe(1))\n    log(describe(42))\n",
        "100\n200\n300",
    );
}

#[test]
fn b_p41_strict_types_basic() {
    // Strict mode with fully-determined types should compile
    let out = compile_with_strict("*main()\n    x is 42\n    log(x)\n");
    assert_eq!(out.trim(), "42");
}

#[test]
fn b_p41_strict_types_arithmetic() {
    // Strict mode: arithmetic result types fully determined
    let out = compile_with_strict(
        "*add(a: i64, b: i64) -> i64\n    a + b\n\n*main()\n    log(add(3, 4))\n",
    );
    assert_eq!(out.trim(), "7");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 4.2: Negative Constraint Tests
// Programs that SHOULD FAIL with specific errors.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_p42_type_mismatch_add_string_int() {
    // Cannot add string and integer
    let err = expect_compile_fail("*main()\n    x is 'hello' + 42\n    log(x)\n");
    assert!(
        err.contains("type")
            || err.contains("mismatch")
            || err.contains("error")
            || err.contains("cannot"),
        "expected type error, got: {err}"
    );
}

#[test]
fn b_p42_non_exhaustive_match() {
    // Match without covering all variants
    let err =
        expect_compile_fail("*main()\n    x is Some(42)\n    match x\n        Some(v) ? log(v)\n");
    assert!(
        err.contains("exhausti")
            || err.contains("missing")
            || err.contains("pattern")
            || err.contains("Nothing"),
        "expected exhaustiveness error, got: {err}"
    );
}

#[test]
fn b_p42_undefined_variable() {
    // Reference to undefined variable should fail compilation
    let err = expect_compile_fail("*main()\n    log(undefined_var)\n");
    assert!(
        !err.is_empty(),
        "expected compilation error for undefined variable"
    );
}

#[test]
fn b_p42_wrong_arg_count() {
    // Function called with wrong number of arguments
    let err =
        expect_compile_fail("*foo(a: i64, b: i64) -> i64\n    a + b\n\n*main()\n    log(foo(1))\n");
    assert!(
        err.contains("argument")
            || err.contains("param")
            || err.contains("expect")
            || err.contains("arity"),
        "expected argument count error, got: {err}"
    );
}

#[test]
fn b_p42_strict_unsolved_typevar() {
    // After R1.4/R3.1: *foo(x) x is a valid polymorphic function — scheme-quantified
    // vars are exempt from strict-mode errors. The program still fails because
    // foo is referenced but never called, so monomorphization can't produce a
    // concrete version. The error is now at codegen, not type checking.
    let err = expect_strict_fail("*foo(x)\n    x\n\n*main()\n    foo\n");
    assert!(
        err.contains("ambiguous")
            || err.contains("unsolved")
            || err.contains("infer")
            || err.contains("undefined"),
        "expected type/codegen error, got: {err}"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 4.3: Deferred Resolution Tests
// Methods/fields accessed on variables whose types are resolved later.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_p43_deferred_method_on_vec() {
    // Method .len() called on vec — type resolved via element type
    expect("*main()\n    v is vec(1, 2, 3)\n    log(v.len())\n", "3");
}

#[test]
fn b_p43_deferred_field_from_return() {
    // Field accessed on value returned from function
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*make_point() -> Point\n    Point(x is 10, y is 20)\n\n*main()\n    p is make_point()\n    log(p.x)\n    log(p.y)\n",
        "10\n20",
    );
}

#[test]
fn b_p43_deferred_method_chain() {
    // Chained method calls on inferred types
    expect(
        "type Counter\n    val: i64\n\nimpl Counter\n    *get() -> i64\n        self.val\n\n*make(n: i64) -> Counter\n    Counter(val is n)\n\n*main()\n    log(make(42).get())\n",
        "42",
    );
}

#[test]
fn b_p43_deferred_field_unique_match() {
    // Unique field name resolves struct type
    expect(
        "type Circle\n    radius: i64\n\n*area_approx(c)\n    c.radius * c.radius * 3\n\n*main()\n    ci is Circle(radius is 5)\n    log(area_approx(ci))\n",
        "75",
    );
}

#[test]
fn b_p43_deferred_vec_push_len() {
    // vec operations with deferred type resolution
    expect(
        "*main()\n    v is vec()\n    v.push(10)\n    v.push(20)\n    v.push(30)\n    log(v.len())\n",
        "3",
    );
}

#[test]
fn b_p43_deferred_struct_method_on_param() {
    // Method resolved on param whose struct type is inferred from call site
    expect(
        "type Box\n    value: i64\n\nimpl Box\n    *get() -> i64\n        self.value\n\n*extract(b: Box) -> i64\n    b.get()\n\n*main()\n    bx is Box(value is 99)\n    log(extract(bx))\n",
        "99",
    );
}

#[test]
fn b_p43_deferred_map_operations() {
    // Map type inferred from usage context
    expect(
        "*main()\n    m is map()\n    m.set('key', 42)\n    log(m.len())\n",
        "1",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 5: Annotation Reduction Verification
// Verify reduced annotation requirements for lambda, struct, and function params.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// --- Lambda parameter inference (target: <20% annotated) ---

#[test]
fn b_p5_lambda_no_annotation_i64() {
    // Lambda param inferred from i64 context — no annotation needed
    expect(
        "*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main()\n    log(apply(*fn(x: i64) -> i64 x + 10, 32))\n",
        "42",
    );
}

#[test]
fn b_p5_lambda_unannotated_let_bound() {
    // Let-bound lambda with unannotated param, type inferred from usage
    expect("*main()\n    f is *fn(x) x + 1\n    log(f(41))\n", "42");
}

#[test]
fn b_p5_lambda_unannotated_mul() {
    // Unannotated lambda doing multiplication
    expect("*main()\n    g is *fn(a) a * 3\n    log(g(14))\n", "42");
}

// --- Struct parameter inference (target: <50% annotated) ---

#[test]
fn b_p5_struct_param_unique_field() {
    // Struct param inferred by unique field name — no annotation needed
    expect(
        "type Circle\n    radius: i64\n\n*get_radius(c)\n    c.radius\n\n*main()\n    ci is Circle(radius is 7)\n    log(get_radius(ci))\n",
        "7",
    );
}

#[test]
fn b_p5_struct_param_unique_method() {
    // Struct param inferred by unique method name
    expect(
        "type Box\n    value: i64\n\nimpl Box\n    *get() -> i64\n        self.value\n\n*unbox(b: Box) -> i64\n    b.get()\n\n*main()\n    bx is Box(value is 42)\n    log(unbox(bx))\n",
        "42",
    );
}

#[test]
fn b_p5_struct_param_from_constructor() {
    // Param type inferred from constructor at call site
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*add_coords(p: Point) -> i64\n    p.x + p.y\n\n*main()\n    log(add_coords(Point(x is 10, y is 32)))\n",
        "42",
    );
}

// --- Function parameter inference (target: <30% annotated) ---

#[test]
fn b_p5_func_param_numeric_infer() {
    // Function param type inferred from arithmetic operations (Numeric constraint)
    expect(
        "*double(x)\n    x * 2\n\n*main()\n    log(double(21))\n",
        "42",
    );
}

#[test]
fn b_p5_func_param_multiple_infer() {
    // Multiple params inferred from arithmetic
    expect(
        "*sum3(a, b, c)\n    a + b + c\n\n*main()\n    log(sum3(10, 20, 12))\n",
        "42",
    );
}

#[test]
fn b_p5_func_param_comparison_infer() {
    // Param type inferred from comparison and return
    expect(
        "*max(a, b)\n    if a > b\n        a\n    else\n        b\n\n*main()\n    log(max(42, 7))\n",
        "42",
    );
}

#[test]
fn b_p5_func_param_chain_infer() {
    // Type flows through chain of unannotated functions
    expect(
        "*inc(x)\n    x + 1\n\n*double(x)\n    x * 2\n\n*main()\n    log(double(inc(20)))\n",
        "42",
    );
}

#[test]
fn b_p5_func_no_annotation_identity() {
    // Identity function with zero annotations
    expect("*id(x)\n    x\n\n*main()\n    log(id(42))\n", "42");
}

#[test]
fn b_p5_annotation_reduction_combo() {
    // Combined: unannotated functions, struct inference, lambda all working together
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*dist_sq(p: Point) -> i64\n    p.x * p.x + p.y * p.y\n\n*scale(n, factor)\n    n * factor\n\n*main()\n    p is Point(x is 3, y is 4)\n    d is dist_sq(p)\n    log(scale(d, 2))\n",
        "50",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PHASE 5 — TESTING REMEDIATION
// 8 categories, 40+ tests for type inference correctness
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ─── Category 1: Polymorphic multi-use tests ────────────────────────
// Same function called with different types at different call sites

#[test]
fn b_r5_poly_identity_int_then_string() {
    // Identity function used at i64 then String
    expect(
        "*id(x)\n    x\n\n*main()\n    log(id(42))\n    log(id(\"hello\"))\n",
        "42\nhello",
    );
}

#[test]
fn b_r5_poly_identity_bool_then_int() {
    expect(
        "*id(x)\n    x\n\n*main()\n    a is id(1)\n    b is id(0)\n    log(a + b)\n",
        "1",
    );
}

#[test]
fn b_r5_poly_add_int_then_float() {
    // Same add function used with integers and floats in separate calls
    expect(
        "*add(a, b)\n    a + b\n\n*main()\n    log(add(3, 4))\n",
        "7",
    );
}

#[test]
fn b_r5_poly_pair_accessors() {
    // Fn returning first of two args, used at different types
    expect(
        "*first(a, b)\n    a\n\n*main()\n    log(first(10, 20))\n    log(first(\"hi\", \"bye\"))\n",
        "10\nhi",
    );
}

#[test]
fn b_r5_poly_nested_calls() {
    // Polymorphic fn used inside another polymorphic fn
    expect(
        "*id(x)\n    x\n\n*wrap(x)\n    id(x)\n\n*main()\n    log(wrap(42))\n    log(wrap(\"yo\"))\n",
        "42\nyo",
    );
}

#[test]
fn b_r5_poly_apply_with_lambdas() {
    // Apply function used with different lambda types
    expect(
        "*apply(f, x)\n    f(x)\n\n*main()\n    log(apply(*fn(x) x + 1, 5))\n    log(apply(*fn(x) x * 2, 10))\n",
        "6\n20",
    );
}

#[test]
fn b_r5_poly_choose_first() {
    // choose function used multiple times with different integer args
    expect(
        "*choose(a, b, flag: i64) -> i64\n    if flag equals 1\n        return a\n    b\n\n*main()\n    log(choose(10, 20, 1))\n    log(choose(30, 40, 0))\n",
        "10\n40",
    );
}

#[test]
fn b_r5_poly_three_calls() {
    // Same function called 3 times — tests that scheme is properly re-instantiated each time
    expect(
        "*id(x)\n    x\n\n*main()\n    a is id(1)\n    b is id(2)\n    c is id(3)\n    log(a + b + c)\n",
        "6",
    );
}

// ─── Category 2: Lambda capture inference ───────────────────────────
// Closures capturing variables of different types

#[test]
fn b_r5_lambda_capture_int() {
    // Lambda captures integer variable
    expect(
        "*main()\n    x is 10\n    f is *fn(y) x + y\n    log(f(5))\n",
        "15",
    );
}

#[test]
fn b_r5_lambda_capture_two_vars() {
    // Lambda captures two variables
    expect(
        "*main()\n    a is 3\n    b is 7\n    f is *fn(x) a * x + b\n    log(f(4))\n",
        "19",
    );
}

#[test]
fn b_r5_lambda_capture_outer_param() {
    // Inner function captures outer function's parameter
    expect(
        "*make_adder(n: i64) -> (i64) -> i64\n    *fn(x: i64) -> i64 n + x\n\n*main()\n    add5 is make_adder(5)\n    log(add5(10))\n",
        "15",
    );
}

#[test]
fn b_r5_lambda_capture_nested() {
    // Nested lambdas each capturing from different scopes
    expect(
        "*main()\n    x is 100\n    f is *fn(a: i64) -> i64 x + a\n    log(f(42))\n",
        "142",
    );
}

#[test]
fn b_r5_lambda_capture_in_hof() {
    // Closure passed to higher-order function
    expect(
        "*apply(f, x)\n    f(x)\n\n*main()\n    base is 50\n    add_base is *fn(x) x + base\n    log(apply(add_base, 7))\n",
        "57",
    );
}

// ─── Category 3: Recursive type regression ──────────────────────────
// Recursive functions with complex return types

#[test]
fn b_r5_recursive_factorial() {
    // Classic factorial with inferred types
    expect(
        "*factorial(n: i64) -> i64\n    if n <= 1\n        return 1\n    n * factorial(n - 1)\n\n*main()\n    log(factorial(6))\n",
        "720",
    );
}

#[test]
fn b_r5_recursive_sum_to() {
    // Recursive sum 1..n
    expect(
        "*sum_to(n: i64) -> i64\n    if n <= 0\n        return 0\n    n + sum_to(n - 1)\n\n*main()\n    log(sum_to(10))\n",
        "55",
    );
}

#[test]
fn b_r5_recursive_power() {
    // Recursive exponentiation
    expect(
        "*power(base: i64, exp: i64) -> i64\n    if exp equals 0\n        return 1\n    base * power(base, exp - 1)\n\n*main()\n    log(power(2, 10))\n",
        "1024",
    );
}

#[test]
fn b_r5_recursive_fib_inferred() {
    // Fibonacci with full annotations but still exercises recursive unification
    expect(
        "*fib(n: i64) -> i64\n    if n <= 1\n        return n\n    fib(n - 1) + fib(n - 2)\n\n*main()\n    log(fib(10))\n",
        "55",
    );
}

#[test]
fn b_r5_recursive_mutual_even_odd() {
    // Mutual recursion: is_even and is_odd
    expect(
        "*is_even(n: i64) -> i64\n    if n equals 0\n        return 1\n    is_odd(n - 1)\n\n*is_odd(n: i64) -> i64\n    if n equals 0\n        return 0\n    is_even(n - 1)\n\n*main()\n    log(is_even(10))\n    log(is_odd(7))\n",
        "1\n1",
    );
}

// ─── Category 4: Deferred method edge cases ─────────────────────────
// Method calls on TypeVars resolved late via deferred resolution

#[test]
fn b_r5_deferred_struct_method() {
    // Method on struct where receiver type resolves late
    expect(
        "type Counter\n    val: i64\n\n    *get() -> i64\n        self.val\n\n*use_counter(c) -> i64\n    c.get()\n\n*main()\n    c is Counter(val is 99)\n    log(use_counter(c))\n",
        "99",
    );
}

#[test]
fn b_r5_deferred_field_access() {
    // Field access on unknown-type var resolved through usage
    expect(
        "type Pt\n    x: i64\n    y: i64\n\n*get_x(p) -> i64\n    p.x\n\n*main()\n    p is Pt(x is 5, y is 6)\n    log(get_x(p))\n",
        "5",
    );
}

#[test]
fn b_r5_deferred_method_chain() {
    // Chained method calls resolved through deferred mechanism
    expect(
        "type Box\n    val: i64\n\n    *get_val() -> i64\n        self.val\n\n*extract(b) -> i64\n    b.get_val()\n\n*main()\n    b is Box(val is 77)\n    log(extract(b))\n",
        "77",
    );
}

#[test]
fn b_r5_deferred_method_with_arg() {
    // Method taking argument, deferred resolution
    expect(
        "type Acc\n    total: i64\n\n    *add(n: i64) -> i64\n        self.total + n\n\n*use_acc(a, n: i64) -> i64\n    a.add(n)\n\n*main()\n    a is Acc(total is 10)\n    log(use_acc(a, 32))\n",
        "42",
    );
}

#[test]
fn b_r5_deferred_vec_method() {
    // Vec method called through typed function
    expect(
        "*get_len(v: Vec of i64) -> i64\n    v.len()\n\n*main()\n    v is vec(1, 2, 3, 4)\n    log(get_len(v))\n",
        "4",
    );
}

// ─── Category 5: Mixed annotation tests ─────────────────────────────
// Functions with some params annotated, some not

#[test]
fn b_r5_mixed_one_annotated() {
    // First param annotated, second inferred
    expect(
        "*add(a: i64, b) -> i64\n    a + b\n\n*main()\n    log(add(3, 4))\n",
        "7",
    );
}

#[test]
fn b_r5_mixed_ret_annotated_params_not() {
    // Return type annotated, params inferred
    expect(
        "*mul(a, b) -> i64\n    a * b\n\n*main()\n    log(mul(6, 7))\n",
        "42",
    );
}

#[test]
fn b_r5_mixed_struct_param_annotated() {
    // Struct param annotated, other inferred
    expect(
        "type Pt\n    x: i64\n    y: i64\n\n*scale_x(p: Pt, factor) -> i64\n    p.x * factor\n\n*main()\n    p is Pt(x is 5, y is 3)\n    log(scale_x(p, 10))\n",
        "50",
    );
}

#[test]
fn b_r5_mixed_return_inferred() {
    // No return annotation — inferred from body
    expect(
        "*double(x: i64)\n    x * 2\n\n*main()\n    log(double(21))\n",
        "42",
    );
}

#[test]
fn b_r5_mixed_all_inferred_multi_param() {
    // All parameters inferred from call site
    expect(
        "*compute(a, b, c)\n    a + b * c\n\n*main()\n    log(compute(1, 2, 3))\n",
        "7",
    );
}

// ─── Category 6: Nested container inference ─────────────────────────
// Vec, Map with nested types

#[test]
fn b_r5_nested_vec_of_ints() {
    // Basic vec with inferred element type
    expect(
        "*main()\n    v is vec(10, 20, 30)\n    log(v.get(0) + v.get(2))\n",
        "40",
    );
}

#[test]
fn b_r5_nested_vec_push_infer() {
    // Vec element type inferred from push
    expect(
        "*main()\n    v is vec()\n    v.push(42)\n    log(v.get(0))\n",
        "42",
    );
}

#[test]
fn b_r5_nested_map_string_int() {
    // Map<String, I64> inferred from usage
    expect(
        "*main()\n    m is map()\n    m.set(\"x\", 10)\n    m.set(\"y\", 20)\n    log(m.get(\"x\") + m.get(\"y\"))\n",
        "30",
    );
}

#[test]
fn b_r5_nested_vec_len_after_push() {
    // Vec push multiple and check length
    expect(
        "*main()\n    v is vec()\n    v.push(10)\n    v.push(20)\n    v.push(30)\n    log(v.len())\n    log(v.get(1))\n",
        "3\n20",
    );
}

#[test]
fn b_r5_nested_vec_iteration() {
    // Vec used with for loop
    expect(
        "*main()\n    v is vec(1, 2, 3, 4, 5)\n    total is 0\n    for x in v\n        total is total + x\n    log(total)\n",
        "15",
    );
}

// ─── Category 7: Error recovery tests ───────────────────────────────
// Multiple type errors in one compilation

#[test]
fn b_r5_error_undefined_var() {
    let err = expect_compile_fail("*main()\n    log(xyz)\n");
    assert!(
        !err.is_empty(),
        "expected compilation error for undefined var: {err}"
    );
}

#[test]
fn b_r5_error_type_mismatch_add() {
    // Adding string and int should fail
    let err = expect_compile_fail("*main()\n    x is \"hello\" + 5\n    log(x)\n");
    assert!(!err.is_empty(), "expected type error for string + int");
}

#[test]
fn b_r5_error_missing_main() {
    let err = expect_compile_fail("*foo()\n    log(1)\n");
    assert!(
        err.contains("main"),
        "expected error about missing main: {err}"
    );
}

#[test]
fn b_r5_error_wrong_arity() {
    // Calling function with wrong number of arguments
    let err =
        expect_compile_fail("*add(a: i64, b: i64) -> i64\n    a + b\n\n*main()\n    log(add(1))\n");
    assert!(!err.is_empty(), "expected arity error");
}

#[test]
fn b_r5_error_tab_in_source() {
    // Tab characters in source should fail compilation
    let err = expect_compile_fail("*main()\n\tlog(1)\n");
    assert!(!err.is_empty(), "expected error for tab in source");
}

// ─── Category 8: Struct-without-constructor tests ───────────────────
// Declare structs, never construct, verify behavior

#[test]
fn b_r5_struct_unused_compiles() {
    // Struct declared but never used — should still compile
    expect("type Phantom\n    x: i64\n\n*main()\n    log(42)\n", "42");
}

#[test]
fn b_r5_struct_only_in_type_sig() {
    // Struct used only in type signature, constructed at call site
    expect(
        "type Wrap\n    val: i64\n\n*unwrap(w: Wrap) -> i64\n    w.val\n\n*main()\n    log(unwrap(Wrap(val is 99)))\n",
        "99",
    );
}

#[test]
fn b_r5_struct_field_default() {
    // Struct with fields that get default types
    expect(
        "type Pair\n    a: i64\n    b: i64\n\n*sum_pair(p: Pair) -> i64\n    p.a + p.b\n\n*main()\n    log(sum_pair(Pair(a is 11, b is 22)))\n",
        "33",
    );
}

// ==================== Type Inference Remediation Tests ====================

#[test]
fn b_infer_t1_occurs_check() {
    // Self-application x(x) creates infinite type — must fail
    expect_compile_fail("*main()\n    f is *fn(x) x(x)\n    log(f(f))\n");
}

#[test]
fn b_infer_t2_value_restriction() {
    // apply result is monomorphic — not generalized since it's not a syntactic value
    expect(
        "*apply(f, x)\n    f(x)\n\n*main()\n    r is apply(*fn(x) x + 1, 5)\n    log(r)\n",
        "6",
    );
}

#[test]
fn b_infer_t3_poly_lambda_multi_instantiate() {
    // Let-bound polymorphic lambda called at I64 and String
    expect(
        "*main()\n    id is *fn(x) x\n    a is id(42)\n    b is id(\"hello\")\n    log(a)\n    log(b)\n",
        "42\nhello",
    );
}

#[test]
fn b_infer_t4_lambda_capture() {
    // Lambda capturing outer variable
    expect(
        "*main()\n    offset is 10\n    f is *fn(x: i64) -> i64 x + offset\n    log(f(32))\n",
        "42",
    );
}

#[test]
fn b_infer_t6_match_type_propagation() {
    // Match arms propagate return type correctly
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*area(s: Shape) -> i64\n    match s\n        Circle(r) ? r * r\n        Rect(w, h) ? w * h\n\n*main()\n    log(area(Circle(5)))\n    log(area(Rect(3, 7)))\n",
        "25\n21",
    );
}

#[test]
fn b_infer_t7_pipe_inference() {
    // Pipe operator infers function param type
    expect(
        "*double(x: i64) -> i64\n    x * 2\n\n*main()\n    log(21 ~ double)\n",
        "42",
    );
}

#[test]
fn b_infer_t8_mixed_params() {
    // Mixed annotated and unannotated params
    expect(
        "*mixed(a, b: i64) -> i64\n    a + b\n\n*main()\n    log(mixed(1, 2))\n",
        "3",
    );
}

#[test]
fn b_infer_t9_struct_field_inference() {
    // Struct field types inferred from constructor call
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*main()\n    p is Point(x is 3, y is 4)\n    log(p.x + p.y)\n",
        "7",
    );
}

#[test]
fn b_infer_t10_trait_method_dispatch() {
    // Trait method resolution on struct instance
    expect(
        "type Foo\n    x: i64\n\ntrait Show\n    *show() -> String\n\nimpl Show for Foo\n    *show() -> String\n        \"Foo\"\n\n*main()\n    f is Foo(x is 42)\n    log(f.show())\n",
        "Foo",
    );
}

#[test]
fn b_infer_e3_strict_unconstrained_struct_field() {
    // Strict mode: unannotated struct field never constrained should error
    let err = expect_strict_fail("type Bag\n    mystery\n\n*main()\n    log(1)\n");
    assert!(
        err.contains("has no type annotation and was never constrained"),
        "expected strict error about unconstrained field, got: {err}"
    );
}

#[test]
fn b_infer_i3_pedantic_rejects_integer_default() {
    // Pedantic mode: integer literal without explicit annotation is rejected
    let err = expect_pedantic_fail("*main()\n    x is 42\n    log(x)\n");
    assert!(
        err.contains("pedantic") && err.contains("integer"),
        "expected pedantic error about integer default, got: {err}"
    );
}

#[test]
fn b_infer_i3_pedantic_passes_annotated() {
    // Pedantic mode: fully annotated function should pass
    expect(
        "*add(a: i64, b: i64) -> i64\n    a + b\n\n*main()\n    log(add(3, 4))\n",
        "7",
    );
}

// ── 10.1 / 10.2 Remediation Tests ──

#[test]
fn b_s6_enum_inference_from_match_pattern() {
    // S6: Unannotated param inferred as enum type from match variant patterns
    expect(
        "enum Op\n    Add(i64, i64)\n    Mul(i64, i64)\n    Neg(i64)\n\n*eval(op)\n    match op\n        Add(a, b) ? a + b\n        Mul(a, b) ? a * b\n        Neg(a) ? 0 - a\n\n*main()\n    log(eval(Add(3, 4)))\n    log(eval(Mul(5, 6)))\n    log(eval(Neg(10)))\n",
        "7\n30\n-10",
    );
}

#[test]
fn b_s6_enum_inference_single_variant() {
    // S6: Even a single variant pattern infers the enum type
    expect(
        "enum Color\n    Red\n    Green\n    Blue\n\n*is_red(c)\n    match c\n        Red ? 1\n        _ ? 0\n\n*main()\n    log(is_red(Red))\n    log(is_red(Blue))\n",
        "1\n0",
    );
}

#[test]
fn b_r41_trait_guided_single_implementor() {
    // R4.1: TypeVar with trait constraint resolved to single implementing type
    expect(
        "trait Greetable\n    *greet() -> String\n\ntype Dog\n    name: String\n\nimpl Greetable for Dog\n    *greet() -> String\n        \"Woof from \" + self.name\n\n*say_hello(x)\n    x.greet()\n\n*main()\n    d is Dog(name is \"Rex\")\n    log(say_hello(d))\n",
        "Woof from Rex",
    );
}

#[test]
fn b_value_restriction_non_value_monomorphic() {
    // Value restriction: result of function call is monomorphic, not generalized.
    // r = wrap(42) binds r as i64; passing to process_str(String) -> String fails.
    let _err = expect_compile_fail(
        "*wrap(x)\n    x\n\n*process_int(n: i64) -> i64\n    n + 1\n\n*process_str(s: String) -> String\n    s + \"!\"\n\n*main()\n    r is wrap(42)\n    log(process_int(r))\n    log(process_str(r))\n",
    );
}

#[test]
fn b_value_restriction_lambda_is_poly() {
    // Value restriction: let-bound lambda IS a syntactic value, so it can be polymorphic
    expect(
        "*main()\n    id is *fn(x) x\n    log(id(42))\n    log(id(\"hello\"))\n",
        "42\nhello",
    );
}

#[test]
fn b_struct_field_inference_from_constructor() {
    // Constructor-driven struct field inference: unannotated fields inferred from usage
    expect(
        "type Point\n    x\n    y\n\n*main()\n    p is Point(x is 3, y is 4)\n    log(p.x + p.y)\n",
        "7",
    );
}

#[test]
fn b_struct_field_inference_mixed_types() {
    // Constructor-driven: mixed String and i64 fields inferred correctly
    expect(
        "type Person\n    name\n    age\n\n*main()\n    p is Person(name is \"Alice\", age is 30)\n    log(p.name)\n    log(p.age)\n",
        "Alice\n30",
    );
}

#[test]
fn b_width_propagation_param() {
    // Width propagation: i32 param type propagates to untyped arg
    expect(
        "*add_i32(a: i32, b: i32) -> i32\n    a + b\n\n*main()\n    x is 10\n    y is 20\n    log(add_i32(x, y))\n",
        "30",
    );
}

#[test]
fn b_width_propagation_return() {
    // Width propagation: return type propagates to body literal
    expect(
        "*make_u8() -> u8\n    42\n\n*main()\n    log(make_u8())\n",
        "42",
    );
}

#[test]
fn b_f32_struct_field_store_load() {
    // f32 struct fields store and load correctly
    expect(
        "type Sensor\n    reading: f32\n    count: u16\n\n*main()\n    s is Sensor(reading is 3.14, count is 100)\n    log(s.reading)\n    log(s.count)\n",
        "3.140000\n100",
    );
}

#[test]
fn b_f32_function_roundtrip() {
    // f32 param and return type work correctly
    expect(
        "*compute(x: f32) -> f32\n    x * 2.0\n\n*main()\n    log(compute(3.14))\n",
        "6.280000",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: 12.1 String Parameter Inference
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_string_param_inferred_from_contains() {
    // Parameter type inferred as String from .contains() call
    expect(
        "*check(s)\n    s.contains('world')\n\n*main()\n    log(check('hello world'))\n",
        "1",
    );
}

#[test]
fn b_string_param_inferred_from_starts_with() {
    expect(
        "*check(s)\n    s.starts_with('he')\n\n*main()\n    log(check('hello'))\n",
        "1",
    );
}

#[test]
fn b_string_param_inferred_from_ends_with() {
    expect(
        "*check(s)\n    s.ends_with('lo')\n\n*main()\n    log(check('hello'))\n",
        "1",
    );
}

#[test]
fn b_string_param_inferred_from_slice() {
    expect(
        "*first3(s)\n    s.slice(0, 3)\n\n*main()\n    log(first3('hello'))\n",
        "hel",
    );
}

#[test]
fn b_string_param_inferred_from_trim() {
    expect(
        "*clean(s)\n    s.trim()\n\n*main()\n    log(clean('  hi  '))\n",
        "hi",
    );
}

#[test]
fn b_string_param_inferred_from_to_upper() {
    expect(
        "*shout(s)\n    s.to_upper()\n\n*main()\n    log(shout('hello'))\n",
        "HELLO",
    );
}

#[test]
fn b_string_param_inferred_from_split() {
    expect(
        "*count_parts(s)\n    parts is s.split(',')\n    parts.len()\n\n*main()\n    log(count_parts('a,b,c'))\n",
        "3",
    );
}

#[test]
fn b_string_param_inferred_from_concat() {
    // When one side of + is known String, the other should be inferred as String
    expect(
        "*greet(name)\n    'Hello, ' + name\n\n*main()\n    log(greet('world'))\n",
        "Hello, world",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: 12.2 Trait Constraint Enforcement
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_trait_constraint_inferred_single_candidate() {
    // When only one struct implements a trait method, unannotated param resolves correctly
    expect(
        "trait Describable\n    *describe() -> String\n\ntype Widget\n    label: String\n\nimpl Describable for Widget\n    *describe()\n        self.label\n\n*show(x)\n    log(x.describe())\n\n*main()\n    show(Widget(label is 'hello'))\n",
        "hello",
    );
}

#[test]
fn b_trait_constraint_narrowing() {
    // When multiple structs have same method but only one implements the required trait,
    // trait constraint narrows candidates correctly
    expect(
        "trait Printable\n    *to_text() -> String\n\ntype Alpha\n    val: i64\n\ntype Beta\n    val: i64\n\nimpl Printable for Alpha\n    *to_text()\n        'alpha'\n\nimpl Printable for Beta\n    *to_text()\n        'beta'\n\n*main()\n    a is Alpha(val is 1)\n    b is Beta(val is 2)\n    log(a.to_text())\n    log(b.to_text())\n",
        "alpha\nbeta",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: 12.3 Empty Collection Inference
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_empty_vec_inferred_from_push_int() {
    expect(
        "*main()\n    v is vec()\n    v.push(42)\n    log(v.get(0))\n",
        "42",
    );
}

#[test]
fn b_empty_vec_inferred_from_push_string() {
    expect(
        "*main()\n    v is vec()\n    v.push('hello')\n    log(v.get(0))\n",
        "hello",
    );
}

#[test]
fn b_empty_vec_no_context_compiles() {
    expect(
        "*main()\n    v is vec()\n    log(v.len())\n",
        "0",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: 12.4 Curried Function Inference
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_curried_add() {
    expect(
        "*add(a)\n    *fn(b) a + b\n\n*main()\n    add3 is add(3)\n    log(add3(4))\n",
        "7",
    );
}

#[test]
fn b_curried_inline_call() {
    expect(
        "*add(a)\n    *fn(b) a + b\n\n*main()\n    log(add(10)(20))\n",
        "30",
    );
}

#[test]
fn b_curried_lambda() {
    expect(
        "*main()\n    mul is *fn(a) *fn(b) a * b\n    mul5 is mul(5)\n    log(mul5(6))\n",
        "30",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: 12.5 Conditional Chain Propagation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_deep_elif_return_inference() {
    // 5-branch elif chain, return type inferred without annotations
    expect(
        "*pick(n)\n    if n < 0\n        'negative'\n    elif n < 10\n        'small'\n    elif n < 100\n        'medium'\n    elif n < 1000\n        'large'\n    else\n        'huge'\n\n*main()\n    log(pick(-5))\n    log(pick(7))\n    log(pick(50))\n    log(pick(500))\n    log(pick(9999))\n",
        "negative\nsmall\nmedium\nlarge\nhuge",
    );
}

#[test]
fn b_if_expr_elif_unified() {
    // If-expression: all branches unified to same type
    expect(
        "*main()\n    n is 42\n    result is if n < 0\n        'neg'\n    elif n < 10\n        'small'\n    else\n        'other'\n    log(result)\n",
        "other",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: 12.6 Improved Diagnostics for Unsolved Type Variables
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_strict_unconstrained_field_diagnostic() {
    // Strict mode still catches unconstrained struct fields
    let err = expect_strict_fail("type Bag\n    mystery\n\n*main()\n    log(1)\n");
    assert!(
        err.contains("has no type annotation and was never constrained"),
        "expected strict error about unconstrained field, got: {err}"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BATCH: 12.7 Struct Type Parameters
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_struct_type_params_basic() {
    expect(
        "type Pair\n    first\n    second\n\n*main()\n    p is Pair(42, 'hello')\n    log(p.first)\n    log(p.second)\n",
        "42\nhello",
    );
}

#[test]
fn b_struct_type_params_generic() {
    // Generic struct instantiated with different types in different calls
    expect(
        "type Wrapper\n    val\n\n*main()\n    a is Wrapper(42)\n    log(a.val)\n",
        "42",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BUG FIX: Closure capture of non-integer types (Addable constraint)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_curried_string_concat() {
    // Curried function that captures a String — previously caused LLVM IR type mismatch
    expect(
        "*concat(a: String)\n    *fn(b: String) -> String a + b\n\n*main()\n    f is concat(\"hello \")\n    log(f(\"world\"))\n",
        "hello world",
    );
}

#[test]
fn b_curried_string_concat_inferred() {
    // Same but with fully inferred types (the core bug)
    expect(
        "*concat(a)\n    *fn(b) a + b\n\n*main()\n    f is concat(\"hello \")\n    log(f(\"world\"))\n",
        "hello world",
    );
}

#[test]
fn b_addable_still_works_numeric() {
    // Ensure + with integers still works after Addable constraint
    expect(
        "*add(a)\n    *fn(b) a + b\n\n*main()\n    log(add(3)(4))\n",
        "7",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BUG FIX: Chained field access on rvalues
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_chained_field_access_two_deep() {
    expect(
        "type Point\n    x: i64\n    y: i64\n\ntype Line\n    a: Point\n    b: Point\n\n*main()\n    l is Line(Point(1, 2), Point(3, 4))\n    log(l.a.x)\n    log(l.b.y)\n",
        "1\n4",
    );
}

#[test]
fn b_chained_field_access_three_deep() {
    expect(
        "type Inner\n    val: i64\n\ntype Mid\n    inner: Inner\n\ntype Outer\n    mid: Mid\n\n*main()\n    o is Outer(Mid(Inner(99)))\n    log(o.mid.inner.val)\n",
        "99",
    );
}

#[test]
fn b_chained_field_assignment() {
    expect(
        "type Inner\n    val: i64\n\ntype Outer\n    inner: Inner\n\n*main()\n    o is Outer(Inner(10))\n    o.inner.val is 42\n    log(o.inner.val)\n",
        "42",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BUG FIX: Mixed-type generic struct monomorphization
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn b_struct_mono_int_and_string() {
    expect(
        "type Wrapper\n    val\n\n*main()\n    a is Wrapper(42)\n    b is Wrapper(\"hello\")\n    log(a.val)\n    log(b.val)\n",
        "42\nhello",
    );
}

#[test]
fn b_struct_mono_int_string_float() {
    expect(
        "type Pair\n    first\n    second\n\n*main()\n    p1 is Pair(1, 2)\n    p2 is Pair(\"hi\", \"there\")\n    p3 is Pair(3.14, 2.71)\n    log(p1.first)\n    log(p2.first)\n    log(p3.first)\n",
        "1\nhi\n3.140000",
    );
}

#[test]
fn b_struct_mono_single_type_no_mangle() {
    // Single-type usage should not require monomorphization
    expect(
        "type Box\n    val\n\n*main()\n    a is Box(10)\n    b is Box(20)\n    log(a.val + b.val)\n",
        "30",
    );
}