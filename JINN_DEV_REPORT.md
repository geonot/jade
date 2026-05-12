# Mid-Level Dev's Honest Report on Jinn

**Setup:** I wrote 100 small, "obviously reasonable" snippets based on a quick read
of `jinn.md`, the examples, the bulk tests, and `std/`. Each snippet is something
I'd type within the first hour of trying a new language: bind a variable, sum
some numbers, map a vector, parse some JSON, spawn an actor.

**Score:** **71 / 100** compile and run correctly. **29 / 100** fail at compile
(0 fail at runtime). Sources at `/tmp/jinn_snippets/s001.jn` through `s100.jn`,
output log at `/tmp/jinn_snippets/results.txt`.

The good news first: most of the language **just works**. Arithmetic, strings,
ternary, loops, conditionals, pattern‑matched function clauses, generics, enums,
basic vectors, lambdas, pipelines, channels, `defer`, comprehensions — all
clean. The places I tripped were almost all in three categories: vector method
coverage, fixed‑array vs. vector confusion, and stdlib breakage.

---

## Things that surprised me in a good way

- `*fib(0) is 0` / `*fib(1) is 1` / `*fib n is …` reading like a math textbook.
  Once I saw it I wanted it in every language.
- `if 0 < x < 100` — chained comparisons.
- `'sum={a + b}'` — interpolation evaluates expressions, not just identifiers.
- `loop items` with bare `$` and `$$` for value/index — extremely terse.
- `vector[1, 2, 3]` literal, `print(v) → [1, 2, 3]`, and statement‑form
  `x.map($ * 2)` mutating `x` (you fixed that earlier this session). With those
  three working the language feels three notches more friendly.
- The `~` pipeline operator is genuinely pleasant once you accept the symbol.

---

## Big pain points (sorted by how often I hit them)

### 1. Vector methods are a minefield (s055, s056, s059, s060, s065–s070)

After reading the docs I expected this list to all work on a `vector[…]`:

| Method      | s###  | Result                                                  |
|-------------|-------|---------------------------------------------------------|
| `.map($*2)` | s057  | OK                                                      |
| `.filter`   | s058  | OK                                                      |
| `.sort()`   | s055  | `mir_codegen: unknown method 'sort'`                    |
| `.sum()`    | s056  | `unknown method 'sum'`                                  |
| `.fold(0,…)`| s059  | `unknown method 'fold'`                                 |
| `.contains` | s060  | `unknown method 'contains'`                             |
| `.any/.all` | s065  | `unknown method 'any'`                                  |
| `.find`     | s066  | `unknown method 'find'`                                 |
| `.reverse`  | s067  | `unknown method 'reverse'`                              |
| `.join(',')`| s068  | `unknown method 'join'`                                 |
| `v from a to b` | s069 | links but **`undefined reference to __jinn_vec_slice`** |
| `.take/.skip` | s070 | `unknown method 'take'`                                 |
| `.zip(b)`   | s064  | `hir: zip() argument must be a Vec`                     |

`jinn.md` advertises every one of these under "Iterator Combinators". This is
the biggest single source of "the docs are lying to me" friction. Two things
need to happen: (a) the **MIR codegen path needs parity with the legacy
`compile_vec_method` path** for the rest of the HOFs, and (b) `__jinn_vec_slice`
needs an actual definition in the runtime.

> Recommendation: a single integration test that calls every method in the
> "Available methods" list of the docs would catch all of these in one shot.

### 2. Fixed array `[a, b, c]` vs. `vector[a, b, c]` is a UX trap (s100, s064, s028)

Three seemingly identical things behave very differently:

```jinn
v1 is [1, 2, 3]              # fixed array of i64
v2 is vector[1, 2, 3]        # heap vector
v3 is vector()               # empty heap vector
```

- `v1.length` works, `v1[i]` works, but `v1.fold(…)` panics in codegen
  (`Found ArrayValue … but expected the IntValue variant`, s100).
- `v1.zip(v2)` errors with "argument must be a Vec" (s064).
- `if x in [1,3,5,7]` panics in codegen with the same `ArrayValue/IntValue`
  mismatch (s028) — even though the docs **explicitly** show
  `if x in [1, 2, 3]` as a working example.

The docs say "Works with arrays, vectors, strings, and maps". It does not.

> Recommendation: either the array literal `[…]` should desugar to `vector[…]`
> in non-typed contexts (most users will never want a fixed-size array), or
> the compiler should produce a clear E2xx telling me to wrap with `vector[…]`.
> A codegen panic with an LLVM type dump is the worst possible outcome.

### 3. `as` annotations require parens — but the docs don't say so up front (s043, s074, s075, s086)

The "Functions" section says, verbatim:

> *Parentheses are always optional on both definitions and calls.*

…then 30 lines later quietly mentions "Parentheses on the function definition
are required when using `as` type annotations". I hit this four separate times:

```jinn
*greet name as String           # parse error at `as`
*describe c as Color            # parse error at `as`
*lookup k as String             # parse error at `as`
```

Worse: when I removed the call-site parens with the *correct* paren’d definition,
the program built and produced **no output, exit 0** (silent dispatch failure):

```jinn
*greet(name as String)
    log 'hi {name}'

*main
    greet 'jinn'        # silently no-ops
```

Adding parens at the call site (`greet('jinn')`) makes it work. So the rule is
actually *also* "calls without parens don't work when the definition uses
parens". The docs explicitly promise that parens are interchangeable.

> Recommendation: pick one rule. Either (a) parens are always required when
> annotations are present (and the parse error should *say* "use parens around
> parameters when using `as`"), or (b) make paren-less call sites genuinely
> equivalent.

### 4. Several `std/` modules are themselves broken (s092, s093, s094)

I tried three completely standard things and all three failed:

- `use regex` → **`std/regex.jn:267:5: unexpected token: or`**. The standard
  library doesn't parse with its own compiler.
- `use random` → **`std/random.jn:68: bad base-16 literal`**. Same.
- `json.parse('{"x": 1}')` → **parse error in user code**. The `{` triggers
  string interpolation, so JSON literals are basically unwritable. After
  experimenting, I found `'\{` is the escape — it's not in the docs. With
  double-quoted strings (which I tried as a workaround), `\` is rejected as
  "unexpected character".

> Recommendation: CI should compile every `.jn` in `std/` on every commit, even
> if no test imports them. And the JSON example in the docs should actually
> show how to write a JSON literal — this is the *first* thing anyone tries.

### 5. Builtin/method name resolution mismatches (s089, s090, s091)

- `time_now()` is documented under "Built-in Operations" → "Global". Not
  defined. The actual function is `jinn_time_now_ns()` (per
  `examples/word_counter`).
- `(-3.5).abs()` → `unknown function '__builtin_FloatMethod(Symbol(abs))'`.
  Float method dispatch is missing for `abs`/`floor`/`ceil` even though they're
  listed in the "Float" built-ins section.
- `use math; math.sin(0.0)` works; `math.sqrt(16.0)` errors with
  `undefined function: 'math_sqrt'`. So `math.X` works for some symbols and
  not others — name mangling is inconsistent.

### 6. Generic enums by value can't be passed (s077)

```jinn
enum Option of T
    Some(T)
    None

*unwrap_or(o as Option of i64, d as i64) …
```

→ `Call parameter type does not match function signature!` from LLVM. Generic
enum ABI for value-passing is broken. This is bad because every "what's
Jinn's `Option`?" demo looks exactly like this.

### 7. Other one-off compiler bugs

- s008: `a as i32 is 100` then `log a` → `Load of undefined variable 'a'`.
  The typed binding form silently breaks scope. `a is 100 as i32` works.
- s038: `outer is for i from 0 to 5 …` (label-by-binding) → `Load of
  undefined variable 'outer'`. The exact pattern shown in the docs panics.
- s095: `gen.next()` on a generator → `unknown method 'next'`. Generators
  parse and codegen partially, but the call interface is missing in MIR.
- s096: actor `t.show` (no-arg handler call) → field-access codegen path.
  `t.show()` (with parens) is the workaround. The actor docs use the no-paren
  form.
- s098: `m + 1.0` on an NDArray (`3 by 3`) → `operator + not defined for
  NDArray of f64 [3 by 3] and ?4`. Broadcasting is documented and doesn't work.

---

## Smaller papercuts

- **`log` vs `print` are subtly different and undocumented.** I used `log` for
  scalars and `print` for vectors out of habit from the existing code; only
  `print` formats `vector[…]` as `[a, b, c]`. The docs say `log(value)` is the
  global "print to stdout", but it gives you a pointer for vectors.
- **`v.length` (property) vs `v.len()` (method).** Both documented, both work.
  Pick one. Two ways to do something this basic creates needless choice fatigue.
- **`equals` / `eq` / `neq` / `not equals` — four ways to do equality.** Same
  comment.
- **No `import { json }` style.** `use math [sin, cos, pi]` is documented and
  great, but I never figured out from the docs whether `use` is even necessary
  for `std/` modules — the examples sometimes have it and sometimes don't.
- **Error messages prefixed with `mir-codegen:` / `hir:` / `line 0:0:`** leak
  compiler internals. A user error like "method `sort` doesn't exist on
  `Vec of i64`" should look the same regardless of which pass found it. And
  "line 0:0: 1 parse error(s):" then a *real* line number on the next line is
  always two lines of noise before the actual diagnostic.
- **Codegen panics with full LLVM `Value` dumps** (s028, s100) for what are
  actually just type errors. From a user's view this looks like an ICE
  ("compiler bug, please report"), not a "you used the wrong type" message.

---

## What I'd fix first if I were on the team

1. **Method-coverage parity test.** Run every method named in `jinn.md`'s
   "Iterator Combinators" and "String" sections against a `vector[…]` and a
   `[…]` literal. Fail the build if any errors. This single test would have
   caught at least 13 of my 29 failures.
2. **`std/` self-compilation gate.** `std/regex.jn` and `std/random.jn` don't
   parse. Add `for f in std/*.jn; do jinnc --check "$f"; done` to CI.
3. **Fixed-array literal default.** Make `[a, b, c]` produce a `vector` unless
   in a typed binding (`x as [i64; 3] is [1,2,3]`). Or at minimum, when a vec
   method is called on an array, suggest "wrap with `vector[…]`".
4. **Fix the `as` parens story.** Either decide parens are mandatory with
   annotations and emit a real diagnostic, or make both forms work everywhere
   (including no-paren call sites for paren'd defs).
5. **Replace LLVM-dump panics with a real `error[Exxx]`.** No user should ever
   see `Found StructValue { struct_value: Value { … "{ ptr, ptr } { ptr
   @lambda.16.env_wrap, ptr null }" } } but expected the IntValue variant`.
6. **Document the string escape rules.** `'\{'` is the brace escape and it's
   nowhere in `jinn.md`. JSON-in-Jinn is the canonical first task and it's
   currently impossible from the docs alone.

---

## Closing impression

When Jinn works, it's *extremely* nice to write — `*fib(0) is 0` plus pattern
clauses plus inferred types is genuinely the syntax I want. The semantics
story (ownership, Perceus, value-types-by-default) is coherent and the
performance numbers back up the design.

The gap is **execution coverage of the documented surface**, not the design.
A mid-level dev hitting 29/100 things that the docs say should work will, very
quickly, lose trust that *anything* in the docs works. The fixes above are
boring grunt-work, not architectural; they would lift the experience from
"interesting, frustrating" to "this is the one I want to use".
