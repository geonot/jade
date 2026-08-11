fn main() {
    let joined = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(jinnc::driver::run)
        .expect("failed to spawn compiler thread")
        .join();
    if joined.is_err() {
        std::process::exit(101);
    }
}
