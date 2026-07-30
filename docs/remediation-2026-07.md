# Remediation Plan — JINN_REVIEW_2026_07

Derived from [`JINN_REVIEW_2026_07.md`](JINN_REVIEW_2026_07.md) §11, items 1–14, 16, 17.
Mirrors the task tree at `.ryu/tasks/8*.task`. Items 15 (sigil reduction) and 18
(in-code TODO markers) are out of scope for this plan by direction.

Legend: ◻ remaining · ✅ done · ⊘ blocked.

---

## 0. How decisions were made

Several review items are not bug reports — they are open design questions that were
left open while implementation continued around them. Those are resolved here
against Jinn's stated goals, in this priority order when they conflict:

1. **Safety of Rust** — no use-after-free, no double-free, no data races. A program
   that would corrupt memory must not compile.
2. **Performance of C** — no GC, no reference-count traffic on the hot path, no
   implicit O(n) copies.
3. **Ease of Python** — no lifetime annotations, no `&`/`&mut`, no explicit
   `clone()`. The compiler infers; the developer writes values.
4. **Silent wrongness is never acceptable** — the store surface already states
   this ("silent data loss is never an option", `jinn.md`). Generalize it: prefer a
   compile error, then a loud trap, then never a plausible-looking wrong answer.

Where a decision forecloses an option, the rejected option and the reason are
recorded so the choice can be revisited on evidence rather than re-litigated.

### D1 — Aggregate memory model (review item 2, blocks the whole safety tier)

**Decision: move-on-assign with compiler-inferred borrows, for all heap aggregates
(`Vec`, `Map`, and aggregate-containing structs), matching what `jinn.md` already
promises.** `b is a` moves; `a` is then unusable until reassigned. Reads borrow
without copying. No annotations, no sigils — the analysis is inferred, exactly as
the existing escape-tier machinery already attempts.

Rejected alternatives:

- *Deep value semantics (copy on assign).* Safe and simple, but turns `b is a` into
  a silent O(n) allocation and memcpy. That is the performance trap a C programmer
  will not accept, and it violates goal 2. It is also not what Python does.
- *Keep today's shared mutable aliasing.* This is the status quo (§3.3) and it is
  what makes §3.1 and §3.2 corrupt memory. Retaining it means abandoning goal 1
  outright, and it cannot be made safe without a GC or refcounts (goal 2).

Consequences to accept deliberately:

- The §3.3 program becomes a **compile error** ("use of moved value `a`"). This is a
  breaking change to current observable behavior. It is the correct one: today that
  program's behavior is undocumented and the drop machinery disagrees with it.
- Shared mutable state across tasks is expressed by **message passing** (actors and
  channels), which is already the first-class concurrency story. The §3.2 race
  becomes a compile error rather than a runtime allocator abort.
- Strings already behave this way (24-byte SSO values, deep clone). D1 makes
  aggregates consistent with strings rather than the reverse.

### D2 — Inference for unannotated container parameters (review item 10)

**Decision: substitute call-site generic arguments into implicit-generic bodies at
instantiation. Do not add required annotations, and do not add a new trait-bound
obligation for this case.**

The review's framing ("no principal type for a method call") turned out to be too
pessimistic once probed. Measured behavior:

| Program | Result |
| --- | --- |
| `*firstof(v)` → `v.get(0)` (tail) | works — return type flows from the call site |
| `*sumall(v)` → `t is t + v.get(i)` | works — `t is 0` pins the element type locally |
| `*peek(v)` → `t is v.get(0)`; `log(t)` | **fails** — nothing local pins it |
| `*peek(v as Vec of i64)` → same body | works |

So the element type of a container parameter is *never* propagated from the call
site; it resolves only when a local constraint happens to pin it. The root cause is
incomplete generic-argument plumbing, not a missing constraint language:
`monomorphize_fn_inner` does not substitute the call-site's concrete type arguments
(the `i64` in `Vec of i64`) into the body's type environment, and
`resolve_core`/`occurs_in` do not recurse into `Struct(_, args)`
(`src/typer/unify/resolve.rs:26-82`), so parameterized types can carry unresolved
variables past resolution. `Struct(n, _) ↔ Enum(n)` unifying with generic arguments
explicitly skipped (`src/typer/unify/mod.rs:555-564`) is the same gap.

This is the ethos-aligned fix: it makes `*bsort(v)` work with zero annotations,
which is what "complete type inference" has to mean to be worth claiming. Errors
surfacing at instantiation rather than definition is the C++-template tradeoff, and
it is acceptable *given* task 8-17 (diagnostics must name both the call site and the
definition). Bounded polymorphism already shipped (task 1-4) and remains the right
tool for definition-site checking of exported library functions — see 8-16 DoD.

### D3 — Store query results (review item 12)

**Decision: a query that can match nothing has type `Option of <Record>`.** Reading
a field of a miss must not be expressible. Today it silently yields a fabricated
zero row (`name=[] age=0`), which is the exact failure mode goal 4 forbids.

This costs no ceremony because the quaternary already handles it:
`users where name equals 'Alice' ? use($) ! not_found()`, and inside a fallible
function a bare use propagates. `first`/`get` follow the same rule. `count` and the
aggregates stay non-optional.

### D4 — Module resolution (review item 8)

**Decision: a compile sees exactly one entry file plus the transitive closure of its
explicit `use` declarations.** Directory-tree absorption and identifier-driven
implicit import are removed.

"Ease of Python" is often misread as "guess what I meant". Python imports are
explicit; it is *packaging* that is convenient. Keep the convenience that `use math`
resolves `math.jn` beside the entry file and under the manifest's source root — drop
the part where an unrelated sibling file changes what your program means and where
diagnostics point at the wrong file and line.

### D5 — Formatter (review item 3)

**Decision: comments are first-class trivia in the token stream; `jinn fmt`
preserves them. Until that lands, `fmt` refuses to modify any file containing a
comment.** Deleting the subcommand was the review's fallback; an "intelligent modern
compiler that frees the developer from clunky syntax" needs a formatter, so fix it.
The interim refusal is one commit and stops active data loss today.

### D6 — Persistence engine strategy (review item 11)

**Decision: fix the write discipline in place; do not swap to SQLite.** The review
floated SQLite as the fast path to credibility. Rejected: the `store` surface is
Jinn's differentiated idea, and an embedded SQL engine underneath re-imports the
database stack the language exists to obviate, plus a query-translation layer and a
second type system. The five fixes in 8-21..8-24 (atomic rename, real WAL replay,
CRC coverage, fsync checking, scoped transactions) are bounded, well-understood
work. `runtime/sqlite.c` stays available as a user-facing escape hatch, not as the
default engine.

Corollary held over: the layer is durable for a **single writer process**. Multi-writer
support needs file locking and is explicitly out of scope here; 8-21 makes the
single-writer contract enforced and documented rather than accidental.

---

## 1. Execution order

Tiers are ordered by dependency, not by severity. Tier 0 ships first because it is
zero-design and it restores the signals every later tier depends on.

| Tier | Theme | Tasks | Review items |
| --- | --- | --- | --- |
| 0 | Stop the bleeding | 8-1 … 8-4 | 4, 3a, 9a, 1a |
| 1 | Memory model | 8-5 … 8-9 | 2, 1, 16 |
| 2 | Runtime concurrency | 8-10 … 8-14 | 5, 6, 7 |
| 3 | Front end & tooling | 8-15 … 8-20 | 8, 10, 13, 14, 3b, 17 |
| 4 | Persistence | 8-21 … 8-25 | 11, 12 |
| 5 | Closing | 8-26 | 9b |

**Global definition of done**, applied to every task below in addition to its own:

- Full `cargo test` suite green, including the two tests 8-1 restores. Zero new warnings.
- The documentation that describes the changed behavior is updated **in the same
  commit** (the repo's existing "docs + tests in lockstep" rule, commit `f3bf9f2`).
- Any behavior a task fixes is pinned by a test that fails before and passes after.
- No task may be marked complete while it relies on `--lenient`, an `#[ignore]`, or a
  program being dropped from the harness.

---

## Tier 0 — Stop the bleeding

### ✅ 8-1 Restore the grammar-drift detector — **P0**, blocked_by: []
*Review item 4.*

`tests/ebnf_roundtrip.rs:35` reads `repo_root().join("jinn.ebnf")`, but commit
`f82402f` ("reorganize docs") moved the file to `docs/jinn.ebnf`.
`ebnf_has_no_dangling_rule_references` and `ebnf_keywords_are_reserved_in_lexer` both
fail with `NotFound`, so the mechanism that keeps `src/parser/`, `docs/jinn.ebnf`,
and `tree-sitter-jinn/grammar.js` in sync is off, and was committed off.

**DoD:** path corrected; both tests pass; the drift detector is verified to still
*fail* when a keyword is deliberately removed from the lexer (i.e. confirm it detects,
not merely that it runs). A CI check asserts the suite has zero failing tests, so a
red test cannot be committed again.

### ✅ 8-2 Make `jinn fmt` non-destructive — **P0**, blocked_by: []
*Review item 3, part 1. Decision D5.*

`jinn fmt` deletes every comment and overwrites the file in place: the lexer discards
`#` comments (`src/lexer/mod.rs:196-197`, no `Comment` token exists) and
`format_source` reprints from the AST (`src/fmt.rs:5-11`), then the driver writes over
the original (`src/driver/mod.rs:304-319`). Verified: a 5-line file lost 3 comments.

**DoD:** `fmt` exits non-zero with a clear diagnostic when the input contains a
comment and no comment-preserving path exists yet; in-place modification requires an
explicit `--write` (default prints to stdout). A test asserts a commented file is
byte-identical after `jinn fmt --write`. Full fix is 8-18.

### ✅ 8-3 Documentation honesty pass — **P0**, blocked_by: []
*Review item 9, part 1.*

Claims contradicted by observed behavior, to be corrected or removed now rather than
after the code is fixed — an accurate description of a broken thing is what gets it
fixed:

- `docs/error-effects.md:3` — remove "**Status: fully implemented and
  conformance-tested**".
- `docs/jinn.md` §Memory and ownership — the "prevents use-after-free, double-free,
  and data races" claim must not stand while §3.1–3.3 reproduce. Replace with the D1
  target state, explicitly marked as in progress, linked to 8-5.
- `docs/jinn.md` §Parallel loops — `sim for` lowers to a sequential counted loop
  (`src/mir/lower/loops.rs:307-460`; measured identical timings to `for`). Either
  document it as sequential-today or remove the section until it parallelizes.
- Broken examples in `docs/jinn.md`, each verified failing: actor `*value returns i64`
  (`expected NEWLINE, got returns`), `channel of i64` as a parameter type
  (`expected type`), `for v in ch` (SIGSEGV), `extern *printf(fmt as %i8, ...)`
  (varargs rejected), `sleep()` (undefined), then-position ternary nesting
  (parse error).
- `docs/jinn.md` §Persistent stores — state the single-writer contract (D6) and that
  a no-match query currently returns a zero row (until 8-25).

**DoD:** every code block in `docs/jinn.md` is extracted and compiled by a test, so
a documented example that does not compile fails CI. Adopt the
`docs/concurrency.md` standard — "derived from the runtime and codegen as they
actually exist today, not aspiration" — as a header contract on `jinn.md` and
`error-effects.md`.

### ✅ 8-4 Pin the review's repros as tests — **P0**, blocked_by: []
*Review item 1, part 1.*

Nine programs from the review, added now so Tier 1 and Tier 3 are verifiable rather
than argued. Each is currently a wrong-behavior test asserting the *observed* bad
outcome with a `// FIXME(8-N)` reference, flipped to the correct assertion by the
owning task.

Memory (→ 8-5..8-8): aggregate-param return double-free (§3.1, `free(): invalid
pointer`); cross-task shared-`Vec` mutation (§3.2, `realloc(): invalid old size`,
3/3 runs); alias-visible mutation (§3.3, `b is a; b.push(4)`).
Typer (→ 8-16, 8-17): cross-type `equals` SIGSEGV; declared-`String`-returns-`i64`
accepted; `'abc' + 1` accepted; heterogeneous `vec()` reading a leaked pointer as an
integer; multi-clause arity mismatch panicking at `src/parser/mod.rs:514` **and
exiting 0**.
Store (→ 8-25): `for u in all users` SIGSEGV.

**DoD:** all nine in a new `tests/review_2026_07.rs`; each names its owning task; the
harness fails if a `FIXME(8-N)` marker outlives its task's completion.

---

## Tier 1 — Memory model

### ✅ 8-5 Specify the aggregate memory model — **P0**, blocked_by: [8-3]
*Review item 2. Decision D1.*

Write `docs/memory-model.md` with the rigor of `docs/access-semantics.md`: what `is`
does for aggregates (move) versus scalars and strings (copy), when a read borrows,
what makes a borrow end, what a function parameter and a return value do to
ownership, how `take` and `copy` fit, and the interaction with `defer`, generators,
channels, and actor message payloads. Enumerate the programs that must be rejected
and the diagnostics they produce.

This document is the contract 8-6, 8-7, and 8-8 implement against; write it first so
the implementation has something to be wrong about.

**DoD:** every rule carries a named example; every rejection carries its exact
diagnostic text; the §3.1–3.3 programs each appear with their required outcome. The
existing "no reference cycles are constructible" property is stated as a consequence
so it is not lost. Reviewed against `access-semantics.md` and `escape/mod.rs` tiers
for contradictions.

### ✅ 8-6 Fix aggregate move-out on return — **P0**, blocked_by: [8-5]
*Review item 1 (§3.1).*

```jinn
*ident(v) returns Vec of i64
    return v
*main
    a is vec(1, 2, 3)
    s is ident(a)
    log(s.length)
```
→ `free(): invalid pointer`, at every optimization level, annotated or not, `return`
or tail expression. Dropping the trailing `log` gives a SIGSEGV instead, confirming a
genuine double-free. `jinn check` reports `check passed`.

Both the callee's parameter drop and the caller's binding drop fire on one buffer.
Contributing: a bind whose value is a plain variable emits no clone
(`needs_auto_clone`, `src/mir/lower/mod.rs:616-621`) and the consumed-set scan does
not recurse into nested statement bodies (`collect_block_consumed_ids`,
`src/typer/lower/block.rs:164-190`, `_ => {}` at 187), so a bind inside an `if` also
escapes the outer scope's accounting.

This is the highest-priority correctness fix in the plan: it is how every sort,
filter, and transform helper is written, and it is why the repo's own merge-sort
snippet (`snippets/101-200/s107.jn`) dies.

**DoD:** §3.1 runs clean under ASan at `--opt 0` and `--opt 3`; `s107.jn` sorts
correctly; nested-scope aggregate binds accounted for; the consumed-set scan recurses
into `if`/`while`/`match`/`for` bodies; 8-4's first assertion flipped.

### ◻ 8-7 One flow-sensitive ownership analysis — **P0**, blocked_by: [8-5]
*Review item 16.*

Replace the two overlapping half-checkers with one analysis. Today `src/ownership/`
sees everything but is the weaker: flow-*insensitive* across branches (a move in
`then` mutates outer `VarState` in place via `lookup_mut`,
`src/ownership/mod.rs:123-130`, no snapshot/merge at `verify.rs:58-68`), single-pass
on loops with no fixpoint (`verify.rs:69-95`), borrow counts that increment and never
release (`mod.rs:148-183`), and call arguments never treated as moves
(`verify.rs:233-241`). Meanwhile the typer has correct snapshot/restore and
branch-union merge (`src/typer/mod.rs:488-514`) plus a loop re-move check — but only
for explicit `take`.

Build on the typer's machinery: branch snapshot and union merge, loop fixpoint,
scoped borrow release, calls as moves-or-borrows per D1.

**DoD:** the analysis is the single authority (the redundant path is deleted, not
disabled); §3.3 is rejected with the D1 diagnostic; the `tests/audit_alpha/negative_*`
corpus still passes; `jinn check` runs it, so `check` and `build` agree — the review
found `check` reports `check passed` on a heap-corrupting program because it skips
ownership entirely (`src/driver/mod.rs:249-280`). Also unify the two driver pipelines
so `jinn build` stops skipping `HirValidator` (`driver/pipeline.rs` omits it,
`driver/mod.rs:437` runs it).

### ◻ 8-8 Reject cross-task aggregate aliasing — **P0**, blocked_by: [8-7]
*Review item 1 (§3.2).*

```jinn
*main
    shared is vec()
    together
        dispatch
            pusher(shared, 0)
        dispatch
            pusher(shared, 1000000)
```
→ `realloc(): invalid old size`, 3/3 runs, no diagnostic. The only cross-thread check
today rejects `@resource` types (`enforce_cross_thread_safe`,
`src/typer/mod.rs:603-618`).

Under D1 the second `dispatch` is a use-after-move and must not compile. The
diagnostic must point at the sharing and name the alternative (send it on a channel,
or give each task its own), because "use of moved value" alone will read as a
limitation rather than a caught race.

**DoD:** §3.2 rejected at compile time with an actionable message; the equivalent
correct programs (per-task vectors merged over a channel; an actor owning the vector)
compile and run; `together`/`dispatch`/`sim for`/actor-payload capture paths all
covered; `tests/concurrency_shutdown.rs` still green.

### ◻ 8-9 Extend the ownership fuzzer — **P1**, blocked_by: [8-6, 8-8]
*Review item 1, part 3.*

`tests/ownership_fuzz.rs` passes today while a 7-line program corrupts the heap,
which means it does not generate returns of aggregate parameters or cross-task
captures. Add both classes, plus nested-scope aggregate binds and aggregate struct
fields.

**DoD:** the generator reproduces §3.1 and §3.2 from a fresh seed **before** 8-6/8-8
are applied (verified by reverting them); every generated program is either cleanly
rejected or runs to completion with no abort, SIGSEGV, or ICE, under ASan via
`ci/sanitize.sh`.

---

## Tier 2 — Runtime concurrency

### ✅ 8-10 Generalize the park lock-handoff — **P0**, blocked_by: []
*Review item 5.*

The channel path is correct and non-obvious: a parking coroutine holds the channel
spinlock across the context swap and the *scheduler* releases it only after the
context is fully saved (`runtime/channel.c:239-244` + `runtime/sched.c:213-222`).
Three structurally identical sites did not get the same treatment, each publishing
the coroutine before its context is saved:

- `select` — `unlock_all` at `select.c:273`, swap at `:283`.
- actor join — `join_unlock` at `actor.c:147`, swap at `:149`.
- scope join — publishes `s->parent = self`, `scope_unlock` at `scope.c:272`, swap at
  `:274`.
- event loop — `jinn_event_loop_poll` unparks `w->coro` directly
  (`event.c:132-134`) with no synchronization against a concurrent
  `jinn_sched_park` (`sched.c:370-384`).

In each window another worker can swap into a `ctx` holding stale registers from a
previous park while the original thread still executes on that stack. This matches the
project's history of rare unreproducible crashes; commit `0219fe7` fixed exactly this
class in channels.

**DoD:** `held_chan_lock` generalized to a release-after-save handoff used by all four
sites; no park site releases its guard before the swap; a stress test drives
select/join/scope-join/event parks concurrently under TSan for a sustained run with
zero reports.

### ◻ 8-11 Redesign `select` — **P0**, blocked_by: [8-10]
*Review item 6.*

Three independent defects, all in `runtime/select.c`:

1. **It can only wait on one channel.** A single intrusive `next` pointer means the
   selector enqueues on one channel per attempt, round-robin (admitted at
   `select.c:227-234`). `select { receive a; receive b }` parked on `a` is never woken
   when only `b` becomes ready — round-robin needs the selector to keep waking, and a
   parked selector rotates nothing. This is a hang.
2. **Retry exhaustion is reported as the default case.** `max_retries = 256`
   (`:242`) prints "possible deadlock" then returns −1, which the caller reads as
   *default fired* (`:334-338`) — wrong semantics, silently.
3. **Close is never checked.** No `closed` test in any readiness scan or after wake.
   A select-receive on a closed empty channel parks forever, because close wakes only
   the waiters present at close time. `jinn_chan_recv` handles this
   (`channel.c:283-288`). Select-send to a closed channel silently reports success.

Plus a silent cap: `int poll_order[16]` with `limit = n < 16 ? n : 16` (`:167-169`)
never polls cases 17+.

**DoD:** per-case waiter nodes so a selector is enqueued on every case simultaneously
(Go's model); close observed in scans and after wake, with send-to-closed reporting
`false` consistently with `jinn_chan_send`; no case cap, or a compile-time diagnostic
if one is retained; retry exhaustion never masquerades as default. Tests: two-channel
select woken by either channel, select on a closed channel, select with 32 cases,
select with no default and no ready case, fairness under contention.

### ✅ 8-12 Scope child lifecycle — **P0**, blocked_by: []
*Review item 7a.*

`s->children[]` is appended at registration (`scope.c:92-94`) and never pruned, but
`sched.c:225-238` calls `jinn_coro_destroy(c)` when a child exits. `jinn_scope_cancel`
then reads `snapshot[i]->cancelled` / `->wait_chan` (`scope.c:159-167`), and
`scope_wake_cancelled` reads `c->state` and can **enqueue a freed coroutine into the
run queue** (`:240-246`). Any `together` where one child errors after siblings have
completed hits this. Separately, children beyond 64 are counted but not registered, so
cancellation silently misses them.

**DoD:** children are removed on exit, or destruction is deferred until the scope
releases its reference — no dangling entry is reachable from cancel or wake; the
64-child limit is either removed or made a hard diagnostic rather than silent
non-registration; a test cancels a scope after some children have completed and runs
clean under ASan.

### ✅ 8-13 Generator TLS staleness — **P0**, blocked_by: []
*Review item 7b.*

`jinn_gen_resume` sets `tl_gen_coro = c` on every resume (`coro.c:277`) but it is
cleared only on first trampoline entry (`:171-174`). Resumes 2..n swap into the middle
of `jinn_gen_suspend` and never reach the trampoline, so the thread's value is
permanently stale. The next *new* coroutine whose first run lands on that thread sees
`gen != NULL`, runs the generator's entry with the generator's argument on the wrong
coroutine, then spins in `for(;;){}` (`:179`).

**DoD:** the flag is cleared at every suspend/resume boundary, not only on first
entry; a test advances a generator several times, then spawns fresh coroutines on the
same worker and asserts correct execution; scan for other TLS values with the same
clear-once pattern.

### ✅ 8-14 Deque buffer reclamation — **P0**, blocked_by: []
*Review §7 critical (adjacent to item 5; not separately numbered in §11 — included
because it is the same bug tier and one worker queueing >1024 coroutines triggers it).*

`jinn_deque_grow` does `free(dq->buffer)` and swaps `buffer`/`capacity` as two
separate non-atomic stores (`deque.c:24-37`) while a thief reads
`dq->buffer[t & (dq->capacity - 1)]` (`:80`) — both a read of freed memory and a torn
buffer/capacity pair. Canonical Chase-Lev leaks retired buffers or reclaims by epoch;
it never frees inline. Slots are also plain pointers rather than atomics, a formal
data race between `:45` and `:80`.

Note the memory *orderings* in this file are textbook-correct (release fence in push,
seq_cst in pop/steal, correct CAS orders) — only the buffer lifecycle is wrong, so
this is a contained fix, not a rewrite.

**DoD:** retired buffers are retained or epoch-reclaimed, never freed inline; slots
are relaxed atomics; capacity and buffer are published as one atomic unit; a test
forces at least two grows with concurrent stealers under TSan and ASan; the
`calloc`-failure path no longer yields `capacity = 0` (`deque.c:14`, `:45`) and grow-OOM
no longer silently overwrites a queued coroutine (`:27-28`).

---

## Tier 3 — Front end and tooling

### ◻ 8-15 Explicit module resolution — **P0**, blocked_by: []
*Review item 8. Decision D4.*

`merge_source_files` (`src/driver/pipeline.rs:53` → `sources/modules.rs:229-239`)
recursively merges **every `.jn` file under the entry file's directory** into the
program, and `resolve_implicit_imports` (`sources/implicit.rs`) additionally parses and
flattens any file whose stem matches an undefined identifier used in `x.y` position.

Observed consequences: parse and type errors from unrelated files surface as
`warning:` lines; `snippets/guide_tour.jn` fails **in place** with
`operator '-' not defined for 'i64' and 'Vec of ?509' (line 18)` — line 18 is
`log('x={x}, doubled={x * 2}')`, which contains no `-` — while the byte-identical file
in an empty directory compiles and runs; and I reproduced the misattribution class
directly (`type 'Vec3' has no field 'x'` for a `Vec3` that declares `x as f64`). What
a program *means* depends on what else is in the directory.

**DoD:** a single-file compile reads that file plus the transitive closure of its
explicit `use` declarations, and nothing else; `use math` still resolves `math.jn`
beside the entry file and under the manifest source root; no diagnostic from a file
outside that closure can appear; `snippets/guide_tour.jn` compiles in place; project
builds resolve from the manifest `entry` only. Tests: a sibling with a syntax error, a
sibling declaring a colliding type, and a sibling declaring a store all leave the
target compile unaffected.

### ◻ 8-16 Propagate call-site generic arguments — **P0**, blocked_by: []
*Review item 10. Decision D2.*

`*bsort(v)` and `*peek(v)` (`t is v.get(0)`) fail with `ambiguous type: cannot infer
type for this expression (unresolved method-call return type)` while `*firstof(v)`
(tail `v.get(0)`) and `*sumall(v)` (`t is t + v.get(i)`) succeed — see D2's table.
The element type of a container parameter is never propagated from the call site;
it resolves only when a local constraint incidentally pins it.

Fix the plumbing: substitute the call-site's concrete type arguments into the
implicit-generic body's type environment at instantiation, and make
`resolve_core`/`occurs_in` recurse into `Struct(_, args)`
(`src/typer/unify/resolve.rs:26-82`, where `Struct` currently falls to
`_ => ty.clone()`, so `?v ~ Struct(n, [?v])` passes the occurs check). Also stop
`Struct(n, _) ↔ Enum(n)` from unifying with generic arguments skipped
(`src/typer/unify/mod.rs:555-564`).

Also in scope, since it is the message users hit most: the fix-it suggests `: i64`,
which the parser rejects — Jinn annotates with `as i64`.

**DoD:** all five previously-failing snippets compile with **no** added annotations
(`s045`, `s090`, `s094`, `s123`, `s168`); `std/random.jn` is strict-clean, since a
shipped stdlib module failing strict mode is not acceptable; the fix-it text is valid
Jinn; the `i64`-defaulting sites (`resolve.rs:130-133`, `unify/mod.rs:83-95`, ~25
`unwrap_or(Type::I64)`) are audited and each is either justified in a comment or
removed — a default that silently returns `i64` while pushing a strict error
(`resolve.rs:254`) is the pattern to eliminate. For an exported function with no
call site in the compilation unit, require an annotation or a trait bound (task 1-4's
bounded polymorphism) and say so in the diagnostic.

### ◻ 8-17 Route all diagnostics through `diagnostic.rs` — **P1**, blocked_by: []
*Review item 13.*

`src/diagnostic.rs` already implements rustc-style diagnostics — severities, codes
E001–W202, labels, notes, suggestions, and a caret-rendering `render()`
(`:61-184`) — and **nothing constructs one**. Real errors are `format!` strings:
`type_errors: Vec<String>` (`src/typer/mod.rs:101`), `eprintln!("ownership: {} (line
{}): {}")` (`driver/mod.rs:474`), newline-joined parse blobs
(`parser/mod.rs:86-106`). Formats differ per subsystem (`line N:C:`, `file:N:C:`,
`(line N)`) and some embed Rust `Debug` spans (`errset.rs:192`).

Two user-visible leaks must close with it: LLVM verifier output reaching users
(`add('one', 2)` prints `Call parameter type does not match function signature!` with
raw IR — a plain argument mismatch that reached codegen), and the multi-clause arity
panic at `src/parser/mod.rs:512-522`, which delivers a *correct* diagnostic as a Rust
panic **and exits 0**.

**DoD:** every user-facing error and warning is a `Diagnostic` with a span; one
rendering path; no `eprintln!`/`panic!` on any user-input path; a test asserts a
non-zero exit for every diagnostic-producing input (the arity panic exiting 0 is the
regression to pin); a codegen-reached type error is impossible or is reported as an
ICE with an issue-report prompt, never as IR. Per D2, instantiation-site errors name
both the call site and the definition.

### ◻ 8-18 Comment-preserving formatter — **P1**, blocked_by: [8-2]
*Review item 3, part 2. Decision D5.*

Add a `Comment` token / trivia channel to the lexer (`src/lexer/mod.rs:196-197`
currently discards them and `token.rs` has no variant), attach trivia to AST nodes,
and preserve it through `format_source`.

**DoD:** round-trip property test — for a corpus including every `snippets/` file,
`jinn fmt --write` twice is idempotent and preserves every comment, its position
(leading, trailing, standalone), and blank-line grouping; the 8-2 refusal is removed;
`docs/fmt.md` documents the trivia rules.

### ◻ 8-19 Repair and wire `tests/programs` — **P1**, blocked_by: [8-16, 8-17]
*Review item 14.*

16 of 89 programs fail in isolation — `binary_search`, `clause_fns_ext`,
`compiler_pipeline`, `data_structures`, `dispatch_fib`, `dispatch_multi`, `ecs`,
`embed`, `error_handling`, `interpreter`, `json_parser`, `linked_list`,
`sim_for_test`, `state_machine`, `store_index`, `syntax` — and **none of the 16 is
referenced by any test**; `tests/integration.rs` has 21 `expect_file` sites for 89
programs. The suite is green partly because broken programs fell out of the harness.

Two are codegen ICEs to fix, not delete: `FieldGet on pointer to unknown struct type
for field '__tag'` (`linked_list`) and `GEP index out of range` (`json_parser`).
`error_handling` still uses the removed prefix-`!` raise and needs migrating to
`err <Variant>`. Several others should resolve via 8-16.

**DoD:** every `.jn` in `tests/programs/` is either referenced by a test with expected
output or deleted with a reason; a harness test enumerates the directory and fails on
any unreferenced file, so rot cannot recur; the two ICEs are fixed with conformance
tests.

### ◻ 8-20 Remove dead aspirational subsystems — **P2**, blocked_by: [8-7]
*Review item 17.*

- `src/codegen/rc.rs` — 250 lines of retain/release/atomic-RC with **zero call sites
  outside the file**. Under D1 there is no shared-ownership model coming, so no
  refcounts are needed: delete it, along with `needs_atomic_rc`
  (`src/types.rs:129-143`) which exists only to serve it. Note "Perceus" in this
  codebase means drop *placement* (elision, sinking, fusion, malloc-reuse) in
  `src/perceus/mir_perceus.rs`, which is real and stays — the naming should stop
  implying reference counting.
- `src/incr.rs` artifact cache — call sites only *log* the dirty count
  (`driver/mod.rs:532-545`); `ArtifactCache::store` is never called outside its own
  unit test, so `lookup` can never hit. The design would also be near-useless if
  wired up: `function_cache_key` mixes in every function's signature rather than
  actual dependencies, and `hash_stmt` hashes `format!("{:?}", stmt)` including
  spans, so a blank line dirties everything below it. Compile times are 64 ms for a
  small file and ~1 s for an app, so delete the cache. `interface.rs` interface
  hashing is real and consumed (`driver/sources/modules.rs:123`) — keep it.
- Unused parallel-codegen scaffolding (`partition_work`, `codegen_threads`,
  `incr.rs:238-260`).

**DoD:** deleted (not `#[allow(dead_code)]`-ed); zero warnings; no behavior change;
if incremental compilation is still wanted, `docs/` records the requirements it would
have to meet, so the next attempt does not rebuild the same broken design.

---

## Tier 4 — Persistence

### ◻ 8-21 Atomic durable write discipline — **P0**, blocked_by: []
*Review item 11. Decision D6.*

Zero `rename()` calls and zero directory fsyncs exist in the layer; `fsync`/`fdatasync`
appear **only in `wal.c`**. Every other rewrite is truncate-in-place via
`fopen(path, "w+b")`:

- migrations — `migrate.c:238-240`, `:313-314`, `:380-382`: a crash between the
  truncating open and the final write destroys the entire store, in the code path
  whose purpose is safe schema evolution;
- `kv_save` — rewrites header plus all entries on *every* `set`/`del`/`incr`
  (`kv.c:93-118`), so a torn write leaves `count = N` with garbage trailing entries
  that reload as live data — silent resurrection of deleted keys;
- `jinn_ver_compact` (`version.c:164-174`) and bloom persistence
  (`bloom.c:92-99`).

**DoD:** every rewrite is write-to-temp, fsync file, `rename`, fsync directory; every
`fsync`/`fdatasync` return value is checked and surfaced; a crash-injection test kills
the process at N points across each rewrite path and asserts the store is always
either the old or the new state, never a mixture; the single-writer contract (D6) is
enforced with an advisory file lock and a clear diagnostic on contention — two
concurrent writers currently both "succeed" by luck.

### ◻ 8-22 WAL integrity — **P0**, blocked_by: []
*Review item 11.*

The skeleton is right (per-entry CRC32, env-selectable sync policy, group commit,
replay stops at first bad entry). The holes:

- CRC of 0 bypasses verification entirely — `if (stored_crc != 0 && computed !=
  stored)` (`wal.c:485`) — and the malloc-failure path deliberately writes CRC 0
  (`:202-204`);
- `payload_len` is outside the CRC (`:190-199`), so corrupted framing can frame
  garbage that verifies;
- `fsync` results discarded (`(void)fsync(fd)` at `:81`, `:88-89`, `:105`) — the
  PostgreSQL fsync-gate class, where an EIO is consumed and "committed" data is gone;
- no directory fsync after creating the log (`:149-157`), so the file itself may not
  survive power loss;
- bad magic silently truncates and recreates the log (`:145-149`) instead of
  triggering recovery;
- a torn tail is never truncated: replay stops at the first bad entry (`:485-489`) but
  open appends at `SEEK_END` (`:143`), so every post-crash append lands after the torn
  record and is unreachable to all future replays, permanently;
- `jinn_wal_write` returns `void`; a short write is an `fprintf` (`:172-186`).

**DoD:** CRC covers the full record including length; no zero-CRC bypass (a
malloc-failure path that cannot checksum must fail the write, not poison the log);
fsync returns checked and propagated; directory fsync on create; bad magic is a
recovery error, never silent truncation; the torn tail is truncated at the first
invalid record before any append; `jinn_wal_write` returns a status callers handle.
Extend `tests/wal_crash.rs` / `wal_property.rs` with mid-record corruption,
zero-CRC injection, and torn-tail-then-append.

### ◻ 8-23 WAL replay on recovery — **P0**, blocked_by: [8-22]
*Review item 11.*

Measured: after `kill -9` mid-insert, recovery was consistent (6,233 rows) — but
corrupting 64 random bytes at WAL offset 2,000 produced *identical* results to the
uncorrupted control, deleting the `.wal` and keeping the `.store` gave the same 6,233
rows, and deleting the `.store` and keeping the `.wal` gave **0**. The WAL is
write-only in practice: it is not replayed to recover records the data file lacks. The
durability story today is "the data file is the database, and the WAL is a log nobody
reads."

**DoD:** open performs recovery — replay committed WAL records not present in the data
file, then checkpoint; a store recovers correctly from a WAL plus a stale or missing
data file; WAL corruption is *detected* and reported rather than being invisible;
checkpointing is the only thing that makes a WAL prefix discardable. Tests assert each
of the three probes above now behaves correctly.

### ◻ 8-24 Scope and bound transactions — **P1**, blocked_by: [8-22]
*Review item 11.*

`static int jinn_txn_depth; static JinnTxnFile *jinn_txn_files;` (`wal.c:267-268`) —
process-global, not thread-local, no lock, in an M:N runtime where store code runs on
any worker. Two coroutines in `transaction` concurrently corrupt the list. Worse,
`jinn_wal_force` checks `jinn_txn_active()` (`:68-71`), so one coroutine's open
transaction silently disables per-record fsync for **every other** coroutine's writes.
The snapshot copies the entire data file into heap memory on first touch (`:312-331`),
and rollback rewrites in place (`:376-399`) — a crash mid-rollback leaves the store
corrupt with no recovery path.

**DoD:** transaction state is per-coroutine (or lock-protected), and one task's
transaction cannot alter another's durability; rollback is crash-safe via the 8-21
temp+rename path; snapshot memory is bounded (undo log rather than whole-file copy),
with a documented limit and a clear error when exceeded; `docs/jinn.md`'s atomicity
claims are verified by a concurrent-transaction test.

### ◻ 8-25 Query results as `Option` and first-class rows — **P1**, blocked_by: []
*Review item 12. Decision D3.*

Two defects. A no-match query silently returns a fabricated zero row:
`missing is users where age > 100; log('name=[{missing.name}] age={missing.age}')`
prints `name=[] age=0` — indistinguishable from a real empty-string, age-0 record.
And iterating all rows segfaults: `for u in all users` → `SIGSEGV at 0x51`, with no
test covering `all <store>` anywhere.

`docs/store-improvement.md:48` already has the right diagnosis — a query result is
"not composable (can't iterate, can't `.name` it, can't pass it to generic code)".
That is the root cause of both, and it should be fixed ahead of new store features.

**DoD:** a possibly-empty query has type `Option of <Record>` and a field read on a
miss is a compile error; the quaternary handles it with no added ceremony
(`users where … ? use($) ! not_found()`), and implicit propagation works in a fallible
function; `all <store>` yields an iterable row set that can be bound, iterated,
`.length`-ed, and passed to a function; the 8-4 store assertion flipped; conformance
tests for hit, miss, iterate-empty, iterate-many, and row-passed-to-a-function.

---

## Tier 5 — Closing

### ◻ 8-26 Regenerate benchmarks and methodology — **P2**, blocked_by: [8-1 … 8-25]
*Review item 9, part 2.*

`benchmarks/results.csv` reports `fibonacci` at 340.82 ms (Jinn) / 339.61 ms (C),
ratio 1.0, but the current `benchmarks/fibonacci.jn` computes `fib(42)`, measured at
1,703 ms (Jinn) / 658 ms (`gcc -O3`). Whatever produced that row is not the source in
the tree. `benchmarks/README.md` is already commendably honest about methodology —
flagging `store_ops` as disk-vs-memory, marking C-side gaps "language-only", noting
that a real M:N C baseline "may flip several ratios" — and that candor should extend
to the numbers themselves.

Also record the two genuine codegen gaps found: `fib(42)` at 2.59× despite LLVM
applying an accumulator transformation and marking the function
`nounwind memory(none)`, and `tight_loop` at 1.48× on a 16×-unrolled 2-billion-iteration
loop. Both merit a `perf` profile; everything else measured 0.34×–1.13× against
`gcc -O3` with byte-identical outputs (median ≈ 0.97×), which is the claim worth
defending.

**DoD:** `results.csv` regenerated from the current sources on stated hardware with
run counts and variance; every ratio involving the Jinn scheduler against a
single-pthread C baseline is either re-baselined or marked non-comparable in the CSV
itself, not only the README; a CI job re-runs the suite and fails on a >10%
regression; the two outliers have profiles attached and follow-up tasks if a fix is
identified.

---

## 2. Coverage

| Review §11 item | Tasks |
| --- | --- |
| 1 — `Vec` double-free + shared-`Vec` race, regression tests | 8-4, 8-6, 8-8, 8-9 |
| 2 — Decide the aggregate memory model | 8-5 (decision D1) |
| 3 — `jinn fmt` comment destruction | 8-2, 8-18 (decision D5) |
| 4 — Red tests / `jinn.ebnf` path | 8-1 |
| 5 — Generalize the park lock-handoff | 8-10 |
| 6 — Redesign `select` | 8-11 |
| 7 — Scope children + `tl_gen_coro` | 8-12, 8-13 |
| 8 — Directory-tree module absorption | 8-15 (decision D4) |
| 9 — Align docs with implementation | 8-3, 8-26 |
| 10 — Inference on unannotated parameters | 8-16 (decision D2) |
| 11 — Persistence durability | 8-21, 8-22, 8-23, 8-24 (decision D6) |
| 12 — Store queries as `Option`; `all` segfault | 8-25 (decision D3) |
| 13 — Diagnostics through `diagnostic.rs` | 8-17 |
| 14 — Orphaned `tests/programs` | 8-19 |
| 16 — One flow-sensitive alias analysis | 8-7 |
| 17 — Delete dead subsystems | 8-20 |

Out of scope by direction: item 15 (reduce sigil overloading) and item 18 (reinstate
in-code TODO markers).

Adjacent critical finding pulled in because it is the same bug tier: review §7's
Chase-Lev grow buffer free (8-14).

## 3. Suggested sequencing

Tier 0 is four small independent tasks and should land first as one batch — it
restores CI signal (8-1), stops active data loss (8-2), makes the claims honest
(8-3), and makes every later fix verifiable (8-4).

Then Tiers 1 and 2 run in parallel: they touch disjoint code (typer/ownership versus
`runtime/*.c`) and are the two P0 clusters. Tier 1 is strictly serial internally
behind the D1 specification (8-5); Tier 2's five tasks are independent of each other
except 8-11, which wants 8-10's handoff first.

Tier 3 is mostly independent and can absorb spare capacity, except 8-19 which wants
8-16 and 8-17 landed. Tier 4 is independent of Tiers 1–3 and can start any time.
8-26 is last by construction.

The alpha bar this plan is written against: **no program that compiles may corrupt
memory, and no documented example may fail to compile.** Tiers 0–2 and 8-25 are what
that bar requires; Tiers 3–5 are what make it credible.

---

## 4. Relationship to existing tracked work

Four pending tasks in `remediation.md` overlap this plan. Fold them in rather than
running both — and note that **two of them rest on premises the review contradicts**,
which is itself an instance of the problem the review flagged: a task's status
description can drift from what the code does.

| Existing | Overlaps | Action |
| --- | --- | --- |
| **2-18** `jinn fmt` formatter | 8-2, 8-18 | **Premise is wrong.** 2-18 records fmt as "ALREADY SHIPPED" and lists its remaining gaps as conformance tests, idempotence, and reparse-equivalence. It does not mention that fmt deletes every comment and overwrites in place. Close 2-18 into 8-2 + 8-18; keep its idempotence and reparse-equivalence requirements, which 8-18's DoD absorbs. |
| **2-17** store crash-consistency tests | 8-21, 8-22, 8-23 | **Premise is wrong.** 2-17 calls `wal_crash.rs` + `wal_property.rs` a "GOOD baseline" with only torn-write simulation missing. But those tests pass while the WAL is never replayed on recovery (8-23) and while mid-WAL corruption is undetectable (measured: corrupting 64 bytes changed nothing; deleting the `.wal` changed nothing; deleting the `.store` gave 0 rows). A suite that green-lights a write-only WAL is not a baseline to extend — 8-23's DoD must be met first, then 2-17's torn-tail, multi-store interleaving, and double-recovery-idempotence items land on top. |
| **2-16** expand store query language | 8-25 | Sequence 8-25 **first**. 2-16 proposes aggregations, projections, and group-by; `store-improvement.md:48` and 8-25 identify the blocker underneath all of them — a query result is not a first-class composable value. Building aggregations on a result type that cannot be bound, iterated, or passed to a function repeats the work. |
| **2-15** generic instantiation diagnostics | 8-16, 8-17 | Complementary, and 8-16 makes it load-bearing: under decision D2 errors surface at instantiation, so the instantiation-chain diagnostic 2-15 specifies ("required by X instantiated at Y") is what keeps that tradeoff acceptable. 2-15's Type-pretty-printing audit (`I64` vs `i64`) belongs in 8-17's single rendering path. |

Related but non-overlapping, kept separate: **2-11/2-12** (safe/unsafe raw-pointer
boundary) should cite `docs/memory-model.md` from 8-5 once it exists, since D1 changes
what a raw pointer can alias. **2-31-11** (`@durable`/`@relaxed`/`@volatile`
decorators) presumes a working durability floor and should follow 8-21/8-22.
**2-31-9** (persistent secondary indexes, marked complete) is affected by 8-23: an
index with no LSN coupling to the store is trusted with no rescan after a crash
(`index.c:103-129`), so 8-23's recovery work must re-validate that claim.

Already tracked, out of scope here: review item 15 (sigil overloading) is task 3's
"disambiguate sigil overloading" under P2. Review item 18 (reinstate in-code TODO
markers) is not tracked anywhere and is excluded by direction.
