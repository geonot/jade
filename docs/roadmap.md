# Roadmap — open work

The single list of known open work on Jinn. Items are grouped by subsystem and
carry a stable id so commits and changelog entries can name them. (This list
was rewritten and renumbered on 2026-08-14; changelog entries [160] and earlier
use the previous ids. The 2026-08-16 pass ([163]) closed every blocker and
major from the 2026-08-15 review; closed ids are retired, not reused.)

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

**Open: 0 blockers, 0 majors, 39 minors, 5 coverage gaps.** The 2026-08-16
pass ([163]) closed all seven blockers and all twelve majors the [162]
adversarial review opened, each with a pinning test. The closures are honest
about their residue: what remains of each closed item is filed below as a
minor with its own id.

- **Memory and ownership** — the [162] accept-then-corrupt holes are closed:
  consuming calls in condition/scrutinee/iterator position are move-tracked,
  multi-level projection binds reject instead of aliasing, read-only
  enforcement (freeze and views) follows the place root and mutation
  inference walks nested receivers, and unannotated borrowed params no
  longer mint second owners through the monomorphization path. The leak
  tail and inference ergonomics remain (`O-1`–`O-10`).
- **Types and diagnostics** — alias-typed arguments reject at the call site,
  annotated method bodies are checked per instantiation, bare `! E`
  functions Ok-wrap their implicit unit exit, numeric method return types
  infer (and `min`/`max`/`is_nan`/`signum`/`recip`/`to_int` gained real
  lowerings), and capturing lambdas stay monomorphic instead of losing
  their captures. Diagnostics never leak compiler internals
  (`tests/diagnostic_hygiene.rs` gates it).
- **Errors and capabilities** — the function-value false-accept class is
  closed at its worst points: a call through a field or element callee, or
  through a local bound from a field/element/call result, now taints the
  row as an indirect call (`needs` rejects it). Residue in `E-2`.
- **Persistent store** — transactions on one store are serialized across
  tasks (rollback can no longer erase another task's committed writes),
  `save` syncs the data file before checkpointing the WAL, `@relaxed`
  actually batches syncs, all sidecars (`@kv`, `@versioned`, `@vector`,
  `@bloom`, `.idx`, `.fts`, `.col`) roll back with the transaction,
  recovery preserves the WAL when replay is incomplete, and store open
  never truncates an existing store on a transient error. Failure-path
  minors remain (`S-1`–`S-4`, `S-9`–`S-12`).
- **Concurrency** — unscoped `dispatch` is rejected instead of silently
  never running, `stop <scope>` works from child tasks, cancellation
  propagates into nested scopes, loop back-edges are cancellation points in
  every task (not just those with a `defer`), `return` inside `together`
  is rejected, and the `!`-arm ICE is a diagnostic. Sleeps and IO parks are
  still not cancellation points (`N-6r`), and the synchronous actor-call
  protocol does not exist (`N-9`).
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
argument places. Since [163], condition, scrutinee, and iterator positions
record moves like every other position, and generic-function instantiation
assigns parameter ownership through the same rules as the direct path. Drop
placement is proven at the MIR level: a must-hold dataflow inserts the drops
the typer omits, and `src/drops/verify.rs` fails the compile if a must-held
container allocation reaches a `return`.

Pinned by `tests/memory_model.rs`, `tests/place_ownership.rs`,
`tests/views.rs`, `tests/freeze.rs`, and `tests/closure_captures.rs`.
Measured by `ci/sanitize-corpus.sh` (the executable corpus under ASan+LSan at
`--opt 0` and `--opt 3`) and `ci/fuzz-ownership.py`.

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
heap-bearing results. Measured surface: 34 of 514 corpus programs leak under
`ci/sanitize-corpus.sh` (re-measured after [163]; zero corruption).

### O-3 (m) Consuming/mutating inference name-buckets unknown receivers

Both inference scans resolve receiver types from single-static AST facts —
parameter annotations, constructor binds, `self` and its fields — and consult
that type's own method table. A receiver that is unknowable at scan time (a
local bound from a call result, an element read, a rebound name) joins a
name-bucket instead: `x.m()` marks by every method named `m`. Since [163]
receivers whose single static annotation is a builtin container (`Vec`,
`Map`, array, `string`) are exempt from the user-method bucket (only the
builtin mutating table applies), which keeps a user type's mutating `get`
from poisoning every `vec.get` in the program. Full resolution needs types at
scan time, which means moving the fixpoint after inference.

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
- by-view capture for provably in-frame closures (the design's step 4);
- `copy x` at the capture site is spelled "bind `copy x` to a fresh name
  first" rather than inline.

### O-8 (m) `freeze`: sharing scopes and actor sends

`freeze` and `Frozen of T` work — structural freezability, compile-time
rejection of every write (including nested-place writes since [163]), and
every `dispatch` inside a `together` sharing one frozen value bound outside
it (no move, no copy, no refcount), with a `FrozenShare` borrow locking the
owner until the join ([`design/freeze.md`](design/freeze.md)). Open:

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

### O-10 (m) Capturing lambdas are monomorphic

The fix for the [162] capture-loss bug ([163]) keeps any lambda that
references an outer variable monomorphic: it goes down the real closure path
(captures work), but it can no longer be used polymorphically at two
differently-typed call sites. Non-capturing lambdas still generalize. Lifting
captures to parameters of the `__poly_` instantiation would restore
polymorphism for capturing lambdas.

---

## Types, errors, and effects

Two deliberate limits are documented in the tour rather than carried as
items: angle-bracket type arguments are annotation-only (the `of` call form
is the expression spelling), and map keys are strings until the runtime grows
typed keys.

### T-4 (m) Alias enforcement covers function arguments, not method arguments

[163] closed `T-2` for plain function calls: passing the underlying type
where an `alias` nominal is expected rejects at the call site (mirroring
return position). Method arguments (`x.m(1.5)` against an alias-annotated
method parameter) still run through paths with no post-hoc argument
unification and are unchecked. `Type::Alias` itself remains unconstructed in
the frontend — aliases are opaque `Type::Struct(name, [])` nominals tracked
by name (`Typer::alias_names`).

### E-1 (m) `From` conversion resolves by name pattern

Error-type conversion is recognized by a name-pattern lookup
(`src/typer/errset.rs` checks for a function named `<Target>_from_<Source>`)
rather than through trait-impl resolution, and the spec's orphan/coherence
rule (error-effects §6 C4) is not enforced at all — any global function with
the right name is a conversion. Error-row diagnostics at generic
instantiation sites are rough (*unverified* — carried forward).

### E-2 (m) Capability method edges are name-buckets; residue after the sound default

`x.m()` joins every user method named `m` into the capability row, and a send
joins handler rows through the same bucket — an over-approximation that can
only produce false rejections for `needs`-annotated functions. Since [163],
the genuine false-accept class is closed at its worst points: a call whose
callee expression is a field or element, and a call through a local bound
from a field, element, or call result, taint the row as an indirect call.
Remaining under-approximations: (1) relation *traversal* (`row.owner`,
`row.children`) reads the target store without deriving its `fs` capability —
the caps scan is AST-level and cannot see that a field access is a store
read; (2) a *method-form* call on an unresolvable receiver whose name matches
no user method contributes nothing (taint here would misfire on every builtin
container method name); (3) modules imported through `.jni` interface files
have no bodies to scan (interface reuse is off by default — `X-3`).

### E-3 (m) Capability ceilings and manifests are design only

Module and project capability ceilings, and the manifest surface, remain
design ([`design/compiler-prereqs.md`](design/compiler-prereqs.md)).

---

## Persistent store

Transactions commit and roll back for real — `tests/store_transactions.rs`
pins commit, rollback on an escaping error, rollback of `set`/`delete`,
secondary-index restore, nested blocks joining the outermost, trap-rollback,
survival across a restart, per-task isolation on separate stores, and (since
[163]) cross-task isolation on a *shared* store and sidecar rollback for
`@kv` and `@versioned`. A transaction's first mutation takes the store's
writer lock for the whole transaction, so another task's writes serialize
behind it instead of interleaving into the snapshot window. Compaction,
schema fingerprinting with migration enforcement, persistent secondary
indexes, WAL recovery, query blocks with aggregation, and relations with
transitive `@cascade` deletes are implemented and gated.

### S-1 (m) Transaction rollback has a crash window

Rollback is ordered for safety — WAL truncated first, then the data file
restored atomically — but a crash exactly between the two steps leaves the
last transaction's writes in the data file. Structurally intact, not torn, and
documented in the language tour. Closing it needs the two steps to become one.

### S-2 (m) Filters are one flat and/or chain

Both filter paths accept `field in [..]` combined with `and` (the desugared
Or-chain is hoisted to the head of the filter so the left-to-right fold stays
correct) and method-form text predicates with ASCII case-insensitive
variants. What a flat chain cannot express remains inexpressible: a second
`in [..]`, `in` combined with `or`, and real parenthesized grouping all
reject with a diagnostic — supporting them needs a grouped predicate encoding
through MIR's name-encoded call scheme. Group queries aggregate row-scans;
the `@column` fast path only covers whole-store `sum`/`min`/`max` on integer
fields.

### S-3 (m) Each transaction snapshots the whole store file

`jinn_txn_track_impl` copies the entire data file, bounded by
`JINN_TXN_SNAPSHOT_MAX` (256 MB default) and then `abort()`. The diagnostic
names the knob and the workaround, so it is not silent, but the cost is
O(store size) per transaction and has never been measured against a realistic
store. `@kv` and `@bloom` additionally snapshot their in-memory state per
transaction; `@versioned` and `@vector` record only a length.

### S-4 (m) A WAL with bad magic terminates the process

A clear message and `exit(2)` — no core dump, but still process termination.
The same failure mode now covers a store data file that cannot be opened or
created (`jinn_store_open_data`, [163]) — deliberate, to stop `EMFILE`
truncating a store, but still process exit. Surfacing these as a typed
`StoreError` needs a fallible store-open surface, which does not exist.

### S-9 (m) Store failure-path minors

The mechanical half of the 2026-08-15 audit landed in [163]: `@kv` key
truncation warns, kv persist failures report, kv open checks its
allocations, the WAL policy table warns when full, an unrecognized
`JINN_WAL_SYNC` value warns that it overrides decorators, `.idx` slot reads
are zero-initialized and checked, migration header reads are checked, and
rollback's reopen-failure path distinguishes "not rolled back" from "rolled
back on disk but this process's handle is stale". Still open: torn `.idx`
slots survive crashes (no checksums), aux sidecar files are `fflush`-only
(`@versioned` history is not power-loss durable even under `@durable`), and
a crash between a migration rewrite and the schema re-stamp leaves
fingerprint 0, which the next open silently adopts as the current schema.

### S-10 (m) Store reads take no lock and race handle swaps

Read paths (`count`, `all`, queries) load the store `FILE*` without taking
the writer lock. A concurrent rollback, compact, or migration swaps and
closes that handle (`jinn_atomic_rewrite_reopen`), so a reader that loaded
the old pointer can read through a freed `FILE*`. Writers are safe since
[163] (the lock is taken before the handle is loaded); readers need either
the same discipline or handle reclamation that defers the `fclose`.

### S-11 (m) Cancellation mid-transaction leaks the store writer lock

A task cancelled inside a `transaction` block never runs commit or rollback,
so the transaction-scoped writer lock ([163]) stays held and other tasks'
writes to that store block forever. Cancellation cleanup needs a txn-unwind
hook, or the lock needs an owner-death check.

### S-12 (m) Transactions on multiple stores can deadlock

Two tasks whose transactions acquire the same two stores' writer locks in
opposite orders spin forever (the lock backoff yields, so the scheduler
stays live, but neither task progresses). Lock ordering by path, or a
deadlock detector with a diagnostic, closes it.

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

### X-2 (m) `jinn bind` is a textual approximation of C

It generates parseable Jinn from real system headers and states what it
skipped, but it is not a C parser: anything it cannot represent is dropped
with a stated reason rather than translated.

### X-3 (m) `.jni` interface reuse has no safe-and-useful configuration

Reading `.jni` files is off by default because a stale-but-newer file made the
compiler accept a type-incorrect program. Either remove the feature or rebuild
it as interface v2 with the hash ladder in
[`design/compiler-prereqs.md`](design/compiler-prereqs.md).

### X-4 (m) LSP residue: packages, cross-file rename, generics

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
defers do run on cancellation (and since [163] every scheduler task has a
cancel-cleanup block, so defer-less tasks cancel too).

### N-2 (m) A parked coroutine parent relies on the direct wake from cancel

The join-loop re-wake only runs when the parent is `*main`. Neither this nor
`N-1` affects the common `together`/`dispatch`/`stop` shape.

### N-3 (m) Cancellation latency is highly variable

Previously measured at 13.7 s–30.0 s for a workload that should be
deterministic. *Unverified* — re-measure before acting on it; [163]'s
universal back-edge cancel checks may have changed it.

### N-4 (m) `supervisor` is parsed but dormant

It should be specified as sugar over `together`: a long-lived scope whose
error handler restarts children per strategy instead of re-raising. The
runtime half that exists races (`sup_on_child_exit` mutates restart state
from worker threads with no locking) and stops supervising silently after a
lifetime cap of 16 restarts.

### N-6r (m) Sleeps, IO parks, and joins are not cancellation points

The [162] cancellation holes are closed — `stop <scope>` works from child
tasks (the scope pointer rides the capture block), `jinn_scope_cancel`
recurses into nested child scopes, and loop back-edges check cancellation in
every scheduler task. Still not cancellation points: `sleep` (a raw
`nanosleep` on the worker thread), IO parks (`jinn_io_waiter_park`),
`jinn_actor_join`, and `jinn_scope_join_no_free` (a join-parked parent wakes
only via `child_done`, which cancellation of its children does trigger).
Routing `sleep` through a scheduler timer is the missing piece with the most
user-visible effect.

### N-9 (m) Synchronous actor calls do not exist

`*` (non-loop) handlers behave exactly like `@` handlers: the call is an
asynchronous send and produces no value. Since [163] this is honest instead
of silent: using a handler call where a value is expected (a bind or `log`
argument) is a compile error with guidance, `returns` on a handler is a
parse-time rejection, and the tour says so. A real synchronous call protocol
(send + park until reply) is the open work. Also open: a synchronous-read
story ordered with respect to earlier `@` messages, and fallible handlers
(a propagating body is a hard typer error).

### N-10r (m) Scheduler landmine residue

[163] fixed the stale-TLS reads in `jinn_coro_trampoline`/`jinn_coro_exit`/
`jinn_coro_yield`/`jinn_current_coro` (all now go through the `noinline`
accessor) and deleted the dead `jinn_actor_park`/`jinn_actor_wake` pair.
Remaining raw `tl_worker`-class reads live in `runtime/sched.c`'s own loop
(safe today — the loop never migrates) and `runtime/scope.c`'s
`jinn_scope_record_current_error`/`check_cancelled` (both `noinline`).
Re-audit if aarch64 misbehaves.

### N-11 (m) Channel-close edges

A `send` to a closed channel now drops the undelivered value instead of
leaking it ([163]); the bare-send form still discards the delivered flag by
design. `select` send arms are rejected with a diagnostic until MIR carries
the send direction (they used to silently behave as receive arms).
`jinn_chan_destroy` frees immediately after close with no defense against a
racing waiter — unreachable from Jinn source today (only the actor retire
list calls it), but the lifetime contract should be written down before
channels become droppable.

---

## Performance, build, and runtime hygiene

### P-1 (m) `sim for` lowers to a sequential loop

`hir::Stmt::SimFor` lowers in `src/mir/lower/loops.rs` to an ordinary counted
loop — a compare and a branch, no spawn and no scheduler involvement. It
measures neither parallelism nor the work it appears to describe, and any
benchmark row derived from it is not comparable across languages and must not
be quoted as a ratio.

### P-2 (m) There is no incremental compilation

The bar a real design must clear is recorded in
[`internals.md`](internals.md#incremental-compilation).

### P-3 (m) The driver has two parallel compile pipelines

Direct file compiles run the inline path in `src/driver/mod.rs::run()`;
`build`/`run`/`test` run `src/driver/pipeline.rs::compile_and_link`. The
stages are the same but the code is duplicated, and the two have already
drifted once (release-mode MIR verify was missing from the inline path until
[162]). Unify them or extract the shared stage sequence.

### P-4r (m) String ownership is explicit at the constructor, not the type

[163] renamed the string constructor to `build_owned_string` (cap carries
ownership; every current site is owned) and fixed the inverted SSO tag
convention in `runtime/vec.c`'s `__jinn_str_slice` trio (it marked heap
strings as SSO — wired to `InstKind::Slice` for strings and one refactor
away from corruption). The residual ask: a type-level Owned/Borrowed
distinction so a future borrowed-string site cannot pass an owned cap by
accident.

### P-5 (m) The runtime's documented error contract is narrower than reality

`runtime/README.md` states the intended contract (no `stderr` from hot
paths, `abort()` only in the allocator) and honestly lists the exceedances:
the store/WAL layer aborts on schema mismatch, sync failure, and
transaction-snapshot OOM; bad WAL magic and unopenable store files
`exit(2)`; kv/index/column/fts mutators report IO errors to `stderr` and
continue. Aligning the code with the contract (typed errors through a
fallible store surface) is the open work — see `S-4`.

---

## Coverage gaps

Nothing here is a known defect — these are surfaces no test or review has ever
exercised, listed so their silence is not mistaken for evidence.

- **V-1** Store subsystems whose only coverage is a smoke case in
  `tests/integration.rs`: `@vector` nearest-neighbour, `@versioned` history,
  `@kv`, `@column`, `@graph`, `@timeseries`, `@bloom`, `@search`.
  (Transactional rollback of `@kv` and `@versioned` gained dedicated cases
  in [163]; the rest of each surface is still smoke-only.) Compaction,
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
