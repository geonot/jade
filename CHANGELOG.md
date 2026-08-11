# Changelog
- **[144]** (2026-08-10) `jinn fmt` no longer destroys code: printer rebuilt against the real grammar, gated by compile-after-format

X-1 measured 164 of 688 corpus files that compiled before `jinnc fmt` and not
after, applied silently by `--write`. The damage was printer grammar drift —
the printer had never been held to the grammar the parser actually accepts.
Measuring by damage class and fixing each:

- **Invented grammar.** Extern printed without `*`, parens, or `as`
  (`extern usleep us i32`); store statements printed SQL (`insert into`,
  `delete from`) with every filter dropped; store headers lost their
  decorators and fields lost theirs (`@search`, `@index`, `@default`,
  relations, store methods — all silently deleted, i.e. reformatting a store
  changed its schema); actors lost their state fields entirely and printed
  handlers in a form the parser rejects.
- **Dropped semantics.** Bind access modifiers (`row is copy t.get(i)` →
  `row is t.get(i)`) and bind type annotations vanished — reformatting changed
  ownership semantics. Generic `of T` clauses disappeared from types, enums,
  and functions; `! E` error rows disappeared from function signatures; layout
  attributes (`@packed`, `@align`) disappeared from types; the implicit `self`
  parameter was printed explicitly.
- **Wrong surface forms.** Float literals printed `0.0` as `0` (silently
  changing arithmetic types), `&`/`*` for the `%`/`@` pointer operators,
  `use std.time` for `use std/time`, `Tree<T>` for `Tree of T`,
  `Map of String, String` in positions where only the value-sugar
  `Map of String` parses, `err` variants with `of` instead of parens, and
  `not equals` for `neq`.
- **Structure loss.** Multi-clause functions desugared to statement-position
  if-expressions printed as `cond ? x ! ...` — now printed as real `if`
  blocks. Query blocks, `select`, and `dispatch` printed as `...` — now
  printed in their block forms. `for` loops lost their `to`/`by` bounds and
  index binders; desugared `if x is pat` matches printed empty arms — now
  `nop`. Precedence was never parenthesized (`(n & (n-1)) equals 0`
  reformatted into a parse error); binary operands, postfix receivers, and
  cast operands now get parens when compound.

Two enforcement changes make the class stay dead.
`fmt_output_still_frontend_checks_over_corpus` formats all 574 files of
`snippets/`, `tests/programs/`, `benchmarks/`, and `std/` and fails if any
file that frontend-checked before formatting stops afterward — the gate X-1
said was missing by construction (the old gate checked idempotence only, and
only over snippets/). And `fmt --write` now re-parses its own output before
writing, refusing with a formatter-bug message instead of damaging the file —
so the next printer regression is a refusal, not silent corruption. X-1 drops
to minor: `apps/` stay ungated (per-file checks need project context) and a
few expression-position fallbacks (`asm`, block expressions) survive behind
the guard.

- **[143]** (2026-08-10) roadmap remediation: 21 items closed — the ownership seams, the verified type-system defects, embed gating, and the store residue

The five memory-model blockers all lived at the same seam — moves of
*variables* were tracked exactly while moves and borrows through *expressions*
were not — and they close together at variable granularity (place granularity
stays open as M-6):

- **Constructor expressions move their sources (M-1).** Struct literals, enum
  payloads, `vec(v1, v2)`, tuples, and arrays now tombstone a bare aggregate
  variable in value position exactly as `b is a` does, with a dedicated
  `CtorCapture` diagnostic. This immediately caught a latent double-free
  pattern; move recording is suppressed inside `return`/`break`/`err` payloads
  because those paths diverge and the conservative loop-repeat check was
  rejecting `return JObject(o)` inside a `loop` (seen in `std/json.jn`).
- **One call site, one owner (M-2).** A per-call alias check rejects the same
  variable in two consuming positions, or in one consuming and any other
  position, of a single call — `combine(v, v)` was minting two owners of one
  buffer and exiting 0.
- **Call-site exclusivity (M-3).** A new parameter-mutation inference
  (`src/typer/mutate_infer.rs`, a fixpoint structurally identical to
  consuming-parameter inference, covering free functions and methods and
  propagated to monomorphized names) rejects passing one variable twice to a
  call that mutates it through either parameter. `app(v, v)` no longer chases
  its own append. The roadmap's `noalias` miscompile half was already stale —
  codegen no longer emits those attributes.
- **Mutating a temporary copy is an error (M-4).** Container element reads
  still copy (M-4r/M-13 record the cost), but `grid.get(0).push(3)` — and any
  builtin mutating method, or user method whose receiver-mutation was
  inferred, on an element-read receiver — is now a compile error instead of a
  silently discarded update. This found two real bugs in std:
  `dataframe.from_csv` built every column into a discarded temporary (all
  frames came back empty), and `add_row` appended every cell into one. Both
  rewritten column-first.
- **Iteration borrows (M-5).** `for x in v` registers an iteration borrow of
  `v` for the body: mutating calls on `v`, moves of `v`, and passing `v` to a
  parameter the callee mutates are compile errors, so the append-while-iterate
  hang is unrepresentable in the direct-loop form. Map/`Iter`-desugar and
  field-place loops remain open as M-5r.
- **Consuming methods by body, not by name (M-8, reduced).** User methods now
  run through the same escape scan as free functions, so a storing method is
  consuming whatever its name; the builtin name list remains only for
  runtime-implemented methods and for receivers whose types are unknown at
  AST-scan time (the residual imprecision).

Type system and diagnostics: generic-type methods are monomorphized *with
bodies* — instantiation queues the substituted method, a post-lowering drain
lowers it through the same path as plain type methods, so `Box of i64(42)`
followed by `b.get()` works (old T-1; annotation-driven instantiation also
declares methods now). Undefined names are rejected in the typer with a span
instead of surviving to a span-less codegen error, which also makes
`--emit-hir` exit non-zero on broken code — the gap that made the frontend
gates vacuous (old T-3; the fix immediately exposed that `transaction` blocks
were typer-scoped while their bindings are function-scoped at runtime, now
lowered scope-free to match pinned behavior). A bare `! E` means
`Result of Unit, E` for real: the ok type is `Void`, a valued tail is a typer
diagnostic, and the old inkwell panic is unreachable (T-4). Trait methods
parse `! E` (T-5; conformance checking still open as T-5r). Duplicate
catch-all clauses are a parse error naming both sites (T-6). Unsolved type
variables warn by default — container elements included, which was the silent
path — and `--strict-types` escalates *unconstrained* unsolved variables to
errors while leaving ordinary literal defaulting alone, so the flag's name is
true without rejecting `x is 42` (T-7). The M-4 check also found
`snippets/101-200/s172.jn` transposing a matrix into discarded row copies —
the differential gate had been pinning agreement on silently wrong output.
Out-of-range literals against an int annotation warn with the wrapped
value (T-8). ALL_CAPS constants cannot be shadowed (T-9). The whole compile
runs on a 256 MiB thread, so flat operator chains bounded only by memory
replace a stack overflow at ~5000 terms (T-11). A UTF-8 BOM is stripped and
non-ASCII lex errors print the decoded character, not mojibake (T-12).
Int→float coercion works in bind annotations and for integer literals against
float operands in binary expressions, and constraint-mismatch messages name
expected/found in the caller's order (T-14).

Effects and store: `embed` rejects absolute paths and canonicalized escapes
from the source directory (C-2). The comptime purity classifier is a least
fixpoint that consults callees instead of deciding purity from argument shapes
(C-3). `search` returns rows — the typer types it as the store's row struct
and codegen resolves the posting ids against the store file, skipping deleted
records, in id order (S-4). `JINN_WAL_SYNC=group` outside a transaction
degrades to per-record `fdatasync` instead of never syncing (S-6). A WAL with
bad magic exits 2 with the existing message instead of `abort()` (S-9,
reduced). `jinn init NAME` scaffolds into `NAME/` (X-2). The stale half of S-7
(`in [..]` "missing" — it exists in the statement filter path) is corrected in
the roadmap.

- **[142]** (2026-08-07) the std gate now means what its name says; apps/ enters the gates and two of them were broken

[141] argued that the gates measuring a narrower surface than their names
implied is what let everything else drift, and then did not change the
definition that caused it. `docs/std.md` still defined **alpha-stable** as
`jinnc std/<m>.jn --lib --emit-hir` exiting 0, and
`tests/std_stable_subset.rs` still ran only that. All 49 modules do import and
link today — that was verified by hand while fixing AR-F1 — but nothing pinned
it, so the exact drift the entry diagnosed could recur silently.

The tier is now defined by two bars and both are enforced.
`std_stable_subset_imports_and_links` writes a five-line `use std/<m>` program
per module, compiles it through codegen, links it against the runtime and runs
it. The frontend check stays, because it covers functions no importer reaches;
the link check catches what lives entirely past the frontend, which is where
AR-F1's ten failures were — missing runtime C symbols and codegen ICEs. Both
gates were checked against a deliberately broken `std/math.jn` to confirm they
are not vacuous.

**`apps/` was in no test at all, and two of the 21 had stopped compiling.**
Only `alpha_release_demo` was ever built, by the smoke script. Sweeping the
rest by hand found `blockchain_node` reading a `Block` after pushing it into a
`Vec`, and `lattice_crypto` reading an `Sk` whole after moving out its `pk`
field. Both are *correct* rejections by the use-after-free diagnostics [141]
added under AR-F3 — the corpus was never updated to match the language it
documents. Fixed where the defect is: read `b.hash` before the push rather
than after it, and `copy sk_a.pk` out instead of moving it. Both apps are
self-checking and now report `invalid_blocks 0` and `key.match 1` /
`roundtrip.ok 1` / `mac.ok 1`. `tests/apps_build.rs` builds and runs all 21 in
1.5s, fanned out through `par_map`, so this cannot recur quietly. Recorded as
AR-F33.

The 36 `benchmarks/` programs were swept the same way and all compile at
`--opt 3`; they are left out of the gates because they are timing harnesses,
not assertions.

- **[141]** (2026-08-06) alpha review remediation: 24 findings fixed, three more found by the new gates

The 2026-08-06 alpha readiness review is written up in
docs/alpha-review-findings.md; per-finding remediation state is in
docs/alpha-review-status.md. This entry records what was decided, because
several of the fixes turned on which of two documents was right rather than
on which code was wrong.

**The gates measured a narrower surface than their names implied, and that
is what let everything else drift.** `docs/std.md` defined "alpha-stable" as
`--emit-hir` exiting 0 — type-checks, nothing more. All 49 modules passed it
while 10 could not be imported at all by a five-line program. Of 546 corpus
programs only the 87 in tests/programs were ever compiled and run; snippets/
was parsed and formatted, apps/ and benchmarks/ were referenced by no test.
MIR verify was behind `#[cfg(debug_assertions)]`, so it was absent from the
compiler users run, and it did not check phi/edge type agreement anyway. A
`KNOWN_ICE` whitelist asserted that a program still crashed.

So the gates came first. `tests/corpus_differential.rs` compiles **and runs**
snippets/ and tests/programs/ at `--opt 0` and `--opt 3` and diffs stdout and
exit code. MIR verify checks phi types and runs in release (`JINN_MIR_VERIFY=0`
opts out). The whitelist is replaced by a list that asserts the *specific*
diagnostic, so it fails both if the program regresses to a panic and if it
starts compiling. `scripts/alpha_release_smoke.sh` pointed at a directory that
moved to apps/, which had been aborting preflight at step 5 of 7 — so steps 6
and 7 had not run in the gate at all.

That paid for itself immediately. The differential harness found a miscompile
in snippets/201-300/s298.jn that was flaky at *both* optimisation levels: an
actor's queued messages were dropped at process exit. Actor coroutines are
daemons, so `jinn_sched_run` returned without waiting and nothing closed the
mailboxes. Actors now register in a live list, and exit closes and drains
them. Two further defects the review had not found came out of neighbouring
fixes: passing a struct that owns a Vec double-freed it (parameter ownership
was decided before inference resolved the type, so an unannotated parameter
defaulted to Owned and the callee emitted drop glue for a value the caller
still owned), and Map values were stored in an 8-byte slot, which made
`Map of String` structurally impossible and was the real cause of the
`logging` and `url` LLVM-verification failures.

**Where the spec and the implementation disagreed, the spec won.** Two
findings were framed backwards in the review and are worth recording as
decisions:

- `q is p` on a struct of scalars aliased. docs/memory-model.md §1 puts such a
  struct in category Value — "deep copy; both live, independent" — so the
  binding now copies. `tests/crash_safety.rs` had a passing test asserting the
  aliasing; it now pins value semantics.
- Writes through a struct *parameter* were reported as "lost inside a loop".
  docs/access-semantics.md §4.1 makes a POD-struct parameter `Owned`, i.e. an
  independent copy, so the mutation *reaching* the caller was the defect. The
  callee now copies POD struct parameters on entry and borrows drop-needing
  ones, which is what the document says and what makes the loop and non-loop
  cases agree.

**Double-quoted strings now take escapes and interpolation.** They were raw,
which docs/jinn.md never said and which quietly broke 16 std modules that
write `"\n"`, `"\r\n"` or `"\x1b["`. Sixteen call sites across terminal,
signal and the tour were already written assuming interpolation, so making it
work fixed more than it changed; one snippet wanting a literal brace now
escapes it. `jinn fmt` was emitting string contents unescaped, which made this
non-idempotent — it now escapes properly.

**Other P0s, each verified by re-running the original reproduction:** join
points are unified in the typer with a diagnostic naming both branches
(this one defect was behind eight of the ten unimportable std modules);
MIR DCE no longer deletes trapping instructions, so `as strict` aborts at
every optimisation level; enum→int reads only the discriminant instead of
four bytes past the object, float→int saturates instead of emitting poison,
and explicit enum discriminants are carried through instead of being parsed
and discarded; `destroy` builds survivors in a temp file and renames instead
of truncating the live data file, and invalidates indexes that its offset
shift would otherwise leave pointing at the wrong row; the store has a real
intra-process writer lock (flock on a shared fd is a no-op between
coroutines); `together` inside a coroutine joins its children, by threading
the scope pointer through spawn rather than reading TLS that a context swap
has already invalidated; `%String` passes the data pointer rather than the
24-byte header, which was silently corrupting every hash and file path over
23 bytes; a constant in pattern position compares instead of binding, which
is why `json.parse` could not parse anything; `main` binds the success value
of a fallible call and exits non-zero on an unhandled error; four
use-after-free constructions in safe code are compile errors; dependency URLs
cannot escape the package cache, and a corrupt store header cannot overflow
the heap.

**ci/sanitize.sh is usable again.** It ran `cargo clean` and then the whole
suite twice, destroying the shared release build and taking about an hour, so
nobody ran it — which is why the review's memory findings rested on crash
signatures rather than an instrumented sweep. It now builds an instrumented
runtime into its own target directory and sweeps nine targeted programs under
ASan+UBSan and TSan in about two minutes. On its first run it caught a data
race in the actor-drain code added earlier in this same change.

**Benchmarks.** `coroutine_spawn` ran 100,000 iterations in Jinn against
1,000,000 in C, so the published ratio was wrong by 10x before the paradigm
difference was even considered. Counts realigned; the remaining gap is
ucontext's per-switch sigprocmask against a syscall-free context switch, and
says so at the top of the file. `run_benchmarks.py` now captures stdout for
every language, not just Jinn, and names any benchmark whose languages
disagree — which would have caught this for free.

**Tooling.** `jinn bind` emitted `//` comments, inlined C block comments into
signatures and bound macros as functions, so its output was rejected at line
1; it now produces parseable Jinn from real system headers and honestly skips
what it cannot represent, with a reason. The LSP treated UTF-16 positions as
byte offsets, so hover and goto-definition stopped working to the right of any
emoji. `jinn run` hashes dependency sources instead of trusting the entry
file alone; `.jni` interface reuse is off by default, since `to_decls()` never
reconstructed the functions list.

Regression coverage is in tests/alpha_review_regressions.rs and the corpus
differential harness. Still open, and recorded as such in the status document:
capabilities remain inert, generics remain narrow, transaction atomicity is
unchanged, and the fmt printer's grammar drift is untouched — the standing
recommendation for capabilities, `fmt --write`, `jinn bind` and `.jni` is
demotion rather than repair.

- **[140]** (2026-08-03 01:23) test suite 65s -> 40.5s: parallel test runner, fan-out corpus harnesses, sharded proptests
- **[140]** (2026-08-02) test suite 65s -> 40.5s

The edit-test loop was the bottleneck, not the build (an incremental
`cargo build --release` is 1.8s). Measured first: nearly every test forks
jinnc, and a two-line program costs ~90ms — ~16-22ms of process startup
(mostly mapping the 157MB libLLVM.so), ~8ms of actual compilation, and
35-50ms of `cc`/`ld`. The suite is process cost, not assertion logic.

Four changes, each measured:

- `scripts/test.sh` runs test *binaries* concurrently. `cargo test` walks
  targets strictly one at a time, so any suite that cannot saturate the box
  leaves cores idle — that was the single biggest waste. Same binaries, same
  assertions, same exit semantics; failing suites' logs are printed.
- Corpus harnesses that were one `#[test]` looping serially now fan out
  through `tests/support/parallel.rs` (`std::thread::scope`, no new
  dependency, input-order-preserving so failure output stays deterministic):
  programs_harness 7.15s -> 1.85s, doc_examples 3.58s -> 1.31s. doc_examples
  needed its sequential `doctest:file` state resolved into per-item state
  before the map, not inside it.
- ownership_fuzz's single 200-case proptest is now 8 shards of 25 so libtest
  can schedule them: 9.38s -> 2.56s. Same total cases.
- Compile-and-run property suites read their case count from
  `tests/support/cases.rs` (`JINN_PROPTEST_CASES`). Local default is small;
  CI and preflight set 64, so release gating keeps the full coverage and
  `.proptest-regressions` seeds replay at any count.

Rejected after measuring: swapping linkers (mold/lld absent, ld.gold is
*slower* than default ld) and raising concurrency past nproc — the reference
box is 4 real cores, and 8x8 oversubscription inflated CPU 207s -> 321s and
made wall time *worse*.

Fixed a bug in the new runner before trusting it: naming logs by basename
collapsed the two binaries both named `jinnc` (from src/lib.rs and
src/main.rs) into one file, silently dropping 309 lib tests. Test-count
parity with `cargo test` is now exact (2019), and failure detection was
verified with deliberate canaries in both an integration suite and
src/lib.rs. Rationale and the cost model are written up in docs/testing.md.
- **[139]** (2026-08-02 19:11) strip all code comments; reformat

Removed every comment from Rust (src, tests, fuzz, build.rs), C (runtime,
test harnesses), and Jinn (std, libjn, apps, examples, benchmarks,
snippets, test programs) sources. ~8100 lines of comment removed.

Used the existing scripts/strip_rust_comments.py for Rust — it is a real
lexer (raw strings with arbitrary hash counts, byte strings, char literals
vs lifetimes, nested block comments) and needed no changes. Added a
companion scripts/strip_c_comments.py for C and Jinn, which also drops
lines that held only a comment rather than leaving blank gaps. Verified
both preserve string and char literal contents: `"https://"`, `b'#'`, and
the printable-ASCII tables in std/binary|bytes|hex are all intact, and the
one surviving `/*` is inside a format string in src/bind.rs.

Reformatting then surfaced a collapsible_if in typer/lower/decl.rs that
the comments had been padding out; folded it into the let-chain. Fixed a
doubled-space run in the adjacent "declares returns X but its body
produces a different type" diagnostic while there.

Zero warnings, clippy clean, fmt clean, 2013/2013 tests, apps 21/21,
benchmarks 36/36, std gate green.
- **[138]** (2026-08-02 18:59) tests run in their own cwd; add gitignored .data/ for ad-hoc runs

A store resolves its .store/.wal relative to the process cwd, so any test
that compiled into a tempdir but ran the binary with the harness's cwd
wrote its state into the repo root. 18 run-sites across 11 test files did
exactly that; tests/error_effects.rs was the live offender, leaving
items.* and jobs.* behind on every `cargo test`.

Every compiled-binary run site now passes .current_dir(dir.path()) (or the
harness's scratch dir). A full suite run now leaves the root clean.

Added .data/ — a gitignored scratch cwd for running programs with
persistent state by hand, with a tracked README explaining why it exists.
Benchmarks already isolate into benchmarks/_build/cwd_*; unchanged.

Removed 30 stale .store/.wal files from the root (untracked, already
covered by .gitignore).
- **[137]** (2026-08-02 18:25) std gate green: all 49 alpha-stable modules type-check

The last failing test suite is now passing. Full suite 2013/2013, zero
failures, clippy clean, fmt clean, apps 21/21, benchmarks 36/36.

Two compiler defects found by working the gate:

1. extern opaque-pointer rule. A declared `%i8`/`%u8`/`%void` parameter is
   C's opaque-handle convention (`char *`/`void *`). The typer accepted a
   String there but rejected every other raw pointer, so passing a vec
   header to `jinn_spawn_exec(const void *)` was impossible — std/process
   could not bind its own runtime. At an extern boundary the C signature
   is the user's assertion; any raw pointer now satisfies an opaque
   pointee. Non-opaque pointees still unify strictly.

2. `copy` is a binding modifier, not an expression (access-semantics.md
   §2), so `copy self` in tail position fails. std/dataframe leaned on
   `return self` from a by-ptr method instead, handing out a borrow where
   the signature promised an owned DataFrame. Added `__clone_df` and
   routed all 7 "no matching column" paths through it.

std source fixes: bangle/http/event/url/terminal/dataframe. Types are
global and unqualified (std.md), so error unions are written `! NetError`,
not `! net.NetError`. http's PoolEntry.fd was i64 while every socket API
is i32.

New regression tests: method + ptr-method `! E` unions, no-Debug-spans in
error diagnostics, extern opaque-pointer FFI round-trip through a real
subprocess.
- **[136]** (2026-08-02 18:12) std gate: remove sqlite; fix `! E` on methods; clean error diagnostics

Removed std/sqlite.jn — the wrapper never type-checked and shipping a
broken binding is worse than shipping none. runtime/sqlite.c stays: it is
live FFI with a passing integration test, usable via `extern
*jinn_sqlite_*`. build.rs/jinn_rt.h/std.md updated.

Compiler bug found while fixing std/net: `! E` was desugared to
`Result of T, E` for free functions (resolve.rs:140) but BOTH method
signature paths dropped f.error_types, so `err X` inside any method was
rejected with "this function returns T". Methods now honor `! E`
end-to-end through codegen.

Six user-facing diagnostics printed raw `Span { start, end, line, .. }`
Debug output. All now use span.loc() -> file:line:col.

std sources fixed:
- argon/rational/date/logging: `"" + n` is not concatenation; the typer
  deliberately rejects it. Use to_string(n), the idiom the rest of std
  already uses.
- net: replaced the `return false` C-ism sentinel with a real `err
  NetError` union. This is what the error-effect system is for, and it
  unblocked bangle/http/event which consumed the sentinel API.

tests/programs/actors_multi.jn was racy — two independent actors have no
cross-mailbox ordering guarantee, so the snapshot flaked ~40% of runs.
Sequenced the drains; 20/20 deterministic.

std gate 13 -> 7 failing modules. clippy clean, fmt clean.
- **[135]** (2026-08-02 17:37) review eval: P1-6 — mir-drops summary is behind --debug-drops only

The driver printed the mir-drops summary on any compile where the pass
did useful work, so a clean user build leaked internal optimizer
telemetry to stderr. The roadmap's B.3 requires internal info to sit
behind an explicit flag. Gate solely on cli.debug_drops; tests/drops_debug
already passes the flag explicitly, so coverage is unchanged.

Tests 2008 passing, clippy clean, fmt clean.
- **[134]** (2026-08-02 17:35) review eval: reject unknown constructors; restore fmt gate

P0-class soundness hole found while auditing the std gate: the typer's
struct-literal path fell through to Type::Struct(name) for ANY unresolved
name, so `x is TotallyUndefinedThing()` compiled, linked, and ran. Every
typo in a constructor position was silently accepted, and the resulting
fabricated type is what produced the misleading "expected `None`, found
`Option__G_i64`" cascade in std/collections.jn.

- Reject constructor calls that name no declared type/actor/variant.
- Levenshtein-based "did you mean" over known types and variants.
- Name the Jinn spelling directly for foreign prelude words
  (None/Null/Nil -> Nothing, Just -> Some, Error -> Err).
- Fix spec divergence: jinn.md said `None`, implementation and
  error-effects.md/fmt.md say `Nothing`. `Nothing` wins; jinn.md and
  std/collections.jn corrected.
- cargo fmt --all: 58 files were drifted at HEAD, so CI's fmt gate was
  already red. Now clean.

std gate 14 -> 13 failing modules. Tests 2004 -> 2008. clippy clean,
fmt clean, apps 21/21, benchmarks 36/36.
- **[133]** (2026-08-02 17:23) review eval: fix consume-and-rebind false positive in move tracking

The post-lowering move pass (record_take_moves_in_stmt) re-marked a
consumed call argument as moved without observing that the enclosing
Bind/Assign re-initialized the target. `g is grow(g, x)` — the canonical
builder idiom — was therefore rejected as use-after-move, breaking 6 of
21 sample apps (blockchain_node, inventory_mgr, iot_telemetry, kv_store,
order_book, route_planner).

Clear the target's moved-state after recording the statement's moves, in
both Bind and Assign. Revival is precise: rebinding a different name
still leaves the source tombstoned.

apps 15/21 -> 21/21. Tests 2001 -> 2004 passing (3 new memory_model
regression tests pinning M2 revival: bind form, loop form, and the
negative case).
