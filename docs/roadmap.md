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
The 2026-08-13 gates pass ([151]) — pins and benchmarks written *ahead of*
M-13r's adoption sweep — found and fixed six compiler defects (match arms
losing their values to trailing drops, match-payload rewraps double-freeing,
String captures aliasing, nested-loop method mutations lost, float `neq`
non-IEEE, one-sided int→float literal coercion) and wired `tests/stdlib/`
into the gates; it added `M-17` below and `T-15` (since closed). The
2026-08-14 types pass ([153]) closed T-10 (tuples now lower to their canonical
LLVM struct layout everywhere — the phi-shape diagnostic's trigger is gone,
and a silent heterogeneous-tuple corruption it was masking is fixed and
pinned) and T-15 (unannotated signatures resolve before callers read them),
and reduced T-1r (single-letter type names no longer shadow-collide with the
parser's type-parameter reading). The 2026-08-14 generics pass ([154]) closed
T-1r's spelling core — constructors stopped minting mono names from unresolved
inference variables, every generic-type spelling canonicalizes to one mono
name, unification sees through mono names via an origin table, and
multi-parameter generics gained their `Pair<A, B>` annotation form — leaving
the residue filed as T-1r2. [155] closed T-13r: `url`, `uuid`, and `bytes`
dropped the last scalar-`.length` byte bounds, `Bytes.to_string` emits raw
bytes instead of `?`, and `Bytes.slice` — which had returned zeros since it
was written (its length guard dropped every copy) — is fixed; both modules
gained behavior suites in the stdlib gate. [156] closed T-5r2 and T-2:
unannotated impl signatures now inherit the trait's substituted declaration
(a conflicting body fails even with no call site), impl types that cannot
instantiate an open trait type are rejected via a scratch unifier instead of
skipped, and diagnostics stopped leaking internals — inference variables
render as `_` (or prose when bare), conformance errors print surface type
syntax, field errors on monomorphized structs render the origin spelling
(`Pair<i64, string>`), and `tests/diagnostic_hygiene.rs` sweeps a battery of
failing programs asserting no `__G_`/`?N`/debug-format symbol ever reaches
stderr (ICE-class messages keep raw symbols deliberately — they report
compiler bugs, not user errors). [157] closed T-1r2 and with it the Types
section: bind annotations and declared returns flow into generic
instantiation before defaulting, `name of type(args)` supplies type
arguments to generic *functions* (return-position-only parameters have a
call-side spelling), a phantom type parameter warns when it defaults and is
bindable by annotation or the `of` ctor form, undefined type names and
arity mismatches in declared signatures and field positions are compile
errors, and a non-string `Map` key annotation states the runtime's
string-key rule instead of surfacing a bare unification mismatch. The same
pass fixed two silent-wrong-code classes it uncovered: method calls through
a not-yet-monomorphized generic receiver read fields at the fallback layout
(garbage values), and a generic enum's unit variant bound against any
annotation whose arguments were not `i64` failed to unify (`Maybe of string
is Nothing` was a type error; only the `i64` instantiation ever worked).
The "unsolved type variable defaulted to i64" warning also stopped firing
for binds that a later statement resolves — each warning is now tagged with
its variable and dropped at reporting time if the variable resolved.
The 2026-08-14 payload pass ([158]) closed M-17 — consuming one bind of a
multi-field payload now drops the unconsumed siblings (bound or wildcarded)
at arm end instead of leaking them, for local and temporary subjects alike —
and fixed three pre-existing defects its probing surfaced: codegen emitted
basic blocks in storage order and crashed at `--opt 0` on any match arm
whose value reached the merge block (blocks now emit in reverse postorder);
enum drop glue never recursed into payload fields spelled as named structs,
leaking every enum-in-enum, enum-in-struct-field, and Vec-of-enum payload;
and a constructor wrapped inside a container insert (`xs.push(Leaf(a))`)
left the source local's drop in place while the element aliased the same
allocation — a use-after-free once element drops worked.
The 2026-08-14 capabilities pass ([159]) closed C-1r's actor/store hole:
store operations derive path-scoped `fs` capabilities, actor handlers are
scan roots whose rows join at send and spawn sites, and `needs pure` now
rejects a store write or a send to a writing handler with the introduction
path named — see the Effects section for the residue that remains.
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

### M-4r — closed by [149]/[150]

The silent lost update became a compile error in [146]; the honest fix the
item demanded — a zero-copy read path for nested containers — shipped as the
view surface: `xs.at_view(i)` and `for p in pts.views()` bind element views,
field reads and read-only method calls go through the element pointer with no
copy, and mutating/consuming methods through a view are compile errors.
Expression-position `get` keeps its copy semantics by contract; `at_view` is
the zero-copy spelling. What remains of the *adoption* (std still uses the
copying idioms) is `M-13r`'s std sweep.

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
(45 of 510 corpus programs leak, 74 B–1.2 MB per run, zero corruption).

### M-10r (m) Sanitizer sweep residue: fiber annotations and the leak tail

[147] closed M-10: `ci/sanitize-corpus.sh` compiles and runs the whole
executable corpus — `tests/programs`, every `apps/` entry, every snippet, 510
programs — under ASan+LSan at `--opt 0` and `--opt 3` (1020 runs since [153]
closed T-10 — every corpus program now compiles — zero memory corruption), and
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

### M-13r — closed by [152]; residue: the rest of std's byte loops

[148] shipped step 1 of [`design/second-class-refs.md`](design/second-class-refs.md)
(`View of T`, creation methods, `View` parameters with whole-container
coercion, `{ptr, len}` codegen, full escape rejection), and [149] shipped
steps 2 and 3: a view may be **bound** (`v is xs.view(1, 3)`), which locks
its root through the same borrow lattice iteration uses — mutation, moves,
and reassignment of the root reject with the view named until the view's
block ends; views of temporaries cannot be bound; alias binds inherit the
root. `for p in pts.views()` is lending iteration (per-element view binder),
and *field* reads through an element view read through the pointer with no
element copy — closing `M-4r`'s field half. `tests/views.rs` pins all of it. [150]
added read-only *method calls* through element views — the receiver passes
the element pointer, so the call operates on the original, and
mutating/consuming methods reject with the view named — closing `M-4r`.
[152] completed step 4: the four modules were rewritten span-based with
`.byte_count` bounds (csv_parse 315x, std_string_ops 30x, json_parse 17x,
checksums identical under [151]'s benchmark gate), a whole `String` coerces
into `View of u8` parameters, `strings.__contains_byte` and
`sort.is_sorted`/`binary_search` take view parameters, and `sort` uses the
builtin byte-lex `<`. Residue: the same `.length`-bounded byte-loop idiom
survives outside the sweep's scope — [155] converted `url`, `uuid`, and
`bytes` (closing T-13r); `toml`, `http`, `regex`, `date`, `path`, `args`,
and friends are correct on ASCII but quadratic and scalar-bounded; convert
them with the same span recipe when they matter.

### M-14r (m) `freeze`: actor-handler classification, function-exit dispatch, std adoption

[148] shipped step 1 of [`design/freeze.md`](design/freeze.md) (the `freeze`
expression, `Frozen of T`, structural freezability, auto-deref reads,
compile-time rejection of every write), and [149] shipped step 2's core: every
`dispatch` inside a `together` shares one frozen value bound outside it — no
move, no copy, no refcount — with a `FrozenShare` borrow rejecting any move
of the shared value until the `together` joins, and the owner readable after
and dropping once. Pipes peel frozen arguments like direct calls (read pipes
work, mutating pipes reject with the frozen wording). `tests/freeze.rs` pins
it. Remaining: frozen values created *inside* the `together` body fall back
to single-task moves (their drop would race the join); function-exit
`dispatch` sharing (the design's second scope supplier); actor sends of
frozen values are conservatively rejected with guidance unless the handler
declares `Frozen of ...` — accepting them for provably read-only handlers
needs handler write-inference. Step 3's std adoption landed in [152] as
`toml.parse_frozen` (`Frozen of TomlTable`, read through the ordinary
accessors); what remains of step 3 is the docs pattern for model-weight
sharing beyond the tour's `together` example.

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
[149] closed the worst of the residue: the caps fixpoint now taints any call
through a function-typed parameter with a conservative "indirect call"
pseudo-capability, so a `needs`-annotated function calling a closure it did
not create is a compile error naming the introduction path (lambda *bodies*
were already scanned at their definition site, so a closure's own effects are
charged to its creator); a generator's aggregate arguments are inferred
consuming, closing the same aliasing unsoundness closures had (resuming after
the caller consumed the vec SIGSEGVed — now a use-after-move error); and
closure temporaries joined `M-9r2`'s drop-obligation set, so an
expression-position capturing closure on a straight-line path frees its
environment at function exit. Remaining: suspended-frame drops (a generator
dropped mid-iteration frees its frame but not the aggregates it still
holds); per-iteration closure temporaries in loops still leak their
environments (the return-repair pass cannot reach them by design — same
class as `M-9r2`'s loop-iteration reallocation); locals that alias a
function-typed parameter are invisible to the caps taint; by-view capture
for provably in-frame closures (step 4); and `copy x` at the capture site is
spelled "bind `copy x` to a fresh name first" rather than inline.

### M-17 — closed by [158]

[151] made consuming a match-payload bind consume the *subject*; [158]
closed both remaining edges. A consuming arm now drops the payload fields
the pattern did not consume at arm end — unconsumed binds through the
ordinary scope machinery, wildcarded fields through synthesized binds — so
the subject's suppressed whole-drop no longer leaks its other fields, and
the fix covers temporary subjects (`match f() ...`) for free because the
cleanup lives in the arm, not on the subject's place.
`tests/programs/enum_payload_drops.jn` pins every shape under the corpus
sanitizer sweep. Residue: a bind whose *field* was moved out (rather than
the bind whole) is conservatively excluded from arm-end drops, so its
remaining fields leak — same class as `M-7r`'s bounded leak side; and a
consumption in one arm still suppresses the subject's scope-end drop even
when a different arm runs (that path-sensitivity is `M-7r`).

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

Nothing open remains in this section after [157]. Two deliberate limits are
documented in the tour rather than carried as items: angle-bracket type
arguments are annotation-only (`empty<string>()` does not parse in
expression position — the `of` call form is the expression spelling), and
map keys are strings until the runtime grows typed keys.

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
it). [159] closed the actor/store hole: every store operation (statement and
expression forms, query blocks included) derives `fs.read`/`fs.write` scoped
to `./<store>.store` — deliberately both, since any operation can trigger
WAL recovery writes on open — store methods and actor handlers are scan
roots in the fixpoint, a send joins the handler's row through the
method-name bucket, and a spawn joins every handler of the spawned actor
(loop handlers run unprompted, so spawning one is using its effects); the
diagnostic renders handler items as `Actor.handler` in the introduction
path. What remains:

- Method-call edges are name-buckets: `x.m()` joins every user method named
  `m`, an over-approximation that can only produce false rejections for
  `needs`-annotated functions, never false acceptance. Sends join handler
  rows through the same bucket, so they share the same over-approximation.
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
