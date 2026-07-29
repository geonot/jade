# §5 Parser & surface syntax review

**Files:** `src/parser/` (6,108 LOC across `mod.rs`, `decl.rs`,
`expr.rs`, `stmt.rs`). **EBNF:** `jinn.ebnf`.

## 5.1 Surface syntax — quick reference (verified from working probes)

- Function definition: `*name(arg as Type, …) returns Type` body indented
- Function definition (returns inferred): `*name(arg as Type, …)`
- Binding / assignment: `x is value`. The same `is` keyword is used for
  initial binding and subsequent reassignment; the typer/HIR validator
  catches type-changing reassignments (see F-PARSE-7 below).
- Constants: `NAME is value` at top level (no `const` keyword).
- Comments: `# line comment`.
- Comparisons: `equals` / `eq`, `neq`, `lt`, `gt`, `lte`, `gte` and
  their negated forms `nlt`, `ngt`, `nlte`, `ngte` (alias mapping to
  the opposite).
- Boolean: `and`, `or`, `not`, `xor`.
- Ternary: `cond ? then ! else`.
- Pattern-arm syntax: `pattern ? body` (no fat arrow).
- Bitwise: `&`, `|`, `^`, `<<`, `>>`, `>>>` (logical shift).
- Power: `**` or `pow`.
- Actor handler: `@name` followed by indented body inside `actor T`.
- Type/struct: `type T` with indented fields `name as Type`.
- Constructor: `T(arg, arg, …)` positional.
- Vec literal: `[1, 2, 3]`.
- Channel: `channel of T (cap)`.
- Spawn: `spawn ActorType` — returns `ActorRef`.
- Store: `store Name` with indented field decls; `insert Name v1, v2`
  positional; `count Name`; `query`; `transaction`; `set`; `delete`.
- Errors: `! Variant` standalone statement is the early-return sugar.
- Deferred: `defer EXPR`.
- Move: `take EXPR` (where supported — see F-PARSE-6).
- For-range: `for i from a to b` (verified) and `for v in iter` (verified).
- Significant indentation defines blocks; no braces.

## 5.2 Findings

### F-PARSE-1 (P0 — alpha blocker): Free-floating expressions accepted as top-level decls

```
$ cat p38_just_literal.jn
42
$ jinnc p38_just_literal.jn -o /tmp/x && /tmp/x; echo rc=$?
rc=42
```

A bare integer at module scope **compiles** and the program exits
with that integer as its exit code. There is no diagnostic, and the
EBNF nominally requires a function or declaration at top level. The
parser is silently accepting expressions as decls. This is a
**correctness hole** — for example, a user who writes a typo at the
top of a file may silently get a no-op binary.

**Fix:** Reject top-level expression statements at parse time with a
diagnostic like "expected `*function`, `type`, `store`, `actor`, `use`,
or a `NAME is value` constant binding".

### F-PARSE-2 (P0): No `main` is a linker error, not a parse error

```
$ cat p39_nomain.jn
*helper()
    log(1)
$ jinnc p39_nomain.jn -o /tmp/x
/usr/bin/ld: /usr/lib/gcc/.../Scrt1.o: in function `_start':
(.text+0x17): undefined reference to `main'
collect2: error: ld returned 1 exit status
linker failed: 1
```

The compiler should refuse to invoke the linker without a `main`. The
diagnostic the user sees is C-linker garbage. The fix lives in
`src/driver/pipeline.rs`: gate object→link on presence of a `main`
DefId (unless `--lib` / `--standalone-no-main` was requested).

### F-PARSE-3 (P1): Tuple type syntax inconsistent with tuple literal

Probe:
```
*pair() returns (i64, i64)
    (1, 2)
```
Result: `line 2:27: expected returns, got NEWLINE`. So `returns (i64, i64)`
does not parse, even though `(1, 2)` does. The EBNF grants tuple
types but the parser disagrees.

### F-PARSE-4 (P1): Match guards do not parse

Probe:
```
match n
    x if x < 0 ? 'neg'
```
Result: `line 4:11: expected ?, got if`. Guards (`if expr` between
pattern and `?`) are listed in the EBNF (`pattern_directed_functions`
memory note also references guards) but the parser rejects them.

### F-PARSE-5 (P1): SIMD literal syntax broken

Probe:
```
a is SIMD of f32, 4 (1.0, 2.0, 3.0, 4.0)
```
Result: `line 3:21: unexpected token: ,`. The keyword `SIMD` (or
`simd`?) is not promoted to a token in the keyword table and the
parse rule is not implemented even if it were. The EBNF and feature
list both reference SIMD; the parser does not.

### F-PARSE-6 (P1): `take` in argument position fails

Probe:
```
*consume(v) returns i64
    v[0]

*main
    v is [1, 2, 3]
    a is consume(take v)
```
Result: `line 7:23: expected ,, got v`. `take` is in the keyword
table but the expression grammar does not accept `take EXPR` in a
function-argument position. Since the entire ownership story relies
on the user being able to write `take`, this is a **major blocker for
exercising the ownership system at all**.

### F-PARSE-7 (P1): Re-binding to a different type errors out of HIR-validate, not the typer

Probe:
```
x is 1
log(x)
x is 'now-a-string'
```
Result:
```
hir-validate: type mismatch in binding `x` at line 5: declared I64 but value is String
compilation aborted due to HIR validation errors
```

The typer **accepted** the program; HIR-validate caught the violation
afterwards. That's a "belt and suspenders" pattern but it is also a
signal that the typer is not the single source of truth for type
correctness. The user-visible message is also imperfect (`I64` should
be `i64` in user-facing diagnostics).

### F-PARSE-8 (P2): The error-cap of 20 silently truncates

`parse_program` stops after 20 errors. The user is not told there
were more. A "(20 errors; further errors suppressed)" footer would be
worth half a line of code.

### F-PARSE-9 (P2): String interpolation supports arbitrary exprs

Verified: `'hello {name}, num {n}, expr {n * 2}'` works correctly.
This is excellent ergonomics. **Recommendation:** document the rules
(escaping `{`, max nesting, type-of-interpolated-value implicit `to_string`)
prominently.

### F-PARSE-10 (P3): The pre/post-stmt splice queue is clever but invisible

`pending_pre_stmts` / `pending_post_stmts` are correct for the
`a is x() ! Variant` desugaring, but they're a special mechanism in
the parser that should arguably be in a separate "desugar" pass run
between parse and typer. Co-locating it in the parser makes both
harder to reason about. Recommended cleanup in §24, not a blocker.

## 5.3 What's correct and good

- `binop!` macro is the right pattern for binary-operator precedence
  chains; clean and DRY.
- The pre/post-stmt splice queue (despite F-PARSE-10's stylistic note)
  *works* and is a creative way to handle sugar without a separate
  desugar pass.
- The `label_stack` for break/continue labels is the right approach.
- The 20-error cap prevents pathological output explosions; the
  presentation logic at the bottom of `parse_program` correctly
  collapses multiple errors into one wrapper message with the first
  error's location.
- String interpolation with computed expressions is a genuinely nice
  ergonomic.

## 5.4 Verdict

**Not alpha-ready.** Three P0-equivalent issues
(F-PARSE-1 top-level expressions, F-PARSE-2 missing-main → linker
garbage) and several P1 syntax holes (tuple-return type, match guards,
SIMD literal, `take` in call args). The grammar in `jinn.ebnf`
documents features the parser does not implement, and the parser
accepts inputs the grammar does not. **Reconciling the two and adding
the missing rules is the single highest-leverage parser task before
alpha.**
