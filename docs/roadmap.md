# Roadmap — open work

The single list of known open work on Jinn. Items are grouped by subsystem and
carry a stable id so commits and changelog entries can name them. (This list
was rewritten and renumbered on 2026-08-14; changelog entries [160] and earlier
use the previous ids.)

Severity:

- **B — blocker.** Unsoundness, silent wrong answers, data loss, or a crash on
  plausible code. Must not ship in a release that calls itself usable.
- **M — major.** A promised surface that does not work, or works only on one
  narrow path.
- **m — minor.** Rough edge, ergonomics, or a gap that is cheap to live with.

Claims with numbers name the harness that measures them. An item marked
*unverified* is carried forward without a fresh reproduction — re-check before
acting on its details.

## Where the language stands

**Open: 0 blockers, 0 majors, 26 minors, 5 coverage gaps.** Every promised
surface now works on its main path; the minors are bounded residue; the
coverage gaps are missing evidence, not known defects.

- **Memory and ownership** — the core holds under attack. Move/borrow analysis
  is place-granular, closures and generators own their captures, `freeze`
  shares read-only data across tasks without copies, and views give zero-copy
  reads with no lifetime syntax. The whole executable corpus (510 programs)
  compiles and runs under ASan+LSan at `--opt 0` and `--opt 3` with **zero
  memory-corruption findings**, and an ownership-syntax fuzzer produces no ICE
  and no memory error. What remains is a measured leak tail (35 of 512
  programs, all in known classes) and inference ergonomics (`O-1`–`O-9`).
- **Types and diagnostics** — no open defects. Generics instantiate from
  annotations, expected types, and the `of` call form; diagnostics never leak
  compiler internals (`tests/diagnostic_hygiene.rs` gates it).
- **Errors and capabilities** — both effect rows are inferred over call-graph
  SCCs, and annotations (`! E`, `needs`) are checked upper bounds. The
  remaining over-approximations can only reject valid programs, never accept
  invalid ones (`E-1`–`E-3`).
- **Persistent store** — transactions, WAL recovery, compaction, schema
  migration, and secondary indexes are implemented, each with a dedicated
  suite. Query blocks group and aggregate (`group`/`select`), relations
  traverse in both directions with transitive `@cascade` deletes, and
  durability is a per-store decorator (`@durable`/`@relaxed`/`@volatile`).
- **Concurrency** — tasks, channels, actors, `together` scopes, and
  cancellation are implemented and specified, and every context switch carries
  ASan/TSan fiber annotations. Open: cancellation edge cases and the dormant
  `supervisor` (`N-4`).
- **Tooling** — `fmt` is corpus-gated and refuses to write output that stops
  parsing; the LSP runs the full frontend per edit (type diagnostics,
  inferred-type hover, scope-aware rename); `jinn bind` is a stated
  approximation; `.jni` interface reuse is off by default.
- **Packages** — design only ([`design/lamp.md`](design/lamp.md)); the
  compiler machinery it assumes is
  [`design/compiler-prereqs.md`](design/compiler-prereqs.md).
- **Coverage** — five surfaces have never been meaningfully exercised
  (`V-1`–`V-5`); their silence is not evidence.

---

## Memory and ownership

The single-owner dataflow core — move-on-assign, branch and loop dataflow,
consuming-parameter inference, task isolation, drop discipline — is
place-granular: moves and borrows live in one place lattice
(`src/typer/place.rs`) with overlap and disjointness queries, iteration
borrows cover field places and map iteration, and call-site exclusivity checks
argument places. Drop placement is proven at the MIR level: a must-hold
dataflow inserts the drops the typer omits, and `src/drops/verify.rs` fails
the compile if a must-held container allocation reaches a `return`.

Pinned by `tests/memory_model.rs`, `tests/place_ownership.rs`,
`tests/views.rs`, `tests/freeze.rs`, and `tests/closure_captures.rs`.
Measured by `ci/sanitize-corpus.sh` (the executable corpus — `tests/programs`,
`apps/`, `snippets/`, 510 programs — under ASan+LSan at `--opt 0` and
`--opt 3`: zero corruption; leaks report but do not gate unless
`JINN_SAN_STRICT=1`) and `ci/fuzz-ownership.py` (ownership-syntax mutants
either reject with a diagnostic or run memory-safe).

### O-1 (m) Conditional consumption is path-insensitive

The move analysis does not split paths: a value consumed on one branch counts
as consumed on all of them. Both directions show it.

- **Over-rejection.** A call that dynamically never consumes is still
  rejected. The diagnostic is honest — it names the consuming site in the
  callee (`it is consumed at file:line:col`) and says when that site sits on a
  conditional path — but the program does not compile.
- **Bounded leak.** A value consumed on only one branch is excluded from
  scope-end drops entirely, so it leaks on the branch that did not consume it.
  The drop-repair pass inserts only must-hold drops deliberately — an
  unconditional drop would double-free the consumed path. The same class
  covers match: consumption in one arm suppresses the subject's scope-end drop
  even when a different arm runs, an arm that `return`s skips the after-match
  drop of its subject temp, and a payload bind whose *field* (rather than the
  bind whole) was moved out is excluded from arm-end drops.

Path-splitting for the common `if`/`return` shape closes all of these at once;
until then the leak side is bounded and measured by `ci/sanitize-corpus.sh`.

### O-2 (m) Drop repair covers containers; a temp leak tail remains

Drop obligations are first-class at the MIR level for container allocations
(`vec_new`, `map_init`, container-typed `clone` and known-function call
results): the must-hold dataflow tracks ownership-precise transfer edges
(consuming slots, container inserts, stores, sends, captures, returns),
inserted drops land after inlined `defer` bodies, temporary `match` subjects
materialize as hidden locals so the ordinary machinery drops them, and a
`?`/`!!` subject temp drops at the desugar block's end when the result type is
trivially droppable (a heap-bearing result may alias the payload).

Vec-returning store calls (`all`, relation traversal, `distinct`, group
queries) are classified as owning allocations
(`src/drops/verify.rs::is_store_vec_alloc`), so their expression-position
temps drop on straight-line paths; a new vec-returning store call must be
added to that list or its temps leak. Still outside the covered set: `String`
temps in expressions, method-call results and subjects on early-return paths
(`O-1`'s class), per-iteration reallocation in loops (including closure
environments), recursive-enum tree temps, and quaternary subjects with
heap-bearing results. Measured surface: 35 of 512 corpus programs leak under
`ci/sanitize-corpus.sh` — the count *rose* from 24 when the runtime gained
sanitizer fiber annotations, because actor-heavy programs that previously
died in spurious ASan SEGVs (masking any leak report) now run to completion
and report honestly.

### O-3 (m) Consuming/mutating inference name-buckets unknown receivers

Both inference scans resolve receiver types from single-static AST facts —
parameter annotations, constructor binds, `self` and its fields — and consult
that type's own method table. A receiver that is unknowable at scan time (a
local bound from a call result, an element read, a rebound name) joins a
name-bucket instead: `x.m()` marks by every method named `m`. Full resolution
needs types at scan time, which means moving the fixpoint after inference.

### O-4 (m) Place granularity stops at dynamic indices and parallel blocks

Element indices are compared only when both are integer literals — any dynamic
index conservatively overlaps — and `sim for`/`together` blocks keep coarser
capture rules than the place lattice used elsewhere.

### O-5 (m) Category transitions warn only at an asserted definition

`type Point @value` / `type Bag @aggregate` pin a struct's ownership category,
and a definition whose fields contradict the assertion is a compile error
naming the field that flips it
(`src/typer/resolve.rs::check_category_assertion`). Unasserted types still
transition silently when a field changes category several embeddings away; the
embedding-site diagnostic ("this change makes `Config` an aggregate:
assignments now move") needs a cross-compile baseline.

### O-6 (m) Boundary ownership explicitness is advisory, not policy

`.jni` interfaces carry per-parameter `consumes`/`mutates` bits populated from
the inferred tables, and a `--lib` compile warns on every exported function
whose parameter consumes *by inference*, naming the escape site and suggesting
the explicit `v as take ...` spelling. Making explicitness *required* at
package boundaries, and having `fmt` insert the annotation, are policy
decisions deferred until the package/visibility surface exists
([`design/lamp.md`](design/lamp.md)); `fmt` insertion additionally needs
inference results at format time.

### O-7 (m) Closures and generators: suspended frames, loop temps, caps aliasing

Closures own their captures — scalars copy, `String`s clone into the
environment, aggregates move; views, `@resource` values, and borrowed
parameters are rejected — the environment is heap-allocated with its own drop
function, closures move into at most one task, calls through function-typed
parameters taint the caps fixpoint, and a generator's aggregate arguments are
inferred consuming ([`design/closure-captures.md`](design/closure-captures.md)).
Open:

- a generator dropped mid-iteration frees its frame but not the aggregates it
  still holds (suspended-frame drops);
- per-iteration closure temporaries in loops leak their environments (`O-2`'s
  loop class — the return-repair pass cannot reach them by design);
- a local that aliases a function-typed parameter is invisible to the caps
  taint;
- by-view capture for provably in-frame closures (the design's step 4);
- `copy x` at the capture site is spelled "bind `copy x` to a fresh name
  first" rather than inline.

### O-8 (m) `freeze`: sharing scopes and actor sends

`freeze` and `Frozen of T` work — structural freezability, compile-time
rejection of every write, and every `dispatch` inside a `together` sharing one
frozen value bound outside it (no move, no copy, no refcount), with a
`FrozenShare` borrow locking the owner until the join
([`design/freeze.md`](design/freeze.md)). Open:

- a frozen value created *inside* the `together` body falls back to
  single-task moves (its drop would race the join);
- function-exit `dispatch` sharing, the design's second scope supplier;
- actor sends of frozen values are conservatively rejected with guidance
  unless the handler declares `Frozen of ...` — accepting them for provably
  read-only handlers needs handler write-inference;
- the docs pattern for model-weight sharing beyond the tour's `together`
  example.

### O-9 (m) std still carries scalar-bounded byte loops

The span recipe — `View of u8` parameters, `.byte_count` bounds, byte-wise
comparison — is applied in `strings`, `csv`, `json`, `sort`, `url`, `uuid`,
and `bytes`, benchmark-gated. `toml`, `http`, `regex`, `date`, `path`, `args`,
and friends still run `.length`-bounded scalar byte loops: correct on ASCII,
quadratic beyond it. Convert them with the same recipe when they matter.

---

## Types, errors, and effects

Nothing is open in the type system itself. Two deliberate limits are
documented in the tour rather than carried as items: angle-bracket type
arguments are annotation-only (the `of` call form is the expression spelling),
and map keys are strings until the runtime grows typed keys.

Both effect rows — error sets (`src/typer/errset.rs`) and capabilities
(`src/typer/caps.rs`) — are inferred bottom-up over call-graph SCCs as least
fixpoints; `! E` and `needs` annotations are compiler-checked upper bounds,
never the source of truth. Capabilities classify at extern leaves
(`src/cap_sites.rs`; an unclassified extern, `syscall`, or `asm` block derives
`ffi.unsafe`), store operations derive path-scoped `fs` capabilities, and
actor handlers are scan roots whose rows join at send and spawn sites. The
residue:

### E-1 (m) `From` conversion resolves by name pattern

Error-type conversion is recognized by a name-pattern lookup
(`src/typer/errset.rs` checks for a function named `<Target>_from_<Source>`)
rather than through trait-impl resolution. Error-row diagnostics at generic
instantiation sites are rough (*unverified* — carried forward).

### E-2 (m) Capability method edges are name-buckets; `.jni` imports are opaque

`x.m()` joins every user method named `m` into the capability row, and a send
joins handler rows through the same bucket — an over-approximation that can
only produce false rejections for `needs`-annotated functions, never false
acceptance. Modules imported through `.jni` interface files have no bodies to
scan (interface reuse is off by default — `X-4`). One genuine
under-approximation: relation *traversal* (`row.owner`, `row.children`) reads
the target store without deriving its `fs` capability — the caps scan is
AST-level and cannot see that a field access is a store read. Reaching it
requires a `Row` value, which almost always means a store operation already
tainted the function; the exception (a row passed as a parameter into a
`needs`-annotated function) can falsely accept.

### E-3 (m) Capability ceilings and manifests are design only

Module and project capability ceilings, and the manifest surface, remain
design ([`design/compiler-prereqs.md`](design/compiler-prereqs.md)).

---

## Persistent store

Transactions commit and roll back for real — `tests/store_transactions.rs`
pins commit, rollback on an escaping error, rollback of `set`/`delete`,
secondary-index restore, nested blocks joining the outermost, trap-rollback,
survival across a restart, and per-task isolation. Compaction, schema
fingerprinting with migration enforcement, persistent secondary indexes, and
WAL recovery are implemented and gated. Query blocks group with aggregate
`select` projections (`tests/integration.rs` pins the surface and its
diagnostics), relations traverse both ways with transitive `@cascade`
deletes (`tests/store_relations.rs`), and per-store durability decorators
override the process default (`JINN_WAL_SYNC` is a testing override).

### S-1 (m) Transaction rollback has a crash window

Rollback is ordered for safety — WAL truncated first, then the data file
restored atomically — but a crash exactly between the two steps leaves the
last transaction's writes in the data file. Structurally intact, not torn, and
documented in the language tour. Closing it needs the two steps to become one.

### S-5 (m) Filters are one flat and/or chain

Both filter paths accept `field in [..]` combined with `and` (the desugared
Or-chain is hoisted to the head of the filter so the left-to-right fold stays
correct) and method-form text predicates with ASCII case-insensitive
variants. What a flat chain cannot express remains inexpressible: a second
`in [..]`, `in` combined with `or`, and real parenthesized grouping all
reject with a diagnostic — supporting them needs a grouped predicate encoding
through MIR's name-encoded call scheme. Group queries aggregate row-scans;
the `@column` fast path only covers whole-store `sum`/`min`/`max` on integer
fields.

### S-6 (m) Each transaction snapshots the whole store file

`jinn_txn_track_impl` copies the entire data file, bounded by
`JINN_TXN_SNAPSHOT_MAX` (256 MB default) and then `abort()`. The diagnostic
names the knob and the workaround, so it is not silent, but the cost is
O(store size) per transaction and has never been measured against a realistic
store.

### S-7 (m) A WAL with bad magic terminates the process

A clear message and `exit(2)` — no core dump, but still process termination.
Surfacing it as a typed `StoreError` needs a fallible store-open surface,
which does not exist; the store-open codegen path assumes success.

---

## Tooling

### X-1 (m) `jinn fmt`: apps are ungated and expression fallbacks remain

The printer covers the real grammar — extern, actor, store (decorators,
relations, methods, filters), query blocks, select, dispatch, generic `of`
clauses, `! E` rows, bind access modifiers, layout attributes, `%`/`@` pointer
forms, `nop` bodies, precedence parenthesization — and the gate
(`fmt_output_still_frontend_checks_over_corpus`) formats every file in
`snippets/`, `tests/programs/`, `benchmarks/`, and `std/` (574 files), failing
if any file that frontend-checked before formatting stops doing so. `--write`
refuses to write output that no longer parses, so a printer regression
degrades to a refusal instead of silent damage.

Open: multi-module `apps/` are not in the gate (a per-file frontend check
needs the project context), and `format_expr` keeps `...`/`do ... end`
fallbacks for expression forms that never appear in expression position in the
corpus. The long-term shape — lossless tree, lint engine,
behaviour-preservation verifier — is
[`design/fmt-and-lint.md`](design/fmt-and-lint.md).

### X-3 (m) `jinn bind` is a textual approximation of C

It generates parseable Jinn from real system headers and states what it
skipped, but it is not a C parser: anything it cannot represent is dropped
with a stated reason rather than translated.

### X-4 (m) `.jni` interface reuse has no safe-and-useful configuration

Reading `.jni` files is off by default because a stale-but-newer file made the
compiler accept a type-incorrect program. Either remove the feature or rebuild
it as interface v2 with the hash ladder in
[`design/compiler-prereqs.md`](design/compiler-prereqs.md).

### X-5 (m) LSP residue: packages, cross-file rename, generics

The LSP runs the full frontend per edit — type diagnostics with positions,
inferred-type hover, `DefId`-resolved definition, and scope-aware rename, all
inside a panic-contained 256 MiB worker thread (`src/lsp/typed.rs`;
`tests/lsp_smoke.rs` pins shadow-correct rename and positioned type errors).
Residue: package imports resolve with an empty package set (a file whose
imports cannot resolve degrades to parse-level analysis with a warning);
cross-file rename and references are still lexical; monomorphized generic
functions lose hover/rename (mangled names do not match source spelling); and
the workspace index populates lazily as files open.

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
deterministic. *Unverified* — re-measure before acting on it.

### N-4 (m) `supervisor` is parsed but dormant

It should be specified as sugar over `together`: a long-lived scope whose
error handler restarts children per strategy instead of re-raising.

---

## Performance and build

### P-1 (m) `sim for` lowers to a sequential loop

`hir::Stmt::SimFor` lowers in `src/mir/lower/loops.rs` to an ordinary counted
loop — a compare and a branch, no spawn and no scheduler involvement. It
measures neither parallelism nor the work it appears to describe, and any
benchmark row derived from it is not comparable across languages and must not
be quoted as a ratio.

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
