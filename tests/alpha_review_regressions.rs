use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

struct Out {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn build(src: &str, opt: &str) -> (Out, tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let jn = dir.path().join("prog.jn");
    std::fs::write(&jn, src).unwrap();
    let bin = dir.path().join("prog.bin");
    let c = Command::new(jinnc())
        .arg(&jn)
        .arg("--opt")
        .arg(opt)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("invoke jinnc");
    (
        Out {
            status: c.status,
            stdout: String::from_utf8_lossy(&c.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&c.stderr).into_owned(),
        },
        dir,
        bin,
    )
}

fn compile_fails_with(src: &str, needle: &str) {
    let (c, _d, _b) = build(src, "0");
    assert!(
        !c.status.success(),
        "expected a compile error containing {needle:?}, but it compiled"
    );
    assert!(
        c.stderr.contains(needle),
        "diagnostic did not mention {needle:?}:\n{}",
        c.stderr
    );
}

fn run(src: &str, opt: &str) -> Out {
    let (c, dir, bin) = build(src, opt);
    assert!(c.status.success(), "compile failed:\n{}", c.stderr);
    let r = Command::new(&bin).current_dir(dir.path()).output().unwrap();
    Out {
        status: r.status,
        stdout: String::from_utf8_lossy(&r.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&r.stderr).into_owned(),
    }
}

fn expect_out(src: &str, want: &str) {
    for opt in ["0", "3"] {
        let r = run(src, opt);
        assert_eq!(
            r.stdout.trim_end(),
            want.trim_end(),
            "wrong output at --opt {opt}"
        );
    }
}

#[test]
fn if_branches_with_different_types_are_a_diagnostic_not_an_ice() {
    compile_fails_with(
        "*pick(f as bool)\n    if f\n        1\n    else\n        'two'\n\n*main\n    log pick(true)\n",
        "branches produce different types",
    );
}

#[test]
fn elif_chain_assigning_a_subset_of_variables_compiles_and_runs() {
    expect_out(
        "*tc(s as String) returns String\n    out is ''\n    flag is true\n    i is 0\n    while i < s.length\n        c is s.char_at(i)\n        if c equals 45\n            out is out + 'a'\n            flag is true\n        elif flag\n            out is out + 'b'\n            flag is false\n        elif c equals 46\n            out is out + 'c'\n        else\n            out is out + 'd'\n            flag is false\n        i is i + 1\n    out\n\n*main\n    log tc('a-b.c')\n    0\n",
        "babcd",
    );
}

#[test]
fn unused_strict_cast_still_traps_at_every_opt_level() {
    let src = "*big() returns i64\n    return 100000\n\n*main() returns i32\n    b is big()\n    s is b as strict i16\n    log(1)\n    0\n";
    for opt in ["0", "1", "3"] {
        let (c, dir, bin) = build(src, opt);
        assert!(c.status.success(), "{}", c.stderr);
        let r = Command::new(&bin).current_dir(dir.path()).output().unwrap();
        assert!(
            !r.status.success(),
            "`as strict` overflow was optimized away at --opt {opt}"
        );
    }
}

#[test]
fn enum_to_int_cast_reads_only_the_discriminant() {
    expect_out(
        "enum Permission\n    Read\n    Write\n\n*main\n    p is Permission.Write\n    log(p as i64)\n    0\n",
        "1",
    );
}

#[test]
fn float_to_int_cast_saturates_instead_of_producing_poison() {
    expect_out(
        "*main\n    a is 1.0e300\n    b is 0.0 - 1.0e300\n    log(a as i64)\n    log(b as i64)\n    0\n",
        "9223372036854775807\n-9223372036854775808",
    );
}

#[test]
fn double_quoted_strings_support_escapes_and_interpolation() {
    expect_out(
        "*main\n    x is 5\n    log(\"a\\nb\")\n    log(\"val={x + 1}\")\n    0\n",
        "a\nb\nval=6",
    );
}

#[test]
fn a_constant_in_pattern_position_compares_instead_of_binding() {
    expect_out(
        "SPACE is 32\n\n*classify(c as i64) returns String\n    match c\n        SPACE ? 'space'\n        _ ? 'other'\n\n*main\n    log(classify(32))\n    log(classify(49))\n    0\n",
        "space\nother",
    );
}

#[test]
fn take_is_usable_as_a_function_name() {
    expect_out(
        "*take(x as i64) returns i64 is x * 2\n\n*main\n    log(take(21))\n    0\n",
        "42",
    );
}

#[test]
fn string_ordering_is_byte_lexicographic() {
    expect_out(
        "*main\n    log('a' < 'b')\n    log('a' < 'a')\n    log('b' < 'a')\n    log('ab' < 'b')\n    0\n",
        "1\n0\n0\n1",
    );
}

#[test]
fn value_struct_bind_copies() {
    expect_out(
        "type P\n    x as i64\n\n*main\n    p is P(x is 1)\n    q is p\n    q.x is 99\n    log(p.x)\n    log(q.x)\n    0\n",
        "1\n99",
    );
}

#[test]
fn borrowed_struct_parameter_mutation_reaches_the_caller_inside_a_loop() {
    expect_out(
        "type Bag\n    items as Vec of i64\n    n as i64\n\n*bump(b)\n    for i in 0 to 3\n        b.n is b.n + 1\n\n*main\n    g is Bag(items is vec(), n is 0)\n    bump(g)\n    log(g.n)\n    0\n",
        "3",
    );
}

#[test]
fn passing_a_struct_that_owns_a_vec_does_not_double_free() {
    expect_out(
        "type Bag\n    items as Vec of i64\n    n as i64\n\n*touch(b)\n    b.n is b.n + 1\n\n*main\n    g is Bag(items is vec(), n is 0)\n    touch(g)\n    log(g.n)\n    0\n",
        "1",
    );
}

#[test]
fn returning_a_closure_over_a_local_aggregate_is_rejected() {
    compile_fails_with(
        "*make_closure()\n    v is vec(10, 20, 30)\n    f is |x| v.get(x)\n    return f\n\n*main\n    g is make_closure()\n    log(g(0))\n",
        "captures the local",
    );
}

#[test]
fn inserting_the_same_aggregate_twice_is_rejected() {
    compile_fails_with(
        "*main\n    bucket is vec()\n    v is vec(1, 2, 3)\n    bucket.push(v)\n    bucket.push(v)\n    log(bucket.length)\n",
        "moved into a container",
    );
}

#[test]
fn reading_a_struct_whole_after_a_field_move_is_rejected() {
    compile_fails_with(
        "type Bag\n    items as Vec of i64\n    label as String\n\n*main\n    b is Bag(items is vec(1, 2, 3), label is 'x')\n    v is b.items\n    c is b\n    v.push(99)\n    log(c.items.length)\n",
        "cannot be read as a whole",
    );
}

#[test]
fn map_of_string_round_trips_its_values() {
    expect_out(
        "*main\n    m is map()\n    v is 'abc'\n    m.set('k', v)\n    log(m.get('k'))\n    log(m.get('k').length)\n    0\n",
        "abc\n3",
    );
}

#[test]
fn an_unhandled_error_in_main_exits_nonzero() {
    let src = "err E1\n    Bad\n\n*get(k as i64) returns i64 ! E1\n    if k < 0\n        err Bad\n    10 + k\n\n*main\n    get(0 - 1)\n    log(99)\n    0\n";
    let r = run(src, "0");
    assert!(
        !r.status.success(),
        "main returned success despite an unhandled error; stdout={:?}",
        r.stdout
    );
    assert!(
        !r.stdout.contains("99"),
        "main continued past the failing call: {:?}",
        r.stdout
    );
}

#[test]
fn a_dependency_url_may_not_escape_the_package_cache() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.jn"),
        "name is 'app'\nversion is '0.1.0'\nrequire('evil', 'https://localhost:1/../../../../TRAVERSAL_CANARY', '1.0.0')\n",
    )
    .unwrap();
    let home = dir.path().join("fakehome");
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(jinnc())
        .arg("fetch")
        .current_dir(dir.path())
        .env("HOME", &home)
        .output()
        .unwrap();
    let canary = dir.path().join("TRAVERSAL_CANARY");
    assert!(
        !canary.exists(),
        "package fetch created a directory outside the cache root"
    );
    let msg = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        msg.contains("not allowed") || msg.contains("cache root"),
        "traversal URL was not rejected with a clear message:\n{msg}"
    );
}

#[test]
fn jinn_bind_emits_parseable_jinn_for_real_headers() {
    let headers = [
        "/usr/include/errno.h",
        "/usr/include/string.h",
        "/usr/include/stdlib.h",
    ];
    let mut checked = 0;
    for h in headers {
        if !std::path::Path::new(h).exists() {
            continue;
        }
        let out = Command::new(jinnc()).arg("bind").arg(h).output().unwrap();
        assert!(
            out.status.success(),
            "jinn bind failed on {h}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut src = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            !src.contains("//"),
            "{h}: bind emitted C-style comments, which Jinn does not accept"
        );
        src.push_str("\n*main\n    0\n");
        let dir = tempfile::tempdir().unwrap();
        let jn = dir.path().join("bound.jn");
        std::fs::write(&jn, &src).unwrap();
        let c = Command::new(jinnc())
            .arg(&jn)
            .arg("--emit-hir")
            .output()
            .unwrap();
        assert!(
            c.status.success(),
            "bindings generated from {h} do not parse:\n{}",
            String::from_utf8_lossy(&c.stderr)
        );
        checked += 1;
    }
    assert!(checked > 0, "no system headers available to test against");
}
