use std::path::{Path, PathBuf};
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

struct Workspace {
    _home: tempfile::TempDir,
    root: tempfile::TempDir,
    cache: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join(".cache").join("jinn").join("cache");
        let root = tempfile::tempdir().unwrap();
        Workspace {
            _home: home,
            root,
            cache,
        }
    }

    fn dep(&self, name: &str, version: &str, manifest: &str) {
        let dir = self.cache.join("example.com").join(name).join(version);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("project.jn"), manifest).unwrap();
        std::fs::create_dir_all(dir.join("source")).unwrap();
        std::fs::write(
            dir.join("source").join(format!("{name}.jn")),
            format!("*{name}_helper\n  1\n"),
        )
        .unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "t@example.com"]);
        git(&dir, &["config", "user.name", "t"]);
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-qm", "fixture"]);
    }

    fn root_manifest(&self, body: &str) {
        std::fs::write(self.root.path().join("project.jn"), body).unwrap();
    }

    fn main_src(&self, body: &str) {
        std::fs::write(self.root.path().join("main.jn"), body).unwrap();
    }

    fn build(&self) -> (bool, String) {
        let out = self.root.path().join("out");
        let result = Command::new(jinnc())
            .arg("build")
            .arg("-o")
            .arg(&out)
            .current_dir(self.root.path())
            .env("HOME", self._home.path())
            .env("JINN_ALLOW_NON_HTTPS_DEPS", "1")
            .output()
            .expect("jinnc failed to start");
        let mut combined = String::from_utf8_lossy(&result.stdout).to_string();
        combined.push_str(&String::from_utf8_lossy(&result.stderr));
        (result.status.success(), combined)
    }

    fn run_output(&self) -> String {
        let out = self.root.path().join("out");
        let result = Command::new(&out)
            .current_dir(self.root.path())
            .output()
            .expect("compiled binary failed to start");
        assert!(
            result.status.success(),
            "binary exited {:?}\n{}",
            result.status.code(),
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8_lossy(&result.stdout).trim().to_string()
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git failed to start");
    assert!(
        status.status.success(),
        "git {args:?} failed in {}",
        dir.display()
    );
}

#[test]
fn single_version_dep_builds_and_runs() {
    let w = Workspace::new();
    w.dep("foo", "1.2.0", "name is 'foo'\nversion is '1.2.0'\n");
    w.root_manifest(
        "name is 'rootpkg'\nversion is '1.0.0'\nentry is 'main.jn'\n\
         require('foo', 'https://example.com/foo', '1.2.0')\n",
    );
    w.main_src("*main\n  log 42\n");
    let (ok, log) = w.build();
    assert!(ok, "scoped single-version build should succeed:\n{log}");
    assert_eq!(w.run_output(), "42");
}

#[test]
fn public_transitive_reach_in_builds() {
    let w = Workspace::new();
    w.dep(
        "bar",
        "1.0.0",
        "name is 'bar'\nversion is '1.0.0'\nvisibility is 'public'\n",
    );
    w.dep(
        "baz",
        "1.0.0",
        "name is 'baz'\nversion is '1.0.0'\n\
         require('bar', 'https://example.com/bar', '1.0.0')\n",
    );
    w.root_manifest(
        "name is 'foo'\nversion is '1.0.0'\nentry is 'main.jn'\n\
         require('baz', 'https://example.com/baz', '1.0.0')\n",
    );
    w.main_src("use baz/bar\n\n*main\n  log 1\n");
    let (ok, log) = w.build();
    assert!(ok, "public reach-in should build:\n{log}");
    assert_eq!(w.run_output(), "1");
}

#[test]
fn internal_transitive_reach_in_is_hard_error() {
    let w = Workspace::new();
    w.dep(
        "bar",
        "1.0.0",
        "name is 'bar'\nversion is '1.0.0'\nvisibility is 'internal'\n",
    );
    w.dep(
        "baz",
        "1.0.0",
        "name is 'baz'\nversion is '1.0.0'\n\
         require('bar', 'https://example.com/bar', '1.0.0')\n",
    );
    w.root_manifest(
        "name is 'foo'\nversion is '1.0.0'\nentry is 'main.jn'\n\
         require('baz', 'https://example.com/baz', '1.0.0')\n",
    );
    w.main_src("use baz/bar\n\n*main\n  log 1\n");
    let (ok, log) = w.build();
    assert!(!ok, "internal reach-in must fail:\n{log}");
    assert!(log.contains("visibility internal"), "{log}");
    assert!(log.contains("main:baz:bar"), "{log}");
    assert!(log.contains("main:baz"), "{log}");
}

#[test]
fn two_live_majors_rejected_with_documented_diagnostic() {
    let w = Workspace::new();
    w.dep("foo", "1.2.0", "name is 'foo'\nversion is '1.2.0'\n");
    w.dep("foo", "2.0.0", "name is 'foo'\nversion is '2.0.0'\n");
    w.dep(
        "mid",
        "1.0.0",
        "name is 'mid'\nversion is '1.0.0'\n\
         require('foo', 'https://example.com/foo', '2.0.0')\n",
    );
    w.root_manifest(
        "name is 'rootpkg'\nversion is '1.0.0'\nentry is 'main.jn'\n\
         require('foo', 'https://example.com/foo', '1.2.0')\n\
         require('mid', 'https://example.com/mid', '1.0.0')\n",
    );
    w.main_src("*main\n  log 1\n");
    let (ok, log) = w.build();
    assert!(!ok, "two live majors must be rejected:\n{log}");
    assert!(log.contains("multi-version coexistence of 'foo'"), "{log}");
    assert!(log.contains("1.2.0"), "{log}");
    assert!(log.contains("2.0.0"), "{log}");
    assert!(log.contains("coherence/traits"), "{log}");
}

#[test]
fn same_major_minor_diff_accepted() {
    let w = Workspace::new();
    w.dep("foo", "1.2.0", "name is 'foo'\nversion is '1.2.0'\n");
    w.dep("foo", "1.0.0", "name is 'foo'\nversion is '1.0.0'\n");
    w.dep(
        "mid",
        "1.0.0",
        "name is 'mid'\nversion is '1.0.0'\n\
         require('foo', 'https://example.com/foo', '1.0.0')\n",
    );
    w.root_manifest(
        "name is 'rootpkg'\nversion is '1.0.0'\nentry is 'main.jn'\n\
         require('foo', 'https://example.com/foo', '1.2.0')\n\
         require('mid', 'https://example.com/mid', '1.0.0')\n",
    );
    w.main_src("*main\n  log 7\n");
    let (ok, log) = w.build();
    assert!(ok, "same-major minor differences must be accepted:\n{log}");
    assert_eq!(w.run_output(), "7");
}
