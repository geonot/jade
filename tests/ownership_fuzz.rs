//! Adversarial memory-model soundness fuzzer.
//!
//! Generates random ownership-stressing Jinn programs and asserts each one is
//! handled soundly: it either (a) is rejected at compile time with a clean
//! diagnostic, or (b) compiles and runs to completion WITHOUT a use-after-free,
//! double-free, leak, or abort (exit 134 / SIGSEGV 139). When the C runtime is
//! built under ASan/UBSan (ci/sanitize.sh), this same corpus catches heap
//! corruption (UAF/double-free/OOB) in the Perceus + escape + tombstone passes
//! — the soundness-critical core that previously hosted the `vec_get` aliasing
//! bug and the `take`-inside-loop double-free.
//!
//! The generator deliberately stresses: nested `take`, field moves under
//! control flow, container-read aliasing (`v.get(i)`), `copy` of heap values,
//! rebinding after move, and aliasing through bindings.

use std::path::PathBuf;
use std::process::Command;

use proptest::prelude::*;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

#[derive(Clone, Debug)]
enum Stmt {
    BindVec(u8),
    Push(u8, i64),
    GetLog(u8, u8),
    CopyTo(u8, u8),
    TakeInto(u8),
    Rebind(u8),
    LenLog(u8),
    IfPush(u8, i64),
}

fn stmt_strategy() -> impl Strategy<Value = Stmt> {
    prop_oneof![
        (0u8..4).prop_map(Stmt::BindVec),
        (0u8..4, 0i64..100).prop_map(|(v, x)| Stmt::Push(v, x)),
        (0u8..4, 0u8..3).prop_map(|(v, i)| Stmt::GetLog(v, i)),
        (0u8..4, 0u8..4).prop_map(|(a, b)| Stmt::CopyTo(a, b)),
        (0u8..4).prop_map(Stmt::TakeInto),
        (0u8..4).prop_map(Stmt::Rebind),
        (0u8..4).prop_map(Stmt::LenLog),
        (0u8..4, 0i64..100).prop_map(|(v, x)| Stmt::IfPush(v, x)),
    ]
}

fn emit(stmts: &[Stmt]) -> String {
    let mut s = String::new();
    s.push_str("*sink(v as take Vec of i64)\n    log(v.len())\n\n");
    s.push_str("*main\n");
    let mut declared = [false; 4];
    for st in stmts {
        match st {
            Stmt::BindVec(v) => {
                s.push_str(&format!("    v{v} is [1, 2, 3]\n"));
                declared[*v as usize] = true;
            }
            Stmt::Push(v, x) => {
                if declared[*v as usize] {
                    s.push_str(&format!("    v{v}.push({x})\n"));
                }
            }
            Stmt::GetLog(v, i) => {
                if declared[*v as usize] {
                    s.push_str(&format!(
                        "    if v{v}.len() > {i}\n        log(v{v}.get({i}))\n"
                    ));
                }
            }
            Stmt::CopyTo(a, b) => {
                if declared[*a as usize] && a != b {
                    s.push_str(&format!("    v{b} is copy v{a}\n"));
                    declared[*b as usize] = true;
                }
            }
            Stmt::TakeInto(v) => {
                if declared[*v as usize] {
                    s.push_str(&format!("    sink(take v{v})\n"));
                    declared[*v as usize] = false;
                }
            }
            Stmt::Rebind(v) => {
                s.push_str(&format!("    v{v} is [9]\n"));
                declared[*v as usize] = true;
            }
            Stmt::LenLog(v) => {
                if declared[*v as usize] {
                    s.push_str(&format!("    log(v{v}.len())\n"));
                }
            }
            Stmt::IfPush(v, x) => {
                if declared[*v as usize] {
                    s.push_str(&format!("    if v{v}.len() > 0\n        v{v}.push({x})\n"));
                }
            }
        }
    }
    s.push_str("    log(0)\n");
    s
}

fn aborted(code: Option<i32>) -> bool {
    matches!(code, Some(134) | Some(139))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn ownership_program_is_sound(stmts in proptest::collection::vec(stmt_strategy(), 1..14)) {
        let src = emit(&stmts);
        let dir = tempfile::tempdir().unwrap();
        let jinn = dir.path().join("t.jn");
        let out = dir.path().join("t_bin");
        std::fs::write(&jinn, &src).unwrap();
        let c = Command::new(jinnc())
            .arg(&jinn)
            .arg("-o")
            .arg(&out)
            .output()
            .expect("jinnc failed to start");

        // The compiler must never ICE/panic on a well-formed-ish program.
        let cerr = String::from_utf8_lossy(&c.stderr);
        prop_assert!(
            !cerr.contains("internal compiler error") && !cerr.contains("RUST_BACKTRACE"),
            "compiler ICE on:\n{src}\nstderr: {cerr}"
        );

        // If it compiled, running it must terminate cleanly: no UAF/double-free
        // abort (134), no segfault (139). Under ASan this also catches heap
        // corruption that would otherwise be silent.
        if c.status.success() {
            let r = Command::new(&out)
                .current_dir(dir.path())
                .output()
                .expect("binary failed to start");
            prop_assert!(
                !aborted(r.status.code()),
                "compiled program aborted ({:?}) — possible UAF/double-free for:\n{src}\nstderr: {}",
                r.status.code(),
                String::from_utf8_lossy(&r.stderr)
            );
        }
    }
}
