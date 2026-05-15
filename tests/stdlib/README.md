# Stdlib Tests

Per-module tests for `std/*.jn` modules. Each file is a runnable Jinn program
using inline `test 'name'` blocks (see `tests/programs/inline_tests.jn`).

Run a single module's tests with:

    ./target/release/jinnc test tests/stdlib/math_tests.jn

Convention: one `*_tests.jn` per std module. Tests exercise the public API
surface and a handful of edge cases per function. They are intentionally fast
(no I/O, no networking) so they can run in a tight inner loop.

Modules exercised here are the high-traffic ones; `libjn/*` is FFI-stub only
(every function is a `nop` returning a sentinel) and is not tested at the
Jinn level — those names are validated by the FFI integration tests in
`tests/integration.rs`.
