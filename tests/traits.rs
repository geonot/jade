//! Conformance suite for Jinn's bounded polymorphism (traits / protocols).
//!
//! Pins the documented semantics of the trait system:
//!   * direct method dispatch through an `impl Trait for T`;
//!   * generic dispatch through a trait bound (`of T: Trait`);
//!   * trait-bound enforcement diagnostics at the call site;
//!   * multi-bound parameters (`of T: A + B`);
//!   * default trait method bodies (synthesized into impls that omit them);
//!   * default methods overridden by an explicit impl method.

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
fn direct_dispatch_through_impl() {
    expect(
        "\
type Point
    x as i64
    y as i64

trait Printable
    *display(self) returns String

impl Printable for Point
    *display(self)
        '({self.x}, {self.y})'

*main()
    p is Point(x is 1, y is 2)
    log(p.display())
",
        "(1, 2)",
    );
}

#[test]
fn generic_dispatch_through_bound() {
    expect(
        "\
type Point
    x as i64
    y as i64

trait Printable
    *display(self) returns String

impl Printable for Point
    *display(self)
        '({self.x}, {self.y})'

*show of T: Printable(x as T) returns String
    x.display()

*main()
    p is Point(x is 3, y is 4)
    log(show(p))
",
        "(3, 4)",
    );
}

#[test]
fn bound_enforcement_rejects_non_conforming_type() {
    let stderr = expect_compile_fail(
        "\
type Bare
    n as i64

trait Printable
    *display(self) returns String

*show of T: Printable(x as T) returns String
    x.display()

*main()
    log(show(Bare(n is 1)))
",
    );
    assert!(
        stderr.contains("does not satisfy trait bound `Printable`"),
        "missing trait-bound diagnostic, got:\n{stderr}"
    );
}

#[test]
fn multi_bound_parameter() {
    expect(
        "\
type Dog
    age as i64

trait Named
    *name(self) returns String

trait Aged
    *years(self) returns i64

impl Named for Dog
    *name(self)
        'rex'

impl Aged for Dog
    *years(self)
        self.age

*describe of T: Named + Aged(x as T) returns String
    '{x.name()} is {x.years()}'

*main()
    d is Dog(age is 5)
    log(describe(d))
",
        "rex is 5",
    );
}

#[test]
fn default_method_body_dispatches() {
    expect(
        "\
type Point
    x as i64
    y as i64

trait Greeter
    *name(self) returns String
    *hello(self) returns String is 'hi, {self.name()}'

impl Greeter for Point
    *name(self)
        'point'

*main()
    p is Point(x is 1, y is 2)
    log(p.hello())
",
        "hi, point",
    );
}

#[test]
fn default_method_overridden_by_impl() {
    expect(
        "\
type Dog
    age as i64

trait Speaker
    *sound(self) returns String
    *speak(self) returns String is 'default: {self.sound()}'

impl Speaker for Dog
    *sound(self)
        'woof'
    *speak(self)
        'overridden: {self.sound()}'

*main()
    d is Dog(age is 3)
    log(d.speak())
",
        "overridden: woof",
    );
}

#[test]
fn default_method_through_bound() {
    expect(
        "\
type Cat
    lives as i64

trait Greeter
    *name(self) returns String
    *hello(self) returns String is 'hi, {self.name()}'

impl Greeter for Cat
    *name(self)
        'cat'

*greet of T: Greeter(x as T) returns String
    x.hello()

*main()
    c is Cat(lives is 9)
    log(greet(c))
",
        "hi, cat",
    );
}
