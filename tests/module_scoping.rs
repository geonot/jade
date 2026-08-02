use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile_and_run_with_module(module_name: &str, module_src: &str, main_src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let module = dir.path().join(format!("{module_name}.jn"));
    let main = dir.path().join("main.jn");
    let out = dir.path().join("main_bin");
    std::fs::write(&module, module_src).unwrap();
    std::fs::write(&main, main_src).unwrap();
    let output = Command::new(jinnc())
        .arg(&main)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("jinnc failed to start");
    assert!(
        output.status.success(),
        "jinnc compilation failed:\nstderr: {}\nmodule:\n{module_src}\nmain:\n{main_src}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(&out)
        .current_dir(dir.path())
        .output()
        .expect("compiled binary failed to start");
    assert!(
        run.status.success(),
        "binary exited with {:?}\nstderr: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8(run.stdout).unwrap()
}

#[test]
fn local_bind_shadows_module_fn() {
    let out = compile_and_run_with_module(
        "mymod",
        r#"
*shadow_test returns i64
    mask is 15
    return mask + 1

*mask(x as i64) returns i64
    return x * 100
"#,
        r#"
use mymod

log(mymod.shadow_test())
"#,
    );
    assert_eq!(out.trim(), "16");
}

#[test]
fn local_and_lambda_binders_shadow_module_const() {
    let out = compile_and_run_with_module(
        "constmod",
        r#"
a is 1000

*lambda_test returns i64
    f is |a| a + 1
    return f(5)

*local_test returns i64
    a is 7
    return a * 2
"#,
        r#"
use constmod

log(constmod.lambda_test())
log(constmod.local_test())
"#,
    );
    assert_eq!(out.trim(), "6\n14");
}

#[test]
fn for_binder_shadows_module_fn() {
    let out = compile_and_run_with_module(
        "formod",
        r#"
*step returns i64
    return 999

*sum_test returns i64
    total is 0
    for step in 0 to 4
        total is total + step
    return total
"#,
        r#"
use formod

log(formod.sum_test())
"#,
    );
    assert_eq!(out.trim(), "6");
}

#[test]
fn module_struct_literal_keeps_named_fields() {
    let out = compile_and_run_with_module(
        "pairmod",
        r#"
type Pair
    a as i64
    b as i64

*make returns Pair
    return Pair(b is 2, a is 1)
"#,
        r#"
use pairmod

p is pairmod.make()
log(p.a)
log(p.b)
"#,
    );
    assert_eq!(out.trim(), "1\n2");
}

#[test]
fn std_bit_and_json_import() {
    for m in ["bit", "json"] {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main.jn");
        let out = dir.path().join("main_bin");
        std::fs::write(&main, format!("use std/{m}\n\nlog('ok')\n")).unwrap();
        let output = Command::new(jinnc())
            .arg(&main)
            .arg("-o")
            .arg(&out)
            .output()
            .expect("jinnc failed to start");
        assert!(
            output.status.success(),
            "use std/{m} failed to compile:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
