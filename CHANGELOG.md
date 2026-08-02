# Changelog
- **[138]** (2026-08-02 18:59) tests run in their own cwd; add gitignored .data/ for ad-hoc runs

A store resolves its .store/.wal relative to the process cwd, so any test
that compiled into a tempdir but ran the binary with the harness's cwd
wrote its state into the repo root. 18 run-sites across 11 test files did
exactly that; tests/error_effects.rs was the live offender, leaving
items.* and jobs.* behind on every `cargo test`.

Every compiled-binary run site now passes .current_dir(dir.path()) (or the
harness's scratch dir). A full suite run now leaves the root clean.

Added .data/ — a gitignored scratch cwd for running programs with
persistent state by hand, with a tracked README explaining why it exists.
Benchmarks already isolate into benchmarks/_build/cwd_*; unchanged.

Removed 30 stale .store/.wal files from the root (untracked, already
covered by .gitignore).
- **[137]** (2026-08-02 18:25) std gate green: all 49 alpha-stable modules type-check

The last failing test suite is now passing. Full suite 2013/2013, zero
failures, clippy clean, fmt clean, apps 21/21, benchmarks 36/36.

Two compiler defects found by working the gate:

1. extern opaque-pointer rule. A declared `%i8`/`%u8`/`%void` parameter is
   C's opaque-handle convention (`char *`/`void *`). The typer accepted a
   String there but rejected every other raw pointer, so passing a vec
   header to `jinn_spawn_exec(const void *)` was impossible — std/process
   could not bind its own runtime. At an extern boundary the C signature
   is the user's assertion; any raw pointer now satisfies an opaque
   pointee. Non-opaque pointees still unify strictly.

2. `copy` is a binding modifier, not an expression (access-semantics.md
   §2), so `copy self` in tail position fails. std/dataframe leaned on
   `return self` from a by-ptr method instead, handing out a borrow where
   the signature promised an owned DataFrame. Added `__clone_df` and
   routed all 7 "no matching column" paths through it.

std source fixes: bangle/http/event/url/terminal/dataframe. Types are
global and unqualified (std.md), so error unions are written `! NetError`,
not `! net.NetError`. http's PoolEntry.fd was i64 while every socket API
is i32.

New regression tests: method + ptr-method `! E` unions, no-Debug-spans in
error diagnostics, extern opaque-pointer FFI round-trip through a real
subprocess.
- **[136]** (2026-08-02 18:12) std gate: remove sqlite; fix `! E` on methods; clean error diagnostics

Removed std/sqlite.jn — the wrapper never type-checked and shipping a
broken binding is worse than shipping none. runtime/sqlite.c stays: it is
live FFI with a passing integration test, usable via `extern
*jinn_sqlite_*`. build.rs/jinn_rt.h/std.md updated.

Compiler bug found while fixing std/net: `! E` was desugared to
`Result of T, E` for free functions (resolve.rs:140) but BOTH method
signature paths dropped f.error_types, so `err X` inside any method was
rejected with "this function returns T". Methods now honor `! E`
end-to-end through codegen.

Six user-facing diagnostics printed raw `Span { start, end, line, .. }`
Debug output. All now use span.loc() -> file:line:col.

std sources fixed:
- argon/rational/date/logging: `"" + n` is not concatenation; the typer
  deliberately rejects it. Use to_string(n), the idiom the rest of std
  already uses.
- net: replaced the `return false` C-ism sentinel with a real `err
  NetError` union. This is what the error-effect system is for, and it
  unblocked bangle/http/event which consumed the sentinel API.

tests/programs/actors_multi.jn was racy — two independent actors have no
cross-mailbox ordering guarantee, so the snapshot flaked ~40% of runs.
Sequenced the drains; 20/20 deterministic.

std gate 13 -> 7 failing modules. clippy clean, fmt clean.
- **[135]** (2026-08-02 17:37) review eval: P1-6 — mir-drops summary is behind --debug-drops only

The driver printed the mir-drops summary on any compile where the pass
did useful work, so a clean user build leaked internal optimizer
telemetry to stderr. The roadmap's B.3 requires internal info to sit
behind an explicit flag. Gate solely on cli.debug_drops; tests/drops_debug
already passes the flag explicitly, so coverage is unchanged.

Tests 2008 passing, clippy clean, fmt clean.
- **[134]** (2026-08-02 17:35) review eval: reject unknown constructors; restore fmt gate

P0-class soundness hole found while auditing the std gate: the typer's
struct-literal path fell through to Type::Struct(name) for ANY unresolved
name, so `x is TotallyUndefinedThing()` compiled, linked, and ran. Every
typo in a constructor position was silently accepted, and the resulting
fabricated type is what produced the misleading "expected `None`, found
`Option__G_i64`" cascade in std/collections.jn.

- Reject constructor calls that name no declared type/actor/variant.
- Levenshtein-based "did you mean" over known types and variants.
- Name the Jinn spelling directly for foreign prelude words
  (None/Null/Nil -> Nothing, Just -> Some, Error -> Err).
- Fix spec divergence: jinn.md said `None`, implementation and
  error-effects.md/fmt.md say `Nothing`. `Nothing` wins; jinn.md and
  std/collections.jn corrected.
- cargo fmt --all: 58 files were drifted at HEAD, so CI's fmt gate was
  already red. Now clean.

std gate 14 -> 13 failing modules. Tests 2004 -> 2008. clippy clean,
fmt clean, apps 21/21, benchmarks 36/36.
- **[133]** (2026-08-02 17:23) review eval: fix consume-and-rebind false positive in move tracking

The post-lowering move pass (record_take_moves_in_stmt) re-marked a
consumed call argument as moved without observing that the enclosing
Bind/Assign re-initialized the target. `g is grow(g, x)` — the canonical
builder idiom — was therefore rejected as use-after-move, breaking 6 of
21 sample apps (blockchain_node, inventory_mgr, iot_telemetry, kv_store,
order_book, route_planner).

Clear the target's moved-state after recording the statement's moves, in
both Bind and Assign. Revival is precise: rebinding a different name
still leaves the source tombstoned.

apps 15/21 -> 21/21. Tests 2001 -> 2004 passing (3 new memory_model
regression tests pinning M2 revival: bind form, loop form, and the
negative case).
