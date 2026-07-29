# §21 Findings — P0 (alpha-blocking)

Listed in priority order. Each finding has: short ID, severity,
one-line description, reproducer, root cause, fix sketch, ETA.

---

## P0-1 — Integer division/mod by zero is UB

- **Severity:** Memory-unsafe behaviour on trivial input.
- **Reproducer:** `*main\n    a is 10\n    b is 0\n    log(a / b)`.
  Prints uninitialized stack value (e.g. `140737459297944`).
- **Root cause:** `src/codegen/mir_codegen/helpers/values.rs:121-138`
  emits `build_int_signed_div` / `build_int_unsigned_div` /
  `build_int_signed_rem` / `build_int_unsigned_rem` with **no zero
  check**. A correct `checked_divmod` already exists in
  `src/codegen/arith.rs:249-281` but is only called by the HIR-direct
  path, which is no longer the active pipeline.
- **Fix:** Lift `checked_divmod` to a shared helper; call from
  MIR-codegen. Also handle the second UB case for signed division
  (`INT_MIN / -1`) the same way.
- **ETA:** Half a day.

---

## P0-2 — Vec out-of-bounds is SIGSEGV, not a diagnostic

- **Severity:** Memory-unsafe on trivial input.
- **Reproducer:** `v is [10, 20]; log(v[5])`. Process exits with
  rc=139 (SIGSEGV) and no output.
- **Root cause:** `emit_vec_bounds_check` exists
  (`src/codegen/vec/core.rs:543`) but (a) is not called from every
  access path in `src/codegen/mir_codegen/emit_inst/aggregates.rs`,
  and (b) the fail branch calls `llvm.trap` rather than the
  language's `__jinn_trap("…")` helper, so the user sees a raw
  signal instead of a diagnostic.
- **Fix:**
  1. Audit every aggregate access in `aggregates.rs` and ensure all
     route through `emit_vec_bounds_check`.
  2. Replace `llvm.trap` in the fail branch with `__jinn_trap("array
     index <N> out of bounds (len <M>) at <file>:<line>")`.
- **ETA:** One day.

---

## P0-3 — Compiler emits LLVM IR that fails LLVM's verifier (generators)

- **Severity:** Compiler soundness.
- **Reproducer:** see `review/probes2/p31_generator.jn`. Error:
  ```
  Function return type does not match operand type of return inst!
    ret i64 0
   ptr
  ```
- **Root cause:** Generator / `yield` lowering chooses an
  inconsistent return type (`ptr` declared, `i64 0` returned).
  Likely in `src/codegen/coroutines.rs` or the generator splitting
  inside `src/mir/lower/`.
- **Fix:** Read the lowering, make the function's declared return
  type and the actual return value agree. Add an MIR verifier
  invariant (see P0-7) that catches this at MIR construction.
- **ETA:** Two days.

---

## P0-4 — `map(v, $ * 2)` ICEs the compiler or crashes at runtime

- **Severity:** First-hour user code crashes; no diagnostic.
- **Reproducer:** `v is [1,2,3]; d is map(v, $ * 2); log(d[0])`.
  Probe v1 panics in `src/codegen/mir_codegen/helpers/values.rs:96`
  (`into_int_value()` on a `PointerValue`); probe v2 exits with rc=16.
- **Root cause:** Typer infers the closure's implicit `$` parameter
  type as `Ptr<i64>` (the slot pointer the codegen passes in) rather
  than `i64` (the element). The integer multiplication then receives
  a pointer operand.
- **Fix:** Closure-over-vec inference must dereference the element
  type. Add a typer test (`map(vec_of_i64, $ + 1)`) and a MIR
  verifier check (P0-7) that a `BinOp::Mul`'s operands are not
  pointers.
- **ETA:** Two-to-three days (touches typer + MIR + codegen + tests).

---

## P0-5 — `take` does not parse in function-call argument position

- **Severity:** The borrow-check / ownership story is unreachable
  from user code.
- **Reproducer:** `consume(take v)` → `expected ,, got v`.
- **Root cause:** `src/parser/expr.rs` does not accept the `take`
  keyword as the start of an argument expression.
- **Fix:** Add `take EXPR` to the expression precedence ladder at
  the right level (unary, same as `not`).
- **ETA:** Half a day.

---

## P0-6 — Stack overflow is silent SIGSEGV

- **Severity:** Common user mistake (`*loop_forever(n) returns i64; return loop_forever(n+1)`) → silent failure.
- **Reproducer:** `review/probes2/p40_inf_recur.jn`. Prints garbage,
  exits rc=0 (!?).
- **Root cause:** 64 KB coroutine stack + 4 KB guard, no SIGSEGV
  handler that recognises guard-page hits and produces a diagnostic.
  The garbage-print + clean exit is even worse than a SIGSEGV — it
  suggests the runtime is reading past stack into adjacent data.
- **Fix:** Install a `sigaction(SIGSEGV)` handler in runtime startup
  that checks `siginfo->si_addr` against the active coroutine's guard
  region and prints "coroutine stack overflow at …" before re-raising
  the default action.
- **ETA:** One day.

---

## P0-7 — MIR has no internal verifier

- **Severity:** Code-quality / soundness; enables P0-1, P0-3, P0-4
  to escape.
- **Reproducer:** Source comment in
  `src/codegen/mir_codegen/helpers/values.rs:91-118`:
  > Auto-widen mismatched integer widths to the wider operand.
  > Required because MIR currently lets `i32 << i64` reach codegen
  > unaltered.
- **Root cause:** No `mir::verify` pass runs after lowering or
  Perceus.
- **Fix:** Add a `src/mir/verify.rs` that walks every function and
  asserts: every instruction's operand types match the operator's
  signature; every block ends in a terminator; every used value
  dominates its uses; every function's declared return type matches
  its return-terminator operands. Run it under `debug_assertions`
  unconditionally and in CI as a hard gate.
- **ETA:** Two days; *deletes* the auto-widen patch (negative LOC
  net).

---

## P0-8 — Bare top-level expression compiles to a program with that exit code

- **Severity:** Silent acceptance of malformed input.
- **Reproducer:** A file containing just `42` → compiles, exits 42.
- **Root cause:** `parser/decl.rs` accepts an expression as a
  top-level decl.
- **Fix:** Reject; emit "expected `*function`, `type`, `store`,
  `actor`, `use`, or `NAME is value` at top level".
- **ETA:** Half a day.

---

## P0-9 — Missing `main` is a linker error

- **Severity:** Confusing user-visible diagnostic.
- **Reproducer:** A file with no `*main` → `ld: undefined reference
  to main`.
- **Root cause:** `src/driver/pipeline.rs` invokes the linker
  without first checking whether the typer registered a `main` def.
- **Fix:** Before link, look up the `main` symbol in the compiled
  module. If absent (and not `--lib` / `--standalone`-without-main),
  emit "error: program has no `*main` function (add `*main\n    ...`
  or pass `--lib`)".
- **ETA:** Half a day.

---

## P0-10 — `.send()`, `.close()`, etc. don't parse (lexer keywords shadow methods)

- **Severity:** Common user code does not compile.
- **Reproducer:** `ch.send(1)` → `expected identifier, got send`.
- **Root cause:** The KEYWORDS table in `src/lexer/mod.rs` promotes
  ≈ 80 words to keywords regardless of context. After a `.` token,
  every keyword in the table is unusable as a method name.
- **Fix:** In the lexer's identifier path (or in the parser at the
  `.` postfix), do not promote identifiers to keywords when the
  previous token was `Dot`. Standard contextual-keyword pattern.
- **ETA:** Half a day.

---

## P0-11 — No fuzzer; no sanitizer runs in CI

- **Severity:** Process-level. Alpha cannot ship without these.
- **Fix:** `cargo-fuzz` harness for lexer + parser; ASan + TSan +
  UBSan builds of the runtime exercised by the existing test suite;
  CI job that runs both.
- **ETA:** Three to five days.

---

## P0-12 — `Type::String` vs `Type::Struct("string")` asymmetry

- **Severity:** Latent in lots of code; one path away from many
  surprise type errors.
- **Root cause:** Two representations of the same type, no
  canonicalisation.
- **Fix:** Canonicalise at the typer entry points (`unify`,
  `subtype`); add a `Type::canonical()` and call from comparison
  sites; add a debug-assert in `Type::eq` that both sides are
  canonical.
- **ETA:** One day.

---

# §22 Findings — P1 (must-have for alpha, can ship soon-after)

These are quality bars that a polished alpha needs but a *very*
ambitious team could defer to 0.1.x if forced:

| ID | One-liner | Where |
| -- | --------- | ----- |
| P1-1 | Tuple-return type `*f() returns (T,U)` does not parse | parser/decl |
| P1-2 | Match guards `pattern if expr ?` do not parse | parser/expr |
| P1-3 | SIMD literal syntax not implemented | parser/expr + codegen |
| P1-4 | UTF-8 output truncation in `log()` | runtime/util + lexer/literals |
| P1-5 | `hir-validate` is the working type checker; typer leaks | typer/ |
| P1-6 | `eprintln!`-style debug lines leak in non-debug runs (e.g. `mir-perceus: …`) | mir_codegen + driver |
| P1-7 | Generator runtime function emits invalid IR (subsumed by P0-3) | – |
| P1-8 | 159 compiler warnings on release build | everywhere |
| P1-9 | Old HIR-direct codegen path is dead-ish; remove it or finish migration | src/codegen |
| P1-10 | Old `PerceusPass` HIR-level shim is dead; delete | src/perceus |
| P1-11 | LSP coverage matrix undocumented and unmeasured | src/lsp + vscode-jinn |
| P1-12 | `pool_allocs` Perceus field computed but never consumed | src/mir, src/codegen |
| P1-13 | EBNF and parser drift; codify "parser matches grammar" tests | jinn.ebnf + tests/ |
| P1-14 | WAL crash recovery has 3 tests; needs randomised property tests | tests/wal_crash |
| P1-15 | Channel spinlocks not stress-tested under contention | runtime/channel.c |
| P1-16 | Cancellation / shutdown semantics for actors undocumented | runtime/sched.c + docs |
| P1-17 | `apps/*/project.jn` is the entry, but `jinnc PATH/project.jn` treats it as source — should detect manifest | src/driver |
| P1-18 | Implicit copy of non-trivials is undocumented (Rc-bump? clone?) | typer + docs |
| P1-19 | Reassign with different type error message uses "I64" not "i64" | typer/diagnostics |
| P1-20 | `OwnershipDiag::ReturnOfBorrowed` exists but no probe triggers it; verify reachable | ownership/ + tests/ |
| P1-21 | Two parallel codegen paths (HIR-direct vs MIR) | src/codegen |
| P1-22 | `setjmp` fallback in `jinn_rt.h` for unknown arches is broken; should refuse to build | runtime/jinn_rt.h |

---

# §23 Findings — P2/P3 (polish, post-alpha)

| ID | One-liner |
| -- | --------- |
| P2-1 | `//` and `/* … */` should be supported as deprecated aliases for `#` |
| P2-2 | No raw / heredoc / triple-quoted strings |
| P2-3 | Parser 20-error cap is silent; show "(N more errors suppressed)" |
| P2-4 | `take` desugaring should be its own pass, not co-located in parser |
| P2-5 | No `Validated<Hir>` newtype to enforce HIR-validation invariant |
| P2-6 | Ownership enum is shallow; per-projection ownership needed for beta |
| P2-7 | `Type` enum has ≈ 20 variants; some are likely collapsible |
| P2-8 | No `jinnc --explain perceus FILE` for onboarding |
| P2-9 | Runtime ABI undocumented; add `runtime/ABI.md` |
| P2-10 | Stdlib two-tier with no documented contract |
| P2-11 | UTF-8 graphemes vs bytes story missing |
| P2-12 | No formatter conformance test |
| P2-13 | `tree-sitter-jinn` may drift from parser |
| P2-14 | No `tokio-console`-equivalent for live coroutine/channel inspection |
| P2-15 | LTO not on by default |
| P2-16 | `panic!` hotspots in `src/escape/mod.rs` should become diagnostics |
| P2-17 | `apps/` and `examples/` overlap in purpose; pick one |
| P2-18 | `benchmarks/results*.json` files should not live in source tree (move to `target/` or `.benchresults/`) |
| P2-19 | `f.jn`, `hello.jn`, `ok` etc. at repo root are scratch files; move out |
| P2-20 | Several memory note files in `/memories/repo/` describe bugs that may now be fixed; cross-check |
| P3-1 | Replace 404 `.expect()` calls with diagnostics where the invariant comes from user input |
| P3-2 | Migrate 110 `.unwrap()` to `.expect(...)` with explanatory messages |
| P3-3 | Add CODEOWNERS / CONTRIBUTING for each major area |
| P3-4 | The "perspectives" / "the-way-of-jinn" / "SNIPPET" docs at repo root mix design notes with marketing; curate |
