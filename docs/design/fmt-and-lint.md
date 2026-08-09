# `jinn fmt` and `jinn lint` — design

> **Status: design.** What exists today is a naive reprinter — `src/fmt.rs`
> lexes, parses to `ast::Program`, and pretty-prints the AST back to source. Its
> grammar coverage has drifted: 164 of 688 corpus files stop compiling after
> formatting, 41 of them in `std/` (`X-1` in
> [`../roadmap.md`](../roadmap.md#tooling)). `--check`, `--diff`, `--stdin`, a
> `jinn lint` subcommand, and the rule engine below do not exist.

The target: take any well-formed Jinn source — including un-idiomatic or
C-transliterated code — and turn it into clean, idiomatic Jinn **without
changing observable behavior**, while surfacing bugs and syntax problems as
diagnostics. The style it converges on is documented in
[`../jinn.md`](../jinn.md#idiomatic-jinn).

## Goals and non-goals

**Goals.** Behavior preservation is sacred, and *verified* rather than merely
intended. Comments, banner sections, blank-line intent, and shebangs survive and
travel with the construct they annotate. Formatting is idempotent —
`fmt(fmt(x)) == fmt(x)` byte-for-byte. Rewrites are tiered by the knowledge each
one needs. There is **one frontend**: the formatter reuses `jinnc`'s lexer,
parser, and HIR, so a construct that parses for the compiler formats here and
vice versa — which is precisely the drift that produced `X-1`. Format-on-save
latency should stay under 50 ms for a 1k-line file.

**Non-goals.** Not a refactoring engine — no rename, no extract-function. Does
not reflow or rewrite string *contents*. Does not optimize: performance idioms
are *suggested* by lint, never silently substituted.

## Architecture

```
source.jn ──▶ 1. Lexer (+ trivia)      tokens + comments/whitespace
              2. Parser                 CST (lossless) + AST
              3. [opt] Typer/HIR        resolved names, types
                    │
                    ▼
              4. Rule engine            T1 lexical · T2 flow · T3 type · T4 lint
                    │
                    ▼
              5. Printer                CST → text, trivia reattached
              6. Verifier               re-lex, re-parse, check equivalence
                    │
                    ▼
              formatted.jn  |  diff  |  diagnostics
```

### The lossless tree is the central new artifact

The existing AST has no slot for comments, blank lines, or the exact surface
spelling of aliased keywords (`eq` versus `equals`). A faithful formatter needs
a tree that retains **trivia** — comments and blank lines, each tagged leading,
trailing, or dangling relative to the nearest node, with its byte span — and
**surface tokens**, so normalization is a deliberate rule rather than an
accident of reprinting.

Two implementations are viable: a green/red tree from which the typed AST is a
view (cleanest long-term, larger parser change), or an AST plus a sidecar trivia
table (the pragmatic first step). **v1 is the sidecar**: the smallest change
that makes the tool safe, with a documented migration path to the green tree.

Either way there is one hard requirement: **the lexer must stop dropping
comments.** Today it does `b'#' => { self.skip_line(); }`; it must instead
record a comment token with a span, filtered out before the parser sees the
stream and collected into the trivia table.

The formatter never re-implements parsing. If the parser rejects the input,
`fmt` reports the parse error and **makes no changes** — you cannot format what
does not parse. `lint` may still run its lexical checks.

## Tiered rule engine

| Tier | Needs | Default | Mutates | Guarantee |
| --- | --- | --- | :-: | --- |
| **T1 lexical** | tokens + tree | on | yes | safe by construction |
| **T2 flow-shape** | tree + local dataflow | on | yes | safe when the shape matches exactly |
| **T3 type-aware** | resolved HIR + types | `--type-aware` | yes | verified against the typer |
| **T4 lint** | any | `lint` only | no (suggests) | author opts in per fix |

Each rule has a stable id, a tier, a one-line title, and a `fixable:
auto | suggest | none` field.

**T1** is pure tree-to-text and keyword-level rewriting that cannot change
behavior: four-space indentation, trailing-whitespace and final-newline
normalization, blank lines between top-level definitions, banner-comment width,
dropping a tail `return`, collapsing `else` containing a lone `if` into `elif`,
and normalizing keyword spellings — including always rewriting the
double-negative comparison aliases (`ngte`, `nlte`, `ngt`, `nlt`).

**T2** recognizes one exact statement or expression shape and rewrites it. If
the shape is not matched exactly — extra reads, aliasing, side effects in the
condition or step — the rule **does nothing**. No partial rewrites, ever. The
matcher *is* the proof obligation. The rules worth having, in corpus-impact
order:

- C-style counted loop → iteration. `loop(0, $ < xs.len(), $ + 1)` whose body
  indexes only `xs.get($)` becomes `loop xs`; a pure-range form becomes
  `for i in 0 to n`. Matching verifies the loop variable is `$`, the bound
  mentions only the length or a constant, the step is `$ + 1`, and every use of
  `$` in the body is an index into the same collection.
- `while` with a manual index → `loop xs`, when nothing else uses the index.
- Integer flag → `bool`, when a variable is only ever assigned 0 or 1 and only
  ever compared against 0 or 1.
- `if c { return 1 } return 0` → `c`.
- `vec()` plus a counted pure `push` → a comprehension.

**T3** runs the resolver and typer first and is skipped entirely if type
checking fails. It covers stripping redundant explicit `self.` inside methods
and casts or annotations the inferencer already supplies.

**T4** never mutates. It reports, and offers fixes the author applies.

## Safety model

The tool lives or dies on never silently breaking code, so the defenses are
layered.

**Tier guarantees by construction.** T1 rewrites are local transforms with no
semantic content, each with an argument for why it is a behavioral no-op,
captured as a property test. T2 fires only on an exactly matched closed shape; a
mismatch is a skip, never a guess. T3 rewrites are re-checked: after applying,
the file is re-typed and the inferred types of all affected bindings must be
unchanged, or the rewrite is rolled back.

**The verifier is an always-on backstop.** After all enabled passes, the printed
output is re-lexed and re-parsed, and the new tree is compared to the original
under a **semantic-equivalence normalization** that erases exactly the
differences the applied rules are allowed to introduce — keyword spelling,
redundant `return`/`self.`/casts, the T2 loop-shape equivalences — and nothing
else. If any other difference appears, `fmt` **aborts and writes nothing**,
emitting an internal-error diagnostic naming the offending node. This is what
turns "the formatter changed behavior" from silent corruption into a loud tool
bug. Under `--type-aware` the verifier additionally requires the post-format
program to type-check.

**Idempotency** is enforced by a fixed-point loop capped at two iterations:
format once, format the result, and a difference is a tool bug. The printer is
written so one pass already reaches the fixed point; the second is a guard, not
a strategy.

## Testing

Mirroring the project's conformance convention:

1. **Golden tests** — `input.jn` → `expected.jn` pairs, one directory per rule
   id, sourced from real code in the corpus.
2. **Idempotency** — for every `.jn` in the repo, `fmt(src) == fmt(fmt(src))`.
3. **Behavior preservation** — for every runnable snippet and app, compile and
   run before and after formatting and assert identical stdout and exit code.
   This is the empirical proof that formatting the whole corpus changes no
   output, and it is the gate `X-1` must clear.
4. **Comment fidelity** — every comment in the input is present in the output,
   attached to the same construct.
5. **Lint snapshots** — for crafted inputs, the exact set of diagnostics.
6. **Property tests** — random well-formed programs round-trip through the
   verifier with zero aborts.

CI then runs `jinn fmt --check .` and `jinn lint` over the repo, so the corpus
stays idiomatic and cannot regress.

## Rollout

| Phase | Deliverable | Risk |
| --- | --- | --- |
| **0** | Stop discarding comments in the lexer; trivia table; printer reattaches comments. Make the *existing* reprinter comment-safe and grammar-complete. | low; unblocks everything |
| **1** | T1 rules, `--check`/`--diff`/`--stdin`, the verifier, idempotency tests. Ship a safe formatter. | low |
| **2** | T2 flow-shape rewrites — the counted-loop rule first, it is the biggest corpus win — with golden and equivalence tests. | medium |
| **3** | `jinn lint` with bug lints, T4 suggestions, `--fix`. | medium |
| **4** | T3 type-aware rewrites behind `--type-aware`. | higher; gated on the typer |
| **5** | LSP integration: format-on-save, lint diagnostics, code actions. | low |

Phase 0 is the immediate priority. Until it lands, `jinn fmt --write` should be
removed or gated behind an explicit unsafe flag rather than recommended.
