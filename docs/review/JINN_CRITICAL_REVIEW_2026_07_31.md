# Jinn — Critical Independent Review & Alpha Remediation Guide

**Date:** 2026-07-31
**Scope:** compiler (front-end, typer, MIR, LLVM backend), C runtime (scheduler, coroutines, channels, actors, durable stores), language/syntax design, standard library, tooling, benchmarks.
**Method:** read the code; built `jinnc` (release, LLVM 21); ran the full test suite (431 pass, 15 ignored, 1 flaky); ran benchmarks vs C `-O3` and Rust `-O`; wrote and compiled ~60 adversarial Jinn probe programs; five deep-read audits across subsystems. Every "VERIFIED" finding below was reproduced on this machine. Docs/memories/task files were deliberately ignored except as leads to confirm against code.

---

## 1. Verdict

Jinn is an ambitious, in places genuinely expert, systems language: an AOT LLVM-backed language with a Koka/Perceus-style tree-ownership memory model (no GC, no reference cycles), an M:N green-thread runtime with a correct Chase-Lev work-stealing deque, structured concurrency, actors, and first-class durable stores. Its performance is in the same class as C and Rust, not Python. The concurrency core and the crash-atomic file primitive are the work of someone who has read the right papers and the right post-mortems.

**But it is not memory-safe.** This is the headline finding, and it is not a matter of polish. It has two independent root causes, each sufficient on its own:

1. **The type system does not enforce its own types.** Safe Jinn source — no `unsafe`, no FFI — can forge a pointer from an integer in two lines and segfault, through at least five paths (`as` casts, fallible-function payloads, indirect/higher-order calls, monomorphization mangling collisions, field assignment). The type checker's unification engine is used to *propagate* types, but the majority of its consistency checks have their results discarded (`let _ = self.infer_ctx.unify_at(...)`), and the one place that collects them is nested in a branch that almost never fires. What safety exists comes from scattered hand-written guards. Where a guard exists, Jinn rejects subtle mistakes with rustc-quality diagnostics; where one is missing, ill-typed IR reaches LLVM and crashes — or silently returns garbage. (§3)

2. **The automatic memory management is unsound — and it is not the Perceus/RC model it appears to be.** There is no refcount anywhere; drops and deep-copies are inserted by a *flow-insensitive, textual, four-way-split* static analysis whose gaps are double-frees and leaks with no diagnostic. `a is s; b is s` on a heap string is a deterministic **double-free**; an early `return` leaks the enclosing scope (1.5 GB over 200k calls); struct rebinding aliases instead of copies; closure/channel/actor captures are freed by the wrong frame. Only straight-line single-scope code is safe. (§3.2)

The two interact: because the memory model has no runtime refcount margin, every static decision is load-bearing for safety, and because the type checker doesn't enforce types, ill-typed programs reach codegen and crash it (`panic!("ICE: ...")`) rather than getting a diagnostic.

Separately, the runtime has a set of serious bugs at the seam between generated code and the C runtime that the test suite never crosses: string/vector slicing via the `from…to` operator segfaults or returns garbage (the `.slice()` method path, which the tests use, is correct), the "safe" subprocess API silently no-ops while reporting success, the durable `save()` erases the write-ahead log before the data it protects is on disk, and the scheduler busy-polls at ~1 core when idle. And the module system itself is unsound: a textual name-flattening resolver captures local variables, so **10 of 23 common standard-library modules (`json`, `bit`, `strings`, `http`, `net`, …) fail to even import** — the stdlib is substantially non-functional through the normal `use` path.

**This is pre-alpha.** The good news is that the defects are concentrated, diagnosable, and mostly not deep design errors — they are missing checks and boundary bugs on top of a sound skeleton. The type-safety hole in particular is one architectural change (make unification failures fatal) plus a handful of specific guards. The remediation list in §7 is finite and ordered.

**One design decision to reconsider now, before more is built on it:** implicit fallibility and silent error/effect discard (§4.3, §4.4) and the string-keyed, CWD-persistent, non-idempotent default for `store` (§5) are baked into the surface language and the durability model respectively; changing them later is a breaking change to every program.

---

## 2. What is genuinely strong (verified, not assumed)

These are real and should be protected as the system is repaired:

- **Chase-Lev work-stealing deque** (`runtime/deque.c`). Memory orderings match Lê/Pop/Cohen/Nardelli 2013; retired buffers are immutable and freed only at destroy (killing the classic grow-time use-after-free and the torn buffer/capacity pair); slots are atomic; OOM aborts loudly. Two independent audits traced the orderings and stress-tested it (millions of transfers across 6 thieves, zero lost/duplicated). This is better than most production hobby runtimes.
- **Lock-handoff park protocol** (`sched.c`, `channel.c`, `select.c`, `scope.c`). Every park path publishes itself under a spinlock the scheduler releases only *after* `jinn_context_swap` has saved the context, so no waker can resume a half-saved coroutine. The `select` variant (release in reverse, re-acquire sorted) is subtle and correct.
- **Crash-atomic rewrite** (`durable.c`): temp-file → fsync → checked close → rename → **directory fsync**. SQLite-grade, and used consistently by kv/migrate/recover/version/bloom/txn. The WAL v2 framing CRCs its own length field and truncates a torn tail before appending.
- **Integer div/rem safety** (`src/codegen/arith.rs`): traps both divide-by-zero *and* the `INT_MIN / -1` overflow case that is UB in LLVM `sdiv`. Many compilers miss the second.
- **Use-after-move diagnostics** where they fire: `w is v` on a `Vec` followed by a read of `v` is a clean compile error pointing at the move site and suggesting `copy`. Bounds checks (including negative indices) trap. Definite-assignment and enum-match exhaustiveness (on the covered types) are enforced. (But the move checker has large holes — see §3.2; it does *not* catch `String`/struct aliasing.)
- **Error-set inference** (`src/typer/errset.rs`): fallibility is inferred bottom-up over the call-graph SCC to a least fixpoint, declared `! E` is checked as an upper bound, and cross-layer widening uses a `From` trait with an orphan rule. This is a good design (Zig error sets + Rust `From` ergonomics) and is the one part of the checker that reliably rejects wrong programs.
- **Performance.** AOT + LLVM + no GC delivers systems-class throughput (see §6). On the *straight-line, single-scope* path that the drop inserter handles correctly, memory is tight (200k×build+drop of a 50-element vector held RSS at 2.2 MB). That path is narrow — see §3.2.
- **Cycles are unconstructible**, so there are no cycle leaks — but this is because the model has no sharing primitive at all (deep-copy value types + boxed recursive enums), not because cycles are handled. The cost — no doubly-linked lists / back-references / cyclic graphs without raw pointers — is real.

---

## 3. Type system — UNSOUND (Critical)

The typer is a best-effort type *annotator*, not a checker. `unify_at` results are discarded at the majority of call sites; `self.type_errors` is drained only inside `if !strict_errors.is_empty()` (`src/typer/lower/mod.rs:623`), a branch that fires only on genuinely unconstrained type variables. `--strict-types` is a no-op (strict is already the default); `--lenient` is no more unsound than the default. MIR verification is `#[cfg(debug_assertions)]` — **compiled out of the release `jinnc`** (`src/driver/mod.rs:498,510`), so LLVM's verifier is the last line of defense and it only catches representation mismatches, not the semantic holes below.

All of the following were reproduced. `0x133f` = 4919 decimal; the fault address *is* the integer being used as a pointer.

| ID | Severity | Finding | Repro result |
|----|----------|---------|--------------|
| T-CAST | Critical | `as` casts are unchecked type puns — no legality check at all (`src/typer/expr/mod.rs:174`). Strictly weaker than C's `as`. | `p is 4919 as Vec of i64; log(p[0])` → **SIGSEGV @0x133f** |
| T-OK | Critical | Fallible functions never unify their `Ok` payload against the declared return type. `auto_wrap_ok`/`variant_ctor` build the `Result` variant without checking the payload (`src/typer/stmt/block.rs:50-100`, `call/method_call.rs:923`). Affects **every `! E` function**. | `returns Vec of i64 ! E` with body returning an `i64` → **SIGSEGV @0x133f** |
| T-INDIRECT | Critical | Indirect / higher-order / `extern` call arguments are never checked (`src/typer/call/fn_call.rs:248,369,411,461` all discard the unify result). Every closure/callback boundary is unsafe. | `g(n)` where `g: (Vec of i64)->i64` called with `i64` → **SIGSEGV @0x133f** |
| T-MANGLE | Critical | Monomorphization mangling does `ty.to_string().replace('_',"U")` (`src/typer/mono.rs:247`), so `A_B` and `AUB` collide and the second instantiation is silently the first. | `A_B{n:i64}` vs `AUB{n:String}`: reads the String field as `i64`, prints a pointer |
| T-ASSIGN | Critical | Assignment `lvalue is expr` discards its unify result (`src/typer/stmt/dispatch.rs:985`); wrong-typed field/var assignment compiles. | `p.x is 'hello'` on an `i64` field → compiles, `p.x + 1` prints garbage pointer |
| T-TRAIT | High | Trait impls are matched by method *name* only (`src/typer/resolve.rs:384`); arity, parameter types, and return type are unchecked. An `impl` returning `Vec of i64` satisfies a trait declaring `String`. | Ill-typed impl compiles |
| T-EXHAUST | High | Match exhaustiveness only covers `i64/f64/String/bool/enum` (`src/typer/lower/exhaust.rs:127`); `i8/i16/i32/u*/f32`, tuples, structs, arrays fall through to "exhaustive". | `match` on `i32` with non-exhaustive arms compiles; silently returns the last arm |
| T-LITRANGE | High | Integer literals are never range-checked (`src/typer/expr/mod.rs:64`). | `x as i8 is 300` → `44`; `f(5000000000)` to an `i32` param → `705032704`, no diagnostic |
| T-RETNUM | High | Declared return type is not enforced when both sides are numeric or contain a `Ptr`/var (`src/typer/lower/decl.rs:451`). | `*f() returns i64` with body `2.5` → returns `1` (silent f64→i64) |
| T-ICE | High | Ill-typed programs that slip past the guards reach codegen and **crash the compiler** with `panic!("ICE: phi node type mismatch")` (`src/codegen/mir_codegen/mod.rs:703`) or "generated LLVM IR failed verification — this is a compiler bug", blaming the user's type error on the compiler. | match-arm / if-branch type mismatch → compiler ICE |
| T-CAPS | High | The `needs` capability/effect system enforces nothing on real code: its site table (`src/cap_sites.rs`) lists `std.fs.*`/`std.net.*` but the stdlib is called as `fs.remove`/`net.tcp_connect`; only non-generic top-level fns are scanned (methods/impls/actors/lambdas skipped); checked only when `needs` is present. `*evil() needs pure` calling `fs.remove(...)` compiles clean. | Capability guarantee is advertised but absent |
| T-COERCE | Medium | Implicit lossy int-truncation and float→int are *warnings*; bool→int is silent (`src/typer/expr/mod.rs:385`). Below C99; far below Rust/Zig/Swift/OCaml (zero implicit numeric conversion). | `takei(3.9)` → `3` with a warning |

**Also:** `has_type_var`/`free_type_vars` don't traverse `Struct(_, args)` (`src/types.rs:243`) — currently under-generalizing (safe), but one traversal change from unsoundness; the `Struct↔Enum` unification shim drops argument unification (`src/typer/unify/mod.rs:580`); user-facing diagnostics leak Rust `Debug` (`{span:?}`) in `src/typer/errset.rs:187`.

**Why earlier smoke tests looked fine:** the guards that *do* exist are good. `Result + i64`, passing a `Result` to an `i64` parameter, use-after-move, and enum-match exhaustiveness are all correctly rejected with clear messages. That is exactly why the hole is dangerous — the language feels safe until you touch an unguarded path.

### 3.1 Name resolution & the module system — breaks the stdlib (Critical)

Separate from the typer, the module-flattening resolver (`src/resolve.rs`) is unsound and, more damagingly, **breaks the shipped standard library through the normal `use` path**.

- **T-MODCAP (Critical) — VERIFIED.** `resolve.rs:14-41` builds one textual rename map from a module's top-level fn/const names and rewrites *every* `Ident` in every body, shielding only function parameters. Local `is`-binds, `for`/match/lambda/tuple binders are **not** shielded, so any local whose name collides with a module-level function or const has its *uses* silently rewritten to the prefixed global. **Measured: 10 of 23 common std modules fail to even import** — `json, bit, strings, collections, http, net, url, bytes, dataframe, logging`. Confirmed mechanisms: `use std/bit` → `operator - not defined for ?3 and (i64)->i64` (local `mask` rewritten to fn `*mask`, `std/bit.jn:135`→`195`); `use std/json` → `unknown method push` (local `keys` rewritten to fn `*keys`). When the collision doesn't fail to typecheck it **silently miscompiles**: an imported lambda `|a| a+1` applied to 5 can return 1001 via a module const `a is 1000`. This should be priority 1 alongside the typer fixes: the module system does not work for any module that reuses a top-level name as a local.
- **T-MODARGS (High) — from audit.** A module-renamed constructor call is rebuilt positionally in literal field order, dropping argument names: `Pair(b is 2, a is 1)` prints `2, 1` (swapped) after import — and the identical code is *rejected* as a root file (`resolve.rs:228-243`).
- **T-VARIANT (High) — VERIFIED.** A unit enum variant is consulted before locals (`src/typer/expr/ident.rs:15-52`), so a variant silently shadows a same-named local: `Red is 42; log(Red)` prints `0` (the tag), no error.
- **T-FABRICATE (High) — from audit.** Undefined identifiers fabricate a `Var(DefId::BUILTIN, name)` (`ident.rs:203`) that `hir_validate` is explicitly wired to skip; combined with MIR `Load` resolving by *name* not `DefId`, a popped loop binder's stale frame slot can be read after the loop. Errors that do fire carry no file/line.
- **HIR denormalization (Medium) — from audit.** Every expression caches its own `ty`, every `Var` use is a denormalized copy of its binder's type with no sync invariant, and the one validator that checks a pair runs *before* `comptime::fold` mutates the HIR. This is the structural reason the §3 typer holes become codegen ICEs instead of being caught downstream.

### 3.2 Automatic memory management — UNSOUND (Critical)

**Jinn does not implement Perceus or reference counting.** There is no refcount word in any runtime value (`Vec`/`String` are `{ptr, len, cap}`); `drop_value` frees unconditionally and `clone_value` deep-copies. What exists is *static, syntax-directed* insertion of deep-copy and free: the typer appends a `Drop` for every scope-live variable at the **textual end of a block**, suppressing drops for a **flow-insensitive** "consumed" set. The job is split across four analyses that disagree — `type_is_aggregate` (moves), `needs_drop` (drop obligation), `collect_block_consumed_ids` (drop suppression), `record_take_moves` (use-after-move) — and the gaps between them are memory bugs, in **both directions** (double-free *and* leak), with no diagnostic. The guarantee actually delivered is narrow: *straight-line, single-scope code where every owned value has exactly one syntactic use and control falls off the end of the block.* Everything else is unsound. (This is why my own first leak test — which was exactly that narrow case — looked clean.)

| ID | Severity | Finding | Status |
|----|----------|---------|--------|
| M-ALIAS | Critical | `a is s; b is s` where `s` is a heap `String` (or a struct containing one) neither moves nor clones, and both bindings get a scope drop → **double free**. `type_is_aggregate` excludes `String` from moves; `needs_drop` includes it. | **VERIFIED** — `free(): double free detected in tcache 2`, deterministic (3/3 aborts). `a is copy s` works — the *default* is what's broken. The equivalent `Vec` program is correctly rejected, so the two analyses disagree on exactly this class. |
| M-STRUCT-ALIAS | Critical | Struct rebinding `b is a` binds `b` to the *same* alloca (`emit_inst/core.rs:722`); mutating `b` mutates `a`. Contradicts value semantics and the move checker's own `Vec` behavior. | **VERIFIED** — `a is Point(1,2); b is a; b.x is 99; log(a.x)` → **99** |
| M-EARLYEXIT | Critical | `return`/`break`/`continue` emit only the *current* block's drops; enclosing scopes' owned values are never freed. Every early-exit / `?`-heavy path leaks proportionally to work. | **VERIFIED** — an early `return` after building a 1000-elem vector, ×200k calls → **1556 MB peak RSS** (vs 2.2 MB straight-line) |
| M-REASSIGN | Critical | Reassigning a local or struct field never drops the previous value (`stmt.rs:122`, `FieldSet` codegen). Loops with accumulators leak. | Audit-VERIFIED — `v is build(1000)` in a 200k loop → ~1.55 GB |
| M-RETURN-PARAM | Critical | Returning a by-value parameter can double-free (`needs_auto_clone` only clones `Field`/`Index`, not a bare `Var`). | Audit-VERIFIED; **not reproduced in my variant** (a `returns String`-annotated identity exited cleanly) — likely repro-shape-sensitive; flagged for the maintainer to pin. |
| M-PUSH / M-CHAN / M-CLOSURE / M-ACTOR | Critical | `v.push(s); v.push(s)` (double-free), `ch.send(s)` (double-free), closure capturing a local `String` (capture dropped before the closure escapes → UAF), `spawn Actor(name is s)` (payload freed by the spawning frame → cross-thread UAF). Each is a "consumed but not moved / not cloned" gap. | Audit-VERIFIED with reproductions; **push not reproduced in my variant** — the others I did not independently re-run. Treat as high-confidence-but-verify. |
| M-ESCAPE | Critical | Escape-analysis borrow demotion (`src/escape/mod.rs:90`) rewrites an owning bind of a field/container read into a borrow and **deletes its drop**, with no check that the owner outlives the borrow → UAF; tier assignment is also inference-order dependent (silent, non-deterministic ownership). | Audit-VERIFIED |
| M-VERIFY | High | `src/mir/verify.rs` has **no ownership invariant** — the double-free repro emits `drop` twice on one value and verifies clean — and MIR verification is `#[cfg(debug_assertions)]`, compiled out of release. | VERIFIED (also §3, T-ICE) |

**Assessment vs the models it resembles:** Perceus (Koka/Lean) is sound because an over-approximate `dup`/`drop` costs *performance*, never correctness — there's a runtime refcount as margin. Jinn has no refcount, so every insertion decision is load-bearing for *safety* with zero margin, and it makes those decisions with a flow-insensitive, four-way-split, textual analysis. Swift ARC would have avoided the closure/actor cases (escaping captures retain); Rust's borrow checker would have rejected the escape-demotion case; Rust's drop elaboration (drop flags for conditional moves, unwinding/early-exit paths, drop-before-reassign) is precisely the machinery Jinn is missing. **The fix is not per-bug:** drop insertion needs to become a single MIR dataflow pass (liveness + per-path drop-flag lattice), the verifier needs a real ownership invariant run in release, and cross-boundary values (closures/channels/actors) need a decided-and-enforced clone-or-move rule. Two cheap interim mitigations turn the double-frees/UAFs into *leaks* (the safe failure direction): clone at every consuming position for clonable types, and disable `apply_demotions`.

---

## 4. Front-end (lexer, parser, formatter) & error model

The lexer/parser are a hand-written byte-oriented lexer (Python-style INDENT/DEDENT) feeding a recursive-descent parser with a macro precedence ladder and heavy parse-time desugaring via mutable side-channels. Fast and readable on the happy path; fragile and context-sensitive off it. The `Diagnostic` renderer (codes, labels, snippets) exists but is **dead** — front-end errors are flat `line N:col: msg` strings.

### 4.1 Formatter destroys programs (Critical) — VERIFIED
`jinn fmt` is an AST pretty-printer covering ~60% of the AST, and `--write` guards only on `formatted != src` — no reparse, no AST-equality check (`src/driver/mod.rs:315`).
- `for i in 0 to 10 by 2` → **`for i in 0`** (bounds silently deleted — a semantic change, not just breakage). VERIFIED.
- `select`/`query`/`receive`/`struct` expressions → literal `...`; type methods, `migration`/`view` decls, supervisor children, fn attrs, `! ErrType`, access modifiers → all dropped or emitted in syntax the parser rejects (`vec<i64>`, `use a.b import x`, `extern puts s string`, `%x`→`&x`). 2 of the project's own 12 corpus files fail fmt round-trip. VERIFIED.

**Fix:** disable `fmt --write` until it either prints every AST node or (better) formats from tokens+trivia (a CST), gated on `parse(format(src)) == parse(src)`.

### 4.2 Parser crashes on ordinary adversarial input (Critical) — VERIFIED
The 256-level depth guard exists only in `parse_expr`; `parse_unary`, `parse_exp`, and `parse_pat` recurse unguarded. `x is ` + `-`×100000 → **stack overflow, core dump**. VERIFIED. Also confirmed independently: a recursive value type used as a parameter (`type Node{next as Node}`) makes *layout* recurse infinitely → compiler stack-overflow/core-dump with no diagnostic (Rust/Zig give "recursive type has infinite size").

### 4.3 Chained comparison double-evaluates the middle operand (High) — VERIFIED
`1 lt f() lt 10` calls `f()` **twice** (`parse_cmp` clones the shared operand, `src/parser/expr/pratt.rs:203`). Side-effecting middle operands are miscompiled. Needs a temp-binding desugar.

### 4.4 Other confirmed front-end defects
- **Duplicate function definitions silently merge** (last-catchall-wins) via the multi-clause desugar (`src/parser/mod.rs:564`) — no "duplicate definition" error; bodies are silently dropped. (High)
- **Desugar state leaks across declarations**: a C-style `loop init, cond, step` at top level strands its init bind (`__cph_0`) or splices it into the *next* function's body (High).
- `$` placeholder isn't traversed into `ListComp`/`Quaternary`/`Slice` → silently unsubstituted wrong results (High).
- `:x` char-literal lookahead hack breaks single-char enum variants (`Color:R`) and slice syntax `a[1:2]` with span-less errors (High).
- `extern` variadic `...` doesn't parse (lexer fuses `...`→`DotDotDot`; parser wants three `Dot`s) — the documented C-interop spelling is unusable (High).
- Byte-oriented lexer mangles non-ASCII (`café`, BOM → mojibake errors), no unicode identifiers (Medium).
- Lex/parse errors bypass the `Diagnostic` renderer entirely; `x = 1` yields "unexpected character: '='" with no "use `is`" hint (Medium).
- Tree-sitter grammar and `docs/jinn.ebnf` both materially diverge from the real parser while claiming fidelity (lambdas are `|x| e` not `(x)=>e`; `for..from..to` not `for..in`; `use a/b [x]` not `use a.b`). Editor highlighting will be wrong for most real code. (Low/Medium)

---

## 5. Durable stores & the `store` default (High)

`store users` with no path silently binds to **persistent, CWD-relative files** (`users.store`/`users.wal`) keyed by the store's name, with no lifecycle. Consequences, all verified:
- **Non-idempotent programs.** `store_basic` prints `3, 6, 9` on successive runs — inserts accumulate across process invocations. This is the root cause of the flaky `programs_harness` snapshot test and of the `.store`/`.wal` litter in the repo root (`items.store`, `data.wal`, `nums.store`, …).
- There is no obvious way, in the example programs, to get an ephemeral/in-memory store.

Beyond the default, the durability implementation has serious holes (two independent runtime audits + code reading):
- **`save()` inverts write-ahead logging** (Critical): generated `emit_store_save` does `fflush(fp)` then `jinn_wal_checkpoint`, which `ftruncate`s and fdatasyncs the WAL to empty. The **data file is never fsynced**. Power loss after `save()` → durable empty WAL + data still in the page cache → total loss of every record since the last checkpoint. One-line fix: fsync the data fd before checkpoint. Same class in `recover.c` on the unchanged path.
- **No commit record** in the WAL format (High): nothing marks a transaction boundary, so kernel-flushed-but-uncommitted records replay as committed, and a multi-store `txn_commit` fsyncs stores serially with no barrier (crash mid-loop commits A, not B).
- **`flock` provides no intra-process exclusion** (Critical): store ops serialize via `flock(fileno(shared_fp), LOCK_EX)`, but all coroutines share one open file description, so the second acquisition returns immediately. Two coroutines inserting into one store interleave `fseek/fwrite` → torn records and lost updates. Needs a real per-store mutex.
- **One corrupt WAL record truncates the rest** (High): a single CRC mismatch in record 1 of 10,000 is treated as a torn tail and unlinks records 2..10,000.
- **Sidecar formats (`.idx`, `.col`, `.fts`, `.ver`, bloom, vector) trust on-disk lengths** (High): file-supplied counts/sizes flow straight into `malloc`/`memcpy`/`for(;;)` probe loops with no validation, no checksums, and mostly no fsync. Bloom filters are never persisted (`jinn_bloom_close` is never called) → false negatives (the one thing a Bloom filter must never do) on the second run.
- **KV/index tombstones are never reclaimed** (High): `kv_del` sets a tombstone *and* decrements the live count, so the grow threshold never fires; after `KV_INIT_CAP` set/del churn the table fills and the next lookup **spins forever**. Reproduced: hang on the 65th key. A first-day-of-production hazard for any counter/session workload.

---

## 6. Runtime & backend — other confirmed issues

- **String/vector slice via `from…to` is broken** (Critical) — VERIFIED. `'abcdefgh' from 2 to 5` → **SIGSEGV**; `[10,20,30,40,50] from 1 to 3` → **garbage** (`0, 0`). Root cause: the SSO string tag convention is *inverted* between codegen (`src/codegen/strings.rs:30`: bit7 set = inline) and runtime (`runtime/vec.c:56`: bit7 set = heap → dereferences string bytes as a pointer), and `__jinn_vec_slice` is called with 3 args but declared with 4 (`elem_size` is register garbage → unchecked `malloc`/`memcpy`). The `.slice(a,b)` *method* path is correct — which is why the tests, which use `.slice()`, pass. **This is a testing gap as much as a bug.**
- **`process.spawn_exec` silently no-ops** (Critical) — VERIFIED. Returns exit code `0` (success) but the program never runs (the "safe argv" path uses the same inverted SSO convention to marshal argv). A subprocess API that reports success without doing anything.
- **Float→int cast emits raw `fptosi`** (High) — VERIFIED. `f as i32`/`i64` (`src/codegen/mir_codegen/helpers/values.rs:292`) has no saturation and no NaN/range guard; out-of-range/NaN/±inf is LLVM poison → UB. `1e300 as i32` = `-1`. Rust saturates; Zig traps. Inconsistent with the careful div/rem guarding in the same backend.
- **SIGPIPE disposition is default** (Critical for networking) — VERIFIED. No `MSG_NOSIGNAL`, only SIGSEGV/SIGBUS handled. Any Jinn TCP/HTTP server is killed by the first client that disconnects mid-response.
- **No async I/O; blocking syscalls starve the scheduler** (High). `net.c` uses raw blocking `send`/`recv`; `event.c`'s epoll machinery is never wired in; workers are hardcoded `min(nproc, 8)`. 8 concurrent socket reads deadlock the runtime. The green threads are decorative for the one workload (I/O concurrency) they exist to serve.
- **Idle scheduler busy-polls** (High) — VERIFIED. One dormant actor + a 2 s sleep burns **~1 core** (0.99 cores measured; second audit measured 0.68). The park path is a 40-spin steal loop + 100 µs `cond_timedwait`, i.e. a ~10 kHz poll papering over a lost-wakeup in `wake_one`.
- **Supervisor has zero synchronization** (High): `children`, `restart_count`, `mb_ptr`, `alive` are plain fields mutated from arbitrary worker threads; a `realloc` in `sup_register` racing `sup_on_child_exit` is a use-after-free, and concurrent exits double-free a mailbox. A data race by C11 rules.
- **Context switch drops FP control state** (High) — VERIFIED numerically. `context_x86_64.S` does not save `MXCSR`/x87 control word (callee-saved per the SysV psABI); a coroutine that changes rounding/denormal mode leaks it, and a probe showed `1.0/3.0` changing in the low bit across a swap. (aarch64 correctly saves d8–d15.) No CFI in either `.S`, so unwinders/profilers can't walk the swap.
- **4 KiB guard page is jumpable** (High) — VERIFIED. 64 KiB stacks + one guard page: any frame >4096 bytes steps over it and corrupts the neighbouring mapping with no fault. Needs a multi-page guard band or compiler-emitted stack probes.
- **`crypto.c` truncates `long`→`int`** across the RNG/AEAD/KDF surface (Medium): `jinn_random_bytes(buf, 4294967296)` → `RAND_bytes(buf, 0)` returns success with the buffer untouched. Crypto itself is correctly delegated to OpenSSL EVP (SHA-256/HMAC/AES-GCM KATs pass) — the right call — but the FFI shims corrupt sizes.
- **`recover.c:205`** multiplies `count * rec_size` (both from the file header) as `int64` with only a `< 0` check → signed-overflow UB and an undersized allocation.

Optimization passes: an `--opt 0` vs `--opt 3` output diff across the deterministic program corpus found no behavioral divergence except `pointy`, which logs struct addresses (address-dependent, not a miscompile). No opt-level miscompile surfaced in this sweep — but note this only exercises the (broken-for-`from..to`) happy paths.

### 6.1 The backend is structurally unsound for any value wider than 8 bytes (Critical)

The MIR→LLVM emitter has no ABI-classification pass: `declare_mir_fn` silently rewrites aggregate parameters to `ptr` while other signature consumers (closures, vtable thunks, indirect calls) reconstruct from `llvm_ty` — they disagree — and layout is computed twice (a hand-rolled `type_store_size` trio *and* LLVM's `DataLayout`). The recurring bug is "an aggregate was assumed to fit in an 8-byte slot." Reproduced:

| ID | Severity | Finding | Status |
|----|----------|---------|--------|
| B-MAP-GROW | Critical | `Map` never grows (`map.rs` has no rehash); the 17th distinct key is an infinite probe loop. | **VERIFIED** — 20+ keys → hang (timeout) |
| B-MAP-VAL | Critical | `Map` values live in an 8-byte slot; a non-scalar value (e.g. `String`) overruns into the occupied byte and next entry. | Audit-VERIFIED — `m.set("k", "<long>")` → garbage |
| B-HOF-STRUCT | Critical | `.map()`/`.filter()`/`.fold()` over a `Vec` of structs is an ABI mismatch (struct passed by-ptr in the wrapper, by-value in the indirect call). | **VERIFIED** — `[P(1),P(2)].map(bump)` → **SIGSEGV @0x1** |
| B-CORO-CAP | Critical | `dispatch`/`together`/generator captures are stored into 8-byte slots; a captured `String` (24 bytes) overruns the frame → heap-metadata corruption. | Audit-VERIFIED — `malloc.c assertion failed` |
| B-SUM | High | `Vec.sum()` on `[i32]` inits an `i64` accumulator (8 bytes into a 4-byte slot) → wrong; on `[f32]` it **ICEs the compiler**. | Audit-VERIFIED |
| B-SORT | High | `Vec.sort()` on any non-`i64`/`f64` element **ICEs the compiler** (`into_int_value()` on a struct/String). | **VERIFIED** — `["pear","apple"].sort()` → `panicked at src/codegen/vec/ordering.rs:208` |
| B-ENUM-ALIGN | High | Enum/actor-message payloads are stored at offset 4 in a struct whose type-alignment LLVM records as its natural alignment → misaligned store; UB on the most common aggregate (`Result`/`Option`). | Audit-VERIFIED (by inspection) |
| B-SELECT-BUF | High | `select` receive buffers are always `i64`; a `channel of f64` prints `0.0`, and a wide element smashes the stack. | Audit-VERIFIED |
| B-NUL | High | Jinn strings aren't NUL-terminated but `string_data(v)` is passed to every `extern` C string param; a 23-byte SSO string puts the tag byte where the NUL should be → `strlen` over-reads. | Audit-VERIFIED (also §6A) |
| B-CHECKED | Medium | `CheckedAdd`/`CheckedMul` builtins call `llvm.*.with.overflow` then **discard the overflow bit**; `SaturatingMul` clamps to the wrong end for negatives; internal FNV hash multiply is marked `nsw` (poison, since FNV requires wrapping). | Audit-VERIFIED |
| B-DEBUG | Medium | `--debug` emits **zero** `!dbg` records (no `DISubprogram`/`DILocation` anywhere); debuggers get nothing. Adoption blocker. | **VERIFIED** — `grep -c '!dbg'` = 0 |

Compiler ICEs on valid programs (`.sum()` on `[f32]`, `.sort()` on `[String]`) are release-blocking on their own — a first user sorting a list of strings gets "please report a compiler bug." Fixed-size array indexing is also unchecked (only `Vec` indexing is bounds-checked), and the comptime folder panics the compiler on `INT_MIN / -1` (the runtime path correctly traps).

---

## 6A. Standard library — systemically broken by a `.length` / byte-count confusion (Critical), plus security defects

The stdlib's algorithms are mostly *chosen* well but *wired* to two compiler semantics that silently corrupt them. Both root causes are VERIFIED; the downstream consequences are traced to file:line in the audit and follow directly.

**Root cause 1 — `.length` is the Unicode scalar count, not the byte count** (`src/codegen/mir_codegen/helpers/values.rs:373`; scalar = bytes where `(b & 0xC0) != 0x80`). VERIFIED: `'äöü'.length` = `3`, `.byte_count` = `6`. `char_at`/`slice`/`find` are byte-indexed. The entire library uses `.length` on binary/non-ASCII data as if it were the byte length:
- **Crypto wrappers reject real keys and truncate data.** `aes.gcm_encrypt` guards `if key.length neq 32` (`std/aes.jn:46`); a random 32-byte key averages ~24 scalars (¼ of random bytes are continuation bytes), so `key.length == 32` is essentially never true → **~99.99% of random keys/IVs/tags rejected**. Byte lengths passed to OpenSSL (`jinn_evp_digest(…, data.length, …)`, `std/sha.jn:23`; AES, HMAC, Argon2) are scalar undercounts → **silently truncated digests/ciphertexts/MACs** on any binary input.
- **`secure_compare` is doubly broken** (`std/crypto.jn:183`, `std/argon.jn:136`): loop bound is `.length` (scalars) while `char_at` is byte-indexed, so two 32-byte tags agreeing on the first ~24 bytes compare **equal** — a real MAC/password-verify weakening.
- **UUID returns `""` ~99.99% of the time** (`uuid.format`/`v4` guard `bytes.length neq 16` against 16 random bytes ≈ 12 scalars).
- **Every hand-written scanner** (`json`, `url`, `http`, `toml`, `csv`, `binary`, `date`, `net`, `sqlite`) mixes `.length` with byte offsets → **truncates or traps** on non-ASCII. Confirmed trap: `url.parse` with multibyte input → `slice(30,19)` → process abort.

**Root cause 2 — `String as i64` is a silent 8-byte reinterpret, not a parse** (the T-CAST hole applied to strings). VERIFIED: `'8443' as i64` → `859059256` (the ASCII bytes, not the number). `url.jn:119` `port is port_str as i64` is therefore garbage. There is no String→int parse path in codegen.

**Also compiler-rooted:** Jinn strings aren't NUL-terminated but are passed to C string APIs — `sqlite3_prepare_v2(…, -1, …)` and `inet_pton` receive non-terminated pointers for strings ≥23 bytes → heap over-read (`std/sqlite.jn:104`, `std/net.jn:133`). `String.slice` with a negative start is unchecked (`src/codegen/string_ops.rs:281`) → OOB read, not a trap.

**Security defects (traced by the audit; consistent with the confirmed semantics):**
- **http.jn: CRLF header injection / request splitting** — method, path, host, and header names/values go to the wire with zero validation (`std/http.jn:65`); `Content-Length` uses `.length` (scalars) → request smuggling on keep-alive.
- **url.jn: open-redirect / host confusion** — scheme detected by `find("://")` over the whole string before stripping query/fragment, so `parse("/redirect?next=https://evil.com")` yields `host="evil.com"`, `is_valid()==true` (`std/url.jn:72`).
- **net.jn:** `inet_pton`'s return is ignored (`std/net.jn:133`) → plain-HTTP requests to a *hostname* connect to `0.0.0.0`/localhost; `read(-1)` → `malloc(0)` + `recv(…, (size_t)-1)` heap overflow.
- **json.jn:** malformed input calls `assert(false)` → libc `abort()` (`src/codegen/.../runtime.rs:66`) → **untrusted JSON hard-crashes the process**; no nesting-depth limit → stack overflow; `\uXXXX ≥ 127` → `"?"` (data loss); all numbers f64.
- **process.cmd()** advertised as shell-safe but only quotes args with spaces/quotes, so `$(…)`, backticks, `;`, `|` in a space-free arg pass unquoted → **command injection** (`std/process.jn:84`). (Shell `exec` itself is correctly gated behind `JINN_ALLOW_SHELL=1` — good.)
- **raft.jn: election-safety bug** — `handle_request_vote_reply` counts votes in a bare `i64` with no per-peer dedup and no `reply.term == current_term` check (`std/raft.jn:151`), so a replayed/stale granted reply is double-counted → **two leaders in one term**.
- **random.jn substantially broken** (audit-traced): `next_f64` scales by `2^-64` not `2^-53` → range `[0, ~0.0005)`; `seed(0)` → all-zero state stuck at 0; untyped `>>` is arithmetic (sign-filling), so `__rotl` isn't a rotate → not xoshiro256** and `flip()` is always false.
- **bigint.jn:** multi-chunk `__divmod` is repeated-subtraction (up to 10⁹ iterations/digit, re-allocating in the loop) → division hangs/OOMs at real sizes; `modulo` drops the sign of `a`.
- **decimal.jn:** `div` has no zero-guard → `decimal.div(x, zero())` aborts; `round_to` wrong for negatives. **bytes.jn:** `slice()` returns all zeros (copy loop no-ops). **binary.jn:** `unpack` is unbounded-recursive on attacker counts → stack overflow / multi-GB alloc from a 5-byte input.

**Positives (verified):** crypto correctly delegates to OpenSSL (SHA-256/HMAC/AES-GCM KATs pass at the C layer); **the TLS client verifies certificates and hostnames** (`SSL_VERIFY_PEER` + default CA paths + `X509_VERIFY_PARAM_set1_host` + SNI, `runtime/tls.c:38`) — not MITM-able; no ECB exposed; GCM decrypt withholds plaintext until the tag verifies; `sort.merge_sort` is stable; `stats` uses numerically-stable Welford/two-pass. **But there are no crypto/json/uuid/decimal/bigint/raft tests at all** (no known-answer vectors anywhere in the tree) — which is why none of the above was caught.

---

## 7. Remediation guide to alpha

Ordered by "a program that should be safe currently isn't." Nothing that claims type or memory safety can ship until §7.1 is done.

### 7.1 Type & memory safety (blockers)
0. **Fix module resolution (do this first — the stdlib doesn't import).** Replace textual name-flattening (`src/resolve.rs`) with `PkgId`/`DefId`-based resolution that never rewrites local binders (the file's own doc comment says this is the intended direction). Until then, 10/23 common std modules fail to compile when used. (T-MODCAP, T-MODARGS)
1. **Make unification failures fatal.** Change `unify_at` to `#[must_use]` (or drain a single always-fatal `Diagnostics` sink); move the `type_errors` drain out of the `!strict_errors.is_empty()` branch (`src/typer/lower/mod.rs:623`). This one change surfaces most of §3 as diagnostics instead of segfaults/ICEs, and turns the T-ICE codegen panics into type errors.
2. **Gate `as`** to a checked numeric lattice; move reinterpretation behind explicit `unsafe`/FFI syntax. (T-CAST)
3. **Unify `Ok` payloads** against the declared return type at all three wrap sites (`block.rs:71,92`, `dispatch.rs:841`). (T-OK)
4. **Check indirect/HOF/extern call arguments** (`fn_call.rs:248,369,411,461`). (T-INDIRECT)
5. **Fix the monomorphization mangler** — hash the canonical structural type; the `_`→`U` transform is not injective. (T-MANGLE)
6. **Check trait-impl signatures** (arity, params, return) against the trait. (T-TRAIT) **Resolve MIR `Load`/`Var` by `DefId`, not symbol name** (`emit_inst/core.rs:683`), and consult locals before enum variants (`ident.rs:15`) — both are latent wrong-value sources. (T-FABRICATE, T-VARIANT)
7. **Invert the exhaustiveness default**: unknown subject type ⇒ require a wildcard. Extend to `i8..u64/f32`, tuples, structs. (T-EXHAUST)
8. **Range-check integer literals** for the target width. (T-LITRANGE)
9. **Enable MIR verification in release builds** (or add a cheap always-on HIR type-consistency pass over `Ret`/`Assign`/`Call`-args/`VariantCtor`). Codegen must never `panic!` on user input. (T-ICE, `src/driver/mod.rs:498`)
10. **Either fix or remove the `needs` capability system.** Derive sites from the typed HIR call graph, make `pure` the default, generate the site table from the stdlib, traverse lambdas/methods — or drop it from the surface until it works. Advertising an effect guarantee a `.map(|x|…)` defeats is worse than none. (T-CAPS)
11. **Add a guard-page recursion limit in the type-layout pass** so recursive value types produce "infinite size" diagnostics, not a compiler core-dump.

### 7.1a Memory management & aggregate codegen (blockers)
These are memory-unsafe or ICE on ordinary programs; several must land before the language can claim safety at all.
- **Rewrite drop insertion as a single MIR dataflow pass** (liveness + per-path drop-flag lattice, à la Rust drop elaboration), replacing the four disagreeing HIR analyses. Fixes M-ALIAS, M-EARLYEXIT, M-REASSIGN, M-RETURN-PARAM. **Interim:** clone at every consuming position for clonable types and disable `apply_demotions` — turns the double-frees/UAFs into leaks (the safe direction). (§3.2)
- **Give `mir/verify.rs` a real ownership invariant** (each value dropped ≤once per path, no use-after-drop, every owned def dropped on every exit) and **run it in release** (it is currently `#[cfg(debug_assertions)]`). (M-VERIFY)
- **Decide and enforce clone-or-move for cross-boundary captures** (closures, `send`, `spawn`, actor message params) in one place. (M-CLOSURE/M-CHAN/M-ACTOR, M4)
- **Add an ABI-classification pass**: one function mapping `Type::Fn` → (LLVM signature, per-arg passing mode with `byval`/`sret`/`zeroext`), consumed by *every* caller and callee; delete the hand-rolled layout engine in favor of `TargetData`. Fixes B-HOF-STRUCT, B-CORO-CAP, B-ENUM-ALIGN, B-SELECT-BUF and the size/align disagreements. (§6.1)
- **Make `Map` grow** (load-factor rehash) and store typed key/value fields, not 8-byte slots. (B-MAP-GROW, B-MAP-VAL)
- **Fix the container ICEs**: `.sum()`/`.sort()` must handle every element type (they currently `into_int_value()` a struct/String and panic). No codegen path may `panic!` on valid input. (B-SUM, B-SORT)
- **Bounds-check fixed-size array indexing** (only `Vec` is checked today); make the comptime folder width-aware and trap-consistent with codegen (it panics on `INT_MIN / -1`). (M5, B-CHECKED)
- **Emit real debug info** (`DISubprogram`/`DILocation`) so `--debug` is usable. (B-DEBUG)

### 7.2 Runtime correctness (blockers)
12. **Fix the SSO tag convention** so codegen and `runtime/vec.c`/`process.c` agree; add an automated codegen-vs-`jinn_rt.h` signature check (the prototype block should be machine-checked against `declare_jinn_runtime`). Fixes string/vector `from..to` slicing and `spawn_exec`. **Add slicing and subprocess tests** — these bugs survived because no test exercises the operator path.
13. **`save()` durability**: fsync the data fd before `jinn_wal_checkpoint`. Same for the recover unchanged-path and `emit_store_compact`.
14. **SIGPIPE**: `signal(SIGPIPE, SIG_IGN)` at startup + `MSG_NOSIGNAL` in `jinn_send`/`sendto`.
15. **Per-store in-process mutex** — the `flock`-on-shared-fd is a no-op against coroutine concurrency.
16. **Bound the KV/index probe loops** and reclaim tombstones (grow on live+tombstone load factor), so churn can't hang the process.
17. **Saturate/guard float→int** (`llvm.fptosi.sat` or an explicit range/NaN check). (§6)
18. **Save `MXCSR`/x87 control word** in `context_x86_64.S` (~6 instructions); add a multi-page guard band or stack probes.
19. **Synchronize the supervisor** (a mutex, or funnel all supervisor mutation onto one coroutine) to close the UAF/double-free.

### 7.3 Front-end (blockers/high)
20. **Disable `fmt --write`** until the printer is total and gated on reparse-equality (prefer a trivia-preserving CST formatter). (§4.1)
21. **Depth-limit `parse_unary`/`parse_exp`/`parse_pat`**; the compiler must not core-dump on adversarial input. (§4.2)
22. **Diagnose duplicate function definitions**; flush/reject `pending_pre_stmts` at declaration level; make unreplaced `$` placeholders a hard error. (§4.4)
23. **Temp-bind chained comparison operands** (fix double-eval). (§4.3)
24. **Route lex/parse errors through `Diagnostic`** with spans/snippets and a `=`→`is` suggestion; fix UTF-8/BOM handling.

### 7.4 Networking / concurrency (high, for the runtime's core claim)
25. **Wire `net.c` through `event.c`** so blocking-looking socket I/O parks the coroutine instead of the OS thread; make the worker count configurable. Until this lands, the green-thread model doesn't deliver I/O concurrency.

### 7.5 Standard library (blockers/high)
- **Sweep `.length` → `.byte_count` wherever bytes are meant** (all crypto/uuid/hash length checks and the byte lengths passed to OpenSSL/C), or make the scanners scalar-correct. This one misuse breaks crypto with real binary keys and makes every hand-written parser truncate or crash on non-ASCII. (§6A)
- **Add a real String→int parse** and stop lowering `str as i64` to a reinterpret. (§6A)
- **Validate at the network/parse boundary**: http header CRLF, `Content-Length` in bytes, `inet_pton` return, json depth limit and no `abort()` on untrusted input, url scheme parsing after fragment-strip, `process.cmd()` quoting. (§6A)
- **Fix `raft` vote counting** (per-peer dedup + term check) — it currently admits two leaders in one term. **Add crypto/json/uuid/decimal/bigint/raft tests** (there are none; known-answer vectors would have caught most of §6A).

### 7.6 Design decisions to settle before more is built on them
26. **`store` default semantics** (§5): a bare `store X` should probably be explicit about persistence (ephemeral by default, or a required path/handle), because non-idempotent hidden global durable state keyed by a name is surprising and un-testable. Changing this later breaks every store program.
27. **Implicit error/effect discard** (§4 of the error model): a bare fallible call at statement position silently drops the error, and `x is f()` in a non-fallible function binds the raw `Result` (which `log` then happily prints as a discriminant). Decide whether "must handle or explicitly discard" should be enforced; this is a language-level choice that gets harder to change with every program written.
28. **Add a commit record to the WAL** (§5) — no per-record fsync discipline makes a multi-record/multi-store transaction atomic without one.
29. **Reconcile the three grammars** (parser, `docs/jinn.ebnf`, tree-sitter) from one conformance corpus run through parse→format→reparse in CI; the "drift is detected by tests" claim is currently false.
30. **Silent integer overflow**: wrapping is a defensible default, but offer checked/`overflowing` arithmetic, and reconcile with the fact that div/rem *do* trap — the current mix is inconsistent.

---

## 8. Comparative positioning

- **vs Python:** not comparable on performance — Jinn is AOT/LLVM/no-GC and runs in C/Rust's class (§6). It is comparable in *ergonomics ambition* (concise syntax, stores, actors) but currently far behind on safety guarantees Python users take for granted (Python never segfaults from pure Python).
- **vs C++:** Jinn's memory model is safer *in intent* than C++ (move/ownership, no manual `free`), and its concurrency primitives are more correct than most hand-rolled C++. But today it delivers *less* safety than modern C++-with-discipline: `a is s; b is s` on a string is a deterministic double-free that no C++ programmer would write, and ordinary early-return code leaks. It does not yet deliver the safety dividend that justifies the restrictions.
- **vs Rust:** the aspiration is Rust-like (ownership, error-as-value, exhaustiveness) and where the guards exist the diagnostics are Rust-quality. The gaps are enforcement and drop elaboration: Rust funnels every type obligation through one fatal pass (Jinn discards most), and Rust's drop elaboration handles exactly the conditional-move/early-exit/reassign cases Jinn leaks or double-frees. Rust's `as` is a restricted lattice; Jinn's is an unchecked bitcast. Until §7.1/§7.1a, Jinn is not in the same safety category.
- **vs Zig:** closest philosophical neighbor (error sets, explicit, systems-level, comptime). Zig checks integer-literal ranges and float→int fit; Jinn silently wraps/UB's both. Zig has no green-thread runtime; Jinn's is more ambitious and, at its core, well-built.
- **Distinctive strengths worth keeping:** first-class durable stores with a real crash-atomic primitive, structured concurrency with a correct handoff protocol, and a no-GC tree-ownership model are a genuinely interesting combination that none of the above offer together.

---

## 9. Bottom line

The skeleton is strong and, in the concurrency and durability primitives, expert. The flesh is not attached. Two independent foundations are unsound — the type checker discards most of its own checks, and the memory manager (which is static drop insertion, not the Perceus/RC it resembles) double-frees and leaks on ordinary code — and on top of them the backend is structurally unsafe for any value wider than 8 bytes, the module resolver breaks half the standard library, the stdlib misuses scalar-count as byte-count throughout its crypto and parsers, and the runtime has crashing bugs at every seam the tests don't cross.

None of this is a verdict on the *ideas*, which are good and in places better than the mainstream alternatives. It is a verdict on maturity: **the gap between what Jinn claims (memory-safe, type-safe, durable) and what it currently guarantees is very large, and a user will discover it with a segfault, a double-free, or silently corrupted data.** The encouraging part is that the failures are concentrated and mostly not deep design errors — they are missing dataflow passes, discarded check results, an absent ABI layer, and boundary bugs, on top of a foundation worth keeping. The work is large but finite and ordered in §7. The single most important structural change is to stop failing *open*: make type errors fatal, give the MIR verifier ownership invariants and run it in release, and add the one ABI/drop-elaboration pass each that the backend and memory model are missing. Until then, treat every safety claim as unproven — and do not point this at untrusted input or trust it with data you cannot lose.
