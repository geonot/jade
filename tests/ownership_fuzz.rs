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

    ChainInto(u8, u8),

    TogetherCapture(u8, u8),

    ScopedBind(u8),

    BindBox(u8),
    FieldMove(u8),
    BoxTagLog(u8),
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
        (0u8..4, 0u8..4).prop_map(|(a, b)| Stmt::ChainInto(a, b)),
        (0u8..4, 0u8..4).prop_map(|(a, b)| Stmt::TogetherCapture(a, b)),
        (0u8..4).prop_map(Stmt::ScopedBind),
        (0u8..2).prop_map(Stmt::BindBox),
        (0u8..2).prop_map(Stmt::FieldMove),
        (0u8..2).prop_map(Stmt::BoxTagLog),
    ]
}

fn emit(stmts: &[Stmt]) -> String {
    let mut s = String::new();
    s.push_str("type Box2\n    items as Vec of i64\n    tag as i64\n\n");
    s.push_str("*sink(v as take Vec of i64)\n    log(v.len())\n\n");
    s.push_str("*chain(v) returns Vec of i64\n    return v\n\n");
    s.push_str("*pusher(v, base as i64)\n    v.push(base)\n\n");
    s.push_str("*main\n");
    let mut declared = [false; 4];
    let mut boxes = [false; 2];
    let mut scoped = 0usize;
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
            Stmt::ChainInto(a, b) => {
                if declared[*a as usize] && a != b {
                    s.push_str(&format!("    v{b} is chain(v{a})\n"));
                    declared[*b as usize] = true;
                }
            }
            Stmt::TogetherCapture(a, b) => {
                if declared[*a as usize] && declared[*b as usize] {
                    s.push_str("    together\n");
                    s.push_str(&format!("        dispatch\n            pusher(v{a}, 1)\n"));
                    s.push_str(&format!("        dispatch\n            pusher(v{b}, 2)\n"));
                }
            }
            Stmt::ScopedBind(v) => {
                if declared[*v as usize] {
                    let w = scoped;
                    scoped += 1;
                    s.push_str(&format!(
                        "    if v{v}.len() >= 0\n        w{w} is v{v}\n        log(w{w}.len())\n"
                    ));
                }
            }
            Stmt::BindBox(k) => {
                s.push_str(&format!("    b{k} is Box2(items is [1, 2], tag is {k})\n"));
                boxes[*k as usize] = true;
            }
            Stmt::FieldMove(k) => {
                if boxes[*k as usize] {
                    let w = scoped;
                    scoped += 1;

                    s.push_str(&format!("    w{w} is b{k}.items\n    log(w{w}.len())\n"));
                }
            }
            Stmt::BoxTagLog(k) => {
                if boxes[*k as usize] {
                    s.push_str(&format!("    log(b{k}.tag)\n"));
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


        let cerr = String::from_utf8_lossy(&c.stderr);
        prop_assert!(
            !cerr.contains("internal compiler error") && !cerr.contains("RUST_BACKTRACE"),
            "compiler ICE on:\n{src}\nstderr: {cerr}"
        );




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

#[test]
fn generator_emits_review_shapes() {
    let src = emit(&[Stmt::BindVec(0), Stmt::ChainInto(0, 1), Stmt::LenLog(1)]);
    assert!(src.contains("v1 is chain(v0)"), "{src}");
    let dir = tempfile::tempdir().unwrap();
    let jinn = dir.path().join("t.jn");
    std::fs::write(&jinn, &src).unwrap();
    let c = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(dir.path().join("t_bin"))
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "§3.1 shape must compile and run single-drop: {}",
        String::from_utf8_lossy(&c.stderr)
    );

    let src = emit(&[Stmt::BindVec(2), Stmt::TogetherCapture(2, 2)]);
    assert!(src.contains("together"), "{src}");
    let jinn = dir.path().join("t2.jn");
    std::fs::write(&jinn, &src).unwrap();
    let c = Command::new(jinnc())
        .arg(&jinn)
        .arg("-o")
        .arg(dir.path().join("t2_bin"))
        .output()
        .unwrap();
    assert!(
        !c.status.success(),
        "§3.2 shape (same vec captured twice) must be rejected"
    );
    assert!(
        String::from_utf8_lossy(&c.stderr).contains("moved into a concurrent task"),
        "{}",
        String::from_utf8_lossy(&c.stderr)
    );
}
