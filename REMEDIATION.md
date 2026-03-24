    # Jade 0.2.0 → 0.5.0 Remediation Plan

**Date:** March 23, 2026
**Baseline:** 753 tests passing (245 integration + 508 bulk), 25.3K LOC, clean release build, zero warnings

---

## Audit Summary

| Phase | Planned Items | Shipped | Gap |
|-------|--------------|---------|-----|
| 0.3.0 — Iterators & Generics | 4 | 4 | None |
| 0.4.0 — Concurrency | 5 | 4 | Actor migration to scheduler |
| 0.5.0 — Ecosystem | 4 | 2 | LSP server, CLI subcommands |

### What's Fully Shipped

- **0.3.0 Trait bounds on generics** — `TypeParam` with bounds, bound checking during monomorphization, built-in trait satisfaction for primitives. 8 tests.
- **0.3.0 Iterator trait** — `Iter` trait in prelude, `desugar_for_iter()` (~160 LOC), `ExprKind::IterNext`, pointer-based `self` for mutation. 10+ tests.
- **0.3.0 Associated types** — `assoc_types` on `TraitDef`, parser handles `type Item is I64`, validation of missing bindings. 6 tests.
- **0.3.0 String iteration** — Byte-level for-loop iteration, integrated with Iter protocol.
- **0.4.0 Stackful coroutines** — `runtime/coro.c` (136 LOC), x86_64 + aarch64 asm context switch, 8KB stacks with guard pages.
- **0.4.0 Typed channels** — `Type::Channel(Box<Type>)`, `runtime/channel.c` (211 LOC), MPMC ring buffer, full codegen.
- **0.4.0 M:N scheduler** — `runtime/sched.c` (238 LOC), `runtime/deque.c` (86 LOC), Chase-Lev work-stealing, init/run/shutdown wired into main.
- **0.4.0 Select** — `runtime/select.c` (236 LOC), Fisher-Yates fairness, address-ordered locking, `compile_select()` complete.
- **0.5.0 Package manager core** — `pkg.rs` (134 LOC), `lock.rs` (93 LOC), `cache.rs` (162 LOC). Manifest parsing, lockfile, cache manager, module resolution all working.
- **0.5.0 Standard library** — 8 modules (io, os, math, fmt, time, sort, rand, path). 15+ builtins wired. argc/argv forwarding done.
- **0.5.0 Comptime (implicit)** — `comptime.rs` (310 LOC) does constant folding for arithmetic, math builtins, casts, ternary, string concat.

### What's Missing or Incomplete

| Issue | Severity | Description |
|-------|----------|-------------|
| Actor migration | **CRITICAL** | Actors still use `pthread_create` + mutex/condvar, not the scheduler |
| `jade_actor_wake()` stub | **CRITICAL** | No-op function in `runtime/actor.c` |
| Channel/select/coroutine tests | **HIGH** | Zero test coverage for all 0.4.0 concurrency primitives |
| Stdlib tests | **HIGH** | Zero test coverage for all 8 standard library modules |
| Package manager tests | **HIGH** | Zero test coverage for pkg/lock/cache parsing |
| CLI subcommands | **MEDIUM** | No `jadec init`, `jadec fetch`, `jadec update` commands |
| LSP server | **MEDIUM** | No `src/lsp/` directory, no `jadec-lsp` binary, no serde deps |
| Explicit comptime keyword | **LOW** | Only implicit constant folding, no `comptime { }` blocks |
| Perceus catch-all arms | **LOW** | 5 silent `_ => {}` patterns in perceus.rs analysis passes |
| Concurrency benchmarks | **LOW** | No channel/coroutine/select benchmarks for perf validation |

---

## TIER 1 — Critical (Must Fix Before Release)

### 1.1 Migrate Actors to Scheduler

**Problem:** The entire scheduler infrastructure (coroutines, channels, work-stealing, select) is built and working, but actors — the original concurrency primitive — are completely disconnected from it. Every `spawn Actor` calls `pthread_create`. Every `send` uses `pthread_mutex_lock/unlock/cond_signal`. This limits actors to ~10K (one OS thread per actor, 8MB stack each), far from the "millions of concurrent tasks" target.

**Files to change:**

| File | Action |
|------|--------|
| `src/codegen/actors.rs` | Rewrite `compile_spawn()`, `compile_send()`, `compile_actor_loop()` |
| `runtime/actor.c` | Implement `jade_actor_wake()` properly |

**Changes in `src/codegen/actors.rs`:**

1. **`compile_spawn()`** (currently ~130 lines, lines 396-550):
   - Replace `pthread_create(thread, NULL, Actor_loop, mailbox)` + `pthread_detach(thread)` with:
     ```
     %coro = call ptr @jade_coro_create(ptr @Actor_loop, ptr %mailbox)
     store ptr %coro -> mailbox.actor_coro   ; store coro pointer in mailbox
     call void @jade_sched_spawn(ptr %coro)
     ```
   - Remove `pthread_mutex_init`, `pthread_cond_init` calls for condvars
   - Keep the mailbox struct layout for the ring buffer, but remove mutex/condvar fields
   - Estimated: ~130 lines → ~50 lines

2. **`compile_send()`** (currently ~150 lines, lines 551-750):
   - Replace mutex lock → enqueue → signal → unlock with:
     ```
     ; Atomic enqueue into ring buffer (MPSC - single consumer is actor)
     %tail = atomic load mailbox.tail
     %idx = and %tail, (cap - 1)
     memcpy(buffer[idx], &message, msg_size)
     atomic store mailbox.tail, %tail + 1
     ; Wake actor if suspended
     %coro = load mailbox.actor_coro
     %state = load coro.state
     if state == SUSPENDED:
       call void @jade_sched_enqueue(ptr %coro)
     ```
   - Estimated: ~150 lines → ~60 lines

3. **`compile_actor_loop()`** (currently ~230 lines, lines 167-390):
   - Replace `pthread_cond_wait` with `jade_actor_park(mailbox_ptr)` (which does `jade_context_swap` back to scheduler)
   - Remove all `pthread_mutex_lock/unlock` from the dequeue loop
   - Keep the switch-on-tag handler dispatch exactly as-is
   - Add `jade_coro_yield()` after each message dispatch for fairness
   - Estimated: ~230 lines → ~120 lines

4. **`declare_actor_runtime()`** (lines 28-70):
   - Remove all pthread function declarations
   - Replace with references to already-declared scheduler functions (`jade_coro_create`, `jade_sched_spawn`, `jade_sched_enqueue`, `jade_actor_park`)

5. **Mailbox struct layout change:**
   - Remove: 40-byte mutex, 48-byte cond_notempty, 48-byte cond_notfull (136 bytes saved)
   - Add: `ptr actor_coro` field (8 bytes) — pointer to the coroutine running this actor
   - Keep: buf_ptr, cap, head, tail, count, alive, state_struct — all unchanged
   - Update all GEP offsets throughout the file

**Changes in `runtime/actor.c`:**

Implement `jade_actor_wake()`:
```c
void jade_actor_wake(void *mailbox_ptr) {
    /* The actor_coro pointer is at a known offset in the mailbox struct.
     * The compiler stores it there during spawn. We cast to access it. */
    jade_coro_t **coro_slot = (jade_coro_t **)((char *)mailbox_ptr + ACTOR_CORO_OFFSET);
    jade_coro_t *coro = *coro_slot;
    if (coro && coro->state == JADE_CORO_SUSPENDED && coro->wait_chan == mailbox_ptr) {
        coro->state = JADE_CORO_READY;
        coro->wait_chan = NULL;
        jade_sched_enqueue(coro);
    }
}
```

Note: The `ACTOR_CORO_OFFSET` needs to match the codegen's mailbox struct layout. Alternatively, the compiler can inline the wake logic (avoiding the offset constant), which is what the current comment in actor.c suggests. The cleanest approach: the compiler emits the wake inline in `compile_send()` using GEP to the coro field, avoiding any offset magic in C.

**Validation:**
- All existing actor tests must pass unchanged (same Jade syntax: `spawn`, `send`, `@handler`)
- New tests: `actor_spawn_10k`, `actor_spawn_100k`, `actor_stop`, `actor_reply_channel`
- Re-run actor benchmarks: `actor_pingpong`, `actor_throughput`, `actor_fanout` — expect significant improvement

**Estimated effort:** 2-3 days
**Risk:** Medium — functional behavior preserved, only transport mechanism changes

---

### 1.2 Add Channel, Select, and Coroutine Tests

**Problem:** Core 0.4.0 features have zero test coverage. They exist in codegen and runtime but are unvalidated.

**Tests to add in `tests/bulk_tests.rs`:**

#### Channel tests (~80 LOC):
```
b_channel_create_i64           — create channel of i64, verify type
b_channel_send_recv_basic      — send 3 values, receive 3 values, verify order
b_channel_bounded_blocking     — fill capacity-2 channel, verify backpressure
b_channel_close_recv           — close channel, verify receiver gets nothing/zero
b_channel_typed_string         — send/recv String values
b_channel_typed_struct         — send/recv struct values
b_channel_multi_producer       — 4 coroutines send to one channel, 1 receiver, verify all arrive
b_channel_pipeline             — chan A → transform → chan B → verify
```

#### Select tests (~60 LOC):
```
b_select_one_ready             — select over 2 channels, one has data, correct arm executes
b_select_default               — select with default when no channels ready
b_select_timeout               — select with timeout arm
b_select_send_arm              — select with send-to-channel arm
```

#### Coroutine/dispatch tests (~40 LOC):
```
b_dispatch_basic_yield         — dispatch block yields values, for-in consumes
b_dispatch_sum_yields          — sum values from dispatch block
b_dispatch_early_break         — break out of dispatch consumer loop
b_coroutine_channel_bridge     — dispatch yields via internal channel
```

**Estimated effort:** 1 day

---

## TIER 2 — Important (Should Fix)

### 2.1 Add CLI Subcommands for Package Manager

**Problem:** Package manager core (pkg.rs, lock.rs, cache.rs, module resolution) is fully implemented but there's no way to invoke it from the command line. Users must create `jade.pkg` files manually.

**Changes in `src/main.rs`:**

Add `clap` subcommand support:

```rust
#[derive(Subcommand)]
enum Command {
    /// Compile a .jade file (default when no subcommand)
    Build(BuildArgs),
    /// Initialize a new package
    Init {
        #[arg(default_value = ".")]
        name: Option<String>,
    },
    /// Fetch and cache all dependencies
    Fetch,
    /// Re-resolve dependencies and update jade.lock
    Update,
}
```

- `jadec init myproject` — creates `jade.pkg` with name + version 0.1.0
- `jadec fetch` — resolves + downloads all deps to `~/.jade/cache/`
- `jadec update` — deletes lockfile, re-resolves, writes new lockfile
- `jadec file.jade` (no subcommand) — remains the default compile flow, backward compatible

**Estimated effort:** 0.5 days

### 2.2 Add Standard Library Tests

**Problem:** All 8 stdlib modules have real implementations but zero tests.

**Tests to add (~150 LOC):**

```
b_std_math_constants           — use std.math; verify PI, E values via computation
b_std_math_functions           — factorial, gcd, lcm, hypot, lerp
b_std_fmt_pad                  — pad_left, pad_right
b_std_fmt_radix                — hex, oct, bin conversion
b_std_fmt_join                 — join with separator
b_std_sort_basic               — sort a Vec of i64, verify order
b_std_sort_already_sorted      — sort pre-sorted input, verify no regression
b_std_sort_reverse             — sort reverse-ordered input
b_std_rand_deterministic       — seed PRNG, verify reproducible sequence
b_std_rand_range               — range() stays within bounds
b_std_path_join                — path_join with/without trailing slash
b_std_path_components          — path_dir, path_base, path_ext
b_std_time_monotonic           — monotonic() returns increasing values
b_std_io_roundtrip             — write_file + read_file round-trip in temp dir
```

**Estimated effort:** 1 day

### 2.3 Add Package Manager Tests

**Problem:** pkg.rs, lock.rs, cache.rs have no test coverage.

**Tests to add (~100 LOC):**

```
b_pkg_parse_basic              — parse simple jade.pkg manifest
b_pkg_parse_multiple_requires  — parse manifest with multiple dependencies
b_pkg_parse_malformed          — reject malformed input with error
b_lock_parse_write_roundtrip   — parse lockfile, write it back, verify byte-identical
b_lock_parse_transitive        — parse lockfile with indented transitive deps
b_module_resolve_local         — use a.b.c resolves to local file
b_module_resolve_package       — use pkgname.module resolves to cache path
```

**Estimated effort:** 0.5 days

### 2.4 Add Comptime Folding Tests

**Problem:** Constant folding runs but has no test coverage.

**Tests to add (~50 LOC):**

```
b_comptime_int_arithmetic      — verify 2 + 3 * 4 folds to 14 at compile time
b_comptime_float_fold          — verify 1.0 / 3.0 folds correctly
b_comptime_bool_fold           — verify true and false folds to false
b_comptime_string_concat       — verify "hello" + " world" folds
b_comptime_cast_fold           — verify 42 as f64 folds to 42.0
b_comptime_math_builtin        — verify ln(1.0) folds to 0.0
b_comptime_nested              — verify (2 + 3) * (4 + 5) folds to 45
```

Note: These tests verify correct output (the folding is transparent). To verify actual folding, use `--emit-ir` and check that constants appear in the IR rather than runtime operations.

**Estimated effort:** 0.5 days

---

## TIER 3 — Improvement (Nice to Have)

### 3.1 Add Concurrency Benchmarks

**Problem:** No benchmarks validate the performance claims in the 0.4.0 plan (channel throughput, coroutine spawn latency, select latency).

**Benchmarks to add:**

| File | Description | Metric |
|------|-------------|--------|
| `benchmarks/channel_throughput.jade` | Send/recv 1M i64 values through buffered channel | msgs/sec |
| `benchmarks/coroutine_spawn.jade` | Spawn 100K coroutines, each increments counter | total time |
| `benchmarks/select_latency.jade` | Select over 4 channels in tight loop | ns/select |

Add C and Rust comparison implementations in `benchmarks/comparison/`.

**Estimated effort:** 1 day

### 3.2 LSP Server

**Problem:** No IDE support beyond syntax highlighting. The plan calls for diagnostics, hover, go-to-definition, document symbols, and basic completion.

**Implementation outline:**

| File | LOC | Purpose |
|------|-----|---------|
| `src/lsp/main.rs` | ~250 | Server main loop, message dispatch |
| `src/lsp/transport.rs` | ~120 | JSON-RPC over stdio |
| `src/lsp/protocol.rs` | ~200 | LSP type definitions (subset) |
| `src/lsp/analysis.rs` | ~300 | Analysis engine (lex → parse → type check, no codegen) |
| `src/lsp/handlers.rs` | ~200 | Request handlers (hover, goto-def, symbols, completion) |
| `src/lsp/mod.rs` | ~10 | Module declarations |

**Dependency changes in `Cargo.toml`:**
```toml
[[bin]]
name = "jadec-lsp"
path = "src/lsp/main.rs"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

**Typer changes** (~30 LOC):
- Add `span_types: Vec<(Span, String, Type)>` for hover
- Add `def_spans: HashMap<DefId, Span>` for go-to-definition
- Add `ref_spans: Vec<(Span, DefId)>` for references
- Populate during existing lowering pass (5 insertion sites)

**VS Code extension changes:**
- Add `vscode-jade/extension.js` (~30 LOC) with `LanguageClient`
- Add `vscode-languageclient` dependency to `package.json`

**Estimated effort:** 3-5 days

### 3.3 Explicit `comptime` Keyword

**Problem:** Only implicit constant folding exists. The plan describes a Zig-style `comptime { }` block with an AST interpreter for static assertions, lookup table generation, and conditional compilation.

**Implementation:**
- Add `Token::Comptime` to lexer
- Add `Decl::Comptime(Vec<Stmt>, Span)` and `Expr::Comptime(Vec<Stmt>, Span)` to AST
- Expand comptime blocks between parsing and type checking
- AST interpreter with execution budget (1M steps max)
- Support: arithmetic, math, assertions, arrays, conditionals, loops

**Estimated effort:** 2-3 days

### 3.4 Harden Perceus Match Arms

**Problem:** 5 `_ => {}` catch-all patterns in `perceus.rs` (lines 446, 813, 930, 1060, 1112) silently ignore unhandled expression/statement types. If new AST nodes are added, Perceus won't analyze them, potentially causing incorrect drop elision or memory leaks.

**Fix:** Replace each `_ => {}` with an explicit list of the handled variants, then add `_ => unreachable!("unhandled variant in Perceus: {stmt:?}")` or use `#[deny(unreachable_patterns)]` to get compile-time enforcement.

**Estimated effort:** 0.5 days

---

## Dependency Graph

```
[1.1 Actor Migration] ──→ [1.2 Concurrency Tests] ──→ [3.1 Concurrency Benchmarks]
[2.1 CLI Subcommands] (independent)
[2.2 Stdlib Tests] (independent)
[2.3 Pkg Manager Tests] (independent)
[2.4 Comptime Tests] (independent)
[3.2 LSP Server] (independent)
[3.3 Explicit Comptime] (independent)
[3.4 Perceus Hardening] (independent)
```

Items 2.1–2.4 are all independent and can be parallelized. Item 1.2 depends on 1.1 (actor tests need the new scheduler-based actors). Item 3.1 depends on 1.2 (benchmarks need working concurrency).

---

## Execution Order

1. **1.1 Actor Migration** — Rewrite codegen, remove pthreads, wire to scheduler
2. **1.2 Concurrency Tests** — Validate channels, select, coroutines, migrated actors
3. **2.1 CLI Subcommands** — Make package manager usable
4. **2.2 Stdlib Tests** — Validate all 8 standard library modules
5. **2.3 Pkg Manager Tests** — Validate manifest/lockfile parsing
6. **2.4 Comptime Tests** — Validate constant folding
7. **3.1 Concurrency Benchmarks** — Performance validation
8. **3.4 Perceus Hardening** — Replace catch-all match arms
9. **3.2 LSP Server** — Developer experience
10. **3.3 Explicit Comptime** — Advanced metaprogramming

---

## Success Criteria

- [ ] All existing 753 tests pass (zero regressions)
- [ ] Actor `spawn` uses `jade_coro_create` + `jade_sched_spawn`, not `pthread_create`
- [ ] Actor `send` uses atomic enqueue + scheduler wake, not mutex/condvar
- [ ] `jade_actor_wake()` in runtime/actor.c is fully implemented
- [ ] Channel tests: 8+ tests covering create/send/recv/close/typed/multi-producer
- [ ] Select tests: 4+ tests covering ready/default/timeout/send-arm
- [ ] Coroutine tests: 4+ tests covering yield/sum/break/channel-bridge
- [ ] Stdlib tests: 14+ tests covering all 8 modules
- [ ] Pkg manager tests: 7+ tests covering parse/roundtrip/resolve
- [ ] Comptime tests: 7+ tests covering arithmetic/float/bool/string/cast/builtin/nested
- [ ] CLI subcommands: `jadec init`, `jadec fetch`, `jadec update` functional
- [ ] Actor benchmarks show improvement over pthread baseline
- [ ] Zero compiler warnings in release build
- [ ] Total test count: 800+ (up from 753)
