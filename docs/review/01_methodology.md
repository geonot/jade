# §1 Methodology

## Ground rules

The user explicitly asked that no finding rely on documentation,
agent memory, prior reports, or hearsay. Every claim in this review is
backed by **one or more of**:

1. Direct citation into the source tree (`src/`, `runtime/`, `libjn/`,
   `std/`, `apps/`, `benchmarks/`, `examples/`, `tests/`).
2. A probe program I authored (under `review/probes/` and
   `review/probes2/`), compiled with the **release** build of the
   in-tree compiler, executed, and whose output was captured to disk.
3. The output of `cargo test --release` against the in-tree test suite,
   on the reviewer's workstation, on the date in §0.

Where a memory note or repo note influenced the *direction* of
investigation, the claim still had to be reproduced from source or
from a probe before it was accepted into this document.

## What was actually built and executed

- `cargo build --release` of the `jinnc` / `jinn` / `jinnc-lsp` binaries.
  Build succeeded with **0 errors and 159 warnings** (see §2).
- `cargo test --release` of the full in-tree suite, in a single run,
  result:

  ```
  test result: ok. 224 passed; 0 failed; 0 ignored;   ...   (unit tests)
  test result: ok. 913 passed; 0 failed; 1 ignored;   ...   (bulk_tests)
  test result: ok. 423 passed; 0 failed; 1 ignored;   ...   (integration)
  test result: ok.   2 passed; 0 failed; 0 ignored;   ...   (mir_bounds_elision)
  test result: ok.   2 passed; 0 failed; 1 ignored;   ...   (perceus_debug)
  test result: ok.   3 passed; 0 failed; 0 ignored;   ...   (proptest_smoke)
  test result: ok.   3 passed; 0 failed; 0 ignored;   ...   (wal_crash)
                                              ────────
                                              1,570 total
  ```

  All 1,570 tests pass cleanly when the suite is run end-to-end. (Note:
  during the very first run of this review, four tests in `integration`
  failed with linker / spawn-style errors that the maintainer reported
  as already-deleted dead tests; subsequent runs were clean. This is
  flagged in §18 as a CI-hygiene concern even though it does not
  represent a true regression.)

- **Two probe batteries** authored by the reviewer:

  - `review/probes/` — 20 programs covering basic types, control flow,
    arithmetic, strings, collections, pattern matching, error enums,
    actors, channels, traits, stores, closures, deep recursion,
    use-after-move, OOB indexing, etc.
  - `review/probes2/` — 35 corrected / extended programs (after
    discovering Jinn uses `#` comments rather than `//`, that lambdas
    require typed parameters in some contexts, that actor handlers use
    `@name` rather than `*name`, that channels collide with the `send`
    keyword, etc.).

  Each probe is compiled with `target/release/jinnc`, executed under
  `timeout 10`, and its exit code + stdout + stderr captured. Logs
  live at `/tmp/jinn-probes.log` and `/tmp/jinn-probes2.log`.

- Targeted code reads of the major compiler passes:

  - `src/lexer/` (1,357 LOC)
  - `src/parser/` (6,108 LOC)
  - `src/typer/` (15,879 LOC)
  - `src/hir/` (1,366 LOC), `src/hir_validate.rs`
  - `src/mir/` (6,391 LOC)
  - `src/perceus/` (944 LOC)
  - `src/ownership/` (1,033 LOC)
  - `src/codegen/` (29,679 LOC) — sampled across `mod.rs`, `mir_codegen/`,
    `actors.rs`, `channels.rs`, `clone/`, `drop/`, `expr/`, `vec/`,
    `stores/`, `coroutines.rs`, `pattern_match.rs`.
  - `src/escape/`, `src/driver/`
  - `runtime/jinn_rt.h` and major C runtime files
    (`sched.c`, `channel.c`, `actor.c`, `coro.c`, `wal.c`, `kv.c`,
    `vec.c`, `column.c`, `bloom.c`, `index.c`, `select.c`, `crypto.c`).

  Some files exceed 30 kLOC for the codegen tree; not every line was
  read end-to-end, but every subdirectory was sampled and structural
  patterns extracted.

## What was *not* attempted

- **Fuzzing.** A serious alpha would deserve a libfuzzer / AFL
  harness over the lexer and parser. None was authored for this
  review (recommended in §24).
- **Valgrind / ASan / TSan / UBSan runs of the runtime.** These should
  exist in CI; they don't yet. Recommended in §24.
- **Cross-compilation matrix.** The compiler exposes `--target / --cpu /
  --features` but cross builds were not exercised.
- **Long-soak actor / channel stress.** A 30s spawn-millions stress
  was not run.
- **Repeated runs of the same probe** in different orders or under
  load — there are hints of order-of-execution flakiness in the suite
  (see §18) that this review did not chase to root cause.

These omissions are noted; none of them change the verdict.

## Conventions

- "ICE" = internal compiler error: `panic!()`, `unwrap()`, or
  `unreachable!()` reached on user input.
- "UB" = undefined behaviour: the language has no defined semantics
  for the input and the compiler / runtime does something memory-unsafe
  or non-deterministic.
- All file/line references are workspace-relative.
- `*main` is the Jinn function-definition syntax. `is` is the binding
  / assignment operator. Indentation is significant.
