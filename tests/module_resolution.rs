use std::path::{Path, PathBuf};
use std::process::Command;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn compile(dir: &Path, entry: &str) -> std::process::Output {
    Command::new(jinnc())
        .arg(entry)
        .arg("-o")
        .arg(dir.join("prog.bin"))
        .current_dir(dir)
        .output()
        .expect("invoke jinnc")
}

fn run(dir: &Path) -> String {
    let out = Command::new(dir.join("prog.bin"))
        .current_dir(dir)
        .output()
        .expect("run");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const ENTRY: &str = "*main\n    x is 41\n    log(x + 1)\n";

#[test]
fn sibling_with_syntax_error_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.jn"), ENTRY).unwrap();
    std::fs::write(dir.path().join("broken.jn"), "*oops(((((\n    ???\n").unwrap();
    let out = compile(dir.path(), "main.jn");
    assert!(
        out.status.success(),
        "sibling syntax error leaked into the compile: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("broken.jn"),
        "no diagnostic may reference a file outside the use-closure: {stderr}"
    );
    assert_eq!(run(dir.path()), "42");
}

#[test]
fn sibling_with_colliding_type_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let entry = "type Vec3\n    x as f64\n    y as f64\n    z as f64\n\n*main\n    v is Vec3(x is 1.5, y is 0.0, z is 0.0)\n    log(v.x)\n";
    std::fs::write(dir.path().join("main.jn"), entry).unwrap();
    std::fs::write(
        dir.path().join("other.jn"),
        "type Vec3\n    a as i64\n    b as i64\n",
    )
    .unwrap();
    let out = compile(dir.path(), "main.jn");
    assert!(
        out.status.success(),
        "colliding sibling type leaked in: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(run(dir.path()), "1.5");
}

#[test]
fn sibling_with_store_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.jn"), ENTRY).unwrap();
    std::fs::write(
        dir.path().join("stored.jn"),
        "store ghosts\n    name as String\n\n*seed()\n    insert ghosts 'boo'\n",
    )
    .unwrap();
    let out = compile(dir.path(), "main.jn");
    assert!(out.status.success());
    assert_eq!(run(dir.path()), "42");
    assert!(
        !dir.path().join("ghosts.store").exists(),
        "a never-imported sibling store must not materialize"
    );
}

#[test]
fn explicit_use_of_sibling_still_resolves() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.jn"),
        "use helper\n\n*main\n    log(helper.triple(14))\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("helper.jn"),
        "*triple(n as i64) returns i64\n    n * 3\n",
    )
    .unwrap();
    let out = compile(dir.path(), "main.jn");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(run(dir.path()), "42");
}

#[test]
fn unimported_sibling_module_is_an_error_naming_use() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.jn"),
        "*main\n    log(helper.triple(14))\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("helper.jn"),
        "*triple(n as i64) returns i64\n    n * 3\n",
    )
    .unwrap();
    let out = compile(dir.path(), "main.jn");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`use helper`"),
        "diagnostic must name the fix: {stderr}"
    );
}

#[test]
fn unannotated_return_resolves_before_callers_read_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("parselib.jn"),
        "*parse\n    rows is vec()\n    row is vec()\n    row.push('xyz')\n    rows.push(row)\n    rows\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("main.jn"),
        "use parselib\n\n*main\n    rows is parselib.parse()\n    total is 0\n    loop rows\n        row is $\n        loop row\n            total is total + $.byte_count\n    log(total)\n",
    )
    .unwrap();
    let out = compile(dir.path(), "main.jn");
    assert!(
        out.status.success(),
        "must compile: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        run(dir.path()),
        "3",
        "before [153] the callee's fns-registry entry kept a raw type variable; the loop \
         binder froze its i64 default and byte_count read vec headers as integers — silent \
         garbage in the hundreds of quintillions"
    );
}

const ALPHA: &str = "type Point\n    x as i64\n    y as i64\n\nenum Mode\n    Fast\n    Slow(i64)\n\n*make() returns Point\n    Point(x is 1, y is 2)\n\n*sum(p as Point) returns i64\n    p.x + p.y\n\n*mode() returns Mode\n    Slow(4)\n\n*mode_val(m as Mode) returns i64\n    match m\n        Fast ? 0\n        Slow(n) ? n\n";

const BETA: &str = "type Point\n    name as string\n    tag as i64\n\nenum Mode\n    Off\n    On(string)\n    Slow\n\n*make() returns Point\n    Point(name is 'b', tag is 7)\n\n*desc(p as Point) returns string\n    p.name + to_string(p.tag)\n\n*mode() returns Mode\n    On('lit')\n\n*mode_val(m as Mode) returns string\n    match m\n        Off ? 'off'\n        On(s) ? s\n        Slow ? 'slow'\n";

fn two_modules(main: &str) -> (tempfile::TempDir, std::process::Output) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.jn"), ALPHA).unwrap();
    std::fs::write(dir.path().join("beta.jn"), BETA).unwrap();
    std::fs::write(dir.path().join("main.jn"), main).unwrap();
    let out = compile(dir.path(), "main.jn");
    (dir, out)
}

#[test]
fn same_named_types_in_two_modules_each_keep_their_own_identity() {
    let (dir, out) = two_modules(
        "use alpha\nuse beta\n\n*main\n    a is alpha.make()\n    b is beta.make()\n    log(alpha.sum(a))\n    log(beta.desc(b))\n    log(a)\n    log(b)\n    log(alpha.mode_val(alpha.mode()))\n    log(beta.mode_val(beta.mode()))\n",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        run(dir.path()),
        "3\nb7\nPoint { x: 1, y: 2 }\nPoint { name: b, tag: 7 }\n4\nlit"
    );
}

#[test]
fn ambiguous_type_reference_names_both_declaration_sites() {
    let (_dir, out) =
        two_modules("use alpha\nuse beta\n\n*main\n    p is Point(x is 1, y is 2)\n    log(p.x)\n");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("`Point` is ambiguous")
            && err.contains("alpha.jn:1:1")
            && err.contains("beta.jn:1:1"),
        "{err}"
    );
}

#[test]
fn cross_module_type_mismatch_renders_module_qualified_names() {
    let (_dir, out) =
        two_modules("use alpha\nuse beta\n\n*main\n    log(beta.desc(alpha.make()))\n");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("`beta.Point`") && err.contains("`alpha.Point`") && !err.contains("__M_"),
        "{err}"
    );
}
