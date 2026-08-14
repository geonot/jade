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
fn frozen_vec_supports_every_read() {
    accepts_and_prints(
        "*main\n    xs is vector(1, 2, 3)\n    fz is freeze xs\n    log(fz.length)\n    log(fz.sum())\n    total is 0\n    for x in fz\n        total is total + x\n    log(total)\n",
        "3\n6\n6",
    );
}

#[test]
fn freeze_consumes_its_operand() {
    rejects(
        "*main\n    xs is vector(1, 2, 3)\n    fz is freeze xs\n    log(xs.length)\n",
        &["used after being frozen", "no thaw"],
    );
}

#[test]
fn freezing_twice_is_rejected() {
    rejects(
        "*main\n    xs is vector(1)\n    fz is freeze xs\n    zz is freeze fz\n    log(zz.length)\n",
        &["already frozen"],
    );
}

#[test]
fn freeze_of_an_rvalue_works() {
    accepts_and_prints(
        "*main\n    fz is freeze vector(1, 2)\n    log(fz.length)\n",
        "2",
    );
}

#[test]
fn frozen_struct_reads_fields_and_readonly_methods() {
    accepts_and_prints(
        "type Config\n    name as String\n    retries as i64\n    hosts as Vec of String\n\n    *describe returns String\n        self.name\n\n*show(c as Config) returns i64\n    c.retries\n\n*main\n    cfg is Config(name is 'prod', retries is 3, hosts is vector('a', 'b'))\n    fz is freeze cfg\n    log(fz.name)\n    log(fz.retries)\n    log(fz.hosts.length)\n    log(fz.describe())\n    log(show(fz))\n",
        "prod\n3\n2\nprod\n3",
    );
}

#[test]
fn builtin_mutating_method_on_frozen_is_rejected() {
    rejects(
        "*main\n    xs is vector(1, 2, 3)\n    fz is freeze xs\n    fz.push(4)\n",
        &[
            "is frozen",
            "`push` mutates its receiver",
            "never be written",
        ],
    );
}

#[test]
fn user_mutating_method_on_frozen_is_rejected() {
    rejects(
        "type Config\n    retries as i64\n\n    *bump\n        self.retries is self.retries + 1\n\n*main\n    cfg is Config(retries is 3)\n    fz is freeze cfg\n    fz.bump()\n",
        &["is frozen", "`bump` mutates its receiver"],
    );
}

#[test]
fn field_assignment_through_frozen_is_rejected() {
    rejects(
        "type Config\n    retries as i64\n\n*main\n    cfg is Config(retries is 3)\n    fz is freeze cfg\n    fz.retries is 9\n",
        &["cannot assign through `fz`", "frozen"],
    );
}

#[test]
fn moving_a_part_out_of_frozen_is_rejected() {
    rejects(
        "type Bag\n    items as Vec of i64\n\n*main\n    b is Bag(items is vector(1, 2))\n    fz is freeze b\n    v is fz.items\n    log(v.length)\n",
        &["cannot move `fz.items` out of `fz`", "frozen", "copy"],
    );
}

#[test]
fn copying_a_part_out_of_frozen_is_allowed() {
    accepts_and_prints(
        "type Bag\n    items as Vec of i64\n\n*main\n    b is Bag(items is vector(1, 2))\n    fz is freeze b\n    v is copy fz.items\n    log(v.length)\n",
        "2",
    );
}

#[test]
fn passing_frozen_to_a_mutating_parameter_is_rejected() {
    rejects(
        "*grow(v as Vec of i64)\n    v.push(9)\n\n*main\n    xs is vector(1, 2)\n    fz is freeze xs\n    grow(fz)\n",
        &["is frozen", "mutates it", "never be written"],
    );
}

#[test]
fn passing_frozen_to_a_readonly_parameter_works() {
    accepts_and_prints(
        "*total(v as Vec of i64) returns i64\n    v.sum()\n\n*main\n    xs is vector(1, 2, 3)\n    fz is freeze xs\n    log(total(fz))\n",
        "6",
    );
}

#[test]
fn frozen_parameter_annotation_demands_immutability() {
    accepts_and_prints(
        "*use_frozen(c as Frozen of Vec of i64) returns i64\n    c.sum()\n\n*main\n    xs is vector(1, 2, 3)\n    fz is freeze xs\n    log(use_frozen(fz))\n",
        "6",
    );
}

#[test]
fn frozen_struct_field_annotation_works() {
    accepts_and_prints(
        "type App\n    cfg as Frozen of Vec of i64\n\n*main\n    xs is vector(1, 2, 3)\n    app is App(cfg is freeze xs)\n    log(app.cfg.length)\n",
        "3",
    );
}

#[test]
fn freeze_of_a_borrowed_loop_binder_is_rejected() {
    rejects(
        "*main\n    grid is vector(vector(1), vector(2))\n    for row in grid\n        fz is freeze row\n        log(fz.length)\n",
        &["cannot freeze `row`", "borrow"],
    );
}

#[test]
fn freeze_of_an_unannotated_parameter_makes_it_consuming() {
    accepts_and_prints(
        "*seal(v as Vec of i64) returns i64\n    fz is freeze v\n    fz.length\n\n*main\n    xs is vector(1, 2)\n    log(seal(xs))\n",
        "2",
    );
    rejects(
        "*seal(v as Vec of i64) returns i64\n    fz is freeze v\n    fz.length\n\n*main\n    xs is vector(1, 2)\n    log(seal(xs))\n    log(xs.length)\n",
        &["use of moved value `xs`", "seal"],
    );
}

#[test]
fn freeze_of_a_resource_type_is_rejected() {
    rejects(
        "type File @resource\n    handle as i64\n\n    *drop\n        nop\n\n*main\n    f is File(handle is 3)\n    fz is freeze f\n    log(fz.handle)\n",
        &["cannot freeze", "@resource"],
    );
}

#[test]
fn frozen_value_moves_into_a_dispatch_task() {
    accepts_and_prints(
        "*worker(cfg as Vec of i64, id as i64) returns i64\n    cfg.sum() + id\n\n*main\n    xs is vector(1, 2, 3)\n    fz is freeze xs\n    together\n        dispatch\n            log(worker(fz, 1))\n",
        "7",
    );
}

#[test]
fn ctor_capture_of_a_bound_vec_does_not_double_free() {
    accepts_and_prints(
        "type App\n    cfg as Vec of i64\n\n*main\n    xs is vector(1, 2, 3)\n    app is App(cfg is xs)\n    log(app.cfg.length)\n",
        "3",
    );
}

#[test]
fn vec_literal_capture_of_a_bound_vec_does_not_double_free() {
    accepts_and_prints(
        "*main\n    xs is vector(1, 2, 3)\n    nested is vector(xs)\n    log(nested.length)\n",
        "1",
    );
}
