# Testing

## Running the suite

```
scripts/test.sh              # everything, cores filled
scripts/test.sh bulk         # only test binaries matching "bulk"
JOBS=2 scripts/test.sh       # cap concurrent binaries
cargo test --release         # equivalent, slower; still fully supported
```

`scripts/test.sh` builds the same test binaries `cargo test` does and runs
them with the same assertions and exit semantics. The only difference is
scheduling: `cargo test` runs test *binaries* strictly one at a time, so
every suite that cannot saturate the box leaves cores idle. The script runs
several binaries concurrently and prints the failing suites' logs.

## Why the suite costs what it costs

Nearly every test forks `jinnc` to compile and link a small program, so the
suite is dominated by process cost, not by assertion logic. Measured on a
4-core/8-thread laptop for a two-line program:

| phase | cost |
| --- | --- |
| `jinnc` startup before any work | ~16-22 ms (mostly mapping the 157 MB `libLLVM.so`) |
| lex → parse → HIR → MIR → LLVM → object | ~8 ms |
| `cc` link (spawns `ld`) | ~35-50 ms |
| **total per compile-and-run test** | **~90 ms** |

Two consequences worth internalising:

- **Concurrency is capped by cores, not by cleverness.** Past roughly `nproc`
  in-flight compiles, total CPU inflates and wall time gets *worse*. More
  threads is not more speed.
- **The fastest test is the one that does not fork.** Prefer asserting on
  `--emit-hir`/`--emit-mir` output over compile-and-run when a test is really
  about the frontend; it skips the linker, the largest single cost.

Switching linkers was evaluated and rejected: `mold`/`lld` are not present on
the reference machine and `ld.gold` measured *slower* than the default `ld`.

## Writing harnesses that scale

A single `#[test]` that loops over a corpus pins one core for its whole
duration while the rest of the box idles. Corpus harnesses must fan out with
`tests/support/parallel.rs`:

```rust
#[path = "support/parallel.rs"]
mod parallel;

let results = parallel::par_map(items, |item| { /* returns Result */ });
```

`par_map` preserves input order, so failure output stays deterministic. Any
sequential setup (for example `doctest:file` companions accumulating across
blocks) must be resolved into per-item state *before* the parallel map, never
inside it.

Property tests that fork the compiler should be sharded across several
`#[test]` functions so libtest can schedule them in parallel — see
`ownership_fuzz.rs`, where 200 cases run as 8 shards of 25.

## Proptest case counts

Compile-and-run property suites read their case count from
`tests/support/cases.rs`:

```
JINN_PROPTEST_CASES=64 cargo test --release --test coercion_property
```

The default is tuned for a fast edit-test loop. CI (`.github/workflows/ci.yml`)
and `scripts/preflight.sh` set `JINN_PROPTEST_CASES=64` so release gating keeps
the full coverage. Shrunk counterexamples are recorded in
`.proptest-regressions` files and replay at any case count, so a failure found
by a soak run stays pinned as a permanent regression test.
