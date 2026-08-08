# Alpha review — findings

The record of what the 2026-08-06 alpha readiness review found, written so the
defects stay legible after the fixes land. Remediation state lives in
[`alpha-review-status.md`](alpha-review-status.md); this file does not change
when a fix ships, so a finding's wording always describes the *original*
defect.

Severity is about the alpha user. **P0** — the user hits it and loses trust
(unsoundness, miscompile, data loss, ICE on plausible code). **P1** — fix
during alpha. **P2** — post-alpha.

Findings marked **[found during remediation]** were not in the original review;
they surfaced while fixing its neighbours and are recorded here so the count is
honest.

## P0

**AR-F1 — gates/std: "alpha-stable" was a frontend-only bar.**
`docs/std.md` defined the tier as `jinnc std/<m>.jn --lib --emit-hir` exiting 0.
All 49 modules passed. A five-line program that did nothing but `use <module>`
and `log(1)` failed for **10 of them**: `strings`, `args`, `http`, `bangle`,
`bytes`, `logging`, `url`, `dataframe`, `glob`, `net` — four Rust panics, three
LLVM-verification ICEs, one internal codegen error, two link failures on
missing runtime C symbols (`c_is_dir`, `jinn_dns_resolve`). The opening example
of `docs/std.md` itself panicked the compiler.

**AR-F2 — codegen: join points were never type-unified; the compiler panicked
on the phi.**
```jinn
*pick(f as bool)
    if f
        1
    else
        'two'
```
→ `ICE: phi node type mismatch` at `src/codegen/mir_codegen/mod.rs`. This one
defect was behind eight of the ten AR-F1 failures and
`tests/programs/compiler_pipeline.jn` (whitelisted as `KNOWN_ICE` rather than
fixed). MIR verify was `#[cfg(debug_assertions)]`, so it was absent from the
release compiler, and it did not check phi/edge agreement anyway.

**AR-F3 — memory: safe code constructed use-after-free and double-free four
ways, with zero diagnostics.**
1. returning a closure that captures a local `Vec` → SIGSEGV, exit 139
2. `bucket.push(v)` twice → `free(): invalid pointer`, exit 134
3. whole-struct bind after a partial field move → SIGSEGV at `0x8` (a literal
   failure of the `docs/memory-model.md` §7 required-rejection table)
4. `m.set('k', v)` then `m.get('k').length` → a garbage heap address

**AR-F4 — store: `kill -9` during `destroy` lost the entire store.**
5000 seeded records, 40 kill points: 5 wiped all 5000, permanently.
`store/delete.rs` did `fclose` then `fopen(path,"w+b")` — truncating the live
data file — before writing survivors. Every other rewrite path correctly used
the `jinn_atomic_rewrite` temp-file-plus-rename helper.

**AR-F5 — store: two coroutines writing one store corrupted it 75% of the
time.** 20 runs of two `dispatch`ed tasks inserting 50 rows each: correct=5,
wrong count=6, unreadable=9. `flock(fileno(fp), …)` locks the open file
*description*; both coroutines shared one `FILE*`, so it was a no-op between
them, and there was no intra-process mutex. `jinn_writer_lock` was fully
implemented and never called.

**AR-F6 — store: a stale `@index` returned another record's row as `Ok(...)`.**
Deterministic, no crash. `destroy`/`compact` rewrote the record file, shifting
every offset, without touching `jinn_idx_*`, and the index's schema fingerprint
was unchanged so nothing detected staleness.

**AR-F7 — optimizer: MIR DCE deleted trapping instructions whose result was
unused.** `as strict` aborted at `--opt 0` and printed happily at `--opt 1` and
`--opt 3`. Same for integer division by zero and shift-count overflow. Scoped:
the trap survived when the result was *used*; it was deleted only when unused —
exactly the validation-assertion pattern the feature exists for.

**AR-F8 — codegen: cast lowering checked neither size nor range.**
Enum→int allocated for a 4-byte enum then `load i64`, reading four bytes past
the object (live stack content — also an information leak); `snippets/1-100/s079.jn`
printed a different garbage value on every run at `--opt 0` and `0` at `--opt 3`.
Float→int emitted raw `fptosi`, i.e. LLVM `poison`: one expression, three
answers, one of them a live heap address. Explicit enum discriminants were
parsed and then discarded.

**AR-F9 — types: unsolved type variables defaulted to `i64` and compilation
proceeded.** A higher-order generic printed `2189672` — the bytes `h i !` read
as an integer. A recursive generic enum through `Vec` compiled with a warning
and SIGSEGV'd. `--strict-types` rejected neither.

**AR-F10 — effects: capability inference was inert; `needs pure` was an
unchecked comment.** A function annotated `needs pure` read a file and wrote
another. `src/cap_sites.rs` keyed every entry on `std.net.connect`,
`std.fs.read_file`, … — none of which are callable names in this language. The
nine unit tests passed because they wrote the `std.`-prefixed form literally.
`grep -rn "needs "` across the whole corpus returned zero uses.

**AR-F11 — effects: `main` was exempted from error propagation and bound the
`Result` tag as the value.** A fallible call bound in `main` printed `0`/`1` —
the `Ok`/`Err` discriminants read as `i64` — and exited 0.

**AR-F12 — tooling: `jinn run` served a stale binary after a dependency-only
edit.** No warning, exit 0. The cache key was the entry file's bytes plus
compiler identity; dependency modules were never hashed. Companion: a
stale-but-newer `.jni` made the compiler accept a type-incorrect program, and a
*fresh* `.jni` broke working builds because `to_decls()` never reconstructed the
functions list.

**AR-F13 — security: the package-cache directory was derived from the
dependency URL with no sanitization.** `require('evil',
'https://localhost:1/../../../../TRAVERSAL_CANARY', '1.0.0')` created a
directory outside the cache root. The clone only failed because the host
refused connections; an attacker-hosted repo writes a full attacker-controlled
tree at an arbitrary path. Transitive dependencies are resolved recursively, so
the URL need not be one the user typed.

**AR-F14 — security: integer overflow on an attacker-controlled `.store` header
overflowed the heap.** `malloc((size_t)(count * rec_size))` with only
`count < 0` checked, and the following `fread` streamed the file's remaining
bytes into the undersized buffer before the short-read check. A patched count
field reproducibly yielded `free(): invalid size`, exit 134.

**AR-F30 [found during remediation] — passing a struct that owns a `Vec`
double-freed it.** Parameter ownership was decided before inference resolved the
parameter's type, so an unannotated parameter whose type was still a variable
defaulted to `Owned` and the callee emitted drop glue for a value the caller
still owned.

**AR-F31 [found during remediation] — actors silently dropped queued messages at
process exit.** A program that spawned an actor, sent it messages and never
`stop`ped it printed nothing in roughly one run in five, at *both* `--opt 0` and
`--opt 3`. Actor coroutines are daemons, so `jinn_sched_run` returned without
waiting, and nothing closed the mailboxes or drained them.
`docs/concurrency.md` promises messages enqueued before shutdown are delivered.

**AR-F32 [found during remediation] — `Map of String` was structurally
impossible.** The map entry reserved 8 bytes for the value; a `String` is a
24-byte SSO handle. Map values were also always loaded as `i64` regardless of
the declared value type, which is why `logging` and `url` failed LLVM
verification.

## P1

**AR-F15 — "Value"-category structs aliased instead of copying, and a passing
test pinned the wrong behavior.** `docs/memory-model.md` §1 says a struct of
scalars and `String` is category **Value** — "deep copy; both live,
independent". `q is p; q.x is 99` left `p.x` at 99.
`tests/crash_safety.rs::aliased_heap_write_is_visible_through_both_names`
asserted that output and passed.

**AR-F16 — writes to a struct parameter's field behaved inconsistently in and
out of loops.** Investigation showed the *documented* rule is the opposite of
the finding's original framing: `docs/access-semantics.md` §4.1 makes a
POD-struct parameter `Owned` — an independent copy — so the mutation reaching
the caller was the defect, not the mutation being lost.

**AR-F17 — `jinn fmt` destroyed code, and `--write` did it silently with exit
0.** Across 652 corpus files: 116 formatted to output `fmt` itself could not
re-parse, 17 more still parsed but failed type-check, 2 were non-idempotent with
a sign flip per pass. `docs/fmt.md` documented `--check`/`--diff`/`--stdin`, ~40
lint rules, a `jinnc lint` subcommand and five conformance test files, none of
which exist.

**AR-F18 — false claims on the stable surface.** 79 testable claims audited: 55
true, 23 false, 1 unverifiable. The ones that bite: `docs/jinn.md` called
`.length` a byte count (it is the Unicode scalar count); called an `alias`
"interchangeable with" its target (it is a distinct type to the checker);
claimed both quote styles support interpolation (double-quoted strings were
raw); `docs/memory-model.md` claimed every §7 row was "enforced and pinned by
conformance tests" and cited two tests that do not exist;
`docs/structured-concurrency.md` said "Not yet implemented" for a feature that
works; ~14 stale file references, notably `src/perceus/mir_perceus.rs`.

**AR-F19 — two of 36 benchmark comparisons are not apples-to-apples.**
`coroutine_spawn`: Jinn runs 100,000 spawns, C runs 1,000,000, and `results.csv`
reports Jinn 100× faster; corrected it is ~0.1×. `sim_for`: Jinn does 200,000
iterations of dead arithmetic while C spawns 1,000 pthreads running `fib(28..32)`
kept live — and `sim for` currently lowers to a sequential loop, so the row
measures neither parallelism nor work. The harness never verifies cross-language
output.

**AR-F20 — anything past the typer has no span and leaks internals.** Every type
error was prefixed with the internal stage name (`hir:`, `hir-validate:`).
Undefined variables were caught only in codegen and emitted `Load of undefined
variable 'y'` with no file, line or column — and `--emit-hir` exited **0** on
that same program, so the repo's own recommended fast frontend check passed
broken code. Other spanless internals reached users, including a mangled name
(`unknown method 'Box_i64_get_it'`). Grade from 22 beginner programs: C−.

**AR-F21 — generics are largely unusable outside one narrow syntactic path.**
The same generic struct had three mutually non-unifying spellings; only
`Box of i64(42)` worked. Methods on generic types never emitted a body.
Return-position-only and phantom type parameters could not be used, with no
turbofish to escape.

**AR-F22 — `take` was a stolen identifier.** `*take(x as i64) returns i64 is x * 2`
was accepted; `take(21)` printed **21**, because the call site parsed as the
`take` access modifier.

**AR-F23 — `<` on Strings compiled and returned nonsense.** `"a" < "b"` was
false and `"a" < "a"` was true — irreflexivity violated. Sorting a
`Vec of String` inherited this.

**AR-F24 — store transactions are neither atomic across a crash nor isolated
between tasks.** A `kill -9` mid-`transaction` left 10/28/46/66 of 100 rows
durably present when the block never committed. One task's rollback restored a
whole-file snapshot, permanently deleting another task's already-committed rows.

**AR-F25 — post-crash index/base disagreement, and an inflated `count` header
served uninitialized heap as records.** 64 of 200 kill points left indexed
lookups returning wrong rows while scans were correct, with recovery reporting
success.

**AR-F26 — `jinn bind` output is not valid Jinn.** On `/usr/include/errno.h` it
emits `//` comments (Jinn uses `#`), inlines C block comments into signatures,
and binds macros as functions. Output is rejected by `jinnc` at line 1, so there
is no path from a real C header to usable externs.

**AR-F27 — LSP UTF-16 positions are treated as byte offsets.** Hover and
goto-definition silently stop working to the right of any emoji or accented
character in every real editor.

**AR-F28 — `embed` reads any absolute or `../` path at compile time, ungated.**
A downloaded package containing `data is embed '/etc/hostname'` bakes those
contents into the user's binary. Compile-time information disclosure, no
capability required.

**AR-F29 — the `! E` surface is still unusable.** A bare `! E` signature ICEs.
`! E` is a parse error in a trait method signature, so no trait method can be
fallible; an impl may silently widen the error row past the trait's declaration.
The quaternary `? … !!` false-rejects on any *method* call.

## P2

Silent lossy narrowing through `as`-annotations (`u as u8 is 300` → 44, no
diagnostic, while `as strict` exists); int→float coercion works only at argument
position and its diagnostic reads backwards; stack overflow on flat `1 + 1 + …`
chains of ~5000 terms while parenthesized nesting is correctly depth-limited;
UTF-8 BOM rejected with a mojibake message; duplicate catch-all function clauses
silently let the last win, contradicting documented first-match order; `jinn init
NAME` scaffolds into the current directory rather than `NAME/`; ALL_CAPS
constants documented as "cannot be reassigned" are silently shadowed; safe code
can construct invalid-UTF-8 Strings via `chr`; `JINN_WAL_SYNC=group` never syncs
outside a transaction; a WAL with bad magic calls `abort()` for a condition its
own message calls recoverable; a committed ELF binary and release tarball under
`apps/alpha_release_demo/`; cancellation latency varies 13.7s–30.0s for a
deterministic workload.

## Appendix — speculative (no reproduction achieved)

- **Comptime purity classifier is unsound but unreachable.**
  `src/comptime/purity.rs` decides `Call` purity from its *arguments*, never
  consulting the callee. `eval_expr` returns `None` for every I/O node, so
  evaluation bails before the impure leaf. Latent, not live — any widening of
  `eval_expr` makes it live immediately.
- **`store_load_forwarding`'s clobber set omits `IndirectCall`/`ClosureCall`/
  `SpawnActor`/`InlineAsm`.** No Jinn program could be written that mutates a
  caller local through a closure.
- **Nested-field bind aliasing → double-free.** `v is o.inner.xs` aliases
  silently and read-after-move is allowed, but the owner always dropped last in
  the shapes that could be built.
- **All `kill -9` testing was on tmpfs**, where `fsync` is near a no-op. These
  sweeps validate application-level crash consistency, not device-level
  durability.
- **`jinn_txn_track_impl` snapshots the entire store file per transaction**
  (bounded by 256 MB, then `abort()`). Not measured.

## What the review did not cover

- `ci/sanitize.sh` was never run by the lead reviewer; the runtime reviewer
  completed both sweeps (9 programs under ASan, 6 under TSan) and found no
  genuine bug, but whole-program TSan is not yet meaningful without
  `__tsan_switch_to_fiber` annotations.
- Device-level durability; power-loss and write-cache-loss behavior.
- Store subsystems: vector/nearest-neighbour, `@versioned` history,
  `@kv`/`@column`/`@graph`/`@cascade`, migrations end-to-end.
- Compiler flags `--lto`, `--target`/`--cpu`/`--features`, `--fast-math`,
  `--deterministic-fp`, `--standalone`. `--opt 2` was only spot-checked.
- Package commands beyond `fetch`; `jinn test`'s harness output.
- `docs/lamp.md`, `docs/libjn.md` and six other docs were read only for status
  disclosure.
- `libjn/` and the trait system beyond `tests/traits.rs`'s surface.
