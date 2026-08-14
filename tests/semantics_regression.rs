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

fn exit_desc(o: &Output) -> String {
    format!(
        "status={:?} stdout={:?} stderr={:?}",
        o.status,
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn ownership_returning_vec_parameter_runs_clean() {
    let c = compile(
        "*ident(v) returns Vec of i64\n    return v\n\n*main\n    a is vec(1, 2, 3)\n    s is ident(a)\n    log(s.length)\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "must run clean: {}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n");
}

#[test]
fn ownership_use_after_consuming_call_is_rejected() {
    let c = compile(
        "*ident(v) returns Vec of i64\n    return v\n\n*main\n    a is vec(1, 2, 3)\n    s is ident(a)\n    log(a.length)\n",
    );
    assert!(!c.ok(), "use-after-move must not compile");
    let stderr = c.stderr();
    assert!(
        stderr.contains("moved value `a`") && stderr.contains("`ident`"),
        "diagnostic must name the move and the consuming callee: {stderr}"
    );

    let ok = compile(
        "*ident(v) returns Vec of i64\n    return v\n\n*main\n    a is vec(1, 2, 3)\n    a2 is copy a\n    s is ident(a2)\n    log(s.length)\n    log(a.length)\n",
    );
    assert!(
        ok.ok(),
        "copy-binding escape hatch must compile: {}",
        ok.stderr()
    );
    let run = ok.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n3\n");
}

#[test]
fn ownership_nested_scope_bind_single_drop() {
    let c = compile(
        "*main\n    a is vec(1, 2, 3)\n    if true\n        b is a\n        log(b.length)\n    log(7)\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "must run clean: {}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n7\n");
}

#[test]
fn ownership_cross_task_shared_vec_is_rejected() {
    let c = compile(
        "*pusher(v, base)\n    for i in 0 to 20000\n        v.push(base + i)\n\n*main\n    shared is vec()\n    together\n        dispatch\n            pusher(shared, 0)\n        dispatch\n            pusher(shared, 1000000)\n    log(shared.length)\n",
    );
    assert!(!c.ok(), "must be rejected under M8 (memory-model.md)");
    let stderr = c.stderr();
    assert!(
        stderr.contains("`shared` used after being moved into a concurrent task")
            && stderr.contains("channel")
            && stderr.contains("actor"),
        "diagnostic must name the message-passing alternatives: {stderr}"
    );
}

#[test]
fn ownership_vec_assignment_is_rejected_as_use_after_move() {
    let c = compile(
        "*main\n    a is vec(1,2,3)\n    b is a\n    b.push(4)\n    log(a.length)\n    log(b.length)\n",
    );
    assert!(!c.ok(), "must be rejected under D1 (memory-model.md M1)");
    let stderr = c.stderr();
    assert!(
        stderr.contains("use of moved value `a`")
            && stderr.contains("`b is a`")
            && stderr.contains("copy"),
        "diagnostic must name the move site and the copy escape hatch: {stderr}"
    );

    let c = compile(
        "*main\n    a is vec(1,2,3)\n    b is copy a\n    b.push(4)\n    log(a.length)\n    log(b.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n4\n");
}

#[test]
fn typing_cross_type_equals_is_rejected() {
    let c = compile("*main\n    if 'abc' equals 5\n        log('huh')\n");
    assert!(!c.ok(), "cross-type equals must be a type error");
    assert!(c.stderr().contains("type mismatch"), "{}", c.stderr());
}

#[test]
fn typing_declared_string_returns_i64() {
    let c = compile("*f(x as i64) returns String\n    x + 1\n\n*main\n    log(f(1))\n");
    assert!(!c.ok(), "must not compile");
    let stderr = c.stderr();
    assert!(
        stderr.contains("`f`") && stderr.contains("returns") && stderr.contains("type mismatch"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("compiler bug") && !stderr.contains("MIR verify"),
        "a plain source error must not carry ICE wording: {stderr}"
    );
}

#[test]
fn typing_string_plus_int() {
    let c = compile("*main\n    x is 'abc' + 1\n    log(x)\n");
    assert!(!c.ok(), "must not compile");
    let stderr = c.stderr();

    assert!(
        stderr.contains("type mismatch") || stderr.contains("operator"),
        "{stderr}"
    );
}

#[test]
fn typing_heterogeneous_vec_is_rejected() {
    let c = compile("*main\n    v is vec()\n    v.push(1)\n    v.push('two')\n    log(v.get(1))\n");
    assert!(!c.ok(), "heterogeneous vec must be a type error");
    assert!(c.stderr().contains("type mismatch"), "{}", c.stderr());
}

#[test]
fn typing_multi_clause_arity_is_a_diagnostic() {
    let c = compile("*f(0) is 0\n*f a, b is a + b\n\n*main\n    log(f(1, 2))\n");
    assert!(!c.ok(), "must not compile (and must exit non-zero)");
    let stderr = c.stderr();
    assert!(
        stderr.contains("multi-clause function `f`") && stderr.contains("parameters"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("RUST_BACKTRACE") && !stderr.contains("panicked"),
        "must not be a Rust panic: {stderr}"
    );
}

#[test]
fn top_level_reassignment_is_cleanly_diagnosed() {
    let c = compile("x is 41\nx is x + 1\nlog(x)\n");
    assert!(!c.ok(), "self-referential top-level const must be rejected");
    let stderr = c.stderr();
    assert!(stderr.contains("defined in terms of itself"), "{stderr}");
    assert!(
        !stderr.contains("stack overflow") && !stderr.contains("RUST_BACKTRACE"),
        "must be a diagnostic, not a crash: {stderr}"
    );
}

#[test]
fn store_all_iteration_works() {
    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    insert users 'Bob', 25\n    for u in all users\n        log(u.name)\n    rows is all users\n    log(rows.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "Alice\nBob\n2\n");
}

#[test]
fn store_query_miss_is_a_value_not_a_zero_row() {
    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice', 30\n    match users where name equals 'Alice'\n        Ok(r) ?\n            log(r.name)\n            log(r.age)\n        Err(e) ? log('hit expected')\n    match users where age > 100\n        Ok(r) ? log(r.name)\n        Err(e) ? log('miss is a miss')\n    q is users where age > 100 ? $.age ! 0 - 1\n    log(q)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "Alice\n30\nmiss is a miss\n-1\n"
    );
}

#[test]
fn store_unhandled_query_is_a_compile_error() {
    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*main\n    missing is users where age > 100\n    log('name=[{missing.name}] age={missing.age}')\n",
    );
    assert!(!c.ok(), "bare bind of a query must not compile");
    let stderr = c.stderr();
    assert!(
        stderr.contains("query result")
            || stderr.contains("propagat")
            || stderr.contains("fallible"),
        "{stderr}"
    );

    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*main\n    log((users where age > 100).age)\n",
    );
    assert!(!c.ok(), "field read on a query result must not compile");
    let stderr = c.stderr();
    assert!(stderr.contains("query result"), "{stderr}");
}

#[test]
fn store_query_miss_propagates_in_fallible_fn() {
    let c = compile(
        "store users\n    name as String\n    age as i64\n\n*find(n as String) returns i64 ! StoreError\n    r is users where name equals n\n    r.age\n\n*main\n    insert users 'Alice', 30\n    match find('Alice')\n        Ok(a) ? log(a)\n        Err(e) ? log('unexpected miss')\n    match find('Zed')\n        Ok(a) ? log(a)\n        Err(e) ? log('propagated')\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "30\npropagated\n");
}

#[test]
fn every_diagnostic_input_exits_nonzero() {
    let bad = [
        "*f(0) is 0\n*f a, b is a + b\n\n*main\n    log(f(1, 2))\n",
        "x is 41\nx is x + 1\nlog(x)\n",
        "*f(x as i64) returns String\n    x + 1\n\n*main\n    log(f(1))\n",
        "*add(a as i64, b as i64) returns i64\n    a + b\n\n*main\n    log(add('one', 2))\n",
        "*main\n    if 'abc' equals 5\n        log('huh')\n",
        "*main\n    v is vec()\n    v.push(1)\n    v.push('two')\n",
        "*main\n    log(oops(((\n",
    ];
    for src in bad {
        let c = compile(src);
        assert!(!c.ok(), "must exit non-zero for:\n{src}");
        let stderr = c.stderr();
        assert!(
            !stderr.contains("RUST_BACKTRACE")
                && !stderr.contains("panicked at")
                && !stderr.contains("Call parameter type"),
            "diagnostic must not be a panic or raw IR for:\n{src}\n{stderr}"
        );
    }
}

#[test]
fn conditional_consumption_diagnostic_names_the_consuming_path() {
    let c = compile(
        "*maybe_keep(v, keep as bool) returns Vec of i64\n    if keep\n        return v\n    vec(0)\n\n*main\n    a is vec(1, 2, 3)\n    s is maybe_keep(a, false)\n    log(s.length)\n    log(a.length)\n",
    );
    assert!(
        !c.ok(),
        "use-after-conditional-consumption must not compile"
    );
    let stderr = c.stderr();
    assert!(
        stderr.contains("it is consumed at") && stderr.contains("conditional path"),
        "diagnostic must name the consuming site and say it is conditional: {stderr}"
    );
}

#[test]
fn unconditional_consumption_diagnostic_names_the_consuming_site() {
    let c = compile(
        "*keep(v) returns Vec of i64\n    return v\n\n*main\n    a is vec(1, 2, 3)\n    s is keep(a)\n    log(s.length)\n    log(a.length)\n",
    );
    assert!(!c.ok(), "use-after-consumption must not compile");
    let stderr = c.stderr();
    assert!(
        stderr.contains("it is consumed at") && !stderr.contains("conditional path"),
        "diagnostic must name the consuming site without a conditional note: {stderr}"
    );
}

#[test]
fn vec_slice_copies_the_requested_window() {
    let c = compile(
        "*main\n    v is vec(10, 20, 30, 40, 50)\n    s is v from 1 to 4\n    log(s.length)\n    log(s.get(0))\n    log(s.get(2))\n    log(v.length)\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "3\n20\n40\n5",
        "vec slice must copy elements 1..4 ([147]: codegen passed 3 of __jinn_vec_slice's 4 \
         arguments, so elem_size was garbage — zero gave [0, 0, 0], other values gave terabyte \
         mallocs)"
    );
}

#[test]
fn match_arm_block_with_trailing_drop_yields_the_tail_value() {
    let c = compile(
        "*f(flag as bool) returns String\n    match flag\n        true ?\n            result is \"[\"\n            result + \"]\"\n        false ? \"no\"\n\n*main\n    log(f(true))\n    log(f(false))\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "[]\nno",
        "a value-position match arm whose block tail only reads a local gets a typer-appended \
         scope-end drop after the tail; MIR lowering took the last statement's value, so the \
         drop's void fed the merge phi and every arm of the match silently returned \"\""
    );
}

#[test]
fn match_arm_block_value_survives_loop_accumulation_and_recursion() {
    let c = compile(
        "enum E\n    N(i64)\n    L(Vec of i64)\n\n*p(v as E) returns String\n    match v\n        N(n) ? to_string(n)\n        L(xs) ?\n            result is \"[\"\n            loop xs\n                result is result + to_string($)\n            result + \"]\"\n\n*main\n    log(p(N(42)))\n    log(p(L(vec(7, 8))))\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42\n[78]");
}

const HEAP_STR_SETUP: &str = "use std/strings\n\n*heap_str returns String\n    sb is strings.builder()\n    sb.write('a very long heap string well beyond any sso inline capacity limit')\n    sb.to_string()\n";

#[test]
fn string_var_captures_clone_instead_of_aliasing() {
    let src = format!(
        "{HEAP_STR_SETUP}\ntype Holder\n    src as String\n    pos as i64\n\n*eat(text as String) returns i64\n    p is Holder(src is text, pos is 0)\n    p.src.byte_count\n\n*main\n    s is heap_str()\n    log(eat(s))\n    v is vector(s, 'x')\n    log(v.length)\n    t is (s, 1)\n    a is [s, 'y']\n    log(s.byte_count)\n"
    );
    let c = compile(&src);
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(
        run.status.success(),
        "capturing a String var into a ctor/vector/tuple/array must clone, not alias \
         (double free otherwise): {}",
        exit_desc(&run)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "65\n2\n65");
}

#[test]
fn channel_send_of_string_var_clones() {
    let src = format!(
        "{HEAP_STR_SETUP}\n*main\n    s is heap_str()\n    ch is channel of String(2)\n    ch.send(s)\n    send ch, s\n    r1 is ch.recv()\n    r2 is ch.recv()\n    log(r1.byte_count)\n    log(r2.byte_count)\n    log(s.byte_count)\n"
    );
    let c = compile(&src);
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "65\n65\n65");
}

#[test]
fn mutating_method_in_nested_loop_persists_scalar_field_writes() {
    let c = compile(
        "type B\n    n as i64\n    parts as Vec of i64\n\n    *inc\n        self.n is self.n + 1\n        self.parts.push(self.n)\n\n*mk returns B\n    B(n is 0, parts is vec())\n\n*main\n    i is 0\n    while i < 2\n        b is mk()\n        j is 0\n        while j < 3\n            b.inc()\n            j is j + 1\n        log(b.n)\n        log(b.parts.length)\n        i is i + 1\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "3\n3\n3\n3",
        "the receiver spill for a method call inside an inner loop re-stored the stale \
         pre-loop struct value every iteration, losing every scalar-field write after the first"
    );
}

#[test]
fn float_neq_is_ieee_unordered() {
    let c = compile(
        "*main\n    n is 0.0 / 0.0\n    log(n neq n)\n    log(n equals n)\n    log(1.5 neq 1.5)\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "1\n0\n0",
        "NaN neq NaN must be true (UNE), NaN equals NaN false (OEQ)"
    );
}

#[test]
fn int_literals_coerce_against_float_calls_on_either_side_and_negated() {
    let c = compile(
        "*sign(x)\n    if x > 0.0\n        return 1.0\n    if x < 0.0\n        return -1.0\n    0.0\n\n*main\n    log(sign(-7) equals -1)\n    log(sign(0) equals 0)\n    log(0 equals sign(0))\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "1\n1\n1");
}

#[test]
fn unannotated_fn_return_type_is_authoritative_across_calls() {
    let c = compile(
        "*f\n    v is vec()\n    v.push(1)\n    v\n\n*main\n    a is f()\n    log(a.get(0) + 1)\n    b is f()\n    b.push('str')\n    log(b.get(1))\n",
    );
    assert!(!c.ok(), "conflicting element type must not compile");
    let stderr = c.stderr();
    assert!(
        stderr.contains("expected `i64`, found `string`") && !stderr.contains("?"),
        "diagnostic must name the resolved type, not a raw type variable: {stderr}"
    );
}

#[test]
fn single_letter_enum_names_unify_with_their_declared_type() {
    let c = compile(
        "type Arr\n    items as Vec of E\n\nenum E\n    N(i64)\n    L(Arr)\n\n*main\n    items is vec()\n    items.push(N(41))\n    a is Arr(items is items)\n    log(a.items.length)\n    match a.items.get(0)\n        N(n) ? log(n + 1)\n        _ ? log(-1)\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "1\n42",
        "the parser reads any single uppercase letter in type position as a type parameter; \
         before [153] a declared enum named E never unified with its own annotation \
         (`expected E, found E`)"
    );
}

#[test]
fn heterogeneous_tuple_layout_survives_returns_and_merges() {
    let c = compile(
        "enum Tok\n    TNum(i64)\n    TStar\n    TEnd\n\n*lex_one(src as String, pos as i64)\n    if pos >= src.length\n        return (TEnd, pos)\n    ch is src.char_at(pos)\n    if ch >= 48 and ch <= 57\n        return (TNum(ch - 48), pos + 1)\n    if ch equals 42\n        return (TStar, pos + 1)\n    (TEnd, pos)\n\n*main\n    t1, p1 is lex_one('3*4', 0)\n    log(p1)\n    match t1\n        TNum(n) ? log(n)\n        _ ? log(-1)\n    t2, p2 is lex_one('3*4', p1)\n    log(p2)\n    match t2\n        TStar ? log(999)\n        _ ? log(-2)\n    0\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "1\n3\n2\n999",
        "tuples were built as [N x first-elem-type] arrays, so a (enum, i64) tuple stored \
         its i64 at the enum-size offset (12) while the canonical {{enum, i64}} return \
         layout reads it at 16 — every position came back as pos >> 32 (usually 0)"
    );
}

#[test]
fn recursive_enum_rebound_in_loop_merges_correctly() {
    let c = compile(
        "enum Node\n    NNum(i64)\n    NMul(Node, Node)\n\n*eval(n as Node)\n    match n\n        NNum(v) ? v\n        NMul(a, b) ? eval(a) * eval(b)\n\n*build(k as i64)\n    left is NNum(3)\n    i is 0\n    loop\n        if i >= k\n            return left\n        left is NMul(left, NNum(2))\n        i is i + 1\n\n*main\n    log(eval(build(2)))\n    0\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "12",
        "a recursive enum payload rebound inside a loop merges values whose LLVM shapes \
         differed per path; before [153] this was a hard codegen error, and the first \
         spill-based fix byte-reinterpreted mismatched layouts instead of coercing \
         element-wise"
    );
}

#[test]
fn generic_struct_args_unify_across_mono_and_structural_spellings() {
    let c = compile(
        "type Box of T\n    value as T\n\n*get_value(b as Box of T) returns T\n    b.value\n\n*rebox(b as Box of T) returns Box of T\n    Box(value is b.value)\n\n*main\n    a as Box of i64 is Box(value is 7)\n    log(get_value(a))\n    b is Box(value is 'a-heap-string-well-past-sso-width')\n    c is rebox(b)\n    log(get_value(c))\n    0\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "7\na-heap-string-well-past-sso-width",
        "constructors minted mono names from unresolved inference variables (Box_?0) while \
         instantiated functions spelled the same type structurally (Struct(Box, [i64])); \
         the spellings never unified and generic-fn type maps defaulted every non-bare \
         parameter to i64"
    );
}

#[test]
fn multi_param_generics_annotate_with_angle_brackets() {
    let c = compile(
        "type Pair of A, B\n    first as A\n    second as B\n\n    *sum_len(self) returns i64\n        self.second.length\n\n*flip(p as Pair<A, B>) returns Pair<B, A>\n    Pair(first is p.second, second is p.first)\n\n*outer_first(p as Pair<i64, Pair<i64, string>>) returns i64\n    p.first\n\n*main\n    p is Pair(first is 5, second is 'hello')\n    log(p.sum_len())\n    q is flip(p)\n    log(q.first)\n    log(q.second)\n    n is Pair(first is 1, second is Pair(first is 2, second is 'deep'))\n    log(outer_first(n))\n    0\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "5\nhello\n5\n1",
        "`p as Pair of A, B` parses as two parameters (the comma is a parameter separator), \
         so multi-argument generics had no expressible annotation; the grammar's documented \
         `Pair<A, B>` form now parses, including nested closers lexed as `>>`"
    );
}

#[test]
fn generic_enums_flow_through_function_boundaries() {
    let c = compile(
        "enum Maybe of T\n    Just(T)\n    Nothing\n\n*or_zero(m as Maybe of i64) returns i64\n    match m\n        Just(v) ? v\n        Nothing ? 0\n\n*wrap(x as T) returns Maybe of T\n    Just(x)\n\n*main\n    a is wrap(41)\n    log(or_zero(a) + 1)\n    b as Maybe of i64 is Nothing\n    log(or_zero(b))\n    0\n",
    );
    assert!(c.ok(), "must compile: {}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "42\n0",
        "a generic enum built through a return-position generic function must reach a \
         concretely-annotated consumer as the same monomorphized enum"
    );
}

fn compile_opt(src: &str, opt: &str) -> Compiled {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("prog.jn");
    std::fs::write(&file, src).unwrap();
    let out = Command::new(jinnc())
        .arg("prog.jn")
        .arg("--opt")
        .arg(opt)
        .arg("-o")
        .arg(dir.path().join("prog.bin"))
        .current_dir(dir.path())
        .output()
        .expect("invoke jinnc");
    Compiled { dir, out }
}

#[test]
fn match_arm_assigning_outer_local_compiles_at_opt0() {
    let c = compile_opt(
        "enum Rec\n    One(Vec of i64)\n\n*sink(xs as take Vec of i64) returns i64\n    xs.length\n\n*main\n    a is vec(1)\n    r is One(a)\n    n is 0\n    match r\n        One(x) ? n is sink(take x)\n    log(n)\n",
        "0",
    );
    assert!(
        c.ok(),
        "codegen emitted blocks in storage order and read a merge-block value before its \
         defining arm block was emitted: {}",
        c.stderr()
    );
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "1\n");
}

#[test]
fn enum_payload_drops_recurse_through_named_spellings() {
    for opt in ["0", "3"] {
        let c = compile_opt(
            "enum Inner\n    Leaf(Vec of i64)\n\nenum Outer\n    Wrap(Inner, Vec of i64)\n\ntype Holder\n    f as Inner\n    n as i64\n\n*main\n    a is vec(1)\n    b is vec(2)\n    o is Wrap(Leaf(a), b)\n    h is Holder(f is Leaf(vec(3)), n is 7)\n    xs as Vec of Inner is vector()\n    xs.push(Leaf(vec(4)))\n    log(h.n)\n    log(xs.length)\n",
            opt,
        );
        assert!(c.ok(), "opt {opt}: {}", c.stderr());
        let run = c.run();
        assert!(run.status.success(), "opt {opt}: {}", exit_desc(&run));
        assert_eq!(String::from_utf8_lossy(&run.stdout), "7\n1\n");
    }
}

#[test]
fn consuming_one_payload_bind_still_rejects_later_subject_use() {
    let c = compile(
        "enum Rec\n    Pair(Vec of i64, Vec of i64)\n\n*sink(xs as take Vec of i64) returns i64\n    xs.length\n\n*main\n    r is Pair(vec(1), vec(2))\n    n is 0\n    match r\n        Pair(x, y) ? n is sink(take x)\n    c is count_of(r)\n    log(n)\n\n*count_of(q as Rec) returns i64\n    1\n",
    );
    assert!(
        !c.ok(),
        "reading the subject after a payload bind was consumed must stay a move error"
    );
}

#[test]
fn ctor_wrapped_push_argument_suppresses_source_drop() {
    let c = compile(
        "enum Inner\n    Leaf(Vec of i64)\n\n*main\n    a is vec(1)\n    xs as Vec of Inner is vector()\n    xs.push(Leaf(a))\n    log(xs.length)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert!(
        run.status.success(),
        "the vec element and the moved-from local both owned the same allocation: {}",
        exit_desc(&run)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "1\n");
}

#[test]
fn store_row_strings_are_owned_and_dropped() {
    let c = compile(
        "store users @simple\n    name as String\n    age as i64\n\n*main\n    insert users 'Alice-with-a-long-heap-name', 30\n    r is users query\n        where age > 10\n    s is r.name\n    log(s)\n    log(r.name)\n    log(r.age)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "Alice-with-a-long-heap-name\nAlice-with-a-long-heap-name\n30\n",
        "store row strings must be readable through a bind and through the row after \
         the ownership change (cap now marks them owned)"
    );
}

#[test]
fn match_on_temp_store_result_returns_cloned_field_at_opt0() {
    for opt in ["0", "3"] {
        let c = compile_opt(
            "store users @simple\n    name as String\n    age as i64\n\n*grab returns String\n    insert users 'Bob-long-heap-string-name-x', 25\n    match users where age > 10\n        Ok(row) ? return row.name\n        Err(e) ? return 'none'\n    return 'unreachable'\n\n*main\n    s is grab()\n    log(s)\n",
            opt,
        );
        assert!(
            c.ok(),
            "opt {opt}: a dead match-merge block after all-returning arms must not \
             reference an elided bind: {}",
            c.stderr()
        );
        let run = c.run();
        assert!(run.status.success(), "opt {opt}: {}", exit_desc(&run));
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "Bob-long-heap-string-name-x\n"
        );
    }
}

#[test]
fn filter_ternary_scalar_result_runs_clean() {
    let c = compile(
        "store users @simple\n    name as String\n    age as i64\n\n*main\n    insert users 'Bob', 25\n    b is users where name equals 'Bob' ? $.age ! 0 - 1\n    log(b)\n",
    );
    assert!(c.ok(), "{}", c.stderr());
    let run = c.run();
    assert!(run.status.success(), "{}", exit_desc(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "25\n");
}
