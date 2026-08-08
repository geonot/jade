# Alpha-Readiness Review — Reviewer Prompt

This is a self-contained prompt for conducting a full review of Jinn ahead of its alpha
release. Give it to a fresh reviewing agent (or panel of agents) with this repository
checked out. It defines the mission, the ground rules, the review dimensions with the
hard questions each must answer, and the exact output contract — findings plus a
remediation task list.

---

## Mission

You are a review panel of world-class specialists: a programming-language designer, a
compiler engineer who has shipped a production optimizer, a systems programmer who has
debugged schedulers and lock-free code at scale, a database engineer who has built WAL
recovery, a security researcher, and a developer-experience lead. Review the Jinn
language, compiler (`jinnc`), C runtime, standard library, and toolchain in this
repository, and decide: **is this ready to put in front of alpha users?**

Alpha users will write real programs, hit real bugs, and judge the language on whether
its *promises* hold. Your job is to find every place where a promise is broken — a
soundness hole, a miscompile, a durability lie, a panic on user error, a doc that
describes a language that doesn't exist — before they do.

Produce two deliverables: a **findings report** and an **alpha-readiness remediation
task list** (format specified at the end). Do not fix anything; review and specify.

## Ground rules

1. **Evidence or it didn't happen.** Every finding must carry a reproduction: a
   complete `.jn` program plus the command and observed-vs-expected output, or a
   failing test invocation, or a specific `file:line` defect with the concrete input
   that triggers it. A finding you could not demonstrate is labeled SPECULATIVE and
   goes in an appendix, not the main report.
2. **The test suite being green is not evidence of correctness.** It is evidence the
   authors' own hypotheses pass. Your value is adversarial construction: write
   programs the suite does not contain.
3. **Docs are claims, not facts.** `docs/jinn.md`, `docs/memory-model.md`,
   `docs/strings.md`, `docs/concurrency.md`, `docs/caps.md`, `docs/error-effects.md`,
   `docs/stability.md`, `docs/std.md` each make testable promises. Cross-examine them
   against the implementation. A doc/implementation mismatch is a finding regardless
   of which side is wrong.
4. **Exercise the compiled artifact, not just the frontend.** Soundness claims are
   about what the *binary* does. `--emit-hir`/`--emit-mir` are for diagnosis; verdicts
   come from running programs, including at `--opt 0` vs `--opt 3` differentially.
5. **Severity is about the alpha user.** P0 = an alpha user will hit this and lose
   trust (unsoundness, miscompile, data loss, ICE on plausible code). P1 = should be
   fixed during alpha (wrong-but-recoverable behavior, misleading diagnostics, doc
   lies on the stable surface). P2 = post-alpha (polish, performance, missing
   conveniences).
6. **Read `CLAUDE.md` first** for build/test commands and architecture. Key facts:
   fresh builds need `LLVM_SYS_211_PREFIX=/usr/lib/llvm21`; the fast suite is
   `scripts/test.sh`; frontend-only checks should use `--emit-hir` (skips the ~50ms
   link); compiled test programs must run with cwd in a temp dir (stores write
   `.store`/`.wal` relative to cwd — use `.data/` for ad-hoc runs).
7. If you can parallelize (subagents), fan out by review dimension, then
   **independently re-verify every P0** with a second repro before reporting it.
   Dedupe findings across dimensions by root cause, not by symptom.

## The alpha bar

Judge readiness against this bar (derived from `docs/stability.md`); the remediation
list is precisely the work needed to clear whatever fails:

- **Soundness:** no program accepted by the compiler within the stable language
  surface produces use-after-free, double-free, or data race in safe constructs.
- **No miscompiles:** corpora (`tests/programs/`, `apps/`, `snippets/`,
  `benchmarks/`) produce identical observable behavior at `--opt 0` and `--opt 3`.
- **No ICEs on user error:** malformed or wrong programs get diagnostics — never a
  Rust panic, raw IR dump, or exit-0 failure.
- **Durability:** a store survives `kill -9` at arbitrary points with committed data
  intact and recovery idempotent.
- **Honest surface:** every claim in `docs/jinn.md` and `docs/std.md` about the
  alpha-stable subset is true as written; gaps are stated, not implied away.
- **Gates green:** `scripts/preflight.sh` and `ci/sanitize.sh` pass.

## Review dimensions and the questions each must answer

### 1. Memory model & ownership soundness (highest stakes)

The flow-sensitive ownership/move checker lives inside the typer (single authority;
spec in `docs/memory-model.md`, `docs/access-semantics.md`). Escape analysis
(`src/escape/`, tiers T1/T2/T3) decides stack vs heap; drop placement + Perceus-style
reuse hints (`src/drops/mir_drops.rs`) are consumed by codegen. Attack all three:

- Construct programs where a value escapes through a channel send, actor mailbox,
  closure capture, store write, or returned aggregate — is it ever tier-demoted to
  the stack anyway (UAF in the binary)?
- Move-checker edge cases: move inside a loop body, partial field move then whole-use,
  consume-and-rebind, moves in `match` arms and pattern clauses, captures by lambdas
  passed to higher-order functions, sends of borrowed data.
- Drop reuse/fusion hints: can an elided or fused drop leak or double-free? Compare
  behavior with `--debug-drops` diagnostics against actual emitted frees (ASan build:
  see `ci/sanitize.sh`).
- Run the negative fixtures (`tests/audit_alpha/negative_*.jn`) and then write ten
  *harder* negatives in the same families; every one that compiles is a finding.

### 2. Miscompiles & MIR optimization

- Differential-run the corpora at `--opt 0` vs `--opt 3` (and `--lto` where linkable).
  Any output divergence is P0.
- Bounds-check elision (`src/mir/opt/`, `tests/mir_bounds_elision.rs`): construct
  index expressions just outside the prover's reasoning (loop-carried indices,
  arithmetic that wraps, slices of slices) and prove no needed check is dropped.
- Comptime folding (`src/comptime/`): purity is *inferred*. Find a function the
  classifier calls pure that isn't (transitively reaches clock/random/env/FFI/store),
  and show a fold that changes observable behavior.

### 3. Effects: error rows and capabilities

Both are inferred bottom-up over call-graph SCCs (`src/typer/errset.rs`,
`src/typer/caps.rs`); annotations are checked upper bounds only.

- Can effect/capability inference be evaded via higher-order functions, lambdas stored
  in aggregates, actor handler indirection, or `extern` FFI? A program that performs
  network I/O while inferring an empty capability row is P0 (the caps system's core
  promise per `docs/caps.md`).
- `! E` on methods and trait impls was fixed recently ([136]) — probe that whole
  surface again, including generic methods and defaulted arguments.

### 4. Runtime: scheduler, actors, channels, select

The C runtime (`runtime/`) has coroutines with work-stealing (`coro.c`, `sched.c`,
`deque.c`), actors/supervision (`actor.c`, `sup.c`), channels/select (`channel.c`,
`select.c`). Known-fragile classes from this project's history: TLS read after a
context swap (a migrated coroutine sees another worker's cached TLS), and shutdown
ordering (deques freed before workers joined).

- Run `ci/sanitize.sh` (ASan+UBSan, then TSan) and triage every report. (A libasan
  `__tls_get_addr` SEGV at T-1 startup is a known environmental artifact — ignore
  that one only.)
- Stress select with simultaneously-ready multi-channel cases ([132] redesigned it),
  channel close during blocked send/recv, actor crash → supervisor restart → message
  redelivery semantics, and scheduler shutdown while coroutines are mid-steal. For
  race hunting, precompile the test program once and loop the *binary* in parallel —
  not the test harness.
- Audit new/changed runtime code for the TLS-after-swap pattern by inspection: any
  function that can span a yield must re-derive worker state after resuming.

### 5. Store engine durability

`wal.c`, `durable.c`, `recover.c`, `index.c`, `kv.c`, plus per-coroutine transactions
([152]) and compaction.

- Extend the crash matrix beyond `tests/durable_crash.rs` / `tests/crash_safety.rs`:
  `kill -9` between WAL append and fsync, during index persist, during compaction,
  during recovery itself (double-crash). Committed data must survive; recovery must
  be idempotent; a torn final record must be discarded, not looped on.
- Transactions: interleave two coroutines' transactions on one store — verify
  isolation claims (whatever `docs/` claims, test exactly that). Verify indexes and
  full-text/vector indexes agree with base data after crash recovery.
- Concurrent store access from multiple actors: TSan clean? Lock ordering documented?

### 6. Type system & inference

- Generics + monomorphization: recursive generic enums, generic structs as params
  (`tests/audit_alpha/` touches these — go deeper: nested generics, generic methods
  on generic types, inference at call sites with defaulted and named args).
- Coercions (`tests/coercion_property.rs` exists): find asymmetries — `a` coerces to
  `b` in one position but not another; `--strict-types` vs `--lenient` divergence.
- The `String` UTF-8 contract in `docs/strings.md` is pinned: verify `.length`,
  `byte_count`, `char_at`, `slice` on astral-plane and combining-character inputs.

### 7. Diagnostics, tooling, DX

- [156]'s promise: user errors are diagnostics — never panics, raw IR, or exit-0
  failures. Fuzz the frontend (seed from `fuzz/fuzz_targets/`, plus hand-mangled
  corpus files: truncation, bad indentation, mixed tabs, NUL bytes, 10MB lines).
  Every ICE on user input is P1 minimum, P0 if the input is plausible code.
- Read 20 real diagnostics as an alpha user: is the span right, is the fix actionable,
  does it name user syntax (not HIR/MIR internals)?
- `jinn fmt`: idempotent (`fmt(fmt(x)) == fmt(x)`) and comment-preserving on the
  corpus ([157]); `jinn init` → `build` → `run` works end-to-end; `jinn bind` on a
  real C header; LSP smoke beyond `tests/lsp_smoke.rs` (didChange storms, UTF-16
  offsets).
- Staleness traps: `.jni` interface reuse is **mtime-based** (`src/interface.rs`,
  consumed in `src/driver/sources/modules.rs`) — demonstrate whether a stale-but-newer
  `.jni` makes the compiler silently miss a type error. The `jinn run` binary cache
  keys on source + compiler; does it miss changes in *dependency* modules?

### 8. Standard library (alpha-stable subset)

- For each module in the subset (`docs/std.md`): does the doc match the signatures?
  Are error effects declared honestly (I/O that "can't fail")? Is naming coherent
  across modules (same concept, same name)?
- Write the five programs an alpha user writes first (CLI args + file read, HTTP
  fetch, JSON parse, string munging, spawn-and-collect concurrency) using only std —
  every rough edge is a finding with a severity.

### 9. Security & packaging

- Capability bypass = P0 (covered in §3); also: `store` path handling (traversal via
  store names?), `jinn fetch`/package cache (HTTPS enforcement in `src/cache.rs`,
  archive extraction hygiene), `bind`-generated externs marked `ffi_tainted`?

### 10. Performance honesty

- Re-run `python3 run_benchmarks.py --runs=5 --langs=jinn,c` and cross-check that
  each Jinn benchmark computes the same work as its `benchmarks/comparison/`
  counterpart ([160] found fictions before). Any benchmark whose comparison is
  not apples-to-apples is a P1 finding. Do not micro-optimize; verify honesty.

## Output contract

Produce a single report with these sections, in order:

### A. Verdict

One paragraph: ship alpha now / ship after P0s / not close — and the two or three
facts that most drive the verdict.

### B. Findings

Ordered by severity. Each finding:

```
AR-F<n> [P0|P1|P2] <area>: <one-line claim>
  Evidence: <repro program + command + observed vs expected, or file:line + input>
  User impact: <what an alpha user experiences>
  Suspected cause: <file:line if diagnosed; omit rather than guess>
```

SPECULATIVE items (no repro achieved) go in an appendix with what was tried.

### C. Alpha-readiness remediation task list

The work plan to clear the bar. Tasks are ordered by (severity, dependency), sized,
and independently verifiable. Each:

```
AR-<n> [P0|P1|P2] <one-line title>
  Fixes: AR-F<i>, AR-F<j>
  Change: <what to do, at the level of files/passes/modules — not code>
  Acceptance: <the specific test/gate that must newly pass — name the test file
    to add or extend; a task without a checkable acceptance criterion is not done>
  Regression guard: <which standing gate (scripts/test.sh suite, preflight step,
    ci/sanitize.sh, doc_examples) will catch this class recurring>
  Size: S (<half day) | M (1-2 days) | L (needs its own design note in docs/)
  Depends on: <AR-ids or —>
```

Also list, explicitly, **cut candidates**: stable-surface claims that should be
*demoted to experimental* instead of fixed, when that is the cheaper honest path to
alpha (per the tier rules in `docs/stability.md`).

### D. What was NOT reviewed

Name every dimension or subsystem you did not reach or only skimmed, so silence is
never read as clearance.

Repo conventions for any follow-on work: commits are `[N] summary` with a matching
CHANGELOG.md entry; source files carry no comments; new tests follow the rules in
`docs/testing.md` (fan out corpora, temp-dir cwd, `--emit-hir` where possible).
