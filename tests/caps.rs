use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn frontend(src: &str) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    let jn = dir.path().join("prog.jn");
    std::fs::write(&jn, src).unwrap();
    let c = Command::new(jinnc())
        .arg(&jn)
        .arg("--emit-hir")
        .current_dir(dir.path())
        .output()
        .expect("invoke jinnc");
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&c.stdout),
        String::from_utf8_lossy(&c.stderr)
    );
    (c.status.success(), out)
}

fn rejects(src: &str, needle: &str) {
    let (ok, out) = frontend(src);
    assert!(
        !ok,
        "expected a capability error containing {needle:?}, but it compiled"
    );
    assert!(
        out.contains(needle),
        "diagnostic did not mention {needle:?}:\n{out}"
    );
}

fn accepts(src: &str) {
    let (ok, out) = frontend(src);
    assert!(ok, "expected the program to type-check:\n{out}");
}

#[test]
fn needs_pure_rejects_file_write() {
    rejects(
        "use io\n\n*sneaky returns i64 needs pure\n    io.write_file('canary.txt', 'written')\n    0\n\n*main\n    sneaky()\n",
        "fs.write 'canary.txt'",
    );
}

#[test]
fn needs_pure_rejects_indirect_write_and_names_the_path() {
    let src = "use io\n\n*helper\n    io.write_file('x.txt', 'x')\n\n*outer needs pure\n    helper()\n\n*main\n    outer()\n";
    rejects(src, "fs.write");
    let (_, out) = frontend(src);
    assert!(out.contains("introduced via"), "missing intro path:\n{out}");
}

#[test]
fn needs_net_client_is_an_upper_bound() {
    accepts(
        "use net\n\n*lookup(host as String) returns String needs net.client\n    net.resolve(host)\n\n*main\n    nop\n",
    );
    rejects(
        "use net\n\n*lookup(host as String) returns String needs pure\n    net.resolve(host)\n\n*main\n    nop\n",
        "net.client",
    );
}

#[test]
fn scoped_fs_write_bounds_literal_paths() {
    accepts(
        "use io\n\n*log_it needs fs.write './out'\n    io.write_file('./out/log.txt', 'x')\n\n*main\n    log_it()\n",
    );
    rejects(
        "use io\n\n*log_it needs fs.write './out'\n    io.write_file('./etc/passwd', 'x')\n\n*main\n    log_it()\n",
        "fs.write",
    );
}

#[test]
fn declared_write_covers_derived_write() {
    accepts(
        "use io\n\n*persist(path as String, data as String) needs fs.write\n    io.write_file(path, data)\n\n*main\n    persist('a.txt', 'x')\n",
    );
}

#[test]
fn user_extern_is_ffi_unsafe_under_pure() {
    rejects(
        "extern *my_mystery_ffi(x as i64) returns i64\n\n*f needs pure\n    extern.my_mystery_ffi(1)\n\n*main\n    f()\n",
        "ffi.unsafe",
    );
}

#[test]
fn method_body_cannot_hide_an_effect() {
    rejects(
        "use io\n\ntype W\n    n as i64\n\n    *hit\n        io.write_file('w.txt', 'x')\n\n*f(w as W) needs pure\n    w.hit()\n\n*main\n    w is W(1)\n    f(w)\n",
        "fs.write",
    );
}

#[test]
fn unannotated_functions_are_never_checked() {
    accepts(
        "use io\n\n*free_writer\n    io.write_file('anything.txt', 'x')\n\n*main\n    free_writer()\n",
    );
}
