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

fn expect(src: &str, expected: &str) {
    let got = compile_and_run(src);
    assert_eq!(got.trim(), expected.trim(), "source:\n{src}");
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

#[test]
fn vec_param_borrows_and_mutates_in_place() {
    expect(
        "\
*push_one(v as Vec of i64)
    v.push(1)

*main
    xs is [1, 2, 3]
    push_one(xs)
    log(xs.len())
",
        "4",
    );
}

#[test]
fn vec_copy_param_is_independent() {
    expect(
        "\
*grow(v as copy Vec of i64)
    v.push(99)
    v.push(99)

*main
    xs is [1, 2, 3]
    grow(xs)
    log(xs.len())
",
        "3",
    );
}

#[test]
fn string_param_borrows_caller_retains() {
    expect(
        "\
*shout(s as String)
    log(s.len())

*main
    name is 'hello'
    shout(name)
    log(name.len())
",
        "5\n5",
    );
}

#[test]
fn string_take_moves_use_after_is_error() {
    let err = expect_compile_fail(
        "\
*consume(s as take String)
    log(s.len())

*main
    name is 'this-is-a-heap-string'
    consume(name)
    log(name.len())
",
    );
    assert!(
        err.contains("move") || err.contains("moved") || err.contains("consumed"),
        "expected use-after-move diagnostic, got: {err}"
    );
}

#[test]
fn struct_copy_is_deep_independent() {
    expect(
        "\
type Bag
    items as Vec of i64

*main
    a is Bag(items is [1, 2, 3])
    b is copy a
    b.items.push(9)
    b.items.push(9)
    log(a.items.len())
    log(b.items.len())
",
        "3\n5",
    );
}

#[test]
fn struct_field_take_preserves_siblings() {
    expect(
        "\
type Pair
    a as Vec of i64
    b as Vec of i64

*main
    p is Pair(a is [1, 2, 3], b is [4, 5, 6, 7])
    taken is take p.a
    log(taken.len())
    log(p.b.len())
",
        "3\n4",
    );
}

#[test]
fn enum_payload_value_semantics() {
    expect(
        "\
enum Shape
    Circle(i64)
    Rect(i64, i64)

*area(s as Shape) returns i64
    match s
        Circle(r) ? r * r
        Rect(w, h) ? w * h

*main
    log(area(Circle(5)))
    log(area(Rect(3, 7)))
",
        "25\n21",
    );
}

#[test]
fn resource_copy_is_compile_error() {
    let err = expect_compile_fail(
        "\
type Handle @resource
    fd as i32

*main
    h is Handle(fd is 3)
    h2 is copy h
    log(h2.fd)
",
    );
    assert!(
        err.contains("resource") || err.contains("copy") || err.contains("linear"),
        "expected linear-resource copy diagnostic, got: {err}"
    );
}

#[test]
fn resource_drop_runs_once_at_scope_exit() {
    expect(
        "\
type Guard @resource
    id as i64

    *drop
        log(self.id)

*main
    g is Guard(id is 7)
    log(100)
",
        "100\n7",
    );
}

#[test]
fn take_then_reassign_is_ok() {
    expect(
        "\
*consume(s as take String)
    log(s.len())

*main
    name is 'hello world here'
    consume(name)
    name is 'rebound now ok!'
    log(name.len())
",
        "16\n15",
    );
}

#[test]
fn field_take_use_after_is_error() {
    let err = expect_compile_fail(
        "\
type Bag
    items as Vec of i64

*main
    a is Bag(items is [1, 2, 3])
    moved is take a.items
    log(moved.len())
    log(a.items.len())
",
    );
    assert!(
        err.contains("move") || err.contains("moved"),
        "expected field use-after-move diagnostic, got: {err}"
    );
}
