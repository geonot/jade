use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile(src: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    let out = dir.path().join("test_bin");
    std::fs::write(&jinn, src).unwrap();
    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("jinnc failed to start");
    assert!(
        output.status.success(),
        "jinnc compilation failed for:\n{src}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (dir, out)
}

fn expect(src: &str, expected: &str) {
    let (dir, out) = compile(src);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        "binary exited with {:?}\nstderr: {}\nsource:\n{src}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        expected.trim(),
        "source:\n{src}"
    );
}

fn aborted(status: &std::process::ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    status.code() == Some(134) || status.signal() == Some(6)
}

fn expect_trap(src: &str, needle: &str) {
    let (dir, out) = compile(src);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        aborted(&output.status),
        "expected abort (exit 134 / SIGABRT); status: {:?}\nstdout: {}\nstderr: {stderr}\nsource:\n{src}",
        output.status,
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        stderr.contains(needle),
        "expected trap diagnostic containing `{needle}`; stderr:\n{stderr}\nsource:\n{src}"
    );
}

fn expect_abort(src: &str) {
    let (dir, out) = compile(src);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        aborted(&output.status),
        "expected abort (exit 134 / SIGABRT); status: {:?}\nstdout: {}\nstderr: {}\nsource:\n{src}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn expect_compile_fail(src: &str, needles: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    std::fs::write(&jinn, src).unwrap();
    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(dir.path().join("test_bin"))
        .output()
        .expect("jinnc failed to start");
    assert!(
        !output.status.success(),
        "expected compilation failure for:\n{src}"
    );
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for needle in needles {
        assert!(
            all.contains(needle),
            "expected diagnostic containing `{needle}`; got:\n{all}\nsource:\n{src}"
        );
    }
}

#[test]
fn signed_remainder_by_zero_traps() {
    expect_trap(
        "*main\n    a is 7\n    b is 0\n    log(a % b)\n",
        "integer remainder by zero",
    );
}

#[test]
fn unsigned_remainder_by_zero_traps() {
    expect_trap(
        "*main\n    a is 7 as u64\n    b is 0 as u64\n    log(a % b)\n",
        "integer remainder by zero",
    );
}

#[test]
fn int_min_rem_neg_one_traps() {
    expect_trap(
        "*main\n    a is -9223372036854775808\n    b is -1\n    log(a % b)\n",
        "signed integer overflow in remainder",
    );
}

#[test]
fn div_by_zero_traps_inside_callee() {
    expect_trap(
        "*ratio(a as i64, b as i64) returns i64\n    a / b\n\n*main\n    log(ratio(10, 0))\n",
        "integer division by zero",
    );
}

#[test]
fn literal_division_by_zero_rejected_at_compile_time() {
    expect_compile_fail("*main\n    log(10 / 0)\n", &["division by zero"]);
}

#[test]
fn vec_read_past_end_traps() {
    expect_trap(
        "*main\n    v is [1, 2]\n    log(v[2])\n",
        "vec index out of bounds",
    );
}

#[test]
fn vec_read_empty_traps() {
    expect_trap(
        "*main\n    v is [1]\n    v.pop()\n    log(v[0])\n",
        "vec index out of bounds",
    );
}

#[test]
fn vec_write_oob_traps() {
    expect_trap(
        "*main\n    v is [1, 2]\n    v.set(9, 5)\n    log(v[0])\n",
        "vec index out of bounds",
    );
}

#[test]
fn vec_remove_oob_traps() {
    expect_trap(
        "*main\n    v is [1, 2]\n    v.remove(9)\n    log(v.len())\n",
        "vec index out of bounds",
    );
}

#[test]
fn nested_vec_inner_oob_traps() {
    expect_trap(
        "*main\n    v is [[1], [2]]\n    log(v[1][7])\n",
        "vec index out of bounds",
    );
}

#[test]
fn vec_oob_via_runtime_index_traps() {
    expect_trap(
        "*main\n    v is [1, 2, 3]\n    n is v.len()\n    log(v[n])\n",
        "vec index out of bounds",
    );
}

#[test]
fn negative_index_wraps_from_end() {
    expect(
        "*main\n    v is [10, 20, 30]\n    i is -1\n    j is -3\n    log(v[i])\n    log(v[j])\n",
        "30\n10",
    );
}

#[test]
fn negative_index_past_front_traps() {
    expect_trap(
        "*main\n    v is [10, 20, 30]\n    i is -4\n    log(v[i])\n",
        "vec index out of bounds",
    );
}

#[test]
fn strict_narrowing_overflow_traps() {
    expect_trap(
        "*main\n    big is 70000\n    z is big as strict i16\n    log(z)\n",
        "strict cast lost information",
    );
}

#[test]
fn strict_negative_to_unsigned_traps() {
    expect_trap(
        "*main\n    n is -1\n    u is n as strict u8\n    log(u)\n",
        "strict cast lost information",
    );
}

#[test]
fn strict_cast_in_range_passes() {
    expect(
        "*main\n    n is 100\n    log(n as strict i8)\n    m is 255\n    log(m as strict u8)\n",
        "100\n255",
    );
}

#[test]
fn unwrap_on_nothing_aborts() {
    expect_abort("*find() returns Option of i64\n    Nothing\n\n*main\n    log(find().unwrap())\n");
}

#[test]
fn unwrap_on_some_passes() {
    expect(
        "*find() returns Option of i64\n    Some(11)\n\n*main\n    log(find().unwrap())\n",
        "11",
    );
}

#[test]
fn failed_assert_aborts() {
    expect_abort("*main\n    x is 1\n    assert(x equals 2)\n    log(x)\n");
}

#[test]
fn passing_assert_continues() {
    expect(
        "*main\n    x is 1\n    assert(x equals 1)\n    log(x)\n",
        "1",
    );
}

#[test]
fn use_after_take_is_compile_error() {
    expect_compile_fail(
        "*eat(v as take Vec of i64)\n    log(v.len())\n\n*main\n    xs is [1,2]\n    eat(take xs)\n    log(xs.len())\n",
        &["moved value `xs`", "`eat`"],
    );
}

#[test]
fn resource_use_after_take_is_compile_error() {
    expect_compile_fail(
        "type Handle @resource\n    fd as i64\n\n*shut(h as take Handle)\n    log(h.fd)\n\n*main\n    h is Handle(fd is 3)\n    shut(h)\n    log(h.fd)\n",
        &["moved value `h`", "`shut`"],
    );
}

#[test]
fn non_exhaustive_match_is_compile_error() {
    expect_compile_fail(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main\n    c is Blue\n    match c\n        Red ? log(1)\n        Green ? log(2)\n",
        &["non-exhaustive", "Blue"],
    );
}

#[test]
fn rebinding_overwrites_value() {
    expect("*main\n    x is 1\n    x is 2\n    log(x)\n", "2");
}

#[test]
fn rebinding_after_take_clears_tombstone() {
    expect(
        "*eat(v as take Vec of i64)\n    log(v.len())\n\n*main\n    xs is [1, 2]\n    eat(take xs)\n    xs is [9, 9, 9]\n    log(xs.len())\n",
        "2\n3",
    );
}

#[test]
fn aliased_heap_write_is_visible_through_both_names() {
    expect(
        "type P\n    x as i64\n\n*main\n    p is P(x is 1)\n    q is p\n    q.x is 99\n    log(p.x)\n",
        "99",
    );
}

#[test]
fn copied_heap_write_does_not_leak_back() {
    expect(
        "type P\n    x as i64\n\n*main\n    p is P(x is 1)\n    q is copy p\n    q.x is 99\n    log(p.x)\n    log(q.x)\n",
        "1\n99",
    );
}

#[test]
fn vec_element_overwrite_in_place() {
    expect(
        "*main\n    v is [1, 2, 3]\n    v.set(1, 20)\n    log(v[1])\n    log(v.len())\n",
        "20\n3",
    );
}

#[test]
fn float_division_by_zero_is_ieee_inf() {
    expect("*main\n    a is 1.0\n    b is 0.0\n    log(a / b)\n", "inf");
}

#[test]
fn float_zero_over_zero_is_nan() {
    expect("*main\n    a is 0.0\n    b is 0.0\n    log(a / b)\n", "nan");
}

#[test]
fn unsigned_subtraction_wraps() {
    expect(
        "*main\n    a is 0 as u8\n    b is 1 as u8\n    log(a - b)\n",
        "255",
    );
}

#[test]
fn signed_addition_wraps_twos_complement() {
    expect(
        "*main\n    a is 9223372036854775807\n    b is 1\n    log(a + b)\n",
        "-9223372036854775808",
    );
}

#[test]
fn take_inside_loop_is_compile_error() {
    expect_compile_fail(
        "*eat(v as take Vec of i64)\n    log(v.len())\n\n*main\n    xs is [1]\n    for i from 0 to 2\n        eat(take xs)\n",
        &["take"],
    );
}

#[test]
fn bound_result_match_does_not_ice() {
    expect(
        "err E\n    Oops\n\n*f(ok as bool) returns Result of i64, E\n    if ok\n        Ok(9)\n    else\n        Err(Oops)\n\n*main\n    r is f(false)\n    match r\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n",
        "-1",
    );
}

#[test]
fn string_slice_oob_is_checked() {
    expect_trap(
        "*main\n    s is 'hello'\n    log(s.slice(2, 99))\n",
        "out of bounds",
    );
}

#[test]
fn string_char_at_oob_is_checked() {
    expect_trap(
        "*main\n    s is 'hi'\n    log(s.char_at(99))\n",
        "out of bounds",
    );
}

#[test]
fn oversized_shift_is_defined() {
    expect_trap("*main\n    a is 1\n    b is 70\n    log(a << b)\n", "shift");
}
