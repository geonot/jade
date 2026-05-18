use std::env;

fn main() {
    let out = env::var("OUT_DIR").unwrap();

    // Detect target architecture
    let target = env::var("TARGET").unwrap_or_default();
    let asm_file = if target.contains("aarch64") {
        "runtime/context_aarch64.S"
    } else {
        "runtime/context_x86_64.S"
    };

    cc::Build::new()
        .file("runtime/coro.c")
        .file("runtime/deque.c")
        .file("runtime/sched.c")
        .file("runtime/channel.c")
        .file("runtime/actor.c")
        .file("runtime/sup.c")
        .file("runtime/select.c")
        .file("runtime/timer.c")
        .file("runtime/vec.c")
        .file("runtime/wal.c")
        .file("runtime/index.c")
        .file("runtime/version.c")
        .file("runtime/migrate.c")
        .file("runtime/kv.c")
        .file("runtime/vector.c")
        .file("runtime/column.c")
        .file("runtime/bloom.c")
        .file("runtime/fts.c")
        .file("runtime/net.c")
        .file("runtime/fs.c")
        .file("runtime/regex_helper.c")
        .file("runtime/process.c")
        .file("runtime/util.c")
        .file("runtime/terminal.c")
        .file("runtime/event.c")
        .file("runtime/random.c")
        .file(asm_file)
        .opt_level(2)
        .warnings(true)
        .flag("-Wall")
        .flag("-Wextra")
        .flag("-Wshadow")
        .flag("-Wstrict-prototypes")
        .flag("-Wmissing-prototypes")
        .flag("-Wno-unused-parameter")
        .compile("jinn_rt");

    println!("cargo:rustc-link-search=native={out}");
    println!("cargo:rustc-link-lib=static=jinn_rt");
    println!("cargo:rustc-link-lib=pthread");
    println!("cargo:rustc-env=JINN_RT_DIR={out}");

    // ── Optional: compile TLS + crypto modules if OpenSSL is available ──
    let has_openssl = std::process::Command::new("pkg-config")
        .args(["--exists", "openssl"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if has_openssl {
        let openssl_cflags = std::process::Command::new("pkg-config")
            .args(["--cflags", "openssl"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();

        let mut ssl_build = cc::Build::new();
        ssl_build
            .file("runtime/tls.c")
            .file("runtime/crypto.c")
            .opt_level(2)
            .warnings(true)
            .flag("-Wall")
            .flag("-Wextra")
            .flag("-Wshadow")
            .flag("-Wstrict-prototypes")
            .flag("-Wmissing-prototypes")
            .flag("-Wno-unused-parameter");

        for flag in openssl_cflags.split_whitespace() {
            ssl_build.flag(flag);
        }

        ssl_build.compile("jinn_ssl");
        println!("cargo:rustc-link-lib=static=jinn_ssl");
        println!("cargo:rustc-env=JINN_HAS_SSL=1");
    } else {
        println!("cargo:rustc-env=JINN_HAS_SSL=0");
        println!("cargo:warning=OpenSSL not found; std.tls and std.crypto will not be available");
    }

    // ── Optional: compile SQLite module if sqlite3 is available ──
    let has_sqlite = std::process::Command::new("pkg-config")
        .args(["--exists", "sqlite3"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if has_sqlite {
        let sqlite_cflags = std::process::Command::new("pkg-config")
            .args(["--cflags", "sqlite3"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();

        let mut sqlite_build = cc::Build::new();
        sqlite_build
            .file("runtime/sqlite.c")
            .opt_level(2)
            .warnings(true)
            .flag("-Wall")
            .flag("-Wextra")
            .flag("-Wshadow")
            .flag("-Wstrict-prototypes")
            .flag("-Wmissing-prototypes")
            .flag("-Wno-unused-parameter");

        for flag in sqlite_cflags.split_whitespace() {
            sqlite_build.flag(flag);
        }

        sqlite_build.compile("jinn_sqlite");
        println!("cargo:rustc-link-lib=static=jinn_sqlite");
        println!("cargo:rustc-env=JINN_HAS_SQLITE=1");
    } else {
        println!("cargo:rustc-env=JINN_HAS_SQLITE=0");
        println!("cargo:warning=SQLite3 not found; std.sqlite will not be available");
    }

    println!("cargo:rerun-if-changed=runtime/");
}
