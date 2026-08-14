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
fn binding_a_view_locks_the_root_for_the_block() {
    accepts_and_prints(
        "*main\n    xs is vector(10, 20, 30, 40)\n    v is xs.view(1, 3)\n    log(v.length)\n    log(v.get(0))\n    total is 0\n    for x in v\n        total is total + x\n    log(total)\n",
        "2\n20\n50",
    );
    rejects(
        "*main\n    xs is vector(1, 2, 3)\n    v is xs.view(0, 2)\n    xs.push(4)\n    log(v.length)\n",
        &["cannot call `push` on `xs`", "the view bound at"],
    );
    rejects(
        "*main\n    xs is vector(1, 2, 3)\n    v is xs.view(0, 2)\n    ys is xs\n    log(v.length)\n",
        &["cannot move `xs`", "the view bound at"],
    );
}

#[test]
fn view_bind_dies_with_its_block_and_unlocks_the_root() {
    accepts_and_prints(
        "*main\n    xs is vector(1, 2, 3)\n    if true\n        v is xs.view(0, 2)\n        log(v.length)\n    xs.push(4)\n    log(xs.length)\n",
        "2\n4",
    );
}

#[test]
fn view_bind_of_a_temporary_is_rejected() {
    rejects(
        "*main\n    v is vector(1, 2).view(0, 1)\n    log(v.length)\n",
        &["view of a temporary"],
    );
}

#[test]
fn view_alias_bind_inherits_the_root_lock() {
    accepts_and_prints(
        "*total(v as View of i64) returns i64\n    t is 0\n    for x in v\n        t is t + x\n    t\n\n*main\n    xs is vector(1, 2, 3, 4)\n    v is xs.view(1, 3)\n    w is v\n    log(total(w))\n    v is xs.view(0, 2)\n    log(total(v))\n",
        "5\n3",
    );
}

#[test]
fn bound_view_of_frozen_data_reads() {
    accepts_and_prints(
        "*main\n    xs is vector(1, 2, 3)\n    fz is freeze xs\n    v is fz.view(0, 2)\n    log(v.length)\n",
        "2",
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

#[test]
fn views_lending_iteration_binds_element_views() {
    accepts_and_prints(
        "type Point\n    x as i64\n    y as i64\n\n*main\n    pts is vector(Point(x is 1, y is 2), Point(x is 3, y is 4))\n    total is 0\n    for p in pts.views()\n        total is total + p.x\n    log(total)\n",
        "4",
    );
    rejects(
        "*main\n    xs is vector(1, 2, 3)\n    for v in xs.views()\n        xs.push(9)\n",
        &["cannot call `push` on `xs`", "is iterating it"],
    );
    rejects(
        "*main\n    xs is vector(1, 2)\n    v is xs.views()\n    log(1)\n",
        &["no method 'views' on Vec"],
    );
}

#[test]
fn field_reads_through_element_views_are_zero_copy_reads() {
    accepts_and_prints(
        "type Point\n    x as i64\n    y as i64\n\n*main\n    pts is vector(Point(x is 1, y is 2), Point(x is 3, y is 4))\n    log(pts.at_view(1).x)\n    v is pts.at_view(0)\n    log(v.y)\n",
        "3\n2",
    );
}

#[test]
fn view_cannot_be_stored_through_an_inferred_struct_field() {
    rejects(
        "type Holder\n    w\n\n*main\n    xs is vector(1, 2, 3)\n    h is Holder(w is xs.view(0, 2))\n    log(1)\n",
        &["a view cannot be stored in a constructed value"],
    );
}

#[test]
fn moving_a_field_out_of_a_view_is_rejected() {
    rejects(
        "type Bag\n    items as Vec of i64\n\n*main\n    bags is vector(Bag(items is vector(1, 2)))\n    v is bags.at_view(0)\n    stolen is v.items\n    log(stolen.length)\n",
        &["cannot move `v.items` out of `v`", "borrowed window"],
    );
}

#[test]
fn readonly_method_calls_through_element_views() {
    accepts_and_prints(
        "type Point\n    x as i64\n    y as i64\n\n    *norm2 returns i64\n        self.x * self.x + self.y * self.y\n\n*main\n    pts is vector(Point(x is 3, y is 4), Point(x is 1, y is 2))\n    log(pts.at_view(0).norm2())\n    total is 0\n    for p in pts.views()\n        total is total + p.norm2()\n    log(total)\n",
        "25\n30",
    );
    rejects(
        "type Point\n    x as i64\n\n    *bump\n        self.x is self.x + 1\n\n*main\n    pts is vector(Point(x is 3))\n    pts.at_view(0).bump()\n",
        &[
            "cannot call `bump` through a view",
            "read-only borrowed window",
        ],
    );
}
