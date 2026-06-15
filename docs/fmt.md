# `jinn fmt` — Technical Specification

`jinn fmt` is Jinn's code formatter and idiomatic-rewrite engine. It is to
Jinn what `rustfmt` is to Rust, **plus** a behavior-preserving idiomatic
rewriter and an integrated linter (`jinn lint`). Its mission: take any
well-formed Jinn source — including the un-idiomatic, C-transliterated, or
just-plain-ugly code catalogued in [`docs/idiomatic.md`](idiomatic.md) — and
turn it into beautiful, clean, idiomatic Jinn **without changing observable
behavior**, while surfacing bugs and syntax problems as diagnostics.

This spec is **Part 2** of the initiative. Part 1 ([`idiomatic.md`](idiomatic.md))
defines *what* idiomatic Jinn is; this document defines *how* the tool gets
there, the safety model, the architecture, the rule taxonomy, the CLI, and the
guarantees.

---

## 1. Status quo & the gap this closes

A naive formatter already exists:

- CLI: `jinnc fmt [paths…]` dispatches in [`src/driver/mod.rs`](../src/driver/mod.rs)
  (`Cmd::Fmt`) to `crate::fmt::format_source`.
- Impl: [`src/fmt.rs`](../src/fmt.rs) (648 LOC) lexes → parses to `ast::Program`
  → **pretty-prints the AST** back to source, writing the file in place.

This is an *AST reprinter*, and it has two disqualifying flaws for a real
formatter:

1. **It destroys comments.** The lexer discards `#` comments at the source
   (`skip_line()` in [`src/lexer/mod.rs`](../src/lexer/mod.rs)); they never reach
   the AST, so the reprint silently deletes every comment in the file. This
   alone makes the current tool unsafe to run on real code.
2. **It cannot rewrite idioms** and offers **no lint, no idempotency proof, no
   diff/check mode, no config.**

`jinn fmt` as specified here is a ground-up redefinition built on the existing
compiler frontend, designed so that the trivial pretty-printer becomes one
(lowest) layer of a layered, safety-tiered pipeline.

---

## 2. Design goals & non-goals

**Goals**

- **Behavior preservation is sacred.** Default-tier formatting and rewrites
  must never change the program's observable behavior. This is *verified*, not
  merely intended (§7).
- **Comment- and trivia-faithful.** Comments, banner sections, blank-line
  intent, and shebangs survive formatting and travel with the construct they
  annotate.
- **Idempotent.** `fmt(fmt(x)) == fmt(x)` for all inputs, byte-for-byte.
- **Idiomatic rewriting**, tiered by the knowledge each rewrite needs
  (lexical → flow-shape → type-aware → lint-only), per idiomatic.md.
- **One frontend.** Reuse jinnc's lexer/parser/HIR. The formatter is not a
  second grammar; a construct that parses for the compiler formats here, and
  vice-versa. This keeps the two from drifting.
- **Fast & incremental.** Format-on-save latency target < 50 ms for a 1k-LOC
  file; integrates with the LSP ([`docs/lsp.md`](lsp.md)).

**Non-goals**

- Not a refactoring engine (no rename, no extract-function).
- Does not reflow or rewrite string *contents*.
- Does not "optimize" — semantic equivalence only; performance idioms (e.g.
  `StringBuilder`) are *suggested* by lint, never silently substituted.

---

## 3. Architecture

```
                 ┌────────────────────────────────────────────┐
   source.jn ──▶ │ 1. Lexer (+ trivia)   tokens + comment/ws    │
                 │ 2. Parser             CST (lossless) + AST    │
                 │ 3. [opt] Typer/HIR    resolved names, types   │
                 └───────────────┬──────────────────────────────┘
                                 ▼
                 ┌────────────────────────────────────────────┐
                 │ 4. Rule engine (tiered passes over CST/HIR)  │
                 │      T1 lexical · T2 flow · T3 type · T4 lint │
                 └───────────────┬──────────────────────────────┘
                                 ▼
                 ┌────────────────────────────────────────────┐
                 │ 5. Printer (CST → text, trivia-reattached)   │
                 │ 6. Verifier (re-lex/parse, equivalence)      │
                 └───────────────┬──────────────────────────────┘
                                 ▼
                       formatted.jn  |  diff  |  diagnostics
```

### 3.1 Lossless syntax tree (CST) — the central new artifact

The existing AST is *lossy*: it has no slot for comments, blank lines, or the
exact surface spelling of aliased keywords (`eq` vs `equals`). A faithful
formatter needs a **concrete syntax tree** that retains:

- **Trivia**: comments and blank lines, each tagged as *leading*, *trailing*,
  or *dangling* relative to the nearest syntax node, with the originating
  byte span.
- **Surface tokens**: the exact keyword spelling chosen by the author, so that
  normalization (S2-2) is a deliberate rule rather than an accident of
  reprinting.

Two viable implementations:

1. **Green/red tree** (Roslyn/`rowan` style): a single lossless tree the
   parser builds, from which the typed AST is a *view*. Cleanest long-term;
   larger change to the parser.
2. **AST + sidecar trivia table** (pragmatic first step): keep `ast::Program`,
   but (a) stop discarding comments in the lexer — emit them as `Trivia` tokens
   with spans — and (b) build a `TriviaMap: Span → Vec<Trivia>` the printer
   consults to re-attach comments to the node whose span they precede/follow.

This spec mandates **(2) as the v1 deliverable** (smallest change that makes
the tool *safe*) with a documented migration path to **(1)**. The single
hard requirement either way: **the lexer must stop dropping comments.** Today
[`src/lexer/mod.rs`](../src/lexer/mod.rs) does `b'#' => { self.skip_line(); }`;
under v1 it instead records a `Token::Comment(span)` (filtered out before the
parser sees the token stream, collected into the trivia table).

### 3.2 Reuse of the compiler frontend

| Stage | Module | Role in fmt |
| --- | --- | --- |
| Lexer | `src/lexer/` | tokens + **new** trivia capture |
| Parser | `src/parser/` | builds AST/CST; spans already present (`Spanned`, `Span`) |
| Resolve | `src/resolve.rs`, `src/bind.rs` | name resolution for T3 (self-strip) |
| Typer/HIR | `src/typer/`, `src/hir/` | inferred types for T3 (cast-strip) |
| Diagnostics | `src/diagnostic.rs` | shared diagnostic rendering for lint |

The formatter never re-implements parsing. If `parse_program` rejects the
input, `fmt` reports the parse error and **makes no changes** (you cannot
format what does not parse). `lint` may still run its lexical checks.

---

## 4. Tiered rule engine

Rules are grouped into four tiers by the knowledge they require and the risk
they carry. The tier determines *when* a rule runs and *whether* it may mutate.

| Tier | Needs | Default | Mutates? | Guarantee |
| --- | --- | --- | --- | --- |
| **T1 Lexical/CST** | tokens + tree | on | yes | safe by construction |
| **T2 Flow-shape** | tree + local dataflow | on | yes | safe when shape matches exactly |
| **T3 Type-aware** | resolved HIR + types | `--type-aware` | yes | verified against typer |
| **T4 Lint** | any | `lint` only | no (suggests) | author opt-in per fix |

Each rule has a stable **id** (e.g. `J1001`), a tier, a one-line title, a
`fixable: auto | suggest | none` field, and a doc link into idiomatic.md.

### 4.1 T1 — Lexical / CST (always on, always safe)

Pure tree-to-text and keyword-level rewrites; cannot change behavior.

| id | rule (→ idiomatic.md) | action |
| --- | --- | --- |
| `J0001` | indentation = 4 spaces; no tabs | reindent |
| `J0002` | trailing whitespace; single trailing newline | strip |
| `J0003` | one blank line between top-level defs; none at block open | normalize |
| `J0004` | banner-comment width normalization (S3-1) | normalize |
| `J0010` | drop tail `return` (S2-1) | rewrite |
| `J0011` | `else { if … }` → `elif` (S1-5, collapse only) | rewrite |
| `J0012` | normalize keyword spelling: `eq`→`equals`, `ngt`→`lte`, … (S2-2) | rewrite |
| `J0013` | always rewrite double-negative comparison aliases (`ngte`/`nlte`/`ngt`/`nlt`) | rewrite |

### 4.2 T2 — Flow-shape (on by default; matches a *closed* shape or skips)

Each rule recognizes one exact statement/expression shape and rewrites it. If
the shape is not matched exactly (extra reads, aliasing, side effects in the
condition/step), the rule **does nothing** — no partial rewrites.

| id | rule | matched shape → rewrite |
| --- | --- | --- |
| `J1001` | C-style counted loop → iteration (S1-1) | `loop(0, $ < xs.len(), $ + 1)` whose body indexes only `xs.get($)` → `loop xs` with `$`; pure-range form → `for i in 0 to n` |
| `J1002` | `while`+manual-index → `loop xs` (S1-2, conservative) | `i is 0; while i < xs.length { …xs[i]…; i is i + 1 }` with no other use of `i` → `loop xs` |
| `J1003` | int-flag → bool (S1-4) | var only ever `0/1`, compared only `equals 0/1` → `bool`/`true`/`false` |
| `J1004` | predicate fn `if c { return 1 } return 0` → `c` (S1-4) | tail boolean collapse |
| `J1005` | `vec()`+counted pure-push → comprehension (S3-3) | `v is vec(); loop(…) v.push(e($))` → `v is [e($) for $ in …]` |

Shape matching uses a small local dataflow check on the CST: for `J1001`,
verify the loop variable is `$`, the bound mentions only `xs.len()`/a constant,
the step is `$ + 1`, and every use of `$` inside the body is `xs.get($)` (which
becomes `$`) or `$$`-as-index (kept). Anything else aborts the rewrite.

### 4.3 T3 — Type-aware (opt-in `--type-aware`; verified)

Runs the typer/resolver first; if type-checking fails, T3 is skipped and a note
is emitted (cannot safely reason about names/types). Each rewrite is re-checked
to type-check identically afterward.

| id | rule | requires |
| --- | --- | --- |
| `J2001` | strip redundant `self.` (S1-6) | name resolution: identifier resolves unambiguously to a receiver field/method **and** is not shadowed by a param/local |
| `J2002` | strip redundant `as T` cast/annotation (S2-3) | inferred type without the cast is identical |
| `J2003` | field-punning `T(x, y)` for `T(x is x, y is y)` (S2-4) | locals match field names; punning sugar exists in grammar |

### 4.4 T4 — Lint (suggest-only; `jinn lint`)

Behavioral risk or aesthetic judgment too high to auto-apply. Emits a
diagnostic with a *suggested* edit; `jinn lint --fix` applies only those the
author has not `#[allow]`-ed and that are marked machine-applicable.

| id | rule | nature |
| --- | --- | --- |
| `J3001` | O(n²) `out is out + …` in loop → `join`/`StringBuilder` (S1-3) | perf+style suggestion |
| `J3002` | dense `char_at`+magic-number byte loops → scalar APIs (S1-7) | readability; stability-pinned surface |
| `J3003` | deeply nested `? / !` chain → `match` (S3-2; see §4.5, `J3111`) | readability |
| `J3004` | `if x equals A … elif x equals B … else` on one scrutinee → `match` (S1-5 promote) | readability |

#### Bug-finding lints (beyond style)

`jinn lint` also runs correctness checks that the type-checker does not gate on
but that almost always indicate bugs:

| id | rule |
| --- | --- |
| `J3100` | unused binding (`x is …` never read) |
| `J3101` | unreachable code after `return`/`break`/`continue` |
| `J3102` | shadowed binding that is likely a typo (same name, never reads outer) |
| `J3103` | `match` missing variants / redundant arm (uses enum info from HIR) |
| `J3104` | comparison that is always true/false (e.g. `x equals x`, `0 < x < 0`) |
| `J3105` | assignment-in-condition / `is` (binding) where `equals` (compare) was meant |
| `J3106` | result/error value ignored where a function `returns` an `err` type |
| `J3107` | empty block / no-op statement |

`J3105` is the standout bug class Jinn's `is`-binds / `equals`-compares split
invites; the linter flags `if x is y` used as a condition (a binding in a
boolean position) as a near-certain mistake.

### 4.5 The conditional operator `? / ! / !!` — canonical layout

Jinn does not have a separate ternary-versus-`if` distinction. `? / ! / !!` is
**one** indentation-aware decision construct, and there is no statement-form
`if` keyword for it to compete with: arms are glyph-tagged so arity is
unambiguous at the marker, an arm is simply **absent** for a guard, **inline**
for a one-liner, and **indented** for a block. The formatter treats this as the
universal conditional and is the single source of truth for its layout.

**Arm grammar (what the markers mean).**

| marker | arm | required? |
| --- | --- | --- |
| `?` | predicate, then opens the consequent | always |
| (consequent) | the `then` arm | optional (absent ⇒ guard) |
| `!` | the `else` arm | optional |
| `!!` | the **error** arm — only legal when the scrutinee is an error-carrying type | optional |

`!!` is the one type-sensitive marker: it lights up only on a result/error-like
scrutinee, where `? / ! / !!` is sugar for `match` over `Ok / else / Err`. On a
plain `bool` scrutinee `!!` is a **compile error** (and `J3110`, below, flags it
in lint). This keeps the construct honest: two-armed forms are conditionals,
the `!!` form is match-on-a-sum.

**Inline vs. block (the layout rule).** An arm is **inline** when it is a single
expression; it goes on the operator line. An arm becomes a **block** the moment
it holds more than one statement (or a `let`/loop/nested decision); it then
opens an indented body under its marker. The formatter never crams a
multi-statement arm into a parenthesized `;`-chain — indentation is the block
form.

```
foo ? ok()                       # one-armed guard (then only)
foo ? ok() ! no()                # symmetric two-armed
not foo ? no()                   # preferred over an empty then-arm: foo ? ! no()
foo ? ok() ! no() !! err         # error-carrying scrutinee (match sugar)

user ?                           # multi-statement arms → indented blocks
    validate()
    save()
!
    return
```

**Canonical-form rules (T1/T2, behavior-preserving).**

| id | rule | action |
| --- | --- | --- |
| `J0014` | one space around `?`, `!`, `!!`; no space before the marker's body when inline | normalize |
| `J0015` | empty then-arm `foo ? ! no()` → `not foo ? no()` (negate predicate, drop empty arm) | rewrite |
| `J1006` | arm whose body is a single expression collapses inline; an arm with ≥2 statements expands to an indented block; markers (`!`, `!!`) sit at the parent indent above their block | normalize |
| `J1007` | parenthesized `;`-chain in an arm `foo ? (a(); b()) ! …` → indented block form | rewrite |

**Lint (T4).**

| id | rule | nature |
| --- | --- | --- |
| `J3110` | `!!` arm on a non-error (`bool`) scrutinee | bug (type misuse) |
| `J3111` | nested `? … ! ?`-chains ≥3 deep on distinct scrutinees → `match` (readability) | suggestion |

This subsumes the older `J3003` (deeply nested ternary): a chain that is really
multi-way dispatch over one scrutinee is promoted to `match` (`J3004`/`J3111`),
while a genuinely two-armed decision stays as `? / !`.

---

## 5. Configuration — `fmt.toml` / `[fmt]` in `project.jn`

Zero-config by default (the rules above with their default tiers). Overridable
per project. Settings are deliberately few — opinionated like `rustfmt`.

```toml
[fmt]
indent = 4                      # spaces; tabs are a lex error anyway
max_width = 96                  # soft wrap target for call args / lists
blank_lines_between_defs = 1
keyword_style = "prose"         # "prose" (equals/neq/lte) | "short" (eq/neq/lte)
banner_width = 60               # section-comment "# ── … ──" width
trailing_return = "strip"       # strip | keep
type_aware = false              # enable T3 by default
newline = "lf"

[fmt.rules]
J1001 = "on"                    # per-rule on | off | warn
J2001 = "on"
J3001 = "warn"                  # lints default to warn

[lint]
deny = ["J3105", "J3106"]       # escalate to errors (fail CI)
allow = ["J3002"]               # silence on this project
```

In-source overrides mirror the diagnostic ecosystem: a line/block can carry
`#[fmt:off]` … `#[fmt:on]` to disable formatting for a region (verbatim
passthrough), and `#[allow(J3002)]` / `#[deny(J3105)]` to tune a single lint at
a definition.

---

## 6. CLI

`fmt` and `lint` are top-level `jinnc` subcommands, extending the existing
`Cmd::Fmt`.

```
jinnc fmt [PATHS…]              format files/dirs in place (default: rewrite)
  --check                       exit non-zero if any file would change; print diff; no write
  --diff                        print unified diff to stdout; no write
  --stdin                       read source from stdin, write formatted to stdout
  --type-aware                  enable T3 rules (runs the typer)
  --tier <1|2|3>                cap the highest tier applied (default 2; 3 with --type-aware)
  --only <ids>                  run only the listed rule ids
  --skip <ids>                  skip the listed rule ids
  --quiet | --verbose

jinnc lint [PATHS…]            run T4 + bug lints; report diagnostics
  --fix                         apply machine-applicable suggestions
  --deny <ids> / --allow <ids>  override config severities
  --format <human|json>         json for editor/CI integration

# convenience: bare `jinn` wrapper forwards `jinn fmt` / `jinn lint`
```

Behavioral notes:

- No paths → recurse CWD for `*.jn`, skipping `target/` and `.git/` (matching
  the existing `collect_jinn_files` walk in the driver).
- `--check` is the CI gate: it formats in memory and diffs; non-empty diff →
  exit 1. This replaces the current always-write behavior with a safe default
  for pipelines.
- Parse failure on a file: report the compiler's parse diagnostic, leave the
  file untouched, continue with the rest, exit non-zero.

---

## 7. Safety & correctness model

The whole tool lives or dies on *never silently breaking code*. Layered
defenses:

### 7.1 Tier guarantees by construction

- **T1** rewrites are local token/tree transforms with no semantic content
  (whitespace, comment width, keyword *alias* normalization, `elif` collapse,
  tail-`return` drop). Each is accompanied by an argument for why it is a
  no-op on behavior, captured as a property test (§8).
- **T2** rewrites only fire on an exactly-matched closed shape; the matcher is
  the proof obligation. A mismatch is a skip, never a guess.
- **T3** rewrites are re-checked: after applying, the file is re-typed and the
  inferred types of all affected bindings must be unchanged; otherwise the
  rewrite is rolled back. `self.`-strip additionally requires the name to be
  unshadowed in the post-rewrite scope.

### 7.2 The verifier (always-on backstop)

After all enabled passes, the printed output is **re-lexed and re-parsed**, and
the new AST is compared to the original AST under a **semantic-equivalence
normalization** that erases exactly the differences the applied rules are
*allowed* to introduce (keyword spelling, redundant `return`/`self.`/`as`, the
T2 loop-shape equivalences) and nothing else. If any other difference appears,
`fmt` **aborts and writes nothing**, emitting an internal-error diagnostic with
the offending node. This makes "the formatter changed behavior" a tool bug that
fails loudly rather than a silent corruption.

For `--type-aware`, the verifier additionally requires the post-format program
to type-check (no new type errors introduced).

### 7.3 Idempotency

`fmt` must satisfy `fmt(fmt(x)) == fmt(x)` byte-for-byte. Enforced by a
fixed-point loop capped at 2 iterations: format once, format the result; if
they differ, that is a tool bug (fails in CI via the property test in §8). The
printer is written so a single pass already reaches the fixed point for all
rules; the second pass is a guard, not a strategy.

---

## 8. Testing

Mirrors the project's conformance-suite convention (compile-and-run tests pin
documented semantics):

1. **Golden tests** (`tests/fmt_golden.rs`): `input.jn` → `expected.jn` pairs,
   one directory per rule id, sourced directly from the real smells in
   idiomatic.md (e.g. the actual `apps/bank_ledger/source/ledger.jn` loop).
2. **Idempotency** (`tests/fmt_idempotent.rs`): for every `*.jn` in the repo,
   assert `fmt(src) == fmt(fmt(src))`.
3. **Behavior preservation** (`tests/fmt_equiv.rs`): for every runnable snippet
   and app, compile-and-run *before* and *after* `fmt --type-aware` and assert
   identical stdout/exit. This is the empirical proof that formatting the whole
   corpus changes no output.
4. **Comment fidelity** (`tests/fmt_trivia.rs`): assert every comment present in
   the input is present in the output, attached to the same construct.
5. **Lint snapshots** (`tests/lint_snapshot.rs`): for crafted inputs, assert the
   exact set of diagnostics (id + span) — including the bug lints J3100–J3107.
6. **Property tests** (`fuzz/`): random well-formed programs round-trip through
   the verifier (§7.2) with zero aborts.

CI runs `jinnc fmt --check .` and `jinnc lint --deny J3105,J3106 .` so the repo
itself is kept idiomatic and the smells in idiomatic.md cannot regress.

---

## 9. Rollout plan

| Phase | Deliverable | Risk |
| --- | --- | --- |
| **0** | Stop discarding comments in the lexer; trivia table; printer re-attaches comments. Make the *existing* reprinter comment-safe. | low; unblocks everything |
| **1** | T1 rules + `--check`/`--diff`/`--stdin` + verifier + idempotency tests. Ship safe formatter. | low |
| **2** | T2 flow-shape rewrites (J1001 first — the biggest corpus win) + golden/equiv tests. | medium |
| **3** | `jinn lint` with bug lints (J3100–J3107) + T4 suggestions + `--fix`. | medium |
| **4** | T3 type-aware rewrites (self-strip, cast-strip) behind `--type-aware`. | higher; gated on typer |
| **5** | LSP integration: format-on-save, lint diagnostics, code-actions for suggestions. | low |

Phase 0 is the immediate priority: the current `jinnc fmt` is *unsafe* (eats
comments) and should be guarded behind `--unsafe-reprint` or fixed before it is
recommended.

---

## 10. Summary

`jinn fmt` is a layered, frontend-reusing, safety-tiered formatter that does
three jobs the current reprinter cannot: it **preserves comments and trivia**,
it **rewrites un-idiomatic Jinn into the idioms specified in
[`idiomatic.md`](idiomatic.md)** under a behavior-preservation guarantee that is
*verified* by re-parsing and (for type-aware rules) re-typing, and it
**lints for bugs** — most distinctively the `is`/`equals` confusion the
language's binding/comparison split invites. Default-tier formatting is safe by
construction and idempotent; deeper rewrites are opt-in and machine-checked;
and the whole corpus's runtime output is pinned identical before and after by
the conformance suite. The result: a single command that turns the accumulated
syntactic debt of the codebase into the clean, inference-driven,
implicit-self, prose-like Jinn the language was designed to be.
