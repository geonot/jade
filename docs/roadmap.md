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

---

## Memory and ownership

The single-owner dataflow core — move-on-assign, branch and loop dataflow,
consuming-parameter inference, task isolation, drop discipline — holds up under
attack. Every open item below lives at a **projection or aliasing seam**: moves
of *variables* are tracked exactly, moves and borrows of *places*
(fields-in-literals, container elements, overlapping call arguments) are not.
`M-6` is the refactor that subsumes `M-1`–`M-5`; the individual fixes are listed
because each is shippable on its own.

### M-1 (B) A struct-literal field initializer does not move its source — *verified*

```jinn
type Holder
    items as Vec of i64

*main
    v is vec(1, 2)
    h is Holder(items is v)   # no tombstone recorded
    log(v.length)             # compiles
    0
```

Aborts with `free(): invalid pointer`, exit 134. `b is a` is tracked; a field
initializer in a constructor literal is not, so `h.items` and `v` both believe
they own one buffer and scope exit drops it twice.

**Done when:** any value-position aggregate variable in any constructor
expression — struct literal, enum payload, `vec(v1, v2)`, map literal —
tombstones its source exactly as `b is a` does.

### M-2 (B) The same variable may be passed to two consuming parameters in one call — *verified*

```jinn
*sink(v) returns Vec of i64
    return v

*combine(a, b) returns i64
    x is sink(a)
    y is sink(b)
    return x.length + y.length

*main
    v is vec(1, 2, 3)
    n is combine(v, v)   # two owners of one buffer
    log(n)               # prints 6, exit 0 — the double free is undetected
    0
```

The tombstone for a consuming argument is recorded *after* the call statement,
so the second `v` in the same argument list never sees it. Sequential calls
(`a is sink(v)` then `b is sink(v)`) are correctly rejected; only
intra-statement duplication slips through.

**Done when:** a per-call-site alias check rejects the same root place appearing
in two consuming positions, or in a consuming and any other position, within one
call.

### M-3 (B) Call-site exclusivity is unenforced; `noalias` is a wish — *verified*

```jinn
*app(dst, src)
    for i in 0 to src.length
        dst.push(src.get(i))

*main
    v is vec(1, 2, 3)
    app(v, v)      # accepted; dst is BorrowMut, src is Borrowed, same object
    0
```

Compiles and runs. `set_ptr_param_attrs` emits `noalias`/`readonly`/`nocapture`
on those parameters, so a read-only parameter mutated through its own alias is
undefined behaviour at the LLVM level — a latent miscompile that can appear at
`--opt 3` on any codegen change. Perceus reuse pairing makes the same
uniqueness assumption.

**Done when:** if any parameter of a call mutably borrows place `P`, no other
parameter may borrow `P` or any overlapping place. Disjoint fields of one struct
may be permitted with a disjointness proof; whole-object overlap is rejected
with a diagnostic naming `copy`.

### M-4 (B) Container element reads deep-copy instead of borrowing; mutations vanish — *verified*

```jinn
*main
    grid is vec()
    grid.push(vec(1, 2))
    grid.get(0).push(3)
    log(grid.get(0).length)   # prints 2 — the push was applied to a temporary
    0
```

The contract says element reads in expression position are borrows. The
implementation copies. Two consequences: silent lost updates (no diagnostic, no
crash, wrong answer), and a hidden O(n) deep copy on every nested-container
read — the exact hidden cost the model exists to prevent.

**Done when:** one of these holds everywhere — true statement-scoped element
borrows (needs `M-5`'s invalidation rules), or copies retained but *mutating an
rvalue copy is a compile error* and an element read may never reach a
`BorrowMut` parameter. Silent copy-and-discard must not survive either way.

### M-5 (B) Iteration borrows are unchecked — *verified*

```jinn
*main
    v is vec(1, 2, 3, 4)
    total is 0
    for x in v
        v.push(9)      # accepted
        total is total + x
    log(total)         # never reached
    0
```

Hangs. Iteration reads the live buffer, so the loop chases the growing vector,
and a mid-iteration realloc leaves the iterator's cached pointer dangling.

**Done when:** `for x in v` takes an implicit iteration borrow of `v` for the
loop body — mutating calls on `v`, moves of `v`, and passing `v` to a
`BorrowMut`/consuming parameter inside the body are compile errors. Escape
hatches: `for x in copy v`, or an index loop.

### M-6 (B) Make the analysis place-based rather than variable-based

Unify `moved_vars`, `moved_fields`, and the element rules into one *place*
lattice (`root.field.elem…`) with overlap and disjointness queries. Every item
above falls out of variable-granularity thinking; every fix falls out of place
granularity. This is the highest-leverage refactor in the compiler.

### M-7 (m) Conditional consumption over-tombstones and the diagnostic hides why

A callee that consumes a parameter on one branch is inferred unconditionally
consuming, so a call that dynamically never consumes is still rejected. Sound
but imprecise, and the diagnostic ("whose parameter takes ownership") does not
say the consumption was conditional. Minimum bar: the diagnostic names the
consuming path. Better: path-splitting for the common `if`/`return` shape.

### M-8 (M) Consuming-method detection is a hardcoded name list — *verified*

`CONSUMING_METHODS` in `src/typer/consume_infer.rs` is a `&[&str]` matched by
name (`push`, `insert`, `append`, `add`, `put`, `set`, `enqueue`, `send`, …). A user method named `set` that does
not store its argument spuriously tombstones its callers' arguments; a method
with any other name that *does* store its argument escapes detection into the
same double-free class as `M-1`.

**Done when:** consumingness is derived from the method body by the same escape
scan already applied to free functions, with the name list retained only for
runtime-implemented builtins that have no body to scan.

### M-9 (M) MIR drop-linearity verifier — *verified absent*

Nothing in `src/` checks that drop placement preserves "exactly one drop per owner" after
sinking, fusion, and reuse. A debug-build verifier asserting that every owner is
dropped exactly once on every path — re-run after each Perceus transform —
turns silent corruption into an ICE.

### M-10 (M) Whole-corpus sanitizer sweep

`ci/sanitize.sh` runs nine targeted programs. Every conformance test and every
`apps/` program should run under ASan+LSan at `--opt 0` and `--opt 3`, plus a
fuzzer that mutates ownership-relevant syntax. `M-1` would have been caught by
the first run.

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

### T-1 (M) Methods on generic types never emit a body — *verified*

```jinn
type Box of T
    it as T

    *get() returns T
        self.it

*main
    b is Box of i64(42)
    log(b.get())
    0
```

Fails with ``unknown method `Box_i64_get` ``. `Box of i64(42)` itself works, so
generic *data* is usable and generic *behaviour* is not. Also open in the same
area: a generic struct has several mutually non-unifying spellings, and
return-position-only and phantom type parameters cannot be used, with no
turbofish to escape.

### T-2 (m) Mangled internal names reach user diagnostics — *verified*

The `T-1` message names `Box_i64_get`, an internal mangling. No diagnostic
should print a mangled symbol.

### T-3 (M) Undefined names are caught in codegen, with no span, and `--emit-hir` exits 0 — *verified*

```jinn
*main
    log(y)
    0
```

`jinnc` reports `Load of undefined variable 'y'` with no file, line, or column.
`jinnc --emit-hir` on the same program **exits 0**, so the repo's own
recommended fast frontend check passes broken code — which also means the
frontend gates cannot certify what their names imply.

**Done when:** name resolution rejects this in the typer with a span, and
`--emit-hir` exits non-zero for it.

### T-4 (M) A bare `! E` signature with a non-unit tail panics the compiler — *verified*

```jinn
err E1
    Bad

*f() ! E1
    0
```

Panics inside inkwell: `Found IntType … but expected the StructType variant`. A
bare `! E` means `Result of Unit, E`, so the body is a type error — it must be
diagnosed in the typer, not reach codegen. (`*f() ! E1` with a unit body works
correctly.)

### T-5 (M) `! E` is a parse error in a trait method signature — *verified*

```jinn
trait Reader
    *read() returns i64 ! E1
```

`line 5:25: expected *, got !`. No trait method can be fallible, and an impl may
silently widen the error row past the trait's declaration.

### T-6 (m) Duplicate catch-all function clauses: last wins — *verified*

Two definitions of `*f(n as i64)` compile with no diagnostic and the *second*
one runs, contradicting the documented first-match clause order. Either reject
the duplicate or honour first-match.

### T-7 (M) Unsolved type variables are defaulted silently — *verified*

```jinn
*empty()
    vec()

*main
    v is empty()
    log(v.length)
    0
```

The element type of the returned vector is never solved. This compiles and runs
under `--strict-types` *and* `--warn-inferred-defaults` with no diagnostic at
all — the flags whose entire purpose is to catch this. Passing a value of the
wrong type into such a vector is then unchecked.

The specific higher-order case that used to print raw string bytes as an integer
now works, so the defaulting is no longer producing garbage on that path; the
silence is what remains.

### T-8 (m) Silent lossy narrowing through an `as` annotation — *verified*

`u as u8 is 300` binds 44 with no diagnostic, while `as strict` exists and
traps. Annotation-position narrowing should either warn or require `as strict`.

### T-9 (m) ALL_CAPS constants are silently shadowed — *verified*

A top-level `MAX is 10` rebound inside `*main` prints 20. Documented as "cannot
be reassigned".

### T-10 (M) Control-flow merges of differently-shaped values are unrepresentable — *verified*

`tests/programs/compiler_pipeline.jn` does not compile: a recursive enum payload
rebound inside a loop produces a merge whose incoming values have different
shapes (array on one path, tuple on another). This is a diagnostic naming the
function and the construct, not an ICE, and the harness asserts that exact
diagnostic — so it reports both a regression to a panic and a fix.

### T-11 (m) Parser stack overflow on long flat operator chains — *verified*

`x is 1 + 1 + … ` with ~5000 terms overflows the compiler's stack and aborts
with no diagnostic. Parenthesized nesting is correctly depth-limited; flat
chains are not.

### T-12 (m) A UTF-8 BOM is rejected with a mojibake message — *verified*

`line 1: unexpected character: 'ï'`. Strip the BOM, or name it.

### T-13 (m) Safe code can construct invalid-UTF-8 Strings — *verified*

`chr(200)` yields a one-byte String holding `0xC8`, which is not valid UTF-8,
while `String` is documented as guaranteed valid UTF-8.

### T-14 (m) int→float coercion works only at argument position, and its diagnostic reads backwards — *verified*

```jinn
*main
    x as f64 is 3
    0
```

```
type mismatch: expected integer type (i8..u64), found `f64` (bind annotation)
```

The annotation is `f64` and the value is an integer, so expected and found are
named the wrong way round. Coercion happens at argument position but not in a
bind annotation, nor between binary operands (`2.5 + 1` is rejected).

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

### C-2 (M) `embed` reads any absolute or `../` path at compile time, ungated — *verified*

`data is embed '/etc/hostname'` compiles and bakes the file's contents into the
binary. A downloaded package can therefore exfiltrate compile-host files into
the user's own artifact, with no capability required and nothing to grep for.

### C-3 (m) The comptime purity classifier is unsound but currently unreachable — *verified*

`src/comptime/purity.rs` has `ExprKind::Call(_, _, args) => args.iter().all(is_pure_expr)`:
purity of a call is decided from its *arguments*, never by consulting the callee. `eval_expr` returns `None` for every I/O node, so
evaluation bails before reaching an impure leaf. Latent, not live — any
widening of `eval_expr` makes it live immediately.

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

### S-4 (m) `search` returns sids, not rows

Most store results are now properly typed: `history` returns the store's row
struct, `distinct` returns the field's declared type, `vector.nearest` returns
`Vec of (i64, f64)`. `search` still returns `Vec of i64` — a vector of sids the
caller must resolve by hand. It should return rows like the others.

### S-5 (m) Durability policy is still a process-global env var

`@transient` gives a per-store relaxed-persistence dial, but the fsync policy
itself is `JINN_WAL_SYNC`, read once at first open. A program mixing a critical
ledger with a throwaway cache gets one policy for both, set outside the source.
Wanted: per-store `@durable` / `@relaxed` / `@volatile`, with the env var
demoted to a testing override.

### S-6 (m) `JINN_WAL_SYNC=group` never syncs outside a transaction — *verified*

In `group` mode `jinn_wal_force` returns without syncing, and only
`jinn_wal_commit_group` — reached at transaction commit — actually flushes. A
program that never opens a transaction therefore runs with no durability at all
under a policy whose name implies deferred, not absent, syncing.

### S-7 (m) `where` has no `in` and no case-insensitive match — *verified*

Parenthesized grouping, `contains`, `starts_with`, `ends_with`, and `between`
are all implemented (`FilterPred` plus `parse_filter_group`). Missing: `in [..]`
and case-insensitive comparison, despite `@search` existing.

### S-8 (m) Each transaction snapshots the whole store file — *verified*

`jinn_txn_track_impl` copies the entire data file, bounded by
`JINN_TXN_SNAPSHOT_MAX` (256 MB default) and then `abort()`. The diagnostic names
the knob and the workaround, so it is not silent, but the cost is O(store size)
per transaction and has never been measured against a realistic store.

### S-9 (m) A WAL with bad magic calls `abort()` — *verified*

Refusing to touch a file that is not a Jinn WAL is right; terminating the
process by `abort()` rather than surfacing a normal runtime error is not.

## Tooling

### X-1 (B) `jinn fmt` destroys code — *verified*

Sweeping all 688 corpus files through `jinnc fmt` and re-checking each output:

| Corpus | Files | Compiled before, not after |
| --- | ---: | ---: |
| `snippets/` | 402 | 17 |
| `tests/programs/` | 87 | 41 |
| `apps/` | 113 | 53 |
| `std/` | 50 | 41 |
| `benchmarks/` | 36 | 12 |
| **total** | **688** | **164** |

`--write` applies this silently and exits 0. String literals escape and
round-trip correctly, and formatting is idempotent (one non-idempotent file, one
file `fmt` refuses outright); the damage is printer grammar drift — store,
extern, actor and query forms, the `...` placeholder, and parenthesization
precedence.

**The gate misses it by construction.** `fmt_roundtrip_over_snippets_corpus`
checks idempotence and comment preservation, and it passes. It never checks that
the output still *compiles*, and it covers only `snippets/` — the corpus with
the lowest damage rate. `std/`, where 41 of 50 files break, is not in any fmt
gate at all.

**Done when** one of: the printer is rebuilt against the real grammar and a
compile-after-format sweep over all 688 files reports 0, gated in CI; or `fmt`
becomes print-only, `--write` is removed, and the design in
[`design/fmt-and-lint.md`](design/fmt-and-lint.md) is what gets built instead.
Either way the gate must assert compilation, not just idempotence.

### X-2 (m) `jinn init NAME` scaffolds into the current directory — *verified*

`jinnc init myproj` writes `project.jn` and `source/` into the cwd rather than
into `myproj/`.

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
