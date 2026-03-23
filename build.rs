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
        .file("runtime/select.c")
        .file("runtime/timer.c")
        .file(asm_file)
        .opt_level(2)
        .warnings(false)
        .compile("jade_rt");

    println!("cargo:rustc-link-search=native={out}");
    println!("cargo:rustc-link-lib=static=jade_rt");
    println!("cargo:rustc-link-lib=pthread");

    // Expose OUT_DIR so main.rs can find libjade_rt.a at runtime
    println!("cargo:rustc-env=JADE_RT_DIR={out}");

    // Rebuild if runtime changes
    println!("cargo:rerun-if-changed=runtime/");
}
