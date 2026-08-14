use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

struct Out {
    dir: tempfile::TempDir,
    out: std::process::Output,
}

fn compile(src: &str) -> Out {
    compile_args(src, &[])
}

fn compile_args(src: &str, extra: &[&str]) -> Out {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("prog.jn"), src).unwrap();
    let out = Command::new(jinnc())
        .arg("prog.jn")
        .args(extra)
        .arg("-o")
        .arg(dir.path().join("prog.bin"))
        .current_dir(dir.path())
        .output()
        .expect("invoke jinnc");
    Out { dir, out }
}

impl Out {
    fn ok(&self) -> bool {
        self.out.status.success()
    }
    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.out.stderr).into_owned()
    }
    fn run_stdout(&self) -> String {
        let r = Command::new(self.dir.path().join("prog.bin"))
            .current_dir(self.dir.path())
            .output()
            .expect("run");
        String::from_utf8_lossy(&r.stdout).into_owned()
    }
    fn warning_free(&self) -> bool {
        !self.stderr().contains("warning:")
    }
}

fn assert_clean_run(src: &str, expect: &str) {
    let c = compile(src);
    assert!(c.ok(), "{}", c.stderr());
    assert!(
        c.warning_free(),
        "must compile without defaulting warnings: {}",
        c.stderr()
    );
    assert_eq!(c.run_stdout(), expect);
}

#[test]
fn d2_peek_unannotated() {
    assert_clean_run(
        "*peek(v)\n    t is v.get(0)\n    log(t)\n\n*main\n    xs is vec(7, 8)\n    peek(xs)\n",
        "7\n",
    );
}

#[test]
fn d2_peek_string_element_stays_string() {
    assert_clean_run(
        "*peek(v)\n    t is v.get(0)\n    log(t)\n\n*main\n    xs is vec('hi', 'yo')\n    peek(xs)\n",
        "hi\n",
    );
}

#[test]
fn d2_mixed_instantiations() {
    assert_clean_run(
        "*peek(v)\n    t is v.get(0)\n    log(t)\n\n*main\n    peek(vec(42))\n    peek(vec('s'))\n",
        "42\ns\n",
    );
}

#[test]
fn d2_bsort_unannotated() {
    assert_clean_run(
        "*bsort(v)\n    n is v.length\n    for i in 0 to n\n        for j in 0 to n - i - 1\n            if v.get(j) > v.get(j + 1)\n                tmp is v.get(j)\n                v.set(j, v.get(j + 1))\n                v.set(j + 1, tmp)\n\n*main\n    xs is vec(3, 1, 2)\n    bsort(xs)\n    log(xs.get(0))\n    log(xs.get(2))\n",
        "1\n3\n",
    );
}

#[test]
fn d2_lib_artifact_requires_annotation_for_uncalled_generic() {
    let c = compile_args("*peek(v)\n    t is v.get(0)\n    log(t)\n", &["--lib"]);
    assert!(!c.ok());
    let stderr = c.stderr();
    assert!(
        stderr.contains("`peek`")
            && stderr.contains("without a call site")
            && stderr.contains("trait bound"),
        "{stderr}"
    );
}

#[test]
fn d2_fixit_text_is_valid_jinn() {
    let c = compile("*main\n    v is vec()\n    log(v.length)\n");
    assert!(c.ok(), "{}", c.stderr());
    let stderr = c.stderr();
    assert!(
        !stderr.contains("`: i64`")
            && !stderr.contains("`: f64`")
            && !stderr.contains("`: String`"),
        "fix-it suggests invalid Jinn: {stderr}"
    );
}

#[test]
fn annotation_binds_phantom_type_param() {
    assert_clean_run(
        "type Box of T\n    tag as i64\n\n*main\n    b as Box<string> is Box(tag is 5)\n    log(b.tag)\n",
        "5\n",
    );
}

#[test]
fn phantom_type_param_default_warns() {
    let c =
        compile("type Box of T\n    tag as i64\n\n*main\n    b is Box(tag is 5)\n    log(b.tag)\n");
    assert!(c.ok(), "{}", c.stderr());
    assert!(
        c.stderr().contains("not fixed by any constructor argument"),
        "a phantom type parameter must warn when it defaults: {}",
        c.stderr()
    );
    assert_eq!(c.run_stdout(), "5\n");
}

#[test]
fn bind_annotation_flows_into_return_position_generic() {
    assert_clean_run(
        "*empty of T() returns Vec of T\n    v is vector()\n    return v\n\n*main\n    xs as Vec of string is empty()\n    xs.push('heap-string-well-past-sso-width')\n    log(xs.get(0))\n",
        "heap-string-well-past-sso-width\n",
    );
}

#[test]
fn of_form_supplies_function_type_args() {
    assert_clean_run(
        "*empty of T() returns Vec of T\n    v is vector()\n    return v\n\n*main\n    ys is empty of string()\n    ys.push('explicit-spelling-heap-payload')\n    log(ys.get(0))\n",
        "explicit-spelling-heap-payload\n",
    );
}

#[test]
fn declared_return_flows_into_return_position_generic() {
    assert_clean_run(
        "*empty of T() returns Vec of T\n    v is vector()\n    return v\n\n*make() returns Vec of string\n    return empty()\n\n*main\n    xs is make()\n    xs.push('via-declared-return')\n    log(xs.get(0))\n",
        "via-declared-return\n",
    );
}

#[test]
fn mismatched_return_annotation_still_rejects() {
    let c = compile(
        "*empty_ints of T() returns Vec of i64\n    v is vector()\n    return v\n\n*main\n    xs as Vec of string is empty_ints()\n    log(xs.length)\n",
    );
    assert!(
        !c.ok(),
        "a bind annotation must not silently override a concrete declared return"
    );
}

#[test]
fn generic_enum_unit_variant_binds_via_annotation() {
    assert_clean_run(
        "enum Maybe of T\n    Just(T)\n    Nothing\n\n*grab(m as Maybe of string) returns string\n    match m\n        Just(v) ? v\n        Nothing ? 'none'\n\n*main\n    a as Maybe of string is Nothing\n    log(grab(a))\n    b as Maybe of string is Just('heap-string-past-sso-length')\n    log(grab(b))\n",
        "none\nheap-string-past-sso-length\n",
    );
}

#[test]
fn phantom_receiver_methods_type_correctly() {
    assert_clean_run(
        "type Tag of T\n    label as string\n\n    *describe(self) returns string\n        self.label\n\n*main\n    t as Tag<string> is Tag(label is 'hello-from-the-heap-not-sso')\n    log(t.describe())\n",
        "hello-from-the-heap-not-sso\n",
    );
    let c = compile(
        "type Tag of T\n    label as string\n\n    *describe(self) returns string\n        self.label\n\n*main\n    u is Tag(label is 'world-heap-payload-string')\n    log(u.describe())\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    assert_eq!(
        c.run_stdout(),
        "world-heap-payload-string\n",
        "a method call through a defaulted phantom receiver must read the real field, \
         not reinterpret it at the fallback layout"
    );
}

#[test]
fn unknown_type_in_uncalled_signature_rejects() {
    let c = compile("*unused(x as Zorp of A)\n    log(1)\n\n*main\n    log(2)\n");
    assert!(
        !c.ok(),
        "an undefined generic base in an uncalled signature must reject"
    );
    assert!(c.stderr().contains("unknown type `Zorp`"), "{}", c.stderr());
}

#[test]
fn unknown_type_in_field_rejects() {
    let c = compile("type Wob\n    f as Zorp\n\n*main\n    log(2)\n");
    assert!(
        !c.ok(),
        "an undefined type in a field annotation must reject"
    );
    assert!(c.stderr().contains("unknown type `Zorp`"), "{}", c.stderr());
}

#[test]
fn generic_arity_mismatch_in_annotation_rejects() {
    let c = compile(
        "type Pair of A, B\n    first as A\n    second as B\n\n*foo(p as Pair of i64)\n    log(p.first)\n\n*main\n    log(1)\n",
    );
    assert!(!c.ok());
    assert!(
        c.stderr().contains("declares 2 type parameter(s)"),
        "{}",
        c.stderr()
    );
}

#[test]
fn non_generic_type_with_args_rejects() {
    let c = compile(
        "type Point\n    x as i64\n\n*foo(p as Point of i64)\n    log(p.x)\n\n*main\n    log(1)\n",
    );
    assert!(!c.ok());
    assert!(
        c.stderr().contains("takes no type arguments"),
        "{}",
        c.stderr()
    );
}

#[test]
fn map_non_string_key_annotation_names_the_runtime_rule() {
    let c = compile("*main\n    m as Map<i64, string> is map()\n    log(1)\n");
    assert!(!c.ok());
    assert!(
        c.stderr().contains("map keys are strings"),
        "the diagnostic must state the runtime rule, not a bare unification mismatch: {}",
        c.stderr()
    );
}

#[test]
fn map_of_sugar_stays_string_keyed() {
    assert_clean_run(
        "*main\n    m as Map of i64 is map()\n    m.set('a', 7)\n    log(m.get('a'))\n",
        "7\n",
    );
}

#[test]
fn resolved_later_bind_emits_no_stale_default_warning() {
    assert_clean_run(
        "*main\n    v is vector()\n    v.push('hi')\n    log(v.get(0))\n",
        "hi\n",
    );
}

#[test]
fn never_resolved_bind_still_warns() {
    let c = compile("*main\n    v is vector()\n    log(v.length)\n");
    assert!(c.ok(), "{}", c.stderr());
    assert!(
        c.stderr()
            .contains("unsolved type variable defaulted to i64"),
        "a genuinely unresolved element type must still warn: {}",
        c.stderr()
    );
}
