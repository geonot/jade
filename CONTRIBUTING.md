# Contributing to Jinn

Thanks for your interest. This guide codifies the conventions enforced by
[CLEANUP.md](CLEANUP.md). Follow them and your patches will sail through review.

## Build & test

```sh
cargo build                 # debug build of the compiler
cargo test --release        # full test suite (~1565 tests, < 30s)
cargo fmt                   # format Rust code
cargo clippy -- -D warnings # lint (must be clean)
```

The C runtime in `runtime/` is built by `build.rs` with strict flags
(`-Wall -Wextra -Wshadow -Wstrict-prototypes -Wmissing-prototypes`). Any new
warning blocks merge.

The toolchain is pinned in `rust-toolchain.toml` (currently 1.91.1).

## Repo layout

See [docs/architecture.md](docs/architecture.md) for the pipeline tour and
file-to-role mapping.

## Coding conventions

### Rust

- **Edition 2024**, max width 100, 4-space indent, Unix newlines (see `rustfmt.toml`).
- **No `.unwrap()` on fallible code paths.**
  - For LLVM builder calls that cannot fail in valid IR, use the project's `b!()` macro.
  - For invariant violations, use `.expect("ICE: <what assumption was violated>")`.
  - For genuinely fallible operations, propagate with `?`.
  - Run `grep -rn '\.unwrap()' src --include='*.rs' | wc -l` — keep production count < 100.
- **No `panic!`** outside `panic!("ICE: ...")` for invariant violations or `unreachable!()`.
- **Top-of-file `//!` doc** stating purpose, key types, public entry points (≤ 10 lines).
- **Imports** grouped: std, external crates, internal crates, super/self. `cargo fmt` handles ordering.
- **Module size**: target ≤ 800 LOC. Split along natural axes (see CLEANUP.md §C.2).

### C runtime

- All public ABI in `runtime/jinn_rt.h`; every `.c` includes it.
- Prefixes: `jinn_*` (public), `c_*` (helper), `__jinn_*` (codegen-internal).
- Opaque types: `typedef struct Foo Foo;` in header + `struct Foo { ... };` in owning `.c`.
- Error returns: resource-creating → pointer/NULL; mutating → int errno-style.
- See [runtime/README.md](runtime/README.md) for "Adding a new runtime function" step-by-step.

### Jinn stdlib

- Every `std/*.jn` opens with a 5-line doc comment: purpose, primary types, primary functions, examples.
- Names are `snake_case`.
- Each exported symbol has at least one test in `tests/programs/std/`.

## Lexer & keywords

- The KEYWORDS table in `src/lexer.rs` is the single source of truth.
- Adding a keyword: extend KEYWORDS, regenerate `docs/lexer/keywords.md`, wire to a parser arm.
- Reserving a keyword: add it with a `// reserved for: <future>` comment and a `#[doc(hidden)]` test asserting it remains reserved.

## Tests

- Add unit tests next to the code under `#[cfg(test)] mod tests`.
- Add integration programs to `tests/programs/<category>/` where category is `lex`, `parse`, `type`, `mir`, `codegen`, `runtime`, `stdlib`, `store`, `actor`, `coro`.
- Diagnostic text is snapshotted; if you intentionally change a diagnostic, update the snapshot.

## Pull request checklist

- [ ] `cargo build` clean (no warnings).
- [ ] `cargo test --release` green (1565+ passing).
- [ ] `cargo fmt` applied.
- [ ] `cargo clippy -- -D warnings` clean.
- [ ] Every new file has a top-of-file doc.
- [ ] Public ABI changes (runtime/std) are documented.
- [ ] No new `.unwrap()` calls on fallible paths.
- [ ] User-visible changes noted in `ROADMAP.md` or release notes.

## Filing issues

Include:
- Jinn source that reproduces the bug (minimum failing example).
- Output of `jinn --version`.
- Expected vs actual behavior.
- For ICE bugs: full panic message including the `ICE: ...` description.

## License

By contributing you agree that your contributions are licensed under the
same terms as the project.
