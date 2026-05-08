# Jinn benchmark methodology

This directory contains microbenchmarks comparing Jinn against equivalent
implementations in C, Python, and Rust. Results are emitted to
`results.csv`, `results.json`, and `history.json`.

## What "comparable" means

A benchmark pair is **comparable** when both sides exercise the same
algorithm against the same memory hierarchy. Where Jinn and the reference
language do not share a feature (e.g. coroutines, on-disk stores), the
benchmark is reported as **single-language** (C cell empty in the CSV) or
explicitly tagged as a cross-paradigm comparison.

## Per-benchmark notes (selected)

| Benchmark | Status | Notes |
|---|---|---|
| `store_ops` | **disk-vs-memory** | Jinn uses an on-disk store (WAL + record file, fsync per group commit). C uses an in-memory `Record[]` array. The ~10⁴× ratio reflects the persistence overhead, not the language gap. For a like-for-like comparison see `store_ops_inmem` (in progress) which uses a Jinn `Vec` to mirror the C-side data structure. |
| `channel_throughput` | **language-only** | C cell intentionally empty — the C baseline used Unix pipes (one syscall per send), which is not representative of Jinn's in-process MPMC channel. A new C baseline using a single-mutex+condvar ring is planned (R9). |
| `select_latency` | **language-only** | Same rationale as above — C baseline used `epoll` per event; Jinn's `select` is in-process. |
| `dispatch_yield`, `sim_for` | **language-only** | C lacks a portable coroutine primitive; reported as Jinn absolute throughput only. |
| `actor_*` | **single-thread C baseline** | C side is a single-pthread loop dequeuing from a mutex-protected ring. Jinn uses its M:N scheduler. The Jinn-faster results reflect that work; replacing the C baseline with a true M:N pthread pool (R9) is planned and may flip several ratios. |
| `coroutine_spawn` | **disk-vs-memory** | Jinn spawn includes scheduler queue insertion + coroutine stack allocation; C is a function call. Reported as a Jinn-internal latency measurement. |

## Running

```sh
python3 run_benchmarks.py --runs=5
```

`--quiet` suppresses per-run output. `--filter=name` runs a subset.
