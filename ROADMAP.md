# Jade — Defect Log & Remediation Roadmap

Evidence-based, derived from the audit session of May 2026. Every item references a
file, line, or empirical observation. Items are sized in t-shirt units (S ≤ 1 day,
M ≤ 1 week, L ≤ 1 month, XL multi-month).

---

## Part A — Defect Log (expanded from Phase 5)

### A.1 Correctness bugs

#### N-2 — Generic constructor `T of U(arg)` miscompiles
- **Severity:** P0 (blocks any user touching generics)
- **Symptom:** `box is Box of i64(7)` type-checks, then MIR codegen emits
  `Load of undefined variable 'Box'`.
- **Repro:** [/tmp/jade_eval/poly2.jade](file:///tmp/jade_eval/poly2.jade)
- **Suspected location:** [src/mir/lower.rs](src/mir/lower.rs) — `Expr::Call` lowering
  for a `Type::Generic` constructor probably resolves the callee as a value-name
  lookup (`Operand::Var("Box")`) instead of as a constructor reference.
- **Tasks:**
  1. Add a failing unit test under `tests/programs/generic_ctor.jade`.
  2. In [src/typer/call.rs](src/typer/call.rs) confirm the call's `resolved_kind` is
     marked as `Constructor` rather than `Function` for generic types.
  3. In `mir::lower::lower_call` branch on `ResolvedKind::Constructor` and emit
     `MirInst::ConstructStruct` with monomorphized type id, instead of `Call`.
  4. Mirror the fix in `MirCodegen::lower_inst` for the `ConstructStruct` arm.
  5. Add 3 more parametric repros (`Pair of (i64, String)(1, "a")`,
     `Option of f64.Some(3.14)`, `Result of (i64, String).Ok(1)`).
- **Estimate:** M
- **Acceptance:** all four programs compile, run, and produce expected output;
  `cargo test` adds 4 passing tests.

#### N-4 — `count <store> where ...` miscompiles
- **Severity:** P0 (advertised query feature)
- **Symptom:** `count users where age > 27` errors with
  `Load of undefined variable 'where'`.
- **Suspected location:** parser likely parses `count users` as expression then
  trips over `where` because `count` is not wired to absorb the `where` clause.
- **Tasks:**
  1. Add failing test `tests/programs/count_where.jade`.
  2. In [src/parser/expr.rs](src/parser/expr.rs), find `Token::Count` handling and
     extend with optional `Token::Where` clause; produce
     `Expr::StoreCount { store, predicate }`.
  3. In [src/typer/expr.rs](src/typer/expr.rs) type-check predicate against the
     store's record type.
  4. In [src/codegen/mir_codegen/store.rs](src/codegen/mir_codegen/store.rs#L838)
     extend the existing "count where deleted == 0" path to take a user predicate.
  5. Reuse `eval_store_predicate` already used by `<store> where ...`.
- **Estimate:** S–M
- **Acceptance:** test passes; `count` and `<store> where` share a single predicate
  evaluator.

#### N-5 — `(<store> where ...).length` is unreachable; diagnostic misleading
- **Severity:** P1
- **Symptom:** `users where age > 27 .length` reports
  `type '__store_users' has no field 'length'` — wrong target type in message.
- **Tasks:**
  1. Add failing test asserting `.length` and `.len` work on the result.
  2. In typer, ensure `Expr::StoreWhere` has result type `Vec<RecordType>`, not
     `__store_users`.
  3. Field lookup on Vec already supports `.length`/`.len` via
     [src/codegen/vec.rs](src/codegen/vec.rs); confirm path is reached.
  4. Improve the diagnostic in [src/typer/expr.rs](src/typer/expr.rs) field-access
     arm to print the Display of the type rather than its store-tag name.
- **Estimate:** S
- **Acceptance:** `(users where age > 27).length` returns the correct `i64`.

#### N-8 — Stale regression test `b_actor_send_syntax_removed`
- **Severity:** P0 (turns CI red)
- **Symptom:** `cargo test --release` shows 1 failure: the test asserts
  `send c, @increment()` is rejected, but it now compiles.
- **Tasks:**
  1. Decide canonically: is `send c, @increment()` valid 0.5 syntax?
  2. If valid: delete the test and add a positive test exercising the syntax.
  3. If invalid: locate the parser rule that admitted it ([src/parser/stmt.rs](src/parser/stmt.rs))
     and reject; the test then passes unchanged.
- **Estimate:** S

#### N-3 — Supervisor codegen is a no-op stub
- **Severity:** P0 (semantic dishonesty)
- **Symptom:** [src/codegen/mod.rs](src/codegen/mod.rs#L1120) ignores
  `sup.strategy` and child arguments; runtime has no link/monitor/restart.
- **Tasks (Option A — implement):**
  1. Add fields to `runtime/actor.c`: `parent_sup`, `link_set`, `restart_policy`.
  2. New runtime APIs: `jade_sup_create(strategy)`, `jade_sup_register_child`,
     `jade_sup_on_child_exit(actor_id, status)`.
  3. Implement OneForOne / OneForAll / RestForOne in `runtime/actor.c`.
  4. Wire `compile_supervisor` to call `jade_sup_create` then register each child
     and bind into a static descriptor.
  5. Spawn each child via `jade_sup_spawn_child(sup, factory_fn, args)`.
  6. Emit a global initializer that starts the supervisor on `main` entry.
  7. Acceptance test: kill child, observe restart counter increment.
- **Tasks (Option B — strike):**
  1. Reject `supervisor` in the parser with a clear "not yet implemented" error.
  2. Move existing supervisor parsing/lowering code behind `#[cfg(feature =
     "supervisor_preview")]` or delete pending re-design.
- **Estimate:** Option A = L; Option B = S
- **Recommended:** Option A — this is foundational to the language's actor pitch.

### A.2 Durability gaps

#### N-1 — WAL is not durable
- **Severity:** P0 (mis-advertised crash safety)
- **Symptom:** zero `fsync`/`fdatasync`/`msync`/`O_SYNC` in `runtime/*.c`; only
  `fflush` in [runtime/wal.c](runtime/wal.c#L61).
- **Tasks:**
  1. After every WAL append, call `fdatasync(fileno(wal->fp))` on Linux;
     `fsync` on macOS.
  2. Add `JADE_WAL_SYNC` env var with values `none|fdatasync|fsync` for tuning.
     Default `fdatasync`.
  3. Add `wal_commit_group()` API for batched fsync at transaction boundaries.
  4. Crash-recovery integration test: child process appends N records,
     `kill -9`, parent reopens, asserts ≥ N records present (records that made
     it through fsync must survive).
  5. Document the durability model in the WAL header comment with explicit
     trade-off (group commit vs single-record latency).
- **Estimate:** M
- **Acceptance:** test passes under `kill -9`; `strace` confirms fdatasync calls.

### A.3 Architecture smells

#### N-12 — Dual Perceus passes
- **Severity:** P1
- **Symptom:** every compile prints both `perceus:` and `mir-perceus:`.
- **Tasks:**
  1. In [src/perceus/mod.rs](src/perceus/mod.rs) and
     [src/perceus/mir_perceus.rs](src/perceus/mir_perceus.rs), enumerate which
     produces hints used by which downstream consumer.
  2. If MIR codegen is the only real consumer of refcount hints, gate the HIR
     pass behind `--debug-perceus` and stop running it on every compile.
  3. Move shared analysis (`uses.rs`, `analysis.rs`) under `src/perceus/common/`
     and consume from both pipelines if both must remain.
- **Estimate:** M
- **Acceptance:** baseline compile time drops by the cost of the redundant pass;
  output diff is empty.

#### N-13 — MIR-native concurrency lowering (CLEANUP §C.1 residual)
- **Severity:** P2 (organizational; no user-visible effect)
- **Symptom:** `src/codegen/{actors,coroutines,lambda,stmt,expr,store_ops,
  store_filter,map,strings,string_ops,string_transform,vec,conversions}.rs`
  (~7,200 LOC) survive solely to serve a small set of MIR entry points
  (`compile_actor_loop`, `compile_supervisor`, `compile_coroutine_create`,
  `compile_spawn`, `gen_migration`, `make_closure`, `eval_store_filter`)
  that walk HIR data structures.
- **Tasks:**
  1. Extend MIR with native opcodes for actor message loops, coroutine state
     machines, supervisor trees, schema migrations, and WHERE-clause
     evaluation (today these are all HIR-shaped).
  2. Teach `src/mir/lower.rs` to lower HIR concurrency / migration / filter
     constructs to those opcodes.
  3. Reimplement the codegen entry points to consume MIR instead of HIR.
  4. Run transitive-reachability sweep and delete the now-dead helper files.
- **Estimate:** L (multi-day MIR project)
- **Acceptance:** `src/codegen/` shrinks by ≈ 5,000 LOC; tests still 1565/0;
  bench parity preserved.

#### Asm path unverified
- **Severity:** P1
- **Tasks:**
  1. Write `tests/programs/asm_smoke.jade`: trivial inline-asm block (e.g.
     `asm x86_64 "mov rax, 42; ret"`).
  2. If codegen path is missing, either implement via LLVM `InlineAsm` or reject
     `asm` at parse time with NYI diagnostic.
- **Estimate:** S–M

### A.4 Documentation

#### N-9 — `jade.md` line 1 corruption
- **Severity:** P0 (first impression)
- **Tasks:** delete the leading `fdddddddddddddd` and trailing whitespace; verify
  with `head -1 jade.md`.
- **Estimate:** trivial

### A.5 Benchmark honesty

#### B-CHANNEL — channel_throughput baseline uses Unix pipes
- **Severity:** P1
- **Tasks:**
  1. Replace [benchmarks/comparison/channel_throughput.c](benchmarks/comparison/channel_throughput.c)
     with an in-process MPMC ring buffer (lock-free or with a single mutex +
     condvar — match the runtime's actual design).
  2. Re-run; expect Jade ≈ 0.9–1.2× C.
- **Estimate:** S

#### B-SELECT — select_latency baseline
- **Severity:** P1
- **Tasks:** rewrite C using `epoll`/condvar over the in-process ring;
  remove the syscall-per-event strawman.
- **Estimate:** S

#### B-DISPATCH — dispatch_yield, sim_for
- **Severity:** P1
- **Tasks:** if the C side cannot model coroutine-yield without ucontext, label
  these "no comparable C baseline" and report Jade absolute throughput only.
- **Estimate:** S

#### B-ACTOR — actor_pingpong, actor_throughput, actor_fanout
- **Severity:** P1
- **Tasks:** rewrite C side as M:N pthread pool with channels; or label as
  "vs naive single-pthread C baseline".
- **Estimate:** S

#### B-STORE — store_ops 43,549×
- **Severity:** P0 (visible in any benchmark report)
- **Tasks:**
  1. Either move Jade benchmark to use `@kv` store (which has true O(1) lookup)
     and keep C as in-memory hash, or
  2. Write an on-disk hash-store C reference for like-for-like comparison.
  3. Tag the existing benchmark `store_ops_inmem_vs_disk` so reviewers know
     the comparison is intentional.
- **Estimate:** S–M

### A.6 Promised but inert features

#### N-6 — `query` blocks parse but don't execute
- **Severity:** P0 (advertised)
- **Tasks:**
  1. Define query semantics: SQL-like? LINQ-like? Comprehension-like?
     Recommend: comprehension form `query users where age > 27 select name`
     desugaring to existing `<store> where … then map`.
  2. Implement desugar in [src/typer/lower.rs](src/typer/lower.rs).
  3. Remove the "future work" comment in
     [tests/programs/query_parse.jade](tests/programs/query_parse.jade); add
     execution assertion.
- **Estimate:** M

#### N-7 — Named-field `insert`
- **Severity:** P1
- **Tasks:**
  1. Extend insert grammar in [src/parser/stmt.rs](src/parser/stmt.rs) to accept
     `insert users (name is "alice", age is 30)` in addition to positional form.
  2. Reorder fields by store schema in typer.
  3. Validate all required fields present; emit clear diagnostic for missing.
- **Estimate:** S

---

## Part B — Roadmap (expanded from Phase 6)

### B.0 Milestones

| Milestone | Contents | Target |
|---|---|---|
| 0.5.1 hotfix | All P0 items in §A | within 2 sprints |
| 0.5.2 quality | All P1 items in §A | next 4 sprints |
| 0.6.0 polish | P2 items in §A + B.4 features | next quarter |

### B.1 P0 — Alpha blockers

#### R1 — WAL durability (covers N-1)
- See §A.2.

#### R2 — Fix `count … where` (covers N-4)
- See §A.1.

#### R3 — Fix generic constructor (covers N-2)
- See §A.1.

#### R4 — Resolve supervisor (covers N-3)
- See §A.1; recommend Option A.

#### R5 — Green CI (covers N-8)
- See §A.1.

#### R6 — Fix jade.md (covers N-9)
- See §A.4.

#### R7 — Re-baseline benchmarks (covers store_ops outlier)
- Minimal version of §A.5 B-STORE before next public benchmark publication.

### B.2 P1 — Material quality wins

#### R8 — Tighten `Addable` constraint (N-10)
- **Tasks:**
  1. In [src/typer/unify.rs](src/typer/unify.rs), when applying the `Addable`
     constraint to a concrete type, reject anything not in
     `{i64, f64, String, Vec, user-impl Add}`.
  2. Move BinOp Add validation out of `hir_validate` into the typer.
  3. Expand the type-mismatch diagnostic with the existing `suggest_fix`
     output (currently only used in some paths).
- **Estimate:** M

#### R9 — Honest C baselines (B-CHANNEL/SELECT/DISPATCH/ACTOR/STORE)
- See §A.5; do all together as one PR with a benchmark methodology document
  in `benchmarks/README.md`.
- **Estimate:** M

#### R10 — Named-field `insert` (N-7)
- See §A.6.

#### R11 — Decide on dual Perceus (N-12)
- See §A.3.

#### R12 — Activate `query` blocks (N-6)
- See §A.6.

### B.3 P2 — Performance & developer experience

#### R13 — Store record-file growth amortization
- **Symptom:** `alloc_churn 1.63×`, `vec_grow 1.21×`.
- **Tasks:**
  1. Pre-allocate record file in 64 KB chunks instead of per-record append.
  2. Track high-watermark separately from logical length.
  3. Add `compact()` runtime call to release unused tail.
- **Estimate:** M

#### R14 — Variable-length string encoding for stores
- **Tasks:**
  1. Add `as String<=N>` to schema syntax for fixed-bound strings (current
     behavior at user request).
  2. Default `as String` becomes `[8B offset][8B length]` indirection into a
     blob heap appended to the record file.
  3. Add migration path for existing fixed-256 stores via
     [runtime/migrate.c](runtime/migrate.c).
- **Estimate:** L
- **Acceptance:** average bytes-per-record on `users` store drops from 812 to
  ~40; full read still O(1) amortized.

#### R15 — DWARF variable locations
- **Tasks:**
  1. In MIR codegen, emit `llvm.dbg.declare`/`llvm.dbg.value` for each
     allocated variable.
  2. Verify with `lldb --batch -o "frame variable"` on a non-trivial program.
- **Estimate:** M

#### R16 — Property tests + fuzzing
- **Tasks:**
  1. Add `proptest` dev-dep; properties:
     - parser: tokens roundtrip via `fmt`;
     - typer: any well-typed program with all annotations is a fixpoint of
       inference + erasure;
     - unify: idempotent and commutative on equivalent types.
  2. `cargo-fuzz` target on the parser entry point with a seed corpus from
     `tests/programs/`.
  3. `cargo-fuzz` target on `unify` with random `Type` trees.
- **Estimate:** M
- **Acceptance:** at least 3 properties + 2 fuzz targets running in CI nightly.

---

## Part C — Cross-cutting acceptance

A single pre-flight script `scripts/preflight.sh` should:

- `cargo build --release` (no warnings)
- `cargo test --release` (1119/1119 green)
- `python3 run_benchmarks.py --runs=3 --quiet`
- `bash scripts/alpha_release_smoke.sh`
- `cargo fmt --check && cargo clippy --release -- -D warnings`
- crash-recovery test from R1
- LSP smoke test (open file, request hover, request go-to-def)

Block release on any non-green step.
