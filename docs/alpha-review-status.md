# Alpha review — remediation status

Tracks the fixes for [`alpha-review-findings.md`](alpha-review-findings.md).
The findings file is the stable record; this file changes as work lands.

**Fixed** means the original reproduction was re-run against the built compiler
and now behaves correctly — not that the code was edited. Where a finding had a
measurable rate, both numbers are given.

Regression coverage lives in `tests/alpha_review_regressions.rs` and
`tests/corpus_differential.rs`.

## Fixed

| Finding | Verification |
| --- | --- |
| AR-F1 std gate | all 49 std modules import and link (was 39) |
| AR-F2 phi ICE | diagnostic naming both branches; MIR verify now checks phi/edge types **and runs in release** |
| AR-F3 UAF ×4 | (1)(2)(3) are compile errors with fix-naming diagnostics; (4) `Map.get` round-trips |
| AR-F4 destroy crash | 40 kill points, 0 total-loss outcomes (was 5) |
| AR-F5 store concurrency | 20/20 runs correct (was 5/20 correct, 9/20 unreadable) |
| AR-F6 stale `@index` | indexed lookup returns the right row after `destroy` |
| AR-F7 DCE traps | `as strict` aborts at `--opt 0`, `1` and `3` |
| AR-F8 casts | enum→int reads only the discriminant; float→int saturates; explicit discriminants carried through (`s079.jn` prints `1 2` at both opt levels, was nondeterministic garbage) |
| AR-F11 main errors | binds the success value; an unhandled error prints and exits non-zero |
| AR-F12 stale build | dependency edits invalidate the `jinn run` cache; `.jni` reuse is off by default |
| AR-F13 path traversal | dependency URLs containing `.`/`..`/absolute segments are rejected; resolved paths asserted inside the cache root |
| AR-F14 header overflow | `jinn_safe_mul` + a file-size cross-check before allocating |
| AR-F15 struct aliasing | `q is p` copies; the test that pinned aliasing now pins value semantics |
| AR-F16 struct params | POD-struct parameters are independent copies, drop-needing ones borrow — matching `access-semantics.md` §4.1 in and out of loops |
| AR-F22 `take` | `take(21)` → 42 |
| AR-F23 `String <` | byte-lexicographic; irreflexivity holds |
| AR-F30 struct double-free | parameter ownership recomputed after inference; borrowed params get no drop glue |
| AR-F31 actor message loss | mailboxes closed and drained at exit; 6/6 deterministic at both opt levels. Registration is scoped to the direct `spawn` path — supervisor-managed actors have their own lifecycle and double-freed when exit-time drain also closed them |
| AR-F32 `Map of String` | entry widened to hold a 24-byte handle; values load at their declared type |
| AR-21 `%String` FFI | sha256 matches `hashlib` at every length across the SSO boundary |
| AR-22 constant patterns | constants compare instead of binding; `json.parse` works |
| AR-23 double-quoted strings | escapes and interpolation, matching the docs; 16 std modules unbroken |
| AR-24 `together` scope | nested `together` inside `dispatch` joins its children, 0/20 failures (was 30/30) |

### Gates

| Item | State |
| --- | --- |
| `scripts/alpha_release_smoke.sh` | path repaired (`examples/` → `apps/`); preflight can reach steps 6 and 7 again |
| Corpus execution | `tests/corpus_differential.rs` compiles **and runs** `snippets/` and `tests/programs/` at `--opt 0` and `--opt 3`, diffing stdout and exit code |
| `KNOWN_ICE` whitelist | replaced by a list that asserts the *specific* diagnostic and fails if the program starts compiling or starts panicking |
| MIR verify | out from behind `#[cfg(debug_assertions)]`; `JINN_MIR_VERIFY=0` opts out |

### Tooling and benchmarks

| Item | State |
| --- | --- |
| AR-F19 benchmarks | `coroutine_spawn` iteration counts realigned (the C side ran 10x the Jinn side); `run_benchmarks.py` captures stdout for **every** language and names any benchmark whose languages disagree; `sim_for` documented as not comparable, with no ratio to be quoted from it |
| AR-F26 `jinn bind` | emits `#` comments, drops multi-line macros, keeps block comments out of signatures, and skips what it cannot represent with a stated reason. Verified against `errno.h`, `string.h` and `stdlib.h`: 97 externs generated, all parseable |
| AR-F27 LSP UTF-16 | incoming positions and outgoing ranges both convert between UTF-16 code units and byte offsets |
| AR-20 `ci/sanitize.sh` | rewritten: builds an instrumented runtime into its own target directory and sweeps nine targeted programs under ASan+UBSan and TSan in ~2 minutes, instead of `cargo clean` plus two full suite runs (~1 hour). Green. It caught a data race in the actor-drain code added in this same change |

### Documentation

`docs/jinn.md` (`.length`, `alias`, ownership status block),
`docs/memory-model.md` (the "enforced and pinned" claim, two fabricated test
citations, `src/perceus/` → `src/drops/`), `docs/access-semantics.md` (same
path), `docs/structured-concurrency.md` (status banner was understating a
working feature).

## Open

| Finding | Note |
| --- | --- |
| AR-F9 unsolved type variables | `std/math.jn` annotated so the defaulting no longer miscompiles there; the general case still warns rather than errors |
| AR-F10 capabilities inert | unchanged. Recommended **cut**: rebrand `docs/caps.md` as design-not-implemented and list capabilities as experimental. `grep -rn "needs "` over the corpus returns zero uses, so demotion breaks nothing |
| AR-F17 `fmt` destroys code | string literals now escape correctly and round-trip; the printer's grammar drift (store/extern/actor/query forms, the `...` placeholder, paren precedence) is untouched. Recommended **cut**: make `fmt` print-only |
| AR-F18 doc claims | the load-bearing ones are fixed; the full 23-item list has not been re-audited |
| AR-F20 diagnostics | compiler panics converted to diagnostics that name the construct and suggest a fix; the `hir:`/`hir-validate:` stage prefixes are gone; `--emit-hir` now exits non-zero on rejected programs. Still open: undefined names are caught in codegen without a span, and mangled names can reach messages |
| AR-F21 generics | unchanged |
| AR-F24 transaction atomicity | unchanged |
| AR-F25 post-crash index disagreement | the `destroy` path now invalidates indexes; the crash-recovery path is unchanged |
| AR-F28 `embed` path reads | unchanged |
| AR-F29 `! E` surface | unchanged |
| P2 list | the committed ELF binary and release tarball are untracked and ignored; the rest unchanged |

## Gate state after remediation

`scripts/test.sh` — 2042 passed, 0 failed.
`cargo fmt --check`, `cargo clippy --release --all-targets -- -D warnings` — clean.
`ci/sanitize.sh` — both sweeps green.
`scripts/preflight.sh` — reaches all seven steps for the first time since the
smoke script's target directory moved.

### Known compiler limitation

`tests/programs/compiler_pipeline.jn` does not compile. A recursive enum payload
rebound inside a loop produces a control-flow merge whose incoming values have
different shapes. This used to be an ICE; it now fails with a diagnostic that
names the function and the construct. The harness asserts that exact diagnostic,
so it will report both a regression to a panic and a fix.

## Cut candidates

Demoting a stable surface pre-1.0 is allowed by `docs/stability.md` and must be
recorded in the changelog. These are cheaper honestly demoted than repaired:

- **Capabilities** — the pass is inert and wiring it correctly is a design-note
  sized job.
- **Raw pointers (`%`/`@`) and `std/volatile`** — documented in the main tour
  with no `unsafe` gate; a `%i64` in a struct field escapes its pointee's scope.
  There is no marker a reviewer could grep for in a downloaded package.
- **`jinn fmt --write`** — remove it rather than block alpha on a printer
  rewrite.
- ~~**`jinn bind`**~~ — repaired instead of demoted: it now generates
  parseable Jinn from real system headers and states what it skipped. Still
  a textual approximation of C, so the generated file says so at the top.
- **`.jni` interface files** — reading them is already off by default; there is
  no configuration in which the feature is both safe and useful.
