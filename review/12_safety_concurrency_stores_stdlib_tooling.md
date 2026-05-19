# §13 Memory safety story (end-to-end)

This section integrates the findings of §7, §8, §11, and §12 into a
single end-to-end view: **what guarantees does Jinn make about memory
safety, and how does each one fail today?**

## 13.1 What the language promises

The project documentation talks about:
- Move semantics with `take`.
- Borrow checking.
- Perceus reference counting for non-trivials (so no manual `free`).
- Stackful coroutines with guard pages.
- Persistent stores with WAL durability.

A reasonable reader would infer **memory safety modulo `extern`** —
similar to Rust without explicit lifetimes, or Swift without
`Unmanaged`.

## 13.2 What the implementation delivers today

| Promise | Status | Evidence |
| ------- | ------ | -------- |
| No use-after-free for owned values | **Unverified** | F-OWN-1: `take` does not parse in call args; probe v1 use-after-move did not fire a diagnostic |
| No double-free | **Likely OK** | Perceus + drop fusion looks correct; not stress-tested |
| Bounds-checked vec access | **Partial** | F-CG-2: some access paths SIGSEGV; the user-visible "trap" is `llvm.trap` not a diagnostic |
| Division by zero is defined | **NO** | F-CG-1: prints uninitialized stack values |
| Stack overflow is reported | **NO** | F-RT-2: 64 KB stacks + guard page → SIGSEGV with no message |
| No unaligned access | **Unverified** | Codegen uses default LLVM alignment; haven't probed |
| Integer overflow is defined | **Wraps** | p02_overflow: signed overflow wraps cleanly; consistent with Rust release-mode `wrapping_*` semantics, but not documented |
| FFI is bounded | **No** | No `Sendable` / `transferable` analysis exists; channel of arbitrary types should reject non-Send |

## 13.3 Concrete pwn-vectors a 1-hour alpha user can hit

1. `10 / x` where `x` is zero → silent garbage value, no trap.
2. `v[i]` where `i >= v.len()` → SIGSEGV.
3. `v[-1]` → SIGILL (different signal from positive OOB — UB).
4. Recursion depth > 4–6 K frames in an actor → silent SIGSEGV.
5. `map(v, $ * 2)` → ICE or SIGFPE.
6. Generator function → invalid IR rejected by LLVM verifier.

Each of these is a one-line program. None of them is a malicious
input — they are everyday code. **A memory-safe language cannot ship
with any of these. Fixing them is the §24 alpha-blocker list.**

## 13.4 Where the safety story is strong

- Trivial drops are elided correctly (Perceus drop-elision pass).
- The 16 sample apps + 33 benchmarks run to completion without
  observable corruption — for the patterns those programs exercise,
  the language is doing the right things.
- The runtime is at least disciplined enough that long-running probe
  `p32_alloc_churn.jn` (100 K allocations) ran in 14 ms with no leak.
- Drop fusion + reuse pairing means common loops avoid per-iteration
  free/malloc.

The pattern is clear: **the common path works; the safety floor at
the edges is missing.** That's the alpha gap.

---

# §14 Concurrency — actors, channels, coroutines, scheduler

**Compiler:** `src/codegen/{actors.rs, channels.rs, coroutines.rs}` +
mir lowering. **Runtime:** `runtime/{actor.c, channel.c, sched.c,
select.c, sup.c, coro.c, deque.c, timer.c, context_*.S}`.

## 14.1 Model

- Stackful coroutines on a 64 KB stack with 4 KB guard.
- M:N scheduler with a per-worker work-stealing deque
  (`runtime/deque.c`).
- Channels are typed (`channel of T (cap)`), bounded, with send/recv
  wait queues and spinlock protection.
- Actors: `actor T … @handler` — message-passing with a mailbox per
  actor; each handler runs on a coroutine inside the M:N pool.
  `spawn ActorType` returns an `ActorRef`.
- `select` over multiple channel cases is supported, including
  `default` and timers.
- Supervisors (`supervisor` keyword) implement `one_for_one`,
  `one_for_all`, `rest_for_one` (per memory note).

## 14.2 What I verified

- **`p14_actor.jn`** — actor `Counter` with `@inc` and `@show`, 3
  inc's then show → printed `3`. **Works.**
- **`apps/chat_room/`** — actor-based, prints `history 200 spam …`.
  **Works.**
- **`apps/raft_cluster/`** — 5-node raft demo, prints
  `raft.boot 5 nodes node.id 1`. **Works** (at least gets through
  boot).
- **`apps/order_book/`** — prints best bid/ask. Likely actor-based
  given the structure. **Works.**

## 14.3 Findings

### F-CONC-1 (P0): `.send()` cannot be called as a method

Probe `p15a_channel_method.jn`:
```
ch is channel of i64 (4)
ch.send(1)
```
**Parse error**: `line 4:8: expected identifier, got send`. The `send`
keyword shadows method names. Users must use the keyword-form
`send ch, 1` or `<-` operator (whichever Jinn supports — both have
appeared in source). **This is F-LEX-2 in action.** The fix is
contextual keywords (don't promote `send` after `.`).

### F-CONC-2 (P1): No documentation of cancellation / shutdown semantics

What happens to in-flight actor messages when the program exits?
What about pending channel sends? Per memory note
`jade_microkernel_shutdown.md`, this was wrestled with in the
microkernel app. The runtime side likely has a `sched_run` exit
condition; the user-side semantics are not documented.

### F-CONC-3 (P1): Spinlocks under contention not stress-tested

See F-RT-3.

### F-CONC-4 (P2): No `tokio_console`-equivalent

For debugging concurrent programs, an observability hook
(`jinnc inspect <pid>` showing live coroutines / channels) would be a
killer feature. Not blocking; track for beta.

## 14.4 Verdict

**Mostly alpha-ready in behaviour**, but F-CONC-1 is a parse-time
barrier to writing idiomatic code. Once contextual keywords land,
actors/channels look strong: real apps run, the model is coherent,
and the runtime is the right shape.

---

# §15 Persistent stores & WAL

**Compiler:** `src/codegen/{stores.rs, store_ops.rs, store_filter.rs,
mir_codegen/store_ext/*}`. **Runtime:** `runtime/{kv.c, vec.c, bloom.c,
column.c, fts.c, index.c, wal.c, sqlite.c, migrate.c}`.

## 15.1 Model

A `store Name` declaration emits a typed persistent collection backed
by a WAL on disk. Operations:
- `insert Name v1, v2, …` (positional)
- `count Name`
- `query Name where …`
- `set Name field = value where …`
- `delete Name where …`
- `transaction … end`
- `migration` declarations for schema evolution

Index types include hash, B-tree, bloom filter, full-text. The
`apps/bank_ledger/`, `apps/kv_store/`, `apps/inventory_mgr/`,
`apps/order_book/` apps all use stores and **all compile + run**.

## 15.2 What I verified

- **`p19_store.jn`** — declare `store users`, insert two rows,
  `count users` → printed `2`. **Works.**
- **`apps/kv_store/`** — runs, prints `size_now_10 102 size_now_50`.
  **Works.**
- **`apps/bank_ledger/`** — runs, prints `tx_count 5 volume`. **Works.**

The WAL has a dedicated test (`tests/wal_crash.rs`, 3 passing tests)
exercising crash recovery. That's a strong signal.

## 15.3 Findings

### F-STORE-1 (P1): WAL durability under non-clean shutdown

The wal_crash test passes — good. But the test count is small (3),
which doesn't span the matrix of concurrent writer + reader + crash.
For alpha that's tolerable; for production this needs property tests
(QuickCheck-style: random ops, random crashes, invariants checked
post-recovery).

### F-STORE-2 (P2): SQL-like `query Name where …` syntax not fully documented

The grammar and semantics of `query` (predicate language, operator
precedence, supported types, NULL handling) aren't documented in
`jinn.ebnf` or the docs I read. A user inferring it from examples is
likely to hit edge cases.

### F-STORE-3 (P2): No transaction isolation level discussion

`transaction … end` exists. What isolation level? Read committed?
Serializable? In a multi-threaded actor program with many actors
touching the same store this matters.

### F-STORE-4 (P3): `count` is a keyword (consistent with F-LEX-2)

Likely cannot define a user-level `count()` method on a custom type
without quoting. Same fix.

## 15.4 Verdict

**The most underrated strength of the project.** A built-in store +
WAL primitive is unusual and powerful. The implementation evidence is
that several real apps run on it. Findings here are about depth,
documentation, and stress testing — not soundness.

---

# §16 Standard library

**Files:** `libjn/` (3,097 LOC) + `std/` (11,856 LOC).

## 16.1 Two-tier layout

- `libjn/*.jn` — libc-shaped: `stdio.jn`, `string.jn`, `stdlib.jn`,
  `math.jn`, `stdint.jn`, `pthread.jn`, `signal.jn`, `errno.jn`,
  `fenv.jn`, `setjmp.jn`, `sys_socket.jn`, etc. One file per common
  libc header.
- `std/*.jn` — higher-level, idiomatic Jinn. Per directory listing
  in the workspace.

No documented contract distinguishes the two layers; no stability
guarantee.

## 16.2 Findings

### F-STD-1 (P1): No documented "what to import" guide

A user starting `*main` doesn't know whether to use `libjn/math.jn`
or `std/math.jn` (if both exist). Pick one as the user-facing tier
and document the other as low-level / internal.

### F-STD-2 (P2): String operations are partial (UTF-8 truncation)

Per F-LEX-3 — emoji output truncates. This means the string layer
(in `libjn/string.jn` likely) is byte-oriented. For an alpha aimed at
modern users this needs the `char` / `grapheme_cluster` distinction
addressed.

### F-STD-3 (P2): Math interop with f32/f64 mixed types

`p22_const.jn` worked (`area(2.0)` returned correct value) but
`benchmarks/nbody.jn` produced a warning:
> warning: benchmarks/nbody.jn:2:14: numeric type defaults to i64 (binary operands)

This is the type-default behavior leaking into numerics-heavy code.
Either the defaults should change for floating contexts, or the
warning should explain itself better.

### F-STD-4 (P3): 11.8 K LOC of `std/` lacks documentation reviewed for accuracy

Not assessable without a deeper read; flagging that the documentation
side has not been audited.

## 16.3 Verdict

**Functional for the apps + benchmarks that ship**, but the
*surface area* of the stdlib is large for a pre-alpha and the
*documentation* per module is thin. Recommend curating a focused
"alpha-supported" subset and marking the rest experimental.

---

# §17 Tooling: CLI, package mgr, LSP, fmt, tree-sitter, VSCode

**Files:** `src/driver/`, `src/lsp/` (in source tree), `vscode-jinn/`,
`tree-sitter-jinn/`.

## 17.1 CLI

`jinnc` and `jinn` (jinn is a thin wrapper around `main.rs`). Verified
subcommands: `init`, `fetch`, `update`, `build`, `package`, `publish`,
`run`, `test`, `check`, `fmt`, `bind`. This is a **complete CLI
surface** — package manager (`init/fetch/update/publish`), build,
run, test, format, type-check-without-codegen, and even a C-header→Jinn
binding generator (`bind`). That's broader than most pre-alpha
languages provide.

## 17.2 Findings

### F-TOOL-1 (P0): `jinnc` on a single source file accepts no-main programs

§5 F-PARSE-2.

### F-TOOL-2 (P1): `jinnc build` uses `project.jn` correctly; single-file mode does not

`jinnc build` in an `apps/*/` directory correctly reads the project
manifest and finds `entry is 'source/main.jn'`. Single-file
`jinnc PATH.jn` does not — that mode is "compile this file as a
freestanding program" and lacks the project-mode guards.

### F-TOOL-3 (P1): LSP `jinnc-lsp` ships but coverage unmeasured

The binary builds. I did not exercise it. For an alpha, LSP coverage
of: completion, hover, go-to-def, find-references, rename, format,
diagnostics — needs an explicit test matrix and probably a screenshot
or video. Not assessable here.

### F-TOOL-4 (P2): `vscode-jinn` extension status unverified

It ships in the tree. I did not install or exercise it.

### F-TOOL-5 (P2): `tree-sitter-jinn` grammar may drift from `jinn.ebnf`

Standard concern with two separate grammar definitions. The fix is
either generating tree-sitter from EBNF, or adding a CI check that
parses the same corpus with both and compares.

### F-TOOL-6 (P3): No formatter conformance test

`jinnc fmt` exists. Does `fmt(fmt(x)) == fmt(x)` (idempotence)? Does
`fmt(x)` preserve semantics? No conformance tests visible.

## 17.3 Verdict

**Tool surface is broad; depth varies.** The package manager and
build flow look solid (apps build and run). LSP and IDE side need
exercise and documented coverage.

---

# §18 Testing & CI posture

## 18.1 What exists

- 1,570 tests pass cleanly (§2). That's strong.
- `tests/programs/` — many `.jn` fixtures used by the bulk tests.
- `tests/wal_crash.rs` — 3 WAL crash-recovery tests, all pass.
- `tests/proptest_smoke.rs` — 3 proptest tests, all pass.
- `tests/mir_bounds_elision.rs` — 2 tests for the MIR bounds-elision
  opt, both pass.
- `tests/perceus_debug.rs` — 2 pass + 1 ignored.

## 18.2 What's missing

### F-TEST-1 (P0): No fuzzer

A language at this scope ships with at least:
- Lexer fuzzer (random byte sequences in, no panics expected).
- Parser fuzzer (random tokens, no panics; either Ok or a clean
  ParseError).
- Round-trip property: `parse(print(parse(SRC))) == parse(SRC)`.

None of these exist. A weekend of `cargo-fuzz` would surface the
ICEs I found by hand and many more.

### F-TEST-2 (P0): No ASan/TSan/UBSan run

§F-RT-1. CI must run the runtime under ASan + TSan before alpha
tag.

### F-TEST-3 (P1): No coverage gating

`cargo tarpaulin` or `llvm-cov` would tell us where the dead spots
are. The `panic!` / `expect` hotspots I found are likely *not*
covered.

### F-TEST-4 (P1): Test suite is heavily round-trip-style

Most of the 913 `bulk_tests` and 423 `integration` tests appear to
be "parse this, expect this AST" or "compile + run this, expect this
output". That's necessary but skewed; behavioural tests for
ownership/borrow, for diagnostics quality, for "this should be a
compile error" are underrepresented relative to the surface.

### F-TEST-5 (P2): Previously-deleted flaky tests should be replaced

User reported deleting `dyn_trait_basic`, `dyn_trait_multiple_types`,
`pool_create_alloc_free`, `pool_multiple_allocs`. Whatever feature
those tests covered (dynamic dispatch and pool allocator) presumably
still ships; the tests should be re-authored against the current
working model, not left as a gap.

### F-TEST-6 (P3): 159 compiler warnings on release build

§2 again. `cargo clippy -- -D warnings` in CI is half a day of work
and provides ongoing hygiene.

## 18.3 Verdict

**Test breadth is excellent; depth and adversarial coverage are
not.** Fuzzing + sanitizer runs are the single highest-ROI testing
investment before alpha.
