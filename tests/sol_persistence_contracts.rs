use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn harness(dir: &Path) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = dir.join("persistence-contracts");
    let mut cc = Command::new("cc");
    cc.args(["-O1", "-g", "-Wall", "-Wextra", "-Werror", "-Iruntime"]);
    for name in [
        "fwrite",
        "fflush",
        "fsync",
        "fdatasync",
        "fopen",
        "rename",
        "ftruncate",
        "unlink",
    ] {
        cc.arg(format!("-Wl,--wrap={name}"));
    }
    cc.args([
        "tests/sol_persistence_contracts.c",
        "runtime/durable.c",
        "runtime/txn_journal.c",
        "runtime/wal.c",
        "runtime/kv.c",
        "runtime/bloom.c",
        "runtime/column.c",
        "runtime/vector.c",
        "runtime/version.c",
        "runtime/fts.c",
        "runtime/index.c",
        "-pthread",
        "-lm",
        "-o",
    ])
    .arg(&bin)
    .current_dir(root);
    let out = cc.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    bin
}

fn run(bin: &Path, dir: &Path, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .env("JINN_WAL_SYNC", "fdatasync")
        .current_dir(dir)
        .output()
        .unwrap()
}

fn clone_state(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if entry.file_type().unwrap().is_file() && !name.to_string_lossy().ends_with(".lock") {
            std::fs::copy(entry.path(), to.join(name)).unwrap();
        }
    }
}

fn compile(dir: &Path, name: &str, source: &str, opt: &str) -> PathBuf {
    let path = dir.join(format!("{name}.jn"));
    let binary = dir.join(name);
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_jinnc"))
        .arg(&path)
        .args(["--opt", opt, "-o"])
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    binary
}

fn journal_image(bytes: &mut Vec<u8>, path: &Path, data: &[u8]) {
    let path = path.to_str().unwrap().as_bytes();
    let start = bytes.len();
    bytes.extend_from_slice(&(path.len() as u32).to_ne_bytes());
    bytes.extend_from_slice(&(data.len() as u64).to_ne_bytes());
    bytes.extend_from_slice(path);
    bytes.extend_from_slice(data);
    let hash = bytes[start..]
        .iter()
        .fold(14695981039346656037u64, |h, byte| {
            (h ^ u64::from(*byte)).wrapping_mul(1099511628211)
        });
    bytes.extend_from_slice(&hash.to_ne_bytes());
}

#[test]
fn corrupt_journal_refuses_before_restoring_any_participant_and_preserves_evidence() {
    let work = tempfile::tempdir().unwrap();
    let bin = harness(work.path());
    for corruption in ["header", "checksum", "path"] {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.store");
        let b = dir.path().join("b.store");
        std::fs::write(&a, b"new a").unwrap();
        std::fs::write(&b, b"new b").unwrap();
        let mut journal = b"JINNTXN1".to_vec();
        journal_image(&mut journal, &a, b"old a");
        if corruption == "path" {
            journal_image(&mut journal, &work.path().join("outside.store"), b"old b");
        } else {
            journal_image(&mut journal, &b, b"old b");
            if corruption == "header" {
                journal[0] ^= 1;
            } else {
                *journal.last_mut().unwrap() ^= 1;
            }
        }
        let evidence = dir.path().join(".jinn-transaction");
        std::fs::write(&evidence, &journal).unwrap();
        let output = run(&bin, dir.path(), &["path", "a.store"]);
        assert!(!output.status.success(), "accepted {corruption}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("jinn: persistence:"));
        assert_eq!(std::fs::read(&a).unwrap(), b"new a");
        assert_eq!(std::fs::read(&b).unwrap(), b"new b");
        assert_eq!(std::fs::read(&evidence).unwrap(), journal);
    }
}

#[test]
fn interrupted_journal_append_restores_all_complete_images() {
    let work = tempfile::tempdir().unwrap();
    let bin = harness(work.path());
    for tail in [
        b"".as_slice(),
        &[2, 0],
        &[2, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0, b'/'],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.store");
        std::fs::write(&a, b"new").unwrap();
        let mut journal = b"JINNTXN1".to_vec();
        journal_image(&mut journal, &a, b"old");
        journal.extend_from_slice(tail);
        std::fs::write(dir.path().join(".jinn-transaction"), journal).unwrap();
        let output = run(&bin, dir.path(), &["path", "a.store"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read(&a).unwrap(), b"old");
        assert!(!dir.path().join(".jinn-transaction").exists());
    }
}

#[test]
fn persistence_directory_scope_and_process_exclusion_fail_closed() {
    let work = tempfile::tempdir().unwrap();
    let bin = harness(work.path());
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    std::fs::write(dir.path().join("regular"), b"data").unwrap();
    std::fs::write(work.path().join("outside"), b"outside").unwrap();
    std::os::unix::fs::symlink(work.path().join("outside"), dir.path().join("symlink")).unwrap();
    std::fs::hard_link(dir.path().join("regular"), dir.path().join("hardlink")).unwrap();
    for path in [
        "nested/data",
        "symlink",
        "hardlink",
        ".jinn-transaction.extra",
        "nested",
    ] {
        let output = run(&bin, dir.path(), &["path", path]);
        assert!(!output.status.success(), "accepted {path}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("jinn: persistence:"));
    }
    let output = run(&bin, dir.path(), &["cwd"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("working directory changed"));
    let mut child = Command::new(&bin)
        .arg("hold")
        .current_dir(dir.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready, "ready\n");
    let output = run(&bin, dir.path(), &["path", "unused"]);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("store-directory process exclusion"));
}

#[test]
fn native_multistore_and_sidecar_commit_rollback_trap_and_exit_recover_together() {
    let schema = "store records @versioned\n    name as String @index @search\n    value as i64 @bloom\n\nstore totals @simple @column\n    value as i64\n\nstore cache @kv\n\n";
    let changes = "        insert records 'new', 2\n        set records where name equals 'old' value 3\n        insert totals 2\n        cache.set('value', 2)\n";
    for opt in ["0", "3"] {
        let work = tempfile::tempdir().unwrap();
        let seed = compile(
            work.path(),
            "seed",
            &format!(
                "{schema}*main\n    transaction\n        insert records 'old', 1\n        insert totals 1\n        cache.set('value', 1)\n"
            ),
            opt,
        );
        let reader = compile(
            work.path(),
            "reader",
            &format!(
                "{schema}*main\n    log(count records)\n    log(count totals)\n    log(cache.get('value').unwrap_or(-1))\n    log(totals.sum(value))\n    log(records.maybe(value, 2))\n    found is records.search(name, 'new')\n    log(found.length)\n    log(records.history(1).length)\n    match records where name equals 'new'\n        Ok(row) ? log(row.value)\n        Err(error) ? log(-1)\n"
            ),
            opt,
        );
        for outcome in ["commit", "rollback", "trap", "exit"] {
            let dir = tempfile::tempdir().unwrap();
            assert!(run(&seed, dir.path(), &[]).status.success());
            let source = match outcome {
                "rollback" => format!(
                    "{schema}err Failure\n    Stop\n    S(StoreError)\n\nimpl From of StoreError for Failure\n    *from(e as StoreError) returns Failure is S(e)\n\n*change returns Result of i64, Failure\n    transaction\n{changes}        err Stop\n    Ok(1)\n\n*main\n    match change()\n        Ok(value) ? log(value)\n        Err(error) ? log('rolled back')\n"
                ),
                "trap" => format!(
                    "{schema}*main\n    transaction\n{changes}        zero is 0\n        log(1 / zero)\n"
                ),
                "exit" => format!(
                    "{schema}extern *_exit(code as i32)\n\n*main\n    transaction\n{changes}        _exit(91)\n"
                ),
                _ => format!("{schema}*main\n    transaction\n{changes}    log('committed')\n"),
            };
            let writer = compile(work.path(), outcome, &source, opt);
            let output = run(&writer, dir.path(), &[]);
            assert_eq!(
                output.status.success(),
                matches!(outcome, "commit" | "rollback"),
                "{outcome}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            if outcome == "exit" {
                assert!(
                    dir.path().join(".jinn-transaction").exists(),
                    "{outcome} opt{opt}: {:?}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            for _ in 0..2 {
                let output = run(&reader, dir.path(), &[]);
                assert!(
                    output.status.success(),
                    "{outcome}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let expected = if outcome == "commit" {
                    "2\n2\n2\n3\ntrue\n1\n1\n2\n"
                } else {
                    "1\n1\n1\n1\nfalse\n0\n0\n-1\n"
                };
                assert_eq!(
                    String::from_utf8_lossy(&output.stdout),
                    expected,
                    "{outcome} opt{opt}"
                );
                assert!(!dir.path().join(".jinn-transaction").exists());
            }
        }
    }
}

#[test]
fn fields_without_a_persistence_encoding_reject_in_check_and_native_modes() {
    let dir = tempfile::tempdir().unwrap();
    for ty in [
        "Vec of i64",
        "Map of i64",
        "%i64",
        "(i64, bool)",
        "([i64])",
        "Option of i64",
        "Result<i64, String>",
        "Boxed",
        "Color",
        "() returns i64",
    ] {
        for decorator in ["", " @mem"] {
            let source = format!(
                "type Boxed\n    value as i64\n\nenum Color\n    Red\n    Blue\n\nstore records{decorator}\n    value as {ty}\n\n*main\n    nop\n"
            );
            let path = dir.path().join("unsupported.jn");
            std::fs::write(&path, source).unwrap();
            for mode in ["check", "0", "3"] {
                let binary = dir.path().join("unsupported");
                let mut command = Command::new(env!("CARGO_BIN_EXE_jinnc"));
                if mode == "check" {
                    command.arg("check").arg(&path);
                } else {
                    command.arg(&path).args(["--opt", mode, "-o"]).arg(&binary);
                }
                let output = command.output().unwrap();
                assert!(
                    !output.status.success(),
                    "accepted {ty}{decorator} in {mode}"
                );
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(
                    stderr.contains("without a defined persistence encoding"),
                    "{ty}{decorator} in {mode}: {stderr}"
                );
                assert!(!binary.exists());
            }
        }
    }
}

#[test]
fn primitive_field_encodings_preserve_values_across_commit_and_reopen() {
    let schema = "store records @simple\n    label as String\n    a as i8\n    b as i16\n    c as i32\n    d as i64\n    e as u8\n    f as u16\n    g as u32\n    h as u64\n    x as f32\n    y as f64\n    flag as bool\n\n";
    for opt in ["0", "3"] {
        let work = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let seed = compile(
            work.path(),
            "primitives-seed",
            &format!(
                "{schema}*main\n    transaction\n        insert records 'values', -128, -32768, -2147483648, -9223372036854775808, 255, 65535, 4294967295, 18446744073709551615, 1.25, -2.5, true\n"
            ),
            opt,
        );
        let reader = compile(
            work.path(),
            "primitives-reader",
            &format!(
                "{schema}*main\n    match records where label equals 'values'\n        Ok(row) ?\n            log(row.label)\n            log(row.a)\n            log(row.b)\n            log(row.c)\n            log(row.d)\n            log(row.e)\n            log(row.f)\n            log(row.g)\n            log(row.h)\n            log(row.x equals 1.25)\n            log(row.y equals -2.5)\n            log(row.flag)\n        Err(error) ? log('missing')\n"
            ),
            opt,
        );
        let output = run(&seed, dir.path(), &[]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for _ in 0..2 {
            let output = run(&reader, dir.path(), &[]);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                "values\n-128\n-32768\n-2147483648\n-9223372036854775808\n255\n65535\n4294967295\n18446744073709551615\ntrue\ntrue\ntrue\n",
                "opt{opt}"
            );
        }
    }
}

#[test]
fn invalid_or_exceeded_snapshot_caps_refuse_before_mutation() {
    let schema = "store records\n    value as i64\n\n";
    for opt in ["0", "3"] {
        let work = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let seed = compile(
            work.path(),
            "cap-seed",
            &format!("{schema}*main\n    insert records 7\n"),
            opt,
        );
        let writer = compile(
            work.path(),
            "cap-writer",
            &format!(
                "{schema}*main\n    transaction\n        insert records 42\n    log('committed')\n"
            ),
            opt,
        );
        let reader = compile(
            work.path(),
            "cap-reader",
            &format!(
                "{schema}*main\n    log(count records)\n    match records where value equals 7\n        Ok(row) ? log(row.value)\n        Err(error) ? log(-1)\n"
            ),
            opt,
        );
        assert!(run(&seed, dir.path(), &[]).status.success());
        for cap in [
            "",
            "0",
            "-1",
            "invalid",
            "1.0",
            "999999999999999999999999",
            "1",
        ] {
            let before = std::fs::read(dir.path().join("records.store")).unwrap();
            let output = Command::new(&writer)
                .env("JINN_TXN_SNAPSHOT_MAX", cap)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(!output.status.success(), "accepted cap {cap:?}");
            assert!(output.stdout.is_empty());
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("JINN_TXN_SNAPSHOT_MAX"),
                "{cap:?}: {stderr}"
            );
            assert_eq!(
                std::fs::read(dir.path().join("records.store")).unwrap(),
                before
            );
            let reopened = run(&reader, dir.path(), &[]);
            assert!(
                reopened.status.success(),
                "{}",
                String::from_utf8_lossy(&reopened.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&reopened.stdout), "1\n7\n");
            assert!(!dir.path().join(".jinn-transaction").exists());
        }
        let output = Command::new(&writer)
            .env("JINN_TXN_SNAPSHOT_MAX", "1048576")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), "committed\n");
        let reopened = run(&reader, dir.path(), &[]);
        assert!(reopened.status.success());
        assert_eq!(String::from_utf8_lossy(&reopened.stdout), "2\n7\n");
    }
}

#[test]
fn every_persistence_boundary_preserves_atomic_recovery_and_acknowledgement() {
    let work = tempfile::tempdir().unwrap();
    let bin = harness(work.path());
    let original = tempfile::tempdir().unwrap();
    let init = run(&bin, original.path(), &["init"]);
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    for action in ["commit", "rollback"] {
        for operation in [
            "fwrite",
            "fflush",
            "fsync",
            "fdatasync",
            "fopen",
            "rename",
            "ftruncate",
            "unlink",
        ] {
            let baseline = tempfile::tempdir().unwrap();
            clone_state(original.path(), baseline.path());
            let out = run(&bin, baseline.path(), &[action, operation, "0", "0"]);
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let text = String::from_utf8_lossy(&out.stdout);
            let count: usize = text.trim().strip_prefix("ack ").unwrap().parse().unwrap();
            for boundary in 1..=count {
                for mode in ["0", "1", "2"] {
                    let dir = tempfile::tempdir().unwrap();
                    clone_state(original.path(), dir.path());
                    let out = run(
                        &bin,
                        dir.path(),
                        &[action, operation, &boundary.to_string(), mode],
                    );
                    assert!(
                        !out.status.success(),
                        "{action} {operation} {boundary} {mode}: unexpected acknowledgement"
                    );
                    let recovered = run(&bin, dir.path(), &["verify"]);
                    assert!(
                        recovered.status.success(),
                        "{action} {operation} {boundary}/{count} mode {mode}:\nfault: {}\nreopen: {}",
                        String::from_utf8_lossy(&out.stderr),
                        String::from_utf8_lossy(&recovered.stderr)
                    );
                    let value = String::from_utf8_lossy(&recovered.stdout);
                    assert!(
                        value.trim() == "1" || (action == "commit" && value.trim() == "2"),
                        "{action} {operation} {boundary} {mode}: {value}"
                    );
                    assert!(!dir.path().join(".jinn-transaction").exists());
                }
            }
            let recovered = run(&bin, baseline.path(), &["verify"]);
            assert!(
                recovered.status.success(),
                "{}",
                String::from_utf8_lossy(&recovered.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&recovered.stdout).trim(),
                if action == "commit" { "2" } else { "1" }
            );
        }
    }
}
