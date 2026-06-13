# Jinn Remediation Tracker

Consolidated, authoritative inventory of all remediation work derived from:

- `JINN_LANGUAGE_REVIEW_2026_06.md` — the standing-panel language review (§8 actionable items).
- `JINN_AUDIT_2026_06_02.md` — the build/test/run audit (findings + P0 soundness bugs).
- `store-improvement.md` — the 12-item persistent-store review.

This file is the single source of truth for *what remains*. It mirrors the task
tree under `.ryu/tasks/`. Each item carries its task ID, source, priority, and
current status. Completed items are recorded for provenance; remaining items are
grouped by theme and ordered for execution.

Legend: ✅ done · ◻ remaining · ⊘ blocked.

---

## 1. Status summary

| Group | Done | Remaining | Blocked |
|---|---:|---:|---:|
| P0 — must-fix before alpha (task 1) | 4 | 0 | 0 |
| Audit P0 soundness bugs (tasks 2-23..2-29) | 7 | 0 | 0 |
| Crash-safety soundness bugs (task 2-30) | 5 | 0 | 0 |
| Error model (tasks 2-1..2-4) | 11 | 0 | 0 |
| Concurrency (tasks 2-5..2-10) | 3 | 3 | 0 |
| Memory / unsafe boundary (tasks 2-11..2-14) | 0 | 3 | 1 |
| Diagnostics / tooling (tasks 2-15, 2-18..2-22) | 0 | 5 | 1 |
| Store query language (tasks 2-16, 2-17) | 0 | 2 | 0 |
| Store improvements (task 2-31, 12 items) | 2 | 9 | 1 |
| Result ctor monomorphization (task 4) | 1 | 0 | 0 |

---

## 2. Completed (provenance)

These shipped and are pinned by tests. Do not reopen.

### 2.1 Review §8.1 — bugs fixed during review
- ✅ Numeric coercion miscompilation (int→float for variables); `Coerce`→`Cast` lowering.
- ✅ `Vec.shift`/`first`/`last` implemented in codegen.
- ✅ Module-qualified free-function calls (`queue.pick_next`) resolve in HIR.
- ✅ `runtime/random.c` missing prototypes.
- ✅ Dead coercion cluster + `tag_param_ownership` deleted (zero warnings).
- ✅ Module path double-prefix bug.
- ✅ `jinn.md` corruption repaired.

### 2.2 P0 (task 1)
- ✅ **1-1** Store artifact hygiene + clippy CI guarantee.
- ✅ **1-2** Single source of truth for builtin method surfaces.
- ✅ **1-3** Pin String/Unicode semantics (UTF-8).
- ✅ **1-4** Design + implement bounded polymorphism (traits/protocols).

### 2.3 Audit P0 soundness bugs
- ✅ **2-23** Integer div/mod by zero: UB → trap.
- ✅ **2-24** Vec OOB: SIGSEGV → diagnostic.
- ✅ **2-25** Generator IR return-type mismatch (invalid LLVM IR).
- ✅ **2-26** MIR internal verifier.
- ✅ **2-27** `take` keyword in argument position.
- ✅ **2-28** Stack-overflow diagnostic instead of silent SIGSEGV.
- ✅ **2-29** Keywords shadow method names after `Dot`.

### 2.4 Error model (tasks 2-1..2-4)
- ✅ Full error-effect system: prelude `Option`/`Result`, `err`-raise, quaternary
  `e ? ok ! nothing !! err`, implicit propagation, checked R1–R6 + SCC fixpoint,
  `From` graph (C1–C4). 27/27 `error_effects` tests pass.

### 2.5 Concurrency (partial)
- ✅ **2-5** Structured concurrency model design (`docs/structured-concurrency.md`).
- ✅ **2-8** Observable send-after-close behavior.

### 2.6 Stores / misc
- ✅ **2-31-1** Real transactions: begin/commit/rollback over WAL.
- ✅ **2-31-2** StoreError: `@unique`/`@required` violations via error model
  (builtin `err StoreError`, quaternary `?`/`!!` handling, precise trap
  diagnostics for bare insert/set; 12-test `tests/store_errors.rs`).
- ✅ **4** Result Ok/Err ctor monomorphization collision for same T, different E.

### 2.7 Crash-safety soundness bugs (task 2-30, all 5)
- ✅ `take` inside a loop body is now a compile error (`check_loop_body_moves`).
- ✅ Bound `Result` match no longer ICEs (`Param`→`Enum` annotation resolution +
  `Param`/`TypeVar` payload sizing in `declare_tagged_union`).
- ✅ `String.slice` out-of-bounds traps.
- ✅ `String.char_at` out-of-bounds traps.
- ✅ Oversized shift (count ≥ bit width) traps.
  All un-ignored; `tests/crash_safety.rs` 37/37 green.

---

## 3. Remaining work (execution order)

### Tier A — Soundness (correctness first)

#### A1. task 2-30 — Crash-safety soundness bugs (5) — ✅ DONE (v42)
All 5 fixed and un-ignored; see §2.7.

#### A2. task 2-14 — Adversarial memory-model soundness fuzzer
Generate random ownership-stressing programs (nested `take`, field moves in loops,
borrows through closures/generators, container-read aliasing); run under ASan/TSan
for leaks/UAF/double-free. Protects Perceus + escape + tombstones.

#### A3. task 2-13 — Property tests for numeric coercion
Random literal/variable × int/float/width combos; compile at `-O0` and `-O3`;
assert runtime result == reference. The coercion miscompilation should have been
caught here.

### Tier B — Concurrency hardening

#### B1. task 2-6 — Structured concurrency scopes & lifetimes
Implement the `together` scope from `docs/structured-concurrency.md`: scoped
`spawn`, join-on-exit, child tasks cannot outlive parent scope.

#### B2. task 2-7 — stop-and-drain channel/scope shutdown
First-class close-mailbox-and-await-drain. Blocked-by intent: 2-6.

#### B3. task 2-9 — Actor join primitive
Make actor completion observable without `usleep`. Blocked-by intent: 2-6/2-7.

#### B4. task 2-10 — Cooperative preemption / yield injection
Prevent a non-yielding handler from wedging a worker / hanging `pthread_join`.
At minimum a watchdog warning.

### Tier C — Safe/unsafe boundary

#### C1. task 2-11 — Safe/unsafe boundary spec for raw pointers
Specify `%`/`@` raw-pointer interaction with ownership tiers with the rigor of
`docs/access-semantics.md`. Make unsafe ops visibly greppable.

#### C2. task 2-12 — Enforce safe/unsafe boundary (⊘ blocked by 2-11).

#### C3. task 2-22 — Raw-pointer & unsafe documentation (⊘ blocked by 2-11).

### Tier D — Store query language + crash tests

#### D1. task 2-16 — Expand store query language
Multi-row `where` results, `order by`, `limit`, projection, `sum`/`min`/`max`/`avg`.

#### D2. task 2-17 — Store crash-consistency test matrix
Torn writes, power-loss simulation, WAL replay (mirror channel suite).

### Tier E — Store improvements (store-improvement.md, task 2-31)

Remaining items (2-31-1, 2-31-2 = done):
- ◻ **2-31-3** Schema fingerprint in store header + migration enforcement.
- ◻ **2-31-4** Typed result sets for history/search/nearest/graph/distinct.
- ◻ **2-31-5** Relation traversal in queries + result rows (+`@cascade`).
- ◻ **2-31-6** Generalize `@kv` to schema-driven key/value types + optional get.
- ◻ **2-31-7** group-by clause + aggregates in query blocks.
- ◻ **2-31-8** Richer `where` filters: grouping, `in`, ranges, string operators.
- ◻ **2-31-9** Persistent secondary indexes (`.idx`, WAL-replayed).
- ◻ **2-31-10** `compact` statement / `@compact` policy.
- ◻ **2-31-11** Per-store durability decorators `@durable`/`@relaxed`/`@volatile`.
- ◻ **2-31-12** Declarative decorator table shared by parser/typer/docs (low).
- ⊘ **2-31-13** Write `docs/stores.md` (blocked by 2-31-2..12).

### Tier F — Diagnostics, tooling, ecosystem

- ◻ **2-15** Improve generic instantiation diagnostics (primary span at call site).
- ◻ **2-18** `jinn fmt` code formatter.
- ◻ **2-19** Package manager + manifest spec.
- ◻ **2-20** Adopt real version numbers (0.1.0 alpha, semver intent).
- ◻ **2-21** Sigil/operator reference documentation.

### Tier G — P2 (post-1.0, task 3, blocked by task 2)
Disambiguate sigil overloading; document `comptime`; codegen consolidation pass;
verifier coverage for `RuntimeOp` resource sets; configurable worker cap;
LSP depth assessment; cross-language benchmark publication; porting guides.

---

## 4. Execution policy

Work Tier A first (soundness), then B (concurrency), then in parallel D/E (stores)
and F (tooling). C is a spec-then-implement chain. Each fix lands with: pinned
conformance test(s), zero build warnings, full `cargo test` green, no source
comments. Update this file and the task tree as items close.
