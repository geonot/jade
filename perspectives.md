# Jade Language — Multi-Perspective Analysis & Remediation Plan

> Evaluation of Jade from 24 professional developer perspectives.
> Each section: what works, what's missing, severity rating, and concrete remediation.
> Severity: **BLOCKER** (unusable), **MAJOR** (painful workaround), **MINOR** (inconvenient), **WISH** (nice-to-have)

---

## 1. Embedded Systems Developer

**Profile**: Ships firmware for microcontrollers (ARM Cortex-M, RISC-V), needs deterministic memory, no OS, sub-KB RAM budgets, MMIO registers, interrupt handlers.

### What Works
- Deterministic Perceus RC with compile-time drop insertion — no GC pauses
- `volatile.read`/`volatile.write` builtins for MMIO registers
- `@packed`, `@strict`, `@align(N)` layout control — essential for hardware register structs
- `%`/`@` raw pointer arithmetic — direct memory access
- Inline assembly (`asm` keyword parsed)
- `--emit-obj` separate compilation — can link with existing C firmware
- Integer width variety: `i8..i64`, `u8..u64`
- Arena allocator — bounded, deterministic allocation
- No exceptions, no hidden allocations in hot paths

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No cross-compilation | **BLOCKER** | Only compiles for host triple. Embedded targets (thumbv7em-none-eabi, riscv32imc) impossible. |
| No `#[no_std]` / freestanding mode | **BLOCKER** | Runtime assumes libc, pthreads, mmap. Bare-metal has none of these. |
| No interrupt handler support | **BLOCKER** | Cannot mark functions as ISRs with correct calling conventions. |
| No linker script integration | **BLOCKER** | Cannot place code/data at specific addresses (flash, SRAM regions). |
| No `const` / `static` global variables | **MAJOR** | Embedded relies on `static mut` or `const` for peripheral singletons and lookup tables. |
| Volatile only supports i64 | **MAJOR** | Hardware registers are typically 8/16/32-bit. Need `volatile_read_u8`, `volatile_read_u32`, etc. |
| No bitfield structs | **MAJOR** | Hardware registers are bit-packed. Manual shift/mask is error-prone. |
| No `repr(C)` guarantee by default | **MAJOR** | `@strict` exists but isn't the default — C interop structs risk silent reordering. |
| No compile-time assertions | **MINOR** | `static_assert(sizeof(Foo) == 16)` for layout verification. |
| No DMA / zero-copy buffer support | **MINOR** | Common pattern in embedded for peripheral data transfer. |
| No `inline(never)` / `inline(always)` | **MINOR** | Critical for flash size budgets and stack depth control. |

### Verdict: **NOT VIABLE** — 4 blockers prevent any embedded use.

---

## 2. DevOps / Infrastructure Developer

**Profile**: Writes CLI tools, deployment scripts, config management, container orchestration helpers, monitoring agents. Values fast startup, small binaries, easy cross-compilation.

### What Works
- Compiles to native binary — no runtime dependency (except libc)
- `os.jade`: env vars, PID, CLI args
- `fs.jade`: mkdir, rmdir, list_dir, walk, copy, rename, file_size, exists
- `io.jade`: read_file, write_file, read_lines, read_stdin
- `path.jade`: join, normalize, dir/base/ext/stem
- `json.jade`: full parse/stringify — handles config files
- `time.jade`: monotonic clock, sleep, elapsed
- `signal.jade`: handle, ignore, raise
- Pattern-directed functions — clean CLI dispatch
- 0.94× C performance — monitoring agents won't bottleneck

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No subprocess spawning | **BLOCKER** | Cannot run `docker`, `kubectl`, `ssh`, `git`. DevOps tools orchestrate other processes. |
| No HTTPS / TLS | **BLOCKER** | Cannot talk to cloud APIs (AWS, GCP, Azure). HTTP-only client is a non-starter. |
| No cross-compilation | **MAJOR** | DevOps needs Linux amd64 + arm64 binaries from one machine. |
| No TOML / YAML parser | **MAJOR** | Standard config formats in DevOps (Kubernetes, Docker Compose, Ansible). |
| No environment variable unsetting | **MAJOR** | `unsetenv` missing — needed for credential scrubbing. |
| No `--flag=value` arg parsing | **MAJOR** | `get_option` only handles `--flag value` — breaks standard CLI conventions. |
| No stdin pipe detection (isatty) | **MINOR** | Cannot distinguish interactive vs piped input. |
| No gzip/tar support | **MINOR** | Common for log processing, artifact handling. |
| No colored terminal output (ANSI) | **MINOR** | Expected in modern CLI tools. |
| No daemon mode / PID file | **WISH** | Background service pattern. |

### Verdict: **NOT VIABLE** — subprocess spawning and HTTPS are non-negotiable.

---

## 3. Data Scientist

**Profile**: Explores datasets, builds models, computes statistics, visualizes results. Uses notebooks or scripts. Prototype speed matters more than production performance.

### What Works
- Full libm math library — sqrt, trig, exp, log, constants
- `rand.jade`: xoshiro256** PRNG, seeding, range, shuffle, choice — good for Monte Carlo
- `sort.jade`: introsort — efficient in-place sorting
- `json.jade`: can parse data files
- NDArray type in grammar — matrix creation `3 by 3`
- Pipeline operator `~` — data transformation chains
- List comprehensions — `[x pow 2 for x in 0 to 100]`
- `sim for` parallel loops — embarrassingly parallel workloads
- 0.94× C performance — competitive for compute-heavy analysis

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No DataFrame / tabular data type | **BLOCKER** | Core data science abstraction. CSV → DataFrame → filter/group/join is the basic workflow. |
| No CSV parser | **BLOCKER** | Most datasets are CSV. No way to ingest data. |
| NDArray codegen not implemented | **BLOCKER** | `3 by 3` parses but the compiled feature set (einsum, grad, SIMD) was removed. |
| No plotting / visualization | **BLOCKER** | Cannot produce charts, histograms, scatter plots. Even text-based output would help. |
| No REPL / notebook integration | **MAJOR** | Exploratory workflow requires rapid iteration. Compile-run cycle is too slow. |
| Collections hardcoded to i64 | **MAJOR** | Stack, Queue, RingBuffer don't work with f64, strings, or structs. |
| No statistical functions | **MAJOR** | No mean, median, mode, stdev, variance, correlation, percentile. |
| No linear algebra | **MAJOR** | No matrix multiply, inverse, determinant, SVD, eigendecomposition built in. |
| No arbitrary-precision numbers | **MINOR** | Overflow at i64 limits for combinatorics, crypto. |
| No complex numbers | **MINOR** | Needed for signal processing, quantum computing. |

### Verdict: **NOT VIABLE** — no data ingestion, no DataFrames, no visualization.

---

## 4. Indie Game Developer

**Profile**: Solo or small team building 2D/3D games. Needs game loop, rendering, input handling, audio, ECS or scene graphs, fast iteration.

### What Works
- 0.94× C performance — CPU-bound game logic is fast
- `math.jade`: trig, lerp, clamp, map_range — useful for game math
- `rand.jade`: good PRNG for procedural generation, loot tables
- `time.jade`: monotonic clock for delta time, frame timing
- Structs with methods — can model entities, components
- Enum pattern matching — state machines for AI, UI
- Actor model — could model game entities as actors
- `sim for` — parallel particle systems, physics batches
- Arena allocator — per-frame allocation pattern

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No graphics API binding (OpenGL/Vulkan/SDL/Metal) | **BLOCKER** | Cannot draw anything to screen. |
| No window creation / event loop | **BLOCKER** | Cannot create a window or poll input events. |
| No audio API | **BLOCKER** | Games need sound effects and music. |
| No vec2/vec3/vec4/mat4 types | **MAJOR** | Game math is vector math. Manual struct-per-type is tedious. |
| No SIMD intrinsics | **MAJOR** | SSE/AVX for batch transforms, physics. SIMD type was removed. |
| No asset loading (PNG, WAV, OBJ) | **MAJOR** | Need to load textures, models, sounds from files. |
| No ECS framework | **MAJOR** | Standard architecture for modern game engines. |
| No C header / FFI generator | **MAJOR** | Manual extern declarations for every SDL/OpenGL function is impractical. |
| No hot-reload / live coding | **MINOR** | Fast iteration during development. |
| No fixed-point math | **MINOR** | Deterministic networking, retro platforms. |
| No serialization (save games) | **MINOR** | Only JSON available — no binary serialization. |

### Verdict: **NOT VIABLE** — no graphics, no windowing, no audio. Even a Pong clone requires extern bindings to SDL.

---

## 5. Web Developer

**Profile**: Builds web applications — frontend (SPA, SSR) or backend (REST APIs, microservices). Expects HTTP, templating, database drivers, auth.

### What Works
- `http.jade`: basic HTTP/1.1 client — fetch, post_json, get_header
- `net.jade`: TCP listener/stream — can accept connections
- `json.jade`: full JSON parse/stringify — API payloads
- `strings.jade`: StringBuilder — efficient response construction
- `fmt.jade`: pad, join, hex — formatting helpers
- Pattern-directed functions — could model route dispatch
- Persistent stores — basic CRUD without external DB
- Actor model — request-per-actor concurrency

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No HTTP server framework | **BLOCKER** | Must hand-parse HTTP from raw TCP. No routing, no middleware, no static files. |
| No HTTPS / TLS | **BLOCKER** | Cannot serve or consume production APIs. |
| No template engine | **BLOCKER** | Cannot render HTML with dynamic data. |
| No database driver (Postgres, MySQL, SQLite) | **BLOCKER** | Persistent stores are file-based, no SQL, no transactions, no concurrent access. |
| No WebSocket support | **MAJOR** | Real-time features (chat, notifications) require WebSocket. |
| No cookie / session management | **MAJOR** | Authentication requires cookies or JWT parsing. |
| No URL routing with path params | **MAJOR** | `/users/:id` style routing is fundamental. |
| No form parsing (multipart, urlencoded) | **MAJOR** | Cannot handle file uploads or form submissions. |
| No HTTP/2 or HTTP/3 | **MINOR** | Modern web performance. |
| No CORS handling | **MINOR** | Required for browser-facing APIs. |
| No WASM compilation target | **WISH** | Frontend web development via WebAssembly. |

### Verdict: **NOT VIABLE** — no HTTP server, no TLS, no database drivers, no templating.

---

## 6. Blockchain / Web3 Developer

**Profile**: Builds smart contracts, consensus protocols, cryptographic systems, distributed ledgers. Needs deterministic execution, cryptographic primitives, serialization.

### What Works
- Deterministic memory model (Perceus RC) — reproducible execution
- Integer operations with wrapping/saturating/checked variants — overflow-safe arithmetic
- Struct layout control (`@packed`, `@strict`) — binary protocol construction
- Raw pointers — low-level memory manipulation
- Pattern matching — state machine modeling for protocols
- `store` keyword — persistent key-value storage (conceptually similar to blockchain state)
- Arena allocator — bounded execution environments
- WAL (write-ahead log) in runtime — transaction durability

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No cryptographic primitives (SHA-256, Keccak, Ed25519, secp256k1) | **BLOCKER** | Hashing and signing are the foundation of every blockchain. |
| No big integer / arbitrary precision arithmetic | **BLOCKER** | Token amounts, addresses, and signatures exceed 64-bit. |
| No binary serialization (RLP, SSZ, Borsh, protobuf) | **BLOCKER** | On-chain data encoding requires exact byte-level control. |
| No Merkle tree / trie data structure | **MAJOR** | Core data structure for state verification. |
| No deterministic float behavior (IEEE 754 strictness) | **MAJOR** | Floating-point non-determinism breaks consensus. |
| No gas metering / execution budgets | **MAJOR** | Smart contract VMs need bounded execution. |
| No formal verification hooks | **MAJOR** | Contract correctness proofs are standard practice. |
| No P2P networking (libp2p, noise protocol) | **MAJOR** | Consensus requires peer discovery and encrypted channels. |
| No const generics | **MINOR** | Fixed-size arrays parameterized by compile-time values. |
| No sandboxed execution | **MINOR** | VM isolation for untrusted contract code. |

### Verdict: **NOT VIABLE** — no crypto primitives, no big integers, no binary serialization.

---

## 7. Database Developer

**Profile**: Builds database engines, query planners, storage engines, indexing structures. Needs precise memory control, disk I/O patterns, concurrent data structures.

### What Works
- `store` keyword — built-in persistent records with query, insert, delete, set, transaction
- Runtime: hash index, column store, bloom filter, WAL, vector store, full-text search, KV store, versioning, migration
- Raw pointers + arena — custom allocator patterns
- `@packed`/`@align` — control over on-disk and in-memory layout
- `sim for` — parallel scan operations
- Channels — inter-component communication (e.g., query executor ↔ storage engine)
- String SSO (23-byte inline) — efficient short key handling
- Separate compilation (`--emit-obj`, `--lib`) — modular builds

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No B-tree / B+ tree implementation | **MAJOR** | The fundamental indexing structure for ordered data. Hash index only supports equality. |
| No mmap support from Jade | **MAJOR** | Memory-mapped I/O is critical for database buffer pools. Runtime uses it internally but not exposed. |
| No async / non-blocking disk I/O (io_uring, AIO) | **MAJOR** | High-throughput databases need async I/O to overlap disk and compute. |
| No fine-grained locking (rwlock, optimistic CC) | **MAJOR** | Runtime only has spinlocks. No reader-writer locks, no MVCC support. |
| No SIMD for scan/filter operations | **MAJOR** | Columnar scan with AVX2 predicate evaluation is 10-20× faster than scalar. |
| Store query is brute-force scan | **MAJOR** | No index-backed queries. Every query reads all records. |
| Store strings truncate at 248 bytes | **MAJOR** | Silent data loss for longer values. No variable-length on-disk format. |
| No custom comparator / sort order | **MINOR** | Sort is hardcoded to i64. Cannot sort by composite keys. |
| No memory-mapped buffer pool | **MINOR** | Standard database pattern for page caching. |
| Transaction is a no-op (no rollback) | **MINOR** | `transaction { ... }` compiles but has no ACID semantics. |
| No concurrent reader support | **MINOR** | Store file format assumes single-process access. |

### Verdict: **PARTIALLY VIABLE** — Jade's built-in store is a strong conceptual foundation, but lacks range queries, indexing, real transactions, and async I/O for a production database.

---

## 8. Networking / Distributed Systems Engineer

**Profile**: Builds proxies, load balancers, RPC frameworks, consensus protocols, service meshes. Needs non-blocking I/O, protocol parsers, connection pools, observability.

### What Works
- `net.jade`: TCP and UDP sockets — listen, accept, connect, send, recv
- Channels — pipeline concurrency between protocol stages
- Actor model — natural fit for per-connection actors
- Select — multiplexing across channels (not sockets, but useful internally)
- Work-stealing scheduler — efficient concurrent request handling
- Coroutines — lightweight connection handlers (64 KB stack each)
- `sim for` — parallel request fanout
- `json.jade` — JSON-based protocol parsing

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No epoll/kqueue/io_uring integration | **BLOCKER** | Cannot handle 10K+ connections. One-thread-per-connection doesn't scale. |
| No TLS (OpenSSL / rustls / boringssl) | **BLOCKER** | Cannot build secure services. |
| No DNS resolution | **BLOCKER** | `getaddrinfo` not exposed. Cannot resolve hostnames. |
| IPv4 only | **MAJOR** | Production networks require IPv6 support. |
| No socket select/poll/epoll from Jade | **MAJOR** | `select` only works on channels, not network sockets. |
| No connection pooling | **MAJOR** | Every request opens a new TCP connection. |
| No gRPC / protobuf support | **MAJOR** | Standard for microservice communication. |
| No binary protocol parser combinators | **MAJOR** | Hand-writing byte-level parsers is error-prone. |
| No sendfile / zero-copy | **MINOR** | Performance optimization for proxies. |
| No UDP multicast | **MINOR** | Service discovery patterns. |
| No socket options (TCP_NODELAY, SO_REUSEADDR) | **MINOR** | Performance tuning for networking code. |
| No observability (metrics, tracing, structured logging) | **MINOR** | Production services need distributed tracing. |

### Verdict: **NOT VIABLE** — no event loop, no TLS, no DNS. A basic echo server works, but nothing production-ready.

---

## 9. Operating System Developer

**Profile**: Writes kernels, drivers, bootloaders, init systems. Needs freestanding compilation, inline assembly, precise control over every byte, calling conventions.

### What Works
- Raw pointers (`%`/`@`) — direct memory access
- Volatile load/store — MMIO register access
- `@packed`/`@strict`/`@align` — hardware struct layout
- `asm` keyword parsed — inline assembly (if codegen works)
- Extern C FFI — can call/export C-ABI functions
- `--emit-obj` — can produce object files for custom linking
- Deterministic drops — no hidden allocations in kernel code
- Integer width control (i8..u64) — register-width matching

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No freestanding target (no libc, no OS) | **BLOCKER** | Kernel code runs before libc exists. Runtime assumes mmap, pthreads. |
| No custom calling conventions | **BLOCKER** | Syscall ABI, interrupt frames require specific register layouts. |
| No naked functions | **BLOCKER** | Bootloaders and ISR trampolines need functions with zero prologue/epilogue. |
| No section/segment placement | **BLOCKER** | `.text.boot`, `.rodata`, `.bss` placement is mandatory for linker scripts. |
| No global mutable state | **BLOCKER** | Kernels need `static mut` globals for page tables, GDT, IDT. |
| Inline assembly codegen unverified | **MAJOR** | `asm` keyword parses but may not produce correct LLVM inline asm. |
| No bitwise struct fields | **MAJOR** | Control registers are bit-packed. |
| No alloca-free mode | **MAJOR** | Stack size is tightly bounded in kernel context. |
| No custom panic handler | **MINOR** | Kernel panic ≠ process abort — needs custom handler. |
| No noinline / cold / hot attributes | **MINOR** | Code placement optimization for icache. |

### Verdict: **NOT VIABLE** — 5 blockers. OS development requires freestanding compilation and ABI control that Jade doesn't offer.

---

## 10. Programming Language / Compiler Developer

**Profile**: Builds compilers, interpreters, static analyzers, language tooling. Needs ADTs, pattern matching, tree transformations, efficient symbol tables, visitor patterns.

### What Works
- Enum with data (tagged unions) — AST node representation
- Exhaustive pattern matching — tree dispatch
- Pattern-directed functions — clean recursive descent: `*eval(Literal(n)) is n`
- Generics with monomorphization — type-safe containers without boxing
- Traits + dyn dispatch — visitor pattern, pluggable backends
- Pipelines `~` — IR transformation chains
- `map`/`filter`/`fold` — functional tree operations
- String SSO — efficient identifier interning
- Struct methods — encapsulated pass state
- Recursive functions — tree traversals
- 0.94× C perf — compile speed matters for compilers

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No sum type with shared fields (sealed classes) | **MAJOR** | AST nodes often share span/type fields. Must duplicate in each variant. |
| No trait method default implementations | **MAJOR** | Visitor pattern needs defaults that do nothing for unhandled nodes. |
| No HashMap with custom types as keys | **MAJOR** | Symbol tables need struct keys. Map keys are String-only. |
| No `?` error propagation operator | **MAJOR** | Compiler passes return Result everywhere — manual match is verbose. |
| No recursive enum (direct) | **MAJOR** | `enum Expr { Add(Expr, Expr) }` requires `Box` — Jade has no explicit Box. |
| No string interning / symbol table | **MINOR** | Repeated string comparison is slow for large programs. |
| No mutable captures in closures | **MINOR** | Transformation passes that accumulate state in closures. |
| No custom iterators easily | **MINOR** | Walking complex trees requires manual iterator implementation. |
| No variadic generics | **WISH** | Heterogeneous IR tuple types. |
| No GADT / existential types | **WISH** | Typed IR representations. |

### Verdict: **PARTIALLY VIABLE** — ADTs and pattern matching are strong. Missing error propagation and map limitations make large compiler codebases painful.

---

## 11. Mathematician / Computational Mathematician

**Profile**: Proves theorems computationally, explores number theory, symbolic algebra, numerical methods, combinatorics. Cares about mathematical notation in code, precision, and the ability to express abstract structures. Often switches between exploratory scripts and production implementations.

### What Works
- Full libm via `math.jade` — sqrt, trig, exp, log, gamma, floor, ceil, copysign, fma
- Constants: PI, E, TAU, INF, NEG_INF
- `pow` operator reads naturally: `x pow 2` instead of `pow(x, 2)`
- Pattern-directed functions — mathematical piecewise definitions feel native:
  ```
  *fib(0) is 0
  *fib(1) is 1
  *fib n is fib(n - 1) + fib(n - 2)
  ```
- List comprehensions — set-builder notation: `[x pow 2 for x in 1 to 100 if is_prime(x)]`
- Pipeline `~` — function composition chains: `x ~ normalize ~ transform ~ round`
- `rand.jade` — Monte Carlo sampling, shuffle for randomized algorithms
- Recursive functions — natural expression of inductive definitions
- Wrapping/saturating/checked arithmetic — control over overflow behavior
- 0.94× C performance — competitive for numerical experiments

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No arbitrary-precision integers | **BLOCKER** | Number theory (RSA, primality, combinatorics) needs integers > 64 bits. `100!` overflows i64. |
| No rational number type | **BLOCKER** | Exact arithmetic without floating-point error. `1/3 + 1/3 + 1/3` must equal `1`, not `0.9999...` |
| No complex numbers | **BLOCKER** | Fundamental to analysis, signal processing, quantum mechanics, polynomial roots. |
| No symbolic expressions | **MAJOR** | Cannot represent `x^2 + 3x + 1` as a data structure for manipulation, differentiation, simplification. |
| No matrix/linear algebra | **MAJOR** | Eigenvalues, SVD, LU decomposition, least squares — the backbone of applied math. NDArray was removed. |
| No operator overloading for custom types | **MAJOR** | Cannot define `+` for `Complex`, `*` for `Matrix`, `==` for `Rational`. Must use named methods. |
| No REPL | **MAJOR** | Mathematical exploration is iterative. Compile-run cycle kills the "try this, see what happens" workflow. |
| No `Inf` / `NaN` handling utilities | **MINOR** | `is_nan` and `is_finite` exist but no `nan_to_zero`, `clamp_finite`, or NaN-propagation policy. |
| No automatic differentiation | **MINOR** | `grad` keyword was removed. AD enables optimization, sensitivity analysis, physics simulation. |
| No polynomial/series types | **WISH** | Polynomial arithmetic, Taylor series, formal power series. |
| No interval arithmetic | **WISH** | Verified numerics — prove bounds on floating-point results. |
| No LaTeX / pretty-print output | **WISH** | Mathematical output formatted for readability. |

### Verdict: **NOT VIABLE** — no bignums, no rationals, no complex numbers. A mathematician hits a wall after basic floating-point experiments.

---

## 12. Security Researcher / Hacker

**Profile**: Writes exploit code, fuzzers, protocol analyzers, reverse engineering tools, CTF solvers. Needs raw memory access, byte-level manipulation, network probing, process introspection, and fast prototyping.

### What Works
- Raw pointers (`%`/`@`) — arbitrary memory read/write
- Integer width control (i8..u64) — match target architecture word sizes
- `net.jade` — TCP/UDP raw sockets for port scanning, protocol probing
- `io.jade` — binary file read/write for parsing file formats
- Extern C FFI — call any C library (ptrace, syscalls, libpcap)
- `@packed`/`@strict` — reproduce exact binary structures from specifications
- Wrapping arithmetic — overflow-deliberate calculations for hash collisions, ROP gadgets
- Pattern matching — protocol state machine parsing
- `regex.jade` (PCRE2) — log analysis, pattern extraction
- `strings.jade` — StringBuilder for payload construction
- Compiles to native binary — no VM/interpreter overhead, harder to reverse
- `signal.jade` — handle SIGSEGV for crash analysis

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No raw byte array / buffer type | **BLOCKER** | Need `[u8; N]` or `ByteArray` for payload construction, shellcode, packet crafting. String type is not byte-safe. |
| No hex literal syntax | **BLOCKER** | `0xdeadbeef` is the language of exploit dev. No hex = writing `3735928559` for every address. |
| No bitwise shift / rotate operations | **BLOCKER** | `<<`, `>>`, `>>>`, ROL, ROR are essential for hash implementations, cipher rounds, bit manipulation. |
| No subprocess spawning | **MAJOR** | Cannot exec target binaries, cannot pipe shellcode to processes. |
| No raw syscall interface | **MAJOR** | `syscall(NR, arg0, arg1, ...)` for direct kernel interaction without libc wrappers. |
| No ptrace / process attach | **MAJOR** | Cannot debug, trace, or inject into running processes. |
| No mmap from user code | **MAJOR** | Cannot allocate RWX memory for JIT shellcode, cannot map files. |
| No struct-to-bytes cast | **MAJOR** | Cannot reinterpret a struct as raw bytes or vice versa. Need `transmute` or `as_bytes`. |
| No HTTPS / TLS | **MAJOR** | Cannot probe HTTPS endpoints, cannot MITM with custom certs. |
| No hex encode/decode utility | **MINOR** | `hex()` exists for integers, but no `hex_decode("4141")` → bytes. |
| No socket options (SO_REUSEADDR, IP_HDRINCL) | **MINOR** | Raw socket manipulation for packet injection. |
| No time-of-check/time-of-use primitives | **WISH** | Race condition exploitation tooling. |

### Verdict: **NOT VIABLE** — no byte buffers, no hex literals, no bitwise shifts. The fundamentals of binary exploitation are absent.

---

## 13. Mobile App Developer

**Profile**: Builds iOS and Android applications. Needs UI frameworks, platform APIs, event-driven architecture, asset management, and distribution tooling.

### What Works
- Actor model — natural fit for UI event dispatch and background task isolation
- Pattern matching — UI state machine management
- Enum variants — model screen states, navigation, loading/error/success
- JSON parsing — API response handling
- Struct methods — encapsulate view model logic
- Lightweight coroutines — background operations without callback hell
- 0.94× C perf — smooth animations, responsive UI

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No iOS/Android compilation target | **BLOCKER** | Cannot produce .ipa, .apk, .so, or .framework outputs. |
| No UI framework binding (UIKit, Jetpack Compose, SwiftUI) | **BLOCKER** | Cannot draw a single pixel on a mobile screen. |
| No JNI / Objective-C bridge | **BLOCKER** | Android requires JNI for native code. iOS requires Obj-C runtime interop. |
| No touch / gesture event system | **BLOCKER** | Mobile apps are touch-driven. No input model. |
| No asset bundling (images, fonts, localization) | **MAJOR** | Cannot package resources into an app bundle. |
| No camera / GPS / sensor APIs | **MAJOR** | Core mobile capabilities inaccessible. |
| No push notification integration | **MAJOR** | Expected feature in nearly all mobile apps. |
| No SQLite (on-device storage) | **MAJOR** | Standard mobile persistence approach. |
| No HTTP/2 multiplexing | **MINOR** | Mobile networks benefit from multiplexed requests. |
| No Keychain / secure storage | **MINOR** | Credential storage on mobile platforms. |

### Verdict: **NOT VIABLE** — 4 blockers. Mobile development requires platform SDK integration that Jade has no path to today. The viable route would be Jade-to-C for shared business logic, with platform-native UI.

---

## 14. Scientific Computing / HPC Developer

**Profile**: Runs large-scale simulations — climate models, molecular dynamics, CFD, astrophysics. Needs parallelism, numerical precision, MPI, vectorization, and the ability to process terabytes of data.

### What Works
- `sim for` — parallel loops for embarrassingly parallel workloads
- Work-stealing scheduler — load balancing across cores
- Full libm math — trig, exp, sqrt, fma all present
- `@align(64)` — cache-line alignment for NUMA-aware layout
- Arena allocator — per-simulation-step memory management
- Channels — pipeline parallelism between simulation stages
- 0.94× C performance — within noise of Fortran for scalar code
- List comprehensions — quick data generation for initial conditions

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No MPI / inter-node communication | **BLOCKER** | HPC means multi-node clusters (10K+ cores). Single-machine only is a non-starter. |
| No SIMD intrinsics | **BLOCKER** | AVX-512 is 8–16× for vectorized physics kernels. Auto-vectorization is unreliable. |
| No matrix / tensor operations | **BLOCKER** | Simulations are matrix math. No matmul, no stencil operators, no sparse matrices. |
| No GPU compute (CUDA, OpenCL, Metal) | **BLOCKER** | Modern HPC is GPU-accelerated. Cannot target accelerators. |
| No HDF5 / NetCDF I/O | **MAJOR** | Standard scientific data formats. Cannot read or write simulation checkpoints. |
| No double-double or quad precision | **MAJOR** | Long-chain summations accumulate error. Need 128-bit floats or compensated sums. |
| `sim for` captures nothing | **MAJOR** | Cannot read shared arrays — parallel loops need read-only captures for domain decomposition. |
| No OpenMP-style annotations | **MINOR** | `@parallel`, `@reduction(+, sum)`, `@schedule(dynamic)` for loop tuning. |
| No profiling / performance counters | **MINOR** | Cannot measure FLOPS, cache misses, or memory bandwidth from code. |
| No distributed arrays | **WISH** | Transparent partitioning of large arrays across nodes. |

### Verdict: **NOT VIABLE** — no MPI, no SIMD, no GPU. Single-node `sim for` handles toy problems but nothing at HPC scale.

---

## 15. Quantitative Finance / Fintech Developer

**Profile**: Builds trading systems, risk models, pricing engines, market data processors. Needs sub-microsecond latency, deterministic memory, precise decimal arithmetic, time-series data, and regulatory auditability.

### What Works
- Deterministic Perceus RC — no GC pauses in hot path (critical for latency)
- 0.94× C performance — competitive for pricing kernels
- Full libm math — Black-Scholes, Greeks calculations
- `time.jade` monotonic clock — nanosecond latency measurement
- Channels — market data fan-out, order routing pipelines
- Actor model — per-instrument actors, per-strategy actors
- Pattern matching — order type dispatch, state machine for order lifecycle
- `@align(64)` — cache-line layout for hot data structures
- Arena allocator — pre-allocated per-tick memory
- `sim for` — parallel Monte Carlo for VaR, option pricing
- `store` keyword — audit trail persistence with WAL durability

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No decimal / fixed-point arithmetic | **BLOCKER** | Financial calculations MUST NOT use floating-point. `0.1 + 0.2 ≠ 0.3` is a regulatory violation. |
| No nanosecond timestamp type | **MAJOR** | Market data timestamps are nanosecond-resolution. `time.monotonic()` returns ambiguous units. |
| No lock-free data structures (SPSC queue, ring buffer) | **MAJOR** | Latency-critical paths cannot touch mutexes. Runtime channels use spinlocks. |
| No TCP_NODELAY / socket tuning | **MAJOR** | Nagle's algorithm adds milliseconds of latency. Cannot set socket options. |
| No FIX protocol parser | **MAJOR** | Industry standard for trade messaging. |
| No binary serialization (SBE, FlatBuffers) | **MAJOR** | Zero-copy deserialization for market data feeds. |
| No deterministic float mode | **MINOR** | Need IEEE 754 strict compliance for reproducible pricing across machines. |
| No statistics library (stdev, correlation, percentile) | **MINOR** | Risk metrics require statistical functions. |
| No date/time calendar (business days, roll conventions) | **MINOR** | Trade settlement, maturity dates. |
| No connection pool / multiplexing | **MINOR** | Multiple exchange connections from one process. |

### Verdict: **NOT VIABLE** — decimal arithmetic is a hard regulatory requirement. No workaround exists for `float` in finance.

---

## 16. Educator / CS Student

**Profile**: Learning to program, taking CS courses. Needs clear syntax, good error messages, fast feedback, and progressive complexity from "Hello World" to data structures to compilers.

### What Works
- English-like syntax — `for i from 1 to 10`, `if x equals 0`, `unless done` reads like pseudocode
- `log()` for output — simpler than `printf` format strings
- Pattern-directed functions — mathematical definitions translate directly to code
- `is` for binding — `x is 5` is more intuitive than `x = 5` or `let x = 5`
- No semicolons, no braces — reduces syntactic noise for beginners
- Indentation-based blocks — Python-familiar, visually clean
- Enum + match — teaches algebraic data types naturally
- Pipelines `~` — introduces functional composition gently
- List comprehensions — familiar from Python, mathematical set notation
- Compiles to native — teaches that code becomes machine instructions
- Actor model — teaches concurrency without shared-memory footguns
- Error types (not exceptions) — teaches error-as-values from day one
- `*main` entry point — clear program structure

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No REPL | **BLOCKER** | Beginners need instant feedback. Compile-run cycle for `log('hello')` is discouraging. |
| Error messages could be more pedagogical | **MAJOR** | Compiler errors are functional but don't suggest *why* something went wrong or *how* to fix it. |
| No playground / web IDE | **MAJOR** | Try-without-installing lowers the barrier to zero. Rust Playground, Go Playground set this expectation. |
| No debugger integration | **MAJOR** | Students need to step through code, inspect variables. DWARF info exists but no GDB/LLDB walkthrough. |
| No standard tutorials / book | **MAJOR** | Learning material. jade.md exists but isn't a progressive tutorial. |
| No package manager | **MINOR** | Students sharing code need `jadec install student-lib` simplicity. |
| No test framework | **MINOR** | Teaching TDD requires built-in assertion + test runner. |
| Confusing dual syntax (`as` vs `:` for types, `is` for binding vs equality) | **MINOR** | Parser accepts both old and new syntax — confusing when reading examples. |
| No string interpolation | **MINOR** | `log('x is {x}')` is more natural than concatenation for beginners. |
| No visual debugger / step-through | **WISH** | Visualize execution flow. |

### Verdict: **PARTIALLY VIABLE** — Jade's English-like syntax is arguably the best beginner syntax of any compiled language. A REPL and better error messages would make it genuinely excellent for education.

---

## 17. Audio / DSP Engineer

**Profile**: Builds synthesizers, audio effects, music production tools, speech processors. Needs real-time guarantees, sample-accurate timing, buffer processing, and mathematical signal operations.

### What Works
- 0.94× C performance — within budget for real-time audio (< 1ms per buffer at 48kHz)
- Full libm math — sin, cos, exp, log for oscillators, filters, envelopes
- `math.lerp`, `math.clamp` — common DSP utilities
- Arena allocator — pre-allocate all buffers at startup, zero allocation in audio thread
- `@align(64)` — cache-aligned buffer layout
- Deterministic Perceus — no GC pauses during audio callback (the #1 killer of real-time audio)
- Wrapping arithmetic — intentional overflow for phase accumulators
- Fixed-width integers — sample format matching (i16, i32, f32)
- Struct methods — encapsulate filter state, oscillator state

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No callback function registration with C | **BLOCKER** | Audio APIs (ALSA, CoreAudio, JACK, PortAudio) call your function. Need to pass Jade function pointer to C callback slot. |
| No `f32` array operations | **BLOCKER** | Audio is f32 buffers. No bulk f32 add/multiply/copy — must loop element-by-element. |
| No SIMD for buffer processing | **MAJOR** | 4× throughput with SSE for sample processing. Without SIMD, cannot meet real-time budget for complex effects. |
| No ring buffer with f32 | **MAJOR** | std/collections RingBuffer is i64-only. Delay lines, circular buffers are core DSP primitives. |
| No complex FFT | **MAJOR** | Spectral analysis, convolution reverb, pitch detection all need FFT. |
| No `@noinline` / `@realtime` annotations | **MAJOR** | Mark functions as "must not allocate, must not block" for audio thread safety. |
| No shared memory / lock-free queue (audio ↔ UI thread) | **MAJOR** | Audio thread and UI thread must communicate without locks. |
| No MIDI support | **MINOR** | Standard for music software input/output. |
| No sample rate conversion | **MINOR** | Resampling between 44.1kHz and 48kHz. |
| No dB / amplitude conversion utilities | **WISH** | `db_to_amp`, `amp_to_db`, `freq_to_midi`, `midi_to_freq`. |

### Verdict: **NOT VIABLE** — cannot register C callbacks (function pointer FFI gap), no f32 bulk operations, no SIMD. The language *almost* has the right properties for real-time audio but can't connect to audio hardware APIs.

---

## 18. Automation / Scripting Developer

**Profile**: Writes glue code, build scripts, data migration tools, cron jobs, system administration scripts. Values fast startup, easy file manipulation, text processing, and low ceremony.

### What Works
- `fs.jade` — walk directories, copy/move/rename files, check existence
- `io.jade` — read/write files, read_lines for text processing
- `path.jade` — join, normalize, dir/base/ext manipulation
- `regex.jade` (PCRE2) — powerful text pattern matching
- `json.jade` — parse configs, transform data files
- `os.jade` — env vars, PID, basic CLI args
- `strings.jade` — StringBuilder for output construction
- `fmt.jade` — pad, join, hex formatting
- Pipeline `~` — chain text transformations naturally
- `for line in read_lines(path)` — idiomatic line processing
- Pattern matching — dispatch on file types, error conditions
- Single-file programs — no project setup needed
- Compiles to native binary — deploy a single file to any box

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No subprocess / shell command execution | **BLOCKER** | Scripts orchestrate other programs. `system()` exists in os.jade but no stdout capture. |
| No glob / pattern matching on filenames | **MAJOR** | `*.log`, `src/**/*.jade` directory filtering is essential for file-processing scripts. |
| No string interpolation | **MAJOR** | Building commands and messages with `'cp ' + src + ' ' + dst` is painful. `f'cp {src} {dst}'` expected. |
| No REPL for quick experiments | **MAJOR** | Often want to test a regex or file operation interactively before scripting. |
| No shebang support | **MAJOR** | `#!/usr/bin/env jadec run` for executable scripts. Currently must compile first. |
| No `system()` with stdout capture | **MAJOR** | `os.system(cmd)` returns exit code only. Need stdout/stderr capture. |
| No temporary file/directory utilities | **MINOR** | `mktemp`, `with_temp_dir { ... }` patterns. |
| No colored terminal output | **MINOR** | Progress indicators, error highlighting for interactive scripts. |
| No CSV parsing | **MINOR** | Common data migration source format. |
| Compile time overhead for small scripts | **MINOR** | Even 100ms compile penalizes rapid iteration vs Python/Bash. |

### Verdict: **NOT VIABLE** — no subprocess execution with output capture is the single blocker. With that plus string interpolation, Jade could be a strong compiled scripting language.

---

## 19. Aviation / Aerospace Engineer (DO-178C)

**Profile**: Develops flight control software, avionics, satellite firmware, missile guidance, autonomous navigation. Subject to DO-178C (aviation), ECSS-E-ST-40C (space), or MIL-STD-498. Every line of code must be traceable, tested, and auditable. Certification costs $50K–$500K per changed LOC.

### What Works
- Deterministic memory (Perceus RC) — no GC pauses, predictable latency
- No exceptions — error-as-values prevents hidden control flow
- Exhaustive match checking — compiler proves all cases handled, reduces unhandled-state risk
- Pattern-directed functions — clean state machine definitions for mode logic
- `@packed`/`@strict`/`@align` — hardware register and data link frame layout control
- Integer width control (i8..u64) — match avionics bus widths (ARINC 429 = 32-bit words)
- Compiles via LLVM to native — can inspect generated machine code
- `--emit-llvm` — auditable intermediate representation output
- No implicit coercions — type safety reduces silent data corruption

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No qualified compiler (DO-330 TQL) | **BLOCKER** | Compilers used in DAL A/B software must be qualified or all output must be verified. Jade has no qualification kit. |
| No MCDC coverage tooling | **BLOCKER** | DO-178C requires Modified Condition/Decision Coverage. No built-in or compatible coverage tool. |
| No formal semantics | **BLOCKER** | Certification requires a precise, unambiguous language specification. jade.md/jade.ebnf is informal. |
| No traceability from requirements → code → tests | **BLOCKER** | Every line must trace to a requirement. No annotation or reporting mechanism. |
| No static analysis (MISRA-like rules) | **BLOCKER** | Avionics requires static analysis proving absence of undefined behavior, dead code, unreachable code. |
| No freestanding / bare-metal target | **BLOCKER** | Avionics runs on VxWorks, RTEMS, or bare-metal ARM/PowerPC. Runtime assumes Linux. |
| No worst-case execution time (WCET) analysis | **MAJOR** | Certifiers need proof that every code path completes within deadline. |
| No stack depth analysis | **MAJOR** | Avionics stacks are fixed and small. Must prove no stack overflow. |
| No `restrict` / alias analysis proofs | **MAJOR** | Proving no aliasing is required for optimization safety in certified builds. |
| No deterministic floating-point | **MAJOR** | IEEE 754 strict mode needed — same result on every target. |
| Runtime uses heap allocation | **MAJOR** | Many avionics standards prohibit dynamic allocation after init. |
| No ARINC 653 / POSIX PSE51 compliance | **MINOR** | Standard APIs for partitioned avionics OSes. |
| No redundancy / voting patterns | **WISH** | Triple modular redundancy patterns for fault tolerance. |

### Verdict: **NOT VIABLE** — 6 blockers. Aviation certification demands formal semantics, qualified toolchains, and code-level traceability that no young language provides. The path to DO-178C qualification is multi-year and requires a dedicated certification team.

---

## 20. Automotive / Industrial Safety Engineer (ISO 26262 / IEC 61508)

**Profile**: Develops ADAS, powertrain control, industrial PLC logic, medical device firmware. Subject to ISO 26262 (automotive ASIL A–D), IEC 61508 (industrial SIL 1–4), or IEC 62304 (medical). Less stringent than aviation but still requires evidence-based safety arguments.

### What Works
- Same strengths as embedded (deterministic RC, no GC, volatile, layout control)
- Exhaustive pattern matching — state machines for control modes
- No hidden control flow (no exceptions, no implicit allocations in tight paths)
- Actor model — could model ECU-to-ECU communication patterns
- `--emit-obj` + extern C — integrate with existing AUTOSAR/ROS stacks
- `store` keyword — event logging for diagnostic trouble codes (DTCs)

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No MISRA-equivalent coding standard | **BLOCKER** | ISO 26262 requires an "appropriate" language subset. No Jade subset is defined. |
| No qualified compiler | **BLOCKER** | ASIL C/D requires tool qualification or independent output verification. |
| No static analysis framework | **BLOCKER** | Polyspace, Coverity equivalents needed for absence-of-runtime-error proofs. |
| No unit test framework with coverage | **BLOCKER** | Must demonstrate structural coverage (statement, branch, MCDC at higher ASIL). |
| No freestanding mode | **BLOCKER** | Automotive targets Infineon AURIX, Renesas RH850 — no Linux, no libc. |
| No cross-compilation | **MAJOR** | Need `--target tricore-elf`, `--target arm-none-eabi`. |
| No AUTOSAR interface generation | **MAJOR** | Industry-standard component model for automotive ECUs. |
| No watchdog / timing supervision hooks | **MINOR** | Safety monitors need to verify task deadlines. |
| No CRC / checksum builtins | **MINOR** | Data integrity on CAN/LIN/FlexRay buses. |
| No CAN / LIN / SPI protocol support | **MINOR** | Automotive bus interfaces. |

### Verdict: **NOT VIABLE** — same fundamental gaps as aviation. The shared root cause is lack of formal specification and toolchain qualification.

---

## 21. Robotics / Autonomous Systems Engineer

**Profile**: Develops control loops, perception pipelines, motion planning, sensor fusion for drones, robots, self-driving vehicles. Needs real-time guarantees, matrix math, and hardware interfacing.

### What Works
- 0.94× C performance — viable for control loops at 1kHz+
- `math.jade` — trig, atan2, sqrt for kinematics/transforms
- `sim for` — parallel sensor data processing
- Channels — pipeline architecture (sensor → filter → planner → actuator)
- Actor model — per-subsystem actors (vision, navigation, control)
- Arena allocator — per-cycle allocation pattern
- Deterministic drops — no GC pauses in the control loop
- Extern C FFI — bind to ROS 2, OpenCV, PCL libraries
- Timer — monotonic clock for loop timing

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No matrix / linear algebra | **BLOCKER** | Rotation matrices, homogeneous transforms, Kalman filters — all matrix math. |
| No ROS 2 / DDS integration | **BLOCKER** | Standard robotics middleware. Without pub/sub message passing to ROS, the robot can't see/move. |
| No real-time scheduling guarantees | **MAJOR** | Control loops need SCHED_FIFO or deadline scheduling. Coroutine scheduler is best-effort. |
| No SIMD | **MAJOR** | Point cloud processing, image convolution need vectorized operations. |
| No GPU compute | **MAJOR** | Neural network inference for perception (YOLO, etc.) typically runs on GPU. |
| No quaternion type | **MAJOR** | 3D rotation representation — avoids gimbal lock. |
| No PID controller library | **MINOR** | Fundamental control pattern, easy to implement but should be standard. |
| No coordinate frame tracking | **MINOR** | Transform trees (TF2 in ROS) manage sensor-to-world transforms. |
| No hardware timer / PWM access | **MINOR** | Motor control on embedded platforms. |
| No state estimation library (EKF, UKF) | **WISH** | Sensor fusion filters. |

### Verdict: **NOT VIABLE** — no matrix math and no ROS integration are the twin blockers. A robotics engineer cannot express a basic transform or publish a message.

---

## 22. DevSecOps / Cloud-Native Developer

**Profile**: Builds container images, Kubernetes operators, service meshes, CI/CD pipelines, infrastructure-as-code tools. Lives in YAML, Dockerfiles, and cloud APIs. Needs small binaries, fast startup, and cloud API clients.

### What Works
- Compiles to static native binary — ideal for `FROM scratch` Docker images
- Fast startup — no runtime initialization (unlike JVM, .NET)
- `json.jade` — Kubernetes API responses are JSON
- `os.jade` — environment variables for 12-factor app config
- `fs.jade` — file manipulation for config templating
- `path.jade` — path operations for build artifact management
- Actor model — operator reconciliation loops
- Channels — event stream processing
- `signal.jade` — graceful SIGTERM handling for container shutdown

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No HTTPS / TLS | **BLOCKER** | Every cloud API (Kubernetes, AWS, GCP) is HTTPS-only. |
| No YAML parser | **BLOCKER** | Kubernetes manifests, Helm charts, GitHub Actions — all YAML. |
| No subprocess execution | **BLOCKER** | Operators run `kubectl`, `helm`, `docker` as child processes. |
| No gRPC / protobuf | **MAJOR** | Kubernetes API server, Envoy xDS, all use gRPC. |
| No static linking (musl) | **MAJOR** | `FROM scratch` requires fully static binaries. Currently links glibc. |
| No YAML generation / templating | **MAJOR** | Producing Kubernetes manifests programmatically. |
| No cloud SDK (AWS, GCP, Azure) | **MAJOR** | API calls for provisioning, monitoring, secrets. |
| No container image building | **MINOR** | Building OCI images programmatically (like Kaniko, Buildpacks). |
| No metrics (Prometheus exposition) | **MINOR** | Operators need `/metrics` endpoint. |
| No structured logging (JSON logs) | **MINOR** | Cloud-native observability standard. |

### Verdict: **NOT VIABLE** — HTTPS, YAML, and subprocess are all blockers. Phase 1 + a YAML parser would make this viable.

---

## 23. Bioinformatics / Computational Biology Developer

**Profile**: Processes genomes (FASTA/FASTQ), runs sequence alignment, builds phylogenetic trees, analyzes protein structures. Deals with terabyte-scale data, string matching on 4-letter alphabets, and statistical models.

### What Works
- `regex.jade` (PCRE2) — pattern matching on DNA sequences
- `strings.jade` / `fmt.jade` — sequence manipulation and formatting
- `io.jade` — line-by-line file reading for FASTA parsing
- `sort.jade` — sorting alignment hits
- `math.jade` — log-likelihood scoring
- `rand.jade` — Monte Carlo for phylogenetics bootstrapping
- `sim for` — parallel alignment of independent sequences
- Pipeline `~` — data transformation chains (read → filter → align → score)
- Compiles to native — performance-sensitive for genome-scale data

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No FASTA/FASTQ/SAM/BAM/VCF parsers | **BLOCKER** | Core file formats. Without these, cannot ingest any biological data. |
| No gzip / zlib decompression | **BLOCKER** | All real-world genomic data is gzipped. FASTQ.gz files are the input. |
| No suffix array / BWT / FM-index | **MAJOR** | Data structures for genome indexing (BWA, Bowtie). Linear search is O(genome) per query. |
| No SIMD for string comparison | **MAJOR** | Smith-Waterman alignment with SSE is 10-50× faster. |
| No bitwise operations | **MAJOR** | 2-bit DNA encoding (A=00, C=01, G=10, T=11) is standard for memory efficiency. |
| No memory-mapped file I/O | **MAJOR** | Reference genomes (3GB human) must be memory-mapped, not buffered. |
| No statistics (p-values, distributions) | **MAJOR** | Significance testing for variant calls, differential expression. |
| No multi-threading with shared read-only data | **MAJOR** | Workers need shared reference genome. `sim for` forbids captures. |
| No streaming/iterator composition | **MINOR** | Process billions of reads without loading into memory. |
| No plotting | **WISH** | Manhattan plots, coverage histograms, alignment visualizations. |

### Verdict: **NOT VIABLE** — no domain file format parsers and no gzip decompression. Even the first step (reading a compressed FASTQ) is impossible.

---

## 24. AI / Machine Learning Engineer

**Profile**: Trains neural networks, builds inference pipelines, processes datasets, deploys models. Needs tensor operations, GPU compute, automatic differentiation, and large-scale data loading.

### What Works
- 0.94× C performance — viable for inference engines if tensors existed
- `math.jade` — activation functions (exp, log, tanh available via libm)
- `rand.jade` — weight initialization, data augmentation shuffling
- `sim for` — parallel data preprocessing
- Pipeline `~` — model as chain of transforms
- Actor model — distributed training coordination (conceptually)
- `json.jade` — model config files, dataset metadata
- Arena allocator — per-batch memory

### What's Missing

| Gap | Severity | Detail |
|-----|----------|--------|
| No tensor / ndarray type | **BLOCKER** | ML is tensor math. No matmul, no broadcasting, no reshape, no einsum. Grammar had these but codegen was removed. |
| No GPU compute (CUDA, ROCm, Metal) | **BLOCKER** | Training and inference run on GPU. CPU-only is 100-1000× slower. |
| No automatic differentiation | **BLOCKER** | Backpropagation requires autodiff. `grad` keyword was parser-only and removed. |
| No model serialization (ONNX, SafeTensors, pickle) | **BLOCKER** | Cannot save/load trained models. |
| No BLAS/LAPACK binding | **MAJOR** | Optimized matrix operations are the foundation of ML math. |
| No data loading pipeline (batching, shuffling, prefetch) | **MAJOR** | Training requires streaming data in parallel with compute. |
| No image loading (PNG, JPEG) | **MAJOR** | Computer vision datasets are images. |
| No sparse tensor support | **MINOR** | NLP models, graph neural networks, recommendation systems. |
| No distributed training (parameter server, all-reduce) | **MINOR** | Multi-GPU / multi-node training. |
| No JIT compilation of compute graphs | **WISH** | Dynamic model shapes require runtime compilation. |

### Verdict: **NOT VIABLE** — 4 blockers. ML is fundamentally a GPU + tensor + autodiff domain. No language competes with Python's ecosystem here without massive framework investment.

---

# Formal Verification: Analysis & Roadmap

## Can Jade Be Formally Verified?

**Short answer**: Partially, with significant investment. Full formal verification of the compiler to aerospace certification standards (DO-330 TQL-1) is theoretically possible but practically a multi-year, multi-million-dollar effort. However, *incremental* formal methods can yield high confidence at much lower cost.

## Current State of Assurance

| Property | Status | Evidence |
|----------|--------|----------|
| **Test coverage** | Moderate | 1,282 tests (221 unit + 800 bulk + 261 integration), all passing |
| **Compiler written in** | Rust | Memory-safe, no undefined behavior in the compiler itself |
| **LLVM backend** | Shared with Clang | LLVM is battle-tested but not formally verified |
| **Formal spec** | None | jade.ebnf is informal, jade.md is a guide — neither is a formal semantics |
| **Type soundness proof** | None | Type system is tested but not proven sound |
| **Memory safety proof** | None | Perceus RC is deterministic but correctness not formally established |
| **Certified compilation** | None | No CompCert-like guarantee that codegen preserves semantics |

## What Formal Verification Would Mean

Three distinct levels of rigor, each independently valuable:

### Level 1: Formal Language Specification
**What**: An unambiguous, mathematical definition of Jade's syntax and semantics.
**Why**: Required for DO-178C/DO-330, ISO 26262, IEC 61508. Also eliminates "implementation is the spec" ambiguity.
**How**:
1. **Formalize the grammar** — Convert jade.ebnf to a parser-generator-verified grammar (ANTLR, Menhir) with proven unambiguity
2. **Operational semantics** — Write small-step or big-step rules for each expression/statement form in a proof assistant (Coq, Lean 4, Isabelle/HOL)
3. **Type system specification** — Define typing judgments (`Γ ⊢ e : τ`) covering inference, unification, monomorphization, and trait resolution
4. **Memory model** — Formalize Perceus RC rules: when retains/releases are inserted, when reuse is safe, when borrows are valid
5. **Concurrency model** — Formalize actor mailbox semantics, channel blocking, coroutine scheduling fairness

**Effort**: 6–12 months, 1–2 formal methods researchers
**Deliverable**: Machine-checked specification in Lean 4 or Coq, publishable as a reference semantics

### Level 2: Type Soundness & Memory Safety Proofs
**What**: Mathematical proof that well-typed Jade programs do not exhibit type errors or memory violations at runtime.
**Why**: The "progress + preservation" theorem. Proves the type checker and ownership system are correct — not just tested.
**How**:
1. **Formalize the core type system** — Hindley-Milner with extensions (TypeVar unification, traits, monomorphization)
2. **Prove progress**: If `Γ ⊢ e : τ` and `e` is not a value, then `e` can take a step
3. **Prove preservation**: If `Γ ⊢ e : τ` and `e → e'`, then `Γ ⊢ e' : τ`
4. **Prove Perceus safety**: If all RC operations are correctly inserted, no use-after-free, no double-free, no leak
5. **Prove exhaustiveness**: Match checker rejects all non-exhaustive patterns (currently checked but not proven complete)

**Effort**: 12–24 months, 2–3 researchers
**Prerequisite**: Level 1 specification
**Deliverable**: Coq/Lean proof artifacts, peer-reviewed paper

### Level 3: Verified Compilation (CompCert-class)
**What**: Machine-checked proof that the compiler's output preserves the semantics of the input program.
**Why**: The gold standard. Guarantees the executable does what the source says. Required for DO-178C DAL A without separate output verification.
**How**:
1. **Formalize HIR** — Define the HIR intermediate representation as a formal language
2. **Formalize LLVM IR subset** — Define the target semantics (or use Vellvm, an existing Coq formalization of LLVM IR)
3. **Prove each compiler pass correct**:
   - **Parser → AST**: Parser produces correct parse tree for grammar (can use verified parser generators)
   - **AST → HIR (Typer)**: Type-checking and lowering preserves semantics (hardest pass — monomorphization, inference, trait resolution)
   - **HIR → HIR (Perceus)**: RC insertion preserves behavior while adding memory management
   - **HIR → LLVM IR (Codegen)**: Each HIR node compiles to semantically equivalent LLVM IR
   - **LLVM IR → Machine code**: Out of scope — relies on LLVM (or replace with verified backend like CompCert's)
4. **Verify optimizations**: Each MIR optimization pass (constant fold, DCE, GVN, LICM, etc.) must be proven to preserve observable behavior

**Effort**: 3–5+ years, 4–6 researchers (CompCert took ~6 person-years for a simpler C subset)
**Prerequisite**: Levels 1 and 2
**Deliverable**: A mechanically verified Jade compiler, potentially the first verified compiler for a language with Perceus RC

## Pragmatic Intermediate Steps

Full Level 3 verification is years away, but these steps provide increasing assurance today:

### Step 1: Property-Based Testing (Effort: Weeks)
- Add QuickCheck/proptest to the Rust test suite
- Generate random Jade programs, compile, execute, check invariants:
  - Type checker accepts → program doesn't crash with type error at runtime
  - Perceus hints → no leak (compare valgrind output)
  - Optimizer → same output as unoptimized
- **Tools**: proptest (Rust), Csmith-like generator for Jade ASTs
- **Already feasible** with current infrastructure

### Step 2: Differential Testing Against C (Effort: Weeks)
- For each Jade program with a C equivalent (28 benchmarks exist), compile both and compare outputs
- Extend to auto-generated programs with known semantics
- Catches codegen bugs, optimizer regressions
- **Already partially done** — benchmark comparison infrastructure exists

### Step 3: Formal Grammar Verification (Effort: 1–2 Months)
- Convert jade.ebnf to a formally verified parser in Menhir or tree-sitter with conflict-free proof
- Guarantees: no ambiguity, every valid program parses to exactly one AST
- **Low-hanging fruit** — tree-sitter-jade already exists; add unambiguity proofs

### Step 4: LLVM IR Validation (Effort: 1–2 Months)
- Run `llvm-verify` / `opt -verify` on all emitted IR
- Add Alive2 (automated LLVM optimization verifier) to CI
- Catches undefined behavior in generated IR, illegal optimizations
- **Already feasible** — just needs CI integration

### Step 5: Rust Type System as Leverage (Effort: Ongoing)
- The Jade compiler is written in Rust — already memory-safe, no buffer overflows, no use-after-free *in the compiler itself*
- Add `#[deny(unsafe_code)]` to the compiler crate (Jade uses zero unsafe blocks already)
- This provides a partial argument: the compiler cannot corrupt its own memory, reducing the class of possible miscompilations
- **Already true** — just needs documentation for certification artifacts

### Step 6: Coverage & Mutation Testing (Effort: 1–2 Months)
- Measure line/branch coverage of the compiler test suite (currently unmeasured)
- Run cargo-mutants or mutagen to find under-tested paths
- Target: 90%+ line coverage on typer.rs, codegen/, perceus.rs
- Provides evidence for safety arguments even without formal proofs

## Aviation/Aerospace Certification Path (DO-178C / DO-330)

For a language to be used in certified avionics software, one of these must be true:

**Option A: Qualify the Compiler (DO-330 TQL-1)**
- Requires formal specification (Level 1), extensive test suite, traceability matrix
- Cost: $2M–$10M, 2–4 years with a certification authority (FAA/EASA)
- Precedent: AdaCore qualifies GNAT compiler for DO-178C projects

**Option B: Verify All Compiler Output**
- Don't qualify the compiler; instead, independently verify every emitted binary
- Disassemble output, prove it matches source semantics via review + analysis
- Cost: $50K–$500K per application (proportional to code size)
- Practical for small, safety-critical Jade programs (< 10K LOC)

**Option C: Use Jade as a Design Language Only**
- Write algorithms in Jade, verify behavior, then manually translate to qualified Ada/C
- Jade serves as executable specification
- Lowest barrier — leverages Jade's readable syntax for design documents

**Recommendation**: Start with Step 1–6 (pragmatic testing hardening, ~3 months). Then pursue Level 1 formalization (formal spec, ~12 months). This combination provides evidence for ISO 26262 ASIL B and below. Aviation DAL A/B certification (Levels 2–3) is a long-term research investment.

---

# Remediation Plan

Prioritized by cross-cutting impact (number of perspectives unblocked).

## Phase 1 — Foundation (Unblocks 16/24 perspectives)

### 1.1 Subprocess Spawning
**Unblocks**: DevOps, Web, Game (asset pipelines), Networking, OS (testing), Hacker, Automation
**Scope**: Add `process.spawn(cmd, args)`, `process.output(cmd)`, `process.pipe()` to std/os.jade
**Effort**: Small — wraps `posix_spawn` / `fork`+`exec` via extern C
**Implementation**:
- Add extern declarations for `posix_spawn`, `waitpid`, `pipe`, `dup2`
- Create `std/process.jade` with `run(cmd, args) returns ProcessResult` and `spawn(cmd, args) returns Process`
- `Process` type with `stdin_write`, `stdout_read`, `wait`, `kill` methods
- `ProcessResult` type with `exit_code`, `stdout`, `stderr` fields

### 1.2 TLS / HTTPS Support  
**Unblocks**: DevOps, Web, Networking, Blockchain (P2P), Hacker, FinTech
**Scope**: Bind to OpenSSL (or BearSSL for minimal footprint)
**Effort**: Medium — extern C bindings + wrapper in `std/tls.jade`
**Implementation**:
- Add extern declarations for `SSL_CTX_new`, `SSL_new`, `SSL_connect`, `SSL_read`, `SSL_write`
- Create `std/tls.jade` with `TlsStream` wrapping `TcpStream`
- Update `std/http.jade` to use TLS when URL scheme is `https`
- Link `-lssl -lcrypto` when TLS module is imported

### 1.3 DNS Resolution
**Unblocks**: DevOps, Web, Networking
**Scope**: Expose `getaddrinfo` via extern
**Effort**: Small
**Implementation**:
- Add `resolve(host) returns Vec of String` to `std/net.jade`
- Extern `getaddrinfo`, `freeaddrinfo`, iterate linked list of results
- Update `tcp_connect` and `http.request` to auto-resolve hostnames

### 1.4 Cross-Compilation Support
**Unblocks**: Embedded, DevOps, Game, OS
**Scope**: `--target <triple>` flag using LLVM's target infrastructure
**Effort**: Medium — Jade already has LLVM; needs target triple propagation
**Implementation**:
- Accept `--target <triple>` in CLI, pass to `TargetMachine::create`
- Accept `--cpu <name>` and `--features <+avx2,+sse4.2>` flags
- Respect target data layout in `type_store_size` and `type_abi_align`
- Add built-in targets: `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`, `wasm32-wasi`
- Emit appropriate `.o` format per target

### 1.5 Error Propagation Operator (`?`)
**Unblocks**: Compiler dev, Web, DevOps, Networking, Automation, FinTech (every non-trivial program)
**Scope**: Desugar `expr?` → `match expr { Ok(v) → v, Err(e) → ! e }`
**Effort**: Small — parser + typer desugaring
**Implementation**:
- Add `Token::Question` postfix handling in `parse_postfix`
- Desugar to match expression in typer or parser
- Works with existing `err` types and Result enum

## Phase 2 — Ecosystem Expansion (Unblocks 16/24 perspectives)

### 2.1 Generic Collections
**Unblocks**: Data Scientist, Game, Compiler, Database, DSP, FinTech, Mathematician
**Scope**: Make Stack, Queue, RingBuffer generic; add custom Map key support
**Effort**: Small — rewrite std/collections.jade using `of T`
**Implementation**:
- Replace `i64` with `of T` parameter in Stack, Queue, RingBuffer
- Extend Map to support struct keys via `Hash` and `Eq` trait requirements
- Add `HashMap of K, V` with FNV-1a on arbitrary types
- Sort with custom comparator: `sort(arr, |a, b| a.name < b.name)`

### 2.2 HTTP Server Framework
**Unblocks**: Web, DevOps (webhook receivers), Networking
**Scope**: Minimal router over existing TCP + coroutines
**Effort**: Medium
**Implementation**:
- Create `std/http_server.jade` with `Server` type
- `Server.route(method, path, handler)` — path with `:param` extraction
- `Server.listen(host, port)` — accept loop, spawn coroutine per connection
- Parse request line + headers, pass `Request` to handler, write `Response`
- Middleware chain via `Server.use(middleware_fn)`

### 2.3 Database Driver (SQLite)
**Unblocks**: Web, DevOps, Database, Data Scientist
**Scope**: Bind to SQLite3 C API
**Effort**: Medium
**Implementation**:
- Extern declarations for `sqlite3_open`, `sqlite3_prepare_v2`, `sqlite3_step`, `sqlite3_column_*`, `sqlite3_finalize`
- Create `std/sqlite.jade` with `Db`, `Statement`, `Row` types
- `Db.open(path)`, `Db.exec(sql)`, `Db.query(sql, params) returns Vec of Row`
- Parameterized queries to prevent SQL injection
- Link `-lsqlite3` when imported

### 2.4 TOML / YAML Parser
**Unblocks**: DevOps, Web (config)
**Scope**: At minimum TOML (simpler spec). YAML can follow.
**Effort**: Medium — pure Jade recursive-descent (like json.jade)
**Implementation**:
- Create `std/toml.jade` with `parse(text) returns TomlValue`
- `enum TomlValue { TStr, TInt, TFloat, TBool, TArray, TTable, TDatetime }`
- Nested table support, array-of-tables, inline tables

### 2.5 CSV Parser
**Unblocks**: Data Scientist, DevOps (log analysis), Automation, FinTech
**Scope**: RFC 4180 CSV parser
**Effort**: Small — pure Jade
**Implementation**:
- Create `std/csv.jade` with `parse(text) returns Vec of Vec of String`
- Handle quoted fields, escaped quotes, newlines in fields
- `parse_with_headers(text) returns Vec of Map`
- Streaming: `read_csv(path, callback)` processes row-by-row

### 2.6 Hex Literals & Bitwise Operations
**Unblocks**: Hacker, Embedded, OS, DSP, Blockchain
**Scope**: `0xFF` literal syntax, `<<` `>>` `>>>` shift operators
**Effort**: Small — lexer + parser + codegen
**Implementation**:
- Lexer: recognize `0x[0-9a-fA-F]+` and `0b[01]+` as integer literals
- Parser/codegen: `shl`, `shr`, `ushr` (or `<<`, `>>`, `>>>`) operators → LLVM `shl`, `ashr`, `lshr`
- Rotate: `rotl(x, n)`, `rotr(x, n)` builtins → LLVM `fshl`/`fshr` intrinsics
- Hex string decode: `hex_decode("deadbeef") returns Vec of u8`

### 2.7 Byte Buffer / Raw Bytes Type
**Unblocks**: Hacker, Networking, DSP, Blockchain, Binary serialization
**Scope**: First-class byte array type distinct from String
**Effort**: Medium
**Implementation**:
- Add `Bytes` type backed by `Vec of u8` with non-UTF8-safe operations
- Methods: `get(idx)`, `set(idx, val)`, `slice(from, to)`, `concat`, `len`
- Construction: `Bytes.from_hex("41414141")`, `Bytes.zeroed(n)`, `Bytes.from_string(s)`
- Conversion: `to_hex()`, `to_string_lossy()`
- Integration: net.read returns Bytes, io.read returns Bytes

### 2.8 String Interpolation
**Unblocks**: Automation, Educator, DevOps, Web, all general-purpose use
**Scope**: `f'hello {name}, you are {age} years old'` syntax
**Effort**: Small — lexer desugaring to concat
**Implementation**:
- Lexer recognizes `f'...'` or backtick strings with `{expr}` interpolation
- Desugar to string concatenation + `to_string()` calls at parse time
- Expressions inside `{}` are full expressions (method calls, arithmetic)

## Phase 3 — Systems Capabilities (Unblocks 14/24 perspectives)

### 3.1 Event Loop / Async I/O
**Unblocks**: Networking, Web, Database, Game
**Scope**: Integrate epoll/kqueue with coroutine scheduler
**Effort**: Large
**Implementation**:
- Add `jade_epoll_create`, `jade_epoll_ctl`, `jade_epoll_wait` to runtime
- When coroutine does `recv()` on non-ready socket, park and register with epoll
- Scheduler polls epoll in idle loop instead of spinning
- Expose as `async_read`, `async_write` that yield coroutine on EAGAIN
- Transparent to user code — `net.read()` becomes non-blocking automatically

### 3.2 Freestanding / No-Runtime Mode
**Unblocks**: Embedded, OS
**Scope**: `--freestanding` flag that excludes libc/pthread/mmap dependencies
**Effort**: Large
**Implementation**:
- Split runtime into `core` (no OS) and `std` (with OS) components
- `--freestanding`: skip scheduler, channels, actors, coroutine stack setup
- Allow user to provide `*_start` entry point instead of `*main`
- Expose LLVM inline assembly for syscalls / interrupts
- No implicit `malloc` — only arena or user-provided allocators

### 3.3 Cryptographic Primitives
**Unblocks**: Blockchain, Networking (TLS support), Web (auth), Hacker, FinTech
**Scope**: Bind to a C crypto library (libsodium or OpenSSL libcrypto)
**Effort**: Medium
**Implementation**:
- Create `std/crypto.jade` with extern bindings
- Hashing: `sha256(data)`, `sha512(data)`, `blake2b(data)`
- HMAC: `hmac_sha256(key, data)`
- Symmetric: `aes256_gcm_encrypt(key, nonce, plaintext)`, `aes256_gcm_decrypt(...)`
- Asymmetric: `ed25519_sign(key, msg)`, `ed25519_verify(pub, msg, sig)`
- Random: `random_bytes(n)` from OS entropy
- Link `-lsodium` or `-lcrypto` when imported

### 3.4 IPv6 Support
**Unblocks**: Networking, Web, DevOps
**Scope**: Extend net.jade socket types
**Effort**: Small
**Implementation**:
- Add `sockaddr_in6` handling alongside `sockaddr_in`
- Auto-detect IPv4 vs IPv6 from address format or DNS result
- Expose `AF_INET6` in socket creation

### 3.5 Big Integer Library
**Unblocks**: Blockchain, Data Scientist, Crypto, Mathematician, FinTech
**Scope**: Arbitrary-precision integer arithmetic
**Effort**: Medium — can bind to GMP or implement in Jade
**Implementation**:
- Create `std/bigint.jade` with `BigInt` type
- Backed by `Vec of u64` limbs with schoolbook multiply (or GMP extern)
- Operations: add, sub, mul, div, mod, pow, gcd, modpow
- Conversion: `from_string`, `to_string`, `to_hex`

### 3.6 Rational & Complex Number Types
**Unblocks**: Mathematician, HPC, DSP, FinTech (exact arithmetic)
**Scope**: `Rational` (exact fractions) and `Complex` (a + bi) types
**Effort**: Medium
**Implementation**:
- Create `std/rational.jade`: `Rational(num as i64, den as i64)` with auto-normalization via GCD
- Operations: add, sub, mul, div, compare, `to_float`, `from_int`
- Create `std/complex.jade`: `Complex(re as f64, im as f64)`
- Operations: add, sub, mul, div, magnitude, phase, conjugate, `exp`, `log`
- Future: extend to `BigRational` backed by BigInt once 3.5 lands

### 3.7 Decimal / Fixed-Point Arithmetic
**Unblocks**: FinTech (regulatory requirement), Blockchain (token amounts)
**Scope**: Exact decimal arithmetic without floating-point error
**Effort**: Medium
**Implementation**:
- Create `std/decimal.jade`: `Decimal` type backed by i128 or BigInt + scale
- Parse: `Decimal.from_string("123.45")`, `Decimal.new(12345, 2)` (value, decimal places)
- Operations: add, sub, mul, div (with rounding mode), compare
- Rounding modes: `ROUND_HALF_UP`, `ROUND_HALF_EVEN` (banker's), `ROUND_DOWN`, `ROUND_CEILING`
- Formatting: `to_string(places)`, `to_currency(symbol, places)`
- Guarantee: `Decimal("0.1") + Decimal("0.2") == Decimal("0.3")` must pass

### 3.8 Operator Overloading for Custom Types
**Unblocks**: Mathematician (Complex + Matrix math), FinTech (Decimal), DSP, HPC
**Scope**: Allow user types to implement `+`, `-`, `*`, `/`, `<`, `>`, `==`
**Effort**: Small — extend existing trait-based operator overloading
**Implementation**:
- Already partially exists: Add/Sub/Mul/Div/Lt/Gt/Le/Ge/Display traits in codegen
- Gap: user-defined impl blocks for these traits don't dispatch to custom methods
- Fix: typer checks for `impl Add for MyType` before falling through to builtin ops
- Desugar `a + b` → `a.add(b)` when Add trait is implemented

## Phase 4 — Developer Experience (Unblocks 14/24 perspectives)

### 4.1 Improved Arg Parsing
**Unblocks**: DevOps, CLI tool builders
**Scope**: Declarative CLI parser
**Effort**: Small
**Implementation**:
- Create `std/cli.jade` with builder pattern:
  ```
  app is Cli 'myapp', '1.0'
  app.flag 'verbose', 'v', 'Enable verbose output'
  app.option 'output', 'o', 'Output file path'
  app.positional 'input', 'Input file'
  parsed is app.parse(args())
  ```
- Handle `--flag=value`, `-f value`, `-abc` (combined short flags)
- Auto-generate `--help` text

### 4.2 Binary Serialization
**Unblocks**: Blockchain, Game (save files), Networking (wire protocols), Database
**Scope**: Encode/decode structs to raw bytes
**Effort**: Medium
**Implementation**:
- Create `std/binary.jade` with `ByteBuffer` type
- `write_u8`, `write_u16_le`, `write_u32_be`, `write_i64_le`, `write_bytes`
- `read_u8`, `read_u16_le`, etc. with cursor position
- Compile-time `@serialize` decorator auto-generates encode/decode for structs
- Endianness control: `le` (little-endian) and `be` (big-endian) suffixes

### 4.3 WASM Compilation Target
**Unblocks**: Web (frontend), Game (browser), Blockchain (smart contracts)
**Scope**: `--target wasm32-wasi` support
**Effort**: Large — requires runtime adaptation
**Implementation**:
- Add wasm32-wasi to cross-compilation targets (Phase 1.4)
- Strip pthreads/mmap from runtime for WASM builds
- Use WASI APIs for filesystem, clock, random
- Export functions as WASM exports with `pub` keyword
- Generate `.wasm` binary instead of ELF/Mach-O

### 4.4 Trait Default Methods
**Unblocks**: Compiler dev, all developers using traits
**Scope**: Allow method bodies in trait definitions
**Effort**: Small — parser/typer change
**Implementation**:
- Allow `*method self` with body inside `trait` blocks
- Typer copies default body into impl if not overridden
- Preserve span information for error reporting

### 4.5 Mmap Exposure
**Unblocks**: Database, OS, Networking (zero-copy), Hacker
**Scope**: Expose mmap/munmap to Jade code
**Effort**: Small — extern bindings
**Implementation**:
- Add `mmap(size, prot, flags)` and `munmap(ptr, size)` to `std/os.jade`
- Prot constants: `PROT_READ`, `PROT_WRITE`, `PROT_EXEC`
- Flag constants: `MAP_PRIVATE`, `MAP_SHARED`, `MAP_ANONYMOUS`
- Return `%i8` (raw pointer to mapped region)

## Phase 5 — Competitive Differentiators (Polish)

### 5.1 SIMD Intrinsics
**Unblocks**: Game, Database (vectorized scans), Data Scientist, HPC, DSP, FinTech
**Scope**: Re-add SIMD type with actual codegen
**Effort**: Medium
**Implementation**:
- Resurrect `SIMD of T, N` type from grammar
- Map to LLVM vector types: `<4 x float>`, `<8 x i32>`
- Expose: `simd_add`, `simd_mul`, `simd_fma`, `simd_shuffle`, `simd_extract`
- Auto-vectorization hint: `@vectorize` on loops

### 5.2 Inline Annotations
**Unblocks**: Embedded (code size), Game (hot path), OS (kernel)
**Scope**: `@inline` and `@noinline` function decorators
**Effort**: Small — set LLVM attributes
**Implementation**:
- Parse `@inline` / `@noinline` / `@cold` / `@hot` before `*fn_name`
- Map to LLVM `alwaysinline`, `noinline`, `cold`, `hot` attributes
- Verify in codegen decl.rs when setting function attributes

### 5.3 Static / Global Variables
**Unblocks**: Embedded, OS, Game (singletons)
**Scope**: Module-level mutable and immutable bindings
**Effort**: Medium
**Implementation**:
- Parse `let NAME is value` at top-level as global constant
- Parse `var NAME is value` at top-level as global mutable
- Codegen as LLVM global variables with appropriate linkage
- Thread safety: `@thread_local` decorator for per-thread globals

### 5.4 REPL / Interpreter Mode
**Unblocks**: Data Scientist, Educator, Mathematician, Automation
**Scope**: `jadec repl` for interactive exploration
**Effort**: Large — requires eval loop over compiler pipeline
**Implementation**:
- Wrap lex → parse → type → compile per-line
- Accumulate declarations across REPL entries
- Use LLVM JIT (OrcJIT) for immediate execution
- Print expression results automatically

### 5.5 Connection Pooling & Keep-Alive HTTP
**Unblocks**: Web, DevOps, Networking, FinTech
**Scope**: Reuse TCP connections across HTTP requests
**Effort**: Small
**Implementation**:
- Add `ConnectionPool` type maintaining open socket map by host:port
- HTTP client checks pool before opening new connection
- `Connection: keep-alive` header by default
- Pool eviction on idle timeout

### 5.6 Glob / File Pattern Matching
**Unblocks**: Automation, DevOps, Build tools
**Scope**: `glob("src/**/*.jade")` returns matching file paths
**Effort**: Small — pure Jade over fs.walk
**Implementation**:
- Create `std/glob.jade` with `glob(pattern) returns Vec of String`
- Support `*` (any file), `**` (recursive), `?` (single char), `[abc]` (char class)
- Implement as filtered `fs.walk()` with pattern matching
- Expose `matches(path, pattern) returns bool` for individual checks

### 5.7 Shebang / Script Mode
**Unblocks**: Automation, Educator, scripting use cases
**Scope**: `jadec run file.jade` for immediate compile-and-execute, plus shebang support
**Effort**: Small
**Implementation**:
- `jadec run file.jade [args]` — compile to temp, execute, delete
- Lexer: skip `#!` line at position 0
- Cache compiled binaries by source hash in `~/.jade/cache/` for instant re-runs
- Unix: `#!/usr/bin/env jadec run` at file top makes .jade files executable

### 5.8 Statistics Library
**Unblocks**: Data Scientist, FinTech, Mathematician, HPC
**Scope**: Common statistical functions
**Effort**: Small — pure Jade
**Implementation**:
- Create `std/stats.jade`
- Descriptive: `mean`, `median`, `mode`, `stdev`, `variance`, `percentile`, `iqr`
- Correlation: `pearson(xs, ys)`, `spearman(xs, ys)`
- Distribution sampling: `normal(rng, mu, sigma)`, `uniform(rng, lo, hi)`, `exponential(rng, lambda)`
- Accumulator: `OnlineStats` type with running mean/variance (Welford's algorithm)

### 5.9 FFT Library
**Unblocks**: DSP, HPC, Mathematician, Data Scientist
**Scope**: Fast Fourier Transform for signal/spectral analysis
**Effort**: Medium — can bind to FFTW or implement Cooley-Tukey in Jade
**Implementation**:
- Create `std/fft.jade`
- `fft(data as Vec of f64) returns Vec of Complex` — radix-2 Cooley-Tukey
- `ifft(freq as Vec of Complex) returns Vec of f64` — inverse FFT
- `magnitude_spectrum(data)`, `power_spectrum(data)` convenience functions
- Real-valued shortcut: `rfft` for purely real input (2× speed)

---

# Summary: Perspective Viability After Full Remediation

| Perspective | Current | After Phase 1 | After Phase 2 | After Phase 3 | After All |
|-------------|---------|---------------|---------------|---------------|-----------|
| Embedded Systems | ✗ | ✗ | ✗ | ~Partial | ~Viable |
| DevOps | ✗ | **Viable** | Comfortable | Comfortable | Strong |
| Data Scientist | ✗ | ✗ | ~Partial | Partial | Partial |
| Indie Game Dev | ✗ | ✗ | ✗ | ✗ | ~Partial |
| Web Developer | ✗ | Partial | **Viable** | Comfortable | Strong |
| Blockchain | ✗ | ✗ | ✗ | **Viable** | Strong |
| Database Dev | Partial | Partial | **Viable** | Strong | Strong |
| Networking/Distributed | ✗ | Partial | Partial | **Viable** | Strong |
| OS Developer | ✗ | ✗ | ✗ | ~Partial | Partial |
| Compiler/PL Dev | Partial | **Viable** | Comfortable | Comfortable | Strong |
| Mathematician | ✗ | ✗ | ✗ | **Viable** | Strong |
| Hacker / Security | ✗ | Partial | **Viable** | Comfortable | Strong |
| Mobile App Dev | ✗ | ✗ | ✗ | ✗ | ✗ |
| HPC / Scientific | ✗ | ✗ | ✗ | Partial | Partial |
| FinTech / Quant | ✗ | ✗ | ✗ | **Viable** | Strong |
| Educator / Student | Partial | Partial | Partial | **Viable** | Strong |
| Audio / DSP | ✗ | ✗ | Partial | Partial | ~Viable |
| Automation / Scripting | ✗ | **Viable** | Comfortable | Comfortable | Strong |
| Aviation / Aerospace | ✗ | ✗ | ✗ | ✗ | ✗† |
| Automotive / Industrial | ✗ | ✗ | ✗ | ✗ | ✗† |
| Robotics / Autonomous | ✗ | ✗ | ✗ | Partial | ~Viable |
| DevSecOps / Cloud-Native | ✗ | Partial | **Viable** | Comfortable | Strong |
| Bioinformatics | ✗ | ✗ | Partial | Partial | ~Viable |
| AI / ML Engineer | ✗ | ✗ | ✗ | ✗ | ✗ |

† Requires formal verification path (see Formal Verification section) — not achievable through stdlib/runtime alone.

### Key Insight
**Phase 1 alone** (subprocess, TLS, DNS, cross-compilation, `?` operator) unblocks DevOps, Compiler, and Automation developers and makes Web and Networking partially viable. These 5 items have the highest ROI.

**Phase 2** (generic collections, HTTP server, SQLite, TOML, CSV, hex/bitwise, byte buffers, string interpolation) makes Web, Database, Hacker, DevSecOps, and Cloud-Native developers productive.

**Phase 3** (event loop, freestanding, crypto, BigInt, rationals, complex, decimals, operator overloading) is the largest phase but unblocks the most specialized audiences: Mathematician, FinTech, Blockchain.

**Three categories of unreachable**:
1. **Platform-locked** (Mobile, AI/ML) — require platform SDK or GPU ecosystem investment that dwarfs language development
2. **Certification-locked** (Aviation, Automotive) — require formal verification and toolchain qualification costing millions
3. **Ecosystem-locked** (Bioinformatics, Robotics) — viable once foundation exists, but need domain-specific libraries

**Jade's strongest near-term positions**: DevOps tooling, automation/scripting, web backends, database engines, and compiler development. The English-like syntax makes it uniquely positioned for education once a REPL exists.

**Formal verification is the long game**: Steps 1–6 from the Formal Verification roadmap (property testing, differential testing, grammar verification, LLVM IR validation, coverage analysis) are achievable in ~3 months and dramatically increase confidence. Full compiler qualification for aviation (DO-330) is a 3–5 year research investment but would make Jade the first Perceus-RC language with a verified compiler — a publishable, fundable outcome.
