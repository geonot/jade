pub fn cases(default: u32) -> u32 {
    std::env::var("JINN_PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
