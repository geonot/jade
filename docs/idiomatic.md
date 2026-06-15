# Idiomatic Jinn — Syntax Refinement & Course Correction

This document is **Part 1** of the `jinn fmt` initiative. It catalogs the
anti-patterns, smells, and non-idiomatic constructs that have accumulated
across the Jinn corpus (tests, `snippets/`, `apps/`, `benchmarks/`, `std/`,
`libjn/`) and proposes the *one true idiom* for each. Every rule here is the
specification a smell maps to in [`docs/fmt.md`](fmt.md): the formatter and
linter exist to mechanically move real code toward this north star **without
changing behavior**.

The guiding philosophy (from project conventions): *implicitness, inference,
clean natural-language syntax. The intelligent compiler does the heavy
lifting; the programmer expresses intent, not mechanism.* The reference for
"what good looks like" is [`snippets/guide_tour.jn`](../snippets/guide_tour.jn),
which is hand-written to be exemplary. Much of the rest of the corpus predates
features it should now use, or was transliterated from C/Rust mental models.

Severity legend:

- **S1 — egregious**: pervasive, actively un-Jinn, must be eradicated.
- **S2 — smell**: locally fine but a cleaner idiom exists; auto-fix.
- **S3 — taste**: stylistic normalization; auto-format.

---

## S1-1 — C-style counted `loop(0, $ < xs.len(), $ + 1)` instead of iteration

**The single worst offender.** 88 occurrences across 41 files.

```jinn
# apps/bank_ledger/source/ledger.jn:6
*post(accounts as Vec of Account, e as Entry) returns Vec of Account
    out is vec()
    loop(0, $ < accounts.len(), $ + 1)
        a is accounts.get($)
        ...
```

This is C's `for (i=0; i<n; i++)` wearing a Jinn costume. It exposes the index
`$`, forces a manual `.get($)`, hardcodes the bound, and reads like a riddle.
The corpus *already knows the right answer* — `guide_tour.jn` iterates with
`for x in xs` and `loop xs` (where `$` is the element and `$$` the index).

**Remediation — iterate the collection directly:**

```jinn
*post(accounts as Vec of Account, e as Entry) returns Vec of Account
    out is vec()
    loop accounts
        if $.id equals e.debit_id
            out.push(account.credit($, e.amount))
        elif $.id equals e.credit_id
            out.push(account.debit($, e.amount))
        else
            out.push($)
    out
```

`loop xs` binds `$` to each element and `$$` to its index — no `.get`, no
bound, no off-by-one surface. When the index is genuinely needed, `$$` is
already there. Counting loops over a pure integer range become
`for i in 0 to n`. The fixer recognizes the canonical shapes:

| Smell | Rewrite |
| --- | --- |
| `loop(0, $ < xs.len(), $ + 1)` + `xs.get($)` | `loop xs` + `$` |
| `loop(0, $ < n, $ + 1)` (no `.get`) | `for i in 0 to n` |
| `loop(a, $ < b, $ + step)` | `for i in a to b by step` |

---

## S1-2 — Manual `while`-with-index transliteration

356 occurrences of the `i is i + 1` increment idiom; many are hand-rolled
counted loops written as `while`.

```jinn
# std/strings.jn — to_upper
*to_upper(s as String) returns String
    out is ""
    i is 0
    while i < s.length
        c is s.char_at(i)
        if c >= 97 and c <= 122
            out is out + chr(c - 32)
        else
            out is out + s.slice(i, i + 1)
        i is i + 1
    out
```

Three problems compound: (a) manual index plumbing, (b) `out is out + …`
quadratic string building, (c) byte-at-a-time work the stdlib should express
as a map. Idiomatic:

```jinn
*to_upper(s as String) returns String
    s.chars().map(|c| c >= 97 and c <= 122 ? chr(c - 32) ! c).join()
```

The fixer is conservative here: it will rewrite a `while i < xs.length` /
`i is i + 1` shape whose body only reads `xs[i]` into `loop xs`, but will **not**
invent `map`/`join` (that changes the value-flow shape and risks behavior). The
linter *flags* the `out is out + …` accumulation (see S1-3) and suggests the
combinator, leaving the rewrite to the author.

**Rewrite (mechanical, safe):**

```jinn
loop s            # $ is the byte, $$ the index
    ...
```

---

## S1-3 — Quadratic string accumulation `out is out + …` in a loop

Appears throughout `std/strings.jn`, `std/json.jn`. O(n²) and noisy.

```jinn
result is ""
loop items
    if $$ > 0
        result is result + ","
    result is result + stringify($)
result
```

**Remediation — `StringBuilder` (which std already ships) or `join`:**

```jinn
items.map(stringify).join(",")
```

or, when interleaving logic, the builder:

```jinn
sb is strings.builder()
loop items
    if $$ > 0
        sb.write(",")
    sb.write(stringify($))
sb.to_string()
```

This is a **lint with suggested fix**, not an unconditional rewrite: `join`
only applies when the accumulation is a pure concatenation with a constant
separator. The formatter detects that exact shape and offers the `join`
rewrite; otherwise it flags the O(n²) pattern and points at the builder.

---

## S1-4 — Integer flags standing in for `bool`

```jinn
# apps/order_book/source/matcher.jn
cross is 0
if order.is_buy(incoming) and order.is_sell(o) and incoming.price >= o.price
    cross is 1
...
if cross equals 1 and o.qty > 0
```

`cross` is a boolean wearing `i64`. Same pattern: `found is 0 … found is 1`,
`return 0 / return 1` for predicates. Jinn has `true`/`false`/`bool` and clean
boolean operators.

**Remediation:**

```jinn
cross is (order.is_buy(incoming) and order.is_sell(o) and incoming.price >= o.price) or (order.is_sell(incoming) and order.is_buy(o) and incoming.price <= o.price)
if cross and o.qty > 0
```

The fixer rewrites the *closed* form — a variable initialized to `0`/`1`,
assigned only `0`/`1`, and compared only with `equals 0/1` — to a `bool` with
`true`/`false`. Functions whose body is `if cond \n return 1 \n return 0` (or
the boolean equivalent) collapse to `cond`.

---

## S1-5 — Nested `if`/`else` ladders instead of `elif` / `match`

27 `else`-then-`if` ladders. Example from the original S1-1 `post`: an `else`
containing a bare `if` is exactly `elif`.

```jinn
if a.id equals e.debit_id
    out.push(account.credit(a, e.amount))
else
    if a.id equals e.credit_id          # <- collapse to elif
        out.push(account.debit(a, e.amount))
    else
        out.push(a)
```

**Remediation:** `else` whose sole child is an `if` becomes `elif`. A chain of
`if x equals A … elif x equals B …` dispatching on one scrutinee over enum
variants or constants becomes a `match`. The `elif` collapse is unconditional
(pure reindent + keyword merge, always behavior-preserving); the `match`
promotion is offered only when every arm tests the *same* expression for
equality and there is a final `else`.

---

## S1-6 — Redundant explicit `self.` inside methods

963 `self.` references in `std/` alone. Yet the canonical tour
([`guide_tour.jn`](../snippets/guide_tour.jn) `type Vec3`) uses **implicit
self**: `x * other.x`, never `self.x`. Field access inside a method resolves to
the receiver automatically.

```jinn
# std/strings.jn — StringBuilder.write
*write(s as String) returns i64
    self.parts.push(s)
    self.total is self.total + s.length
```

**Remediation — drop `self.` for unambiguous field/method access:**

```jinn
*write(s as String) returns i64
    parts.push(s)
    total is total + s.length
```

The fixer strips `self.` only where the identifier resolves *unambiguously* to
a field/method of the receiver and is **not shadowed** by a parameter or local
of the same name. Where a local shadows a field, `self.` is required for
disambiguation and is **kept** (the linter may instead suggest renaming the
local). This rule needs name resolution from the typed HIR, so it runs in the
`--type-aware` tier (see fmt.md §Tiers).

---

## S1-7 — Manual integer byte-loops instead of `chars()` / scalar APIs

`std/json.jn` and `std/strings.jn` are written almost entirely against
`char_at` / numeric byte codes (`QUOTE is 34`, `c >= 65 and c <= 90`). This
predates the documented `String` scalar view ([`docs/strings.md`](strings.md))
and the `chr`/`is_alpha`/`is_digit` helpers. It is fast but unreadable and
ties stdlib semantics to ASCII.

**Remediation (lint-only, author-driven):** flag dense `char_at`+magic-number
clusters and suggest the scalar helpers (`s.chars()`, `is_space`, range checks
behind named predicates). Not auto-rewritten — the value semantics and the
byte/scalar boundary are subtle, and these are stability-pinned surfaces.

---

## S2-1 — `return X` on the tail expression

Jinn functions return their last expression. Explicit tail `return` is noise.

```jinn
*trim(s as String) returns String
    ...
    return s.slice(lo, hi)     # tail return
```

**Remediation:** drop `return` on a function/branch tail position; keep it for
genuine early exits. Behavior-preserving and unconditional. (Note: `guide_tour`
already models this — `*add a, b \n a + b`.)

---

## S2-2 — `equals` where value-binding `is` reads cleaner, and vice-versa

`is` binds; `equals`/`eq` compares. The corpus is mostly correct but mixes
`eq`/`equals` spellings and occasionally uses `equals` in `match` arms where a
bare pattern suffices.

```jinn
match ch
    SPACE ? self.pos is self.pos + 1     # ok
if got neq expected                       # ok
if min_i < 0 or ind < min_i               # ok
```

**Remediation (S3, normalization):** pick one spelling per project
(`equals` vs `eq`, `lte` vs `ngt`, etc.) from config and normalize. Default:
the long, prose form (`equals`, `neq`, `lte`, `gte`). Comparison-operator
aliases (`ngte`/`nlte`/`ngt`/`nlt`) are double-negative traps — always rewrite
to the positive spelling.

---

## S2-3 — Explicit casts/annotations the inferencer supplies

```jinn
offset is 0 as i64        # std/strings.jn — the 0 is already i64 here
```

HM-style inference fixes the type from use. Redundant `as T` on a literal whose
type is forced by context is removable.

**Remediation (type-aware tier):** drop `as T` when the inferred type without
it is identical. Conservative: only when the type-checker confirms equality.

---

## S2-4 — Verbose constructor field echo `keys is keys, vals is vals`

```jinn
JObject(JsonObject(keys is keys, vals is vals))
```

When a field is initialized from a local of the same name, the binding is pure
echo.

**Remediation:** support and normalize to field-punning `JsonObject(keys, vals)`
where `keys`/`vals` are in-scope locals matching the field names. (Requires the
punning sugar to exist; if not yet, this is a *language proposal* the spec
records rather than a fixer rule.)

---

## S3-1 — Whitespace, indentation, blank-line, and comment-banner normalization

The corpus uses 4-space indent (good) but ad-hoc banner comments
(`# ── … ──` of varying widths), trailing whitespace, and inconsistent blank
runs between definitions. Pure formatting.

**Remediation:** canonical 4-space indent; exactly one blank line between
top-level definitions, none at block start; strip trailing whitespace; normalize
section-banner width; ensure single trailing newline.

---

## S3-2 — Ternary chains vs `match`/`elif`

```jinn
grade is 85 > 90 ? 'A' ! 85 > 80 ? 'B' ! 'C'
```

Short ternaries are idiomatic and stay. **Deeply nested** ternaries (≥3 `!`
arms) are flagged with a suggestion to use `match`/`elif` — readability only,
never auto-rewritten.

---

## S3-3 — `vec()` + repeated `.push` for a known literal

```jinn
msg is vec()
loop(0, $ < 96, $ + 1)
    msg.push($ & 0xFF)
```

When the pushes are a static enumeration, prefer a comprehension:

```jinn
msg is [$ & 0xFF for $ in 0 to 96]
```

Offered as a fix only for the exact `vec()` + counted-loop-of-pure-push shape.

---

## Cross-cutting principle for the fixer

Every rule above is classified by **how much knowledge it needs**:

1. **Lexical/CST** (S3-1, S2-1, S2-2, S1-5 elif-collapse) — operate on tokens +
   concrete syntax tree. Always safe, always auto-applied.
2. **Flow-shape** (S1-1, S1-4, S3-3) — recognize a *closed* statement shape and
   rewrite it. Auto-applied when the shape matches exactly; otherwise skipped.
3. **Type-aware** (S1-6 self-strip, S2-3 cast-strip, S2-4 punning) — require
   name resolution / inference from the typed HIR. Run only in
   `jinn fmt --type-aware`, gated on the compiler frontend succeeding.
4. **Lint-only** (S1-3 join, S1-7 byte-loops, S3-2 ternary depth) — high
   behavioral risk or aesthetic judgment; emit a diagnostic with a suggested
   edit the author opts into, never silently applied.

This tiering is the contract `jinn fmt` enforces: tiers 1–2 run by default and
are guaranteed behavior-preserving by construction; tier 3 is opt-in and
verified against the type-checker; tier 4 never mutates without consent.
