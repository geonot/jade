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
    assert_eq!(
        compile_and_run(src).trim(),
        expected.trim(),
        "source:\n{src}"
    );
}

fn compile_fails(src: &str) -> String {
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
        "expected compilation to fail, but it succeeded:\n{src}"
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const FILE_ERR: &str = "err FileError\n    NotFound\n    Denied\n\n";

#[test]
fn propagate_happy_path_passes_value_through() {
    expect(
        &format!(
            "{FILE_ERR}*read(ok as bool) returns Result of i64, FileError\n    if ok\n        Ok(42)\n    else\n        Err(NotFound)\n\n*load(ok as bool) returns Result of i64, FileError\n    raw is read(ok)\n    Ok(raw + 1)\n\n*main()\n    match load(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n"
        ),
        "43",
    );
}

#[test]
fn propagate_early_exits_on_err() {
    expect(
        &format!(
            "{FILE_ERR}*read(ok as bool) returns Result of i64, FileError\n    if ok\n        Ok(42)\n    else\n        Err(NotFound)\n\n*load(ok as bool) returns Result of i64, FileError\n    raw is read(ok)\n    Ok(raw + 1)\n\n*main()\n    match load(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n"
        ),
        "-1",
    );
}

#[test]
fn propagate_converts_via_from_across_layers() {
    let src = "err FileError\n    NotFound\n\nerr NetError\n    Timeout\n\nerr AppError\n    Io(FileError)\n    Net(NetError)\n\nimpl From of FileError for AppError\n    *from(e as FileError) returns AppError\n        Io(e)\n\nimpl From of NetError for AppError\n    *from(e as NetError) returns AppError\n        Net(e)\n\n*fetch(ok as bool) returns Result of i64, NetError\n    if ok\n        Ok(7)\n    else\n        Err(Timeout)\n\n*save(ok as bool) returns Result of i64, FileError\n    if ok\n        Ok(1)\n    else\n        Err(NotFound)\n\n*backup(a as bool, b as bool) returns Result of i64, AppError\n    data is fetch(a)\n    n is save(b)\n    Ok(data + n)\n\n*main()\n    match backup(true, true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match backup(false, true)\n        Ok(v) ? log(0)\n        Err(e) ? match e\n            Net(_) ? log(2)\n            Io(_) ? log(3)\n    match backup(true, false)\n        Ok(v) ? log(0)\n        Err(e) ? match e\n            Net(_) ? log(2)\n            Io(_) ? log(3)\n";
    expect(src, "8\n2\n3");
}

#[test]
fn propagate_on_option_returns_nothing() {
    expect(
        "*find(ok as bool) returns Option of i64\n    if ok\n        Some(5)\n    else\n        Nothing\n\n*chain(ok as bool) returns Option of i64\n    v is find(ok)\n    Some(v + 1)\n\n*main()\n    log(chain(true).unwrap_or(-1))\n    log(chain(false).unwrap_or(-1))\n",
        "6\n-1",
    );
}

#[test]
fn propagate_outside_fallible_fn_is_error() {
    let out = compile_fails(
        "err E\n    Oops\n\n*read() returns Result of i64, E\n    Err(Oops)\n\n*bad() returns i64\n    x is read()\n    x\n\n*main()\n    log(0)\n",
    );
    assert!(
        !out.is_empty(),
        "expected propagation-outside-fallible diagnostic, got:\n{out}"
    );
}

#[test]
fn err_raise_early_return_still_works() {
    expect(
        "err MyErr\n    Oops\n\n*f(ok as bool) returns Result of i64, MyErr\n    if ok\n        Ok(1)\n    else\n        err Oops\n\n*main()\n    match f(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-9)\n",
        "-9",
    );
}

#[test]
fn option_predicates_and_unwrap() {
    expect(
        "*main()\n    a is Some(5)\n    log(a.is_some())\n    log(a.is_none())\n    log(a.unwrap())\n    log(a.unwrap_or(99))\n    n is Nothing\n    log(n.unwrap_or(42))\n",
        "1\n0\n5\n5\n42",
    );
}

#[test]
fn option_map_and_then_ok_or() {
    expect(
        "*main()\n    a is Some(5)\n    log(a.map(|x| x * 2).unwrap())\n    log(a.and_then(|x| Some(x * 10)).unwrap())\n    log(a.ok_or(7).unwrap())\n",
        "10\n50\n5",
    );
}

#[test]
fn option_map_short_circuits_on_nothing() {
    expect(
        "*parse(x as i64) returns Option of i64\n    if x > 0\n        Some(x)\n    else\n        Nothing\n\n*main()\n    log(parse(5).map(|x| x + 1).unwrap_or(0))\n    log(parse(-1).map(|x| x + 1).unwrap_or(-99))\n",
        "6\n-99",
    );
}

#[test]
fn result_predicates_and_unwrap() {
    expect(
        "*ok(x as i64) returns Result of i64, i64\n    Ok(x)\n\n*er(x as i64) returns Result of i64, i64\n    Err(x)\n\n*main()\n    a is ok(5)\n    log(a.is_ok())\n    log(a.is_err())\n    log(a.unwrap())\n    log(a.unwrap_or(99))\n    e is er(8)\n    log(e.is_err())\n    log(e.unwrap_or(77))\n",
        "1\n0\n5\n5\n1\n77",
    );
}

#[test]
fn result_map_map_err_and_then() {
    expect(
        "*ok(x as i64) returns Result of i64, i64\n    Ok(x)\n\n*main()\n    a is ok(5)\n    log(a.map(|x| x * 2).unwrap())\n    log(a.map_err(|e| e + 1).unwrap())\n    log(a.and_then(|x| ok(x * 10)).unwrap())\n",
        "10\n5\n50",
    );
}

#[test]
fn result_ok_and_err_projections() {
    expect(
        "*ok(x as i64) returns Result of i64, i64\n    Ok(x)\n\n*er(x as i64) returns Result of i64, i64\n    Err(x)\n\n*main()\n    log(ok(5).ok().unwrap())\n    log(er(8).err().unwrap())\n    log(ok(5).err().is_none())\n    log(er(8).ok().is_none())\n",
        "5\n8\n1\n1",
    );
}

#[test]
fn propagate_missing_conversion_is_error() {
    let out = compile_fails(
        "err FileError\n    NotFound\n\nerr NetError\n    Timeout\n\n*read(ok as bool) returns Result of i64, FileError\n    if ok\n        Ok(1)\n    else\n        Err(NotFound)\n\n*load(ok as bool) returns Result of i64, NetError\n    raw is read(ok)\n    Ok(raw)\n\n*main()\n    log(0)\n",
    );
    assert!(
        out.contains("FileError") && out.contains("NetError"),
        "expected missing-conversion diagnostic relating FileError and NetError, got:\n{out}"
    );
}

#[test]
fn err_raise_undeclared_error_is_error() {
    let out = compile_fails(
        "err FileError\n    NotFound\n\nerr NetError\n    Timeout\n\n*load(ok as bool) returns Result of i64, NetError\n    if ok\n        Ok(1)\n    else\n        err NotFound\n\n*main()\n    log(0)\n",
    );
    assert!(
        out.contains("no conversion `FileError -> NetError`") || out.contains("FileError"),
        "expected undeclared-error/soundness diagnostic, got:\n{out}"
    );
}

#[test]
fn soundness_inferred_must_convert_into_declared() {
    let out = compile_fails(
        "err FileError\n    NotFound\n\nerr NetError\n    Timeout\n\n*read(ok as bool) returns i64 ! FileError\n    if ok\n        9\n    else\n        err NotFound\n\n*load(ok as bool) returns i64 ! NetError\n    raw is read(ok) ? $ !! err\n    raw + 1\n\n*main()\n    log(0)\n",
    );
    assert!(
        out.contains("FileError") && out.contains("NetError"),
        "expected R1 soundness diagnostic relating FileError and NetError, got:\n{out}"
    );
}

#[test]
fn soundness_passes_when_from_conversion_exists() {
    expect(
        "err FileError\n    NotFound\n\nerr AppError\n    Io(FileError)\n\nimpl From of FileError for AppError\n    *from(e as FileError) returns AppError\n        Io(e)\n\n*read(ok as bool) returns i64 ! FileError\n    if ok\n        9\n    else\n        err NotFound\n\n*load(ok as bool) returns i64 ! AppError\n    raw is read(ok) ? $ !! err\n    raw + 1\n\n*main()\n    match load(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match load(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n",
        "10\n-1",
    );
}

#[test]
fn propagate_reflexive_conversion_needs_no_impl() {
    expect(
        "err FileError\n    NotFound\n\n*read(ok as bool) returns Result of i64, FileError\n    if ok\n        Ok(7)\n    else\n        Err(NotFound)\n\n*load(ok as bool) returns Result of i64, FileError\n    raw is read(ok)\n    Ok(raw + 1)\n\n*main()\n    match load(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match load(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n",
        "8\n-1",
    );
}

const READ_RES: &str = "err FileError\n    NotFound\n    Denied\n\n*read(ok as bool) returns Result of i64, FileError\n    if ok\n        Ok(42)\n    else\n        Err(NotFound)\n\n";

#[test]
fn quaternary_success_arm_binds_dollar() {
    expect(
        &format!(
            "{READ_RES}*main()\n    read(true) ? log($) !! log(-1)\n    read(false) ? log($) !! log(-1)\n"
        ),
        "42\n-1",
    );
}

#[test]
fn quaternary_fallback_value_on_error() {
    expect(
        &format!(
            "{READ_RES}*main()\n    a is read(true) ? $ !! 7\n    log(a)\n    b is read(false) ? $ !! 7\n    log(b)\n"
        ),
        "42\n7",
    );
}

#[test]
fn quaternary_err_arm_binds_err_value() {
    expect(
        &format!(
            "{READ_RES}*describe(e as FileError) returns i64\n    match e\n        NotFound ? 404\n        Denied ? 403\n\n*main()\n    read(false) ? log($) !! log(describe(err))\n"
        ),
        "404",
    );
}

#[test]
fn quaternary_explicit_propagation_with_bang_bang_err() {
    expect(
        &format!(
            "{READ_RES}*load(ok as bool) returns Result of i64, FileError\n    raw is read(ok) ? $ !! err\n    Ok(raw + 1)\n\n*main()\n    match load(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match load(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n"
        ),
        "43\n-1",
    );
}

#[test]
fn implicit_propagation_on_bare_result_bind() {
    expect(
        &format!(
            "{READ_RES}*load(ok as bool) returns Result of i64, FileError\n    raw is read(ok)\n    Ok(raw + 1)\n\n*main()\n    match load(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match load(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n"
        ),
        "43\n-1",
    );
}

#[test]
fn implicit_propagation_on_bare_option_bind() {
    expect(
        "*find(ok as bool) returns Option of i64\n    if ok\n        Some(7)\n    else\n        Nothing\n\n*pick(ok as bool) returns Option of i64\n    v is find(ok)\n    Some(v + 1)\n\n*main()\n    log(pick(true).unwrap_or(-1))\n    log(pick(false).unwrap_or(-1))\n",
        "8\n-1",
    );
}

#[test]
fn quaternary_option_success_and_empty_arms() {
    expect(
        "*find(ok as bool) returns Option of i64\n    if ok\n        Some(9)\n    else\n        Nothing\n\n*main()\n    find(true) ? log($) ! log(-1)\n    find(false) ? log($) ! log(-1)\n",
        "9\n-1",
    );
}

#[test]
fn quaternary_propagation_converts_via_from() {
    let src = "err FileError\n    NotFound\n\nerr NetError\n    Timeout\n\nerr AppError\n    Io(FileError)\n    Net(NetError)\n\nimpl From of FileError for AppError\n    *from(e as FileError) returns AppError\n        Io(e)\n\nimpl From of NetError for AppError\n    *from(e as NetError) returns AppError\n        Net(e)\n\n*fetch(ok as bool) returns Result of i32, NetError\n    if ok\n        Ok(7)\n    else\n        Err(Timeout)\n\n*save(ok as bool) returns Result of i16, FileError\n    if ok\n        Ok(1)\n    else\n        Err(NotFound)\n\n*backup(a as bool, b as bool) returns Result of i64, AppError\n    data is fetch(a) ? $ !! err\n    n is save(b) ? $ !! err\n    Ok(data as i64 + n as i64)\n\n*main()\n    match backup(true, true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match backup(false, true)\n        Ok(v) ? log(0)\n        Err(e) ? match e\n            Net(_) ? log(2)\n            Io(_) ? log(3)\n    match backup(true, false)\n        Ok(v) ? log(0)\n        Err(e) ? match e\n            Net(_) ? log(2)\n            Io(_) ? log(3)\n";
    expect(src, "8\n2\n3");
}

#[test]
fn quaternary_multiline_arm_form() {
    expect(
        &format!(
            "{READ_RES}*main()\n    read(true)\n        ? log($)\n        !! log(-1)\n    read(false)\n        ? log($)\n        !! log(-1)\n"
        ),
        "42\n-1",
    );
}

#[test]
fn err_raise_propagates_to_result_return() {
    expect(
        "err FileError\n    NotFound\n\n*read(ok as bool) returns i64 ! FileError\n    if ok\n        99\n    else\n        err NotFound\n\n*main()\n    match read(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match read(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n",
        "99\n-1",
    );
}

#[test]
fn ternary_err_guard_raises() {
    expect(
        "err E\n    Bad\n    Code(i64)\n\n*g(d as i64) returns i64 ! E\n    d < 0 ? err Bad\n    d > 100 ? err Code(d)\n    d\n\n*main()\n    g(5) ? log($) !! log(-1)\n    g(-1) ? log($) !! log(-2)\n    g(200) ? log($) !! log(-3)\n",
        "5\n-2\n-3",
    );
}

#[test]
fn qualified_variant_construction_and_match() {
    expect(
        "enum Sev\n    Info\n    Warn\n    Error\n\n*main()\n    a is Sev.Warn\n    match a\n        Warn ? log(1)\n        _ ? log(0)\n    log('{Sev.Error}')\n",
        "1\nError",
    );
}

#[test]
fn enum_value_interpolation_renders_variant_name() {
    expect(
        "err E\n    Bad\n    Code(i64)\n\n*main()\n    a is Bad\n    b is Code(7)\n    log('{a}')\n    log('{b}')\n",
        "Bad\nCode",
    );
}

#[test]
fn explicit_return_autowraps_ok_in_fallible_fn() {
    expect(
        "err E\n    Bad\n\n*f(x as i64) returns i64 ! E\n    if x equals 0\n        return 0\n    x\n\n*main()\n    f(0) ? log($) !! log(-1)\n    f(7) ? log($) !! log(-1)\n",
        "0\n7",
    );
}

#[test]
fn fn_returning_enum_stored_in_struct_field() {
    expect(
        "enum Sev\n    Info\n    Warn\n\n*pick(n as i64) returns Sev\n    n > 0 ? Warn ! Info\n\ntype Rec\n    sev as Sev\n\n*main()\n    r is Rec(sev is pick(1))\n    match r.sev\n        Warn ? log(1)\n        _ ? log(0)\n    log('{r.sev}')\n",
        "1\nWarn",
    );
}

#[test]
fn fallible_fn_returning_struct_payload_intact() {
    expect(
        "err E\n    Bad\n\ntype Rec\n    a as i64\n    b as i64\n\n*mk(x as i64) returns Rec ! E\n    if x < 0\n        err Bad\n    Rec(a is x, b is x * 2)\n\n*main()\n    mk(21) ? log($.b) !! log(-1)\n",
        "42",
    );
}

#[test]
fn bare_fallible_call_stmt_propagates() {
    expect(
        "err E\n    Bad\n\n*may_fail(x as i64) returns i64 ! E\n    x < 0 ? err Bad\n    x * 2\n\n*chain(x as i64) returns i64 ! E\n    may_fail(x)\n    99\n\n*main()\n    chain(5) ? log($) !! log(-1)\n    chain(-1) ? log($) !! log(-2)\n",
        "99\n-2",
    );
}

#[test]
fn bare_insert_propagates_and_yields_result() {
    expect(
        "err AppErr\n    S(StoreError)\n\nimpl From of StoreError for AppErr\n    *from(e as StoreError) returns AppErr is S(e)\n\nstore items\n    name as String\n    qty as i64\n\n*land(n as String, q as i64) returns i64 ! AppErr\n    insert items n, q\n\n*main()\n    land('a', 7) ? log('ok') !! log('err')\n",
        "ok",
    );
}

#[test]
fn insert_propagation_through_multivariant_from_enum() {
    expect(
        "err RErr\n    Fail\n    S(StoreError)\n\nimpl From of StoreError for RErr\n    *from(e as StoreError) returns RErr is S(e)\n\nstore jobs\n    pri as i64\n\n*enqueue(ok as bool) returns Result of i64, RErr\n    insert jobs 7\n    if not ok\n        err Fail\n    Ok(7)\n\n*main\n    match enqueue(false)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match enqueue(true)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n",
        "-1\n7",
    );
}

#[test]
fn two_variant_enum_with_byvalue_enum_payload() {
    expect(
        "enum Box\n    Empty\n    Has(StoreError)\n\n*main\n    b is Has(Duplicate)\n    match b\n        Empty ? log 0\n        Has(e) ? log 1\n",
        "1",
    );
}

#[test]
fn bare_fallible_call_tail_autowraps_ok() {
    expect(
        "err E\n    Bad\n\n*inner(x as i64) returns i64 ! E\n    x < 0 ? err Bad\n    x + 1\n\n*outer(x as i64) returns i64 ! E\n    inner(x)\n\n*main()\n    outer(10) ? log($) !! log(-1)\n    outer(-1) ? log($) !! log(-2)\n",
        "11\n-2",
    );
}

#[test]
fn ternary_mixed_fallible_arm_reconciles() {
    expect(
        "err E\n    Bad\n\n*ti(ok as bool) returns i64 ! E\n    if ok\n        5\n    else\n        err Bad\n\n*bo(z as bool, ok as bool) returns i64 ! E\n    z ? 0 ! ti(ok)\n\n*main()\n    bo(true, true) ? log($) !! log(-1)\n    bo(false, true) ? log($) !! log(-1)\n    bo(false, false) ? log($) !! log(-2)\n",
        "0\n5\n-2",
    );
}

#[test]
fn plain_struct_return_payload_intact() {
    expect(
        "type P\n    x as i64\n    y as i64\n\n*mk returns P\n    P(1, 2)\n\n*main()\n    a is mk()\n    log(a.x)\n    log(a.y)\n",
        "1\n2",
    );
}

#[test]
fn match_on_fallible_struct_return() {
    expect(
        "err E\n    Bad\n\ntype P\n    x as i64\n    y as i64\n\n*mk(n as i64) returns P ! E\n    if n < 0\n        err Bad\n    P(n, n * 2)\n\n*main()\n    match mk(3)\n        Ok(p) ? log(p.y)\n        Err(e) ? log(-1)\n    match mk(-1)\n        Ok(p) ? log(p.y)\n        Err(e) ? log(-2)\n",
        "6\n-2",
    );
}

#[test]
fn method_declares_error_union() {
    expect(
        "err MyErr\n    Bad\n\ntype Box\n    v as i64\n\n    *get(n as i64) returns i64 ! MyErr\n        if n < 0\n            err Bad\n        n * 2\n\n*main()\n    b is Box(v is 1)\n    match b.get(5)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match b.get(-1)\n        Ok(v) ? log(v)\n        Err(e) ? log(-2)\n",
        "10\n-2",
    );
}

#[test]
fn ptr_method_declares_error_union() {
    expect(
        "err MyErr\n    Bad\n\ntype Counter\n    n as i64\n\n    *bump(by as i64) returns i64 ! MyErr\n        if by < 0\n            err Bad\n        self.n is self.n + by\n        self.n\n\n*main()\n    c is Counter(n is 0)\n    match c.bump(3)\n        Ok(v) ? log(v)\n        Err(e) ? log(-1)\n    match c.bump(-1)\n        Ok(v) ? log(v)\n        Err(e) ? log(-2)\n",
        "3\n-2",
    );
}

#[test]
fn error_diagnostics_have_no_debug_spans() {
    let err = compile_fails(
        "err E1\n    Bad\nerr E2\n    Other\n\n*f(n as i64) returns i64 ! E1\n    if n < 0\n        err Other\n    n\n\n*main()\n    log(1)\n",
    );
    assert!(
        !err.contains("Span {"),
        "diagnostic leaked a Debug span:\n{err}"
    );
    assert!(err.contains("no conversion `E2 -> E1`"), "{err}");
    assert!(
        err.contains(".jn:8:9"),
        "expected file:line:col, got:\n{err}"
    );
}
