use std::fmt::Write;
use std::path::Path;
use std::process::Command;

fn expect(source: &str, expected: &str) {
    let formatted = jinnc::fmt::format_source(source).unwrap();
    assert_eq!(jinnc::fmt::format_source(&formatted).unwrap(), formatted);
    for text in [source, &formatted] {
        for opt in ["0", "3"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("formatter.jn");
            let binary = dir.path().join("formatter");
            std::fs::write(&path, text).unwrap();
            let compiled = Command::new(env!("CARGO_BIN_EXE_jinnc"))
                .arg(&path)
                .args(["--opt", opt, "-o"])
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "{}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            let output = Command::new(binary)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                expected,
                "opt {opt}: {text}"
            );
        }
    }
}

#[test]
fn subtraction_division_and_unary_oracle() {
    expect(
        "*main\n    log(10 - (3 - 1))\n    log(16 / (4 / 2))\n    log(-(1 + 2))\n",
        "8\n8\n-3\n",
    );
}

#[test]
fn float_reassociation_is_not_formatting() {
    expect(
        "*main\n    a is 10000000000000000.0\n    b is -10000000000000000.0\n    c is 1.0\n    log((a + (b + c)) equals 0.0)\n    log(((a + b) + c) equals 1.0)\n",
        "true\ntrue\n",
    );
}

#[test]
fn expression_operand_positions_execute_identically() {
    expect(
        "*increment(x as i64) returns i64\n    x + 1\n\n*decrement(x as i64) returns i64\n    x - 1\n\n*main\n    log((2 pow 3) pow 2)\n    log(2 pow (3 pow 2))\n    log((-(1 + 2)) as i64)\n    log((~(1 + 2)) as strict i64)\n    log(not (true and false))\n    log((true ? increment ! decrement)(4))\n    log((true ? [7, 8] ! [9, 10])[1])\n    log((true ? (false ? 1 ! 2) ! 3))\n    log(((true ? false ! true) ? 4 ! 5))\n    log((|x| x + 1)(6))\n    log((true ? 'abc' ! 'de').byte_count)\n",
        "64\n512\n-3\n-4\ntrue\n5\n8\n2\n5\n7\n3\n",
    );
}

#[test]
fn every_binary_pair_preserves_grouping() {
    let operators = [
        "+", "-", "*", "/", "%", "pow", "equals", "neq", "<", "<=", ">", ">=", "and", "or", "xor",
        "|", "^", "&", "<<", ">>", ">>>",
    ];
    for left in operators {
        for right in operators {
            for expr in [
                format!("(a {left} b) {right} c"),
                format!("a {left} (b {right} c)"),
            ] {
                let source = format!("*main\n    result is {expr}\n");
                let formatted =
                    jinnc::fmt::format_source(&source).unwrap_or_else(|e| panic!("{expr}: {e}"));
                assert_eq!(
                    jinnc::fmt::format_source(&formatted).unwrap(),
                    formatted,
                    "{expr}"
                );
            }
        }
    }
}

#[test]
fn prefix_cast_postfix_and_handler_matrix() {
    let expressions = [
        "-(a + b)",
        "~(a + b)",
        "not (a and b)",
        "(not a) equals b",
        "(-a) as u8",
        "(-a) as strict i64",
        "(-a) as json",
        "(a + b) as i64",
        "(a + b)(c)",
        "(a ? b ! c)(d)",
        "(a + b)[c]",
        "(a ? b ! c).field",
        "(a ? b ! c).method(d)",
        "(|x| x + 1)(a)",
        "(|x| x + 1) + a",
        "a + (|x| x + 1)",
        "(a ? b ! c) ? d ! e",
        "a ? (b ? c ! d) ! e",
        "a ? b ! (c ? d ! e)",
        "(a !! b) ? c ! d",
        "a ? (b !! c) ! d",
        "(a ? b ! c) !! d",
        "a !! (b !! c)",
        "(a ~ b) ~ c",
        "a ~ (b ~ c)",
        "%(a + b)",
        "@(a + b)",
        "freeze (a + b)",
        "(a + b) from c to d",
        "a from (b and c) to (d or e)",
        "a of (b + c)",
        "(a,)",
        "()",
        "a + (receive b)",
        "a + (get rows b)",
        "a + (yield b)",
    ];
    for expr in expressions {
        let source = format!("*main\n    result is {expr}\n");
        let formatted =
            jinnc::fmt::format_source(&source).unwrap_or_else(|e| panic!("{expr}: {e}"));
        assert_eq!(
            jinnc::fmt::format_source(&formatted).unwrap(),
            formatted,
            "{expr}"
        );
    }
}

#[test]
fn desugared_control_flow_executes_identically() {
    expect(include_str!("../snippets/1-100/s046.jn"), "55\n");
    expect(
        "*main\n    loop 0, $ < 3, $ + 1\n        log($)\n    if Some(4) is Some(x)\n        log(x)\n",
        "0\n1\n2\n4\n",
    );
}

#[test]
fn metadata_desugaring_and_top_level_statements_survive() {
    for source in [
        "enum Permission\n    Read is 1\n    Write is 2\n\n*main\n    log(Read as i64)\n",
        "@inline @cold\n*id of T: Display(x as T) needs pure\n    x\n",
        "*effect(x as i64) needs fs.read '/tmp/escaped\\'name', io\n    x\n",
        "*function(self as i64)\n    self\n",
        "*function(self)\n    self\n",
        "value is 1\nlog(value)\n",
        "*main\n    atomic x is 1\n    atomic x += 2\n",
        include_str!("../snippets/1-100/s038.jn"),
        include_str!("../snippets/1-100/s046.jn"),
        include_str!("../snippets/guide_tour.jn"),
    ] {
        let formatted =
            jinnc::fmt::format_source(source).unwrap_or_else(|e| panic!("{e}: {source}"));
        assert_eq!(jinnc::fmt::format_source(&formatted).unwrap(), formatted);
    }
}

#[test]
fn unsupported_printer_forms_refuse_without_emitting_or_writing() {
    for source in [
        "migration 'Upgrade' version 2\n    up\n        alter Rows\n            add value as i64\n",
        "view Recent from Rows\n    limit 2\n",
        "*main\n    syscall(1)\n",
        "*main\n    f is |x| do\n        log(x)\n        x\n    end\n",
        "supervisor Root\n    strategy is one_for_all\n    children\n        Worker\n",
        "trait Comparable of T\n    type Item\n    *compare(other as T) returns bool\n",
    ] {
        let tokens = jinnc::lexer::Lexer::new(source).tokenize().unwrap();
        jinnc::parser::Parser::new(tokens)
            .parse_program()
            .unwrap_or_else(|e| panic!("invalid fixture: {e}: {source}"));
        let error = jinnc::fmt::format_source(source).expect_err(source);
        assert!(error.contains("refusing output"), "{error}");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unsupported.jn");
        std::fs::write(&path, source).unwrap();
        for args in [vec!["fmt"], vec!["fmt", "--write"]] {
            let output = Command::new(env!("CARGO_BIN_EXE_jinn"))
                .args(args)
                .arg(&path)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        }
    }
}

fn copy_project(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let path = entry.unwrap().path();
        let target = dst.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_project(&path, &target);
        } else if path.extension().is_some_and(|e| e == "jn") {
            let source = std::fs::read_to_string(&path).unwrap();
            let formatted = jinnc::fmt::format_source(&source)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(
                jinnc::fmt::format_source(&formatted).unwrap(),
                formatted,
                "{}",
                path.display()
            );
            std::fs::write(target, formatted).unwrap();
        }
    }
}

#[test]
fn app_projects_frontend_check_in_resolved_context() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("apps");
    let mut checked = 0;
    let mut failures = String::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let app = entry.unwrap().path();
        let main = app.join("source/main.jn");
        if !main.exists() {
            continue;
        }
        let before = Command::new(env!("CARGO_BIN_EXE_jinnc"))
            .arg(&main)
            .arg("--emit-hir")
            .output()
            .unwrap();
        assert!(
            before.status.success(),
            "{}: {}",
            main.display(),
            String::from_utf8_lossy(&before.stderr)
        );
        let dir = tempfile::tempdir().unwrap();
        copy_project(&app.join("source"), &dir.path().join("source"));
        let after = Command::new(env!("CARGO_BIN_EXE_jinnc"))
            .arg(dir.path().join("source/main.jn"))
            .arg("--emit-hir")
            .output()
            .unwrap();
        if !after.status.success() {
            writeln!(
                failures,
                "{}: {}",
                app.display(),
                String::from_utf8_lossy(&after.stderr)
            )
            .unwrap();
        }
        checked += 1;
    }
    assert!(checked >= 20, "checked {checked}");
    assert!(failures.is_empty(), "{failures}");
}
