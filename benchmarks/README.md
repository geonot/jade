# Jinn benchmark methodology

Microbenchmarks comparing Jinn against C (and optionally Rust/Python)
references. `results.csv` is the **committed baseline**: regenerated from
the sources in this directory by `run_benchmarks.py`, carrying its own
provenance header (timestamp, commit, CPU, core count, governor, run
count, warmup count, C compiler) and per-row run counts, medians, stddev,
variance, and min/max. Regenerate and commit it only as a deliberate
re-baselining; `ci/bench_regression.py` gates every PR against it.

Current baseline: i7-8650U (8 threads, powersave governor), 5 runs +
1 warmup per benchmark, gcc 16.1.1 `-O3 -march=native` for the C side.
A laptop under the powersave governor is not a quiet lab machine; that is
why the CSV records variance and why the CI gate compares ratios, not
milliseconds, with a 50ms noise floor.

## What "comparable" means

A ratio is a language comparison only when both sides run the same
algorithm against the same memory hierarchy. Every row's `comparability`
column says what its ratio means — in the CSV itself, not just here:

- **comparable** — same algorithm, same memory. The jinn/c ratio is a
  real language/codegen comparison, and these rows are what CI gates.
- **disk-vs-memory** — `store_ops`: Jinn writes an on-disk store (WAL,
  fsync discipline, fresh files every run); C fills an in-memory array.
  The ~5×10³× ratio prices persistence, not the language. For like-for-like
  see `store_ops_inmem` (comparable, Vec-vs-array).
- **cross-paradigm** — the C side simulates a feature C lacks with a
  different mechanism: OS threads for coroutines (`sim_for`,
  `coroutine_spawn`), a plain queue loop with no scheduler for
  channels/select/generators (`channel_throughput`, `select_latency`,
  `dispatch_yield`). Ratios in either direction measure the paradigm
  difference, not codegen quality.
- **single-thread-C-baseline** — `actor_*`, `parallel_*`: the C side is a
  single pthread; Jinn uses its M:N scheduler. Jinn-faster ratios here
  reflect parallelism the C baseline doesn't attempt. A true pthread-pool
  C baseline may flip several of them.
- **single-language** — no meaningful C counterpart is run
  (`store_perf`, `dispatch_vs_direct`); Jinn-absolute numbers only.

Output equality for the comparable set is verified (byte-identical
stdout, jinn vs C) as of the 2026-07-30 baseline. That check caught
`array_ops` running 30× more iterations on the C side than the Jinn side
— see below.

## State of the comparable set (2026-07-30 baseline)

Median comparable ratio ≈ **1.00×** (range 0.38×–24.66×, 20 rows). The
old `results.csv` this baseline replaces contained rows matching no
source in the tree (e.g. `fibonacci` 340ms/1.0× where current sources
measure 708ms/1.15×; `ackermann` 1.0× vs honest 1.91×; `array_ops` 0.87×
vs honest 24.66×).

Outliers above 1.3× have profiles in `profiles/`:

| Benchmark | Ratio | Diagnosis (see profile) |
|---|---|---|
| `array_ops` | 24.66× | heap malloc/free pair per iteration for a non-escaping array literal; follow-up filed to stack-promote (`profiles/array_ops.md`) |
| `alloc_churn` | 2.16× | measures jinn_xmalloc vs glibc malloc deliberately; allocator tuning, not codegen |
| `ackermann` | 1.91× | gcc inlines recursion deeper; jinn IR clean (`profiles/ackermann.md`) |
| `tight_loop` | 1.48× | LLVM reassociation adds a 3rd op to the loop-carried chain; llvm-mca predicts 1.49× (`profiles/tight_loop.md`) |
| `matrix_mul` | 1.37× | unprofiled; below the 1.5× bar, gated by CI |

`fibonacci`, the review's 2.59× outlier, measures 1.15× from current
sources (`profiles/fibonacci.md`).

Ratios below ~0.8× on comparable rows (e.g. `sieve` 0.38×,
`enum_dispatch` 0.57×) are real measurements but should be read with the
same skepticism the high side gets: where investigated they come from
LLVM-vs-GCC differences on the same IR-shaped program, not Jinn magic.

## Regression gating

`ci/bench_regression.py` (run by the `bench-regression` CI job) re-runs
the suite and fails on a >10% regression of any comparable row's jinn/c
ratio versus the committed CSV. Ratios are hardware-portable (both sides
measured on the same machine in the same run), so the gate works on CI
runners that are slower than the baseline machine. Non-comparable rows
and rows whose baseline jinn median is under 50ms (timer/spawn noise) are
listed in the job log as skipped, never silently dropped.

## Running

```sh
python3 run_benchmarks.py --runs=5 --warmup=1 --langs=jinn,c --csv
```

`scripts/bench_bg.sh` wraps the runner with memory caps and a wall-clock
backstop (`quick` = 1-run smoke, `perf` = timing configuration; note
`vec_grow` needs `BENCH_MEM_KB` ≥ ~8GB and `sim_for`'s C baseline spawns
1000 pthreads whose stack reservations exceed the default cap).

Every timed run (and warmup) starts in a scratch directory scrubbed of
`*.wal`/`*.store` — store benchmarks persist state, and without this,
run N measures a store holding N copies of the data.
