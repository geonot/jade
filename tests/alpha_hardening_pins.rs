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

fn emit(src: &str, flag: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("prog.jn");
    std::fs::write(&file, src).unwrap();
    let out = Command::new(jinnc())
        .arg("prog.jn")
        .arg(flag)
        .current_dir(dir.path())
        .output()
        .expect("invoke jinnc");
    assert!(
        out.status.success(),
        "emit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
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
fn generic_call_checks_later_args_against_bound_param() {
    rejects(
        "*pick of T(a as T, b as T)\n    a\n\n*main\n    r is pick(1, 'hello')\n    log(r)\n",
        &["argument 2 of `pick` has the wrong type"],
    );
}

#[test]
fn method_call_args_are_checked_on_concrete_types() {
    rejects(
        "type P\n    v as i64\n\n    *addv(n as i64)\n        v + n\n\n*main\n    p is P(v is 1)\n    log(p.addv('hello'))\n",
        &["argument 1 of `P.addv` has the wrong type"],
    );
}

#[test]
fn wrong_arity_lambda_is_rejected() {
    rejects(
        "*apply(f as (i64) returns i64, x as i64)\n    f(x)\n\n*main\n    r is apply(|a, b| a * 1000 + b, 5)\n    log(r)\n",
        &["is a function taking 2 argument(s)"],
    );
}

#[test]
fn copy_of_nonclonable_map_is_rejected() {
    rejects(
        "*main\n    m is map()\n    m.set('a', 1)\n    m2 is copy m\n    log(m2.get('a'))\n",
        &["cannot `copy` a value of type Map"],
    );
}

#[test]
fn top_level_stmts_with_explicit_main_are_rejected() {
    rejects(
        "log('top level')\n\n*main\n    log('main')\n",
        &["top-level statements cannot be combined with an explicit `*main`"],
    );
}

#[test]
fn missing_required_struct_field_is_rejected() {
    rejects(
        "type Config\n    retries is 3\n    host as String\n    tags as Vec of i64\n\n*main\n    c is Config(host is 'api')\n    log(c.retries)\n",
        &["missing required field(s) `tags`"],
    );
}

#[test]
fn function_value_call_cannot_launder_capabilities() {
    rejects(
        "*get_f returns (String) returns i64\n    |s| s.length\n\n*sneaky(s as String) returns i64 needs pure\n    get_f()(s)\n\n*main\n    log(sneaky('x'))\n",
        &["function value"],
    );
}

#[test]
fn small_int_log_extends_correctly() {
    accepts_and_prints(
        "*main\n    a is -5 as i8\n    log(a)\n    b is -300 as i16\n    log(b)\n",
        "-5\n-300",
    );
}

#[test]
fn nested_field_access_resolves_leaf_type() {
    accepts_and_prints(
        "type P\n    a as i8\n    b as f64\n\ntype Q2\n    x as i64\n    p as P\n\n*main\n    q is Q2(x is 3, p is P(a is 5 as i8, b is 2.5))\n    log(q.p.a)\n    log(q.p.b)\n    t is q.p\n    log(t.b)\n",
        "5\n2.500000\n2.500000",
    );
}

#[test]
fn heap_valued_map_survives_inserts_and_drop() {
    accepts_and_prints(
        "*main\n    m is map()\n    m.set('content-type', 'application/json-very-long-value-here')\n    m.set('authorization', 'Bearer some-quite-long-token-value-0123456789')\n    m.set('x-request-id', 'abcdef-0123456789-abcdef-0123456789')\n    log(m.get('content-type'))\n    log(m.get('x-request-id'))\n",
        "application/json-very-long-value-here\nabcdef-0123456789-abcdef-0123456789",
    );
}

#[test]
fn vec_combinators_work_on_struct_elements() {
    accepts_and_prints(
        "type Item\n    name as String\n    qty as i64\n\n*main\n    v is [Item(name is 'apple', qty is 3), Item(name is 'pear', qty is 5), Item(name is 'fig', qty is 1)]\n    big is v.filter(|it| it.qty > 2)\n    log(big.length)\n    doubled is v.map(|it| it.qty * 2)\n    log(doubled.get(0))\n    total is v.fold(0, |acc, it| acc + it.qty)\n    log(total)\n",
        "2\n6\n9",
    );
}

#[test]
fn vec_slice_of_strings_owns_its_elements() {
    accepts_and_prints(
        "*main\n    v is ['aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', 'cc']\n    w is v.slice(0, 2)\n    log(w.get(0))\n    log(v.get(0))\n    r is v.reverse()\n    log(r.get(0))\n",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\naaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\ncc",
    );
}

#[test]
fn maps_grow_past_sixteen_entries() {
    accepts_and_prints(
        "*main\n    m is map()\n    for i in 0 to 40\n        m.set('key-number-{i}', i * 10)\n    log(m.len())\n    ok is 0\n    for i in 0 to 40\n        if m.get('key-number-{i}') equals i * 10\n            ok is ok + 1\n    log(ok)\n",
        "40\n40",
    );
}

#[test]
fn map_bracket_indexing_reads_and_writes() {
    accepts_and_prints(
        "*main\n    m is map()\n    m['a'] is 42\n    m['b'] is 7\n    log(m['a'])\n    log(m['b'])\n    m['a'] is 43\n    log(m['a'])\n",
        "42\n7\n43",
    );
}

#[test]
fn json_nested_aggregate_roundtrip_is_clean() {
    accepts_and_prints(
        "use json\n\n*main\n    v is json.parse('{\"a\": 1, \"b\": [1, 2, 3], \"c\": \"hello\"}')\n    s is json.stringify(v)\n    log(s)\n",
        "{\"a\":1,\"b\":[1,2,3],\"c\":\"hello\"}",
    );
}

#[test]
fn channel_per_iteration_lifecycle_is_clean() {
    accepts_and_prints(
        "*main\n    total is 0\n    for i in 0 to 50\n        ch is channel of i64(4)\n        send ch, i\n        x is receive ch\n        total is total + x\n    log(total)\n",
        "1225",
    );
}

#[test]
fn string_rebind_loop_runs_clean() {
    accepts_and_prints(
        "*main\n    buf is ''\n    for i in 0 to 100\n        buf is buf + 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'\n    log(buf.length)\n",
        "4000",
    );
}

#[test]
fn comptime_min_div_neg_one_does_not_ice() {
    let c = compile(
        "*mindiv(a as i64, b as i64)\n    a / b\n\n*main\n    log(mindiv(-9223372036854775807 - 1, -1))\n",
    );
    assert!(c.ok(), "must compile without ICE: {}", c.stderr());
}

#[test]
fn rebind_emits_old_value_drop_in_loop_body() {
    let mir = emit(
        "*main\n    buf is ''\n    for i in 0 to 100\n        buf is buf + 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'\n    log(buf.length)\n",
        "--emit-mir",
    );
    let body = mir
        .split("for.body")
        .nth(1)
        .expect("loop body present in MIR");
    let body_head = body.split("goto").next().unwrap_or(body);
    assert!(
        body_head.contains("drop"),
        "loop body must drop the superseded string:\n{mir}"
    );
}

#[test]
fn emitted_ir_carries_stack_probes_and_internal_xmalloc() {
    let ir = emit(
        "*main\n    m is map()\n    m.set('k', 1)\n    log(m.get('k'))\n",
        "--emit-llvm",
    );
    assert!(
        ir.contains("\"probe-stack\"=\"inline-asm\""),
        "probe-stack attribute missing from emitted IR"
    );
    assert!(
        !ir.contains("define weak ptr @jinn_xmalloc"),
        "jinn_xmalloc must not be a weak definition — WeakAny blocks allocation elision"
    );
    assert!(
        ir.contains("jinn_xmalloc.exit") || ir.contains("define internal ptr @jinn_xmalloc"),
        "jinn_xmalloc must be internal and inlinable"
    );
}
