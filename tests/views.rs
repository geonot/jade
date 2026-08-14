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
fn view_creation_get_length_and_iteration() {
    accepts_and_prints(
        "*main\n    xs is vector(10, 20, 30, 40)\n    log(xs.view(1, 3).length)\n    log(xs.view(1, 3).get(0))\n    log(xs.at_view(2).get(0))\n    total is 0\n    for x in xs.view(0, 4)\n        total is total + x\n    log(total)\n",
        "2\n20\n30\n100",
    );
}

#[test]
fn view_parameter_accepts_whole_vec_and_subslice() {
    accepts_and_prints(
        "*total(v as View of i64) returns i64\n    t is 0\n    for x in v\n        t is t + x\n    t\n\n*main\n    xs is vector(1, 2, 3, 4)\n    log(total(xs))\n    log(total(xs.view(1, 3)))\n",
        "10\n5",
    );
}

#[test]
fn view_flows_down_into_nested_calls() {
    accepts_and_prints(
        "*inner(v as View of i64) returns i64\n    v.get(0)\n\n*outer(v as View of i64) returns i64\n    inner(v) + v.length\n\n*main\n    xs is vector(5, 6, 7)\n    log(outer(xs.view(1, 3)))\n",
        "8",
    );
}

#[test]
fn string_view_is_a_byte_window() {
    accepts_and_prints(
        "*count_bytes(v as View of u8) returns i64\n    v.length\n\n*main\n    s is 'hello'\n    log(count_bytes(s.view(1, 4)))\n",
        "3",
    );
}

#[test]
fn frozen_data_can_be_viewed() {
    accepts_and_prints(
        "*total(v as View of i64) returns i64\n    t is 0\n    for x in v\n        t is t + x\n    t\n\n*main\n    xs is vector(1, 2, 3, 4)\n    fz is freeze xs\n    log(total(fz.view(1, 3)))\n",
        "5",
    );
}

#[test]
fn out_of_range_view_traps_at_runtime() {
    let c = compile("*main\n    xs is vector(1, 2, 3)\n    log(xs.view(1, 9).length)\n");
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(!run.status.success(), "out-of-range view must trap");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("view range out of bounds"),
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn binding_a_view_is_rejected() {
    rejects(
        "*main\n    xs is vector(1, 2, 3)\n    v is xs.view(0, 2)\n    log(v.length)\n",
        &["a view cannot be bound", "lives only within its statement"],
    );
}

#[test]
fn returning_a_view_is_rejected() {
    rejects(
        "*head(xs as Vec of i64) returns View of i64\n    xs.view(0, 1)\n\n*main\n    xs is vector(1, 2)\n    log(head(xs).length)\n",
        &["returns a view", "second-class borrow"],
    );
}

#[test]
fn storing_a_view_in_a_struct_field_is_rejected() {
    rejects(
        "type Holder\n    w as View of i64\n\n*main\n    log(1)\n",
        &["a view cannot be stored in a struct field"],
    );
}

#[test]
fn capturing_a_view_in_a_task_is_rejected() {
    rejects(
        "*scan(v as View of i64) returns i64\n    together\n        dispatch\n            log(v.length)\n    v.length\n\n*main\n    xs is vector(1, 2)\n    log(scan(xs))\n",
        &["cannot be captured by a task", "borrows memory"],
    );
}

#[test]
fn nested_view_in_parameter_type_is_rejected() {
    rejects(
        "*keep(vs as Vec of View of i64) returns i64\n    vs.length\n\n*main\n    log(keep(vector()))\n",
        &["a view cannot appear in"],
    );
}

#[test]
fn view_get_bounds_check_traps() {
    let c = compile("*main\n    xs is vector(1, 2, 3)\n    log(xs.view(0, 2).get(5))\n");
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(!run.status.success(), "oob get through view must trap");
}

#[test]
fn view_of_strings_copies_elements_out() {
    accepts_and_prints(
        "*first_of(v as View of String) returns String\n    v.get(0)\n\n*main\n    names is vector('ada', 'brin')\n    log(first_of(names.view(0, 2)))\n    log(names.length)\n",
        "ada\n2",
    );
}
