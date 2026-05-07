# Jade Runtime (`runtime/`)

The C runtime backing every compiled Jade program. ~5K LOC across 28 `.c`
files plus one shared header [jade_rt.h](jade_rt.h). Linked statically into
each `jadec`-produced binary as `libjade_rt.a` (always) plus optional
`libjade_ssl.a` (OpenSSL detected at build time) and `libjade_sqlite.a`
(sqlite3 detected at build time).

## Conventions

- All public symbols start with `jade_`. Helpers exposed for codegen-name
  reasons (`c_*`) wrap libc functions whose names collide with Jade
  identifiers; `__jade_*` are codegen-internal trampolines emitted directly
  by `src/codegen/`.
- **Resource-creating** functions return a pointer or `NULL` on failure
  (`jade_*_open`, `jade_*_create`, `jade_*_prepare`).
- **Mutating** functions return `int` with `0`/positive on success and
  `-1` on failure (`c_mkdir`, `jade_chmod`, `jade_close`).
- **Query** functions return the queried value directly (`int64_t` count,
  `double` aggregate, `const char *` for borrowed strings) and use a sentinel
  for "missing" where applicable.
- File-descriptor-style state lives in opaque structs whose definition is in
  the owning `.c` file; the typedef is forward-declared in
  [jade_rt.h](jade_rt.h) (e.g. `JadeKV`, `JadeIndex`, `jade_chan_t`).
- Cross-module helpers (FNV-1a, sorts, bit-cast) live in
  [util.c](util.c) — never re-implement them.
- The runtime never prints to `stderr` from a hot path; only fatal allocator
  failures call `abort()` (see `jade_xmalloc`).

## Build flags

Compiled with `-O2 -Wall -Wextra -Wshadow -Wstrict-prototypes
-Wmissing-prototypes -Wno-unused-parameter`. Warnings are errors in CI
(see [build.rs](../build.rs)).

## File map

Each `.c` file owns a single subsystem and exposes its public surface in
[jade_rt.h](jade_rt.h). Codegen call sites for each module live in the
corresponding `src/codegen/` file.

| File | Subsystem | Codegen client(s) |
|---|---|---|
| `actor.c` | Actor lifecycle (park / wake / stop / destroy) | `src/codegen/actors.rs`, `src/codegen/mir_codegen/concurrency.rs` |
| `bloom.c` | Bloom filter store extension | `src/codegen/mir_codegen/store_ext.rs` |
| `channel.c` | Bounded/unbounded channels | `src/codegen/channels.rs` |
| `column.c` | Columnar store extension (sum/min/max/avg) | `src/codegen/mir_codegen/store_ext.rs` |
| `coro.c` | Coroutine + generator suspend/resume | `src/codegen/coroutines.rs` |
| `coro.c` + `context_*.S` | Architecture-specific stack switch | linked in `build.rs` |
| `crypto.c` | SHA-256/512, HMAC, AES-GCM, RNG (OpenSSL) | `std/crypto.jade` |
| `deque.c` | Work-stealing deque (used by scheduler) | `runtime/sched.c` |
| `event.c` | Async I/O event loop (poll/epoll/kqueue) | `src/codegen/builtins.rs` (io waits) |
| `fs.c` | Filesystem helpers + libc wrappers | `std/fs.jade` |
| `fts.c` | Full-text search store extension | `src/codegen/mir_codegen/store_ext.rs` |
| `index.c` | Hash-index store extension | `src/codegen/mir_codegen/store.rs` (where-clauses) |
| `kv.c` | Key/value store extension | `src/codegen/mir_codegen/store_ext.rs` |
| `migrate.c` | Schema migration log + field add/drop | `src/codegen/mir_codegen/store.rs` |
| `net.c` | BSD socket wrappers (collision-free names) | `std/net.jade` |
| `pool.c` | Slab pool allocator | `src/codegen/builtins.rs` |
| `process.c` | Spawn/exec/system + Vec<String> ABI helpers | `std/process.jade` |
| `regex_helper.c` | PCRE2 ovector accessor | `std/regex.jade` |
| `sched.c` | Worker pool + run queue | `src/codegen/coroutines.rs` (spawn) |
| `select.c` | `select { … }` multiplexer | `src/codegen/mir_codegen/concurrency.rs` |
| `sqlite.c` | SQLite3 binding (optional) | `std/sqlite.jade` |
| `sup.c` | Supervisor (one-for-one restart) | `src/codegen/mir_codegen/concurrency.rs` |
| `timer.c` | Monotonic time + timer wheel | `src/codegen/builtins.rs` |
| `tls.c` | TLS client + DNS resolver (OpenSSL) | `std/tls.jade` |
| `util.c` | FNV-1a, sorts, bit-cast, xmalloc | shared |
| `vec.c` | SSO string ABI + slice helpers + deque ABI | `src/codegen/strings.rs`, `src/codegen/vec.rs` |
| `vector.c` | Vector / nearest-neighbour store extension | `src/codegen/mir_codegen/store_ext.rs` |
| `version.c` | Versioned record store extension | `src/codegen/mir_codegen/store_ext.rs` |
| `wal.c` | Write-ahead log + group commit + replay | `src/codegen/mir_codegen/store.rs` |

## Adding a new runtime function

1. Implement in the appropriate `.c` file. Use `static` for helpers; only
   public symbols (callable from generated LLVM IR) need to be visible.
2. Add the prototype to [jade_rt.h](jade_rt.h) under the matching `runtime/X.c`
   comment block.
3. If the function needs a new opaque type, declare it as
   `typedef struct Foo Foo;` in `jade_rt.h` and `struct Foo { … };` in the `.c`
   file (so a single forward declaration is sufficient at every call site).
4. The build picks up new files via the `cc::Build::file` calls in
   [build.rs](../build.rs); add yours there.
5. Verify zero warnings: `cargo build` (no `-Wmissing-prototypes` should
   trigger).
