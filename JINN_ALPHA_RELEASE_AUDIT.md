# Jinn Alpha Release Audit

Date: 2026-05-19

Scope: source review, build/test execution, example/app execution, targeted probes,
and benchmark experiments against the current workspace. This audit does not rely
on prior reports, repo docs, memories, or claims that were not rechecked against
source or live commands in this session.

## Executive Verdict

Jinn is a real language implementation, not a sketch. The repository contains a
complete compiler pipeline from lexer/parser through HIR, ownership checks, MIR,
Perceus, LLVM codegen, and a C runtime. It has actors, channels, persistent
stores, WAL support, a standard library surface, app-scale examples, and a large
test corpus. The current tree can build a release compiler, pass the full Rust
test suite, compile all sampled apps, run most examples, and hit near-C speed on
several numeric microbenchmarks.

The alpha release answer is still **no for external alpha**.

The reason is not lack of ambition or lack of working code. The reason is the
safety floor. A one-line malformed program can compile and return an arbitrary
exit code. Missing `main` falls through to the system linker. Integer division by
zero produces stack garbage and exits successfully. Bounds failure exits with no
diagnostic. The flagship `examples/alpha_release_demo` fails LLVM verification.
Generic enum matching with an empty variant fails in codegen. Formatter, warning,
tooling, and tree-sitter health are not release-clean.

My recommendation: tag the current state as **pre-alpha / internal dogfood**.
Do not publish it as an external alpha until the P0 list below is closed and
converted into non-ignored regression tests.

## Evidence Collected

Commands run directly in this workspace:

- `cargo test -q`: exit 0.
  - Largest chunks reported `913 passed; 0 failed; 1 ignored` and
    `423 passed; 0 failed; 1 ignored`, plus smaller suites all passed.
  - The test log still emitted 155 numeric/default-type warnings and 18
    `mir-perceus:` progress lines.
- `cargo test -q --test alpha_release_audit -- --nocapture`: exit 0 before the
  final full suite run; 6 added audit tests passed.
- `cargo build --release -q`: exit 0, but emitted 159 Rust warnings.
- `cargo fmt --check`: exit 1 after formatting the new audit harness. Remaining
  diffs include `src/codegen/clone/mod.rs`, `src/codegen/decl.rs`,
  `src/codegen/types.rs`, `src/mir/lower/stmt.rs`, `src/mir/opt/subst.rs`,
  `src/mir/opt/uses.rs`, `src/ownership/tests.rs`, `src/typer/*`, and
  `tests/bulk_tests.rs`.
- `cargo clippy --all-targets --all-features -- -D warnings`: could not run
  because `cargo clippy` is not installed in the current toolchain.
- Example projects through `target/release/jinnc run`:
  - Passed: `examples/chat_sim`, `examples/monte_carlo`,
    `examples/prime_sieve`, `examples/word_counter`.
  - Failed: `examples/alpha_release_demo` with LLVM call signature mismatch.
- App projects through `target/release/jinnc build`:
  - 16 / 16 compiled: `bank_ledger`, `blockchain_node`, `chat_room`,
    `inventory_mgr`, `iot_telemetry`, `kv_store`, `lattice_crypto`,
    `markdown_engine`, `microkernel`, `ml_autodiff`, `order_book`,
    `physics_engine`, `raft_cluster`, `regex_vm`, `route_planner`,
    `task_scheduler`.
  - Selected runtime smoke runs passed: `bank_ledger`, `blockchain_node`,
    `kv_store`, `microkernel`, `regex_vm`, `order_book`, `physics_engine`.
- Benchmark slice:
  - Command: `python3 run_benchmarks.py --bench=fibonacci,sieve,math_compute,store_ops_inmem,channel_throughput,actor_pingpong,vec_grow --runs=3 --warmup=1 --langs=jinn,c,rust,python --quiet --timeout=60`.
  - Result: exit 0. Total Jinn time 6.27s vs C 6.24s on the slice.
  - Highlights: `fibonacci` 0.98x C, `sieve` 1.00x C, `math_compute` 1.01x C,
    `actor_pingpong` 0.66x C, `channel_throughput` 6.76x C,
    `store_ops_inmem` 3.76x C, `vec_grow` 1.22x C.
- `tree-sitter-jinn` tooling:
  - `npm test`: exit 1. The basics corpus failed 3 / 6 cases: hello world,
    if/elif/else, and for loop.
- Targeted probes:
  - `10 / 0` compiled, printed stack-looking garbage, and exited 0.
  - A file containing only `42` compiled and exited 42.
  - A file with no `*main` reached `/usr/bin/ld` and failed with
    `undefined reference to main`.
  - `tests/audit_alpha/runtime_bounds_fail.jn` compiled, then exited 16 with
    empty stdout/stderr.
  - `tests/audit_alpha/negative_generic_empty_enum.jn` failed with
    `FieldGet on pointer to unknown struct type for field __tag`.

## Added Audit Tests

New files:

- `tests/alpha_release_audit.rs`
- `tests/audit_alpha/core_semantics.jn`
- `tests/audit_alpha/types_patterns_ownership.jn`
- `tests/audit_alpha/store_channels_actors.jn`
- `tests/audit_alpha/std_test_mode.jn`
- `tests/audit_alpha/negative_partial_move.jn`
- `tests/audit_alpha/negative_resource_copy.jn`
- `tests/audit_alpha/negative_resource_channel.jn`
- `tests/audit_alpha/negative_type_mismatch.jn`
- `tests/audit_alpha/runtime_bounds_fail.jn`
- `tests/audit_alpha/negative_generic_empty_enum.jn`

The positive fixtures cover arithmetic, bit ops, shifts, exponentiation,
floating point, strings, loops, recursion, structs with heap fields, enum match,
partial move and reassignment, `@resource` drop, stores, row write-through,
snapshots, channels, actors, `use std/math`, and `--test` mode.

The negative fixtures cover partial-move read-after-move, copying a resource,
sending a resource across a thread boundary, type mismatch, runtime bounds
failure, and the discovered generic enum empty-variant codegen failure.

## P0 Findings: Alpha Blockers

### P0-1: Integer division/modulo by zero is undefined behavior

Probe:

```jinn
*main() returns i32
    a is 10
    b is 0
    log(a / b)
    0
```

Observed result: compile exit 0, run exit 0, stdout like
`140735329981000`.

Root evidence: `src/codegen/mir_codegen/helpers/values.rs` emits LLVM integer
`sdiv`, `udiv`, `srem`, and `urem` without checking zero. LLVM treats integer
division by zero as poison/undefined. This must never ship as a language-level
success path.

Required fix: lower checked integer division/rem into MIR or a shared codegen
helper. Trap with a Jinn diagnostic for divisor zero and the signed overflow
case `INT_MIN / -1`.

### P0-2: Top-level expressions become implicit programs

Probe: a source file containing only `42`.

Observed result: compile exit 0, run exit 42.

Root evidence: `src/parser/decl/mod.rs` intentionally falls back to parsing
arbitrary top-level statements, and `src/parser/mod.rs` wraps top-level
statements into an implicit `main` when no explicit `main` exists. That is fine
for a script mode only if it is deliberate, documented, and has a stable
semantic. In the current release posture it silently accepts malformed program
shape and turns literal values into process exit status.

Required fix: decide whether Jinn has script mode. If yes, gate it under an
explicit mode and make expression statements have defined side-effect semantics.
If no, reject top-level statements except approved declarations and constants.

### P0-3: Missing main reports a linker error

Probe:

```jinn
*helper() returns i64
    42
```

Observed result: compile exit 1 with `/usr/bin/ld: undefined reference to main`.

Required fix: after lowering and before object/link, check the program entry.
Emit a compiler diagnostic: `program has no *main function` with the input file
span or a file-level diagnostic. The linker should not be the user-facing
semantic validator.

### P0-4: Flagship alpha demo fails LLVM verification

Command: `cd examples/alpha_release_demo && ../../target/release/jinnc run`.

Observed error:

```text
Call parameter type does not match function signature!
  %m8 = load %Metric, ptr %m7, align 8
 ptr  call void @reporter_persist_metric(%Metric %m8)
Call parameter type does not match function signature!
  %m15 = load %Metric, ptr %m7, align 8
 ptr  %metric_is_hot = call i1 @metric_is_hot(%Metric %m15)
```

This is especially important because the demo exercises project modules,
actors, stores, stdlib time, and cross-module struct calls. It is exactly the
kind of program an alpha release would showcase.

Likely root area: call ABI normalization for struct values vs pointers across
module-qualified functions. `Metric` is passed as a value in the source surface,
but the generated callee signature and call site disagree.

Required fix: make aggregate calling convention a single compiler invariant.
Either value-pass or pointer-pass structs consistently, encode the choice in
HIR/MIR function types, verify it before LLVM, and add the alpha demo as a
regression test.

### P0-5: Generic enum with empty variant fails in codegen

Fixture: `tests/audit_alpha/negative_generic_empty_enum.jn`.

Observed error:

```text
FieldGet on pointer to unknown struct type for field `__tag`
```

This program should be valid: `Maybe of i64` with `Some(i64)` and `Empty`, then
matching both cases. The bug appears when generic enum layout, empty variants,
and match/tag access meet in lowering/codegen.

Required fix: make enum representation explicit in MIR. Generic instantiation
must produce a concrete enum layout before field/tag lowering. Empty variants
should not go through an unknown struct pointer path.

### P0-6: Bounds failure exits silently

Fixture: `tests/audit_alpha/runtime_bounds_fail.jn`.

Observed result: compile exit 0, run exit 16, empty stdout and stderr.

This is better than memory corruption, but not alpha-grade. Bounds errors need
clear diagnostics with file/line and index/length where possible.

Source evidence: MIR aggregate codegen has vector bounds checking paths, but the
failure mode currently surfaces as a raw trap/exit rather than a Jinn runtime
diagnostic.

Required fix: route bounds traps through `__jinn_trap` or an equivalent runtime
diagnostic function. Add tests that assert stderr contains `out of bounds`, the
bad index, and the container length.

### P0-7: No MIR verifier

Search result: no `src/mir` verifier was found for operand type correctness,
return type consistency, terminators, dominance, or pointer/integer misuse.

Source evidence: `src/codegen/mir_codegen/helpers/values.rs` performs integer
auto-widening in codegen because mismatched integer widths can reach LLVM. That
is an architectural smell: codegen should not be repairing unverified MIR.

Required fix: add a MIR verifier and run it after MIR lowering, after Perceus,
and before LLVM emission. Minimum invariants: block terminators, value
dominance, operator operand types, call signatures, aggregate layout, and return
type agreement.

### P0-8: Release hygiene is not clean

Release build succeeds, but with 159 warnings. Formatting fails. Clippy is not
installed in the current toolchain. Tests pass, but emit repeated numeric type
default warnings from `std/math.jn`, `std/signal.jn`, and test fixtures, plus
`mir-perceus:` progress output.

Alpha users and contributors need a clean baseline. Warnings should mean
something. Release logs should not look like internal debug output.

Required fix: run `cargo fmt` across intentional Rust files, install clippy in
the pinned toolchain, make CI run `cargo clippy --all-targets --all-features --
-D warnings`, silence or fix compiler warnings, and gate internal progress
output behind a debug/verbose flag.

### P0-9: Tree-sitter tooling is failing its own tests

Command: `cd tree-sitter-jinn && npm test`.

Observed result: exit 1, 3 / 6 corpus tests failed. The failures involved hello
world, if/elif/else, and for loop parse trees.

Required fix: update `grammar.js`, regenerate parser artifacts, update corpus
expectations if the grammar intentionally changed, and add this test to release
CI.

## P1 Findings: Serious but Not Immediate Ship-Stoppers

### P1-1: Stale feature remnants are widespread

Source search found removed or stale feature surfaces for `Arena`, `Pool`,
`NDArray`, dynamic dispatch helpers, and retired Perceus/HIR paths. Examples:

- `Arena type removed` and `Pool type removed` in typer paths.
- Codegen still contains arena/drop/pool/dynamic-dispatch helpers.
- MIR and Perceus still carry pool hint fields that are informational or unused.
- Dynamic dispatch helpers exist while the core type system has no live
  `DynTrait` type.

This is not merely cosmetic. Stale surfaces make it harder to know which
semantic model is authoritative, and they increase the chance that new work
hooks into dead machinery.

Required fix: delete dead code or put it behind an explicit experimental gate.
Every feature name in public syntax, typer, MIR, codegen, runtime, stdlib, and
docs should be either live, rejected cleanly, or absent.

### P1-2: Type default warnings leak into standard workflows

The current test run produced 155 numeric/default-type warnings, mostly from
stdlib and fixtures. The warnings are useful in isolation, but noisy in a green
test run.

Required fix: annotate stdlib and fixture code, or adjust warning policy so
stdlib-internal warnings are fixed before release. User warnings should remain
useful and scarce.

### P1-3: Runtime allocation policy is inconsistent

`runtime/util.c` defines `jinn_xmalloc` with a clear OOM diagnostic and abort,
but many runtime modules call `malloc`, `calloc`, or `realloc` directly. Some
paths check, some return null, some proceed through runtime-specific behavior.

Required fix: centralize allocation policy for runtime-owned memory. Decide
which APIs can return null and which abort. Audit all runtime allocation sites,
especially stores, channels, scheduler, actors, vector/index/column modules,
TLS/process helpers, and migration/WAL paths.

### P1-4: Channel and actor runtime needs sanitizer/stress proof

The channel runtime is sophisticated and fast, but it is raw C concurrency using
atomic spinlocks, coroutine parking, and scheduler wakeups. `runtime/actor.c`
keeps legacy ABI helpers; `jinn_actor_wake` is a no-op. None of that is
automatically wrong, but it is exactly the kind of code that needs ASan, UBSan,
TSan, stress tests, and deterministic shutdown tests before external alpha.

Required fix: add stress tests for bounded/unbounded channels, close while
senders/receivers are parked, actor stop/destroy races, and multi-worker
scheduling. Run under sanitizers in CI.

### P1-5: VS Code packaging does not appear to wire the LSP server

The Rust source contains `src/lsp/*` and the release build includes the
`jinnc-lsp` binary. The VS Code package under `vscode-jinn` contributes syntax
highlighting and language configuration only; no extension activation or LSP
client wiring is present in `package.json`.

Required fix: either describe the VS Code extension as syntax-only, or add a
client that launches `jinnc-lsp` and smoke-test diagnostics/hover/definition/
completion/references/rename.

### P1-6: Benchmark infrastructure is useful but easy to clobber

`run_benchmarks.py` writes `benchmarks/results.json` for whatever subset was
run. A partial audit slice overwrote the tracked full result file until it was
restored. This is risky for contributors.

Required fix: write ad-hoc benchmark output to a named file or require
`--save/--output` for tracked results. CI should distinguish official baseline
runs from local experiments.

## Pipeline Review

### Lexer

The lexer supports a broad keyword and token set, including indentation-sensitive
syntax. It is adequate for the current tests, but release confidence needs a
lexer fuzz target: random bytes should produce either tokens or structured
lexing errors, never a panic.

### Parser and Syntax

The parser is feature-rich: functions, types, enums, actors, stores, traits,
impls, tests, control flow, pattern matching, imports, and top-level wrapping.
The danger is permissiveness. Arbitrary top-level statements compile into an
implicit program today. That should be a conscious language mode, not an
accidental fallback.

The parser also has tooling drift: tree-sitter currently fails its own corpus
tests. Parser, formatter, tree-sitter, VS Code grammar, and `jinn.ebnf` need a
single conformance loop.

### AST, HIR, and Type System

The type system has strong ingredients: explicit numeric types, structs, enums,
generic instantiation, resources, rows/stores, function types, actors, and
resource/thread-boundary checks. The audit fixtures verified several important
negative cases: read after partial move, copying resources, resource send across
thread boundary, and type mismatch.

The weak point is consistency at the HIR/MIR/codegen boundary. Type defaulting
warnings are noisy. Diagnostics still expose internal names like `I64` and
`String` in some paths. Generic enum layout can reach codegen in an invalid
shape.

### Ownership and Resource Safety

The ownership verifier is doing real work. Partial moves are tracked, resource
copy is rejected, and resource cross-thread send is rejected by the new audit
tests. `@resource` drop ran in the expected order in the positive fixture.

This is one of Jinn's strongest differentiators versus Go/Python/JavaScript-like
languages. It is not yet Rust-grade: the model still depends on correct HIR/MIR
lowering, codegen aggregate conventions, and runtime drop behavior. It needs
more adversarial tests around nested heap fields, enum payloads, closures,
actors, channels, and early returns.

### MIR, SSA, and Perceus

MIR exists and receives optimization and Perceus passes. This is the right
architectural direction. The concern is verification. Codegen currently repairs
some type mismatches that should not be able to reach it. Perceus emits progress
lines during ordinary test runs, and the older HIR Perceus surface is still
visible as retired/dead machinery.

Alpha bar: MIR must be verified before LLVM, and Perceus must be both silent by
default and regression-tested for drop correctness.

### LLVM Codegen

Codegen is the current risk center. It works for a lot of programs, including
all app compile smoke tests and most example runs. It also produces the most
serious failures: unchecked integer division, signature mismatch in the alpha
demo, generic enum layout/tag failure, and silent bounds trap behavior.

The broad fix is not a pile of local guards. The fix is to move invariants
upstream: typed MIR calls, typed aggregate layouts, checked integer operations,
and a MIR verifier. LLVM should receive boring, obviously valid IR.

### Runtime

The C runtime is ambitious: coroutines, scheduler, channels, actors, WAL,
stores, KV/index/column/vector/FTS helpers, process/fs/net/TLS pieces, and more.
The store/WAL tests passing is a good signal. The app smoke runs are also a good
signal.

The runtime is still raw C with mixed allocation policy and complex concurrency.
External alpha needs sanitizer coverage, stress tests, deterministic shutdown
semantics, and crisp runtime diagnostics for traps.

### Stores and WAL

Stores are one of Jinn's most distinctive ideas. The audit fixture verified
insert, first, count, sum, row field write-through, snapshot isolation, and
channel/actor interop in one program. Existing full-suite tests include WAL and
store coverage, and they pass.

Release risks: generated `.store`/`.wal` files are easy to mutate during local
runs, and store semantics need a compact public stability contract: row value vs
snapshot, transaction/durability guarantees, crash recovery scope, and schema
migration behavior.

### Standard Library

The stdlib is broad: math, strings, fs/os/process/net/tls, crypto, uuid, json,
csv, regex, sorting, dates, decimal/rational/complex/bigint, event, raft, and
more. Breadth is impressive, but release alpha should declare a smaller stable
subset. Current stdlib code emits type default warnings during tests.

Recommended alpha subset: math, strings, bytes, collections, fs/path/os/process,
time/date, args/io/logging, json/csv, crypto hashes, channels/actors surface,
stores. Mark the rest experimental until it has tests and examples.

### Tooling

Formatter command exists for Jinn code; Rust formatter is not clean. Tree-sitter
tests fail. VS Code syntax package exists. Rust LSP server source exists. VS Code
extension does not wire the LSP. This is enough for internal development, not
for a polished external alpha.

## Comparative Assessment

### Versus Rust

Jinn is trying to occupy some of Rust's safety/performance territory with a
simpler surface, built-in stores, actors, and Perceus-style ownership. That is a
good bet if the compiler invariants become solid. Today Rust is far ahead on
soundness, diagnostics, tooling, package management, fuzzing, and ecosystem.
Jinn should not claim Rust-like safety until P0-1 through P0-7 are fixed.

### Versus Zig

Zig's strength is explicit control, small runtime assumptions, allocator
discipline, and excellent cross-compilation posture. Jinn has higher-level
language services and a much broader runtime, but less predictable failure
semantics right now. Jinn needs a clear allocator/runtime policy and better
diagnostics before it can appeal to Zig's systems audience.

### Versus Go

Go has proven concurrency, a production scheduler, excellent tooling, and a
stable standard library. Jinn's actors/channels/stores are more integrated and
potentially more interesting for dataful concurrent systems, but the runtime and
toolchain are not yet hardened.

### Versus C and C++

Jinn can already approach C performance on selected compute benchmarks and gives
the programmer safer, higher-level constructs. But current division-by-zero UB,
silent traps, and raw runtime C risks mean Jinn cannot yet be sold as a safer C
replacement. It can be sold as an experimental systems language with serious
potential.

### Versus Python

The benchmark slice shows Jinn massively ahead of Python on CPU-bound code while
keeping a compact syntax. Python still wins on ecosystem, REPL/iteration speed,
libraries, packaging, data science, and beginner experience. Jinn's path is not
to replace Python broadly; it is to be a compiled language with Python-level
approachability for selected systems/data workloads.

### Versus Swift

Swift has mature ARC, value semantics, protocol-oriented design, diagnostics,
and industrial tooling. Jinn's Perceus/resource direction is promising, but it
is far less proven. Jinn can learn from Swift's aggregate ABI discipline and ARC
debuggability.

### Versus Mojo

Mojo targets Python-adjacent performance and ML/compiler workloads. Jinn is less
ML-centered and more systems/runtime/store centered. Jinn's unique angle is
actors plus persistent stores plus ownership. Mojo-like audiences will expect
excellent numeric arrays; Jinn's removed/stale NDArray surface is a warning not
to overclaim data science readiness.

### Versus Kotlin and Scala

Kotlin/Scala win on ecosystem, IDEs, JVM libraries, and high-level application
development. Jinn can win on native performance, low-level control, predictable
deployment, and built-in persistence/concurrency if it hardens. Today it is not
ready for ordinary CRUD/backend teams unless they are language hackers.

### Versus Erlang/Elixir

Jinn has actors, channels, and supervisors in the surface area, but Erlang/Elixir
have decades of VM semantics, fault tolerance, hot code, observability, and OTP.
Jinn's actor model is promising, but needs shutdown semantics, supervision
contracts, scheduler stress tests, and docs before comparison is fair.

## Persona Readiness

- Systems programmer: exciting, not safe enough yet. Needs UB/trap fixes, ABI
  invariants, sanitizer runs, and allocator discipline.
- OS/embedded developer: not ready. The current runtime assumes a substantial C
  runtime/LLVM/libc environment.
- Distributed/blockchain developer: promising. The app suite compiling/running
  is a good signal, but determinism, serialization, crypto audit, and runtime
  failure semantics need hardening.
- Data science/numerics developer: not ready as a primary target. Numeric
  performance is promising, but arrays/dataframe/FFT/bigint surfaces need a
  stable subset and more tests.
- Web/backend/CRUD developer: too early. Package management, HTTP/TLS/database
  ergonomics, docs, and IDE support need work.
- Safety-critical/aerospace developer: no. Needs formalized semantics,
  sanitizers, fuzzing, deterministic builds, conformance tests, and a much
  longer stability record.
- Low-level hacker/compiler nerd: yes, excellent dogfood target. The language is
  rich and the codebase is alive.
- Beginner: not yet. Diagnostics and tooling are too rough.
- Veteran language engineer: worth attention. The architecture has real pieces;
  the hard work is now invariant discipline and deletion of stale surfaces.

## Alpha Remediation Plan

### Phase A: Safety floor

1. Add MIR verifier and wire it after lowering, after Perceus, and before LLVM.
2. Fix integer division/rem traps, including signed overflow.
3. Normalize aggregate call ABI and fix `examples/alpha_release_demo`.
4. Fix generic enum layout/tag lowering for empty variants.
5. Replace silent bounds trap with Jinn diagnostic.
6. Decide and enforce top-level script mode semantics.
7. Add missing-main compiler diagnostic before link.
8. Convert the P0 probes into regression tests. Known-bad tests may start as
   ignored, but alpha requires them to run green.

Exit criterion: no one-line user program can produce UB, stack garbage, invalid
LLVM, silent trap, or raw linker diagnostics.

### Phase B: Hygiene and tooling

1. Make `cargo fmt --check` clean.
2. Install clippy in the pinned toolchain and gate `-D warnings` in CI.
3. Remove or gate stale `Arena`, `Pool`, `NDArray`, dynamic dispatch, and retired
   Perceus surfaces.
4. Silence stdlib/test type-default warnings by fixing annotations or warning
   policy.
5. Fix tree-sitter tests and add them to CI.
6. Wire VS Code extension to `jinnc-lsp` or label it syntax-only.
7. Add ASan/UBSan/TSan jobs for runtime-linked tests.
8. Add lexer/parser fuzz targets.

Exit criterion: release build/test/tooling logs are clean and reproducible.

### Phase C: External alpha package

1. Declare an alpha-stable stdlib subset.
2. Publish a stability document: stable, experimental, removed.
3. Consolidate install/build/run/test/package documentation.
4. Add conformance tests for syntax, type system, stores, actors, resources, and
   runtime errors.
5. Run full examples/apps/benchmarks in CI or nightly.
6. Cut `0.1.0-alpha.1` only after P0 is closed and Phase B gates are green.

## Final Assessment

Jinn has earned a serious audit because it already behaves like a serious
language in many places. The passing test volume, working apps, LLVM backend,
runtime ambition, store model, actors, ownership checks, and benchmark results
are all real.

But an alpha release is a promise about the floor, not the ceiling. The ceiling
is high. The floor currently has holes. Close the P0s first, then ship the
alpha with confidence.