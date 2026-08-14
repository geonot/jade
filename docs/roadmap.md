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
the test suite and CHANGELOG entry [143]. The 2026-08-12 pass ([145]) closed
C-1, M-7, M-9, T-5r, T-13, and S-7, leaving the residues filed as C-1r, M-7r,
M-9r, T-5r2, T-13r, and S-7r below. The 2026-08-12 place-granularity pass
([146]) closed M-6 and M-5r — see the Memory and ownership header for what it
fixed along the way — leaving M-6r. The 2026-08-13 memory-model pass ([147])
closed M-9r, M-10, and M-15, reduced M-6r, M-8, M-11, and M-12, gave M-13,
M-14, and M-16 their designs (`design/second-class-refs.md`,
`design/freeze.md`, `design/closure-captures.md`), and fixed two live
unsoundness classes (idiomatic field writes invisible to both inference scans;
early-return drop omission) plus a silently-broken vec-slice codegen. The
2026-08-13 pass ([148]) shipped step 1 of all three designs — `freeze` and
`Frozen of T` (M-14), `View of T` second-class references (M-13), and
ownership-correct closure captures with owned environments (M-16) — and fixed
three live memory-unsoundness classes it uncovered on the way: ctor/container
captures of bound aggregates double-freed at scope exit, closure captures
aliased the enclosing frame (use-after-free after any invalidating operation),
and scope-task capture slots truncated every capture wider than 8 bytes.
Items below are what remains.

---

## Memory and ownership

The single-owner dataflow core — move-on-assign, branch and loop dataflow,
consuming-parameter inference, task isolation, drop discipline — holds up under
attack. [143] closed the worst of the projection/aliasing seams at *variable*
granularity, and [146] made the analysis **place-based** (old M-6, the last
blocker here): `moved_vars`/`moved_fields` are one place lattice
(`src/typer/place.rs`) with overlap and disjointness queries, iteration
borrows cover field places, map iteration, and `Iter`-desugared loops,
call-site exclusivity checks argument *places*, and the sweep surfaced and
fixed four runtime double-frees (moving out of a borrowed parameter,
constructor capture of a borrowed parameter, the inert user-method consuming
check, nested `take` SIGSEGV) plus two silently-broken corpus components
(`std/collections` heaps never sifted; `apps/physics_engine` integrated
copies). `tests/place_ownership.rs` pins all of it, including the previously
unpinned [143] diagnostics.

[147] extended the same probe-first method to the rest of this section.
Idiomatic field writes in methods (bare `data is x` — the documented style)
were invisible to *both* inference scans: a method storing a parameter that
way double-freed at runtime, and a method mutating a field that way silently
lost updates through nested receivers; both scans now treat a bind to a
`self`-field as what it is. Moves inside quaternary arms were never recorded
(the post-lowering walk skipped `Block` expressions), and `x ~ consuming_fn`
pipes compiled and then SIGSEGVed on the next read of `x`; both are ordinary
use-after-move errors now. On the drop side, MIR now proves and repairs its
own placement: a must-hold dataflow over container allocations inserts the
drops the typer omits on early-return paths (previously every `return` inside
an `if` leaked every live local container), and the verifier fails the compile
if any must-held allocation still reaches a `return`. `ci/sanitize-corpus.sh`
runs the whole executable corpus (510 programs) under ASan+LSan at `--opt 0`
and `--opt 3`: **zero memory-corruption findings**, and its first run caught
`v from a to b` on vectors passing 3 of the 4 runtime arguments — a silent
wrong answer on every vec slice, now fixed and pinned. `std/arena` ships the
blessed replacement for pointer-linked structures (old M-15): a generational
`Arena of T` whose `Handle`s detect staleness instead of dangling.

[148] built the three surfaces this section had only designed — `freeze`
(M-14), second-class views (M-13), and ownership-correct closures (M-16) —
and its probing found and fixed three more live memory bugs: a constructor or
container literal capturing a bound aggregate left the original's scope-end
drop in place (silent double-free at exit on `App(cfg is xs)`), closure
captures aliased the enclosing frame instead of owning anything (the [143]
"returns a lambda that captures the local" ban treated one symptom; the alias
itself use-after-freed on any invalidation), and scope-task capture slots
were hardcoded to 8 bytes, truncating any capture wider than a word (a
24-byte `String` or a 16-byte closure captured by a `dispatch` block
corrupted the heap). All three are pinned; the post-pass
`ci/sanitize-corpus.sh` sweep stays at zero corruption.

### M-6r (m) Place residue

What place granularity deliberately does not yet do: element indices are
compared only when both are integer literals (any dynamic index conservatively
overlaps), and `sim for`/`together` blocks keep their own coarser capture
rules. Closed by [147]: quaternary/ternary arms now record moves with
snapshot-and-union semantics (a consuming call inside a `? !!` arm tombstones;
the same call in both `? !` arms counts once), pipes move-mark through
`fn_param_access` like ordinary calls, and probing showed `defer` observes
variable state at scope exit, so mutation during a pending defer is sound and
needs no new rejection (moves were already blocked root-based).

### M-4r (m) Element reads still deep-copy

The silent lost update is now a compile error — including through call
arguments and nested method receivers since [146] — but every nested-container
read in expression position still pays a hidden O(n) deep copy, and a
*read-only* method call on an element read still operates on a copy without a
diagnostic. The honest fix is `M-13r`'s remaining steps (field and method
reads through element views).

### M-7r (m) Conditional consumption: over-tombstones on one side, leaks on the other

[145] hit M-7's minimum bar: the use-after-move diagnostic now names the
consuming site in the callee (`it is consumed at file:line:col`) and says when
that site sits on a conditional path, with the callee name demangled. The
analysis itself is still path-insensitive — a call that dynamically never
consumes is still rejected. [147] found the dual on the drop side: a value
consumed on only *one* branch is excluded from scope-end drops entirely, so it
leaks on the branch that did not consume it (the drop-repair pass inserts only
must-held drops, deliberately — an unconditional drop there would double-free
the consumed path). Path-splitting for the common `if`/`return` shape would
close both halves at once; until then the leak side is bounded and measured by
`ci/sanitize-corpus.sh`.

### M-8r (m) Consuming/mutating inference falls back to name-buckets for unknown receivers

[147] closed the knowable half: both inference scans resolve receiver types
from single-static AST facts (parameter annotations, constructor binds, `self`
and its fields) and consult that type's own method table instead of the
builtin name list — `g.set(x)` on a `Gauge` no longer marks `x` consuming
because `Vec` has a `set` (`tests/place_ownership.rs` pins it). The same pass
found and fixed both scans being blind to idiomatic bare field writes
(`data is x` stored without consuming → double-free; `total is total + x`
mutated without marking → silent lost updates through nested receivers). What
remains is the genuinely unknowable receiver — locals bound from calls,
element reads, rebound names — which still joins the name-bucket; full
resolution needs types at scan time, i.e. moving the fixpoint after inference.

### M-9r2 (m) Drop repair and leak verification cover containers, not every temp

[147] closed M-9r's stated gap and went one further: drop obligations are now
first-class at the MIR level for container allocations (`vec_new`, `map_init`,
container-typed `clone` and known-function call results). A must-hold dataflow
with ownership-precise transfer edges (consuming slots from `fn_param_access`,
container-insert builtin methods, stores, sends, captures, returns) *inserts*
the drops the typer omits — early returns leaked every live container before
this — and `src/drops/verify.rs` then fails the compile if any must-held
allocation still reaches a `return`. Inserted drops land after inlined `defer`
bodies, so defer-reads-then-drop ordering holds on early returns too. Residue:
method-call results (`p.split('/')` on an early-return path), `String` temps,
loop-iteration reallocation, and the conditional-path leaks of `M-7r` are
outside the obligation set; `ci/sanitize-corpus.sh` measures that surface
(49 of 510 corpus programs leak, 76 B–49 KB per run, zero corruption).

### M-10r (m) Sanitizer sweep residue: fiber annotations and the leak tail

[147] closed M-10: `ci/sanitize-corpus.sh` compiles and runs the whole
executable corpus — `tests/programs`, every `apps/` entry, every snippet, 510
programs — under ASan+LSan at `--opt 0` and `--opt 3` (919 clean runs, zero
memory corruption, the two compile failures are `T-10`'s pinned program), and
`ci/fuzz-ownership.py` mutates ownership-relevant syntax (duplicated
arguments, inserted/swapped `take`/`copy`, late uses, rebinds) and asserts the
compiler either rejects with a diagnostic or the binary runs memory-safe under
ASan — 197 mutants, 128 ran clean, 69 cleanly rejected, no ICE, no memory
error. Its first full run caught vec slices passing 3 of the runtime's 4
arguments (silent wrong answers since the surface existed; fixed, pinned in
`tests/semantics_regression.rs`). Residue: the runtime lacks
`__sanitizer_start_switch_fiber` annotations, so actor-heavy programs can
SEGV spuriously (no report) under ASan — the ASan sibling of `N-5`'s TSan gap
— and the leak tail belongs to `M-9r2`. Leaks report but do not gate
(`JINN_SAN_STRICT=1` makes them gate).

### M-11r (m) Category transitions warn only at an asserted definition

[147] shipped the assertion half: `type Point @value` / `type Bag @aggregate`
pin a struct's ownership category, and a definition whose fields contradict
the assertion is a compile error naming the field that flips it
(`src/typer/resolve.rs::check_category_assertion`). Unasserted types still
transition silently when a field changes category several embeddings away; the
embedding-site diagnostic ("this change makes `Config` an aggregate:
assignments now move") needs a cross-compile baseline and remains open.

### M-12r (m) Boundary ownership: policy and `fmt` insertion remain

[147] shipped the mechanical half: `.jni` interfaces are version 2 and carry
per-parameter `consumes`/`mutates` bits populated from the inferred tables
(inferred consumingness is in the interface, as required), and a `--lib`
compile warns on every exported function whose parameter consumes *by
inference*, naming the escape site and suggesting the explicit
`v as take ...` spelling (`tests/place_ownership.rs` pins it). Making
explicitness *required* at package boundaries, and having `fmt` insert the
annotation, are policy decisions deferred until the package/visibility surface
exists (`design/lamp.md`); `fmt` insertion additionally needs inference results
at format time.

### M-13r (m) Second-class references: bind position, lending iteration, std adoption

[148] shipped step 1 of [`design/second-class-refs.md`](design/second-class-refs.md):
`View of T` exists (`{ptr, len}`, trivially droppable, bounds-checked reads),
created by `xs.view(a, b)`, `xs.at_view(i)`, and `s.view(a, b)` over string
bytes; a `View of T` parameter accepts a whole `Vec`, an array, or a sub-slice
view via automatic coercion; views support `.length`, `.get(i)`, and
iteration; and every escaping position rejects at compile time — binds,
returns, struct/enum/store fields, nested annotations, container elements,
task and closure captures, sends, and yields (`tests/views.rs` pins all of
it). Remaining, in the design's own sequencing: bind-position views with
root-locking through the place lattice (step 2 — until then a view lives only
inside its statement, which the existing statement borrows already make
sound), `views()` lending iteration (step 3), the std adoption sweep with
benchmarks (step 4), and field/method reads through an element view — the
part of `M-4r` step 1 does not yet reach.

### M-14r (m) `freeze`: the multi-task sharing exception

[148] shipped step 1 of [`design/freeze.md`](design/freeze.md): `freeze x`
consumes its aggregate operand through the move lattice and produces
`Frozen of T` — structurally checked freezability with the offending field
path named, auto-deref on every read path, and compile-time rejection of
every write (mutating and consuming methods, assignment through a frozen
component, partial moves, mutating/consuming parameters), with `Frozen of T`
usable in parameter and field position as an API contract
(`tests/freeze.rs` pins all of it). Remaining: step 2's whole point — the
capture exception that lets every `dispatch` inside a `together` share one
frozen value without copying (today a frozen value moves into at most one
task, like any aggregate); step 3's std adoption; and peeling `Frozen`
arguments at actor-send and pipe boundaries, which today fail unification
with a generic type mismatch instead of the frozen-aware diagnostic the
direct-call path gives.

### M-16r (m) Closures: caps edges, generators, temp environments

The premise of M-16 was wrong in the fortunate direction: capturing lambdas
already parsed and ran — by silently aliasing the enclosing frame, a
use-after-free whenever a captured aggregate was consumed or reallocated.
[148] implemented the specification's step 1
([`design/closure-captures.md`](design/closure-captures.md)): captures
classify by category (scalars copy, `String`s clone into the environment,
aggregates move with `MoveReason::ClosureCapture`; views, `@resource` values,
and borrowed parameters are rejected), the closure value is an aggregate that
owns a heap environment carrying its own drop function, function-typed
parameters borrow, and closures move into at most one task
(`tests/closure_captures.rs` pins it; returning a closure over a local
aggregate is now sound and pinned in `tests/alpha_review_regressions.rs`).
Remaining: the caps fixpoint still has no edge for an indirect call through a
closure value, so a `needs`-annotated function calling one under-reports
(design step 2); generators still need suspended-frame drops before the same
rules apply to them (step 3); by-view capture for provably in-frame closures
(step 4); `copy x` at the capture site is spelled "bind `copy x` to a fresh
name first" rather than inline; and a closure created in expression position
with captures (`xs.map` with a capturing lambda argument) leaks its
environment — closure temps are outside `M-9r2`'s drop-obligation set, and
the leak tail is measured by `ci/sanitize-corpus.sh`.

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

### T-5r2 (m) Trait/impl conformance skips inference-eligible signatures

[143] parsed `! E` on trait methods; [145] enforces conformance: an impl may
neither widen nor narrow the trait's declared error row, and annotated return
types and parameter counts must match (both sites named in the diagnostic,
`Self` and trait type arguments substituted). Unannotated impl parameters and
returns are still accepted by adoption, and trait-side types that stay generic
after substitution are skipped rather than deferred to inference.

### T-10 (M) Control-flow merges of differently-shaped values are unrepresentable — *verified*

`tests/programs/compiler_pipeline.jn` does not compile: a recursive enum payload
rebound inside a loop produces a merge whose incoming values have different
shapes (array on one path, tuple on another). This is a diagnostic naming the
function and the construct, not an ICE, and the harness asserts that exact
diagnostic — so it reports both a regression to a panic and a fix.

### T-13r (m) String-as-byte-buffer residue in std

[145] decided T-13: `chr(code)` UTF-8-encodes a Unicode scalar (invalid and
surrogate codes encode U+FFFD), a new `byte(code)` builtin emits the raw byte
(documented as outside the UTF-8 contract, like mid-scalar slices), both
reject non-integer arguments, and the byte-assembling std callers (`uuid`,
`url`, `codec`, `strings.StringBuilder`) moved to `byte` — which also fixed
`uuid.v7()` returning `""`, `percent_encode`/`codec.to_hex` truncating
non-ASCII input, `to_lower`/`to_upper` dropping trailing bytes, and
`StringBuilder` undercounting byte sizes (all were scalar-`.length` bounds on
byte loops). Residue: the `.slice(i, s.length)` scalar-bound idiom survives on
ASCII-expected text paths (`url` parsing, `uuid.parse`), and `Bytes.to_string`
is still lossy, so `std/bytes.jn` cannot yet serve as the byte-buffer bridge.

---

## Effects and capabilities

### C-1r (m) Capability classification gaps

[145] made the capability pass real: effects are classified at the extern
leaves (`src/cap_sites.rs` classifies every extern std declares; an
unclassified extern call, `syscall`, or `asm` block derives `ffi.unsafe`),
std-vetted `io.*`/`fs.*` calls carry path-scoped capability signatures whose
row replacement is trusted only for modules loaded from a std directory, and
the fixpoint in `src/typer/caps.rs` scans free functions, generic functions,
and type/impl methods — so `needs` is a checked upper bound (the old sneaky
canary is a compile error naming the introduction path; `tests/caps.rs` pins
it). What remains:

- Actor handlers and store operations (a store is a filesystem write) carry no
  classification and are invisible to the pass.
- Method-call edges are name-buckets: `x.m()` joins every user method named
  `m`, an over-approximation that can only produce false rejections for
  `needs`-annotated functions, never false acceptance.
- Modules imported through `.jni` interface files have no bodies to scan
  (interface reuse is off by default — X-5).
- Module and project capability ceilings, and the manifest surface, remain
  design ([`design/compiler-prereqs.md`](design/compiler-prereqs.md)).

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

### S-7r (m) Flat filter chains cap what `in` can compose with

[145] closed S-7's two gaps. Query blocks accept `field in [..]` (the
desugared Or-chain is hoisted to the head of the filter so the left-to-right
fold stays correct; combining it with `or`, a second `in`, or an empty list is
a diagnostic) and method-form text predicates (`name.contains(..)` and
friends). ASCII case-insensitive comparison exists on both paths: `iequals`,
`icontains`, `istarts_with`, `iends_with` in statement filters and their
method forms in query blocks, compiled against `jinn_ascii_imemcmp` (the same
ASCII folding `@search` and `to_lower` use). Residue: the *statement* parser
still rejects `in` combined with `and` (its parse-time mixed-connector
ambiguity check predates the hoist), and a filter is still one flat and/or
chain — supporting `a in [..] and b in [..]` or real grouping needs a grouped
predicate encoding through MIR's name-encoded call scheme.

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

### N-5 (m) Whole-program sanitizers need fiber annotations

Coroutines migrate between OS threads, so TSan needs `__tsan_switch_to_fiber`
annotations at the context-switch points before its results can be trusted.
The same gap exists for ASan ([147]): without
`__sanitizer_start_switch_fiber`/`__sanitizer_finish_switch_fiber` around the
switches in `runtime/sched.c`, ASan's stack-bounds tracking can SEGV
spuriously (no report) on actor-heavy programs — `ci/sanitize-corpus.sh`
classifies those runs as `segv?` and does not gate on them. One annotation
pass at the context-switch points serves both sanitizers.

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
