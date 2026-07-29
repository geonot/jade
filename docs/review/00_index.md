# Jinn — Comprehensive Alpha-Readiness Review

**Date:** 2026-05-19
**Reviewer:** Independent compiler / language engineer
**Method:** Source review + own probes + own builds + own test runs. All
findings reproducible from `review/probes/` and `review/probes2/`. No
documentation, memory, or third-party claims were taken at face value;
every assertion below is backed by a compiled and executed program or a
direct citation into the source tree.

## Verdict (TL;DR)

**Jinn is closer to a real language than its bug list suggests, but
NOT alpha-ready today.** It is a remarkably ambitious, broad-surface
project (≈ 84 kLOC of compiler + runtime + stdlib + tests; full LLVM
backend; bespoke Perceus reference-counting pipeline; actors,
coroutines, channels, persistent stores, WAL, SIMD, FFI).

**Strong positive signals:**
- **1,570 / 1,570 tests pass** on a clean release build.
- **16 / 16 sample apps compile and run** — including a raft-cluster
  demo, a blockchain node, an ML autodiff engine, a microkernel
  simulator, lattice crypto, a regex VM, a physics engine, an order
  book, a route planner, IoT telemetry. These are not toys.
- **33 / 33 benchmarks compile and run.**
- **Tail-call optimisation works** (probe verified to 10,000,000-deep).
- **Actors, traits + impl, generics monomorphisation, generic stores +
  WAL, atomic vars, string interpolation, deferred actions, pattern
  matching, mutual recursion** all verified working end-to-end.
- **The runtime is disciplined** (zero TODO/FIXME, clean public ABI,
  per-arch context switch in real assembly).

**But the safety floor is below the bar that any alpha-tier language
must meet:**

1. **Integer division by zero produces uninitialized stack garbage**
   instead of trapping or returning a defined value.
2. **Vec/array out-of-bounds access segfaults the user's process** —
   there is no bounds check in the runtime or codegen, only LLVM's
   raw GEP+load.
3. **The compiler emits LLVM IR that fails LLVM's own verifier**
   (generator probe `p31_generator.jn`). A sound compiler must never
   produce IR the verifier rejects; this means MIR→LLVM lowering is
   unsound for at least one supported construct.
4. **Idiomatic `map(v, $ * 2)` over an `[i64]` literal vec ICEs the
   compiler** (probe v1) or **crashes at runtime with rc=16** (probe v2).
   This is exactly the kind of "hello-world-plus-one-step" code an alpha
   user will write in their first hour.
5. **A program with no `main` reaches the system linker** and the user
   sees a raw `ld: undefined reference to main` error rather than a
   compiler diagnostic.
6. **A bare top-level literal `42` compiles**, runs, and returns exit
   code 42 — the grammar accepts free-floating expressions as
   "declarations" and the literal value silently becomes the program's
   side-effect.
7. **159 compiler warnings** (mostly unused imports) on the release
   build — strong code-hygiene smell.
8. **Codegen has 29 kLOC** — larger than typer (16 k) + parser (6 k) +
   lexer (1.4 k) + MIR (6 k) + HIR (1.4 k) + Perceus (0.9 k) +
   ownership (1 k) combined. The center of gravity is in the wrong
   place; the codegen is doing work (auto-widening, type recovery,
   pointer-arith fallbacks) that should have been settled upstream.

The project also has genuine strengths that should not be undersold —
see §3 for the architectural overview, the strengths list above, and
§24 for a prioritized plan to alpha.

## How this document is organised

The review is split into sections so each can be read (and revised) in
isolation. Read in order if you are evaluating Jinn; jump to §21 + §24
if you only want the bugs and the plan.

| §   | File                                          | Topic |
| --- | --------------------------------------------- | ----- |
| 0   | [00_index.md](00_index.md) (this file)         | Index & TL;DR |
| 1   | [01_methodology.md](01_methodology.md)         | How findings were produced |
| 2   | [02_metrics.md](02_metrics.md)                 | Quantitative codebase metrics |
| 3   | [03_architecture.md](03_architecture.md)       | Pipeline overview |
| 4   | [04_lexer.md](04_lexer.md)                     | Lexer review |
| 5   | [05_parser_syntax.md](05_parser_syntax.md)     | Parser & surface syntax review |
| 6   | [06_ast_hir.md](06_ast_hir.md)                 | AST / HIR review, type system, ownership |
| 7   | [07_type_system.md](07_type_system.md)         | MIR / SSA review, Perceus |
| 11  | [11_codegen_runtime.md](11_codegen_runtime.md) | LLVM codegen + C runtime review |
| 12  | [12_safety_concurrency_stores_stdlib_tooling.md](12_safety_concurrency_stores_stdlib_tooling.md) | Memory safety, concurrency, stores, stdlib, tooling, tests |
| 13  | [13_perf_comparison.md](13_perf_comparison.md) | Performance + comparison to 10 languages |
| 14  | [14_findings.md](14_findings.md)               | P0 / P1 / P2 / P3 findings catalogue |
| 15  | [15_roadmap.md](15_roadmap.md)                 | Three-phase remediation plan to alpha |
| 16  | [16_appendix.md](16_appendix.md)               | Probe appendix (35 probes + apps + benchmarks sweep) |
| 17  | [17_perspectives.md](17_perspectives.md)       | Voiced verdicts: Linus, Geohot, Carmack, Knuth, Antirez |
