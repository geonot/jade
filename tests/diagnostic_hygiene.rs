use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile_stderr(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("prog.jn");
    std::fs::write(&jinn, src).unwrap();
    let out = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(dir.path().join("prog.bin"))
        .output()
        .expect("invoke jinnc");
    assert!(
        !out.status.success(),
        "expected a diagnostic (non-zero exit) for:\n{src}"
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const INTERNAL_MARKERS: &[&str] = &[
    "__G_",
    "__poly_",
    "TypeVar(",
    "Symbol(",
    "Param(",
    "DefId",
    "RUST_BACKTRACE",
];

fn assert_clean(stderr: &str, src: &str) {
    for marker in INTERNAL_MARKERS {
        assert!(
            !stderr.contains(marker),
            "diagnostic leaks internal marker {marker:?}:\n---\n{stderr}\n---\nfor source:\n{src}"
        );
    }
    let bytes = stderr.as_bytes();
    for (i, w) in bytes.windows(2).enumerate() {
        if w[0] == b'?' && w[1].is_ascii_digit() {
            let ctx = &stderr[i.saturating_sub(60)..(i + 20).min(stderr.len())];
            panic!(
                "diagnostic leaks a raw inference variable `?N`:\n...{ctx}...\nfor source:\n{src}"
            );
        }
    }
}

#[test]
fn diagnostics_never_leak_internal_names() {
    let bad_programs = [
        "*f(x as i64) returns String\n    x + 1\n\n*main\n    log(f(1))\n",
        "*main\n    v is vec()\n    v.push(1)\n    v.push('two')\n",
        "type Box of T\n    value as T\n\n*get(b as Box of T) returns T\n    b.value\n\n*main\n    b is Box(value is 42)\n    log(get(b) + 'x')\n",
        "type Pair of A, B\n    first as A\n    second as B\n\n*main\n    p is Pair(first is 1, second is 'x')\n    q as Pair<string, i64> is p\n    log(q.first)\n",
        "enum Maybe of T\n    Just(T)\n    Nothing\n\n*main\n    m is Just(5)\n    match m\n        Just(v) ? log(v + 'x')\n        Nothing ? log(0)\n",
        "*main\n    s is 'abc'\n    t is s\n    log(s)\n    log(t)\n    u is take s\n    log(s)\n",
        "trait Greet\n    *greet(self, name as String) returns String\n\ntype Cat\n    id as i64\n\nimpl Greet for Cat\n    *greet(self, name)\n        name + 1\n\n*main\n    c is Cat(id is 1)\n    log(c.id)\n",
        "*main\n    x is 1\n    y is x.unknown_method()\n    log(y)\n",
        "type Box of T\n    value as T\n\n*main\n    b is Box(value is vec())\n    b.value.push('s')\n    n as i64 is b.value.get(0)\n    log(n)\n",
    ];
    for src in bad_programs {
        let stderr = compile_stderr(src);
        assert_clean(&stderr, src);
    }
}

#[test]
fn mono_struct_names_render_with_their_type_arguments() {
    let stderr = compile_stderr(
        "type Pair of A, B\n    first as A\n    second as B\n\n*main\n    p is Pair(first is 1, second is 'x')\n    log(p.third)\n",
    );
    assert!(
        stderr.contains("Pair<i64, string>"),
        "field errors on monomorphized structs must render the origin spelling, not the \
         mangled name: {stderr}"
    );
}
