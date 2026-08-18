use std::path::PathBuf;
use std::process::{Command, Output};

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

struct Compiled {
    dir: tempfile::TempDir,
    out: Output,
}

impl Compiled {
    fn ok(&self) -> bool {
        self.out.status.success()
    }
    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.out.stderr).into_owned()
    }
    fn run(&self) -> Output {
        Command::new(self.dir.path().join("prog.bin"))
            .current_dir(self.dir.path())
            .output()
            .expect("run compiled program")
    }
}

fn compile(src: &str) -> Compiled {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("prog.jn");
    std::fs::write(&file, src).unwrap();
    let out = Command::new(jinnc())
        .arg("prog.jn")
        .arg("-o")
        .arg(dir.path().join("prog.bin"))
        .current_dir(dir.path())
        .output()
        .expect("invoke jinnc");
    Compiled { dir, out }
}

fn accepts_and_prints(src: &str, expected: &str) {
    let c = compile(src);
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(
        run.status.success(),
        "must run clean, got {:?}\nstderr: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        expected.trim(),
        "source:\n{src}"
    );
}

fn rejects(src: &str, needles: &[&str]) {
    let c = compile(src);
    assert!(!c.ok(), "must not compile:\n{src}");
    let stderr = c.stderr();
    for n in needles {
        assert!(stderr.contains(n), "missing {n:?} in:\n{stderr}");
    }
}

#[test]
fn generic_take_param_moves_the_argument() {
    rejects(
        "*consume(v as take Vec of T) returns i64\n    v.length\n\n*main\n    a is vec(1, 2, 3)\n    n is consume(a)\n    log(a.length)\n",
        &["use of moved value `a`", "takes ownership"],
    );
}

#[test]
fn nonexhaustive_i32_match_is_rejected() {
    rejects(
        "*classify(n as i32) returns i64\n    match n\n        0 ? 10\n        1 ? 20\n\n*main\n    log(classify(0))\n",
        &["non-exhaustive match on `i32`", "missing _"],
    );
}

#[test]
fn nonexhaustive_tuple_match_is_rejected() {
    rejects(
        "*pick(a as i64, b as i64) returns i64\n    match (a, b)\n        (0, 0) ? 1\n        (1, 1) ? 2\n\n*main\n    log(pick(0, 0))\n",
        &["non-exhaustive match on `(i64, i64)`", "missing (_, _)"],
    );
}

#[test]
fn generic_enum_variant_instantiates_from_argument_type() {
    accepts_and_prints(
        "enum Opt of T\n    Som(T)\n    Non\n\n*main\n    a as Opt of i64 is Non\n    b is Som('hi there')\n    match b\n        Som(v) ? log(v)\n        Non ? log('none')\n",
        "hi there",
    );
}

#[test]
fn container_element_type_mismatch_is_rejected() {
    rejects(
        "*total(xs as Vec of i64) returns i64\n    xs.length\n\n*main\n    ys is vec(1.5, 2.5)\n    log(total(ys))\n",
        &["wrong element type", "must match exactly"],
    );
}

#[test]
fn numeric_narrowing_argument_is_rejected() {
    rejects(
        "*small(x as i8) returns i64\n    x as i64\n\n*main\n    big is 300 as i64\n    log(small(big))\n",
        &["wrong type", "expected `i8`", "found `i64`"],
    );
}

#[test]
fn assigning_a_loop_counter_is_rejected() {
    rejects(
        "*main\n    for i in 0 to 5\n        i is i + 10\n        log(i)\n",
        &["cannot assign to loop counter `i`"],
    );
}

#[test]
fn returning_an_enum_payload_runs_clean() {
    accepts_and_prints(
        "enum Box\n    Full(Vec of i64)\n    Empty\n\n*unwrap(b as Box) returns i64\n    match b\n        Full(v) ? v.length\n        Empty ? 0\n\n*main\n    x is Full(vec(1, 2, 3))\n    log(unwrap(x))\n",
        "3",
    );
}

#[test]
fn resource_drop_runs_on_early_return() {
    accepts_and_prints(
        "type Guard @resource\n    id as i64\n\n    *drop\n        log('DROP')\n        log(self.id)\n\n*run(early as bool) returns i64\n    g is Guard(id is 100)\n    if early\n        return 1\n    2\n\n*main\n    log(run(true))\n",
        "DROP\n100\n1",
    );
}

#[test]
fn enum_i64_payload_roundtrips() {
    accepts_and_prints(
        "enum Tag\n    Small(i8)\n    Big(i64)\n\n*val(t as Tag) returns i64\n    match t\n        Small(n) ? n as i64\n        Big(n) ? n\n\n*main\n    a is Big(9223372036854775807)\n    log(val(a))\n",
        "9223372036854775807",
    );
}

#[test]
fn chained_comparison_with_side_effect_is_rejected() {
    rejects(
        "*bump() returns i64\n    5\n\n*main\n    if 0 < bump() < 10\n        log(1)\n",
        &["chained comparison would evaluate the middle operand twice"],
    );
}

#[test]
fn chained_comparison_over_a_pure_operand_still_works() {
    accepts_and_prints(
        "*main\n    n is 5\n    if 0 < n < 10\n        log(1)\n",
        "1",
    );
}

#[test]
fn a_second_statement_on_a_binding_line_is_rejected() {
    rejects(
        "*main\n    x is 1 y is 2\n    log(x)\n",
        &["unexpected token after binding"],
    );
}
