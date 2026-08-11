# Roadmap — open work

The single list of known, unfinished work on Jinn. Items are grouped by
subsystem and carry a stable id so commits and changelog entries can name them.

Severity:

- **B — blocker.** Unsoundness, silent wrong answers, data loss, or a crash on
  plausible code. Must not ship in a release that calls itself usable.
- **M — major.** A promised surface that does not work, or works only on one
  narrow path.
- **m — minor.** Rough edge, ergonomics, or a gap that is cheap to live with.

Every item marked *verified* was reproduced against `target/release/jinnc` on
2026-08-08, and the reproduction is given inline so it can be re-run. Items
without the marker are design work, coverage gaps, or observations carried
forward and not re-checked in that pass — check before acting on the details.

The 2026-08-10 remediation pass ([143]) closed M-1, M-2, M-3, M-4, M-5, T-1,
T-3, T-4, T-5, T-6, T-7, T-8, T-9, T-11, T-12, T-14, C-2, C-3, S-4, S-6, and
X-2, and reduced M-8, T-2, and S-9; the closed items' reproductions now live in
the test suite and CHANGELOG entry [143]. Items below are what remains.

---

## Memory and ownership

The single-owner dataflow core — move-on-assign, branch and loop dataflow,
consuming-parameter inference, task isolation, drop discipline — holds up under
attack. [143] closed the worst of the projection/aliasing seams at *variable*
granularity: constructor expressions tombstone their aggregate sources (old
M-1), one call site may not consume the same variable twice or consume it and
read it again (old M-2), a variable may not be passed twice to one call when a
parameter mutates it (old M-3, backed by a new parameter-mutation inference
fixpoint), mutating a temporary copy of a container element is a compile error
(old M-4), and `for x in v` takes an iteration borrow of `v` that rejects
mutating calls, moves, and mutation-through-calls in the body (old M-5). What
remains below is *place* granularity — fields, elements, overlapping
projections — which is `M-6`, plus the residuals it subsumes.

### M-6 (B) Make the analysis place-based rather than variable-based

Unify `moved_vars`, `moved_fields`, and the element rules into one *place*
lattice (`root.field.elem…`) with overlap and disjointness queries. The [143]
checks are variable-granular: `f(s.a, s.a)` aliasing, moves of
fields-inside-constructors, iteration borrows of `s.field` or map/`Iter`-trait
desugared loops, and element-place overlap all pass unchecked today. Every
residual falls out of place granularity. This is the highest-leverage refactor
in the compiler.

### M-4r (m) Element reads still deep-copy

The silent lost update is now a compile error, but every nested-container read
in expression position still pays a hidden O(n) deep copy, and a *read-only*
method call on an element read still operates on a copy without a diagnostic.
The honest fix is `M-13`'s second-class borrows.

### M-5r (m) Iteration borrows cover only `for x in <var>`

Map iteration (`for k, v in m`), `Iter`-trait desugared loops, and iteration
over a field place (`for x in s.items`) do not register an iteration borrow;
mutation during those loops is still accepted. Falls out of `M-6`.

### M-7 (m) Conditional consumption over-tombstones and the diagnostic hides why

A callee that consumes a parameter on one branch is inferred unconditionally
consuming, so a call that dynamically never consumes is still rejected. Sound
but imprecise, and the diagnostic ("whose parameter takes ownership") does not
say the consumption was conditional. Minimum bar: the diagnostic names the
consuming path. Better: path-splitting for the common `if`/`return` shape.

### M-8 (m) Consuming-method inference over-approximates by name for unknown receivers

[143] closed the unsound half: user methods now run through the same
body-derived escape scan as free functions (fixpoint over methods and free
functions together), so a method that stores its argument is detected
regardless of its name. What remains is the imprecise half: the AST-level scan
in `consume_infer.rs` runs before types exist, so a call `x.set(v)` still
matches the builtin name list even when `x` will turn out to be a user type
whose `set` does not store — the enclosing function's parameter is then
over-inferred as consuming. Resolving this needs receiver types at scan time,
i.e. moving the scan after inference or into `M-6`'s place framework.

### M-9 (M) MIR drop-linearity verifier — *verified absent*

Nothing in `src/` checks that drop placement preserves "exactly one drop per owner" after
sinking, fusion, and reuse. A debug-build verifier asserting that every owner is
dropped exactly once on every path — re-run after each Perceus transform —
turns silent corruption into an ICE.

### M-10 (M) Whole-corpus sanitizer sweep

`ci/sanitize.sh` runs nine targeted programs. Every conformance test and every
`apps/` program should run under ASan+LSan at `--opt 0` and `--opt 3`, plus a
fuzzer that mutates ownership-relevant syntax. The constructor-move double-free
class ([143], old M-1) is now compile-rejected, but the sweep is what would
catch the next seam this list has not named.

### M-11 (m) Category transitions are silent

A type's category is inferred from its field types, transitively, so adding a
`Vec` field to a leaf struct silently changes assignment semantics for every
struct that embeds it, several levels up. Wanted: a diagnostic on category
transition ("this change makes `Config` an aggregate: assignments now move"),
and `@value` / `@aggregate` assertions so a library author can pin the category.

### M-12 (M) Ownership at public boundaries

A call site's meaning (move vs borrow) depends on the callee's body,
transitively. That is fine inside a module and corrosive across a package
boundary: editing a library function's *body* can break downstream callers with
no signature change. Exported functions should be required to state
`take`/`copy` explicitly (compiler suggests it, `fmt` inserts it), and inferred
consumingness must enter the interface hash either way.

### M-13 (M) Second-class references and slices

There is no way to express a zero-copy sub-slice, a lending iterator, or a
returned view into an argument. Borrows usable in parameter position,
expression position, and yield-accessors — but never storable — keep the
"no lifetime syntax" property while unlocking all three, and give `M-4` its
honest fix.

### M-14 (M) `freeze` and shared immutables

Large read-only data shared across tasks (config, model weights, tables) must
today be copied per task or funnelled through one actor. A one-way transition
to a deeply-immutable value that may be shared across tasks without copying
needs no refcount if frozen values are scope-bounded — and `together` already
supplies the scope.

### M-15 (m) Arena / generational-index type in `std`

Doubly-linked lists, parent pointers, and graphs are inexpressible by design.
The blessed replacement pattern should ship as a documented `std` type rather
than be reinvented per project.

### M-16 (M) Closure and generator capture rules are unaudited

Closures (`f is *() …`) do not parse, so no capture rule could be tested. When
they land they inherit the whole cross-task capture problem and must be
specified before implementation, not after.

---

## Types, inference, and diagnostics

[143] closed the verified defects here: generic-type methods now emit bodies
(old T-1), undefined names are rejected in the typer with a span and
`--emit-hir` exits non-zero on them (old T-3), a bare `! E` signature means
`Result of Unit, E` and a valued tail is a typer diagnostic instead of an
inkwell panic (old T-4), trait method signatures parse `! E` (old T-5),
duplicate catch-all clauses are a parse error naming both sites (old T-6),
unsolved type variables warn by default and are errors under `--strict-types`
(old T-7), out-of-range integer literals against an annotation warn with the
wrapped value (old T-8), ALL_CAPS constants cannot be shadowed (old T-9), flat
operator chains no longer overflow the stack (old T-11, 256 MiB compile
thread), a UTF-8 BOM is stripped and non-ASCII lex errors are decoded before
display (old T-12), and int→float coercion works in bind annotations and for
literals in binary operands, with mismatch diagnostics naming expected/found in
the caller's order (old T-14).

### T-1r (M) Generic types: non-unifying spellings, phantom and return-only parameters

Method bodies on generic types are emitted since [143], but a generic struct
still has several mutually non-unifying spellings, and return-position-only and
phantom type parameters cannot be used, with no turbofish to escape.

### T-2 (m) Mangled internal names can reach user diagnostics

The old T-1 instance is gone and the [143] ownership diagnostics strip the
`__G_` specialization suffix before printing, but no sweep asserts that *no*
diagnostic prints a mangled symbol; codegen-level errors still name raw
symbols.

### T-5r (m) Trait/impl signature conformance is unchecked

Trait methods parse and declare `! E` since [143], but an impl may still
silently widen (or narrow) the error row past the trait's declaration — impl
conformance checks only method presence, not signatures.

### T-10 (M) Control-flow merges of differently-shaped values are unrepresentable — *verified*

`tests/programs/compiler_pipeline.jn` does not compile: a recursive enum payload
rebound inside a loop produces a merge whose incoming values have different
shapes (array on one path, tuple on another). This is a diagnostic naming the
function and the construct, not an ICE, and the harness asserts that exact
diagnostic — so it reports both a regression to a panic and a fix.

### T-13 (m) Safe code can construct invalid-UTF-8 Strings — *verified*

`chr(200)` yields a one-byte String holding `0xC8`, which is not valid UTF-8,
while `String` is documented as guaranteed valid UTF-8. Unresolved because
`docs/strings.md` *also* documents `chr` as the byte-level inverse of indexing,
and `std/url.jn` percent-decoding and `std/uuid.jn` byte assembly depend on
byte semantics — closing this needs a decision (UTF-8-encoding `chr` plus a
separate byte builtin, or renaming the contract), not just a fix.

---

## Effects and capabilities

### C-1 (M) Capability inference is inert — *verified*

```jinn
use io

*sneaky() returns i64 needs pure
    io.write_file('canary.txt', 'written')
    0
```

Compiles, runs, and writes the file. `src/cap_sites.rs` keys every entry on
`std.net.connect`, `std.fs.read_file`, … — none of which are callable names in
this language, so nothing ever matches. `grep -rn "needs " ` over the whole
corpus finds no real annotation, so nothing depends on the current behaviour.

Capabilities are documented as **design, not implemented** in
[`design/compiler-prereqs.md`](design/compiler-prereqs.md). Closing this item
means making the pass real: key the table on callable names, and check `needs`
as an upper bound on the inferred set.

[143] gated `embed` to the source directory — absolute paths are rejected and
the canonicalized target must stay inside the canonicalized source dir (old
C-2) — and rebuilt the comptime purity classifier as a least fixpoint that
consults callees, so a call is pure only if the named function has itself been
proven pure (old C-3).

---

## Persistent store

The store surface is in better shape than its reputation. Transactions commit
and roll back for real (`tests/store_transactions.rs` pins commit, rollback on
an escaping error, rollback of `set`/`delete`, secondary-index restore, nested
blocks joining the outermost, trap-rollback, survival across a restart, and
per-task isolation). Compaction, schema fingerprinting with migration
enforcement, persistent secondary indexes, and WAL recovery are all implemented
and gated. What follows is the residue.

### S-1 (m) Transaction rollback has a crash window

Rollback is ordered for safety — WAL truncated first, then the data file
restored atomically — but a crash exactly between the two steps leaves the last
transaction's writes in the data file. Structurally intact, not torn, and
documented in the language tour. Closing it needs the two steps to become one.

### S-2 (M) No grouping or aggregate combinations in query blocks

Aggregations are single-shot whole-store methods; query blocks filter and sort
but cannot group. "Average age per city" requires a manual loop. Wanted: a
`group <field>` clause with aggregate expressions in `select`. The columnar
runtime is already positioned to make this fast for `@column` stores.

### S-3 (M) `has-many` relations are declared but not traversable

`&owner as Owner` resolves — belongs-to traversal reads through the foreign sid,
and `tests/store_relations.rs` pins it. `&items as [Item]` does not:
`has_many_is_not_directly_traversable` pins the *absence*. Cross-store queries
are likewise unavailable. Either finish the traversal and give `@cascade` real
`delete`/`destroy` semantics on the has-many side, or remove that half of the
syntax — as it stands it parses and does nothing.


### S-5 (m) Durability policy is still a process-global env var

`@transient` gives a per-store relaxed-persistence dial, but the fsync policy
itself is `JINN_WAL_SYNC`, read once at first open. A program mixing a critical
ledger with a throwaway cache gets one policy for both, set outside the source.
Wanted: per-store `@durable` / `@relaxed` / `@volatile`, with the env var
demoted to a testing override.

### S-7 (m) `where` has no `in` in query blocks and no case-insensitive match

The statement-level filter parser supports `in [..]` (desugared to an `Eq`
chain), `between`, `contains`, `starts_with`, `ends_with`, and grouping — the
old claim that `in` was missing is stale for that path. Still missing:
`in [..]` inside *query blocks* (`flatten_filter_expr` accepts only
comparisons), and case-insensitive comparison anywhere, despite `@search`
existing.

### S-8 (m) Each transaction snapshots the whole store file — *verified*

`jinn_txn_track_impl` copies the entire data file, bounded by
`JINN_TXN_SNAPSHOT_MAX` (256 MB default) and then `abort()`. The diagnostic names
the knob and the workaround, so it is not silent, but the cost is O(store size)
per transaction and has never been measured against a realistic store.

### S-9 (m) A WAL with bad magic terminates the process

[143] downgraded the `abort()` to a clear message plus `exit(2)` — no more
SIGABRT/core dump — but it is still process termination. Surfacing it as a
typed `StoreError` needs a fallible store-open surface, which does not exist;
the store-open codegen path assumes success.

## Tooling

### X-1 (m) `jinn fmt`: apps are ungated and expression fallbacks remain

[144] rebuilt the printer against the real grammar — extern, actor, store
(decorators, field decorators, relations, methods, filters), query blocks,
select, dispatch, generic `of` clauses on types/enums/functions, `! E` rows,
bind access modifiers and annotations, layout attributes, `%`/`@`
pointer forms, float literals, `nop` bodies, and precedence parenthesization —
and the gate now asserts what X-1 demanded:
`fmt_output_still_frontend_checks_over_corpus` formats every file in
`snippets/`, `tests/programs/`, `benchmarks/`, and `std/` (574 files) and
fails if any file that frontend-checked before formatting stops doing so.
`--write` additionally refuses to write output that no longer parses, so a
future printer regression degrades to a refusal instead of silent damage.

Residue: multi-module `apps/` are not in the gate (a per-file frontend check
needs the project context); `format_expr` still has `...`/`do ... end`
fallbacks for expression forms that never appear in expression position in the
corpus (`asm`, block expressions) — the corpus proves their absence, the
`--write` guard covers their appearance; and the design in
[`design/fmt-and-lint.md`](design/fmt-and-lint.md) remains the long-term shape.

### X-3 (M) The LSP has no type-aware analysis

Hover, definition, and rename use the parser only, so shadowed identifiers are
one symbol and rename is a lexical find-and-replace. Diagnostics are parse-level
only — type errors never reach the editor.

### X-4 (m) `jinn bind` is a textual approximation of C

It generates parseable Jinn from real system headers and states what it skipped,
but it is not a C parser: anything it cannot represent is dropped with a stated
reason rather than translated.

### X-5 (m) `.jni` interface reuse has no safe-and-useful configuration

Reading `.jni` files is off by default because a stale-but-newer file made the
compiler accept a type-incorrect program. Either remove the feature or rebuild
it as interface v2 with the hash ladder in
[`design/compiler-prereqs.md`](design/compiler-prereqs.md).

---

## Concurrency

### N-1 (m) Block-scoped `defer`s are not materialised under cancellation

A `defer` whose cleanup code references values defined inside a loop is not
specially materialised at the synthesized cancel-cleanup block. Function-level
defers do run on cancellation.

### N-2 (m) A parked coroutine parent relies on the direct wake from cancel

The join-loop re-wake only runs when the parent is `*main`. Neither this nor
`N-1` affects the common `together`/`dispatch`/`stop` shape.

### N-3 (m) Cancellation latency is highly variable

Previously measured at 13.7 s–30.0 s for a workload that should be
deterministic. Not re-measured; re-measure before acting on it.

### N-4 (m) `supervisor` is parsed but dormant

It should be specified as sugar over `together`: a long-lived scope whose error
handler restarts children per strategy instead of re-raising.

### N-5 (m) Whole-program TSan is not yet meaningful

Coroutines migrate between OS threads, so TSan needs `__tsan_switch_to_fiber`
annotations at the context-switch points before its results can be trusted.

---

## Performance and build

### P-1 (m) `sim for` lowers to a sequential loop — *verified*

`hir::Stmt::SimFor` lowers in `src/mir/lower/loops.rs` to an ordinary counted
loop — `simfor.cond`/`body`/`inc`/`exit` with a compare and a branch, no spawn
and no scheduler involvement. It therefore measures neither parallelism nor the
work it appears to describe, and any benchmark row derived from it is not
comparable across languages and must not be quoted as a ratio.

### P-2 (m) There is no incremental compilation

The bar a real design must clear is recorded in
[`internals.md`](internals.md#incremental-compilation).

---

## Coverage gaps

Nothing here is a known defect — these are surfaces no test or review has ever
exercised, listed so their silence is not mistaken for evidence.

- **V-1** Store subsystems whose only coverage is a smoke case in
  `tests/integration.rs`: `@vector` nearest-neighbour, `@versioned` history,
  `@kv`, `@column`, `@graph`, `@timeseries`, `@bloom`, `@search`. Compaction,
  schema fingerprinting, index persistence, relations, transactions, and
  recovery each have a dedicated suite; these do not.
- **V-2** Compiler flags `--lto`, `--target` / `--cpu` / `--features`,
  `--fast-math`, `--deterministic-fp`, `--standalone`. `--opt 2` has only been
  spot-checked.
- **V-3** Package commands beyond `fetch`; `jinn test`'s harness output.
- **V-4** Device-level durability. All crash testing has run on tmpfs, where
  `fsync` is near a no-op, so the sweeps validate application-level crash
  consistency and say nothing about power loss or write-cache loss.
- **V-5** The trait system beyond the surface `tests/traits.rs` covers, and
  `libjn/` (stub bodies, not part of `std` or the runtime).
