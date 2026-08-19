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

#[test]
fn not_binds_looser_than_in() {
    accepts_and_prints(
        "*main\n    xs is [1, 2, 3]\n    if not 5 in xs\n        log('absent')\n    if not 1 in xs\n        log('bug')\n    else\n        log('present')\n",
        "absent\npresent",
    );
}

#[test]
fn narrow_literal_bind_rejects_out_of_range() {
    rejects(
        "*main\n    y as i8 is 300\n    log(y)\n",
        &["does not fit in `i8`"],
    );
}

#[test]
fn negative_out_of_range_literal_rejects() {
    rejects(
        "*main\n    w as i8 is -200\n    log(w)\n",
        &["does not fit in `i8`"],
    );
}

#[test]
fn negative_min_literal_is_accepted() {
    accepts_and_prints("*main\n    z as i8 is -128\n    log(z)\n", "-128");
}

#[test]
fn user_fn_named_vector_is_rejected_not_hijacking_literals() {
    rejects(
        "*vector(x as i64) returns i64\n    x\n\n*main\n    xs is [1, 2]\n    log(xs.length)\n",
        &["reserved"],
    );
}

#[test]
fn module_fn_name_collision_is_rejected() {
    rejects(
        "use math\n\n*math_ln(x as f64) returns f64\n    x\n\n*main\n    log(math.ln(1.0))\n",
        &["defined more than once"],
    );
}

#[test]
fn duplicate_int_literal_match_arms_reject_instead_of_ice() {
    rejects(
        "*main\n    x is 2\n    match x\n        1 ? log('one')\n        1 ? log('again')\n        _ ? log('other')\n",
        &["duplicate match arm"],
    );
}

#[test]
fn store_plus_extern_malloc_links() {
    let c = compile(
        "extern *malloc(size as i64) returns %i8\nextern *free(ptr as %i8)\n\nstore things @simple\n    name as String\n\n*main\n    p is extern.malloc(16)\n    extern.free(p)\n    insert things 'a'\n    log(count things)\n",
    );
    assert!(c.ok(), "store + extern malloc must link: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success());
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "1");
}

#[test]
fn float_to_string_round_trips_beyond_six_digits() {
    accepts_and_prints(
        "*main\n    log(to_string(123456789.5))\n    log(to_string(0.1))\n    log(to_string(2.0))\n",
        "123456789.5\n0.1\n2",
    );
}

#[test]
fn comptime_does_not_fold_unsigned_arithmetic_with_signed_semantics() {
    accepts_and_prints("*main\n    x as u64 is 0 - 1\n    log(x > 100)\n", "1");
}

#[test]
fn comptime_does_not_fold_pure_calls_through_failed_branches() {
    accepts_and_prints(
        "*f(a as i64) returns i64\n    if a > 0\n        s is 'hi'\n        return 10\n    return 20\n\n*main\n    log(f(5))\n",
        "10",
    );
}

#[test]
fn store_block_methods_reject_with_guidance() {
    rejects(
        "store users\n    name as String\n\n    *total() returns i64\n        count users\n\n*main\n    log(1)\n",
        &["not supported in alpha"],
    );
}

#[test]
fn migration_drop_without_down_type_rejects() {
    rejects(
        "store items @simple\n    name as String\n\nmigration 'drop_price' version 1\n    up\n        alter items\n            drop price\n\n*main\n    log(count items)\n",
        &["cannot determine the field's position"],
    );
}

#[test]
fn quaternary_err_arm_type_mismatch_rejects() {
    rejects(
        "err OpErr\n    Boom\n\n*risky() returns Result of i64, OpErr\n    Ok(1)\n\n*main\n    v is risky() ? $ !! 'text'\n    log(v)\n",
        &["type"],
    );
}

#[test]
fn mutation_through_a_free_function_param_reaches_the_caller() {
    accepts_and_prints(
        "type Server\n    name as String\n    term as i64\n    log_entries as Vec of i64\n\n*new_server(n as String) returns Server\n    Server(name is n, term is 0, log_entries is vec())\n\n*tick(s as Server)\n    s.term is s.term + 1\n\n*main\n    s is new_server('n1')\n    tick(s)\n    tick(s)\n    tick(s)\n    log s.term\n",
        "3",
    );
}

#[test]
fn same_name_binders_in_sibling_and_nested_loops_do_not_share_a_slot() {
    accepts_and_prints(
        "*main\n    total is 0\n    for i in 0 to 3\n        for j in 0 to 3\n            total is total + 1\n    log total\n    for i in 0 to 2\n        log i\n    for i in 0 to 2\n        log i * 10\n",
        "9\n0\n1\n0\n10",
    );
}

#[test]
fn a_match_arm_binding_that_shadows_an_existing_name_rejects() {
    rejects(
        "enum E\n    A(i64)\n    B\n\n*main\n    y is 5\n    e is A(1)\n    match e\n        y ? log y\n        B ? log 0\n",
        &["always matches", "shadows"],
    );
}

#[test]
fn a_generic_instantiation_cannot_collide_with_a_declared_type() {
    accepts_and_prints(
        "type Pair of A, B\n    l as A\n    r as B\n\ntype Pair_i64_i64\n    v as i64\n\n*main\n    p is Pair(l is 1, r is 2)\n    q is Pair_i64_i64(v is 9)\n    log p.l\n    log q.v\n",
        "1\n9",
    );
}

#[test]
fn a_runtime_tuple_index_rejects_instead_of_compiling() {
    rejects(
        "*main\n    t is (1, 'two', 3.0)\n    i is 1\n    log t[i]\n",
        &["tuple indices must be integer literals"],
    );
    rejects("*main\n    t is (1, 2)\n    log t[7]\n", &["out of range"]);
}

#[test]
fn a_function_local_use_is_rejected_with_placement_guidance() {
    rejects(
        "*main\n    use math\n    log 1\n",
        &["only allowed at the top level"],
    );
}
