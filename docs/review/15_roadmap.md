# §24 Roadmap to alpha

Three-phase plan. Each phase is internally orderable in parallel
across two engineers; the inter-phase dependencies are spelled out.

The numbering matches the finding IDs in §21–§23 so each task in
this plan is traceable to a concrete bug + reproducer.

---

## Phase A — Safety floor (P0 sweep)

**Goal:** After Phase A, no one-line Jinn program produces undefined
behaviour, ICE, or invalid LLVM IR. Every safety violation produces a
diagnostic with file:line.

**Order matters within A** — fix in this sequence to avoid rework:

### A.1 — MIR verifier (P0-7) — **blocks A.2, A.3, A.4**

- New file: `src/mir/verify.rs`.
- Invariants enforced:
  - Every block has a terminator.
  - Every used value has a dominating definition.
  - Operand types match operator signature (`Add`/`Sub`/`Mul`/`Div`
    over matching integer or float widths; `Shl`/`Shr` over matching
    integer widths; etc.).
  - No pointer operand to integer arithmetic.
  - Declared function return type matches every `Return` terminator's
    operand type.
- Run on every function after MIR lowering and after each Perceus
  pass.
- Under `debug_assertions`, panic with a high-detail message; in
  release, downgrade to a `tracing::error!` + abort (so CI catches
  it but customers don't see a `panic`).
- Add `tests/mir_verifier.rs` with: hand-crafted broken MIRs +
  full corpus replays.

### A.2 — Push types up to the typer (P0-12, P1-5)

- Canonicalise `Type::String` ↔ `Type::Struct("string")` at the
  typer's unification entry. Add `Type::canonical()`. Audit every
  `==` on `Type` and replace with `Type::eq_canonical()`.
- Make the typer the single source of truth for type correctness.
  HIR-validate should reduce to invariant checks, not type checks.
- Insert explicit `Cast` HIR/MIR nodes for any mixed-width int
  arithmetic; never let `i32 << i64` reach codegen.
- After this, `helpers/values.rs:91-118`'s auto-widen block can be
  deleted (negative LOC win).

### A.3 — Fix the three concrete codegen bugs

- **A.3a (P0-1):** Lift `checked_divmod` to a shared helper; call
  from `mir_codegen::helpers::values`. Add `INT_MIN / -1` case.
- **A.3b (P0-2):** Audit every aggregate access in `aggregates.rs`
  for bounds-check coverage; replace `llvm.trap` with
  `__jinn_trap("array index N out of bounds (len M) at FILE:LINE")`.
- **A.3c (P0-3):** Read `src/codegen/coroutines.rs` + `mir/lower/`
  generator path; align declared return type with actual return
  value. Add a test that runs `p31_generator.jn`.

### A.4 — Closure inference (P0-4)

- Typer rule for `map(v, |elem| body)` / `map(v, $ * 2)`: the
  closure's parameter type equals the vec's element type, not the
  slot pointer.
- Probe `map(v, $ * 2)` must compile and run.
- Same for `filter`, `reduce`, `for_each`, any HOF that destructures
  through a slot.

### A.5 — Parser holes (P0-5, P0-8, P0-9, P0-10)

- **P0-5:** Add `take EXPR` to the unary tier of the expression
  grammar.
- **P0-8:** Reject top-level expressions; emit specific error.
- **P0-9:** Driver checks for `main` symbol before linking; emits
  "program has no `*main` function".
- **P0-10:** Don't promote keywords after `Dot`. Audit affected
  identifiers (send, close, count, query, view, set, insert, delete,
  …) — they should all work as method names.

### A.6 — Stack overflow diagnostic (P0-6)

- In `runtime/sched.c` startup, install a `sigaction(SIGSEGV)`
  handler that checks `siginfo->si_addr` against the current
  coroutine's guard region.
- Print `coroutine stack overflow at FILE:LINE (stack size 64KB; consider increasing JINN_STACK_SIZE or refactoring recursion)`.
- Then restore default and re-raise.

### A.7 — Fuzz + sanitize CI (P0-11)

- `fuzz/lexer/` (cargo-fuzz harness): random bytes → no panic; only
  Ok or LexError.
- `fuzz/parser/` (cargo-fuzz harness): random tokens → no panic.
- `fuzz/roundtrip/` (cargo-fuzz harness): `parse(print(parse(SRC)))
  == parse(SRC)` for any SRC that parses.
- `ci/sanitize.sh`: build runtime + linked tests with `-fsanitize=address,undefined`
  and again with `-fsanitize=thread`; run `cargo test --release`.

**Phase A exit criterion:** all probes in `review/probes2/` either
compile + run correctly, or fail with a clean diagnostic (not
SIGSEGV / SIGILL / garbage stdout). Fuzz CI runs for 30 min with no
new panics.

---

## Phase B — Polish (P1 sweep)

**Goal:** A user can write substantial code without hitting holes in
the surface syntax or seeing internal-looking output.

- **B.1 (P1-1, P1-2, P1-3):** Implement tuple return types, match
  guards, SIMD literal in parser + lowering. Each backed by tests
  in `tests/programs/`.
- **B.2 (P1-4):** UTF-8 correctness pass over `runtime/util.c`'s
  `jinn_log` family; fix emoji truncation.
- **B.3 (P1-6):** Audit diagnostic prefixes; route all internal info
  through `tracing` and behind `--verbose` / `--debug-*` flags.
- **B.4 (P1-8):** `cargo fix --all-features --lib --tests --bins`
  then enable `-D warnings` in CI.
- **B.5 (P1-9, P1-10, P1-21):** Delete the HIR-direct codegen path;
  delete dead `PerceusPass` shim; aim for codegen ≤ 20 kLOC.
- **B.6 (P1-11):** Author an LSP feature matrix and a smoke-test
  driver. Coverage: completion, hover, go-to-def, find-refs, rename,
  diagnostics. Document in `docs/lsp.md`.
- **B.7 (P1-12):** Wire up `pool_allocs` or remove it.
- **B.8 (P1-13):** Add a CI job that parses `jinn.ebnf` and round-trips
  through `parser/` and `tree-sitter-jinn/`; flag drift.
- **B.9 (P1-14):** Property-test the WAL: random ops, random crash
  points, post-recovery invariant check. Run for 5 minutes in CI.
- **B.10 (P1-15):** Channel stress test: N producers × M consumers,
  unbounded vs bounded, measure tail latency, run under TSan.
- **B.11 (P1-16):** Document actor / channel shutdown semantics in
  `docs/concurrency.md`; back with tests.
- **B.12 (P1-17):** `jinnc PATH/` should detect `PATH/project.jn`
  and switch to project mode. `jinnc PATH/project.jn` should also.
- **B.13 (P1-18):** Document implicit copy / Rc-bump rules in
  `docs/access-semantics.md`; back with tests for each non-trivial
  type.
- **B.14 (P1-19):** Pretty-print `Type` for user diagnostics
  (`i64` not `I64`, `string` not `String`).
- **B.15 (P1-20):** Add a `ReturnOfBorrowed` test; if not reachable,
  delete the variant.
- **B.16 (P1-22):** Build of `runtime/` on a non-x86_64 non-aarch64
  arch must `#error` out, not silently use the broken setjmp
  fallback.

**Phase B exit criterion:** zero warnings on `cargo build --release`,
LSP feature matrix at green, EBNF↔parser CI green, WAL property tests
in CI.

---

## Phase C — Documentation, conformance, release

- **C.1:** Author a single canonical "Jinn book" — split into
  Tutorial / Reference / Stdlib API. Today there are ≥ 14 markdown
  files at repo root that overlap; consolidate.
- **C.2:** Pin a "stdlib alpha subset" inside `std/`; mark the rest
  experimental. Document in `docs/std.md`.
- **C.3:** Author a "what we promise / what we don't" doc:
  `docs/stability.md`. List which features are stable, which are
  experimental, which are deprecated.
- **C.4:** Conformance test corpus: build a public test corpus that
  any reimplementation could be checked against.
- **C.5:** Performance regression CI: turn `benchmarks/results.csv`
  into a gating file; fail PRs that regress by > X %.
- **C.6:** Security / soundness disclosure policy. SECURITY.md.
- **C.7:** Cut `0.1.0-alpha.1`. Publish.

**Phase C exit criterion:** A new user can read the book, install
`jinnc`, run `jinnc init`, write a small actor + store program,
build it, and ship it — without ever opening the source tree.

---

## Estimated total effort (no claims on calendar time)

Phase A: ~ 2–3 engineer-weeks. Highest ROI, must come first.
Phase B: ~ 4–6 engineer-weeks. Parallelisable.
Phase C: ~ 3–4 engineer-weeks of focused writing.

Net: a small, focused team can bring this to alpha. Without Phase A,
the project should not be tagged alpha at all.
