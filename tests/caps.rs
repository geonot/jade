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

#[test]
fn store_write_rejects_needs_pure() {
    rejects(
        "store users @simple\n    name as String\n\n*pure_math(x as i64) returns i64 needs pure\n    insert users 'sneaky'\n    x + 1\n\n*main\n    pure_math(1)\n",
        "fs.read './users.store'",
    );
}

#[test]
fn store_read_rejects_needs_pure() {
    rejects(
        "store users @simple\n    name as String\n\n*peek returns i64 needs pure\n    count users\n\n*main\n    peek()\n",
        "./users.store",
    );
}

#[test]
fn store_ops_satisfy_scoped_fs_declaration() {
    accepts(
        "store users @simple\n    name as String\n\n*persist(n as String) needs fs.read './users.store', fs.write './users.store'\n    insert users n\n\n*main\n    persist('alice')\n",
    );
}

#[test]
fn send_to_writing_handler_rejects_needs_pure_and_names_the_handler() {
    rejects(
        "use io\n\nactor Logger\n    count as i64\n\n    @log_line s as String\n        io.write_file('log.txt', s)\n\n*quiet(lg as Logger) needs pure\n    lg.log_line('hello')\n\n*main\n    lg is spawn Logger\n    quiet(lg)\n    stop lg\n    join lg\n",
        "Logger.log_line",
    );
}

#[test]
fn send_to_pure_handler_is_accepted() {
    accepts(
        "actor Counter\n    count as i64\n\n    @bump n as i64\n        count is count + n\n\n*quiet(c as Counter) needs pure\n    c.bump(1)\n\n*main\n    c is spawn Counter\n    quiet(c)\n    stop c\n    join c\n",
    );
}

#[test]
fn spawn_joins_loop_handler_caps() {
    rejects(
        "use io\n\nactor Ticker\n    n as i64\n\n    *loop\n        io.write_file('tick.txt', 'x')\n\n*boot needs pure\n    t is spawn Ticker\n    stop t\n    join t\n\n*main\n    boot()\n",
        "fs.write 'tick.txt'",
    );
}

#[test]
fn function_passed_as_argument_launders_its_caps() {
    rejects(
        "use io\n\n*writer\n    io.write_file('x.txt', 'x')\n\n*apply(f)\n    f()\n\n*launder needs pure\n    apply(writer)\n\n*main\n    launder()\n",
        "fs.write 'x.txt'",
    );
}

#[test]
fn aliased_function_argument_launders_its_caps() {
    rejects(
        "use io\n\n*writer\n    io.write_file('x.txt', 'x')\n\n*apply(f)\n    f()\n\n*launder needs pure\n    g is writer\n    apply(g)\n\n*main\n    launder()\n",
        "fs.write 'x.txt'",
    );
}
