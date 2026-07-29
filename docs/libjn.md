# libjn — A Drop-in Replacement for the C Standard Library, in Idiomatic Jinn

> **Status:** Design / Alpha-stub. Every header file in [libjn/](libjn/) has
> compileable Jinn stubs (`nop` bodies) that match the C ABI signature.
> Bodies will be filled in over the milestones in §13.

---

## Table of Contents

1. [Vision & Rationale](#1-vision--rationale)
2. [Architecture & File Layout](#2-architecture--file-layout)
3. [Naming, Style & Type Conventions](#3-naming-style--type-conventions)
4. [Namespacing — `std.libjn.*` and `compat`](#4-namespacing--stdlibjn-and-compat)
5. [Exception-Handling Strategy](#5-exception-handling-strategy)
6. [Language Limitations & Remediation Plan](#6-language-limitations--remediation-plan)
7. [Variadic Functions — A Special Case](#7-variadic-functions--a-special-case)
8. [Memory Model & Ownership Contracts](#8-memory-model--ownership-contracts)
9. [Thread-Safety Classification](#9-thread-safety-classification)
10. [Performance Considerations](#10-performance-considerations)
11. [Safety Considerations](#11-safety-considerations)
12. [Testing Strategy](#12-testing-strategy)
13. [Roadmap & Milestones](#13-roadmap--milestones)
14. [Exhaustive Task & Subtask Breakdown](#14-exhaustive-task--subtask-breakdown)
15. [Formal Technical Specification](#15-formal-technical-specification)
16. [Open Questions & Future Work](#16-open-questions--future-work)

---

## 1. Vision & Rationale

### 1.1 Why a libjn?

C's standard library is the most widely used API in the history of computing.
Every operating system kernel, every embedded firmware, every interpreter,
every database, every browser — touches it. Replacing it with a pure-Jinn
implementation is a forcing function for the language: it surfaces every
deficiency in our type system, FFI surface, codegen, runtime, and standard
library design.

`libjn` exists for four reasons:

1. **Proof.** A language that cannot reproduce libc cannot replace C. Period.
   This is the gauntlet every systems language has thrown itself through —
   Rust's `libstd` over `libc` crate, Zig's `std.c`, Go's `runtime/cgo`,
   D's `core.stdc`. Jinn must do the same.

2. **Compatibility shim.** Existing C programs, ported header-by-header to
   Jinn, can rely on `use libjn.compat` and have all the familiar names in
   scope. This is essential for the porting story (e.g. moving CPython,
   sqlite, redis to Jinn).

3. **Audit & safety surface.** A Jinn-native libc lets us *replace* the C
   functions at link time with bounds-checked, leak-tracked, signal-safe
   variants — without recompiling the C code. The `compat` shim becomes the
   point at which we audit all foreign control flow.

4. **Self-hosting.** The Jinn compiler currently links libc transitively
   through LLVM and the platform CRT. Replacing libc with libjn shrinks the
   trusted computing base of a Jinn program from `compiler ∪ libc ∪ kernel`
   to `compiler ∪ libjn ∪ kernel`. Every byte of libjn we write is a byte
   of libc we no longer trust.

### 1.2 Non-goals

- **Bug-for-bug glibc compatibility.** Where glibc has undefined behavior
  (e.g., `strtok` mutating its argument, `gets` having no bound), libjn will
  document and may *deviate* — typically by returning an error.
- **Locale-perfect ICU coverage.** The C locale model is a historical wart.
  libjn implements the `"C"` locale faithfully and the `"C.UTF-8"` locale
  to a useful subset; full ICU lives in `std.intl`.
- **Wide-character correctness for legacy codepages.** `wchar_t` is pinned
  to UTF-32 (i32). EBCDIC, Shift-JIS, etc. live in `std.text`.
- **Static-link compatibility with glibc symbol versioning.**
  `__GLIBC_2_34` etc. are not exported.

### 1.3 Design ethos

> **Idiomatic Jinn first, C-shape second.** Every libjn function exposes a
> C-ABI signature for the FFI boundary, but its *implementation* is written
> as the Jinn programmer would write it: bounds-checked slices, pattern
> matching, `Result` returns wrapped at the boundary, no manual goto-cleanup.

---

## 2. Architecture & File Layout

```
libjn/
├── mod.jn              # aggregator; re-exports every header
├── compat.jn           # global-namespace shim (mirrors `std.compat`)
│
├── assert.jn           # ISO C
├── complex.jn
├── ctype.jn
├── errno.jn
├── fenv.jn
├── float.jn
├── inttypes.jn
├── iso646.jn
├── limits.jn
├── locale.jn
├── math.jn
├── setjmp.jn           # ★ exception substrate — see §5
├── signal.jn
├── stdalign.jn
├── stdarg.jn           # ★ variadic substrate — see §7
├── stdatomic.jn
├── stdbool.jn
├── stddef.jn
├── stdint.jn
├── stdio.jn
├── stdlib.jn
├── stdnoreturn.jn
├── string.jn
├── tgmath.jn
├── threads.jn
├── time.jn
├── uchar.jn
├── wchar.jn
├── wctype.jn
│
├── dirent.jn           # POSIX
├── fcntl.jn
├── pthread.jn
├── semaphore.jn
├── sys_mman.jn         # POSIX <sys/mman.h>  (path-flattened)
├── sys_socket.jn       #          <sys/socket.h>
├── sys_stat.jn         #          <sys/stat.h>
├── sys_types.jn        #          <sys/types.h>
├── sys_wait.jn         #          <sys/wait.h>
└── unistd.jn
```

### 2.1 Why one Jinn file per C header?

Three reasons:

- **Discoverability.** A C programmer reaching for `<stdio.h>` finds it at
  `libjn/stdio.jn`. The mapping is mechanical.
- **Granular import.** A program that only needs `string` and `stdio`
  pulls in only those two compilation units; the symbol table for libjn
  in a small program is tiny.
- **Header-isolated forward declarations.** Each file may declare its own
  opaque types (`FILE`, `DIR`, `pthread_t`) without worrying about
  cross-file ordering — `use std.libjn.<x>` is acyclic by design.

### 2.2 Why path-flattening for POSIX `<sys/...>`?

Jinn module paths use `.`, but filenames must avoid `/`. We map
`<sys/stat.h>` to `libjn/sys_stat.jn` and expose it as `std.libjn.sys_stat`.
A future Jinn release that adds nested module folders will let us migrate
to `libjn/sys/stat.jn` transparently — `compat.jn` will still work.

### 2.3 Linkage model

libjn is an *idiomatic Jinn library*. It does not currently link the host
libc. There are two link modes:

- **Pure mode (`--no-libc`):** all libjn implementations are written in
  Jinn over direct syscalls (Linux: `syscall(2)`; macOS: `syscall(2)` to
  the Mach trap table; Windows: NT system services). This is the long-term
  goal and is the only mode that can claim "drop-in replacement."
- **Bridge mode (default during alpha):** libjn functions delegate to host
  libc via FFI declarations (`extern *<name>(...)`). This lets us bring
  libjn up incrementally — every function not yet implemented natively
  forwards to glibc/musl/Apple libSystem.

The boundary is determined per-function at build time. A function table in
`build.rs` lists which functions are "native" vs "bridged" per platform.

---

## 3. Naming, Style & Type Conventions

### 3.1 Function declarations

```
*funcname(arg1 as type1, arg2 as type2) returns ReturnType
    nop
    default_value
```

- Leading `*` marks a function definition (Jinn convention).
- Stub bodies use `nop` followed by a default return value (e.g. `0`,
  `0.0`, `0 as %i8`, `false`).
- Void-returning functions omit the value: `nop` alone.

### 3.2 Type spellings

| C type            | Jinn (libjn) type    | Rationale |
|-------------------|----------------------|-----------|
| `char`            | `i8`                 | Matches POSIX signedness on Linux. |
| `unsigned char`   | `u8`                 |  |
| `short`           | `i16`                |  |
| `int`             | `i32`                | LP64. |
| `long`            | `i64`                | LP64. |
| `long long`       | `i64`                |  |
| `unsigned int`    | `u32`                |  |
| `unsigned long`   | `u64`                |  |
| `size_t`          | `u64`                | LP64. |
| `ssize_t`         | `i64`                |  |
| `ptrdiff_t`       | `i64`                |  |
| `off_t`           | `i64`                |  |
| `float`           | `f32`                |  |
| `double`          | `f64`                |  |
| `long double`     | `f64`                | LP64 platforms ship 80-bit but we expose 64; see §6.5. |
| `void`            | (none / no return)   |  |
| `void *`          | `%i8`                | Raw byte pointer. |
| `char *` (cstring)| `%i8`                |  |
| `int *`           | `%i32`               |  |
| `FILE *`, `DIR *` | `%i8`                | Opaque. |
| `struct foo`      | `type Foo` (record)  | PascalCase. |

### 3.3 Naming

- Function names match C exactly: `printf`, `pthread_mutex_lock`.
- Constants use C's exact spelling: `EXIT_SUCCESS`, `O_CREAT`, `SIGTERM`.
- Record types are PascalCase Jinn names of the C struct: `Stat`, `Tm`,
  `SockAddrIn`, `Dirent`. Field names mirror the C struct (`st_dev`, etc.)
  for source-port compatibility.
- Type aliases use the C `_t` form: `pid_t`, `size_t`.

### 3.4 Comment style

Each header file opens with:

```
# libjn.<header> — drop-in for C <header.h>
#
# One-paragraph summary, including non-trivial deviations and references
# to libjn.md sections that explain them.
```

---

## 4. Namespacing — `std.libjn.*` and `compat`

### 4.1 Default qualified namespace

After `use std.libjn.stdio`, all symbols are reached as `stdio.printf(...)`,
`stdio.EOF`, `stdio.SEEK_SET`. This is the **recommended** form.

```jinn
use std.libjn.stdio
use std.libjn.string

*main()
    msg as %i8 = "hello world\n"
    stdio.fputs(msg, stdio.stdout_ptr())
    return 0
```

### 4.2 Why under `std.libjn` and not `std.c`?

Three reasons:

- **Avoid collision** with future `std.c` for native C-interop helpers.
- **Honor the implementation language**: these symbols are Jinn re-imps,
  not raw C bindings. `libjn` makes the boundary explicit.
- **Mirror pattern from other languages** that have done this well:
  Rust's `libc`, D's `core.stdc`, Zig's `std.c`. The triple-name `std.libjn`
  reads as "the Jinn rewrite that lives in std."

### 4.3 The `compat` global-namespace door

`std.libjn.compat` re-exports every symbol via `pub use ...*`. After
`use std.libjn.compat`, the C namespace is open in the current scope:

```jinn
use std.libjn.compat

*main()
    printf("hello world\n")
    fd as i32 = open("/tmp/foo", O_RDONLY)
    if fd < 0
        perror("open")
        exit(EXIT_FAILURE)
    close(fd)
    return EXIT_SUCCESS
```

**Best practice:**
- Use `compat` only when *mechanically porting* C source. Once stable,
  migrate to `stdio.printf` / `unistd.open` etc.
- Never use `compat` in libraries — it pollutes downstream namespaces.
- The `compat` shim is a no-cost abstraction at link time; `pub use` is
  pure name re-binding.

### 4.4 Selective imports

The Jinn `use` form supports symbol lists; users who want partial-compat
can write:

```jinn
use std.libjn.stdio.{printf, fprintf, fputs, EOF}
```

without importing the rest of `<stdio.h>`.

---

## 5. Exception-Handling Strategy

### 5.1 The problem

C's library is built around two mutually exclusive error styles:

- **Sentinel returns + errno:** `open` returns -1, sets `errno`.
- **Out-of-band non-local jump:** `setjmp` / `longjmp` (and SIGABRT on
  `assert`).

A Jinn program is built around a third style:

- **Result types and `?` propagation:** `Result<T, Error>`, `Option<T>`.

Bridging these is the single most important design choice in libjn,
because the bridge defines what *Jinn programs* feel like when they call
libc, and what *C programs* see when they call libjn.

### 5.2 Three options, evaluated

#### Option A — `setjmp` / `longjmp` substrate, plus `try` / `raise` keywords

Implement non-local control flow at the runtime level using `setjmp`/
`longjmp` (libjn/setjmp.jn). On top of that, add language-level keywords
`try`, `raise`, and `catch`:

```jinn
try
    do_thing_that_might_throw()
catch (e as Error)
    handle(e)
```

Sugar for:

```jinn
env as %i8 = stack_alloc(JMP_BUF_SIZE)
if setjmp(env) == 0
    push_handler(env)
    do_thing_that_might_throw()
    pop_handler()
else
    e as Error = current_exception()
    handle(e)
```

**Pros:**
- Maps cleanly to C's `signal` and `setjmp` worlds.
- Zero overhead on the happy path (one `setjmp` per `try` block).
- Familiar syntax for newcomers from Java/Python/C++.

**Cons:**
- `longjmp` skips destructors — Perceus reference-count drops are *missed*
  when the stack is unwound. This is a soundness hole that requires a
  cleanup-walker compiler pass keyed off frame pointer ranges.
- Forces a runtime ABI choice that's hard to undo.
- Hostile to inlining and tail-call optimization across `try` boundaries.

#### Option B — `Result<T, E>` enum + `!` propagation operator

Promote `Result` to a first-class language type. libjn functions return
typed errors; the `!` postfix operator propagates them up the call stack
(equivalent to Rust's `?`):

```jinn
*read_config(path as String) returns Result<Config, IoError>
    bytes is fs.read_all(path)!
    return parse(bytes)!
```

For C interop, every libjn function has *two* faces:
- `unistd.open_raw(path, flags) returns i32` — the C ABI face. Returns
  -1, sets errno. For drop-in replacement and FFI from C programs.
- `unistd.open(path, flags) returns Result<Fd, OsError>` — the idiomatic
  Jinn face. Returns a typed error. For Jinn programs.

`compat.jn` exports the `_raw` versions as `open`, `read`, `write`, etc.

**Pros:**
- Sound. No surprises with Perceus, drops, or inlining.
- Composable. `!` chains naturally; pattern-match on `Result` for
  recovery.
- Already partially in Jinn (the `!` operator exists).
- Doesn't force a runtime ABI decision.

**Cons:**
- Doubles the surface area: every fallible function has two faces.
- Doesn't model C's `signal` / `setjmp` / `assert` cleanly — those still
  need a non-local mechanism (likely OS signals or `panic!`).
- C callers calling libjn from FFI see only the `_raw` face; documentation
  burden.

#### Option C — Algebraic effects (effect handlers)

Add effect rows to function signatures: `*foo() / [Io, Throws<IoError>]`.
Handlers are first-class:

```jinn
handle Throws<IoError>
    do_thing()
with
    raise e -> log(e); resume default_value
```

**Pros:**
- The most expressive option. Subsumes both Result and exceptions.
- Allows user-defined effects (logging, transactions, dependency injection).
- Clean interaction with concurrency: effects compose with actors.

**Cons:**
- Massive language and compiler change. Effect inference is an open
  research area (Koka, Eff, OCaml 5).
- ABI is novel — can't be "drop-in" for C.
- Defers shipping libjn by a year+.

### 5.3 Decision

> **Adopt Option B (`Result<T, E>` + `!`) as the language-native surface,
> with Option A (`setjmp`/`longjmp`) implemented in libjn as the C-ABI
> substrate but NOT promoted to a `try`/`catch` keyword.**

This means:
1. **Every libjn function has two faces** — a `_raw` C-ABI face and a
   typed Jinn face. The `compat` shim exports the `_raw` face under the C
   name.
2. **`setjmp`/`longjmp` are exposed in `libjn/setjmp.jn`** for porting
   C programs that use them. The compiler must flag any Jinn function
   that *could* be `longjmp`'d over and refuse to allocate Perceus-managed
   values on its stack frame (or insert cleanup hooks).
3. **No `try` / `catch` keywords are added.** Pattern matching on
   `Result` is the canonical recovery mechanism. `panic!` (already in
   Jinn) is the canonical "this should never happen" exit, equivalent to
   `assert` failures.
4. **Option C remains the long-term research direction.** Effect handlers
   would compose beautifully with actors and the proposed `comptime`
   effect system. Tracked in the `effects-research` branch.

### 5.4 The two-face pattern, by example

```jinn
# C-ABI face — what `compat` exports as `open`.
*open_raw(path as %i8, flags as i32) returns i32
    nop
    -1   # implementation will: set errno, return -1 on failure

# Idiomatic Jinn face.
*open(path as String, flags as OpenFlags) returns Result<Fd, OsError>
    rc is open_raw(path.as_cstr(), flags.bits())
    if rc < 0
        return Err(OsError.from_errno(errno()))
    return Ok(Fd(rc))
```

The `_raw` form is the leaf. The typed form is one wrapper away. C
callers (via libjn's exported symbols) hit `_raw` directly.

### 5.5 What about `assert` and `abort`?

- `assert` lowers to a `panic!` in pure-Jinn mode and to `__assert_fail`
  in compat mode.
- `abort` calls `__libjn_abort` which raises `SIGABRT` and never returns.
- `exit` is `noreturn`, but Perceus must still drop everything in the
  current scope before the call. The compiler inserts a cleanup walker.

---

## 6. Language Limitations & Remediation Plan

This section enumerates every known feature gap that blocks, hinders, or
clouds the libjn implementation, with a concrete remediation per gap.

### 6.1 Variadic functions

**Gap:** Jinn has no native variadic function syntax. `printf` cannot be
expressed without one.

**Impact:** `printf`, `scanf`, `execl`, `open` (3-arg form), `fcntl`,
`ioctl`, `syscall`, ~30 functions total.

**Remediation (short term):**
- Add a compiler-internal `extern variadic *printf(...)` form that lowers
  to LLVM's varargs ABI. This is a one-week change; LLVM already supports
  it. The Jinn surface is a `va_list`-taking variant: callers must build
  the va_list explicitly via `va_start` / `va_end` or via a typed slice
  that libjn lowers.

**Remediation (long term):**
- Typed varargs via `Vec<Any>` / `&[Any]` slice — caller passes a slice,
  callee iterates with type tags. Compile-time format-string checking
  via comptime: `printf!("%d %s", n, s)` validates types at compile time
  and lowers to direct `fputs`/`fprintf` calls (zero runtime overhead).
- Tracked in the `comptime-varargs` branch.

### 6.2 Function pointers as opaque callbacks

**Gap:** `qsort`, `bsearch`, `signal`, `pthread_create`, `atexit` all take
function pointers. Jinn's first-class functions are closures; passing one
to C requires a trampoline.

**Impact:** ~25 functions across stdlib, signal, pthread, threads.

**Remediation:**
- Export-as-extern: every Jinn function whose type matches a `extern "C"`
  signature can be coerced to `%i8` (raw fn ptr) via the operator
  `@as_extern_fn(my_func)`. The compiler emits a thunk that translates
  Jinn's calling convention to SysV/cdecl.
- Closure → fn-ptr is harder (closures carry environments). Solution: the
  compiler refuses; users must extract a free function. A future
  enhancement: spawn a per-closure thunk allocated on a guard page that
  packs the environment into the trampoline (à la libffi `closure_alloc`).

### 6.3 Weak symbols / link-time aliases

**Gap:** glibc uses weak aliases extensively (`__libc_start_main`,
`__GI_strlen`). Jinn has no weak-symbol attribute.

**Impact:** Link-time interception, LD_PRELOAD compat, optional libc
overrides.

**Remediation:**
- Add `@weak` attribute on `extern` declarations. Lowers to LLVM's
  `available_externally` linkage with a fallback symbol.
- Add `@alias("strlen")` attribute on definitions. Lowers to a GAS
  `.weak`/`.set` directive in the emitted object file.

### 6.4 Struct-by-value calling convention

**Gap:** C passes small structs in registers per SysV-x86_64; large structs
by hidden pointer. Jinn currently always uses a hidden pointer.

**Impact:** `divmod` results, `timespec`, `stat` — anywhere a libjn
function returns a struct.

**Remediation:**
- Implement an ABI-lowering pass in the codegen. When a Jinn function
  has C-ABI linkage (`extern "C"` or libjn export), rewrite signatures
  to match SysV: ≤16-byte aggregates become `i64`/`i64` register pairs;
  larger ones get a hidden pointer in `rdi`.
- Mirror logic for AArch64 (AAPCS64) and Windows x64.
- Tracked: `abi-lowering` branch.

### 6.5 `long double`

**Gap:** Linux x86_64 `long double` is 80-bit x87. Jinn has no `f80` type.

**Impact:** `LDBL_*` constants in float.h are slightly misrepresented;
`sqrtl`, `cosl`, etc. lose precision.

**Remediation:**
- Add `f80` type, lowered to LLVM's `x86_fp80`. Restricted to x86 targets;
  on AArch64 it's a synonym for `f64`.
- Until then, `long double` libjn functions degrade to `double`. Document
  the deviation.

### 6.6 Bit-fields

**Gap:** Some POSIX structs (`struct fd_set` historically, `struct stat`
on some platforms) use bit-fields. Jinn has none.

**Impact:** Limited — modern POSIX moves to explicit shifts.

**Remediation:**
- Provide `bit.{get,set,clear,toggle}` helpers in std (already partial).
- Where C bit-fields appear, libjn uses an integer field plus accessor
  functions. Document in §15 per-function entries.

### 6.7 `errno` thread-locality

**Gap:** errno must be thread-local. Jinn's TLS support is partial.

**Impact:** Every libjn function that sets errno.

**Remediation:**
- Land full TLS variable support (`thread_local` qualifier on globals).
  Tracked: `tls` branch. Currently planned for v0.7.
- Until then, `__errno_location()` returns a per-thread pointer maintained
  by the runtime (libjn/runtime/errno.c stub).

### 6.8 Variable-length arrays / flexible array members

**Gap:** C99 VLAs (`int arr[n]` where n is runtime) and C99 flexible array
members (`struct foo { ...; int data[]; }`) have no Jinn equivalent.

**Impact:** Some POSIX structs (`struct sockaddr_storage` style padding,
`struct dirent`'s `d_name`).

**Remediation:**
- VLAs: callers use `Vec<T>` (heap) or a `[T; N]` with comptime-known N.
  Document that libjn never returns VLA-sized stack types.
- Flexible array members: use opaque pointer + accessor function pair.
  e.g., `dirent_d_name(entry as %i8) returns %i8` reads the trailing name.

### 6.9 Pointer arithmetic on `void *`

**Gap:** C lets you do `void *p; p + 1;` (GCC extension; UB in strict C
but ubiquitous). Jinn requires a typed cast first.

**Impact:** Internal libjn implementations of memcpy/memset/etc.

**Remediation:**
- Spell as `(p as %i8) + 1` — explicit cast through byte ptr. Ergonomic
  enough; document the idiom.

### 6.10 Setjmp/longjmp interaction with destructors

**Gap:** `longjmp` skips frames without running drops/destructors. With
Perceus reference counting this *leaks* memory at best, double-frees at
worst.

**Impact:** Catastrophic if not addressed. Blocks correctness of any
Jinn program that goes through `setjmp`.

**Remediation:**
- Compiler pass: scan all `try` / `setjmp` blocks; for each frame in the
  potential unwind range, emit a cleanup table (think DWARF EH unwind
  tables, but minimal).
- Runtime: `longjmp` walks the table and runs drops before resuming.
- Alternatively: forbid `setjmp` from skipping over Perceus-managed values
  at the type level. Compiler errors on the call site.
- Decision: implement *both*. Default: cleanup-walker. Annotation:
  `@no_unwind` on a function disables drops over its frame.

### 6.11 Function-like macros

**Gap:** C macros like `WIFEXITED(x)`, `htonl(x)`, `assert(cond)` are not
expressible directly.

**Impact:** Many. Both inline-perf (`htonl` should be one BSWAP) and
correctness (multiple-evaluation traps).

**Remediation:**
- Use Jinn `inline` functions with `comptime` evaluation. The `comptime`
  pass folds them at the call site, matching macro semantics without the
  evaluation-order traps.
- Where macros do textual substitution (e.g., `offsetof`), expose them
  as compiler-builtin functions.

### 6.12 C preprocessor includes

**Gap:** `#include <foo.h>` is C-source-level; Jinn modules are different.

**Impact:** None — Jinn's `use` covers it. Cross-compilation of C code
itself is out of scope.

### 6.13 Wide-char width mismatch (16-bit Windows vs 32-bit Linux)

**Gap:** `wchar_t` is platform-dependent in C.

**Impact:** Source ported from Windows C may break on Linux.

**Remediation:**
- libjn pins `wchar_t = i32` (UTF-32). Windows port provides `wchar16_t = i16`
  alias and converters. Document as an intentional simplification.

### 6.14 `%i8` raw byte ptr vs `String` / `&str`

**Gap:** C uses NUL-terminated `char *`; Jinn `String` is length-prefixed.

**Impact:** Every string-taking function.

**Remediation:**
- Helpers in `std.libjn.string`: `from_cstr(p as %i8) returns String`,
  `to_cstr(s as String) returns OwnedCstr` (RAII type that frees on
  drop). The typed face uses `String`; the `_raw` face uses `%i8`.
- The compiler enforces that `%i8` arguments to libjn functions are
  NUL-terminated when constructed from string literals (literals are
  always NUL-terminated by codegen).

### 6.15 Atomic memory orderings

**Gap:** Jinn currently exposes only `seq_cst` atomics. C11 has six orderings.

**Impact:** Performance — `relaxed` matters in hot paths.

**Remediation:**
- Extend `atomic` qualifier to take an order parameter:
  `x.atomic_load(.relaxed)`. Already designed; pending implementation.

### 6.16 `volatile` semantics

**Gap:** Jinn has a `volatile` token but no codegen.

**Impact:** Memory-mapped I/O, signal-handler shared state, `sig_atomic_t`.

**Remediation:**
- Plumb `volatile` through codegen. Each load/store annotated with LLVM's
  `volatile` flag. ~3-day change.

### 6.17 Signal-safe code (async-signal-safe subset)

**Gap:** POSIX defines a small set of async-signal-safe functions; Jinn
allocates freely (Perceus, Vec growth, GC-style bumps).

**Impact:** Any Jinn signal handler is unsafe.

**Remediation:**
- `@signal_safe` function attribute. Compiler refuses any allocation,
  any `mut` global access, any non-async-signal-safe call.
- `signal_safe` zone: a syntactic block where the rules are enforced.
- libjn's signal handlers are written inside such zones.

### 6.18 Locale and wchar are stateful globals

**Gap:** `setlocale` mutates a process-global. Jinn discourages such state.

**Impact:** `printf`'s locale-aware formatting, `strcoll`, etc.

**Remediation:**
- Hidden global is a `Lazy<Mutex<Locale>>`. Functions take the lock
  briefly to read.
- Per-thread override via `uselocale` (POSIX). Already in libjn/locale.jn
  as `uselocale`.

### 6.19 Re-entrancy of `errno` inside signal handlers

**Gap:** Setting errno inside a signal handler races with the interrupted
caller's errno read.

**Impact:** Subtle bugs.

**Remediation:**
- libjn convention: signal handlers save and restore errno explicitly.
  Document in §11. The `@signal_safe` macro emits the save/restore.

### 6.20 `time_t` overflow (Y2038)

**Gap:** `time_t = i32` overflows in 2038.

**Impact:** Embedded targets often use 32-bit `time_t`.

**Remediation:**
- libjn pins `time_t = i64` everywhere. The bridge layer translates
  to/from 32-bit on platforms that demand it.

### 6.21 Summary table

| # | Limitation | Severity | Plan | Tracked |
|---|------------|----------|------|---------|
| 6.1 | Variadics | Critical | LLVM varargs + comptime format check | §7 |
| 6.2 | Fn-ptr callbacks | High | `@as_extern_fn` thunk | abi-lowering |
| 6.3 | Weak symbols | Medium | `@weak`, `@alias` attrs | linkage-attrs |
| 6.4 | Struct-ABI | High | Codegen ABI lowering pass | abi-lowering |
| 6.5 | `long double` | Low | `f80` type, x86 only | f80 |
| 6.6 | Bit-fields | Low | Accessor pairs | none |
| 6.7 | `errno` TLS | Critical | Native TLS support | tls |
| 6.8 | VLA / FAM | Medium | `Vec<T>` + accessors | none |
| 6.9 | `void *` arith | Low | Cast through `%i8` | doc-only |
| 6.10 | Longjmp + drops | Critical | Cleanup walker pass | unwind |
| 6.11 | Macros | High | `inline` + `comptime` | comptime |
| 6.12 | `#include` | — | Use Jinn `use` | n/a |
| 6.13 | wchar width | Low | Pin UTF-32 | doc-only |
| 6.14 | cstring boundary | High | `from_cstr`/`to_cstr` | string-helpers |
| 6.15 | Atomic orderings | Medium | Order parameter | atomics |
| 6.16 | `volatile` | Medium | Plumb to codegen | volatile |
| 6.17 | Signal-safe subset | High | `@signal_safe` attr | signal-safe |
| 6.18 | Locale state | Medium | `Lazy<Mutex<Locale>>` | locale |
| 6.19 | errno in handlers | Medium | save/restore convention | doc + macro |
| 6.20 | Y2038 | High | Pin `time_t = i64` | done |

---

## 7. Variadic Functions — A Special Case

Variadics deserve their own section because the entire `printf`/`scanf`
family — about a third of the visible libc surface — depends on them.

### 7.1 Three layers

We expose **three** layers, each more idiomatic and safer than the previous:

#### Layer 1: Raw va_list (C-ABI compatible)

```jinn
*vfprintf(stream as %i8, fmt as %i8, ap as %i8) returns i32
```

Caller allocates a va_list, calls `va_start`, passes through. This is the
leaf and the only thing C callers ever see.

#### Layer 2: Slice-of-Any (idiomatic Jinn, runtime-typed)

```jinn
*printf(fmt as String, args as &[Any]) returns Result<i32, IoError>
```

Caller builds a slice. Runtime walks fmt and args. Type tag mismatches
are returned as `Err(...)`. This is what the typed face looks like.

#### Layer 3: Comptime format-checked macro (idiomatic Jinn, zero-cost)

```jinn
printf!("%d items, %s mode\n", count, mode_name)
```

Compile-time pass:
1. Parses the literal format string.
2. Extracts placeholder list.
3. Type-checks each placeholder against the call's positional args.
4. Lowers to a series of direct `fputs` / `i64_to_decimal` / etc. calls.
5. Reports type mismatches as compile-time errors with carets at the
   bad placeholder.

This is the recommended form. Zero runtime overhead, total type safety.

### 7.2 Mapping table

| C call              | libjn Layer 1            | libjn Layer 2          | libjn Layer 3        |
|---------------------|--------------------------|------------------------|----------------------|
| `printf(fmt, ...)`  | `vprintf(fmt, ap)`       | `print(fmt, args)`     | `printf!(fmt, ...)`  |
| `fprintf(s, fmt,...)`| `vfprintf(s, fmt, ap)`  | `fprint(s, fmt, args)` | `fprintf!(s, fmt,...)`|
| `snprintf(...)`     | `vsnprintf(...)`         | `format_into(...)`     | `snprintf!(...)`     |
| `sprintf(...)`      | `vsprintf(...)`          | (banned: no bound)     | `sprintf!(...)`      |
| `scanf(fmt, ...)`   | `vscanf(fmt, ap)`        | `scan(fmt, args)`      | `scanf!(...)`        |

### 7.3 `sprintf` is banned in Layer 2/3

There is no way to write a safe `sprintf` — the buffer size is unknown.
libjn keeps `sprintf` only at Layer 1 for compat shim use. Layer 2 and 3
expose `snprintf` only.

---

## 8. Memory Model & Ownership Contracts

Each libjn function declares an **ownership contract** in its docstring,
classifying every pointer parameter and return:

| Tag         | Meaning                                                |
|-------------|--------------------------------------------------------|
| `[in]`      | Caller-owned, read-only, no escape.                    |
| `[in mut]`  | Caller-owned, written.                                 |
| `[out]`     | Caller-owned buffer, callee writes.                    |
| `[in own]`  | Ownership transferred to callee (callee frees).        |
| `[out own]` | Newly allocated by callee, caller must `free`.         |
| `[static]`  | Points to internal static storage; do not free.        |
| `[opaque]`  | Pass back unchanged to a libjn function.               |

Examples:

- `malloc(n) -> [out own] %i8` — caller frees.
- `strdup([in] %i8) -> [out own] %i8` — caller frees.
- `getenv([in] %i8) -> [static] %i8` — do not free.
- `strerror(int) -> [static] %i8` — do not free; thread-unsafe; use `strerror_r`.
- `fopen([in] %i8, [in] %i8) -> [opaque] %i8` — pass back to `fclose`.
- `fread([out] %i8, ...) -> u64` — buffer is caller-owned.

These contracts are enforced by the compiler when calling libjn from typed
Jinn code (the typed face wraps `[out own]` returns in `Box<T>` with `Drop`).

---

## 9. Thread-Safety Classification

Each function is classified per POSIX terminology:

| Class                  | Meaning                                            |
|------------------------|----------------------------------------------------|
| **MT-safe**            | Safe to call concurrently from multiple threads.  |
| **MT-unsafe**          | Not safe; caller must serialize.                   |
| **AS-safe**            | Async-signal-safe (callable from signal handler). |
| **AC-safe**            | Async-cancel-safe.                                 |
| **race**               | MT-safe but causes data races on shared resource.  |
| **locale**             | Behavior depends on current locale.                |

Classification is per-function in §15 and reflected at the type level via
function attributes:

```
@mt_safe @as_safe
*memcpy(...) returns ...
```

The compiler checks these at call sites inside `@signal_safe` zones.

---

## 10. Performance Considerations

### 10.1 Stub overhead

Stubs have `nop` bodies, which lower to a `void` MIR instruction and are
elided by LLVM's simplifycfg. The stub functions still emit a function
prologue/epilogue (one `ret` + register save). In bridge mode, this
becomes a tail-call to the underlying libc symbol — equivalent to a PLT
hop, which a future link-time pass will inline away.

### 10.2 Inlining policy

- **Always-inline:** trivial wrappers (`isupper`, `islower`, `htonl`),
  attribute `@inline(always)`.
- **Inline:** small ops (`memcpy` ≤ 16 bytes, `strlen` ≤ unrolled), the
  compiler decides.
- **No-inline:** large or rarely-hot (`fopen`, `qsort`).

### 10.3 Comparison to glibc fast paths

glibc has hand-tuned SIMD versions of `memcpy`, `memset`, `strlen`,
`strchr`, and more. libjn's plan:

- v1: scalar-only, correctness-first.
- v1.1: portable SIMD via `std.simd` (already in std).
- v1.2: target-specific intrinsics (AVX2, SVE2) gated behind feature
  flags. Use `comptime` to select at compile time.

### 10.4 Vectorization decisions

- `memcpy`/`memmove`/`memset`: yes, mandatory, defer to LLVM's
  `@llvm.memcpy.*` intrinsic for v1; hand-tuned for v1.2.
- `memcmp`: yes; falls back to LLVM intrinsic.
- `strlen`/`strchr`: yes; classic SWAR (8-byte word zero-byte detection).
- `strcpy`/`strcat`: discouraged in new code. Provide `memccpy` and
  bounded variants as the recommended path.

### 10.5 Allocator strategy

- `malloc`/`free` in libjn delegate to the same allocator as the rest
  of the Jinn runtime (currently mimalloc-style; pluggable). This gives
  C code allocated through libjn's malloc the same telemetry, fragmentation
  behavior, and tuning as Jinn-native code.
- `aligned_alloc` and `posix_memalign` use the allocator's aligned-alloc
  path; falls back to overallocate-and-shift if not supported.

---

## 11. Safety Considerations

### 11.1 Bounds checking

- Layer 2/3 of every string and buffer function performs bounds checks.
- Layer 1 (C-ABI face) does not — caller is responsible, matching C
  semantics.
- The `compat` shim exports Layer 1, so C-ported code retains C semantics
  (and C bugs).

### 11.2 Null-check policy

- Every libjn function documents whether each pointer parameter accepts
  NULL.
- Functions that do not accept NULL emit a `panic!` on NULL in Layer 2/3,
  match C UB in Layer 1.
- This is documented in the per-function entry in §15.

### 11.3 Integer overflow

- Jinn's default integer arithmetic traps on overflow. libjn functions
  that match C semantics (which wraps) annotate themselves
  `@overflow(wrap)` to opt into wrapping arithmetic for that function.
- Functions that compute byte counts (e.g., `calloc(n, sz)` computing
  `n * sz`) do explicit overflow checks and return NULL on overflow,
  matching C99 specs.

### 11.4 Format-string injection

- Layer 1 (`printf(fmt, ...)`) trusts the caller. Format-string injection
  is possible.
- Layer 3 (`printf!(...)`) requires a literal format string at the call
  site. Format-string injection is *impossible* at the type level.
- A linter warns when Layer 1 is called with a non-literal format string.

### 11.5 TOCTOU

- File-system functions document TOCTOU windows. libjn's typed wrappers
  prefer file-descriptor-based variants (`openat`, `fstatat`) when
  available, which close most TOCTOU gaps.

### 11.6 Signal handling

- Per §6.17 and §6.19, signal handlers are restricted to `@signal_safe`
  zones with errno save/restore.

### 11.7 Random-number quality

- `rand()` is documented as low-quality, suitable only for non-crypto.
- libjn provides `arc4random_buf` (POSIX-ish) backed by `getrandom(2)`
  for crypto-quality entropy. Jinn-native code uses `std.rand.crypto`.

---

## 12. Testing Strategy

### 12.1 Test layers

1. **Unit tests** per function. Stored in `tests/libjn/<header>.rs` (Rust
   side) for compiler-level harness, or `tests/libjn/<header>.jn` for
   Jinn-level integration.
2. **Differential tests against glibc.** Generate random inputs, run
   through both libjn and the host libc, compare results. Shared corpus
   in `tests/libjn/corpus/`.
3. **Golden output tests.** Capture expected output strings (e.g.,
   `printf("%5.2f", 3.14)` → `" 3.14"`). Stored as `.golden` files.
4. **Property tests.** `proptest` / `quickcheck` style. Properties:
   - `strlen(strdup(s)) == strlen(s)`
   - `memcmp(memcpy(b, a, n), a, n) == 0`
   - `atoi(itoa(n)) == n` for valid n.
5. **ABI conformance tests.** Compile a small C program against the
   libjn shared library; verify it links and runs.
6. **Fuzzing.** AFL++ on every parser-style function (`strtol`, `strtod`,
   `scanf`).

### 12.2 CI matrix

- Linux x86_64 (glibc, musl), Linux aarch64 (glibc, musl), macOS arm64,
  Windows x64.
- Each combination: pure mode and bridge mode.
- Smoke test: build + link a non-trivial C program (sqlite, lua) against
  libjn; verify identical output to the libc-linked build.

### 12.3 Coverage targets

- 100% line coverage on libjn implementations (excluding error paths
  that need rare conditions to trigger).
- 90% branch coverage.
- Tracked per-header in `tests/libjn/coverage.md`.

---

## 13. Roadmap & Milestones

### Alpha — "compiles cleanly"

- ✅ Every header has a stub file with correct signatures.
- ✅ `mod.jn` aggregator and `compat.jn` shim exist.
- ✅ `nop` keyword integrated into the language.
- ⏳ Bridge mode wires every stub to the host libc symbol.
- ⏳ A trivial C program (`hello world`) links with `-llibjn -nostdlib`
  and runs.

### Beta — "runs real programs"

- ⏳ Top-50 functions implemented natively (memcpy/memset/strlen/strcmp/
  strchr/strstr/printf/snprintf/malloc/free/open/read/write/close/etc.).
- ⏳ Differential test suite passing on Linux x86_64.
- ⏳ Two reference programs link and run correctly: `sqlite`, `lua`.

### v1 — "drop-in replacement"

- ⏳ All ISO C11 functions implemented natively. Bridge mode optional.
- ⏳ POSIX 2008 base subset implemented.
- ⏳ Differential suite green on all CI matrix targets.
- ⏳ Self-host: Jinn compiler links against libjn instead of libc.
- ⏳ Public release; docs published.

### v1.x — "competitive"

- ⏳ Hand-tuned SIMD `memcpy`/`memset`/`strlen` etc.
- ⏳ Hardened mode (`-D_FORTIFY_SOURCE=2` equivalent) enabled by default.
- ⏳ Comptime format-checked `printf!` everywhere.

### v2 — "post-libc"

- ⏳ Pure mode is the default. Bridge mode is an opt-in compatibility
  fallback.
- ⏳ Effects-handler exception model (Option C from §5).
- ⏳ libjn's signal-safe subset is the canonical model for handler code
  in Jinn.

---

## 14. Exhaustive Task & Subtask Breakdown

Each task below has subtasks: **(S)** signature, **(B)** body, **(T)** tests,
**(D)** docs, **(A)** ABI conformance, **(P)** perf bench.

### Common subtasks per function

For *every* libjn function the subtasks are:

1. **(S) Signature.** ✅ done in stubs.
2. **(B) Body.** Replace `nop` with native implementation OR bridge call.
3. **(T) Tests.** Unit + differential vs glibc + property tests.
4. **(D) Docs.** Docstring with ownership tags, MT/AS class, deviations.
5. **(A) ABI test.** Compile a C program calling the symbol, verify.
6. **(P) Perf bench.** `criterion`-style benchmark vs glibc.

### 14.1 assert.jn (3 functions)

- `assert` — (B) lower to panic in pure, `__assert_fail` in compat.
- `static_assert` — (B) compiler-builtin; comptime check.
- `__assert_fail` — (B) prints diagnostic, calls `abort`.

### 14.2 ctype.jn (14 functions)

Each of `isalnum/isalpha/isblank/iscntrl/isdigit/isgraph/islower/isprint/
ispunct/isspace/isupper/isxdigit/tolower/toupper`:

- (B) C locale: branchless table lookup or arithmetic predicate.
- (B) Other locales: defer to `std.libjn.locale`.
- (T) Test every byte 0..256.
- (P) Compare against glibc's `__ctype_b_loc` table fast path.

### 14.3 errno.jn (~50 constants + 3 functions)

- Constants: define POSIX numbers per platform.
- `errno()` — (B) reads thread-local.
- `set_errno(v)` — (B) writes thread-local.
- `__errno_location()` — (B) returns &mut to the TLS slot.
- (T) Multi-thread test verifies isolation.

### 14.4 stdio.jn (~50 functions)

Critical path:

- **FILE representation.** Decision: opaque struct holding `{ fd, buffer,
  mode, pos, error_flag, eof_flag }`. (B) defines this.
- `fopen` — (B) parse mode string, call `open` with flags, allocate FILE.
- `fclose` — (B) flush, close fd, free FILE.
- `fread`/`fwrite` — (B) loop over buffer + read/write syscalls.
- `fgetc`/`fputc` — (B) buffered single-char.
- `fgets` — (B) bounded line read.
- `fseek`/`ftell` — (B) wrap `lseek`.
- `setvbuf`/`setbuf` — (B) replace buffer.
- `printf` family — (B) Layer 1 via va_list; Layer 3 via comptime
  (separate task in compiler).
- `scanf` family — (B) state-machine parse over format string.
- `tmpfile`/`tmpnam`/`mkstemp` — (B) random-name + open.
- `perror` — (B) `strerror` + `fputs(stderr)`.
- (T) per function; differential test for printf format coverage
  (the matrix of `%[flags][width][.prec][length]conv` is large; generate
  test cases programmatically).

### 14.5 stdlib.jn (~50 functions)

- `malloc`/`calloc`/`realloc`/`free`/`aligned_alloc` — (B) delegate to
  Jinn allocator (already exists in runtime).
- `atoi`/`atol`/`atof` — (B) via `strtol`/`strtod`.
- `strtol`/`strtod`/`strtoul`/`strtoull` — (B) careful overflow handling.
- `qsort`/`bsearch` — (B) generic sort/search; callback via
  `@as_extern_fn` thunk.
- `rand`/`srand` — (B) classic LCG; document low quality.
- `exit` — (B) run atexit handlers, flush streams, `_exit`.
- `_Exit` — (B) syscall directly.
- `atexit` — (B) push to global stack of handlers.
- `getenv`/`setenv`/`unsetenv` — (B) walk `environ`.
- `system` — (B) `fork` + `execve` + `waitpid`.
- `abs`/`labs`/`div`/`ldiv` — (B) trivial.
- (T) extensive: malloc stress tests, qsort property test, strtol corner
  cases (LONG_MIN/MAX, base 2-36).

### 14.6 string.jn (~25 functions)

- `memcpy`/`memmove`/`memset`/`memcmp`/`memchr` — (B) defer to LLVM
  intrinsic in v1; hand-tuned in v1.2.
- `strlen` — (B) SWAR word-loop.
- `strcpy`/`strncpy`/`strcat`/`strncat` — (B) loop; document that
  `strncpy` does *not* NUL-terminate on truncation.
- `strcmp`/`strncmp`/`strchr`/`strrchr`/`strstr` — (B) scalar; tunable.
- `strerror`/`strerror_r` — (B) static table; thread-safe variant.
- `strdup`/`strndup` — (B) malloc + memcpy.
- `strtok`/`strtok_r` — (B) classic; document re-entrancy.
- `strspn`/`strcspn`/`strpbrk` — (B) bitset-based.
- (T) heavy property tests; fuzz `memcpy`/`memset`.

### 14.7 math.jn (~50 functions)

- All trig/exp/log/pow — (B) defer to LLVM intrinsics where possible
  (`@llvm.sin.f64`, etc.); else implement via known polynomials (e.g.,
  Sun's fdlibm).
- `fpclassify`/`isnan`/`isinf`/`isfinite`/`isnormal`/`signbit` — (B)
  bit-level inspection; trivial.
- `frexp`/`ldexp`/`modf`/`scalbn` — (B) bit manipulation.
- (T) compare against glibc's libm to ULP within tolerance per function.
- (P) bench against libm.

### 14.8 time.jn (~20 functions)

- `time` — (B) syscall `clock_gettime(CLOCK_REALTIME)`.
- `clock_gettime`/`clock_settime`/`clock_getres` — (B) syscall.
- `nanosleep` — (B) syscall.
- `mktime`/`gmtime`/`localtime` — (B) Howard Hinnant date algorithms.
- `strftime`/`strptime` — (B) parse format string, output fields.
- (T) timezone tests; leap-second handling documented.

### 14.9 signal.jn (~15 functions)

- `signal` — (B) wrap `sigaction`.
- `sigaction` — (B) syscall.
- `raise`/`kill` — (B) syscall.
- `sig*set` family — (B) bitmap manipulation.
- `sigprocmask` — (B) syscall.
- (T) handler-installation tests; SIGSEGV catching test.

### 14.10 setjmp.jn (4 + 3 helpers)

- `setjmp`/`longjmp`/`sigsetjmp`/`siglongjmp` — (B) hand-written
  assembly per arch (libjn/runtime/setjmp_x86_64.S etc.).
- `__libjn_throw`/`__libjn_try_enter`/`__libjn_try_leave` — (B) the
  cleanup-walker integration (depends on §6.10 unwind tables).
- (T) jump across N frames; verify Perceus drops happen.

### 14.11 stdarg.jn (4 functions)

- `va_start`/`va_arg`/`va_copy`/`va_end` — (B) mostly compiler-builtin;
  va_list ABI per platform.
- (T) varargs forwarding tests.

### 14.12 stdatomic.jn (~25 functions)

- All `atomic_*` operations — (B) lower to LLVM atomic intrinsics with
  the appropriate memory order.
- (T) lock-free queue test; concurrency stress.

### 14.13 threads.jn (~25 functions)

- `thrd_*`/`mtx_*`/`cnd_*`/`tss_*`/`call_once` — (B) wrap pthread
  equivalents.
- (T) producer/consumer test; ensure C11 semantics.

### 14.14 locale.jn (5 functions + Lconv)

- `setlocale`/`localeconv` — (B) global Locale state, returns C/POSIX
  locale by default.
- `newlocale`/`duplocale`/`freelocale`/`uselocale` — (B) heap-allocated
  locales.
- (T) limited; full locale data deferred to v1.x.

### 14.15 fenv.jn (~10 functions)

- All `fe*` — (B) read/write x87/SSE control word (x86); FPCR (aarch64).
- (T) round mode test; exception flag test.

### 14.16 complex.jn (~15 functions)

- `creal`/`cimag`/`cabs`/`carg`/`conj` — (B) trivial.
- `cexp`/`clog`/`csqrt`/`cpow`/csin/etc — (B) Kahan's textbook formulas.
- (T) compare against libm complex.

### 14.17 inttypes.jn

- `imaxabs`/`imaxdiv` — (B) trivial.
- `strtoimax`/`strtoumax` — (B) wrap strtoll.
- PRId/PRIu/PRIx — string constants.
- (T) format-string concatenation test.

### 14.18 wchar.jn / wctype.jn / uchar.jn

- All `wcs*`/`isw*`/`mbrtoc*`/`c*rtomb` — (B) UTF-8 ↔ UTF-32 conversion
  + scalar word loops.
- (T) UTF-8 corner cases (overlong, surrogates, > U+10FFFF).

### 14.19 unistd.jn (~70 functions)

- `read`/`write`/`close`/`lseek`/`pread`/`pwrite` — (B) syscall.
- `fork`/`exec*`/`_exit`/`wait*` — (B) syscall.
- `getpid`/`getppid`/`getuid`/`getgid`/etc — (B) syscall.
- `pipe`/`dup`/`dup2` — (B) syscall.
- `chdir`/`getcwd`/`access` — (B) syscall.
- `link`/`unlink`/`symlink`/`readlink`/`rmdir` — (B) syscall.
- `sleep`/`usleep`/`alarm`/`pause` — (B) syscall.
- `isatty`/`ttyname` — (B) `tcgetattr` + `/proc/self/fd/*`.
- `gethostname`/`sethostname` — (B) syscall.
- `sysconf`/`pathconf`/`fpathconf` — (B) static table per platform.
- (T) per syscall; especially fork/exec round-trips.

### 14.20 fcntl.jn (~10 functions)

- `open`/`openat`/`creat` — (B) syscall.
- `fcntl` — (B) syscall (variadic-ish; handle each cmd).
- `posix_fadvise`/`posix_fallocate` — (B) syscall.

### 14.21 dirent.jn (~12 functions)

- `opendir`/`readdir`/`closedir`/`rewinddir` — (B) wrap `getdents64`.
- `scandir`/`alphasort` — (B) build vec, qsort.

### 14.22 sys_stat.jn (~15 functions)

- `stat`/`fstat`/`lstat`/`fstatat` — (B) syscall.
- `chmod`/`fchmod`/`fchmodat` — (B) syscall.
- `mkdir`/`mkdirat`/`mkfifo`/`mknod` — (B) syscall.
- `umask` — (B) syscall.
- S_IS* macros — (B) bitmask checks.

### 14.23 sys_socket.jn (~20 functions)

- `socket`/`bind`/`listen`/`accept`/`connect` — (B) syscall.
- `send`/`recv`/`sendto`/`recvfrom`/`sendmsg`/`recvmsg` — (B) syscall.
- `shutdown`/`getsockopt`/`setsockopt` — (B) syscall.

### 14.24 sys_mman.jn (~12 functions)

- `mmap`/`munmap`/`mprotect`/`msync`/`madvise` — (B) syscall.
- `mlock`/`munlock`/`mlockall`/`munlockall` — (B) syscall.
- `shm_open`/`shm_unlink` — (B) wrap open/unlink under `/dev/shm`.

### 14.25 sys_wait.jn (~5 functions)

- `wait`/`waitpid`/`waitid` — (B) syscall.
- `WIFEXITED`/etc — (B) trivial bit checks.

### 14.26 pthread.jn (~50 functions)

- All pthread ops — (B) syscall to futex-based primitives. Largest
  single sub-project; can be sub-divided per primitive.

### 14.27 semaphore.jn (~10 functions)

- `sem_*` — (B) atomic int + futex.

### 14.28 Compiler tasks (cross-cutting)

- (C1) `nop` keyword — ✅ DONE.
- (C2) `extern variadic` LLVM lowering — see §6.1.
- (C3) `comptime` format-string check (`printf!` macro) — see §7.
- (C4) `@as_extern_fn(f)` thunk emission — see §6.2.
- (C5) `@weak` and `@alias` attributes — see §6.3.
- (C6) ABI lowering pass for struct-by-value — see §6.4.
- (C7) `f80` type — see §6.5.
- (C8) Native TLS — see §6.7.
- (C9) Cleanup-walker for setjmp/longjmp — see §6.10.
- (C10) `volatile` codegen — see §6.16.
- (C11) `@signal_safe` attribute and zone — see §6.17.

### 14.29 Runtime tasks

- (R1) `runtime/setjmp_x86_64.S` and `setjmp_aarch64.S` — handwritten.
- (R2) Per-platform syscall dispatcher (`syscall0` .. `syscall6`).
- (R3) Per-thread errno slot (until C8 lands).
- (R4) `environ` global initialization at startup.
- (R5) `atexit` handler stack.
- (R6) Locale-state Lazy<Mutex<Locale>>.

### 14.30 Documentation tasks

- (D1) Per-header API reference page on the website.
- (D2) Migration guide: "Porting a C program to Jinn via libjn."
- (D3) Comparison page: libjn vs glibc vs musl vs Cosmopolitan.
- (D4) Security model document.

---

## 15. Formal Technical Specification

> **Format.** For every function, this section gives:
> `name(args) -> ret  [MT/AS class]  [ownership tags]  description`
>
> Only a representative subset is reproduced inline below; the full
> per-function spec lives in `docs/libjn-spec/<header>.md` (one file per
> header, generated from the `.jn` source plus annotations).

### 15.1 Example entry — `fopen`

> ```
> fopen(path: [in] %i8, mode: [in] %i8) -> [opaque] %i8
> ```
>
> **Class:** MT-safe. AS-unsafe. AC-unsafe.
>
> **Description.** Opens the file at `path` for reading/writing per `mode`.
> `mode` is a NUL-terminated string consisting of one of `r`, `w`, `a`,
> `r+`, `w+`, `a+`, optionally followed by `b`/`t`/`x` modifiers.
>
> **Returns.** Opaque FILE pointer on success. NULL on failure with
> `errno` set (most commonly `ENOENT`, `EACCES`, `EMFILE`, `ENOSPC`).
>
> **Errors.**
> - `EINVAL` — invalid `mode` string.
> - `ENOENT` — file does not exist (when not creating).
> - `EACCES` — permission denied.
> - `EEXIST` — `x` modifier and file exists.
> - All errors propagated from `open(2)`.
>
> **Deviations from POSIX.** None.
>
> **Concurrency.** The returned FILE has its own lock (default
> `flockfile` semantics).
>
> **Memory.** Allocates a FILE on the heap; freed by `fclose` (or
> implicit cleanup at `exit` for unfreed streams).
>
> **Cross-references.** §11.5 (TOCTOU), §6.14 (cstring boundary).

### 15.2 Example entry — `memcpy`

> ```
> memcpy(dst: [out] %i8, src: [in] %i8, n: u64) -> [out] %i8
> ```
>
> **Class:** MT-safe. AS-safe. AC-safe.
>
> **Description.** Copies `n` bytes from `src` to `dst`. Behavior is
> undefined if the regions overlap; use `memmove` if they may.
>
> **Returns.** `dst`.
>
> **Preconditions.** `dst` and `src` point to at least `n` valid bytes;
> regions do not overlap.
>
> **Implementation.** v1: `@llvm.memcpy.p0.p0.i64(dst, src, n, false)`.
> v1.2: target-specific SIMD (rep movsb on Intel ERMS; SVE2 on Arm).
>
> **Performance.** Within 5% of glibc's `memcpy` for n ≥ 64; within 2%
> for n ≥ 1024.

### 15.3 Generation strategy

Rather than reproduce 300+ entries inline (this document is already
substantial), the per-header spec files are generated by a small `genspec`
tool that reads the `.jn` stub, looks up the annotations file
(`docs/libjn-spec/_annotations/<header>.toml`), and emits a Markdown
reference. This keeps spec and code in lock-step.

The annotation TOML format:

```toml
[fopen]
mt = "safe"
as = "unsafe"
ac = "unsafe"
ownership.path = "in"
ownership.mode = "in"
ownership.return = "opaque"
errors = ["EINVAL", "ENOENT", "EACCES", "EEXIST"]
deviations = "none"
notes = """
The returned FILE has its own lock (default flockfile semantics).
"""
```

Tracked: `tools/genspec/`.

---

## 16. Open Questions & Future Work

1. **Module-path support for nested folders.** Once the Jinn `use` system
   supports `std.libjn.sys.stat`, migrate `sys_*.jn` filenames to nested
   folders without breaking the public API (the `compat` shim hides
   the change).
2. **Should libjn ship its own `<sys/epoll.h>`, `<sys/inotify.h>`,
   `<sys/eventfd.h>`?** Probably yes — they are non-portable but
   foundational on Linux. Tracked for v1.x.
3. **Cosmopolitan-style polyglot binaries.** Could libjn be the basis
   for a Jinn binary that runs on Linux/macOS/Windows/BSD without
   recompilation? Investigate.
4. **Static vs dynamic libjn.** Today the design assumes dynamic linking.
   A statically linked, dead-code-eliminated libjn (linker GC of unused
   symbols) is desirable for embedded.
5. **The kernel boundary.** Pure-mode libjn currently issues raw syscalls
   on Linux. On macOS the syscall ABI is unstable; we may need a thin
   `libSystem` bridge there permanently. On Windows, the NT API is
   unstable too — `ntdll.dll` is the de-facto stable surface.
6. **Effect handlers (Option C from §5).** A research branch. Goal: by
   v2, libjn errors are expressible as effects; legacy `Result` returns
   are sugar over the effect form.
7. **The ABI for `printf!`.** The macro lowers to direct calls; but
   what about a `printf!`-like *function* (not macro) that takes a
   compile-time-checked format string? This requires `comptime` arguments,
   which exist in Jinn but interact subtly with currying. Resolve by
   v1.x.

---

*End of libjn.md — current revision tracks libjn alpha stubs as of the
initial scaffolding commit.*
