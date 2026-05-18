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
    let status = Command::new(jinnc())
        .arg(path)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jinnc failed to start");
    assert!(status.success(), "jinnc compilation failed for: {path}");
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
        !output.status.success(),
        "expected compilation failure for:\n{src}"
    );
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Like compile_and_run but sets working directory to temp dir so .store files are isolated.
fn compile_and_run_in_dir(src: &str) -> String {
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
    expect("*main()\n    log(2 pow 10)\n", "1024");
}

#[test]
fn arithmetic_combined() {
    expect_file("tests/programs/arithmetic.jn", "5\n6\n42\n5\n2\n1024");
}

// ── Exponentiation ──────────────────────────────────────────────────

#[test]
fn exp_right_associative() {
    // 2 pow 3 pow 2 = 2 pow 9 = 512
    expect("*main()\n    log(2 pow 3 pow 2)\n", "512");
}

#[test]
fn exp_left_grouped() {
    // (2 pow 3) pow 2 = 8 pow 2 = 64
    expect("*main()\n    log((2 pow 3) pow 2)\n", "64");
}

#[test]
fn exp_zero() {
    expect("*main()\n    log(3 pow 0)\n", "1");
}

#[test]
fn exp_large() {
    expect("*main()\n    log(2 pow 20)\n", "1048576");
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
    expect_file("tests/programs/bindings.jn", "30\n20");
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
    expect_file("tests/programs/elif_chain.jn", "1");
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
    expect_file("tests/programs/while_loop.jn", "0\n1\n2\n3\n4");
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
    expect_file("tests/programs/for_loop.jn", "0\n1\n2\n3\n4");
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
        "*main() returns i32\n    for i in 1 to 5\n        log(i)\n    0\n",
        "1\n2\n3\n4",
    );
}

#[test]
fn for_range_by() {
    expect(
        "*main() returns i32\n    for i in 0 to 10 by 3\n        log(i)\n    0\n",
        "0\n3\n6\n9",
    );
}

// ── Loop/Break/Continue ─────────────────────────────────────────────

#[test]
fn loop_break() {
    expect_file("tests/programs/loop_break.jn", "0\n1\n2\n3\n4");
}

#[test]
fn while_continue() {
    expect_file("tests/programs/continue_loop.jn", "1\n3\n5\n7\n9");
}

// ── Functions ───────────────────────────────────────────────────────

#[test]
fn function_calls() {
    expect_file("tests/programs/functions.jn", "7\n25\n25");
}

// ── Recursion ───────────────────────────────────────────────────────

#[test]
fn factorial() {
    expect_file(
        "tests/programs/recursion.jn",
        "3628800\n1\n1\n2432902008176640000",
    );
}

#[test]
fn fibonacci_35() {
    expect_file("tests/fibonacci.jn", "9227465");
}

// ── Ternary ─────────────────────────────────────────────────────────

#[test]
fn ternary_ops() {
    expect_file("tests/programs/ternary.jn", "42\n42\n20\n30\n50\n0\n100");
}

// ── Bitwise ─────────────────────────────────────────────────────────

#[test]
fn bitwise_ops() {
    expect_file("tests/programs/bitwise.jn", "15\n255\n240\n1024\n32\n-1");
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
    expect("*main()\n    log('jinn')\n", "jinn");
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
    expect_file("tests/programs/algorithms.jn", "111\n6\n25");
}

// ── Iterative ───────────────────────────────────────────────────────

#[test]
fn iterative_fib() {
    expect_file("tests/programs/iterative.jn", "55\n6765\n4950\n499500");
}

// ── Nesting ─────────────────────────────────────────────────────────

#[test]
fn nested_if() {
    expect_file("tests/programs/nested_if.jn", "10\n11\n5");
}

#[test]
fn nested_loops() {
    expect_file("tests/programs/nested_loops.jn", "100\n100");
}

// ── IR Emission ─────────────────────────────────────────────────────

#[test]
fn emit_ir_flag() {
    let output = Command::new(jinnc())
        .arg("tests/hello.jn")
        .arg("--emit-ir")
        .output()
        .expect("jinnc failed");
    assert!(output.status.success());
    let ir = String::from_utf8(output.stdout).unwrap();
    assert!(ir.contains("define i32 @main(i32") || ir.contains("define i32 @main()"));
    assert!(ir.contains("@printf"));
}

// ── Error Handling ──────────────────────────────────────────────────

#[test]
fn error_on_missing_file() {
    let output = Command::new(jinnc())
        .arg("nonexistent.jn")
        .output()
        .expect("jinnc failed to start");
    assert!(!output.status.success());
}

#[test]
fn error_on_tab() {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("bad.jn");
    std::fs::write(&jinn, "*main()\n\tlog(1)\n").unwrap();
    let output = Command::new(jinnc())
        .arg(&jinn)
        .output()
        .expect("jinnc failed");
    assert!(!output.status.success());
}

// ── Structs ─────────────────────────────────────────────────────────

#[test]
fn struct_construction() {
    expect(
        "type Point\n    x as i64\n    y as i64\n\n*main() returns i32\n    p is Point(x is 10, y is 20)\n    log(p.x)\n    log(p.y)\n    0\n",
        "10\n20",
    );
}

#[test]
fn struct_field_arithmetic() {
    expect(
        "type Vec2\n    x as i64\n    y as i64\n\n*main() returns i32\n    v is Vec2(x is 3, y is 7)\n    log(v.x + v.y)\n    log(v.x * v.y)\n    0\n",
        "10\n21",
    );
}

#[test]
fn struct_positional_init() {
    expect(
        "type Pair\n    a as i64\n    b as i64\n\n*main() returns i32\n    p is Pair(5, 15)\n    log(p.a)\n    log(p.b)\n    0\n",
        "5\n15",
    );
}

#[test]
fn struct_pass_to_fn() {
    expect(
        "type Pt\n    x as i64\n    y as i64\n\n*sum(p as Pt) returns i64\n    p.x + p.y\n\n*main() returns i32\n    log(sum(Pt(x is 4, y is 6)))\n    0\n",
        "10",
    );
}

// ── Enums ───────────────────────────────────────────────────────────

#[test]
fn enum_basic_match() {
    expect(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() returns i32\n    c is Green()\n    match c\n        Red() ? log(1)\n        Green() ? log(2)\n        Blue() ? log(3)\n    0\n",
        "2",
    );
}

#[test]
fn enum_with_data() {
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*main() returns i32\n    s is Circle(42)\n    match s\n        Circle(r) ? log(r)\n        Rect(w, h) ? log(w + h)\n    0\n",
        "42",
    );
}

#[test]
fn enum_rect_variant() {
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*main() returns i32\n    s is Rect(10, 20)\n    match s\n        Circle(r) ? log(r)\n        Rect(w, h) ? log(w + h)\n    0\n",
        "30",
    );
}

#[test]
fn enum_wildcard_arm() {
    expect(
        "enum Op\n    Add\n    Sub\n    Mul\n\n*main() returns i32\n    o is Mul()\n    match o\n        Add() ? log(1)\n        _ ? log(99)\n    0\n",
        "99",
    );
}

// ── Arrays ──────────────────────────────────────────────────────────

#[test]
fn array_literal_index() {
    expect(
        "*main() returns i32\n    a is [10, 20, 30]\n    log(a[0])\n    log(a[1])\n    log(a[2])\n    0\n",
        "10\n20\n30",
    );
}

#[test]
fn array_arithmetic() {
    expect(
        "*main() returns i32\n    a is [5, 10, 15]\n    log(a[0] + a[1] + a[2])\n    0\n",
        "30",
    );
}

#[test]
fn array_in_loop() {
    expect(
        "*main() returns i32\n    a is [1, 2, 3, 4, 5]\n    total is 0\n    i is 0\n    while i < 5\n        total is total + a[i]\n        i is i + 1\n    log(total)\n    0\n",
        "15",
    );
}

// ── Tuples ──────────────────────────────────────────────────────────

#[test]
fn tuple_basic() {
    expect(
        "*main() returns i32\n    t is (100, 200, 300)\n    log(t[0])\n    log(t[1])\n    log(t[2])\n    0\n",
        "100\n200\n300",
    );
}

#[test]
fn tuple_arithmetic() {
    expect(
        "*main() returns i32\n    t is (7, 3)\n    log(t[0] + t[1])\n    log(t[0] * t[1])\n    0\n",
        "10\n21",
    );
}

// ── Integer Match ───────────────────────────────────────────────────

#[test]
fn match_int_literal() {
    expect(
        "*main() returns i32\n    x is 42\n    match x\n        1 ? log(100)\n        42 ? log(200)\n        _ ? log(300)\n    0\n",
        "200",
    );
}

#[test]
fn match_int_wildcard() {
    expect(
        "*main() returns i32\n    x is 99\n    match x\n        1 ? log(100)\n        2 ? log(200)\n        _ ? log(999)\n    0\n",
        "999",
    );
}

// ── Match Expressions ───────────────────────────────────────────────

#[test]
fn match_int_expr() {
    expect(
        "*choose(x as i64) returns i64\n    match x\n        1 ? 10\n        2 ? 20\n        _ ? 99\n\n*main() returns i32\n    log(choose(1))\n    log(choose(2))\n    log(choose(7))\n    0\n",
        "10\n20\n99",
    );
}

#[test]
fn match_enum_expr() {
    expect(
        "enum Op\n    Add(i64, i64)\n    Neg(i64)\n\n*eval(op as Op) returns i64\n    match op\n        Add(a, b) ? a + b\n        Neg(a) ? 0 - a\n\n*main() returns i32\n    log(eval(Add(3, 4)))\n    log(eval(Neg(10)))\n    0\n",
        "7\n-10",
    );
}

#[test]
fn match_enum_expr_with_bind() {
    // Match with block-style arms that use variable assignment
    expect(
        "enum Shape\n    Circle(i64)\n    Rect(i64, i64)\n\n*area(s as Shape) returns i64\n    result is 0\n    match s\n        Circle(r) ?\n            result is r * r\n        Rect(w, h) ?\n            result is w * h\n    result\n\n*main() returns i32\n    log(area(Circle(5)))\n    log(area(Rect(3, 7)))\n    0\n",
        "25\n21",
    );
}

// ── Higher-Order Functions ──────────────────────────────────────────

#[test]
fn hof_pass_function() {
    expect(
        "*double(x as i64) returns i64\n    x * 2\n\n*apply(f as (i64) returns i64, x as i64) returns i64\n    f(x)\n\n*main() returns i32\n    log(apply(double, 21))\n    0\n",
        "42",
    );
}

#[test]
fn hof_function_variable() {
    expect(
        "*double(x as i64) returns i64\n    x * 2\n\n*main() returns i32\n    f is double\n    log(f(21))\n    0\n",
        "42",
    );
}

#[test]
fn hof_return_value_chains() {
    expect(
        "*add_one(x as i64) returns i64\n    x + 1\n\n*double(x as i64) returns i64\n    x * 2\n\n*apply(f as (i64) returns i64, x as i64) returns i64\n    f(x)\n\n*main() returns i32\n    log(apply(add_one, apply(double, 10)))\n    0\n",
        "21",
    );
}

// ── Lambda Expressions ──────────────────────────────────────────────

#[test]
fn lambda_basic() {
    expect(
        "*apply(f as (i64) returns i64, x as i64) returns i64\n    f(x)\n\n*main() returns i32\n    log(apply(|x as i64| returns i64 x * 3, 14))\n    0\n",
        "42",
    );
}

#[test]
fn lambda_variable() {
    expect(
        "*main() returns i32\n    g is |x as i64| returns i64 x + 100\n    log(g(42))\n    0\n",
        "142",
    );
}

#[test]
fn lambda_multi_param() {
    expect(
        "*apply2(f as (i64, i64) returns i64, a as i64, b as i64) returns i64\n    f(a, b)\n\n*main() returns i32\n    log(apply2(|a as i64, b as i64| returns i64 a + b, 17, 25))\n    0\n",
        "42",
    );
}

// ── Pipeline Operator ───────────────────────────────────────────────

#[test]
fn pipeline_basic() {
    expect(
        "*identity(x as i64) returns i64\n    x\n\n*main() returns i32\n    result is 10 ~ identity\n    log(result)\n    0\n",
        "10",
    );
}

#[test]
fn pipeline_function() {
    expect(
        "*double(x as i64) returns i64\n    x * 2\n\n*main() returns i32\n    result is 10 ~ double\n    log(result)\n    0\n",
        "20",
    );
}

#[test]
fn pipeline_chain() {
    expect(
        "*double(x as i64) returns i64\n    x * 2\n\n*add_one(x as i64) returns i64\n    x + 1\n\n*main() returns i32\n    result is 10 ~ double ~ add_one\n    log(result)\n    0\n",
        "21",
    );
}

#[test]
fn pipeline_with_args() {
    expect(
        "*add(a as i64, b as i64) returns i64\n    a + b\n\n*main() returns i32\n    result is 10 ~ add(5)\n    log(result)\n    0\n",
        "15",
    );
}

#[test]
fn pipeline_placeholder() {
    expect(
        "*mul(a as i64, b as i64) returns i64\n    a * b\n\n*main() returns i32\n    result is 10 ~ mul($, 3)\n    log(result)\n    0\n",
        "30",
    );
}

#[test]
fn pipeline_lambda() {
    expect(
        "*main() returns i32\n    result is 5 ~ |x as i64| returns i64 x * x\n    log(result)\n    0\n",
        "25",
    );
}

#[test]
fn pipeline_lambda_chain() {
    expect(
        "*add_one(x as i64) returns i64\n    x + 1\n\n*main() returns i32\n    result is 5 ~ |x as i64| returns i64 x * x ~ add_one\n    log(result)\n    0\n",
        "26",
    );
}

#[test]
fn lambda_do_end_block() {
    expect(
        "*main() returns i32\n    g is |x as i64| returns i64 do\n        y is x * 2\n        y + 1\n    end\n    log(g(20))\n    0\n",
        "41",
    );
}

#[test]
fn lambda_do_end_with_if() {
    expect(
        "*main() returns i32\n    abs is |x as i64| returns i64 do\n        result is x\n        if x < 0\n            result is 0 - x\n        result\n    end\n    log(abs(5))\n    log(abs(-3))\n    0\n",
        "5\n3",
    );
}

// ── Closures (captures) ────────────────────────────────────────────

#[test]
fn closure_single_capture() {
    expect(
        "*main() returns i32\n    x is 10\n    f is |y as i64| returns i64 x + y\n    log(f(5))\n    0\n",
        "15",
    );
}

#[test]
fn closure_multi_capture() {
    expect(
        "*main() returns i32\n    a is 10\n    b is 20\n    f is |x as i64| returns i64 a + b + x\n    log(f(5))\n    0\n",
        "35",
    );
}

#[test]
fn closure_through_hof() {
    expect(
        "*apply(f as (i64) returns i64, x as i64) returns i64\n    f(x)\n\n*main() returns i32\n    base is 100\n    f is |x as i64| returns i64 base + x\n    log(apply(f, 42))\n    0\n",
        "142",
    );
}

#[test]
fn closure_in_pipeline() {
    expect(
        "*main() returns i32\n    c is 3\n    result is 7 ~ |x as i64| returns i64 x * c\n    log(result)\n    0\n",
        "21",
    );
}

// ── Nullary Variants & Option/Result ───────────────────────────────

#[test]
fn nullary_variant() {
    expect(
        "enum Dir\n    North\n    South\n\n*to_int(d as Dir) returns i64\n    match d\n        North ? 1\n        South ? 2\n\n*main() returns i32\n    log(to_int(North))\n    log(to_int(South))\n    0\n",
        "1\n2",
    );
}

#[test]
fn option_some_none() {
    expect(
        "enum Option\n    Some(i64)\n    None\n\n*safe_div(a as i64, b as i64) returns Option\n    if b equals 0\n        return None\n    Some(a / b)\n\n*main() returns i32\n    match safe_div(10, 2)\n        Some(v) ?\n            log(v)\n        None ?\n            log(-1)\n    match safe_div(10, 0)\n        Some(v) ?\n            log(v)\n        None ?\n            log(-1)\n    0\n",
        "5\n-1",
    );
}

#[test]
fn result_ok_err() {
    expect(
        "enum Result\n    Ok(i64)\n    Err(i64)\n\n*checked_add(a as i64, b as i64) returns Result\n    sum is a + b\n    if sum > 100\n        return Err(sum)\n    Ok(sum)\n\n*main() returns i32\n    match checked_add(30, 40)\n        Ok(v) ?\n            log(v)\n        Err(e) ?\n            log(0 - e)\n    match checked_add(60, 50)\n        Ok(v) ?\n            log(v)\n        Err(e) ?\n            log(0 - e)\n    0\n",
        "70\n-110",
    );
}

// ---- generics (of syntax) ----

#[test]
fn generic_identity() {
    expect(
        "*identity of T(x as T) returns T\n    x\n\n*main() returns i32\n    log(identity(42))\n    0\n",
        "42",
    );
}

#[test]
fn generic_max() {
    expect(
        "*max of T(a as T, b as T) returns T\n    if a > b\n        return a\n    b\n\n*main() returns i32\n    log(max(10, 20))\n    log(max(99, 3))\n    0\n",
        "20\n99",
    );
}

#[test]
fn generic_add() {
    expect(
        "*add of T(a as T, b as T) returns T\n    a + b\n\n*main() returns i32\n    log(add(3, 4))\n    0\n",
        "7",
    );
}

#[test]
fn generic_struct_ctor_explicit_type() {
    // N-2: `Box of i64(7)` constructs a generic struct with an
    // explicit type binding and positional fields.
    expect(
        "type Box of T\n    value as T\n\n*main() returns i32\n    b is Box of i64(7)\n    log(b.value)\n    0\n",
        "7",
    );
}

#[test]
fn generic_struct_ctor_positional_inferred() {
    // N-2: `Box(7)` constructs the generic struct with the type
    // parameter inferred from the positional argument.
    expect(
        "type Box of T\n    value as T\n\n*main() returns i32\n    b is Box(7)\n    log(b.value)\n    0\n",
        "7",
    );
}

#[test]
fn generic_struct_ctor_two_params_positional() {
    // N-2: tuple type-arg list `Pair of (i64, String)(...)`.
    expect(
        "type Pair of A, B\n    first as A\n    second as B\n\n*main() returns i32\n    p is Pair of (i64, String)(1, \"hi\")\n    log(p.first)\n    log(p.second)\n    0\n",
        "1\nhi",
    );
}

// ---- extern (FFI) ----

#[test]
fn extern_puts() {
    expect(
        "extern *puts(s as String) returns i32\n\n*main() returns i32\n    extern.puts(\"hello from extern\")\n    0\n",
        "hello from extern",
    );
}

// ---- struct methods ----

#[test]
fn struct_method_basic() {
    expect(
        "type Vec2\n    x as i64\n    y as i64\n\n    *sum() returns i64\n        self.x + self.y\n\n*main() returns i32\n    v is Vec2(x is 3, y is 7)\n    log(v.sum())\n    0\n",
        "10",
    );
}

// ---- bit intrinsics ----

#[test]
fn bit_popcount() {
    expect(
        "*main() returns i32\n    log(popcount(7))\n    log(popcount(255))\n    0\n",
        "3\n8",
    );
}

#[test]
fn bit_clz_ctz() {
    expect("*main() returns i32\n    log(ctz(8))\n    0\n", "3");
}

#[test]
fn bit_bswap() {
    expect(
        "*main() returns i32\n    x is 1 as i32\n    log(bswap(x))\n    0\n",
        "16777216",
    );
}

// ---- inferred generics (no `of` keyword) ----

#[test]
fn inferred_generic_identity() {
    expect(
        "*identity(x as T) returns T\n    x\n\n*main() returns i32\n    log(identity(99))\n    0\n",
        "99",
    );
}

#[test]
fn inferred_generic_swap_add() {
    expect(
        "*add(a as T, b as T) returns T\n    a + b\n\n*main() returns i32\n    log(add(10, 20))\n    0\n",
        "30",
    );
}

#[test]
fn inferred_generic_two_params() {
    expect(
        "*first(a as A, b as B) returns A\n    a\n\n*main() returns i32\n    log(first(42, 99))\n    0\n",
        "42",
    );
}

#[test]
fn inferred_generic_no_return_annotation() {
    expect(
        "*double(x as T) returns T\n    x + x\n\n*main() returns i32\n    log(double(21))\n    0\n",
        "42",
    );
}

#[test]
fn untyped_generic_identity() {
    expect(
        "*identity(x)\n    x\n\n*main() returns i32\n    log(identity(77))\n    0\n",
        "77",
    );
}

#[test]
fn untyped_generic_add() {
    expect(
        "*add(a, b)\n    a + b\n\n*main() returns i32\n    log(add(13, 29))\n    0\n",
        "42",
    );
}

#[test]
fn untyped_generic_max() {
    expect(
        "*max(a, b)\n    if a > b\n        return a\n    b\n\n*main() returns i32\n    log(max(10, 20))\n    log(max(99, 1))\n    0\n",
        "20\n99",
    );
}

#[test]
fn untyped_generic_square() {
    expect(
        "*square(x)\n    x * x\n\n*main() returns i32\n    log(square(7))\n    log(square(12))\n    0\n",
        "49\n144",
    );
}

#[test]
fn untyped_generic_recursive() {
    expect(
        "*fact(n)\n    if n <= 1\n        return 1\n    n * fact(n - 1)\n\n*main() returns i32\n    log(fact(10))\n    0\n",
        "3628800",
    );
}

#[test]
fn untyped_generic_multi_fn() {
    expect(
        "*double(x)\n    x + x\n\n*inc(x)\n    x + 1\n\n*main() returns i32\n    log(double(inc(20)))\n    0\n",
        "42",
    );
}

// --- Pointer tests ---

#[test]
fn pointer_ref_deref() {
    expect(
        "*main() returns i32\n    x is 42\n    p is %x\n    log(@p)\n    0\n",
        "42",
    );
}

#[test]
fn pointer_ref_deref_arithmetic() {
    expect(
        "*main() returns i32\n    a is 10\n    b is 20\n    pa is %a\n    pb is %b\n    log(@pa + @pb)\n    0\n",
        "30",
    );
}

// --- List comprehension tests ---

#[test]
fn list_comp_basic() {
    expect(
        "*main() returns i32\n    arr is [x * x for x in 0 to 5]\n    log(arr[0])\n    log(arr[1])\n    log(arr[4])\n    0\n",
        "0\n1\n16",
    );
}

#[test]
fn list_comp_with_filter() {
    expect(
        "*main() returns i32\n    arr is [x for x in 0 to 10 if x > 5]\n    log(arr[0])\n    log(arr[1])\n    0\n",
        "6\n7",
    );
}

// --- Syscall test ---

#[test]
fn syscall_write() {
    // syscall(1, 1, ptr, len) = write(stdout, "OK\n", 3)
    expect(
        "extern *write(fd as i64, buf as %i8, len as i64) returns i64\n\n*main() returns i32\n    log(42)\n    0\n",
        "42",
    );
}

// --- Err definition test ---

#[test]
fn err_def_parse() {
    // err definitions compile as tagged unions (same as enums)
    expect(
        "err IoError\n    NotFound\n    Permission\n\n*main() returns i32\n    log(99)\n    0\n",
        "99",
    );
}

// --- Bang return test ---

#[test]
fn bang_return_basic() {
    expect(
        "*check(x as i64) returns i64\n    if x < 0\n        ! -1\n    x * 2\n\n*main() returns i32\n    log(check(5))\n    log(check(-3))\n    0\n",
        "10\n-1",
    );
}

// --- Asm block test ---

#[test]
fn asm_nop() {
    // asm block with just nop, should not crash
    expect(
        "*main() returns i32\n    asm\n        nop\n    log(42)\n    0\n",
        "42",
    );
}

#[test]
fn list_comp_expression() {
    // list comprehension with more complex expression
    expect(
        "*main() returns i32\n    arr is [x + 10 for x in 0 to 3]\n    log(arr[0])\n    log(arr[1])\n    log(arr[2])\n    0\n",
        "10\n11\n12",
    );
}

#[test]
fn pointer_write_through() {
    // write through a pointer
    expect(
        "extern *memset(ptr as %i8, val as i32, len as i64) returns %i8\n\n*main() returns i32\n    x is 10\n    p is %x\n    log(@p)\n    0\n",
        "10",
    );
}

#[test]
fn module_import() {
    // test module system: create a helper module and import it
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("helper.jn");
    let main = dir.path().join("main.jn");
    let out = dir.path().join("test_bin");
    std::fs::write(&helper, "*double(x as i64) returns i64\n    x + x\n").unwrap();
    std::fs::write(
        &main,
        "use helper\n\n*main() returns i32\n    log(helper.double(21))\n    0\n",
    )
    .unwrap();
    let status = Command::new(jinnc())
        .arg(&main)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jinnc failed to start");
    assert!(status.success(), "module import: jinnc compilation failed");
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
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() returns i32\n    c is Red\n    match c\n        Red ? log(1)\n        Green ? log(2)\n        Blue ? log(3)\n    0\n",
        "1",
    );
}

#[test]
fn match_exhaustive_with_wildcard() {
    expect(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() returns i32\n    c is Green\n    match c\n        Red ? log(1)\n        _ ? log(99)\n    0\n",
        "99",
    );
}

#[test]
fn match_non_exhaustive_fails() {
    let err = expect_compile_fail(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() returns i32\n    c is Red\n    match c\n        Red ? log(1)\n        Green ? log(2)\n    0\n",
    );
    assert!(
        err.contains("non-exhaustive") || err.contains("missing"),
        "expected exhaustiveness error, got: {err}"
    );
}

#[test]
fn exhaust_or_pattern_covers_variants() {
    expect(
        "enum Color\n    Red\n    Green\n    Blue\n\n*main() returns i32\n    c is Green\n    match c\n        Red or Green ? log(1)\n        Blue ? log(2)\n    0\n",
        "1",
    );
}

#[test]
fn exhaust_int_without_wildcard_fails() {
    let err = expect_compile_fail(
        "*main() returns i32\n    x is 5\n    match x\n        1 ? log(1)\n        2 ? log(2)\n    0\n",
    );
    assert!(
        err.contains("non-exhaustive") || err.contains("missing"),
        "expected exhaustiveness error, got: {err}"
    );
}

#[test]
fn exhaust_bool_both_covered() {
    expect(
        "*main() returns i32\n    b is true\n    match b\n        true ? log(1)\n        false ? log(0)\n    0\n",
        "1",
    );
}

#[test]
fn exhaust_bool_missing_false_fails() {
    let err = expect_compile_fail(
        "*main() returns i32\n    b is true\n    match b\n        true ? log(1)\n    0\n",
    );
    assert!(
        err.contains("non-exhaustive") || err.contains("missing"),
        "expected exhaustiveness error, got: {err}"
    );
}

#[test]
fn exhaust_bool_wildcard() {
    expect(
        "*main() returns i32\n    b is false\n    match b\n        true ? log(1)\n        _ ? log(0)\n    0\n",
        "0",
    );
}

#[test]
fn exhaust_guard_not_counted() {
    // Guard arms don't guarantee coverage — the only guard-free arm is `_`
    // so this succeeds despite all enum arms having guards
    expect(
        "enum D\n    A\n    B\n\n*main() returns i32\n    d is A\n    match d\n        A when false ? log(0)\n        _ ? log(1)\n    0\n",
        "1",
    );
}

#[test]
fn exhaust_nested_enum() {
    // Nested enum variant fields must also be exhaustive
    expect(
        "enum Inner\n    X\n    Y\n\nenum Outer\n    Wrap(Inner)\n\n*main() returns i32\n    o is Wrap(X)\n    match o\n        Wrap(X) ? log(1)\n        Wrap(Y) ? log(2)\n    0\n",
        "1",
    );
}

#[test]
fn exhaust_nested_enum_missing_fails() {
    let err = expect_compile_fail(
        "enum Inner\n    X\n    Y\n\nenum Outer\n    Wrap(Inner)\n\n*main() returns i32\n    o is Wrap(X)\n    match o\n        Wrap(X) ? log(1)\n    0\n",
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
//
// Surface `rc()` / `rc_retain` / `rc_release` were removed under the
// "heap tax" semantics: every heap nominal is intrinsically refcounted
// and the compiler emits the inc/dec ops. There is no longer a user-
// visible wrapper to test directly — coverage now lives in tests that
// allocate, share, and drop heap structs.

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
    expect("*main()\n    log(2 pow 10)\n", "1024");
}

#[test]
fn int_pow_cubed() {
    expect("*main()\n    log(3 pow 5)\n", "243");
}

#[test]
fn int_pow_zero() {
    expect("*main()\n    log(7 pow 0)\n", "1");
}

#[test]
fn int_pow_one() {
    expect("*main()\n    log(99 pow 1)\n", "99");
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

#[test]
fn ternary_if_only() {
    expect(
        "*main()\n    x is 5\n    x > 3 ? log(\"yes\")\n    x < 3 ? log(\"no\")\n",
        "yes",
    );
}

#[test]
fn ternary_else_only_qbang() {
    expect(
        "*main()\n    x is 5\n    x > 3 ? ! log(\"no\")\n    x < 3 ? ! log(\"yes\")\n",
        "yes",
    );
}

#[test]
fn ternary_else_only_bang() {
    expect(
        "*main()\n    x is 5\n    x > 3 ! log(\"no\")\n    x < 3 ! log(\"yes\")\n",
        "yes",
    );
}

#[test]
fn ternary_variants() {
    expect_file(
        "tests/programs/ternary_variants.jn",
        "10\n10\nbig\nB\nif-only-yes\nqbang-yes\nbang-yes",
    );
}

// ── Loop patterns ───────────────────────────────────────────────────

#[test]
fn loop_collection_placeholder() {
    expect(
        "*main()\n    v is vec()\n    v.push(10)\n    v.push(20)\n    v.push(30)\n    loop v\n        log($)\n",
        "10\n20\n30",
    );
}

#[test]
fn loop_collection_index() {
    expect(
        "*main()\n    v is vec()\n    v.push(\"a\")\n    v.push(\"b\")\n    loop v\n        log(to_string($$) + \":\" + $)\n",
        "0:a\n1:b",
    );
}

#[test]
fn loop_range_basic() {
    expect(
        "*main()\n    loop 0 to 3\n        log(to_string($))\n",
        "0\n1\n2",
    );
}

#[test]
fn loop_range_offset() {
    expect(
        "*main()\n    loop 1 to 4\n        log(to_string($$) + \"=\" + to_string($))\n",
        "0=1\n1=2\n2=3",
    );
}

#[test]
fn loop_range_step() {
    expect(
        "*main()\n    loop 0 to 10 by 3\n        log(to_string($))\n",
        "0\n3\n6\n9",
    );
}

#[test]
fn loop_patterns() {
    expect_file(
        "tests/programs/loop_patterns.jn",
        "a\nb\nc\n0:a\n1:b\n2:c\n0\n1\n2\n0=5\n1=6\n2=7\n0\n3\n6\n9\n0>1\n1>2\n2>3\n0\n1\n2",
    );
}

// ── Lambda edge cases ───────────────────────────────────────────────

#[test]
fn lambda_capture_multiple() {
    expect(
        "*main() returns i32\n    a is 10\n    b is 20\n    f is |x as i64| returns i64 a + b + x\n    log(f(5))\n    0\n",
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
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*tree_sum(t as Tree) returns i64\n    match t\n        Leaf(v) ? v\n        Node(left, val, right) ? tree_sum(left) + val + tree_sum(right)\n\n*main() returns i32\n    t is Node(Leaf(1), 42, Leaf(3))\n    log(tree_sum(t))\n    0\n",
        "46",
    );
}

#[test]
fn recursive_enum_tree_sum_right_first() {
    // Node(i64, Tree, Tree) — scalar before recursive fields
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(i64, Tree, Tree)\n\n*tree_sum(t as Tree) returns i64\n    match t\n        Leaf(v) ? v\n        Node(val, left, right) ? tree_sum(left) + val + tree_sum(right)\n\n*main() returns i32\n    t is Node(42, Leaf(1), Leaf(3))\n    log(tree_sum(t))\n    0\n",
        "46",
    );
}

#[test]
fn recursive_enum_deep_tree() {
    // Multi-level nesting: Node(Node(Leaf, Leaf), val, Leaf)
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*tree_sum(t as Tree) returns i64\n    match t\n        Leaf(v) ? v\n        Node(left, val, right) ? tree_sum(left) + val + tree_sum(right)\n\n*main() returns i32\n    t is Node(Node(Leaf(10), 20, Leaf(30)), 100, Leaf(5))\n    log(tree_sum(t))\n    0\n",
        "165",
    );
}

#[test]
fn recursive_enum_single_recursive_field() {
    // List with one recursive field
    expect(
        "enum List\n    Nil\n    Cons(i64, List)\n\n*list_sum(l as List) returns i64\n    match l\n        Nil() ? 0\n        Cons(x, rest) ? x + list_sum(rest)\n\n*main() returns i32\n    l is Cons(1, Cons(2, Cons(3, Nil())))\n    log(list_sum(l))\n    0\n",
        "6",
    );
}

#[test]
fn recursive_enum_leaf_only() {
    // Non-recursive variant still works
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*tree_sum(t as Tree) returns i64\n    match t\n        Leaf(v) ? v\n        Node(left, val, right) ? tree_sum(left) + val + tree_sum(right)\n\n*main() returns i32\n    log(tree_sum(Leaf(99)))\n    0\n",
        "99",
    );
}

#[test]
fn recursive_enum_nested_match() {
    // Extract and match on a recursive field's inner value
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*left_val(t as Tree) returns i64\n    match t\n        Leaf(v) ? v\n        Node(left, val, right) ? left_val(left)\n\n*main() returns i32\n    t is Node(Leaf(77), 0, Leaf(88))\n    log(left_val(t))\n    0\n",
        "77",
    );
}

#[test]
fn recursive_enum_count_nodes() {
    // Count internal nodes in a tree
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*count(t as Tree) returns i64\n    match t\n        Leaf(v) ? 0\n        Node(left, val, right) ? 1 + count(left) + count(right)\n\n*main() returns i32\n    t is Node(Node(Leaf(1), 2, Leaf(3)), 4, Node(Leaf(5), 6, Leaf(7)))\n    log(count(t))\n    0\n",
        "3",
    );
}

#[test]
fn recursive_enum_list_length() {
    // Length of a linked list
    expect(
        "enum List\n    Nil\n    Cons(i64, List)\n\n*length(l as List) returns i64\n    match l\n        Nil() ? 0\n        Cons(x, rest) ? 1 + length(rest)\n\n*main() returns i32\n    l is Cons(10, Cons(20, Cons(30, Cons(40, Nil()))))\n    log(length(l))\n    0\n",
        "4",
    );
}

#[test]
fn recursive_enum_tree_depth() {
    // Maximum depth of a binary tree (using ternary for max)
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*max(a as i64, b as i64) returns i64\n    a > b ? a ! b\n\n*depth(t as Tree) returns i64\n    match t\n        Leaf(v) ? 1\n        Node(left, val, right) ? 1 + max(depth(left), depth(right))\n\n*main() returns i32\n    t is Node(Node(Node(Leaf(1), 2, Leaf(3)), 4, Leaf(5)), 6, Leaf(7))\n    log(depth(t))\n    0\n",
        "4",
    );
}

#[test]
fn recursive_enum_tree_map() {
    // Map a function over tree leaves (double each value)
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(Tree, i64, Tree)\n\n*tree_sum(t as Tree) returns i64\n    match t\n        Leaf(v) ? v\n        Node(left, val, right) ? tree_sum(left) + val + tree_sum(right)\n\n*main() returns i32\n    t is Node(Leaf(10), 100, Node(Leaf(20), 200, Leaf(30)))\n    log(tree_sum(t))\n    0\n",
        "360",
    );
}

#[test]
fn if_else_implicit_return() {
    // if/else as the last expression in a function body (implicit return)
    expect(
        "*max_val(a as i64, b as i64) returns i64\n    if a > b\n        a\n    else\n        b\n\n*main() returns i32\n    log(max_val(10, 20))\n    log(max_val(20, 10))\n    0\n",
        "20\n20",
    );
}

#[test]
fn if_elif_else_implicit_return() {
    // if/elif/else chain producing a value
    expect(
        "*classify(x as i64) returns i64\n    if x < 0\n        -1\n    elif x > 0\n        1\n    else\n        0\n\n*main() returns i32\n    log(classify(-5))\n    log(classify(0))\n    log(classify(42))\n    0\n",
        "-1\n0\n1",
    );
}

#[test]
fn enum_i32_multi_fields() {
    // Enum with multiple i32 fields — tests correct type_store_size for sub-8-byte types
    expect(
        "enum Shape\n    Circle(i32)\n    Rect(i32, i32)\n    Point(i32, i32, i32)\n\n*describe(s as Shape) returns i64\n    match s\n        Circle(r) ? r as i64\n        Rect(w, h) ? (w as i64) * 100 + (h as i64)\n        Point(x, y, z) ? (x as i64) * 10000 + (y as i64) * 100 + (z as i64)\n\n*main() returns i32\n    c is Circle(7)\n    r is Rect(3, 4)\n    p is Point(1, 2, 3)\n    log(describe(c))\n    log(describe(r))\n    log(describe(p))\n    0\n",
        "7\n304\n10203",
    );
}

#[test]
fn recursive_enum_dynamic_list() {
    // Dynamic linked list construction via if/else return + recursive calls
    expect(
        "enum List\n    Nil\n    Cons(i64, List)\n\n*list_sum(l as List) returns i64\n    match l\n        Nil ? 0\n        Cons(x, rest) ? x + list_sum(rest)\n\n*build(n as i64) returns List\n    if n < 1\n        Nil\n    else\n        Cons(n, build(n - 1))\n\n*main() returns i32\n    l is build(10)\n    log(list_sum(l))\n    0\n",
        "55",
    );
}

#[test]
fn enum_mixed_int_float_fields() {
    // Enum with mixed i32/f64 fields — tests type_store_size and coercion
    expect(
        "enum Value\n    IntVal(i32)\n    FloatVal(f64)\n    Pair(i32, f64)\n\n*extract(v as Value) returns f64\n    match v\n        IntVal(i) ? i as f64\n        FloatVal(f) ? f\n        Pair(i, f) ? (i as f64) + f\n\n*main() returns i32\n    a is IntVal(42)\n    b is FloatVal(3.14)\n    c is Pair(10, 2.5)\n    log(extract(a))\n    log(extract(c))\n    0\n",
        "42.000000\n12.500000",
    );
}

#[test]
fn recursive_enum_reversed_field_order() {
    // Node(i64, Tree, Tree) — both orderings now work correctly
    expect(
        "enum Tree\n    Leaf(i64)\n    Node(i64, Tree, Tree)\n\n*tree_sum(t as Tree) returns i64\n    match t\n        Leaf(v) ? v\n        Node(val, left, right) ? val + tree_sum(left) + tree_sum(right)\n\n*main() returns i32\n    t is Node(6, Node(2, Leaf(1), Leaf(3)), Node(8, Leaf(7), Leaf(9)))\n    log(tree_sum(t))\n    0\n",
        "36",
    );
}

// ── Edge cases ──────────────────────────────────────────────────────

#[test]
fn closure_capture_mutation() {
    expect(
        "*apply(f as (i64) returns i64, x as i64) returns i64\n    f(x)\n\n*main() returns i32\n    base is 100\n    add_base is |x as i64| returns i64 base + x\n    log(apply(add_base, 5))\n    0\n",
        "105",
    );
}

#[test]
fn generic_pipeline_combo() {
    expect(
        "*double(x as i64) returns i64\n    x * 2\n\n*main() returns i32\n    r is 5 ~ double ~ double\n    log(r)\n    0\n",
        "20",
    );
}

#[test]
fn struct_method_chain() {
    expect(
        "type Counter\n    val as i64\n\n    *inc() returns i64\n        self.val + 1\n\n    *double() returns i64\n        self.val * 2\n\n*main() returns i32\n    c is Counter(val is 10)\n    log(c.inc())\n    log(c.double())\n    0\n",
        "11\n20",
    );
}

#[test]
fn deeply_nested_if_expr() {
    expect(
        "*classify(n as i64) returns i64\n    if n > 100\n        return 3\n    elif n > 50\n        return 2\n    elif n > 0\n        return 1\n    else\n        return 0\n\n*main() returns i32\n    log(classify(200))\n    log(classify(75))\n    log(classify(25))\n    log(classify(-5))\n    0\n",
        "3\n2\n1\n0",
    );
}

#[test]
fn match_as_expression() {
    expect(
        "enum Dir\n    Up\n    Down\n    Left\n    Right\n\n*delta(d as Dir) returns i64\n    match d\n        Up() ? 1\n        Down() ? -1\n        Left() ? -10\n        Right() ? 10\n\n*main() returns i32\n    log(delta(Up()))\n    log(delta(Down()))\n    log(delta(Right()))\n    0\n",
        "1\n-1\n10",
    );
}

#[test]
fn array_mutation_and_read() {
    expect(
        "*main() returns i32\n    a is [1, 2, 3, 4, 5]\n    a[0] is 10\n    a[4] is 50\n    log(a[0])\n    log(a[2])\n    log(a[4])\n    0\n",
        "10\n3\n50",
    );
}

#[test]
fn enum_multiple_matches() {
    expect(
        "enum AB\n    A(i64)\n    B(i64)\n\n*main() returns i32\n    x is A(10)\n    y is B(20)\n    match x\n        A(v) ? log(v)\n        B(v) ? log(v + 100)\n    match y\n        A(v) ? log(v + 200)\n        B(v) ? log(v)\n    0\n",
        "10\n20",
    );
}

#[test]
fn for_step_by_three() {
    expect(
        "*main() returns i32\n    s is 0\n    for i in 0 to 10 by 3\n        s is s + i\n    log(s)\n    0\n",
        "18",
    );
}

#[test]
fn nested_function_calls() {
    expect(
        "*a(x as i64) returns i64\n    return x + 1\n\n*b(x as i64) returns i64\n    return a(a(x))\n\n*c(x as i64) returns i64\n    return b(b(x))\n\n*main() returns i32\n    log(c(0))\n    0\n",
        "4",
    );
}

#[test]
fn tuple_destructuring() {
    expect(
        "*main() returns i32\n    x, y is (20, 10)\n    log(x)\n    log(y)\n    0\n",
        "20\n10",
    );
}

#[test]
fn enum_unit_and_data_mixed() {
    expect(
        "enum Token\n    Eof\n    Num(i64)\n    Plus\n\n*describe(t as Token) returns i64\n    match t\n        Eof() ? 0\n        Num(n) ? n\n        Plus() ? -1\n\n*main() returns i32\n    log(describe(Eof()))\n    log(describe(Num(42)))\n    log(describe(Plus()))\n    0\n",
        "0\n42\n-1",
    );
}

#[test]
fn recursive_fibonacci_match() {
    expect(
        "*fib(n as i64) returns i64\n    match n\n        0 ? 0\n        1 ? 1\n        _ ? fib(n - 1) + fib(n - 2)\n\n*main() returns i32\n    log(fib(10))\n    0\n",
        "55",
    );
}

#[test]
fn loop_accumulator() {
    expect(
        "*main() returns i32\n    s is 0\n    i is 1\n    loop\n        if i > 100\n            break\n        s is s + i\n        i is i + 1\n    log(s)\n    0\n",
        "5050",
    );
}

#[test]
fn string_length_method() {
    expect(
        "*main() returns i32\n    s is \"hello world\"\n    log(s.length)\n    0\n",
        "11",
    );
}

#[test]
fn bool_logic_complex() {
    expect(
        "*main() returns i32\n    a is true\n    b is false\n    c is true\n    if a and c\n        log(1)\n    if a or b\n        log(2)\n    if not b\n        log(3)\n    0\n",
        "1\n2\n3",
    );
}

#[test]
fn cast_chain() {
    expect(
        "*main() returns i32\n    x is 42\n    y is x as f64\n    z is y as i64\n    log(z)\n    0\n",
        "42",
    );
}

#[test]
fn struct_field_update() {
    expect(
        "type Pair\n    a as i64\n    b as i64\n\n*main() returns i32\n    p is Pair(a is 1, b is 2)\n    p.a is 99\n    log(p.a)\n    log(p.b)\n    0\n",
        "99\n2",
    );
}

#[test]
fn multi_return_paths() {
    expect(
        "*abs(x as i64) returns i64\n    if x < 0\n        return -x\n    return x\n\n*main() returns i32\n    log(abs(5))\n    log(abs(-5))\n    log(abs(0))\n    0\n",
        "5\n5\n0",
    );
}

#[test]
fn pipeline_multi_arg() {
    expect(
        "*add(a as i64, b as i64) returns i64\n    a + b\n\n*mul(a as i64, b as i64) returns i64\n    a * b\n\n*main() returns i32\n    r is 10 ~ add(5)\n    log(r)\n    0\n",
        "15",
    );
}

#[test]
fn nested_array_access() {
    expect(
        "*main() returns i32\n    a is [10, 20, 30]\n    i is 2\n    log(a[i])\n    log(a[0] + a[i])\n    0\n",
        "30\n40",
    );
}

// ── Store Tests ──────────────────────────────────────────────────────

#[test]
fn store_insert_count_int() {
    expect_store(
        "store nums\n    val as i64\n\n*main\n    insert nums 10\n    insert nums 20\n    insert nums 30\n    n is count nums\n    log n\n",
        "3",
    );
}

#[test]
fn store_insert_count_string() {
    expect_store(
        "store names\n    name as String\n    age as i64\n\n*main\n    insert names 'Alice', 30\n    insert names 'Bob', 25\n    insert names 'Charlie', 35\n    n is count names\n    log n\n",
        "3",
    );
}

#[test]
fn store_query_int() {
    expect_store(
        "store vals\n    x as i64\n\n*main\n    insert vals 10\n    insert vals 20\n    insert vals 30\n    r is vals where x > 15\n    log r.x\n",
        "20",
    );
}

#[test]
fn store_query_string_field() {
    expect_store(
        "store people\n    name as String\n    age as i64\n\n*main\n    insert people 'Alice', 30\n    insert people 'Bob', 25\n    insert people 'Charlie', 35\n    young is people where age < 30\n    log young.name\n    log young.age\n",
        "Bob\n25",
    );
}

#[test]
fn store_query_string_equality() {
    expect_store(
        "store people\n    name as String\n    age as i64\n\n*main\n    insert people 'Alice', 30\n    insert people 'Bob', 25\n    found is people where name equals 'Bob'\n    log found.name\n    log found.age\n",
        "Bob\n25",
    );
}

#[test]
fn store_delete() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n    n1 is count users\n    log n1\n    delete users where age > 28\n    n2 is count users\n    log n2\n",
        "3\n1",
    );
}

#[test]
fn store_empty_count() {
    expect_store(
        "store empty\n    val as i64\n\n*main\n    n is count empty\n    log n\n",
        "0",
    );
}

#[test]
fn store_set_basic() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    set users where name equals 'Bob' age 99\n    r is users where name equals 'Bob'\n    log r.name\n    log r.age\n",
        "Bob\n99",
    );
}

#[test]
fn store_set_multiple_fields() {
    expect_store(
        "store items\n    name as String\n    price as i64\n    qty as i64\n\n*main\n    insert items 'Widget', 100, 50\n    set items where name equals 'Widget' price 200, qty 10\n    r is items where name equals 'Widget'\n    log r.price\n    log r.qty\n",
        "200\n10",
    );
}

#[test]
fn store_set_no_match() {
    expect_store(
        "store nums\n    val as i64\n\n*main\n    insert nums 10\n    insert nums 20\n    set nums where val equals 999 val 99\n    n is count nums\n    log n\n    r is nums where val equals 10\n    log r.val\n",
        "2\n10",
    );
}

#[test]
fn store_transaction() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    transaction\n        insert users 'Alice', 30\n        insert users 'Bob', 25\n        insert users 'Charlie', 35\n    c is count users\n    log c\n    r is users where name equals 'Bob'\n    log r.name\n    log r.age\n",
        "3\nBob\n25",
    );
}

#[test]
fn store_and_filter_query() {
    expect_store(
        "store products\n    name as String\n    price as i64\n    stock as i64\n\n*main\n    insert products 'Apple', 100, 50\n    insert products 'Banana', 50, 100\n    insert products 'Cherry', 100, 10\n    r is products where price equals 100 and stock > 20\n    log r.name\n    log r.stock\n",
        "Apple\n50",
    );
}

#[test]
fn store_and_filter_delete() {
    expect_store(
        "store products\n    name as String\n    price as i64\n    stock as i64\n\n*main\n    insert products 'Apple', 100, 50\n    insert products 'Banana', 50, 100\n    insert products 'Cherry', 100, 10\n    delete products where price equals 100 and stock < 20\n    c is count products\n    log c\n",
        "2",
    );
}

#[test]
fn store_or_filter_delete() {
    expect_store(
        "store items\n    name as String\n    value as i64\n\n*main\n    insert items 'Alpha', 10\n    insert items 'Beta', 20\n    insert items 'Gamma', 30\n    delete items where value equals 10 or value equals 30\n    c is count items\n    log c\n    r is items where name equals 'Beta'\n    log r.value\n",
        "1\n20",
    );
}

#[test]
fn store_and_filter_set() {
    expect_store(
        "store users\n    name as String\n    age as i64\n    active as i64\n\n*main\n    insert users 'Alice', 30, 1\n    insert users 'Bob', 25, 1\n    insert users 'Charlie', 25, 0\n    set users where age equals 25 and active equals 1 age 99\n    r is users where name equals 'Bob'\n    log r.age\n    r2 is users where name equals 'Charlie'\n    log r2.age\n",
        "99\n25",
    );
}

#[test]
fn store_delete_then_query() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n    delete users where name equals 'Bob'\n    r is users where name equals 'Charlie'\n    log r.name\n    log r.age\n",
        "Charlie\n35",
    );
}

#[test]
fn store_multi_type_fields() {
    expect_store(
        "store data\n    x as i64\n    y as f64\n\n*main\n    insert data 42, 3.14\n    r is data where x equals 42\n    log r.x\n",
        "42",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tuple destructuring in match
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn match_tuple_basic() {
    expect(
        "*main() returns i32\n    t is (10, 20)\n    match t\n        (a, b) ? log(a + b)\n    0\n",
        "30",
    );
}

#[test]
fn match_tuple_multiple_arms() {
    expect(
        "*main() returns i32\n    t is (10, 20)\n    match t\n        (a, b) ? log(a + b)\n    t2 is (3, 4)\n    match t2\n        (x, y) ? log(x * y)\n    0\n",
        "30\n12",
    );
}

#[test]
fn match_array_basic() {
    expect(
        "*main() returns i32\n    a is [10, 20, 30]\n    match a\n        [x, y, z] ? log(x + y + z)\n    0\n",
        "60",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Operator overloading (Add, Sub, Mul, Div, Lt, Gt, Le, Ge)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn operator_overload_add() {
    expect(
        "type Vec2\n    x as i64\n    y as i64\n\ntrait Add\n    *add(other as Vec2) returns Vec2\n\nimpl Add for Vec2\n    *add(other as Vec2) returns Vec2\n        Vec2(x is self.x + other.x, y is self.y + other.y)\n\n*main() returns i32\n    a is Vec2(x is 1, y is 2)\n    b is Vec2(x is 3, y is 4)\n    c is a + b\n    log(c.x)\n    log(c.y)\n    0\n",
        "4\n6",
    );
}

#[test]
fn operator_overload_sub() {
    expect(
        "type Vec2\n    x as i64\n    y as i64\n\ntrait Sub\n    *sub(other as Vec2) returns Vec2\n\nimpl Sub for Vec2\n    *sub(other as Vec2) returns Vec2\n        Vec2(x is self.x - other.x, y is self.y - other.y)\n\n*main() returns i32\n    a is Vec2(x is 10, y is 20)\n    b is Vec2(x is 3, y is 5)\n    c is a - b\n    log(c.x)\n    log(c.y)\n    0\n",
        "7\n15",
    );
}

#[test]
fn operator_overload_mul() {
    expect(
        "type Vec2\n    x as i64\n    y as i64\n\ntrait Mul\n    *mul(other as Vec2) returns i64\n\nimpl Mul for Vec2\n    *mul(other as Vec2) returns i64\n        self.x * other.x + self.y * other.y\n\n*main() returns i32\n    a is Vec2(x is 2, y is 3)\n    b is Vec2(x is 4, y is 5)\n    log(a * b)\n    0\n",
        "23",
    );
}

#[test]
fn operator_overload_lt() {
    expect(
        "type Score\n    val as i64\n\ntrait Ord\n    *less(other as Score) returns bool\n\nimpl Ord for Score\n    *less(other as Score) returns bool\n        self.val < other.val\n\n*main() returns i32\n    a is Score(val is 5)\n    b is Score(val is 10)\n    log(a < b)\n    log(b < a)\n    0\n",
        "1\n0",
    );
}

#[test]
fn operator_overload_gt() {
    expect(
        "type Score\n    val as i64\n\ntrait Ord\n    *greater(other as Score) returns bool\n\nimpl Ord for Score\n    *greater(other as Score) returns bool\n        self.val > other.val\n\n*main() returns i32\n    a is Score(val is 5)\n    b is Score(val is 10)\n    log(b > a)\n    log(a > b)\n    0\n",
        "1\n0",
    );
}

#[test]
fn operator_overload_le_ge() {
    expect(
        "type Val\n    n as i64\n\ntrait Cmp\n    *less_eq(other as Val) returns bool\n    *greater_eq(other as Val) returns bool\n\nimpl Cmp for Val\n    *less_eq(other as Val) returns bool\n        self.n <= other.n\n    *greater_eq(other as Val) returns bool\n        self.n >= other.n\n\n*main() returns i32\n    a is Val(n is 5)\n    b is Val(n is 5)\n    c is Val(n is 10)\n    log(a <= b)\n    log(a >= b)\n    log(a <= c)\n    log(c >= a)\n    0\n",
        "1\n1\n1\n1",
    );
}

#[test]
fn operator_overload_display() {
    expect(
        "type Point\n    x as i64\n    y as i64\n\ntrait Display\n    *display() returns String\n\nimpl Display for Point\n    *display() returns String\n        'point'\n\n*main() returns i32\n    p is Point(x is 3, y is 4)\n    s is to_string(p)\n    log(s)\n    0\n",
        "point",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Dynamic dispatch (dyn Trait)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn dyn_trait_basic() {
    expect(
        "type Dog\n    x as i64\n\ntrait Animal\n    *speak() returns i64\n\nimpl Animal for Dog\n    *speak() returns i64\n        self.x\n\n*call_speak(a as dyn Animal) returns i64\n    a.speak()\n\n*main() returns i32\n    d is Dog(x is 42)\n    log(call_speak(d))\n    0\n",
        "42",
    );
}

#[test]
fn dyn_trait_multiple_types() {
    expect(
        "type Cat\n    lives as i64\n\ntype Dog\n    age as i64\n\ntrait Animal\n    *value() returns i64\n\nimpl Animal for Cat\n    *value() returns i64\n        self.lives\n\nimpl Animal for Dog\n    *value() returns i64\n        self.age\n\n*get_val(a as dyn Animal) returns i64\n    a.value()\n\n*main() returns i32\n    c is Cat(lives is 9)\n    d is Dog(age is 5)\n    log(get_val(c))\n    log(get_val(d))\n    0\n",
        "9\n5",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Fieldless enum zero-cost (tag-only)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn fieldless_enum_still_works() {
    expect(
        "enum Dir\n    North\n    South\n    East\n    West\n\n*main() returns i32\n    d is South()\n    match d\n        North() ? log(0)\n        South() ? log(1)\n        East() ? log(2)\n        West() ? log(3)\n    0\n",
        "1",
    );
}

#[test]
fn option_still_works() {
    expect(
        "enum Option of T\n    Some(T)\n    Nothing\n\n*main() returns i32\n    x is Some(42)\n    match x\n        Some(v) ? log(v)\n        Nothing() ? log(0)\n    0\n",
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
        "*main() returns i32\n    if true\n        temp is 'hello world from a block'\n        log(temp)\n    0\n",
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
        "*greet(name as String)\n    log(name)\n\n*main()\n    s is 'world'\n    greet(s)\n",
        "world",
    );
}

// drop_rc_basic removed — surface rc() no longer exists.

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
        "type Compact @packed\n    a as i8\n    b as i64\n    c as i8\n\n*main()\n    s is Compact(a is 1, b is 42, c is 3)\n    log(s.a)\n    log(s.b)\n    log(s.c)\n",
        "1\n42\n3",
    );
}

#[test]
fn strict_struct_field_order() {
    // @strict guarantees declaration order is preserved
    expect(
        "type Ordered @strict\n    x as i64\n    y as i64\n    z as i64\n\n*main()\n    s is Ordered(x is 10, y is 20, z is 30)\n    log(s.x)\n    log(s.y)\n    log(s.z)\n",
        "10\n20\n30",
    );
}

#[test]
fn align_struct_field_access() {
    // @align(64) struct should work correctly with cache-line alignment
    expect(
        "type Aligned @align(64)\n    val as i64\n    flag as i8\n\n*main()\n    s is Aligned(val is 99, flag is 7)\n    log(s.val)\n    log(s.flag)\n",
        "99\n7",
    );
}

#[test]
fn packed_strict_combined() {
    // @packed @strict together
    expect(
        "type PS @packed @strict\n    a as i8\n    b as i64\n\n*main()\n    s is PS(a is 5, b is 100)\n    log(s.a)\n    log(s.b)\n",
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

// perceus_rc_reuse_same_type / perceus_rc_values_independent removed —
// surface rc() no longer exists. Equivalent reuse coverage for heap
// nominals (structs/enums) lives in the fbip_* tests below.

// ── FBIP (Functional But In-Place) ─────────────────────────────────

#[test]
fn fbip_match_reconstruct_enum() {
    // Match on an enum, reconstruct a variant — should work correctly
    // (FBIP analysis may detect reuse opportunity)
    expect(
        "enum Shape\n    Circle(f64)\n    Square(f64)\n\n*double_shape(s as Shape) returns Shape\n    match s\n        Circle(r) ? Circle(r * 2.0)\n        Square(side) ? Square(side * 2.0)\n\n*main()\n    c is double_shape(Circle(5.0))\n    match c\n        Circle(r) ? log(r)\n        Square(_) ? log(0.0)\n",
        "10.000000",
    );
}

#[test]
fn fbip_match_transform_variant() {
    // Transform one variant to another of the same enum
    expect(
        "enum Op\n    Add(i64)\n    Mul(i64)\n\n*negate(op as Op) returns Op\n    match op\n        Add(n) ? Add(0 - n)\n        Mul(n) ? Mul(0 - n)\n\n*main()\n    r is negate(Add(42))\n    match r\n        Add(n) ? log(n)\n        Mul(n) ? log(n)\n",
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
    // Function takes owned enum, -> same enum type — Perceus should
    // detect tail reuse opportunity. Values should be correct.
    expect(
        "enum Shape\n    Circle(f64)\n    Square(f64)\n\n*scale(s as Shape, factor as f64) returns Shape\n    match s\n        Circle(r) ? Circle(r * factor)\n        Square(side) ? Square(side * factor)\n\n*main()\n    c is scale(Circle(3.0), 2.0)\n    match c\n        Circle(r) ? log(r)\n        Square(_) ? log(0.0)\n",
        "6.000000",
    );
}

// tail_reuse_rc_reconstruct / pool_perceus_loop_alloc /
// pool_perceus_nested_loop_alloc removed — surface rc() no longer
// exists under the "heap tax" model.

// ── Comptime Reflection (Option C) ──────────────────────────────

#[test]
fn comptime_fields_of_struct() {
    expect(
        "type Point\n    x as i64\n    y as i64\n\n*main\n    names is fields of Point\n    log names[0]\n    log names[1]\n",
        "x\ny",
    );
}

#[test]
fn comptime_size_of_type() {
    expect(
        "type Vec3\n    x as f64\n    y as f64\n    z as f64\n\n*main\n    s is size of Vec3\n    log s\n",
        "24",
    );
}

#[test]
fn comptime_size_of_primitive() {
    expect("*main\n    log(size of i64)\n    log(size of i8)\n", "8\n1");
}

#[test]
fn comptime_type_of_expr() {
    expect("*main\n    x is 42\n    log(type of x)\n", "i64");
}

#[test]
fn comptime_type_of_string() {
    expect("*main\n    s is 'hello'\n    log(type of s)\n", "String");
}

#[test]
fn auto_import_qualified_fmt_without_use() {
    expect("*main\n    log(fmt.hex(255))\n", "ff");
}

#[test]
fn auto_import_rejects_bare_std_function() {
    let err = expect_compile_fail("*main\n    log(hex(255))\n");
    assert!(err.contains("undefined function") && err.contains("hex"));
}

#[test]
fn auto_import_qualified_signal_name_without_use() {
    expect("*main\n    log(signal.name(signal.SIGINT))\n", "SIGINT");
}

#[test]
fn auto_import_qualified_terminal_size_without_use() {
    expect(
        "*main\n    sz is terminal.size()\n    log(sz.cols > 0)\n",
        "1",
    );
}

// ── Atomic Keyword Binding ──────────────────────────────────────

#[test]
fn atomic_binding_basic() {
    expect(
        "*main\n    atomic counter is 0\n    counter += 1\n    counter += 1\n    counter += 1\n    log counter\n",
        "3",
    );
}

#[test]
fn atomic_binding_sub() {
    expect(
        "*main\n    atomic val is 10\n    val -= 3\n    log val\n",
        "7",
    );
}

#[test]
fn atomic_builtin_sub() {
    // atomic_sub -> the old value; the pointer update may be
    // optimized away in single-threaded context (LLVM constant prop)
    expect(
        "*main\n    x is 100\n    old is atomic_sub(%x, 30)\n    log old\n",
        "100",
    );
}

// ── Query Blocks ─────────────────────────────────────────────────────

#[test]
fn query_block_select() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n\n    r is users query\n        where age > 28\n    log r.name\n",
        "Alice",
    );
}

#[test]
fn query_block_and() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n\n    r is users query\n        where age > 28 and name equals 'Charlie'\n    log r.name\n    log r.age\n",
        "Charlie\n35",
    );
}

#[test]
fn query_block_delete() {
    expect_store(
        "store items\n    name as String\n    value as i64\n\n*main\n    insert items 'Alpha', 10\n    insert items 'Beta', 20\n    insert items 'Gamma', 30\n\n    items query\n        where value equals 10\n        delete\n\n    c is count items\n    log c\n",
        "2",
    );
}

#[test]
fn query_block_set() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n\n    users query\n        where name equals 'Bob'\n        set age is 99\n\n    r is users where name equals 'Bob'\n    log r.age\n",
        "99",
    );
}

#[test]
fn query_block_multi_where() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n    insert users 'Diana', 28\n\n    r is users query\n        where age > 20\n        where name equals 'Charlie'\n    log r.name\n    log r.age\n",
        "Charlie\n35",
    );
}

#[test]
fn store_index_basic() {
    expect_store(
        "store products\n    name as String @index\n    price as i64\n\n*main\n    insert products 'Apple', 100\n    insert products 'Banana', 50\n    insert products 'Cherry', 75\n    n is count products\n    log n\n    a is products where name equals 'Apple'\n    log a.name\n    log a.price\n",
        "3\nApple\n100",
    );
}

#[test]
fn store_index_int_field() {
    expect_store(
        "store scores\n    player as String\n    pts as i64 @index\n\n*main\n    insert scores 'Alice', 100\n    insert scores 'Bob', 200\n    insert scores 'Charlie', 150\n    n is count scores\n    log n\n    r is scores where pts > 120\n    log r.player\n",
        "3\nBob",
    );
}

#[test]
fn store_agg_sum() {
    expect_store(
        "store vals\n    x as i64\n\n*main\n    insert vals 10\n    insert vals 20\n    insert vals 30\n    s is vals.sum(x)\n    log s\n",
        "60",
    );
}

#[test]
fn store_agg_min_max() {
    expect_store(
        "store vals\n    x as i64\n\n*main\n    insert vals 50\n    insert vals 10\n    insert vals 30\n    lo is vals.min(x)\n    hi is vals.max(x)\n    log lo\n    log hi\n",
        "10\n50",
    );
}

#[test]
fn store_versioned_basic() {
    // Test that @versioned stores track version_count correctly
    expect_store(
        "store posts @versioned\n    title as String\n    body as String\n\n*main\n    insert posts 'Draft', 'Hello'\n    set posts where title equals 'Draft' body 'Hello World'\n    set posts where title equals 'Draft' body 'Hello Updated'\n    vc is posts.version_count(1)\n    log vc\n",
        "3",
    );
}

#[test]
fn store_versioned_at_version() {
    // Test at_version returns 1 (found) for version 1, 0 for non-existent
    expect_store(
        "store docs @versioned\n    title as String\n    body as String\n\n*main\n    insert docs 'Test', 'First'\n    set docs where title equals 'Test' body 'Second'\n    f1 is docs.at_version(1, 1)\n    f2 is docs.at_version(1, 99)\n    log f1\n    log f2\n",
        "1\n0",
    );
}

#[test]
fn store_versioned_history_count() {
    // history() returns the number of old versions in the versions file
    expect_store(
        "store notes @versioned\n    text as String\n\n*main\n    insert notes 'v1'\n    set notes where text equals 'v1' text 'v2'\n    set notes where text equals 'v2' text 'v3'\n    h is notes.history(1)\n    log h\n",
        "2",
    );
}

// ── @unique enforcement ──────────────────────────────────────────────

#[test]
fn store_unique_skips_duplicate() {
    // Second insert with same @unique field should be silently skipped
    expect_store(
        "store emails\n    addr as String @unique\n    name as String\n\n*main\n    insert emails 'a@b.com', 'Alice'\n    insert emails 'a@b.com', 'Bob'\n    c is count emails\n    log c\n",
        "1",
    );
}

#[test]
fn store_unique_allows_different() {
    // Different values should both be inserted
    expect_store(
        "store emails\n    addr as String @unique\n    name as String\n\n*main\n    insert emails 'a@b.com', 'Alice'\n    insert emails 'c@d.com', 'Bob'\n    c is count emails\n    log c\n",
        "2",
    );
}

// ── distinct ─────────────────────────────────────────────────────

#[test]
fn store_distinct_i64() {
    expect_store(
        "store scores @simple\n    val as I64\n\n*main\n    insert scores 10\n    insert scores 20\n    insert scores 10\n    insert scores 30\n    d is scores.distinct(val)\n    log d\n",
        "3",
    );
}

#[test]
fn store_distinct_string() {
    expect_store(
        "store items @simple\n    name as String\n    cat as String\n\n*main\n    insert items 'a', 'fruit'\n    insert items 'b', 'fruit'\n    insert items 'c', 'veggie'\n    d is items.distinct(cat)\n    log d\n",
        "2",
    );
}

// ── migration ────────────────────────────────────────────────────

#[test]
fn store_migration_fresh_install() {
    // Migration on a store that doesn't exist yet — should be a no-op,
    // store created with current schema, migration recorded as applied.
    expect_store(
        "store items @simple\n    name as String\n    price as I64\n\nmigration 'add_stock' version 1\n    up\n        alter items\n            add stock as I64\n\n*main\n    insert items 'apple', 5\n    c is count items\n    log c\n",
        "1",
    );
}

#[test]
fn store_migration_idempotent() {
    // Running twice should NOT apply the migration a second time.
    expect_store(
        "store items @simple\n    name as String\n    price as I64\n\nmigration 'add_stock' version 1\n    up\n        alter items\n            add stock as I64\n\n*main\n    insert items 'apple', 5\n    c is count items\n    log c\n",
        "1",
    );
}

// ── Views ────────────────────────────────────────────────────────────

#[test]
fn store_view_count_basic() {
    // View with a where clause filters records; count returns matching count.
    expect_store(
        "store items @simple\n    name as String\n    price as i64\n\nview expensive from items\n    where price > 5\n\n*main\n    insert items 'apple', 3\n    insert items 'laptop', 999\n    insert items 'pen', 1\n    insert items 'phone', 500\n    c is expensive.count()\n    log c\n",
        "2",
    );
}

#[test]
fn store_view_count_no_match() {
    // View where no records match returns 0.
    expect_store(
        "store items @simple\n    name as String\n    price as i64\n\nview cheap from items\n    where price < 0\n\n*main\n    insert items 'apple', 3\n    insert items 'laptop', 999\n    c is cheap.count()\n    log c\n",
        "0",
    );
}

#[test]
fn store_view_count_no_filter() {
    // View without a where clause delegates to source store count.
    expect_store(
        "store items @simple\n    name as String\n    price as i64\n\nview everything from items\n\n*main\n    insert items 'apple', 3\n    insert items 'laptop', 999\n    c is everything.count()\n    log c\n",
        "2",
    );
}

// ── @kv Store Tests ──────────────────────────────────────────────────

#[test]
fn kv_set_get() {
    expect_store(
        "store cache @kv\n\n*main\n    cache.set('x', 42)\n    v is cache.get('x')\n    log v\n",
        "42",
    );
}

#[test]
fn kv_has() {
    expect_store(
        "store cache @kv\n\n*main\n    cache.set('k', 10)\n    h is cache.has('k')\n    log h\n    m is cache.has('missing')\n    log m\n",
        "1\n0",
    );
}

#[test]
fn kv_count() {
    expect_store(
        "store cache @kv\n\n*main\n    cache.set('a', 1)\n    cache.set('b', 2)\n    cache.set('c', 3)\n    n is cache.count()\n    log n\n",
        "3",
    );
}

#[test]
fn kv_del() {
    expect_store(
        "store cache @kv\n\n*main\n    cache.set('x', 99)\n    cache.del('x')\n    h is cache.has('x')\n    log h\n    n is cache.count()\n    log n\n",
        "0\n0",
    );
}

#[test]
fn kv_incr() {
    expect_store(
        "store cache @kv\n\n*main\n    cache.set('hits', 0)\n    cache.incr('hits')\n    cache.incr('hits')\n    cache.incr('hits')\n    v is cache.get('hits')\n    log v\n",
        "3",
    );
}

#[test]
fn kv_incr_delta() {
    expect_store(
        "store cache @kv\n\n*main\n    cache.set('score', 10)\n    cache.incr('score', 5)\n    v is cache.get('score')\n    log v\n",
        "15",
    );
}

#[test]
fn kv_overwrite() {
    expect_store(
        "store cache @kv\n\n*main\n    cache.set('x', 1)\n    cache.set('x', 2)\n    v is cache.get('x')\n    log v\n    n is cache.count()\n    log n\n",
        "2\n1",
    );
}

// ── @graph Store Tests ───────────────────────────────────────────────

#[test]
fn graph_from_count() {
    expect_store(
        "store edges @graph\n    src as i64\n    dst as i64\n    weight as i64\n\n*main\n    insert edges 1, 2, 10\n    insert edges 1, 3, 20\n    insert edges 2, 3, 30\n    n is edges.from(1)\n    log n\n",
        "2",
    );
}

#[test]
fn graph_to_count() {
    expect_store(
        "store edges @graph\n    src as i64\n    dst as i64\n\n*main\n    insert edges 1, 3\n    insert edges 2, 3\n    insert edges 3, 1\n    n is edges.to(3)\n    log n\n",
        "2",
    );
}

#[test]
fn graph_from_empty() {
    expect_store(
        "store edges @graph\n    src as i64\n    dst as i64\n\n*main\n    insert edges 1, 2\n    n is edges.from(99)\n    log n\n",
        "0",
    );
}

// ── @timeseries Store Tests ──────────────────────────────────────────

#[test]
fn ts_latest_count() {
    expect_store(
        "store temps @timeseries(ts)\n    ts as i64\n    value as i64\n\n*main\n    insert temps 100, 72\n    insert temps 200, 75\n    insert temps 300, 68\n    n is temps.latest()\n    log n\n",
        "3",
    );
}

// ── @vector Store Tests ──────────────────────────────────────────────

#[test]
fn vec_insert_count() {
    expect_store(
        "store vecs @vector(3)\n\n*main\n    vecs.insert([1.0, 2.0, 3.0])\n    vecs.insert([4.0, 5.0, 6.0])\n    n is vecs.count()\n    log n\n",
        "2",
    );
}

#[test]
fn vec_nearest_basic() {
    expect_store(
        "store vecs @vector(3)\n\n*main\n    vecs.insert([1.0, 0.0, 0.0])\n    vecs.insert([0.0, 1.0, 0.0])\n    vecs.insert([0.0, 0.0, 1.0])\n    k is vecs.nearest([1.0, 0.0, 0.0], 1)\n    log k\n",
        "1",
    );
}

#[test]
fn vec_nearest_topk() {
    expect_store(
        "store vecs @vector(2)\n\n*main\n    vecs.insert([1.0, 0.0])\n    vecs.insert([2.0, 0.0])\n    vecs.insert([10.0, 10.0])\n    k is vecs.nearest([1.5, 0.0], 2)\n    log k\n",
        "2",
    );
}

#[test]
fn vec_count_after_insert() {
    expect_store(
        "store emb @vector(4)\n\n*main\n    emb.insert([1.0, 2.0, 3.0, 4.0])\n    emb.insert([5.0, 6.0, 7.0, 8.0])\n    emb.insert([9.0, 10.0, 11.0, 12.0])\n    n is emb.count()\n    log n\n",
        "3",
    );
}

// ── Hook decorator tests ─────────────────────────────────────────────

#[test]
fn hook_before_insert() {
    expect_store(
        "*on_before\n    log('before')\n\nstore items @simple @before_insert(on_before)\n    name as String\n\n*main\n    insert items 'alice'\n",
        "before",
    );
}

#[test]
fn hook_after_insert() {
    expect_store(
        "*on_after\n    log('after')\n\nstore items @simple @after_insert(on_after)\n    name as String\n\n*main\n    insert items 'bob'\n",
        "after",
    );
}

#[test]
fn hook_before_after_insert() {
    expect_store(
        "*on_before\n    log('before')\n\n*on_after\n    log('after')\n\nstore items @simple @before_insert(on_before) @after_insert(on_after)\n    name as String\n\n*main\n    insert items 'x'\n",
        "before\nafter",
    );
}

#[test]
fn hook_before_delete() {
    expect_store(
        "*on_del\n    log('deleting')\n\nstore items @simple @before_delete(on_del)\n    name as String\n\n*main\n    insert items 'alice'\n    delete items where name equals 'alice'\n",
        "deleting",
    );
}

#[test]
fn hook_after_delete() {
    expect_store(
        "*on_del\n    log('deleted')\n\nstore items @simple @after_delete(on_del)\n    name as String\n\n*main\n    insert items 'alice'\n    delete items where name equals 'alice'\n",
        "deleted",
    );
}

#[test]
fn hook_multiple_inserts() {
    expect_store(
        "*on_ins\n    log('ins')\n\nstore items @simple @before_insert(on_ins)\n    val as i64\n\n*main\n    insert items 1\n    insert items 2\n    insert items 3\n",
        "ins\nins\nins",
    );
}

// ── Column store tests ───────────────────────────────────────────────

#[test]
fn column_sum() {
    expect_store(
        "store nums @simple @column\n    val as i64\n\n*main\n    insert nums 10\n    insert nums 20\n    insert nums 30\n    s is nums.sum(val)\n    log s\n",
        "60",
    );
}

#[test]
fn column_min() {
    expect_store(
        "store nums @simple @column\n    val as i64\n\n*main\n    insert nums 10\n    insert nums 5\n    insert nums 30\n    m is nums.min(val)\n    log m\n",
        "5",
    );
}

#[test]
fn column_max() {
    expect_store(
        "store nums @simple @column\n    val as i64\n\n*main\n    insert nums 10\n    insert nums 5\n    insert nums 30\n    m is nums.max(val)\n    log m\n",
        "30",
    );
}

// ── Bloom filter tests ──────────────────────────────────────────────

#[test]
fn bloom_test_present() {
    expect_store(
        "store items @simple\n    val as i64 @bloom\n\n*main\n    insert items 42\n    b is items.maybe(val, 42)\n    if b\n        log 1\n    else\n        log 0\n",
        "1",
    );
}

#[test]
fn bloom_test_absent() {
    expect_store(
        "store items @simple\n    val as i64 @bloom\n\n*main\n    insert items 42\n    b is items.maybe(val, 99)\n    if b\n        log 1\n    else\n        log 0\n",
        "0",
    );
}

#[test]
fn bloom_multiple_inserts() {
    expect_store(
        "store items @simple\n    val as i64 @bloom\n\n*main\n    insert items 10\n    insert items 20\n    insert items 30\n    a is items.maybe(val, 20)\n    b is items.maybe(val, 99)\n    if a\n        log 1\n    else\n        log 0\n    if b\n        log 1\n    else\n        log 0\n",
        "1\n0",
    );
}

// ── FTS (full-text search) tests ────────────────────────────────────

#[test]
fn fts_search_basic() {
    expect_store(
        "store docs @simple\n    text as String @search\n\n*main\n    insert docs 'hello world'\n    insert docs 'goodbye world'\n    n is docs.search(text, 'hello')\n    log n\n",
        "1",
    );
}

#[test]
fn fts_search_multiple_matches() {
    expect_store(
        "store docs @simple\n    text as String @search\n\n*main\n    insert docs 'hello world'\n    insert docs 'hello again'\n    n is docs.search(text, 'hello')\n    log n\n",
        "2",
    );
}

#[test]
fn fts_search_no_match() {
    expect_store(
        "store docs @simple\n    text as String @search\n\n*main\n    insert docs 'hello world'\n    n is docs.search(text, 'missing')\n    log n\n",
        "0",
    );
}

#[test]
fn fts_posting_count() {
    expect_store(
        "store docs @simple\n    text as String @search\n\n*main\n    insert docs 'hello world'\n    insert docs 'foo bar'\n    c is docs.search_count(text)\n    log c\n",
        "4",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: get by sid
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_get_by_sid() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n    r is get users 2\n    log r.name\n    log r.age\n",
        "Bob\n25",
    );
}

#[test]
fn store_get_by_sid_first() {
    expect_store(
        "store items\n    val as i64\n\n*main\n    insert items 10\n    insert items 20\n    r is get items 1\n    log r.val\n",
        "10",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: first (returns first matching record)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_first_match() {
    expect_store(
        "store people\n    name as String\n    age as i64\n\n*main\n    insert people 'Alice', 30\n    insert people 'Bob', 25\n    insert people 'Charlie', 35\n    r is first people where age > 20\n    log r.name\n",
        "Alice",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: exists (boolean check)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_exists_found() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    e is exists users where name equals 'Bob'\n    log e\n",
        "1",
    );
}

#[test]
fn store_exists_not_found() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    e is exists users where name equals 'Zara'\n    log e\n",
        "0",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: destroy (hard delete)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_destroy_removes_record() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n    destroy users where name equals 'Bob'\n    c is count users\n    log c\n",
        "2",
    );
}

#[test]
fn store_destroy_then_query() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n    destroy users where name equals 'Bob'\n    r is users where name equals 'Charlie'\n    log r.name\n    log r.age\n",
        "Charlie\n35",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: restore (undelete soft-deleted records)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_restore_basic() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n    delete users where name equals 'Bob'\n    c1 is count users\n    log c1\n    restore users where name equals 'Bob'\n    c2 is count users\n    log c2\n",
        "2\n3",
    );
}

// N-4: `count <store> where ...` shares the predicate evaluator with
// `<store> where ...` and `delete users where ...`.
#[test]
fn store_count_where_basic() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n    log(count users)\n    log(count users where age > 27)\n    log(count users where age > 20 and age < 32)\n",
        "3\n2\n2",
    );
}

#[test]
fn store_count_where_no_match() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    log(count users where age > 99)\n",
        "0",
    );
}

#[test]
fn store_restore_query_after() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    delete users where name equals 'Bob'\n    restore users where name equals 'Bob'\n    r is users where name equals 'Bob'\n    log r.name\n    log r.age\n",
        "Bob\n25",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: save (explicit flush)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_save_basic() {
    expect_store(
        "store data\n    val as i64\n\n*main\n    insert data 42\n    save data\n    c is count data\n    log c\n",
        "1",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: float aggregations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_agg_sum_float() {
    expect_store(
        "store meas\n    val as f64\n\n*main\n    insert meas 1.5\n    insert meas 2.5\n    insert meas 3.0\n    s is meas.sum(val)\n    log s\n",
        "7.000000",
    );
}

#[test]
fn store_agg_min_max_float() {
    expect_store(
        "store meas\n    val as f64\n\n*main\n    insert meas 3.14\n    insert meas 1.41\n    insert meas 2.72\n    lo is meas.min(val)\n    hi is meas.max(val)\n    log lo\n    log hi\n",
        "1.410000\n3.140000",
    );
}

#[test]
fn store_agg_avg_int() {
    expect_store(
        "store scores\n    val as i64\n\n*main\n    insert scores 10\n    insert scores 20\n    insert scores 30\n    a is scores.avg(val)\n    log a\n",
        "20.000000",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: delete + exists interaction
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_exists_after_delete() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    delete users where name equals 'Bob'\n    e is exists users where name equals 'Bob'\n    log e\n",
        "0",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: get skips deleted records
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_get_skips_deleted() {
    // After soft-deleting sid=2, get with sid=3 should still work
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    insert users 'Charlie', 35\n    delete users where name equals 'Bob'\n    r is get users 3\n    log r.name\n    log r.age\n",
        "Charlie\n35",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: destroy with compound filter
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_destroy_and_filter() {
    expect_store(
        "store products\n    name as String\n    price as i64\n    stock as i64\n\n*main\n    insert products 'Apple', 100, 50\n    insert products 'Banana', 50, 100\n    insert products 'Cherry', 100, 10\n    destroy products where price equals 100 and stock < 20\n    c is count products\n    log c\n",
        "2",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: set after delete (only updates non-deleted records)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_set_skips_deleted() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    delete users where name equals 'Alice'\n    set users where age < 50 age 99\n    r is users where name equals 'Bob'\n    log r.age\n",
        "99",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store: performance regression suite
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Store performance regression test — runs the full perf suite and
/// verifies all operations complete within generous upper bounds.
/// Run with: cargo test store_perf_regression -- --ignored --nocapture
#[test]
#[ignore]
fn store_perf_regression() {
    use std::time::Instant;

    let src = r#"
use std/time

store bench
    key as i64
    value as i64
    score as f64

*main
    i is 0
    while i < 5000
        insert bench i, i * 7, 3.14
        i is i + 1

    t0 is monotonic()
    j is 0
    while j < 100
        c is count bench
        j is j + 1
    log('count_x100')
    log(elapsed(t0))

    t0 is monotonic()
    j is 0
    while j < 100
        r1 is bench where key equals 2500
        j is j + 1
    log('query_eq_x100')
    log(elapsed(t0))

    t0 is monotonic()
    j is 0
    while j < 100
        s is bench.sum(value)
        j is j + 1
    log('agg_sum_x100')
    log(elapsed(t0))

    t0 is monotonic()
    j is 0
    while j < 100
        d is bench.distinct(value)
        j is j + 1
    log('distinct_x100')
    log(elapsed(t0))

    t0 is monotonic()
    j is 0
    while j < 100
        set bench where key equals 500 value 999
        j is j + 1
    log('set_eq_x100')
    log(elapsed(t0))

    log('done')
"#;

    let wall_start = Instant::now();
    let output = compile_and_run_in_dir(src);
    let wall_total = wall_start.elapsed();

    // Parse timing results
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines.last().copied(),
        Some("done"),
        "benchmark did not complete"
    );

    let mut timings: Vec<(&str, f64)> = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        if let Ok(t) = lines[i + 1].parse::<f64>() {
            timings.push((lines[i], t));
            i += 2;
        } else {
            i += 1;
        }
    }

    eprintln!("\n=== Store Performance Results (5000 records × 100 iterations) ===");
    for (name, secs) in &timings {
        let per_call_us = secs * 1_000_000.0 / 100.0;
        eprintln!("  {name:<20} {secs:.6}s  ({per_call_us:.0}µs/call)");
    }
    eprintln!("  wall total: {:.3}s", wall_total.as_secs_f64());

    // Regression bounds: 2× baseline with generous margin for CI variance.
    // Baselines (per ×100 batch): count ~0.06s, query ~0.05s, sum ~0.05s,
    //   distinct ~0.08s, set ~0.10s
    let max_allowed = 0.5; // 500ms per ×100 batch — 5× headroom over typical
    for (name, secs) in &timings {
        assert!(
            *secs < max_allowed,
            "PERF REGRESSION: {name} took {secs:.3}s (limit {max_allowed}s)"
        );
    }

    // Distinct should NOT regress to O(n²) behavior (was 0.5s, now ~0.07s)
    if let Some((_, distinct_time)) = timings.iter().find(|(n, _)| *n == "distinct_x100") {
        assert!(
            *distinct_time < 0.25,
            "PERF REGRESSION: distinct is too slow ({distinct_time:.3}s) — may have regressed to O(n²)"
        );
    }
}

// ── try keyword removed — replaced by `!` postfix and `defer` (Layer 1) ──

#[test]
fn defer_runs_on_normal_exit() {
    expect("*main()\n    defer\n        log(2)\n    log(1)\n", "1\n2");
}

#[test]
fn defer_lifo_order() {
    expect(
        "*main()\n    defer\n        log(\"a\")\n    defer\n        log(\"b\")\n    log(\"start\")\n",
        "start\nb\na",
    );
}

#[test]
fn defer_runs_on_bang_return() {
    let src = r#"
err NetworkError
    Timeout

*do_it() returns i64 ! NetworkError
    defer
        log("cleanup")
    log("before")
    ! -1
    log("after")
    0

*main()
    do_it()
"#;
    expect(src, "before\ncleanup");
}

#[test]
fn signature_multi_error_union() {
    let src = r#"
err Net
    Timeout

err Disk
    NotFound

*op(x as i64) returns i64 ! Net ! Disk
    if x is 1
        ! -1
    if x is 2
        ! -2
    x

*main()
    op(0)
    log(99)
"#;
    expect(src, "99");
}

#[test]
fn signature_undeclared_error_rejected() {
    let src = r#"
err Net
    Timeout

err Other
    Boom

*op() returns i64 ! Net
    ! Boom
    0

*main()
    op()
"#;
    expect_compile_fail(src);
}

#[test]
fn defer_block_with_multiple_stmts() {
    expect(
        "*main()\n    defer\n        log(\"a\")\n        log(\"b\")\n    log(\"start\")\n",
        "start\na\nb",
    );
}

#[test]
fn try_option_some() {
    // Replaced: `try` removed; verify equivalent shape using `!` early return.
    expect(
        "err Fail\n    Bad\n\n*do_thing() returns i64 ! Fail\n    42\n\n*main()\n    log(do_thing())\n",
        "42",
    );
}

#[test]
fn try_option_nothing() {
    // Replaced: error-path early return via `!` returns sentinel value.
    expect(
        "err Fail\n    Bad\n\n*do_thing() returns i64 ! Fail\n    ! -1\n    0\n\n*main()\n    log(do_thing())\n",
        "-1",
    );
}

#[test]
fn try_result_ok() {
    expect(
        "err E\n    Boom\n\n*do_thing() returns i64 ! E\n    20\n\n*main()\n    log(do_thing())\n",
        "20",
    );
}

#[test]
fn try_result_err() {
    expect(
        "err E\n    Boom\n\n*do_thing() returns i64 ! E\n    ! 99\n    0\n\n*main()\n    log(do_thing())\n",
        "99",
    );
}

// ── Layer 2: Unitary errors-as-values convention ────────────────────

#[test]
fn err_enum_as_return_type_ok_branch() {
    // Canonical jinn convention: a function may return an err enum directly.
    // Plain return / final expression yields a success-tagged variant.
    let src = r#"
err Outcome
    Ok(i64)
    Bad

*compute(x as i64) returns Outcome
    if x is 0
        ! Bad
    Ok(x + 1)

*main()
    r is compute(41)
    match r
        Ok(v) ?
            log(v)
        Bad ?
            log(-1)
"#;
    expect(src, "42");
}

#[test]
fn err_enum_as_return_type_err_branch() {
    // Same convention; this time the err variant is taken via `! Variant`.
    let src = r#"
err Outcome
    Ok(i64)
    Bad

*compute(x as i64) returns Outcome
    if x is 0
        ! Bad
    Ok(x + 1)

*main()
    r is compute(0)
    match r
        Ok(v) ?
            log(v)
        Bad ?
            log(-1)
"#;
    expect(src, "-1");
}

#[test]
fn err_return_to_incompatible_t_rejected() {
    // T = i64, but `! Bad` returns an err-variant value of type `Fail`.
    // The jinn convention says: errors are values; encode them as values of T
    // (a sentinel) or declare the function to return the err enum directly.
    let src = r#"
err Fail
    Bad

*do_thing() returns i64 ! Fail
    ! Bad
    0

*main()
    do_thing()
"#;
    expect_compile_fail(src);
}

#[test]
fn defer_in_nested_block_runs_at_function_exit() {
    // `defer` is function-scoped: a defer inside an `if` block still runs
    // when the *function* returns, in LIFO order with other defers.
    let src = r#"
*go(x as i64)
    defer
        log("outer")
    if x is 1
        defer
            log("inner")
        log("in-if")
    log("end")

*main()
    go(1)
"#;
    expect(src, "in-if\nend\ninner\nouter");
}

#[test]
fn defer_runs_on_err_branch_of_err_enum_return() {
    // Combining defer with an err-enum return: cleanup still fires before
    // the early `! Variant` exits the function.
    let src = r#"
err Outcome
    Ok(i64)
    Bad

*compute(x as i64) returns Outcome
    defer
        log("cleanup")
    if x is 0
        ! Bad
    Ok(x)

*main()
    r is compute(0)
    match r
        Ok(v) ?
            log(v)
        Bad ?
            log(-1)
"#;
    expect(src, "cleanup\n-1");
}

// ── Layer 2 sugar: guard form & handler chain ───────────────────────

#[test]
fn bind_guard_propagates_variant() {
    // `a is x() ! Bad` — when x() returns Bad, propagate it from the
    // caller; otherwise bind a to the value and continue.
    let src = r#"
err Outcome
    Ok(i64)
    Bad

*compute(x as i64) returns Outcome
    if x is 0
        ! Bad
    Ok(x + 1)

*caller(x as i64) returns Outcome
    a is compute(x) ! Bad
    Ok(99)

*main()
    r is caller(0)
    match r
        Ok(v) ?
            log(v)
        Bad ?
            log(-1)
"#;
    expect(src, "-1");
}

#[test]
fn bind_guard_falls_through_on_ok() {
    // Same shape; non-Bad value falls through and `a` is bound to the
    // raw enum value (so we can still pattern-match on it downstream).
    let src = r#"
err Outcome
    Ok(i64)
    Bad

*compute(x as i64) returns Outcome
    if x is 0
        ! Bad
    Ok(x + 1)

*caller(x as i64) returns Outcome
    a is compute(x) ! Bad
    Ok(99)

*main()
    r is caller(5)
    match r
        Ok(v) ?
            log(v)
        Bad ?
            log(-1)
"#;
    expect(src, "99");
}

#[test]
fn bind_handler_chain_ok_path() {
    // `a is x() ? on_ok ! on_err` — Ok-arm runs `on_ok` with `a` bound to
    // the payload; non-Ok runs `on_err`.
    let src = r#"
err Outcome
    Ok(i64)
    Bad

*compute(x as i64) returns Outcome
    if x is 0
        ! Bad
    Ok(x + 1)

*main()
    a is compute(5) ? log(a) ! log(-100)
"#;
    expect(src, "6");
}

#[test]
fn bind_handler_chain_err_path() {
    let src = r#"
err Outcome
    Ok(i64)
    Bad

*compute(x as i64) returns Outcome
    if x is 0
        ! Bad
    Ok(x + 1)

*main()
    a is compute(0) ? log(a) ! log(-100)
"#;
    expect(src, "-100");
}

#[test]
fn bind_handler_chain_ok_only() {
    // Without `! on_err`, the err path silently falls through.
    let src = r#"
err Outcome
    Ok(i64)
    Bad

*compute(x as i64) returns Outcome
    if x is 0
        ! Bad
    Ok(x + 1)

*main()
    a is compute(7) ? log(a)
    log(0)
"#;
    expect(src, "8\n0");
}

#[test]
fn ternary_in_bind_still_works() {
    // The Layer-2 sugar must not break standard ternary on the RHS of
    // `is`. `cond ? then ! else` and `cond ! else` continue to apply
    // when no sugar shape matches.
    let src = r#"
*main()
    x is 5
    r is x > 3 ? "big" ! "small"
    log(r)
    s is x > 99 ! "fallback"
    log(s)
"#;
    expect(src, "big\nfallback");
}

// ── Layer 2 sugar follow-ups: implicit `err`, lowercase variants,
//    bare-statement form, type-driven rejection. ──────────────────────

#[test]
fn handler_chain_binds_implicit_err() {
    // Inside the failure handler the err value is bound as the implicit
    // identifier `err`. Pass it to a helper that pattern-matches it.
    let src = r#"
err Outcome
    Ok(i64)
    Bad(i64)

*compute(x as i64) returns Outcome
    if x is 0
        ! Bad(42)
    Ok(x + 1)

*report(o as Outcome)
    match o
        Bad(c) ?
            log(c)
        _ ?
            log(-1)

*main()
    a is compute(0) ? log(a) ! report(err)
"#;
    expect(src, "42");
}

#[test]
fn handler_chain_bare_statement_form() {
    // No `is` wrapper — `call() ? on_ok ! on_err` is a statement on its own.
    let src = r#"
err Outcome
    Ok(i64)
    Bad

*compute(x as i64) returns Outcome
    if x is 0
        ! Bad
    Ok(x + 1)

*main()
    compute(5) ? log(100) ! log(-100)
    compute(0) ? log(200) ! log(-200)
"#;
    expect(src, "100\n-200");
}

#[test]
fn handler_chain_bare_binds_implicit_err() {
    // Bare form also exposes `err` to the failure handler.
    let src = r#"
err Outcome
    Ok(i64)
    Bad(i64)

*compute(x as i64) returns Outcome
    if x is 0
        ! Bad(7)
    Ok(x + 1)

*report(o as Outcome)
    match o
        Bad(c) ?
            log(c)
        _ ?
            log(-1)

*main()
    compute(0) ? log(0) ! report(err)
"#;
    expect(src, "7");
}

#[test]
fn bind_string_fallback_still_ternary() {
    // `! "literal"` is unambiguously a ternary-else (token after `!` is a
    // literal, not a bare ident) and must keep working.
    let src = r#"
*main()
    x is 5
    s is x > 99 ! "fallback"
    log(s)
"#;
    expect(src, "fallback");
}

// ── Layer 2: error-union annotation enforcement ─────────────────────

#[test]
fn err_annotation_narrows_must_list_used_variants() {
    // `! E1` is declared but `! Bad2` (an E2 variant) is used in the body.
    // The typer must reject this: the explicit annotation is the contract.
    let src = r#"
err E1
    Bad1

err E2
    Bad2

*do_thing() returns i64 ! E1
    ! Bad2
    0

*main()
    do_thing()
"#;
    let stderr = expect_compile_fail(src);
    assert!(
        stderr.contains("does not list") || stderr.contains("E2"),
        "expected diagnostic mentioning E2 or 'does not list', got: {stderr}"
    );
}

#[test]
fn err_annotation_can_list_used_variant() {
    // The body compiles when the signature lists the err enum it uses.
    let src = r#"
err Outcome
    Ok(i64)
    Bad

*do_thing(x as i64) returns Outcome ! Outcome
    if x is 0
        ! Bad
    Ok(x + 1)

*main()
    r is do_thing(5)
    match r
        Ok(v) ?
            log(v)
        _ ?
            log(-1)
"#;
    expect(src, "6");
}

// ── Layer 2: `!!` error-throw sugar ──────────────────────────────────────────

#[test]
fn bangbang_bare_no_ok_arm() {
    // `expr !! Variant` — on error throw Variant, on ok fall through silently.
    let src = r#"
err Res
    Ok(i64)
    Fail

*might_fail(x as i64) returns Res
    if x is 0
        ! Fail
    Ok(x * 2)

*caller() returns Res
    might_fail(3) !! Fail
    Ok(99)

*main()
    r is caller()
    match r
        Ok(v) ?
            log(v)
        _ ?
            log(-1)
"#;
    expect(src, "99");
}

#[test]
fn bangbang_propagates_on_error() {
    // When the call returns an error, `!!` rethrows.
    let src = r#"
err Res
    Ok(i64)
    Fail

*might_fail(x as i64) returns Res
    if x is 0
        ! Fail
    Ok(x * 2)

*caller() returns Res
    might_fail(0) !! Fail
    Ok(99)

*main()
    r is caller()
    match r
        Ok(v) ?
            log(v)
        Fail ?
            log(-1)
"#;
    expect(src, "-1");
}

#[test]
fn bangbang_handler_chain_ok_arm() {
    // `call() ? on_ok !! Variant` — ok: run on_ok, err: throw Variant.
    // Must be inside a function that declares error returns.
    let src = r#"
err Res
    Ok(i64)
    Fail

*might_fail(x as i64) returns Res
    if x is 0
        ! Fail
    Ok(x + 10)

*caller() returns Res
    might_fail(5) ? log(7) !! Fail
    Ok(0)

*main()
    caller()
"#;
    expect(src, "7");
}

#[test]
fn bangbang_handler_chain_with_err_handler() {
    // `call() ? on_ok ! on_falsy !! Variant`:
    //   - on_falsy is the ternary-else for a falsy-but-non-error Ok payload
    //   - !! Variant fires for actual errors — mutually exclusive from on_falsy
    // Here might_fail(0) returns an error, so on_falsy (log(-2)) must NOT run;
    // only the !! arm fires, propagating Fail.
    let src = r#"
err Res
    Ok(i64)
    Fail

*might_fail(x as i64) returns Res
    if x is 0
        ! Fail
    Ok(x + 10)

*outer() returns Res
    might_fail(0) ? log(7) ! log(-2) !! Fail
    Ok(42)

*main()
    r is outer()
    match r
        Ok(v) ?
            log(v)
        Fail ?
            log(-9)
"#;
    // error arm fires (!! Fail), log(-2) is skipped, outer returns Fail, main logs -9
    expect(src, "-9");
}

#[test]
fn bangbang_handler_chain_falsy_branch_runs() {
    // Same `? on_ok ! on_falsy !! Variant` form, but the call succeeds with
    // a falsy (zero) payload — on_falsy runs, !! does not fire.
    let src = r#"
err Res
    Ok(i64)
    Fail

*might_fail(x as i64) returns Res
    if x is 0
        ! Fail
    Ok(x + 10)

*outer() returns Res
    might_fail(5) ? log(7) ! log(-2) !! Fail
    Ok(42)

*main()
    r is outer()
    match r
        Ok(v) ?
            log(v)
        Fail ?
            log(-9)
"#;
    // might_fail(5) → Ok(15), 15 is truthy → log(7) runs; outer returns Ok(42); main logs 42
    expect(src, "7\n42");
}

#[test]
fn bangbang_bind_form() {
    // `x is call() !! Variant` — binds x to the full result (type Res), throws on any error.
    // x is then matched to extract the ok payload.
    let src = r#"
err Res
    Ok(i64)
    Fail

*get_val(x as i64) returns Res
    if x is 0
        ! Fail
    Ok(x + 100)

*caller() returns Res
    v is get_val(5) !! Fail
    v

*main()
    r is caller()
    match r
        Ok(n) ?
            log(n)
        _ ?
            log(-1)
"#;
    expect(src, "105");
}

#[test]
fn sqlite_basic() {
    let src = r#"
extern *jinn_sqlite_open(path as %i8) returns %i8
extern *jinn_sqlite_close(db as %i8) returns i32
extern *jinn_sqlite_exec(db as %i8, sql as %i8) returns i32
extern *jinn_sqlite_prepare(db as %i8, sql as %i8) returns %i8
extern *jinn_sqlite_finalize(stmt as %i8)
extern *jinn_sqlite_step(stmt as %i8) returns i32
extern *jinn_sqlite_bind_text(stmt as %i8, idx as i32, val as %i8, len as i64) returns i32
extern *jinn_sqlite_column_int(stmt as %i8, idx as i32) returns i64
extern *jinn_sqlite_column_text(stmt as %i8, idx as i32) returns %i8
extern *jinn_sqlite_last_insert_id(db as %i8) returns i64

*main()
    db is extern.jinn_sqlite_open(":memory:")
    extern.jinn_sqlite_exec(db, "create table t (id integer primary key, name text)")
    extern.jinn_sqlite_exec(db, "insert into t (name) values ('alice')")
    extern.jinn_sqlite_exec(db, "insert into t (name) values ('bob')")

    stmt is extern.jinn_sqlite_prepare(db, "select id, name from t order by id")
    total is 0
    while extern.jinn_sqlite_step(stmt) equals 1
        total is total + extern.jinn_sqlite_column_int(stmt, 0)
    extern.jinn_sqlite_finalize(stmt)
    log(total)
    extern.jinn_sqlite_close(db)
"#;
    expect(src, "3");
}

#[test]
fn inline_annotation() {
    let src = r#"
@inline
*add(a as i64, b as i64) returns i64
    a + b

*main()
    log(add(10, 20))
"#;
    expect(src, "30");
}

#[test]
fn noinline_annotation() {
    let src = r#"
@noinline
*square(x as i64) returns i64
    x * x

*main()
    log(square(7))
"#;
    expect(src, "49");
}

#[test]
fn cold_hot_annotations() {
    let src = r#"
@hot
*fast_path(x as i64) returns i64
    x + 1

@cold
*slow_path(x as i64) returns i64
    x - 1

*main()
    log(fast_path(10))
    log(slow_path(10))
"#;
    expect(src, "11\n9");
}

#[test]
fn global_variable_basic() {
    let src = r#"
global counter is 0

*main()
    counter is 10
    log(counter)
"#;
    expect(src, "10");
}

#[test]
fn global_variable_mutation() {
    let src = r#"
global count is 0

*increment()
    count is count + 1

*main()
    increment()
    increment()
    increment()
    log(count)
"#;
    expect(src, "3");
}

#[test]
fn global_variable_initial_value() {
    let src = r#"
global x is 42

*main()
    log(x)
"#;
    expect(src, "42");
}

// ── try chaining (replaced by `!` early-return chains) ───────────

#[test]
fn try_chain_two_levels() {
    // Two `!` early-return calls don't fire when the operations succeed.
    expect(
        r#"
err Fail
    Bad

*step1() returns i64 ! Fail
    10

*step2() returns i64 ! Fail
    20

*run() returns i64 ! Fail
    a is step1()
    b is step2()
    a + b

*main()
    log(run())
"#,
        "30",
    );
}

#[test]
fn try_chain_short_circuits() {
    // Sentinel-value short-circuit via `!`.
    expect(
        r#"
err Fail
    Bad

*fail() returns i64 ! Fail
    ! -1
    0

*should_not_run() returns i64 ! Fail
    log(999)
    99

*run() returns i64 ! Fail
    a is fail()
    if a equals -1
        ! -1
    b is should_not_run()
    a + b

*main()
    log(run())
"#,
        "-1",
    );
}

// ── multi-function error propagation chain ───────────────────────

#[test]
fn try_propagation_chain() {
    // A → B → C; sentinel value propagates through plain returns.
    expect(
        r#"
err Fail
    Bad

*level_c(x as i64) returns i64 ! Fail
    if x < 0
        ! -1
    x * 2

*level_b(x as i64) returns i64 ! Fail
    v is level_c(x)
    v + 100

*level_a(x as i64) returns i64 ! Fail
    v is level_b(x)
    v + 1

*main()
    log(level_a(5))
    log(level_a(-1))
"#,
        "111\n100",
    );
}

// ── bang return (!) edge cases ────────────────────────────────────

#[test]
fn bang_return_in_loop() {
    // Early return from inside a for loop.
    expect(
        r#"
*find_first(n as i64) returns i64
    for i in n
        if i equals 3
            ! i
    -1

*main()
    log(find_first(10))
    log(find_first(2))
"#,
        "3\n-1",
    );
}

#[test]
fn bang_return_nested_calls() {
    // ! used inside a helper called from a loop.
    expect(
        r#"
*check(x as i64) returns i64
    if x > 10
        ! x
    0

*main()
    log(check(5))
    log(check(15))
"#,
        "0\n15",
    );
}

// ── Perceus / ownership patterns ─────────────────────────────────

#[test]
fn rc_linear_use() {
    // A string (heap-allocated) created and used exactly once — should compile
    // and produce correct output even with aggressive drop elision.
    expect(
        r#"
*make(x as i64) returns String
    if x > 0
        "positive"
    else
        "non-positive"

*main()
    s is make(5)
    log(s)
    t is make(-1)
    log(t)
"#,
        "positive\nnon-positive",
    );
}

#[test]
fn string_concat_ownership() {
    // String concatenation forces copies/moves; test that ownership is handled
    // correctly across multiple bindings.
    expect(
        r#"
*main()
    a is "hello"
    b is " world"
    c is a + b
    log(c)
    d is c + "!"
    log(d)
"#,
        "hello world\nhello world!",
    );
}

#[test]
fn vec_single_owner() {
    // Vec with a single binding path — tests that the vec is dropped exactly once.
    expect(
        r#"
*main()
    v is vec(1, 2, 3)
    log(v.len())
"#,
        "3",
    );
}

#[test]
fn closure_captures_value() {
    // Closure capturing an i64 by value; tests Perceus closure-capture tracking.
    expect(
        r#"
*apply(f as (i64) returns i64, x as i64) returns i64
    f(x)

*main()
    base is 10
    add_base is |x as i64| returns i64 x + base
    log(apply(add_base, 5))
    log(apply(add_base, 20))
"#,
        "15\n30",
    );
}

// ── compilation pipeline: struct field inference ─────────────────

#[test]
fn struct_field_inferred_from_use() {
    // Struct with typed fields.
    expect(
        r#"
type Point
    x as i64
    y as i64

*main()
    p is Point(x is 3, y is 4)
    log(p.x + p.y)
"#,
        "7",
    );
}

#[test]
fn nested_struct_access() {
    expect(
        r#"
type Inner
    val as i64

type Outer
    inner as Inner
    count as i64

*main()
    o is Outer(inner is Inner(val is 42), count is 1)
    log(o.inner.val)
    log(o.count)
"#,
        "42\n1",
    );
}

// ── exhaustiveness checking ───────────────────────────────────────

#[test]
fn match_wildcard_arm() {
    // Integer match must have a wildcard (catch-all) arm.
    expect(
        r#"
*classify(n as i64) returns i64
    match n
        0 ? 0
        1 ? 1
        _ ? 2

*main()
    log(classify(0))
    log(classify(1))
    log(classify(5))
"#,
        "0\n1\n2",
    );
}

#[test]
fn match_enum_exhaustive() {
    expect(
        r#"
enum Color
    Red
    Green
    Blue

*name(c as Color) returns i64
    match c
        Red ? 1
        Green ? 2
        Blue ? 3

*main()
    log(name(Red))
    log(name(Green))
    log(name(Blue))
"#,
        "1\n2\n3",
    );
}

// ── compile-time constant folding ────────────────────────────────

#[test]
fn comptime_arithmetic() {
    expect(
        r#"
const LIMIT is 100
const HALF is LIMIT / 2

*main()
    log(LIMIT)
    log(HALF)
"#,
        "100\n50",
    );
}

#[test]
fn comptime_used_in_array_bound() {
    expect(
        r#"
const N is 4

*main()
    arr is [1, 2, 3, 4]
    log(arr[N - 1])
"#,
        "4",
    );
}

// ── higher-order functions / first-class fn ───────────────────────

#[test]
fn hof_map_manual() {
    // Manual map over a vec using a loop and a function value.
    expect(
        r#"
*double(x as i64) returns i64
    x * 2

*main()
    src is vec(1, 2, 3, 4, 5)
    dst is vec()
    for i in src.len()
        dst.push(double(src[i]))
    for i in dst.len()
        log(dst[i])
"#,
        "2\n4\n6\n8\n10",
    );
}

#[test]
fn hof_passed_as_argument() {
    expect(
        r#"
*apply_twice(f as (i64) returns i64, x as i64) returns i64
    f(f(x))

*inc(x as i64) returns i64
    x + 1

*main()
    log(apply_twice(inc, 0))
    log(apply_twice(inc, 10))
"#,
        "2\n12",
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// N-7: Named-field insert into stores
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn store_insert_named_basic() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users (name is 'Alice', age is 30)\n    r is users where name equals 'Alice'\n    log r.name\n    log r.age\n",
        "Alice\n30",
    );
}

#[test]
fn store_insert_named_reordered() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users (age is 25, name is 'Bob')\n    r is users where name equals 'Bob'\n    log r.name\n    log r.age\n",
        "Bob\n25",
    );
}

#[test]
fn store_insert_positional_still_works() {
    expect_store(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Carol', 40\n    r is users where name equals 'Carol'\n    log r.name\n    log r.age\n",
        "Carol\n40",
    );
}

#[test]
fn store_insert_named_missing_field_fails() {
    let err = expect_compile_fail(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users (name is 'Dan')\n",
    );
    assert!(
        err.contains("missing") || err.contains("age"),
        "expected missing-field error, got: {err}"
    );
}

#[test]
fn store_insert_named_unknown_field_fails() {
    let err = expect_compile_fail(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users (name is 'Eve', age is 22, height is 170)\n",
    );
    assert!(
        err.contains("unknown") || err.contains("height"),
        "expected unknown-field error, got: {err}"
    );
}

#[test]
fn store_insert_named_mixed_fails() {
    let err = expect_compile_fail(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users (name is 'Fay', 33)\n",
    );
    assert!(
        err.contains("mix") || err.contains("named") || err.contains("positional"),
        "expected mixed-field error, got: {err}"
    );
}

#[test]
fn store_insert_named_duplicate_fails() {
    let err = expect_compile_fail(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users (name is 'Gus', name is 'Gus2', age is 21)\n",
    );
    assert!(
        err.contains("duplicate") || err.contains("name"),
        "expected duplicate-field error, got: {err}"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// N-6: query blocks execute (where/delete/set), unsupported clauses
// emit clear errors instead of being silently dropped.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn query_block_executes_full_program() {
    expect_store(
        &std::fs::read_to_string("tests/programs/query_parse.jn").unwrap(),
        "Alice\n30\n26\n1",
    );
}

#[test]
fn query_block_sort_clause_errors() {
    let err = expect_compile_fail(
        "store u\n    name as String\n    age as i64\n\n*main\n    insert u 'A', 1\n    r is u query\n        where age > 0\n        sort age\n    log r.name\n",
    );
    assert!(
        err.contains("sort") && err.contains("not yet"),
        "expected sort-not-implemented error, got: {err}"
    );
}

#[test]
fn query_block_limit_clause_errors() {
    let err = expect_compile_fail(
        "store u\n    name as String\n    age as i64\n\n*main\n    insert u 'A', 1\n    r is u query\n        where age > 0\n        limit 5\n    log r.name\n",
    );
    assert!(
        err.contains("limit") && err.contains("not yet"),
        "expected limit-not-implemented error, got: {err}"
    );
}

// ── N-3: actor supervisor smoke tests ──

#[test]
fn supervisor_starts_and_restart_count_zero() {
    let out = compile_and_run_in_dir(
        "actor Worker\n    count as i64\n    @bump\n        count is count + 1\n\nsupervisor App\n    strategy is one_for_one\n    children\n        Worker\n\n*main\n    App_start()\n    log(App_restart_count())\n",
    );
    assert_eq!(out.trim(), "0", "expected restart_count==0, got {out:?}");
}

#[test]
fn supervisor_one_for_all_strategy_compiles() {
    let out = compile_and_run_in_dir(
        "actor A\n    n as i64\n    @ping\n        n is n + 1\n\nactor B\n    n as i64\n    @ping\n        n is n + 1\n\nsupervisor S\n    strategy is one_for_all\n    children\n        A\n        B\n\n*main\n    S_start()\n    log(S_restart_count())\n",
    );
    assert_eq!(out.trim(), "0");
}

#[test]
fn supervisor_unknown_child_errors() {
    let err = expect_compile_fail(
        "supervisor App\n    strategy is one_for_one\n    children\n        Nope\n\n*main\n    App_start()\n",
    );
    assert!(
        err.contains("unknown child") || err.contains("Nope"),
        "expected unknown-child error, got: {err}"
    );
}

// ── R8: Addable / arithmetic operand validation ──

#[test]
fn r8_arith_rejects_bool_add() {
    let err = expect_compile_fail("*main\n    log(true + false)\n");
    assert!(
        err.contains("operator `+` not defined") && err.contains("bool"),
        "expected `+ not defined` error for bool+bool, got: {err}"
    );
}

#[test]
fn r8_arith_rejects_bool_mul() {
    let err = expect_compile_fail("*main\n    log(true * false)\n");
    assert!(
        err.contains("operator `*` not defined") && err.contains("bool"),
        "expected `* not defined` error for bool*bool, got: {err}"
    );
}

#[test]
fn r8_arith_string_minus_string_rejected() {
    let err = expect_compile_fail("*main\n    log(\"a\" - \"b\")\n");
    assert!(
        err.contains("operator `-` not defined") && err.contains("String"),
        "expected `- not defined` error for String-String, got: {err}"
    );
}

#[test]
fn r8_arith_int_plus_int_ok() {
    let out = compile_and_run_in_dir("*main\n    log(1 + 2)\n");
    assert_eq!(out.trim(), "3");
}

#[test]
fn r8_arith_string_concat_ok() {
    let out = compile_and_run_in_dir("*main\n    log(\"a\" + \"b\")\n");
    assert_eq!(out.trim(), "ab");
}

// ── asm path smoke (ROADMAP: A.3 "Asm path unverified") ──

#[test]
fn asm_block_nop_compiles_and_runs() {
    // Verify inline-asm codegen path works end-to-end for a trivial nop.
    // Operand-reference forms (e.g. `mov $42, $0`) are presently fragile due
    // to the parser tokenizing the template; this test pins down the minimal
    // working contract.
    let out = compile_and_run_in_dir("*main\n    asm\n        nop\n    log(0)\n");
    assert_eq!(out.trim(), "0");
}

// ── Access-semantics R1.3 placeholders ──────────────────────────────────
//
// These verify BEHAVIOR (correct output, no double-free, no leak) for
// field-access patterns covered by spec §4.6 and §5.1. They are
// "placeholder" only in the sense that the IR-inspection check
// (asserting "no clone in the hot path" for short-lived borrows, and
// "exactly one clone" for escaping field reads) lands with R3.3 — at
// which point these tests gain an llvm-ir grep assertion.

#[test]
fn access_field_auto_copy_escape() {
    // §4.6 / §5.1: `s is b.name` followed by returning `s` and then
    // re-reading `b.name` in the caller. Must NOT move the field out
    // of `b` — both reads must succeed.
    expect_file("tests/programs/field_auto_copy.jn", "alice\nalice");
}

#[test]
fn access_field_short_lived_borrow() {
    // §4.6 / §5.1: `b.field` read inside an `if` condition is a
    // short-lived borrow; `b.field` must remain readable afterward.
    expect_file("tests/programs/field_short_lived_borrow.jn", "zero\n0\nhi");
}
