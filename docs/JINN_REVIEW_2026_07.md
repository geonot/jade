# Jinn: Critical Review

**Date:** 2026-07-29
**Reviewed at:** commit `f82402f` (branch `main`, 253 commits)
**Scope:** language design, compiler (`src/`, 71,271 lines Rust), C runtime (`runtime/`, 7,621 lines), standard library (`std/`, 11,862 lines Jinn), persistence layer, tooling, benchmarks.
**Method:** built the compiler from source, ran the full `cargo test` suite, ran every app in `apps/`, ran all 400 snippets in `snippets/` and all 89 programs in `tests/programs/` in isolation, compiled and timed 10 benchmark pairs against `gcc -O3`, and wrote ~60 original Jinn programs to probe semantics, inference, memory safety, concurrency, and the store. Findings below are from observed behavior and read code; the pre-existing review documents and commit messages were deliberately not used as evidence.

---

## 1. Verdict

Jinn is a real compiler, not a toy. There is a genuine four-stage pipeline (AST → HIR → MIR with basic blocks and phis → LLVM 21), a legitimate union-find type inference engine, a hand-written stackful-coroutine runtime with correct Chase-Lev deque memory orderings, and an unusually serious execution-based test suite (1,613 test functions in `tests/`, 318 in `src/`). The scalar code it emits is competitive with C, and on several benchmarks it beats `gcc -O3`. The surface syntax is coherent enough that I could write non-trivial programs after an hour with the docs.

But the three headline claims do not currently hold, and one of them fails on a six-line program.

| Claim | Status |
| --- | --- |
| Performance of C | **Substantially true** for scalar code. Measured 0.34×–2.59× vs `gcc -O3` across 10 benchmarks (median ≈ 0.97×). This is real and it is the project's strongest asset. |
| Safety of Rust | **False today.** A 6-line program that returns a `Vec` parameter corrupts the heap. Two `dispatch` bodies mutating a shared `Vec` race and abort the allocator on every run, with no diagnostic. There is no borrow checker, and `Vec` assignment is silent shared mutable aliasing. |
| Ease of Python | **Partially true, undermined by inference gaps.** The syntax is pleasant, but you cannot write `*bsort(v)` — an unannotated parameter with a method call on it fails to infer. Three of the repo's own 400 snippets fail on exactly this. |
| Complete type inference | **Overstated.** Inference is local and defaults aggressively to `i64`; method calls on unconstrained parameters have no principal type and are rejected (or silently defaulted with `--lenient`). |
| Persistence layer obviates the database stack | **No.** The WAL is the only component in the layer that calls `fsync` at all. Every other store rewrite is truncate-in-place. Multi-row iteration segfaults. |
| First-class actor system | **Mostly real** for the message-passing core; supervision is unsynchronized and `select` cannot correctly wait on more than one channel. |

The gap between the documentation's confidence and the implementation's state is the single biggest risk to this project. `docs/error-effects.md` opens with "**Status: fully implemented and conformance-tested**"; `docs/jinn.md` states "the language prevents use-after-free, double-free, and data races." Both statements are contradicted by programs in §3 below. Pre-alpha software is allowed to be incomplete — it is not allowed to describe itself as finished, because that is what stops the holes from getting fixed.

---

## 2. What is genuinely good

These are not consolation prizes; they are the parts worth building on.

**Codegen quality is the real achievement.** Jinn beat `gcc -O3` on 3 of 10 benchmarks and matched it on 3 more (§9). The generated IR is clean: for `*fib`, LLVM applied an accumulator transformation and tail-call optimization through Jinn's lowering (`fib__G_i64` is `nounwind memory(none)` with a self-recursive loop), which means the front end is emitting IR that LLVM can actually reason about rather than opaque pointer soup. There is no interpretive tax on the hot path.

**Bounds checks are sound where they are elided.** `for` loops emit `IndexUnchecked` but re-read `VecLen` in the loop header on every iteration before the compare (`src/mir/lower/loops.rs:228-245`), so shrinking a vector mid-iteration cannot walk off the end. That is the correct design, and `tests/mir_bounds_elision.rs` locks it in.

**MIR is a real IR with a real verifier.** `src/mir/verify.rs` is 603 lines and runs after both lowering and optimization in debug builds. The MIR opt pipeline is deliberately scoped to passes LLVM cannot do (`src/mir/opt/mod.rs:22-28`) — notably `inject_yields`, which inserts cooperative scheduler yields at loop back-edges. Injecting that at MIR level rather than hoping LLVM cooperates is exactly right, and it is correctly classified as a correctness pass that runs at every optimization level.

**The channel park protocol is subtle and correct.** A coroutine parking on a channel keeps the channel spinlock held *across* the context swap, and the scheduler releases it only after the context is fully saved (`runtime/channel.c:239-244` + `runtime/sched.c:213-222`). This closes the "resumed before saved" race — two threads executing on one coroutine stack — which is the hardest race in an M:N design. Whoever wrote this understood the problem deeply. (§7 covers the three other park sites where the same protocol was not applied.)

**TLS-staleness discipline is documented and enforced.** `jinn_worker_self()` and `jinn_scope_current()` are `noinline` accessors with contracts explaining that caching `tl_worker` across a swap is invalid because migration leaves a stale address in a callee-saved register (`runtime/jinn_rt.h:193-212`). Channel, actor, and scope loops re-derive per iteration.

**Crash diagnostics are better than most production runtimes.** `runtime/signals.c` installs per-thread `sigaltstack`, uses only async-signal-safe writes, caches stack bounds outside the handler, distinguishes coroutine guard-page overflow from native stack overflow, and re-raises to preserve core dumps.

**No homegrown crypto.** `runtime/crypto.c` is a thin OpenSSL EVP wrapper; GCM decrypt sets the tag before final and `OPENSSL_cleanse`s partial plaintext on auth failure. The TLS client does real verification (`SSL_VERIFY_PEER` + `X509_VERIFY_PARAM_set1_host` + SNI). `random.c` is `getrandom` → `/dev/urandom` → fallback. Shell execution in `process.c` is off by default behind `JINN_ALLOW_SHELL=1`. Someone made good security calls here.

**Actors have backpressure by default.** Mailboxes are bounded channels; senders park rather than growing an unbounded queue. `stop` is stop-and-drain, and `join` waits on a one-shot latch whose lazy creation is published with a correct CAS (`runtime/actor.c:88-104`). The semantics documented in `docs/concurrency.md` matched what I observed: `stop s` followed by `join s` reliably drained 100 queued messages and printed the correct total.

**Store persistence and constraints work for the single-writer basic case.** Across process restarts, a store correctly reloaded its rows, `count` was accurate, `@unique` rejected a duplicate insert, and the `? / !!` handler arms produced the documented `Result of i64, StoreError`. The schema fingerprint guard aborts with a genuinely helpful message on mismatch (`runtime/migrate.c:56-92`).

---

## 3. Critical: the memory-safety claim fails on trivial programs

`docs/jinn.md` §"Memory and ownership" states: *"the language prevents use-after-free, double-free, and data races."* All three probes below were compiled with the default settings and no warnings.

### 3.1 Returning a `Vec` parameter corrupts the heap

```jinn
*ident(v) returns Vec of i64
    return v

*main
    a is vec(1, 2, 3)
    s is ident(a)
    log(s.length)
```

```
$ jinn run m.jn
free(): invalid pointer          # exit 1
```

This reproduces at every optimization level (`--opt 0` through `--opt 3`), with the parameter annotated (`v as Vec of i64`) or inferred, and with `return v` or a bare tail expression `v`. A variant that drops the trailing `log` produces `SIGSEGV at 0x000000056141d9c8` instead — i.e. the corruption is nondeterministic in its expression, which is the signature of a genuine double-free rather than a bad free of a known-bad pointer.

The mechanism is visible in the source: a bind whose value is a plain variable emits no clone (`needs_auto_clone`, `src/mir/lower/mod.rs:616-621`), and drop obligations are tracked by a syntactic consumed-set scan that does not recurse into nested statement bodies (`collect_block_consumed_ids`, `src/typer/lower/block.rs:164-190`, with `_ => {}` at line 187). Both the parameter and the returned binding end up owning the same buffer and both get dropped.

**This is not an exotic corner.** It is how every sort, filter, or transform helper is written. The repo's own merge-sort snippet (`snippets/101-200/s107.jn`) dies with `free(): invalid pointer`, and I minimized it to the seven lines above.

`jinn check` reports `check passed` on this program.

### 3.2 Concurrent mutation of a shared `Vec` races, every run

```jinn
*pusher(v, base)
    for i in 0 to 20000
        v.push(base + i)

*main
    shared is vec()
    together
        dispatch
            pusher(shared, 0)
        dispatch
            pusher(shared, 1000000)
    log(shared.length)
```

```
$ jinn run race1.jn     # 3/3 runs
realloc(): invalid old size      # exit 1
```

No compile-time diagnostic. The only cross-thread safety check in the typer rejects `@resource`-annotated types (`enforce_cross_thread_safe`, `src/typer/mod.rs:603-618`); everything else relies on a move discipline that does not model aliasing. Rust's entire value proposition is that this program does not compile. In Jinn it compiles clean and corrupts the allocator deterministically.

### 3.3 `Vec` assignment is silent shared mutable aliasing

```jinn
*main
    a is vec(1,2,3)
    b is a
    b.push(4)
    log(a.length)    # 4
    log(b.length)    # 4
```

Mutating through `b` is visible through `a`. The same holds for a field read (`inner is b.items; inner.push(4)` mutates `b.items`) and for passing a vector to a function that mutates it. So the actual semantics are *shared mutable references with no borrow tracking* — neither the value semantics the docs describe ("Reading a value borrows it without copying") nor the single-owner move semantics the ownership verifier assumes. Strings behave differently: `*ident(s as String) returns String` is fine, because strings are 24-byte SSO values that get deep-cloned. Two different ownership models coexist for two different builtin types, and the drop machinery is written against the string one.

### 3.4 The ownership verifier cannot catch these by construction

`src/ownership/verify.rs` is flow-*insensitive* across branches: a move recorded in a `then` block mutates the outer `VarState` in place (`lookup_mut`, `src/ownership/mod.rs:123-130`) and persists into the `else` branch — there is no snapshot/merge at `verify_stmt`'s `If` arm (verify.rs:58-68). Loops get a single pass with no fixpoint (verify.rs:69-95), so a loop-carried move is missed. Borrow counts increment but never release (`record_borrow`, ownership/mod.rs:148-183). Ordinary call arguments are never treated as moves (verify.rs:233-241).

Worse, there are **two overlapping half-checkers**: the typer has its own, better move machinery with snapshot/restore and branch-union merge (`src/typer/mod.rs:488-514`), but it only applies to explicit `take` expressions. The HIR verifier is the one that sees everything, and it is the weaker of the two.

Separately: `src/codegen/rc.rs` is 250 lines of retain/release/atomic-RC with **zero call sites outside its own file**. The "Perceus" machinery in `src/perceus/mir_perceus.rs` is drop *placement* (elision, sinking, fusion, malloc-reuse tokens), not reference counting. One upside: because there are no shared reference counts, reference cycles cannot be constructed, so cycle leaks are vacuously absent.

### 3.5 What this section costs

Pick a memory model and enforce it. The current state is the worst of both: value semantics in the drop code, reference semantics at runtime for `Vec`/`Map`, and a verifier too weak to notice. The three programs above should become the first three entries in a regression suite, and `tests/ownership_fuzz.rs` should be extended until it generates §3.1 on its own — that it passes today while a 7-line program corrupts the heap means the fuzzer is not exploring returns of aggregate parameters.

---

## 4. Type inference: local, defaulting, and not complete

The unifier itself is respectable: union-find with rank and path compression (`src/typer/unify/mod.rs:278-287`, 579-598), an occurs check (443), kinded constraints (Numeric/Integer/Float/Addable/Trait) with a real merge lattice (170-216), and error messages that carry constraint origins and suggest fixes (289-388). Function typing is ordered by Tarjan SCC over the call graph so mutually recursive groups check together (`src/typer/lower/mod.rs:334-414`), with a "in mutually recursive group with: …" note. Strict mode is on by default and turns ambiguity into a hard error with usage-site notes.

But "complete type inference" is not what ships.

### 4.1 The canonical generic function does not infer

```jinn
*bsort(v)
    n is v.length
    ...
            t is v.get(j)
```

```
ambiguous type: cannot infer type for this expression (unresolved method-call return type)
  help: consider adding a type annotation, e.g. `: i64` or `: String`
```

An unannotated parameter is promoted to an implicit generic (`src/typer/lower/mod.rs:52-61`), and then `v.get(j)` has no principal return type because nothing constrains what `v` is. There is no trait bound, row polymorphism, or structural constraint to express "any type with a `get`". So the rule in practice is: **an unannotated parameter may not have methods called on it.** Annotating `v as Vec of i64` fixes it immediately.

This is not a corner case either — it is `map`, `filter`, `sort`, `reverse`, and every container helper. Of the repo's 400 snippets, 8 fail in isolation and 5 of those 8 fail on exactly this pattern (`s123` reverse, `s168` bubble sort, `s090` float methods, `s045`, `s094`). `std/random.jn` itself fails to type-check in strict mode (`std/random.jn:155:19`, `167:15`) — a shipped stdlib module is not strict-clean.

### 4.2 The suggested fix-it is not Jinn syntax

`help: consider adding a type annotation, e.g. \`: i64\` or \`: String\`` — Jinn annotates with `as i64`, not `: i64`. The diagnostic tells the user to write something the parser rejects. Small, but it is the message users will hit most.

### 4.3 `i64` defaulting is pervasive

Unsolved variables default to `Type::I64` in `resolve_core` (`src/typer/unify/resolve.rs:130-133`), in `default_quantified_vars` (unify/mod.rs:83-95), and at roughly 25 `unwrap_or(Type::I64)` / `or_insert(Type::I64)` sites across the typer. Strict mode converts some of these to errors, but `resolve_core` still *returns* i64 while pushing the strict error (resolve.rs:254), and `--lenient` reverts to silent defaulting with a warning. Enum variants monomorphize unbound type parameters to i64 (mono.rs:639). Coroutine yield-type inference falls back to i64 when no `yield` is found (`src/typer/infer.rs:8-18`).

### 4.4 Trait solving is string-based and collapses integer widths

Trait impls are stored as `IndexMap<Symbol, Vec<String>>`, and bound checks go through `type_name_for_bound_check`, which collapses `i8`/`i16`/`i32` → `"i64"` and `f32` → `"f64"` (`src/typer/mono.rs:645-668`). An impl on `i64` therefore satisfies a bound for `i32`. Unifier-side trait checking silently passes when `trait_impls` is empty (unify/mod.rs:475-478).

### 4.5 Name mangling can collide, and `From` is resolved by string pattern

Two inconsistent schemes coexist: `mangle_generic` produces `base__G_<Type>` with `_`→`U` escaping (mono.rs:235-252), while struct monomorphization produces `"{base}_{suffix}"` with no escaping (mono.rs:317-322). A user struct named `Box_i64` collides with `Box of i64`. Error conversions are looked up purely by the name pattern `"{tgt}_from_{src}"` (`src/typer/errset.rs:161-163`), so a user function that happens to be named that is silently treated as a `From` impl.

### 4.6 Soundness holes I could trigger from source

```jinn
*main
    if 'abc' equals 5        # String vs i64
        log('huh')
```
→ `jinn runtime: SIGSEGV at 0x00007fe0f501dde8`. A cross-type `equals` is accepted by the typer and segfaults at runtime.

```jinn
*f(x as i64) returns String
    x + 1                   # returns i64 where String is declared
```
→ compiles and runs silently (exit 0). The declared return type is not checked against the body here.

```jinn
*main
    x is 'abc' + 1
    log(x)                  # prints "abc"
```
→ `String + i64` is accepted and silently discards the operand.

A heterogeneous `vec()` also type-checks: `v.push(1); v.push('two')` compiles, and reading element 1 as an integer prints `7305076` (a leaked pointer value). Reversing the order (`push('hello'); push(42)`) and calling `.to_upper()` on element 1 produces empty output with exit 0.

Also, `occurs_in` and `resolve_core` do not recurse into `Struct(_, args)` (resolve.rs:26-51, 53-82 — `Struct` falls to `_ => ty.clone()`), so `?v ~ Struct(n, [?v])` passes the occurs check, and struct type arguments can carry unresolved variables past resolution. `Struct(n, _) ↔ Enum(n)` unifies unconditionally on name match with generic arguments *explicitly not unified* (unify/mod.rs:555-564).

### 4.7 A user-reachable compiler panic

```jinn
*f(0) is 0
*f a, b is a + b
```
→ `thread 'main' panicked at src/parser/mod.rs:514:13: multi-clause function 'f' clause 2 has 2 parameters, but first clause has 1`. A correct diagnostic delivered as a Rust panic with a backtrace hint. Note also that the process exits **0** in this path.

---

## 5. Language design

### 5.1 What works well

Indentation with word operators genuinely reads well. `if x equals 0`, `a and not b`, chained comparison `0 < x < 100`, `x in [1,2,3]`, and `for i in 0 to 100 by 2` all did what they look like they do. The `~` pipeline with `$` positional substitution is clean. Pattern clauses (`*fib(0) is 0`) work and are charming. `err` for both declaration and raise is disambiguated grammatically without ceremony, and it reads naturally. String interpolation with single quotes and raw with double quotes is a defensible split, and `log('x={x}, doubled={x * 2}')` worked including expressions.

The quaternary `e ? ok ! nothing !! err` is the most interesting design decision in the language. In practice `x is may_fail() !! -1` and `may_fail(5) ? log($) !! log('failed')` both read better than Rust's `unwrap_or` / `match`, and implicit propagation really does make the happy path free of ceremony. When it works, it is the best thing in Jinn.

### 5.2 Sigil overloading is at its limit

`!` means: ternary else-arm, quaternary empty-arm, error-union declaration (`returns i64 ! E`), and — until recently — a raise. `?` means: ternary then-arm, quaternary success-arm, and match-arm separator. `@` means: pointer dereference, actor async handler, and store field decorator (`@unique`, `@index`). `%` means: modulo, and address-of. `*` means: function declaration, actor sync handler, and multiplication.

Every individual case I tested parsed correctly, so this is not a soundness complaint. It is a legibility complaint: `*loop 100` (a sync handler named loop with a sleep) and `x is a % b` next to `p is %a` are asking a lot of a reader, and it costs the language its best feature — that code looks like what it means. The four meanings of `!` are the ones I would revisit first.

One concrete gap: docs say ternaries "can nest (they associate to the right)", but nesting in the *then* position fails to parse — `true ? false ? 1 ! 2 ! 3` gives `line 5:26: unexpected token: ?`. Only nesting in the else position works, which is what the doc's example happens to show.

### 5.3 Compiling one file silently absorbs the whole directory tree

`jinn run foo.jn` calls `merge_source_files(&mut prog, base_dir, &input_canon)` (`src/driver/pipeline.rs:53`), which recursively collects **every `.jn` file under the containing directory** (`collect_jinn_files`, `src/driver/sources/modules.rs:229-239`) and merges their declarations into the program. Additionally, any bare identifier in a `x.y` position that is not locally defined is treated as a module name and, if a file with that stem exists anywhere in the scanned index, that file is parsed and flattened in too (`resolve_implicit_imports`, `src/driver/sources/implicit.rs`).

Observable consequences:

- Parse and type errors from unrelated sibling files appear as `warning:` lines when compiling your file.
- The repo's own `snippets/guide_tour.jn` **fails to compile in place** with `operator '-' not defined for 'i64' and 'Vec of ?509' (line 18)` — line 18 is `log('x={x}, doubled={x * 2}')`, which has no `-` in it. Copy the identical file to an empty directory and it compiles and runs correctly. The error is caused by other files in `snippets/` and misattributed to a line in the target file.
- I reproduced the misattribution class directly: a directory with truncated copies of the same program produced `type 'Vec3' has no field 'x'` for a `Vec3` that plainly declares `x as f64`.

This is a correctness problem, not just ergonomics: what your program means depends on what else is in the directory, and the resulting diagnostics point at the wrong file and line. Module resolution should be driven by explicit `use` plus the project manifest, and single-file compiles should compile a single file.

### 5.4 Store queries: silent zeros and a segfault

A query that matches nothing returns a zero-initialized record rather than an `Option`:

```jinn
insert users 'Alice', 30
missing is users where age > 100
log('name=[{missing.name}] age={missing.age}')     # name=[] age=0
```

There is no way to distinguish "no such user" from "a user named empty-string aged 0". For a layer whose pitch is replacing the database, returning a fabricated row for a miss is the wrong default — this should be `Option of Record`, and the compile-time query checking that the docs advertise is exactly the leverage needed to make that ergonomic.

Iterating all rows crashes:

```jinn
store users
    name as String
    age as i64

*main
    insert users 'Alice', 30
    insert users 'Bob', 25
    for u in all users
        log(u.name)
```
→ `jinn runtime: SIGSEGV at 0x0000000000000051`. `grep` finds no test covering `all <store>` iteration.

`docs/store-improvement.md:48` already identifies the root issue honestly ("can't iterate, can't `.name` it, can't pass it to generic code") — a query result is not a first-class value. That is the right diagnosis and it should be the next store work item, ahead of features.

---

## 6. Compiler engineering

**Test coverage is a real strength.** 1,613 test functions in `tests/` and 318 in `src/`, including 104 typer unit tests asserting HIR types and instantiation, 25 unifier tests, property tests with committed proptest regressions, an ownership fuzzer, crash-safety tests, and ~453 compile-and-run integration assertions. For pre-alpha this is far above the norm.

**The suite is currently red at HEAD.** `cargo test --release` fails 2 tests: `ebnf_has_no_dangling_rule_references` and `ebnf_keywords_are_reserved_in_lexer`, both with `jinn.ebnf must exist: NotFound`. The last commit (`f82402f "reorganize docs"`) moved `jinn.ebnf` into `docs/` without updating `tests/ebnf_roundtrip.rs:35`, which reads `repo_root().join("jinn.ebnf")`. So the grammar-drift detector — the mechanism that keeps the parser, the EBNF, and the tree-sitter grammar in sync — is disabled, and was committed that way. Everything else passes (39 of 41 test binaries green).

**`tests/programs/` has rotted.** 16 of 89 programs fail to compile or run in isolation: `binary_search`, `clause_fns_ext`, `compiler_pipeline`, `data_structures`, `dispatch_fib`, `dispatch_multi`, `ecs`, `embed`, `error_handling`, `interpreter`, `json_parser`, `linked_list`, `sim_for_test`, `state_machine`, `store_index`, `syntax`. **None of the 16 is referenced by any test** — `tests/integration.rs` has only 21 `expect_file` call sites for 89 programs. The suite is green partly because broken programs fell out of the harness rather than being fixed. Two of the failures are codegen ICEs leaking to users: `FieldGet on pointer to unknown struct type for field '__tag'` (`linked_list`) and `GEP index out of range` (`json_parser`). `error_handling` still uses the removed prefix-`!` raise.

**Type errors can escape as raw LLVM verifier output.** `add('one', 2)` against `*add(a as i64, b as i64)` produces:

```
Call parameter type does not match function signature!
  %slit3 = load %String, ptr %slit, align 8
 i64  %call = call i64 @add(%String %slit3, i64 2)
```

A basic argument-type mismatch reached codegen and was caught by LLVM's module verifier. Users should never see LLVM IR.

**`src/diagnostic.rs` is entirely dead code.** It defines a rustc-style `Diagnostic` with severities, error codes E001–W202, labels, notes, suggestions, and a caret-rendering `render()` (diagnostic.rs:61-184). Nothing in the compiler constructs one. Every real error is a `format!` String: the typer accumulates `type_errors: Vec<String>` (`src/typer/mod.rs:101`), ownership diagnostics are `eprintln!("ownership: {} (line {}): {}")` (driver/mod.rs:474), parse errors are newline-joined into a single blob (parser/mod.rs:86-106). Formats differ per subsystem: `line N:C:` from the parser, `file:N:C:` from the typer, `(line N)` from ownership. Some messages embed Rust `Debug` spans (`errset.rs:192`).

**The pipeline is implemented twice and has drifted.** `driver/mod.rs::run` (~640 lines) and `driver/pipeline.rs::compile_and_link` (315 lines) both implement the compile flow. `pipeline.rs` runs comptime folding before ownership and **never runs `HirValidator`**, while `mod.rs` validates HIR at line 437. So `jinn build` skips a verification layer that direct-file compiles have.

**`jinn check` is weaker than `jinn build`.** It type-checks only (`driver/mod.rs:249-280`) — no ownership verification, no HIR validation, no MIR verify. It reports `check passed` on the §3.1 heap-corrupting program.

**Incremental compilation is aspirational.** `incr.rs` has a `StableHasher`, per-function keys, and `compute_dirty_set` — but both call sites only *log* the dirty count (driver/mod.rs:532-545), the program is fully recompiled regardless, and `ArtifactCache::store` is never called outside its own unit test, so `lookup` can never hit. If it were wired up it would be near-useless as designed: `function_cache_key` mixes in the signatures of *every* function in the program rather than actual dependencies, and `hash_stmt` hashes `format!("{:?}", stmt)` including spans, so inserting a blank line dirties everything below it. Compile times are currently fast enough (64 ms for a small program, ~1 s for an app) that this is not urgent — but it should be marked as unimplemented rather than half-present.

**Comment history has been scrubbed.** Exactly **0** TODO/FIXME/HACK/XXX comments across 71,271 lines, alongside conspicuous blank gaps where comments were removed (`src/codegen/rc.rs:36-39`, `200-249`) and a leaked reference to an external planning file in a doc comment (`src/mir/opt/mod.rs:24`). Combined with `.ryu/` task files and commit subjects like `[131] task 2-6-5-3`, the effect is that known-incomplete work is invisible in the code and legible only in commit archaeology. Since the aspirational-vs-real gap is this project's main risk, removing every in-code marker of incompleteness is precisely the wrong hygiene rule.

**`jinn fmt` destroys comments, in place.** The lexer discards `#` comments entirely (`src/lexer/mod.rs:196-197`; there is no `Comment` token), `format_source` re-prints from the AST (`src/fmt.rs:5-11`), and the driver overwrites the user's file when output differs (driver/mod.rs:304-319). Verified:

```jinn
# Top-level comment explaining the module
*main
    x is 1    # trailing comment
    # standalone comment
    log(x)
```
after `jinn fmt`:
```jinn
*main
    x is 1
    log x
```
Three comments irrecoverably deleted. This is the most user-hostile bug in the repo and it should be fixed or the subcommand removed today.

**The LSP is syntax-only.** `src/lsp/analysis.rs` uses only the lexer and parser and never invokes the typer, so there are no type-aware diagnostics, hover, or completion despite the compiler having all the machinery.

---

## 7. Runtime and concurrency

The core is more sophisticated than expected, and the flaws cluster in a recognizable pattern: a hard race was diagnosed and correctly fixed in one place, and the same fix was not applied to the three structurally identical places.

### Critical

**Three park sites publish a coroutine before its context is saved.** The channel path solves this with lock handoff (§2). The other three do not: `select` calls `unlock_all` at `select.c:273` before swapping at `:283`; actor join calls `join_unlock` at `actor.c:147` before swapping at `:149`; scope join publishes `s->parent = self` and calls `scope_unlock` at `scope.c:272` before swapping at `:274`. In each window another worker can pick the coroutine up and swap into a `ctx` holding stale registers from a previous park, while the original thread is still executing on that stack. This is the same bug class the channel fix closed, and it matches a project history of rare unreproducible crashes.

**`select` can only wait on one channel at a time.** The comment at `select.c:227-234` admits it: a single intrusive `next` pointer means the selector enqueues on one channel per attempt, round-robin. So `select { receive a; receive b }` parked on `a`'s waitq is never woken when only `b` becomes ready — round-robin only helps if the selector keeps getting woken, and a parked selector rotates nothing. This is a hang. When it *does* wake repeatedly, the `max_retries = 256` bound (`select.c:242`) prints "possible deadlock" and returns −1, which the caller interprets as *the default case fired* (`select.c:334-338`) — silently wrong semantics.

**`select` never checks channel close.** There is no `closed` test anywhere in `select.c` — not in the readiness scans, not after wake. A select-receive on an already-closed empty channel parks forever, because close wakes only the waiters present at close time. `jinn_chan_recv` handles this correctly (`channel.c:283-288`); select does not. Select-send to a closed channel silently reports success.

**Scope `children[]` holds dangling pointers.** Children are appended at registration (`scope.c:92-94`) and never removed, but `sched.c:225-238` calls `jinn_coro_destroy(c)` when a child exits. A later `jinn_scope_cancel` reads `snapshot[i]->cancelled` and `->wait_chan` (`scope.c:159-167`), and `scope_wake_cancelled` reads `c->state` and may enqueue a freed coroutine into the run queue (`scope.c:240-246`). Any `together` block where one child errors after siblings have completed hits this. Also, more than 64 children are counted but not registered, so cancellation silently misses them.

**Stale `tl_gen_coro` mis-identifies the next fresh coroutine as a generator.** `jinn_gen_resume` sets `tl_gen_coro = c` on every resume (`coro.c:277`) but it is cleared only on first trampoline entry (`coro.c:171-174`). Resumes 2..n swap into the middle of `jinn_gen_suspend` and never touch the trampoline, leaving the thread's value permanently stale. The next *new* coroutine whose first run lands on that thread sees `gen != NULL`, runs the generator's entry on the wrong coroutine, then spins in `for(;;){}` (`coro.c:179`).

**Chase-Lev grow frees the buffer under concurrent stealers.** `jinn_deque_grow` does `free(dq->buffer)` and swaps `buffer`/`capacity` as two separate non-atomic stores (`deque.c:24-37`) while a thief reads `dq->buffer[t & (dq->capacity - 1)]` (`deque.c:80`) — both a read of freed memory and a torn buffer/capacity pair. Canonical Chase-Lev either leaks retired buffers or reclaims by epoch; it never frees inline. Triggered whenever a worker queues more than 1,024 coroutines. Buffer slots are also plain pointers rather than atomics, a formal data race between `deque.c:45` and `:80`. The *orderings* elsewhere in this file are textbook-correct — release fence in push, seq_cst in pop/steal, correct CAS success/failure orders — which makes the lifecycle bug the odd one out.

**`jinn_actor_destroy` frees a channel that just-woken waiters will touch.** `actor.c:161-171` calls `jinn_chan_close(ch)` — which *enqueues* parked waiters to run later — then immediately `jinn_chan_destroy(ch)`, freeing the channel. The woken coroutines resume inside the send/receive retry loop and spin on `chan_lock(ch)` of freed memory. Reachable from `sup_on_child_exit` (`sup.c:139`) and supervisor restart.

### Major

**The supervisor is completely unsynchronized.** `sup_on_child_exit` runs on whichever worker the child died on; `slot->alive`, `restart_count`, and `mb_ptr` are read, written, and freed with no lock (`sup.c:128-175`). ONE_FOR_ALL iterates sibling mailbox pointers while a concurrent sibling exit may be destroying them — racing double-destroy. Also `JINN_SUP_MAX_RESTARTS = 16` is a lifetime total, not OTP-style intensity-per-period: after 16 restarts, supervision silently stops forever.

**`select` silently truncates to 16 cases.** `int poll_order[16]; limit = n < 16 ? n : 16;` (`select.c:167-169`). Cases 17+ are never polled and never diagnosed.

**Two contradictory SSO string ABIs in one file.** `sso_is_heap()` treats byte-23 bit-7 *set* as heap (`vec.c:55-57`, matching `process.c:27-36`), while `__jinn_str_clone` implements and documents the opposite — bit 7 set = inline — with a comment noting it "differs from the C helpers above" (`vec.c:107-131`). At most one matches codegen.

**`tl_worker` is read across context swaps, against the project's own documented rule.** `jinn_coro_trampoline` caches `w = tl_worker` before calling `entry()` (`coro.c:183`), and `jinn_coro_exit` — `static`, freely inlinable into the trampoline — re-reads `tl_worker` after `entry()` returns (`coro.c:195`), i.e. after arbitrarily many migrations. Safe today only because initial-exec TLS re-derives per access; compiled `-fPIC` into a shared object this becomes the exact stale-register bug `jinn_rt.h:193-212` warns about. Should use `jinn_worker_self()`.

**Blocking I/O pins workers.** `SSL_read`/`SSL_write` (`tls.c:92-100`) and `waitpid` block an entire worker thread; only plain sockets integrate with the event loop. Channel operations from non-coroutine context busy-spin indefinitely (`channel.c:210-222`). No EINTR retry anywhere in `net.c` or `process.c`; `process.c:230`'s `if (n <= 0) break;` means a signal during output capture silently truncates subprocess output.

**FP control state is not preserved across coroutine switches on x86-64.** MXCSR and the x87 control word are callee-saved per the SysV ABI but are not saved or restored (`context_x86_64.S:19-38`), so a rounding-mode change leaks across coroutines. The aarch64 path correctly handles d8–d15. Stack alignment and trampoline setup are correct on both.

### On `sim for`

`sim for` does not parallelize. `src/mir/lower/loops.rs:307-460` lowers it to an ordinary sequential counted loop — same block structure as `for`, no spawn, no scheduler interaction. Measured: an 8-way Collatz workload took 0.640 s with `for` and 0.640 s with `sim for`; a 20 M-iteration body ran in 0.185 s either way; `--emit-mir` shows only `simfor.cond/body/exit` blocks with no spawn instruction. `docs/jinn.md` presents it as "runs iterations in parallel on a work-stealing scheduler."

The upside is that §3.2-style races don't appear via `sim for` (I confirmed `sim for` pushing to a shared vec is stable, because it is serial). But `together` + `dispatch` *does* parallelize — the same Collatz workload went from 1.28 s serial to 0.355 s wall with 1.50 s user time across 8 workers, a real ~3.6× speedup — and that is the path where the data race in §3.2 lives.

---

## 8. Persistence layer

The architecture is reasonable: WAL, rebuildable index, fingerprint-guarded schema, plus `sqlite.c` as an escape hatch. The implementation is a prototype, and the honest one-line summary is that **`fsync`/`fdatasync` appear only in `wal.c`** across the entire layer.

**Everything except the WAL is truncate-in-place.** There are zero `rename()` calls and zero directory fsyncs in the layer. Migrations use `fopen(path, "w+b")` — truncate, then write (`migrate.c:238-240`, `:313-314`, `:380-382`), so a crash between the truncating open and the final write destroys the entire store, in the code path whose purpose is safe schema evolution. `kv_save` rewrites header plus all entries on *every* `set`/`del`/`incr` (`kv.c:93-118`), so a torn write leaves `count = N` with garbage trailing entries that reload as live data — silent resurrection of deleted keys. Bloom persistence is a truncating rewrite at close.

**The WAL has the right skeleton and real holes.** Per-entry CRC32, env-selectable sync policy, group commit, and replay-stops-at-first-bad-entry are all correct design. But: a CRC of 0 bypasses verification entirely (`if (stored_crc != 0 && computed != stored)`, `wal.c:485`) and the malloc-failure path deliberately writes CRC 0 (`:202-204`); `payload_len` is outside the CRC (`:190-199`), so corrupted framing can frame garbage that replays as valid; `fsync` results are discarded (`(void)fsync(fd)` at `:81`, `:88-89`, `:105`), which is the PostgreSQL fsync-gate bug class; there is no directory fsync after creating the log; bad magic causes the log to be silently truncated and recreated (`:145-149`) rather than triggering recovery; and a torn tail is never truncated — replay stops at the first bad entry but open appends at `SEEK_END` (`:143`), so every post-crash append lands after the torn record and is unreachable to all future replays forever.

I probed this empirically. After `kill -9` mid-insert, recovery was consistent and repeatable (6,233 rows). But corrupting 64 random bytes at offset 2,000 of the WAL produced *identical* results to the uncorrupted control — because the WAL is not the source of truth on reopen. Deleting the `.wal` and keeping the `.store` gave the same 6,233 rows; deleting the `.store` and keeping the `.wal` gave **0**. The WAL is write-only in practice; it is not replayed to recover records that the store file lacks. Corrupting the WAL tail with garbage was also silently ignored. So the durability story is "the data file is the database, and the WAL is a log nobody reads."

**Two concurrent writer processes both succeeded and produced 200,000 rows** with no locking and no complaint. That happened to be the benign interleaving; there is no file lock to make it reliably so.

**`transaction` is process-global, unsynchronized, and O(file-size) in RAM.** `static int jinn_txn_depth; static JinnTxnFile *jinn_txn_files;` (`wal.c:267-268`) — not thread-local, no lock, in an M:N runtime where store code runs on any worker. Two coroutines using `transaction` concurrently corrupt the list. Worse, `jinn_wal_force` checks `jinn_txn_active()` (`:68-71`), so one coroutine's open transaction silently disables per-record fsync for every other coroutine's writes. The snapshot mechanism copies the entire data file into heap memory on first touch (`:312-331`), and rollback rewrites the file in place (`:376-399`) — a crash mid-rollback leaves the store corrupt with no recovery path.

**Index/store crash coherence is missing.** No LSN or epoch ties the index to the store, so a crash between store-append and `jinn_idx_insert` leaves a stale index that passes `header_is_sane` and is trusted with no rescan (`index.c:103-129`) — silently wrong query results. Lookups match on the 64-bit hash alone (`index.c:267-277`), so a collision returns the wrong record, and `jinn_idx_contains` used for `@unique` can reject a legitimate insert.

**The bloom filter is undersized and unvalidated.** Its magic is read but never compared (`bloom.c:67-68`); `num_bits` from disk is unvalidated, so 0 gives SIGFPE at `:111` and a huge value gives an unbounded `calloc`; all four `fread`s are unchecked. Sizing omits the `/(ln 2)²` factor (`m = -n·ln(p)`, `:40-44`, with `ln(p)` bucketed to −2.3/−4.6/−6.9), making filters ~2.08× too small, so the delivered false-positive rate is far above the requested one. Since a lost or zeroed filter yields false "definitely absent" answers, this is a correctness issue, not just a durability one.

Broadly: 66 `fread` calls across the persistence files, roughly half unchecked (`bloom.c` 4/4 and `vector.c` 5/5 fully unchecked); `column.c` reads `buf[0]` when `fread` returned 0 (`:152`, `:165-166`); all store formats are native-endian raw struct dumps, so files are not portable across architectures; FTS is a linear file scan per query and `fts_count` disagrees with `len(fts_search)` (`fts.c:148-158`); vector search is brute force. `migrate.c:13` uses magic `"JADESTR\0"` while the adjacent comment says `"JINNSTR\0"` — rename residue worth auditing against codegen's writer.

**Assessment:** as a zero-config store for prototypes and single-process apps, this is genuinely convenient and I enjoyed using it. As a database replacement it is not close, and the gap is not incremental — it is atomic-rename discipline, real WAL replay, index/store coupling, and multi-process locking, none of which exist yet. The `store` *language surface* is the valuable, differentiated idea here; the storage engine behind it could be swapped for the in-tree SQLite without changing a line of user code, and that is probably the fastest path to making the pitch true.

---

## 9. Performance: measured

`gcc -O3` vs `jinn compile --opt 3`, best of 5 runs, same machine (8 cores), outputs verified byte-identical for every pair.

| Benchmark | C (`gcc -O3`) | Jinn | Ratio |
| --- | --- | --- | --- |
| sieve (trial division, 5M) | 6,334 ms | 2,140 ms | **0.34×** |
| matrix_mul | 3,096 ms | 1,504 ms | **0.49×** |
| spectral_norm | 1,896 ms | 1,312 ms | **0.69×** |
| struct_ops | 1,638 ms | 1,192 ms | **0.73×** |
| collatz | 1,002 ms | 791 ms | **0.79×** |
| vec_grow | 2,608 ms | 2,526 ms | 0.97× |
| string_ops | 1,408 ms | 1,377 ms | 0.98× |
| nbody | 2,594 ms | 2,929 ms | 1.13× |
| tight_loop (2B iterations) | 1,164 ms | 1,725 ms | 1.48× |
| fibonacci(42) | 658 ms | 1,703 ms | **2.59×** |

Median ≈ 0.97×. **The "performance of C" claim is substantially earned for scalar code**, and the wins are not measurement artifacts — I verified outputs match and read both sources for equivalence (e.g. `sieve.jn` and `sieve.c` are line-for-line the same algorithm). The wins are LLVM 21 outperforming GCC 15 on these shapes, which is a legitimate benefit of the backend choice.

Two outliers deserve attention:

- **fibonacci(42) at 2.59×** is the one clear codegen deficiency. The emitted IR looks *better* than expected — LLVM applied an accumulator transformation and marked `fib__G_i64` as `nounwind memory(none)` — yet it runs 2.6× slower than GCC's straightforward double recursion. Worth an `perf` profile; something about the transformed loop is losing to the naive version.
- **tight_loop at 1.48×** on a 2-billion-iteration loop that LLVM unrolled 16×. Also worth a look.

For reference on the Python comparison: `fib(32)` took Jinn 15 ms and CPython 946 ms — a 63× gap. Jinn is unambiguously in the compiled-language performance class.

**The committed benchmark numbers do not reproduce.** `benchmarks/results.csv` reports `fibonacci` at 340.82 ms for Jinn and 339.61 ms for C, ratio 1.0. The current `benchmarks/fibonacci.jn` computes `fib(42)`, which takes GCC 658 ms and Jinn 1,703 ms on this machine. Whatever produced that CSV row was not the source now in the tree. To the project's credit, `benchmarks/README.md` is unusually honest about methodology limits — it flags `store_ops` as disk-vs-memory, marks C-side gaps as "language-only," and notes that replacing the single-threaded C actor baseline with a real M:N pool "may flip several ratios." That candor should be extended to regenerating the CSV, and the ratios that involve the Jinn scheduler against a single-pthread C baseline should not be quoted as language comparisons until that happens.

---

## 10. Tooling and documentation

Working: `jinn compile`, `jinn run`, `jinn build`, `jinn test`, `jinn check`, `--emit-llvm/--emit-mir/--emit-hir`, project manifests (`project.jn`), the package cache (`cache.rs` is real and sane — git-fetched deps keyed by URL+semver with an https-only policy), and `.jni` interface files. `jinn test` correctly discovered and ran a `test 'name'` block. All 21 apps in `apps/` build and run.

Rough edges:

- `jinn test <file>` and `jinn check <file>` reject a file argument (`error: unexpected argument 't.jn' found`) — they are project-mode only, which is not discoverable from `--help`.
- `jinn fmt` deletes all comments and overwrites in place (§6).
- The LSP does no type analysis (§6).
- The parser panic in §4.7 exits **0**.
- Untracked run artifacts litter the repo root (`data.wal`, `hits.wal`, `items.store`, `nums.wal`, `s.wal`, `sales.wal`, `jobs.store`, …) — neither committed nor gitignored, because store files are written to the working directory.

On documentation: `docs/jinn.md` is well-written and I could learn the language from it. But several documented features do not work as shown — the actor example uses `*value returns i64`, which fails to parse (`expected NEWLINE, got returns`); `channel of i64` as a parameter type annotation fails (`expected type`); `for v in ch` over a channel segfaults; `extern *printf(fmt as %i8, ...)` fails on the varargs `...`; `sleep()` does not exist; ternary nesting in the then-position doesn't parse; and `sim for` is documented as parallel but is sequential. Meanwhile `docs/concurrency.md` is a model of what the rest should be: it states explicitly that it describes "the runtime and codegen as they actually exist today, not aspiration," documents footguns (never `join` from inside the actor's own handler), and every rule maps to a named passing test. That document's discipline, applied to `jinn.md` and `error-effects.md`, would fix most of §1's credibility problem.

---

## 11. Priorities

**Fix now (safety and trust):**

1. **The `Vec`-return double-free** (§3.1) and the **shared-`Vec` data race** (§3.2). Until these are fixed, the safety claim must come out of the docs. Add all three §3 programs as regression tests.
2. **Decide the memory model for aggregates.** `Vec`/`Map` are shared mutable references at runtime; strings are deep-copied values; the drop machinery assumes the latter. This inconsistency is the root cause of §3.1 and §3.3 and cannot be patched around.
3. **Make `jinn fmt` comment-preserving, or delete the subcommand.** It silently destroys source.
4. **Fix the two red tests** (`jinn.ebnf` path) so the grammar-drift detector runs again. It was disabled by the most recent commit.
5. **Generalize the channel lock-handoff protocol** to the `select`, actor-join, scope-join, and event-loop park sites (§7). You already have the correct fix; apply it three more times.
6. **Redesign `select`**: per-case waiter nodes so it can wait on all channels, plus close handling, plus no silent 16-case truncation, plus never return "default fired" on retry exhaustion.
7. **Remove the scope `children[]` dangling pointers and the stale `tl_gen_coro`** — both are small, contained fixes for critical bugs.

**Fix soon (correctness and credibility):**

8. **Stop merging the directory tree into single-file compiles** (§5.3). Diagnostics currently point at the wrong file and line, and program meaning depends on directory contents.
9. **Align the docs with the implementation.** Remove "fully implemented" from `docs/error-effects.md`, correct the `sim for` parallelism claim, fix the broken examples in `jinn.md`, and regenerate `benchmarks/results.csv`. Adopt `docs/concurrency.md`'s "as it actually exists today" standard everywhere.
10. **Fix the inference gap for methods on unannotated parameters** (§4.1) — this is the biggest usability blocker and it invalidates the "complete inference" claim. It needs a real constraint mechanism (trait bounds or structural/row constraints), not more defaulting. Also fix the `: i64` fix-it to say `as i64`.
11. **Persistence: atomic rename + directory fsync everywhere; CRC the length field; drop the CRC==0 bypass; check `fsync` returns; truncate torn WAL tails; actually replay the WAL on recovery.** Consider whether the `store` surface should sit on SQLite for now — the language-level idea is the valuable part.
12. **Store queries should be `Option`, not zero-filled records**, and `for u in all users` must not segfault.
13. **Route all diagnostics through `src/diagnostic.rs`** — it is already written. That also stops LLVM verifier output from reaching users.
14. **Repair or delete the 16 orphaned programs in `tests/programs/`**, and wire every program in that directory into the harness so rot is impossible.

**Consider (design):**

15. **Reduce sigil overloading**, starting with the four meanings of `!` (§5.2).
16. **Add a real borrow or alias analysis.** The two overlapping half-checkers (§3.4) should become one flow-sensitive analysis with branch merging and loop fixpoint. The typer's `take` machinery is the better foundation.
17. **Delete the dead aspirational subsystems** (`codegen/rc.rs`, the `incr.rs` artifact cache) or gate them behind a feature flag, so the codebase stops advertising capabilities it doesn't have.
18. **Reinstate in-code TODO/FIXME markers.** Zero across 71k lines is not a sign of completeness; it hides the work that remains.

---

## 12. Closing

The pieces that are hard to build are, surprisingly, the ones that work: LLVM codegen at C-competitive speed, a stackful coroutine runtime with correct deque orderings and a correct park handoff, an SSO string type, SCC-ordered inference over a real union-find unifier, and a 1,900-test execution-based suite. That is a strong foundation and it did not happen by accident.

The pieces that are failing are the ones that need a decision rather than more code. What are the aliasing rules for `Vec`? Which of the two ownership checkers is the real one? Is the WAL the source of truth or is the data file? What constrains a method call on an unannotated parameter? Each of §3, §4, and §8 traces back to a question like that being left open while implementation continued around it.

My concrete recommendation: freeze feature work and spend a cycle on §11 items 1–7. Then update the documentation to describe exactly what exists — the `docs/concurrency.md` standard, applied everywhere. A pre-alpha language with a 7-line heap corruption and honest docs is in far better shape than one with polished claims, because only the first one gets fixed.
