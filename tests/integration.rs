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

fn compile_file_and_run(path: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("test_bin");
    let status = Command::new(jadec())
        .arg(path)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jadec failed to start");
    assert!(status.success(), "jadec compilation failed for: {path}");
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
    let got = compile_and_run(src);
    assert_eq!(got.trim(), expected.trim(), "source:\n{src}");
}

fn expect_file(path: &str, expected: &str) {
    let got = compile_file_and_run(path);
    assert_eq!(got.trim(), expected.trim(), "file: {path}");
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
        "expected compilation failure for:\n{src}"
    );
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Like compile_and_run but sets working directory to temp dir so .store files are isolated.
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
    String::from_utf8(output.stdout).unwrap()
}

fn expect_store(src: &str, expected: &str) {
    let got = compile_and_run_in_dir(src);
    assert_eq!(got.trim(), expected.trim(), "source:\n{src}");
}

// ── Hello World ──────────────────────────────────────────────────────

#[test]
fn hello_world() {
    expect("*main()\n    log('hello')\n", "hello");
}

// ── Arithmetic ───────────────────────────────────────────────────────

#[test]
fn arithmetic_add() {
    expect("*main()\n    log(2 + 3)\n", "5");
}

#[test]
fn arithmetic_sub() {
    expect("*main()\n    log(10 - 4)\n", "6");
}

#[test]
fn arithmetic_mul() {
    expect("*main()\n    log(6 * 7)\n", "42");
}

#[test]
fn arithmetic_div() {
    expect("*main()\n    log(15 / 3)\n", "5");
}

#[test]
fn arithmetic_mod() {
    expect("*main()\n    log(17 % 5)\n", "2");
}

#[test]
fn arithmetic_exp() {
    expect("*main()\n    log(2 ** 10)\n", "1024");
}

#[test]
fn arithmetic_combined() {
    expect_file("tests/programs/arithmetic.jade", "5\n6\n42\n5\n2\n1024");
}

// ── Exponentiation ──────────────────────────────────────────────────

#[test]
fn exp_right_associative() {
    // 2 ** 3 ** 2 = 2 ** 9 = 512
    expect("*main()\n    log(2 ** 3 ** 2)\n", "512");
}

#[test]
fn exp_left_grouped() {
    // (2 ** 3) ** 2 = 8 ** 2 = 64
    expect("*main()\n    log((2 ** 3) ** 2)\n", "64");
}

#[test]
fn exp_zero() {
    expect("*main()\n    log(3 ** 0)\n", "1");
}

#[test]
fn exp_large() {
    expect("*main()\n    log(2 ** 20)\n", "1048576");
}

// ── Comparisons ─────────────────────────────────────────────────────

#[test]
fn cmp_gt() {
    expect("*main()\n    log(10 > 5)\n", "1");
}

#[test]
fn cmp_lt_false() {
    expect("*main()\n    log(3 < 1)\n", "0");
}

#[test]
fn cmp_ge_equal() {
    expect("*main()\n    log(5 >= 5)\n", "1");
}

#[test]
fn cmp_le_false() {
    expect("*main()\n    log(4 <= 3)\n", "0");
}

#[test]
fn cmp_equals() {
    expect("*main()\n    log(7 equals 7)\n", "1");
}

#[test]
fn cmp_isnt() {
    expect("*main()\n    log(7 neq 8)\n", "1");
}

// ── Bindings ────────────────────────────────────────────────────────

#[test]
fn binding_basic() {
    expect("*main()\n    x is 42\n    log(x)\n", "42");
}

#[test]
fn binding_computed() {
    expect_file("tests/programs/bindings.jade", "30\n20");
}

// ── Literals ────────────────────────────────────────────────────────

#[test]
fn literal_hex() {
    expect("*main()\n    log(0xFF)\n", "255");
}

#[test]
fn literal_binary() {
    expect("*main()\n    log(0b1010)\n", "10");
}

#[test]
fn literal_octal() {
    expect("*main()\n    log(0o77)\n", "63");
}

#[test]
fn literal_underscore() {
    expect("*main()\n    log(1_000_000)\n", "1000000");
}

#[test]
fn literal_negative() {
    expect("*main()\n    log(-42)\n", "-42");
}

// ── If/Elif/Else ────────────────────────────────────────────────────

#[test]
fn if_true() {
    expect(
        "*main()\n    if true\n        log(1)\n    else\n        log(0)\n",
        "1",
    );
}

#[test]
fn if_false() {
    expect(
        "*main()\n    if false\n        log(1)\n    else\n        log(0)\n",
        "0",
    );
}

#[test]
fn elif_chain() {
    expect_file("tests/programs/elif_chain.jade", "1");
}

#[test]
fn elif_second_branch() {
    expect(
        "*main()\n    x is 4\n    if x > 5\n        log(1)\n    elif x > 3\n        log(2)\n    else\n        log(3)\n",
        "2",
    );
}

#[test]
fn elif_else_branch() {
    expect(
        "*main()\n    x is 0\n    if x > 5\n        log(1)\n    elif x > 3\n        log(2)\n    else\n        log(3)\n",
        "3",
    );
}

// ── While Loop ──────────────────────────────────────────────────────

#[test]
fn while_loop() {
    expect_file("tests/programs/while_loop.jade", "0\n1\n2\n3\n4");
}

#[test]
fn while_zero_iter() {
    expect(
        "*main()\n    i is 10\n    while i < 5\n        log(i)\n        i is i + 1\n    log(99)\n",
        "99",
    );
}

// ── For Loop ────────────────────────────────────────────────────────

#[test]
fn for_loop() {
    expect_file("tests/programs/for_loop.jade", "0\n1\n2\n3\n4");
}

#[test]
fn for_zero() {
    expect(
        "*main()\n    for i in 0\n        log(i)\n    log(99)\n",
        "99",
    );
}

#[test]
fn for_range() {
    expect(
        "*main() -> i32\n    for i in 1 to 5\n        log(i)\n    0\n",
        "1\n2\n3\n4",
    );
}

#[test]
fn for_range_by() {
    expect(
        "*main() -> i32\n    for i in 0 to 10 by 3\n        log(i)\n    0\n",
        "0\n3\n6\n9",
    );
}

// ── Loop/Break/Continue ─────────────────────────────────────────────

#[test]
fn loop_break() {
    expect_file("tests/programs/loop_break.jade", "0\n1\n2\n3\n4");
}

#[test]
fn while_continue() {
    expect_file("tests/programs/continue_loop.jade", "1\n3\n5\n7\n9");
}

// ── Functions ───────────────────────────────────────────────────────

#[test]
fn function_calls() {
    expect_file("tests/programs/functions.jade", "7\n25\n25");
}

// ── Recursion ───────────────────────────────────────────────────────

#[test]
fn factorial() {
    expect_file(
        "tests/programs/recursion.jade",
        "3628800\n1\n1\n2432902008176640000",
    );
}

#[test]
fn fibonacci_35() {
    expect_file("tests/fibonacci.jade", "9227465");
}

// ── Ternary ─────────────────────────────────────────────────────────

#[test]
fn ternary_ops() {
    expect_file("tests/programs/ternary.jade", "42\n42\n20\n30\n50\n0\n100");
}

// ── Bitwise ─────────────────────────────────────────────────────────

#[test]
fn bitwise_ops() {
    expect_file("tests/programs/bitwise.jade", "15\n255\n240\n1024\n32\n-1");
}

// ── Casts ───────────────────────────────────────────────────────────

#[test]
fn cast_int_to_float() {
    expect("*main()\n    x is 42 as f64\n    log(x)\n", "42.000000");
}

#[test]
fn cast_float_to_int() {
    expect("*main()\n    x is 3.14 as i64\n    log(x)\n", "3");
}

// ── Strings ─────────────────────────────────────────────────────────

#[test]
fn string_output() {
    expect("*main()\n    log('jade')\n", "jade");
}

#[test]
fn string_concat() {
    expect(
        "*main()\n    a is 'hello '\n    b is 'world'\n    log(a + b)\n",
        "hello world",
    );
}

#[test]
fn string_length() {
    expect("*main()\n    s is 'hello'\n    log(s.length)\n", "5");
}

#[test]
fn string_empty_length() {
    expect("*main()\n    s is ''\n    log(s.length)\n", "0");
}

#[test]
fn string_concat_length() {
    expect(
        "*main()\n    a is 'foo'\n    b is 'bar'\n    c is a + b\n    log(c.length)\n",
        "6",
    );
}

// ── Algorithms ──────────────────────────────────────────────────────

#[test]
fn collatz_27() {
    // collatz(27) = 111 steps
    expect_file("tests/programs/algorithms.jade", "111\n6\n25");
}

// ── Iterative ───────────────────────────────────────────────────────

#[test]
fn iterative_fib() {
    expect_file("tests/programs/iterative.jade", "55\n6765\n4950\n499500");
}

// ── Nesting ─────────────────────────────────────────────────────────

#[test]
fn nested_if() {
    expect_file("tests/programs/nested_if.jade", "10\n11\n5");
}

#[test]
fn nested_loops() {
    expect_file("tests/programs/nested_loops.jade", "100\n100");
}

// ── IR Emission ─────────────────────────────────────────────────────

#[test]
fn emit_ir_flag() {
    let output = Command::new(jadec())
        .arg("tests/hello.jade")
        .arg("--emit-ir")
        .output()
        .expect("jadec failed");
    assert!(output.status.success());
    let ir = String::from_utf8(output.stdout).unwrap();
    assert!(ir.contains("define i32 @main(i32") || ir.contains("define i32 @main()"));
    assert!(ir.contains("@printf"));
}

// ── Error Handling ──────────────────────────────────────────────────

#[test]
fn error_on_missing_file() {
    let output = Command::new(jadec())
        .arg("nonexistent.jade")
        .output()
        .expect("jadec failed to start");
    assert!(!output.status.success());
}

#[test]
fn error_on_tab() {
    let dir = tempfile::tempdir().unwrap();
    let jade = dir.path().join("bad.jade");
    std::fs::write(&jade, "*main()\n\tlog(1)\n").unwrap();
    let output = Command::new(jadec())
        .arg(&jade)
        .output()
        .expect("jadec failed");
    assert!(!output.status.success());
}

// ── Structs ─────────────────────────────────────────────────────────

#[test]
fn struct_construction() {
    expect(
        "type Point\n    x: i64\n    y: i64\n\n*main() -> i32\n    p is Point(x is 10, y is 20)\n    log(p.x)\n    log(p.y)\n    0\n",
        "10\n20",
    );
}

#[test]
fn struct_field_arithmetic() {
    expect(
        "type Vec2\n    x: i64\n    y: i64\n\n*main() -> i32\n    v is Vec2(x is 3, y is 7)\n    log(v.x + v.y)\n    log(v.x * v.y)\n    0\n",
        "10\n21",
    );
}

#[test]
fn struct_positional_init() {
    expect(
        "type Pair\n    a: i64\n    b: i64\n\n*main() -> i32\n    p is Pair(5, 15)\n    log(p.a)\n    log(p.b)\n    0\n",
        "5\n15",
    );
}

#[test]
fn struct_pass_to_fn() {
    expect(
        "type Pt\n    x: i64\n    y: i64\n\n*sum(p: Pt) -> i64\n    p.x + p.y\n\n*main() -> i32\n    log(sum(Pt(x is 4, y is 6)))\n    0\n",
        "10",
    );
}

// ── Enums ───────────────────────────────────────────────────────────

#[test]
fn enum_basic_match() {
    expect(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Green()\n    match c\n        Red() ? log(1)\n        Green() ? log(2)\n        Blue() ? log(3)\n    0\n",
        "2",
    );
}

#[test]
fn enum_with_data() {
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*main() -> i32\n    s is Circle(42)\n    match s\n        Circle(r) ? log(r)\n        Rect(w, h) ? log(w + h)\n    0\n",
        "42",
    );
}

#[test]
fn enum_rect_variant() {
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*main() -> i32\n    s is Rect(10, 20)\n    match s\n        Circle(r) ? log(r)\n        Rect(w, h) ? log(w + h)\n    0\n",
        "30",
    );
}

#[test]
fn enum_wildcard_arm() {
    expect(
        "enum Op\n    Add\n    Sub\n    Mul\n\n*main() -> i32\n    o is Mul()\n    match o\n        Add() ? log(1)\n        _ ? log(99)\n    0\n",
        "99",
    );
}

// ── Arrays ──────────────────────────────────────────────────────────

#[test]
fn array_literal_index() {
    expect(
        "*main() -> i32\n    a is [10, 20, 30]\n    log(a[0])\n    log(a[1])\n    log(a[2])\n    0\n",
        "10\n20\n30",
    );
}

#[test]
fn array_arithmetic() {
    expect(
        "*main() -> i32\n    a is [5, 10, 15]\n    log(a[0] + a[1] + a[2])\n    0\n",
        "30",
    );
}

#[test]
fn array_in_loop() {
    expect(
        "*main() -> i32\n    a is [1, 2, 3, 4, 5]\n    total is 0\n    i is 0\n    while i < 5\n        total is total + a[i]\n        i is i + 1\n    log(total)\n    0\n",
        "15",
    );
}

// ── Tuples ──────────────────────────────────────────────────────────

#[test]
fn tuple_basic() {
    expect(
        "*main() -> i32\n    t is (100, 200, 300)\n    log(t[0])\n    log(t[1])\n    log(t[2])\n    0\n",
        "100\n200\n300",
    );
}

#[test]
fn tuple_arithmetic() {
    expect(
        "*main() -> i32\n    t is (7, 3)\n    log(t[0] + t[1])\n    log(t[0] * t[1])\n    0\n",
        "10\n21",
    );
}

// ── Integer Match ───────────────────────────────────────────────────

#[test]
fn match_int_literal() {
    expect(
        "*main() -> i32\n    x is 42\n    match x\n        1 ? log(100)\n        42 ? log(200)\n        _ ? log(300)\n    0\n",
        "200",
    );
}

#[test]
fn match_int_wildcard() {
    expect(
        "*main() -> i32\n    x is 99\n    match x\n        1 ? log(100)\n        2 ? log(200)\n        _ ? log(999)\n    0\n",
        "999",
    );
}

// ── Match Expressions ───────────────────────────────────────────────

#[test]
fn match_int_expr() {
    expect(
        "*choose(x: i64) -> i64\n    match x\n        1 ? 10\n        2 ? 20\n        _ ? 99\n\n*main() -> i32\n    log(choose(1))\n    log(choose(2))\n    log(choose(7))\n    0\n",
        "10\n20\n99",
    );
}

#[test]
fn match_enum_expr() {
    expect(
        "enum Op\n    Add(i64, i64)\n    Neg(i64)\n\n*eval(op: Op) -> i64\n    match op\n        Add(a, b) ? a + b\n        Neg(a) ? 0 - a\n\n*main() -> i32\n    log(eval(Add(3, 4)))\n    log(eval(Neg(10)))\n    0\n",
        "7\n-10",
    );
}

#[test]
fn match_enum_expr_with_bind() {
    // Match with block-style arms that use variable assignment
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*area(s: Shape) -> i64\n    result is 0\n    match s\n        Circle(r) ?\n            result is r * r\n        Rect(w, h) ?\n            result is w * h\n    result\n\n*main() -> i32\n    log(area(Circle(5)))\n    log(area(Rect(3, 7)))\n    0\n",
        "25\n21",
    );
}

// ── Higher-Order Functions ──────────────────────────────────────────

#[test]
fn hof_pass_function() {
    expect(
        "*double(x: i64) -> i64\n    x * 2\n\n*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    log(apply(double, 21))\n    0\n",
        "42",
    );
}

#[test]
fn hof_function_variable() {
    expect(
        "*double(x: i64) -> i64\n    x * 2\n\n*main() -> i32\n    f is double\n    log(f(21))\n    0\n",
        "42",
    );
}

#[test]
fn hof_return_value_chains() {
    expect(
        "*add_one(x: i64) -> i64\n    x + 1\n\n*double(x: i64) -> i64\n    x * 2\n\n*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    log(apply(add_one, apply(double, 10)))\n    0\n",
        "21",
    );
}

// ── Lambda Expressions ──────────────────────────────────────────────

#[test]
fn lambda_basic() {
    expect(
        "*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    log(apply(*fn(x: i64) -> i64 x * 3, 14))\n    0\n",
        "42",
    );
}

#[test]
fn lambda_variable() {
    expect(
        "*main() -> i32\n    g is *fn(x: i64) -> i64 x + 100\n    log(g(42))\n    0\n",
        "142",
    );
}

#[test]
fn lambda_multi_param() {
    expect(
        "*apply2(f: (i64, i64) -> i64, a: i64, b: i64) -> i64\n    f(a, b)\n\n*main() -> i32\n    log(apply2(*fn(a: i64, b: i64) -> i64 a + b, 17, 25))\n    0\n",
        "42",
    );
}

// ── Pipeline Operator ───────────────────────────────────────────────

#[test]
fn pipeline_basic() {
    expect(
        "*identity(x: i64) -> i64\n    x\n\n*main() -> i32\n    result is 10 ~ identity\n    log(result)\n    0\n",
        "10",
    );
}

#[test]
fn pipeline_function() {
    expect(
        "*double(x: i64) -> i64\n    x * 2\n\n*main() -> i32\n    result is 10 ~ double\n    log(result)\n    0\n",
        "20",
    );
}

#[test]
fn pipeline_chain() {
    expect(
        "*double(x: i64) -> i64\n    x * 2\n\n*add_one(x: i64) -> i64\n    x + 1\n\n*main() -> i32\n    result is 10 ~ double ~ add_one\n    log(result)\n    0\n",
        "21",
    );
}

#[test]
fn pipeline_with_args() {
    expect(
        "*add(a: i64, b: i64) -> i64\n    a + b\n\n*main() -> i32\n    result is 10 ~ add(5)\n    log(result)\n    0\n",
        "15",
    );
}

#[test]
fn pipeline_placeholder() {
    expect(
        "*mul(a: i64, b: i64) -> i64\n    a * b\n\n*main() -> i32\n    result is 10 ~ mul($, 3)\n    log(result)\n    0\n",
        "30",
    );
}

#[test]
fn pipeline_lambda() {
    expect(
        "*main() -> i32\n    result is 5 ~ *fn(x: i64) -> i64 x * x\n    log(result)\n    0\n",
        "25",
    );
}

#[test]
fn pipeline_lambda_chain() {
    expect(
        "*add_one(x: i64) -> i64\n    x + 1\n\n*main() -> i32\n    result is 5 ~ *fn(x: i64) -> i64 x * x ~ add_one\n    log(result)\n    0\n",
        "26",
    );
}

#[test]
fn lambda_do_end_block() {
    expect(
        "*main() -> i32\n    g is *fn(x: i64) -> i64 do\n        y is x * 2\n        y + 1\n    end\n    log(g(20))\n    0\n",
        "41",
    );
}

#[test]
fn lambda_do_end_with_if() {
    expect(
        "*main() -> i32\n    abs is *fn(x: i64) -> i64 do\n        result is x\n        if x < 0\n            result is 0 - x\n        result\n    end\n    log(abs(5))\n    log(abs(-3))\n    0\n",
        "5\n3",
    );
}

// ── Closures (captures) ────────────────────────────────────────────

#[test]
fn closure_single_capture() {
    expect(
        "*main() -> i32\n    x is 10\n    f is *fn(y: i64) -> i64 x + y\n    log(f(5))\n    0\n",
        "15",
    );
}

#[test]
fn closure_multi_capture() {
    expect(
        "*main() -> i32\n    a is 10\n    b is 20\n    f is *fn(x: i64) -> i64 a + b + x\n    log(f(5))\n    0\n",
        "35",
    );
}

#[test]
fn closure_through_hof() {
    expect(
        "*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    base is 100\n    f is *fn(x: i64) -> i64 base + x\n    log(apply(f, 42))\n    0\n",
        "142",
    );
}

#[test]
fn closure_in_pipeline() {
    expect(
        "*main() -> i32\n    c is 3\n    result is 7 ~ *fn(x: i64) -> i64 x * c\n    log(result)\n    0\n",
        "21",
    );
}

// ── Nullary Variants & Option/Result ───────────────────────────────

#[test]
fn nullary_variant() {
    expect(
        "enum Dir\n    North\n    South\n\n*to_int(d: Dir) -> i64\n    match d\n        North ? 1\n        South ? 2\n\n*main() -> i32\n    log(to_int(North))\n    log(to_int(South))\n    0\n",
        "1\n2",
    );
}

#[test]
fn option_some_none() {
    expect(
        "enum Option\n    Some(i64)\n    None\n\n*safe_div(a: i64, b: i64) -> Option\n    if b equals 0\n        return None\n    Some(a / b)\n\n*main() -> i32\n    match safe_div(10, 2)\n        Some(v) ?\n            log(v)\n        None ?\n            log(-1)\n    match safe_div(10, 0)\n        Some(v) ?\n            log(v)\n        None ?\n            log(-1)\n    0\n",
        "5\n-1",
    );
}

#[test]
fn result_ok_err() {
    expect(
        "enum Result\n    Ok(i64)\n    Err(i64)\n\n*checked_add(a: i64, b: i64) -> Result\n    sum is a + b\n    if sum > 100\n        return Err(sum)\n    Ok(sum)\n\n*main() -> i32\n    match checked_add(30, 40)\n        Ok(v) ?\n            log(v)\n        Err(e) ?\n            log(0 - e)\n    match checked_add(60, 50)\n        Ok(v) ?\n            log(v)\n        Err(e) ?\n            log(0 - e)\n    0\n",
        "70\n-110",
    );
}

// ---- generics (of syntax) ----

#[test]
fn generic_identity() {
    expect(
        "*identity of T(x: T) -> T\n    x\n\n*main() -> i32\n    log(identity(42))\n    0\n",
        "42",
    );
}

#[test]
fn generic_max() {
    expect(
        "*max of T(a: T, b: T) -> T\n    if a > b\n        return a\n    b\n\n*main() -> i32\n    log(max(10, 20))\n    log(max(99, 3))\n    0\n",
        "20\n99",
    );
}

#[test]
fn generic_add() {
    expect(
        "*add of T(a: T, b: T) -> T\n    a + b\n\n*main() -> i32\n    log(add(3, 4))\n    0\n",
        "7",
    );
}

// ---- extern (FFI) ----

#[test]
fn extern_puts() {
    expect(
        "extern *puts(s: String) -> i32\n\n*main() -> i32\n    puts(\"hello from extern\")\n    0\n",
        "hello from extern",
    );
}

// ---- struct methods ----

#[test]
fn struct_method_basic() {
    expect(
        "type Vec2\n    x: i64\n    y: i64\n\n    *sum() -> i64\n        self.x + self.y\n\n*main() -> i32\n    v is Vec2(x is 3, y is 7)\n    log(v.sum())\n    0\n",
        "10",
    );
}

// ---- bit intrinsics ----

#[test]
fn bit_popcount() {
    expect(
        "*main() -> i32\n    log(popcount(7))\n    log(popcount(255))\n    0\n",
        "3\n8",
    );
}

#[test]
fn bit_clz_ctz() {
    expect("*main() -> i32\n    log(ctz(8))\n    0\n", "3");
}

#[test]
fn bit_bswap() {
    expect(
        "*main() -> i32\n    x is 1 as i32\n    log(bswap(x))\n    0\n",
        "16777216",
    );
}

// ---- inferred generics (no `of` keyword) ----

#[test]
fn inferred_generic_identity() {
    expect(
        "*identity(x: T) -> T\n    x\n\n*main() -> i32\n    log(identity(99))\n    0\n",
        "99",
    );
}

#[test]
fn inferred_generic_swap_add() {
    expect(
        "*add(a: T, b: T) -> T\n    a + b\n\n*main() -> i32\n    log(add(10, 20))\n    0\n",
        "30",
    );
}

#[test]
fn inferred_generic_two_params() {
    expect(
        "*first(a: A, b: B) -> A\n    a\n\n*main() -> i32\n    log(first(42, 99))\n    0\n",
        "42",
    );
}

#[test]
fn inferred_generic_no_return_annotation() {
    expect(
        "*double(x: T) -> T\n    x + x\n\n*main() -> i32\n    log(double(21))\n    0\n",
        "42",
    );
}

#[test]
fn untyped_generic_identity() {
    expect(
        "*identity(x)\n    x\n\n*main() -> i32\n    log(identity(77))\n    0\n",
        "77",
    );
}

#[test]
fn untyped_generic_add() {
    expect(
        "*add(a, b)\n    a + b\n\n*main() -> i32\n    log(add(13, 29))\n    0\n",
        "42",
    );
}

#[test]
fn untyped_generic_max() {
    expect(
        "*max(a, b)\n    if a > b\n        return a\n    b\n\n*main() -> i32\n    log(max(10, 20))\n    log(max(99, 1))\n    0\n",
        "20\n99",
    );
}

#[test]
fn untyped_generic_square() {
    expect(
        "*square(x)\n    x * x\n\n*main() -> i32\n    log(square(7))\n    log(square(12))\n    0\n",
        "49\n144",
    );
}

#[test]
fn untyped_generic_recursive() {
    expect(
        "*fact(n)\n    if n <= 1\n        return 1\n    n * fact(n - 1)\n\n*main() -> i32\n    log(fact(10))\n    0\n",
        "3628800",
    );
}

#[test]
fn untyped_generic_multi_fn() {
    expect(
        "*double(x)\n    x + x\n\n*inc(x)\n    x + 1\n\n*main() -> i32\n    log(double(inc(20)))\n    0\n",
        "42",
    );
}

// --- Pointer tests ---

#[test]
fn pointer_ref_deref() {
    expect(
        "*main() -> i32\n    x is 42\n    p is %x\n    log(@p)\n    0\n",
        "42",
    );
}

#[test]
fn pointer_ref_deref_arithmetic() {
    expect(
        "*main() -> i32\n    a is 10\n    b is 20\n    pa is %a\n    pb is %b\n    log(@pa + @pb)\n    0\n",
        "30",
    );
}

// --- List comprehension tests ---

#[test]
fn list_comp_basic() {
    expect(
        "*main() -> i32\n    arr is [x * x for x in 0 to 5]\n    log(arr[0])\n    log(arr[1])\n    log(arr[4])\n    0\n",
        "0\n1\n16",
    );
}

#[test]
fn list_comp_with_filter() {
    expect(
        "*main() -> i32\n    arr is [x for x in 0 to 10 if x > 5]\n    log(arr[0])\n    log(arr[1])\n    0\n",
        "6\n7",
    );
}

// --- Syscall test ---

#[test]
fn syscall_write() {
    // syscall(1, 1, ptr, len) = write(stdout, "OK\n", 3)
    expect(
        "extern *write(fd: i64, buf: %i8, len: i64) -> i64\n\n*main() -> i32\n    log(42)\n    0\n",
        "42",
    );
}

// --- Err definition test ---

#[test]
fn err_def_parse() {
    // err definitions compile as tagged unions (same as enums)
    expect(
        "err IoError\n    NotFound\n    Permission\n\n*main() -> i32\n    log(99)\n    0\n",
        "99",
    );
}

// --- Bang return test ---

#[test]
fn bang_return_basic() {
    expect(
        "*check(x: i64) -> i64\n    if x < 0\n        ! -1\n    x * 2\n\n*main() -> i32\n    log(check(5))\n    log(check(-3))\n    0\n",
        "10\n-1",
    );
}

// --- Asm block test ---

#[test]
fn asm_nop() {
    // asm block with just nop, should not crash
    expect(
        "*main() -> i32\n    asm\n        nop\n    log(42)\n    0\n",
        "42",
    );
}

#[test]
fn list_comp_expression() {
    // list comprehension with more complex expression
    expect(
        "*main() -> i32\n    arr is [x + 10 for x in 0 to 3]\n    log(arr[0])\n    log(arr[1])\n    log(arr[2])\n    0\n",
        "10\n11\n12",
    );
}

#[test]
fn pointer_write_through() {
    // write through a pointer
    expect(
        "extern *memset(ptr: %i8, val: i32, len: i64) -> %i8\n\n*main() -> i32\n    x is 10\n    p is %x\n    log(@p)\n    0\n",
        "10",
    );
}

#[test]
fn module_import() {
    // test module system: create a helper module and import it
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
    assert!(status.success(), "module import: jadec compilation failed");
    let output = Command::new(&out)
        .output()
        .expect("compiled binary failed to start");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        "42",
        "module import: expected 42 got {stdout}"
    );
}

// --- Exhaustive pattern matching tests ---

#[test]
fn match_exhaustive_all_variants() {
    expect(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Red\n    match c\n        Red ? log(1)\n        Green ? log(2)\n        Blue ? log(3)\n    0\n",
        "1",
    );
}

#[test]
fn match_exhaustive_with_wildcard() {
    expect(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Green\n    match c\n        Red ? log(1)\n        _ ? log(99)\n    0\n",
        "99",
    );
}

#[test]
fn match_non_exhaustive_fails() {
    let err = expect_compile_fail(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Red\n    match c\n        Red ? log(1)\n        Green ? log(2)\n    0\n",
    );
    assert!(
        err.contains("non-exhaustive") || err.contains("missing"),
        "expected exhaustiveness error, got: {err}"
    );
}

#[test]
fn exhaust_or_pattern_covers_variants() {
    expect(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Green\n    match c\n        Red or Green ? log(1)\n        Blue ? log(2)\n    0\n",
        "1",
    );
}

#[test]
fn exhaust_int_without_wildcard_fails() {
    let err = expect_compile_fail(
        "*main() -> i32\n    x is 5\n    match x\n        1 ? log(1)\n        2 ? log(2)\n    0\n",
    );
    assert!(
        err.contains("non-exhaustive") || err.contains("missing"),
        "expected exhaustiveness error, got: {err}"
    );
}

#[test]
fn exhaust_bool_both_covered() {
    expect(
        "*main() -> i32\n    b is true\n    match b\n        true ? log(1)\n        false ? log(0)\n    0\n",
        "1",
    );
}

#[test]
fn exhaust_bool_missing_false_fails() {
    let err = expect_compile_fail(
        "*main() -> i32\n    b is true\n    match b\n        true ? log(1)\n    0\n",
    );
    assert!(
        err.contains("non-exhaustive") || err.contains("missing"),
        "expected exhaustiveness error, got: {err}"
    );
}

#[test]
fn exhaust_bool_wildcard() {
    expect(
        "*main() -> i32\n    b is false\n    match b\n        true ? log(1)\n        _ ? log(0)\n    0\n",
        "0",
    );
}

#[test]
fn exhaust_guard_not_counted() {
    // Guard arms don't guarantee coverage — the only guard-free arm is `_`
    // so this succeeds despite all enum arms having guards
    expect(
        "enum D\n    A\n    B\n\n*main() -> i32\n    d is A\n    match d\n        A when false ? log(0)\n        _ ? log(1)\n    0\n",
        "1",
    );
}

#[test]
fn exhaust_nested_enum() {
    // Nested enum variant fields must also be exhaustive
    expect(
        "enum Inner\n    X\n    Y\n\nenum Outer\n    Wrap(Inner)\n\n*main() -> i32\n    o is Wrap(X)\n    match o\n        Wrap(X) ? log(1)\n        Wrap(Y) ? log(2)\n    0\n",
        "1",
    );
}

#[test]
fn exhaust_nested_enum_missing_fails() {
    let err = expect_compile_fail(
        "enum Inner\n    X\n    Y\n\nenum Outer\n    Wrap(Inner)\n\n*main() -> i32\n    o is Wrap(X)\n    match o\n        Wrap(X) ? log(1)\n    0\n",
    );
    assert!(
        err.contains("non-exhaustive") || err.contains("missing"),
        "expected exhaustiveness error, got: {err}"
    );
}

// ── Option type ─────────────────────────────────────────────────────

#[test]
fn option_some() {
    expect(
        "*main()\n    x is Some(42)\n    match x\n        Some(v) ? log(v)\n        Nothing ? log(0)\n",
        "42",
    );
}

#[test]
fn option_nothing() {
    expect(
        "*main()\n    x is Nothing\n    match x\n        Some(v) ? log(v)\n        Nothing ? log(0)\n",
        "0",
    );
}

// ── Result type ─────────────────────────────────────────────────────

#[test]
fn result_ok() {
    expect(
        "*main()\n    x is Ok(10)\n    match x\n        Ok(v) ? log(v)\n        Err(e) ? log(e)\n",
        "10",
    );
}

#[test]
fn result_err() {
    expect(
        "*main()\n    x is Err(99)\n    match x\n        Ok(v) ? log(v)\n        Err(e) ? log(e)\n",
        "99",
    );
}

// ── Array iteration ─────────────────────────────────────────────────

#[test]
fn for_in_array() {
    expect(
        "*main()\n    arr is [10, 20, 30]\n    for x in arr\n        log(x)\n",
        "10\n20\n30",
    );
}

#[test]
fn for_in_array_sum() {
    expect(
        "*main()\n    arr is [1, 2, 3, 4, 5]\n    sum is 0\n    for x in arr\n        sum is sum + x\n    log(sum)\n",
        "15",
    );
}

// ── Reference counting ──────────────────────────────────────────────

#[test]
fn rc_create_deref() {
    expect("*main()\n    x is rc(42)\n    log(@x)\n", "42");
}

#[test]
fn rc_retain_release() {
    expect(
        "*main()\n    x is rc(100)\n    rc_retain(x)\n    rc_release(x)\n    log(@x)\n",
        "100",
    );
}

// ── Equals/Neq correctness (zext, not sext) ────────────────────────

#[test]
fn equals_returns_one_not_neg_one() {
    expect(
        "*main()\n    x is 5\n    y is 5\n    z is x equals y\n    log(z)\n",
        "1",
    );
}

#[test]
fn isnt_returns_one_not_neg_one() {
    expect(
        "*main()\n    x is 5\n    y is 6\n    z is x neq y\n    log(z)\n",
        "1",
    );
}

#[test]
fn equals_false_returns_zero() {
    expect(
        "*main()\n    x is 5\n    y is 6\n    z is x equals y\n    log(z)\n",
        "0",
    );
}

#[test]
fn equals_in_arithmetic() {
    expect(
        "*main()\n    a is (3 equals 3) as i64\n    b is (2 equals 2) as i64\n    log(a + b)\n",
        "2",
    );
}

// ── Integer exponentiation ──────────────────────────────────────────

#[test]
fn int_pow_basic() {
    expect("*main()\n    log(2 ** 10)\n", "1024");
}

#[test]
fn int_pow_cubed() {
    expect("*main()\n    log(3 ** 5)\n", "243");
}

#[test]
fn int_pow_zero() {
    expect("*main()\n    log(7 ** 0)\n", "1");
}

#[test]
fn int_pow_one() {
    expect("*main()\n    log(99 ** 1)\n", "99");
}

// ── Nested control flow ─────────────────────────────────────────────

#[test]
fn nested_for_while() {
    expect(
        "*main()\n    sum is 0\n    for i in 0 to 3\n        j is 0\n        while j < 3\n            sum is sum + 1\n            j is j + 1\n    log(sum)\n",
        "9",
    );
}

#[test]
fn for_break_early() {
    expect(
        "*main()\n    sum is 0\n    for i in 0 to 100\n        if i equals 5\n            break\n        sum is sum + i\n    log(sum)\n",
        "10",
    );
}

#[test]
fn while_continue_skip() {
    expect(
        "*main()\n    i is 0\n    sum is 0\n    while i < 10\n        i is i + 1\n        if i % 2 equals 0\n            continue\n        sum is sum + i\n    log(sum)\n",
        "25",
    );
}

// ── Recursion variants ──────────────────────────────────────────────

#[test]
fn mutual_recursion_like() {
    expect(
        "*double(n)\n    n * 2\n\n*main()\n    log(double(21))\n",
        "42",
    );
}

#[test]
fn deep_recursion() {
    expect(
        "*sum_to(n)\n    if n equals 0\n        return 0\n    n + sum_to(n - 1)\n\n*main()\n    log(sum_to(100))\n",
        "5050",
    );
}

// ── Ternary edge cases ──────────────────────────────────────────────

#[test]
fn ternary_nested() {
    expect(
        "*main()\n    x is 5\n    y is x > 10 ? 1 ! x > 3 ? 2 ! 3\n    log(y)\n",
        "2",
    );
}

#[test]
fn ternary_in_expr() {
    expect(
        "*main()\n    a is 10\n    b is 20\n    log(a > b ? a ! b)\n",
        "20",
    );
}

// ── Lambda edge cases ───────────────────────────────────────────────

#[test]
fn lambda_capture_multiple() {
    expect(
        "*main() -> i32\n    a is 10\n    b is 20\n    f is *fn(x: i64) -> i64 a + b + x\n    log(f(5))\n    0\n",
        "35",
    );
}

// ── Generic function specialization ─────────────────────────────────

#[test]
fn generic_fn_different_types() {
    expect(
        "*id(x)\n    x\n\n*main()\n    log(id(42))\n    log(id(99))\n",
        "42\n99",
    );
}

// ── Comparison chain correctness ────────────────────────────────────

#[test]
fn comparison_lt_gt_combined() {
    expect(
        "*main()\n    a is 5\n    log(a < 10)\n    log(a > 3)\n    log(a <= 5)\n    log(a >= 5)\n",
        "1\n1\n1\n1",
    );
}

// ── Modular arithmetic ──────────────────────────────────────────────

#[test]
fn mod_operator() {
    expect(
        "*main()\n    log(10 % 3)\n    log(7 % 2)\n    log(100 % 10)\n",
        "1\n1\n0",
    );
}

// ── Loop with accumulator ───────────────────────────────────────────

#[test]
fn loop_break_with_counter() {
    expect(
        "*main()\n    i is 0\n    loop\n        i is i + 1\n        if i equals 10\n            break\n    log(i)\n",
        "10",
    );
}

// ── Array operations ────────────────────────────────────────────────

#[test]
fn array_index_access() {
    expect(
        "*main()\n    arr is [100, 200, 300]\n    log(arr[0])\n    log(arr[1])\n    log(arr[2])\n",
        "100\n200\n300",
    );
}

#[test]
fn array_iteration_sum() {
    expect(
        "*main()\n    arr is [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]\n    s is 0\n    for x in arr\n        s is s + x\n    log(s)\n",
        "55",
    );
}

// ── Enum with multiple variants ─────────────────────────────────────

#[test]
fn enum_three_variants() {
    expect(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main()\n    c is Green\n    match c\n        Red ? log(1)\n        Green ? log(2)\n        Blue ? log(3)\n",
        "2",
    );
}

// ── Option chain ────────────────────────────────────────────────────

#[test]
fn option_some_unwrap_math() {
    expect(
        "*main()\n    x is Some(21)\n    match x\n        Some(v) ? log(v * 2)\n        Nothing ? log(0)\n",
        "42",
    );
}

#[test]
fn result_ok_err_both() {
    expect(
        "*main()\n    a is Ok(1)\n    b is Err(2)\n    match a\n        Ok(v) ? log(v)\n        Err(e) ? log(e)\n    match b\n        Ok(v) ? log(v)\n        Err(e) ? log(e)\n",
        "1\n2",
    );
}

// ── Recursive Enums ─────────────────────────────────────────────────

#[test]
fn recursive_enum_tree_sum_left_first() {
    // Node(Tree, i64, Tree) — recursive fields before scalar
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*tree_sum(t: Tree) -> i64\n    match t\n        Leaf(v) ? v\n        Node(left, val, right) ? tree_sum(left) + val + tree_sum(right)\n\n*main() -> i32\n    t is Node(Leaf(1), 42, Leaf(3))\n    log(tree_sum(t))\n    0\n",
        "46",
    );
}

#[test]
fn recursive_enum_tree_sum_right_first() {
    // Node(i64, Tree, Tree) — scalar before recursive fields
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(i64, Tree, Tree)\n\n*tree_sum(t: Tree) -> i64\n    match t\n        Leaf(v) ? v\n        Node(val, left, right) ? tree_sum(left) + val + tree_sum(right)\n\n*main() -> i32\n    t is Node(42, Leaf(1), Leaf(3))\n    log(tree_sum(t))\n    0\n",
        "46",
    );
}

#[test]
fn recursive_enum_deep_tree() {
    // Multi-level nesting: Node(Node(Leaf, Leaf), val, Leaf)
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*tree_sum(t: Tree) -> i64\n    match t\n        Leaf(v) ? v\n        Node(left, val, right) ? tree_sum(left) + val + tree_sum(right)\n\n*main() -> i32\n    t is Node(Node(Leaf(10), 20, Leaf(30)), 100, Leaf(5))\n    log(tree_sum(t))\n    0\n",
        "165",
    );
}

#[test]
fn recursive_enum_single_recursive_field() {
    // List with one recursive field
    expect(
        "enum List\n    Nil\n    Cons(i64, List)\n\n*list_sum(l: List) -> i64\n    match l\n        Nil() ? 0\n        Cons(x, rest) ? x + list_sum(rest)\n\n*main() -> i32\n    l is Cons(1, Cons(2, Cons(3, Nil())))\n    log(list_sum(l))\n    0\n",
        "6",
    );
}

#[test]
fn recursive_enum_leaf_only() {
    // Non-recursive variant still works
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*tree_sum(t: Tree) -> i64\n    match t\n        Leaf(v) ? v\n        Node(left, val, right) ? tree_sum(left) + val + tree_sum(right)\n\n*main() -> i32\n    log(tree_sum(Leaf(99)))\n    0\n",
        "99",
    );
}

#[test]
fn recursive_enum_nested_match() {
    // Extract and match on a recursive field's inner value
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*left_val(t: Tree) -> i64\n    match t\n        Leaf(v) ? v\n        Node(left, val, right) ? left_val(left)\n\n*main() -> i32\n    t is Node(Leaf(77), 0, Leaf(88))\n    log(left_val(t))\n    0\n",
        "77",
    );
}

#[test]
fn recursive_enum_count_nodes() {
    // Count internal nodes in a tree
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*count(t: Tree) -> i64\n    match t\n        Leaf(v) ? 0\n        Node(left, val, right) ? 1 + count(left) + count(right)\n\n*main() -> i32\n    t is Node(Node(Leaf(1), 2, Leaf(3)), 4, Node(Leaf(5), 6, Leaf(7)))\n    log(count(t))\n    0\n",
        "3",
    );
}

#[test]
fn recursive_enum_list_length() {
    // Length of a linked list
    expect(
        "enum List\n    Nil\n    Cons(i64, List)\n\n*length(l: List) -> i64\n    match l\n        Nil() ? 0\n        Cons(x, rest) ? 1 + length(rest)\n\n*main() -> i32\n    l is Cons(10, Cons(20, Cons(30, Cons(40, Nil()))))\n    log(length(l))\n    0\n",
        "4",
    );
}

#[test]
fn recursive_enum_tree_depth() {
    // Maximum depth of a binary tree (using ternary for max)
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*max(a: i64, b: i64) -> i64\n    a > b ? a ! b\n\n*depth(t: Tree) -> i64\n    match t\n        Leaf(v) ? 1\n        Node(left, val, right) ? 1 + max(depth(left), depth(right))\n\n*main() -> i32\n    t is Node(Node(Node(Leaf(1), 2, Leaf(3)), 4, Leaf(5)), 6, Leaf(7))\n    log(depth(t))\n    0\n",
        "4",
    );
}

#[test]
fn recursive_enum_tree_map() {
    // Map a function over tree leaves (double each value)
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*tree_sum(t: Tree) -> i64\n    match t\n        Leaf(v) ? v\n        Node(left, val, right) ? tree_sum(left) + val + tree_sum(right)\n\n*main() -> i32\n    t is Node(Leaf(10), 100, Node(Leaf(20), 200, Leaf(30)))\n    log(tree_sum(t))\n    0\n",
        "360",
    );
}

#[test]
fn if_else_implicit_return() {
    // if/else as the last expression in a function body (implicit return)
    expect(
        "*max_val(a: i64, b: i64) -> i64\n    if a > b\n        a\n    else\n        b\n\n*main() -> i32\n    log(max_val(10, 20))\n    log(max_val(20, 10))\n    0\n",
        "20\n20",
    );
}

#[test]
fn if_elif_else_implicit_return() {
    // if/elif/else chain producing a value
    expect(
        "*classify(x: i64) -> i64\n    if x < 0\n        -1\n    elif x > 0\n        1\n    else\n        0\n\n*main() -> i32\n    log(classify(-5))\n    log(classify(0))\n    log(classify(42))\n    0\n",
        "-1\n0\n1",
    );
}

#[test]
fn enum_i32_multi_fields() {
    // Enum with multiple i32 fields — tests correct type_store_size for sub-8-byte types
    expect(
        "enum Shape\n    Circle(i32)\n    Rect(i32, i32)\n    Point(i32, i32, i32)\n\n*describe(s: Shape) -> i64\n    match s\n        Circle(r) ? r as i64\n        Rect(w, h) ? (w as i64) * 100 + (h as i64)\n        Point(x, y, z) ? (x as i64) * 10000 + (y as i64) * 100 + (z as i64)\n\n*main() -> i32\n    c is Circle(7)\n    r is Rect(3, 4)\n    p is Point(1, 2, 3)\n    log(describe(c))\n    log(describe(r))\n    log(describe(p))\n    0\n",
        "7\n304\n10203",
    );
}

#[test]
fn recursive_enum_dynamic_list() {
    // Dynamic linked list construction via if/else return + recursive calls
    expect(
        "enum List\n    Nil\n    Cons(i64, List)\n\n*list_sum(l: List) -> i64\n    match l\n        Nil ? 0\n        Cons(x, rest) ? x + list_sum(rest)\n\n*build(n: i64) -> List\n    if n < 1\n        Nil\n    else\n        Cons(n, build(n - 1))\n\n*main() -> i32\n    l is build(10)\n    log(list_sum(l))\n    0\n",
        "55",
    );
}

#[test]
fn enum_mixed_int_float_fields() {
    // Enum with mixed i32/f64 fields — tests type_store_size and coercion
    expect(
        "enum Value\n    IntVal(i32)\n    FloatVal(f64)\n    Pair(i32, f64)\n\n*extract(v: Value) -> f64\n    match v\n        IntVal(i) ? i as f64\n        FloatVal(f) ? f\n        Pair(i, f) ? (i as f64) + f\n\n*main() -> i32\n    a is IntVal(42)\n    b is FloatVal(3.14)\n    c is Pair(10, 2.5)\n    log(extract(a))\n    log(extract(c))\n    0\n",
        "42.000000\n12.500000",
    );
}

#[test]
fn recursive_enum_reversed_field_order() {
    // Node(i64, Tree, Tree) — both orderings now work correctly
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(i64, Tree, Tree)\n\n*tree_sum(t: Tree) -> i64\n    match t\n        Leaf(v) ? v\n        Node(val, left, right) ? val + tree_sum(left) + tree_sum(right)\n\n*main() -> i32\n    t is Node(6, Node(2, Leaf(1), Leaf(3)), Node(8, Leaf(7), Leaf(9)))\n    log(tree_sum(t))\n    0\n",
        "36",
    );
}

// ── Edge cases ──────────────────────────────────────────────────────

#[test]
fn closure_capture_mutation() {
    expect(
        "*apply(f: (i64) -> i64, x: i64) -> i64\n    f(x)\n\n*main() -> i32\n    base is 100\n    add_base is *fn(x: i64) -> i64 base + x\n    log(apply(add_base, 5))\n    0\n",
        "105",
    );
}

#[test]
fn generic_pipeline_combo() {
    expect(
        "*double(x: i64) -> i64\n    x * 2\n\n*main() -> i32\n    r is 5 ~ double ~ double\n    log(r)\n    0\n",
        "20",
    );
}

#[test]
fn struct_method_chain() {
    expect(
        "type Counter\n    val: i64\n\n    *inc() -> i64\n        self.val + 1\n\n    *double() -> i64\n        self.val * 2\n\n*main() -> i32\n    c is Counter(val is 10)\n    log(c.inc())\n    log(c.double())\n    0\n",
        "11\n20",
    );
}

#[test]
fn deeply_nested_if_expr() {
    expect(
        "*classify(n: i64) -> i64\n    if n > 100\n        return 3\n    elif n > 50\n        return 2\n    elif n > 0\n        return 1\n    else\n        return 0\n\n*main() -> i32\n    log(classify(200))\n    log(classify(75))\n    log(classify(25))\n    log(classify(-5))\n    0\n",
        "3\n2\n1\n0",
    );
}

#[test]
fn match_as_expression() {
    expect(
        "enum Dir\n    Up\n    Down\n    Left\n    Right\n\n*delta(d: Dir) -> i64\n    match d\n        Up() ? 1\n        Down() ? -1\n        Left() ? -10\n        Right() ? 10\n\n*main() -> i32\n    log(delta(Up()))\n    log(delta(Down()))\n    log(delta(Right()))\n    0\n",
        "1\n-1\n10",
    );
}

#[test]
fn array_mutation_and_read() {
    expect(
        "*main() -> i32\n    a is [1, 2, 3, 4, 5]\n    a[0] is 10\n    a[4] is 50\n    log(a[0])\n    log(a[2])\n    log(a[4])\n    0\n",
        "10\n3\n50",
    );
}

#[test]
fn enum_multiple_matches() {
    expect(
        "enum AB\n    A(i64)\n    B(i64)\n\n*main() -> i32\n    x is A(10)\n    y is B(20)\n    match x\n        A(v) ? log(v)\n        B(v) ? log(v + 100)\n    match y\n        A(v) ? log(v + 200)\n        B(v) ? log(v)\n    0\n",
        "10\n20",
    );
}

#[test]
fn for_step_by_three() {
    expect(
        "*main() -> i32\n    s is 0\n    for i in 0 to 10 by 3\n        s is s + i\n    log(s)\n    0\n",
        "18",
    );
}

#[test]
fn nested_function_calls() {
    expect(
        "*a(x: i64) -> i64\n    return x + 1\n\n*b(x: i64) -> i64\n    return a(a(x))\n\n*c(x: i64) -> i64\n    return b(b(x))\n\n*main() -> i32\n    log(c(0))\n    0\n",
        "4",
    );
}

#[test]
fn tuple_destructuring() {
    expect(
        "*main() -> i32\n    x, y is (20, 10)\n    log(x)\n    log(y)\n    0\n",
        "20\n10",
    );
}

#[test]
fn enum_unit_and_data_mixed() {
    expect(
        "enum Token\n    Eof\n    Num(i64)\n    Plus\n\n*describe(t: Token) -> i64\n    match t\n        Eof() ? 0\n        Num(n) ? n\n        Plus() ? -1\n\n*main() -> i32\n    log(describe(Eof()))\n    log(describe(Num(42)))\n    log(describe(Plus()))\n    0\n",
        "0\n42\n-1",
    );
}

#[test]
fn recursive_fibonacci_match() {
    expect(
        "*fib(n: i64) -> i64\n    match n\n        0 ? 0\n        1 ? 1\n        _ ? fib(n - 1) + fib(n - 2)\n\n*main() -> i32\n    log(fib(10))\n    0\n",
        "55",
    );
}

#[test]
fn loop_accumulator() {
    expect(
        "*main() -> i32\n    s is 0\n    i is 1\n    loop\n        if i > 100\n            break\n        s is s + i\n        i is i + 1\n    log(s)\n    0\n",
        "5050",
    );
}

#[test]
fn string_length_method() {
    expect(
        "*main() -> i32\n    s is \"hello world\"\n    log(s.length)\n    0\n",
        "11",
    );
}

#[test]
fn bool_logic_complex() {
    expect(
        "*main() -> i32\n    a is true\n    b is false\n    c is true\n    if a and c\n        log(1)\n    if a or b\n        log(2)\n    if not b\n        log(3)\n    0\n",
        "1\n2\n3",
    );
}

#[test]
fn cast_chain() {
    expect(
        "*main() -> i32\n    x is 42\n    y is x as f64\n    z is y as i64\n    log(z)\n    0\n",
        "42",
    );
}

#[test]
fn struct_field_update() {
    expect(
        "type Pair\n    a: i64\n    b: i64\n\n*main() -> i32\n    p is Pair(a is 1, b is 2)\n    p.a is 99\n    log(p.a)\n    log(p.b)\n    0\n",
        "99\n2",
    );
}

#[test]
fn multi_return_paths() {
    expect(
        "*abs(x: i64) -> i64\n    if x < 0\n        return -x\n    return x\n\n*main() -> i32\n    log(abs(5))\n    log(abs(-5))\n    log(abs(0))\n    0\n",
        "5\n5\n0",
    );
}

#[test]
fn pipeline_multi_arg() {
    expect(
        "*add(a: i64, b: i64) -> i64\n    a + b\n\n*mul(a: i64, b: i64) -> i64\n    a * b\n\n*main() -> i32\n    r is 10 ~ add(5)\n    log(r)\n    0\n",
        "15",
    );
}

#[test]
fn nested_array_access() {
    expect(
        "*main() -> i32\n    a is [10, 20, 30]\n    i is 2\n    log(a[i])\n    log(a[0] + a[i])\n    0\n",
        "30\n40",
    );
}

// ── Store Tests ──────────────────────────────────────────────────────

#[test]
fn store_insert_count_int() {
    expect_store(
        "store nums\n    val: i64\n\n*main\n    insert nums 10\n    insert nums 20\n    insert nums 30\n    n is count nums\n    log n\n",
        "3",
    );
}

#[test]
fn store_insert_count_string() {
    expect_store(
        "store names\n    name: String\n    age: i64\n\n*main\n    insert names 'Alice', 30\n    insert names 'Bob', 25\n    insert names 'Charlie', 35\n    n is count names\n    log n\n",
        "3",
    );
}

#[test]
fn store_query_int() {
    expect_store(
        "store vals\n    x: i64\n\n*main\n    insert vals 10\n    insert vals 20\n    insert vals 30\n    r is vals where x > 15\n    log r.x\n",
        "20",
    );
}

#[test]
fn store_query_string_field() {
    expect_store(
        "store people\n    name: String\n    age: i64\n\n*main\n    insert people 'Alice', 30\n    insert people 'Bob', 25\n    insert people 'Charlie', 35\n    young is people where age < 30\n    log young.name\n    log young.age\n",
        "Bob\n25",
    );
}

#[test]
fn store_query_string_equality() {
    expect_store(
        "store people\n    name: String\n    age: i64\n\n*main\n    insert people 'Alice', 30\n    insert people 'Bob', 25\n    found is people where name equals 'Bob'\n    log found.name\n    log found.age\n",
        "Bob\n25",
    );
}

#[test]
fn store_delete() {
    expect_store(
        "store users\n    name: String\n    age: i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n    n1 is count users\n    log n1\n    delete users where age > 28\n    n2 is count users\n    log n2\n",
        "3\n1",
    );
}

#[test]
fn store_empty_count() {
    expect_store(
        "store empty\n    val: i64\n\n*main\n    n is count empty\n    log n\n",
        "0",
    );
}

#[test]
fn store_set_basic() {
    expect_store(
        "store users\n    name: String\n    age: i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    set users where name equals 'Bob' age 99\n    r is users where name equals 'Bob'\n    log r.name\n    log r.age\n",
        "Bob\n99",
    );
}

#[test]
fn store_set_multiple_fields() {
    expect_store(
        "store items\n    name: String\n    price: i64\n    qty: i64\n\n*main\n    insert items 'Widget', 100, 50\n    set items where name equals 'Widget' price 200, qty 10\n    r is items where name equals 'Widget'\n    log r.price\n    log r.qty\n",
        "200\n10",
    );
}

#[test]
fn store_set_no_match() {
    expect_store(
        "store nums\n    val: i64\n\n*main\n    insert nums 10\n    insert nums 20\n    set nums where val equals 999 val 99\n    n is count nums\n    log n\n    r is nums where val equals 10\n    log r.val\n",
        "2\n10",
    );
}

#[test]
fn store_transaction() {
    expect_store(
        "store users\n    name: String\n    age: i64\n\n*main\n    transaction\n        insert users 'Alice', 30\n        insert users 'Bob', 25\n        insert users 'Charlie', 35\n    c is count users\n    log c\n    r is users where name equals 'Bob'\n    log r.name\n    log r.age\n",
        "3\nBob\n25",
    );
}

#[test]
fn store_and_filter_query() {
    expect_store(
        "store products\n    name: String\n    price: i64\n    stock: i64\n\n*main\n    insert products 'Apple', 100, 50\n    insert products 'Banana', 50, 100\n    insert products 'Cherry', 100, 10\n    r is products where price equals 100 and stock > 20\n    log r.name\n    log r.stock\n",
        "Apple\n50",
    );
}

#[test]
fn store_and_filter_delete() {
    expect_store(
        "store products\n    name: String\n    price: i64\n    stock: i64\n\n*main\n    insert products 'Apple', 100, 50\n    insert products 'Banana', 50, 100\n    insert products 'Cherry', 100, 10\n    delete products where price equals 100 and stock < 20\n    c is count products\n    log c\n",
        "2",
    );
}

#[test]
fn store_or_filter_delete() {
    expect_store(
        "store items\n    name: String\n    value: i64\n\n*main\n    insert items 'Alpha', 10\n    insert items 'Beta', 20\n    insert items 'Gamma', 30\n    delete items where value equals 10 or value equals 30\n    c is count items\n    log c\n    r is items where name equals 'Beta'\n    log r.value\n",
        "1\n20",
    );
}

#[test]
fn store_and_filter_set() {
    expect_store(
        "store users\n    name: String\n    age: i64\n    active: i64\n\n*main\n    insert users 'Alice', 30, 1\n    insert users 'Bob', 25, 1\n    insert users 'Charlie', 25, 0\n    set users where age equals 25 and active equals 1 age 99\n    r is users where name equals 'Bob'\n    log r.age\n    r2 is users where name equals 'Charlie'\n    log r2.age\n",
        "99\n25",
    );
}

#[test]
fn store_delete_then_query() {
    expect_store(
        "store users\n    name: String\n    age: i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n    delete users where name equals 'Bob'\n    r is users where name equals 'Charlie'\n    log r.name\n    log r.age\n",
        "Charlie\n35",
    );
}

#[test]
fn store_multi_type_fields() {
    expect_store(
        "store data\n    x: i64\n    y: f64\n\n*main\n    insert data 42, 3.14\n    r is data where x equals 42\n    log r.x\n",
        "42",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tuple destructuring in match
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn match_tuple_basic() {
    expect(
        "*main() -> i32\n    t is (10, 20)\n    match t\n        (a, b) ? log(a + b)\n    0\n",
        "30",
    );
}

#[test]
fn match_tuple_multiple_arms() {
    expect(
        "*main() -> i32\n    t is (10, 20)\n    match t\n        (a, b) ? log(a + b)\n    t2 is (3, 4)\n    match t2\n        (x, y) ? log(x * y)\n    0\n",
        "30\n12",
    );
}

#[test]
fn match_array_basic() {
    expect(
        "*main() -> i32\n    a is [10, 20, 30]\n    match a\n        [x, y, z] ? log(x + y + z)\n    0\n",
        "60",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Operator overloading (Add, Sub, Mul, Div, Lt, Gt, Le, Ge)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn operator_overload_add() {
    expect(
        "type Vec2\n    x: i64\n    y: i64\n\ntrait Add\n    *add(other: Vec2) -> Vec2\n\nimpl Add for Vec2\n    *add(other: Vec2) -> Vec2\n        Vec2(x is self.x + other.x, y is self.y + other.y)\n\n*main() -> i32\n    a is Vec2(x is 1, y is 2)\n    b is Vec2(x is 3, y is 4)\n    c is a + b\n    log(c.x)\n    log(c.y)\n    0\n",
        "4\n6",
    );
}

#[test]
fn operator_overload_sub() {
    expect(
        "type Vec2\n    x: i64\n    y: i64\n\ntrait Sub\n    *sub(other: Vec2) -> Vec2\n\nimpl Sub for Vec2\n    *sub(other: Vec2) -> Vec2\n        Vec2(x is self.x - other.x, y is self.y - other.y)\n\n*main() -> i32\n    a is Vec2(x is 10, y is 20)\n    b is Vec2(x is 3, y is 5)\n    c is a - b\n    log(c.x)\n    log(c.y)\n    0\n",
        "7\n15",
    );
}

#[test]
fn operator_overload_mul() {
    expect(
        "type Vec2\n    x: i64\n    y: i64\n\ntrait Mul\n    *mul(other: Vec2) -> i64\n\nimpl Mul for Vec2\n    *mul(other: Vec2) -> i64\n        self.x * other.x + self.y * other.y\n\n*main() -> i32\n    a is Vec2(x is 2, y is 3)\n    b is Vec2(x is 4, y is 5)\n    log(a * b)\n    0\n",
        "23",
    );
}

#[test]
fn operator_overload_lt() {
    expect(
        "type Score\n    val: i64\n\ntrait Ord\n    *less(other: Score) -> bool\n\nimpl Ord for Score\n    *less(other: Score) -> bool\n        self.val < other.val\n\n*main() -> i32\n    a is Score(val is 5)\n    b is Score(val is 10)\n    log(a < b)\n    log(b < a)\n    0\n",
        "1\n0",
    );
}

#[test]
fn operator_overload_gt() {
    expect(
        "type Score\n    val: i64\n\ntrait Ord\n    *greater(other: Score) -> bool\n\nimpl Ord for Score\n    *greater(other: Score) -> bool\n        self.val > other.val\n\n*main() -> i32\n    a is Score(val is 5)\n    b is Score(val is 10)\n    log(b > a)\n    log(a > b)\n    0\n",
        "1\n0",
    );
}

#[test]
fn operator_overload_le_ge() {
    expect(
        "type Val\n    n: i64\n\ntrait Cmp\n    *less_eq(other: Val) -> bool\n    *greater_eq(other: Val) -> bool\n\nimpl Cmp for Val\n    *less_eq(other: Val) -> bool\n        self.n <= other.n\n    *greater_eq(other: Val) -> bool\n        self.n >= other.n\n\n*main() -> i32\n    a is Val(n is 5)\n    b is Val(n is 5)\n    c is Val(n is 10)\n    log(a <= b)\n    log(a >= b)\n    log(a <= c)\n    log(c >= a)\n    0\n",
        "1\n1\n1\n1",
    );
}

#[test]
fn operator_overload_display() {
    expect(
        "type Point\n    x: i64\n    y: i64\n\ntrait Display\n    *display() -> String\n\nimpl Display for Point\n    *display() -> String\n        'point'\n\n*main() -> i32\n    p is Point(x is 3, y is 4)\n    s is to_string(p)\n    log(s)\n    0\n",
        "point",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Dynamic dispatch (dyn Trait)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn dyn_trait_basic() {
    expect(
        "type Dog\n    x: i64\n\ntrait Animal\n    *speak() -> i64\n\nimpl Animal for Dog\n    *speak() -> i64\n        self.x\n\n*call_speak(a: dyn Animal) -> i64\n    a.speak()\n\n*main() -> i32\n    d is Dog(x is 42)\n    log(call_speak(d))\n    0\n",
        "42",
    );
}

#[test]
fn dyn_trait_multiple_types() {
    expect(
        "type Cat\n    lives: i64\n\ntype Dog\n    age: i64\n\ntrait Animal\n    *value() -> i64\n\nimpl Animal for Cat\n    *value() -> i64\n        self.lives\n\nimpl Animal for Dog\n    *value() -> i64\n        self.age\n\n*get_val(a: dyn Animal) -> i64\n    a.value()\n\n*main() -> i32\n    c is Cat(lives is 9)\n    d is Dog(age is 5)\n    log(get_val(c))\n    log(get_val(d))\n    0\n",
        "9\n5",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Fieldless enum zero-cost (tag-only)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn fieldless_enum_still_works() {
    expect(
        "enum Dir\n    North\n    South\n    East\n    West\n\n*main() -> i32\n    d is South()\n    match d\n        North() ? log(0)\n        South() ? log(1)\n        East() ? log(2)\n        West() ? log(3)\n    0\n",
        "1",
    );
}

#[test]
fn option_still_works() {
    expect(
        "enum Option of T\n    Some(T)\n    Nothing\n\n*main() -> i32\n    x is Some(42)\n    match x\n        Some(v) ? log(v)\n        Nothing() ? log(0)\n    0\n",
        "42",
    );
}

// ── Perceus / Drop tests ──────────────────────────────────────────────

#[test]
fn drop_short_string_sso() {
    // SSO strings (≤23 chars) should not crash — no heap to free
    expect("*main()\n    s is 'short'\n    log(s)\n", "short");
}

#[test]
fn drop_long_string_heap() {
    // Long strings (>23 chars) use heap — drop should not crash
    expect(
        "*main()\n    s is 'this is a long string that exceeds sso'\n    log(s.length)\n",
        "38",
    );
}

#[test]
fn drop_string_in_scope_block() {
    // String bound inside an if-block should be dropped at block exit
    expect(
        "*main() -> i32\n    if true\n        temp is 'hello world from a block'\n        log(temp)\n    0\n",
        "hello world from a block",
    );
}

#[test]
fn drop_multiple_strings() {
    expect(
        "*main()\n    a is 'first'\n    b is 'second'\n    c is 'third'\n    log(a)\n    log(b)\n    log(c)\n",
        "first\nsecond\nthird",
    );
}

#[test]
fn drop_string_after_fn_call() {
    // String passed to a function should not double-free
    expect(
        "*greet(name: String)\n    log(name)\n\n*main()\n    s is 'world'\n    greet(s)\n",
        "world",
    );
}

#[test]
fn drop_rc_basic() {
    expect("*main()\n    x is rc(42)\n    log(@x)\n", "42");
}

#[test]
fn drop_vec_basic() {
    expect("*main()\n    v is vec(1, 2, 3)\n    log(v.len())\n", "3");
}

#[test]
fn drop_does_not_affect_scalars() {
    // Scalars should have elided drops — no crash
    expect(
        "*main()\n    x is 42\n    y is 3.14\n    z is true\n    log(x)\n    log(y)\n    log(z)\n",
        "42\n3.140000\n1",
    );
}

// ── Layout attributes (@packed, @strict, @align) ───────────────────

#[test]
fn packed_struct_field_access() {
    // @packed struct should work correctly with no padding
    expect(
        "type Compact @packed\n    a: i8\n    b: i64\n    c: i8\n\n*main()\n    s is Compact(a is 1, b is 42, c is 3)\n    log(s.a)\n    log(s.b)\n    log(s.c)\n",
        "1\n42\n3",
    );
}

#[test]
fn strict_struct_field_order() {
    // @strict guarantees declaration order is preserved
    expect(
        "type Ordered @strict\n    x: i64\n    y: i64\n    z: i64\n\n*main()\n    s is Ordered(x is 10, y is 20, z is 30)\n    log(s.x)\n    log(s.y)\n    log(s.z)\n",
        "10\n20\n30",
    );
}

#[test]
fn align_struct_field_access() {
    // @align(64) struct should work correctly with cache-line alignment
    expect(
        "type Aligned @align(64)\n    val: i64\n    flag: i8\n\n*main()\n    s is Aligned(val is 99, flag is 7)\n    log(s.val)\n    log(s.flag)\n",
        "99\n7",
    );
}

#[test]
fn packed_strict_combined() {
    // @packed @strict together
    expect(
        "type PS @packed @strict\n    a: i8\n    b: i64\n\n*main()\n    s is PS(a is 5, b is 100)\n    log(s.a)\n    log(s.b)\n",
        "5\n100",
    );
}

// ── Branch hints (likely/unlikely) ─────────────────────────────────

#[test]
fn likely_builtin() {
    expect(
        "*main()\n    x is 10\n    if likely(x > 5)\n        log(1)\n    else\n        log(0)\n",
        "1",
    );
}

#[test]
fn unlikely_builtin() {
    expect(
        "*main()\n    x is 10\n    if unlikely(x > 100)\n        log(1)\n    else\n        log(0)\n",
        "0",
    );
}

// ── Perceus reuse codegen ──────────────────────────────────────────

#[test]
fn perceus_rc_reuse_same_type() {
    // Two sequential rc allocs of same type — second should reuse the first's memory
    expect(
        "*main()\n    x is rc(42)\n    log(@x)\n    y is rc(99)\n    log(@y)\n",
        "42\n99",
    );
}

#[test]
fn perceus_rc_values_independent() {
    // Ensure reused memory has correct new values
    expect(
        "*main()\n    a is rc(10)\n    log(@a)\n    b is rc(20)\n    log(@b)\n    c is rc(30)\n    log(@c)\n",
        "10\n20\n30",
    );
}

// ── FBIP (Functional But In-Place) ─────────────────────────────────

#[test]
fn fbip_match_reconstruct_enum() {
    // Match on an enum, reconstruct a variant — should work correctly
    // (FBIP analysis may detect reuse opportunity)
    expect(
        "enum Shape\n    Circle(f64)\n    Square(f64)\n\n*double_shape(s: Shape) -> Shape\n    match s\n        Circle(r) ? Circle(r * 2.0)\n        Square(side) ? Square(side * 2.0)\n\n*main()\n    c is double_shape(Circle(5.0))\n    match c\n        Circle(r) ? log(r)\n        Square(_) ? log(0.0)\n",
        "10.000000",
    );
}

#[test]
fn fbip_match_transform_variant() {
    // Transform one variant to another of the same enum
    expect(
        "enum Op\n    Add(i64)\n    Mul(i64)\n\n*negate(op: Op) -> Op\n    match op\n        Add(n) ? Add(0 - n)\n        Mul(n) ? Mul(0 - n)\n\n*main()\n    r is negate(Add(42))\n    match r\n        Add(n) ? log(n)\n        Mul(n) ? log(n)\n",
        "-42",
    );
}

// ── Pool Allocator ─────────────────────────────────────────────────

#[test]
fn pool_create_alloc_free() {
    // Create a pool, allocate from it, free back, allocate again
    expect(
        "*main()\n    p is Pool(8, 16)\n    slot is p.alloc()\n    p.free(slot)\n    slot2 is p.alloc()\n    log(42)\n    p.destroy()\n",
        "42",
    );
}

#[test]
fn pool_multiple_allocs() {
    // Allocate multiple slots and verify pool works
    expect(
        "*main()\n    p is Pool(8, 64)\n    a is p.alloc()\n    b is p.alloc()\n    c is p.alloc()\n    p.free(b)\n    d is p.alloc()\n    log(100)\n    p.destroy()\n",
        "100",
    );
}

// ── Tail Reuse ─────────────────────────────────────────────────────

#[test]
fn tail_reuse_enum_transform() {
    // Function takes owned enum, returns same enum type — Perceus should
    // detect tail reuse opportunity. Values should be correct.
    expect(
        "enum Shape\n    Circle(f64)\n    Square(f64)\n\n*scale(s: Shape, factor: f64) -> Shape\n    match s\n        Circle(r) ? Circle(r * factor)\n        Square(side) ? Square(side * factor)\n\n*main()\n    c is scale(Circle(3.0), 2.0)\n    match c\n        Circle(r) ? log(r)\n        Square(_) ? log(0.0)\n",
        "6.000000",
    );
}

#[test]
fn tail_reuse_rc_reconstruct() {
    // Rc values get correct computed results in a function
    expect(
        "*main()\n    x is rc(7)\n    v is @x * 10\n    a is rc(v)\n    log(@a)\n",
        "70",
    );
}

#[test]
fn pool_perceus_loop_alloc() {
    // Rc allocations inside a loop should still produce correct results
    // (Perceus pool hints detect this pattern for optimization)
    expect(
        "*main()\n    i is 0\n    while i < 5\n        x is rc(i * 10)\n        log(@x)\n        i is i + 1\n",
        "0\n10\n20\n30\n40",
    );
}

#[test]
fn pool_perceus_nested_loop_alloc() {
    // Nested loops with Rc allocs should work correctly
    expect(
        "*main()\n    i is 0\n    while i < 3\n        j is 0\n        while j < 2\n            x is rc(i + j)\n            log(@x)\n            j is j + 1\n        i is i + 1\n",
        "0\n1\n1\n2\n2\n3",
    );
}
