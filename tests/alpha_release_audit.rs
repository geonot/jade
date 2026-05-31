use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/audit_alpha/{name}")).unwrap()
}

fn compile_source(src: &str, extra_args: &[&str]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    let out = dir.path().join("test_bin");
    std::fs::write(&jinn, src).unwrap();
    let mut cmd = Command::new(jinnc());
    for arg in extra_args {
        cmd.arg(arg);
    }
    let status = cmd
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("jinnc failed to start");
    assert!(status.success(), "jinnc compilation failed for:\n{src}");
    (dir, out)
}

fn run_fixture(name: &str) -> String {
    let src = fixture(name);
    let (dir, out) = compile_source(&src, &[]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        output.status.success(),
        "binary exited with {:?}\nstderr: {}\nsource:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
        src
    );
    String::from_utf8(output.stdout).unwrap()
}

fn expect_fixture(name: &str, expected: &str) {
    let got = run_fixture(name);
    assert_eq!(got.trim(), expected.trim(), "fixture: {name}");
}

fn expect_compile_fail(name: &str, needles: &[&str]) {
    let src = fixture(name);
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("test.jn");
    let out = dir.path().join("test_bin");
    std::fs::write(&jinn, &src).unwrap();
    let output = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("jinnc failed to start");
    assert!(
        !output.status.success(),
        "expected compile failure for:\n{src}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for needle in needles {
        assert!(
            stderr.contains(needle),
            "expected diagnostic to contain `{needle}`; stderr was:\n{stderr}"
        );
    }
}

fn expect_runtime_fail(name: &str) {
    let src = fixture(name);
    let (dir, out) = compile_source(&src, &[]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        !output.status.success(),
        "expected runtime failure; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn alpha_audit_core_semantics() {
    expect_fixture(
        "core_semantics.jn",
        "37\n255\n64\n1024\n2.500000\nalpha-audit\n10\n720",
    );
}

#[test]
fn alpha_audit_types_patterns_ownership() {
    expect_fixture(
        "types_patterns_ownership.jn",
        "6\nrestored\n15\n9\n7\nscope-end\nresource-dropped",
    );
}

#[test]
fn alpha_audit_store_channels_actors() {
    expect_fixture("store_channels_actors.jn", "250\n333\n333\n11\n42");
}

#[test]
fn alpha_audit_generic_empty_enum() {
    // Regression: a generic enum with an empty variant, used through a function
    // whose parameter is annotated with a concrete instantiation (`Maybe of
    // i64`). Previously failed codegen with `FieldGet on pointer to unknown
    // struct type for field __tag` because the annotation was never
    // monomorphized to the concrete enum type. See AUDIT P0-5 / CRITICAL-1.
    expect_fixture("generic_empty_enum.jn", "9\n7");
}

#[test]
fn alpha_audit_generic_struct_param() {
    // Regression sibling of `alpha_audit_generic_empty_enum`: a generic struct
    // named in a concrete function-parameter annotation (`Box of i64`) must be
    // monomorphized to a struct with resolved field types so field access
    // (`b.value`) compiles. See AUDIT P0-5 / CRITICAL-1.
    expect_fixture("generic_struct_param.jn", "42");
}

#[test]
fn alpha_audit_recursive_generic_enum() {
    // Regression: a recursive generic enum (`Tree of T` with a
    // `Branch(Tree of T, Tree of T)` variant). The variant field must parse as a
    // full type rather than a bare identifier (parser previously emitted
    // `expected ,, got of`), and monomorphizing `Tree of i64` must canonicalize
    // the recursive self-reference to the same instantiation without recursing
    // forever. See AUDIT recursive-generic-enum parser/monomorphization fix.
    expect_fixture("recursive_generic_enum.jn", "15\n4");
}

#[test]
fn alpha_audit_soft_keyword_idents() {
    // Regression (MAJOR-1): reserved-but-contextual keywords (`from`, `to`, `by`,
    // `at`) used as parameter names and as plain variables in expression position.
    // They lex as keywords for range/slice/index syntax but must parse as
    // identifiers everywhere an identifier is expected. window(0,10,2) sums
    // 0+2+4+6+8 = 20; at_index(5) = 105.
    expect_fixture("soft_keyword_idents.jn", "20\n105");
}

#[test]
fn alpha_audit_bracket_list_type() {
    // Regression (MAJOR-2): `[T]` list-type syntax, the symmetric counterpart of
    // the `[...]` list literal, must parse as a type in struct fields, params,
    // return types, and `as` casts. Equivalent to `Vec of T`. Bag([10,20,30])
    // totals 60; first_two([7,8,9]) sums 7+8 = 15.
    expect_fixture("bracket_list_type.jn", "60\n15");
}

#[test]
fn alpha_audit_chr_builtin() {
    // Regression: the `chr(code)` builtin builds a one-byte string from an
    // integer character code (inverse of `String.char_at`). Used throughout
    // libjn/strings. "ABC" then "7".
    expect_fixture("chr_builtin.jn", "ABC\n7");
}

#[test]
fn alpha_audit_soft_keyword_nouns() {
    // Regression: contextual-noun soft keywords (`default`, `end`, `query`,
    // `view`) used as parameter names, struct field names in a field-init
    // construction, field access, and bare expression-atom variables. c.query +
    // c.view = 7; span(2, 10) = 8.
    expect_fixture("soft_keyword_nouns.jn", "7\n8");
}

#[test]
fn alpha_audit_word_operator_idents() {
    // Regression: the intentional word-operator aliases (`eq`/`equals`, `neq`,
    // `lt`/`gt`/`lte`/`gte`, `pow`) must double as ordinary identifiers — as
    // function names (`*equals`, `*pow`) and as bare variables in
    // expression-atom position (`if eq < 0`, `total + neq`). Their operator
    // role only fires infix. total = 5 (eq) + 3 (neq) + 100 (equals(eq,5)) +
    // 8 (pow(2,3)) = 116.
    expect_fixture("word_operator_idents.jn", "116");
}

#[test]
fn alpha_audit_std_test_mode() {
    let src = fixture("std_test_mode.jn");
    let (dir, out) = compile_source(&src, &["--test"]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("test binary failed to start");
    assert!(output.status.success(), "test binary failed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("test math and assertions ..."), "{stdout}");
    assert!(stdout.contains("test float guardrails ..."), "{stdout}");
    assert!(!stdout.contains("main-should-not-run"), "{stdout}");
}

#[test]
fn alpha_audit_negative_compile_cases() {
    expect_compile_fail("negative_partial_move.jn", &["moved", "`a`"]);
    expect_compile_fail("negative_resource_copy.jn", &["resource"]);
    expect_compile_fail("negative_resource_channel.jn", &["resource", "thread"]);
    expect_compile_fail(
        "negative_type_mismatch.jn",
        &["type mismatch", "i64", "string"],
    );
    expect_compile_fail(
        "negative_return_of_borrowed.jn",
        &["returning reference to local variable", "`x`"],
    );
}

#[test]
fn alpha_audit_runtime_bounds_case() {
    expect_runtime_fail("runtime_bounds_fail.jn");
}

/// `eprint` writes to stderr (with a trailing newline) while `print`/`log`
/// write to stdout. Verifies the two streams stay separate and that both
/// `String` and scalar arguments are supported.
#[test]
fn alpha_audit_eprint_to_stderr() {
    let src = "*main\n    \
        eprint('hello stderr')\n    \
        eprint(42)\n    \
        print('stdout only')\n";
    let (dir, out) = compile_source(src, &[]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(output.status.success(), "binary exited non-zero");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        stdout, "stdout only\n",
        "stdout should hold only print output"
    );
    assert_eq!(
        stderr, "hello stderr\n42\n",
        "stderr should hold eprint output with trailing newlines"
    );
}

/// A native (main-thread) stack overflow from unbounded recursion produces a
/// *specific* "stack overflow (native thread)" diagnostic and exits 134,
/// rather than a bare SIGSEGV. Compiled at `--opt 0` so LLVM cannot linearise
/// the non-tail recursion into a loop. The bound keeps the recursion from
/// being provably infinite (which LLVM may legally replace with a spin),
/// while the stack is exhausted long before the bound is reached.
#[test]
fn alpha_audit_native_stack_overflow_diagnostic() {
    let src = r#"*blow(n)
    if n > 100000000
        return n
    m is blow(n + 1)
    return m + n

*main
    x is blow(0)
    log(x)
    0
"#;
    let (dir, out) = compile_source(src, &["--opt", "0"]);
    let output = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(134),
        "expected exit 134 from stack-overflow handler; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("stack overflow (native thread)"),
        "expected specific native stack-overflow diagnostic; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("ulimit -s"),
        "expected remediation advice mentioning the OS stack limit; stderr:\n{stderr}"
    );
}

