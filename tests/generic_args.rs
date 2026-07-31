//! Task 8-16 (decision D2) — call-site generic arguments reach
//! implicit-generic bodies. The D2 table's failing rows compile with
//! zero annotations and run CORRECTLY (not "compile via a silent i64
//! default"): a String element stays a String.

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

/// D2 row: `*peek(v)` — nothing local pins the element type; the call
/// site's `Vec of i64` must reach the body.
#[test]
fn d2_peek_unannotated() {
    assert_clean_run(
        "*peek(v)\n    t is v.get(0)\n    log(t)\n\n*main\n    xs is vec(7, 8)\n    peek(xs)\n",
        "7\n",
    );
}

/// The same body with a String vec must yield the String — under the old
/// i64 default this class of program was silently wrong or rejected.
#[test]
fn d2_peek_string_element_stays_string() {
    assert_clean_run(
        "*peek(v)\n    t is v.get(0)\n    log(t)\n\n*main\n    xs is vec('hi', 'yo')\n    peek(xs)\n",
        "hi\n",
    );
}

/// One generic used at two element types in one program: both
/// instantiations must be correct simultaneously.
#[test]
fn d2_mixed_instantiations() {
    assert_clean_run(
        "*peek(v)\n    t is v.get(0)\n    log(t)\n\n*main\n    peek(vec(42))\n    peek(vec('s'))\n",
        "42\ns\n",
    );
}

/// D2's motivating example: `*bsort(v)` works with zero annotations.
#[test]
fn d2_bsort_unannotated() {
    assert_clean_run(
        "*bsort(v)\n    n is v.length\n    for i in 0 to n\n        for j in 0 to n - i - 1\n            if v.get(j) > v.get(j + 1)\n                tmp is v.get(j)\n                v.set(j, v.get(j + 1))\n                v.set(j + 1, tmp)\n\n*main\n    xs is vec(3, 1, 2)\n    bsort(xs)\n    log(xs.get(0))\n    log(xs.get(2))\n",
        "1\n3\n",
    );
}

/// An exported library function with NO call site cannot be instantiated;
/// a real artifact build must say so rather than silently dropping it.
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

/// Any surviving fix-it must be valid Jinn: annotations use `as`, never
/// the `: i64` form the parser rejects.
#[test]
fn d2_fixit_text_is_valid_jinn() {
    // A genuinely ambiguous NON-generic body still warns — with `as`.
    let c = compile("*main\n    v is vec()\n    log(v.length)\n");
    assert!(c.ok(), "{}", c.stderr());
    let stderr = c.stderr();
    assert!(
        !stderr.contains("`: i64`") && !stderr.contains("`: f64`") && !stderr.contains("`: String`"),
        "fix-it suggests invalid Jinn: {stderr}"
    );
}
