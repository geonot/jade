# Jinn Alpha Readiness — Remediation Plan

Date: 2026-05-31
Status: **pre-alpha / internal dogfood** — one open P0 codegen blocker remains.

## Purpose

This document is the prioritized remediation backlog produced by a pre-Phase-C
verification sweep. It cross-checks every prior finding from:

- `JINN_ALPHA_RELEASE_AUDIT.md` (2026-05-19 audit; P0-1..P0-9, P1-1..P1-6)
- `review/14_findings.md` (P0-1..P0-12, P1-1..P1-22, P2/P3)
- `review/15_roadmap.md` (Phase A / B / C plan)

against the **current** tree, using live, memory-capped probes (not prior
claims). Each item below is tagged with its verified current status and, where
still open, a concrete reproducer, root-cause area, and fix sketch.

> Note on the two numbering schemes: `JINN_ALPHA_RELEASE_AUDIT.md` and
> `review/14_findings.md` assign overlapping IDs (e.g. both have a "P0-5") to
> **different** bugs. Below, audit-doc IDs are written `AUDIT P0-n` and
> findings-doc IDs are written `F P0-n` to keep them distinct.

---

## Executive Summary

The Phase A safety floor is **almost** complete. Of the original P0 set, all but
one are verified fixed: division-by-zero, vector out-of-bounds, generator IR,
HOF closure inference, `take` in argument position, the missing-MIR-verifier,
bare-top-level-expression acceptance, missing-`main` linker error, and the
flagship `alpha_release_demo` LLVM signature mismatch all now behave correctly.

**One genuine P0 alpha blocker remains:** generic enums with an empty variant
fail in codegen (`FieldGet on pointer to unknown struct type for field __tag`).
This is `AUDIT P0-5` and was never closed.

A second, severe defect was discovered and fixed **during** this sweep: loop
bodies, match arms, value-position `if`, and expression blocks never dropped
their heap-allocated locals, producing an unbounded memory leak that OOM-froze
the host. Fixed in commits `d23c949` and `94f4838`.

Beyond the safety floor, several **major** stdlib defects block a clean alpha
stdlib subset, and several **significant** Phase-B/C items (LSP wiring,
stability/std docs, SECURITY.md, residual keyword-as-identifier conflicts)
remain.

**Recommendation:** do not enter Phase C until CRITICAL-1 is closed and the
MAJOR stdlib items are either fixed or explicitly excluded from the alpha-stable
subset.

---

## Verification Methodology

- All Jinn programs were run under `ulimit -v` memory caps + `timeout` so a
  leak/overflow aborts (rc 134/139) instead of freezing the OS.
- P0 reproducers were re-implemented as minimal probes and compiled + run with
  the release `jinnc`.
- Stdlib modules were type-checked via `jinnc <mod>.jn --lib --emit-hir` (runs
  lexer→parser→typer without LLVM; light and safe).
- Tree-sitter health was confirmed via the corpus suite (10/10) plus serial
  spot-parses of real stdlib/benchmark files.
- `cargo clippy` is **not installed** in the pinned toolchain and could not be
  run; this remains an open hygiene gap (see SIG-5).

---

## Status Matrix — Prior Findings

### Phase A (P0 safety floor)

| ID | Description | Status | Evidence |
| -- | ----------- | ------ | -------- |
| F P0-1 | Integer div/mod by zero is UB | ✅ FIXED | `10/0` → `runtime error: integer division by zero`, rc 134 |
| F P0-2 / AUDIT P0-6 | Vec OOB is SIGSEGV/silent | ✅ FIXED | `v[5]` → `runtime error: vec index out of bounds`, rc 134 |
| F P0-3 | Generator/`yield` emits invalid IR | ✅ FIXED | `for x in counter()` prints `0,1,2`, rc 0 |
| F P0-4 | `map(v, $ * 2)` ICE/crash | ✅ FIXED | prints `2`, rc 0 |
| F P0-5 | `take` in argument position | ✅ FIXED | `consume(take v)` prints `3`, rc 0 |
| F P0-6 | Stack overflow silent SIGSEGV | ⚠️ PARTIAL | now `jinn runtime: SIGSEGV at 0x…`, rc 139 (was garbage + rc 0). No "stack overflow"/file:line text yet. See MAJOR-4. Fixed handler install in `a620baf`. |
| F P0-7 / AUDIT P0-7 | No MIR verifier | ✅ FIXED | `src/mir/verify.rs` present |
| F P0-8 / AUDIT P0-2 | Bare top-level expr → exit code | ✅ FIXED | `42` → compile error "bare expression at top level has no effect" |
| F P0-9 / AUDIT P0-3 | Missing `main` = linker error | ✅ FIXED | "program has no `*main` function (use `--lib`…)" |
| F P0-10 | Keywords shadow methods after `.` | ⚠️ MOSTLY | `x.delete()` parses (type error, not parse error). Residual: keyword as **parameter/identifier** still breaks (see MAJOR-1). |
| F P0-11 | No fuzzer / sanitizer CI | ⚠️ PARTIAL | `fuzz/` and `ci/sanitize.sh` exist; CI gating not verified. See SIG-6. |
| F P0-12 | `Type::String` vs `Struct("string")` | ❓ UNVERIFIED | not directly re-probed; treat as latent. See SIG-7. |
| **AUDIT P0-5** | **Generic enum w/ empty variant fails codegen** | ❌ **OPEN** | `FieldGet on pointer to unknown struct type for field __tag`. **The remaining P0 blocker — CRITICAL-1.** |
| AUDIT P0-4 | Flagship `alpha_release_demo` fails LLVM verify | ✅ FIXED | builds rc 0, runs rc 0 (`metrics_rows 100`, `window_rows 8`) |
| AUDIT P0-8 | Release hygiene (159 warnings, fmt, clippy) | ⚠️ PARTIAL | `cargo fmt` clean (`0c31103`); clippy unavailable; warning count unverified. See SIG-5. |
| AUDIT P0-9 / F P2-13 | Tree-sitter failing its own tests | ✅ FIXED | corpus 10/10 (`4db4365`); real files parse clean |

### Phase B (P1 polish) — spot status

| ID | Description | Status |
| -- | ----------- | ------ |
| F P1-1 | Tuple return types parse | ✅ confirmed working (prior session) |
| F P1-2 | Match guards parse | ✅ confirmed working (prior session) |
| F P1-3 | SIMD literal syntax | ⛔ DEFERRED (explicitly skipped) |
| F P1-6 | Internal debug lines leak (`mir-perceus:`) | ✅ FIXED (gated on `--debug-perceus`) |
| F P1-5 / P1-11 / AUDIT P1-5 | LSP not wired into VS Code | ❌ OPEN — `vscode-jinn/package.json` is syntax-only (0 LSP refs). See SIG-1. |
| F P1-8 / AUDIT P0-8 | Release-build warnings | ⚠️ UNVERIFIED count |
| F P1-14 / AUDIT — | WAL randomised property tests | ✅ added (`41c3cb7`) |
| F P1-15 | Channel contention stress test | ✅ added (`7b9dc3f`) |
| F P1-16 | Actor/channel shutdown docs | ✅ `docs/concurrency.md` exists |
| F P1-9 / P1-10 / P1-21 | Dead HIR-direct codegen / Perceus shim | ⚠️ UNVERIFIED — dual driver paths (`driver/mod.rs` vs `driver/pipeline.rs`) both live |

---

## NEW Findings (this sweep)

### Discovered & FIXED during the sweep

- **LEAK-1 (severe, FIXED `d23c949` + `94f4838`):** Loop bodies (for-range,
  for-iterator, for-map, sim-for), match arms, value-position `if`, and
  expression blocks never dropped heap-allocated locals. A small program in a
  tight loop (`array_ops.jn`: 5-element vec × 1.5 B iterations) leaked
  unbounded heap → OOM → froze IDE/terminal/whole OS. Root cause spanned the
  HIR lowering of every loop/branch construct (`src/typer/lower/block.rs`,
  `iter.rs`, `stmt/dispatch.rs`, `stmt/block.rs`, `expr/control.rs`) plus the
  MIR value-block lowering (`src/mir/lower/control.rs`). Fix inserts scope-exit
  drops at HIR level and routes value-position blocks through a drop-skipping
  tail lowering. A latent ill-typed `Stmt::If` result-phi (bigint ICE) exposed
  by the fix was also corrected. Verified: 20 M-iter `array_ops` completes
  <1 s, rc 0, under a 256 MB cap.

### Discovered & STILL OPEN

These are itemized in the prioritized backlog below: CRITICAL-1 (generic enum
empty variant), MAJOR-1..3 (stdlib defects), and the SIGNIFICANT list.

---

## Prioritized Remediation Backlog

### CRITICAL — alpha blockers (must fix before any alpha tag)

#### CRITICAL-1 — Generic enum with an empty variant fails in codegen
- **Maps to:** `AUDIT P0-5` (never closed).
- **Severity:** Valid first-hour user code fails to compile; core language
  feature (generic `Option`/`Result`-style enums) unusable.
- **Reproducer:**
  ```jinn
  enum Maybe of T
      Some(T)
      Empty

  *unwrap(m as Maybe of i64) returns i64
      match m
          Some(x) ? x
          Empty ? 0

  *main() returns i32
      log(unwrap(Maybe.Some(7)))
      log(unwrap(Maybe.Empty))
      0
  ```
  → `error: FieldGet on pointer to unknown struct type for field __tag`.
- **Root cause (area):** Generic enum instantiation does not produce a concrete
  enum layout before tag/field lowering; the empty variant routes through an
  "unknown struct pointer" path in `FieldGet` codegen. Likely in the enum
  monomorphization + MIR enum-layout lowering and
  `mir_codegen` aggregate `FieldGet`.
- **Fix sketch:** Make enum representation explicit in MIR. Generic
  instantiation must emit a concrete, registered struct layout (tag + payload
  union) for every instantiated `Maybe of i64`; empty variants get a
  payload-less but still-registered layout so `__tag` access resolves. Add this
  exact probe as a non-ignored regression test. Add a MIR verifier invariant
  that every `FieldGet` target struct type is registered.

### MAJOR — block a clean alpha stdlib subset / serious quality gaps

#### MAJOR-1 — Reserved keywords cannot be used as parameter names / identifiers
- **Maps to:** residual of `F P0-10` (the after-`.` case is fixed; the
  parameter/identifier case is not).
- **Severity:** Breaks two stdlib modules outright and is a sharp, surprising
  edge for users.
- **Reproducers (current stdlib):**
  - `std/strings.jn:110` and `:125` — `*replace(s as String, from as String, …)`
    → `unexpected token: from` (`from` is a hard keyword, lexer
    `src/lexer/mod.rs:76`). This also fails `std/http.jn` transitively (it
    imports `strings`).
  - `std/dataframe.jn:75` — uses `select` as an identifier → `expected
    identifier, got select`.
- **Root cause (area):** `from`, `select` (and ~80 others) are promoted to
  keywords unconditionally in the lexer KEYWORDS table. The parser already has
  a contextual escape in `src/parser/mod.rs:290` (`Token::From` → identifier)
  for some positions, but not for parameter declarations or general identifier
  positions.
- **Fix sketch (pick one, prefer the first):**
  1. **Root fix:** make `from`/`select`/`by`/`to`/etc. *contextual* keywords —
     only special in their grammar context (store queries, `for … in … to …
     by …`). Elsewhere they lex as identifiers. This is the same
     contextual-keyword pattern already used after `.`.
  2. **Stopgap:** rename the offending stdlib parameters/identifiers
     (`from` → `needle`/`old`, `select` → `cols`). Cheap, but leaves the trap
     for users.
- **Recommendation:** do (1) for the small set of clearly-contextual words; it
  removes a whole class of "this identifier won't compile" surprises.

#### MAJOR-2 — `std/sqlite.jn` lexer error on `{`
- **Severity:** Module does not type-check.
- **Reproducer:** `jinnc std/sqlite.jn --lib --emit-hir` → `line 172:
  unexpected character: '{'`.
- **Root cause (area):** Most likely string interpolation `{…}` used inside a
  double-quoted **raw** string (raw strings do not interpolate in Jinn;
  interpolation is single-quote `'…{expr}…'`), or a stray brace. Needs a read
  of `std/sqlite.jn:172`.
- **Fix sketch:** Convert the intended-interpolated literal to a single-quoted
  string, or escape the brace; confirm the raw-vs-interpolated string contract
  in `docs` and lexer.

#### MAJOR-3 — stdlib references to a nonexistent runtime function / missing Map method
- **Severity:** Two more modules fail type-check.
- **Reproducers:**
  - `std/crypto.jn` → `undefined function: '__string_from_ptr_len'` (references
    a runtime symbol that does not exist; compare to the auto-recognized
    `__string_from_ptr`).
  - `std/collections.jn` → `no method 'delete' on Map` (Map's removal method is
    named differently — likely `del` — or the method is genuinely missing).
- **Fix sketch:** Either add `__string_from_ptr_len` to the runtime/string
  builtins or rewrite the crypto call to use the existing API; add `delete` as
  an alias on `Map` or fix the `collections` call site to use the real method
  name. Add both modules to the stdlib type-check CI gate.

#### MAJOR-4 — Stack-overflow diagnostic is generic, not specific
- **Maps to:** polish remainder of `F P0-6`.
- **Severity:** Common user mistake produces `jinn runtime: SIGSEGV at 0x…`
  rather than a stack-overflow-specific, file:line diagnostic. The dangerous
  original behavior (garbage + rc 0) is already fixed.
- **Fix sketch:** In the SIGSEGV handler, compare `si_addr` against the active
  coroutine guard region (and the main-thread stack guard) and, on a guard hit,
  print `coroutine/main stack overflow (consider refactoring recursion or
  raising JINN_STACK_SIZE)` before re-raising.

#### MAJOR-5 — Declare and verify the alpha-stable stdlib subset
- **Maps to:** `Phase C C.2` / `AUDIT "Standard Library"`.
- **Severity:** With ~51 `std/` modules of mixed health, an alpha cannot ship
  "the standard library" wholesale.
- **Action:** Pin a small alpha-stable subset (suggested: `math`, `strings`,
  `bytes`, `collections`, `fs/path/os/process`, `time/date`, `args/io/logging`,
  `json/csv`, crypto hashes, channels/actors surface, stores). Gate that subset
  in CI with `--lib --emit-hir` (and where possible, run their tests). Mark the
  rest experimental. Fixing MAJOR-1..3 is a prerequisite for several of these.

### SIGNIFICANT — needed for a polished alpha, can trail slightly

- **SIG-1 — Wire (or formally scope-out) the VS Code LSP.** `src/lsp/*` and the
  `jinnc-lsp` binary exist, but `vscode-jinn/package.json` contributes syntax
  only (no `activationEvents`, no `main`, no language client). Either add a
  client that launches `jinnc-lsp` with a smoke test
  (diagnostics/hover/def/refs/rename) or label the extension syntax-only in its
  README. Maps to `F P1-5/P1-11`, `AUDIT P1-5`.
- **SIG-2 — VS Code TextMate grammar staleness.** `jinn.tmLanguage.json`
  highlights `rc`, `weak`, `do`, `end` — `rc()/weak()` were removed and Jinn is
  indentation-based (no `do`/`end`). Prune these to avoid misleading
  highlighting. (Grammar is otherwise correct: `.jn`, single-quote interpolated
  vs double-quote raw strings, current type/keyword set.)
- **SIG-3 — Author `docs/stability.md` and `docs/std.md`.** Both missing.
  Required for Phase C (`C.2`, `C.3`): the stable/experimental/deprecated
  contract and the stdlib API/subset doc.
- **SIG-4 — Author `SECURITY.md`.** Missing. Phase C `C.6` disclosure policy.
- **SIG-5 — Release hygiene baseline.** Install `clippy` in the pinned
  toolchain and gate `cargo clippy --all-targets -- -D warnings` in CI; quantify
  and drive the release-build warning count to zero (or justified allowances).
  `cargo fmt --check` is already clean. Maps to `AUDIT P0-8`, `F P1-8`.
- **SIG-6 — Verify fuzz + sanitizer CI actually run.** `fuzz/fuzz_targets/` and
  `ci/sanitize.sh` exist; confirm they are wired into CI and run for a bounded
  duration with no new panics (lexer/parser/roundtrip fuzz; ASan/UBSan/TSan
  runtime). Maps to `F P0-11`.
- **SIG-7 — Canonicalize `Type::String` ↔ `Type::Struct("string")`** at typer
  unification entry and add a debug-assert that compared types are canonical.
  Latent source of surprise type errors. Maps to `F P0-12`.
- **SIG-8 — Reconcile the dual codegen driver paths.** `src/driver/mod.rs`
  (`jinnc`) and `src/driver/pipeline.rs` (`jinn`) both drive compilation and
  were patched identically for the runtime-link fix. Confirm there is a single
  shared `compile_program` invariant and delete any dead HIR-direct path. Maps
  to `F P1-9/P1-10/P1-21`.

### MINOR / Test debt — polish, post-alpha

- **MIN-1 — Ignored test `perceus_debug.rs:83` (`drop_fusion_coalesces`)** and
  **MIN-2 — `bulk_tests.rs:6832` (`weak_roundtrip`)** are both blocked on a
  missing heap-tax inference path (the surface `rc()`/`weak()`/`weak_upgrade()`
  builtins were removed with no replacement). This is a real Phase-7 feature
  gap, not a bug; the tests are correctly ignored until the feature returns.
- **MIN-3 — `bulk_tests.rs:5556` (`b_p42_strict_unsolved_typevar`)** is obsolete
  (the bug it guarded is fixed); replace it with a test that exercises a
  genuinely-unsolved type variable.
- **MIN-4 — `integration.rs:2751` (`store_perf_regression`)** is a deliberate
  performance guard mis-housed in the correctness suite; move it to the
  benchmark suite. Not a bug.
- **MIN-5 — Move `benchmarks/results*.json` and root scratch files** (`a.out`,
  stray `*.store`/`*.wal`, scratch `.jn`) out of the source tree. Maps to
  `F P2-18/P2-19`.
- **MIN-6 — Benchmark `array_ops.jn` is a 1.5 B-iteration allocation-bound
  outlier.** Not a compiler bug (and the leak it exposed is now fixed), but its
  iteration count makes it a host-stressing outlier; consider scaling it down
  for the default benchmark run.

---

## Benchmarks & Tooling — verification notes

- **No performance regression.** A uniform +70–100 % apparent slowdown observed
  mid-sweep was thermal throttling on a powersave-governed laptop CPU under
  sustained load; an idle re-measure was flat (fibonacci −2.2 %, enum_dispatch
  ±0.0 %). Benchmarks run without crashes after the LEAK-1 fix.
- **Tree-sitter** corpus is 10/10; real stdlib/benchmark files parse with zero
  ERROR nodes.
- **Comments stripped** from the Rust compiler sources (`3a31f20`), suite green.

---

## Phase-C Entry Gate (recommended)

Do not begin Phase C until:

1. **CRITICAL-1** (generic enum empty variant) is fixed and covered by a
   non-ignored regression test.
2. **MAJOR-1..3** are fixed (or the affected modules excluded from the
   alpha-stable subset declared in **MAJOR-5**).
3. **SIG-5** baseline is green (clippy installed + `-D warnings`, fmt already
   clean) so the release log is clean.

Everything else (LSP wiring, stability/std/security docs, the MINOR test-debt
items) can proceed in parallel with early Phase-C documentation work.
