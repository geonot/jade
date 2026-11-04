# SOL review remediation plan

This is the execution index for `sol-review.md`, the historical review at revision
`1745166`. The review remains unchanged. Task root `17` contains five release gates
and 28 executable leaf tasks. Each leaf includes implementation, specification and
conformance work; completion of individual leaves does not establish production
readiness or complete a gate.

The task records in `.ryu/tasks/` carry priority and dependencies. This index keeps
the scope, acceptance criteria and verified execution state in the repository.
Dependencies on a gate apply to all its descendants. Gates A and B are complete
within the verified native scope below. Gates C and D remain open; Gate E requires
B, C and D. Existing implementation tasks are referenced, not duplicated.

## Gate A — restore basic semantic trust (`17-1`, high)

| Task | Report | Action and acceptance | Depends on | State |
| --- | --- | --- | --- | --- |
| `17-1-1` | SOL-01 | Replace static function defer lists with dynamically reached cleanup obligations. Specify capture timing, scope, LIFO, exactly-once execution, cleanup failure and cancellation. Trace conditional/zero-or-many-loop/nested cleanup across return, break, continue, propagation and cancellation; verify heap operands remain live. | — | Complete; verification below |
| `17-1-2` | SOL-02 | Implement exact width-aware checked integer exponentiation without floating intermediates. Specify zero/negative exponents. Test identities beyond 2^53, signed negative bases and overflow in all eight integer domains, including unused results. | — | Complete; verification below |
| `17-1-3` | SOL-03 | Replace rounded float-to-int guards and poison-producing conversions with saturating LLVM conversions. Independently check f32/f64 to all integer domains at both bounds and neighboring floats, fractions, negatives, NaN and infinities. | — | Complete; verification below |
| `17-1-4` | SOL-04, SOL-05 | Share integer builtin contracts across typer, MIR verification and codegen. Preserve the full checked tuple; use correct width/signedness and saturation direction. Check all nine operations in all eight domains against independent mathematical oracles; reject malformed signatures. | — | Complete; verification below |
| `17-1-5` | SOL-06 | Define strict casts by mathematical source/destination domains, including equal-width sign changes and float fraction/NaN/infinity policy. Check every domain pair and preserve unused traps; bit roundtrips are not the oracle. | `17-1-3` | Complete; verification below |
| `17-1-6` | SOL-07, SOL-16 | Preserve literal sign and magnitude until typed range validation. Support annotated u64 maximum and i64 minimum without host overflow or double-minus formatting; pin defaulting, annotations, casts and unary negation in debug/release and the std/bit.jn formatter corpus. | — | Complete; verification below |
| `17-1-7` | SOL-15 | Print expressions using precedence, associativity and operand position. Refuse writes unless normalized AST structure is preserved; fail closed on unsupported forms. Pin the arithmetic outputs 8, 8, -3 before/after formatting, floating grouping, casts, unary/calls/index/ternary/lambda and corpus idempotence. | — | Complete; verification below |
| `17-1-8` | Gate A | Run independent semantic oracles at Jinn opt0/3 using debug/release compiler hosts, all conformance suites, warning-clean release build and Clippy. Resolve rather than mask failures; document every retained ignore. | All other Gate A leaves | Complete; verification below |

## Gate B — dependable state and shutdown (`17-2`, high)

Requires Gate A.

| Task | Report | Action and acceptance | Additional dependencies | State |
| --- | --- | --- | --- | --- |
| `17-2-1` | SOL-09 | Specify recoverable, fail-stop and indeterminate persistence outcomes. Persist undo before mutation; acknowledge commit only after required data/sidecar/commit durability. Retain recovery evidence until restore is durable; poison failed handles. Inject short writes, flush/sync/ENOSPC/reopen/rename failures and corrupt tails at each phase. | — | Complete; verification below |
| `17-2-2` | SOL-10 | Integrate transaction cleanup with task unwind. Prove cancellation releases writer locks; choose deadlock-free permitted multi-store acquisition or reject unsupported cases. Specify return/error/trap/cancel/process/crash outcomes and single-store versus multi-store crash atomicity; stress opposite lock orders and worker counts. | `17-1-1`, `17-2-1` | Complete; verification below |
| `17-2-3` | Store/shutdown audit | Reject persisted fields without defined encoding; document sidecar durability, snapshot caps/cost and migration limits. Leak-gate long-running generator/closure/early-exit/task cleanup, failed channel payloads, actor drain, migrations and repeated stop/join. Generated instrumentation is tracked separately in `17-4-2`. | `17-2-1`, `17-2-2` | Complete; verification below |

## Gate C — unified compiler contracts (`17-3`, high)

Requires Gate A.

| Task | Report | Action and acceptance | Additional dependencies |
| --- | --- | --- | --- |
| `17-3-1` | SOL-11 | Establish one target-aware layout authority; distinguish ABI layout from storage encoding. Remove unknown-layout-to-i64 fallbacks. Probe closures, arrays, tuples, enums, views, sret and C signatures on supported targets; reject unsupported layouts explicitly. | — |
| `17-3-2` | SOL-12 | Replace cryptographic SCC fixed-point iteration in the design with finite canonical graph serialization. Define internal indices, external hashes, symmetry and nominal/API/ABI/object identity. Test insertion/process determinism; coordinate `2-32-4`. | — |
| `17-3-3` | SOL-12 | Hash a recursive path/length/content manifest plus resolved configuration/toolchain/dependency inputs. Define package interner lifetime and symbol-prefix collision policy. Test nested edits, partitioning, stale inputs, collisions and determinism. | — |
| `17-3-4` | SOL-13 | Resolve shared access/error/capability summaries by callable identity and instantiation, not AST name buckets. Propagate through methods, relations, matches, higher-order values and imports with conservative unknown/FFI effects. Pin public compatibility and both false-accept/false-reject cases; do not claim sandboxing. | — |
| `17-3-5` | Error-effects audit | Authorize implicit From conversions by trait provenance/coherence, not lookalike names. Test ambiguity/import/orphan boundaries and recursive/generic/multi-error SCC inference. Reconcile store-discard prose with actual fail-stop behavior without weakening it. | `17-3-4` |
| `17-3-6` | Interface/security audit | Keep interfaces write-only until resolved types, generic bodies, access/effects, layout, target and toolchain contracts can be losslessly validated. Reject stale/corrupt reuse; specify static caps versus OS confinement, handle-relative filesystem policy and bind ABI/coherence boundaries. | `17-3-1`, `17-3-2`, `17-3-4` |

## Gate D — supported native tooling and credible CI (`17-4`)

May proceed independently of Gate A except where noted.

| Task | Report | Action and acceptance | Dependencies |
| --- | --- | --- | --- |
| `17-4-1` | SOL-08 | Carry spans and shared-registry suggestions through method resolution; use structured phase diagnostics instead of parsing display text where touched. Test path/line/column, stderr, non-TTY and exit classes. Publish meaningful revision/runtime/interface version information. | — |
| `17-4-2` | SOL-14 | Make unexpected compile/link/crash/signal/timeout/harness outcomes fail CI with explicit negative-test expectations. Remove required continue-on-error, correct smoke labels and LLVM_SYS_211_PREFIX. Instrument generated LLVM accesses and runtime/fibers; leak-gate a named subset and stress races/cancellation. Test the classifier itself. | — |
| `17-4-3` | SOL-17 | Emit native function/instruction/local debug information. Verify debug sections and a debugger stopping at a Jinn line, inspecting a local and unwinding. Test direct/project parity; distinguish coroutine support. | — |
| `17-4-4` | Driver audit | Share semantic phase orchestration across compile/project/run/cache/check/test/library/interface/LSP, with command-specific edges only. Fast checks may omit codegen, not safety validation. Pin matching ownership/effect/type diagnostics; coordinate task `12`. | — |
| `17-4-5` | LSP audit | Resolve actual projects/packages and preserve binder/source identity through generics. Refuse ambiguous cross-file rename; support cancellable incremental checking. Exercise stdio lifecycle, UTF-16 edits, imports, diagnostics, errors, cancellation and safe rename end to end. | `17-4-4` |
| `17-4-6` | Stdlib/FFI audit | Add stable-module behavioral and malformed-input tests, Unicode scalar/byte boundaries, crypto known-answer/authentication/length/error cases and C ABI probes. State fast-math/deterministic-FP and consensus limits; import success alone is not acceptance. | Gate A |
| `17-4-7` | Documentation/readiness audit | Publish one current status/evidence source and archive stale Tree-sitter, GATE-2, interface and T-4 allegations. Reconcile store policy, syntax/precedence/evaluation/binders/nominal aliases and ownership. Declare the tested native target/runtime/FFI/debug/stability matrix, threat model and alpha non-goals. | — |

## Gate E — expand from the verified native core (`17-5`, low)

Requires Gates B, C and D. These are integration/acceptance tasks over existing work,
not permission to expand the supported surface before its prerequisites pass.

| Task | Action and acceptance | Existing work |
| --- | --- | --- |
| `17-5-1` | Deliver reactor or explicit blocking-pool I/O, cancellation-safe wait deregistration and bounded behavior under idle/slow connections and cancellation storms. Tie actor fallibility/supervision to ownership and structured scopes; pin worker exhaustion. | `13` |
| `17-5-2` | Replace whole-store scaling with bounded undo/WAL/checkpoint architecture and typed query/migration IR. Specify portable encoding/schema identity and restartable durable migration progress; measure amplification, recovery and sync tails. | `2-16` |
| `17-5-3` | Deliver deterministic resolution and locked offline source integrity/cache correctness with hermetic provenance. Require validated lossless interfaces before binary/generic reuse; specify signing, revocation, rollback and namespace policy before registry rollout. Configuration remains deterministic data. | `2-19`, `2-32` |
| `17-5-4` | Benchmark equivalent work with distributions, memory, flags and environment across values/views/closures/generators/arithmetic/actors/stores/build/LSP. Require target/runtime/coroutine/FFI CI before additional-platform claims; state freestanding/UI/deployment non-goals and rollback limits. | `8-26` |

## Execution record — numerical batch

The first implementation batch addresses `17-1-2`, `17-1-3` and `17-1-4`
(SOL-02 through SOL-05):

- Integer power uses checked exponentiation by squaring, without float conversion
  or an unnecessary final square. Exponent zero returns one, including zero to the
  zero power; negative integer exponents and overflow trap.
- Ordinary float-to-integer casts use LLVM saturating conversion intrinsics, so
  NaN, infinity and rounded upper bounds cannot introduce conversion poison.
- A shared integer arithmetic table defines source/builtin operation and result
  contracts for typing, MIR verification and codegen. Checked arithmetic returns
  the complete wrapped-value/overflow tuple. Saturation respects mathematical
  direction, actual width and signedness.
- MIR dead-code elimination retains potentially trapping addition, subtraction,
  multiplication and power even when the result is unused.
- `docs/jinn.md` pins these numerical contracts. `tests/sol_numeric_contracts.rs`
  checks independent exact outputs at opt0/3, and MIR unit tests reject malformed
  builtin contracts. The systems-demo snapshot now expects true for overflow of
  i64 maximum plus one; its previous false expectation encoded the reviewed bug.

Verification on 2026-10-29:

| Check | Result |
| --- | --- |
| `cargo build --release` | Pass, no Rust warnings |
| `cargo clippy --release --all-targets -- -D warnings` | Pass |
| `cargo test --release --no-fail-fast` | 2,436 passed, zero failed, 15 ignored |
| `cargo test --no-fail-fast` | 2,435 passed, one failed, 15 ignored |
| New numeric conformance suite | All 15 pass with both debug/release compiler hosts; each output/trap oracle runs at Jinn opt0 and opt3 |
| MIR intrinsic contract test | Pass in both profiles; malformed arity, operand domains and result shapes reject |
| Access, concurrency shutdown, stable stdlib and program snapshots | Pass in both full runs |
| Task index and dependency validation | All 34 records and 17 findings indexed; 28 leaves; acyclic, resolved dependencies |
| Targeted Rust formatting and patch whitespace | Pass for this batch |

The sole debug failure is the already-reviewed
`fmt_nondestructive::fmt_output_still_frontend_checks_over_corpus` on
`std/bit.jn` (SOL-16, task `17-1-6`). The test remains enabled and failing; it is
not ignored or relabeled as passing. The 15 existing ignores are unchanged:
14 integration cases await the task `2-4-7` migration from deprecated prefix-`!`
error syntax, and `store_perf_regression` is an explicitly ignored timing benchmark.
The incoming extra blank EOF line in `src/driver/sources/modules.rs` still causes
global formatting/whitespace checks to fail and was deliberately not modified.

All seven incoming modified files remain byte-for-byte unchanged relative to their
initial diff. No commit or version increment was made, avoiding accidental capture
of that work. Strict casts, literal representation, formatter preservation and
dynamic cleanup remain open; this batch does not close Gate A. Next work is the
coherent literal/formatter repair (`17-1-6`, `17-1-7`), strict casts (`17-1-5`) and
dynamic cleanup (`17-1-1`), before gate-wide acceptance (`17-1-8`).


## Execution record — literal boundary batch

The second implementation batch completes `17-1-6` (SOL-07 and SOL-16):

- Lexer tokens and AST literals retain unsigned `u64` magnitudes; unary minus
  remains a separate operation. Typing uses mathematical `i128` values and checks
  literal obligations after inference, before accepting typed HIR. This preserves
  `u64` maximum, validates the full `i64` domain and prevents host negation overflow
  without enlarging every AST expression for a signed 128-bit payload.
- Unconstrained integer literals still default to `i64`. Explicit casts accept a
  positive magnitude above `i64` maximum as a `u64` source, allowing deliberate
  low-bit conversion without silently wrapping annotations. Decimal, hexadecimal,
  octal and binary spellings obey the same mathematical bounds.
- Integer-to-float adaptation and constant folding retain unsigned magnitude.
  Signed minimum negation remains a runtime trap, including nested negation and
  unused results; compile-time folding and MIR elimination no longer erase it.
- Store migration defaults use the shared mathematical integer ranges and preserve
  `u64` maximum and `i64` minimum across migration and reopen. Oversized enum
  discriminants and migration versions reject rather than reinterpreting bits.
- `std/random.jn`, the lattice-crypto app and the crypto program fixture now cast
  their intentional signed 64-bit bit patterns explicitly. C compatibility limit
  constants use explicit `u64` casts for unsigned maxima. Only those constants are
  exercised here; this is not a compatibility-module import or alias-syntax claim.
- Nine additional numeric conformance tests cover every integer domain and base,
  annotations, arguments, returns, inference, generic calls, fields, arrays,
  explicit casts, float adaptation, diagnostics, unary traps and formatting.
  Lexer magnitude and migration/reopen tests add two more cases. Formatter CLI
  writes and repeated formatting preserve extreme literal signs and values.
  General precedence/structural preservation is still task `17-1-7`; `src/fmt.rs`
  is unchanged by this batch.

Verification on 2026-10-29, superseding the earlier batch's outstanding failure:

| Check | Result |
| --- | --- |
| `cargo build --release` | Pass, no Rust warnings |
| `cargo clippy --release --all-targets -- -D warnings` | Pass |
| `cargo test --no-fail-fast` | 2,447 passed, zero failed, 15 existing ignores |
| `cargo test --release --no-fail-fast` | 2,447 passed, zero failed, 15 existing ignores |
| Numeric conformance suite | All 24 pass in both host profiles; output/trap oracles run at Jinn opt0 and opt3 |
| Formatter frontend corpus, including `std/bit.jn` | Pass in both profiles; prior sole debug failure resolved |
| Access, concurrency shutdown, stable stdlib, app execution, program snapshots and opt-level corpus differential | Pass in both full runs |
| Targeted Rust formatting and patch whitespace | Pass for owned changes; incoming EOF whitespace left intact |

No failures were ignored or reclassified. The 15 inherited ignores remain those
recorded above. Test/build/lint wrappers report no configured command despite the
local configuration, so the corresponding Cargo commands were run directly.
Verification logs are `/tmp/jinn-sol-literal-final-{debug,release,build,clippy}.log`;
all four recorded exit statuses are zero. Earlier exploratory failures exposed
intentional signed bit-pattern constants; those sources were migrated rather than
weakening literal validation or changing snapshots.

All seven incoming file diffs retain SHA-256
`e868d323bbbd96922e598f080df0d750d2983672438550533262e167df52e5cf`.
No commit, staging or version increment was made. Task completion was recorded
without the task tool's automatic save side effect. Gate A remains open for
dynamic cleanup, strict casts, structural formatter preservation and gate-wide
acceptance. The next focused task is `17-1-7`.

## Execution record — formatter preservation batch

The third implementation batch completes `17-1-7` (SOL-15):

- Expression printing follows parser precedence, associativity and operand
  position. It preserves nested subtraction/division, unary operands, right-hand
  floating grouping, power associativity, comparison grouping, casts, callable
  and postfix receivers, conditional arms, pipes, references and slices.
- Both stdout and write modes require exact AST equality after replacing token
  spans with a common empty span. The comparison retains every other AST field;
  it does not use debug text, hashes, frontend acceptance or semantic rewrites.
  Unsupported printer forms fail before any candidate is emitted or written.
- The formatter shares the compiler parser but stops before top-level-main
  synthesis and multi-clause-function merging. Other parser desugarings remain
  shared. The compiler's existing lowered parse path is unchanged.
- The guard exposed existing corpus losses, which were repaired rather than
  skipped: function attributes, generic bounds, capability annotations, all
  parameters including `self`, enum discriminants, atomic bindings, loop access
  modes, synchronous actor handlers and parser-generated control-flow forms.
  Unit and single-element tuple syntax also retain their distinct AST shapes.
- Nine new integration tests include independent execution outputs before and
  after formatting at Jinn opt0/3, the `8, 8, -3` arithmetic oracle, a floating
  non-reassociation witness, an 882-case binary-grouping matrix, prefix/cast/
  postfix/handler cases, metadata and parser-desugared control flow. Six valid
  but unsupported printer forms must refuse in both CLI modes without output
  or changed source bytes. Every app project is formatted and checked with its
  imports resolved before and after; repeated formatting must be idempotent.
- Two new formatter unit tests directly reject parseable changed trees and pin
  preservation of real stdlib/parser-desugaring cases. Existing formatter corpus
  checks remain enabled with their original acceptance criteria and exclusions.
  `docs/design/fmt-and-lint.md` states the shipped preservation contract and the
  remaining unsupported syntax/tooling; grammar completeness is not claimed.

Verification on 2026-10-30:

| Check | Result |
| --- | --- |
| `cargo build --release` | Pass, no Rust warnings |
| `cargo clippy --release --all-targets -- -D warnings` | Pass, no findings |
| `cargo test --no-fail-fast --quiet` | 2,458 passed, zero failed, 15 existing ignores |
| `cargo test --release --no-fail-fast --quiet` | 2,458 passed, zero failed, 15 existing ignores |
| New formatter contracts | All nine pass in both host profiles; execution oracles compile original/formatted source at Jinn opt0 and opt3 |
| Binary expression grouping and idempotence | All 882 combinations pass in both host profiles |
| Existing formatter corpus and comment/shebang checks | All seven pass in both profiles, including `std/bit.jn`; no acceptance changes |
| App formatting in resolved project context | All 20 app projects pass before/after frontend checks and source idempotence |
| Numeric conformance suite | All 24 pass in both host profiles, retaining opt0/3 output and trap checks |
| Access, concurrency shutdown, stable stdlib, app execution, program snapshots and opt-level corpus differential | Pass in both full runs |
| Task graph, targeted Rust formatting and patch whitespace | Pass; 34 records, 28 leaves, acyclic/resolved dependencies; incoming EOF whitespace untouched |

The full test logs are `/tmp/jinn-sol-formatter-final-{debug,release}.log`, with
both companion `.exit` files recording zero. The dedicated build and lint tools
now execute the configured Cargo commands successfully. No failures were ignored
or reclassified; the 15 inherited ignores remain the 14 deprecated error-syntax
cases and the store timing benchmark recorded above.

All seven incoming file diffs retain SHA-256
`e868d323bbbd96922e598f080df0d750d2983672438550533262e167df52e5cf`.
No commit, staging or version increment was made, and task status was recorded
without automatic save. Gate A still requires strict casts (`17-1-5`), dynamic
cleanup (`17-1-1`) and gate-wide acceptance (`17-1-8`). The next focused task is
strict casts; the formatter's remaining grammar coverage is explicitly fail-closed,
not a production-readiness claim.

## Execution record — strict numeric cast batch

The fourth implementation batch completes `17-1-5` (SOL-06):

- Strict casts accept only numeric source and destination domains and preserve
  mathematical value, not just low bits. Equal-width signed/unsigned conversions
  reject negative or out-of-range inputs. Integer-to-float checks use a signed
  128-bit mathematical comparison, avoiding the rounded `u64` maximum alias.
- Float sources must be finite, integral for integer destinations, in the exact
  destination range and exactly representable. Equal-width float casts reject
  NaN and infinity too. Signed zeros compare numerically equal. Ordinary casts
  retain the poison-free saturation contract from the earlier batch.
- Source typing constrains inference to numeric types. HIR and MIR validation
  reject malformed strict-cast domains/result types. Unused failures still trap.
- Five new integration tests independently check all 100 numeric domain pairs,
  integer boundaries, neighboring floats, 24/53-bit precision transitions,
  fractions, nonfinite values, equal-width conversions and unused traps at opt0/3.
  A MIR unit test rejects malformed contracts.

Verification on 2026-10-30: warning-clean release build and all-target release
Clippy with `-D warnings` pass. Both full debug and release runs pass 2,464 tests,
zero failures, with the same 15 inherited ignores. All 29 numeric conformance tests
pass in both compiler-host profiles. Full logs are
`/tmp/jinn-sol-strict-final-{debug,release}.log`; all 60 suite summaries per profile
report success. The test wrapper's 120-second foreground timeout required an
asynchronous release run and inspection of the completed logs; it did not justify
ignoring a test. Targeted Rust formatting passes. The incoming seven-file diff
retains its recorded SHA-256. No staging, commit or version increment was made.
Gate A remains open for dynamic cleanup and gate-wide acceptance.

## Execution record — dynamic cleanup and Gate A acceptance

The fifth batch completes `17-1-1` and `17-1-8`. Gate A is complete.

- Executing `defer` registers a runtime obligation in its callable's frame.
  Actions run in reverse registration order, once, at return, normal completion,
  propagated error or cooperative task cancellation. Unreached branches register
  nothing; repeated loop entries register distinct actions and lexical cells.
- Captured bindings use stable typed cells and observe exit state. Transitive
  owners of captured views remain live until actions and their environments have
  been released. Returning a noncopyable owner reserved by cleanup rejects rather
  than creating a dangling returned value. Cleanup control flow is validated
  recursively; nested callable and local loop control remain legal.
- Runtime cleanup shields cancellation while actions run, then releases cells.
  The language and memory/concurrency contracts document LIFO, failure and
  cancellation semantics. Runtime traps remain fail-stop, not general unwinding.
- Independent tests exposed and fixed closure identity collisions, incorrect
  pointer-to-aggregate parameter handling, missing generator coroutine destruction,
  temporary iterator ownership on normal completion, scheduler deque retention,
  actor join retention and task environment retention. The closure snapshot now
  records the mathematical results instead of prior miscompiled outputs.
- `tests/sol_cleanup_contracts.rs` adds 29 contracts, including negative ownership
  and nested-control cases. Every positive output oracle runs at Jinn opt0/3.
  Suspended-generator abandonment and general early-exit resource retention remain
  explicit Gate B shutdown work, not claims made by this cleanup acceptance.

Verification on 2026-10-30:

| Check | Result |
| --- | --- |
| Full debug suite | 2,493 passed, zero failed, 15 inherited ignores |
| Full release suite | 2,493 passed, zero failed, 15 inherited ignores |
| Release build | Pass, no Rust warnings |
| Release all-target Clippy, `-D warnings` | Pass, no findings |
| Numeric/cleanup/formatter contracts | All 29/29/9 pass in both host profiles; independent execution at Jinn opt0/3 |
| Access, concurrency shutdown, stable stdlib, apps, program snapshots, opt-level differential | Pass in both full runs |
| Generated LLVM and complete runtime ASAN/LSAN | All 56 positive cleanup runs pass, opt0/3, leak detection enabled |

Final logs are `/tmp/jinn-sol-cleanup-final-{debug,release,build}6.log`.
All full-suite summaries report success and the Cargo processes completed.
The successful dedicated lint invocation completed after the last source edit;
previous timeout or error summaries were not treated as success. The sanitizer
runner is `/tmp/jinn-cleanup-asan.py`, with results in
`/tmp/jinn-sol-cleanup-asan5.log`. It instruments every generated function and all
runtime C files plus the fiber-aware context switch. Generated access checks were
independently confirmed in emitted LLVM. This local runner is not durable CI;
that integration remains `17-4-2`. Host Valgrind could not run against this glibc
and is not cited as evidence. No zero-allocation/static-cleanup claim is made.

The 15 ignores remain exactly the 14 deprecated error-syntax cases and the store
performance benchmark recorded above. No failures were ignored or reclassified.
The seven incoming file diffs retain SHA-256
`e868d323bbbd96922e598f080df0d750d2983672438550533262e167df52e5cf`.
No staging, commit or version increment was made. Gate B was next at this
acceptance point; its subsequent completion is recorded below. Gates C through E
remain open and neither acceptance establishes production readiness.

## Execution record — dependable state and shutdown

Gate B (`17-2-1` through `17-2-3`, SOL-09 and SOL-10) is complete within the
bounded Linux native contract. Gate A remains verified by both final full runs.

- Checked store, WAL and sidecar I/O fails closed instead of warning and continuing
  with a reusable failed handle. Constraint errors retain their ordinary
  `StoreError` behavior; persistence, allocation, snapshot and recovery-integrity
  failures terminate without success acknowledgement. Post-replacement directory
  sync and reopen failures are fail-stop, not recoverable mutation results.
- A directory-level undo journal persists each complete participant image before
  mutation. Managed opens recover it before opening store, WAL and sidecar state.
  Commit syncs participants and WAL decisions before journal retirement; rollback
  restores and syncs every participant before retirement. Recovery validates all
  complete records before restoring any participant, tolerates an interrupted
  append tail, preserves corrupt evidence, and can retry interrupted restoration.
- The same atomic recovery contract covers multiple stores, WAL, key/value, bloom,
  column, vector, full-text, index and version participants. The fault matrix
  injects short writes/ENOSPC and exits immediately before or after every reached
  write, flush, file/data sync, reopen, rename, truncate and unlink boundary in
  commit and rollback. Reopen must expose the whole old or whole new state when
  commit acknowledgement was absent, and the acknowledged state otherwise.
  Corrupt headers/checksums/paths and truncated journal tails have separate oracles.
- Transaction state follows the executing coroutine. A process-wide reentrant
  writer gate serializes transactions and ordinary mutations; opposite multi-store
  acquisition orders no longer deadlock or permit rollback over another writer.
  Cancellation rolls back and releases writer ownership before reached user
  cleanup, including a deferred action that starts and commits a new transaction.
  Blocking channel/select operations, joins and generator switches while holding
  transaction ownership fail-stop; lexical yield rejects during compilation.
- Cancellation checkpoints cover function entry and loop backedges, including
  helper calls and generators. Nested scopes settle before their parent cleanup.
  Destroying an unstarted generator never enters its body; destroying a suspended
  generator resumes only to unwind reached cleanup and release owned captures.
  Cleanup is cancellation-shielded. Named consuming callable/generator parameters
  transfer ownership; unsupported consuming closure-literal parameters and borrowed
  move-only generator retention reject rather than aliasing an owner.
- Typed channel destruction releases buffered payloads, rejected sends release their
  inputs, and actor shutdown drains or releases pending messages and frees heap
  state before join acknowledgement. Retired mailbox handles remain immutable and
  valid for repeated stop/join and rejected post-join sends until runtime shutdown.
  Select arms have isolated lexical binders, typed receive storage and owning
  cleanup, including unbound selected payloads and cancellation after a receive.
- Ownership verification now normalizes dead/duplicate PHIs and mixed borrowed/
  owned inputs, retaining borrowed inputs only on their incoming edge. Branch and
  loop string owners, moved closures and early task exits release exactly once;
  real URL parsing/percent-encoding probes additionally check the affected paths.
- Fiber-aware TSAN found actual runtime races, which were fixed without
  suppressions: coroutine state is atomic; cancelled wait-channel access uses a
  synchronized retained snapshot; deque publication carries explicit release/
  acquire synchronization; published actor mailbox channels are immutable; and
  generator TLS is fetched/set through non-inlined helpers after fiber migration,
  never through a cached address belonging to the previous worker.
- Field-encoding validation is shared by typing, HIR validation and native lowering.
  All eight integer domains, f32/f64, bool and String commit/reopen with exact
  independent outputs. Aggregate, pointer, tuple, enum and callable fields reject
  in check and opt0/3 native modes, including `@mem` stores. Invalid or exceeded
  snapshot caps refuse before mutation and preserve reader-visible state.
  `docs/jinn.md`, `docs/concurrency.md`, `docs/memory-model.md` and
  `runtime/README.md` specify the outcome and supported ownership boundaries.

Final verification on 2026-10-30, after all implementation and conformance edits:

| Check | Result |
| --- | --- |
| `cargo build --release` | Pass, no Rust or C warnings |
| `cargo clippy --release --all-targets -- -D warnings` | Pass, no findings |
| `cargo test --no-fail-fast --quiet` | 2,523 passed, zero failed, 15 inherited ignores |
| `cargo test --release --no-fail-fast --quiet` | 2,523 passed, zero failed, 15 inherited ignores |
| Persistence contracts | All eight pass in both host profiles, including the complete fault matrix; native output/refusal/reopen oracles run at Jinn opt0/3 |
| Shutdown contracts | All 20 pass in both host profiles; every positive native oracle runs opt0/3 with 1, 2 and 4 workers and bounded timeout |
| Access, concurrency shutdown, stable stdlib, apps, program snapshots, opt-level differential | Pass in both full runs |
| Generated LLVM and complete runtime ASAN/LSAN | All 90 cleanup/shutdown runs pass, opt0/3, leak detection enabled |
| Generated LLVM and complete runtime fiber-aware TSAN | All 102 shutdown runs pass, opt0/3 and 1/2/4 workers, no suppressions |
| Migration ASAN/LSAN | 40 real schema migrations and 80 subsequent reopens pass at opt0/3, 160 instrumented processes including seed runs |
| URL ASAN/LSAN probes | All four parsing/Unicode percent-encoding runs pass, opt0/3 |
| Task graph and owned Rust formatting | 34 records, 28 leaves, resolved acyclic dependencies; all 93 owned Rust files pass targeted formatting |

The full final logs are `/tmp/jinn-gate-b-final-debug2.log` and
`/tmp/jinn-gate-b-final-release.log`; all 63 test-suite summaries in each report
success and both Cargo processes completed. The final build log is
`/tmp/jinn-gate-b-final-build2.log`; the dedicated final lint invocation also
completed cleanly. Earlier probe failures and sanitizer race reports were
reproduced and corrected, not ignored or reclassified. The same 15 inherited
ignores remain the 14 deprecated error-syntax cases and store timing benchmark.

Sanitizer results are `/tmp/jinn-gate-b-final-asan2.log`,
`/tmp/jinn-gate-b-final-tsan6.log`,
`/tmp/jinn-gate-b-final-migration-asan2.log` and
`/tmp/jinn-gate-b-final-url-asan.log`. The local runners are
`/tmp/jinn-cleanup-asan.py`, `/tmp/jinn-shutdown-tsan.py`,
`/tmp/jinn-migration-asan.py` and `/tmp/jinn-probe-asan.py`. They instrument the
complete runtime and generated LLVM functions, including fiber-aware context
switches. These are local verification results, not a claim that durable CI
integration (`17-4-2`) is complete or that all possible schedules are race-free.

Persistence remains limited to one cooperating process, ordinary unaliased files
directly in the initialized working directory, and functioning Linux rename and
file/directory synchronization. It is not a hostile-filesystem sandbox. Writer
serialization is not concurrent-reader snapshot isolation. Snapshot memory and
journal space remain O(total participant bytes), with a default 256 MiB per-file
preimage cap and additional WAL undo images. Storage uses native ABI/endianness;
String fields truncate beyond 248 payload bytes. Interrupted migrations refuse
open rather than promising automatic restartable progress. Scalable bounded-page
undo, portable encoding and restartable migration remain Gate E work.

All seven incoming file diffs retain SHA-256
`e868d323bbbd96922e598f080df0d750d2983672438550533262e167df52e5cf`.
The only global patch-whitespace finding is the preserved incoming blank EOF line
in `src/driver/sources/modules.rs`. HEAD remains `1745166`, VERSION remains `191`,
and no staging, commit, automatic save or version increment was performed.
Gate B task status was recorded only after verification, without the task tool's
automatic save side effect. Gates C, D and E remain open; completing A and B is
not a general production-readiness declaration.

## Execution record — target-aware layout authority (`17-3-1`)

`size of T` and `size_of(T)` now lower to `hir::ExprKind::SizeOf` and
`mir::InstKind::SizeOf`, and codegen answers them from LLVM `TargetData`
store size of the lowered type. The duplicated field-count-times-eight tables in
the typer and the dead `CompTimeTypeOf`/`FieldsOf`/`SizeOf` builtins are gone.
Unknown or non-concrete operands are compile errors rather than i64-sized
guesses, and `validate_native_program` checks every `SizeOf` operand. Validation
now runs after actor state structs are declared, so actor layouts are covered.

`@packed` and `@align(N)` are rejected by the typer
(`check_native_layout_attrs`) because codegen never honoured them. Silently
emitting natural layout under a packed or aligned contract was a layout lie.
`docs/memory-model.md` documents the authority and the rejection. Real packed
and custom-aligned storage is follow-up work.

Acceptance exposed three latent faults, all fixed at the root:

- The success value of a fallible function was not unified with the declared
  success type, so a `String` tail in `returns i64 ! E` caused an ICE. It is now
  a typed diagnostic. `tests/programs/error_handling.jn` was corrected, with
  unchanged output.
- An `@resource` payload moved out of an escaping enum value was cloned and
  then dropped twice, closing the fd twice. Move-only payloads (transitive
  `@resource`) now transfer via `__builtin_PayloadMove`, which consumes the owner
  in strict transfer checking. Mixed-ownership phis skip move-only values.
  Known limit: sibling heap fields in the same variant as a moved resource may
  leak; this case is not exercised.
- A void tail call in a unit fallible function zero-filled an i64 result slot
  and caused an i64-to-i8 coercion ICE. The fallback now uses the lowered void
  representation.

New integration tests: `size_of_uses_native_target_layout`,
`size_of_unknown_type_is_rejected`, packed and align rejection,
`fallible_success_value_must_match_declared_type`,
`resource_payload_moved_out_of_result_in_loop_is_dropped_once` and
`void_call_as_tail_of_unit_fallible_function`.

The release build is warning-clean. `cargo clippy --release --all-targets -- -D
warnings` is clean. The full release suite and the full debug suite each report
2541 passed, 0 failed and 15 ignored, the same inherited ignores. A Clippy
`nonminimal_bool` fix to the success-value check, behaviour-preserving, came
after the full runs. It was rebuilt warning-clean and the affected integration
tests were rerun (10 passed). All seven
incoming file diffs retain SHA-256
`e868d323bbbd96922e598f080df0d750d2983672438550533262e167df52e5cf`. HEAD
remains `1745166`, VERSION remains `191`, and nothing was staged, committed or
saved. Gate C tasks `17-3-2` through `17-3-6` remain open.

## Execution record — finite identity hashing (`17-3-2`)

`src/identity.rs` replaces the cryptographic SCC fixpoint with a single-pass
scheme. An iterative Tarjan condenses the graph. Each node is hashed once over
a rooted breadth-first serialization of its SCC that is restricted to the SCC
and follows declared edge order, with length-prefixed labels, local indices for
internal edges and 32-byte digests for completed or external edges. Rung and
target are BLAKE3 derive-key domain separators. `docs/design/identity-contracts.md`
is normative and defines the nominal/api/abi/object ladder, symmetry, the
FFI-escaping exclusion from object dedup, and the fact that equal ABI never
licenses merging. Twelve unit tests pin insertion independence, a hand-built
process-stable vector, edge-order significance, symmetry, cycle shape,
propagation through cycles, compositionality via external digests, label
boundaries, rung/target separation, fail-closed errors, a 200k-deep chain, and a
2000-node ring. The incoming `compiler-prereqs.md` is untouched and still
byte-identical. Populating the graph from resolved types is `2-32-4`.

## Execution record — source manifests and package keys (`17-3-3`)

`src/source_manifest.rs` replaces flat, boundary-free source concatenation.
The scan is recursive and records sorted relative UTF-8 paths, lengths and
BLAKE3 content hashes. It fails closed on symlinks, I/O errors, invalid paths
and more than 65,536 files. Package `semantic_hash` (v2) takes the manifest
digest. The root package hashes exactly its resolved `use` closure, and
`flatten_workspace` now receives that root `PkgId` instead of re-deriving a
second, different root identity. Workspace resolution errors are fatal
instead of being silently dropped. The package interner key is the full
record (hash and visibility included). Symbol prefixes are 128-bit, and
`check_mangle_prefixes` rejects collisions between distinct packages. The
`jinn run` cache is keyed by BLAKE3 over the entry, toolchain, target options
and tree manifest. Its `.deps` sidecar is a `jinn-sources v1` manifest that is
rejected when empty, unparsable or stale. The policy is specified in
`docs/design/identity-contracts.md`.

Tests: six manifest unit tests cover nested edits, partition/rename,
location independence, fail-closed scanning, stale round-trip and duplicates.
New interner/collision unit tests, an updated mangle-length test and the
integration test `run_cache_tracks_imported_module_edits` were added. The
release and debug suites each report 2562 passed, 0 failed and 15 ignored.
`cargo clippy --release --all-targets -D warnings` is clean. The incoming
seven-file diff SHA is `e868d323…` again. Running rustfmt on
`src/driver/mod.rs` had recursively reformatted `sources/modules.rs`, and the
removed trailing blank line was restored.

## Execution record — resolved callable capability authority (`17-3-4-1`)

The capability pass no longer scans AST declarations. It runs after HIR type
checking, method selection, generic instantiation and display synthesis have
finished, over the resolved program. Graph nodes are `DefId`s, not spellings.
Direct calls use the resolved definition; methods use the selected lowered
symbol so two receivers that share a method name no longer share a row; actor
handlers are keyed by actor identity and handler tag, so identical handler
names on different actors stay disjoint.

`hir::Fn` carries the declared `needs` annotation through lowering, which is
what lets the check happen after resolution instead of before it. An
uninstantiated generic therefore keeps its annotation for validation but has no
executable row, and its unknown capability classes are still rejected. When a
generic is instantiated, the instantiated body is checked against the original
ceiling.

The fixpoint is a monotone worklist over a finite lattice: the union of
primitive rows. Intrinsic store effects are enumerated at their `DefId`s rather
than inferred from rendered names, including KV methods, relation traversal
targets, delete cascades and insert/delete hooks. Restore is deliberately not a
cascade and does not run delete hooks, which is now pinned. Implicit display,
implicit resource drops (transitively through fields), global loads,
global initialisers and parameter defaults all participate. Extern leaves stay
with `cap_sites.rs`; unclassified externs, `syscall` and `asm` fail closed to
`ffi.unsafe`. Std aperture signatures are matched on the resolved `DefId` *and*
on the definition originating in a vetted std file, so a user function that
merely shares an aperture's spelling gains no authority. Diagnostics name a
shortest introducing call path.

What remains explicitly incomplete, and is not claimed:

- Higher-order capability precision (`17-3-4-2`). Function references and
  lambda bodies are charged at construction, and an unresolved indirect
  invocation rejects every finite ceiling. A callable stored in a value does
  not yet carry an effect row in its type.
- Default expressions are conservatively charged to the callee even when the
  caller supplies the argument.
- Shared frozen callable contracts for interface reuse (`17-3-4-3`).

`docs/design/compiler-prereqs.md` is updated to state exactly this: what is
live, what is conservative, and that none of it is a runtime sandbox or a
complete purity proof. Filesystem scope comparison remains lexical, and vetted
extern and aperture classifications remain trusted boundaries.

`tests/sol_callable_caps.rs` adds 31 integration pins. Thirteen are run cases
checked at `-O0` and `-O3` so a regression cannot hide behind optimisation,
covering disjoint method spellings, disjoint actor handlers, receiver names
that look like std modules, selected-only generic dispatch, benign externs,
recursion to a fixed point, restore without cascades, and enum formatting that
does not reach payload `display`. Eighteen are diagnostic cases covering
unclassified externs, `clock`, scoped paths, unreachable branches, `syscall`,
lambda and match-bound callables, defaults, global state, KV and relation
access, both insert lowerings, delete cascades, implicit display and drop,
method `needs`, a faked std spelling, and unknown capabilities in generics that
are never instantiated. The eighteen AST-level unit tests that exercised the
removed bucket scanner were retired with it; the integration file is the
replacement and is strictly stronger, because it sees resolved identities.

One defect surfaced by the new authority is filed rather than papered over.
With `use io` in scope, a top-level `*read` collides with the std extern
`read`: MIR verification reports that `io_read_stdin` calls `read` with three
arguments where the callee takes one. MIR `InstKind::Call` carries only a
symbol, and codegen registers externs under their bare C name, so extern and
user identity are not yet distinguished downstream of HIR even though HIR has
the `DefId`. The pin for that case uses a differently spelled user function so
the capability semantics stay tested; the collision itself is `18`.

Verification for this batch: release build warning-clean; `cargo clippy
--release --all-targets -D warnings` clean; full release suite
2575 passed, 0 failed, 15 ignored; full debug suite 2575 passed, 0 failed,
15 ignored. `tests/sol_callable_caps.rs` reports 31 passed.
