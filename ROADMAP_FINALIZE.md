# ROADMAP_FINALIZE.md

Authoritative audit of every item in [ROADMAP.md](ROADMAP.md), the validation
evidence for the items that landed, and the precise breakdown of work that
still remains. Compiled after the second R13/R15/R16 sweep against
`HEAD` (test totals: **1565 passing / 0 failing / 1 ignored** across 7 binaries
in `cargo test --release`).

---

## 1. Validation status of items marked complete

Each row lists the ROADMAP id, the artifact that proves landing, and the
test (or runtime command) that exercises it.

| ROADMAP id | Item | Evidence | Validation |
|------------|------|----------|------------|
| **N-1** (R1) | WAL durability — `fdatasync` on commit | [runtime/wal.c](runtime/wal.c) | `cargo test --release --test wal_crash` (3 tests pass) |
| **N-2** (R3) | Generic constructor `Box<i32>(7)` | [src/typer/expr.rs](src/typer/expr.rs) Struct arm — Uppercase(args) fallback to `lower_call` | `tests::generic_struct_*` (integration.rs) |
| **N-3** (R4) | Supervisor `<sup>_start` / `<sup>_restart_count` registration | [src/typer/lower.rs](src/typer/lower.rs) Decl::Supervisor arm | 3 supervisor integration tests |
| **N-4** | `count <store> where ...` shares predicate evaluator with delete/restore | [tests/integration.rs](tests/integration.rs#L3008-L3025) `store_count_where_basic`, `store_count_where_no_match` | both pass |
| **N-6** (R12) | Query-block desugaring (`store products … where … select …`) | [tests/integration.rs](tests/integration.rs#L2474-L2511) `query_block_select`/`and`/`delete`/`set`/`multi_where` | 5 tests pass |
| **N-7** (R10) | Named-field insert `insert users (name is 'Alice', age is 30)` | [tests/integration.rs](tests/integration.rs#L4402-L4415) | passes; smoke run prints `30` |
| **N-8** (R5) | Green CI / `*main` send syntax | [src/parser/expr.rs](src/parser/expr.rs) | `tests::actor_*` |
| **N-9** (R6) | `jinn.md` reflects current syntax | [jinn.md](jinn.md) | doc only |
| **R7** | Re-baseline benchmarks with documented stub flags | [benchmarks/README.md](benchmarks/README.md), [benchmarks/comparison/](benchmarks/comparison/) | `python3 run_benchmarks.py --runs=3 --quiet` runs cleanly |
| **R8** | Tighten BinOp validation — concrete-type check + actionable diagnostic | [src/typer/expr.rs](src/typer/expr.rs) BinOp arm | 5 negative tests in [tests/integration.rs](tests/integration.rs); ad-hoc `log(true + false)` rejected with `operator '+' not defined for 'bool' and 'bool' (line 2); requires numeric, String, types or a struct with an 'add' method` |
| **R9** | Honest C baselines for `select_latency`, `channel_throughput` | [benchmarks/comparison/select_latency.c](benchmarks/comparison/select_latency.c) (SPSC ring), [benchmarks/comparison/channel_throughput.c](benchmarks/comparison/channel_throughput.c) | both compile via `run_benchmarks.py` |
| **R11** | Dual-Perceus gating (`--debug-perceus` → HIR PerceusPass) | [src/main.rs](src/main.rs) `debug_perceus: bool`, [tests/perceus_debug.rs](tests/perceus_debug.rs) | 1 test passes |
| **R13** (impl) | Amortised store growth via `ftruncate` 64 KiB chunks | [runtime/util.c](runtime/util.c) `jinn_store_reserve`, [runtime/jinn_rt.h](runtime/jinn_rt.h), [src/codegen/store_ops.rs](src/codegen/store_ops.rs), [src/codegen/mir_codegen/store.rs](src/codegen/mir_codegen/store.rs) (logical-end seek replaces SEEK_END after ftruncate) | smoke: `store_ops`, `store_ops_inmem`, `store_perf` benchmarks compile and run correctly |
| **R16** (proptest) | Property tests for lexer/parser stability | [tests/proptest_smoke.rs](tests/proptest_smoke.rs) — 3 tests × 256 cases (`lexer_never_panics`, `parser_never_panics_on_lexer_output`, `known_good_program_parses`); `proptest = "1.5"` in [Cargo.toml](Cargo.toml) | all 3 pass |
| **Asm path** (A.3) | Inline asm acceptance in unsafe blocks | [src/parser/expr.rs](src/parser/expr.rs) | 1 integration test passes |
| **Preflight script** (Part C) | 7-step gate authored | [scripts/preflight.sh](scripts/preflight.sh) | written, executable; **not yet executed end-to-end** — see §3 |

### Items where the *original* claim was inaccurate (corrected here)

* **R8 inference quirk** — re-tested. `a is true; log(a)` prints `true` and HIR
  shows `let a [d2]: bool = true`. The earlier session-summary note ("`a is true`
  infers as i64") was a false alarm; the BinOp validation R8 added catches
  `bool + i64` correctly. **No work needed.**
* **R13 perf measurement** — earlier session compared R13 against `alloc_churn`
  (1.71 → 1.60x J/C this run) and `vec_grow` (1.29 → 1.24x). Those benches do
  not touch stores; R13 cannot affect them. The correct R13 benches are
  `store_ops` / `store_perf`. See §2 for the proper analysis plan.

---

## 2. Remaining work — full breakdown

Ordered by recommended sprint priority (smaller / higher-leverage items first).

### 2.1 Tech-debt blocker — clean lib warnings

Without this, `cargo clippy --release -- -D warnings` (preflight step 4) fails
and the entire preflight gate is unusable.

* **Symptom**: ~16 unused-variable / unused-import warnings in `jinnc` lib
  (`HashMap` unused, `ptr_ty` unused, etc.).
* **Tasks**:
  1. `cargo clippy --release --all-targets 2>&1 | grep -E "^(warning|error)"`
     and fix each (prefer deletion over `#[allow]`).
  2. `cargo fmt --all` to settle formatting drift before step 3 of preflight.
* **Acceptance**: `cargo clippy --release -- -D warnings` exits 0.

### 2.2 N-5 — `(<store> where ...).length` resolution

* **Current state**: Diagnostic was improved
  (`type 'users (query result)' has no field 'length' — for the number of
  matching records use 'count users where …'`) but the *feature* — making
  `.length` work on a `StoreWhere` expression — was never enabled.
* **Tasks**:
  1. In [src/typer/expr.rs](src/typer/expr.rs) field-access arm, add a special
     case for `Type::StoreWhere(_)` + field name `length` returning `Type::I64`.
  2. In codegen, lower it to the same path as `count <store> where ...` (reuse
     `compile_store_count_where`).
  3. Add a passing integration test mirroring `store_count_where_basic` but
     using `.length`.
* **Acceptance**: `(users where age > 20).length` returns `i64` and the new
  test passes. Diagnostic remains for unsupported field accesses.
* **Effort**: S.

### 2.3 R8 — already complete (no work)

See §1 correction. No outstanding work; if a regression is suspected later,
add a guard test:

```jinn
*main
    a is true
    log(a)        # expect: true
    b is true
    log(b + 1)    # expect: hir error
```

### 2.4 R13 — performance analysis (no measured win on store benches)

R13 wired the `ftruncate(round_up_64KiB)` reservation but no benchmark
currently demonstrates the speedup. Investigate before declaring win:

* **Hypotheses to falsify**:
  1. `store_ops` is dominated by `fwrite` stdio buffer flushes, not by
     allocator block allocation — the 14.79s J vs 870 µs C gap (~17,000×) is
     symptomatic of per-record `fflush` or full WAL fsync per insert.
  2. Per-insert `fstat` syscall is itself the bottleneck — cache file size in
     a sidecar struct keyed by `FILE *`.
  3. `JINN_STORE_CHUNK = 64 KiB` may be too small for a 1 M-row insert load;
     try 256 KiB / 1 MiB.
* **Tasks**:
  1. `perf record -F 999 ./store_ops_bench && perf report` — identify the top
     three syscalls / functions.
  2. If `fwrite`/`fflush` dominates: switch the store-write path to direct
     `pwrite(2)` against the chunked region.
  3. Add a microbench that *isolates* the chunk-reservation win (e.g.
     `store_insert_million_no_wal`) and re-measure.
  4. Document final ratio in [benchmarks/README.md](benchmarks/README.md).
* **Acceptance**: An identified, measurable benchmark improves by ≥10% with
  R13 enabled vs `JINN_STORE_CHUNK = sizeof(record)` (one-record reservation).
* **Effort**: M.

### 2.5 N-7 / N-6 — already complete (no work)

Verified: 5 query-block tests + named-field insert test pass; ad-hoc
`insert users (name is 'Alice', age is 30); log r.age` prints `30`.

### 2.6 R15 — full DWARF debug info

* **Current state**: helper `attach_dbg_declare` lives on `Codegen` in
  [src/codegen/mod.rs](src/codegen/mod.rs); two call-sites in
  [src/codegen/stmt.rs](src/codegen/stmt.rs) Stmt::Bind arm (Array case +
  general case). Subprogram-scope push is *commented out* in
  [src/codegen/mir_codegen/mod.rs](src/codegen/mir_codegen/mod.rs)
  (`compile_mir_fn`) because LLVM verifier rejects calls without per-instruction
  `!dbg` location when the enclosing function carries a DI subprogram.
  Result: with `--debug`, `attach_dbg_declare` is a no-op.
* **Root cause**: every `BasicBlock` builder call inside MIR opcode handlers
  must emit `set_current_debug_location` before the call to satisfy
  the verifier rule "inlinable function call in a function with debug info
  must have a !dbg location".
* **Tasks**:
  1. In `mir::Inst` (see [src/mir/](src/mir/)), confirm each instruction
     carries a `Span`. If not, plumb `Span` through MIR lowering — failing
     instructions can fall back to the function's first line.
  2. Introduce `Codegen::set_debug_location(line: u32, col: u32)` wrapping
     `inkwell::debug_info::DebugInfoBuilder::create_debug_location` +
     `Builder::set_current_debug_location`.
  3. In every MIR opcode handler in
     [src/codegen/mir_codegen/](src/codegen/mir_codegen/), invoke
     `self.set_debug_location(inst.span.line, inst.span.col)` *before* the
     first `build_*` call of that handler.
  4. Replace stand-in DIType (currently `__jinn_local`, 64-bit
     `DW_ATE_unsigned`) with proper basic types: `i32` / `i64`
     (`DW_ATE_signed`), `u32`/`u64` (`DW_ATE_unsigned`), `f64`
     (`DW_ATE_float`), pointer types via
     `DIBuilder::create_pointer_type`, struct types via
     `create_struct_type`.
  5. Re-enable `self.comp.create_debug_function(fv, &func.name.as_str(), 1)`
     in `compile_mir_fn`.
  6. Re-enable `self.attach_dbg_declare(...)` calls in stmt.rs (already
     written).
  7. Add `tests/programs/dwarf_smoke.jn` with `--debug -O0` and an LLDB-
     scripted assertion.
* **Acceptance**:
  ```
  jinnc --debug -O0 tests/programs/dwarf_smoke.jn -o /tmp/d
  lldb --batch -o "br set -n main" -o "run" -o "frame variable" /tmp/d
  ```
  prints `(i64) x = 7` (or equivalent) for at least one local.
* **Effort**: L.

### 2.7 R14 — variable-length string encoding for stores

Largest single item. Currently every `String` field in a store schema occupies
a fixed slot (≥256 B based on `runtime/util.c` SSO + padding) — the `users`
record bench measured 812 B/row. Goal is to ship a default `String` =
`[u64 offset][u64 length]` (16 B) into a per-store blob heap appended to the
main records file.

* **Tasks**:
  1. **Schema syntax**: extend [src/parser/decl.rs](src/parser/decl.rs)
     and [src/typer/lower.rs](src/typer/lower.rs) to parse
     `as String<=N>` (fixed-bound, preserves current 256-style behavior) and
     `as String` (default = variable, blob-heap-backed).
  2. **Record layout**: in
     [src/codegen/store_ops.rs](src/codegen/store_ops.rs) and
     [src/codegen/mir_codegen/store.rs](src/codegen/mir_codegen/store.rs),
     emit `[i64 offset][i64 length]` for variable strings; insert path appends
     the bytes to a sidecar `.blob` file (or to the tail of `.store` past a
     reserved heap region).
  3. **Read path**: extend `jinn_store_get_*` runtime helpers in
     [runtime/util.c](runtime/util.c) to dereference offset+length into a
     freshly-allocated `JinnString`.
  4. **Predicate evaluator**: update `jinn_store_predicate_eval` to load
     variable strings before comparison.
  5. **Migration**: stub a `runtime/migrate.c` routine
     `jinn_store_migrate_v1_to_v2(path)` that walks the old fixed-256 file
     and rewrites it into the new layout. Gate with a one-shot `migrate <store>`
     CLI subcommand or auto-detect on open via header magic.
  6. **WAL compatibility**: bump WAL record version; ensure replay handles
     both encodings during the migration window.
  7. **Tests**: integration test inserting 100 strings of length 1..1000,
     reading them back, restarting the process, and reading again.
  8. **Bench**: rerun `store_ops` and verify avg bytes-per-record on `users`
     drops from ~812 to ~40 (16 B record fields + ~24 B string payload).
* **Acceptance**: bench size target met, all 1565 existing tests + new test
  pass, migration round-trip succeeds.
* **Effort**: XL. Recommend its own milestone.

### 2.8 R16 — `cargo-fuzz` targets

Proptest layer landed; coverage-guided fuzzing still missing.

* **Tasks**:
  1. `cd jinn && cargo install cargo-fuzz && cargo fuzz init`.
  2. Write `fuzz/fuzz_targets/fuzz_target_parser.rs` — feed bytes into
     `jinn::lexer::Lexer` then `jinn::parser::Parser`; assert no panic.
  3. Write `fuzz/fuzz_targets/fuzz_target_unify.rs` — generate random
     `Type` trees via `arbitrary` and assert `unify(a, b) == unify(b, a)` and
     `unify(a, a) == a`.
  4. Add a CI invocation `cargo fuzz run fuzz_target_parser -- -max_total_time=60`
     gated behind a `fuzz` workflow tag (don't add to preflight — too slow).
  5. Document in [tests/proptest_smoke.rs](tests/proptest_smoke.rs)
     header + repo `README.md`.
* **Acceptance**: both targets build and run for 60 s without crash.
* **Effort**: M.

### 2.9 Preflight — end-to-end execution

[scripts/preflight.sh](scripts/preflight.sh) was authored but never run as a
single command. Sequence:

1. `cargo build --release`
2. `cargo test --release`
3. `cargo fmt --check`
4. `cargo clippy --release -- -D warnings`  ← will fail until §2.1
5. `bash scripts/alpha_release_smoke.sh`
6. `python3 run_benchmarks.py --runs=3 --quiet`
7. `cargo test --release --test wal_crash`

* **Tasks**:
  1. After §2.1 lands, run `bash scripts/preflight.sh` and capture stdout/stderr.
  2. Triage every failure to a follow-up issue; do not silence with `|| true`.
  3. Add the script as a CI job.
* **Acceptance**: zero non-zero exit codes on a clean checkout.
* **Effort**: S (assuming §2.1 is done).

### 2.10 B-DISPATCH / B-ACTOR baselines

Current state: documented as "no comparable C baseline" in
[benchmarks/README.md](benchmarks/README.md). Acceptable for alpha but
eventually rewrite the C side as an M:N pthread pool to avoid being labelled
strawman comparisons. **Effort**: M, low priority.

---

## 3. Tech debt and known caveats

| # | Item | Where | Impact |
|---|------|-------|--------|
| 1 | ~16 unused-variable / unused-import warnings in lib | jinnc lib | Blocks `cargo clippy -D warnings` → blocks preflight step 4 |
| 2 | DI subprogram scope never pushed | [src/codegen/mir_codegen/mod.rs](src/codegen/mir_codegen/mod.rs) `compile_mir_fn` | `attach_dbg_declare` is a silent no-op even with `--debug` |
| 3 | R13 has no measured win on any benchmark yet | runtime/util.c | Unverified speedup claim — see §2.4 |
| 4 | `JINN_STORE_CHUNK` = 64 KiB hard-coded | [runtime/util.c](runtime/util.c) | Possibly suboptimal for large bulk insert; needs §2.4 analysis |
| 5 | Per-insert `fstat` syscall in `jinn_store_reserve` | [runtime/util.c](runtime/util.c) | Could cache file size in a sidecar struct |
| 6 | Proptest char alphabet excludes tabs | [tests/proptest_smoke.rs](tests/proptest_smoke.rs) | Tabs short-circuit lexer with error — by design but not fuzzed |
| 7 | `(<store> where).length` rejected (good diagnostic) | [src/typer/expr.rs](src/typer/expr.rs) | UX gap — see §2.2 |
| 8 | preflight.sh never run end-to-end in CI | [scripts/preflight.sh](scripts/preflight.sh) | Gate is theoretical until executed |
| 9 | Fixed-size string fields in stores (~256 B/field) | [runtime/util.c](runtime/util.c) | Disk waste; root cause of R14 |
| 10 | One MIR test ignored (`#[ignore]`) | run `cargo test --release -- --ignored` to identify | Document or fix |

---

## 4. Recommended sprint sequence

1. **§2.1 — clean lib warnings** (S, unblocks everything)
2. **§2.9 — run preflight end-to-end** (S, surfaces hidden failures)
3. **§2.2 — N-5 `.length` resolution** (S, polish)
4. **§2.4 — R13 perf analysis** (M, validates an already-shipped change)
5. **§2.6 — R15 full DI** (L, big DX win)
6. **§2.8 — R16 cargo-fuzz** (M, hardening)
7. **§2.7 — R14 variable-length strings** (XL, own milestone)
8. **§2.10 — B-DISPATCH / B-ACTOR baselines** (M, optional polish)

After §2.1 + §2.9 succeed and §2.2 lands, the project can legitimately claim
ROADMAP.md sections A.1–A.5, A.6 except `.length`, B.1, B.2, and B.3 R13/R16
(proptest layer) as complete. R14 + R15 + cargo-fuzz remain the only
substantive open work after that.
