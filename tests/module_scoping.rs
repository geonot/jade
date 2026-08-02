//! Regression tests for module flattening (src/resolve.rs).
//!
//! The flattener renames a module's top-level fns/consts to `module_name` and
//! rewrites references in module bodies. These tests pin the lexical-scoping
//! contract: a local binder (`is`-bind, `for`/match/lambda binder, ...) with
//! the same name as a module-level fn or const shadows it — local uses are
//! never rewritten to the flattened global — and module struct literals keep
//! their named-field meaning after flattening.

use std::path::PathBuf;
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

/// Compile a two-file program (module + main importing it) and run it.
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

/// A local `is`-bind whose name collides with a module-level fn must shadow
/// it: `mask` here is the local 15, not the flattened fn `mymod_mask`.
/// (Review T-MODCAP: this class broke `use std/bit` and `use std/json`.)
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

/// A local that collides with a module-level const must shadow it too, and a
/// lambda parameter must shadow a module fn/const of the same name.
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

/// A `for` binder colliding with a module fn shadows it inside the loop body.
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

/// Named struct-literal fields inside a module keep their meaning after the
/// literal is rewritten to a flattened ctor call: out-of-declaration-order
/// named inits must not be silently made positional.
/// (Review T-MODARGS: `Pair(b is 2, a is 1)` printed `2, 1` after import.)
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

/// After the fix, previously-broken std modules must import cleanly through
/// the normal `use` path. (`bit` and `json` were the review's two confirmed
/// T-MODCAP casualties fixed by scope-aware renaming.)
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
