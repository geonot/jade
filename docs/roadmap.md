# Roadmap — open work

The single list of known open work on Jinn. Items are grouped by subsystem and
carry a stable id so commits and changelog entries can name them. (This list
was rewritten and renumbered on 2026-08-14; changelog entries [160] and earlier
use the previous ids. The 2026-08-16 pass ([163]) closed every blocker and
major from the 2026-08-15 review; closed ids are retired, not reused. The
2026-08-18 pass ([164]) ran a fresh nine-perspective assessment adversarially
against the [163] "0 blockers" claim, closed the accept-then-corrupt holes it
found — each pinned in `tests/assessment_soundness.rs` — and filed their
residue below.)

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

**Open: ~8 blockers, ~30 majors, 40+ minors, 5+ coverage gaps** — the
remainder of the 2026-08-18 18-area adversarial review (ids `TYP-*`, `STO-*`,
`STD-*`, `DIST-*`, `HIR-*`, `SYN-*`, `PAR-*`, `TDX-*`, `MIR-*`, `CG-*`,
`PERF-*`, `DIAG-*`, `GATE-*`), which falsified the previous "0 blockers /
0 majors" headline by reproducing ~28 blockers and ~53 majors from clean
directories on first-week-plausible code, all invisible to the green gate
battery. The [165] pass closed the two soundness workstreams on the critical
path (type/ownership interior and memory-safety completeness — 18 of the
review's blockers) plus a batch of cheap high-signal blockers. The [166]
pass closed the store-engine workstream (WS-3: transactional WAL framing
with undo images, read locking, validated headers, real migration
defaults/drops, the fingerprint sentinel), cut comptime to literal-only
folding, swept the std `.length`-as-byte-count class (118 sites), and took
the cheap parser/typer/MIR/fmt blockers (`not` precedence, literal range
errors, duplicate-fn and duplicate-arm rejection, `vector` reservation,
shortest-round-trip float printing) — pinned across
`tests/alpha_hardening_pins.rs`, `tests/store_schema.rs`,
`tests/store_transactions.rs`, and `tests/fmt_nondestructive.rs`.
Remaining blockers: `DIST-1` (dead spill temp), `DIST-5r` (cross-process
store access), `STD-5` (dataframe sort), `SYN-4`, `SYN-6`, `MIR-1`,
`TYP-5`, `TDX-3`. The pre-review sections further down ([162]/[164]
residue, ids `O-*`, `T-*`, `E-*`, `S-*`, `X-*`, `N-*`, `P-*`) still stand.

- **Memory and ownership** — the [162] accept-then-corrupt holes are closed:
  consuming calls in condition/scrutinee/iterator position are move-tracked,
  multi-level projection binds reject instead of aliasing, read-only
  enforcement (freeze and views) follows the place root and mutation
  inference walks nested receivers, and unannotated borrowed params no
  longer mint second owners through the monomorphization path. [164] closed
  three more double-free / missed-destructor shapes: returning a heap
  payload out of a `match` arm no longer double-frees, a generic `take`
  parameter actually moves its argument (the double-free is now a
  use-after-move diagnostic), and `@resource` destructors run on
  early-return paths. The leak tail and inference ergonomics remain
  (`O-1`–`O-10`).
- **Types and diagnostics** — alias-typed arguments reject at the call site,
  annotated method bodies are checked per instantiation, bare `! E`
  functions Ok-wrap their implicit unit exit, numeric method return types
  infer (and `min`/`max`/`is_nan`/`signum`/`recip`/`to_int` gained real
  lowerings), and capturing lambdas stay monomorphic instead of losing
  their captures. Since [164], non-exhaustive matches on integer, float,
  string, and tuple scrutinees are rejected instead of compiling to a
  crash, generic-enum variant construction instantiates from the argument
  type instead of a first-write-wins cache, and call-site unification
  rejects container-element and numeric-narrowing mismatches the
  tolerant-unify escape used to accept. Diagnostics never leak compiler
  internals (`tests/diagnostic_hygiene.rs` gates it).
- **Errors and capabilities** — the function-value false-accept class is
  closed at its worst points: a call through a field or element callee, or
  through a local bound from a field/element/call result, now taints the
  row as an indirect call (`needs` rejects it). Since [164], passing a
  named function — or a one/two-hop local alias of one — as an argument
  taints the caller with the callee's real capabilities, so a `needs pure`
  function cannot launder a writer through a higher-order call. Residue in
  `E-2`.
- **Persistent store** — transactions on one store are serialized across
  tasks (rollback can no longer erase another task's committed writes),
  `save` syncs the data file before checkpointing the WAL, `@relaxed`
  actually batches syncs, all sidecars (`@kv`, `@versioned`, `@vector`,
  `@bloom`, `.idx`, `.fts`, `.col`) roll back with the transaction,
  recovery preserves the WAL when replay is incomplete, and store open
  never truncates an existing store on a transient error. Since [166],
  a killed process no longer persists a partial transaction (WAL
  begin/commit framing with undo images), reads take the store lock and
  validate the header, migrations apply literal defaults and drop for real,
  and a crashed migration refuses to auto-adopt. Failure-path minors remain
  (`S-1`–`S-4`, `S-9`, `S-11`, `S-12`).
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

## Alpha review backlog (2026-08-18 review)

The open remainder of the 18-area adversarial review, grouped by workstream.
Ids are the review's; severity per the taxonomy above. Items marked
*unverified* carry finder-confirmed evidence but no adversarial re-check.

### Store engine (the review's WS-3 — closed in [166], residue below)

[166] closed the store-engine blockers: the WAL brackets every transaction
with `TXN_BEGIN`/`TXN_COMMIT` frames — the begin frame carries the
pre-transaction data-file snapshot as an undo image, and recovery restores it
and drops the frames of an unmatched begin, so kill -9 mid-`transaction` no
longer persists a partial transaction (STO-1; pinned in
`tests/store_transactions.rs`). Every read path (`count`, `all`, queries,
aggregates, graph/fts traversal, index rebuild, and store open itself) now
takes the store writer lock and validates the header count against the file
size through `jinn_store_read_count_checked` — corrupt or truncated stores
refuse loudly instead of yielding garbage rows, and in-process reader/writer
and double-open races are gone (STO-2 in-process, STO-5, S-10; pinned in
`tests/store_schema.rs`). Migration `add ... default <literal>` applies the
default to existing rows, `drop` removes the field for real (see residue),
migration rewrites stamp a `-1` in-progress fingerprint that the next open
refuses with guidance instead of silently adopting (STO-6), mismatched
quaternary arms reject at the type level (STORE-1), and extern declarations
reuse the compiler's own (`store` + `extern *malloc` links — DIST-4).

Residue:

- **STO-3r** (m) migration `drop` removes the record's *final* field only and
  requires the matching `add <field> as <type>` in the `down` block (that is
  where the type comes from); any other shape is a compile error. Mid-record
  drops need a persisted schema history.
- **STO-8r** (M) store string fields still cap at 248 bytes (fixed 256-byte
  slots). [166] made the failure honest: the truncation warning can no longer
  be silenced by link order (the weak no-op in `kv.c` now warns), and the read
  path clamps corrupt on-disk lengths instead of reading out of bounds.
  Raising the cap is an on-disk format change.
- **DIST-5r** (B) *cross-process* store access remains unsafe: per-op `flock`
  serializes operations, but a rewrite (compact, rollback, migration) in one
  process leaves other processes' `FILE*` handles stale. The single-writer
  policy is only enforced per-op, not per-open.
- Read serialization is coarse: readers now exclude each other and writers per
  store. Fine for alpha; a shared-read lock is the obvious refinement.

### Distributed / std surface (B unless noted)

- **DIST-1** mutating a call-returned struct through a free-function
  parameter writes to a dead spill temp — the caller sees stale values.
  Root of **STD-7** (raft inertness).
- ~~DIST-3 / STD-1 / STD-2 / STD-3 / STD-4~~ closed in [166]: a line-audited
  sweep converted every inventoried string-byte context — FFI length
  arguments, `malloc` sizes, send loops, `Content-Length`, byte-indexed
  `slice`/`char_at` bounds — from `.length` (scalar count) to `.byte_count`
  across net, http, tls, io, crypto, aes, argon, sha, blake, regex, os, and
  process (118 sites). This closes the named breakages (net/http truncation
  and deadlock, AEAD decrypt, argon verify, every digest hashing a truncated
  prefix, PCRE2 offset mixing). *Caveat:* the sweep is site-exact but mostly
  behaviorally untested — GATE-3 still applies. Remaining `.length`-bounded
  scalar byte *loops* (toml, glob, args, bangle, path, date, hex tails) are
  `O-9`'s class: correct on ASCII, wrong-or-quadratic beyond it.
- **STD-5** dataframe sort loses and duplicates rows. ~~STD-8~~ closed in
  [166]: float `to_string` (and therefore json stringify) uses a
  shortest-round-trip formatter (`jinn_f64_format`: `%.15g` → `%.17g` with a
  `strtod` round-trip check) instead of 6-significant-digit `%g`.
- **DIST-8** (M) blocking socket syscalls pin scheduler workers — 8 idle
  connections starve a server; needs an IO reactor. **DIST-9** (M)
  `supervisor` does not parse (see N-4). **DIST-2/STD-13** (M) std/raft is
  aspirational — demote from stable, with dataframe, bangle, and the crypto
  stack, until behaviorally tested (**Tier demotion**, hours).

### Comptime and name resolution (closed in [166])

- ~~HIR-1 / HIR-2 / HIR-5~~ `src/comptime/` was cut to literal-only folding:
  the pure-function-call evaluator is deleted (its taken-branch fall-through
  was HIR-1), and the remaining expression folds are type-gated to
  `i64`/`f64` operands so a fold can never change width or signedness
  semantics. Both pinned in `tests/alpha_hardening_pins.rs`.
- ~~HIR-6~~ a top-level function whose name collides with another (including
  a module function's flattened `{module}_{fn}` spelling) is a compile error
  naming both sites, instead of last-writer-wins.
- ~~HIR-7~~ methods declared inside a `store` block are rejected at type
  checking with guidance (move to a top-level function) instead of failing
  at codegen; they were never lowered and are not in the grammar. Real
  store-method support would need a `self` story and its own lowering queue.

### Parser and surface (B unless noted)

- ~~PAR-1~~ closed in [166]: the `vec`/`vector` builtin now always wins (the
  same class as `to_string`/`log`), and *defining* a function with either
  name is a compile error naming the reservation.
- **SYN-4** the global type namespace silently merges same-named types
  across modules, last-loaded wins. *unverified*
- **SYN-6** `if x is y` with an existing binding `y` is an always-true
  pattern match that also overwrites the outer `y`. *unverified*
- ~~SYN-7~~ closed in [166]: `not` moved to its own precedence level between
  `and` and the comparisons (matching the EBNF's `not_expr`), so
  `not x in xs` is `not (x in xs)`; the formatter parenthesizes a `not`
  operand under tighter operators so old trees still round-trip.
- **PAR-2** (M) function-local `use` parses but imports nothing; **PAR-3**
  (M) `save`/`destroy`/`restore`/`compact` are undocumented
  context-sensitive statement keywords; **SYN-8** (M) the EBNF is wrong on
  ≥7 constructs; **SYN-9** (M) contextual keywords reserve identifiers
  inconsistently; **SYN-10** (M) no sort-by-key for Vec-of-struct;
  **SYN-11** (M) unknown-method errors carry no location and type as i64.

### Types (B unless noted)

- **TYP-5** generic-struct monomorphization mangles names unescaped with
  first-write-wins registration — `Pair_i64_i64` collides.
- **TYP-6..TYP-10** (M) nested generic enum construction, enum trait impls
  on self, mono cache misses, literal range bypass via backward inference,
  trait coherence only via the duplicate-DefId backstop.

### Codegen / MIR (B unless noted)

- **MIR-1** same-name loop/comprehension binders share one function-global
  MIR memory slot — silent wrong answers in nested loops. *unverified*
- **MIR-2** (M) comprehension-binder shadowing ICEs three ways; ~~MIR-4~~
  closed in [166] (duplicate unguarded literal arms are a typed
  "duplicate match arm" error, the dense-switch path dedupes literal case
  values as a backstop, and non-int/bool literal matches take the
  comparison-chain path); **MIR-5** (M) runtime tuple index ICEs; **MIR-6**
  (M) the LLVM-array indexing path emits no upper bounds check; **CG-4** (M)
  `--debug` is a stub (no DWARF).

### Tooling / fmt (B unless noted)

- ~~TDX-1 / TDX-2~~ closed in [166]: the printer emits list-comprehension
  `to`/`if` clauses (pinned in `tests/fmt_nondestructive.rs`), and under the
  new `not` precedence `not a equals b` re-parses as the same tree while a
  `not` operand under a tighter operator is parenthesized. The HIR-diff fmt
  gate remains open work. **TDX-3** `jinn run` serves a stale cached binary
  after a dependency update.
- **TDX-4..TDX-6, TDX-13** (M) silent test failure off-tty, `jinn bind`
  emits zero externs from zlib.h, tree-sitter fails 15/15 snippets, `.jni`
  reuse breaks multi-module compiles. **STD-10..STD-12, STD-15** (M) bangle
  404s every route, process.run truncates at 64KB, fmt+os ICE, five error
  dialects across std.

### Performance and benchmark honesty (M)

- **PERF-1b** stack-promote non-escaping bracket-list literals (the
  `jinn_xmalloc` linkage half landed in [165]; array_ops is ~1× vs C).
- **PERF-2** the in-tree Rust array_ops baseline runs 30× the iterations —
  the published J/RUST ratio is false. **PERF-5/6/7/9** store_ops,
  `sim_for`/`dispatch_yield`, concurrency, and actor benchmark baselines
  are strawmen or unverified — fix or drop the rows.
- **PERF-3/4/STO-7** WAL recovery is O(n²) with no clean-exit checkpoint;
  point queries re-read the whole table; WAL grows unboundedly under churn.

### Diagnostics (M)

- ~~DIAG-2~~ closed in [166]: `-<literal>` constant-folds at lowering with
  the expected type propagated, so negative out-of-range literals hit the
  same (now hard-error) range check as positive ones; **DIAG-3** the
  common newcomer error class lacks locations and did-you-mean; **DIAG-4**
  one error per run across phase boundaries; `src/diagnostic.rs`'s
  structured machinery is dead code.

### Leak-class residue from [165] (m)

- **LK-1** chained-concat intermediates (`a + b + c`) leak the heap
  intermediate; the rebind case is closed.
- **LK-2** heap-struct values on early-exit paths can still leak: the
  struct-init owning extension was reverted (struct SSA forks on field
  mutation made "drop the init value" unsound; the audit suite caught it).
- **LK-3** `continue`-path scope leaks in some shapes: the edge-escape drop
  sweep's cycle guard skips in-loop blocks.
- **LK-4** map overwrite leaks the superseded value and the new key's
  buffer (MAP-1 class); heap-key maps leak key buffers on drop.
- **LK-5** channels held by actors at stop, and unreceived channel-handle
  messages, release nothing — bounded leak, never a use-after-free.

### Gate hardening (G)

- **GATE-1** the documented channel-race TSan gate does not exist;
  **GATE-2** `ci/sanitize-corpus.sh` and `ci/fuzz-ownership.py` run in no
  pipeline; **GATE-3** 36/51 std modules have zero executed behavior
  coverage; **GATE-4** doc examples compile but never run; wire an
  LSan-gated leak corpus (the WS-2 probes) as the highest-value single
  change.

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
added to that list or its temps leak. Since [164] a `StructInit` of a type
with a `{name}_drop` function (a `@resource`) is also an owning allocation, so
its destructor runs on early-return paths, and the whole-enum `Drop` is
suppressed when a `__v`-prefixed heap payload field escapes through a store,
send, return, or phi (returning a payload out of a `match` no longer
double-frees). Still outside the covered set: `String` temps in expressions,
plain-struct heap fields and method-call results and subjects on early-return
paths (`O-1`'s class), per-iteration reallocation in loops (including closure
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

### T-5 — closed in [166]

[164] closed the call-site half (`numeric_lossless`); [166] closed the
binding-site half: an out-of-range integer literal at an annotated type
(`y as i8 is 300`, and the negative form `-200`) is a compile error naming
the range, instead of a wrap-with-warning. Pinned in
`tests/alpha_hardening_pins.rs`.

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
Since [164] a named function — or a one/two-hop local alias of one — passed
as a bare-identifier argument (`apply(writer)`, `g is writer` then
`apply(g)`) taints the caller with the callee's real capabilities
(`scan_call_arg` + `alias_root`), closing the higher-order laundering path.
Remaining under-approximations: (1) relation *traversal* (`row.owner`,
`row.children`) reads the target store without deriving its `fs` capability —
the caps scan is AST-level and cannot see that a field access is a store
read; (2) a *method-form* call on an unresolvable receiver whose name matches
no user method contributes nothing (taint here would misfire on every builtin
container method name); (3) a consuming call *inside* a `match` arm can still
launder its capability where the arm-local dataflow hides the callee; (4)
modules imported through `.jni` interface files have no bodies to scan
(interface reuse is off by default — `X-3`).

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
store. Since [166] the same snapshot is also written into the WAL as the
`TXN_BEGIN` undo image (fdatasync'd before the first in-transaction write),
doubling the per-transaction I/O — the fix for both is a page-level or
row-level undo log. `@kv` and `@bloom` additionally snapshot their in-memory
state per transaction; `@versioned` and `@vector` record only a length.

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
back on disk but this process's handle is stale". The
crashed-migration-fingerprint hole closed in [166]: migration rewrites stamp
a `-1` in-progress sentinel that the next open refuses with guidance instead
of silently adopting (pinned in `tests/store_schema.rs`). Still open: torn
`.idx` slots survive crashes (no checksums), and aux sidecar files are
`fflush`-only (`@versioned` history is not power-loss durable even under
`@durable`).

### S-10 — closed in [166]

Every read path (`count`, `all`, queries, aggregates, graph/fts, index
rebuild) and store open itself now takes the writer lock before loading the
`FILE*` and releases it after the last file access, so a concurrent
rollback, compact, or migration can no longer swap the handle under a
reader. Readers serialize against each other too — coarse but safe; a
shared-read mode is the refinement if it ever shows up in a profile.

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

### P-6 (m) Actor-message payload packing is not 8-byte aligned

[164] fixed tagged-union payload layout — enum payloads now lay out as
`[⌈N/8⌉ × i64]` (`src/codegen/decl.rs::declare_tagged_union`), forcing
8-byte alignment and removing the `align 8` at offset-4 UB on an enum
carrying an `i64`/`f64` payload. Actor-message payload packing was left on
its own path: an aligned message store/load pair is a separate change with
a matching-offset risk, and enum layout was the confirmed UB. Round the
actor payload offsets the same way, in lockstep on the store and load
sides.

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
