use std::path::PathBuf;
use std::process::Command;

use proptest::prelude::*;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile_run(src: &str, opt: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("t.jn");
    let out = dir.path().join("t_bin");
    std::fs::write(&jinn, src).unwrap();
    let c = Command::new(jinnc())
        .arg(&jinn)
        .arg("--opt")
        .arg(opt)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("jinnc failed to start");
    assert!(
        c.status.success(),
        "jinnc -O{opt} failed for:\n{src}\nstderr: {}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("binary failed to start");
    assert!(
        r.status.success(),
        "binary -O{opt} exited {:?} for:\n{src}",
        r.status.code()
    );
    String::from_utf8_lossy(&r.stdout).trim().to_string()
}

fn assert_stable(src: &str, expected: &str) {
    let o0 = compile_run(src, "0");
    let o3 = compile_run(src, "3");
    assert_eq!(o0, o3, "-O0 vs -O3 mismatch for:\n{src}");
    assert_eq!(o0, expected, "wrong result for:\n{src}");
}

fn fmt_f64(v: f64) -> String {
    format!("{v:.6}")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]


    #[test]
    fn int_var_to_float_param(a in -1000i64..1000, b in -1000i64..1000) {
        let src = format!(
            "*add(x as f64, y as f64) returns f64\n    x + y\n\n*main\n    a is {a}\n    b is {b}\n    log(add(a, b))\n"
        );
        assert_stable(&src, &fmt_f64(a as f64 + b as f64));
    }


    #[test]
    fn int_literal_to_float_param(a in -1000i64..1000, b in -1000i64..1000) {
        let src = format!(
            "*add(x as f64, y as f64) returns f64\n    x + y\n\n*main\n    log(add({a}, {b}))\n"
        );
        assert_stable(&src, &fmt_f64(a as f64 + b as f64));
    }


    #[test]
    fn int_var_identity(a in -100000i64..100000) {
        let src = format!(
            "*inc(x as i64) returns i64\n    x + 1\n\n*main\n    a is {a}\n    log(inc(a))\n"
        );
        assert_stable(&src, &(a + 1).to_string());
    }


    #[test]
    fn int_var_float_division(a in 1i64..1000, b in 1i64..1000) {
        let src = format!(
            "*div(x as f64, y as f64) returns f64\n    x / y\n\n*main\n    a is {a}\n    b is {b}\n    log(div(a, b))\n"
        );
        assert_stable(&src, &fmt_f64(a as f64 / b as f64));
    }


    #[test]
    fn int_var_to_wider_param(a in -1000i64..1000) {
        let src = format!(
            "*twice(x as i64) returns i64\n    x + x\n\n*main\n    a as i32 is {a}\n    log(twice(a))\n"
        );
        assert_stable(&src, &(a + a).to_string());
    }


    #[test]
    fn chained_float_coercion(a in -500i64..500) {
        let src = format!(
            "*half(x as f64) returns f64\n    x / 2.0\n\n*main\n    a is {a}\n    r is half(a)\n    log(half(r))\n"
        );
        assert_stable(&src, &fmt_f64(a as f64 / 2.0 / 2.0));
    }
}
