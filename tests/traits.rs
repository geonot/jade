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

#[test]
fn impl_may_not_widen_the_trait_error_row() {
    let stderr = expect_compile_fail(
        "\
err FileError
    NotFound

err NetError
    Down

type F
    n as i64

trait Reader
    *read(self) returns i64 ! FileError

impl Reader for F
    *read(self) returns i64 ! NetError
        err NetError:Down

*main()
    f is F(n is 1)
    log(f.n)
",
    );
    assert!(
        stderr.contains("error row") && stderr.contains("NetError") && stderr.contains("FileError"),
        "diagnostic must name both rows: {stderr}"
    );
}

#[test]
fn impl_may_not_narrow_the_trait_error_row() {
    let stderr = expect_compile_fail(
        "\
err FileError
    NotFound

type F
    n as i64

trait Reader
    *read(self) returns i64 ! FileError

impl Reader for F
    *read(self) returns i64
        self.n

*main()
    f is F(n is 1)
    log(f.n)
",
    );
    assert!(
        stderr.contains("error row") && stderr.contains("(none)"),
        "diagnostic must show the narrowed row: {stderr}"
    );
}

#[test]
fn conforming_error_row_is_accepted() {
    expect(
        "\
err FileError
    NotFound

type F
    n as i64

trait Reader
    *read(self) returns i64 ! FileError

impl Reader for F
    *read(self) returns i64 ! FileError
        if self.n < 0
            err FileError:NotFound
        self.n

*main()
    f is F(n is 7)
    r is f.read()
    log(7)
",
        "7",
    );
}

#[test]
fn impl_return_type_must_match_the_trait() {
    let stderr = expect_compile_fail(
        "\
type F
    n as i64

trait Sized2
    *size(self) returns i64

impl Sized2 for F
    *size(self) returns String
        'big'

*main()
    f is F(n is 1)
    log(f.n)
",
    );
    assert!(
        stderr.contains("returns") && stderr.contains("trait"),
        "diagnostic must name the mismatched return: {stderr}"
    );
}

#[test]
fn impl_param_count_must_match_the_trait() {
    let stderr = expect_compile_fail(
        "\
type F
    n as i64

trait Adder
    *add_to(self, x as i64) returns i64

impl Adder for F
    *add_to(self) returns i64
        self.n

*main()
    f is F(n is 1)
    log(f.n)
",
    );
    assert!(
        stderr.contains("parameter") && stderr.contains("trait"),
        "diagnostic must flag the arity mismatch: {stderr}"
    );
}
