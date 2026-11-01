# Jinn: comprehensive language and implementation review

Review date: 2026-10-29
Reviewed checkout: `1745166` (`[191]`), including the working-tree changes already present when the review began.
Scope: language design, documented contracts, compiler architecture, ownership, numerical semantics, concurrency, persistence, effects, capabilities, standard library, interoperability, tooling, packaging, verification, and production readiness.
Change policy: review only. No compiler, runtime, library, test, or specification fixes were implemented.

## 1. Executive judgment

Jinn has a coherent and worthwhile core: inferred static types, readable native-code programming, single-owner aggregates, automatic drops, inexpensive borrows, explicit copying where ownership matters, and structured concurrency. Stores as a checked language feature are a genuine differentiator. The compiler has substantial executable coverage and meaningful verification infrastructure. This is considerably more than a syntax prototype.

It is not production-ready as a general-purpose systems language. The immediate reasons are concrete, not a demand for every planned feature:

- Ordinary accepted programs can produce wrong numerical results or LLVM poison through integer exponentiation and float-to-integer conversion.
- The documented checked-arithmetic tuple contract does not survive code generation.
- A `defer` inside a branch that is never taken still runs. Loop defers are collected statically rather than registered dynamically.
- The formatter changes the behavior of simple, valid arithmetic programs. Its parse-after-print safeguard cannot detect that damage.
- Transaction failure paths can continue after the runtime itself reports that recovery or durability is no longer guaranteed. Cancellation and multi-store locking have documented liveness gaps.
- A clean release suite does not imply a clean development suite: the full debug run has one failure that the release run conceals.
- Current capability inference is useful checking, not a security sandbox or a complete higher-order effect calculus. Package ceilings and the intended binary-distribution machinery are still designs.

The next milestone should be a small, explicitly supported, semantically reliable native alpha, not simultaneous expansion into deployment orchestration, a registry, freestanding targets, and a broader language surface. Do not discard the ownership model or add a garbage collector to address these findings. Most defects are broken contracts between existing phases, not evidence that the central model is wrong.

### What deserves preservation

1. **The ownership model's economy.** Numbers and text copy, containers move, reads borrow; views and freezing cover important zero-copy cases without lifetime syntax.
2. **Inference with escape hatches.** Local inference, named access overrides, nominal declarations, traits, and explicit type annotations can coexist without making ordinary source ceremonial.
3. **Structured shutdown as a first-class contract.** Scope-owned tasks, actor draining, observable closed-channel sends, and cancellation tests address the hard part of concurrency rather than just spawning work.
4. **Executable language contracts.** Access, memory, errors, stores, Unicode, real applications, and optimization-level comparisons are already tested.
5. **A compact native pipeline.** Typed HIR, SSA MIR, ownership/drop passes, and LLVM are a defensible division of labor. Keep MIR transformations focused on language-specific facts LLVM cannot recover.
6. **Honesty in the documentation.** Several documents explicitly distinguish compiling, linking, behavioral coverage, alpha stability, and future design. Make that discipline consistent everywhere.

## 2. Review method, snapshot, and verification

### 2.1 Evidence labels

- **Reproduced:** executed against this checkout, with the observed result reported below.
- **Code-audited:** the relevant implementation path was inspected; no fresh fault-injection or stress reproduction is claimed.
- **Documented limitation:** an existing contract or roadmap limitation, distinguished from a newly demonstrated failure.
- **Design finding:** a problem in a proposed specification or architecture, not a claim that the unimplemented feature currently misbehaves.
- **Recommendation:** a proposed next step; it was not implemented during this review.

Severity is a release-priority assessment, not a CVSS score. **P0** blocks a trustworthy supported-language release; **P1** is a major correctness, reliability, or boundary-contract issue; **P2** is important hardening or usability work; **P3** is longer-term design or expansion. A P0 is not automatically an exploitable memory-safety vulnerability.

### 2.2 Checkout and environment

The review used Linux x86-64, eight logical CPUs, Rust/Cargo 1.91.1, and the LLVM 21.1 binding configuration (`inkwell` 0.8, `llvm21-1`). The binary reports version `0.0.0`. Rough physical source counts were 86,097 Rust lines, 7,169 runtime C lines, 10,363 `std/` lines, and 2,888 `libjn/` lines; these are not executable-LOC or complexity measurements.

The seven pre-existing modified files were `CHANGELOG.md`, `docs/design/compiler-prereqs.md`, `docs/internals.md`, `docs/roadmap.md`, `docs/tooling.md`, `src/driver/sources/modules.rs`, and `src/interface.rs`. Their changes remove unsafe/broken interface readback and update its documentation. That work belongs to the incoming checkout, not this review. It was preserved. The only new review deliverable is this file.

The current authoritative documents are under `docs/`, particularly `docs/jinn.md`, `docs/jinn.ebnf`, `docs/memory-model.md`, `docs/concurrency.md`, `docs/error-effects.md`, `docs/strings.md`, and `docs/stdlib.md`. Older architectural descriptions of generic Perceus reference counting do not describe the current user-value ownership contract.

### 2.3 Commands and results

| Verification | Observed result | Qualification |
| --- | --- | --- |
| `cargo build --release` | Passed, no warnings | Incremental build; not a clean-room distribution build |
| `cargo clippy --release --all-targets -- -D warnings` | Passed | Includes Rust targets; not a proof that all C warnings are fatal |
| `cargo test --release` | **2,420 passed, 0 failed, 15 ignored** | 58 harness summaries, including zero-test harnesses/doc tests |
| `cargo test` | Failed at the formatter corpus gate | Confirmed by an isolated rerun |
| `cargo test --no-fail-fast` | **2,419 passed, 1 failed, 15 ignored** | All test targets completed; sole failure was `fmt_output_still_frontend_checks_over_corpus` |
| `cargo fmt --all --check` | Failed | Pre-existing trailing blank line at `src/driver/sources/modules.rs:244` |
| `git diff --check` | Same pre-existing EOF issue | Not introduced or fixed by this review |
| Tree-sitter unit corpus | **10/10 passed** | Local Tree-sitter CLI 0.24.7 |
| Tree-sitter parsing of `tests/ebnf_corpus/*.jn` | **12/12 passed** | The top-level twelve files, not every Jinn source in the repository |
| `timeout 240 bash ci/sanitize.sh` | Passed | Nine programs under ASan/UBSan and again under TSan: 18 smoke executions |
| Targeted semantic probes | 68 compile/run attempts, plus formatter follow-ups | 34 initial/follow-up source cases at `--opt 0` and `--opt 3`; rejected cases did not execute |
| Formatter behavior probe | Wrong output reproduced at both optimization levels | Three arithmetic expressions, before and after formatting |
| `--debug` native probe | No `.debug_*` sections in inspected executable | One small native program; not a target matrix |

The complete release run includes the access-semantics, memory-model, concurrency-shutdown, standard-library, store, trait, application, and corpus-differential suites. The applications harness exercises 21 projects. The fifteen ignored cases are legacy error-syntax tests, not fifteen demonstrated passing behaviors.

Local audit logs are `/tmp/sol-review-build.log`, `/tmp/sol-review-clippy.log`, `/tmp/sol-review-tests.log`, `/tmp/sol-review-tests-debug.log`, `/tmp/sol-review-tests-debug-full.log`, `/tmp/sol-review-fmt-debug-retry.log`, `/tmp/sol-review-fmt.log`, `/tmp/sol-review-tree-sitter.log`, `/tmp/sol-review-sanitize.log`, `/tmp/sol-review-probes.log`, and `/tmp/sol-review-followup.log`. These are convenience evidence, not durable dependencies of the report: the important reproducers and outcomes are included here.

### 2.4 What was not established

This was not a formal soundness proof, an exhaustive audit of every runtime line, a fresh benchmark campaign, or a security certification. The full corpus sanitizer sweep, long-running fuzz campaigns, fresh multi-node consensus tests, an OS/architecture matrix, and systematic disk-failure injection were not run. Existing benchmark numbers are not re-certified here.

The sanitizer smoke instruments the C runtime. Merely linking a generated LLVM object to an instrumented runtime does **not** instrument every Jinn-generated load/store or synchronization operation. The smoke also disables leak detection. Its success must not be described as proof of memory safety or leak freedom for the language.

## 3. Finding register

| ID | Priority | Evidence | Finding |
| --- | --- | --- | --- |
| SOL-01 | P0 | Reproduced + code-audited | Defers are collected statically, not registered along executed paths |
| SOL-02 | P0 | Reproduced + code-audited | Integer `pow` goes through floating point and can yield LLVM poison |
| SOL-03 | P0 | Reproduced + code-audited | Saturating float casts select out-of-range LLVM conversions at rounded boundaries |
| SOL-04 | P0 | Reproduced + code-audited | `checked_*` loses the documented tuple and overflow flag |
| SOL-05 | P1 | Reproduced + code-audited | Saturating integer intrinsics ignore signedness and correct multiplication bounds |
| SOL-06 | P1 | Reproduced + code-audited | `as strict` checks bit round-trips, not all value-range requirements |
| SOL-07 | P1 | Reproduced | Valid upper-half `u64` annotated literals reject with an incorrect range |
| SOL-08 | P2 | Reproduced | Unknown-method diagnostic has no source location |
| SOL-09 | P0 for durable-store claims | Code-audited | Transaction begin/commit/rollback failures can continue after losing guarantees |
| SOL-10 | P1 | Documented + code-audited | Cancellation can strand transaction locks; multiple stores can deadlock |
| SOL-11 | P1 before ABI stabilization | Code-audited | Layout calculation has divergent models and fail-open type fallbacks |
| SOL-12 | P1 before binary identity/cache rollout | Design + code-audited | Proposed cyclic hash fixpoint has no termination argument; current source hashing is incomplete |
| SOL-13 | P1 for capability/security claims | Documented + code-audited | Effects/access summaries are not a complete callable contract |
| SOL-14 | P1 for gate credibility | Code-audited | Sanitizer gates can miss failures and do not instrument the full generated program |
| SOL-15 | P0 for `fmt --write` reliability | Reproduced + code-audited | Formatter drops essential arithmetic grouping and changes program behavior |
| SOL-16 | P1 | Reproduced + code-audited | Formatting `i64::MIN` emits double negation; debug typer panics |
| SOL-17 | P2 | Reproduced + code-audited | Native `--debug` probe lacks source-level debug information |

The register intentionally distinguishes fresh reproducers from roadmap carry-forwards. It is not a replacement count for every historical issue in `docs/roadmap.md`.

## 4. Reproduced semantic defects

For ordinary executable reproducers, save the fenced source as `/tmp/repro.jn`, compile with `target/release/jinnc /tmp/repro.jn --opt 0 -o /tmp/repro`, and execute `/tmp/repro`. Repeat with `--opt 3`. Run store examples in a disposable directory. Results below were observed at both levels unless stated otherwise.

### SOL-01 — `defer` is not dynamically registered

```jinn
*f(x as bool)
    if x
        defer log('registered')
    log('body')

*main
    f(false)
```

Expected: `body` only. Actual:

```text
body
registered
```

A second reproducer:

```jinn
*main
    for i in 0 to 3
        defer log(i)
    log('body')
```

Actual: `body` followed by a single `3`. The loop's reached defer statements are not represented by distinct runtime registrations. The branch reproducer already establishes the defect without relying on a particular loop-capture interpretation.

**Root cause:** `src/mir/lower/stmt.rs:193-195` appends the body to a compiler-side `function_defers` vector. `src/mir/lower/ctx.rs:206-212` later lowers that list in reverse. Traversing source syntax is being used as a substitute for executing a registration operation. A never-taken branch therefore contributes cleanup.

**Required design correction:** represent reached cleanup obligations in control flow. Static activation flags may suffice for bounded registrations; loops require either an appropriate cleanup scope per iteration or a dynamic registration representation, depending on the deliberately specified semantics. Define capture timing, cleanup scope, early returns, loop exits, errors, cancellation, and cleanup failure together. Reordering a final list cannot repair the model.

**Acceptance:** branch false/true, mutually exclusive branches, zero/many loop iterations, nested scopes, break/continue/return, error propagation, and cancellation, with exactly-once LIFO cleanup and live operands at the final generated-code level.

This review does **not** freshly demonstrate a heap use-after-free for a normal function-exit defer. `xs is vec(123456789, 222222222); defer log(xs.get(0))` produced the correct value. Although early MIR placed a drop before the deferred read, the drop-sinking pass (`src/drops/mir_drops.rs:501-541`) moved the final free after the read. Pre-drop MIR alone would have supported a false diagnosis. Conditional registration remains independently broken.

### SOL-02 — integer exponentiation is neither exact nor safely checked

```jinn
*main
    x is 9007199254740993
    log(x pow 1)
```

Expected: `9007199254740993`. Actual: `9007199254740992`.

```jinn
*main
    log(2 pow 63)
```

Under the documented default overflow discipline this must not silently become an arbitrary integer. Actual: `-9223372036854775808` at optimization level 0 and `140736931370448` in the level-3 run.

**Root cause:** `src/codegen/mir_codegen/helpers/values.rs:144-157` converts both integer operands with signed integer-to-`f64`, calls C `pow`, then performs unchecked float-to-signed-integer conversion. This loses integer precision, disregards unsigned interpretation, and generates LLVM poison when the result is outside the conversion domain. The arbitrary output is wrong-code evidence, not a demonstrated memory disclosure.

**Recommendation:** exact exponentiation by squaring using the intended overflow policy and the operand's signedness/width. Specify negative exponents and `0 pow 0` explicitly. Keep floating exponentiation as a separate floating operation. Test all widths and signedness, identities beyond `2^53`, zero, negative bases, and overflow at each relevant optimization level.

### SOL-03 — rounded float-cast boundaries let poison escape

```jinn
*main
    a is 9223372036854775808.0
    log(a as i64)
```

The implemented ordinary float-cast policy is saturation, also pinned for large values in `tests/alpha_review_regressions.rs:112-116`. Expected here: `9223372036854775807`. Actual in one level-0 run: `140735239157008`; the level-3 result differed.

```jinn
*main
    a is 18446744073709551616.0
    log(a as u64)
```

Expected: `18446744073709551615`. Actual: another arbitrary value, differing between executions/optimization levels.

**Root cause:** `src/codegen/mir_codegen/helpers/values.rs:529-589` constructs the high bound as `i64::MAX as f64` or `u64::MAX as f64`. These round up to `2^63` and `2^64`. The `OGT` check does not reject equality with that rounded boundary, so the selected raw `fptosi`/`fptoui` is out of range. Smaller destinations converted from lower-precision float formats need the same careful treatment.

**Recommendation:** use LLVM's saturating conversion intrinsics where appropriate, or a rigorously derived range guard with representable/exclusive bounds. Test exact boundaries and adjacent representable floats, signed and unsigned widths, `f32` and `f64`, negatives, fractions, infinities, and NaNs. A test of `1.0e300` alone cannot exercise the rounded-boundary hole.

### SOL-04 — checked arithmetic discards its result contract

```jinn
*main
    a is 12
    b is 30
    value, overflowed is checked_add(a, b)
    log(value)
    log(overflowed)
```

Expected:

```text
42
false
```

Actual:

```text
0
false
```

Additional observed cases:

| Operation | Expected `(value, overflowed)` | Actual |
| --- | --- | --- |
| `checked_add(i64::MAX, 1)` | `(-9223372036854775808, true)` | `(0, false)` |
| `checked_add(250 as u8, 10 as u8)` | `(4, true)` | `(0, false)` |
| `checked_sub(-128 as i8, 1 as i8)` | `(127, true)` | `(0, false)` |

**Root cause:** `src/typer/builtins.rs:87-122` advertises a tuple of operand type and boolean. `src/codegen/mir_codegen/intrinsics/overflow.rs:633-686` calls signed overflow intrinsics but extracts only element zero and returns an integer. The overflow bit is discarded and the resulting LLVM value does not implement the typed result. Both tuple indexing and tuple binding were exercised.

**Recommendation:** return the complete typed LLVM aggregate, select signed/unsigned intrinsics correctly, and enforce the intrinsic signature across typing, MIR verification, and codegen. Do not repair only destructuring or special-case the example. This finding is particularly serious because `checked_*` is the supposed alternative to trapping overflow.

**Testing lesson:** the existing systems-demo corpus uses checked arithmetic, but optimization-level agreement can certify two identically wrong executions. Add independent expected outputs and an intrinsic-contract matrix.

### SOL-05 — saturating arithmetic uses the wrong domains and bounds

```jinn
*main
    a as u8 is 250
    b as u8 is 10
    log(saturating_add(a, b))
```

Expected `255`; actual `4`.

Other observed results:

| Expression / operand types | Expected | Actual |
| --- | --- | --- |
| `saturating_sub(0, 1)`, `u8` | `0` | `255` |
| `saturating_mul(100, 2)`, `u8` | `200` | `255` |
| `saturating_mul(100, 2)`, `i8` | `127` | `-1` |
| `saturating_mul(-9223372036854775807, 2)`, `i64` | `-9223372036854775808` | `9223372036854775807` |

**Root cause:** `src/codegen/mir_codegen/intrinsics/overflow.rs:579-630` uses `llvm.sadd.sat`, `llvm.ssub.sat`, and signed multiplication overflow regardless of operand unsignedness. Multiplication chooses a truncated `i64::MAX` for every width and every overflow direction.

**Recommendation:** a single numeric-domain description must provide bit width, signedness, minimum, maximum, and the relevant intrinsic family. Saturating multiplication must select the correct endpoint from sign/domain information. Add a Cartesian test matrix, not another isolated `i64` positive test.

### SOL-06 — strict casts are not generally value-range checks

```jinn
*main
    a as i64 is -1
    log(a as strict u64)
```

Actual: `18446744073709551615`, rather than a range failure. The reverse case, `a is -1 as u64; log(a as strict i64)`, prints `-1`.

```jinn
*main
    a is 1000.0
    log(a as strict i8)
```

Actual: `127`, rather than a strict-range failure. `1.5 as strict i8` also produces `1`; whether fractions must fail requires an explicit exactness policy, but accepting the out-of-range `1000.0` is already inconsistent with the documented strict-cast intent (`docs/jinn.md:325-328`).

**Root cause:** `src/codegen/mir_codegen/emit_inst/aggregates.rs:683-709` checks integer bit round-tripping and has no corresponding general floating strictness check. Equal-width signedness changes can preserve bits while changing the mathematical value. A saturating ordinary conversion is not a strict conversion.

**Recommendation:** specify ordinary versus strict casts by source/destination numeric domains, including equal-width sign changes and floating fractional/NaN/infinite inputs. Implement range/exactness checks from that specification. Preserve their observable trap effects even when the converted value is unused.

### SOL-07 — annotated `u64` literals lose their unsigned magnitude

```jinn
*main
    x as u64 is 18446744073709551615
    log(x)
```

Rejected with a diagnostic claiming that literal `-1` does not fit in `u64`, whose range is printed as `0..9223372036854775807`.

The explicit cast `x is 18446744073709551615 as u64` is accepted and prints `18446744073709551615`. This is not an absence of `u64` machine support; it is a literal-representation/range-checking inconsistency.

**Recommendation:** preserve literal magnitude independently from unary sign until the expected numeric domain is known. Check the full destination range without squeezing the upper half of `u64` through `i64`. Treat `i64::MIN`, `u64::MAX`, defaulting, explicit wrapping casts, unary negation, formatting, and diagnostics as one boundary problem. SOL-16 is another manifestation of this boundary design.

### SOL-08 — a basic method error still lacks a location

```jinn
*main
    v is vec(1)
    log(v.unknown())
```

Actual diagnostic: `no method 'unknown' on Vec`, without a path, line, column, or source span.

This is smaller than the wrong-code findings, but a common newcomer error is a high-value diagnostic path. Preserve structured spans through method resolution and suggest nearby valid methods from the shared method registry. Do not recover a location by parsing display text in the LSP.

### SOL-15 — formatting changes arithmetic behavior

```jinn
*main
    log(10 - (3 - 1))
    log(24 / (6 / 2))
    log(-(1 + 2))
```

The original executable prints:

```text
8
8
-3
```

`target/release/jinnc fmt /tmp/repro.jn` prints the following source:

```jinn
*main
    log(10 - 3 - 1)
    log(24 / 6 / 2)
    log(-1 + 2)
```

That executable prints:

```text
6
2
1
```

Both before/after executions were checked at optimization levels 0 and 3. All programs parse and type-check. This is not merely unstable whitespace or an unsupported exotic construct.

**Root cause:** `src/fmt.rs:923-959` parenthesizes a nested binary expression when its operator differs, but not when identical operators have an associativity-sensitive right operand. `src/fmt.rs:960-966` prints a unary operand without protecting a lower-precedence binary expression. Syntax preservation requires precedence **and** associativity **and** operand position, not operator-name equality.

**Recommendation:** a complete precedence-aware expression printer, with a structural round-trip check before writing. Preserve the normalized AST for formatting-only changes; use a deliberately defined typed normalization for approved semantic rewrites. Refuse to write on a mismatch. Idempotence and reparsing are useful checks, but neither implies equivalence.

**Acceptance:** parenthesized left/right operands for every binary operator; same-operator non-associative forms; floating addition/multiplication, where reassociation is also observable; unary operands; casts, calls, indexing, ternaries, and nested lambdas; full project-context corpus checks. Add exact-output oracles for the source above. Until then, `fmt --write` cannot be trusted on valuable source without inspecting the diff.

### SOL-16 — `i64::MIN` prints as double negation and crashes the debug typer

Reproduction using existing source:

```sh
target/debug/jinnc std/bit.jn --emit-hir --lib
target/debug/jinnc fmt std/bit.jn > /tmp/bit-formatted.jn
target/debug/jinnc /tmp/bit-formatted.jn --emit-hir --lib
```

The first command succeeds. The formatter changes the expression containing `-9223372036854775808` into:

```jinn
if (x & --9223372036854775808) equals 0
```

The formatted file's debug frontend exits 101 with:

```text
panicked at src/typer/expr/op.rs:20:38:
attempt to negate with overflow
```

The release frontend accepts the same formatted file. This is the sole failure in the complete debug suite: `tests/fmt_nondestructive.rs:173-249`, reported at its assertion on line 244. It reproduced in an isolated rerun.

**Root cause:** signed literal payloads and separately represented unary negation do not have a consistently printable boundary form. `src/fmt.rs:910,960-966` mechanically prints both signs, and `src/typer/expr/op.rs:15-20` applies host-Rust `-n` to a possible `i64::MIN`. Release host overflow behavior hides the frontend panic rather than proving the source transformation correct.

**Recommendation:** share a sign/magnitude boundary model with SOL-07, and never use unchecked host integer arithmetic when interpreting source-language literals. Make debug/release frontend behavior agree. Expand formatter gates from “still checks” to structural preservation; the present diagnostic truncates stderr to its first line, which was empty in the failed test and concealed the useful panic text.

### SOL-17 — debug mode does not yet establish a useful source-debugging contract

A small program compiled with `--debug` yielded an ELF executable with no `.debug_*` sections under `readelf -S`. `src/driver/mod.rs:581-584` enables debug mode, and `src/codegen/support/module.rs:185-206` creates a compile unit. The inspected `src/codegen/debug.rs` only finalizes the builder and pops scopes; creating a compile unit alone does not provide function, variable, and instruction locations.

**Recommendation:** treat native source debugging as unfinished until a test can break at a Jinn line, inspect a local, and unwind a native call. Test coroutine-stack behavior separately. Do not infer full debugger support from a flag or from the existence of an LLVM DI builder. This observation is limited to the tested program and target.

### Controls and historical claims that did not reproduce

The review also checked positive/negative controls to avoid carrying obsolete allegations forward:

- A method expecting a nominal `alias Seconds is f64` rejected a plain `f64` argument. The roadmap's broad T-4 statement did not reproduce for this case.
- Iterating a closed, buffered typed channel drained its `42` successfully.
- A missing `Map.get` trapped with `runtime error: map key not found`; it did not silently return zero.
- Ordinary narrow unsigned overflow trapped with the integer-overflow diagnostic.
- `false and noisy()` and `true or noisy()` did not evaluate their right operands.
- A bare duplicate unique-store insertion aborted with the named store/field diagnostic; it was not silently discarded.
- Unicode scalar length versus byte count behaved as documented in the exercised case: `é` had scalar length 1 and byte count 2. Byte slicing could produce a one-byte fragment, and `byte(255)` produced one byte.
- A store declaration with a `Vec of i64` field was accepted. No aggregate insert/persistence round-trip was exercised, so declaration acceptance is not evidence of valid aggregate storage.
- The normal-exit heap defer control produced the expected value after final drop sinking, as discussed under SOL-01.

## 5. Surface language, readability, and semantic economy

### 5.1 The central idiom works

`is` for binding/rebinding, `as` for annotations/casts, `*name` for functions, indentation, and word operators create a recognizable language. Code can remain close to intent while being statically checked. Inference is particularly valuable when the compiler also owns the bookkeeping for moves, temporary copies, drops, and error propagation.

The surface is no longer “one conditional construct.” Current Jinn includes `if`/`elif`/`else`, `unless`, `while`/`until`, `match`, and the `?`/`!`/`!!` family. Review and documentation should describe the language that exists, not force it into an older slogan.

The principal ergonomic risk is the accumulated number of equivalent spellings and context-sensitive meanings: optional call parentheses, statement/expression alternatives, word and symbolic operators, `of` versus annotation-only angle arguments, compact arms, contextual `$`, and several meanings of `!`. None individually invalidates the design; together they increase parser, formatter, editor, and teaching obligations.

### 5.2 Recommended surface policy

- Specify one canonical style for everyday code, while accepting compatible alternatives deliberately. Do not silently remove syntax as a cleanup exercise.
- Publish precedence, associativity, evaluation order, truthiness rules, loop bounds, implicit returns, and assignment/rebinding behavior in one normative account with runtime examples.
- Treat the formatter as the canonical printer only after SOL-15/SOL-16 are closed. Today it can erase necessary meaning.
- Define `$` by lexical binder identity, not “nearest construct that happens to understand it.” Pipes, success arms, lambdas, and dispatch interact badly when the binder changes implicitly. The roadmap's CG-8 dispatch case deserves a dedicated binding test.
- Explain prominently that `alias` is nominal. In many languages that word means transparent substitution; Jinn's behavior is a useful newtype-like abstraction but an avoidable source of surprise.
- Distinguish source convenience from semantic inference. A programmer should not need to predict an AST name bucket to know whether a function consumes a value or needs a capability.

Config blocks can reuse the parser without executing arbitrary build-time Jinn. Preserve a restricted, deterministic data model, explicit allowed forms, and clear diagnostics. “Same syntax” must not turn a manifest into an implicit code-execution boundary.

## 6. Types, generics, traits, and compilation boundaries

### 6.1 What is already real

The implementation has HM-style inference, constraint solving, monomorphization, recursive/SCC-oriented inference machinery, structural records, nominal types and enums, traits, generic bounds, and default trait methods. `tests/traits.rs` exercises actual trait behavior. Describing traits as wholly future work would be inaccurate.

Defaulting otherwise-unsolved numeric variables to `i64`, with warnings and strict-mode controls, is a pragmatic alpha choice. It must not become a substitute for solving constraints or a silent fallback for unrelated types. SOL-07 and SOL-16 show why literal representation needs to survive inference without premature signed narrowing.

### 6.2 The missing unified callable contract

`src/types.rs:23` represents a function as `Fn(Vec<Type>, Box<Type>)`. Error effects, capability summaries, and parameter access behavior live elsewhere; they are not represented in that function type. This is not automatically unsound—conservative analyses can be safe—but it makes correctness depend on several side tables and traversal conventions agreeing.

The current consumption/mutation inference operates on pretyped syntax and name-based call relationships. It is impressive that this permits implicit borrows, but its long-term boundary is visible: resolved method identity, higher-order calls, generic instantiations, imported interfaces, and incremental rechecking all want the same resolved contract.

**Recommendation:** establish a shared callable summary, keyed by resolved identity/instantiation, with parameter access modes, result ownership, error information, capabilities, and calling convention where appropriate. This need not expose a full row-polymorphic syntax to users. First unify the compiler's semantic data; add user-visible abstraction only when a real use case requires it.

Keep body inference inside compilation units where practical. At public package boundaries, freeze or explicitly assert inferred consumption/mutation behavior so that adding a `Vec` field or changing a callee's implementation does not silently change downstream ownership semantics.

### 6.3 Traits and coherence

Traits need one coherent resolution path for ordinary dispatch, generic bounds, conversions, and serialization of exported contracts. The current `From` name-pattern shortcut is a concrete exception (section 8). Before multi-version linking, define impl provenance, orphan rules, overlap, default methods, visibility, specialization policy if any, and identity across owner scopes.

Do not deduplicate nominal types merely because their current field layout matches. Conversely, do not make every private implementation detail part of source-level type identity accidentally. Separate nominal identity, API compatibility, ABI compatibility, and object equivalence.

### 6.4 Fail closed late in the pipeline — SOL-11

`src/codegen/types.rs:21-24,44-47` correctly treats unresolved type variables/parameters as compiler bugs. The fallback for a missing nominal struct/enum (`26-30`) or store row (`52-57`) instead silently uses `i64`. A missing layout is an invariant violation, not a valid scalar representation.

Layout also has multiple models: `llvm_ty(Fn)` is a two-pointer closure (`79-81`), but `type_size_of(Fn)` falls in an eight-byte case (`132-139`). Array and tuple logical sizing round elements to eight bytes (`140-145`), while LLVM aggregate layout follows another path; pointer sizing is hardcoded to eight bytes (`100`). Some helpers may intentionally calculate allocation/storage slots rather than ABI size, but their names and use must make that distinction explicit.

These observations establish an architectural inconsistency, not a freshly reproduced out-of-bounds closure program. The acceptance condition is one target-aware layout authority, based on LLVM `TargetData` or a rigorously shared layout model, with explicitly named non-ABI storage formats. Compare size/alignment/offsets for every supported type family and target, including closures inside aggregates, narrow arrays, enums, views, and `sret` thresholds.

## 7. Ownership, borrowing, views, freezing, and resource lifetime

### 7.1 Review the current model, not the historical name

The binding contract in `docs/memory-model.md` is single ownership, inferred borrowing, and automatic drops. Scalars copy; strings and value-only aggregates deep-copy; containers and transitively aggregate types move; resources are linear. Channels and actor references have purpose-built shared runtime handles. There is not a generic reference-counted user object graph on every path.

That is a strong fit for predictable native programs. It gives the compiler a tractable default while allowing text to feel value-like. It also means documentation should not attribute current semantics to generic Perceus reference counting, or claim that reference counting magically solves all lifetime and cycle questions.

### 7.2 Strengths and costs

Place-based move/borrow checking is the right foundation. It can distinguish sibling fields, overlapping arguments, and partial moves rather than treating a whole variable as one indivisible permission. Rejecting difficult multi-projection moves is an honest temporary restriction; accepting them without a sound place update would be worse.

The cost is that ownership categories are semantically observable. Adding an aggregate field can turn assignments into moves through multiple embedding types. Existing `@value`/`@aggregate` assertions are valuable. Public-category changes should be compatibility-relevant and accompanied by an explanation path to the field that changed the category.

`.get` copies by contract; it is not a hidden persistent borrow. Document that cost at container boundaries. Avoid claiming all inferred reads are free when nested values or strings may require copying. Benchmarks should measure clone volume, allocations, and retained memory, not only final arithmetic throughput.

### 7.3 Views

Second-class views—pointer/length pairs with a compiler-known root and no escaping lifetime—are a good reduction of the problem. Root locking and conservative block lifetimes are reasonable before non-lexical lifetime refinement. Do not expand views into ordinary storable first-class references until the escape/lifetime model can justify it.

The essential tests are invalidation under nested mutation, sibling disjointness, dynamic versus literal indices, closure capture, task capture, returns, container insertion, and calls that infer consumption. Rejection must be stable across syntactic positions, not only direct assignment.

### 7.4 Freezing

`freeze` consumes ownership and provides deep read-only access. Scope-bounded sharing of an outside binding across `together` avoids generic refcounts and is a particularly good design point. The known restrictions—inside-scope binding fallback, conservative actor sends, and incomplete additional sharing scopes—are understandable safety boundaries, not defects to bypass heuristically.

The critical proof obligation is that the owner cannot drop or mutate before the last permitted reader ends. Use the structured join as the source of lifetime truth; do not infer safety merely from the source block looking read-only.

### 7.5 Closures, generators, and known retention

Captures currently copy scalars, clone strings, and move aggregates into owned environments. Rejection of borrowed parameters, views, and resources where they cannot safely escape is appropriate. Capturing lambdas are intentionally monomorphic; noncapturing lambdas can generalize. Explain that limitation instead of pretending inference has full principal polymorphic closure types.

The roadmap identifies suspended-generator aggregate drops, per-iteration closure-environment leaks, some early-exit/continue leak classes, and runtime-handle retention. These were not all freshly reproduced here. They are enough to disqualify an unconditional “every owned value drops exactly once in the implementation” promise until a leak-enforced corpus supports it.

Single ownership rules out many user heap cycles, but internal channels, actors, closures, and FFI have separate retention behavior. State cycle and deterministic-destruction claims for the safe, implemented ownership subset, not for every possible runtime graph.

### 7.6 One cleanup model

`defer`, resource drops, temporary drops, error exits, transaction rollback, generator destruction, task cancellation, and actor termination are all cleanup obligations. They currently meet in several passes and runtime hooks. SOL-01 and SOL-10 show why they need a shared exit/cleanup model rather than unrelated lists of epilogue actions.

Prioritize explicit cleanup edges/obligations in MIR, verification of their activation and ordering, and a final LLVM-level execution oracle. The compiler should be able to explain which obligation runs on each exit, including cancellation, without relying on an incidental same-block sinking pass.

## 8. Errors, effects, and capabilities

### 8.1 Error values and implicit propagation

Using `Option`/`Result`-like value enums, `err`, declared error bounds, and `?`/`!`/`!!` handlers is consistent with Jinn's prose-oriented intent. Implicit propagation is useful precisely because a programmer does not want to repeat ceremony at every bare call or store mutation.

It raises the specification bar. The distinction between a bound raw result, a propagated value, an explicitly handled arm, and an unhandled statement must be positionally precise. `main` behavior must be documented separately from ordinary fallible functions. No expression should change its error meaning merely because a lowering path forgot to carry context.

### 8.2 `From` resolution is not trait resolution

`src/typer/errset.rs:152-158` recognizes conversions through a globally named `<Target>_from_<Source>` function. The roadmap's E-1 correctly notes that this is not trait-impl provenance/coherence checking. A name convention should not authorize a semantic conversion across package boundaries.

Replace that shortcut with the same resolved impl mechanism used by the trait system, with the documented orphan/coherence policy enforced. Tests should distinguish a real permitted `From` impl from a similarly named ordinary function, ambiguous conversions, and conversions imported from incompatible package scopes.

The error-set implementation seeds selected cases and scans/narrows inferred results; the design describes a general SCC union fixpoint. This review does not claim every recursive error program fails. It does require explicit mutual-recursion, generic-instantiation, and multi-error-union tests to demonstrate that implementation and stated fixed-point semantics agree.

### 8.3 Capability checking — SOL-13

Shared capability/decorator/builtin metadata is a good existing single-source-of-truth direction. Direct-call inference and conservative tainting of many indirect/higher-order calls already prevent important laundering shapes. It would be incorrect to say higher-order capabilities are wholly unchecked.

The remaining model is nevertheless AST-level and partly name-bucketed. The roadmap's E-2 records method-name overapproximation and underapproximations involving relation traversal, unresolved receivers, and certain match-arm dataflow. These are not all freshly exploited here. Their existence prevents treating `needs` as a complete security boundary.

**Recommendation:** infer over resolved typed calls and operations, including relation reads and instantiated methods; propagate callable summaries through values; retain a conservative unknown/FFI top effect; and test both false-accept and false-reject cases. Parameterize higher-order effects when needed rather than permanently making every useful callback maximally effectful.

### 8.4 Paths are not a sandbox

The compiler-prerequisites design says paths normalize away `.`, `..`, and symlinks (`docs/design/compiler-prereqs.md:82-83`). Lexical normalization can address the first two. Symlink identity depends on a mutable filesystem; canonicalizing once still permits time-of-check/time-of-use races.

If filesystem confinement is a real promise, it requires runtime-enforced directory/file capabilities and appropriate OS handle-relative operations, not just string comparison. Specify the platform contract and escape behavior. Static capability declarations remain useful audit information even without a sandbox; simply label them honestly.

The same boundary applies to arbitrary extern calls, environment variables, shell execution, process creation, network endpoints, and dynamically assembled paths. Package/module ceilings are explicitly design-only (E-3), not current protection for untrusted dependencies.

### 8.5 Conflicting store-error documentation

`docs/error-effects.md:176-177` calls unhandled store mutation in nonfallible code fire-and-forget and says its error is discarded. The same document's R4 (`193-196`) describes stopping `main` for an unhandled fallible statement. Current store tests and the fresh duplicate-insert probe trap with an error diagnostic and SIGABRT.

Choose and document the intended store-specific policy. The observed trap is safer than silently losing writes, but contradictory normative text is a language defect even when the runtime chooses the safer alternative. Do not silently change the runtime to match the stale discard sentence.

## 9. Concurrency and scheduler readiness

### 9.1 Strong base, bounded claims

The runtime is an M:N stackful coroutine scheduler with work stealing and a worker cap of eight. Channels are bounded MPMC queues with explicit close behavior. Scope-owned work joins; top-level actors are daemon-like, with best-effort bounded draining at exit rather than an unconditional delivery guarantee. Aggregate captures transfer ownership. These are substantive semantics, and the shutdown suite passed in both compiler profiles.

Unscoped dispatch rejection is preferable to silently creating work that never runs. Actor stop/drain/join semantics and failure-driven sibling cancellation are also the right primitives. Keep shutdown contracts more prominent than spawn syntax.

### 9.2 Blocking operations are the service-readiness limit

Blocking socket operations can pin scheduler workers; the existing DIST-8 workload description reaches starvation with eight idle connections. Sleeps and some I/O/join paths are not cancellable in the same way as channel parks and loop backedges. The presence of `runtime/timer.c` does not establish a completed nonblocking I/O reactor.

A scalable server needs either readiness-based I/O integrated with task cancellation or a clearly separated blocking-operation pool. Cancellation must deregister waiters safely, preserve buffer ownership, and ensure a parked task can be awakened after worker migration. Measure scheduler utilization and tail latency under idle connections, partial writes, slow peers, and cancellation storms.

Do not advertise “millions of lightweight tasks” as equivalent to “millions of independently cancellable network operations.” The latter is an I/O and resource-lifetime claim.

### 9.3 Actor and select boundaries

Synchronous actor-call behavior, fallible handlers, supervisor behavior, and send-select must be described according to current acceptance/rejection, not design prose. Unsupported send-select is currently rejected rather than silently accepted. Some synchronous/fallible handler forms are also rejected. A diagnostic is a valid alpha boundary; a dormant runtime feature must not be presented as a usable language guarantee.

The supervisor roadmap includes races and a restart-limit policy. Specify supervision as structured scope behavior before extending the syntax. Restart requires a fresh ownership/capability environment and must not resurrect moved resources accidentally.

### 9.4 Required concurrency acceptance matrix

Test scope entry/exit, nested scopes, child failure, cancellation while runnable/parked/in cleanup, full/empty/closed channels, failed sends with heap payloads, actor mailbox drain, repeated stop/join, and task migration. Include cancellation during a transaction, blocking I/O, and generator suspension. Stress on one worker and many workers; use deterministic seeds and bounded timeout diagnostics.

The runtime contains sanitizer/fiber support; the claim is not that no annotations exist. The question is whether each stack switch and cancellation/destroy path is covered under the actual instrumentation used in CI.

## 10. Stores, durability, and transaction semantics

Stores combine schema checking, persistence, queries, indexes, relations, migrations, and error handling in a way ordinary libraries cannot fully match. That is valuable, but also makes database-like guarantees part of the language's trust surface.

### 10.1 What is implemented

Current tests exercise commit/rollback, nested transactions joining an outer transaction, WAL recovery, trap/error rollback, persistent indexes, schema checks, compaction, relations, sidecars, and shared-store isolation. Reader locking prevents swapping a `FILE*` underneath a concurrent reader. The single-process policy is enforced by an advisory lock at store open; it is not unrestricted multi-process transactional access.

A transaction's first mutation takes a store writer lock until commit/rollback. Whole-file snapshots supply rollback state. This is a reasonable correctness-first prototype architecture, but it is not a scalable row/page transaction engine.

### 10.2 SOL-09 — failure paths lose guarantees without stopping success

**Code-audited, not freshly fault-injected:**

- `runtime/wal.c:506-521` reports that a transaction begin frame could not be written, or was skipped for a snapshot beyond the frame limit, then continues tracking the transaction. Its own diagnostic warns that crash recovery may retain partial writes.
- `runtime/wal.c:590-600` does not turn a data flush/sync failure into a failed commit. It prints that the batch may not be durable.
- `runtime/wal.c:604` explicitly discards the commit-frame write result before releasing locks and transaction state.
- `runtime/wal.c:628-635` truncates the WAL before restoring the data file at `650-660`. A crash in between can remove the undo evidence while leaving transaction writes in the data file; this is the documented S-1 window.
- Rollback restore/reopen failures report that the store remains unrolled-back or that the process handle is stale and unsafe, but the outer rollback routine still completes and releases state.

The problem is not that storage hardware can fail. The problem is allowing an operation to appear completed after detecting that its promised postcondition is false or unknown. For a durable store, “printed a warning” is not an error protocol.

**Required contract:** before mutating, either persist sufficient recovery state or fail the transaction. Commit acknowledgement must follow the selected durability policy's successful persistence boundary. Preserve undo/recovery evidence until rollback completion is durable. If restoration/reopen leaves a store unusable, poison the handle and fail subsequent operations deterministically. Specify indeterminate outcomes when a commit may have reached disk but acknowledgement fails; do not promise that every error implies no mutation.

A typed `StoreError`/transaction result is preferable for recoverable faults. A diagnosed fail-stop is still better than continued operation with a known unsafe handle. The design must decide which class each fault belongs to.

### 10.3 SOL-10 — cancellation and multiple stores

The documented S-11 case strands a writer lock if a task is cancelled mid-transaction without commit/rollback. S-12 permits opposite-order acquisition of two store locks, causing deadlock. The code structure supports these concerns; no new hanging stress case is claimed here.

Tie transaction state into task unwind/cleanup. Select a deterministic multi-store lock protocol or reject unsupported acquisition shapes. Lock ordering alone does **not** create multi-store crash atomicity: independent per-store commit frames can be persisted on different sides of a crash. Either limit the promise to single-store crash atomicity or design a real shared decision/recovery protocol.

Cancellation, returned errors, traps, process termination, and power loss are different events. Give each an explicit transaction-outcome table. A lexical `transaction` block is not proof that all of them have identical rollback behavior.

### 10.4 Complexity and storage-format limits

Whole-store snapshots cost O(store bytes) memory and I/O per transaction, with a default 256 MB cap that aborts. The same snapshot is written as a WAL undo image. Small updates to large stores therefore scale with database size, not update size. Before promising production throughput, measure realistic data volumes, crash recovery, concurrent readers, write amplification, and latency under sync policies.

The roadmap also records WAL growth/recovery complexity and full-table query work. Plan bounded WAL/checkpointing and row/page undo only after the crash/failure contract is correct. A faster incorrect engine is not progress.

Sidecars have their own durability story; notably standalone version history is documented as `fflush`-only even under `@durable`. Transaction commit does attempt syncing tracked files, so it would be inaccurate to say that no sidecar is ever synced. Test each decorator, mutation path, and recovery relationship explicitly.

Aggregate field declarations are accepted, while the documented persistence surface centers on scalar fields. Reject unsupported persisted field types at declaration time unless their encoding, ownership, schema evolution, and recovery paths are actually defined and tested.

### 10.5 Query representation and migrations

Name-encoded, flat predicate calls make grouped filters difficult to represent faithfully. Use a structured typed predicate IR for grouping, `in`, text predicates, and lowering decisions instead of expanding a string-encoded mini-language. Rejecting unsupported grouping is better than silently folding it with the wrong precedence.

Migrations require durable progress markers, identity/version rules, crash restart behavior, and a declared supported transformation set. Do not infer portable on-disk ABI from native field types. Endianness, checksums, maximum lengths, schema fingerprints, and format-version upgrades are separate contracts from the compiler's native ABI.

## 11. Standard library, strings, numerics, and FFI

### 11.1 Stability should describe behavior, not only reachability

`docs/stdlib.md` lists 48 alpha-stable modules, provisional `raft` and `bangle`, and experimental `test`. Its two automated bars—whole-module frontend checking and an import/link/run program—are useful and passed. The document explicitly says this is not proof that every function returns the right answer.

That qualification is correct but weaker than many readers infer from “stable.” Keep the tier label, but add per-module behavioral coverage and known limits. A trivial importer cannot establish sorting, crypto validation, parser rejection, process I/O, or network shutdown correctness. Existing `tests/stdlib/*_tests.jn` suites are the right direction; do not describe their growing coverage with an obsolete blanket count.

`process` is no longer provisional in the current stdlib document. Single-node Raft election now has behavior tests; multi-node replication, log matching, and commitment remain unexercised. Do not re-file the old “Raft cannot mutate anything” root cause as current, and do not promote a consensus implementation on single-node election alone.

### 11.2 String and byte domains

The string contract intentionally distinguishes Unicode scalar length from byte count and allows byte-level construction/slicing. That is workable for systems code, provided every operation states which domain it consumes and whether invalid UTF-8 can be produced or observed.

FFI lengths, allocation sizes, network protocol lengths, and byte indices must use byte counts. User-facing character iteration/length may use Unicode scalars. Neither is a grapheme-cluster count. Fresh controls confirmed the scalar/byte distinction, not universal Unicode correctness across all modules.

Views over bytes are an appropriate zero-copy foundation. Audit remaining scalar-bounded byte loops for both correctness and complexity. Avoid repeated scalar-to-byte rescanning inside loops. Add non-ASCII and malformed-byte inputs to parsers, regex offset handling, URL/path utilities, and binary/text boundaries.

### 11.3 Numerics require an independent oracle

The fresh defects show a need for domain-wide testing, not only more examples. For every integer width and signedness, test normal, wrapping, checked, saturating, cast, shift, division, modulo, and power policies against an independent arbitrary-precision reference. Include destination boundaries and values one step either side. For floats, include representable neighbors, signed zero, NaNs, infinities, conversion boundaries, and formatting round trips.

`--fast-math` and `--deterministic-fp` need explicit, mutually understandable promises. Determinism across CPUs, math libraries, compiler versions, and targets is much stronger than disabling one LLVM option. Do not claim bitwise reproducibility for transcendental functions without pinning their implementation.

### 11.4 FFI and binding generation

The current `bind` work preprocesses through the system C compiler and handles more real headers than the old textual-only description suggests. It is still not a complete C semantic frontend. Layout, calling convention, varargs, function pointers, ownership transfer, out parameters, errno, and callback lifetime must be explicit contracts.

Skipping unsupported declarations with reasons is good. Emitting a plausible but ABI-wrong declaration is worse than rejecting it. For system headers, compare generated layouts/signatures against small C probes. Test narrow signedness, nested structs/unions, arrays, aliases, and platform calling conventions.

Unsafe pointer, volatile, allocator, syscall, and foreign callback primitives should remain a clearly labeled trust boundary. Capability inference cannot make arbitrary C safe. Hosted `libjn` wrappers are useful infrastructure, but do not constitute a completed freestanding libc replacement simply because the design calls for one.

## 12. Compiler architecture and backend discipline

### 12.1 Keep the good phase boundaries

The current pipeline lexes/parses/resolves, infers and lowers typed HIR, performs ownership/escape work and HIR validation, folds supported compile-time expressions, lowers SSA MIR, verifies/optimizes it, adds drops/reuse work, emits LLVM, and links native objects. Release MIR verification is enabled by default. These are valuable guardrails.

The arithmetic and defer findings show where verification stops short: it can validate an IR while a backend implements a different semantic signature, or while a cleanup list encodes the wrong dynamic program. Strengthen the contract, not just the count of assertions.

A verifier should compare actual operand/result types for builtin/intrinsic contracts, explicit access/effect summaries, cleanup obligations, and fully resolved layouts. Codegen should not invent an emergency scalar type after a missing nominal declaration.

### 12.2 One driver pipeline

`src/driver/mod.rs` and `src/driver/pipeline.rs` contain overlapping compilation orchestration. Differences in flags, debug setup, linking, project settings, and checking paths are a persistent source of “works in one command” bugs. Consolidate shared phase construction into one pipeline with explicit outputs/options, leaving command-specific behavior at the edges.

Acceptance must compare direct compile, project build, run cache, check, test, LSP frontend, and library/interface generation for the same semantic diagnostics. Fast check paths should omit expensive code generation, not ownership or effect validation.

### 12.3 LLVM is not a semantic repair pass

LLVM can optimize correct IR; it cannot infer that an integer power routed through `f64` was meant to be exact, or that an integer return value was meant to be a tuple. Poison-sensitive flags and conversions need language-level proofs. Range-based overflow-check elision is useful precisely because MIR knows source ranges; keep its interval/branch/phi proofs conservative and regression-tested at boundaries.

Do not duplicate broad LLVM optimization passes in MIR. Invest there in ownership, cleanup, alias/category facts, source ranges, effectful-operation preservation, and representation invariants LLVM cannot rediscover.

### 12.4 Target support is a tested contract

`build.rs:7-11` selects AArch64 context assembly when the target string matches, otherwise x86-64 assembly. Unsupported architectures should fail explicitly rather than silently receiving the x86-64 default. Native linking and Wasm branches have distinct toolchain/runtime constraints; a `--target` option is not a support matrix.

Before claiming a target: build the runtime for it, validate layout/ABI, compile/run core semantics, exercise coroutine switching and stack probes, verify linking/sysroot flags, and test FFI. Pointer-size assumptions from SOL-11 make 32-bit/freestanding claims particularly premature.

Rust's warning-clean release build and Clippy run are positive evidence. C warning flags in `build.rs:52-59` do not include a warnings-as-errors policy; documentation should not claim that they do. Clean incremental build output also does not prove a fresh C compilation emitted no warning.

## 13. Packages, identity, binary interfaces, and lamp

### 13.1 Separate shipped machinery from the target design

`docs/design/lamp.md:3-9` correctly marks lamp as design, distinct from the shipped git-tag package-command prototype. There is real compiler machinery already: `ScopePath`, `PkgId`, a resolution DAG, visibility/scoping tests, and module-local type identity. Saying package identity is wholly unimplemented would be wrong.

Conversely, these foundations do not provide the complete signed registry, multi-version ownership-scoped ecosystem, `.jnb` distribution, deployment records, or freestanding substrate promised by the design. Keep unsupported combinations fail-closed. The current resolver explicitly rejects multi-major coexistence (`src/driver/sources/dag.rs:68-99`) and still uses name-to-path maps in materialization.

### 13.2 Interface files are not safe reusable binary libraries

The incoming working-tree change removes `.jni` readback; the reviewed state is write-only. That is the right boundary when a reader cannot reconstruct bodies/methods/contracts safely. It is not a fix implemented by this review.

`src/interface.rs:5` uses interface version **3**, not the old function-only v1 described in some design-status prose. It serializes substantially more declarations, including parameter consumption/mutation bits. It still does not encode a complete reusable generic/body/MIR/effect/layout contract. `TypeVar` and `Row` become `I64`; aliases/newtypes erase in the generic conversion; `Generator` becomes `Coroutine` (`140-145`). Those choices are unacceptable as silent semantic loss in a future trusted read path, even where current frontend nominal representation makes a particular variant uncommon.

Do not revive interface reuse with a timestamp comparison or a larger JSON header. Define exactly what is reusable, how it is validated against toolchain/target/contracts, how generic bodies are supplied, and how stale/corrupt/mismatched artifacts are rejected.

### 13.3 SOL-12 — the proposed cyclic hash fixpoint is invalid

`docs/design/compiler-prereqs.md:383-400` proposes hashing within an SCC until placeholder hashes stabilize, describing the process as monotone and finite. Cryptographic digest iteration has no such monotone lattice structure. A finite state space guarantees eventual repetition, not a fixed point, and a cycle may be astronomically long. “Until stable” therefore has no termination proof.

**Recommendation:** canonically encode the finite SCC as a graph with stable node indices and labeled internal edges, incorporate already-known external dependency hashes, and hash that finite serialization once. Define canonicalization for symmetric graphs and identity-bearing nominal nodes precisely. Condensing SCCs is useful; iterating a cryptographic hash is not a substitute for graph canonicalization.

The hashing design also must distinguish API, target ABI, object content, and nominal identity. Equal ABI means compatible calling/layout/ownership conventions, not equal behavior. Capability purity plus equal ABI does not justify merging different implementations. Function identity can also be observable through permitted pointer/integer operations; define whether such observability is supported before deduplicating addressable code.

### 13.4 Current hashes are source fingerprints, not semantic ABI hashes

`src/pkgid.rs:327-351` hashes package name, owner scope, version, source bytes, and dependency hashes. This is a useful identity input, but it is not the proposed typed recursive ABI model.

`src/driver/sources/dag.rs:102-117` collects only immediate `.jn` files and concatenates their contents without per-file names/boundaries. Nested source edits can escape this collection, and different file partitions can yield the same byte stream. Add a canonical recursive manifest of relative paths, lengths, contents, configuration/toolchain determinants, and dependency identities before using it as a trustworthy cache/content key.

`PkgId::intern` keys records by name/scope/version, excluding content hash/visibility (`126-138`); define its lifetime and update policy before long-lived/incremental reuse. The object prefix uses only four hash bytes (`180-186`). A 32-bit prefix needs collision detection/disambiguation, not an assumption that collisions never happen. These are code-audited design risks, not newly demonstrated cache corruption in a running project.

### 13.5 Resolver and reproducibility design

Minimal-version selection with ordinary caret/tilde upper-bounded constraints is not automatically Go-style monotone MVS. Specify conflict solving, deterministic selection, backtracking/objective policy, and scope isolation for incompatible dependencies. Add adversarial diamond and upper-bound tests before promising one simple rule solves them all.

A lockfile alone cannot guarantee bit-identical output on any machine at any time. Toolchain, target CPU/features, C compiler/linker, sysroot, external libraries such as OpenSSL/PCRE2, environment, paths, build metadata, and deterministic serialization matter. Define a hermetic build envelope and artifact provenance rather than an unqualified slogan.

API/ABI hashes can automate parts of compatibility checking, not infer all behavioral semantic versioning. Adding a required trait method, changing an error/capability contract, or changing observable behavior can be breaking despite a familiar function signature. Preserve a reviewed release policy above the hash machinery.

### 13.6 Reduce packaging scope before expansion

Prioritize identity, deterministic resolution, source integrity, capability policy, offline locked builds, cache correctness, and reproducible artifacts. Signing/federation need key ownership, revocation, rollback protection, namespace policy, and mirror behavior, not just a signature field.

Issue tracking, changelog automation, deployment orchestration, and freestanding output are separate large products. Atomic rollback of an application plus irreversible schema changes and external network effects is not generally achievable by swapping a binary. Specify phased, forward-compatible migrations, durable deployment progress, and a recovery policy instead of presenting deployment as a universal transaction.

## 14. Developer tooling and documentation

### 14.1 Formatter

The formatter has worthwhile comment preservation, idempotence tests, corpus coverage, and a reparse-before-write safeguard. SOL-15 demonstrates why all four can pass while behavior changes. SOL-16 additionally shows debug/release disagreement on formatted existing stdlib source.

Treat structural equivalence as the formatting gate. For intentionally idiomatic rewrites, use a separately specified rule system with preconditions; arbitrary semantic equivalence is undecidable and should not be promised as a general verifier. Multi-module `apps/` need project-context formatting checks. Placeholder printer fallbacks must not silently become written source.

### 14.2 LSP

The full frontend per edit, panic containment, positioned type diagnostics, UTF-16 conversion, and local `DefId`-aware rename are real strengths. Package imports still use an empty package set, cross-file fallback is lexical, generic instantiation names lose source associations, and the workspace index is populated from open files. Full-document replacement and synchronous per-request analysis also limit responsiveness.

A lexical rename across files must not be sold as semantically safe refactoring. Refuse or clearly limit ambiguous edits rather than changing unrelated same-spelled identifiers. Preserve source binder identity through monomorphization, resolve the actual project/package graph, and add a cancellable incremental analysis cache before performance becomes an editor-blocking problem. A 256 MiB-stack worker per edit is containment, not incrementality.

The current LSP smoke suite calls handlers directly; it does not certify stdio framing, process lifecycle, or editor integration. Add at least one end-to-end client session for diagnostics, UTF-16 positions, change notifications, and workspace rename boundaries.

### 14.3 Diagnostics and CLI

Move to structured diagnostic data across phases rather than display-string extraction. Carry spans through method resolution and runtime trap generation; show the causal ownership/effect chain, not merely a failed final constraint. Test stderr, non-TTY behavior, stable exit classes, and multiple source files.

The current partial task-12 work has already improved single-file test/check behavior, binding preprocessing, and assertion positions. Credit those changes; do not repeat old failures as if the current tests had not changed. Native debug information remains a separate gap (SOL-17).

The toolchain should report a meaningful version/revision instead of `0.0.0`, including sufficient runtime/interface compatibility information for bug reports and reproducible artifacts.

### 14.4 Documentation drift

The roadmap is valuable evidence but mixes historical review claims, closed issues, duplicate subsystem lists, and newer corrections. Examples of drift observed here:

- An old Tree-sitter failure claim does not match the passing local unit and twelve-file grammar checks.
- GATE-2 says corpus sanitizing/ownership mutation run in no pipeline, while the current workflow invokes both.
- Old interface status prose does not describe version 3 or the current write-only boundary.
- T-4's method-alias allegation did not reproduce in the fresh tested case.
- Store-error discard prose conflicts with current tests and observed abort behavior.
- Formatter “still checks” claims do not establish preservation and fail in the debug profile on current `std/bit.jn`.

Use one status source per feature with links to executable acceptance criteria, affected version, and evidence date. Archive historical narratives instead of allowing them to compete with current contracts. Do not update all these documents during a review-only task; record the discrepancies and reconcile deliberately later.

## 15. Verification, CI, security, and performance claims

### 15.1 A green suite is valuable but not sufficient

The 2,420 passing release tests are meaningful evidence. They are not a soundness certificate. Fresh wrong-code programs outside their oracle coverage demonstrate the difference. The debug-only failure also shows that release-host wrapping behavior can conceal compiler implementation mistakes.

Keep both Rust profiles in verification for semantic boundary code. Add independent expected outputs, property-based numeric models, cleanup-event traces, and fault-injection outcomes. Optimization differential testing detects disagreement; it cannot detect shared wrong answers. Frontend checks detect rejection; they cannot detect backend corruption.

### 15.2 SOL-14 — sanitizer gate semantics need tightening

The workflow's runtime sanitizer job is `continue-on-error: true` (`.github/workflows/ci.yml:229-254`). Its comment says full suite, but the script executes nine smoke programs. The separate corpus sanitizer/ownership-mutation job is wired (`256-288`), contrary to the older roadmap statement.

`ci/sanitize-corpus.sh:105-143` reports compile failures, link failures, timeouts, signals, and nonzero exits. Its final hard-fail condition at `187-195` checks `CORRUPT` and optionally leaks, so an unexpected signal without the recognized ASan error text need not fail the job. Do not print “no memory corruption” as if unclassified crashes were exonerated.

Classify expected traps separately from unexpected compiler crashes, program signals, timeouts, and harness failures. Require an explicit expected outcome per negative test. Fail closed on unclassified failures. Enable leak gating for the subset claimed leak-free, with any residual exceptions individually named.

Instrument generated Jinn code with the appropriate LLVM sanitizer passes in addition to the C runtime, and preserve coroutine/fiber annotations. Run race tests for channel/select/scope cancellation and generated accesses, not merely the runtime's instrumented functions.

Several workflow jobs set `LLVM_SYS_210_PREFIX`, whereas the current `llvm21-1` binding uses `LLVM_SYS_211_PREFIX`; the benchmark/corpus jobs use the latter. Tool discovery may make some jobs work anyway, but the mismatch should be removed and tested in a clean CI environment. Do not infer guaranteed CI failure from that variable alone.

### 15.3 Security boundaries

The security policy correctly includes compiler miscompilation, runtime memory safety, and unsafe parsing/crypto behavior. The supported threat model should also separate trusted source, hostile input data, untrusted packages, FFI, and compiler/LSP resource exhaustion.

Safe-looking arithmetic producing poison is a compiler trust failure, even without a demonstrated exploit. Store durability warnings and static capability annotations must not be promoted into stronger security guarantees. Untrusted dependencies need enforced runtime/OS boundaries if sandboxing is promised.

Cryptographic wrappers require known-answer, negative-authentication, boundary-length, error-propagation, and constant-time implementation provenance checks. This review did not certify cryptographic security or audit third-party library internals.

### 15.4 Performance evidence

No new performance numbers are asserted. Existing corrected loop-count comparisons and normalized benchmark CI are useful. Retire or repair strawman concurrency/store baselines rather than averaging them into a headline.

Measure costs specific to Jinn's design: implicit string/value copies, view speedups, closure allocation, generator retention, checked-overflow elision, actor/channel contention, idle-connection worker exhaustion, transaction snapshot amplification, WAL growth/recovery, build latency, and editor recheck latency. Record workload, inputs, compiler flags, CPU, runs, distributions, and memory, and compare implementations doing equivalent work.

Deterministic memory management is not equivalent to bounded memory use, zero allocations, real-time scheduling, or bounded cancellation latency. Those are separate measurable properties.

## 16. Readiness by use case

| Use case | Current assessment | Principal exit condition |
| --- | --- | --- |
| Language experiments and compiler research | Strong fit now | Preserve explicit alpha expectations |
| Small trusted native CLI tools | Useful experimental fit | Close numerical/cleanup/formatter P0s; pin toolchain and outputs |
| Local scripts replaced by native programs | Plausible after core fixes | Reliable diagnostics, process/error semantics, packaging/cache correctness |
| Data parsing and transformation | Promising, not generally certified | Behavioral/hostile-input coverage, byte/Unicode contracts, numerical fixes |
| Long-lived services | Not ready for broad production claims | Nonblocking/cancellable I/O, leak/liveness gates, debugging and observability |
| Durable local applications using stores | Prototype/controlled deployment only | Fail-closed I/O outcomes, crash recovery, cancellation cleanup, bounded scaling |
| Distributed consensus/storage | Experimental | Multi-node correctness, network failure testing, durability model |
| Reusable binary libraries/plugins | Not yet a supported contract | Complete validated interface, ABI/layout identity, coherence and artifact model |
| Untrusted package execution/sandboxing | Not provided by current static caps | Real enforced policy and OS/runtime capability boundary |
| Hard real-time or safety-critical work | Not justified | Bounded latency/allocation/resource proofs and substantially stronger assurance |
| Cross-platform native development | Needs a declared tested subset | Target/runtime/FFI/debugger CI matrix |
| Embedded/freestanding/Wasm systems | Design/partial surfaces | Complete substrate, layout, linking, runtime, and target acceptance gates |
| GPU/mobile/browser UI ecosystems | No readiness claim established | Dedicated runtime/platform/FFI/tooling strategy after core stabilization |

A modern language need not implement every row itself. A reliable core, strong C interoperation, and clear platform boundaries can serve many ecosystems through libraries. Breadth should follow a trustworthy foundation.

## 17. Prioritized completion plan — recommendations only

### Gate A: restore basic semantic trust

Close SOL-01 through SOL-04 and SOL-15, then rerun all existing suites plus new independent oracles. Close SOL-05/SOL-06/SOL-07/SOL-16 as one coherent numeric/literal-contract workstream rather than treating them as unrelated output patches.

Acceptance:

- Every numeric width/signedness and cast family has boundary/property tests; no unchecked poison-producing conversion is reachable from accepted safe source.
- Builtin result signatures agree from typer through final LLVM values.
- Conditional/loop cleanup is dynamically correct on all exits.
- Formatter-only changes preserve normalized syntax and exact behavior on the arithmetic reproducers; writing is refused on unsupported forms/mismatches.
- Debug and release compiler profiles agree on accepted/rejected source and no host-overflow panic remains.
- Release build and Clippy remain warning-clean; both test profiles pass with deliberately documented ignores.

### Gate B: make state and shutdown dependable

Address SOL-09/SOL-10 and the associated cleanup integration before claiming durable or long-lived service readiness.

Acceptance:

- Inject short writes, failed flush/sync, ENOSPC, failed reopen/rename, corrupt tails, and cancellation at each transaction phase.
- Assert acknowledged commit durability under the selected policy and an explicit indeterminate-outcome contract where appropriate.
- Preserve recovery evidence until restore/commit is durable; poison unusable handles.
- Demonstrate lock release on cancellation and deadlock-free permitted multi-store acquisition.
- Specify single-store versus multi-store crash atomicity; test only the promises actually made.
- Add generated-code sanitizer coverage and leak-enforced long-running cleanup tests.

### Gate C: unify compiler contracts before stabilizing boundaries

Address SOL-11/SOL-12/SOL-13: resolved callable summaries, target-aware layout, trait-based conversions, complete effect/capability traversal, and a finite canonical hashing specification.

Acceptance:

- Missing layouts/types fail as compiler invariant errors, never as silent `i64` substitutions.
- FFI layout probes agree with generated layouts on every supported target.
- Public access/category/effect changes are represented in compatibility checks.
- Cyclic hash examples terminate by construction and serialize deterministically across insertion orders/processes.
- Source/content keys include all resolved inputs and detect prefix collisions.
- Interface artifacts remain write-only until lossless, validated reuse is demonstrably safe.

### Gate D: earn the supported native developer experience

Consolidate driver paths, complete useful debug information, strengthen project-aware LSP behavior and safe rename, harden formatter writes, and publish an honest target/stability matrix.

Acceptance:

- Compile/check/run/test/LSP agree on semantic diagnostics for shared project cases.
- A debugger can stop at a Jinn line and inspect a local in a tested native executable.
- End-to-end LSP sessions handle UTF-16, failures, imports, and cancellation without unsafe edits.
- Representative stable-stdlib functions have behavioral and malformed-input tests, not only import tests.
- Unexpected sanitizer/harness failures fail CI, and required gates cannot silently continue on error.

### Gate E: expand from a reliable base

Only after the previous gates: scalable cancellable I/O, bounded storage architecture, safe binary-library reuse, deterministic offline packaging, signing/registry rollout, additional targets, and any broader lamp/deployment product work.

Choose explicit non-goals per release. Reject unsupported syntax/types/targets with good diagnostics rather than accepting them optimistically. Maintain compatibility where documented, and change the spec and compile/run conformance cases deliberately when semantics change.

## 18. Final assessment

Jinn's most valuable work is already visible: a compact, inferred, value-oriented systems language that tries to remove ownership and error-handling ceremony without giving up native control. Its current constraints—second-class views, scope-bounded frozen sharing, explicit aggregate moves, and conservative rejection at difficult boundaries—are often good design, not shortcomings to erase.

The production gap is primarily **semantic closure**: every accepted construct must mean the same thing in the specification, typer, ownership analysis, MIR, LLVM, runtime, formatter, and tools. This review found several small programs where that chain breaks, even though the release suite is green. Close those breaks, make the gates observe the promised properties, and resist adding new trust surfaces until the old ones are dependable.

No fixes were implemented as part of this review. The existing working-tree changes were left intact.
