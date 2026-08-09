# libjn — a C standard library in Jinn

> **Status: design / alpha stub.** Every header in `libjn/` has compileable Jinn
> stubs with `nop` bodies matching the C ABI signature. No body is implemented.
> `libjn` is not part of `std`, is not shipped by any gate, and is not covered by
> any test (`V-5` in [`../roadmap.md`](../roadmap.md#coverage-gaps)).

## Why

Replacing libc with a pure-Jinn implementation is a **forcing function for the
language**: it surfaces every deficiency in the type system, the FFI surface,
codegen, the runtime, and the standard library design. Four concrete reasons:

1. **Proof.** A language that cannot reproduce libc cannot replace C. Every
   systems language runs this gauntlet — Rust's `libstd` over the `libc` crate,
   Zig's `std.c`, D's `core.stdc`.
2. **Compatibility shim.** C programs ported header by header can rely on
   `use libjn.compat` and have the familiar names in scope. This is the porting
   story for moving real C codebases to Jinn.
3. **Audit surface.** A Jinn-native libc lets us replace the C functions at link
   time with bounds-checked, leak-tracked, signal-safe variants *without
   recompiling the C code*. The `compat` shim is where foreign control flow gets
   audited.
4. **Self-hosting.** A Jinn program's trusted computing base shrinks from
   `compiler ∪ libc ∪ kernel` to `compiler ∪ libjn ∪ kernel`. Every byte of
   libjn written is a byte of libc no longer trusted.

`libjn` is part of the **toolchain**, not a package: it is never resolved
through a dependency graph and never `lamp add`ed. Because it is layered —
freestanding primitives, then the allocator, then syscall shims, then the
scheduler — it is also what makes the freestanding output kinds and the
substrate dials in [`lamp.md`](lamp.md) possible: only the layers an artifact's
used surface actually reaches get linked.

**Non-goals.** Bug-for-bug glibc compatibility — where glibc has undefined
behavior (`strtok` mutating its argument, `gets` having no bound), libjn
documents and may deviate, typically by returning an error. Locale-perfect ICU
coverage — the `"C"` locale faithfully and `"C.UTF-8"` to a useful subset; full
internationalization belongs elsewhere. Legacy codepage wide characters —
`wchar_t` is pinned to UTF-32. glibc symbol versioning.

**Ethos: idiomatic Jinn first, C shape second.** Every function exposes a C-ABI
signature at the FFI boundary, but its *implementation* is written as a Jinn
programmer would write it — bounds-checked slices, pattern matching, `Result`
returns wrapped at the boundary, no manual goto-cleanup.

## Layout

One Jinn file per C header, under `libjn/`, with `mod.jn` as the aggregator and
`compat.jn` as the global-namespace shim. ISO C headers (`assert`, `ctype`,
`errno`, `math`, `setjmp`, `signal`, `stdarg`, `stdatomic`, `stdio`, `stdlib`,
`string`, `threads`, `time`, `wchar`, …) sit alongside POSIX ones (`dirent`,
`fcntl`, `pthread`, `semaphore`, `unistd`, and the path-flattened `sys_mman`,
`sys_socket`, `sys_stat`, `sys_types`, `sys_wait`).

One file per header buys three things: **discoverability** — a C programmer
reaching for `<stdio.h>` finds `libjn/stdio.jn`, mechanically; **granular
import** — a program needing only `string` and `stdio` pulls in two compilation
units; and **header-isolated forward declarations** — each file declares its own
opaque types (`FILE`, `DIR`, `pthread_t`) without cross-file ordering concerns,
because the import graph is acyclic by design.

POSIX `<sys/...>` headers are path-flattened because module paths cannot contain
`/`. When nested module folders land, the migration is transparent and
`compat.jn` keeps working.

## Language gaps this work depends on

These are the features libjn cannot be written without. Each is a compiler
change, not a libjn change.

**Variadic functions.** Jinn has no variadic syntax, so `printf` cannot be
expressed. Roughly 30 functions are blocked (`printf`, `scanf`, `execl`, the
three-argument `open`, `fcntl`, `ioctl`, `syscall`). Short term: an
`extern variadic *printf(...)` form lowering to LLVM's varargs ABI, with a
`va_list`-taking Jinn surface. Long term: typed varargs over a slice of tagged
values, plus compile-time format-string checking through comptime — `printf`
with a literal format validates its argument types at compile time and lowers to
direct calls with zero runtime overhead.

**Function pointers as opaque callbacks.** `qsort`, `bsearch`, `signal`,
`pthread_create`, and `atexit` all take function pointers; Jinn's first-class
functions are closures, so passing one to C needs a trampoline. About 25
functions are affected. The fix: any Jinn function whose type matches a C
signature can be coerced to a raw function pointer, with the compiler emitting a
thunk that translates the calling convention. Closure-to-pointer is harder,
since closures carry environments — the compiler should **refuse** and require a
free function rather than guess. A per-closure trampoline that packs the
environment, in the style of libffi's closure allocation, is a possible later
enhancement.

**Weak symbols and link-time aliases.** glibc uses weak aliases extensively, and
Jinn has no weak-symbol attribute, which blocks link-time interception,
`LD_PRELOAD` compatibility, and optional libc overrides. The fix: a `@weak`
attribute on `extern` declarations and an `@alias("name")` attribute on
definitions.

**Struct-by-value calling convention.** C passes small structs in registers per
the platform ABI and large ones by hidden pointer; Jinn always uses a hidden
pointer. This affects anything returning a struct — `divmod` results,
`timespec`, `stat`. The fix is an ABI-lowering pass in codegen: when a function
has C linkage, rewrite the signature to match the platform ABI (aggregates up to
16 bytes become register pairs on SysV x86-64, larger ones get a hidden
pointer), with parallel logic for AArch64 and Windows x64.

**`long double`** has no Jinn type, and the exception-handling substrate
(`setjmp`/`longjmp`) needs a decision that composes with the error model in
[`../error-effects.md`](../error-effects.md) rather than introducing a second
control-flow mechanism.

## Milestones

**Alpha — "compiles cleanly."** Every header has a stub file with correct
signatures; the aggregator and compat shim exist; the `nop` keyword is
integrated into the language. *Remaining:* bridge mode wiring every stub to the
host libc symbol, and a hello-world C program that links against libjn with
`-nostdlib` and runs.

**Beta — "runs real programs."** The top ~50 functions implemented natively
(`memcpy`, `memset`, `strlen`, `strcmp`, `strchr`, `strstr`, `printf`,
`snprintf`, `malloc`, `free`, `open`, `read`, `write`, `close`); a differential
test suite against the host libc passing on Linux x86-64; two reference C
programs linking and running correctly.

**v1 — "drop-in replacement."** All ISO C11 functions implemented natively with
bridge mode optional; the POSIX 2008 base subset implemented; the differential
suite green across the CI matrix; and the milestone that matters — the Jinn
compiler itself links against libjn instead of libc.

**v1.x — "competitive."** Hand-tuned SIMD `memcpy`/`memset`/`strlen`, hardened
mode on by default, comptime format checking everywhere.

**v2 — "post-libc."** Pure mode is the default and bridge mode is the opt-in
fallback; libjn's signal-safe subset becomes the canonical model for handler
code in Jinn.

## Testing strategy

The only credible test for a libc is a **differential suite**: for every
implemented function, run the libjn version and the host libc version over the
same generated inputs — including boundary values, empty and maximal buffers,
overlapping ranges, and invalid arguments — and assert identical results,
identical `errno`, and identical observable memory. Anywhere libjn deliberately
deviates from glibc undefined behavior, the deviation is pinned by its own test
and documented at the deviation site.
