# §4 Lexer review

**File:** `src/lexer/mod.rs` (1,357 LOC). **Submodules:** `literals`,
`token`.

## 4.1 Summary

The lexer is **the strongest pass in the compiler.** It is small,
focused, single-purpose, has a clean `Lexer::tokenize() -> Result<Vec<Spanned>, LexError>`
contract, a hand-maintained `KEYWORDS` table (`LazyLock<HashMap>`), and
correctly handles:

- Indentation as a first-class concept (with `indents: Vec<u32>` stack
  to drive INDENT/DEDENT tokens; reads as Python's lexer with the
  Python warts intact — no mixing tabs and spaces; `\t` is a hard
  lexer error).
- Shebang lines (`#!/usr/bin/env jinnc run`) skipped at file head.
- A clean `Spanned { token, span }` shape with optional `file: Symbol`
  for multi-file diagnostics.

No `unsafe` in the lexer. No `panic!()` in the lexer.

## 4.2 Findings

### F-LEX-1 (medium): `#` is the only comment form

Verified by failing probe v1: `// comment` is lexed as `/` `/` and
fails parsing. Jinn's `#` comments are documented but not advertised;
a user coming from C / Rust / Go / Java / Zig will reach for `//`
first. The `/ /` token sequence then produces confusing parse errors
several tokens later.

**Recommendation:** Lex `//` and `/*…*/` as comments **with a
*deprecation diagnostic*** ("Jinn uses `#` for comments; this `//` is
treated as a comment but will be removed in a future release"). This
buys the language one major UX papercut on day-one users for free.

### F-LEX-2 (low): The keyword table is enormous

≈ 80 reserved words including `send`, `receive`, `close`, `select`,
`dispatch`, `view`, `migration`, `transaction`, `set`, `default`,
`pow`, `mod`, `eq`, `neq`, `lt`, `gt`, `lte`, `gte`, `nlt`, `ngt`,
`nlte`, `ngte`, `xor`, `unreachable`, `defer`, `grad`, `einsum`,
`build`, `syscall`, `global`, `nop`, `embed`, `assert`, `query`,
`store`, `insert`, `delete`, `set`, `transaction`, `actor`, `spawn`,
`send`, `receive`, `trait`, `impl`, `dispatch`, `yield`, `channel`,
`close`, `select`, `stop`, `default`, `sim`, `supervisor`, `atomic`,
`strict`, `alias`, …

Consequences observed in probes:

- `ch.send(1)` fails to parse: `send` is a keyword, so the field-access
  position sees a keyword token and breaks. This is the immediate
  cause of `p15a_channel_method.jn` failing.
- The same applies to `.close()`, `.select()`, `.delete()`, `.set()`,
  `.insert()`, `.query()`, `.view()`, `.spawn()`, etc.

This means **users cannot give their own types method names matching
common verbs.** That is a significant ergonomic tax.

**Recommendation:** The lexer should NOT promote identifiers to
keywords when the previous token is `.` (member access). This is the
standard Python / Swift / Kotlin trick: contextual keywords. The
change is local to `lex_token` / `lex_ident` and is small.

### F-LEX-3 (low): String literal escape handling truncates UTF-8

Probe `p43_escape.jn`: the emoji `🎉` (4 UTF-8 bytes) round-trips as
`ð` (1 byte). Inspection should be in `src/lexer/literals.rs`. The
likely cause is either a bytewise length being used where char-count
is needed, or `Write` going through `write!` with an `i8` cast. Worth
a separate ticket — it is a footgun for any user who types a non-ASCII
character.

### F-LEX-4 (low): No raw / heredoc / triple-quoted strings

Probes show that escape handling works correctly for `\n`, `\t`, `\'`,
`\\`, but there is no raw-string form. For SQL, JSON, regex literals,
shader code, and especially `embed` / `migration` blocks the absence
is felt.

### F-LEX-5 (very low): `pow` is a keyword aliased to `StarStar`

`("pow", Token::StarStar)`. This is harmless but unusual; it means
`pow` can never be a function name.

## 4.3 What's correct

- `\t`-rejection is principled (avoids the indentation ambiguity that
  Python suffered from for decades).
- `Spanned` carrying full file/line/col info is exactly what diagnostics
  need.
- Shebang skip allows `jinnc run`-style scripting.
- The numeric literal parser supports `0x...`, `0b...`, underscores
  in numbers, `_i32` / `_u64` suffixes etc. (verified by `tests/programs/`).

## 4.4 Verdict

The lexer is **alpha-ready** modulo F-LEX-2 (contextual keywords)
which is the only finding here that materially impedes user code.
F-LEX-1 (`//` comments) is a UX nice-to-have, not a blocker.
