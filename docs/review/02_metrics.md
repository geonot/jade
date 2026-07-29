# §2 Quantitative metrics

All counts produced from the working tree on the review date, excluding
`target/`, `.git/`, `.venv/`, and `node_modules/`.

## 2.1 Lines of code by area

| Area                       | LOC      | Notes |
| -------------------------- | -------- | ----- |
| `src/codegen/`             | **29,679** | 35 % of all compiler code |
| `src/typer/`               | 15,879   | inference + monomorphisation |
| `src/mir/`                 | 6,391    | SSA IR + lowering + opt |
| `src/parser/`              | 6,108    | hand-rolled recursive descent |
| `runtime/`                 | 6,273    | C: scheduler, channels, stores, crypto, … |
| `src/driver/`              | 3,099    | CLI + project + pipeline |
| `src/lexer/`               | 1,357    | indent-aware lexer + literals + tests |
| `src/hir/` + `hir_validate.rs` | 1,366 | HIR data + invariants |
| `src/ownership/`           | 1,033    | borrow-check walker on HIR |
| `src/perceus/`             |   944    | MIR-level RC opts (mostly `mir_perceus`) |
| `libjn/`                   | 3,097    | low-level C-shim std (libc-like) |
| `std/`                     | 11,856   | high-level std (collections, fmt, net, etc.) |
| `tests/`                   | 17,545   | mostly `integration.rs` + `bulk_tests.rs` |
| `examples/`                | 645      | a handful of demo apps |
| `apps/`                    | 2,787    | 16 sample apps (blockchain, raft, lattice_crypto, …) |
| `benchmarks/`              | 1,948    | 33 microbenchmarks |

**Total compiler + runtime + stdlib + tests ≈ 84,006 LOC.**

### Observations

- Codegen is **larger than typer + parser + lexer + MIR + HIR + Perceus +
  ownership combined.** That is upside-down. In a healthy LLVM-backed
  compiler the codegen is mostly a structural translation, with most of
  the work done by the typer and the IR. The size here reflects that
  Jinn's codegen re-derives a lot of structure that should have been
  resolved upstream (see §11 for examples — auto-widening, pointer
  arithmetic fallbacks, dynamic-dispatch synthesis).
- The standard library (`std/` 11.8 k + `libjn/` 3.1 k) is sizeable for
  a pre-alpha language but is in two tiers with no documented contract
  between them — see §16.
- Tests are 21 % of the project (17.5 k / 84 k). That is **healthy on
  paper**, but the test corpus is heavily skewed toward
  surface-syntax round-trip checks; see §18.

## 2.2 Compiler-internal code smell census

| Indicator                                        | Count |
| ------------------------------------------------ | ----- |
| `panic!` / `unimplemented!` / `todo!` in `src/` (non-test) | **18** |
| `.unwrap()` in `src/` (non-test)                | **110** |
| `.expect("…")` in `src/` (non-test)             | **404** |
| `unsafe { … }` blocks in `src/`                  | 171 |
| `TODO` / `FIXME` / `HACK` / `XXX` in `src/` (non-test) | **0** |
| `TODO` / `FIXME` / `HACK` / `XXX` in `runtime/`  | **0** |
| `eprintln!` outside cli/driver/lsp               | 14 |
| Cargo build warnings (release)                   | **159** |

Interpretation:

- **0 TODO/FIXME/HACK** anywhere is impressive — it means the codebase
  has been kept scrupulously clean of comment-level tech-debt markers.
  (It does *not* mean there is no tech debt; it means the debt is
  not labelled.)
- **159 unused-import warnings** in a release build is a strong
  code-hygiene smell. It signals that pre-flight `cargo fix` /
  `cargo clippy -- -D warnings` is not part of CI. See §18.
- **404 `.expect()`s** is a lot for 84 kLOC. Most are likely invariant
  guards on `IndexMap::get` and similar; nevertheless, every `.expect()`
  is a latent ICE if the invariant is wrong. This is the same class of
  issue that produced the `into_int_value()` ICE in probe v1's
  `map(v, $ * 2)` case (see §11 and §21).
- **18 `panic!()` / `unreachable!()` / `unimplemented!()`** in non-test
  source. Hotspot files:

  ```
  5  src/escape/mod.rs       "expected Bind in slot 0"
  3  src/codegen/mir_codegen/mod.rs
  2  src/mir/mod.rs
  2  src/codegen/types.rs
  2  src/codegen/mod.rs
  1  src/typer/resolve.rs
  1  src/parser/mod.rs
  1  src/mir/lower/ctx.rs
  1  src/codegen/expr/core.rs   "yield expression outside of coroutine body"
  ```

  Most of these are reachable from user input by construction
  (probes have already hit some).

## 2.3 Test surface

| Suite                          | Tests | Pass | Fail | Ignored |
| ------------------------------ | -----:| ----:| ----:| -------:|
| Library unit tests             |   224 |  224 |    0 |       0 |
| `bulk_tests.rs`                |   913 |  913 |    0 |       1 |
| `integration.rs`               |   423 |  423 |    0 |       1 |
| `mir_bounds_elision.rs`        |     2 |    2 |    0 |       0 |
| `perceus_debug.rs`             |     2 |    2 |    0 |       1 |
| `proptest_smoke.rs`            |     3 |    3 |    0 |       0 |
| `wal_crash.rs`                 |     3 |    3 |    0 |       0 |
| **Total**                      | **1,570** | **1,570** | **0** | **3** |

End-to-end clean. See §18 for what those numbers *don't* cover (no
fuzzing, no sanitizers, no flake detection, no coverage gating).
