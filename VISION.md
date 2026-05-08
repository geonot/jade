# Jinn — Vision Roadmap

A forward-looking design document for the features, syntax, sugar, tooling, and
ecosystem moves that would make Jinn the quintessential modern systems +
applications language. Each item is grounded in (a) what Jinn already promises
(C performance, Python readability, Erlang-class concurrency, built-in
persistence), (b) what's empirically missing as of the May 2026 audit, and (c)
what each developer archetype needs to choose Jinn over the incumbent.

Items are tagged by **archetype** — Systems (`SYS`), Application (`APP`),
Data/Scientific (`DAT`), Web/Backend (`WEB`), Embedded (`EMB`), Game/Realtime
(`RT`), Distributed (`DIST`), Beginner (`BEG`).

Sized in t-shirt units, oriented from "low-hanging signature wins" through
"strategic bets".

---

## V.0 Design Principles

These constrain every item below.

1. **One way to do it.** Reject features that duplicate an existing capability
   without replacing it.
2. **No syntactic novelty without semantic justification.** New syntax must
   reflect a new compile-time guarantee or runtime behavior, not just sugar
   that hides familiar primitives.
3. **Cost transparency.** A feature that allocates, or might block, or might
   panic — surface it in the syntax. Rust's `?` is the canonical example.
4. **Failure is a value, not a control flow.** Lean on `Result`/`Option` and
   structured supervision; reserve panics for invariants.
5. **Compile-time over runtime.** Anything provable at compile time should not
   pay runtime cost. Const-eval and comptime are first-class.
6. **The standard library is the brand.** A great stdlib is the cheapest way
   to make a language feel ten years older than it is.
7. **Tooling is part of the language.** Formatter, LSP, debugger,
   profiler, package manager are not afterthoughts.

---

## V.1 Type System & Semantics

### V1.1 Sum types with payload patterns done right `[APP][SYS][BEG]` — M
- **Today:** `enum` with named cases works; pattern matching exists.
- **Add:**
  - Pattern guards: `case Some(x) if x > 0 -> ...`
  - Or-patterns: `case Red | Blue -> ...`
  - Range patterns: `case 1..10 -> ...`
  - Binding patterns: `case point as Point(x, y) -> ...`
  - Exhaustiveness checking with helpful diagnostics: "missing case `None`,
    cases handled: `Some(_)`".
- **Why:** Pattern matching is the most-used syntactic feature in modern
  languages; if Jinn's is even slightly worse than Rust's, users notice
  immediately.

### V1.2 Traits / typeclasses, real ones `[APP][SYS]` — L
- **Today:** the typer mentions `TypeConstraint::Trait(Vec<String>)` and trait
  impls are checked, but trait declarations and dispatch are not the
  centerpiece of the type system.
- **Add:**
  - `trait Comparable<T> { *cmp self, other as T returns Ordering }`
  - `impl Comparable<i64> for MyType { ... }`
  - Default methods, associated types, blanket impls.
  - Type-class style monomorphization (Rust model) — no v-tables unless
    requested via `dyn Trait`.
- **Why:** without traits, Jinn can't express generic algorithms cleanly. The
  HM core is already there; lifting it to Hindley-Milner-with-typeclasses
  (System F-ω lite) is a known design.

### V1.3 Lifetime-free borrow tracking `[SYS][APP]` — XL
- **Today:** Perceus refcounting handles ownership transparently; no lifetimes.
- **Add:** static "borrow shape" analysis without surface lifetime annotations.
  Reuse existing ownership analysis ([src/ownership.rs](src/ownership.rs)) to
  prove most cases at compile time and elide refcount ops; fall back to RC
  only where the analysis can't decide.
- **Why:** keeps the Python-readable surface while approaching Rust's
  zero-overhead aspiration. This is the single most ambitious item in the
  document.

### V1.4 Dependent enums (refinement types lite) `[SYS][DAT]` — L
- `enum Vec<T, N as i64> where N >= 0` — length tracked in the type.
- `*head v as Vec<T, N> where N > 0 returns T`
- Enables compile-time bounds checking for hot loops.

### V1.5 Effects in the type signature `[APP][DIST]` — XL
- `*read_file path returns String !io`
- `*compute_hash data returns Hash !pure`
- Effects: `!io`, `!alloc`, `!await`, `!panic`, user-defined.
- Effects propagate; `!pure` functions can be const-evaluated, parallelized,
  memoized automatically.

### V1.6 First-class `Option`/`Result` with `?` propagation `[APP][BEG]` — S
- `let x = maybe_get()?` returns early on `None`/`Err`.
- Today this is implicit / awkward.

### V1.7 Const generics & const evaluation `[SYS][DAT]` — M
- `*matmul a as Mat<R, C>, b as Mat<C, K> returns Mat<R, K>`
- `comptime` already exists ([src/comptime.rs](src/comptime.rs#L1)) — promote
  to a first-class feature with a clear const-eval interpreter.

---

## V.2 Concurrency, Distribution, Resilience

### V2.1 Real supervision trees `[DIST][WEB]` — L
- See defect log N-3.
- **Promote to** `supervisor` blocks producing typed handles:
  ```
  let sup = supervisor MySupervisor
      strategy is one_for_one
      max_restarts is 3 in 60s
      children
          Counter(initial is 0) as counter
          Logger() as logger

  send sup.counter, @inc
  ```
- Runtime: link/monitor primitives, supervisor process model, restart
  intensity bounded by token bucket.

### V2.2 Selective receive `[DIST]` — M
- Erlang's killer feature for protocols.
- `receive case @ack(id) when id == 7 -> ... case @timeout -> ...`
- Mailbox is iterated, non-matching messages are saved.

### V2.3 Distributed actors `[DIST]` — XL
- `let pid = spawn @"node@host" Counter()`
- Network transport over TLS-by-default; serialization via existing codec
  framework; node-up/node-down monitors.
- Designed for in-cluster trust, not adversarial networks.

### V2.4 Structured concurrency `[APP][BEG]` — M
- `concurrent { let a = spawn f(); let b = spawn g(); }` — block waits for
  all children, propagates first failure, cancels siblings.
- Cancellation tokens propagate through `await`/`<-` boundaries.

### V2.5 Async/await over coroutines `[WEB][APP]` — M
- Today: explicit channels and `select`. Powerful but verbose for typical
  request/response.
- Add: `await` keyword desugaring to channel-recv on a one-shot reply
  channel; integrates with structured concurrency.

### V2.6 Backpressure-first channels `[DIST][WEB]` — S
- Default channels are bounded; unbounded must be opt-in
  (`channel<T>(unbounded)`).
- Send on full bounded channel suspends; `try_send` returns `Result`.

### V2.7 Deterministic test mode `[DIST][APP]` — M
- `jinn test --deterministic`: scheduler runs in a single thread with a
  reproducible task ordering seeded from CLI; channels record their
  receive order; reruns match exactly.
- Critical for actor-system testing.

---

## V.3 Persistence & Data

### V3.1 Honest WAL with group commit `[DIST][WEB]` — covered in defect log R1.

### V3.2 Schema migrations as code `[WEB][APP]` — M
- `migrate users at version 2 from version 1 ...`
- Today: [runtime/migrate.c](runtime/migrate.c) exists; surface it with
  language-level migration declarations checked against schema diff.

### V3.3 Indexes as part of the type `[WEB][DAT]` — M
- `store users name as String<=64 indexed, age as i64, email as String<=128 unique indexed`
- Index plan visible at compile time; queries that would full-scan emit a
  warning.

### V3.4 Query optimizer with EXPLAIN `[WEB][DAT]` — L
- After R12 makes `query` blocks executable, add a trivial cost-based
  optimizer that picks index vs scan; `EXPLAIN <query>` prints the chosen
  plan.

### V3.5 First-class transactions `[WEB][DIST]` — M
- `transaction { insert users (...); update accounts where id == n set balance = balance - amount }`
- ACID over the WAL; deadlock detector for multi-store.

### V3.6 Streaming results `[DAT][WEB]` — S
- `for row in stream users where age > 30 { ... }` — pipelined; record
  decoded only when consumed.

### V3.7 Time-travel queries `[DAT][WEB]` — L
- WAL is append-only; expose `as of <timestamp>`:
  `users as of "2026-04-01" where ...`
- Historical analytics for free.

### V3.8 Built-in dataframe ↔ store interop `[DAT]` — S
- `df is dataframe.from_store(users where age > 30)`
- And the reverse: `df.to_store(users)`.

---

## V.4 Performance & Systems Capability

### V4.1 SIMD as first-class types `[SYS][DAT][RT]` — M
- `f64x4`, `i32x8`, etc., with arithmetic operators that auto-lower to
  LLVM `<N x T>` IR.
- Vectorized stdlib for `dot`, `axpy`, `mean`, `variance`.

### V4.2 GPU offload `[DAT][RT]` — XL
- `gpu kernel matmul(a, b, c) { ... }` — compiled to PTX/SPIR-V via LLVM.
- Memory transfer explicit; enforces cost transparency principle.

### V4.3 Profile-guided optimization `[SYS]` — M
- `jinn build --pgo-collect` then `--pgo-use`.
- Reuses LLVM PGO machinery.

### V4.4 No-allocation mode for hot paths `[RT][EMB]` — M
- `*tick #[no_alloc] returns ()` — compiler errors on any code path that
  allocates.
- Enables Jinn in real-time game loops, audio callbacks, embedded ISRs.

### V4.5 Inline assembly with verified clobbers `[SYS][EMB]` — M
- Today: `asm` parses; codegen unverified.
- Add: full LLVM `InlineAsm` integration with explicit input/output/clobber
  declarations checked against the constraint string.

### V4.6 `#[link_section]` and freestanding builds `[EMB][SYS]` — M
- `--target thumbv7m-none-eabi --no-runtime` for bare-metal.
- Provide `runtime/freestanding.c` with stubs.

### V4.7 Stack size declaration on coroutines `[RT][EMB]` — S
- `spawn #[stack(64k)] heavy_compute()`.

---

## V.5 Tooling

### V5.1 LSP at parity with rust-analyzer `[ALL]` — L
- Today: 1,616 LOC of LSP exists ([src/lsp/](src/lsp/)). Status of each
  capability unverified.
- **Required:** hover with type, go-to-def, find-refs, rename, completion
  (with type-aware suggestions), inlay hints (inferred types, lifetime
  elisions), code actions (quick fixes for typer suggestions), workspace
  symbols, document outline, semantic tokens.

### V5.2 Native debugger via DWARF `[ALL]` — covered in roadmap R15.

### V5.3 Built-in profiler `[SYS][RT]` — M
- `jinn run --profile` writes a flame graph compatible with Brendan Gregg's
  tooling.
- Sampling profiler in the runtime; symbol resolution via DWARF.

### V5.4 `jinn fmt` is a hard requirement `[ALL]` — S
- Today: [src/fmt.rs](src/fmt.rs) exists; verify it's complete and
  idempotent; CI gate.

### V5.5 `jinn fix` for compiler suggestions `[BEG][APP]` — M
- Auto-applies `suggest_fix` outputs from typer (e.g., `use \`x as i64\` to
  convert`).

### V5.6 Package manager `jinn pkg` `[ALL]` — L
- Today: [src/pkg.rs](src/pkg.rs) exists. Need:
  - `jinn pkg add <name>` resolves and installs.
  - Lockfile with content hash.
  - Registry (cargo-style) or git-based (Go-style) — pick one, ship.
  - SBOM emission for security archetypes.

### V5.7 REPL `[BEG][DAT]` — M
- `jinn repl` — incremental compile, persistent variable bindings, multiline
  editing.

### V5.8 Notebook integration `[DAT][BEG]` — M
- Jupyter kernel using the REPL backend.

### V5.9 `jinn doc` with hosted docs `[ALL]` — M
- Extract `///` doc comments, render to static site.
- Embed runnable examples that the test suite verifies.

### V5.10 `jinn bench` first-class `[SYS][RT]` — S
- `*bench tight_loop { ... }` — recognized and compiled with PGO,
  iteration count auto-tuned.

---

## V.6 Standard Library

### V6.1 Audit & strengthen the 40 modules `[ALL]` — covered in §C.10.

### V6.2 Missing modules to add `[ALL]` — M each
- `std.url` — URL parser/builder (curl-grade).
- `std.uuid` — v4 + v7.
- `std.uri_template` — RFC 6570.
- `std.process` — cross-platform spawn + pipe.
- `std.signal` — POSIX signal handling.
- `std.path` — cross-platform path manipulation.
- `std.env` — environment variables, args.
- `std.term` — ANSI colors, raw mode, cursor.
- `std.log` — structured logging with levels and sinks.
- `std.metrics` — counters, gauges, histograms; Prometheus export.
- `std.tracing` — distributed tracing (W3C Trace Context).
- `std.test` — assertion library, table-driven tests, snapshot tests,
  benchmarks (already partial via `*bench`).

### V6.3 Time done right `[ALL]` — M
- `Instant`, `Duration`, `DateTime<TZ>` distinct types.
- Monotonic vs wall clock distinction enforced by type.
- Calendar arithmetic that handles DST.

### V6.4 i18n `[APP][WEB]` — M
- Unicode-aware string ops (graphemes, normalization, case folding) by
  default; byte ops require explicit cast.
- ICU integration for collation/locale-aware formatting.

### V6.5 Cryptography surface `[WEB][SYS]` — M
- Today: `std/crypto.jn` 239 LOC. Verify covers: hashes (SHA2/3, BLAKE3),
  HMAC, AEAD (ChaCha20-Poly1305, AES-GCM), KDF (Argon2id, scrypt), Ed25519,
  X25519, secure random, constant-time compare.

### V6.6 Stdlib for distributed systems `[DIST]` — L
- Raft / Paxos primitives, leader election, distributed locks (etcd-style).
- Build atop V2.3 distributed actors.

---

## V.7 Syntax Sugar (carefully)

Each of these is a small, targeted convenience that pulls weight.

### V7.1 String interpolation with format specifiers `[ALL]` — S
- `"Hello, {name}! You have {count:>5} messages."`
- Compiler-checked: `{name}` requires `name` in scope; format spec checked
  against type.

### V7.2 Multi-line string with auto-dedent `[APP][BEG]` — S
- ```
  let html = """
      <html>
          <body>{content}</body>
      </html>
  """
  ```

### V7.3 Trailing closures `[APP][BEG]` — S
- `numbers.map { it * 2 }` instead of `numbers.map(*x { x * 2 })`.

### V7.4 Pipe operator `[DAT][APP]` — S
- `data |> filter(...) |> map(...) |> collect`.

### V7.5 Tuple destructuring everywhere `[ALL]` — S
- `let (a, b) = pair`
- `for (k, v) in map { ... }`

### V7.6 Spread / rest `[APP]` — S
- `let new_v = [head, ..tail]`
- `*log args... { ... }` — variadic.

### V7.7 Numeric literals with separators & types `[ALL]` — S
- `1_000_000`, `0xff_aa`, `0b1010_0101`, `3.14_f32`, `100_u8`.

### V7.8 Postfix `if`/`unless` `[BEG][APP]` — S
- `return early if condition`
- `cleanup() unless aborted`

### V7.9 Implicit return on last expression `[ALL]` — S
- Already exists in many forms; codify and document.

### V7.10 Method-call syntax for free functions `[ALL]` — M
- `value.transform()` desugars to `transform(value)` if `transform` is in
  scope and first param matches.
- Universal Function Call Syntax (Rust/D inspired).

### V7.11 `with` blocks for resource management `[APP][SYS]` — S
- ```
  with open("f.txt") as f
      f.read()
  # f closed even on panic
  ```
- Desugars to existing drop semantics.

---

## V.8 Ecosystem & Community

### V8.1 Public registry & docs site `[ALL]` — L
- `pkg.jn.dev` with searchable index, versioning, ownership.
- `docs.jn.dev` auto-built from any published package.

### V8.2 Editor support beyond VS Code `[ALL]` — M each
- Today: `vscode-jinn/` and `tree-sitter-jinn/` exist.
- Add: Neovim (LSP + tree-sitter), Emacs (eglot + tree-sitter), Zed,
  IntelliJ.

### V8.3 Online playground `[ALL]` — M
- Browser-based: edit, compile (server-side or wasm-jinn), run, share by URL.

### V8.4 First-party reference applications `[ALL]` — L
- A web framework, a TUI framework, a small game (verifying RT archetype),
  a numerical workbook (verifying DAT archetype), a CLI tool template.
- These prove the language can do real work and seed user habits.

### V8.5 Style guide & idiom collection `[ALL]` — M
- "Effective Jinn" — patterns, anti-patterns, performance tips.

---

## V.9 Archetype Coverage Matrix

What does each archetype need *today* to choose Jinn?

| Archetype | Top-3 must-have | Strategic |
|---|---|---|
| **Systems (`SYS`)** | V1.3 borrow tracking · V4.1 SIMD · V5.10 bench | V4.5 asm · V4.6 freestanding |
| **Application (`APP`)** | V1.6 `?` · V7.1 interpolation · V5.5 jinn fix | V1.5 effects |
| **Data/Scientific (`DAT`)** | V5.7 REPL · V5.8 notebooks · V3.8 dataframe↔store | V4.2 GPU |
| **Web/Backend (`WEB`)** | V3.5 transactions · V3.4 EXPLAIN · V2.5 await | V2.3 distributed actors |
| **Embedded (`EMB`)** | V4.4 no_alloc · V4.6 freestanding · V4.7 stack size | V4.5 asm |
| **Game/Realtime (`RT`)** | V4.4 no_alloc · V4.1 SIMD · V5.3 profiler | V4.2 GPU |
| **Distributed (`DIST`)** | V2.1 supervisors · V2.2 selective recv · V2.7 deterministic | V2.3 dist actors · V6.6 raft |
| **Beginner (`BEG`)** | V5.5 jinn fix · V5.7 REPL · V7.1 interpolation | V8.4 reference apps |

---

## V.10 Sequencing Strategy

A pragmatic order across two years:

**Year 1 — Trust**
- Q1: defect-log P0s (separate ROADMAP doc) + V1.1 + V1.6 + V7.1 + V7.4 + V7.7 — quick wins users feel.
- Q2: V2.1 supervisors real + V2.4 structured concurrency + V2.6 backpressure + V5.1 LSP push.
- Q3: V3.1–V3.5 persistence honesty + V5.4 fmt + V5.6 pkg manager.
- Q4: V1.2 traits + V6.2 missing stdlib modules + V8.4 reference apps.

**Year 2 — Lead**
- Q5: V1.7 const generics + V4.1 SIMD + V5.7 REPL.
- Q6: V2.2 selective recv + V2.5 async/await + V2.7 deterministic test.
- Q7: V1.5 effects + V3.7 time-travel queries.
- Q8: V1.3 borrow tracking + V2.3 distributed actors.

This sequence front-loads developer-trust items (correctness, durability,
ergonomics, tooling) and defers research-flavored bets (effects, lifetime
elision, distributed actors) until the foundation is unimpeachable.

---

## V.11 Anti-features (explicitly out of scope)

To preserve focus, these are intentionally rejected:

- **Macros.** Compile-time meta-programming via `comptime` is enough; Lisp/Rust-style macros are a complexity cliff and an LSP nightmare.
- **Inheritance.** Composition + traits cover every legitimate use case.
- **Implicit conversions.** Even between numerics — explicit `as` is the rule.
- **Two ways to spell `nil`.** `None` only; no `null`, no `nil`, no
  zero-value-as-absence.
- **Exceptions.** `Result`/`Option` + supervision tree only.
- **Multiple return values.** Use a tuple or a record type; one return value.
- **A garbage collector.** Perceus + V1.3 borrow tracking is the answer.
- **C++-style overloading.** Traits provide ad-hoc polymorphism cleanly.
- **Color-coding async.** `await` is sugar over coroutines that already
  pervade the runtime; no `async fn` infection.

---

## Appendix — How to read this document

- **Defect log → ROADMAP.md** — what must be fixed.
- **CLEANUP.md** — how the existing code becomes a model citizen.
- **VISION.md (this file)** — what the language becomes once the foundation
  is honest.

The three documents are sequenced: ship ROADMAP first, refactor with CLEANUP
second, build VISION on the resulting foundation. Skipping the first two and
sprinting to the third is how good languages turn into bloated ones.
