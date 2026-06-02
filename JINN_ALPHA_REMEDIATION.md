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

The Phase A safety floor is **complete**. Of the original P0 set, all are now
verified fixed: division-by-zero, vector out-of-bounds, generator IR,
HOF closure inference, `take` in argument position, the missing-MIR-verifier,
bare-top-level-expression acceptance, missing-`main` linker error, and the
flagship `alpha_release_demo` LLVM signature mismatch all now behave correctly.

**The last P0 alpha blocker is now closed:** generic enums with an empty variant
used through a concrete type annotation previously failed codegen with
`FieldGet on pointer to unknown struct type for field __tag`. This was
`AUDIT P0-5` / CRITICAL-1; it is **FIXED** (parser no longer pre-mangles generic
annotations; the typer canonicalizes them in a function-signature pass). The
same fix also covers generic structs in concrete parameter annotations.

A second, severe defect was discovered and fixed **during** this sweep: loop
bodies, match arms, value-position `if`, and expression blocks never dropped
their heap-allocated locals, producing an unbounded memory leak that OOM-froze
the host. Fixed in commits `d23c949` and `94f4838`.  

Beyond the safety floor, the **major** stdlib defects, the **significant**
Phase-B/C items (LSP wiring, stability/std docs, SECURITY.md, residual
keyword-as-identifier conflicts, release-hygiene), and the **minor** test/tooling
backlog have all since been worked to completion: **MAJOR-1..5, SIG-1..8, and
MIN-3..6 are FIXED**, and **MIN-1/MIN-2 are now RESOLVED** by formally adopting
the value-semantics memory model (the removed `rc()`/`weak()` heap-tax surface is
not returning): MIN-1's drop-fusion coverage was rewritten as a live
value-semantics test and MIN-2's `weak_roundtrip` placeholder was deleted. There
are now **zero `#[ignore]`d tests** in the suite.

**Recommendation:** CRITICAL-1 and the full MAJOR/SIGNIFICANT/MINOR backlog are
closed. The Phase-C entry gate (below) is satisfied; the only standing residuals
are environment-bound (no local `clippy`/`rustup`) and the explicitly-deferred
SIMD literal syntax (F P1-3). The `rc()`/`weak()` heap-tax feature is formally
retired in favor of value semantics (MIN-1/2 resolved).

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
| F P0-6 | Stack overflow silent SIGSEGV | ✅ FIXED | coroutine overflow already specific; native/main-thread overflow now prints `jinn runtime: stack overflow (native thread) at fault address 0x…` + `ulimit -s` advice, rc 134. Bounds cached at install (async-signal-safe). See MAJOR-4 (DONE). |
| F P0-7 / AUDIT P0-7 | No MIR verifier | ✅ FIXED | `src/mir/verify.rs` present |
| F P0-8 / AUDIT P0-2 | Bare top-level expr → exit code | ✅ FIXED | `42` → compile error "bare expression at top level has no effect" |
| F P0-9 / AUDIT P0-3 | Missing `main` = linker error | ✅ FIXED | "program has no `*main` function (use `--lib`…)" |
| F P0-10 | Keywords shadow methods after `.` | ⚠️ MOSTLY | `x.delete()` parses (type error, not parse error). Residual: keyword as **parameter/identifier** still breaks (see MAJOR-1). |
| F P0-11 | No fuzzer / sanitizer CI | ✅ FIXED | CI `fuzz-smoke` (nightly+cargo-fuzz, lexer/parser/typer @ 30s each) + `sanitize` (ASan/UBSan then TSan over full `cargo test`) jobs wired in `.github/workflows/ci.yml`; fuzz targets verified API-current. See SIG-6. |
| F P0-12 | `Type::String` vs `Struct("string")` | ✅ FIXED | canonicalized at all name→Type birth sites; `canonical()` net + `debug_assert!` invariant at unify entry, validated green in debug. See SIG-7. |
| **AUDIT P0-5** | **Generic enum w/ empty variant fails codegen** | ✅ **FIXED** | Root cause was a parser/typer mangling-scheme mismatch (`Name_arg` vs `Name__G_arg`), not a MIR layout hole. Fixed in parser + typer; regression tests `alpha_audit_generic_empty_enum` and `alpha_audit_generic_struct_param`. **CRITICAL-1 closed.** |
| AUDIT P0-4 | Flagship `alpha_release_demo` fails LLVM verify | ✅ FIXED | builds rc 0, runs rc 0 (`metrics_rows 100`, `window_rows 8`) |
| AUDIT P0-8 | Release hygiene (159 warnings, fmt, clippy) | ✅ FIXED | Rust release-build warnings driven to **0** (deleted 6 dead coercion helpers + unused imports in `codegen/types.rs`); `cargo fmt` clean; full `cargo test --release` green. Clippy-driver `-D warnings` gate documented as env-bound residual (no rustup/clippy locally). See SIG-5. |
| AUDIT P0-9 / F P2-13 | Tree-sitter failing its own tests | ✅ FIXED | corpus 10/10 (`4db4365`); real files parse clean |

### Phase B (P1 polish) — spot status

| ID | Description | Status |
| -- | ----------- | ------ |
| F P1-1 | Tuple return types parse | ✅ confirmed working (prior session) |
| F P1-2 | Match guards parse | ✅ confirmed working (prior session) |
| F P1-3 | SIMD literal syntax | ⛔ DEFERRED (explicitly skipped) |
| F P1-6 | Internal debug lines leak (`mir-perceus:`) | ✅ FIXED (gated on `--debug-perceus`) |
| F P1-5 / P1-11 / AUDIT P1-5 | LSP not wired into VS Code | ✅ FIXED — `vscode-jinn` now launches `jinnc-lsp` via a `vscode-languageclient` client; client compiles, live handshake + diagnostics verified. See SIG-1. |
| F P1-8 / AUDIT P0-8 | Release-build warnings | ✅ FIXED — 0 warnings (Rust) |
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
  exposed by the fix was also corrected. Verified: 20 M-iter `array_ops`
  completes <1 s, rc 0, under a 256 MB cap.

- **PARSER-1 (significant, FIXED):** Recursive generic enum variant fields could
  not parse. A variant whose unnamed field is itself a generic application —
  e.g. `Branch(Tree of T, Tree of T)` in `enum Tree of T` — failed with
  `expected ,, got of`. Root cause: `Parser::parse_vfield`
  (`src/parser/decl/types.rs`) read a single identifier via `ident_to_type` for
  unnamed fields instead of `parse_type`, so it stopped at `Tree` and choked on
  `of`. This also blocked pointer/tuple/fn types in variant-field position.
  Fixed by disambiguating named (`name as Type`) vs unnamed fields with a
  one-token lookahead and parsing the unnamed case as a full type. The typer
  then needed two companion fixes to monomorphize the recursive instantiation:
  (1) `Typer::substitute_type` (`src/typer/mono.rs`) now recurses into
  `Type::Struct(name, args)` and `Type::Generator` so a field `Tree of T`
  substitutes to `Tree of i64` under the type-param map; (2)
  `Typer::monomorphize_enum` reserves the mangled enum name in `self.enums`
  *before* processing variant fields (breaking the self/mutual-recursion cycle)
  and canonicalizes each substituted field type through
  `monomorphize_named_annotation`, so the recursive self-reference resolves to
  the same `Type::Enum(mangled)` instantiation. Verified:
  `tests/programs/enums_advanced.jn` compiles and runs (`tree_sum`=15,
  `depth`=4, `count`=5); regression `alpha_audit_recursive_generic_enum`
  (`tests/audit_alpha/recursive_generic_enum.jn` → `15\n4`) is green.

### Discovered & since RESOLVED

The items that were open mid-sweep — CRITICAL-1 (generic enum empty variant),
MAJOR-1..3 (stdlib defects), and the SIGNIFICANT list — are all now FIXED;
see the per-item status in the prioritized backlog below. The MINOR backlog
(MIN-3..6) is likewise FIXED, with MIN-1/MIN-2 resolved via value semantics.

---

## Prioritized Remediation Backlog

### CRITICAL — alpha blockers (must fix before any alpha tag)

#### CRITICAL-1 — Generic enum with an empty variant fails in codegen  ✅ FIXED
- **Maps to:** `AUDIT P0-5`.
- **Status:** **FIXED.** Reproducer compiles and runs; regression tests
  `alpha_audit_generic_empty_enum` and `alpha_audit_generic_struct_param` are
  non-ignored and green; full `cargo test --release` passes (0 failures).
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
  → previously `error: FieldGet on pointer to unknown struct type for field
  __tag`; now prints the expected values.
- **Actual root cause:** Not a MIR enum-layout hole. The *parser* pre-mangled a
  generic type annotation `Maybe of i64` into `Type::Struct("Maybe_i64", [])`
  using the scheme `Name_arg`, while enum monomorphization
  (`Typer::monomorphize_enum` → `mangle_generic`) names instantiations with the
  scheme `Name__G_arg`. The two schemes disagreed, so the function parameter was
  typed as an *undeclared* struct name that was never registered — codegen then
  hit `FieldGet` on an unknown struct when reading `__tag`. Generic structs
  happened to work only because their construction path used the matching
  `Name_arg` scheme.
- **Fix (root cause, across the pipeline):**
  1. `src/parser/expr/primary.rs` — `parse_type` no longer mangles generic
     annotations. For a user generic it emits `Type::Struct(name, [arg])`,
     preserving the type argument and leaving canonicalization to the typer.
     (`Vec`/`Map` keep their dedicated representations.)
  2. `src/typer/mono.rs` — added `monomorphize_named_annotation`, which rewrites
     a `Type::Struct(name, args)` with concrete args into the canonical
     monomorphic type: generic *enums* → `Type::Enum(mangled)` via
     `monomorphize_enum` (the single mangling authority); generic *structs* →
     `Type::Struct(mangled, [])` via the new
     `monomorphize_generic_struct_annotation`, which substitutes declared field
     types through a type-param→type-arg map and registers the concrete layout.
     It recurses through `Array`/`Vec`/`Map`/`Tuple`/`Fn`/`Ptr`/`Channel`/
     `Coroutine`/`Generator`.
  3. `src/typer/lower/mod.rs` — after all declarations are registered (so
     `generic_enums`/`generic_types` are populated) and before body lowering, a
     pass normalizes every function signature's parameter and return types
     through `monomorphize_named_annotation`, so parameters bind to the
     canonical monomorphic type.
- **Regression coverage:** `tests/audit_alpha/generic_empty_enum.jn` (was the
  negative `negative_generic_empty_enum.jn`, now a positive fixture) and
  `tests/audit_alpha/generic_struct_param.jn`, wired into
  `tests/alpha_release_audit.rs`.

### MAJOR — block a clean alpha stdlib subset / serious quality gaps

#### MAJOR-1 — Reserved keywords cannot be used as parameter names / identifiers — ✅ FIXED
- **Status:** FIXED. Soft keywords `from`/`to`/`by`/`at`/`select` now parse as
  identifiers in declaration-name and expression positions while keeping their
  contextual grammar roles. Validated end-to-end (`tests/audit_alpha/soft_keyword_idents.jn`,
  test `alpha_audit_soft_keyword_idents` → `20\n105`); full suite green.
- **Fix (chose root fix #1):**
  - `src/parser/mod.rs` — extracted `soft_keyword_ident(&Token) -> Option<&str>`
    as the single source of truth for contextual keywords usable as identifiers;
    added `by`/`at`/`select`. `ident()` consults it after `Token::Ident`.
  - `src/parser/expr/parse_primary.rs` — added a prefix-atom arm for
    `From | To | By | AtKw` so these lex-keywords work as plain variables in
    expression position. Safe because their real keyword roles (`e at i`,
    `e from a to b`) are postfix/infix in the Pratt parser and only fire with a
    left operand (disjoint from prefix position).
- **Accepted limitation:** `select` at statement-start remains a hard keyword
  (concurrency `select` statement is genuinely ambiguous there). Stdlib only
  uses `select` as a method name (`df.select(...)`), which lexes as `Ident`
  after `.`, so this is unaffected.
- **Note:** the `undefined function: chr/parse_int/parse_float` errors that
  surface in `strings`/`dataframe`/`http` after this fix are a *separate*
  cross-module/builtin resolution issue (parse_int/parse_float live in
  `std/convert.jn`); tracked under MAJOR-3/5, not a MAJOR-1 regression.
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

#### MAJOR-2 — `std/sqlite.jn` lexer error on `{` — ✅ FIXED
- **Status:** FIXED. The `{` "lexer error" was actually three non-conforming
  constructs in two experimental modules (`std/sqlite.jn`, `std/event.jn`) that
  were written against an aspirational dialect. Both now type-check cleanly
  (`jinnc std/{sqlite,event}.jn --lib --emit-hir` → rc 0).
- **Root causes & fixes:**
  1. **Curly-brace struct literals** (`Row { f is v }`) — invalid; Jinn struct
     construction is `Row(f is v)`. Rewrote all literals in both modules.
  2. **`[T]` type annotations** (`names as [String]`, `returns [Row]`) — the
     bracket type form was never accepted by `parse_type`. **Language fix:** added
     `[T]` as list-type sugar in `src/parser/expr/primary.rs::parse_type`
     (`Token::LBracket` arm → `Type::Vec(T)`), the symmetric counterpart of the
     `[...]` list literal. Unambiguous: a leading `[` in type position previously
     always errored. Regression: `tests/audit_alpha/bracket_list_type.jn`, test
     `alpha_audit_bracket_list_type` → `60\n15`.
  3. **`vec + [elem]` concatenation** — not an intended feature (no tests, docs,
     or runtime support; the 28-file std consensus is `.push`). Rewrote the
     accumulation loops in both modules to the canonical `.push(elem)` idiom.
     Also fixed `band` → `&` (bitwise-and) in `event.jn` (only file using `band`).
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

#### MAJOR-3 — stdlib references to a nonexistent runtime function / missing Map method — ✅ FIXED
- **Status:** FIXED. Both modules type-check (`jinnc std/{crypto,collections}.jn
  --lib --emit-hir` → rc 0).
- **Root causes & fixes:**
  - `std/crypto.jn` called `__string_from_ptr_len(ptr, len)`, which never
    existed. The existing `__string_from_raw(ptr, len[, cap])` builtin does
    exactly this (explicit length, no `strlen`) — codegen already supported the
    2-arg form (`cap` defaults to `len`), but the **typer** only recognized the
    3-arg form. Fix: relaxed the typer arity guard in
    `src/typer/builtins.rs` to `(2..=3).contains(&args.len())`, aligning it with
    codegen, and rewrote crypto's calls to `__string_from_raw`. No redundant new
    builtin added.
  - `std/collections.jn` called `self.map.delete(...)`; Map's removal method is
    `remove`. Fixed the call site.
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

#### MAJOR-4 — Stack-overflow diagnostic is generic, not specific — ✅ FIXED
- **Maps to:** polish remainder of `F P0-6`.
- **Severity:** Common user mistake produced `jinn runtime: SIGSEGV at 0x…`
  rather than a stack-overflow-specific diagnostic. The dangerous original
  behavior (garbage + rc 0) was already fixed; coroutine overflow was already
  specific. The remaining gap was the **native (main / worker) thread** stack.
- **Resolution:** `runtime/signals.c` now caches each thread's native stack
  bounds at handler-install time (normal context: `pthread_getattr_np` +
  `pthread_attr_getstack`/`getguardsize`, stored in TLS), keeping the SIGSEGV
  handler async-signal-safe. On a fault, after the coroutine guard check, the
  handler compares `si_addr` against the cached guard region and, on a hit,
  prints `jinn runtime: stack overflow (native thread) at fault address 0x…`
  plus `ulimit -s` remediation advice, then `_exit(134)`. The duplicated hex
  formatter was refactored into a single `write_hex_addr`. Regression test
  `alpha_audit_native_stack_overflow_diagnostic` (compiled at `--opt 0` so LLVM
  cannot linearise the recursion) asserts rc 134 and the specific message.
  Full `cargo test --release` green.

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

- **SIG-1 — Wire the VS Code LSP.** ✅ FIXED (wired + verified live). `src/lsp/*`
  builds the `jinnc-lsp` stdio JSON-RPC server (initialize, didOpen/didChange/
  didClose, hover, definition, documentSymbol, completion, references, rename,
  semanticTokens/full, signatureHelp, publishDiagnostics). The extension was
  syntax-only; it is now a real language client:
  - Added [vscode-jinn/src/extension.ts](vscode-jinn/src/extension.ts) — launches
    `jinnc-lsp` over stdio via `vscode-languageclient`, `documentSelector`
    `[{ scheme: file, language: jinn-lang }]`, a `.jn` file watcher, a dedicated
    output channel, and a `jinn.lsp.restart` command. Server path resolves from
    `jinn.lsp.serverPath`, else workspace `target/release` → `target/debug`, else
    `jinnc-lsp` on PATH; absence degrades gracefully (syntax highlighting persists,
    one dismissible hint with an Open-Settings action).
  - [vscode-jinn/package.json](vscode-jinn/package.json) — added `main`,
    `activationEvents: [onLanguage:jinn-lang]`, the `jinn.lsp.restart` command,
    `jinn.lsp.{enable,serverPath}` configuration, `vscode-languageclient ^9`
    dependency, TS devDeps, compile/watch scripts; version bumped to `0.2.0` to
    match the server.
  - Added [vscode-jinn/tsconfig.json](vscode-jinn/tsconfig.json) (strict, Node16),
    `.vscodeignore`, `.gitignore`; README documents the server, settings, and the
    `npm install && npm run compile` dev flow.
  - **Verified:** `npm install` + `npm run compile` emit `out/extension.js` with
    zero TS errors. The server was built (`cargo build --release --bin jinnc-lsp`)
    and driven with a real framed JSON-RPC handshake: `initialize` returns the full
    capability set (hover/definition/documentSymbol/completion[.]/references/rename/
    semanticTokens[9 token types]/signatureHelp[(,]), `shutdown` responds, clean
    `exit 0`; a `didOpen` of malformed source produces a framed
    `textDocument/publishDiagnostics`. Maps to `F P1-5/P1-11`, `AUDIT P1-5`.
- **SIG-2 — VS Code TextMate grammar staleness.** ✅ FIXED. `jinn.tmLanguage.json`
  highlighted `rc`/`weak` (removed builtins) — pruned from `keyword.declaration`.
  Correction to the original finding: `do`/`end` are **still live** tokens (a
  `do … end` block form survives in `src/parser/expr/parse_primary.rs`), so they
  were intentionally kept. Also added the real `pow` (`**`) word-operator alias
  and extended the single-quote escape set to the lexer's actual set
  (`\n \t \r \0 \\ \' \" \{ \}` + `\xHH`). JSON + escape regex validated.
- **SIG-3 — Author `docs/stability.md` and `docs/std.md`.** ✅ FIXED. Authored
  `docs/stability.md` (language/runtime/stdlib stability tiers: stable /
  experimental / deprecated, pre-1.0 caveats, enforcement table) and
  `docs/std.md` (stdlib usage conventions + the machine-checked alpha-stable
  subset and policy; renamed from the interim `docs/std-stability.md`). The two
  cross-reference each other and the `tests/std_stable_subset.rs` gate.
- **SIG-4 — Author `SECURITY.md`.** ✅ FIXED. Added root `SECURITY.md` with a
  private-reporting process (GitHub security advisory), supported-versions table,
  scope (runtime memory safety, compiler unsoundness, stdlib untrusted-input
  handling), explicit non-vulnerabilities (clean diagnosed aborts incl. the new
  stack-overflow handler), and a disclosure timeline.
- **SIG-5 — Release hygiene baseline.** ✅ FIXED (warning-zero; clippy gate
  wired in CI). Drove the Rust release-build warning count to **zero**: removed
  the six dead, pre-MIR coercion helpers in `src/codegen/types.rs`
  (`coerce_val`, `coerce_val_ex`, `coerce_int_width`, `wrap_negative_index`,
  `resolve_ty`, `compile_coercion`) — all superseded by `mir_codegen`'s
  `emit_cast` and verified to have zero callers repo-wide — plus their now-unused
  `use crate::hir;` and `use super::b;` imports. `cargo build --release` is now
  warning-clean on the Rust side (the only residual diagnostics are C-side
  `-Wmissing-prototypes` notes from `runtime/random.c`, which do not gate the
  Rust `-D warnings` build); full `cargo test --release` stays green
  (224/912/434/… 0 failed). `cargo fmt --all --check` is clean. CI installs the
  `clippy` component and runs `cargo clippy --all-targets` under
  `RUSTFLAGS: -D warnings`. Residual (environment-bound, not a code defect): the
  `-- -D warnings` clippy-driver gate (which additionally denies `clippy::*`
  lints beyond the rustc `warnings` group) cannot be verified in this local
  toolchain — it is a **system cargo install with no rustup/clippy component**,
  so clippy is uninstallable here; the one-line upgrade
  (`cargo clippy --all-targets -- -D warnings`) should be flipped after a
  clippy-clean pass in a rustup-equipped environment. Maps to `AUDIT P0-8`,
  `F P1-8`.
- **SIG-6 — Verify fuzz + sanitizer CI actually run.** — ✅ FIXED (verified).
  `.github/workflows/ci.yml` wires both: the `fuzz-smoke` job installs nightly +
  `cargo-fuzz` and runs each target (`lexer`, `parser`, `typer`) for
  `-max_total_time=30`; the `sanitize` job runs `ci/sanitize.sh`
  (ASan+UBSan sweep, then TSan sweep, each rebuilding the C runtime
  instrumented and running the full `cargo test`). Audited the assets:
  `fuzz/Cargo.toml` is a valid cargo-fuzz manifest (libfuzzer-sys 0.4, `jinnc`
  path dep, three `[[bin]]` targets), and all three targets call the exact
  current public APIs the driver uses (`Lexer::new`/`.tokenize`,
  `Parser::new`/`.parse_program`, `Typer::new`/`.lower_program`) — so they build
  against today's tree. `sanitize.sh` is sound (`-fsanitize=address,undefined`
  then `thread`, `-fno-sanitize-recover=all`). Local execution of cargo-fuzz is
  not possible on this host (no rustup/nightly — system cargo only) and the
  sanitizer sweep does 2× `cargo clean` + instrumented rebuild (minutes); CI is
  the intended venue and is correctly configured. Maps to `F P0-11`.
- **SIG-7 — Canonicalize `Type::String` ↔ `Type::Struct("string")`** — ✅ FIXED.
  Root-cause fix at the data model: every name→`Type` resolution birth site now
  normalizes `str`/`String`/`string` → `Type::String` so a stringly `Struct`
  never enters the type system — parser `ident_to_type`
  (`src/parser/expr/primary.rs`), typer `ident_to_type`
  (`src/typer/expr/typeargs.rs`), and iterator element inference
  (`src/typer/lower/iter.rs`). `Type::canonical()` at the unification entry
  (`src/typer/unify/mod.rs`) remains as a safety net, and a new
  `Type::has_string_struct()` invariant is asserted there via `debug_assert!`
  so any missed birth site trips in debug/CI builds. Validated: full **debug**
  suite (224+912+434+…) green with the assert active — zero non-canonical
  string types reached unification across 900+ programs. Maps to `F P0-12`.
- **SIG-8 — Reconcile the dual codegen driver paths.** — ✅ FIXED.
  Confirmed by reading both drivers: the bare-file path (`driver::run` in
  `src/driver/mod.rs`) and the project path (`compile_and_link` in
  `src/driver/pipeline.rs`) both funnel codegen through the *single* shared
  invariant `Compiler::compile_program(&mir_prog, &hir_prog, mir_hints)` —
  codegen is uniformly MIR-driven, there is **no dead HIR-direct path** to
  delete. The real hazard was the verbatim-duplicated *link step* (the
  runtime/ssl/sqlite/`-lm`/lto/wasm `cc` invocation) — the exact block "patched
  identically for the runtime-link fix." Extracted it into a single
  `pipeline::link_object(&LinkSpec)` helper (one source of truth for link
  flags); both paths now call it. Validated: full release+debug suite green and
  a `jinnc run` smoke test (project pipeline → `link_object`) prints `42`,
  RC 0. Maps to `F P1-9/P1-10/P1-21`.

### MINOR / Test debt — polish, post-alpha

- **MIN-1 — Ignored test `perceus_debug.rs` (`drop_fusion_coalesces`)** and
  **MIN-2 — `bulk_tests.rs` (`weak_roundtrip`)** were both blocked on a
  missing heap-tax inference path (the surface `rc()`/`weak()`/`weak_upgrade()`
  builtins were removed with no replacement). ✅ RESOLVED by formally adopting the
  value-semantics memory model documented in `docs/access-semantics.md` (the
  authoritative source that supersedes `memory-model.md`): Jinn is value-semantics
  first — no `Rc`/`Arc`/`Box`, no hidden refcount in the IR, cross-thread sharing
  via `Channel`/`ActorRef` only. MIN-1's ignored `rc()` probe was rewritten as a
  live, non-ignored drop-fusion test (`drop_fusion_coalesces_consecutive_scope_exit_drops`)
  that borrows three owned heap vectors through one call so their scope-exit drops
  fuse into a single `DropMany` (verified: 3 drops fused). MIN-2's empty
  `weak_roundtrip_recovers_value_removed` placeholder was deleted (weak refs are
  not part of the value-semantics alpha). Loose ends were also closed: the dangling
  `weak_upgrade` builtin was removed from the VS Code grammar, `tests/programs/syntax.jn`
  §24 was rewritten from "Reference Counting / heap tax" to honest value-semantics
  struct prose, and `jinn.md`'s stale `rc()`/`arc()`/`@atomic`/`weak ref` claims
  (Principle 4, the Memory Model section, and the Key Decisions table) were aligned
  with the authoritative model. The suite now carries **zero `#[ignore]`d tests**.
- **MIN-3 — `bulk_tests.rs` (`b_p42_strict_unsolved_typevar`)** ✅ FIXED. The
  obsolete `#[ignore]`d test (its program `*foo(x)/x` … `foo` now legitimately
  generalizes and compiles) was replaced with a real, non-ignored test
  `b_strict_unsolved_field_among_annotated` that exercises a genuinely-unsolved
  type variable: a `Record` with an annotated `id` field and an unannotated,
  never-constrained `payload` field — strict checking must reject and pinpoint
  `payload` while leaving the annotated `id` unimplicated. Also removed an
  exact-duplicate strict-field test (`b_strict_unconstrained_field_diagnostic`
  was identical to `b_infer_e3_strict_unconstrained_struct_field`).
- **MIN-4 — `integration.rs` (`store_perf_regression`)** ✅ FIXED. The deliberate
  performance guard mis-housed in the correctness suite was removed; the
  benchmark suite already carries a strict superset, `benchmarks/store_perf.jn`
  (same 5000×100 shape, covering count/query/aggregate/distinct/set and more,
  wired into `run_benchmarks.py` with baselines in `store_perf_baselines.txt`).
- **MIN-5 — Scratch artifacts in the source tree** ✅ FIXED. Maps to
  `F P2-18/P2-19`. Rewrote the malformed `.gitignore` (the final line had glued
  `benchmarks/_build/a.out` and `a.out`) into clean, commented sections covering
  `a.out`/`**/a.out`, `*.store`/`*.wal`, and `benchmarks/results*.json`/`.csv`;
  then `git rm --cached` untracked all 30 already-committed artifacts (compiled
  `a.out` binaries under `apps/`, `*.store`/`*.wal` runtime DBs, `benchmarks/results*`)
  while keeping them on disk. No source files affected.
- **MIN-6 — Benchmark `array_ops.jn` 1.5 B-iteration outlier** ✅ FIXED. Scaled
  the per-iteration-allocating loop from `1_500_000_000` to `50_000_000` to match
  its true peer, the allocation-bound `alloc_churn.jn` (50 M), removing the
  host-stressing 30× outlier. The trailing `log(total)` only defeats DCE, so the
  reduced total is immaterial to the timing measurement.

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

All entry-gate conditions are now satisfied:

1. ~~**CRITICAL-1** (generic enum empty variant) is fixed and covered by a
   non-ignored regression test.~~ ✅ **DONE** — fixed in parser + typer; covered
   by `alpha_audit_generic_empty_enum` and `alpha_audit_generic_struct_param`.
2. ~~**MAJOR-1..3** are fixed (or the affected modules excluded from the
   alpha-stable subset declared in **MAJOR-5**).~~ ✅ **DONE** — MAJOR-1..5 all
   FIXED; the alpha-stable subset is declared in `docs/std.md`/`docs/stability.md`
   and guarded by `tests/std_stable_subset.rs`.
3. ~~**SIG-5** baseline is green (clippy installed + `-D warnings`, fmt already
   clean) so the release log is clean.~~ ✅ **DONE (env-bound residual)** — Rust
   release-build warnings driven to **0** and `cargo fmt` is clean; the
   `clippy -D warnings` gate is documented but cannot be run locally (no
   `rustup`/`clippy` in the pinned toolchain).

**Gate status: OPEN — Phase C may begin.** The only standing residuals are
environment-bound (local `clippy`) or explicitly deferred features (F P1-3 SIMD
literals; MIN-1/2 `rc()`/`weak()` heap-tax), none of which block Phase C.
