# Access Semantics Sprint — Remediation Plan

Status: living document. Tracks completion of the access-semantics sprint
([docs/access-semantics-sprint.md](access-semantics-sprint.md)) after the
audit revealed Phase 2 was largely skipped and Phases 3-5 have residual gaps.

Baseline at plan creation: **917/917 bulk tests passing**.

## Audit summary (per-phase)

| Phase | Status | Notes |
| ----- | ------ | ----- |
| P1 surface & HIR plumbing                 | ✅ complete   | `LayoutAttrs` not renamed to `TyAttrs` (cosmetic). |
| P2 escape analysis & tiered lowering      | ⚠️ ~30%      | Ownership enum exists; no `src/escape/`, no Tier annotation, no T1 borrow codegen, no distinct T2/T3 lowering, no closure capture promotion, no acceptance tests. |
| P3 `@resource` / `@atomic` semantics      | ⚠️ ~70%      | Predicates + scope-exit drop work; stdlib annotations partial; no `*drop` user methods on stdlib resources; `@weakable` not lowered. |
| P4 field auto-copy + Perceus partial-move | ⚠️ ~60%      | `FieldTombstone` MIR instruction works; §5.1 auto-copy-on-escape not implemented (depends on P2). |
| P5 stores & smart rows                    | ⚠️ ~75%      | Snapshot, write-through, Row-is-resource done; no `Row.update`, no `Query.snapshot`, no multi-statement transaction grouping. |
| P6 cleanup                                | ⚠️ ~30%      | Annotated `is_aliased_read_of_heap` with TODO; surface `Rc` dead arm removed; rest gated on P2. |

## Why P2 was skipped

The Ownership enum + parser plumbing landed in P1, giving the appearance of
tier support. The actual analysis pass (escape walk) and the distinct codegen
(T1 raw borrow vs T2 Rc<Cell> vs T3 Arc<Mutex>) were never written. Without
escape analysis, `ref` and `copy` produce nearly identical IR — the only
difference is whether the slot is dropped at scope exit; the underlying clone
is still emitted by `vec_get_idx` and friends. The `is_aliased_read_of_heap`
heuristic in `src/typer/stmt/dispatch.rs` is the safety net that prevents
double-free on container reads today.

## Remediation sprints

### R1 — Finish P3/P4/P5 (no new analysis required)

**R1.1 — P3 completion** ✅
- [x] `@atomic`/`@resource` for built-in cross-thread / coroutine types are
      handled in the predicate (correct architecture until R3.4 promotes
      them to first-class HIR types).
- [x] Added idempotent `*shut`/`*close` + auto `*drop` on stdlib `@resource`
      types: `File`, `TcpListener`, `TcpStream`, `UdpSocket`, `TlsStream`,
      `TlsListener`, `Db`, `MmapRegion`. Each releases its OS handle at
      scope exit and is safe to call after an explicit shut.
- [x] Fixed latent `__builtin_FileExists` stdlib bug exposed by R1.1
      tests: `io.file_exists` now calls `extern.access` directly (the
      builtin's MIR-codegen path was never wired).
- [x] Tests: `file_drop_auto_flushes_writes`,
      `file_drop_idempotent_after_explicit_shut` (in addition to existing
      `resource_drop_runs_at_scope_exit`,
      `resource_cross_thread_channel_rejected`,
      `resource_cross_thread_actor_rejected`).

Baseline now: **919/919 bulk tests.**

**R1.2 — P5 completion** ✅ (with deferred items)
- [x] Audited `apps/bank_ledger` for current API — uses plain `Vec of Account`,
      no `store`/`Row` usage at all. Builds and runs cleanly. No code changes
      required for this app.
- [x] **Discovered + fixed latent codegen bug**: Jinn has two codegen
      pipelines — MIR-codegen (used by `*main` and most functions) and
      HIR-direct (used by coroutine bodies, actor handlers; see
      `src/codegen/coroutines.rs:225` → `compile_coroutine_stmt` →
      `compile_stmt`). The HIR-direct `compile_field` and
      `compile_lvalue_ptr` in `src/codegen/expr/access.rs` did not handle
      `Type::Row(_)` and errored with "field access on non-struct: Row<T>"
      when a row's field was read or written inside a coroutine body. Fixed
      by mapping `Type::Row(name)` → `Symbol::intern("__store_{name}")`
      and treating it as a struct (Row's LLVM layout *is* the
      `__store_{T}` record struct, per `src/codegen/types.rs:85`). Both
      `Type::Row` and `Type::Ptr(Type::Row)` now lower correctly in both
      pipelines.
- [x] Test: `store_row_field_access_in_coroutine_body` (Row created and
      mutated entirely inside a coroutine body — exercises the HIR-direct
      Row field paths).
- [ ] **Deferred** — `Row.update(|it| …)` batching helper. API addition;
      orthogonal to safety/perf core. The batch-write path already exists
      via `*store / where x / set f is v`.
- [ ] **Deferred** — `Query.snapshot()` returning `Vec of` value snapshots.
      API addition; the existing `*store / where x` already returns
      Row<T> by value.
- [ ] **Deferred** — adjacent-stmt coalescing
      (`row.a is …; row.b is …` → single StoreSet). Pure optimization;
      individual writes are already write-through and ACID-safe.
- [ ] **Deferred** — `store_row_write_through_two_coroutines`. Blocked on
      separate work: Jinn coroutines do not yet capture outer-scope
      variables (`compile_coroutine_create` replaces `self.vars` with
      an empty map at `src/codegen/coroutines.rs:82`). Cross-coroutine
      Row sharing requires implementing coroutine capture first, which
      is out of scope for the access-semantics sprint.

Baseline now: **920/920 bulk tests.**

**R1.3 — P4 placeholder tests** ✅
- [x] Added `tests/programs/field_auto_copy.jn` (struct field whose value
      escapes via function return; both reads succeed, no move) and
      wired `access_field_auto_copy_escape` in `tests/integration.rs`.
- [x] Added `tests/programs/field_short_lived_borrow.jn` (struct field
      read inside an `if` condition; field remains readable after) and
      wired `access_field_short_lived_borrow` in `tests/integration.rs`.
- [ ] **Deferred (R3.3)** — IR-inspection assertions: "exactly one
      clone" for the escape path, "zero clones / raw load" for the
      short-lived path. Both tests currently pin behavior; the
      optimization assertion lands with the T1 raw-pointer borrow
      codegen.

### R2 — `@weakable` lowering (self-contained)

- [ ] Weak-count slot when type carries `@weakable @atomic`.
- [ ] Parser: `weak ref T` → `Type::Weak(T)`.
- [ ] `weak_upgrade()` builtin returns Option<&T>.
- [ ] Reject `@weakable` on non-`@atomic` types (already done at parse).
- [ ] Test: `weak_upgrade_after_drop_returns_none`.

### R3 — Escape analysis + tiered codegen (the big one)

**R3.1 — `src/escape/mod.rs` analysis module**
- [ ] `Tier { T1, T2, T3 }`, `EscapeInfo` map per fn.
- [ ] Forward HIR walk producing initial tier per Bind.
- [ ] Promotion rules: return / struct store / container store / closure
      capture / channel send / spawn capture / `@atomic`-source.
- [ ] Inspection tests.

**R3.2 — Wire EscapeInfo into typer**
- [ ] Replace `is_aliased_read_of_heap` with `EscapeInfo` lookup.
- [ ] `ownership_with_mod` consults EscapeInfo when modifier is absent.
- [ ] Soundness gate: existing 917+ tests must still pass.

**R3.3 — T1 raw-pointer borrow codegen**
- [ ] `Ownership::Borrowed` slot = raw pointer (no clone).
- [ ] Container readers (`vec.get`, `map.get`, `set.peek`, `deque.front`,
      `pq.peek_min`/`max`) skip `clone_value` when caller binding is T1.
- [ ] `for x in xs` per-iteration borrow load.
- [ ] IR-snapshot tests: no `clone_value` for canonical short-lived borrow.
- [ ] Benchmark regression gate: struct_ops, spectral_norm, sim_for,
      tight_loop, store_perf, vec_grow.

**R3.4 — T2 / T3 codegen**
- [ ] New HIR `Type::RcCell(inner)`, `Type::Arc(inner)`, `Type::Mutex(inner)`.
- [ ] `runtime/cell.c`, `runtime/atomic_rc.c`, `runtime/sync.c` as distinct
      units (extracted from current inline implementations).
- [ ] Promotion lowering: T2/T3 bindings rewrite their type to
      Rc<Cell<T>> / Arc<Mutex<T>> with matching alloc/clone/drop helpers.
- [ ] Test: cross-thread channel-send borrow promoted to Arc.

### R4 — P6 finalization

- [ ] Delete `is_aliased_read_of_heap` and all call sites.
- [ ] Remove the P6.1 TODO from `src/typer/stmt/dispatch.rs`.
- [ ] Update `jinn.md`, `JINN.md`, `docs/access-semantics.md` to reflect the
      final landed surface.
- [ ] Run full bulk + all benchmark suites, record numbers.

## Acceptance criteria (whole sprint)

1. Bulk test suite 100% green.
2. The 6 benchmarks in P2 §3.7 do not regress vs. pre-sprint baseline
   (`benchmarks/results_pre_sim.json`).
3. The `is_aliased_read_of_heap` heuristic no longer exists.
4. All sprint spec §10 acceptance tests present.
5. `@resource` types reliably release OS resources at scope exit without an
   explicit `.close()`.

## Ordering rationale

R1 lands first because it consolidates context from this session and produces
visible user-facing wins (`*drop` for File, transactional store writes) with
zero risk to the type system. R2 is a self-contained add. R3 is the heavy
lift — staged so analysis can land before codegen so we never have a half-on
state. R4 is purely deletion + docs.
