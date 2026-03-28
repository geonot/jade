# Jade Syntax Audit — Authoritative Review

**Date:** 2026-03-27  
**Scope:** All `.jade` source files (std/, tests/programs/, benchmarks/), tree-sitter grammar, VSCode extension, compiler (lexer/parser)

---

## Language Ethos

> Jade prefers **clear English over opaque symbols**. Minimal sugar, minimal dressing — reads like clean pseudocode. Jade defaults to **not having the extra stuff**: no empty parens, no type annotations unless needed, no ceremony. Easy to read, easy to write, clear, but powerfully expressive. Code should look like what it *means*, not what the machine *needs*.

---

## 1. Changes Required (Summary)

| # | Change | Old | New | Scope | Count |
|---|--------|-----|-----|-------|-------|
| **A** | Type annotation syntax | `x: i32` | `x as i32` | All `.jade` + compiler + treesitter + vscode | ~238 |
| **B** | Return type syntax | `-> Type` | `returns Type` | Extern decls, fn types, string_builder | ~53 |
| **C** | Modulo operator | `%` (binary) | `mod` | All `.jade` + compiler + treesitter + vscode | ~35 |
| **D** | Equality shorthand aliases | `equals` / `isnt` only | Add `eq` / `neq` as aliases | Compiler (lexer) | 0 (additive) |
| **E** | Comparison word operators | `< > <= >=` only | Add `lt gt lte gte` as aliases (keep symbols too) | Compiler (lexer) | 0 (additive) |
| **F** | Index with `at` | `arr[4]` only | Add `arr at 4` as alias | Compiler (parser) | 0 (additive) |
| **G** | Enum variant paren-free | `Bar(String)` | `Bar String` (parens optional) | `.jade` files + compiler + treesitter | ~156 |
| **H** | Augmented `%=` | `x %= 7` | `x mod= 7` *(or remove)* | syntax.jade only | 1 |
| **I** | `*RNG.method(self)` pattern | External method defs | Move into type body or extension | std/rand.jade | 3 |
| **J** | Empty parens in fn defs | `*main()` | `*main` | `.jade` files (optional, cosmetic) | ~130 |
| **K** | `string_builder() -> StringBuilder` | Arrow return type | `*string_builder returns StringBuilder` | std/strings.jade | 1 |
| **L** | Lambda fn type syntax | `(i64) -> i64` | `(i64) returns i64` | tests, syntax.jade | ~15 |

---

## 2. Detailed Findings

### A. Type Annotations — `:` → `as`

The colon `:` is used for type annotations in parameters, struct fields, and typed bindings. Jade uses `as` for casting already. The `as` token becomes **dual-purpose** (contextual):

**Current:**
```jade
extern *fopen(path: %i8, mode: %i8) -> %i8
*apply(f: (i64) -> i64, x: i64)
b: i32 is 100
type File
    handle: %i8
    path: String
```

**Target:**
```jade
extern *fopen(path as %i8, mode as %i8) returns %i8
*apply(f as (i64) returns i64, x as i64)
b as i32 is 100
type File
    handle as %i8
    path as String
```

**Files affected:**
- `std/io.jade` (29 occurrences — extern params + struct fields + method params)
- `std/os.jade` (14 — extern params)
- `std/math.jade` (1 — extern param)
- `std/fmt.jade` (6 — method params)
- `std/path.jade` (4 — method params)
- `std/strings.jade` (5 — struct fields + method params)
- `std/rand.jade` (0 — no typed params currently)
- `tests/programs/syntax.jade` (~35)
- `tests/programs/higher_order.jade` (~12)
- `tests/programs/closures.jade` (~8)
- `tests/programs/neural_net.jade` (~15)
- `tests/programs/actors_*.jade` (~4)
- `tests/programs/systems_demo.jade` (1)
- Various other test files (~30+)
- `benchmarks/closure_capture.jade` (1)

**Compiler changes:**
- `src/lexer.rs` — No change (Token::As and Token::Colon both exist)
- `src/parser.rs` — `parse_field()` and `parse_param()`: change `Token::Colon` → `Token::As`
- `src/parser/mod.rs` or `src/parser.rs` — wherever params are parsed

**Tree-sitter:**
- `grammar.js` line 82: `parameter` rule change `":"` → `"as"`
- `grammar.js` line 109: `field_definition` rule change `":"` → `"as"`

**VSCode:**
- `jade.tmLanguage.json` — `punctuation.separator.colon.jade` can be removed; `as` is already in keyword.operator.word

### B. Return Type — `->` → `returns`

**Current:**
```jade
extern *fopen(path: %i8, mode: %i8) -> %i8
*string_builder() -> StringBuilder
*apply(f: (i64) -> i64, x: i64)
```

**Target:**
```jade
extern *fopen(path as %i8, mode as %i8) returns %i8
*string_builder returns StringBuilder
*apply(f as (i64) returns i64, x as i64)
```

**Files affected:**
- `std/io.jade` — 14 extern declarations
- `std/os.jade` — 10 extern declarations
- `std/math.jade` — 1 extern declaration
- `std/strings.jade` — 1 (`string_builder`)
- `tests/programs/actors_*.jade` — 4 extern declarations
- `tests/programs/higher_order.jade` — 5 function type annotations
- `tests/programs/closures.jade` — 2 function type annotations
- `tests/programs/syntax.jade` — 6 (extern + fn types)
- `tests/programs/systems_demo.jade` — 1 extern

**Compiler changes:**
- `src/lexer.rs` — Add `Token::Returns` keyword, recognize `"returns"` string
- `src/parser.rs` — Change `Token::Arrow` to `Token::Returns` in fn def, extern, fn type parsing.  Keep `Token::Arrow` temporarily for backward compat if desired.

**Tree-sitter:**
- `grammar.js` line 71: `"->"` → `"returns"` in function_definition
- `grammar.js` line 89: `"->"` → `"returns"` in function_type
- `grammar.js` line 400: `"->"` → `"returns"` in lambda_expression

**VSCode:**
- `jade.tmLanguage.json` — Add `returns` to `keyword.operator.word.jade` pattern
- Remove `keyword.operator.arrow.jade` pattern

### C. Modulo — `%` → `mod`

The `%` prefix already means "pointer-to" in Jade (`%i8`, `%data`). Using `%` for modulo too is ambiguous at a glance. `mod` is clearer.

**Current:**
```jade
if i % 15 equals 0
b is a % b
result is (result * b) % modulus
```

**Target:**
```jade
if i mod 15 equals 0
b is a mod b
result is (result * b) mod modulus
```

**Files affected (approx 35 occurrences):**
- `tests/programs/fizzbuzz.jade` — 3
- `tests/programs/algorithms.jade` — 2
- `tests/programs/arithmetic.jade` — 1
- `tests/programs/crypto.jade` — 7
- `tests/programs/continue_loop.jade` — 1
- `tests/programs/game_of_life.jade` — 2
- `tests/programs/guessing_game.jade` — 1
- `tests/programs/power.jade` — 1
- `tests/programs/primes.jade` — 2
- `tests/programs/string_processing.jade` — 2
- `tests/programs/syntax.jade` — 4
- `std/math.jade` — 1 (gcd)
- `std/rand.jade` — 1
- `std/sort.jade` — 0
- `benchmarks/` — several

**Compiler changes:**
- `src/lexer.rs` — Add `"mod"` → `Token::Percent` (or new `Token::Mod`) keyword mapping
- Note: `%` prefix for pointer-of stays. Only binary `%` (with spaces) becomes `mod`.
- Parser: `Token::Percent` in binary expr → also accept `Token::Mod`

**Tree-sitter:**
- `grammar.js` binary_expression: change `"%"` entry → `"mod"`

**VSCode:**
- Add `mod` to `keyword.operator.word.jade`
- Remove `%` from `keyword.operator.arithmetic.jade` (keep for pointer prefix)

**Note on `%=`:** Currently `x %= 7` exists in syntax.jade. Should become `x mod= 7` or be removed. Only 1 occurrence, only in the reference file.

### D. Equality Shorthand — Add `eq` / `neq`

`equals` and `isnt` stay as primary. Add short aliases:
- `eq` → alias for `equals`
- `neq` → alias for `isnt` (clearer than `isnt` for some contexts)

**Compiler changes only:**
- `src/lexer.rs` — `"eq"` → `Token::Equals`, `"neq"` → `Token::Isnt`

**Tree-sitter:**
- `grammar.js` binary_expression: add `"eq"` at `PREC.EQUALITY`, add `"neq"` at `PREC.EQUALITY`

**VSCode:**
- Add `eq|neq` to `keyword.operator.word.jade`

### E. Comparison Word Operators — Add `lt gt lte gte`

Keep `< > <= >=` symbols (for now), add word aliases:

| Symbol | Word |
|--------|------|
| `<` | `lt` |
| `>` | `gt` |
| `<=` | `lte` |
| `>=` | `gte` |

**Compiler changes only:**
- `src/lexer.rs` — `"lt"` → `Token::Lt`, `"gt"` → `Token::Gt`, `"lte"` → `Token::LtEq`, `"gte"` → `Token::GtEq`

**Tree-sitter:**
- `grammar.js` binary_expression: add word variants alongside symbol variants

**VSCode:**
- Add `lt|gt|lte|gte` to `keyword.operator.word.jade`

### F. `at` as Index Operator

`foo at 4` = `foo[4]`. The `at` keyword is an alternative to bracket indexing.

**Compiler changes:**
- `src/lexer.rs` — `"at"` → `Token::At` (already exists as `@` dereference!)
  - **ISSUE:** `Token::At` is `@` (dereference). Need a new `Token::AtWord` or repurpose.
  - Recommendation: Add `"at"` as a keyword token separate from `@`. `Token::AtKw` or reuse `Token::At` contextually.
- `src/parser.rs` — In expression parsing: after primary, if followed by `Token::AtKw`, parse like index_expression.

**Tree-sitter:**
- Add `at` as alternative in index_expression

### G. Enum Variant Paren-Free  

**Current:**
```jade
enum Foo
    Bar(String)
    Baz(i64, i64)
```

**Target (both valid):**
```jade
enum Foo
    Bar String
    Baz i64, i64
```

Parenthesized form stays valid for clarity when needed. Paren-free is the preferred/conventional style.

**Files affected:** ~156 enum variant definitions across all test files.

**Compiler changes:**
- `src/parser.rs` — `parse_enum_variant()`: if no `(`, peek for type names instead. Comma-separated types until newline.

**Tree-sitter:**
- `grammar.js` — `variant_definition`: allow `optional(commaSep1($.type_annotation))` without parens as alternative

### H. `*RNG.method(self)` Pattern in std/rand.jade

**Current (std/rand.jade):**
```jade
*RNG.next_u64(self)
    ...
*RNG.next_f64(self)
    ...
*RNG.range(self, lo, hi)
    ...
```

This is an **external method definition** pattern — methods defined outside the type body. The `self` parameter is explicit.

**Issue:** This is the old style. Self should be inferred as the first argument. These should be:
1. Moved inside the `type RNG` body, OR
2. Use extension/impl syntax (if supported), OR
3. At minimum, remove explicit `self` param since `*TypeName.method` implies it

**Recommendation:** Move into the type body:
```jade
type RNG
    s0
    s1
    s2
    s3

    *next_u64
        result is rotate_left(self.s1 * 5, 7) * 9
        ...

    *next_f64
        (self.next_u64 >> 11) as f64 * 5.421010862427522e-20

    *range lo, hi
        span is (hi - lo) as u64
        r is self.next_u64
        lo + (r mod span) as i64
```

### I. `__` Prefixed Functions in std/sort.jade

**Finding:** `__introsort`, `__partition`, `__insertion_sort`, `__heapsort`, `__sift_down`, `__swap`

These are **private helper functions** for the sort algorithm. The `__` prefix is a naming convention for "internal, don't call directly." This is fine — it's the same convention used in:
- `std/io.jade`: `__string_from_raw`, `__file_exists`
- `std/os.jade`: `__string_from_ptr`, `__get_args`
- `std/time.jade`: `__time_monotonic`

**Assessment:** These `__` functions are **compiler builtins** (intrinsics mapped in the typer). The naming convention is appropriate. No change needed.

### J. Empty Parens in Function Definitions

**Current:**  
```jade
*main()
*close_file()
*eof()
*hello
```

Both forms exist. Jade convention is paren-free when no args. ~130 fn defs use empty `()`.

**This is cosmetic for the batch script.** Parser already supports both. The `.jade` source files should be updated to prefer:
```jade
*main
*close_file
*eof
```

### K. Function Type Syntax

**Current:** `(i64) -> i64`  
**Target:** `(i64) returns i64`

This follows from change B. All function type annotations in parameters change.

### L. Augmented Assignment `%=` → `mod=`

Only in syntax.jade reference. Either:
- Rename to `mod=` for consistency
- Remove (it's rarely used)

---

## 3. Patterns That Are Already Correct ✓

| Pattern | Status | Notes |
|---------|--------|-------|
| Pipeline `~` | ✓ Correct | No `\|>` found anywhere |
| `equals` / `isnt` | ✓ Correct | No `==` or `!=` in active code |
| `is` for binding | ✓ Correct | Universal |
| `*` for function def | ✓ Correct | Universal |
| `and` / `or` / `not` | ✓ Correct | No `&&` `\|\|` `!` |
| Ternary `? !` | ✓ Correct | ~50 uses |
| `of` for generics | ✓ Correct | `Option of T`, `Vec of String` |
| Paren-free calls | ✓ Correct | `log 'hi'`, `log x` |
| `for x from a to b` | ✓ Correct | Range loops |
| `for x in arr` | ✓ Correct | Iterator loops |
| `yield` from loops | ✓ Correct | |
| `match` / `?` arms | ✓ Correct | |
| String interpolation `{expr}` | ✓ Correct | |
| `__` for intrinsics | ✓ Correct | Convention, not syntax |

---

## 4. Ideas for Consideration

Things to evaluate from other languages that fit Jade's ethos:

### Already Fits Jade's Ethos
- **`unless`** (Ruby) — `unless condition` instead of `if not condition`. Reads clearer.
- **`until`** (Ruby) — `until condition` instead of `while not condition`. Natural English.
- **`then`** — Optional noise word: `if x gt 5 then log x`. Could make one-liners clearer.
- **`where`** clauses — `result is compute x where x is normalize input`. Declarative.
- **`given`** — Alternative to `match`: `given x` reads as "given x, what do we do?"

### Probably Don't Want
- **`let`/`var`/`val`** — Jade uses `is`, which is better.
- **`:=`/`<-`** — Assignment operators are noise. `is` is perfect.
- **`begin`/`end`** — Jade uses indentation. Good.
- **`def`/`func`/`function`** — Jade uses `*`, which is minimal and distinctive.
- **`=>` fat arrow** — Jade has `?` for match arms and `~` for pipe. No need.
- **Semicolons** — Never.
- **Braces** — Never.

### Things to Watch
- **`with`** — Could be useful for context managers / resource scoping. `with open('f') as fp`.
- **`is a`/`is an`** — Type checking: `x is a String` → bool. Reads naturally.
- **`responds to`** — Duck typing: `x responds to .length`. Interesting for traits.
- **`otherwise`** — Alternative to `else`. More verbose but reads well.
- **Named arguments without `is`** — Currently `Point(x is 10, y is 20)`. This is good as-is.

---

## 5. Update Checklist

### Phase 1: Compiler (src/)
- [ ] `src/lexer.rs` — Add tokens: `Returns`, keyword mappings for `eq`, `neq`, `gt`, `lt`, `gte`, `lte`, `mod`, `at` (word)
- [ ] `src/lexer.rs` — Add `"returns"` → `Token::Returns` keyword
- [ ] `src/lexer.rs` — Map `"mod"` → `Token::Percent` (or new Mod token, same semantics)
- [ ] `src/lexer.rs` — Map `"eq"` → `Token::Equals`, `"neq"` → `Token::Isnt`
- [ ] `src/lexer.rs` — Map `"lt"` → `Token::Lt`, `"gt"` → `Token::Gt`, `"lte"` → `Token::LtEq`, `"gte"` → `Token::GtEq`
- [ ] `src/lexer.rs` — Add `"at"` keyword for index-by-word
- [ ] `src/parser.rs` — Change `Token::Colon` → `Token::As` in `parse_field()` and `parse_param()`
- [ ] `src/parser.rs` — Change `Token::Arrow` → `Token::Returns` in fn def return type, extern return type
- [ ] `src/parser.rs` — Support `at` as index operator (infix, same precedence as `[`)
- [ ] `src/parser.rs` — Support paren-free enum variants
- [ ] `src/parser.rs` — Function type parsing: `(types) returns type` instead of `(types) -> type`

### Phase 2: Source Files (std/, tests/, benchmarks/)
- [ ] All `:` type annotations → `as`
- [ ] All `->` return types → `returns`
- [ ] All binary `%` → `mod`
- [ ] All `%=` → `mod=` (or remove)
- [ ] `std/rand.jade` — Move methods into type body
- [ ] `std/strings.jade` — `*string_builder() -> StringBuilder` → `*string_builder returns StringBuilder`
- [ ] Empty `()` in fn defs → remove (cosmetic, optional)
- [ ] Enum variants — remove parens where possible (cosmetic, optional)
- [ ] Update `tests/programs/syntax.jade` as the canonical reference

### Phase 3: Tree-sitter Grammar
- [ ] `grammar.js` — parameter rule: `":"` → `"as"` 
- [ ] `grammar.js` — field_definition rule: `":"` → `"as"`
- [ ] `grammar.js` — function_definition: `"->"` → `"returns"`
- [ ] `grammar.js` — function_type: `"->"` → `"returns"`
- [ ] `grammar.js` — lambda_expression: `"->"` → `"returns"`
- [ ] `grammar.js` — binary_expression: add `"mod"`, `"eq"`, `"neq"`, `"gt"`, `"lt"`, `"gte"`, `"lte"`
- [ ] `grammar.js` — binary_expression: replace `"%"` with `"mod"` (or keep both)
- [ ] `grammar.js` — index_expression: add `"at"` alternative
- [ ] `grammar.js` — variant_definition: make parens optional
- [ ] `grammar.js` — log_expression: parens should be optional (`log x` not just `log(x)`)
- [ ] Regenerate parser: `cd tree-sitter-jade && npx tree-sitter generate`

### Phase 4: VSCode Extension
- [ ] `jade.tmLanguage.json` — Add `returns|eq|neq|gt|lt|gte|lte|mod|at` to keyword.operator.word
- [ ] `jade.tmLanguage.json` — Remove `keyword.operator.arrow.jade` (`->` pattern)
- [ ] `jade.tmLanguage.json` — Remove `%` from arithmetic operators (keep for pointer prefix)
- [ ] `jade.tmLanguage.json` — Consider removing `punctuation.separator.colon.jade`
- [ ] `jade.tmLanguage.json` — Add `unless|until` if adopted
- [ ] `package.json` — Update keywords if needed

---

## 6. Migration Risk Assessment

| Change | Risk | Notes |
|--------|------|-------|
| `:` → `as` | **Medium** | `as` is dual-purpose (cast + annotation). Parser must handle context. |
| `->` → `returns` | **Low** | Isolated to extern decls and fn types. Clean substitution. |
| `%` → `mod` | **Low** | `%` already overloaded (pointer prefix). `mod` is unambiguous. Must distinguish `%` prefix from binary `%`. |
| `eq`/`neq` aliases | **None** | Purely additive. |
| `lt`/`gt`/`lte`/`gte` | **None** | Purely additive, symbols stay. |
| `at` index | **Low** | New syntax path, doesn't break existing `[]`. |
| Enum paren-free | **Medium** | Parser must disambiguate `Variant typename` from `Variant\nnextvariant`. |
| Empty parens removal | **None** | Parser already supports both forms. |
